// PostgreSQL SQL 执行器（T04）。
//
// 复用 mysql/sqlite 的语句切分与分页语义（`query_statements_for_execution`、
// `query_rows_to_page`、`push_query_summary`），差异仅在：
// - 连接取自共享 runtime 的会话（`pg_session_acquire`），而非每请求新建连接；
// - tokio-postgres 行/列读取与错误映射。

/// 单语句执行超时兜底；建连后由 `statement_timeout`（若配置）先行约束。
const PG_STATEMENT_TIMEOUT: Duration = Duration::from_secs(30);

/// 执行查询。带会话复用；无会话走隔离连接。
fn pg_execute_query(
    config: &ConnectionConfig,
    request: &QueryRequest,
) -> fluxdb_core::Result<QueryExecutionResult> {
    pg_execute_query_with_progress(config, request, &mut |_| {}, &|| false)
}

fn pg_execute_query_with_progress(
    config: &ConnectionConfig,
    request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statements = query_statements_for_execution(request);
    if statements.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }

    let database = pg_request_database(config, request.database.as_deref());

    // 在共享 runtime 上：取会话并执行。连接层错误允许重拨一次（会话可能已死）。
    let mut attempt = 0;
    loop {
        let (error, outcome) = pg_runtime().block_on(async {
            let session = match pg_session_acquire(config, request, &database).await {
                Ok(session) => session,
                Err(error) => return (error, None),
            };
            pg_run_statements(&session.client, statements.clone(), request, on_summary, should_cancel)
                .await
        });
        match outcome {
            // 连接/认证层错误：必要时重拨一次；仍失败则原样上抛。
            Some(result) => return Ok(result),
            None if error.kind == ErrorKind::Connection && attempt == 0 => {
                attempt += 1;
                continue;
            }
            None => return Err(error),
        }
    }
}

/// 会话拨号成功的错误 → 需要区分「连接建立失败」（返回 None，可重拨）vs
/// 「执行期产生的语句级失败」（已写入 summaries，返回 Some(result)）。
///
/// 这儿让 `pg_run_statements` 在遇到「会话本身无法使用」时才返回 `(error, None)`；
/// 其余语句级错误全部折进 summaries，正常返回 `(_ , Some(result))`（首元素占位无需使用）。
async fn pg_run_statements(
    client: &tokio_postgres::Client,
    statements: Vec<String>,
    request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
    should_cancel: &dyn Fn() -> bool,
) -> (fluxdb_core::Error, Option<QueryExecutionResult>) {
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
            match tokio::time::timeout(PG_STATEMENT_TIMEOUT, client.query(&statement, &[])).await {
                Ok(Ok(rows)) => {
                    let elapsed_ms = elapsed_ms(started);
                    let page = pg_rows_to_page(rows, request.options.page_offset, request.options.page_size);
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
                }
                Ok(Err(error)) => {
                    let error = pg_error(error);
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
                }
                Err(_) => {
                    let summary = failed_query_summary(
                        statement,
                        QueryStatementKind::ResultSet,
                        "查询超时".to_string(),
                        elapsed_ms(started),
                    );
                    push_query_summary(&mut execution, summary.clone(), on_summary);
                    if !request.options.continue_on_error {
                        break;
                    }
                }
            }
        } else {
            match tokio::time::timeout(PG_STATEMENT_TIMEOUT, client.batch_execute(&statement)).await {
                Ok(Ok(())) => {
                    push_query_summary(
                        &mut execution,
                        QueryExecutionSummary {
                            sql: statement,
                            kind: QueryStatementKind::Command,
                            success: true,
                            message: "OK".to_string(),
                            returned_rows: 0,
                            affected_rows: 0,
                            elapsed_ms: elapsed_ms(started),
                        },
                        on_summary,
                    );
                }
                Ok(Err(error)) => {
                    let error = pg_error(error);
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
                }
                Err(_) => {
                    let summary = failed_query_summary(
                        statement,
                        QueryStatementKind::Command,
                        "查询超时".to_string(),
                        elapsed_ms(started),
                    );
                    push_query_summary(&mut execution, summary.clone(), on_summary);
                    if !request.options.continue_on_error {
                        break;
                    }
                }
            }
        }
    }

    (Error::new(ErrorKind::Internal, ""), Some(execution))
}

/// 把 `query()` 结果集的分页 DataPage（列类型来自 extended protocol 的 RowDescription）。
fn pg_rows_to_page(rows: Vec<tokio_postgres::Row>, offset: u64, limit: u64) -> DataPage {
    let columns = rows
        .first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|column| query_column(column.name(), column.type_().name()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    query_rows_to_page(columns, rows, offset, limit, pg_query_cell_value)
}

fn pg_query_cell_value(row: &tokio_postgres::Row, index: usize, _column: &Column) -> CellValue {
    // 与 mysql/sqlite 一致：逐类型尝试，取首个可成功解析的值；
    // 二进制类型（bytea 等）统一按 Bytes 处理（is_binary_type_name 控制展示）。
    if let Ok(value) = row.try_get::<_, Option<i64>>(index) {
        return value.map(CellValue::I64).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<f64>>(index) {
        return value.map(CellValue::F64).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<bool>>(index) {
        return value.map(CellValue::Bool).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<String>>(index) {
        return value.map(CellValue::Text).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<Vec<u8>>>(index) {
        return value.map(CellValue::Bytes).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<f32>>(index) {
        return value.map(|v| CellValue::F64(v as f64)).unwrap_or(CellValue::Null);
    }
    CellValue::Null
}
