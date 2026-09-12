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
            pg_run_statements(config, &session.client, statements.clone(), request, on_summary, should_cancel)
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
    config: &ConnectionConfig,
    client: &std::sync::Arc<tokio_postgres::Client>,
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
    // 上次失败的语句是否使会话进入 aborted 事务态（显式事务内错误后）。
    let mut aborted = false;

    for statement in statements {
        if should_cancel() {
            break;
        }
        let started = std::time::Instant::now();
        // 会话已因此前语句失败进入 aborted（显式事务内），后续语句一律不可执行（R27：
        // 不得在 ROLLBACK 前继续、也不得自动回滚用户事务），逐条跳过并标注。
        if aborted {
            push_query_summary(
                &mut execution,
                skipped_query_summary(statement, elapsed_ms(started)),
                on_summary,
            );
            continue;
        }
        // 单语句执行：走真实 CancelToken（用户可立即停止长查询，无需等 30s 硬超时）+ 30s 兜底。
        match if statement_returns_rows(&statement) {
            pg_run_result_statement(
                config,
                client,
                &statement,
                request.options.page_offset,
                request.options.page_size,
                should_cancel,
            )
            .await
        } else {
            pg_run_command_statement(config, client, &statement, should_cancel).await
        } {
            PgStatementOutcome::ResultSet(page) => {
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
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
            }
            PgStatementOutcome::CommandOk => {
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
            PgStatementOutcome::Failed(kind, message) => {
                aborted |= kind == PgStatementFailure::AbortsTransaction;
                let statement_kind = if statement_returns_rows(&statement) {
                    QueryStatementKind::ResultSet
                } else {
                    QueryStatementKind::Command
                };
                let message = qualify_cancel_message(message);
                push_query_summary(
                    &mut execution,
                    failed_query_summary(statement, statement_kind, message, elapsed_ms(started)),
                    on_summary,
                );
                // continue_on_error 时继续：下一语句若在 aborted 内会返回 25P02/被跳过。
                if !request.options.continue_on_error {
                    break;
                }
            }
            PgStatementOutcome::Cancelled => {
                // 用户主动取消：成功与否未知（尤其写语句），标「结果待核实」，不下结论。
                let cancelled_kind = if statement_returns_rows(&statement) {
                    QueryStatementKind::ResultSet
                } else {
                    QueryStatementKind::Command
                };
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: cancelled_kind,
                        success: false,
                        message: "已取消：结果待核实（服务端可能已完成也可能已回滚）".to_string(),
                        returned_rows: 0,
                        affected_rows: 0,
                        elapsed_ms: elapsed_ms(started),
                    },
                    on_summary,
                );
                if !request.options.continue_on_error {
                    break;
                }
            }
        }
    }

    (Error::new(ErrorKind::Internal, ""), Some(execution))
}

/// 单语句执行结果（供 `pg_run_statements` 分支归并）。
enum PgStatementOutcome {
    /// 返回结果集（已分页）。
    ResultSet(fluxdb_core::DataPage),
    /// 命令成功执行（无结果集）。
    CommandOk,
    /// 语句级失败。写明是否使会话进入 aborted。
    Failed(PgStatementFailure, String),
    /// 用户主动取消且结果不确定（尤其写语句）——不谎报成功，也不断言回滚。
    Cancelled,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PgStatementFailure {
    AbortsTransaction,
    Plain,
}

/// 取消协商循环的异常终止原因（与语句任务自身的 `Result` 区分）。
enum PgRaceAbort {
    /// 客户端看门狗 30s 超时。
    Timeout,
    /// 语句任务 JoinHandle 异常（任务 panic）。
    Join,
}

/// 通过真实 `CancelToken` 向服务端发送取消请求（同传输/TLS 策略新建连接）。
/// TLS 模式用 `MakeRustlsConnect`，否则 `NoTls`；失败（如代理无 socket_config）静默忽略，
/// 由调用方 30s 看门狗兜底，不误报。
async fn pg_send_cancel(
    token: &tokio_postgres::CancelToken,
    profile: &fluxdb_core::PostgresConnectionProfile,
) {
    match pg_tls_connect(profile) {
        Ok(Some(tls)) => {
            let _ = token.cancel_query(tls).await;
        }
        Ok(None) => {
            let _ = token.cancel_query(tokio_postgres::NoTls).await;
        }
        Err(_) => {}
    }
}

/// 执行返回结果集的语句，并在用户取消时通过真实 `CancelToken` 立即停止（无需等 30s 硬超时）。
///
/// 玩法：在共享 runtime 上派生一次 `select!`——主分支执行语句（带 30s 兜底），次分支每 ~120ms
/// 轮询 `should_cancel()`；一旦置位，用 `client.cancel_token().cancel_query(tls)` 走**同传输/TLS 策略**
/// 新开连接向服务端发 `CancelRequest`（tokio-postgres runtime 特性自动重建 socket_config，直连/SSH
/// 路径成立；代理 connect_raw 无 socket_config 时退化为等 30s 兜底），然后继续等原语句返回 57014 或完成。
async fn pg_run_result_statement(
    config: &ConnectionConfig,
    client: &std::sync::Arc<tokio_postgres::Client>,
    statement: &str,
    page_offset: u64,
    page_size: u64,
    should_cancel: &dyn Fn() -> bool,
) -> PgStatementOutcome {
    let Some(profile) = config.postgres_profile.as_ref() else {
        return PgStatementOutcome::Failed(
            PgStatementFailure::Plain,
            "PostgreSQL 连接档案缺失".to_string(),
        );
    };
    let token = client.cancel_token();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancelled_flag = cancelled.clone();
    let client_for_task = std::sync::Arc::clone(client);
    let statement_for_task = statement.to_string();
    let mut task = pg_runtime().spawn(async move {
        pg_query_rows_owned(client_for_task, statement_for_task).await
    });
    // 客户端看门狗兜底：语句 30s 未完成且未被取消 → 视为超时（不挂死）。
    let mut watchdog = pg_runtime().spawn(tokio::time::sleep(PG_STATEMENT_TIMEOUT));
    let outcome = loop {
        tokio::select! {
            result = &mut task => {
                break result.map_err(|_| PgRaceAbort::Join);
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(120)) => {
                if should_cancel() && !cancelled_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    cancelled_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    // 真实 CancelToken：走 `cancel_query`（同传输策略新建连接发送 CancelRequest）。
                    // 失败（如代理 connect_raw 无 socket_config）仅忽略，靠 30s 兜底终止。
                    pg_send_cancel(&token, profile).await;
                }
            }
            _ = &mut watchdog => {
                // 客户端看门狗兜底：语句 30s 未完成且未被取消 → 视为超时。
                break Err(PgRaceAbort::Timeout);
            }
        }
    };
    match outcome {
        Ok(Ok((columns, rows))) => {
            // 取消请求已发出但仍返回成功的行 —— 服务端已完成，按成功处理（无数据丢失）。
            PgStatementOutcome::ResultSet(query_rows_to_page(
                columns,
                rows,
                page_offset,
                page_size,
                pg_query_cell_value,
            ))
        }
        Ok(Err(error)) => {
            // 用户主动取消且服务端报 57014 query_canceled → Cancelled；否则普通失败。
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                if is_query_cancelled(&error) {
                    PgStatementOutcome::Cancelled
                } else {
                    PgStatementOutcome::Failed(PgStatementFailure::Plain, pg_error(error).message)
                }
            } else {
                let aborts = pg_error_aborts_transaction(&error);
                PgStatementOutcome::Failed(
                    if aborts {
                        PgStatementFailure::AbortsTransaction
                    } else {
                        PgStatementFailure::Plain
                    },
                    pg_error(error).message,
                )
            }
        }
        Err(PgRaceAbort::Timeout) => {
            PgStatementOutcome::Failed(PgStatementFailure::Plain, "查询超时".to_string())
        }
        Err(PgRaceAbort::Join) => {
            PgStatementOutcome::Failed(PgStatementFailure::Plain, "执行任务异常退出".to_string())
        }
    }
}

/// 执行命令（无结果集）语句，同样走真实 CancelToken；取消时写语句结果未知。
async fn pg_run_command_statement(
    config: &ConnectionConfig,
    client: &std::sync::Arc<tokio_postgres::Client>,
    statement: &str,
    should_cancel: &dyn Fn() -> bool,
) -> PgStatementOutcome {
    let Some(profile) = config.postgres_profile.as_ref() else {
        return PgStatementOutcome::Failed(
            PgStatementFailure::Plain,
            "PostgreSQL 连接档案缺失".to_string(),
        );
    };
    let token = client.cancel_token();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancelled_flag = cancelled.clone();
    let client_for_task = std::sync::Arc::clone(client);
    let statement_for_task = statement.to_string();
    let mut task = pg_runtime().spawn(async move {
        pg_command_owned(client_for_task, statement_for_task).await
    });
    // 客户端看门狗兜底：语句 30s 未完成且未被取消 → 视为超时。
    let mut watchdog = pg_runtime().spawn(tokio::time::sleep(PG_STATEMENT_TIMEOUT));
    let outcome = loop {
        tokio::select! {
            result = &mut task => {
                break result.map_err(|_| PgRaceAbort::Join);
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(120)) => {
                if should_cancel() && !cancelled_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    cancelled_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    pg_send_cancel(&token, profile).await;
                }
            }
            _ = &mut watchdog => break Err(PgRaceAbort::Timeout),
        }
    };
    match outcome {
        Ok(Ok(())) => PgStatementOutcome::CommandOk,
        Ok(Err(error)) => {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                // 写语句被用户取消：服务端可能已提交也可能已回滚，标记结果待核实。
                if is_query_cancelled(&error) {
                    PgStatementOutcome::Cancelled
                } else {
                    PgStatementOutcome::Failed(
                        if pg_error_aborts_transaction(&error) {
                            PgStatementFailure::AbortsTransaction
                        } else {
                            PgStatementFailure::Plain
                        },
                        pg_error(error).message,
                    )
                }
            } else {
                PgStatementOutcome::Failed(
                    if pg_error_aborts_transaction(&error) {
                        PgStatementFailure::AbortsTransaction
                    } else {
                        PgStatementFailure::Plain
                    },
                    pg_error(error).message,
                )
            }
        }
        Err(PgRaceAbort::Timeout) => {
            PgStatementOutcome::Failed(PgStatementFailure::Plain, "查询超时".to_string())
        }
        Err(PgRaceAbort::Join) => {
            PgStatementOutcome::Failed(PgStatementFailure::Plain, "执行任务异常退出".to_string())
        }
    }
}

/// SQLSTATE 是否为 57014 query_canceled（用户取消 / statement_timeout 触发）。
fn is_query_cancelled(error: &tokio_postgres::Error) -> bool {
    use tokio_postgres::error::SqlState;
    matches!(error.code(), Some(&SqlState::QUERY_CANCELED))
}

/// 若失败消息命中 57014（query_canceled），标注「结果待核实」；普通失败原样返回。
fn qualify_cancel_message(message: String) -> String {
    if message.contains("57014")
        || message.contains("query_canceled")
        || message.contains("canceling statement")
    {
        format!("{message}（已取消，结果待核实）")
    } else {
        message
    }
}

/// owned 包装（供 'static spawn 任务调用，避免借用逃逸到 runtime）。`Client` 非 Clone，用 `Arc` 共享。
async fn pg_query_rows_owned(
    client: std::sync::Arc<tokio_postgres::Client>,
    statement: String,
) -> Result<(Vec<Column>, Vec<tokio_postgres::Row>), tokio_postgres::Error> {
    pg_query_with_columns(&client, &statement).await
}

/// owned 包装（命令批执行，供 'static spawn 任务调用）。
async fn pg_command_owned(
    client: std::sync::Arc<tokio_postgres::Client>,
    statement: String,
) -> Result<(), tokio_postgres::Error> {
    client.batch_execute(&statement).await
}

/// 先 `prepare` 取 RowDescription 列头，再执行取行。空结果仍保留列头（§8.2「空行结果仍有列头」）。
/// 无法 prepare（SQL 方言差异）时退回 execute 后从首行取列。
async fn pg_query_with_columns(
    client: &std::sync::Arc<tokio_postgres::Client>,
    statement: &str,
) -> Result<(Vec<Column>, Vec<tokio_postgres::Row>), tokio_postgres::Error> {
    let prepared = client.prepare(statement).await;
    match prepared {
        Ok(prepared) => {
            let columns = prepared
                .columns()
                .iter()
                .map(|column| query_column(column.name(), column.type_().name()))
                .collect::<Vec<_>>();
            let rows = client.query(&prepared, &[]).await?;
            Ok((columns, rows))
        }
        Err(_) => {
            let rows = client.query(statement, &[]).await?;
            let columns = rows
                .first()
                .map(|row| {
                    row.columns()
                        .iter()
                        .map(|column| query_column(column.name(), column.type_().name()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok((columns, rows))
        }
    }
}

fn pg_query_cell_value(row: &tokio_postgres::Row, index: usize, column: &Column) -> CellValue {
    // 与 execute 路径（RowDescription 的 type_name）一致：按类型矩阵分类解码（T09）。
    // 覆盖整数/浮点/布尔/文本/json/时间/数值/decimal/bytea，未知类型按文本回退。
    pg_projected_cell_value(row, index, column)
}

/// 失败的语句并未执行（会话进入 aborted / 或循环因中止而跳过）时使用的摘要。
fn skipped_query_summary(sql: String, elapsed_ms: u64) -> QueryExecutionSummary {
    QueryExecutionSummary {
        sql,
        kind: QueryStatementKind::ResultSet,
        success: false,
        message: "已跳过：会话处于 aborted 事务态，需 ROLLBACK 后继续".to_string(),
        returned_rows: 0,
        affected_rows: 0,
        elapsed_ms,
    }
}
