//! 目标端结构变更 SQL 的生成与执行。
//!
//! 数据同步只在「目标表缺失 + 用户显式允许建表」时用到这里；结构同步会在此基础上
//! 扩展 ALTER 生成。跨数据库类型不翻译 DDL：直接报错，避免生成破坏性语句。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, DatabaseKind, Error, ErrorKind, ObjectPath, QueryExecutionOptions, QueryMode,
    QueryRequest, WriteIntent,
};

use super::model::{
    ColumnDef, ForeignKeyDef, IndexDef, StructureChange, StructureChangeAction,
    StructureChangeDetail, StructureObject,
};

/// 目标表不存在时按源端 DDL 建表。已存在则什么都不做。
pub(crate) fn ensure_target_table(
    source_config: &ConnectionConfig,
    target_config: &ConnectionConfig,
    source: &ObjectPath,
    target: &ObjectPath,
) -> fluxdb_core::Result<()> {
    let existing = connector_for(target_config)?.list_completion_columns(
        target.database.as_deref(),
        target.schema.as_deref(),
        &target.name,
    )?;
    if !existing.is_empty() {
        return Ok(());
    }
    if !super::data_sync::structure_kinds_compatible(source_config.kind, target_config.kind) {
        // 跨类型：按目标方言翻译类型/默认值/注释后建表；外键留给结构同步统一添加。
        let metadata = connector_for(source_config)?.load_table_design_metadata(source)?;
        let table =
            super::structure_diff::table_def_from_metadata(&metadata).map_err(|reason| {
                Error::new(
                    ErrorKind::Unsupported,
                    format!("{}：{}", source.name, reason.message()),
                )
            })?;
        let (mapped, _) =
            super::cross_type_ddl::map_table_def(source_config.kind, target_config.kind, &table);
        for statement in
            super::cross_type_ddl::create_table_statements(target_config.kind, target, &mapped)
                .statements
        {
            execute_change_sql(target_config, target, &statement)?;
        }
        return Ok(());
    }
    let ddl = connector_for(source_config)?.table_ddl(source)?;
    let ddl = rewrite_table_ddl(source_config.kind, source, target, &ddl);
    execute_change_sql(target_config, target, &ddl)
}

/// 把源端 DDL 里的表名替换成目标端的限定名。
pub(crate) fn rewrite_table_ddl(
    kind: DatabaseKind,
    source: &ObjectPath,
    target: &ObjectPath,
    ddl: &str,
) -> String {
    let dialect = fluxdb_core::sql_dialect(kind);
    let source_name = dialect.qualified_object_name(source);
    let target_name = dialect.qualified_object_name(target);
    if source_name != target_name && ddl.contains(&source_name) {
        return ddl.replace(&source_name, &target_name);
    }
    // MySQL 家族等连接器返回的建表语句常只带裸表名，限定名替换匹配不到；
    // 此时只替换第一处裸表名（CREATE TABLE 的位置），避免误改列名/索引名。
    let source_bare = dialect.quote_identifier(&source.name);
    let target_bare = dialect.quote_identifier(&target.name);
    if source_bare != target_bare {
        return ddl.replacen(&source_bare, &target_bare, 1);
    }
    ddl.to_string()
}

/// 执行一条结构变更语句。只读连接在发语句前拦截。
pub(crate) fn execute_change_sql(
    config: &ConnectionConfig,
    path: &ObjectPath,
    text: &str,
) -> fluxdb_core::Result<()> {
    crate::ensure_read_only_allows(config, WriteIntent::Mutation)?;
    let result = connector_for(config)?.execute(&QueryRequest {
        connection_id: config.id,
        database: path.database.clone(),
        schema: path.schema.clone(),
        session_id: None,
        text: text.to_string(),
        mode: QueryMode::All,
        options: QueryExecutionOptions::default(),
    })?;
    match result.summaries.iter().find(|summary| !summary.success) {
        Some(summary) => Err(Error::new(ErrorKind::Query, summary.message.clone())),
        None => Ok(()),
    }
}

/// 结构变更的执行顺序：先建表，再拆约束/索引/列，最后补回新增对象。
///
/// 删除必须先于新增：同名索引/外键的「修改」是先 DROP 再 ADD，顺序反了会撞名；
/// 删列前必须已有外键/索引被拆掉，否则目标库拒绝。
pub(crate) fn change_order(change: &StructureChange) -> u8 {
    match (change.object, change.action) {
        (StructureObject::Table, StructureChangeAction::Create) => 0,
        (StructureObject::ForeignKey, StructureChangeAction::Drop) => 1,
        (StructureObject::ForeignKey, StructureChangeAction::Alter) => 1,
        // CHECK 可能引用被删的列，必须先于删列解除。
        (StructureObject::Check, StructureChangeAction::Drop) => 1,
        (StructureObject::Index, StructureChangeAction::Drop) => 2,
        (StructureObject::Index, StructureChangeAction::Alter) => 2,
        (StructureObject::PrimaryKey, StructureChangeAction::Drop) => 3,
        (StructureObject::PrimaryKey, StructureChangeAction::Alter) => 3,
        (StructureObject::Column, StructureChangeAction::Drop) => 4,
        (StructureObject::Column, StructureChangeAction::Alter) => 5,
        (StructureObject::Column, StructureChangeAction::Create) => 6,
        (StructureObject::PrimaryKey, StructureChangeAction::Create) => 7,
        (StructureObject::Index, StructureChangeAction::Create) => 8,
        (StructureObject::ForeignKey, StructureChangeAction::Create) => 9,
        // CHECK 的「修改」是 DROP + ADD，放在列变更之后，表达式可引用新增列。
        (StructureObject::Check, StructureChangeAction::Create) => 9,
        (StructureObject::Check, StructureChangeAction::Alter) => 9,
        // 删表放最后：其它表的差异处理完再动它，避免连带引用先失效。
        (StructureObject::Table, StructureChangeAction::Drop) => 10,
        (StructureObject::Table, StructureChangeAction::Alter) => 10,
    }
}

/// 执行前整表校验方言能力，避免「改了一半才报不支持」。
///
/// 目前只有 MySQL 家族与 PostgreSQL 家族支持对象级 ALTER；SQLite 仅支持加列/删列；
/// 其余方言只能建表/删表，差异需用户手工处理。
pub(crate) fn ensure_changes_supported(
    kind: DatabaseKind,
    changes: &[StructureChange],
) -> fluxdb_core::Result<()> {
    for change in changes {
        let supported = match kind {
            DatabaseKind::MySql
            | DatabaseKind::TiDb
            | DatabaseKind::MariaDb
            | DatabaseKind::Postgres
            | DatabaseKind::Kingbase => true,
            DatabaseKind::Sqlite => matches!(
                (change.object, change.action),
                (StructureObject::Table, _)
                    | (StructureObject::Column, StructureChangeAction::Create)
                    | (StructureObject::Column, StructureChangeAction::Drop)
                    // SQLite 原生支持独立索引的建删；约束类变更需要整表重建。
                    | (StructureObject::Index, StructureChangeAction::Create)
                    | (StructureObject::Index, StructureChangeAction::Drop)
                    | (StructureObject::Index, StructureChangeAction::Alter)
            ),
            _ => matches!(change.object, StructureObject::Table),
        };
        if !supported {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("{} 暂不支持自动修改 {}", kind_label(kind), change.name),
            ));
        }
        // PostgreSQL 无法通过 ALTER 改生成列表达式，只能重建列（会丢数据），拒绝。
        if matches!(kind, DatabaseKind::Postgres | DatabaseKind::Kingbase)
            && let StructureChangeDetail::Column {
                before: Some(before),
                after: Some(after),
            } = &change.detail
            && generated_expression(before) != generated_expression(after)
        {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!(
                    "{} 不支持修改生成列 {} 的表达式，请手工重建该列",
                    kind_label(kind),
                    change.name
                ),
            ));
        }
    }
    Ok(())
}

fn generated_expression(column: &ColumnDef) -> Option<&str> {
    column
        .generated
        .as_ref()
        .map(|generated| generated.expression.trim())
}

fn kind_label(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "MySQL",
        DatabaseKind::TiDb => "TiDB",
        DatabaseKind::MariaDb => "MariaDB",
        DatabaseKind::Postgres => "PostgreSQL",
        DatabaseKind::Kingbase => "Kingbase",
        DatabaseKind::Sqlite => "SQLite",
        DatabaseKind::SqlServer => "SQL Server",
        DatabaseKind::Oracle => "Oracle",
        DatabaseKind::Dameng => "达梦",
        DatabaseKind::MongoDb => "MongoDB",
        DatabaseKind::Redis => "Redis",
    }
}

/// 把一条对象级变更翻译成若干条目标端语句。表级变更由编排层用源端 DDL 处理。
pub(crate) fn change_sql(
    kind: DatabaseKind,
    path: &ObjectPath,
    change: &StructureChange,
) -> fluxdb_core::Result<Vec<String>> {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            mysql_change_sql(path, change)
        }
        DatabaseKind::Postgres | DatabaseKind::Kingbase => postgres_change_sql(path, change),
        DatabaseKind::Sqlite => sqlite_change_sql(path, change),
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            format!("{} 暂不支持自动生成结构变更语句", kind_label(kind)),
        )),
    }
}

pub(crate) fn drop_table_sql(kind: DatabaseKind, path: &ObjectPath) -> String {
    let dialect = fluxdb_core::sql_dialect(kind);
    format!("DROP TABLE {}", dialect.qualified_object_name(path))
}

fn mysql_change_sql(
    path: &ObjectPath,
    change: &StructureChange,
) -> fluxdb_core::Result<Vec<String>> {
    let dialect = fluxdb_core::sql_dialect(DatabaseKind::MySql);
    let table = dialect.qualified_object_name(path);
    let join = |clause: &str| vec![format!("ALTER TABLE {table} {clause}")];
    match &change.detail {
        StructureChangeDetail::Table => Err(table_change_handled_elsewhere()),
        StructureChangeDetail::Column { before, after } => match (before, after) {
            (None, Some(after)) => Ok(join(&format!(
                "ADD COLUMN {}",
                mysql_column_definition(dialect, after)
            ))),
            (Some(_), Some(after)) => Ok(join(&format!(
                "MODIFY COLUMN {}",
                mysql_column_definition(dialect, after)
            ))),
            (Some(before), None) => Ok(join(&format!(
                "DROP COLUMN {}",
                dialect.quote_identifier(&before.name)
            ))),
            (None, None) => Ok(Vec::new()),
        },
        StructureChangeDetail::PrimaryKey { before, after } => {
            let mut clauses = Vec::new();
            if before.is_some() {
                clauses.push("DROP PRIMARY KEY".to_string());
            }
            if let Some(after) = after {
                clauses.push(format!(
                    "ADD PRIMARY KEY ({})",
                    quote_columns(dialect, &after.columns)
                ));
            }
            Ok(clauses
                .into_iter()
                .map(|clause| format!("ALTER TABLE {table} {clause}"))
                .collect())
        }
        StructureChangeDetail::Index { before, after } => {
            let mut statements = Vec::new();
            if let Some(before) = before {
                statements.push(format!(
                    "ALTER TABLE {table} DROP INDEX {}",
                    dialect.quote_identifier(&before.name)
                ));
            }
            if let Some(after) = after {
                statements.push(format!(
                    "ALTER TABLE {table} ADD {}",
                    mysql_index_definition(dialect, after)
                ));
            }
            Ok(statements)
        }
        StructureChangeDetail::ForeignKey { before, after } => {
            let mut statements = Vec::new();
            if let Some(before) = before {
                statements.push(format!(
                    "ALTER TABLE {table} DROP FOREIGN KEY {}",
                    dialect.quote_identifier(&before.name)
                ));
            }
            if let Some(after) = after {
                statements.push(format!(
                    "ALTER TABLE {table} ADD {}",
                    mysql_foreign_key_definition(dialect, after)
                ));
            }
            Ok(statements)
        }
        StructureChangeDetail::Check { before, after } => {
            let mut statements = Vec::new();
            if let Some(before) = before {
                statements.push(format!(
                    "ALTER TABLE {table} DROP CHECK {}",
                    dialect.quote_identifier(&before.name)
                ));
            }
            if let Some(after) = after {
                statements.push(format!(
                    "ALTER TABLE {table} ADD CONSTRAINT {} CHECK ({})",
                    dialect.quote_identifier(&after.name),
                    after.expression
                ));
            }
            Ok(statements)
        }
    }
}

fn postgres_change_sql(
    path: &ObjectPath,
    change: &StructureChange,
) -> fluxdb_core::Result<Vec<String>> {
    let dialect = fluxdb_core::sql_dialect(DatabaseKind::Postgres);
    let table = dialect.qualified_object_name(path);
    match &change.detail {
        StructureChangeDetail::Table => Err(table_change_handled_elsewhere()),
        StructureChangeDetail::Column { before, after } => {
            postgres_column_statements(dialect, &table, before.as_ref(), after.as_ref())
        }
        StructureChangeDetail::PrimaryKey { before, after } => {
            let mut statements = Vec::new();
            if let Some(before) = before {
                // 未命名主键由数据库生成 `<table>_pkey`；元数据里已有真实名字时优先用它。
                let name = if before.name.is_empty() {
                    format!("{}_pkey", path.name)
                } else {
                    before.name.clone()
                };
                statements.push(format!(
                    "ALTER TABLE {table} DROP CONSTRAINT {}",
                    dialect.quote_identifier(&name)
                ));
            }
            if let Some(after) = after {
                let name = if after.name.is_empty() {
                    format!("{}_pkey", path.name)
                } else {
                    after.name.clone()
                };
                statements.push(format!(
                    "ALTER TABLE {table} ADD CONSTRAINT {} PRIMARY KEY ({})",
                    dialect.quote_identifier(&name),
                    quote_columns(dialect, &after.columns)
                ));
            }
            Ok(statements)
        }
        StructureChangeDetail::Index { before, after } => {
            let mut statements = Vec::new();
            if let Some(before) = before {
                statements.push(format!(
                    "DROP INDEX {}",
                    postgres_index_name(dialect, path, &before.name)
                ));
            }
            if let Some(after) = after {
                statements.push(postgres_create_index(dialect, &table, after));
            }
            Ok(statements)
        }
        StructureChangeDetail::ForeignKey { before, after } => {
            let mut statements = Vec::new();
            if let Some(before) = before {
                statements.push(format!(
                    "ALTER TABLE {table} DROP CONSTRAINT {}",
                    dialect.quote_identifier(&before.name)
                ));
            }
            if let Some(after) = after {
                statements.push(format!(
                    "ALTER TABLE {table} ADD {}",
                    postgres_foreign_key_definition(dialect, after)
                ));
            }
            Ok(statements)
        }
        StructureChangeDetail::Check { before, after } => {
            let mut statements = Vec::new();
            if let Some(before) = before {
                statements.push(format!(
                    "ALTER TABLE {table} DROP CONSTRAINT {}",
                    dialect.quote_identifier(&before.name)
                ));
            }
            if let Some(after) = after {
                statements.push(format!(
                    "ALTER TABLE {table} ADD CONSTRAINT {} CHECK ({})",
                    dialect.quote_identifier(&after.name),
                    after.expression
                ));
            }
            Ok(statements)
        }
    }
}

fn postgres_column_statements(
    dialect: &dyn fluxdb_core::SqlDialect,
    table: &str,
    before: Option<&ColumnDef>,
    after: Option<&ColumnDef>,
) -> fluxdb_core::Result<Vec<String>> {
    match (before, after) {
        (None, Some(after)) => {
            let mut statements = vec![format!(
                "ALTER TABLE {table} ADD COLUMN {}",
                postgres_column_definition(dialect, after)
            )];
            if let Some(comment) = &after.comment {
                statements.push(postgres_column_comment(
                    dialect,
                    table,
                    &after.name,
                    Some(comment),
                ));
            }
            Ok(statements)
        }
        (Some(before), None) => Ok(vec![format!(
            "ALTER TABLE {table} DROP COLUMN {}",
            dialect.quote_identifier(&before.name)
        )]),
        (Some(before), Some(after)) => {
            let mut statements = Vec::new();
            let quoted = dialect.quote_identifier(&after.name);
            if normalize_type_text(&before.type_text) != normalize_type_text(&after.type_text) {
                // 类型窄化或跨类转换可能失败，交给目标库报错；不做静默截断。
                statements.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {quoted} TYPE {} USING {quoted}::{}",
                    after.type_text, after.type_text
                ));
            }
            if before.nullable != after.nullable {
                statements.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {quoted} {}",
                    if after.nullable {
                        "DROP NOT NULL"
                    } else {
                        "SET NOT NULL"
                    }
                ));
            }
            // 生成列不接受显式 DEFAULT，表达式变化已由方言能力校验提前拒绝。
            if after.generated.is_none() && before.default_expr != after.default_expr {
                statements.push(match &after.default_expr {
                    Some(default) => {
                        format!("ALTER TABLE {table} ALTER COLUMN {quoted} SET DEFAULT {default}")
                    }
                    None => format!("ALTER TABLE {table} ALTER COLUMN {quoted} DROP DEFAULT"),
                });
            }
            if normalize_comment(before.comment.as_deref())
                != normalize_comment(after.comment.as_deref())
            {
                statements.push(postgres_column_comment(
                    dialect,
                    table,
                    &after.name,
                    after.comment.as_deref(),
                ));
            }
            Ok(statements)
        }
        (None, None) => Ok(Vec::new()),
    }
}

fn sqlite_change_sql(
    path: &ObjectPath,
    change: &StructureChange,
) -> fluxdb_core::Result<Vec<String>> {
    let dialect = fluxdb_core::sql_dialect(DatabaseKind::Sqlite);
    let table = dialect.qualified_object_name(path);
    match &change.detail {
        StructureChangeDetail::Table => Err(table_change_handled_elsewhere()),
        StructureChangeDetail::Column {
            before: None,
            after: Some(after),
        } => Ok(vec![format!(
            "ALTER TABLE {table} ADD COLUMN {}",
            sqlite_column_definition(dialect, after)
        )]),
        StructureChangeDetail::Column {
            before: Some(before),
            after: None,
            ..
        } => Ok(vec![format!(
            "ALTER TABLE {table} DROP COLUMN {}",
            dialect.quote_identifier(&before.name)
        )]),
        StructureChangeDetail::Index { before, after } => {
            let mut statements = Vec::new();
            if let Some(before) = before {
                statements.push(format!(
                    "DROP INDEX {}",
                    sqlite_index_name(dialect, path, &before.name)
                ));
            }
            if let Some(after) = after {
                // CREATE INDEX 的表名不能带 schema 前缀，索引名也不带。
                statements.push(format!(
                    "CREATE {}INDEX {} ON {} ({})",
                    if after.unique { "UNIQUE " } else { "" },
                    dialect.quote_identifier(&after.name),
                    dialect.quote_identifier(&path.name),
                    quote_columns(dialect, &after.columns)
                ));
            }
            Ok(statements)
        }
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            "SQLite 暂不支持该结构变更",
        )),
    }
}

/// SQLite 索引与表同属一个附加库：删除时带库名前缀，避免同名索引歧义。
fn sqlite_index_name(
    dialect: &dyn fluxdb_core::SqlDialect,
    path: &ObjectPath,
    name: &str,
) -> String {
    match path
        .database
        .as_deref()
        .filter(|database| !database.is_empty())
    {
        Some(database) => format!(
            "{}.{}",
            dialect.quote_identifier(database),
            dialect.quote_identifier(name)
        ),
        None => dialect.quote_identifier(name),
    }
}

fn table_change_handled_elsewhere() -> Error {
    Error::new(ErrorKind::Internal, "表级变更应由编排层用源端 DDL 处理")
}

fn quote_columns(dialect: &dyn fluxdb_core::SqlDialect, columns: &[String]) -> String {
    columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ")
}

fn mysql_column_definition(dialect: &dyn fluxdb_core::SqlDialect, column: &ColumnDef) -> String {
    let mut sql = format!(
        "{} {}",
        dialect.quote_identifier(&column.name),
        column.type_text
    );
    // 生成列的语法位置在类型之后、可空性之前，且不接受 DEFAULT。
    if let Some(generated) = &column.generated {
        sql.push_str(&format!(
            " GENERATED ALWAYS AS ({}) {}",
            generated.expression,
            if generated.stored == Some(true) {
                "STORED"
            } else {
                "VIRTUAL"
            }
        ));
    }
    if !column.nullable {
        sql.push_str(" NOT NULL");
    }
    if column.generated.is_none()
        && let Some(default) = &column.default_expr
    {
        sql.push_str(&format!(" DEFAULT {}", mysql_default_literal(default)));
    }
    if let Some(comment) = &column.comment {
        sql.push_str(&format!(" COMMENT {}", mysql_string_literal(comment)));
    }
    sql
}

fn postgres_column_definition(dialect: &dyn fluxdb_core::SqlDialect, column: &ColumnDef) -> String {
    let mut sql = format!(
        "{} {}",
        dialect.quote_identifier(&column.name),
        column.type_text
    );
    // PostgreSQL 只支持 STORED 生成列。
    if let Some(generated) = &column.generated {
        sql.push_str(&format!(
            " GENERATED ALWAYS AS ({}) STORED",
            generated.expression
        ));
    }
    if !column.nullable {
        sql.push_str(" NOT NULL");
    }
    if column.generated.is_none()
        && let Some(default) = &column.default_expr
    {
        sql.push_str(&format!(" DEFAULT {default}"));
    }
    sql
}

fn sqlite_column_definition(dialect: &dyn fluxdb_core::SqlDialect, column: &ColumnDef) -> String {
    let mut sql = format!(
        "{} {}",
        dialect.quote_identifier(&column.name),
        column.type_text
    );
    // ALTER TABLE ADD COLUMN 只接受 VIRTUAL 生成列（SQLite 限制）。
    if let Some(generated) = &column.generated {
        sql.push_str(&format!(" GENERATED ALWAYS AS ({})", generated.expression));
    }
    if !column.nullable {
        sql.push_str(" NOT NULL");
    }
    if column.generated.is_none()
        && let Some(default) = &column.default_expr
    {
        sql.push_str(&format!(" DEFAULT {default}"));
    }
    sql
}

fn postgres_column_comment(
    dialect: &dyn fluxdb_core::SqlDialect,
    table: &str,
    column: &str,
    comment: Option<&str>,
) -> String {
    let value = match comment {
        Some(comment) => postgres_string_literal(comment),
        None => "NULL".to_string(),
    };
    format!(
        "COMMENT ON COLUMN {table}.{} IS {value}",
        dialect.quote_identifier(column)
    )
}

fn postgres_index_name(
    dialect: &dyn fluxdb_core::SqlDialect,
    path: &ObjectPath,
    name: &str,
) -> String {
    match path.schema.as_deref().filter(|schema| !schema.is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            dialect.quote_identifier(schema),
            dialect.quote_identifier(name)
        ),
        None => dialect.quote_identifier(name),
    }
}

fn mysql_index_definition(dialect: &dyn fluxdb_core::SqlDialect, index: &IndexDef) -> String {
    let index_type = index
        .index_type
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    let special = index_type.to_ascii_uppercase();
    // FULLTEXT / SPATIAL 是索引种类前缀，不能与 UNIQUE 组合；其余类型走 USING 后缀。
    let (kind_prefix, using) = match special.as_str() {
        "FULLTEXT" => ("FULLTEXT ", None),
        "SPATIAL" => ("SPATIAL ", None),
        "HASH" => ("", Some("HASH")),
        _ => ("", None),
    };
    let unique = if index.unique && kind_prefix.is_empty() {
        "UNIQUE "
    } else {
        ""
    };
    let mut sql = format!(
        "{unique}{kind_prefix}INDEX {} ({})",
        dialect.quote_identifier(&index.name),
        quote_columns(dialect, &index.columns)
    );
    if let Some(method) = using {
        sql.push_str(&format!(" USING {method}"));
    }
    sql
}

/// PostgreSQL 建索引：保留访问方法（gin/gist/brin…）与部分索引谓词。
fn postgres_create_index(
    dialect: &dyn fluxdb_core::SqlDialect,
    table: &str,
    index: &IndexDef,
) -> String {
    let mut sql = format!(
        "CREATE {}INDEX {} ON {table}",
        if index.unique { "UNIQUE " } else { "" },
        dialect.quote_identifier(&index.name)
    );
    if let Some(index_type) = index
        .index_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sql.push_str(&format!(" USING {index_type}"));
    }
    sql.push_str(&format!(" ({})", quote_columns(dialect, &index.columns)));
    if let Some(predicate) = index
        .predicate
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sql.push_str(&format!(" WHERE {predicate}"));
    }
    sql
}

fn mysql_foreign_key_definition(
    dialect: &dyn fluxdb_core::SqlDialect,
    foreign_key: &ForeignKeyDef,
) -> String {
    format!(
        "CONSTRAINT {} FOREIGN KEY ({}) {}",
        dialect.quote_identifier(&foreign_key.name),
        quote_columns(dialect, &foreign_key.columns),
        mysql_references_clause(dialect, foreign_key)
    )
}

fn postgres_foreign_key_definition(
    dialect: &dyn fluxdb_core::SqlDialect,
    foreign_key: &ForeignKeyDef,
) -> String {
    format!(
        "CONSTRAINT {} FOREIGN KEY ({}) {}",
        dialect.quote_identifier(&foreign_key.name),
        quote_columns(dialect, &foreign_key.columns),
        postgres_references_clause(dialect, foreign_key)
    )
}

fn mysql_references_clause(
    dialect: &dyn fluxdb_core::SqlDialect,
    foreign_key: &ForeignKeyDef,
) -> String {
    let table = match &foreign_key.ref_schema {
        Some(schema) => format!(
            "{}.{}",
            dialect.quote_identifier(schema),
            dialect.quote_identifier(&foreign_key.ref_table)
        ),
        None => dialect.quote_identifier(&foreign_key.ref_table),
    };
    let mut clause = format!(
        "REFERENCES {table} ({})",
        quote_columns(dialect, &foreign_key.ref_columns)
    );
    if let Some(action) = &foreign_key.on_delete {
        clause.push_str(&format!(" ON DELETE {action}"));
    }
    if let Some(action) = &foreign_key.on_update {
        clause.push_str(&format!(" ON UPDATE {action}"));
    }
    clause
}

fn postgres_references_clause(
    dialect: &dyn fluxdb_core::SqlDialect,
    foreign_key: &ForeignKeyDef,
) -> String {
    let table = match &foreign_key.ref_schema {
        Some(schema) => format!(
            "{}.{}",
            dialect.quote_identifier(schema),
            dialect.quote_identifier(&foreign_key.ref_table)
        ),
        None => dialect.quote_identifier(&foreign_key.ref_table),
    };
    let mut clause = format!(
        "REFERENCES {table} ({})",
        quote_columns(dialect, &foreign_key.ref_columns)
    );
    if let Some(action) = &foreign_key.on_delete {
        clause.push_str(&format!(" ON DELETE {action}"));
    }
    if let Some(action) = &foreign_key.on_update {
        clause.push_str(&format!(" ON UPDATE {action}"));
    }
    clause
}

/// MySQL 的默认值区分「字面量」与「表达式」：`column_default` 返回的是裸文本
/// （字符串没有引号），直接拼会得到 `DEFAULT abc` 语法错误。
///
/// 判断顺序：NULL → 数字 → 已带引号/位串 → 含 `(` 的表达式 → 关键字/函数名；
/// 其余一律当字符串字面量转义。`ponytail:` 启发式规则，遇到新函数再加。
fn mysql_default_literal(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return "NULL".to_string();
    }
    if trimmed.parse::<f64>().is_ok()
        || trimmed.starts_with('\'')
        || trimmed.starts_with('"')
        || trimmed.starts_with('(')
        || trimmed.to_ascii_lowercase().starts_with("b'")
        || trimmed.contains('(')
    {
        return trimmed.to_string();
    }
    let keyword = trimmed.to_ascii_lowercase();
    if matches!(
        keyword.as_str(),
        "current_timestamp" | "current_date" | "current_time" | "localtimestamp" | "now" | "uuid"
    ) {
        return trimmed.to_string();
    }
    mysql_string_literal(trimmed)
}

fn mysql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn postgres_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn normalize_type_text(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_comment(comment: Option<&str>) -> Option<&str> {
    comment.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::model::CheckDef;
    use fluxdb_core::ConnectionId;

    fn path(schema: Option<&str>) -> ObjectPath {
        ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("app".into()),
            schema: schema.map(str::to_string),
            name: "orders".into(),
            kind: fluxdb_core::ObjectKind::Table,
        }
    }

    fn column(name: &str, type_text: &str, nullable: bool) -> ColumnDef {
        ColumnDef {
            name: name.into(),
            type_text: type_text.into(),
            nullable,
            primary_key: false,
            auto_increment: false,
            default_expr: None,
            generated: None,
            comment: None,
        }
    }

    #[test]
    fn mysql_add_column_quotes_identifiers_and_defaults() {
        let mut added = column("status", "varchar(20)", false);
        added.default_expr = Some("new".into());
        let change = StructureChange {
            object: StructureObject::Column,
            action: StructureChangeAction::Create,
            name: "status".into(),
            detail: StructureChangeDetail::Column {
                before: None,
                after: Some(added),
            },
        };
        let statements = change_sql(DatabaseKind::MySql, &path(None), &change).unwrap();
        assert_eq!(
            statements,
            vec![
                "ALTER TABLE `app`.`orders` ADD COLUMN `status` varchar(20) NOT NULL DEFAULT 'new'"
                    .to_string()
            ]
        );
    }

    #[test]
    fn mysql_expression_default_is_not_quoted() {
        assert_eq!(
            mysql_default_literal("current_timestamp()"),
            "current_timestamp()"
        );
        assert_eq!(
            mysql_default_literal("CURRENT_TIMESTAMP"),
            "CURRENT_TIMESTAMP"
        );
        assert_eq!(mysql_default_literal("42"), "42");
        assert_eq!(mysql_default_literal("it's"), "'it\\'s'");
    }

    #[test]
    fn postgres_alter_column_emits_separate_clauses() {
        let before = column("note", "varchar(10)", true);
        let mut after = column("note", "varchar(20)", false);
        after.default_expr = Some("''::character varying".into());
        let change = StructureChange {
            object: StructureObject::Column,
            action: StructureChangeAction::Alter,
            name: "note".into(),
            detail: StructureChangeDetail::Column {
                before: Some(before),
                after: Some(after),
            },
        };
        let statements =
            change_sql(DatabaseKind::Postgres, &path(Some("public")), &change).unwrap();
        assert_eq!(
            statements,
            vec![
                "ALTER TABLE \"public\".\"orders\" ALTER COLUMN \"note\" TYPE varchar(20) USING \"note\"::varchar(20)",
                "ALTER TABLE \"public\".\"orders\" ALTER COLUMN \"note\" SET NOT NULL",
                "ALTER TABLE \"public\".\"orders\" ALTER COLUMN \"note\" SET DEFAULT ''::character varying",
            ]
        );
    }

    #[test]
    fn changed_index_is_dropped_before_created() {
        let before = IndexDef {
            name: "idx_a".into(),
            columns: vec!["a".into()],
            unique: false,
            primary: false,
            index_type: None,
            predicate: None,
            has_expression: false,
        };
        let after = IndexDef {
            unique: true,
            ..before.clone()
        };
        let change = StructureChange {
            object: StructureObject::Index,
            action: StructureChangeAction::Alter,
            name: "idx_a".into(),
            detail: StructureChangeDetail::Index {
                before: Some(before),
                after: Some(after),
            },
        };
        let statements =
            change_sql(DatabaseKind::Postgres, &path(Some("public")), &change).unwrap();
        assert_eq!(statements[0], "DROP INDEX \"public\".\"idx_a\"");
        assert!(statements[1].starts_with("CREATE UNIQUE INDEX \"idx_a\" ON"));
    }

    #[test]
    fn mysql_check_alter_drops_then_adds_constraint() {
        let change = StructureChange {
            object: StructureObject::Check,
            action: StructureChangeAction::Alter,
            name: "chk_amount".into(),
            detail: StructureChangeDetail::Check {
                before: Some(CheckDef {
                    name: "chk_amount".into(),
                    expression: "amount > 0".into(),
                }),
                after: Some(CheckDef {
                    name: "chk_amount".into(),
                    expression: "amount >= 0".into(),
                }),
            },
        };
        assert_eq!(
            change_sql(DatabaseKind::MySql, &path(None), &change).unwrap(),
            vec![
                "ALTER TABLE `app`.`orders` DROP CHECK `chk_amount`",
                "ALTER TABLE `app`.`orders` ADD CONSTRAINT `chk_amount` CHECK (amount >= 0)",
            ]
        );
        // SQLite 不支持对象级约束变更，必须在执行前整体拒绝。
        assert!(
            ensure_changes_supported(DatabaseKind::Sqlite, std::slice::from_ref(&change)).is_err()
        );
    }

    #[test]
    fn sqlite_index_create_and_drop_are_supported() {
        let index = IndexDef {
            name: "idx_a".into(),
            columns: vec!["a".into()],
            unique: true,
            primary: false,
            index_type: None,
            predicate: None,
            has_expression: false,
        };
        let create = StructureChange {
            object: StructureObject::Index,
            action: StructureChangeAction::Create,
            name: "idx_a".into(),
            detail: StructureChangeDetail::Index {
                before: None,
                after: Some(index.clone()),
            },
        };
        assert!(
            ensure_changes_supported(DatabaseKind::Sqlite, std::slice::from_ref(&create)).is_ok()
        );
        assert_eq!(
            change_sql(DatabaseKind::Sqlite, &path(None), &create).unwrap(),
            vec!["CREATE UNIQUE INDEX \"idx_a\" ON \"orders\" (\"a\")"]
        );
        let drop = StructureChange {
            object: StructureObject::Index,
            action: StructureChangeAction::Drop,
            name: "idx_a".into(),
            detail: StructureChangeDetail::Index {
                before: Some(index),
                after: None,
            },
        };
        assert_eq!(
            change_sql(DatabaseKind::Sqlite, &path(None), &drop).unwrap(),
            vec!["DROP INDEX \"app\".\"idx_a\""]
        );
    }

    #[test]
    fn mysql_fulltext_index_uses_kind_prefix() {
        let change = StructureChange {
            object: StructureObject::Index,
            action: StructureChangeAction::Create,
            name: "ft_body".into(),
            detail: StructureChangeDetail::Index {
                before: None,
                after: Some(IndexDef {
                    name: "ft_body".into(),
                    columns: vec!["body".into()],
                    unique: false,
                    primary: false,
                    index_type: Some("FULLTEXT".into()),
                    predicate: None,
                    has_expression: false,
                }),
            },
        };
        assert_eq!(
            change_sql(DatabaseKind::MySql, &path(None), &change).unwrap(),
            vec!["ALTER TABLE `app`.`orders` ADD FULLTEXT INDEX `ft_body` (`body`)"]
        );
    }

    #[test]
    fn mysql_generated_column_emits_virtual_or_stored() {
        let mut generated = column("total", "int", true);
        generated.generated = Some(fluxdb_core::GeneratedColumn {
            expression: "a + b".into(),
            stored: Some(true),
        });
        let change = StructureChange {
            object: StructureObject::Column,
            action: StructureChangeAction::Create,
            name: "total".into(),
            detail: StructureChangeDetail::Column {
                before: None,
                after: Some(generated),
            },
        };
        assert_eq!(
            change_sql(DatabaseKind::MySql, &path(None), &change).unwrap(),
            vec![
                "ALTER TABLE `app`.`orders` ADD COLUMN `total` int GENERATED ALWAYS AS (a + b) STORED"
            ]
        );
    }

    #[test]
    fn foreign_key_create_keeps_referential_actions() {
        let change = StructureChange {
            object: StructureObject::ForeignKey,
            action: StructureChangeAction::Create,
            name: "fk_order".into(),
            detail: StructureChangeDetail::ForeignKey {
                before: None,
                after: Some(ForeignKeyDef {
                    name: "fk_order".into(),
                    columns: vec!["customer_id".into()],
                    ref_schema: None,
                    ref_table: "customers".into(),
                    ref_columns: vec!["id".into()],
                    on_delete: Some("CASCADE".into()),
                    on_update: Some("RESTRICT".into()),
                }),
            },
        };
        let statements = change_sql(DatabaseKind::MySql, &path(None), &change).unwrap();
        assert!(
            statements[0].contains("ON DELETE CASCADE ON UPDATE RESTRICT"),
            "{}",
            statements[0]
        );
    }

    #[test]
    fn postgres_rejects_generated_expression_alter() {
        let before = column("total", "int", true);
        let mut after = before.clone();
        after.generated = Some(fluxdb_core::GeneratedColumn {
            expression: "a - b".into(),
            stored: Some(true),
        });
        let change = StructureChange {
            object: StructureObject::Column,
            action: StructureChangeAction::Alter,
            name: "total".into(),
            detail: StructureChangeDetail::Column {
                before: Some(before),
                after: Some(after),
            },
        };
        assert!(
            ensure_changes_supported(DatabaseKind::Postgres, std::slice::from_ref(&change))
                .is_err()
        );
    }

    #[test]
    fn unsupported_dialect_fails_before_any_statement() {
        let column = column("note", "text", true);
        let change = StructureChange {
            object: StructureObject::Column,
            action: StructureChangeAction::Create,
            name: "note".into(),
            detail: StructureChangeDetail::Column {
                before: None,
                after: Some(column),
            },
        };
        assert!(
            ensure_changes_supported(DatabaseKind::Oracle, std::slice::from_ref(&change)).is_err()
        );
        assert!(ensure_changes_supported(DatabaseKind::Sqlite, &[change]).is_ok());
    }

    #[test]
    fn order_drops_constraints_before_columns() {
        let drop_fk = StructureChange {
            object: StructureObject::ForeignKey,
            action: StructureChangeAction::Drop,
            name: "fk".into(),
            detail: StructureChangeDetail::ForeignKey {
                before: None,
                after: None,
            },
        };
        let drop_column = StructureChange {
            object: StructureObject::Column,
            action: StructureChangeAction::Drop,
            name: "col".into(),
            detail: StructureChangeDetail::Column {
                before: None,
                after: None,
            },
        };
        assert!(change_order(&drop_fk) < change_order(&drop_column));
    }

    /// 连接器返回的建表语句常是裸表名；显式配对改名时仍必须改到目标表。
    #[test]
    fn rewrite_renames_bare_table_name_in_source_ddl() {
        let source = path(None);
        let mut target = path(None);
        target.name = "orders_old".into();
        let ddl = "CREATE TABLE `orders` (`id` int NOT NULL, PRIMARY KEY (`id`))";

        let rewritten = rewrite_table_ddl(DatabaseKind::MySql, &source, &target, ddl);
        assert_eq!(
            rewritten,
            "CREATE TABLE `orders_old` (`id` int NOT NULL, PRIMARY KEY (`id`))"
        );

        // 限定名出现在 DDL 时按完整限定名替换。
        let ddl = "CREATE TABLE `app`.`orders` (`id` int NOT NULL)";
        assert_eq!(
            rewrite_table_ddl(DatabaseKind::MySql, &source, &target, ddl),
            "CREATE TABLE `app`.`orders_old` (`id` int NOT NULL)"
        );
    }
}
