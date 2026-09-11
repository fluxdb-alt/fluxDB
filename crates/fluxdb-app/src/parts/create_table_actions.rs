struct MySqlTableActionSqlProvider;
struct SqliteTableActionSqlProvider;
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
            quote_mysql_identifier(old_name),
            quote_mysql_identifier(new_name)
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
        let source = quote_mysql_identifier(source_name);
        let target = quote_mysql_identifier(target_name);
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
        Ok(format!("DROP TABLE {};", quote_mysql_identifier(table_name)))
    }

    fn truncate_table_sql(
        &self,
        _: Option<&str>,
        table_name: &str,
        _: bool,
    ) -> Result<String, String> {
        Ok(format!(
            "TRUNCATE TABLE {};",
            quote_mysql_identifier(table_name)
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
            quote_sqlite_identifier(old_name),
            quote_sqlite_identifier(new_name)
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
        let source = quote_sqlite_identifier(source_name);
        let target = quote_sqlite_identifier(target_name);
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
        Ok(format!("DROP TABLE {};", quote_sqlite_identifier(table_name)))
    }

    fn truncate_table_sql(
        &self,
        _: Option<&str>,
        table_name: &str,
        _: bool,
    ) -> Result<String, String> {
        Ok(format!("DELETE FROM {};", quote_sqlite_identifier(table_name)))
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
    sql.push_str(&quote_sqlite_identifier(target_name));
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
static POSTGRES_TABLE_ACTION_SQL_PROVIDER: PostgresTableActionSqlProvider =
    PostgresTableActionSqlProvider;
static UNSUPPORTED_TABLE_ACTION_SQL_PROVIDER: UnsupportedTableActionSqlProvider =
    UnsupportedTableActionSqlProvider;

fn table_action_sql_provider(database_kind: DatabaseKind) -> &'static dyn TableActionSqlProvider {
    match database_kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => &MYSQL_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::Sqlite => &SQLITE_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::Postgres => &POSTGRES_TABLE_ACTION_SQL_PROVIDER,
        DatabaseKind::MongoDb | DatabaseKind::Redis => &UNSUPPORTED_TABLE_ACTION_SQL_PROVIDER,
    }
}

