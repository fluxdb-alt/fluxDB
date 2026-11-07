struct MySqlTableActionSqlProvider;
struct SqliteTableActionSqlProvider;
struct SqlServerTableActionSqlProvider;
struct UnsupportedTableActionSqlProvider;

impl TableActionSqlProvider for MySqlTableActionSqlProvider {
    fn rename_table_sql(
        &self,
        _: Option<&str>,
        old_name: &str,
        new_name: &str,
    ) -> Result<String, String> {
        Ok(format!(
            "ALTER TABLE {} RENAME TO {};",
            fluxdb_core::MySqlDialect.quote_identifier(old_name),
            fluxdb_core::MySqlDialect.quote_identifier(new_name)
        ))
    }

    fn copy_table_sql(
        &self,
        _: Option<&str>,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        _: Option<&str>,
    ) -> Result<String, String> {
        let source = fluxdb_core::MySqlDialect.quote_identifier(source_name);
        let target = fluxdb_core::MySqlDialect.quote_identifier(target_name);
        let mut statements = vec![format!("CREATE TABLE {target} LIKE {source};")];
        if copy_data {
            statements.push(format!("INSERT INTO {target} SELECT * FROM {source};"));
        }
        Ok(statements.join("\n"))
    }

    fn drop_table_sql(
        &self,
        _: ObjectKind,
        _: Option<&str>,
        table_name: &str,
    ) -> Result<String, String> {
        Ok(format!("DROP TABLE {};", fluxdb_core::MySqlDialect.quote_identifier(table_name)))
    }

    fn truncate_table_sql(
        &self,
        _: Option<&str>,
        table_name: &str,
        _: bool,
    ) -> Result<String, String> {
        Ok(format!(
            "TRUNCATE TABLE {};",
            fluxdb_core::MySqlDialect.quote_identifier(table_name)
        ))
    }

    fn with_foreign_key_check(
        &self,
        sql: String,
        foreign_key_check: ForeignKeyCheckMode,
    ) -> Result<String, String> {
        match foreign_key_check {
            ForeignKeyCheckMode::Default => Ok(sql),
            ForeignKeyCheckMode::Enable => Ok(format!("SET FOREIGN_KEY_CHECKS = 1;\n{sql}")),
            ForeignKeyCheckMode::Disable => Ok(format!("SET FOREIGN_KEY_CHECKS = 0;\n{sql}")),
        }
    }
}
impl TableActionSqlProvider for SqliteTableActionSqlProvider {
    fn rename_table_sql(
        &self,
        _: Option<&str>,
        old_name: &str,
        new_name: &str,
    ) -> Result<String, String> {
        Ok(format!(
            "ALTER TABLE {} RENAME TO {};",
            fluxdb_core::SqliteDialect.quote_identifier(old_name),
            fluxdb_core::SqliteDialect.quote_identifier(new_name)
        ))
    }

    fn copy_table_sql(
        &self,
        _: Option<&str>,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        source_ddl: Option<&str>,
    ) -> Result<String, String> {
        let source = fluxdb_core::SqliteDialect.quote_identifier(source_name);
        let target = fluxdb_core::SqliteDialect.quote_identifier(target_name);
        let mut statements = vec![if let Some(source_ddl) = source_ddl {
            sqlite_copy_table_structure_sql(source_ddl, target_name)?
        } else {
            format!("CREATE TABLE {target} AS SELECT * FROM {source} WHERE 0;")
        }];
        if copy_data {
            statements.push(format!("INSERT INTO {target} SELECT * FROM {source};"));
        }
        Ok(statements.join("\n"))
    }

    fn drop_table_sql(
        &self,
        _: ObjectKind,
        _: Option<&str>,
        table_name: &str,
    ) -> Result<String, String> {
        Ok(format!("DROP TABLE {};", fluxdb_core::SqliteDialect.quote_identifier(table_name)))
    }

    fn truncate_table_sql(
        &self,
        _: Option<&str>,
        table_name: &str,
        _: bool,
    ) -> Result<String, String> {
        Ok(format!("DELETE FROM {};", fluxdb_core::SqliteDialect.quote_identifier(table_name)))
    }
}

impl TableActionSqlProvider for UnsupportedTableActionSqlProvider {
    fn rename_table_sql(&self, _: Option<&str>, _: &str, _: &str) -> Result<String, String> {
        Err("当前连接类型暂不支持重命名表".to_string())
    }

    fn copy_table_sql(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: bool,
        _: Option<&str>,
    ) -> Result<String, String> {
        Err("当前连接类型暂不支持复制表".to_string())
    }

    fn drop_table_sql(
        &self,
        _: ObjectKind,
        _: Option<&str>,
        _: &str,
    ) -> Result<String, String> {
        Err("当前连接类型暂不支持删除表".to_string())
    }

    fn truncate_table_sql(
        &self,
        _: Option<&str>,
        _: &str,
        _: bool,
    ) -> Result<String, String> {
        Err("当前连接类型暂不支持清空表".to_string())
    }
}

fn sqlite_copy_table_structure_sql(source_ddl: &str, target_name: &str) -> Result<String, String> {
    // ponytail: 只复制 CREATE TABLE DDL；需要完整复制二级索引/触发器时再改写 sqlite_schema 相关 DDL。
    let ddl = source_ddl.trim().trim_end_matches(';').trim();
    let lower = ddl.to_ascii_lowercase();
    let Some(mut index) = lower.find("create table") else {
        return Err("未读取到可复制的 SQLite 表结构 DDL".to_string());
    };
    index += "create table".len();
    index = skip_ascii_whitespace(ddl, index);
    for keyword in ["if", "not", "exists"] {
        if ascii_keyword_at(ddl, index, keyword) {
            index += keyword.len();
            index = skip_ascii_whitespace(ddl, index);
        }
    }
    let name_start = index;
    let name_end = sqlite_create_table_name_end(ddl, name_start)?;
    let mut sql = String::with_capacity(ddl.len() + target_name.len() + 4);
    sql.push_str(&ddl[..name_start]);
    sql.push_str(&fluxdb_core::SqliteDialect.quote_identifier(target_name));
    sql.push_str(ddl[name_end..].trim_end());
    sql.push(';');
    Ok(sql)
}

fn skip_ascii_whitespace(value: &str, mut index: usize) -> usize {
    while value
        .as_bytes()
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

fn ascii_keyword_at(value: &str, index: usize, keyword: &str) -> bool {
    value
        .get(index..index + keyword.len())
        .is_some_and(|part| part.eq_ignore_ascii_case(keyword))
        && value
            .as_bytes()
            .get(index + keyword.len())
            .is_none_or(|byte| byte.is_ascii_whitespace())
}

fn sqlite_create_table_name_end(value: &str, start: usize) -> Result<usize, String> {
    let bytes = value.as_bytes();
    let Some(first) = bytes.get(start).copied() else {
        return Err("未读取到 SQLite 表名".to_string());
    };
    match first {
        b'"' | b'\'' | b'`' => quoted_identifier_end(bytes, start, first),
        b'[' => bytes[start + 1..]
            .iter()
            .position(|byte| *byte == b']')
            .map(|offset| start + 1 + offset + 1)
            .ok_or_else(|| "SQLite 表名引用未闭合".to_string()),
        _ => {
            let end = bytes[start..]
                .iter()
                .position(|byte| byte.is_ascii_whitespace() || *byte == b'(')
                .map(|offset| start + offset)
                .unwrap_or(value.len());
            if end == start {
                Err("未读取到 SQLite 表名".to_string())
            } else {
                Ok(end)
            }
        }
    }
}

fn quoted_identifier_end(bytes: &[u8], start: usize, quote: u8) -> Result<usize, String> {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == quote {
            if bytes.get(index + 1) == Some(&quote) {
                index += 2;
            } else {
                return Ok(index + 1);
            }
        } else {
            index += 1;
        }
    }
    Err("SQLite 表名引用未闭合".to_string())
}

static MYSQL_TABLE_ACTION_SQL_PROVIDER: MySqlTableActionSqlProvider = MySqlTableActionSqlProvider;
static SQLITE_TABLE_ACTION_SQL_PROVIDER: SqliteTableActionSqlProvider =
    SqliteTableActionSqlProvider;
static SQLSERVER_TABLE_ACTION_SQL_PROVIDER: SqlServerTableActionSqlProvider =
    SqlServerTableActionSqlProvider;
static POSTGRES_TABLE_ACTION_SQL_PROVIDER: PostgresTableActionSqlProvider =
    PostgresTableActionSqlProvider;
static UNSUPPORTED_TABLE_ACTION_SQL_PROVIDER: UnsupportedTableActionSqlProvider =
    UnsupportedTableActionSqlProvider;

fn table_action_sql_provider(database_kind: DatabaseKind) -> &'static dyn TableActionSqlProvider {
    match database_kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            &MYSQL_TABLE_ACTION_SQL_PROVIDER
        }
        DatabaseKind::Sqlite => &SQLITE_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::SqlServer => &SQLSERVER_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::Postgres | DatabaseKind::Kingbase => {
            &POSTGRES_TABLE_ACTION_SQL_PROVIDER
        }
        DatabaseKind::MongoDb | DatabaseKind::Redis => &UNSUPPORTED_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::Dameng => &DAMENG_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::Oracle => &ORACLE_TABLE_ACTION_SQL_PROVIDER,
    }
}

/// Oracle 表操作。
///
/// 复制表不能用 `CREATE TABLE … AS SELECT`：那会丢掉约束、索引和注释，也会把
/// identity 变成普通列。这里改为重写源表 DDL 再显式插数。
struct OracleTableActionSqlProvider;

impl TableActionSqlProvider for OracleTableActionSqlProvider {
    fn rename_table_sql(
        &self,
        schema: Option<&str>,
        old_name: &str,
        new_name: &str,
    ) -> Result<String, String> {
        if new_name.contains('.') {
            return Err("重命名只接受单段新表名（不带 schema）".to_string());
        }
        Ok(format!(
            "ALTER TABLE {} RENAME TO {};",
            oracle_action_qualified(schema, old_name),
            fluxdb_core::OracleDialect.quote_identifier(new_name.trim())
        ))
    }

    fn copy_table_sql(
        &self,
        schema: Option<&str>,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        source_ddl: Option<&str>,
    ) -> Result<String, String> {
        let source_ddl =
            source_ddl.ok_or_else(|| "未读取到源表 Oracle DDL，无法复制表".to_string())?;
        oracle_copy_table_sql(schema, source_name, target_name, copy_data, source_ddl)
    }

    fn drop_table_sql(
        &self,
        kind: ObjectKind,
        schema: Option<&str>,
        table_name: &str,
    ) -> Result<String, String> {
        Ok(format!(
            "DROP {} {};",
            if kind == ObjectKind::View { "VIEW" } else { "TABLE" },
            oracle_action_qualified(schema, table_name)
        ))
    }

    fn truncate_table_sql(
        &self,
        schema: Option<&str>,
        table_name: &str,
        _: bool,
    ) -> Result<String, String> {
        Ok(format!(
            "TRUNCATE TABLE {};",
            oracle_action_qualified(schema, table_name)
        ))
    }

    fn with_foreign_key_check(
        &self,
        sql: String,
        foreign_key_check: ForeignKeyCheckMode,
    ) -> Result<String, String> {
        match foreign_key_check {
            ForeignKeyCheckMode::Default => Ok(sql),
            // Oracle 只能 ALTER TABLE … DISABLE CONSTRAINT（DDL 且需要表级权限），
            // 没有连接级开关；不生成会静默失败或改坏约束的语句。
            ForeignKeyCheckMode::Enable | ForeignKeyCheckMode::Disable => Err(
                "Oracle 不支持以连接级开关临时禁用外键检查；请先删除依赖外键或改用 DELETE"
                    .to_string(),
            ),
        }
    }
}

static ORACLE_TABLE_ACTION_SQL_PROVIDER: OracleTableActionSqlProvider =
    OracleTableActionSqlProvider;

fn oracle_action_qualified(schema: Option<&str>, name: &str) -> String {
    match schema.map(str::trim).filter(|schema| !schema.is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            fluxdb_core::OracleDialect.quote_identifier(schema),
            fluxdb_core::OracleDialect.quote_identifier(name.trim())
        ),
        None => fluxdb_core::OracleDialect.quote_identifier(name.trim()),
    }
}

fn oracle_copy_table_sql(
    schema: Option<&str>,
    source_name: &str,
    target_name: &str,
    copy_data: bool,
    source_ddl: &str,
) -> Result<String, String> {
    let source = oracle_action_qualified(schema, source_name);
    let target = oracle_action_qualified(schema, target_name);
    let statements = oracle_split_sql_statements(source_ddl)?;
    if statements.is_empty() {
        return Err("未读取到可复制的 Oracle 表结构 DDL".to_string());
    }
    let mut rewritten = Vec::with_capacity(statements.len() + 2);
    for statement in &statements {
        rewritten.push(oracle_rewrite_copy_statement(
            statement,
            &source,
            &target,
            target_name,
        )?);
    }
    if copy_data {
        let columns = oracle_copy_columns(source_ddl, true);
        let insert = if columns.is_empty() {
            // 没有可写列（全为 ALWAYS identity）时不生成插数语句，避免必然失败。
            return Ok(rewritten.join("\n"));
        } else {
            let column_list = columns
                .iter()
                .map(|column| fluxdb_core::OracleDialect.quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ");
            format!("INSERT INTO {target} ({column_list}) SELECT {column_list} FROM {source};")
        };
        rewritten.push(insert);
    }
    Ok(rewritten.join("\n"))
}

fn oracle_split_sql_statements(sql: &str) -> Result<Vec<String>, String> {
    let mut statements = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<char> = None;
    for (index, ch) in sql.char_indices() {
        if let Some(end_quote) = quote {
            if ch == end_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ';' => {
                let statement = sql[start..index].trim();
                if !statement.is_empty() {
                    statements.push(format!("{statement};"));
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() {
        return Err("Oracle 表结构 DDL 引号未闭合".to_string());
    }
    let tail = sql[start..].trim();
    if !tail.is_empty() {
        statements.push(format!("{tail};"));
    }
    Ok(statements)
}

fn oracle_rewrite_copy_statement(
    statement: &str,
    source: &str,
    target: &str,
    target_name: &str,
) -> Result<String, String> {
    let trimmed = statement.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("CREATE TABLE ") {
        return oracle_rewrite_create_table(trimmed, target, target_name);
    }
    if upper.starts_with("CREATE INDEX ")
        || upper.starts_with("CREATE UNIQUE INDEX ")
        || upper.starts_with("CREATE BITMAP INDEX ")
    {
        return oracle_rewrite_index(trimmed, target, target_name);
    }
    if upper.starts_with("COMMENT ON ") {
        // COMMENT ON TABLE/COLUMN 的目标必须是新表；其余文本原样保留。
        return Ok(trimmed.replacen(source, target, 1));
    }
    Ok(trimmed.to_string())
}

fn oracle_rewrite_create_table(
    statement: &str,
    target: &str,
    target_name: &str,
) -> Result<String, String> {
    let upper = statement.to_ascii_uppercase();
    let Some(create_pos) = upper.find("CREATE TABLE") else {
        return Err("Oracle DDL 缺少 CREATE TABLE".to_string());
    };
    let name_start = create_pos + "CREATE TABLE".len();
    let Some(paren_offset) = statement[name_start..].find('(') else {
        return Err("Oracle CREATE TABLE 缺少字段定义".to_string());
    };
    let body_start = name_start + paren_offset;
    let mut body = format!(
        "CREATE TABLE {target}\n{}",
        oracle_rename_constraint_names(&statement[body_start..], target_name)
    );
    if !body.trim_end().ends_with(';') {
        body.push(';');
    }
    Ok(body)
}

/// 复制出来的约束必须换名：同一 schema 内约束名唯一，沿用源名会报 ORA-02264。
fn oracle_rename_constraint_names(statement: &str, target_name: &str) -> String {
    let mut output = String::with_capacity(statement.len());
    let mut rest = statement;
    while let Some(offset) = rest.to_ascii_uppercase().find("CONSTRAINT") {
        let keyword_end = offset + "CONSTRAINT".len();
        output.push_str(&rest[..keyword_end]);
        let after = rest[keyword_end..].trim_start();
        let whitespace = &rest[keyword_end..keyword_end + (rest[keyword_end..].len() - after.len())];
        output.push_str(whitespace);
        if !after.starts_with('"') {
            output.push_str(after);
            return output;
        }
        let Some(end) = after[1..].find('"').map(|index| index + 2) else {
            output.push_str(after);
            return output;
        };
        let old_name = &after[1..end - 1];
        output.push('"');
        output.push_str(&oracle_copy_object_name(target_name, old_name).replace('"', "\"\""));
        output.push('"');
        rest = &after[end..];
    }
    output.push_str(rest);
    output
}

fn oracle_rewrite_index(
    statement: &str,
    target: &str,
    target_name: &str,
) -> Result<String, String> {
    let upper = statement.to_ascii_uppercase();
    let Some(index_pos) = upper.find("INDEX") else {
        return Err("Oracle 索引 DDL 缺少 INDEX".to_string());
    };
    let after_index = &statement[index_pos + "INDEX".len()..];
    let name_start = after_index
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, _)| index)
        .ok_or_else(|| "Oracle 索引 DDL 缺少索引名".to_string())?;
    let name_rest = &after_index[name_start..];
    let (old_name, old_end) = oracle_parse_quoted_identifier(name_rest)?;
    let before_name = &statement[..index_pos + "INDEX".len() + name_start];
    let after_name = &name_rest[old_end..];
    let upper_after = after_name.to_ascii_uppercase();
    let Some(on_pos) = upper_after.find(" ON ") else {
        return Err("Oracle 索引 DDL 缺少 ON".to_string());
    };
    let table_start = on_pos + " ON ".len();
    let table_rest = &after_name[table_start..];
    let table_end = table_rest
        .find(char::is_whitespace)
        .or_else(|| table_rest.find('('))
        .ok_or_else(|| "Oracle 索引 DDL 缺少索引字段".to_string())?;
    let new_name = oracle_copy_object_name(target_name, &old_name);
    Ok(format!(
        "{before_name}\"{new_name}\"{} ON {target}{}",
        &after_name[..on_pos],
        &table_rest[table_end..]
    ))
}

fn oracle_parse_quoted_identifier(value: &str) -> Result<(String, usize), String> {
    if !value.starts_with('"') {
        return Err("Oracle DDL 标识符未引用".to_string());
    }
    let end = value[1..]
        .find('"')
        .map(|index| index + 2)
        .ok_or_else(|| "Oracle DDL 标识符引用未闭合".to_string())?;
    Ok((value[1..end - 1].to_string(), end))
}

fn oracle_copy_object_name(target_name: &str, old_name: &str) -> String {
    // Oracle 标识符上限 30 字节（12.2 之前）/128（之后），统一按 128 截断。
    format!("{}_{}", target_name.trim(), old_name.trim())
        .chars()
        .take(128)
        .collect()
}

/// 复制时用于 INSERT 的列清单。
///
/// `GENERATED ALWAYS AS IDENTITY` 列不接受显式写入（ORA-32795），必须排除；
/// `BY DEFAULT` 列可以写，保留原值。
fn oracle_copy_columns(source_ddl: &str, exclude_always_identity: bool) -> Vec<String> {
    let Some(open) = source_ddl.find('(') else {
        return Vec::new();
    };
    let body = &source_ddl[open + 1..];
    let mut columns = Vec::new();
    for entry in dameng_split_top_level_entries(body) {
        let trimmed = entry.trim();
        let upper = trimmed.to_ascii_uppercase();
        if trimmed.is_empty()
            || ["CONSTRAINT", "PRIMARY", "FOREIGN", "CHECK", "UNIQUE"]
                .iter()
                .any(|keyword| upper.starts_with(keyword))
        {
            continue;
        }
        if exclude_always_identity && upper.contains("GENERATED ALWAYS AS IDENTITY") {
            continue;
        }
        let Some(name) = dameng_ddl_identifier(trimmed) else {
            continue;
        };
        columns.push(name);
    }
    columns
}

struct DamengTableActionSqlProvider;

impl TableActionSqlProvider for DamengTableActionSqlProvider {
    fn rename_table_sql(
        &self,
        schema: Option<&str>,
        old_name: &str,
        new_name: &str,
    ) -> Result<String, String> {
        if new_name.contains('.') {
            return Err("重命名只接受单段新表名（不带 schema）".to_string());
        }
        Ok(format!(
            "ALTER TABLE {} RENAME TO {};",
            dameng_action_qualified(schema, old_name),
            fluxdb_core::DamengDialect.quote_identifier(new_name)
        ))
    }

    fn copy_table_sql(
        &self,
        schema: Option<&str>,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        source_ddl: Option<&str>,
    ) -> Result<String, String> {
        let source_ddl =
            source_ddl.ok_or_else(|| "未读取到源表达梦 DDL，无法复制表".to_string())?;
        dameng_copy_table_sql(schema, source_name, target_name, copy_data, source_ddl)
    }

    fn drop_table_sql(
        &self,
        kind: ObjectKind,
        schema: Option<&str>,
        table_name: &str,
    ) -> Result<String, String> {
        Ok(format!(
            "DROP {} {};",
            if kind == ObjectKind::View { "VIEW" } else { "TABLE" },
            dameng_action_qualified(schema, table_name)
        ))
    }

    fn truncate_table_sql(
        &self,
        schema: Option<&str>,
        table_name: &str,
        _: bool,
    ) -> Result<String, String> {
        Ok(format!(
            "TRUNCATE TABLE {};",
            dameng_action_qualified(schema, table_name)
        ))
    }

    fn with_foreign_key_check(
        &self,
        sql: String,
        foreign_key_check: ForeignKeyCheckMode,
    ) -> Result<String, String> {
        match foreign_key_check {
            ForeignKeyCheckMode::Default => Ok(sql),
            ForeignKeyCheckMode::Enable | ForeignKeyCheckMode::Disable => Err(
                "达梦不支持以连接级开关临时禁用外键检查".to_string(),
            ),
        }
    }
}

static DAMENG_TABLE_ACTION_SQL_PROVIDER: DamengTableActionSqlProvider =
    DamengTableActionSqlProvider;

impl TableActionSqlProvider for SqlServerTableActionSqlProvider {
    fn rename_table_sql(
        &self,
        schema: Option<&str>,
        old_name: &str,
        new_name: &str,
    ) -> Result<String, String> {
        sqlserver_table_rename_statement(schema, old_name, new_name)
    }

    fn copy_table_sql(
        &self,
        schema: Option<&str>,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        source_ddl: Option<&str>,
    ) -> Result<String, String> {
        let source_ddl =
            source_ddl.ok_or_else(|| "未读取到源表 SQL Server DDL，无法复制表".to_string())?;
        sqlserver_copy_table_sql(schema, source_name, target_name, copy_data, source_ddl)
    }

    fn drop_table_sql(
        &self,
        kind: ObjectKind,
        schema: Option<&str>,
        table_name: &str,
    ) -> Result<String, String> {
        Ok(format!(
            "DROP {} {};",
            if kind == ObjectKind::View { "VIEW" } else { "TABLE" },
            sqlserver_action_qualified(schema, table_name)
        ))
    }

    fn truncate_table_sql(
        &self,
        schema: Option<&str>,
        table_name: &str,
        _: bool,
    ) -> Result<String, String> {
        Ok(format!(
            "TRUNCATE TABLE {};",
            sqlserver_action_qualified(schema, table_name)
        ))
    }

    fn with_foreign_key_check(
        &self,
        sql: String,
        foreign_key_check: ForeignKeyCheckMode,
    ) -> Result<String, String> {
        match foreign_key_check {
            ForeignKeyCheckMode::Default => Ok(sql),
            ForeignKeyCheckMode::Enable | ForeignKeyCheckMode::Disable => Err(
                "SQL Server 不支持连接级临时禁用外键检查；请先删除依赖外键或改用 DELETE"
                    .to_string(),
            ),
        }
    }
}

fn sqlserver_action_qualified(schema: Option<&str>, name: &str) -> String {
    match schema.map(str::trim).filter(|schema| !schema.is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            fluxdb_core::SqlServerDialect.quote_identifier(schema),
            fluxdb_core::SqlServerDialect.quote_identifier(name.trim())
        ),
        None => fluxdb_core::SqlServerDialect.quote_identifier(name.trim()),
    }
}

fn sqlserver_table_rename_statement(
    schema: Option<&str>,
    old_name: &str,
    new_name: &str,
) -> Result<String, String> {
    if new_name.contains('.') {
        return Err("重命名只接受单段新表名（不带 schema）".to_string());
    }
    Ok(format!(
        "EXEC sp_rename N'{}', N'{}', 'OBJECT';",
        sqlserver_action_qualified(schema, old_name).replace('\'', "''"),
        new_name.trim().replace('\'', "''")
    ))
}

fn sqlserver_copy_table_sql(
    schema: Option<&str>,
    source_name: &str,
    target_name: &str,
    copy_data: bool,
    source_ddl: &str,
) -> Result<String, String> {
    let source = sqlserver_action_qualified(schema, source_name);
    let target = sqlserver_action_qualified(schema, target_name);
    let statements = sqlserver_split_ddl_statements(source_ddl)?;
    if statements.is_empty() {
        return Err("未读取到可复制的 SQL Server 表结构 DDL".to_string());
    }

    let mut rewritten = Vec::with_capacity(statements.len() + 3);
    for statement in statements {
        let trimmed = statement.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("create table") {
            rewritten.push(sqlserver_replace_create_table_name(
                trimmed,
                schema,
                target_name,
            )?);
        } else if lower.starts_with("create index")
            || lower.starts_with("create unique index")
        {
            rewritten.push(sqlserver_rewrite_copy_index(
                trimmed,
                &source,
                &target,
                target_name,
            )?);
        } else if lower.starts_with("alter table") {
            rewritten.push(sqlserver_rewrite_copy_foreign_key(
                trimmed,
                &source,
                &target,
                target_name,
            )?);
        } else {
            rewritten.push(trimmed.to_string());
        }
    }

    if copy_data {
        let columns = sqlserver_copy_columns(source_ddl)?;
        if columns.is_empty() {
            rewritten.push(format!("INSERT INTO {target} SELECT * FROM {source};"));
        } else {
            let column_list = columns
                .iter()
                .map(|column| fluxdb_core::SqlServerDialect.quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ");
            let identity_columns = sqlserver_copy_identity_columns(source_ddl)?;
            let insert = format!(
                "INSERT INTO {target} ({column_list}) SELECT {column_list} FROM {source};"
            );
            if identity_columns.is_empty() {
                rewritten.push(insert);
            } else {
                // 显式复制 IDENTITY 值：SQL Server 同一会话仅允许一张表开启。
                rewritten.push(format!("SET IDENTITY_INSERT {target} ON;"));
                rewritten.push(insert);
                rewritten.push(format!("SET IDENTITY_INSERT {target} OFF;"));
            }
        }
    }
    Ok(rewritten.join("\n"))
}

fn sqlserver_replace_create_table_name(
    statement: &str,
    schema: Option<&str>,
    target_name: &str,
) -> Result<String, String> {
    let trimmed = statement.trim().trim_end_matches(';').trim();
    let upper = trimmed.to_ascii_uppercase();
    let Some(create_pos) = upper.find("CREATE TABLE") else {
        return Err("SQL Server 表 DDL 缺少 CREATE TABLE".to_string());
    };
    let name_start = create_pos + "CREATE TABLE".len();
    let mut index = skip_ascii_whitespace(trimmed, name_start);
    for keyword in ["IF", "NOT", "EXISTS"] {
        if ascii_keyword_at(trimmed, index, keyword) {
            index += keyword.len();
            index = skip_ascii_whitespace(trimmed, index);
        }
    }
    let Some(open) = trimmed[index..].find('(') else {
        return Err("SQL Server CREATE TABLE 缺少字段定义".to_string());
    };
    let body_start = index + open;
    let target = sqlserver_action_qualified(schema, target_name);
    Ok(format!(
        "CREATE TABLE {target}\n{};",
        sqlserver_rename_copy_constraints(&trimmed[body_start..], target_name)?
    ))
}

fn sqlserver_rename_copy_constraints(
    statement: &str,
    target_name: &str,
) -> Result<String, String> {
    let mut output = String::with_capacity(statement.len());
    let mut rest = statement;
    while let Some(offset) = rest.to_ascii_uppercase().find("CONSTRAINT") {
        let keyword_end = offset + "CONSTRAINT".len();
        output.push_str(&rest[..keyword_end]);
        let after = rest[keyword_end..].trim_start();
        let whitespace = &rest[keyword_end..keyword_end + (rest[keyword_end..].len() - after.len())];
        output.push_str(whitespace);
        let Some(_first) = after.chars().next() else {
            output.push_str(after);
            return Ok(output);
        };
        let Ok((old_name, end)) = sqlserver_parse_ddl_identifier(after) else {
            output.push_str(after);
            return Ok(output);
        };
        let new_name = sqlserver_copy_object_name(target_name, &old_name);
        output.push_str(&fluxdb_core::SqlServerDialect.quote_identifier(&new_name));
        rest = &after[end..];
    }
    output.push_str(rest);
    Ok(output)
}

fn sqlserver_rewrite_copy_index(
    statement: &str,
    source: &str,
    target: &str,
    target_name: &str,
) -> Result<String, String> {
    let trimmed = statement.trim().trim_end_matches(';').trim();
    let upper = trimmed.to_ascii_uppercase();
    let Some(index_pos) = upper.find("INDEX") else {
        return Err("SQL Server 索引 DDL 缺少 INDEX".to_string());
    };
    let after_index = &trimmed[index_pos + "INDEX".len()..];
    let name_start = after_index
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, _)| index)
        .ok_or_else(|| "SQL Server 索引 DDL 缺少索引名".to_string())?;
    let name_rest = &after_index[name_start..];
    let (old_name, old_end) = sqlserver_parse_ddl_identifier(name_rest)?;
    let before_name = &trimmed[..index_pos + "INDEX".len() + name_start];
    let after_name = &name_rest[old_end..];
    let upper_after = after_name.to_ascii_uppercase();
    let Some(on_pos) = upper_after.find(" ON ") else {
        return Err("SQL Server 索引 DDL 缺少 ON".to_string());
    };
    let table_start = on_pos + " ON ".len();
    let table_rest = &after_name[table_start..];
    let table_identifier = table_rest.split_whitespace().next().unwrap_or("");
    if !sqlserver_ddl_identifier_matches(table_identifier, source) {
        return Err("SQL Server 索引 DDL 引用了非源表，无法安全复制".to_string());
    }
    let table_end = table_rest
        .find(char::is_whitespace)
        .or_else(|| table_rest.find('('))
        .ok_or_else(|| "SQL Server 索引 DDL 缺少字段".to_string())?;
    let new_name = sqlserver_copy_object_name(target_name, &old_name);
    Ok(format!(
        "{before_name}{}{} ON {target}{};",
        fluxdb_core::SqlServerDialect.quote_identifier(&new_name),
        &after_name[..on_pos],
        &table_rest[table_end..]
    ))
}

/// 复制表的外键约束也必须换成新名称。SQL Server 的约束名在 schema 内唯一；
/// 保留源约束名会让 `ALTER TABLE ... ADD CONSTRAINT` 与源表冲突。
fn sqlserver_rewrite_copy_foreign_key(
    statement: &str,
    source: &str,
    target: &str,
    target_name: &str,
) -> Result<String, String> {
    let trimmed = statement.trim().trim_end_matches(';').trim();
    let upper = trimmed.to_ascii_uppercase();
    let Some(alter_pos) = upper.find("ALTER TABLE") else {
        return Err("SQL Server 外键 DDL 缺少 ALTER TABLE".to_string());
    };
    let table_start = skip_ascii_whitespace(trimmed, alter_pos + "ALTER TABLE".len());
    let source_table = &trimmed[table_start..];
    let (parsed_table, table_end) = sqlserver_parse_qualified_ddl_identifier(source_table)?;
    let (source_name, _) = sqlserver_parse_qualified_ddl_identifier(source)?;
    if !parsed_table.eq_ignore_ascii_case(&source_name) {
        return Err("SQL Server 外键 DDL 引用了非源表，无法安全复制".to_string());
    }
    let after_table = &source_table[table_end..];
    let upper_after = after_table.to_ascii_uppercase();
    let Some(add_pos) = upper_after.find("ADD") else {
        return Err(format!(
            "SQL Server 外键 DDL 缺少 ADD：{}",
            after_table.trim()
        ));
    };
    let constraint_keyword = skip_ascii_whitespace(after_table, add_pos + "ADD".len());
    if !ascii_keyword_at(after_table, constraint_keyword, "CONSTRAINT") {
        return Err("SQL Server 外键 DDL 缺少 CONSTRAINT".to_string());
    }
    let constraint_start =
        skip_ascii_whitespace(after_table, constraint_keyword + "CONSTRAINT".len());
    let constraint_rest = &after_table[constraint_start..];
    let (old_name, constraint_end) = sqlserver_parse_ddl_identifier(constraint_rest)?;
    let remainder = &constraint_rest[constraint_end..];
    if !remainder.to_ascii_uppercase().contains("FOREIGN KEY") {
        return Err("SQL Server 复制仅支持重写外键约束".to_string());
    }
    let new_name = sqlserver_copy_object_name(target_name, &old_name);
    Ok(format!(
        "ALTER TABLE {target}{}{}{};",
        &after_table[..constraint_start],
        fluxdb_core::SqlServerDialect.quote_identifier(&new_name),
        remainder,
    ))
}

fn sqlserver_parse_qualified_ddl_identifier(value: &str) -> Result<(String, usize), String> {
    let (first, first_end) = sqlserver_parse_ddl_identifier(value)?;
    let mut parts = vec![first];
    let mut end = first_end;
    loop {
        let dot = skip_ascii_whitespace(value, end);
        if !value[dot..].starts_with('.') {
            break;
        }
        let next_start = skip_ascii_whitespace(value, dot + 1);
        let (part, part_end) = sqlserver_parse_ddl_identifier(&value[next_start..])?;
        parts.push(part);
        end = next_start + part_end;
    }
    Ok((parts.join("."), end))
}

fn sqlserver_ddl_identifier_matches(identifier: &str, qualified: &str) -> bool {
    let identifier = identifier.trim().to_ascii_lowercase();
    let qualified = qualified.trim().to_ascii_lowercase();
    identifier == qualified
        || identifier
            == qualified
                .rsplit_once('.')
                .map(|(_, table)| table)
                .unwrap_or(&qualified)
}

fn sqlserver_split_ddl_statements(sql: &str) -> Result<Vec<String>, String> {
    let mut statements = Vec::new();
    let mut start = 0usize;
    let mut quote = None;
    let mut bracket = false;
    for (index, ch) in sql.char_indices() {
        if bracket {
            if ch == ']' {
                bracket = false;
            }
            continue;
        }
        if let Some(end_quote) = quote {
            if ch == end_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' => bracket = true,
            ';' => {
                let statement = sql[start..index].trim();
                if !statement.is_empty() {
                    statements.push(format!("{statement};"));
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() || bracket {
        return Err("SQL Server 表结构 DDL 引号未闭合".to_string());
    }
    let tail = sql[start..].trim();
    if !tail.is_empty() {
        statements.push(format!("{tail};"));
    }
    Ok(statements)
}

fn sqlserver_parse_ddl_identifier(value: &str) -> Result<(String, usize), String> {
    let value = value.trim_start();
    let first = value.chars().next().ok_or_else(|| "缺少标识符".to_string())?;
    match first {
        '[' => {
            let end = value[1..]
                .find(']')
                .map(|index| index + 2)
                .ok_or_else(|| "SQL Server 标识符引用未闭合".to_string())?;
            Ok((value[1..end - 1].replace("]]", "]"), end))
        }
        '"' | '`' => {
            let end = value[1..]
                .find(first)
                .map(|index| index + 2)
                .ok_or_else(|| "SQL Server 标识符引用未闭合".to_string())?;
            Ok((value[1..end - 1].to_string(), end))
        }
        _ => {
            let end = value
                .find(char::is_whitespace)
                .or_else(|| value.find('('))
                .unwrap_or(value.len());
            Ok((value[..end].to_string(), end))
        }
    }
}

fn sqlserver_copy_object_name(target_name: &str, old_name: &str) -> String {
    format!("{}_{}", target_name.trim(), old_name.trim())
        .chars()
        .take(128)
        .collect()
}

fn sqlserver_copy_columns(source_ddl: &str) -> Result<Vec<String>, String> {
    let Some(open) = source_ddl.find('(') else {
        return Ok(Vec::new());
    };
    let Some(close) = sqlserver_matching_paren(source_ddl, open) else {
        return Err("SQL Server CREATE TABLE 字段列表未闭合".to_string());
    };
    let entries = dameng_split_top_level_entries(&source_ddl[open + 1..close]);
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let upper = entry.trim().to_ascii_uppercase();
            if upper.starts_with("CONSTRAINT")
                || upper.starts_with("PRIMARY")
                || upper.starts_with("FOREIGN")
                || upper.starts_with("CHECK")
                || upper.starts_with("UNIQUE")
            {
                return None;
            }
            sqlserver_parse_ddl_identifier(&entry).ok().map(|(name, _)| name)
        })
        .collect())
}

fn sqlserver_copy_identity_columns(source_ddl: &str) -> Result<Vec<String>, String> {
    let Some(open) = source_ddl.find('(') else {
        return Ok(Vec::new());
    };
    let Some(close) = sqlserver_matching_paren(source_ddl, open) else {
        return Err("SQL Server CREATE TABLE 字段列表未闭合".to_string());
    };
    Ok(dameng_split_top_level_entries(&source_ddl[open + 1..close])
        .into_iter()
        .filter(|entry| entry.to_ascii_uppercase().contains(" IDENTITY"))
        .filter_map(|entry| sqlserver_parse_ddl_identifier(&entry).ok().map(|(name, _)| name))
        .collect())
}

fn sqlserver_matching_paren(value: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut bracket = false;
    for (index, ch) in value.char_indices().skip_while(|(index, _)| *index < start) {
        if bracket {
            if ch == ']' {
                bracket = false;
            }
            continue;
        }
        if let Some(end_quote) = quote {
            if ch == end_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' => bracket = true,
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn dameng_action_qualified(schema: Option<&str>, name: &str) -> String {
    match schema.map(str::trim).filter(|schema| !schema.is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            fluxdb_core::DamengDialect.quote_identifier(schema),
            fluxdb_core::DamengDialect.quote_identifier(name.trim())
        ),
        None => fluxdb_core::DamengDialect.quote_identifier(name.trim()),
    }
}

fn dameng_copy_table_sql(
    schema: Option<&str>,
    source_name: &str,
    target_name: &str,
    copy_data: bool,
    source_ddl: &str,
) -> Result<String, String> {
    let source = dameng_action_qualified(schema, source_name);
    let target = dameng_action_qualified(schema, target_name);
    let statements = dameng_split_sql_statements(source_ddl)?;
    if statements.is_empty() {
        return Err("未读取到可复制的达梦表结构 DDL".to_string());
    }
    let mut rewritten = statements
        .iter()
        .map(|sql| dameng_rewrite_copy_statement(sql, &source, &target, target_name))
        .collect::<Result<Vec<_>, _>>()?;
    if copy_data {
        let columns = dameng_copy_table_columns(source_ddl)?;
        let column_list = columns
            .iter()
            .map(|column| fluxdb_core::DamengDialect.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let identity_columns = dameng_copy_identity_columns(source_ddl)?;
        let has_identity = !identity_columns.is_empty();
        let insert = if column_list.is_empty() {
            format!("INSERT INTO {target} SELECT * FROM {source};")
        } else {
            format!(
                "INSERT INTO {target} ({column_list}) SELECT {column_list} FROM {source};"
            )
        };
        if has_identity {
            // DM 只在 SET IDENTITY_INSERT 为 ON 且语句带列清单时才允许给自增列赋值。
            rewritten.push(format!("SET IDENTITY_INSERT {target} ON;"));
            rewritten.push(insert);
            rewritten.push(format!("SET IDENTITY_INSERT {target} OFF;"));
        } else {
            rewritten.push(insert);
        }
    }
    Ok(rewritten.join("\n"))
}

/// 从建表 DDL 里取字段名列表（保持 DDL 中的顺序）。
fn dameng_copy_table_columns(source_ddl: &str) -> Result<Vec<String>, String> {
    let Some(open) = source_ddl.find('(') else {
        return Ok(Vec::new());
    };
    let body = &source_ddl[open + 1..];
    let mut columns = Vec::new();
    for entry in dameng_split_top_level_entries(body) {
        let trimmed = entry.trim();
        let upper = trimmed.to_ascii_uppercase();
        if trimmed.is_empty()
            || upper.starts_with("CONSTRAINT")
            || upper.starts_with("PRIMARY")
            || upper.starts_with("FOREIGN")
            || upper.starts_with("CHECK")
            || upper.starts_with("UNIQUE")
        {
            continue;
        }
        let Some(name) = dameng_ddl_identifier(trimmed) else {
            continue;
        };
        columns.push(name);
    }
    Ok(columns)
}

/// 从建表 DDL 里取自增字段名列表。
fn dameng_copy_identity_columns(source_ddl: &str) -> Result<Vec<String>, String> {
    let Some(open) = source_ddl.find('(') else {
        return Ok(Vec::new());
    };
    let body = &source_ddl[open + 1..];
    let mut identity = Vec::new();
    for entry in dameng_split_top_level_entries(body) {
        let trimmed = entry.trim();
        let upper = trimmed.to_ascii_uppercase();
        if !upper.contains("IDENTITY") && !upper.contains("AUTO_INCREMENT") {
            continue;
        }
        let Some(name) = dameng_ddl_identifier(trimmed) else {
            continue;
        };
        identity.push(name);
    }
    Ok(identity)
}

/// 取一条 DDL 条目的第一个标识符（跳过引号与空白）。
fn dameng_ddl_identifier(entry: &str) -> Option<String> {
    let trimmed = entry.trim_start();
    let first = trimmed.chars().next()?;
    if matches!(first, '"' | '`') {
        let end = trimmed[1..].find(first)? + 2;
        return Some(trimmed[1..end - 1].replace(&format!("{first}{first}"), &first.to_string()));
    }
    trimmed
        .split_whitespace()
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
}

/// 按顶层逗号切分 DDL 字段列表（忽略括号与引号内的逗号）。
fn dameng_split_top_level_entries(body: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in body.char_indices() {
        if let Some(end_quote) = quote {
            if ch == end_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                let entry = body[start..index].trim();
                if !entry.is_empty() {
                    entries.push(entry.to_string());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let entry = body[start..].trim();
    if !entry.is_empty() {
        entries.push(entry.to_string());
    }
    entries
}

fn dameng_split_sql_statements(sql: &str) -> Result<Vec<String>, String> {
    let mut statements = Vec::new();
    let mut start = 0usize;
    let mut quote = None;
    for (index, ch) in sql.char_indices() {
        if let Some(end_quote) = quote {
            if ch == end_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ';' => {
                let statement = sql[start..index].trim();
                if !statement.is_empty() {
                    statements.push(format!("{statement};"));
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() {
        return Err("达梦表结构 DDL 引号未闭合".to_string());
    }
    let tail = sql[start..].trim();
    if !tail.is_empty() {
        statements.push(format!("{tail};"));
    }
    Ok(statements)
}

fn dameng_rewrite_copy_statement(
    statement: &str,
    source: &str,
    target: &str,
    target_name: &str,
) -> Result<String, String> {
    let trimmed = statement.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("CREATE TABLE ") {
        return dameng_rewrite_create_table(trimmed, target, target_name);
    }
    if upper.starts_with("CREATE INDEX ")
        || upper.starts_with("CREATE UNIQUE INDEX ")
        || upper.starts_with("CREATE BITMAP INDEX ")
    {
        return dameng_rewrite_index(trimmed, target, target_name);
    }
    if upper.starts_with("COMMENT ON TABLE ") || upper.starts_with("COMMENT ON COLUMN ") {
        return Ok(trimmed.replacen(source, target, 1));
    }
    Ok(trimmed.to_string())
}

fn dameng_rewrite_create_table(
    statement: &str,
    target: &str,
    target_name: &str,
) -> Result<String, String> {
    let upper = statement.to_ascii_uppercase();
    let Some(create_pos) = upper.find("CREATE TABLE") else {
        return Err("达梦 DDL 缺少 CREATE TABLE".to_string());
    };
    let name_start = create_pos + "CREATE TABLE".len();
    let Some(paren_offset) = statement[name_start..].find('(') else {
        return Err("达梦 CREATE TABLE 缺少字段定义".to_string());
    };
    let body_start = name_start + paren_offset;
    let mut body = format!(
        "CREATE TABLE {target}\n{}",
        dameng_rename_constraint_names(&statement[body_start..], target_name)
    );
    if !body.trim_end().ends_with(';') {
        body.push(';');
    }
    Ok(body)
}

fn dameng_rename_constraint_names(statement: &str, target_name: &str) -> String {
    let mut output = String::with_capacity(statement.len());
    let mut rest = statement;
    while let Some(offset) = rest.to_ascii_uppercase().find("CONSTRAINT") {
        let keyword_end = offset + "CONSTRAINT".len();
        output.push_str(&rest[..keyword_end]);
        let after = rest[keyword_end..].trim_start();
        let whitespace = &rest[keyword_end..keyword_end + (rest[keyword_end..].len() - after.len())];
        output.push_str(whitespace);
        let Some(quote) = after.chars().next().filter(|ch| matches!(ch, '"' | '`')) else {
            output.push_str(after);
            return output;
        };
        let Some(end) = after[1..].find(quote).map(|index| index + 2) else {
            output.push_str(after);
            return output;
        };
        let old_name = &after[1..end - 1];
        let new_name = dameng_copy_object_name(target_name, old_name);
        output.push(quote);
        output.push_str(&new_name.replace(quote, &format!("{quote}{quote}")));
        output.push(quote);
        rest = &after[end..];
    }
    output.push_str(rest);
    output
}

fn dameng_rewrite_index(
    statement: &str,
    target: &str,
    target_name: &str,
) -> Result<String, String> {
    let upper = statement.to_ascii_uppercase();
    let Some(index_pos) = upper.find("INDEX") else {
        return Err("达梦索引 DDL 缺少 INDEX".to_string());
    };
    let after_index = &statement[index_pos + "INDEX".len()..];
    let name_start = after_index
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, _)| index)
        .ok_or_else(|| "达梦索引 DDL 缺少索引名".to_string())?;
    let name_rest = &after_index[name_start..];
    let (old_name, old_end) = dameng_parse_quoted_identifier(name_rest)?;
    let before_name = &statement[..index_pos + "INDEX".len() + name_start];
    let after_name = &name_rest[old_end..];
    let upper_after = after_name.to_ascii_uppercase();
    let Some(on_pos) = upper_after.find(" ON ") else {
        return Err("达梦索引 DDL 缺少 ON".to_string());
    };
    let table_start = on_pos + " ON ".len();
    let table_rest = &after_name[table_start..];
    let table_end = table_rest
        .find(char::is_whitespace)
        .or_else(|| table_rest.find('('))
        .ok_or_else(|| "达梦索引 DDL 缺少索引字段".to_string())?;
    let new_name = dameng_copy_object_name(target_name, &old_name);
    Ok(format!(
        "{before_name}\"{new_name}\"{} ON {target}{}",
        &after_name[..on_pos],
        &table_rest[table_end..]
    ))
}

fn dameng_parse_quoted_identifier(value: &str) -> Result<(String, usize), String> {
    let Some(quote) = value.chars().next().filter(|ch| matches!(ch, '"' | '`')) else {
        return Err("达梦 DDL 标识符未引用".to_string());
    };
    let end = value[1..]
        .find(quote)
        .map(|index| index + 2)
        .ok_or_else(|| "达梦 DDL 标识符引用未闭合".to_string())?;
    Ok((value[1..end - 1].to_string(), end))
}

fn dameng_copy_object_name(target_name: &str, old_name: &str) -> String {
    format!("{}_{}", target_name.trim(), old_name.trim())
        .chars()
        .take(128)
        .collect()
}

#[cfg(test)]
mod dameng_table_action_tests {
    use super::*;

    #[test]
    fn dameng_actions_are_schema_qualified_and_rename_uses_bare_target() {
        assert_eq!(
            rename_table_sql_preview(DatabaseKind::Dameng, Some("APPS"), "ORDERS", "ORDERS_2")
                .expect("rename SQL"),
            "ALTER TABLE \"APPS\".\"ORDERS\" RENAME TO \"ORDERS_2\";"
        );
        assert_eq!(
            drop_table_sql_preview(
                DatabaseKind::Dameng,
                ObjectKind::Table,
                Some("APPS"),
                "ORDERS",
                ForeignKeyCheckMode::Default,
            )
            .expect("drop SQL"),
            "DROP TABLE \"APPS\".\"ORDERS\";"
        );
        assert_eq!(
            truncate_table_sql_preview(
                DatabaseKind::Dameng,
                Some("APPS"),
                "ORDERS",
                false,
                ForeignKeyCheckMode::Default,
            )
            .expect("truncate SQL"),
            "TRUNCATE TABLE \"APPS\".\"ORDERS\";"
        );
    }

    #[test]
    fn dameng_copy_rewrites_table_constraint_index_and_comment_names() {
        let ddl = "CREATE TABLE \"APPS\".\"ORDERS\" (\n  \"ID\" INT IDENTITY(1,1) NOT NULL,\n  \"NAME\" VARCHAR(50),\n  CONSTRAINT \"FK_ORDERS_CUSTOMER\" FOREIGN KEY (\"ID\") REFERENCES \"APPS\".\"CUSTOMERS\" (\"ID\")\n);\nCREATE UNIQUE INDEX \"UK_ORDERS_NAME\" ON \"APPS\".\"ORDERS\" (\"NAME\");\nCOMMENT ON TABLE \"APPS\".\"ORDERS\" IS 'orders';\nCOMMENT ON COLUMN \"APPS\".\"ORDERS\".\"NAME\" IS 'name';";
        let sql = copy_table_sql_preview_with_source_ddl(
            DatabaseKind::Dameng,
            Some("APPS"),
            "ORDERS",
            "ORDERS_COPY",
            true,
            Some(ddl),
        )
        .expect("copy SQL");

        assert!(sql.contains("CREATE TABLE \"APPS\".\"ORDERS_COPY\""));
        assert!(sql.contains("CONSTRAINT \"ORDERS_COPY_FK_ORDERS_CUSTOMER\""));
        assert!(sql.contains("REFERENCES \"APPS\".\"CUSTOMERS\""));
        assert!(sql.contains("CREATE UNIQUE INDEX \"ORDERS_COPY_UK_ORDERS_NAME\" ON \"APPS\".\"ORDERS_COPY\""));
        assert!(sql.contains("COMMENT ON TABLE \"APPS\".\"ORDERS_COPY\" IS 'orders';"));
        assert!(sql.contains("COMMENT ON COLUMN \"APPS\".\"ORDERS_COPY\".\"NAME\" IS 'name';"));
        assert!(sql.contains("SET IDENTITY_INSERT \"APPS\".\"ORDERS_COPY\" ON;"));
        assert!(sql.contains(
            "INSERT INTO \"APPS\".\"ORDERS_COPY\" (\"ID\", \"NAME\") SELECT \"ID\", \"NAME\" FROM \"APPS\".\"ORDERS\";"
        ));
        assert!(sql.contains("SET IDENTITY_INSERT \"APPS\".\"ORDERS_COPY\" OFF;"));
    }

    #[test]
    fn dameng_copy_keeps_generated_primary_index_column_once() {
        let ddl = "CREATE TABLE \"FLUXSALES\".\"SALES_ORDER\" (\n  \"ORDER_ID\" BIGINT NOT NULL,\n  CONSTRAINT \"CONS1\" PRIMARY KEY (\"ORDER_ID\")\n);\nCREATE UNIQUE INDEX \"INDEX33555634\" ON \"FLUXSALES\".\"SALES_ORDER\" (\"ORDER_ID\");\nCREATE UNIQUE INDEX \"INDEX33555635\" ON \"FLUXSALES\".\"SALES_ORDER\" (\"ORDER_NO\");";
        let sql = copy_table_sql_preview_with_source_ddl(
            DatabaseKind::Dameng,
            Some("FLUXSALES"),
            "SALES_ORDER",
            "SALES_ORDER_copy",
            false,
            Some(ddl),
        )
        .expect("copy SQL");
        assert!(sql.contains(
            "CREATE UNIQUE INDEX \"SALES_ORDER_copy_INDEX33555634\" ON \"FLUXSALES\".\"SALES_ORDER_copy\" (\"ORDER_ID\");"
        ));
        assert!(!sql.contains("\"ORDER_ID\", \"ORDER_ID\""));
    }

    #[test]
    fn dameng_copy_accepts_formatted_ddl() {
        let ddl = "CREATE TABLE \"APPS\".\"ORDERS\" (\n  \"ID\" INT IDENTITY(1,1) NOT NULL,\n  \"NAME\" VARCHAR(50)\n);";
        let formatted = format_sql_text_for_dialect(ddl, DatabaseKind::Dameng);
        let sql = copy_table_sql_preview_with_source_ddl(
            DatabaseKind::Dameng,
            Some("APPS"),
            "ORDERS",
            "ORDERS_COPY",
            true,
            Some(&formatted),
        )
        .expect("copy formatted DDL");
        assert!(sql.contains("IDENTITY"));
        assert!(sql.contains("SET IDENTITY_INSERT \"APPS\".\"ORDERS_COPY\" ON;"));
    }
}

#[cfg(test)]
mod oracle_table_action_tests {
    use super::*;

    #[test]
    fn oracle_copy_rewrites_table_index_and_comment_to_target() {
        // UI 与 controller 共用同一判定：漏掉 Oracle 会让 source_ddl 恒为 None。
        assert!(copy_table_requires_source_ddl(DatabaseKind::Oracle));
        let ddl = "CREATE TABLE \"FLUXDB_DEMO\".\"CLOB_SAMPLES\" (\n  \"ID\" NUMBER NOT NULL,\n  \"DATA\" CLOB,\n  CONSTRAINT \"PK_CLOB_SAMPLES\" PRIMARY KEY (\"ID\")\n);\nCREATE INDEX \"IX_CLOB_SAMPLES_DATA\" ON \"FLUXDB_DEMO\".\"CLOB_SAMPLES\" (\"DATA\");\nCOMMENT ON TABLE \"FLUXDB_DEMO\".\"CLOB_SAMPLES\" IS 'samples';";
        let sql = copy_table_sql_preview_with_source_ddl(
            DatabaseKind::Oracle,
            Some("FLUXDB_DEMO"),
            "CLOB_SAMPLES",
            "CLOB_SAMPLES_copy",
            false,
            Some(ddl),
        )
        .expect("copy SQL");

        assert!(sql.contains("CREATE TABLE \"FLUXDB_DEMO\".\"CLOB_SAMPLES_copy\""));
        assert!(sql.contains("CONSTRAINT \"CLOB_SAMPLES_copy_PK_CLOB_SAMPLES\""));
        assert!(sql.contains("CREATE INDEX \"CLOB_SAMPLES_copy_IX_CLOB_SAMPLES_DATA\""));
        assert!(sql.contains("COMMENT ON TABLE \"FLUXDB_DEMO\".\"CLOB_SAMPLES_copy\" IS 'samples';"));
        // 仅结构：不生成插数语句。
        assert!(!sql.contains("INSERT INTO"));
    }
}

#[cfg(test)]
mod sqlserver_table_action_tests {
    use super::*;

    #[test]
    fn sqlserver_rename_drop_and_truncate_are_schema_qualified() {
        assert_eq!(
            rename_table_sql_preview(DatabaseKind::SqlServer, Some("sales"), "orders", "orders_2")
                .expect("rename SQL"),
            "EXEC sp_rename N'[sales].[orders]', N'orders_2', 'OBJECT';"
        );
        assert!(rename_table_sql_preview(
            DatabaseKind::SqlServer,
            Some("sales"),
            "orders",
            "dbo.orders_2",
        )
        .is_err());
        assert_eq!(
            drop_table_sql_preview(
                DatabaseKind::SqlServer,
                ObjectKind::View,
                Some("sales"),
                "orders_view",
                ForeignKeyCheckMode::Default,
            )
            .expect("drop view SQL"),
            "DROP VIEW [sales].[orders_view];"
        );
        assert_eq!(
            truncate_table_sql_preview(
                DatabaseKind::SqlServer,
                Some("sales"),
                "orders",
                false,
                ForeignKeyCheckMode::Default,
            )
            .expect("truncate SQL"),
            "TRUNCATE TABLE [sales].[orders];"
        );
        assert!(drop_table_sql_preview(
            DatabaseKind::SqlServer,
            ObjectKind::Table,
            Some("sales"),
            "orders",
            ForeignKeyCheckMode::Disable,
        )
        .is_err());
    }

    #[test]
    fn sqlserver_copy_keeps_constraints_indexes_and_identity_values() {
        let ddl = "CREATE TABLE [sales].[orders] (\n  [id] int IDENTITY(1,1) NOT NULL,\n  [name] nvarchar(80) NULL,\n  CONSTRAINT [PK_orders] PRIMARY KEY ([id])\n);\nCREATE UNIQUE INDEX [IX_orders_name] ON [sales].[orders] ([name] ASC);";
        let sql = copy_table_sql_preview_with_source_ddl(
            DatabaseKind::SqlServer,
            Some("sales"),
            "orders",
            "orders_copy",
            true,
            Some(ddl),
        )
        .expect("copy SQL");

        assert!(sql.contains("CREATE TABLE [sales].[orders_copy]"));
        assert!(sql.contains("CONSTRAINT [orders_copy_PK_orders] PRIMARY KEY ([id])"));
        assert!(!sql.contains("SELECT * INTO"));
        assert!(sql.contains("CREATE UNIQUE INDEX [orders_copy_IX_orders_name] ON [sales].[orders_copy]"));
        assert!(sql.contains("SET IDENTITY_INSERT [sales].[orders_copy] ON;"));
        assert!(sql.contains(
            "INSERT INTO [sales].[orders_copy] ([id], [name]) SELECT [id], [name] FROM [sales].[orders];"
        ));
        assert!(sql.contains("SET IDENTITY_INSERT [sales].[orders_copy] OFF;"));
    }

    #[test]
    fn sqlserver_copy_accepts_formatted_bracket_identifiers() {
        // 展示层 sqlformat 曾经把 `[id]` 变成 `[ id ]`，复制表会把它当作真实列名。
        // SQL Server DDL 必须先保护方括号标识符再格式化。
        let ddl = "CREATE TABLE [dbo].[orders] (\n  [id] int IDENTITY(1,1) NOT NULL,\n  [name] nvarchar(80) NULL,\n  CONSTRAINT [PK_orders] PRIMARY KEY ([id])\n);";
        let formatted = format_sql_text_for_dialect(ddl, DatabaseKind::SqlServer);
        assert!(formatted.contains("[id]"), "{formatted}");
        assert!(!formatted.contains("[ id ]"), "{formatted}");
        let sql = copy_table_sql_preview_with_source_ddl(
            DatabaseKind::SqlServer,
            Some("dbo"),
            "orders",
            "orders_copy",
            true,
            Some(&formatted),
        )
        .expect("copy formatted SQL Server DDL");
        assert!(!sql.contains("[ id ]"), "{sql}");
        assert!(sql.contains("[id]"), "{sql}");
    }

    #[test]
    fn sqlserver_copy_renames_foreign_key_constraints() {
        let ddl = "CREATE TABLE [finance].[order_items] (\n  [id] int NOT NULL,\n  [order_id] int NOT NULL,\n  CONSTRAINT [PK_order_items] PRIMARY KEY ([id])\n);\nALTER TABLE [finance].[order_items] ADD CONSTRAINT [fk_oi_order] FOREIGN KEY ([order_id]) REFERENCES [finance].[orders] ([id]);";
        let sql = copy_table_sql_preview_with_source_ddl(
            DatabaseKind::SqlServer,
            Some("finance"),
            "order_items",
            "order_items_copy",
            false,
            Some(ddl),
        )
        .expect("copy SQL with foreign key");

        assert!(sql.contains(
            "ALTER TABLE [finance].[order_items_copy] ADD CONSTRAINT [order_items_copy_fk_oi_order] FOREIGN KEY ([order_id]) REFERENCES [finance].[orders] ([id]);"
        ));
        assert!(!sql.contains("ADD CONSTRAINT [fk_oi_order]"));
    }
}
