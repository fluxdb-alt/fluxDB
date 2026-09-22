fn mysql_execute_query(
    config: &ConnectionConfig,
    request: &QueryRequest,
) -> fluxdb_core::Result<QueryExecutionResult> {
    mysql_execute_query_with_progress(config, request, &mut |_| {}, &|| false)
}

/// 单条 MySQL 语句的执行结局：服务端产出 + 失败 + 是否用户「停止」取消。
struct MySqlStatementOutcome {
    rows: Option<Vec<MySqlRow>>,
    affected_rows: u64,
    error: Option<fluxdb_core::Error>,
    user_cancelled: bool,
}

const MYSQL_CANCELLED_OUTCOME_UNKNOWN: &str = "已取消：结果待核实";
const MYSQL_CANCELLED_NOT_EXECUTED: &str = "已取消：未执行";

/// MySQL 单语句取消：sqlx 不会在连接上暴露可 KILL 的 `CONNECTION_ID()`，
/// 故按需读一次连接 id，取消时另开一条短连接下发 `KILL QUERY <id>` 中止服务端执行
/// （sqlx 自身无 CancelToken，等效 PG 的 CancelToken）。`KILL` 只是中止服务端语句，
/// 主连接本体保留、会话不被摧毁；驱动不记得 obsolescent connection_id，取一次即可。
///
/// `should_cancel` 返回 true 时机要求：语句必须整体交给服务端跑起来后，取消才可能
/// 中断它——否则逐一 `KILL` 还没开始的语句反而把下一语句误杀。为对齐 PG 语义
/// （用户取消后批次里**未发送**的语句标「未执行」），取消只对**正在跑**的那条生效：
/// 调用方在语句之间检查 `should_cancel` 决定是否继续发下一条，本函数只在运行中的
/// 语句上触发 kill。
async fn mysql_run_statement_cancellable(
    url: &str,
    connection: &mut MySqlConnection,
    conn_id: u64,
    statement: &str,
    should_cancel: &dyn Fn() -> bool,
) -> MySqlStatementOutcome {
    let returns_rows = statement_returns_rows(statement);
    let run = async {
        if returns_rows {
            let rows = sqlx::query(statement)
                .fetch_all(&mut *connection)
                .await
                .map(|rows| MySqlStatementOutcome {
                    rows: Some(rows),
                    affected_rows: 0,
                    error: None,
                    user_cancelled: false,
                });
            match rows {
                Ok(outcome) => outcome,
                Err(error) => MySqlStatementOutcome {
                    rows: None,
                    affected_rows: 0,
                    error: Some(mysql_error(error)),
                    user_cancelled: false,
                },
            }
        } else {
            // 非结果集语句（DDL/命令）走文本协议（raw_sql），不走 COM_STMT_PREPARE：
            // 部分 MySQL 兼容服务端（如 TiDB/OceanBase 等对部分 DDL）会在预编译协议下
            // 报 1295 "This command is not supported in the prepared statement protocol yet"，
            // 同一 SQL 用文本协议执行则正常。语句均为无绑定参数的字面 SQL，raw_sql 安全。
            let outcome = sqlx::raw_sql(statement)
                .execute(&mut *connection)
                .await
                .map(|result| MySqlStatementOutcome {
                    rows: None,
                    affected_rows: result.rows_affected(),
                    error: None,
                    user_cancelled: false,
                });
            match outcome {
                Ok(outcome) => outcome,
                Err(error) => MySqlStatementOutcome {
                    rows: None,
                    affected_rows: 0,
                    error: Some(mysql_error(error)),
                    user_cancelled: false,
                },
            }
        }
    };
    tokio::pin!(run);
    let mut cancel_sent = false;
    let mut cancel_succeeded = false;
    loop {
        tokio::select! {
            outcome = &mut run => {
                let mut outcome = outcome;
                // 只有服务端确认接收 KILL，且主连接以错误结束时，才能认定本语句由
                // 用户取消；若 KILL 没赶上而语句正常完成，仍保留真实成功结果。
                outcome.user_cancelled = cancel_succeeded && outcome.error.is_some();
                return outcome;
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if !cancel_sent && should_cancel() {
                    cancel_sent = true;
                    // KILL QUERY 只中止当前语句，随后主连接会收到该语句超时/中断错误。
                    match mysql_send_kill_query(url, conn_id).await {
                        Ok(()) => {
                            cancel_succeeded = true;
                            tracing::info!(
                                target: "fluxdb_connectors",
                                connection_id = conn_id,
                                "MySQL 已向服务端发送 KILL QUERY"
                            );
                        }
                        Err(error) => tracing::warn!(
                            target: "fluxdb_connectors",
                            connection_id = conn_id,
                            message = %error.message,
                            "MySQL KILL QUERY 发送失败"
                        ),
                    }
                }
            }
        }
    }
}

/// 另开一条短连接对 `conn_id` 下发 `KILL QUERY`（只中止语句、不关连接）。失败不冒泡，
/// 由主连接收到的错误表达；避免 KILL 通了但因意外关闭 kill 连接而静默。
async fn mysql_send_kill_query(url: &str, conn_id: u64) -> fluxdb_core::Result<()> {
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;
    let mut kill_conn = match tokio::time::timeout(Duration::from_secs(5), options.connect()).await {
        Ok(Ok(connection)) => connection,
        Ok(Err(error)) => return Err(mysql_error(error)),
        Err(_) => return Err(Error::new(ErrorKind::Connection, "取消连接超时")),
    };
    let killed = sqlx::raw_sql(&format!("KILL QUERY {}", conn_id))
        .execute(&mut kill_conn)
        .await
        .map_err(mysql_error);
    let _ = kill_conn.close().await;
    killed.map(|_| ())
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

        // 读取本连接的 server-side connection id，供取消时 `KILL QUERY` 定位。
        // CONNECTION_ID() 的 MySQL 类型是 BIGINT UNSIGNED，必须用 u64 解码；使用 i64
        // 会在用户 SQL 发出前触发 sqlx 类型不匹配，表现为结果与执行摘要同时为空。
        let conn_id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
            .fetch_one(&mut connection)
            .await
            .map_err(|error| {
                let error = mysql_error(error);
                tracing::warn!(
                    target: "fluxdb_connectors",
                    message = %error.message,
                    "MySQL 读取 CONNECTION_ID() 失败"
                );
                error
            })?;

        let mut execution = QueryExecutionResult {
            summaries: Vec::new(),
            results: Vec::new(),
            rollback_snapshots: Vec::new(),
        };
        // 用户已停止：一旦取消生效，本批次剩下语句一律不再发送（与 should_cancel 是否仍为
        // true 无关，避免取消标志被调用方复位后又继续往下跑）。
        let mut stopped = false;
        for statement in statements {
            if stopped || should_cancel() {
                // 不再向服务端发送语句，逐条标注「未执行」保持与语句列表对齐。
                let kind = if statement_returns_rows(&statement) {
                    QueryStatementKind::ResultSet
                } else {
                    QueryStatementKind::Command
                };
                push_query_summary(
                    &mut execution,
                    failed_query_summary(
                        statement,
                        kind,
                        MYSQL_CANCELLED_NOT_EXECUTED.to_string(),
                        0,
                    ),
                    on_summary,
                );
                continue;
            }
            let started = std::time::Instant::now();
            let outcome = mysql_run_statement_cancellable(
                &url, &mut connection, conn_id, &statement, should_cancel,
            )
            .await;
            let full_statement = statement.clone();
            if outcome.user_cancelled {
                // 取消命中正在运行的语句：服务端执行到哪一步未知，标「结果待核实」。
                let returns_rows = statement_returns_rows(&full_statement);
                push_query_summary(
                    &mut execution,
                    failed_query_summary(
                        full_statement.clone(),
                        if returns_rows {
                            QueryStatementKind::ResultSet
                        } else {
                            QueryStatementKind::Command
                        },
                        MYSQL_CANCELLED_OUTCOME_UNKNOWN.to_string(),
                        elapsed_ms(started),
                    ),
                    on_summary,
                );
                stopped = true;
                continue;
            }
            if let Some(rows) = outcome.rows {
                let page = mysql_query_rows_to_page(
                    rows,
                    request.options.page_offset,
                    request.options.page_size,
                );
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: full_statement,
                        kind: QueryStatementKind::ResultSet,
                        success: true,
                        message: format!("返回 {} 行结果表", page.rows.len()),
                        returned_rows: page.rows.len() as u64,
                        affected_rows: 0,
                        elapsed_ms: elapsed_ms(started),
                    },
                    on_summary,
                );
                execution.results.push(page);
            } else if let Some(error) = outcome.error {
                let error_kind = if statement_returns_rows(&full_statement) {
                    QueryStatementKind::ResultSet
                } else {
                    QueryStatementKind::Command
                };
                push_query_summary(
                    &mut execution,
                    failed_query_summary(full_statement, error_kind, error.message, elapsed_ms(started)),
                    on_summary,
                );
                if !request.options.continue_on_error {
                    break;
                }
            } else {
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: full_statement,
                        kind: QueryStatementKind::Command,
                        success: true,
                        message: "OK".to_string(),
                        returned_rows: 0,
                        affected_rows: outcome.affected_rows,
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
    let pagination = query_execution_pagination(offset, limit);
    let offset = pagination.offset.min(rows.len() as u64) as usize;
    let row_capacity = query_execution_row_capacity(pagination.limit);
    let has_more = rows.len().saturating_sub(offset) > row_capacity;
    let rows = rows
        .into_iter()
        .skip(offset)
        .take(row_capacity)
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

/// 查询编辑器约定 `limit == 0` 表示不限制；通用 `Pagination::new` 则会把 0
/// 收敛为 1，适用于数据表分页但不适用于 SQL 查询结果。
fn query_execution_pagination(offset: u64, limit: u64) -> Pagination {
    if limit == 0 {
        Pagination { offset, limit }
    } else {
        Pagination::new(offset, limit)
    }
}

fn query_execution_row_capacity(limit: u64) -> usize {
    if limit == 0 {
        usize::MAX
    } else {
        usize::try_from(limit).unwrap_or(usize::MAX)
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
