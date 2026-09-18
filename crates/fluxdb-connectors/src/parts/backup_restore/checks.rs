// 还原预检查（设计文档 §15.2）：针对「清空后导入 / 追加数据」这类保留目标结构的动作，
// 查询目标库元数据评估风险；查询失败一律降级为“未知”，不阻断预检查（避免元数据库差异误拦）。

use std::collections::{BTreeMap, BTreeSet};

/// 单表的目标侧元数据检查结果。
pub(crate) struct TableChecks {
    /// 引用本表的外键来源表（其他表 FK 指向本表）；清空/删除会波及它们。
    pub(crate) incoming_fk: Vec<String>,
    /// 本表指向其他表的外键目标表。
    pub(crate) outgoing_fk: Vec<String>,
    /// 本表是否有主键或唯一约束（追加/清空后重导可能触发唯一冲突）。
    pub(crate) has_pk_unique: bool,
    /// 目标列集合（小写）；None 表示未能获取（视图、查询失败等）。
    pub(crate) target_columns: Option<BTreeSet<String>>,
}

impl Default for TableChecks {
    fn default() -> Self {
        Self {
            incoming_fk: Vec::new(),
            outgoing_fk: Vec::new(),
            has_pk_unique: false,
            target_columns: None,
        }
    }
}

/// 转义 SQL 字符串字面量里的单引号，防止拼接注入。
fn lit(value: &str) -> String {
    value.replace('\'', "''")
}

/// 在目标库执行只读查询，返回每行首列的文本值。任何错误都吞掉并返回 None（降级为“未知”）。
fn query_first_column(
    connector: &dyn Connector,
    request: &RestoreRequest,
    sql: &str,
) -> Option<Vec<String>> {
    let result = connector
        .execute(&fluxdb_core::QueryRequest {
            connection_id: request.config.id,
            database: Some(request.target.clone()),
            schema: None,
            text: sql.to_string(),
            mode: fluxdb_core::QueryMode::All,
            options: fluxdb_core::QueryExecutionOptions {
                continue_on_error: false,
                ..Default::default()
            },
            session_id: None,
        })
        .ok()?;
    let panel = result.results.first()?;
    Some(
        panel
            .rows
            .iter()
            .filter_map(|row| match row.values.first() {
                Some(CellValue::Text(v)) => Some(v.clone()),
                Some(CellValue::I64(v)) => Some(v.to_string()),
                Some(CellValue::Null) | None => None,
                Some(other) => Some(format!("{other:?}")),
            })
            .collect(),
    )
}

/// 统计查询（返回首行首列整数）。
fn query_count(
    connector: &dyn Connector,
    request: &RestoreRequest,
    sql: &str,
) -> Option<i64> {
    let result = connector
        .execute(&fluxdb_core::QueryRequest {
            connection_id: request.config.id,
            database: Some(request.target.clone()),
            schema: None,
            text: sql.to_string(),
            mode: fluxdb_core::QueryMode::All,
            options: fluxdb_core::QueryExecutionOptions {
                continue_on_error: false,
                ..Default::default()
            },
            session_id: None,
        })
        .ok()?;
    match result
        .results
        .first()?
        .rows
        .first()?
        .values
        .first()?
    {
        CellValue::I64(v) => Some(*v),
        CellValue::Text(v) => v.parse().ok(),
        _ => None,
    }
}

/// 为需要保留目标结构的表（清空后导入 / 追加数据）收集元数据检查。
/// 仅对 `keys` 中列出的表查询，减少往返；未列出的表不产生条目。
pub(crate) fn collect_table_checks(
    kind: DatabaseKind,
    request: &RestoreRequest,
    connector: &dyn Connector,
    parts: &[TablePart],
    keys: &BTreeSet<String>,
) -> BTreeMap<String, TableChecks> {
    let mut out = BTreeMap::new();
    if keys.is_empty() {
        return out;
    }
    for part in parts {
        if !keys.contains(&part.key) {
            continue;
        }
        let schema = part.schema.clone().unwrap_or_else(|| "public".to_string());
        let checks = if kind == DatabaseKind::Postgres {
            pg_checks(connector, request, &schema, &part.name)
        } else {
            mysql_checks(connector, request, &part.name)
        };
        out.insert(part.key.clone(), checks);
    }
    out
}

fn mysql_checks(
    connector: &dyn Connector,
    request: &RestoreRequest,
    table: &str,
) -> TableChecks {
    let t = lit(table);
    let mut checks = TableChecks::default();
    checks.incoming_fk = query_first_column(
        connector,
        request,
        &format!(
            "SELECT DISTINCT TABLE_NAME FROM information_schema.KEY_COLUMN_USAGE \
             WHERE TABLE_SCHEMA=DATABASE() AND REFERENCED_TABLE_NAME='{t}'"
        ),
    )
    .unwrap_or_default();
    checks.outgoing_fk = query_first_column(
        connector,
        request,
        &format!(
            "SELECT DISTINCT REFERENCED_TABLE_NAME FROM information_schema.KEY_COLUMN_USAGE \
             WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='{t}' AND REFERENCED_TABLE_NAME IS NOT NULL"
        ),
    )
    .unwrap_or_default();
    checks.has_pk_unique = query_count(
        connector,
        request,
        &format!(
            "SELECT COUNT(*) FROM information_schema.TABLE_CONSTRAINTS \
             WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='{t}' AND CONSTRAINT_TYPE IN ('PRIMARY KEY','UNIQUE')"
        ),
    )
    .unwrap_or(0)
        > 0;
    checks.target_columns = query_first_column(
        connector,
        request,
        &format!(
            "SELECT COLUMN_NAME FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='{t}'"
        ),
    )
    .map(|cols| cols.into_iter().map(|c| c.to_lowercase()).collect());
    checks
}

fn pg_checks(
    connector: &dyn Connector,
    request: &RestoreRequest,
    schema: &str,
    table: &str,
) -> TableChecks {
    let s = lit(schema);
    let t = lit(table);
    let mut checks = TableChecks::default();
    checks.incoming_fk = query_first_column(
        connector,
        request,
        &format!(
            "SELECT DISTINCT cl.relname FROM pg_constraint c \
             JOIN pg_class cl ON cl.oid=c.conrelid \
             JOIN pg_class tf ON tf.oid=c.confrelid \
             JOIN pg_namespace nf ON nf.oid=tf.relnamespace \
             WHERE c.contype='f' AND nf.nspname='{s}' AND tf.relname='{t}'"
        ),
    )
    .unwrap_or_default();
    checks.outgoing_fk = query_first_column(
        connector,
        request,
        &format!(
            "SELECT DISTINCT tf.relname FROM pg_constraint c \
             JOIN pg_class cl ON cl.oid=c.conrelid \
             JOIN pg_namespace n ON n.oid=cl.relnamespace \
             JOIN pg_class tf ON tf.oid=c.confrelid \
             WHERE c.contype='f' AND n.nspname='{s}' AND cl.relname='{t}'"
        ),
    )
    .unwrap_or_default();
    checks.has_pk_unique = query_count(
        connector,
        request,
        &format!(
            "SELECT count(*) FROM pg_constraint c \
             JOIN pg_class cl ON cl.oid=c.conrelid \
             JOIN pg_namespace n ON n.oid=cl.relnamespace \
             WHERE c.contype IN ('p','u') AND n.nspname='{s}' AND cl.relname='{t}'"
        ),
    )
    .unwrap_or(0)
        > 0;
    checks.target_columns = query_first_column(
        connector,
        request,
        &format!(
            "SELECT a.attname FROM pg_attribute a \
             JOIN pg_class cl ON cl.oid=a.attrelid \
             JOIN pg_namespace n ON n.oid=cl.relnamespace \
             WHERE n.nspname='{s}' AND cl.relname='{t}' AND a.attnum>0 AND NOT a.attisdropped"
        ),
    )
    .map(|cols| cols.into_iter().map(|c| c.to_lowercase()).collect());
    checks
}

/// 从备份脚本里解析某表 `CREATE TABLE` 的列名集合（小写）。无 DDL 或解析失败返回 None。
pub(crate) fn source_columns(kind: DatabaseKind, stmts: &[PStmt], key: &str) -> Option<BTreeSet<String>> {
    use sqlparser::{ast::Statement, dialect::{MySqlDialect, PostgreSqlDialect}, parser::Parser};
    let create = stmts.iter().find(|s| {
        s.kind == StmtKind::TableDdl
            && s.table_key.as_deref() == Some(key)
            && s.raw.trim_start().to_ascii_uppercase().starts_with("CREATE TABLE")
    })?;
    let dialect: &dyn sqlparser::dialect::Dialect = if kind == DatabaseKind::Postgres {
        &PostgreSqlDialect {}
    } else {
        &MySqlDialect {}
    };
    let parsed = Parser::parse_sql(dialect, &create.raw).ok()?;
    let mut cols = BTreeSet::new();
    for stmt in parsed {
        if let Statement::CreateTable(table) = stmt {
            for c in &table.columns {
                cols.insert(c.name.value.to_lowercase());
            }
        }
    }
    if cols.is_empty() {
        None
    } else {
        Some(cols)
    }
}

/// 依据动作与元数据，产出该表的风险提示（非阻断）。阻断级校验在 resolve_table_decisions 里做。
pub(crate) fn risks_for(
    action: RestoreTableAction,
    checks: Option<&TableChecks>,
    source_cols: Option<&BTreeSet<String>>,
) -> Vec<String> {
    let mut risks = Vec::new();
    match action {
        RestoreTableAction::Append | RestoreTableAction::TruncateAndLoad => {}
        _ => return risks,
    }
    let Some(checks) = checks else {
        risks.push("无法读取目标元数据，未能校验外键与列兼容性".into());
        return risks;
    };
    if checks.has_pk_unique {
        risks.push("目标含主键/唯一约束，导入重复键可能失败".into());
    }
    if action == RestoreTableAction::TruncateAndLoad && !checks.outgoing_fk.is_empty() {
        risks.push(format!(
            "本表外键引用 {}，清空后重导期间引用完整性依赖导入顺序",
            checks.outgoing_fk.join("、")
        ));
    }
    if action == RestoreTableAction::Append && !checks.incoming_fk.is_empty() {
        risks.push(format!(
            "有其它表（{}）外键引用本表，追加数据需满足其约束",
            checks.incoming_fk.join("、")
        ));
    }
    // 列兼容：源列必须是目标列子集，否则 INSERT 失败。
    match (source_cols, &checks.target_columns) {
        (Some(src), Some(dst)) => {
            let missing: Vec<&String> = src.iter().filter(|c| !dst.contains(*c)).collect();
            if !missing.is_empty() {
                risks.push(format!(
                    "源表列 [{}] 在目标表中不存在，导入将失败",
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                ));
            }
        }
        (Some(_), None) => risks.push("无法读取目标列，未能校验列兼容性".into()),
        (None, _) => risks.push("备份不含结构，无法离线校验列兼容性".into()),
    }
    risks
}

/// 阻断级校验：清空后导入时，若有其它表外键引用本表，清空会破坏其完整性 → 拒绝。
pub(crate) fn truncate_block_reason(checks: Option<&TableChecks>) -> Option<String> {
    let checks = checks?;
    if checks.incoming_fk.is_empty() {
        None
    } else {
        Some(format!(
            "有其它表（{}）外键引用本表，清空会破坏其引用完整性；请改用重建或先处理引用表",
            checks.incoming_fk.join("、")
        ))
    }
}
