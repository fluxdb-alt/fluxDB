//! 跨数据库类型的建表 / 结构变更语句生成。
//!
//! 同类型同步沿用源端原始 DDL（保真）；只有两端类型不同时才走这里：先按
//! [`super::type_mapping`] 把源结构翻译成目标方言，再生成目标库可执行的语句。
//!
//! 生成范围：列（类型、自增、可空、默认值、注释）、主键、普通/唯一索引、外键、
//! 表注释。触发器、检查约束、分区等无法安全翻译的对象不生成，只在 `warnings` 中说明。
//! 外键单独返回，由调用方在所有表建好后统一添加，避免引用顺序问题。

use std::collections::BTreeMap;

use fluxdb_core::{DatabaseKind, ObjectPath};

use super::model::{ColumnDef, ForeignKeyDef, IndexDef};
use super::structure_diff::TableDef;
use super::type_mapping::{map_default, map_type};

/// 建表计划：可立即执行的语句 + 需要展示的告警。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CreateTablePlan {
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
}

/// 把源结构翻译成目标方言。差异比对与建表共用这一结果。
pub(crate) fn map_table_def(
    source: DatabaseKind,
    target: DatabaseKind,
    table: &TableDef,
) -> (TableDef, Vec<String>) {
    map_table_def_with(source, target, table, None)
}

/// 同上，但允许按列覆盖自动映射的目标类型。
///
/// `overrides` 的键是列名（不区分大小写），值是用户填写的目标类型文本；
/// 命中时直接采用该文本（仍会做布尔默认值适配），并记录一条转换提示。
pub(crate) fn map_table_def_with(
    source: DatabaseKind,
    target: DatabaseKind,
    table: &TableDef,
    overrides: Option<&BTreeMap<String, String>>,
) -> (TableDef, Vec<String>) {
    let mut warnings = Vec::new();
    let columns = table
        .columns
        .iter()
        .map(|column| {
            let mapped = map_type(source, target, &column.type_text);
            warnings.extend(
                mapped
                    .warnings
                    .into_iter()
                    .map(|warning| format!("{}：{warning}", column.name)),
            );
            let default_expr = column.default_expr.as_ref().and_then(|default| {
                let mapped_default = map_default(source, target, default);
                if let Some(warning) = mapped_default.warning {
                    warnings.push(format!("{}：{warning}", column.name));
                }
                mapped_default.expr
            });
            let mut type_text = mapped.text;
            if let Some(override_text) = override_type(overrides, &column.name)
                && !override_text.eq_ignore_ascii_case(&type_text)
            {
                warnings.push(format!(
                    "{}：已按自定义映射改用 {}（自动映射为 {type_text}）",
                    column.name, override_text
                ));
                type_text = override_text;
            }
            // 布尔列在 PostgreSQL 上是严格类型：MySQL 的 `DEFAULT 0/1` 必须写成
            // FALSE/TRUE，否则建表直接报类型不符。
            let default_expr = boolean_default(target, &type_text, default_expr);
            if column.generated.is_some() {
                // 生成表达式通常含方言函数，无法安全翻译；按普通列迁移，数据由同步写入。
                warnings.push(format!(
                    "{}：生成列表达式未跨方言翻译，已按普通列迁移",
                    column.name
                ));
            }
            ColumnDef {
                name: column.name.clone(),
                type_text,
                nullable: column.nullable,
                primary_key: column.primary_key,
                auto_increment: column.auto_increment,
                default_expr,
                generated: None,
                comment: column.comment.clone(),
            }
        })
        .collect();
    if target == DatabaseKind::Sqlite && !table.foreign_keys.is_empty() {
        warnings.push("SQLite 不支持追加外键约束，外键未迁移".to_string());
    }
    // schema 名在跨类型时没有对应关系（MySQL 的「schema」其实是库名），
    // 外键统一引用目标表所在 schema，避免生成不存在的限定名。
    let foreign_keys = table
        .foreign_keys
        .iter()
        .map(|foreign_key| ForeignKeyDef {
            ref_schema: None,
            ..foreign_key.clone()
        })
        .collect();
    // 索引方法/谓词/函数键项是方言私有概念，跨类型时清空，按目标方言普通索引重建。
    let indexes = table
        .indexes
        .iter()
        .filter(|index| {
            if index.has_expression {
                warnings.push(format!(
                    "索引 {}：函数/表达式索引无法跨方言重建，已跳过",
                    index.name
                ));
                return false;
            }
            if let Some(index_type) = index
                .index_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                warnings.push(format!(
                    "索引 {}：源端索引类型 {index_type} 未跨方言翻译，按普通索引创建",
                    index.name
                ));
            }
            true
        })
        .map(|index| IndexDef {
            index_type: None,
            predicate: None,
            ..index.clone()
        })
        .collect();
    if !table.checks.is_empty() {
        // 检查表达式可能引用方言函数；跨类型不迁移，避免生成目标库不存在的语法。
        warnings.push("CHECK 约束未跨方言迁移".to_string());
    }
    let mapped = TableDef {
        columns,
        primary_key: table.primary_key.clone(),
        indexes,
        foreign_keys,
        checks: Vec::new(),
        checks_loaded: false,
        comment: table.comment.clone(),
    };
    (mapped, warnings)
}

/// 读取列的自定义目标类型；空白值视为未设置。键不区分大小写。
fn override_type(overrides: Option<&BTreeMap<String, String>>, column: &str) -> Option<String> {
    let map = overrides?;
    let value = map.get(column).or_else(|| {
        map.iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(column))
            .map(|(_, value)| value)
    })?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// PostgreSQL 的 boolean 列不接受 0/1 默认值，跨类型时改写成 true/false。
fn boolean_default(target: DatabaseKind, type_text: &str, expr: Option<String>) -> Option<String> {
    if !target.is_postgres_family() || type_text != "boolean" {
        return expr;
    }
    // 用与 `pg_get_expr` 一致的小写字面量，避免下次比对把 true/TRUE 当成差异。
    match expr.as_deref().map(str::trim) {
        Some("0") | Some("'0'") => Some("false".to_string()),
        Some("1") | Some("'1'") => Some("true".to_string()),
        _ => expr,
    }
}

/// 生成目标表的 CREATE TABLE、索引与注释语句；外键不在此列。
pub(crate) fn create_table_statements(
    target: DatabaseKind,
    path: &ObjectPath,
    table: &TableDef,
) -> CreateTablePlan {
    let dialect = fluxdb_core::sql_dialect(target);
    let table_name = dialect.qualified_object_name(path);
    let mut plan = CreateTablePlan::default();
    let mut lines = Vec::new();
    let primary_columns = table
        .primary_key
        .as_ref()
        .map(|key| key.columns.clone())
        .unwrap_or_default();

    // SQLite 的自增只能写成单列 `INTEGER PRIMARY KEY AUTOINCREMENT`，
    // 多列主键或自增列不在主键里都无法表达。
    let sqlite_inline_identity = target == DatabaseKind::Sqlite
        && primary_columns.len() == 1
        && table.columns.iter().any(|column| {
            column.auto_increment
                && column.primary_key
                && column.name.eq_ignore_ascii_case(&primary_columns[0])
        });

    let mut auto_increment_seen = false;
    for column in &table.columns {
        if column.auto_increment {
            if auto_increment_seen {
                plan.warnings.push(format!(
                    "{}：目标库只允许一个自增列，已忽略该列的自增属性",
                    column.name
                ));
            }
            auto_increment_seen = true;
        }
        lines.push(column_definition(
            target,
            column,
            sqlite_inline_identity && column.auto_increment,
            &mut plan.warnings,
        ));
    }

    if !primary_columns.is_empty() && !sqlite_inline_identity {
        lines.push(format!(
            "PRIMARY KEY ({})",
            quote_columns(dialect, &primary_columns)
        ));
    }
    // 同类型同步的整表预览也走这里：把 CHECK 约束内联进建表语句，差异视图才完整。
    for (position, check) in table.checks.iter().enumerate() {
        let name = if check.name.trim().is_empty() {
            format!("chk_{}_{}", path.name, position + 1)
        } else {
            check.name.clone()
        };
        lines.push(format!(
            "CONSTRAINT {} CHECK ({})",
            dialect.quote_identifier(&name),
            check.expression
        ));
    }
    if lines.is_empty() {
        plan.warnings
            .push("源表没有可迁移的字段，未生成建表语句".to_string());
        return plan;
    }

    let mut create = format!(
        "CREATE TABLE {table_name} (\n{}\n)",
        lines
            .into_iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join(",\n")
    );
    if target.is_mysql_family()
        && let Some(comment) = table
            .comment
            .as_deref()
            .filter(|text| !text.trim().is_empty())
    {
        create.push_str(&format!(" COMMENT={}", literal(comment, target)));
    }
    create.push(';');
    plan.statements.push(create);

    for index in &table.indexes {
        if let Some(statement) =
            create_index_statement(target, &table_name, index, table, &mut plan.warnings)
        {
            plan.statements.push(statement);
        }
    }
    plan.statements
        .extend(comment_statements(target, path, table, &mut plan.warnings));
    plan
}

fn column_definition(
    target: DatabaseKind,
    column: &ColumnDef,
    sqlite_inline_identity: bool,
    warnings: &mut Vec<String>,
) -> String {
    let dialect = fluxdb_core::sql_dialect(target);
    // SQLite 的 AUTOINCREMENT 只认 `INTEGER PRIMARY KEY` 字面量。
    let type_text = if sqlite_inline_identity {
        "integer"
    } else {
        &column.type_text
    };
    let mut sql = format!("{} {}", dialect.quote_identifier(&column.name), type_text);
    if column.auto_increment && !sqlite_inline_identity {
        if target == DatabaseKind::Sqlite {
            warnings.push(format!(
                "{}：SQLite 自增需为单列 INTEGER 主键，已忽略自增属性",
                column.name
            ));
        } else {
            match identity_clause(target) {
                Some(clause) if identity_capable(&column.type_text) => sql.push_str(clause),
                Some(_) => warnings.push(format!(
                    "{}：类型 {} 不支持自增，已忽略自增属性",
                    column.name, column.type_text
                )),
                None => warnings.push(format!(
                    "{}：目标数据库不支持自增语法，已忽略自增属性",
                    column.name
                )),
            }
        }
    }
    if !column.nullable {
        sql.push_str(" NOT NULL");
    }
    // 自增列不接受显式 DEFAULT。
    if let Some(default) = column
        .default_expr
        .as_deref()
        .filter(|_| !column.auto_increment)
    {
        // MySQL 的 TEXT / BLOB / JSON 列不允许字面量默认值（8.0.13+ 也只支持
        // 表达式默认），跨类型落到这些列时移除默认值并提示。
        if target.is_mysql_family() && mysql_lob_type(&column.type_text) {
            warnings.push(format!(
                "{}：MySQL 的 {} 列不支持默认值，已移除默认值 {default}",
                column.name, column.type_text
            ));
        } else {
            sql.push_str(&format!(" DEFAULT {default}"));
        }
    }
    if target.is_mysql_family()
        && let Some(comment) = column
            .comment
            .as_deref()
            .filter(|text| !text.trim().is_empty())
    {
        sql.push_str(&format!(" COMMENT {}", literal(comment, target)));
    }
    if sqlite_inline_identity {
        // `INTEGER PRIMARY KEY AUTOINCREMENT` 必须紧跟类型，NULL 约束会破坏该形式。
        return sql.replace(" NOT NULL", "") + " PRIMARY KEY AUTOINCREMENT";
    }
    sql
}

fn identity_clause(target: DatabaseKind) -> Option<&'static str> {
    match target {
        kind if kind.is_mysql_family() => Some(" AUTO_INCREMENT"),
        kind if kind.is_postgres_family() => Some(" GENERATED BY DEFAULT AS IDENTITY"),
        DatabaseKind::SqlServer => Some(" IDENTITY(1,1)"),
        DatabaseKind::Oracle => Some(" GENERATED BY DEFAULT AS IDENTITY"),
        DatabaseKind::Dameng => Some(" IDENTITY(1,1)"),
        // SQLite 走 INTEGER PRIMARY KEY AUTOINCREMENT 内联形式。
        _ => None,
    }
}

/// MySQL 的 TEXT / BLOB / JSON / GEOMETRY 列不接受字面量默认值。
fn mysql_lob_type(type_text: &str) -> bool {
    let lower = type_text.to_ascii_lowercase();
    lower.contains("text")
        || lower.contains("blob")
        || lower.starts_with("json")
        || lower.contains("geometry")
}

fn identity_capable(type_text: &str) -> bool {
    let lower = type_text.to_ascii_lowercase();
    lower.contains("int")
        || lower.contains("number")
        || lower.contains("decimal")
        || lower.contains("numeric")
        || lower.contains("serial")
}

fn create_index_statement(
    target: DatabaseKind,
    table_name: &str,
    index: &IndexDef,
    table: &TableDef,
    warnings: &mut Vec<String>,
) -> Option<String> {
    if index.primary || index.columns.is_empty() {
        return None;
    }
    if index.name.trim().is_empty() {
        warnings.push("存在未命名索引，已跳过".to_string());
        return None;
    }
    if index.has_expression {
        warnings.push(format!(
            "索引 {}：含表达式键项，无法按列名重建，已跳过",
            index.name
        ));
        return None;
    }
    let dialect = fluxdb_core::sql_dialect(target);
    let columns = index
        .columns
        .iter()
        .map(|name| {
            let quoted = dialect.quote_identifier(name);
            // MySQL 不允许对 TEXT / BLOB 列建无长度前缀的索引；按 255 字符前缀生成。
            let is_lob = target.is_mysql_family()
                && table
                    .columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(name))
                    .is_some_and(|column| mysql_lob_type(&column.type_text));
            if is_lob {
                warnings.push(format!(
                    "索引 {} 的列 {name} 为 TEXT/BLOB，MySQL 只支持前缀索引，已按 255 字符前缀创建",
                    index.name
                ));
                format!("{quoted}(255)")
            } else {
                quoted
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    // 索引方法按目标方言表达：MySQL 的 FULLTEXT/SPATIAL 是前缀，PG 用 USING 子句。
    let index_type = index
        .index_type
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_uppercase();
    let (kind_prefix, unique_prefix, using) = if target.is_mysql_family() {
        match index_type.as_str() {
            "FULLTEXT" => ("FULLTEXT ", "", None),
            "SPATIAL" => ("SPATIAL ", "", None),
            "HASH" => ("", if index.unique { "UNIQUE " } else { "" }, Some("HASH")),
            _ => ("", if index.unique { "UNIQUE " } else { "" }, None),
        }
    } else {
        (
            "",
            if index.unique { "UNIQUE " } else { "" },
            (!index_type.is_empty()).then_some(index.index_type.as_deref().unwrap_or_default()),
        )
    };
    let mut statement = format!(
        "CREATE {unique_prefix}{kind_prefix}INDEX {} ON {table_name}",
        dialect.quote_identifier(&index.name),
    );
    if let Some(using) = using {
        statement.push_str(&format!(" USING {using}"));
    }
    statement.push_str(&format!(" ({columns})"));
    if let Some(predicate) = index
        .predicate
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && target.is_postgres_family()
    {
        statement.push_str(&format!(" WHERE {predicate}"));
    }
    statement.push(';');
    Some(statement)
}

fn comment_statements(
    target: DatabaseKind,
    path: &ObjectPath,
    table: &TableDef,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let has_comments = table
        .comment
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
        || table.columns.iter().any(|column| {
            column
                .comment
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty())
        });
    if !has_comments {
        return Vec::new();
    }
    let dialect = fluxdb_core::sql_dialect(target);
    let table_name = dialect.qualified_object_name(path);
    match target {
        DatabaseKind::Sqlite => {
            warnings.push("SQLite 不支持表和列注释，注释未迁移".to_string());
            Vec::new()
        }
        kind if kind.is_mysql_family() => Vec::new(),
        DatabaseKind::SqlServer => {
            let schema = path.schema.as_deref().unwrap_or("dbo");
            let mut statements = Vec::new();
            if let Some(comment) = table
                .comment
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                statements.push(format!(
                    "EXEC sp_addextendedproperty N'MS_Description', {}, N'SCHEMA', {}, N'TABLE', {}, N'COLUMN', NULL;",
                    literal(comment, target),
                    literal(schema, target),
                    literal(&path.name, target)
                ));
            }
            for column in &table.columns {
                if let Some(comment) = column
                    .comment
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                {
                    statements.push(format!(
                        "EXEC sp_addextendedproperty N'MS_Description', {}, N'SCHEMA', {}, N'TABLE', {}, N'COLUMN', {};",
                        literal(comment, target),
                        literal(schema, target),
                        literal(&path.name, target),
                        literal(&column.name, target)
                    ));
                }
            }
            statements
        }
        _ => {
            let mut statements = Vec::new();
            if let Some(comment) = table
                .comment
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                statements.push(format!(
                    "COMMENT ON TABLE {table_name} IS {};",
                    literal(comment, target)
                ));
            }
            for column in &table.columns {
                if let Some(comment) = column
                    .comment
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                {
                    statements.push(format!(
                        "COMMENT ON COLUMN {table_name}.{} IS {};",
                        dialect.quote_identifier(&column.name),
                        literal(comment, target)
                    ));
                }
            }
            statements
        }
    }
}

/// 外键语句：必须在所有目标表创建完成后执行。
///
/// SQLite 只能在建表时内联声明外键，不支持 `ALTER TABLE ... ADD CONSTRAINT`，
/// 因此目标为 SQLite 时返回空集合，由预览提示外键未迁移。
pub(crate) fn foreign_key_statements(
    target: DatabaseKind,
    path: &ObjectPath,
    foreign_keys: &[ForeignKeyDef],
) -> Vec<String> {
    if foreign_keys.is_empty() || target == DatabaseKind::Sqlite {
        return Vec::new();
    }
    let dialect = fluxdb_core::sql_dialect(target);
    let table_name = dialect.qualified_object_name(path);
    foreign_keys
        .iter()
        .filter(|foreign_key| !foreign_key.columns.is_empty())
        .map(|foreign_key| {
            let name = if foreign_key.name.trim().is_empty() {
                foreign_key_name(&path.name, foreign_key)
            } else {
                foreign_key.name.clone()
            };
            // 外键可能跨 schema：优先用源端记录的引用 schema，缺失才回落到当前表 schema。
            let referenced = if target.uses_schemas() {
                match foreign_key.ref_schema.as_deref().or(path.schema.as_deref()) {
                    Some(schema) => format!(
                        "{}.{}",
                        dialect.quote_identifier(schema),
                        dialect.quote_identifier(&foreign_key.ref_table)
                    ),
                    None => dialect.quote_identifier(&foreign_key.ref_table),
                }
            } else {
                dialect.quote_identifier(&foreign_key.ref_table)
            };
            let mut statement = format!(
                "ALTER TABLE {table_name} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {referenced} ({})",
                dialect.quote_identifier(&name),
                quote_columns(dialect, &foreign_key.columns),
                quote_columns(dialect, &foreign_key.ref_columns)
            );
            if let Some(action) = &foreign_key.on_delete {
                statement.push_str(&format!(" ON DELETE {action}"));
            }
            if let Some(action) = &foreign_key.on_update {
                statement.push_str(&format!(" ON UPDATE {action}"));
            }
            statement.push(';');
            statement
        })
        .collect()
}

fn foreign_key_name(table: &str, foreign_key: &ForeignKeyDef) -> String {
    let mut name = format!("fk_{}_{}", table, foreign_key.columns.join("_"));
    name.retain(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    if name.len() > 63 {
        name.truncate(63);
    }
    name
}

fn quote_columns(dialect: &dyn fluxdb_core::SqlDialect, columns: &[String]) -> String {
    columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ")
}

fn literal(value: &str, target: DatabaseKind) -> String {
    if target == DatabaseKind::SqlServer {
        format!("N'{}'", value.replace('\'', "''"))
    } else if target.is_mysql_family() {
        format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_core::{ConnectionId, ObjectKind};

    fn path() -> ObjectPath {
        ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("app".into()),
            schema: Some("public".into()),
            name: "orders".into(),
            kind: ObjectKind::Table,
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

    fn mysql_source() -> TableDef {
        let mut id = column("id", "bigint unsigned", false);
        id.primary_key = true;
        id.auto_increment = true;
        let mut name = column("name", "varchar(64)", false);
        name.comment = Some("订单名".into());
        let mut created = column("created_at", "datetime", false);
        created.default_expr = Some("CURRENT_TIMESTAMP".into());
        TableDef {
            columns: vec![id, name, created],
            primary_key: Some(IndexDef {
                name: "PRIMARY".into(),
                columns: vec!["id".into()],
                unique: true,
                primary: true,
                index_type: None,
                predicate: None,
                has_expression: false,
            }),
            indexes: vec![IndexDef {
                name: "idx_name".into(),
                columns: vec!["name".into()],
                unique: false,
                primary: false,
                index_type: None,
                predicate: None,
                has_expression: false,
            }],
            foreign_keys: Vec::new(),
            checks: Vec::new(),
            checks_loaded: false,
            comment: Some("订单表".into()),
        }
    }

    #[test]
    fn mysql_to_postgres_create_table_maps_types_and_comments() {
        let (mapped, warnings) =
            map_table_def(DatabaseKind::MySql, DatabaseKind::Postgres, &mysql_source());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings.iter().any(|warning| warning.contains("无符号")),
            "{warnings:?}"
        );
        let plan = create_table_statements(DatabaseKind::Postgres, &path(), &mapped);
        assert_eq!(
            plan.statements[0].lines().next().unwrap(),
            "CREATE TABLE \"public\".\"orders\" ("
        );
        let create = &plan.statements[0];
        assert!(create.contains("\"id\" bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL"));
        assert!(create.contains("\"name\" varchar(64) NOT NULL"));
        assert!(create.contains("PRIMARY KEY (\"id\")"));
        assert!(
            plan.statements
                .iter()
                .any(|statement| statement.contains("CREATE INDEX \"idx_name\" ON"))
        );
        assert!(
            plan.statements
                .iter()
                .any(|statement| statement.contains("COMMENT ON TABLE"))
        );
        assert!(
            plan.statements
                .iter()
                .any(|statement| statement.contains("COMMENT ON COLUMN"))
        );
    }

    #[test]
    fn mysql_to_sqlserver_uses_identity_and_extended_properties() {
        let (mapped, _) = map_table_def(
            DatabaseKind::MySql,
            DatabaseKind::SqlServer,
            &mysql_source(),
        );
        let plan = create_table_statements(DatabaseKind::SqlServer, &path(), &mapped);
        let create = &plan.statements[0];
        assert!(
            create.contains("[id] decimal(20,0) IDENTITY(1,1) NOT NULL"),
            "{create}"
        );
        assert!(
            plan.statements
                .iter()
                .any(|statement| statement.contains("sp_addextendedproperty"))
        );
    }

    #[test]
    fn sqlite_target_uses_inline_autoincrement() {
        let mut id = column("id", "int", false);
        id.primary_key = true;
        id.auto_increment = true;
        let table = TableDef {
            columns: vec![id],
            primary_key: Some(IndexDef {
                name: String::new(),
                columns: vec!["id".into()],
                unique: true,
                primary: true,
                index_type: None,
                predicate: None,
                has_expression: false,
            }),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            checks: Vec::new(),
            checks_loaded: false,
            comment: None,
        };
        let plan = create_table_statements(DatabaseKind::Sqlite, &path(), &table);
        assert!(
            plan.statements[0].contains("\"id\" integer PRIMARY KEY AUTOINCREMENT"),
            "{}",
            plan.statements[0]
        );
    }

    #[test]
    fn sqlite_multi_column_table_keeps_inline_autoincrement() {
        let mut id = column("id", "int", false);
        id.primary_key = true;
        id.auto_increment = true;
        let table = TableDef {
            columns: vec![id, column("name", "varchar(64)", false)],
            primary_key: Some(IndexDef {
                name: "PRIMARY".into(),
                columns: vec!["id".into()],
                unique: true,
                primary: true,
                index_type: None,
                predicate: None,
                has_expression: false,
            }),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            checks: Vec::new(),
            checks_loaded: false,
            comment: None,
        };
        let plan = create_table_statements(DatabaseKind::Sqlite, &path(), &table);
        assert!(
            plan.statements[0].contains("\"id\" integer PRIMARY KEY AUTOINCREMENT"),
            "{}",
            plan.statements[0]
        );
        assert!(plan.statements[0].contains("\"name\" varchar(64) NOT NULL"));
    }

    #[test]
    fn postgres_boolean_default_is_literal_lowercase() {
        let mut flag = column("is_active", "tinyint(1)", false);
        flag.default_expr = Some("1".into());
        let table = TableDef {
            columns: vec![flag],
            primary_key: None,
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            checks: Vec::new(),
            checks_loaded: false,
            comment: None,
        };
        let (mapped, _) = map_table_def(DatabaseKind::MySql, DatabaseKind::Postgres, &table);
        assert_eq!(mapped.columns[0].default_expr.as_deref(), Some("true"));
        let plan = create_table_statements(DatabaseKind::Postgres, &path(), &mapped);
        assert!(
            plan.statements[0].contains("\"is_active\" boolean NOT NULL DEFAULT true"),
            "{}",
            plan.statements[0]
        );
    }

    #[test]
    fn mysql_text_default_dropped_and_index_uses_prefix() {
        let mut status = column("status", "text", false);
        status.default_expr = Some("'active'".into());
        let table = TableDef {
            columns: vec![status],
            primary_key: None,
            indexes: vec![IndexDef {
                name: "idx_status".into(),
                columns: vec!["status".into()],
                unique: false,
                primary: false,
                index_type: None,
                predicate: None,
                has_expression: false,
            }],
            foreign_keys: Vec::new(),
            checks: Vec::new(),
            checks_loaded: false,
            comment: None,
        };
        let (mapped, _) = map_table_def(DatabaseKind::Postgres, DatabaseKind::MySql, &table);
        let plan = create_table_statements(DatabaseKind::MySql, &path(), &mapped);
        assert!(
            !plan.statements[0].contains("DEFAULT 'active'"),
            "{}",
            plan.statements[0]
        );
        assert!(
            plan.statements
                .iter()
                .any(|statement| statement.contains("(`status`(255))")),
            "{:?}",
            plan.statements
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("不支持默认值")),
            "{:?}",
            plan.warnings
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("前缀索引")),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn cross_type_foreign_key_drops_source_schema() {
        let mut table = mysql_source();
        table.foreign_keys = vec![ForeignKeyDef {
            name: "fk_orders_customer".into(),
            columns: vec!["customer_id".into()],
            ref_schema: Some("fluxdb_cross_src".into()),
            ref_table: "customers".into(),
            ref_columns: vec!["id".into()],
            on_delete: None,
            on_update: None,
        }];
        let (mapped, _) = map_table_def(DatabaseKind::MySql, DatabaseKind::Postgres, &table);
        assert_eq!(mapped.foreign_keys[0].ref_schema, None);
        let statements =
            foreign_key_statements(DatabaseKind::Postgres, &path(), &mapped.foreign_keys);
        assert!(statements[0].contains("REFERENCES \"public\".\"customers\""));
    }

    #[test]
    fn sqlite_target_skips_foreign_keys() {
        let foreign_keys = vec![ForeignKeyDef {
            name: String::new(),
            columns: vec!["customer_id".into()],
            ref_schema: None,
            ref_table: "customers".into(),
            ref_columns: vec!["id".into()],
            on_delete: None,
            on_update: None,
        }];
        assert!(foreign_key_statements(DatabaseKind::Sqlite, &path(), &foreign_keys).is_empty());
    }

    #[test]
    fn column_type_override_replaces_auto_mapping() {
        // `created_at` 自动映射为 `timestamp without time zone`，用户要求带时区。
        let overrides = BTreeMap::from([("created_at".to_string(), "  timestamptz  ".to_string())]);
        let (mapped, warnings) = map_table_def_with(
            DatabaseKind::MySql,
            DatabaseKind::Postgres,
            &mysql_source(),
            Some(&overrides),
        );
        let created = mapped
            .columns
            .iter()
            .find(|column| column.name == "created_at")
            .expect("created_at");
        assert_eq!(created.type_text, "timestamptz");
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("自定义映射")),
            "{warnings:?}"
        );

        // 键不区分大小写；空白值视为未设置。
        let overrides = BTreeMap::from([("CREATED_AT".to_string(), "   ".to_string())]);
        let (mapped, _) = map_table_def_with(
            DatabaseKind::MySql,
            DatabaseKind::Postgres,
            &mysql_source(),
            Some(&overrides),
        );
        assert_eq!(
            mapped
                .columns
                .iter()
                .find(|column| column.name == "created_at")
                .expect("created_at")
                .type_text,
            "timestamp without time zone"
        );
    }

    #[test]
    fn generated_foreign_key_is_added_separately() {
        let mut fk = vec![ForeignKeyDef {
            name: String::new(),
            columns: vec!["customer_id".into()],
            ref_schema: Some("public".into()),
            ref_table: "customers".into(),
            ref_columns: vec!["id".into()],
            on_delete: None,
            on_update: None,
        }];
        let statements = foreign_key_statements(DatabaseKind::Postgres, &path(), &fk);
        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("ADD CONSTRAINT \"fk_orders_customer_id\""));
        assert!(statements[0].contains("REFERENCES \"public\".\"customers\" (\"id\")"));
        fk.clear();
    }
}
