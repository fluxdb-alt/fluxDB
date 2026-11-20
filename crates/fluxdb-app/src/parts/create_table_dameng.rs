// 达梦 DM8 建表/设计表 SQL。
//
// DM 使用双引号标识符、`IDENTITY(1,1)` 自增和独立 `COMMENT ON`；
// 这里集中生成可重建的领域结构，UI 不直接拼 SQL。

pub(crate) const DAMENG_CREATE_TABLE_TYPE_OPTIONS: &[&str] = &[
    "char",
    "varchar",
    "varchar2",
    "nchar",
    "nvarchar",
    "nvarchar2",
    "clob",
    "text",
    "blob",
    "image",
    "binary",
    "varbinary",
    "tinyint",
    "smallint",
    "int",
    "integer",
    "bigint",
    "number",
    "numeric",
    "decimal",
    "real",
    "float",
    "double",
    "double precision",
    "bit",
    "boolean",
    "date",
    "time",
    "timestamp",
    "datetime",
    "json",
];

pub(crate) fn create_table_dameng_validation_error(
    create: &CreateTableState,
) -> Option<&'static str> {
    if create.table_name.trim().is_empty() {
        return Some("请输入表名");
    }

    let named_columns = create
        .columns
        .iter()
        .filter(|column| !column.name.trim().is_empty())
        .collect::<Vec<_>>();
    if named_columns.is_empty() {
        return Some("请至少填写一个字段");
    }

    let mut names = BTreeSet::new();
    for column in &named_columns {
        if !names.insert(column.name.trim().to_ascii_lowercase()) {
            return Some("字段名不能重复");
        }
        if column.data_type.trim().is_empty() {
            return Some("字段类型不能为空");
        }
        if column.auto_increment
            && (!column.primary_key || !dameng_is_integer_type(&column.data_type))
        {
            return Some("自增字段必须是整数主键");
        }
        if column.auto_increment && !column.default_value.trim().is_empty() {
            return Some("达梦自增字段不能同时设置默认值");
        }
        if create_table_is_text_type(&column.data_type)
            && create_table_unquoted_string_default(&column.default_value)
        {
            return Some("字符串默认值需要用引号包裹");
        }
    }

    if create
        .columns
        .iter()
        .any(|column| column.primary_key && column.name.trim().is_empty())
    {
        return Some("主键字段名不能为空");
    }
    match &create.mode {
        CreateTableMode::Create if !create.triggers.is_empty() => {
            return Some("达梦建表向导暂不支持创建触发器");
        }
        CreateTableMode::Design { original, .. } => {
            if original.triggers != create.triggers {
                return Some("达梦设计表暂不支持修改触发器");
            }
            if original.partition_enabled != create.partition_enabled
                || original.partition_sql != create.partition_sql
                || original.partition_method != create.partition_method
                || original.partition_expression != create.partition_expression
            {
                return Some("达梦建表向导暂不支持分区");
            }
        }
        _ => {}
    }
    if !create.is_design() && create.partition_enabled {
        return Some("达梦建表向导暂不支持分区");
    }

    if let Some(message) = create_table_foreign_key_validation_error(create) {
        return Some(message);
    }
    if let Some(message) = create_table_check_validation_error(create) {
        return Some(message);
    }
    None
}

pub(crate) fn create_table_dameng_sql_preview(
    create: &CreateTableState,
) -> Result<String, String> {
    if let Some(message) = create_table_dameng_validation_error(create) {
        return Err(message.to_string());
    }
    Ok(create_table_dameng_statements(create)?.join("\n"))
}

fn create_table_dameng_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let table = dameng_qualified_table(create, create.table_name.trim());
    let mut lines = create
        .columns
        .iter()
        .filter_map(create_table_dameng_column_line)
        .collect::<Vec<_>>();
    if let Some(primary_key) = create_table_dameng_primary_key_line(&create.columns) {
        lines.push(primary_key);
    }
    lines.extend(create_table_foreign_key_lines(
        create,
        CreateTableSqlDialect::Dameng,
    )?);
    lines.extend(create_table_check_lines(
        create,
        CreateTableSqlDialect::Dameng,
    )?);

    let mut statements = vec![format!(
        "CREATE TABLE {table} (\n{}\n);",
        lines
            .into_iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join(",\n")
    )];

    let comment = create.comment.trim();
    if !comment.is_empty() {
        statements.push(format!(
            "COMMENT ON TABLE {table} IS {};",
            quote_dameng_string(comment)
        ));
    }
    for column in &create.columns {
        let name = column.name.trim();
        let column_comment = column.comment.trim();
        if name.is_empty() || column_comment.is_empty() {
            continue;
        }
        statements.push(format!(
            "COMMENT ON COLUMN {table}.{} IS {};",
            fluxdb_core::DamengDialect.quote_identifier(name),
            quote_dameng_string(column_comment)
        ));
    }
    statements.extend(create_table_dameng_index_statements(create)?);
    Ok(statements)
}

fn create_table_dameng_column_line(column: &CreateTableColumn) -> Option<String> {
    let name = column.name.trim();
    if name.is_empty() {
        return None;
    }
    let mut sql = format!(
        "{} {}",
        fluxdb_core::DamengDialect.quote_identifier(name),
        create_table_dameng_column_type(column)
    );
    if column.auto_increment {
        sql.push_str(" IDENTITY(1,1)");
    }
    if !column.nullable || column.primary_key {
        sql.push_str(" NOT NULL");
    }
    let default_value = column.default_value.trim();
    if !default_value.is_empty() {
        sql.push_str(" DEFAULT ");
        sql.push_str(default_value);
    }
    Some(sql)
}

fn create_table_dameng_column_type(column: &CreateTableColumn) -> String {
    let base = create_table_base_type(&column.data_type);
    let length = column.length.trim();
    let scale = column.scale.trim();
    let modifiers = if length.is_empty() || !dameng_supports_length(&base) {
        String::new()
    } else if dameng_supports_scale(&base) && !scale.is_empty() {
        format!("({length},{scale})")
    } else {
        format!("({length})")
    };
    if modifiers.is_empty() {
        base
    } else {
        format!("{base}{modifiers}")
    }
}

fn create_table_dameng_primary_key_line(columns: &[CreateTableColumn]) -> Option<String> {
    let parts = columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .map(|column| fluxdb_core::DamengDialect.quote_identifier(column.name.trim()))
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| format!("PRIMARY KEY ({})", parts.join(", ")))
}

fn create_table_dameng_index_statements(
    create: &CreateTableState,
) -> Result<Vec<String>, String> {
    let qualified = dameng_qualified_table(create, create.table_name.trim());
    let valid_columns = create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
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
            return Err(format!("达梦不支持 {index_type} 索引"));
        }
        if !create_table_normalized_index_method(&index.index_method).is_empty()
            && create_table_normalized_index_method(&index.index_method) != "BTREE"
        {
            return Err("达梦设计表暂不支持指定哈希索引".to_string());
        }

        let mut seen = BTreeSet::new();
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
                return Err("达梦索引不支持前缀长度".to_string());
            }
            let mut part = fluxdb_core::DamengDialect.quote_identifier(column_name);
            let sort_order = create_table_normalized_sort_order(&column.sort_order);
            if !sort_order.is_empty() {
                part.push(' ');
                part.push_str(sort_order);
            }
            parts.push(part);
        }
        let unique = if index_type == "UNIQUE" { "UNIQUE " } else { "" };
        statements.push(format!(
            "CREATE {unique}INDEX {} ON {qualified} ({});",
            fluxdb_core::DamengDialect.quote_identifier(name),
            parts.join(", ")
        ));
    }
    Ok(statements)
}

pub(crate) fn create_table_dameng_design_statements(
    create: &CreateTableState,
) -> Result<Vec<String>, String> {
    let CreateTableMode::Design {
        object,
        original,
        original_ddl,
        ..
    } = &create.mode
    else {
        return Ok(Vec::new());
    };
    if original.triggers != create.triggers {
        return Err("达梦设计表暂不支持修改触发器".to_string());
    }

    let mut statements = Vec::new();
    let schema = dameng_design_schema(create, object);
    let old_table = dameng_qualified_name(schema.as_deref(), object.name.trim());
    let new_table_name = create.table_name.trim();
    let table = dameng_qualified_name(schema.as_deref(), new_table_name);

    if !object.name.trim().eq_ignore_ascii_case(new_table_name) {
        statements.push(format!(
            "ALTER TABLE {old_table} RENAME TO {};",
            fluxdb_core::DamengDialect.quote_identifier(new_table_name)
        ));
    }

    let old_primary_key = create_table_primary_key_names(&original.columns);
    let new_primary_key = create_table_primary_key_names(&create.columns);
    let primary_key_changed = old_primary_key != new_primary_key;
    if primary_key_changed && !old_primary_key.is_empty() {
        if let Some(name) = dameng_primary_key_constraint_name(original_ddl.as_deref()) {
            statements.push(format!(
                "ALTER TABLE {table} DROP CONSTRAINT {};",
                fluxdb_core::DamengDialect.quote_identifier(&name)
            ));
        } else {
            statements.push(format!("ALTER TABLE {table} DROP PRIMARY KEY;"));
        }
    }

    for column in &original.columns {
        if !create.columns.iter().any(|current| current.id == column.id) {
            statements.push(format!(
                "ALTER TABLE {table} DROP COLUMN {};",
                fluxdb_core::DamengDialect.quote_identifier(column.name.trim())
            ));
        }
    }
    for column in &create.columns {
        match original
            .columns
            .iter()
            .find(|original| original.id == column.id)
        {
            None => {
                if let Some(line) = create_table_dameng_column_line(column) {
                    statements.push(format!("ALTER TABLE {table} ADD COLUMN {line};"));
                }
            }
            Some(previous) if previous != column => {
                statements.extend(dameng_column_change_statements(
                    &table,
                    previous,
                    column,
                )?);
            }
            Some(_) => {}
        }
    }

    for index in &original.indexes {
        if !create
            .indexes
            .iter()
            .any(|current| current.id == index.id && current == index)
            && !index.name.trim().is_empty()
        {
            statements.push(format!(
                "DROP INDEX {};",
                dameng_qualified_name(schema.as_deref(), index.name.trim())
            ));
        }
    }
    let mut create_with_changed_indexes = create.clone();
    create_with_changed_indexes.indexes = create
        .indexes
        .iter()
        .filter(|index| {
            !original
                .indexes
                .iter()
                .any(|previous| previous.id == index.id && previous == *index)
        })
        .cloned()
        .collect();
    statements.extend(create_table_dameng_index_statements(
        &create_with_changed_indexes,
    )?);

    if primary_key_changed
        && let Some(line) = create_table_dameng_primary_key_line(&create.columns)
    {
        statements.push(format!("ALTER TABLE {table} ADD {line};"));
    }

    for foreign_key in &original.foreign_keys {
        let unchanged = create
            .foreign_keys
            .iter()
            .any(|current| current.id == foreign_key.id && current == foreign_key);
        if !unchanged && !foreign_key.name.trim().is_empty() {
            statements.push(format!(
                "ALTER TABLE {table} DROP CONSTRAINT {};",
                fluxdb_core::DamengDialect.quote_identifier(foreign_key.name.trim())
            ));
        }
    }
    statements.extend(
        create_table_foreign_key_lines(
            &create_table_with_foreign_keys(
                create,
                create_table_changed_items(&original.foreign_keys, &create.foreign_keys),
            ),
            CreateTableSqlDialect::Dameng,
        )?
        .into_iter()
        .map(|line| format!("ALTER TABLE {table} ADD {line};")),
    );

    for check in &original.checks {
        let unchanged = create
            .checks
            .iter()
            .any(|current| current.id == check.id && current == check);
        if !unchanged && !check.name.trim().is_empty() {
            statements.push(format!(
                "ALTER TABLE {table} DROP CONSTRAINT {};",
                fluxdb_core::DamengDialect.quote_identifier(check.name.trim())
            ));
        }
    }
    statements.extend(
        create_table_check_lines(
            &create_table_with_checks(
                create,
                create_table_changed_items(&original.checks, &create.checks),
            ),
            CreateTableSqlDialect::Dameng,
        )?
        .into_iter()
        .map(|line| format!("ALTER TABLE {table} ADD {line};")),
    );

    if create.comment.trim() != original.comment.trim() {
        statements.push(format!(
            "COMMENT ON TABLE {table} IS {};",
            quote_dameng_string(create.comment.trim())
        ));
    }
    Ok(statements)
}

fn dameng_column_change_statements(
    table: &str,
    previous: &CreateTableColumn,
    current: &CreateTableColumn,
) -> Result<Vec<String>, String> {
    let mut statements = Vec::new();
    let column = fluxdb_core::DamengDialect.quote_identifier(current.name.trim());
    if !previous.name.trim().eq_ignore_ascii_case(current.name.trim()) {
        statements.push(format!(
            "ALTER TABLE {table} RENAME COLUMN {} TO {column};",
            fluxdb_core::DamengDialect.quote_identifier(previous.name.trim())
        ));
    }

    let definition_changed = previous.data_type != current.data_type
        || previous.length != current.length
        || previous.scale != current.scale
        || previous.nullable != current.nullable
        || previous.primary_key != current.primary_key
        || previous.auto_increment != current.auto_increment;
    if definition_changed {
        if previous.auto_increment && !current.auto_increment {
            return Err("达梦设计表暂不支持移除自增属性".to_string());
        }
        let line = create_table_dameng_column_line(current)
            .ok_or_else(|| "字段名不能为空".to_string())?;
        statements.push(format!("ALTER TABLE {table} MODIFY {line};"));
    }
    // 默认值单独修改，避免仅清除默认值时重写类型、可空性或已有自增定义。
    if previous.default_value.trim() != current.default_value.trim() {
        let default_value = current.default_value.trim();
        let action = if default_value.is_empty() {
            "DROP DEFAULT".to_string()
        } else {
            format!("SET DEFAULT {default_value}")
        };
        statements.push(format!("ALTER TABLE {table} ALTER COLUMN {column} {action};"));
    }
    if previous.comment.trim() != current.comment.trim() {
        statements.push(format!(
            "COMMENT ON COLUMN {table}.{column} IS {};",
            quote_dameng_string(current.comment.trim())
        ));
    }
    Ok(statements)
}

fn dameng_design_schema(create: &CreateTableState, object: &ObjectPath) -> Option<String> {
    let schema = create.schema.trim();
    if !schema.is_empty() {
        return Some(schema.to_string());
    }
    object
        .schema
        .as_deref()
        .map(str::trim)
        .filter(|schema| !schema.is_empty())
        .map(str::to_string)
}

fn dameng_qualified_name(schema: Option<&str>, table: &str) -> String {
    match schema {
        Some(schema) => format!(
            "{}.{}",
            fluxdb_core::DamengDialect.quote_identifier(schema),
            fluxdb_core::DamengDialect.quote_identifier(table)
        ),
        None => fluxdb_core::DamengDialect.quote_identifier(table),
    }
}

fn dameng_qualified_table(create: &CreateTableState, table: &str) -> String {
    let schema = create.schema.trim();
    dameng_qualified_name((!schema.is_empty()).then_some(schema), table)
}

fn dameng_is_integer_type(data_type: &str) -> bool {
    matches!(
        create_table_base_type(data_type).as_str(),
        "tinyint" | "smallint" | "int" | "integer" | "bigint"
    )
}

fn dameng_supports_length(data_type: &str) -> bool {
    matches!(
        data_type,
        "char"
            | "varchar"
            | "varchar2"
            | "nchar"
            | "nvarchar"
            | "nvarchar2"
            | "binary"
            | "varbinary"
            | "number"
            | "numeric"
            | "decimal"
    )
}

fn dameng_supports_scale(data_type: &str) -> bool {
    matches!(data_type, "number" | "numeric" | "decimal")
}

fn quote_dameng_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn dameng_primary_key_constraint_name(ddl: Option<&str>) -> Option<String> {
    let ddl = ddl?;
    let upper = ddl.to_ascii_uppercase();
    let mut search_from = 0usize;
    while let Some(offset) = upper[search_from..].find("CONSTRAINT") {
        let start = search_from + offset + "CONSTRAINT".len();
        let rest = &ddl[start..];
        let trimmed = rest.trim_start();
        let quote = trimmed.chars().next().filter(|ch| matches!(ch, '"' | '`'))?;
        let end = trimmed[1..].find(quote)? + 2;
        let name = &trimmed[1..end - 1];
        let after = trimmed[end..].trim_start();
        if after.to_ascii_uppercase().starts_with("PRIMARY KEY") {
            return Some(name.to_string());
        }
        search_from = start;
    }
    None
}

fn create_table_apply_dameng_design_ddl(create: &mut CreateTableState, ddl: Option<&str>) {
    let Some(ddl) = ddl else {
        return;
    };
    for entry in create_table_ddl_entries(ddl) {
        let Some(name) = dameng_ddl_column_name(&entry) else {
            continue;
        };
        let Some(column) = create
            .columns
            .iter_mut()
            .find(|column| column.name.eq_ignore_ascii_case(&name))
        else {
            continue;
        };
        let upper = entry.to_ascii_uppercase();
        column.auto_increment = upper.contains("IDENTITY") || upper.contains("AUTO_INCREMENT");
        if let Some(index) = upper.find(" DEFAULT ") {
            let default_value = entry[index + " DEFAULT ".len()..].trim();
            if !default_value.is_empty() && !default_value.eq_ignore_ascii_case("NULL") {
                column.default_value = default_value.to_string();
            }
        }
    }
}

fn dameng_ddl_column_name(entry: &str) -> Option<String> {
    let entry = entry.trim();
    let upper = entry.to_ascii_uppercase();
    if ["CONSTRAINT", "PRIMARY", "FOREIGN", "CHECK", "UNIQUE"]
        .iter()
        .any(|keyword| upper.starts_with(keyword))
    {
        return None;
    }
    let first = entry.chars().next()?;
    if matches!(first, '"' | '`' | '\'') {
        let end = entry[1..].find(first)? + 2;
        return Some(entry[1..end - 1].replace(&format!("{first}{first}"), &first.to_string()));
    }
    entry
        .split_whitespace()
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod dameng_create_table_tests {
    use super::*;
    use fluxdb_core::ConnectionId;

    fn dameng_state() -> CreateTableState {
        let mut create =
            CreateTableState::new(ConnectionId(1), Some("DAMENG".into()), DatabaseKind::Dameng);
        create.schema = "APPS".into();
        create.table_name = "ORDERS".into();
        create.columns[0].name = "ID".into();
        create.columns[0].data_type = "int".into();
        create.columns[0].auto_increment = true;
        create.columns[0].nullable = false;
        create.add_column();
        let name = create.columns.last_mut().expect("name column");
        name.name = "NAME".into();
        name.data_type = "varchar".into();
        name.length = "50".into();
        name.comment = "订单名".into();
        create
    }

    #[test]
    fn dameng_create_table_uses_identity_comment_and_schema() {
        let create = dameng_state();
        let sql = create_table_dameng_sql_preview(&create).expect("DM create SQL");

        assert!(sql.contains("CREATE TABLE \"APPS\".\"ORDERS\""));
        assert!(sql.contains("\"ID\" int IDENTITY(1,1) NOT NULL"));
        assert!(sql.contains("\"NAME\" varchar(50)"));
        assert!(sql.contains("PRIMARY KEY (\"ID\")"));
        assert!(sql.contains("COMMENT ON COLUMN \"APPS\".\"ORDERS\".\"NAME\" IS '订单名';"));
    }

    #[test]
    fn dameng_design_preserves_loaded_default_and_adds_column() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("DAMENG".into()),
            schema: Some("APPS".into()),
            name: "ORDERS".into(),
            kind: ObjectKind::Table,
        };
        let columns = vec![
            CompletionColumn {
                database: object.database.clone(),
                schema: object.schema.clone(),
                table: object.name.clone(),
                name: "ID".into(),
                type_name: Some("INT".into()),
                nullable: false,
                primary_key: true,
                comment: None,
                stable: None,
            },
            CompletionColumn {
                database: object.database.clone(),
                schema: object.schema.clone(),
                table: object.name.clone(),
                name: "NAME".into(),
                type_name: Some("VARCHAR(50)".into()),
                nullable: false,
                primary_key: false,
                comment: Some("订单名".into()),
                stable: None,
            },
        ];
        let ddl = "CREATE TABLE \"APPS\".\"ORDERS\" (\n  \"ID\" INT IDENTITY(1,1) NOT NULL,\n  \"NAME\" VARCHAR(50) NOT NULL DEFAULT 'new',\n  PRIMARY KEY (\"ID\")\n);";
        let mut create =
            CreateTableState::design(object, DatabaseKind::Dameng, columns, Vec::new(), Vec::new(), Vec::new(), Some(ddl.into()));

        assert!(create.columns[0].auto_increment);
        assert_eq!(create.columns[1].default_value, "'new'");

        create.add_column();
        let note = create.columns.last_mut().expect("note column");
        note.name = "NOTE".into();
        note.data_type = "varchar".into();
        note.length = "200".into();
        create.table_name = "ORDERS_2".into();

        let statements = create_table_dameng_design_statements(&create).expect("DM design SQL");
        assert_eq!(
            statements[0],
            "ALTER TABLE \"APPS\".\"ORDERS\" RENAME TO \"ORDERS_2\";"
        );
        assert!(statements.iter().any(|sql| {
            sql == "ALTER TABLE \"APPS\".\"ORDERS_2\" ADD COLUMN \"NOTE\" varchar(200);"
        }));
    }

    #[test]
    fn dameng_default_changes_are_in_place_and_identity_removal_is_rejected() {
        let mut create = dameng_state();
        create.columns[1].default_value = "'old'".into();
        create.columns[1].nullable = false;
        create.triggers.push(CreateTableTrigger {
            id: 1,
            name: "EXISTING_TRIGGER".into(),
            timing: "BEFORE".into(),
            event: "INSERT".into(),
            body: "BEGIN NULL; END".into(),
        });
        create.mode = CreateTableMode::Design {
            object: ObjectPath {
                connection_id: create.connection_id,
                database: create.database.clone(),
                schema: Some(create.schema.clone()),
                name: create.table_name.clone(),
                kind: ObjectKind::Table,
            },
            original: create.design_snapshot(),
            original_ddl: None,
        };

        for (value, expected) in [
            ("", "DROP DEFAULT"),
            ("'replacement'", "SET DEFAULT 'replacement'"),
            ("NULL", "SET DEFAULT NULL"),
        ] {
            create.columns[1].default_value = value.into();
            assert_eq!(
                create.sql_preview().expect("default change SQL"),
                format!("ALTER TABLE \"APPS\".\"ORDERS\" ALTER COLUMN \"NAME\" {expected};")
            );
        }
        create.columns[1].default_value = "'old'".into();
        create.columns[0].auto_increment = false;
        assert_eq!(
            create.sql_preview().expect_err("reject identity removal"),
            "达梦设计表暂不支持移除自增属性"
        );
        create.columns[0].auto_increment = true;
        create.triggers.clear();
        assert_eq!(create.validation_error(), Some("达梦设计表暂不支持修改触发器"));
        if let CreateTableMode::Design { original, .. } = &create.mode {
            create.triggers = original.triggers.clone();
        }
        create.partition_enabled = true;
        assert_eq!(create.validation_error(), Some("达梦建表向导暂不支持分区"));
    }

    #[test]
    fn design_details_do_not_overwrite_user_edits_or_original_columns() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("DAMENG".into()),
            schema: Some("APPS".into()),
            name: "ORDERS".into(),
            kind: ObjectKind::Table,
        };
        let columns = vec![CompletionColumn {
            database: Some("DAMENG".into()),
            schema: Some("APPS".into()),
            table: "ORDERS".into(),
            name: "ID".into(),
            type_name: Some("INT".into()),
            nullable: false,
            primary_key: true,
            comment: None,
            stable: None,
        }];
        let ddl = "CREATE TABLE \"APPS\".\"ORDERS\" (\"ID\" INT PRIMARY KEY);";
        let mut create = CreateTableState::design(
            object,
            DatabaseKind::Dameng,
            columns,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(ddl.into()),
        );
        create.details_loading = true;
        assert_eq!(
            create.validation_error(),
            Some("正在读取索引、外键和触发器，请稍候再保存")
        );
        create.columns[0].comment = "edited".into();
        let metadata = fluxdb_core::TableDesignMetadata {
            columns: Vec::new(),
            indexes: vec![IndexInfo {
                name: "IDX_ORDERS_ID".into(),
                columns: vec!["ID".into()],
                is_unique: false,
                is_primary: false,
                index_type: Some("NORMAL".into()),
                comment: None,
            }],
            foreign_keys: Vec::new(),
            triggers: Vec::new(),
            column_defaults: Default::default(),
            auto_increment_columns: Default::default(),
            generated_columns: Default::default(),
            index_attributes: Default::default(),
            foreign_key_actions: Default::default(),
            checks: Vec::new(),
            checks_loaded: false,
            table_comment: None,
            ddl: "CREATE TABLE \"APPS\".\"ORDERS\" (\"ID\" INT PRIMARY KEY); \
                  CREATE INDEX \"IDX_ORDERS_ID\" ON \"APPS\".\"ORDERS\" (\"ID\");"
                .into(),
            details_loaded: true,
        };
        create.finish_design_details(metadata).expect("finish details");
        assert_eq!(create.columns[0].comment, "edited");
        assert!(!create.details_loading);
        assert_eq!(create.indexes.len(), 1);
        let sql = create.sql_preview().expect("detail change SQL");
        assert!(sql.contains("COMMENT ON COLUMN"));
        assert!(!sql.contains("CREATE INDEX"));
        create.indexes.clear();
        assert!(
            create
                .sql_preview()
                .expect("drop detail index")
                .contains("DROP INDEX \"APPS\".\"IDX_ORDERS_ID\"")
        );
        let CreateTableMode::Design { original, .. } = &create.mode else {
            panic!("design mode");
        };
        assert_eq!(original.columns[0].comment, "");
    }
}
