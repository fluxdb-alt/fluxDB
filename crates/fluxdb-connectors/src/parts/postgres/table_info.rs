// PostgreSQL 表结构元数据（T08）：把 pg_catalog 目录投影到 core 的 TableStructure。
//
// 单一综合加载器 `pg_table_metadata` 一次建连读出 列/主键/索引/FK/CHECK/触发器/注释，
// 供 DDL 重建与各 table-info tab 复用；视图 DDL 另走 `pg_get_viewdef`。
// 名字是值时用 `$n` 参数绑定，是标识符时逐段 `pg_quote_identifier`；同名对象由
// database+schema 限定，FK 的多列配对按 conkey/confkey 同序位投影，不做笛卡尔积。

/// 表/视图结构元数据总入口：解析 `path` 定位 database.schema.relation 后读全量结构。
fn pg_table_metadata(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<TableStructure> {
    if config.kind != DatabaseKind::Postgres {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }
    let database = path
        .database
        .clone()
        .filter(|db| !db.is_empty())
        .unwrap_or_else(|| pg_request_database(config, None));
    let schema_name = path
        .schema
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Connection, "缺少 schema 上下文"))?;

    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        pg_load_table_structure(&session.client, &database, &schema_name, &path.name).await
    })
}

/// 在已建立的客户端上读全量结构（供各 table-info tab 与 DDL 复用同一连接，避免嵌套 runtime）。
async fn pg_load_table_structure(
    client: &tokio_postgres::Client,
    database: &str,
    schema_name: &str,
    relation_name: &str,
) -> fluxdb_core::Result<TableStructure> {
    // 定位关系并读类型/注释/行数（视图/物化视图 relkind=v/m 单独走 viewdef）。
    let rel_row = client
        .query_opt(
            "SELECT c.oid, c.relkind::text, c.relname, \
                    pg_catalog.obj_description(c.oid, 'pg_class') AS comment \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema_name, &relation_name],
        )
        .await
        .map_err(pg_error)?;

    let (rel_oid, relkind, relname, comment) = match rel_row {
        Some(row) => {
            let oid: u32 = row.get(0);
            let relkind: String = row.get(1);
            let name: String = row.get(2);
            let comment: Option<String> = row.get(3);
            (oid, relkind, name, comment)
        }
        None => {
            return Err(Error::new(
                ErrorKind::Query,
                format!("关系 {schema_name}.{relation_name} 不存在"),
            ));
        }
    };

    let kind = if matches!(relkind.as_str(), "v" | "m") {
        ObjectKind::View
    } else {
        ObjectKind::Table
    };
    let is_view = kind == ObjectKind::View;

    let columns = if is_view {
        Vec::new()
    } else {
        pg_load_columns(client, rel_oid).await?
    };
    let (primary_key, foreign_keys, checks, unique_keys) = if is_view {
        (Vec::new(), Vec::new(), Vec::new(), Vec::new())
    } else {
        pg_load_constraints(client, rel_oid, schema_name).await?
    };
    let indexes = if is_view {
        Vec::new()
    } else {
        pg_load_indexes(client, rel_oid).await?
    };
    let triggers = pg_load_triggers(client, rel_oid, schema_name).await?;

    Ok(TableStructure {
        database: Some(database.to_string()),
        schema: Some(schema_name.to_string()),
        name: relname,
        kind,
        columns,
        primary_key,
        foreign_keys,
        checks,
        unique_keys,
        indexes,
        triggers,
        comment: comment.filter(|c| !c.trim().is_empty()),
    })
}

/// 读取列：`pg_attribute` + `pg_type` + `pg_attrdef` + `pg_class`（主键标记）。
///
/// 只取 `attnum>0` 且未 `attisdropped` 的列；序位即 `attnum` 序，保证字段顺序不因
/// dropped column 错位。identity/generated 由 `attidentity`/`attgenerated` 判定。
async fn pg_load_columns(
    client: &tokio_postgres::Client,
    rel_oid: u32,
) -> fluxdb_core::Result<Vec<ColumnMeta>> {
    let rows = client
        .query(
            "SELECT a.attname, a.attnum::int4, pg_catalog.format_type(a.atttypid, a.atttypmod), \
                    tn.nspname, t.typname, a.attnotnull, a.attidentity::text, a.attgenerated::text, \
                    pg_catalog.col_description(a.attrelid, a.attnum) AS comment, \
                    pg_catalog.pg_get_expr(ad.adbin, ad.adrelid) AS default_expr \
             FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
             JOIN pg_catalog.pg_namespace tn ON tn.oid = t.typnamespace \
             LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
             WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    let mut columns = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.get(0);
        let ordinal: i32 = row.get(1);
        let data_type: String = row.get(2);
        let type_schema: String = row.get(3);
        let type_name: String = row.get(4);
        let not_null: bool = row.get(5);
        let identity: String = row.get(6);
        let generated: String = row.get(7);
        let comment: Option<String> = row.get(8);
        let default_expr: Option<String> = row.get(9);
        let is_identity = !identity.is_empty();
        let is_generated = !generated.is_empty();
        let default_expr = default_expr.filter(|d| !d.trim().is_empty());
        columns.push(ColumnMeta {
            name,
            ordinal: ordinal as u32,
            data_type,
            type_schema: Some(type_schema),
            type_name: Some(type_name),
            // identity/generated 列由数据库持有，不可手写（T09 据此确定可编辑性）。
            nullable: !not_null,
            default_expr,
            is_identity,
            identity_generation: is_identity.then(|| identity),
            is_generated,
            is_editable: !is_identity && !is_generated,
            primary_key: false, // 由主键索引回填
            unique_key: false,  // 由唯一键约束回填
            comment,
        });
    }

    // 主键标记回填（与 pg_load_constraints 会重复一点，但列主键标记独立于约束展示）。
    let pk = pg_load_primary_key_columns(client, rel_oid).await?;
    for col in columns.iter_mut() {
        col.primary_key = pk.contains(&col.name);
    }
    Ok(columns)
}

/// 主键列名集合（pg_index.indisprimary → indkey）。
async fn pg_load_primary_key_columns(
    client: &tokio_postgres::Client,
    rel_oid: u32,
) -> fluxdb_core::Result<Vec<String>> {
    let rows = client
        .query(
            "SELECT unnest(i.indkey::int2[])::int4 AS attnum \
             FROM pg_catalog.pg_index i \
             WHERE i.indrelid = $1 AND i.indisprimary ORDER BY 1",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;
    let attnums: Vec<i32> = rows.iter().map(|r| r.get(0)).collect();
    if attnums.is_empty() {
        return Ok(Vec::new());
    }
    let names = client
        .query(
            "SELECT a.attname FROM pg_catalog.pg_attribute a \
             WHERE a.attrelid = $1 AND a.attnum = ANY($2::int4[]) AND a.attnum > 0 \
             ORDER BY a.attnum",
            &[&rel_oid, &attnums],
        )
        .await
        .map_err(pg_error)?;
    Ok(names.iter().map(|r| r.get::<_, String>(0)).collect())
}

/// 读取约束：主键/唯一/FK/CHECK（pg_constraint），FK 按 conkey/confkey 同序位配对。
async fn pg_load_constraints(
    client: &tokio_postgres::Client,
    rel_oid: u32,
    schema_name: &str,
) -> fluxdb_core::Result<(
    Vec<String>,
    Vec<ForeignKeyMeta>,
    Vec<CheckMeta>,
    Vec<UniqueKeyMeta>,
)> {
    let rows = client
        .query(
            "SELECT c.conname, c.contype::text, \
                    pg_catalog.pg_get_constraintdef(c.oid), \
                    COALESCE(c.conkey::int2[], '{}'::int2[])::int4[] AS conkey, \
                    COALESCE(c.confkey::int2[], '{}'::int2[])::int4[] AS confkey, \
                    c.confrelid, c.confupdtype::text, c.confdeltype::text, \
                    c.confmatchtype::text, c.condeferrable, c.condeferred \
             FROM pg_catalog.pg_constraint c \
             WHERE c.conrelid = $1 ORDER BY c.conname",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    let mut primary_key: Vec<String> = Vec::new();
    let mut foreign_keys: Vec<ForeignKeyMeta> = Vec::new();
    let mut checks: Vec<CheckMeta> = Vec::new();
    let mut unique_keys: Vec<UniqueKeyMeta> = Vec::new();

    for row in rows {
        let name: String = row.get(0);
        let contype: String = row.get(1);
        let definition: String = row.get(2);
        let conkey: Vec<i32> = row.get(3);
        match contype.as_str() {
            "p" => primary_key = pg_attnums_to_names(client, rel_oid, &conkey).await?,
            "f" => {
                let confrelid: u32 = row.get(5);
                let on_update: String = row.get(6);
                let on_delete: String = row.get(7);
                let match_type: String = row.get(8);
                let deferrable: bool = row.get(9);
                let initially_deferred: bool = row.get(10);
                let confkey: Vec<i32> = row.get(4);
                foreign_keys.push(
                    pg_build_foreign_key(
                        client,
                        name,
                        rel_oid,
                        &conkey,
                        confrelid,
                        &confkey,
                        &on_update,
                        &on_delete,
                        &match_type,
                        deferrable,
                        initially_deferred,
                        definition,
                    )
                    .await?,
                );
            }
            "u" => {
                let columns = pg_attnums_to_names(client, rel_oid, &conkey).await?;
                unique_keys.push(UniqueKeyMeta {
                    name,
                    columns,
                    is_constraint: true,
                    definition,
                });
            }
            "c" => checks.push(CheckMeta {
                name,
                expression: pg_check_expression(&definition),
                definition,
            }),
            _ => {} // 排除约束(x)/外键以外暂不细分；PG 约束类型仅 p/u/f/c/x。
        }
    }

    // 独立唯一索引（contype 不在 pg_constraint，但在 pg_index.indisunique 且非主键、非常量）。
    // 约束展示以 pg_constraint 的 'u' 为准；纯唯一索引在索引 tab 呈现（IndexMeta.is_unique）。
    let _ = schema_name;
    Ok((primary_key, foreign_keys, checks, unique_keys))
}

/// 由 attnum 数组解析为本表列名（保持序位）。
async fn pg_attnums_to_names(
    client: &tokio_postgres::Client,
    rel_oid: u32,
    attnums: &[i32],
) -> fluxdb_core::Result<Vec<String>> {
    if attnums.is_empty() {
        return Ok(Vec::new());
    }
    let rows = client
        .query(
            "SELECT a.attnum::int4, a.attname FROM pg_catalog.pg_attribute a \
             WHERE a.attrelid = $1 AND a.attnum = ANY($2::int4[]) AND a.attnum > 0 \
             ORDER BY a.attnum",
            &[&rel_oid, &attnums],
        )
        .await
        .map_err(pg_error)?;
    let mut by_attnum: std::collections::HashMap<i32, String> = std::collections::HashMap::new();
    for r in rows {
        let an: i32 = r.get(0);
        let nm: String = r.get(1);
        by_attnum.insert(an, nm);
    }
    Ok(attnums.iter().filter_map(|an| by_attnum.get(an).cloned()).collect())
}

/// 组装外键领域模型：本表列 conkey<->被引用表列 confkey 同序位配对（非笛卡尔积）。
#[allow(clippy::too_many_arguments)]
async fn pg_build_foreign_key(
    client: &tokio_postgres::Client,
    name: String,
    rel_oid: u32,
    conkey: &[i32],
    confrelid: u32,
    confkey: &[i32],
    on_update: &str,
    on_delete: &str,
    match_type: &str,
    deferrable: bool,
    initially_deferred: bool,
    definition: String,
) -> fluxdb_core::Result<ForeignKeyMeta> {
    let columns = pg_attnums_to_names(client, rel_oid, conkey).await?;
    let ref_columns = pg_attnums_to_names(client, confrelid, confkey).await?;
    // 被引用表名与 schema。
    let ref_row = client
        .query_opt(
            "SELECT n.nspname, c.relname FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = $1",
            &[&confrelid],
        )
        .await
        .map_err(pg_error)?;
    let (ref_schema, ref_table) = match ref_row {
        Some(r) => (Some(r.get::<_, String>(0)), r.get::<_, String>(1)),
        None => (None, String::new()),
    };
    Ok(ForeignKeyMeta {
        name,
        columns,
        ref_schema,
        ref_table,
        ref_columns,
        on_delete: map_fk_action(on_delete),
        on_update: map_fk_action(on_update),
        match_type: Some(match_type.to_string()),
        deferrable,
        initially_deferred,
        definition,
    })
}

/// PG 外键动作码 → 语义文本（`a`=NO ACTION、`r`=RESTRICT、`c`=CASCADE、`n`=SET NULL、`d`=SET DEFAULT）。
fn map_fk_action(code: &str) -> Option<String> {
    Some(
        match code {
            "a" => "NO ACTION",
            "r" => "RESTRICT",
            "c" => "CASCADE",
            "n" => "SET NULL",
            "d" => "SET DEFAULT",
            _ => return None,
        }
        .to_string(),
    )
}

/// 从 `CHECK (expr)` 定义提取纯表达式（重建 DDL 时与其它约束拼接）。
fn pg_check_expression(definition: &str) -> String {
    let trimmed = definition.trim();
    trimmed
        .strip_prefix("CHECK (")
        .and_then(|s| s.strip_suffix(')'))
        .map(|s| s.to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

/// 读取表级用户触发器（排除内部约束触发器），投影事件/时相/级别/函数身份与完整定义。
async fn pg_load_triggers(
    client: &tokio_postgres::Client,
    rel_oid: u32,
    schema_name: &str,
) -> fluxdb_core::Result<Vec<TriggerMeta>> {
    let rows = client
        .query(
            "SELECT t.tgname, t.tgtype::int4, t.tgenabled::text, \
                    pg_catalog.pg_get_triggerdef(t.oid), \
                    n.nspname || '.' || p.proname AS function \
             FROM pg_catalog.pg_trigger t \
             JOIN pg_catalog.pg_proc p ON p.oid = t.tgfoid \
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             WHERE t.tgrelid = $1 AND NOT t.tgisinternal \
             ORDER BY t.tgname",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    let mut triggers = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.get(0);
        let tgtype: i32 = row.get(1);
        let enabled: String = row.get(2);
        let definition: String = row.get(3);
        let function: String = row.get(4);
        triggers.push(TriggerMeta {
            name,
            event: pg_trigger_event(tgtype),
            timing: pg_trigger_timing(tgtype),
            level: if tgtype & 0x01 != 0 { "ROW".to_string() } else { "STATEMENT".to_string() },
            function,
            enabled: !matches!(enabled.as_str(), "D" | "O"),
            definition,
        });
    }
    let _ = schema_name;
    Ok(triggers)
}

/// 触发事件文本：tgtype 低字节事件位（INSERT=4/DELETE=8/UPDATE=16/TRUNCATE=32）。
fn pg_trigger_event(tgtype: i32) -> String {
    let mut events = Vec::new();
    if tgtype & 0x04 != 0 {
        events.push("INSERT");
    }
    if tgtype & 0x08 != 0 {
        events.push("DELETE");
    }
    if tgtype & 0x10 != 0 {
        events.push("UPDATE");
    }
    if tgtype & 0x20 != 0 {
        events.push("TRUNCATE");
    }
    events.join(" OR ")
}

/// 触发时相：tgtype 位（BEFORE=2/INSTEAD=64，否则 AFTER）。
fn pg_trigger_timing(tgtype: i32) -> String {
    if tgtype & 0x40 != 0 {
        "INSTEAD OF".to_string()
    } else if tgtype & 0x02 != 0 {
        "BEFORE".to_string()
    } else {
        "AFTER".to_string()
    }
}

/// 索引 tab：从 TableStructure 投影到轻量 `IndexInfo`；表达式键项保留为 `(expr)` 文本。
fn pg_indexes_from_structure(structure: &TableStructure) -> Vec<IndexInfo> {
    structure
        .indexes
        .iter()
        .map(|index| IndexInfo {
            name: index.name.clone(),
            columns: index
                .columns
                .iter()
                .map(|item| {
                    item.column
                        .clone()
                        .or_else(|| item.expression.clone())
                        .unwrap_or_default()
                })
                .collect(),
            is_unique: index.is_unique,
            is_primary: index.is_primary,
            index_type: index.index_type.clone(),
            comment: None,
        })
        .collect()
}

/// 外键 tab：多列外键按序位折叠为逗号拼接（兼容现有单列展示，不丢复合键序位）。
fn pg_foreign_keys_from_structure(structure: &TableStructure) -> Vec<ForeignKeyInfo> {
    structure
        .foreign_keys
        .iter()
        .map(|fk| ForeignKeyInfo {
            name: fk.name.clone(),
            column: fk.columns.join(", "),
            ref_schema: fk.ref_schema.clone(),
            ref_table: fk.ref_table.clone(),
            ref_column: fk.ref_columns.join(", "),
        })
        .collect()
}

/// 触发器 tab：从 TableStructure 投影到轻量 `TriggerInfo`。
fn pg_triggers_from_structure(structure: &TableStructure) -> Vec<TriggerInfo> {
    structure
        .triggers
        .iter()
        .map(|trigger| TriggerInfo {
            name: trigger.name.clone(),
            event: format!("{} {}", trigger.timing, trigger.event),
            timing: trigger.level.clone(),
            body: Some(trigger.definition.clone()),
        })
        .collect()
}

/// 表 DDL：视图/物化视图走 `pg_get_viewdef`；普通表由 TableStructure 组装重建。
fn pg_table_ddl(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<String> {
    let database = path
        .database
        .clone()
        .filter(|db| !db.is_empty())
        .unwrap_or_else(|| pg_request_database(config, None));
    let schema_name = path
        .schema
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Connection, "缺少 schema 上下文"))?;
    let relation_name = path.name.clone();
    let is_view = path.kind == ObjectKind::View;

    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        if is_view {
            let row = session
                .client
                .query_opt(
                    "SELECT pg_catalog.pg_get_viewdef(c.oid, true) \
                     FROM pg_catalog.pg_class c \
                     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('v', 'm')",
                    &[&schema_name, &relation_name],
                )
                .await
                .map_err(pg_error)?;
            let Some(row) = row else {
                return Err(Error::new(
                    ErrorKind::Query,
                    format!("视图 {schema_name}.{relation_name} 不存在"),
                ));
            };
            let viewdef: String = row.get(0);
            return Ok(format!(
                "CREATE OR REPLACE VIEW {} AS\n{}",
                pg_qualified_relation(&schema_name, &relation_name),
                viewdef
            ));
        }

        // 复用当前会话读结构（不再经 pg_table_metadata 重建连接，避免嵌套 runtime）。
        let structure = pg_load_table_structure(
            &session.client,
            &database,
            &schema_name,
            &relation_name,
        )
        .await?;
        Ok(build_table_ddl(&structure))
    })
}

/// 由 TableStructure 组装普通表完整 DDL（含列/约束/索引/注释，顺序可重建）。
fn build_table_ddl(structure: &TableStructure) -> String {
    let schema = structure.schema.as_deref().unwrap_or("public");
    let relation = pg_qualified_relation(schema, &structure.name);
    let mut lines: Vec<String> = Vec::new();

    // 表体：列 + 主键 + 唯一 + CHECK + 外键。
    let mut table_parts: Vec<String> = Vec::new();
    for col in &structure.columns {
        table_parts.push(pg_column_line(col));
    }
    if !structure.primary_key.is_empty() {
        table_parts.push(format!(
            "PRIMARY KEY ({})",
            pg_quote_csv(&structure.primary_key)
        ));
    }
    for unique in &structure.unique_keys {
        table_parts.push(format!(
            "CONSTRAINT {} UNIQUE ({})",
            pg_quote_identifier(&unique.name),
            pg_quote_csv(&unique.columns)
        ));
    }
    for check in &structure.checks {
        table_parts.push(format!(
            "CONSTRAINT {} CHECK ({})",
            pg_quote_identifier(&check.name),
            check.expression
        ));
    }
    for fk in &structure.foreign_keys {
        table_parts.push(pg_fk_line(fk));
    }
    lines.push(format!(
        "CREATE TABLE {relation} (\n  {}\n);",
        table_parts.join(",\n  ")
    ));

    // 独立索引：跳过主键索引与唯一约束的背衬索引（其名与约束同名），避免重复。
    let constraint_index_names: std::collections::HashSet<&str> = structure
        .unique_keys
        .iter()
        .map(|u| u.name.as_str())
        .collect();
    for index in &structure.indexes {
        if index.is_primary || constraint_index_names.contains(index.name.as_str()) {
            continue;
        }
        lines.push(format!("{};", index.definition.trim_end_matches(';')));
    }

    // 注释：表 + 列。
    if let Some(comment) = structure.comment.as_deref().filter(|c| !c.trim().is_empty()) {
        lines.push(format!(
            "COMMENT ON TABLE {} IS {};",
            relation,
            pg_sql_string_literal(comment)
        ));
    }
    for col in &structure.columns {
        if let Some(comment) = col.comment.as_deref().filter(|c| !c.trim().is_empty()) {
            lines.push(format!(
                "COMMENT ON COLUMN {}.{} IS {};",
                relation,
                pg_quote_identifier(&col.name),
                pg_sql_string_literal(comment)
            ));
        }
    }

    lines.join("\n\n")
}

/// 单列定义行：`"name" type [DEFAULT x] [ident/generated] [NOT NULL]`。
fn pg_column_line(col: &ColumnMeta) -> String {
    let mut line = format!(
        "{} {}",
        pg_quote_identifier(&col.name),
        col.data_type
    );
    if let Some(default) = col.default_expr.as_deref().filter(|d| !d.is_empty()) {
        line.push_str(&format!(" DEFAULT {default}"));
    }
    if col.is_identity {
        let generation = if col.identity_generation.as_deref() == Some("a") {
            "ALWAYS"
        } else {
            "BY DEFAULT"
        };
        line.push_str(&format!(" GENERATED {generation} AS IDENTITY"));
    } else if col.is_generated {
        line.push_str(" GENERATED ALWAYS AS (expr) STORED");
    }
    if !col.nullable {
        line.push_str(" NOT NULL");
    }
    line
}

/// 外键约束行完整重建（含动作/match/延迟属性；无默认值动作）。
fn pg_fk_line(fk: &ForeignKeyMeta) -> String {
    let ref_schema = fk.ref_schema.as_deref().unwrap_or("public");
    let mut line = format!(
        "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}.{} ({})",
        pg_quote_identifier(&fk.name),
        pg_quote_csv(&fk.columns),
        pg_quote_identifier(ref_schema),
        pg_quote_identifier(&fk.ref_table),
        pg_quote_csv(&fk.ref_columns)
    );
    if let Some(m) = &fk.match_type {
        if matches!(m.as_str(), "f" | "p" | "s") && m != "s" {
            line.push_str(&format!(
                " MATCH {}",
                match m.as_str() {
                    "f" => "FULL",
                    "p" => "PARTIAL",
                    _ => "SIMPLE",
                }
            ));
        }
    }
    if let Some(action) = &fk.on_delete {
        line.push_str(&format!(" ON DELETE {action}"));
    }
    if let Some(action) = &fk.on_update {
        line.push_str(&format!(" ON UPDATE {action}"));
    }
    if fk.deferrable {
        line.push_str(if fk.initially_deferred {
            " DEFERRABLE INITIALLY DEFERRED"
        } else {
            " DEFERRABLE"
        });
    }
    line
}

/// 逗号分隔的引号标识符列表。
fn pg_quote_csv(columns: &[String]) -> String {
    columns
        .iter()
        .map(|c| pg_quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ")
}

/// schema 限定关系名（双引号逐段）。
fn pg_qualified_relation(schema: &str, relation: &str) -> String {
    format!(
        "{}.{}",
        pg_quote_identifier(schema),
        pg_quote_identifier(relation)
    )
}

/// SQL 字符串字面量（单引号转义）。
fn pg_sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 面向 tab 的公开接口（connector 调用）：索引/外键/触发器/DDL。
fn pg_list_indexes(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
    pg_table_metadata(config, path).map(|s| pg_indexes_from_structure(&s))
}

fn pg_list_foreign_keys(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    pg_table_metadata(config, path).map(|s| pg_foreign_keys_from_structure(&s))
}

fn pg_list_triggers(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    pg_table_metadata(config, path).map(|s| pg_triggers_from_structure(&s))
}
