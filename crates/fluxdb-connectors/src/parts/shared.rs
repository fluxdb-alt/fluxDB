/// 压缩用于日志的 SQL 文本：去除空白并截断到 400 字符，
/// 避免超长/多语句查询刷屏且不泄露过多内容。
fn truncate_sql_for_log(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 400 {
        let mut truncated: String = compact.chars().take(400).collect();
        truncated.push_str("…[截断]");
        truncated
    } else {
        compact
    }
}

/// 等待驱动 future，同时轮询取消回调。
/// 返回 `Ok(None)` 表示 future 被取消；调用方必须丢弃当前连接，避免复用半途状态。
async fn await_with_cancel<T, E, F>(
    future: F,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Option<T>, E>
where
    F: Future<Output = Result<T, E>>,
{
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => {
                let value = result?;
                return if should_cancel() { Ok(None) } else { Ok(Some(value)) };
            },
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                if should_cancel() {
                    return Ok(None);
                }
            }
        }
    }
}

fn database_object(connection_id: ConnectionId, database: &str) -> ObjectSummary {
    ObjectSummary {
        path: ObjectPath {
            connection_id,
            database: Some(database.to_string()),
            schema: None,
            name: database.to_string(),
            kind: ObjectKind::Database,
        },
        rows: None,
        modified_at: None,
        comment: None,
    }
}

fn completion_database(
    config: &ConnectionConfig,
    database: Option<&str>,
) -> fluxdb_core::Result<String> {
    database
        .map(str::to_string)
        .or_else(|| match &config.endpoint {
            Endpoint::Tcp { database, .. } => database.clone(),
            _ => None,
        })
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Connection, "缺少数据库上下文"))
}

fn completion_like_filter(filter: &str) -> String {
    format!("{}%", filter.trim().to_ascii_lowercase())
}

fn completion_fuzzy_like_filter(filter: &str) -> String {
    let filter = filter.trim().to_ascii_lowercase();
    if filter.is_empty() {
        return "%".to_string();
    }

    let mut pattern = String::from("%");
    for ch in filter.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
        pattern.push('%');
    }
    pattern
}

fn matches_completion_filter(value: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty()
        || value
            .to_ascii_lowercase()
            .starts_with(&filter.to_ascii_lowercase())
}

fn matches_completion_fuzzy_filter(value: &str, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }

    let value = value.to_ascii_lowercase();
    let mut value_chars = value.chars();
    filter
        .to_ascii_lowercase()
        .chars()
        .all(|filter_char| value_chars.any(|value_char| value_char == filter_char))
}

fn columns_to_completion(table: &str, columns: Vec<Column>) -> Vec<CompletionColumn> {
    columns
        .into_iter()
        .map(|column| CompletionColumn {
            table: table.to_string(),
            name: column.name,
            type_name: column.type_name,
            nullable: column.nullable,
            primary_key: column.primary_key,
            comment: column.comment,
        })
        .collect()
}

fn mysql_select_expr(column: &Column) -> String {
    let name = mysql_quote_identifier(&column.name);
    if is_binary_column(column) {
        let is_null = mysql_quote_identifier(&binary_alias(&column.name, "is_null"));
        let byte_length = mysql_quote_identifier(&binary_alias(&column.name, "byte_length"));
        let preview_hex = mysql_quote_identifier(&binary_alias(&column.name, "preview_hex"));
        return format!(
            "CAST({name} IS NULL AS UNSIGNED) AS {is_null}, COALESCE(OCTET_LENGTH({name}), 0) AS {byte_length}, UPPER(HEX(SUBSTRING({name}, 1, 64))) AS {preview_hex}"
        );
    }

    format!("CAST({name} AS CHAR) AS {name}")
}

fn sqlite_select_expr(column: &Column) -> String {
    let name = sqlite_quote_identifier(&column.name);
    if is_binary_column(column) {
        let is_null = sqlite_quote_identifier(&binary_alias(&column.name, "is_null"));
        let byte_length = sqlite_quote_identifier(&binary_alias(&column.name, "byte_length"));
        let preview_hex = sqlite_quote_identifier(&binary_alias(&column.name, "preview_hex"));
        return format!(
            "{name} IS NULL AS {is_null}, COALESCE(length({name}), 0) AS {byte_length}, upper(hex(substr({name}, 1, 64))) AS {preview_hex}"
        );
    }

    format!("CAST({name} AS TEXT) AS {name}")
}

fn is_binary_column(column: &Column) -> bool {
    column.type_name.as_deref().is_some_and(is_binary_type_name)
}

fn binary_alias(column: &str, suffix: &str) -> String {
    format!("{column}__{suffix}")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn mysql_rows_to_page(
    columns: Vec<Column>,
    rows: Vec<MySqlRow>,
    pagination: Pagination,
) -> DataPage {
    let has_more = rows.len() > pagination.limit as usize;
    let rows = rows
        .into_iter()
        .take(pagination.limit as usize)
        .map(|row| Row {
            values: (0..columns.len())
                .map(|index| mysql_cell_value(&row, &columns[index]))
                .collect(),
        })
        .collect();

    DataPage {
        columns,
        rows,
        offset: pagination.offset,
        limit: pagination.limit,
        has_more,
    }
}

fn sqlite_rows_to_page(
    columns: Vec<Column>,
    rows: Vec<SqliteRow>,
    pagination: Pagination,
) -> DataPage {
    let has_more = rows.len() > pagination.limit as usize;
    let rows = rows
        .into_iter()
        .take(pagination.limit as usize)
        .map(|row| Row {
            values: (0..columns.len())
                .map(|index| sqlite_cell_value(&row, &columns[index]))
                .collect(),
        })
        .collect();

    DataPage {
        columns,
        rows,
        offset: pagination.offset,
        limit: pagination.limit,
        has_more,
    }
}

fn mysql_cell_value(row: &MySqlRow, column: &Column) -> CellValue {
    if is_binary_column(column) {
        return mysql_binary_summary(row, column);
    }

    mysql_plain_cell_value(row, column)
}

fn mysql_plain_cell_value(row: &MySqlRow, column: &Column) -> CellValue {
    mysql_plain_cell_value_by(row, column.name.as_str())
}

fn mysql_plain_cell_value_by<I>(row: &MySqlRow, index: I) -> CellValue
where
    I: ColumnIndex<MySqlRow> + Copy,
{
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<u64>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<BigDecimal>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<DateTime<Utc>>, _>(index) {
        return mysql_datetime_cell_value(value.map(|value| value.naive_utc()));
    }
    if let Ok(value) = row.try_get::<Option<NaiveDateTime>, _>(index) {
        return mysql_datetime_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<NaiveDate>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<NaiveTime>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<MySqlTime>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value
            .map(mysql_query_bytes_cell_value)
            .unwrap_or(CellValue::Null);
    }

    CellValue::Null
}

fn mysql_optional_display_cell_value(value: Option<impl ToString>) -> CellValue {
    value
        .map(|value| CellValue::Text(value.to_string()))
        .unwrap_or(CellValue::Null)
}

fn sqlite_cell_value(row: &SqliteRow, column: &Column) -> CellValue {
    if is_binary_column(column) {
        return sqlite_binary_summary(row, column);
    }

    row.try_get::<Option<String>, _>(column.name.as_str())
        .map(|value| value.map(CellValue::Text).unwrap_or(CellValue::Null))
        .unwrap_or_else(|_| {
            row.try_get::<Option<Vec<u8>>, _>(column.name.as_str())
                .map(|value| value.map(CellValue::Bytes).unwrap_or(CellValue::Null))
                .unwrap_or(CellValue::Null)
        })
}

fn mysql_binary_summary(row: &MySqlRow, column: &Column) -> CellValue {
    let is_null_alias = binary_alias(&column.name, "is_null");
    let byte_length_alias = binary_alias(&column.name, "byte_length");
    let preview_hex_alias = binary_alias(&column.name, "preview_hex");
    let is_null = row
        .try_get::<u64, _>(is_null_alias.as_str())
        .map(|value| value != 0)
        .unwrap_or(false);
    let byte_length = row
        .try_get::<u64, _>(byte_length_alias.as_str())
        .unwrap_or_default();
    let preview_hex = row
        .try_get::<Option<String>, _>(preview_hex_alias.as_str())
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());

    CellValue::BinarySummary(binary_summary(column, is_null, byte_length, preview_hex))
}

fn sqlite_binary_summary(row: &SqliteRow, column: &Column) -> CellValue {
    let is_null_alias = binary_alias(&column.name, "is_null");
    let byte_length_alias = binary_alias(&column.name, "byte_length");
    let preview_hex_alias = binary_alias(&column.name, "preview_hex");
    let is_null = row
        .try_get::<i64, _>(is_null_alias.as_str())
        .map(|value| value != 0)
        .unwrap_or(false);
    let byte_length = row
        .try_get::<i64, _>(byte_length_alias.as_str())
        .map(|value| value.max(0) as u64)
        .unwrap_or_default();
    let preview_hex = row
        .try_get::<Option<String>, _>(preview_hex_alias.as_str())
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());

    CellValue::BinarySummary(binary_summary(column, is_null, byte_length, preview_hex))
}

fn binary_summary(
    column: &Column,
    is_null: bool,
    byte_length: u64,
    preview_hex: Option<String>,
) -> BinaryCellSummary {
    BinaryCellSummary {
        type_name: column
            .type_name
            .clone()
            .unwrap_or_else(|| "BINARY".to_string()),
        is_null,
        byte_length,
        preview_hex,
    }
}

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

fn mysql_execute_query(
    config: &ConnectionConfig,
    request: &QueryRequest,
) -> fluxdb_core::Result<QueryExecutionResult> {
    mysql_execute_query_with_progress(config, request, &mut |_| {}, &|| false)
}

fn mysql_execute_query_with_progress(
    config: &ConnectionConfig,
    request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statements = query_statements_for_execution(request);
    if statements.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }

    let url = mysql_connection_url(config)?;
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

        if let Some(database) = request.database.as_deref().filter(|database| !database.is_empty())
        {
            sqlx::raw_sql(&format!("USE {}", mysql_quote_identifier(database)))
                .execute(&mut connection)
                .await
                .map_err(mysql_error)?;
        }

        let mut execution = QueryExecutionResult {
            summaries: Vec::new(),
            results: Vec::new(),
            rollback_snapshots: Vec::new(),
        };
        for statement in statements {
            if should_cancel() {
                break;
            }
            let started = std::time::Instant::now();
            if statement_returns_rows(&statement) {
                let rows = match sqlx::query(&statement).fetch_all(&mut connection).await {
                    Ok(rows) => rows,
                    Err(error) => {
                        let error = mysql_error(error);
                        push_query_summary(
                            &mut execution,
                            failed_query_summary(
                                statement,
                                QueryStatementKind::ResultSet,
                                error.message,
                                elapsed_ms(started),
                            ),
                            on_summary,
                        );
                        if !request.options.continue_on_error {
                            break;
                        }
                        continue;
                    }
                };
                let elapsed_ms = elapsed_ms(started);
                let page = mysql_query_rows_to_page(rows, request.options.page_offset, request.options.page_size);
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: QueryStatementKind::ResultSet,
                        success: true,
                        message: format!("返回 {} 行结果表", page.rows.len()),
                        returned_rows: page.rows.len() as u64,
                        affected_rows: 0,
                        elapsed_ms,
                    },
                    on_summary,
                );
                execution.results.push(page);
            } else {
                let result = match sqlx::query(&statement).execute(&mut connection).await {
                    Ok(result) => result,
                    Err(error) => {
                        let error = mysql_error(error);
                        push_query_summary(
                            &mut execution,
                            failed_query_summary(
                                statement,
                                QueryStatementKind::Command,
                                error.message,
                                elapsed_ms(started),
                            ),
                            on_summary,
                        );
                        if !request.options.continue_on_error {
                            break;
                        }
                        continue;
                    }
                };
                let affected_rows = result.rows_affected();
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: QueryStatementKind::Command,
                        success: true,
                        message: "OK".to_string(),
                        returned_rows: 0,
                        affected_rows,
                        elapsed_ms: elapsed_ms(started),
                    },
                    on_summary,
                );
            }
        }

        connection.close().await.map_err(mysql_error)?;
        Ok(execution)
    })
}

fn sqlite_execute_query(
    config: &ConnectionConfig,
    request: &QueryRequest,
) -> fluxdb_core::Result<QueryExecutionResult> {
    sqlite_execute_query_with_progress(config, request, &mut |_| {}, &|| false)
}

fn sqlite_execute_query_with_progress(
    config: &ConnectionConfig,
    request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statements = query_statements_for_execution(request);
    if statements.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let connect = tokio::time::timeout(Duration::from_secs(5), sqlite_connect(config)).await;
        let mut connection = match connect {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
        };

        let mut execution = QueryExecutionResult {
            summaries: Vec::new(),
            results: Vec::new(),
            rollback_snapshots: Vec::new(),
        };
        for statement in statements {
            if should_cancel() {
                break;
            }
            let started = std::time::Instant::now();
            if statement_returns_rows(&statement) {
                let rows = match sqlx::query(&statement).fetch_all(&mut connection).await {
                    Ok(rows) => rows,
                    Err(error) => {
                        let error = sqlite_error(error);
                        push_query_summary(
                            &mut execution,
                            failed_query_summary(
                                statement,
                                QueryStatementKind::ResultSet,
                                error.message,
                                elapsed_ms(started),
                            ),
                            on_summary,
                        );
                        if !request.options.continue_on_error {
                            break;
                        }
                        continue;
                    }
                };
                let elapsed_ms = elapsed_ms(started);
                let page = sqlite_query_rows_to_page(rows, request.options.page_offset, request.options.page_size);
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: QueryStatementKind::ResultSet,
                        success: true,
                        message: format!("返回 {} 行结果表", page.rows.len()),
                        returned_rows: page.rows.len() as u64,
                        affected_rows: 0,
                        elapsed_ms,
                    },
                    on_summary,
                );
                execution.results.push(page);
            } else {
                let result = match sqlx::query(&statement).execute(&mut connection).await {
                    Ok(result) => result,
                    Err(error) => {
                        let error = sqlite_error(error);
                        push_query_summary(
                            &mut execution,
                            failed_query_summary(
                                statement,
                                QueryStatementKind::Command,
                                error.message,
                                elapsed_ms(started),
                            ),
                            on_summary,
                        );
                        if !request.options.continue_on_error {
                            break;
                        }
                        continue;
                    }
                };
                let affected_rows = result.rows_affected();
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: QueryStatementKind::Command,
                        success: true,
                        message: "OK".to_string(),
                        returned_rows: 0,
                        affected_rows,
                        elapsed_ms: elapsed_ms(started),
                    },
                    on_summary,
                );
            }
        }

        connection.close().await.map_err(sqlite_error)?;
        Ok(execution)
    })
}

fn mysql_query_rows_to_page(rows: Vec<MySqlRow>, offset: u64, limit: u64) -> DataPage {
    let columns = rows
        .first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|column| query_column(column.name(), column.type_info().name()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    query_rows_to_page(columns, rows, offset, limit, mysql_query_cell_value)
}

fn sqlite_query_rows_to_page(rows: Vec<SqliteRow>, offset: u64, limit: u64) -> DataPage {
    let columns = rows
        .first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|column| query_column(column.name(), column.type_info().name()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    query_rows_to_page(columns, rows, offset, limit, sqlite_query_cell_value)
}

fn failed_query_summary(
    sql: String,
    kind: QueryStatementKind,
    message: String,
    elapsed_ms: u64,
) -> QueryExecutionSummary {
    QueryExecutionSummary {
        sql,
        kind,
        success: false,
        message,
        returned_rows: 0,
        affected_rows: 0,
        elapsed_ms,
    }
}

fn push_query_summary(
    execution: &mut QueryExecutionResult,
    summary: QueryExecutionSummary,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
) {
    on_summary(summary.clone());
    execution.summaries.push(summary);
}

fn query_rows_to_page<R>(
    columns: Vec<Column>,
    rows: Vec<R>,
    offset: u64,
    limit: u64,
    cell_value: fn(&R, usize, &Column) -> CellValue,
) -> DataPage {
    let pagination = Pagination::new(offset, limit);
    let offset = pagination.offset.min(rows.len() as u64) as usize;
    let limit = pagination.limit as usize;
    let has_more = rows.len().saturating_sub(offset) > limit;
    let rows = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|row| Row {
            values: columns
                .iter()
                .enumerate()
                .map(|(index, column)| cell_value(&row, index, column))
                .collect(),
        })
        .collect();
    DataPage {
        columns,
        rows,
        offset: pagination.offset,
        limit: pagination.limit,
        has_more,
    }
}

fn query_column(name: &str, type_name: &str) -> Column {
    Column {
        name: name.to_string(),
        type_name: Some(type_name.to_string()),
        nullable: true,
        primary_key: false,
        comment: None,
    }
}

fn mysql_query_cell_value(row: &MySqlRow, index: usize, _column: &Column) -> CellValue {
    mysql_plain_cell_value_by(row, index)
}

const MYSQL_QUERY_TEXT_BYTES_LIMIT: usize = 256;

fn mysql_query_bytes_cell_value(value: Vec<u8>) -> CellValue {
    if value.len() <= MYSQL_QUERY_TEXT_BYTES_LIMIT {
        if let Ok(text) = std::str::from_utf8(&value) {
            if text.chars().all(is_printable_query_text_char) {
                return CellValue::Text(text.to_string());
            }
        }
    }
    CellValue::Bytes(value)
}

fn is_printable_query_text_char(ch: char) -> bool {
    !ch.is_control() || matches!(ch, '\n' | '\r' | '\t')
}

fn mysql_datetime_cell_value(value: Option<NaiveDateTime>) -> CellValue {
    value
        .map(|value| CellValue::Text(value.format("%Y-%m-%d %H:%M:%S%.f").to_string()))
        .unwrap_or(CellValue::Null)
}

fn sqlite_query_cell_value(row: &SqliteRow, index: usize, _column: &Column) -> CellValue {
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value.map(CellValue::I64).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return value.map(CellValue::F64).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return value.map(CellValue::Text).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value.map(CellValue::Bytes).unwrap_or(CellValue::Null);
    }
    CellValue::Null
}

fn elapsed_ms(started: std::time::Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn statement_returns_rows(statement: &str) -> bool {
    matches!(
        first_sql_word(statement).as_deref(),
        Some("select" | "show" | "describe" | "desc" | "explain" | "with" | "pragma" | "values")
    )
}

fn first_sql_word(statement: &str) -> Option<String> {
    statement
        .trim_start()
        .chars()
        .skip_while(|ch| !ch.is_ascii_alphabetic())
        .take_while(|ch| ch.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase()
        .into_non_empty()
}

trait IntoNonEmpty {
    fn into_non_empty(self) -> Option<String>;
}

impl IntoNonEmpty for String {
    fn into_non_empty(self) -> Option<String> {
        (!self.is_empty()).then_some(self)
    }
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut chars = sql.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if line_comment {
            if ch == '\n' {
                line_comment = false;
            }
            continue;
        }
        if block_comment {
            if ch == '*'
                && let Some((_, '/')) = chars.peek().copied()
            {
                chars.next();
                block_comment = false;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == '\\' {
                chars.next();
                continue;
            }
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '\'' | '"' | '`' => quote = Some(ch),
            '-' if matches!(chars.peek(), Some((_, '-'))) => {
                chars.next();
                line_comment = true;
            }
            '#' => line_comment = true,
            '/' if matches!(chars.peek(), Some((_, '*'))) => {
                chars.next();
                block_comment = true;
            }
            ';' | '；' => {
                if create_trigger_statement_needs_more(&sql[start..index]) {
                    continue;
                }
                push_statement(sql, start, index, &mut statements);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    push_statement(sql, start, sql.len(), &mut statements);
    statements
}

fn create_trigger_statement_needs_more(statement: &str) -> bool {
    let statement = statement.trim();
    let upper = statement.to_ascii_uppercase();
    let words = upper.split_whitespace().take(4).collect::<Vec<_>>();
    let is_create_trigger = matches!(
        words.as_slice(),
        ["CREATE", "TRIGGER", ..]
            | ["CREATE", "TEMP", "TRIGGER", ..]
            | ["CREATE", "TEMPORARY", "TRIGGER", ..]
    );
    is_create_trigger && upper.contains("BEGIN") && upper.split_whitespace().last() != Some("END")
}

fn query_statements_for_execution(request: &QueryRequest) -> Vec<String> {
    if request.options.split_statements {
        return split_sql_statements(&request.text);
    }

    let statement = request.text.trim();
    if statement.is_empty() {
        Vec::new()
    } else {
        vec![statement.to_string()]
    }
}

fn push_statement(sql: &str, start: usize, end: usize, statements: &mut Vec<String>) {
    let statement = sql[start..end].trim();
    if !statement.is_empty() {
        statements.push(statement.to_string());
    }
}

fn sqlite_trigger_word(sql: &str, words: &[&str]) -> String {
    let upper = sql.to_ascii_uppercase();
    words
        .iter()
        .find(|word| upper.contains(**word))
        .copied()
        .unwrap_or("UNKNOWN")
        .to_string()
}

fn data_order_by_clause(
    sort: &[SortSpec],
    columns: &[Column],
    quote_identifier: fn(&str) -> String,
) -> String {
    let parts = sort
        .iter()
        .filter_map(|spec| {
            let column = columns.iter().find(|column| column.name == spec.field)?;
            let direction = match spec.direction {
                SortDirection::Asc => "ASC",
                SortDirection::Desc => "DESC",
            };
            Some(format!("{} {direction}", quote_identifier(&column.name)))
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}

fn data_export_preview_sql(
    table_name: &str,
    fields: &[String],
    sort: &[SortSpec],
    filters: &[FilterSpec],
    columns: &[Column],
    quote_identifier: fn(&str) -> String,
) -> String {
    let select_list = data_export_preview_select_list(fields, columns, quote_identifier);
    let mut sql = format!("SELECT {select_list}\nFROM {table_name}");
    sql.push_str(&data_where_clause_preview(filters, columns, quote_identifier));
    sql.push_str(&data_order_by_clause(sort, columns, quote_identifier));
    sql
}

fn data_export_preview_select_list(
    fields: &[String],
    columns: &[Column],
    quote_identifier: fn(&str) -> String,
) -> String {
    let selected = fields
        .iter()
        .filter_map(|field| columns.iter().find(|column| column.name == *field))
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        "*".to_string()
    } else {
        selected.join(", ")
    }
}

fn data_where_clause_preview(
    filters: &[FilterSpec],
    columns: &[Column],
    quote_identifier: fn(&str) -> String,
) -> String {
    let clauses = filters
        .iter()
        .filter(|filter| filter.enabled && data_filter_clause_is_pushable(filter))
        .filter_map(|filter| {
            let column = columns.iter().find(|column| column.name == filter.field)?;
            data_filter_clause_preview(filter, column, quote_identifier)
        })
        .collect::<Vec<_>>();
    if clauses.is_empty() {
        String::new()
    } else {
        format!("\nWHERE {}", clauses.join("\n  AND "))
    }
}

fn data_filter_clause_preview(
    filter: &FilterSpec,
    column: &Column,
    quote_identifier: fn(&str) -> String,
) -> Option<String> {
    let column = quote_identifier(&column.name);
    Some(match filter.op {
        FilterOp::IsNull | FilterOp::NotExists => format!("{column} IS NULL"),
        FilterOp::IsNotNull | FilterOp::Exists => format!("{column} IS NOT NULL"),
        FilterOp::IsEmpty => format!("{column} = ''"),
        FilterOp::IsNotEmpty => format!("{column} != ''"),
        FilterOp::Between | FilterOp::NotBetween => {
            let (Some(start), Some(end)) = (filter.values.first(), filter.values.get(1)) else {
                return None;
            };
            let negative = if filter.op == FilterOp::NotBetween { " NOT" } else { "" };
            format!(
                "{column}{negative} BETWEEN {} AND {}",
                data_filter_literal(start),
                data_filter_literal(end)
            )
        }
        FilterOp::InList | FilterOp::NotInList => {
            if filter.values.is_empty() {
                return None;
            }
            let negative = if filter.op == FilterOp::NotInList { " NOT" } else { "" };
            let values = filter
                .values
                .iter()
                .map(data_filter_literal)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{column}{negative} IN ({values})")
        }
        FilterOp::Eq | FilterOp::NotEq => data_filter_multi_value_preview(
            &column,
            if filter.op == FilterOp::Eq { " = " } else { " != " },
            if filter.op == FilterOp::Eq { " OR " } else { " AND " },
            &filter.values,
        )?,
        FilterOp::Contains
        | FilterOp::NotContains
        | FilterOp::StartsWith
        | FilterOp::NotStartsWith
        | FilterOp::EndsWith
        | FilterOp::NotEndsWith => data_filter_like_preview(&column, filter.op, &filter.values)?,
        FilterOp::GreaterThan
        | FilterOp::GreaterThanOrEqual
        | FilterOp::LessThan
        | FilterOp::LessThanOrEqual => {
            let value = filter.values.first()?;
            let op = match filter.op {
                FilterOp::GreaterThan => " > ",
                FilterOp::GreaterThanOrEqual => " >= ",
                FilterOp::LessThan => " < ",
                FilterOp::LessThanOrEqual => " <= ",
                _ => unreachable!(),
            };
            format!("{column}{op}{}", data_filter_literal(value))
        }
    })
}

fn data_filter_multi_value_preview(
    column: &str,
    op: &str,
    joiner: &str,
    values: &[CellValue],
) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let parts = values
        .iter()
        .map(|value| format!("{column}{op}{}", data_filter_literal(value)))
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        Some(format!("({})", parts.join(joiner)))
    } else {
        parts.into_iter().next()
    }
}

fn data_filter_like_preview(column: &str, op: FilterOp, values: &[CellValue]) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let negative = matches!(
        op,
        FilterOp::NotContains | FilterOp::NotStartsWith | FilterOp::NotEndsWith
    );
    let joiner = if negative { " AND " } else { " OR " };
    let parts = values
        .iter()
        .map(|value| {
            let text = data_filter_value_text(value);
            let pattern = match op {
                FilterOp::Contains | FilterOp::NotContains => format!("%{text}%"),
                FilterOp::StartsWith | FilterOp::NotStartsWith => format!("{text}%"),
                FilterOp::EndsWith | FilterOp::NotEndsWith => format!("%{text}"),
                _ => unreachable!(),
            };
            let negative = if negative { " NOT" } else { "" };
            format!("{column}{negative} LIKE {}", data_filter_literal(&CellValue::Text(pattern)))
        })
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        Some(format!("({})", parts.join(joiner)))
    } else {
        parts.into_iter().next()
    }
}

fn data_filter_literal(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Bool(value) => {
            if *value {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) => value.to_string(),
        CellValue::Text(value) | CellValue::Json(value) => {
            format!("'{}'", value.replace('\'', "''"))
        }
        CellValue::Bytes(value) => {
            let hex = value
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>();
            format!("X'{hex}'")
        }
        CellValue::BinarySummary(summary) => {
            if summary.is_null {
                "NULL".to_string()
            } else {
                "'<binary>'".to_string()
            }
        }
    }
}

fn push_data_where_clause<DB>(
    builder: &mut QueryBuilder<'_, DB>,
    filters: &[FilterSpec],
    columns: &[Column],
    quote_identifier: fn(&str) -> String,
    push_bind: fn(&mut QueryBuilder<'_, DB>, &CellValue),
) where
    DB: sqlx::Database,
{
    let mut pushed = false;
    for filter in filters.iter().filter(|filter| filter.enabled) {
        if !data_filter_clause_is_pushable(filter) {
            continue;
        }
        let Some(column) = columns.iter().find(|column| column.name == filter.field) else {
            continue;
        };
        if !pushed {
            builder.push(" WHERE ");
            pushed = true;
        } else {
            builder.push(" AND ");
        }

        push_data_filter_clause(builder, filter, column, quote_identifier, push_bind);
    }
}

fn data_filter_clause_is_pushable(filter: &FilterSpec) -> bool {
    match filter.op {
        FilterOp::IsNull
        | FilterOp::IsNotNull
        | FilterOp::IsEmpty
        | FilterOp::IsNotEmpty
        | FilterOp::Exists
        | FilterOp::NotExists => true,
        FilterOp::Between | FilterOp::NotBetween => filter.values.len() >= 2,
        _ => !filter.values.is_empty(),
    }
}

fn push_data_filter_clause<DB>(
    builder: &mut QueryBuilder<'_, DB>,
    filter: &FilterSpec,
    column: &Column,
    quote_identifier: fn(&str) -> String,
    push_bind: fn(&mut QueryBuilder<'_, DB>, &CellValue),
) -> bool
where
    DB: sqlx::Database,
{
    let column = quote_identifier(&column.name);
    match filter.op {
        FilterOp::IsNull | FilterOp::NotExists => {
            builder.push(column).push(" IS NULL");
            true
        }
        FilterOp::IsNotNull | FilterOp::Exists => {
            builder.push(column).push(" IS NOT NULL");
            true
        }
        FilterOp::IsEmpty => {
            builder.push(column).push(" = ");
            push_bind(builder, &CellValue::Text(String::new()));
            true
        }
        FilterOp::IsNotEmpty => {
            builder.push(column).push(" != ");
            push_bind(builder, &CellValue::Text(String::new()));
            true
        }
        FilterOp::Between | FilterOp::NotBetween => {
            let (Some(start), Some(end)) = (filter.values.first(), filter.values.get(1)) else {
                return false;
            };
            builder.push(column);
            if filter.op == FilterOp::NotBetween {
                builder.push(" NOT");
            }
            builder.push(" BETWEEN ");
            push_bind(builder, start);
            builder.push(" AND ");
            push_bind(builder, end);
            true
        }
        FilterOp::InList | FilterOp::NotInList => {
            if filter.values.is_empty() {
                return false;
            }
            builder.push(column);
            if filter.op == FilterOp::NotInList {
                builder.push(" NOT");
            }
            builder.push(" IN (");
            for (index, value) in filter.values.iter().enumerate() {
                if index > 0 {
                    builder.push(", ");
                }
                push_bind(builder, value);
            }
            builder.push(")");
            true
        }
        FilterOp::Eq | FilterOp::NotEq => push_data_multi_value_clause(
            builder,
            &column,
            if filter.op == FilterOp::Eq {
                " = "
            } else {
                " != "
            },
            if filter.op == FilterOp::Eq {
                " OR "
            } else {
                " AND "
            },
            &filter.values,
            push_bind,
        ),
        FilterOp::Contains
        | FilterOp::NotContains
        | FilterOp::StartsWith
        | FilterOp::NotStartsWith
        | FilterOp::EndsWith
        | FilterOp::NotEndsWith => {
            push_data_like_clause(builder, &column, filter.op, &filter.values, push_bind)
        }
        FilterOp::GreaterThan
        | FilterOp::GreaterThanOrEqual
        | FilterOp::LessThan
        | FilterOp::LessThanOrEqual => {
            let Some(value) = filter.values.first() else {
                return false;
            };
            let op = match filter.op {
                FilterOp::GreaterThan => " > ",
                FilterOp::GreaterThanOrEqual => " >= ",
                FilterOp::LessThan => " < ",
                FilterOp::LessThanOrEqual => " <= ",
                _ => unreachable!(),
            };
            builder.push(column).push(op);
            push_bind(builder, value);
            true
        }
    }
}

fn push_data_multi_value_clause<DB>(
    builder: &mut QueryBuilder<'_, DB>,
    column: &str,
    op: &str,
    joiner: &str,
    values: &[CellValue],
    push_bind: fn(&mut QueryBuilder<'_, DB>, &CellValue),
) -> bool
where
    DB: sqlx::Database,
{
    if values.is_empty() {
        return false;
    }
    if values.len() > 1 {
        builder.push("(");
    }
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            builder.push(joiner);
        }
        builder.push(column).push(op);
        push_bind(builder, value);
    }
    if values.len() > 1 {
        builder.push(")");
    }
    true
}

fn push_data_like_clause<DB>(
    builder: &mut QueryBuilder<'_, DB>,
    column: &str,
    op: FilterOp,
    values: &[CellValue],
    push_bind: fn(&mut QueryBuilder<'_, DB>, &CellValue),
) -> bool
where
    DB: sqlx::Database,
{
    if values.is_empty() {
        return false;
    }
    let negative = matches!(
        op,
        FilterOp::NotContains | FilterOp::NotStartsWith | FilterOp::NotEndsWith
    );
    if values.len() > 1 {
        builder.push("(");
    }
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            builder.push(if negative { " AND " } else { " OR " });
        }
        let text = data_filter_value_text(value);
        let pattern = match op {
            FilterOp::Contains | FilterOp::NotContains => format!("%{text}%"),
            FilterOp::StartsWith | FilterOp::NotStartsWith => format!("{text}%"),
            FilterOp::EndsWith | FilterOp::NotEndsWith => format!("%{text}"),
            _ => unreachable!(),
        };
        builder.push(column);
        if negative {
            builder.push(" NOT");
        }
        builder.push(" LIKE ");
        push_bind(builder, &CellValue::Text(pattern));
    }
    if values.len() > 1 {
        builder.push(")");
    }
    true
}

fn data_filter_value_text(value: &CellValue) -> String {
    match value {
        CellValue::Null => String::new(),
        CellValue::Bool(value) => value.to_string(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) => value.to_string(),
        CellValue::Text(value) | CellValue::Json(value) => value.clone(),
        CellValue::Bytes(value) => value.iter().map(|byte| format!("{byte:02X}")).collect(),
        CellValue::BinarySummary(_) => value.display_label(),
    }
}

fn mysql_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database_error) = &error
        && database_error.code().as_deref() == Some("1045")
    {
        return Error::new(ErrorKind::Authentication, database_error.message());
    }

    Error::new(ErrorKind::Connection, error.to_string())
}

fn sqlite_error(error: sqlx::Error) -> Error {
    Error::new(ErrorKind::Connection, error.to_string())
}

fn mock_objects(connection_id: ConnectionId) -> Vec<ObjectSummary> {
    // demo 元数据集（T081）：4 张关联表，供补全 fixture 覆盖表/列/FK/排序等场景。
    vec![
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Product".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(504),
            modified_at: None,
            comment: Some("Products sold or used in manufacturing.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "ProductCategory".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(4),
            modified_at: None,
            comment: Some("High-level product categories.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Order".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(1200),
            modified_at: None,
            comment: Some("Sales orders referencing products and customers.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Customer".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(88),
            modified_at: None,
            comment: Some("Customers who placed orders.".to_string()),
        },
    ]
}

fn mock_data_page(offset: u64, limit: u64) -> DataPage {
    let pagination = Pagination::new(offset, limit);

    DataPage {
        columns: vec![
            Column {
                name: "id".to_string(),
                type_name: Some("INTEGER".to_string()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            Column {
                name: "name".to_string(),
                type_name: Some("TEXT".to_string()),
                nullable: false,
                primary_key: false,
                comment: Some("Product display name".to_string()),
            },
        ],
        rows: vec![
            Row {
                values: vec![CellValue::I64(1), CellValue::Text("Road Bike".to_string())],
            },
            Row {
                values: vec![CellValue::I64(2), CellValue::Text("Helmet".to_string())],
            },
        ],
        offset: pagination.offset,
        limit: pagination.limit,
        has_more: false,
    }
}

/// T081 demo 各表的补全列（表名区分，供列/FK/类型排序 fixture 使用）。
fn mock_completion_columns(table: &str) -> Vec<Column> {
    let cols = |pairs: &[(&str, &str, bool, bool, Option<&str>)]| -> Vec<Column> {
        pairs
            .iter()
            .map(|(name, type_name, nullable, primary_key, comment)| Column {
                name: (*name).to_string(),
                type_name: Some((*type_name).to_string()),
                nullable: *nullable,
                primary_key: *primary_key,
                comment: comment.map(str::to_string),
            })
            .collect()
    };
    match table {
        "Product" | "product" => cols(&[
            ("id", "INTEGER", false, true, None),
            ("name", "TEXT", false, false, Some("Product display name")),
            ("category_id", "INTEGER", true, false, Some("FK to ProductCategory.id")),
            ("price", "REAL", true, false, None),
            ("active", "INTEGER", true, false, Some("1 if sellable")),
            ("created_at", "TEXT", true, false, Some("ISO-8601 timestamp")),
        ]),
        "ProductCategory" | "productcategory" => cols(&[
            ("id", "INTEGER", false, true, None),
            ("name", "TEXT", false, false, Some("Category display name")),
            ("sort_order", "INTEGER", true, false, None),
        ]),
        "Order" | "order" => cols(&[
            ("id", "INTEGER", false, true, None),
            ("product_id", "INTEGER", false, false, Some("FK to Product.id")),
            ("quantity", "INTEGER", true, false, None),
            ("total", "REAL", true, false, None),
            ("customer_id", "INTEGER", false, false, Some("FK to Customer.id")),
            ("created_at", "TEXT", true, false, Some("ISO-8601 timestamp")),
        ]),
        "Customer" | "customer" => cols(&[
            ("id", "INTEGER", false, true, None),
            ("name", "TEXT", false, false, None),
            ("email", "TEXT", true, false, Some("Customer email")),
            ("city", "TEXT", true, false, None),
        ]),
        // 未知表（含大小写变体）兜底：沿用通用 id+name，保证既有断言不破坏。
        _ => cols(&[
            ("id", "INTEGER", false, true, None),
            ("name", "TEXT", false, false, Some("Display name")),
        ]),
    }
}

/// T081 demo 各表外键（P2.13 FK JOIN 建议）。复合外键以同名多条 ForeignKeyInfo 表达，
/// fk_join_completion_items 会按 name 分组生成 `ON a.x = b.x AND a.y = b.y`。
fn mock_completion_foreign_keys(table: &str) -> Vec<ForeignKeyInfo> {
    match table {
        // Product.category_id -> ProductCategory.id
        "Product" | "product" => vec![ForeignKeyInfo {
            name: "fk_product_category".to_string(),
            column: "category_id".to_string(),
            ref_schema: None,
            ref_table: "ProductCategory".to_string(),
            ref_column: "id".to_string(),
        }],
        // Order 的表连接：product_id + customer_id 独立 FK。
        "Order" | "order" => vec![
            ForeignKeyInfo {
                name: "fk_order_product".to_string(),
                column: "product_id".to_string(),
                ref_schema: None,
                ref_table: "Product".to_string(),
                ref_column: "id".to_string(),
            },
            ForeignKeyInfo {
                name: "fk_order_customer".to_string(),
                column: "customer_id".to_string(),
                ref_schema: None,
                ref_table: "Customer".to_string(),
                ref_column: "id".to_string(),
            },
        ],
        _ => Vec::new(),
    }
}
