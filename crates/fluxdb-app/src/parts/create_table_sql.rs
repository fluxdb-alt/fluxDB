fn create_table_mysql_sql_preview(create: &CreateTableState) -> Result<String, String> {
    if let Some(message) = create.validation_error() {
        return Err(message.to_string());
    }

    let table_name = create.table_name.trim();
    let mut lines = create
        .columns
        .iter()
        .filter_map(CreateTableColumn::sql_line)
        .collect::<Vec<_>>();
    if let Some(primary_key) = create_table_primary_key_line(&create.columns) {
        lines.push(primary_key);
    }
    lines.extend(create_table_index_lines(create)?);
    lines.extend(create_table_foreign_key_lines(
        create,
        CreateTableSqlDialect::MySql,
    )?);
    lines.extend(create_table_check_lines(
        create,
        CreateTableSqlDialect::MySql,
    )?);

    let mut sql = format!(
        "CREATE TABLE {} (\n{}\n)",
        quote_mysql_identifier(table_name),
        lines
            .into_iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join(",\n")
    );
    let comment = create.comment.trim();
    if !comment.is_empty() {
        sql.push_str(&format!(" COMMENT={}", quote_mysql_string(comment)));
    }
    let table_options = create_table_mysql_table_options(create);
    if !table_options.is_empty() {
        sql.push(' ');
        sql.push_str(&table_options.join(" "));
    }
    let partition_sql = create_table_mysql_partition_sql(create);
    if !partition_sql.is_empty() {
        sql.push('\n');
        sql.push_str(&partition_sql);
    }
    sql.push(';');
    let trigger_statements = create_table_mysql_trigger_statements(create)?;
    if !trigger_statements.is_empty() {
        sql.push('\n');
        sql.push_str(&trigger_statements.join("\n"));
    }
    Ok(sql)
}

fn create_table_sqlite_sql_preview(create: &CreateTableState) -> Result<String, String> {
    if let Some(message) = create.validation_error() {
        return Err(message.to_string());
    }

    let mut sql = create_table_sqlite_create_table_statement(create, create.table_name.trim())?;
    let index_statements = create_table_sqlite_index_statements(create)?;
    if !index_statements.is_empty() {
        sql.push('\n');
        sql.push_str(&index_statements.join("\n"));
    }
    let trigger_statements = create_table_sqlite_trigger_statements(create)?;
    if !trigger_statements.is_empty() {
        sql.push('\n');
        sql.push_str(&trigger_statements.join("\n"));
    }
    Ok(sql)
}

fn create_table_sqlite_create_table_statement(
    create: &CreateTableState,
    table_name: &str,
) -> Result<String, String> {
    let primary_key_columns = create
        .columns
        .iter()
        .filter(|column| column.primary_key && !column.name.trim().is_empty())
        .collect::<Vec<_>>();
    let inline_auto_increment_id = primary_key_columns.len() == 1
        && primary_key_columns[0].auto_increment
        && matches!(
            create_table_base_type(&primary_key_columns[0].data_type).as_str(),
            "integer" | "int"
        );

    let mut lines = create
        .columns
        .iter()
        .filter_map(|column| {
            create_table_sqlite_column_line(column, inline_auto_increment_id)
        })
        .collect::<Vec<_>>();
    if !inline_auto_increment_id {
        let primary_key = primary_key_columns
            .iter()
            .map(|column| quote_sqlite_identifier(column.name.trim()))
            .collect::<Vec<_>>();
        if !primary_key.is_empty() {
            lines.push(format!("PRIMARY KEY ({})", primary_key.join(", ")));
        }
    }
    lines.extend(create_table_foreign_key_lines(
        create,
        CreateTableSqlDialect::Sqlite,
    )?);
    lines.extend(create_table_check_lines(
        create,
        CreateTableSqlDialect::Sqlite,
    )?);

    Ok(format!(
        "CREATE TABLE {table_name} (\n{}\n);",
        lines
            .into_iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join(",\n"),
        table_name = quote_sqlite_identifier(table_name)
    ))
}

fn create_table_sqlite_column_line(
    column: &CreateTableColumn,
    inline_auto_increment_id: bool,
) -> Option<String> {
    let name = column.name.trim();
    if name.is_empty() {
        return None;
    }

    let mut sql = format!(
        "{} {}",
        quote_sqlite_identifier(name),
        create_table_sqlite_column_type(&column.data_type)
    );
    if inline_auto_increment_id && column.auto_increment {
        sql.push_str(" PRIMARY KEY AUTOINCREMENT");
    } else if !column.nullable || column.primary_key {
        sql.push_str(" NOT NULL");
    }
    if !column.default_value.trim().is_empty() {
        sql.push_str(" DEFAULT ");
        sql.push_str(column.default_value.trim());
    }
    Some(sql)
}

fn create_table_sqlite_column_type(data_type: &str) -> String {
    let base = create_table_base_type(data_type);
    match base.as_str() {
        "int" | "integer" | "tinyint" | "smallint" | "mediumint" | "bigint" | "boolean" => {
            "INTEGER".to_string()
        }
        "real" | "float" | "double" => "REAL".to_string(),
        "numeric" | "decimal" => "NUMERIC".to_string(),
        "blob" | "binary" | "varbinary" | "tinyblob" | "mediumblob" | "longblob" => {
            "BLOB".to_string()
        }
        _ => "TEXT".to_string(),
    }
}

fn create_table_design_sql_preview_for_provider(
    provider: &dyn CreateTableProvider,
    create: &CreateTableState,
) -> Result<String, String> {
    if let Some(message) = provider.validation_error(create) {
        return Err(message.to_string());
    }
    let statements = provider.design_statements(create)?;
    if statements.is_empty() {
        return Err("没有需要保存的变更".to_string());
    }
    Ok(statements.join("\n"))
}

pub fn rename_table_sql_preview(
    database_kind: DatabaseKind,
    schema: Option<&str>,
    old_name: &str,
    new_name: &str,
) -> Result<String, String> {
    let provider = table_action_sql_provider(database_kind);
    let old_name = old_name.trim();
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("表名不能为空".to_string());
    }
    if old_name == new_name {
        return Err("表名没有变化".to_string());
    }
    provider.rename_table_sql(schema, old_name, new_name)
}

pub fn copy_table_sql_preview(
    database_kind: DatabaseKind,
    schema: Option<&str>,
    source_name: &str,
    target_name: &str,
    copy_data: bool,
) -> Result<String, String> {
    copy_table_sql_preview_with_source_ddl(
        database_kind,
        schema,
        source_name,
        target_name,
        copy_data,
        None,
    )
}

pub fn copy_table_sql_preview_with_source_ddl(
    database_kind: DatabaseKind,
    schema: Option<&str>,
    source_name: &str,
    target_name: &str,
    copy_data: bool,
    source_ddl: Option<&str>,
) -> Result<String, String> {
    let provider = table_action_sql_provider(database_kind);
    let source_name = source_name.trim();
    let target_name = target_name.trim();
    if target_name.is_empty() {
        return Err("表名不能为空".to_string());
    }
    if source_name == target_name {
        return Err("新表名不能和原表相同".to_string());
    }
    provider.copy_table_sql(schema, source_name, target_name, copy_data, source_ddl)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForeignKeyCheckMode {
    Default,
    Enable,
    Disable,
}

pub fn drop_table_sql_preview(
    database_kind: DatabaseKind,
    kind: ObjectKind,
    schema: Option<&str>,
    table_name: &str,
    foreign_key_check: ForeignKeyCheckMode,
) -> Result<String, String> {
    let provider = table_action_sql_provider(database_kind);
    let table_name = table_name.trim();
    if table_name.is_empty() {
        return Err("表名不能为空".to_string());
    }
    let sql = provider.drop_table_sql(kind, schema, table_name)?;
    provider.with_foreign_key_check(sql, foreign_key_check)
}

pub fn truncate_table_sql_preview(
    database_kind: DatabaseKind,
    schema: Option<&str>,
    table_name: &str,
    restart_identity: bool,
    foreign_key_check: ForeignKeyCheckMode,
) -> Result<String, String> {
    let provider = table_action_sql_provider(database_kind);
    let table_name = table_name.trim();
    if table_name.is_empty() {
        return Err("表名不能为空".to_string());
    }
    let sql = provider.truncate_table_sql(schema, table_name, restart_identity)?;
    provider.with_foreign_key_check(sql, foreign_key_check)
}

trait TableActionSqlProvider: Sync {
    /// `schema` 仅 PG 使用：同名表必须只操作指定 schema 的对象。
    fn rename_table_sql(
        &self,
        schema: Option<&str>,
        old_name: &str,
        new_name: &str,
    ) -> Result<String, String>;
    fn copy_table_sql(
        &self,
        schema: Option<&str>,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        source_ddl: Option<&str>,
    ) -> Result<String, String>;
    fn drop_table_sql(
        &self,
        kind: ObjectKind,
        schema: Option<&str>,
        table_name: &str,
    ) -> Result<String, String>;
    fn truncate_table_sql(
        &self,
        schema: Option<&str>,
        table_name: &str,
        restart_identity: bool,
    ) -> Result<String, String>;

    fn with_foreign_key_check(
        &self,
        sql: String,
        foreign_key_check: ForeignKeyCheckMode,
    ) -> Result<String, String> {
        match foreign_key_check {
            ForeignKeyCheckMode::Default => Ok(sql),
            ForeignKeyCheckMode::Enable | ForeignKeyCheckMode::Disable => {
                Err("当前连接类型不支持外键检查选项".to_string())
            }
        }
    }
}

