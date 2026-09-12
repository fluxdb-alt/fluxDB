fn validate_data_changes(changes: &DataChangeSet) -> fluxdb_core::Result<()> {
    if changes.object.kind != ObjectKind::Table {
        return Err(Error::new(ErrorKind::Unsupported, "仅表对象支持数据提交"));
    }

    if changes.is_empty() {
        return Err(Error::new(ErrorKind::Query, "没有需要提交的更改"));
    }

    Ok(())
}

fn validate_identity(identity: &fluxdb_core::RowIdentity, columns: &[Column]) -> fluxdb_core::Result<()> {
    if identity.values.is_empty() {
        return Err(Error::new(ErrorKind::Query, "缺少行身份条件，已取消提交"));
    }

    for column in identity.values.keys() {
        ensure_column_exists(column, columns)?;
    }

    Ok(())
}

fn ensure_column_exists(column: &str, columns: &[Column]) -> fluxdb_core::Result<()> {
    if columns.iter().any(|existing| existing.name == column) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::Query,
            format!("字段不存在或已变更：{column}"),
        ))
    }
}

fn non_null_insert_values<'a>(
    row: &'a Row,
    columns: &'a [Column],
) -> fluxdb_core::Result<Vec<(&'a Column, &'a CellValue)>> {
    if row.values.len() != columns.len() {
        return Err(Error::new(
            ErrorKind::Query,
            "新增行字段数量和当前表结构不一致",
        ));
    }

    Ok(columns
        .iter()
        .zip(row.values.iter())
        .filter(|(_, value)| !matches!(value, CellValue::Null))
        .collect())
}

fn push_insert_column_list<DB>(
    builder: &mut QueryBuilder<'_, DB>,
    values: &[(&Column, &CellValue)],
    quote_identifier: fn(&str) -> String,
) where
    DB: sqlx::Database,
{
    for (index, (column, _)) in values.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push(quote_identifier(&column.name));
    }
}

fn push_mysql_bind(builder: &mut QueryBuilder<'_, MySql>, value: &CellValue) {
    match value {
        CellValue::Null => {
            builder.push_bind(Option::<String>::None);
        }
        CellValue::Bool(value) => {
            builder.push_bind(*value);
        }
        CellValue::I64(value) => {
            builder.push_bind(*value);
        }
        CellValue::F64(value) => {
            builder.push_bind(*value);
        }
        CellValue::Text(value) | CellValue::Json(value) => {
            builder.push_bind(value.clone());
        }
        CellValue::Bytes(value) => {
            builder.push_bind(value.clone());
        }
        CellValue::BinarySummary(_) => {
            builder.push_bind(Option::<Vec<u8>>::None);
        }
    }
}

fn push_sqlite_bind(builder: &mut QueryBuilder<'_, Sqlite>, value: &CellValue) {
    match value {
        CellValue::Null => {
            builder.push_bind(Option::<String>::None);
        }
        CellValue::Bool(value) => {
            builder.push_bind(*value);
        }
        CellValue::I64(value) => {
            builder.push_bind(*value);
        }
        CellValue::F64(value) => {
            builder.push_bind(*value);
        }
        CellValue::Text(value) | CellValue::Json(value) => {
            builder.push_bind(value.clone());
        }
        CellValue::Bytes(value) => {
            builder.push_bind(value.clone());
        }
        CellValue::BinarySummary(_) => {
            builder.push_bind(Option::<Vec<u8>>::None);
        }
    }
}

fn push_mysql_identity_where(
    builder: &mut QueryBuilder<'_, MySql>,
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    push_identity_where(
        builder,
        identity,
        columns,
        mysql_quote_identifier,
        push_mysql_bind,
    )
}

fn push_sqlite_identity_where(
    builder: &mut QueryBuilder<'_, Sqlite>,
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
) -> fluxdb_core::Result<()> {
    push_identity_where(
        builder,
        identity,
        columns,
        sqlite_quote_identifier,
        push_sqlite_bind,
    )
}

fn push_identity_where<DB>(
    builder: &mut QueryBuilder<'_, DB>,
    identity: &fluxdb_core::RowIdentity,
    columns: &[Column],
    quote_identifier: fn(&str) -> String,
    push_bind: fn(&mut QueryBuilder<'_, DB>, &CellValue),
) -> fluxdb_core::Result<()>
where
    DB: sqlx::Database,
{
    validate_identity(identity, columns)?;
    builder.push(" WHERE ");

    for (index, (column, value)) in identity.values.iter().enumerate() {
        if index > 0 {
            builder.push(" AND ");
        }
        builder.push(quote_identifier(column));
        if matches!(value, CellValue::Null) {
            builder.push(" IS NULL");
        } else {
            builder.push(" = ");
            push_bind(builder, value);
        }
    }

    Ok(())
}

fn mysql_quote_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn mysql_create_database_sql(request: &CreateDatabaseRequest) -> fluxdb_core::Result<String> {
    let database = request.name.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "数据库名称不能为空"));
    }

    let charset = request.charset.trim();
    if charset.is_empty() {
        return Err(Error::new(ErrorKind::Query, "字符集不能为空"));
    }
    if !is_mysql_option_name(charset) {
        return Err(Error::new(ErrorKind::Query, "字符集名称不合法"));
    }

    let collation = request.collation.trim();
    if collation.is_empty() {
        return Err(Error::new(ErrorKind::Query, "排序规则不能为空"));
    }
    if !is_mysql_option_name(collation) {
        return Err(Error::new(ErrorKind::Query, "排序规则名称不合法"));
    }

    Ok(format!(
        "CREATE DATABASE {} DEFAULT CHARACTER SET {} COLLATE {}",
        mysql_quote_identifier(database),
        charset,
        collation
    ))
}

fn mysql_delete_database_sql(database: &str) -> fluxdb_core::Result<String> {
    let database = database.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "数据库名称不能为空"));
    }

    Ok(format!("DROP DATABASE {}", mysql_quote_identifier(database)))
}

fn is_mysql_option_name(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn mysql_create_database(
    config: &ConnectionConfig,
    request: &CreateDatabaseRequest,
) -> fluxdb_core::Result<()> {
    if !is_mysql_protocol_kind(config.kind) {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持新建数据库"));
    }
    if config.id != request.connection_id {
        return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
    }

    let sql = mysql_create_database_sql(request)?;
    let mut server_config = config.clone();
    if let Endpoint::Tcp { database, .. } = &mut server_config.endpoint {
        *database = None;
    }
    let url = mysql_connection_url(&server_config)?;
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), options.connect()).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(mysql_error(error)),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        sqlx::query(&sql)
            .execute(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)
    })
}

fn mysql_delete_database(
    config: &ConnectionConfig,
    connection_id: ConnectionId,
    database: &str,
) -> fluxdb_core::Result<()> {
    if !is_mysql_protocol_kind(config.kind) {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持删除数据库"));
    }
    if config.id != connection_id {
        return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
    }

    let sql = mysql_delete_database_sql(database)?;
    let mut server_config = config.clone();
    if let Endpoint::Tcp { database, .. } = &mut server_config.endpoint {
        *database = None;
    }
    let url = mysql_connection_url(&server_config)?;
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), options.connect()).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(mysql_error(error)),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        sqlx::query(&sql)
            .execute(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)
    })
}

fn sqlite_quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn sqlite_quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
