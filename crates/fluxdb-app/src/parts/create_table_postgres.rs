// PostgreSQL 新建表 provider 的 DDL 生成（T16，设计 §9.2）。
//
// PG 与 MySQL 的建表语义差别集中在：schema 限定、identity 而非 AUTO_INCREMENT、
// 无 engine/charset/unsigned/zerofill、触发器必须引用已存在函数。
// 这里只生成「可重建的领域结构计划」，UI 不拼 SQL；值一律经 quote 处理，标识符不裸拼。

/// PG 常用列类型（设计 §7 覆盖范围；serial 保留以兼容既有 serial 列）。
pub(crate) const POSTGRES_CREATE_TABLE_TYPE_OPTIONS: &[&str] = &[
    "smallint",
    "integer",
    "bigint",
    "serial",
    "bigserial",
    "numeric",
    "real",
    "double precision",
    "boolean",
    "text",
    "varchar",
    "char",
    "bytea",
    "date",
    "time",
    "timetz",
    "timestamp",
    "timestamptz",
    "interval",
    "uuid",
    "json",
    "jsonb",
    "xml",
    "inet",
    "cidr",
    "macaddr",
    "money",
];

pub(crate) fn quote_postgres_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// 单引号字符串字面量（注释体等值位），转义内部单引号。
pub(crate) fn quote_postgres_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 整数族类型：identity 只能挂在整数列上。
pub(crate) fn postgres_is_integer_type(data_type: &str) -> bool {
    matches!(
        create_table_base_type(data_type).as_str(),
        "smallint" | "int2" | "integer" | "int" | "int4" | "bigint" | "int8" | "serial"
            | "bigserial" | "smallserial"
    )
}

/// identity 列必须落在整数类型上，否则 PG 拒绝建表。
pub(crate) fn postgres_identity_column_error(column: &CreateTableColumn) -> Option<String> {
    if column.auto_increment && !postgres_is_integer_type(&column.data_type) {
        return Some(format!(
            "列 {} 使用 identity 时必须是整数类型（smallint/integer/bigint）",
            column.name.trim()
        ));
    }
    None
}

/// PG 触发器由「已存在的函数」驱动，不接受 MySQL 的 BEGIN…END 行内 body。
///
/// body 空白时视为未填写；含分号或换行的文本按行内 body 处理并给出明确提示，
/// 而不是生成一条 PG 无法解析的 CREATE TRIGGER。
pub(crate) fn postgres_trigger_error(trigger: &CreateTableTrigger) -> Option<String> {
    let body = trigger.body.trim();
    if body.is_empty() {
        return None;
    }
    if body.contains(';') || body.contains('\n') {
        return Some(format!(
            "触发器 {} 需要引用已存在的函数；PG 不支持行内 BEGIN…END 触发器体",
            trigger.name.trim()
        ));
    }
    None
}

/// 生成 PG 建表 DDL 预览（CREATE TABLE + 注释 + 索引 + 触发器，按依赖顺序）。
pub(crate) fn create_table_postgres_sql_preview(
    create: &CreateTableState,
) -> Result<String, String> {
    if let Some(message) = create.validation_error() {
        return Err(message.to_string());
    }
    let statements = create_table_postgres_statements(create)?;
    Ok(statements.join("\n"))
}

/// 领域结构 → PG 语句序列（预览与执行同一份计划）。
fn create_table_postgres_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let table_name = create.table_name.trim();
    let qualified = postgres_qualified_table(create, table_name);

    let mut lines = create
        .columns
        .iter()
        .filter_map(create_table_postgres_column_line)
        .collect::<Vec<_>>();
    // PG 的主键可内联，且标识符用双引号（MySQL 版本是反引号）；
    // 普通/唯一索引是独立的 CREATE INDEX 语句，不放进 CREATE TABLE 体内。
    if let Some(primary_key) = create_table_postgres_primary_key_line(&create.columns) {
        lines.push(primary_key);
    }
    lines.extend(create_table_foreign_key_lines(
        create,
        CreateTableSqlDialect::Postgres,
    )?);
    lines.extend(create_table_check_lines(
        create,
        CreateTableSqlDialect::Postgres,
    )?);

    let mut statements = vec![format!(
        "CREATE TABLE {qualified} (\n{}\n);",
        lines
            .into_iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join(",\n")
    )];

    // 表/列注释：PG 用 COMMENT ON，不是 MySQL 的列内 COMMENT。
    let comment = create.comment.trim();
    if !comment.is_empty() {
        statements.push(format!(
            "COMMENT ON TABLE {qualified} IS {};",
            quote_postgres_string(comment)
        ));
    }
    for column in create.columns.iter() {
        let name = column.name.trim();
        let column_comment = column.comment.trim();
        if name.is_empty() || column_comment.is_empty() {
            continue;
        }
        statements.push(format!(
            "COMMENT ON COLUMN {qualified}.{} IS {};",
            quote_postgres_identifier(name),
            quote_postgres_string(column_comment)
        ));
    }

    statements.extend(create_table_postgres_index_statements(create)?);
    statements.extend(create_table_postgres_trigger_statements(create)?);
    Ok(statements)
}

/// schema 限定表名：显式 schema 优先，未指定时交给连接 search_path 解析（不硬编码 public）。
fn postgres_qualified_table(create: &CreateTableState, table_name: &str) -> String {
    let schema = create.schema.trim();
    if schema.is_empty() {
        quote_postgres_identifier(table_name)
    } else {
        format!(
            "{}.{}",
            quote_postgres_identifier(schema),
            quote_postgres_identifier(table_name)
        )
    }
}

/// 单列定义行：类型、identity、NOT NULL、DEFAULT。
///
/// PG 不含 MySQL 属性（unsigned/zerofill/ON UPDATE/charset/binary）；这些字段在 PG provider
/// 的能力位里关闭，UI 不展示，也不会被写进 SQL。
fn create_table_postgres_column_line(column: &CreateTableColumn) -> Option<String> {
    let name = column.name.trim();
    if name.is_empty() {
        return None;
    }
    let mut sql = format!(
        "{} {}",
        quote_postgres_identifier(name),
        create_table_postgres_column_type(column)
    );
    if column.auto_increment {
        // 设计 §9.2：优先 GENERATED BY DEFAULT AS IDENTITY（可用显式值覆盖，便于导入）。
        sql.push_str(" GENERATED BY DEFAULT AS IDENTITY");
    }
    if !column.nullable || column.primary_key {
        sql.push_str(" NOT NULL");
    }
    let default_value = column.default_value.trim();
    if !default_value.is_empty() {
        sql.push_str(" DEFAULT ");
        sql.push_str(default_value);
    }
    // 列注释在 PG 里是独立的 COMMENT ON 语句，见 create_table_postgres_statements。
    Some(sql)
}

/// 列类型文本：保留类型名（含 schema/长度/精度），显式长度/精度按用户输入补齐。
fn create_table_postgres_column_type(column: &CreateTableColumn) -> String {
    let base = create_table_base_type(&column.data_type);
    let length = column.length.trim();
    let scale = column.scale.trim();
    // PG 精度语法按类型族判断，不复用 MySQL 的 varchar/decimal 判定（numeric ≠ decimal）。
    let supports_scale = matches!(base.as_str(), "numeric" | "decimal");
    let supports_length = matches!(base.as_str(), "varchar" | "char" | "bpchar" | "bit" | "varbit");
    let modifiers = if length.is_empty() || !(supports_length || supports_scale) {
        String::new()
    } else if supports_scale && !scale.is_empty() {
        format!("({length},{scale})")
    } else {
        format!("({length})")
    };

    match modifiers.is_empty() {
        true => base,
        false => format!("{base}{modifiers}"),
    }
}

/// 主键约束行（PG：双引号标识符；key length 前缀是 MySQL 概念，PG 不用）。
fn create_table_postgres_primary_key_line(columns: &[CreateTableColumn]) -> Option<String> {
    let parts = columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .map(|column| quote_postgres_identifier(column.name.trim()))
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| format!("PRIMARY KEY ({})", parts.join(", ")))
}

/// 索引语句：`CREATE [UNIQUE] INDEX "name" ON <table> [USING method] ("col" [ASC|DESC], ...)`。
///
/// 校验与 MySQL 路径保持一致（名称非空/不重复/列存在/列不重复/sub_part 为数字），
/// 但语法按 PG 生成；MySQL 的 FULLTEXT/SPATIAL 与索引前缀长度在 PG 无对应，明确拒绝。
fn create_table_postgres_index_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let qualified = postgres_qualified_table(create, create.table_name.trim());
    let valid_columns = create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    let mut names = std::collections::BTreeSet::new();
    let mut statements = Vec::with_capacity(create.indexes.len());

    for index in &create.indexes {
        let name = index.name.trim();
        if name.is_empty() {
            return Err("索引名称不能为空".to_string());
        }
        if name.eq_ignore_ascii_case("PRIMARY") {
            return Err("索引名称不能为 PRIMARY".to_string());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("索引名称重复：{name}"));
        }
        if index.columns.is_empty() {
            return Err(format!("索引 {name} 至少需要一个字段"));
        }
        let index_type = create_table_normalized_index_type(&index.index_type);
        if matches!(index_type, "FULLTEXT" | "SPATIAL") {
            return Err(format!("PostgreSQL 不支持 {index_type} 索引"));
        }

        let mut seen = std::collections::BTreeSet::new();
        let mut parts = Vec::with_capacity(index.columns.len());
        for column in &index.columns {
            let column_name = column.name.trim();
            if column_name.is_empty() {
                return Err(format!("索引 {name} 存在空字段"));
            }
            if !valid_columns.contains(&column_name.to_ascii_lowercase()) {
                return Err(format!("索引 {name} 引用了不存在的字段：{column_name}"));
            }
            if !seen.insert(column_name.to_ascii_lowercase()) {
                return Err(format!("索引 {name} 存在重复字段：{column_name}"));
            }
            if !column.sub_part.trim().is_empty() {
                return Err("PostgreSQL 索引不支持前缀长度".to_string());
            }
            let mut part = quote_postgres_identifier(column_name);
            let sort_order = create_table_normalized_sort_order(&column.sort_order);
            if !sort_order.is_empty() {
                part.push(' ');
                part.push_str(sort_order);
            }
            parts.push(part);
        }

        let unique = if index_type == "UNIQUE" { "UNIQUE " } else { "" };
        let method = create_table_normalized_index_method(&index.index_method);
        let using = if method.is_empty() || method == "BTREE" {
            // BTREE 是 PG 默认，不写 USING 保持 DDL 干净。
            String::new()
        } else {
            format!(" USING {method}")
        };
        statements.push(format!(
            "CREATE {unique}INDEX {} ON {qualified}{using} ({});",
            quote_postgres_identifier(name),
            parts.join(", ")
        ));
    }
    Ok(statements)
}

/// 触发器：引用已存在函数，`EXECUTE FUNCTION <body>()`。
fn create_table_postgres_trigger_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let qualified = postgres_qualified_table(create, create.table_name.trim());
    let mut statements = Vec::new();
    for trigger in create.triggers.iter() {
        let name = trigger.name.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(message) = postgres_trigger_error(trigger) {
            return Err(message);
        }
        let body = trigger.body.trim();
        if body.is_empty() {
            return Err(format!("触发器 {name} 需要指定已存在的函数名"));
        }
        // 函数名允许带 schema 限定；逐段转义，不接受任意表达式。
        let function = body
            .split('.')
            .map(|part| quote_postgres_identifier(part.trim()))
            .collect::<Vec<_>>()
            .join(".");
        let timing = create_table_normalized_trigger_timing(&trigger.timing);
        let event = create_table_normalized_trigger_event(&trigger.event);
        statements.push(format!(
            "CREATE TRIGGER {} {timing} {event} ON {qualified} FOR EACH ROW EXECUTE FUNCTION {function}();",
            quote_postgres_identifier(name)
        ));
    }
    Ok(statements)
}
