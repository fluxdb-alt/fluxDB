//! AI SQL 安全策略。
//!
//! 每个数据库一个 `SqlPolicy` 实现，集中描述该方言的：
//! - sqlparser 方言；
//! - 有副作用函数与高泄露系统对象 deny 列表；
//! - 只读元数据语句子集；
//! - 对象限定名的命名空间语义；
//! - 执行计划的生成方式。
//!
//! 只读判定始终基于 AST，不靠 SQL 前缀猜安全。客户端策略是纵深的一层：
//! 执行侧还必须叠加服务端只读/回滚与数据库账号权限。

use crate::ToolSpec;
use fluxdb_core::DatabaseKind;
use sqlparser::ast::{
    Expr, ObjectName, ObjectNamePart, Query, SetExpr, Statement, TableFactor, Visit, Visitor,
};
use std::ops::ControlFlow;

mod dameng;
mod mysql;
mod oracle;
mod postgres;
mod sqlite;
mod sqlserver;

pub use dameng::DAMENG;
pub use mysql::{MARIADB, MYSQL};
pub use oracle::ORACLE;
pub use postgres::POSTGRES;
pub use sqlite::SQLITE;
pub use sqlserver::SQLSERVER;

/// 对象限定名的命名空间语义。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamespaceScope {
    /// 第一层限定是 database（MySQL/TiDB）。
    Database,
    /// 第一层限定是 schema（PostgreSQL/达梦）。
    Schema,
    /// 限定名是附加数据库；请求未指定时按 `main`（SQLite）。
    AttachedDatabase,
    /// 两段名是 `schema.table`，三段名是 `database.schema.table`（SQL Server）。
    SchemaAndDatabase,
}

impl NamespaceScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Schema => "schema",
            Self::AttachedDatabase => "attached database",
            Self::SchemaAndDatabase => "schema",
        }
    }
}

/// 查询计划的生成方式。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplainMode {
    /// 在 SQL 前加 `EXPLAIN`（PostgreSQL/MySQL/SQLite/达梦）。
    Prefix,
    /// SQL Server 的 `SET SHOWPLAN_ALL ON` 专用会话。
    Showplan,
    /// 方言没有真正无副作用的计划入口（Oracle 的 `EXPLAIN PLAN FOR` 会写计划表）。
    Unsupported,
}

/// 数据库方言的安全策略。
///
/// 实现必须是零状态、可全局共享的；策略只描述静态规则，动态授权（连接、
/// 对象白名单、审批）不属于这里。
pub trait SqlPolicy: Send + Sync {
    fn kind(&self) -> DatabaseKind;

    /// 对模型与页面上下文展示的稳定标识。
    fn label(&self) -> &'static str;

    fn parser_dialect(&self) -> &'static dyn sqlparser::dialect::Dialect;

    /// 函数 denylist。命中即降级为需授权。
    fn function_denylist(&self) -> &'static [&'static str];

    /// 系统表/视图 denylist。命中即降级为需授权。
    fn relation_denylist(&self) -> &'static [&'static str];

    /// 额外的事务/会话控制前缀（通用前缀始终拒绝）。
    fn extra_transaction_markers(&self) -> &'static [&'static str] {
        &[]
    }

    fn namespace_scope(&self) -> NamespaceScope;

    /// MySQL `SHOW <对象>` 只读子集的头部关键字。
    fn read_only_show_heads(&self) -> &'static [&'static str] {
        &[]
    }

    /// PostgreSQL/达梦 `SHOW <名称>` 只读子集的完整名称。
    fn read_only_show_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// SQLite 只读 PRAGMA 子集。
    fn read_only_pragmas(&self) -> &'static [&'static str] {
        &[]
    }

    /// 已知无副作用的内置函数表。为 `Some` 时，不在表内的函数调用一律视为
    /// 无法证明只读（Oracle 用户函数可有自治事务）。
    fn known_functions(&self) -> Option<&'static [&'static str]> {
        None
    }

    /// 表达式的已知副作用（如 `seq.NEXTVAL`）。
    fn expression_side_effect(&self, _: &Expr) -> Option<&'static str> {
        None
    }

    /// 对象限定名的范围问题（如 `table@dblink`）。
    fn relation_scope_reason(&self, _: &str) -> Option<&'static str> {
        None
    }

    fn explain_mode(&self) -> ExplainMode {
        ExplainMode::Prefix
    }
}

/// 按连接类型取策略；没有 SQL 策略的类型返回 `None`。
pub fn policy_for(kind: DatabaseKind) -> Option<&'static dyn SqlPolicy> {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => Some(&MYSQL),
        DatabaseKind::MariaDb => Some(&MARIADB),
        DatabaseKind::Postgres => Some(&POSTGRES),
        DatabaseKind::Sqlite => Some(&SQLITE),
        DatabaseKind::SqlServer => Some(&SQLSERVER),
        DatabaseKind::Dameng => Some(&DAMENG),
        DatabaseKind::Oracle => Some(&ORACLE),
        DatabaseKind::MongoDb | DatabaseKind::Redis => None,
    }
}

/// 风险分类结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SqlRisk {
    /// 只读、单语句、无已知副作用 → 可以在只读事务中自动执行。
    ReadOnly,
    /// 需要授权/审批：写操作，或无法保证只读（未知语法、锁定读、危险函数/系统表）。
    RequiresAuthorization { reasons: Vec<String> },
    /// 直接拒绝：多语句等结构性禁止。
    Rejected { reasons: Vec<String> },
}

impl SqlRisk {
    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::ReadOnly)
    }

    pub fn reasons(&self) -> &[String] {
        match self {
            Self::ReadOnly => &[],
            Self::RequiresAuthorization { reasons } | Self::Rejected { reasons } => reasons,
        }
    }
}

/// 所有方言都拒绝的事务/会话控制前缀。
const COMMON_TRANSACTION_MARKERS: &[&str] = &[
    "begin",
    "start transaction",
    "commit",
    "rollback",
    "savepoint",
    "set session",
    "set local",
    "set ",
];

fn statement_text(statement: &Statement) -> String {
    statement.to_string().to_lowercase()
}

/// 事务/会话控制的 AST 判定。
///
/// PostgreSQL 方言会把 `ALTER SESSION` 解析成 AST 节点，`to_string()` 不一定保留
/// 原前缀；只靠文本前缀会漏掉达梦/PG 的会话切换。因此先看稳定节点，再看文本。
fn is_transaction_control(policy: &dyn SqlPolicy, statement: &Statement) -> bool {
    // `SET ...` 无论具体设置项，都是会话/交易状态修改；AI 单语句里不放开。
    if matches!(statement, Statement::Set(_)) {
        return true;
    }
    if matches!(
        statement,
        Statement::StartTransaction { .. }
            | Statement::Commit { .. }
            | Statement::Rollback { .. }
            | Statement::Savepoint { .. }
            | Statement::ReleaseSavepoint { .. }
            | Statement::AlterSession { .. }
    ) {
        return true;
    }
    let text = statement_text(statement);
    let text = text.trim_start();
    COMMON_TRANSACTION_MARKERS
        .iter()
        .chain(policy.extra_transaction_markers().iter())
        .any(|marker| text.starts_with(marker.trim_end()))
}

fn normalize_name(name: &ObjectName) -> (String, String) {
    let parts: Vec<String> = name
        .0
        .iter()
        .filter_map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.value.to_ascii_lowercase()),
            ObjectNamePart::Function(_) => None,
        })
        .collect();
    let full = parts.join(".");
    let last = parts.last().cloned().unwrap_or_default();
    (full, last)
}

fn function_name_denied(policy: &dyn SqlPolicy, name: &ObjectName) -> bool {
    let (full, last) = normalize_name(name);
    policy.function_denylist().iter().any(|candidate| {
        let candidate = candidate.to_ascii_lowercase();
        candidate == full || (!candidate.contains('.') && candidate == last)
    })
}

fn relation_denied(policy: &dyn SqlPolicy, name: &ObjectName) -> bool {
    let parts: Vec<String> = name
        .0
        .iter()
        .filter_map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.value.to_ascii_lowercase()),
            ObjectNamePart::Function(_) => None,
        })
        .collect();
    let full = parts.join(".");
    let suffix2 = parts
        .iter()
        .rev()
        .take(2)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join(".");
    let last = parts.last().cloned().unwrap_or_default();
    policy.relation_denylist().iter().any(|entry| {
        let entry = entry.to_ascii_lowercase();
        entry == full || entry == suffix2 || (!entry.contains('.') && entry == last)
    })
}

/// 分析一条 SQL。
pub fn analyze_sql(sql: &str, policy: &dyn SqlPolicy) -> SqlRisk {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return SqlRisk::Rejected {
            reasons: vec!["empty".into()],
        };
    }
    let statements = match sqlparser::parser::Parser::parse_sql(policy.parser_dialect(), trimmed) {
        Ok(statements) => statements,
        Err(_) => {
            return SqlRisk::RequiresAuthorization {
                reasons: vec!["unknown_syntax".into()],
            };
        }
    };
    if statements.len() != 1 {
        return SqlRisk::Rejected {
            reasons: vec![format!("multi_statement:{}", statements.len())],
        };
    }
    let statement = &statements[0];
    let mut reasons = Vec::new();

    if is_transaction_control(policy, statement) {
        return SqlRisk::Rejected {
            reasons: vec!["transaction_or_session_control".into()],
        };
    }

    match statement {
        Statement::Explain {
            analyze,
            options,
            statement,
            ..
        } => {
            if *analyze || explain_options_request_analyze(options.as_deref()) {
                reasons.push("explain_analyze".into());
            }
            match statement.as_ref() {
                Statement::Query(query) => classify_query(query, &mut reasons),
                other => reasons.push(format!(
                    "explain_non_read_statement:{}",
                    variant_name(other)
                )),
            }
        }
        Statement::Query(query) => classify_query(query, &mut reasons),
        other if classify_metadata_read(other, policy, &mut reasons) => {}
        other => reasons.push(format!("non_read_statement:{}", variant_name(other))),
    }

    if let Some(name) = first_denied_function(statement, policy) {
        reasons.push(format!("denied_function:{name}"));
    }
    if let Some(reason) = first_locking_query(statement) {
        reasons.push(reason.into());
    }
    if let Some(table) = first_denied_relation(statement, policy) {
        reasons.push(format!("denied_table:{table}"));
    }
    if let Some(reason) = first_expression_side_effect(statement, policy) {
        reasons.push(format!("denied_syntax:{reason}"));
    }
    if let Some(name) = first_unknown_function(statement, policy) {
        reasons.push(format!("unknown_function:{name}"));
    }
    if let Some(name) = first_scoped_relation(statement, policy) {
        reasons.push(format!("scope_unproven:{name}"));
    }

    if reasons.is_empty() {
        SqlRisk::ReadOnly
    } else {
        SqlRisk::RequiresAuthorization { reasons }
    }
}

fn first_locking_query(statement: &Statement) -> Option<&'static str> {
    struct LockScan;
    impl Visitor for LockScan {
        type Break = &'static str;

        fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<&'static str> {
            if !query.locks.is_empty() {
                return ControlFlow::Break("locking_read");
            }
            if query.for_clause.is_some() {
                return ControlFlow::Break("for_clause");
            }
            ControlFlow::Continue(())
        }
    }

    match statement.visit(&mut LockScan) {
        ControlFlow::Break(reason) => Some(reason),
        ControlFlow::Continue(()) => None,
    }
}

fn first_denied_function(statement: &Statement, policy: &dyn SqlPolicy) -> Option<String> {
    struct DeniedFunctions<'a> {
        policy: &'a dyn SqlPolicy,
    }
    impl Visitor for DeniedFunctions<'_> {
        type Break = String;

        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<String> {
            if let Expr::Function(function) = expr
                && function_name_denied(self.policy, &function.name)
            {
                return ControlFlow::Break(function.name.to_string().to_ascii_lowercase());
            }
            ControlFlow::Continue(())
        }

        fn pre_visit_table_factor(&mut self, table_factor: &TableFactor) -> ControlFlow<String> {
            let name = match table_factor {
                TableFactor::Table { name, args, .. } if args.is_some() => Some(name),
                TableFactor::TableFunction {
                    expr: Expr::Function(function),
                    ..
                } => Some(&function.name),
                TableFactor::Function { name, .. } => Some(name),
                _ => None,
            };
            if let Some(name) = name
                && function_name_denied(self.policy, name)
            {
                return ControlFlow::Break(name.to_string().to_ascii_lowercase());
            }
            ControlFlow::Continue(())
        }
    }

    match statement.visit(&mut DeniedFunctions { policy }) {
        ControlFlow::Break(name) => Some(name),
        ControlFlow::Continue(()) => None,
    }
}

fn first_denied_relation(statement: &Statement, policy: &dyn SqlPolicy) -> Option<String> {
    let mut found = None;
    let _ = sqlparser::ast::visit_relations(statement, |name| {
        if relation_denied(policy, name) {
            found = Some(name.to_string().to_ascii_lowercase());
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    found
}

fn first_expression_side_effect(
    statement: &Statement,
    policy: &dyn SqlPolicy,
) -> Option<&'static str> {
    struct SideEffectScan<'a> {
        policy: &'a dyn SqlPolicy,
    }
    impl Visitor for SideEffectScan<'_> {
        type Break = &'static str;

        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<&'static str> {
            match self.policy.expression_side_effect(expr) {
                Some(reason) => ControlFlow::Break(reason),
                None => ControlFlow::Continue(()),
            }
        }
    }
    match statement.visit(&mut SideEffectScan { policy }) {
        ControlFlow::Break(reason) => Some(reason),
        ControlFlow::Continue(()) => None,
    }
}

/// 只有声明了已知函数表的方言才做正向白名单：表外的函数一律视为副作用未知。
fn first_unknown_function(statement: &Statement, policy: &dyn SqlPolicy) -> Option<String> {
    let known = policy.known_functions()?;
    struct UnknownFunctions<'a> {
        known: &'a [&'a str],
    }
    impl Visitor for UnknownFunctions<'_> {
        type Break = String;

        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<String> {
            let Expr::Function(function) = expr else {
                return ControlFlow::Continue(());
            };
            let name = function
                .name
                .to_string()
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            if self
                .known
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&name))
            {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(function.name.to_string().to_ascii_lowercase())
            }
        }
    }
    match statement.visit(&mut UnknownFunctions { known }) {
        ControlFlow::Break(name) => Some(name),
        ControlFlow::Continue(()) => None,
    }
}

/// 对象名的范围无法从语句文本证明（如数据库链接）。
fn first_scoped_relation(statement: &Statement, policy: &dyn SqlPolicy) -> Option<String> {
    let mut found = None;
    let _ = sqlparser::ast::visit_relations(statement, |name| {
        let full = name.to_string().to_ascii_lowercase();
        if policy.relation_scope_reason(&full).is_some() {
            found = Some(full);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    found
}

fn classify_metadata_read(
    statement: &Statement,
    policy: &dyn SqlPolicy,
    reasons: &mut Vec<String>,
) -> bool {
    match statement {
        Statement::ShowCreate { obj_type, .. } => {
            use sqlparser::ast::ShowCreateObject;
            if matches!(
                obj_type,
                ShowCreateObject::Table | ShowCreateObject::View | ShowCreateObject::Trigger
            ) {
                true
            } else {
                reasons.push("metadata_disclosure:show_create_object".into());
                true
            }
        }
        Statement::ExplainTable { .. } => true,
        Statement::ShowColumns { .. }
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowCollation { .. } => true,
        Statement::ShowDatabases { .. } => {
            reasons.push("metadata_disclosure:show_databases".into());
            true
        }
        Statement::ShowSchemas { .. } => {
            reasons.push("metadata_disclosure:show_schemas".into());
            true
        }
        Statement::ShowCatalogs { .. } => {
            reasons.push("metadata_disclosure:show_catalogs".into());
            true
        }
        Statement::ShowStatus { .. } => {
            reasons.push("metadata_disclosure:show_status".into());
            true
        }
        Statement::ShowVariables { .. } => {
            reasons.push("metadata_disclosure:show_variables".into());
            true
        }
        Statement::ShowProcessList { .. } => {
            reasons.push("metadata_disclosure:show_processlist".into());
            true
        }
        Statement::ShowVariable { variable } => {
            let head = variable
                .first()
                .map(|ident| ident.value.to_ascii_uppercase())
                .unwrap_or_default();
            let name = variable
                .iter()
                .map(|ident| ident.value.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join(" ");
            let read_only = policy.read_only_show_heads().contains(&head.as_str())
                || policy.read_only_show_names().contains(&name.as_str());
            if !read_only {
                reasons.push(format!("metadata_disclosure:show_{}", head.to_lowercase()));
            }
            true
        }
        Statement::Pragma { name, value, is_eq } => {
            let pragmas = policy.read_only_pragmas();
            if pragmas.is_empty() || *is_eq {
                return false;
            }
            let pragma = name
                .0
                .last()
                .and_then(|part| part.as_ident())
                .map(|ident| ident.value.to_ascii_lowercase())
                .unwrap_or_default();
            if pragmas.contains(&pragma.as_str()) {
                return value.is_none()
                    || matches!(
                        pragma.as_str(),
                        "table_info"
                            | "table_xinfo"
                            | "table_list"
                            | "index_list"
                            | "index_info"
                            | "index_xinfo"
                            | "foreign_key_list"
                    );
            }
            reasons.push(format!("metadata_disclosure:pragma_{pragma}"));
            true
        }
        _ => false,
    }
}

fn classify_query(query: &Query, reasons: &mut Vec<String>) {
    classify_set_expr(&query.body, reasons);
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            classify_query(&cte.query, reasons);
        }
    }
}

fn classify_set_expr(body: &SetExpr, reasons: &mut Vec<String>) {
    match body {
        SetExpr::Select(select) => {
            if select.into.is_some() {
                reasons.push("select_into".into());
            }
        }
        SetExpr::Query(inner) => classify_query(inner, reasons),
        SetExpr::SetOperation { left, right, .. } => {
            classify_set_expr(left, reasons);
            classify_set_expr(right, reasons);
        }
        SetExpr::Insert(_) => reasons.push("modifying_cte_insert".into()),
        SetExpr::Update(_) => reasons.push("modifying_cte_update".into()),
        SetExpr::Delete(_) => reasons.push("modifying_cte_delete".into()),
        SetExpr::Merge(_) => reasons.push("modifying_cte_merge".into()),
        _ => {}
    }
}

fn explain_options_request_analyze(options: Option<&[sqlparser::ast::UtilityOption]>) -> bool {
    let Some(options) = options else {
        return false;
    };
    options.iter().any(|option| {
        option.name.value.eq_ignore_ascii_case("analyze")
            && !matches!(
                &option.arg,
                Some(Expr::Value(sqlparser::ast::ValueWithSpan {
                    value: sqlparser::ast::Value::Boolean(false),
                    ..
                }))
            )
    })
}

fn variant_name(statement: &Statement) -> String {
    statement
        .to_string()
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .to_lowercase()
}

/// `db.run` 的统一 SQL 入口。
pub const DB_RUN_INPUT_SCHEMA: &str = r##"{
  "type": "object",
  "additionalProperties": false,
  "required": ["target", "sql"],
  "properties": {
    "target": { "type": "string", "minLength": 1 },
    "sql": { "type": "string", "minLength": 1, "maxLength": 65536 },
    "parameters": { "type": "array", "maxItems": 1024, "items": { "$ref": "#/$defs/parameter" } },
    "max_rows": { "type": "integer", "minimum": 1, "maximum": 1000 },
    "timeout_ms": { "type": "integer", "minimum": 100, "maximum": 30000 },
    "mode": { "type": "string", "enum": ["auto", "explain"] }
  },
  "$defs": {
    "parameter": {
      "oneOf": [
        { "type": "object", "additionalProperties": false, "required": ["type"], "properties": { "type": { "const": "null" } } },
        { "type": "object", "additionalProperties": false, "required": ["type", "value"], "properties": { "type": { "const": "bool" }, "value": { "type": "boolean" } } },
        { "type": "object", "additionalProperties": false, "required": ["type", "value"], "properties": { "type": { "const": "i64" }, "value": { "type": "string" } } },
        { "type": "object", "additionalProperties": false, "required": ["type", "value"], "properties": { "type": { "const": "u64" }, "value": { "type": "string" } } },
        { "type": "object", "additionalProperties": false, "required": ["type", "value"], "properties": { "type": { "const": "f64" }, "value": { "type": "number" } } },
        { "type": "object", "additionalProperties": false, "required": ["type", "value"], "properties": { "type": { "const": "decimal" }, "value": { "type": "string" } } },
        { "type": "object", "additionalProperties": false, "required": ["type", "value"], "properties": { "type": { "const": "text" }, "value": { "type": "string" } } },
        { "type": "object", "additionalProperties": false, "required": ["type", "value"], "properties": { "type": { "const": "bytes" }, "value": { "type": "string" } } }
      ]
    }
  }
}"##;

pub const DB_SEARCH_OBJECTS_INPUT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["target"],
  "properties": {
    "target": { "type": "string", "minLength": 1 },
    "query": { "type": "string", "maxLength": 256 },
    "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
  }
}"#;

pub const DB_DESCRIBE_OBJECT_INPUT_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["target", "object_ref"],
  "properties": {
    "target": { "type": "string", "minLength": 1 },
    "object_ref": {
      "type": "object",
      "additionalProperties": false,
      "required": ["name", "kind"],
      "properties": {
        "database": { "type": "string", "maxLength": 256 },
        "schema": { "type": "string", "maxLength": 256 },
        "name": { "type": "string", "minLength": 1, "maxLength": 256 },
        "kind": { "type": "string", "enum": ["table", "view"] }
      }
    },
    "include": {
      "type": "array",
      "maxItems": 6,
      "items": {
        "type": "string",
        "enum": ["columns", "indexes", "foreign_keys", "comment", "ddl"]
      }
    }
  }
}"#;

pub fn standard_sql_tool_specs() -> fluxdb_core::Result<Vec<ToolSpec>> {
    let descriptors = [
        fluxdb_core::ToolDescriptor {
            id: "db.run".into(),
            version: 1,
            description:
                "执行单条 SQL：只读语句直接返回结果，写语句需审批；mode=explain 只看计划。".into(),
            risk: fluxdb_core::ToolRisk::Write,
            input_schema_json: DB_RUN_INPUT_SCHEMA.into(),
        },
        fluxdb_core::ToolDescriptor {
            id: "db.search_objects".into(),
            version: 1,
            description: "按名字模糊搜索表/视图（渐进元数据，只读）".into(),
            risk: fluxdb_core::ToolRisk::Read,
            input_schema_json: DB_SEARCH_OBJECTS_INPUT_SCHEMA.into(),
        },
        fluxdb_core::ToolDescriptor {
            id: "db.describe_object".into(),
            version: 1,
            description: "读取表或视图的列、索引、外键、注释和可选 DDL（只读）".into(),
            risk: fluxdb_core::ToolRisk::Read,
            input_schema_json: DB_DESCRIBE_OBJECT_INPUT_SCHEMA.into(),
        },
    ];
    let mut specs = descriptors
        .iter()
        .map(ToolSpec::from_descriptor)
        .collect::<fluxdb_core::Result<Vec<_>>>()?;
    for spec in &mut specs {
        if spec.id == "db.run" {
            spec.effect = crate::EffectClass::Mutating;
            spec.output_shape = crate::ToolOutputShape::Sample {
                omit_field: "rows".into(),
                rows_omitted_field: Some("rows_omitted".into()),
            };
        }
    }
    Ok(specs)
}

#[cfg(test)]
mod tests {
    use super::{SqlRisk, analyze_sql, policy_for};
    use fluxdb_core::DatabaseKind;

    #[test]
    fn every_sql_database_has_a_policy() {
        for kind in [
            DatabaseKind::MySql,
            DatabaseKind::TiDb,
            DatabaseKind::Postgres,
            DatabaseKind::Sqlite,
            DatabaseKind::SqlServer,
            DatabaseKind::Dameng,
        ] {
            assert!(policy_for(kind).is_some(), "{kind:?}");
        }
        assert!(policy_for(DatabaseKind::Redis).is_none());
        assert!(policy_for(DatabaseKind::MongoDb).is_none());
    }

    #[test]
    fn common_select_is_read_only_for_every_policy() {
        for kind in [
            DatabaseKind::MySql,
            DatabaseKind::Postgres,
            DatabaseKind::Sqlite,
            DatabaseKind::SqlServer,
            DatabaseKind::Dameng,
        ] {
            let policy = policy_for(kind).expect("policy");
            assert_eq!(
                analyze_sql("SELECT 1", policy),
                SqlRisk::ReadOnly,
                "{kind:?}"
            );
        }
    }
}
