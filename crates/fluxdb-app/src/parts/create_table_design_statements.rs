fn create_table_mysql_design_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let CreateTableMode::Design {
        object, original, ..
    } = &create.mode
    else {
        return Ok(Vec::new());
    };
    let mut statements = Vec::new();
    let old_table_name = object.name.trim();
    let new_table_name = create.table_name.trim();
    if !old_table_name.eq_ignore_ascii_case(new_table_name) {
        statements.push(format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_mysql_identifier(old_table_name),
            quote_mysql_identifier(new_table_name)
        ));
    }
    let table_name = quote_mysql_identifier(new_table_name);
    let old_primary_key = create_table_primary_key_names(&original.columns);
    let new_primary_key = create_table_primary_key_names(&create.columns);
    if old_primary_key != new_primary_key && !old_primary_key.is_empty() {
        statements.push(format!("ALTER TABLE {table_name} DROP PRIMARY KEY;"));
    }
    for column in &original.columns {
        if !create.columns.iter().any(|current| current.id == column.id) {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP COLUMN {};",
                quote_mysql_identifier(column.name.trim())
            ));
        }
    }
    for column in &create.columns {
        let Some(line) = column.sql_line() else {
            continue;
        };
        match original.columns.iter().find(|original| original.id == column.id) {
            Some(original) if original != column => statements.push(format!(
                "ALTER TABLE {table_name} CHANGE COLUMN {} {line};",
                quote_mysql_identifier(original.name.trim())
            )),
            None => statements.push(format!("ALTER TABLE {table_name} ADD COLUMN {line};")),
            _ => {}
        }
    }

    statements.extend(create_table_mysql_drop_missing_indexes(create, original, &table_name));
    if old_primary_key != new_primary_key {
        if let Some(line) = create_table_primary_key_line(&create.columns) {
            statements.push(format!("ALTER TABLE {table_name} ADD {line};"));
        }
    }

    statements.extend(
        create_table_index_lines(&create_table_with_indexes(
            create,
            create_table_changed_items(&original.indexes, &create.indexes),
        ))?
            .into_iter()
            .map(|line| format!("ALTER TABLE {table_name} ADD {line};")),
    );
    for foreign_key in &original.foreign_keys {
        if !create
            .foreign_keys
            .iter()
            .any(|current| current.id == foreign_key.id && current == foreign_key)
            && !foreign_key.name.trim().is_empty()
        {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP FOREIGN KEY {};",
                quote_mysql_identifier(foreign_key.name.trim())
            ));
        }
    }
    statements.extend(
        create_table_foreign_key_lines(
            &create_table_with_foreign_keys(
                create,
                create_table_changed_items(&original.foreign_keys, &create.foreign_keys),
            ),
            CreateTableSqlDialect::MySql,
        )?
            .into_iter()
            .map(|line| format!("ALTER TABLE {table_name} ADD {line};")),
    );
    for check in &original.checks {
        if !create
            .checks
            .iter()
            .any(|current| current.id == check.id && current == check)
            && !check.name.trim().is_empty()
        {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP CHECK {};",
                quote_mysql_identifier(check.name.trim())
            ));
        }
    }
    statements.extend(
        create_table_check_lines(
            &create_table_with_checks(
                create,
                create_table_changed_items(&original.checks, &create.checks),
            ),
            CreateTableSqlDialect::MySql,
        )?
            .into_iter()
            .map(|line| format!("ALTER TABLE {table_name} ADD {line};")),
    );
    for trigger in &original.triggers {
        if !create
            .triggers
            .iter()
            .any(|current| current.id == trigger.id && current == trigger)
        {
            statements.push(format!(
                "DROP TRIGGER {};",
                quote_mysql_identifier(trigger.name.trim())
            ));
        }
    }
    statements.extend(create_table_mysql_trigger_statements(
        &create_table_with_triggers(
            create,
            create_table_changed_items(&original.triggers, &create.triggers),
        ),
    )?);
    if create.comment != original.comment {
        statements.push(format!(
            "ALTER TABLE {table_name} COMMENT={};",
            quote_mysql_string(create.comment.trim())
        ));
    }
    if create_table_options_changed(create, original) {
        let options = create_table_mysql_table_options(create);
        if !options.is_empty() {
            statements.push(format!("ALTER TABLE {table_name} {};", options.join(" ")));
        }
    }
    if create.partition_sql != original.partition_sql || create.partition_enabled != original.partition_enabled {
        let partition_sql = create_table_mysql_partition_sql(create);
        if !partition_sql.is_empty() {
            statements.push(format!("ALTER TABLE {table_name} {partition_sql};"));
        }
    }
    Ok(statements)
}

fn create_table_sqlite_design_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
    let CreateTableMode::Design {
        object, original, ..
    } = &create.mode
    else {
        return Ok(Vec::new());
    };
    if create_table_sqlite_design_needs_rebuild(create, original) {
        return create_table_sqlite_rebuild_design_statements(create, original, &object.name);
    }

    let mut statements = Vec::new();
    let old_table_name = object.name.trim();
    let new_table_name = create.table_name.trim();
    if !old_table_name.eq_ignore_ascii_case(new_table_name) {
        statements.push(format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_sqlite_identifier(old_table_name),
            quote_sqlite_identifier(new_table_name)
        ));
    }
    let table_name = quote_sqlite_identifier(new_table_name);
    for column in &original.columns {
        if !create.columns.iter().any(|current| current.id == column.id) {
            statements.push(format!(
                "ALTER TABLE {table_name} DROP COLUMN {};",
                quote_sqlite_identifier(column.name.trim())
            ));
        }
    }
    for column in &create.columns {
        match original.columns.iter().find(|original| original.id == column.id) {
            Some(original) if original != column => {
                if create_table_columns_equal_except_name(original, column) {
                    statements.push(format!(
                        "ALTER TABLE {table_name} RENAME COLUMN {} TO {};",
                        quote_sqlite_identifier(original.name.trim()),
                        quote_sqlite_identifier(column.name.trim())
                    ));
                } else {
                    return Err("SQLite 暂不支持通过设计表修改已有字段类型或约束".to_string());
                }
            }
            None => {
                if let Some(line) = create_table_sqlite_column_line(column, false) {
                    statements.push(format!("ALTER TABLE {table_name} ADD COLUMN {line};"));
                }
            }
            _ => {}
        }
    }
    for index in &original.indexes {
        if !create
            .indexes
            .iter()
            .any(|current| current.id == index.id && current == index)
        {
            statements.push(format!(
                "DROP INDEX {};",
                quote_sqlite_identifier(index.name.trim())
            ));
        }
    }
    statements.extend(create_table_sqlite_index_statements(
        &create_table_with_indexes(
            create,
            create_table_changed_items(&original.indexes, &create.indexes),
        ),
    )?);
    for trigger in &original.triggers {
        if !create
            .triggers
            .iter()
            .any(|current| current.id == trigger.id && current == trigger)
        {
            statements.push(format!(
                "DROP TRIGGER {};",
                quote_sqlite_identifier(trigger.name.trim())
            ));
        }
    }
    statements.extend(create_table_sqlite_trigger_statements(
        &create_table_with_triggers(
            create,
            create_table_changed_items(&original.triggers, &create.triggers),
        ),
    )?);
    Ok(statements)
}

fn create_table_sqlite_design_needs_rebuild(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
) -> bool {
    if original.foreign_keys != create.foreign_keys || original.checks != create.checks {
        return true;
    }

    let original_column_ids = original
        .columns
        .iter()
        .map(|column| column.id)
        .collect::<Vec<_>>();
    let current_original_column_ids = create
        .columns
        .iter()
        .filter(|column| original.columns.iter().any(|original| original.id == column.id))
        .map(|column| column.id)
        .collect::<Vec<_>>();
    if original_column_ids != current_original_column_ids {
        return true;
    }

    create.columns.iter().any(|column| {
        original
            .columns
            .iter()
            .find(|original| original.id == column.id)
            .is_some_and(|original| {
                original != column && !create_table_columns_equal_except_name(original, column)
            })
            || (!original.columns.iter().any(|original| original.id == column.id)
                && (column.primary_key || column.auto_increment))
    })
}

fn create_table_sqlite_rebuild_design_statements(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
    old_table_name: &str,
) -> Result<Vec<String>, String> {
    let new_table_name = create.table_name.trim();
    // ponytail: 不查库探测临时表冲突；真遇到同名表时再改成基于 sqlite_schema 生成唯一名。
    let temp_table_name = create_table_sqlite_rebuild_temp_table_name(old_table_name, new_table_name);
    let mut statements = vec![
        "PRAGMA foreign_keys = OFF;".to_string(),
        "BEGIN TRANSACTION;".to_string(),
        format!(
            "ALTER TABLE {} RENAME TO {};",
            quote_sqlite_identifier(old_table_name),
            quote_sqlite_identifier(&temp_table_name)
        ),
        create_table_sqlite_create_table_statement(create, new_table_name)?,
    ];

    let copy_columns = create_table_sqlite_rebuild_copy_columns(create, original);
    if !copy_columns.is_empty() {
        let target_columns = copy_columns
            .iter()
            .map(|(_, current)| quote_sqlite_identifier(current.name.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let source_columns = copy_columns
            .iter()
            .map(|(original, _)| quote_sqlite_identifier(original.name.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        statements.push(format!(
            "INSERT INTO {} ({target_columns}) SELECT {source_columns} FROM {};",
            quote_sqlite_identifier(new_table_name),
            quote_sqlite_identifier(&temp_table_name)
        ));
    }

    statements.push(format!(
        "DROP TABLE {};",
        quote_sqlite_identifier(&temp_table_name)
    ));
    statements.extend(create_table_sqlite_index_statements(create)?);
    statements.extend(create_table_sqlite_trigger_statements(create)?);
    statements.push("COMMIT;".to_string());
    statements.push("PRAGMA foreign_keys = ON;".to_string());
    Ok(statements)
}

fn create_table_sqlite_rebuild_temp_table_name(old_table_name: &str, new_table_name: &str) -> String {
    let base = format!("__gdb_rebuild_{}", old_table_name.trim());
    if base.eq_ignore_ascii_case(new_table_name.trim()) {
        format!("{base}_old")
    } else {
        base
    }
}

fn create_table_sqlite_rebuild_copy_columns<'a>(
    create: &'a CreateTableState,
    original: &'a CreateTableDesignSnapshot,
) -> Vec<(&'a CreateTableColumn, &'a CreateTableColumn)> {
    create
        .columns
        .iter()
        .filter_map(|column| {
            original
                .columns
                .iter()
                .find(|original| original.id == column.id)
                .map(|original| (original, column))
        })
        .collect()
}

fn create_table_primary_key_names(columns: &[CreateTableColumn]) -> Vec<String> {
    columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .map(|column| column.name.trim().to_ascii_lowercase())
        .collect()
}

fn create_table_columns_equal_except_name(
    left: &CreateTableColumn,
    right: &CreateTableColumn,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.name = String::new();
    right.name = String::new();
    left == right
}

fn create_table_changed_items<T: Clone + PartialEq>(original: &[T], current: &[T]) -> Vec<T> {
    current
        .iter()
        .filter(|item| !original.iter().any(|original| original == *item))
        .cloned()
        .collect()
}

fn create_table_with_indexes(
    create: &CreateTableState,
    indexes: Vec<CreateTableIndex>,
) -> CreateTableState {
    let mut create = create.clone();
    create.indexes = indexes;
    create
}

fn create_table_with_foreign_keys(
    create: &CreateTableState,
    foreign_keys: Vec<CreateTableForeignKey>,
) -> CreateTableState {
    let mut create = create.clone();
    create.foreign_keys = foreign_keys;
    create
}

fn create_table_with_checks(
    create: &CreateTableState,
    checks: Vec<CreateTableCheck>,
) -> CreateTableState {
    let mut create = create.clone();
    create.checks = checks;
    create
}

fn create_table_with_triggers(
    create: &CreateTableState,
    triggers: Vec<CreateTableTrigger>,
) -> CreateTableState {
    let mut create = create.clone();
    create.triggers = triggers;
    create
}

fn create_table_mysql_drop_missing_indexes(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
    table_name: &str,
) -> Vec<String> {
    original
        .indexes
        .iter()
        .filter(|index| {
            !create
                .indexes
                .iter()
                .any(|current| current.id == index.id && current == *index)
        })
        .map(|index| {
            format!(
                "ALTER TABLE {table_name} DROP INDEX {};",
                quote_mysql_identifier(index.name.trim())
            )
        })
        .collect()
}

fn create_table_options_changed(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
) -> bool {
    create.engine != original.engine
        || create.tablespace != original.tablespace
        || create.charset != original.charset
        || create.collation != original.collation
        || create.row_format != original.row_format
        || create.avg_row_length != original.avg_row_length
        || create.max_rows != original.max_rows
        || create.min_rows != original.min_rows
        || create.key_block_size != original.key_block_size
}

fn create_table_sqlite_index_statements(create: &CreateTableState) -> Result<Vec<String>, String> {
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
            return Err("Index name is required.".to_string());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Duplicate index name: {name}."));
        }
        let index_type = create_table_normalized_index_type(&index.index_type);
        if matches!(index_type, "FULLTEXT" | "SPATIAL") {
            return Err(format!("SQLite does not support {index_type} indexes."));
        }

        let mut column_names = BTreeSet::new();
        let mut parts = Vec::with_capacity(index.columns.len());
        for column in &index.columns {
            let column_name = column.name.trim();
            if column_name.is_empty() {
                return Err(format!("Index {name} has an empty field."));
            }
            if !valid_columns.contains(&column_name.to_ascii_lowercase()) {
                return Err(format!("Index {name} references unknown field: {column_name}."));
            }
            if !column_names.insert(column_name.to_ascii_lowercase()) {
                return Err(format!("Index {name} has duplicate field: {column_name}."));
            }
            let mut part = quote_sqlite_identifier(column_name);
            let sort_order = create_table_normalized_sort_order(&column.sort_order);
            if !sort_order.is_empty() {
                part.push(' ');
                part.push_str(sort_order);
            }
            parts.push(part);
        }
        if parts.is_empty() {
            return Err(format!("Index {name} needs at least one field."));
        }

        let unique = if index_type == "UNIQUE" { "UNIQUE " } else { "" };
        statements.push(format!(
            "CREATE {unique}INDEX {} ON {} ({});",
            quote_sqlite_identifier(name),
            quote_sqlite_identifier(create.table_name.trim()),
            parts.join(", ")
        ));
    }

    Ok(statements)
}

fn create_table_mysql_validation_error(create: &CreateTableState) -> Option<&'static str> {
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
            && (!column.primary_key || !create_table_mysql_is_integer_type(&column.data_type))
        {
            return Some("自增字段必须是整数主键");
        }
        if create_table_mysql_is_text_type(&column.data_type)
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

    if let Some(message) = create_table_foreign_key_validation_error(create) {
        return Some(message);
    }
    if let Some(message) = create_table_check_validation_error(create) {
        return Some(message);
    }
    if let Some(message) = create_table_trigger_validation_error(create) {
        return Some(message);
    }
    if let Some(message) = create_table_partition_validation_error(create) {
        return Some(message);
    }

    None
}

impl CreateTableIndex {
    fn new(id: u64) -> Self {
        Self {
            id,
            name: String::new(),
            columns: Vec::new(),
            index_type: String::new(),
            index_method: String::new(),
            comment: String::new(),
        }
    }
}

impl CreateTableIndexColumn {
    fn new(name: String) -> Self {
        Self {
            name,
            sub_part: String::new(),
            sort_order: String::new(),
        }
    }
}

impl CreateTableCheck {
    fn new(id: u64) -> Self {
        Self {
            id,
            name: String::new(),
            expression: String::new(),
            not_enforced: false,
        }
    }
}

impl CreateTableTrigger {
    fn new(id: u64) -> Self {
        Self {
            id,
            name: String::new(),
            timing: "BEFORE".to_string(),
            event: "INSERT".to_string(),
            body: String::new(),
        }
    }
}

impl CreateTableTriggerEvent {
    fn as_sql(self) -> &'static str {
        match self {
            CreateTableTriggerEvent::Insert => "INSERT",
            CreateTableTriggerEvent::Update => "UPDATE",
            CreateTableTriggerEvent::Delete => "DELETE",
        }
    }
}

fn create_table_check_validation_error(create: &CreateTableState) -> Option<&'static str> {
    let mut names = BTreeSet::new();
    for check in &create.checks {
        let name = check.name.trim();
        if !name.is_empty() && !names.insert(name.to_ascii_lowercase()) {
            return Some("检查名称不能重复");
        }
        if check.expression.trim().is_empty() {
            return Some("检查表达式不能为空");
        }
        if check.not_enforced
            && create.database_kind != DatabaseKind::MySql
            && create.database_kind != DatabaseKind::TiDb
        {
            return Some("当前连接类型不支持不强制实施检查");
        }
    }
    None
}

fn create_table_check_lines(
    create: &CreateTableState,
    dialect: CreateTableSqlDialect,
) -> Result<Vec<String>, String> {
    let mut names = BTreeSet::new();
    let mut lines = Vec::with_capacity(create.checks.len());
    for check in &create.checks {
        let name = check.name.trim();
        if !name.is_empty() && !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Duplicate check name: {name}."));
        }
        let expression = check.expression.trim();
        if expression.is_empty() {
            return Err("Check expression is required.".to_string());
        }

        let mut line = if name.is_empty() {
            format!("CHECK ({expression})")
        } else {
            match dialect {
                CreateTableSqlDialect::MySql => {
                    format!(
                        "CONSTRAINT {} CHECK ({expression})",
                        quote_mysql_identifier(name)
                    )
                }
                CreateTableSqlDialect::Sqlite => {
                    format!(
                        "CONSTRAINT {} CHECK ({expression})",
                        quote_sqlite_identifier(name)
                    )
                }
            }
        };
        if check.not_enforced {
            match dialect {
                CreateTableSqlDialect::MySql => line.push_str(" NOT ENFORCED"),
                CreateTableSqlDialect::Sqlite => {
                    return Err("SQLite does not support NOT ENFORCED checks.".to_string());
                }
            }
        }
        lines.push(line);
    }
    Ok(lines)
}

fn create_table_trigger_validation_error(create: &CreateTableState) -> Option<&'static str> {
    let mut names = BTreeSet::new();
    for trigger in &create.triggers {
        if trigger.name.trim().is_empty() {
            return Some("触发器名称不能为空");
        }
        if !names.insert(trigger.name.trim().to_ascii_lowercase()) {
            return Some("触发器名称不能重复");
        }
        if !matches!(
            create_table_normalized_trigger_timing(&trigger.timing),
            "BEFORE" | "AFTER"
        ) {
            return Some("触发器触发时机无效");
        }
        if !matches!(
            create_table_normalized_trigger_event(&trigger.event),
            "INSERT" | "UPDATE" | "DELETE"
        ) {
            return Some("请选择触发器事件");
        }
        if trigger.body.trim().is_empty() {
            return Some("触发器定义不能为空");
        }
    }
    None
}

fn create_table_mysql_trigger_statements(
    create: &CreateTableState,
) -> Result<Vec<String>, String> {
    create_table_trigger_statements(create, CreateTableSqlDialect::MySql)
}

fn create_table_sqlite_trigger_statements(
    create: &CreateTableState,
) -> Result<Vec<String>, String> {
    create_table_trigger_statements(create, CreateTableSqlDialect::Sqlite)
}

fn create_table_trigger_statements(
    create: &CreateTableState,
    dialect: CreateTableSqlDialect,
) -> Result<Vec<String>, String> {
    let table_name = match dialect {
        CreateTableSqlDialect::MySql => quote_mysql_identifier(create.table_name.trim()),
        CreateTableSqlDialect::Sqlite => quote_sqlite_identifier(create.table_name.trim()),
    };
    create
        .triggers
        .iter()
        .map(|trigger| {
            let name = trigger.name.trim();
            let body = trigger.body.trim();
            if name.is_empty() {
                return Err("触发器名称不能为空".to_string());
            }
            if body.is_empty() {
                return Err("触发器定义不能为空".to_string());
            }
            let trigger_name = match dialect {
                CreateTableSqlDialect::MySql => quote_mysql_identifier(name),
                CreateTableSqlDialect::Sqlite => quote_sqlite_identifier(name),
            };
            Ok(format!(
                "CREATE TRIGGER {trigger_name}\n{} {} ON {table_name}\nFOR EACH ROW\n{body};",
                create_table_normalized_trigger_timing(&trigger.timing),
                create_table_normalized_trigger_event(&trigger.event)
            ))
        })
        .collect()
}

fn create_table_primary_key_line(columns: &[CreateTableColumn]) -> Option<String> {
    let parts = columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .map(|column| {
            let mut part = quote_mysql_identifier(column.name.trim());
            if column.supports_primary_key_prefix_length() && !column.key_length.trim().is_empty() {
                part.push('(');
                part.push_str(column.key_length.trim());
                part.push(')');
            }
            part
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        None
    } else {
        Some(format!("PRIMARY KEY ({})", parts.join(", ")))
    }
}

fn create_table_mysql_table_options(create: &CreateTableState) -> Vec<String> {
    let mut options = Vec::new();
    if !create.engine.trim().is_empty() {
        options.push(format!("ENGINE={}", create.engine.trim()));
    }
    if !create.tablespace.trim().is_empty() {
        options.push(format!(
            "TABLESPACE {}",
            quote_mysql_identifier(create.tablespace.trim())
        ));
    }
    if !create.charset.trim().is_empty() {
        options.push(format!("DEFAULT CHARSET={}", create.charset.trim()));
    }
    if !create.collation.trim().is_empty() {
        options.push(format!("COLLATE={}", create.collation.trim()));
    }
    if !create.row_format.trim().is_empty() {
        options.push(format!("ROW_FORMAT={}", create.row_format.trim()));
    }
    create_table_push_nonzero_option(&mut options, "AVG_ROW_LENGTH", &create.avg_row_length);
    create_table_push_nonzero_option(&mut options, "MAX_ROWS", &create.max_rows);
    create_table_push_nonzero_option(&mut options, "MIN_ROWS", &create.min_rows);
    create_table_push_nonzero_option(&mut options, "KEY_BLOCK_SIZE", &create.key_block_size);
    options
}

fn create_table_push_nonzero_option(options: &mut Vec<String>, name: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() && value != "0" {
        options.push(format!("{name}={value}"));
    }
}

fn create_table_mysql_partition_sql(create: &CreateTableState) -> String {
    if !create.partition_enabled {
        return String::new();
    }
    create.partition_sql.trim().trim_end_matches(';').to_string()
}

fn create_table_partition_validation_error(create: &CreateTableState) -> Option<&'static str> {
    if !create.partition_enabled {
        return None;
    }
    if !matches!(create.database_kind, DatabaseKind::MySql | DatabaseKind::TiDb) {
        return Some("当前连接类型不支持分区");
    }
    let sql = create.partition_sql.trim();
    if sql.is_empty() {
        return Some("请填写分区定义 SQL");
    }
    if !sql.to_ascii_uppercase().starts_with("PARTITION BY ") {
        return Some("分区定义需要以 PARTITION BY 开头");
    }
    None
}

fn create_table_partition_template(method: &str, expression: &str) -> String {
    match create_table_normalized_partition_method(method) {
        "LIST" => format!("PARTITION BY LIST ({expression}) (\n  PARTITION p0 VALUES IN (0)\n)"),
        "HASH" => format!("PARTITION BY HASH ({expression})\nPARTITIONS 4"),
        "KEY" => format!("PARTITION BY KEY ({expression})\nPARTITIONS 4"),
        _ => format!("PARTITION BY RANGE ({expression}) (\n  PARTITION p0 VALUES LESS THAN (MAXVALUE)\n)"),
    }
}

fn create_table_normalized_partition_method(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "LIST" => "LIST",
        "HASH" => "HASH",
        "KEY" => "KEY",
        _ => "RANGE",
    }
}

fn create_table_digits_or_zero(value: &str) -> String {
    let digits = create_table_digits_only(value);
    if digits.is_empty() {
        "0".to_string()
    } else {
        digits
    }
}

fn create_table_normalized_table_engine(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "INNODB" => "InnoDB",
        "MYISAM" => "MyISAM",
        "MEMORY" => "MEMORY",
        "CSV" => "CSV",
        "ARCHIVE" => "ARCHIVE",
        "BLACKHOLE" => "BLACKHOLE",
        "FEDERATED" => "FEDERATED",
        _ => "",
    }
}

fn create_table_normalized_row_format(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "DEFAULT" => "DEFAULT",
        "DYNAMIC" => "DYNAMIC",
        "FIXED" => "FIXED",
        "COMPRESSED" => "COMPRESSED",
        "REDUNDANT" => "REDUNDANT",
        "COMPACT" => "COMPACT",
        _ => "",
    }
}

fn create_table_index_lines(create: &CreateTableState) -> Result<Vec<String>, String> {
    let valid_columns = create
        .columns
        .iter()
        .map(|column| column.name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    let mut lines = Vec::with_capacity(create.indexes.len());

    for index in &create.indexes {
        let name = index.name.trim();
        if name.is_empty() {
            return Err("Index name is required.".to_string());
        }
        if name.eq_ignore_ascii_case("PRIMARY") {
            return Err("Index name cannot be PRIMARY.".to_string());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Duplicate index name: {name}."));
        }
        if index.columns.is_empty() {
            return Err(format!("Index {name} needs at least one field."));
        }

        let mut column_names = BTreeSet::new();
        let mut parts = Vec::with_capacity(index.columns.len());
        for column in &index.columns {
            let column_name = column.name.trim();
            if column_name.is_empty() {
                return Err(format!("Index {name} has an empty field."));
            }
            if !valid_columns.contains(&column_name.to_ascii_lowercase()) {
                return Err(format!("Index {name} references unknown field: {column_name}."));
            }
            if !column_names.insert(column_name.to_ascii_lowercase()) {
                return Err(format!("Index {name} has duplicate field: {column_name}."));
            }
            if !column.sub_part.trim().chars().all(|ch| ch.is_ascii_digit()) {
                return Err(format!("Index {name} field {column_name} has invalid sub part."));
            }
            let mut part = quote_mysql_identifier(column_name);
            if !column.sub_part.trim().is_empty() {
                part.push('(');
                part.push_str(column.sub_part.trim());
                part.push(')');
            }
            let sort_order = create_table_normalized_sort_order(&column.sort_order);
            if !sort_order.is_empty() {
                part.push(' ');
                part.push_str(sort_order);
            }
            parts.push(part);
        }

        let keyword = match create_table_normalized_index_type(&index.index_type) {
            "UNIQUE" => "UNIQUE KEY",
            "FULLTEXT" => "FULLTEXT KEY",
            "SPATIAL" => "SPATIAL KEY",
            _ => "KEY",
        };
        let mut line = format!("{keyword} {}", quote_mysql_identifier(name));
        let method = create_table_normalized_index_method(&index.index_method);
        if !method.is_empty() && matches!(keyword, "KEY" | "UNIQUE KEY") {
            line.push_str(" USING ");
            line.push_str(method);
        }
        line.push_str(" (");
        line.push_str(&parts.join(", "));
        line.push(')');
        if !index.comment.trim().is_empty() {
            line.push_str(" COMMENT ");
            line.push_str(&quote_mysql_string(index.comment.trim()));
        }
        lines.push(line);
    }

    Ok(lines)
}

fn create_table_normalized_index_type(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "UNIQUE" => "UNIQUE",
        "FULLTEXT" => "FULLTEXT",
        "SPATIAL" => "SPATIAL",
        _ => "NORMAL",
    }
}

fn create_table_normalized_index_method(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "HASH" => "HASH",
        "" => "",
        _ => "BTREE",
    }
}

fn create_table_index_type_supports_method(value: &str) -> bool {
    !matches!(
        create_table_normalized_index_type(value),
        "FULLTEXT" | "SPATIAL"
    )
}

fn create_table_normalized_sort_order(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "ASC" => "ASC",
        "DESC" => "DESC",
        "" => "",
        _ => "ASC",
    }
}

fn create_table_normalized_trigger_timing(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "AFTER" => "AFTER",
        _ => "BEFORE",
    }
}

fn create_table_normalized_trigger_event(value: &str) -> &'static str {
    match value.trim().to_ascii_uppercase().as_str() {
        "UPDATE" => "UPDATE",
        "DELETE" => "DELETE",
        _ => "INSERT",
    }
}

fn create_table_auto_index_name<'a>(
    table_name: &str,
    index_type: &str,
    columns: impl Iterator<Item = &'a str>,
) -> String {
    let prefix = match create_table_normalized_index_type(index_type) {
        "UNIQUE" => "uk",
        "FULLTEXT" => "ft",
        "SPATIAL" => "sp",
        _ => "idx",
    };
    let columns = columns.collect::<Vec<_>>();
    if columns.is_empty() {
        return String::new();
    }
    let mut name = format!("{prefix}_{table_name}_{}", columns.join("_"));
    if name.len() > 64 {
        name.truncate(64);
    }
    name
}

fn create_table_digits_only(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_digit()).collect()
}

fn create_table_column_type(data_type: &str, length: &str, scale: &str) -> String {
    let data_type = data_type.trim();
    if data_type.is_empty() {
        return "varchar(255)".to_string();
    }
    if !create_table_supports_length(data_type) || data_type.contains('(') {
        return data_type.to_string();
    }
    let length = length.trim();
    let scale = scale.trim();
    if !length.is_empty() && !scale.is_empty() && create_table_supports_scale(data_type) {
        return format!("{data_type}({length},{scale})");
    }
    if length.is_empty() {
        data_type.to_string()
    } else {
        format!("{data_type}({length})")
    }
}

fn create_table_split_column_type(raw_type: &str) -> (String, String, String) {
    let raw_type = raw_type.trim();
    let Some((base, rest)) = raw_type.split_once('(') else {
        return (raw_type.to_ascii_lowercase(), String::new(), String::new());
    };
    let Some((args, _)) = rest.split_once(')') else {
        return (raw_type.to_ascii_lowercase(), String::new(), String::new());
    };
    let mut parts = args.split(',').map(|part| part.trim().to_string());
    (
        base.trim().to_ascii_lowercase(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    )
}

fn create_table_supports_length(data_type: &str) -> bool {
    create_table_is_char_length_type(data_type)
        || create_table_is_binary_length_type(data_type)
        || create_table_is_decimal_type(data_type)
}

fn create_table_default_length(data_type: &str) -> Option<&'static str> {
    if create_table_is_char_length_type(data_type) || create_table_is_binary_length_type(data_type) {
        Some("255")
    } else if create_table_is_decimal_type(data_type) {
        Some("10")
    } else {
        None
    }
}

fn create_table_supports_scale(data_type: &str) -> bool {
    create_table_is_decimal_type(data_type)
}

fn create_table_is_char_length_type(data_type: &str) -> bool {
    let data_type = data_type.to_ascii_lowercase();
    data_type.contains("varchar") || data_type == "char"
}

fn create_table_is_decimal_type(data_type: &str) -> bool {
    data_type.to_ascii_lowercase().contains("decimal")
}

fn create_table_is_text_type(data_type: &str) -> bool {
    let data_type = create_table_base_type(data_type);
    matches!(data_type.as_str(), "char" | "varchar") || data_type.ends_with("text")
}

fn create_table_is_binary_length_type(data_type: &str) -> bool {
    let data_type = create_table_base_type(data_type);
    matches!(data_type.as_str(), "binary" | "varbinary")
}

fn create_table_supports_key_length(data_type: &str) -> bool {
    let data_type = create_table_base_type(data_type);
    matches!(data_type.as_str(), "char" | "varchar" | "binary" | "varbinary")
        || data_type.ends_with("text")
        || data_type.ends_with("blob")
}

fn create_table_base_type(data_type: &str) -> String {
    data_type
        .trim()
        .split_once('(')
        .map_or(data_type.trim(), |(base, _)| base.trim())
        .to_ascii_lowercase()
}

fn create_table_is_number_type(data_type: &str) -> bool {
    [
        "tinyint",
        "smallint",
        "mediumint",
        "int",
        "integer",
        "bigint",
        "float",
        "double",
        "decimal",
    ]
    .iter()
    .any(|value| data_type.to_ascii_lowercase().contains(value))
}

fn create_table_is_integer_type(data_type: &str) -> bool {
    matches!(
        create_table_base_type(data_type).as_str(),
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint"
    )
}

fn create_table_unquoted_string_default(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.eq_ignore_ascii_case("NULL")
        && !((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
}

fn create_table_is_auto_update_time_type(data_type: &str) -> bool {
    let data_type = data_type.to_ascii_lowercase();
    data_type.contains("datetime") || data_type.contains("timestamp")
}

fn quote_mysql_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn quote_mysql_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

fn quote_sqlite_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn create_table_mysql_supports_length(data_type: &str) -> bool {
    create_table_supports_length(data_type)
}

fn create_table_mysql_default_length(data_type: &str) -> Option<&'static str> {
    create_table_default_length(data_type)
}

fn create_table_mysql_supports_scale(data_type: &str) -> bool {
    create_table_supports_scale(data_type)
}

fn create_table_mysql_is_text_type(data_type: &str) -> bool {
    create_table_is_text_type(data_type)
}

fn create_table_mysql_supports_key_length(data_type: &str) -> bool {
    create_table_supports_key_length(data_type)
}

fn create_table_mysql_is_number_type(data_type: &str) -> bool {
    create_table_is_number_type(data_type)
}

fn create_table_mysql_is_integer_type(data_type: &str) -> bool {
    create_table_is_integer_type(data_type)
}

fn create_table_mysql_is_auto_update_time_type(data_type: &str) -> bool {
    create_table_is_auto_update_time_type(data_type)
}
