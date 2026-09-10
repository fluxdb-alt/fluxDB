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
