// PostgreSQL 执行使用服务端结果描述与 CommandComplete，不按首关键字猜测结果。
// 文本协议只执行一次；prepare 仅补充类型。流始终排空，内存只保存当前页。
use futures_util::StreamExt;

/// 用户点「停止」后的语句结局：取消请求已发给服务端，但服务端处理到哪一步无法确认，
/// 既不能报成功也不能报成 SQL 错误（历史据此把本次执行标为「已回滚/未知」，见 query_history）。
const PG_CANCELLED_OUTCOME_UNKNOWN: &str = "已取消：结果待核实";
/// 用户取消后同一批次里**未发送**的语句。
const PG_CANCELLED_NOT_EXECUTED: &str = "已取消：未执行";
/// 连接自身查询超时触发的取消（与用户主动取消区分，见设计 §3.3）。
const PG_TIMEOUT_OUTCOME_UNKNOWN: &str = "已超时：结果待核实";
/// 事务已中止时被本地跳过的语句，提示恢复方式而不是自动回滚。
const PG_SKIPPED_NEEDS_ROLLBACK: &str = "已跳过：需 ROLLBACK 后继续";

/// 单条语句的执行结局：服务端产出 + 失败原因 + 本语句是否因取消/超时被中断。
struct PgStatementOutcome {
    completed: Vec<PgCommandResult>,
    error: Option<Error>,
    /// 本语句执行期间发出的取消来源；None 表示未取消（或取消未生效、语句已正常收尾）。
    cancel: Option<PgCancelReason>,
}

/// 取消来源：用户点「停止」还是本连接的查询超时。
#[derive(Clone, Copy)]
enum PgCancelReason {
    User,
    Timeout,
}

/// 会话是否已进入「事务中止」态。服务端对事务内失败后的语句统一回 25P02，
/// 而 `pg_db_error_text` 已把 SQLSTATE 前置为 `[SQLSTATE xx]`，按前缀识别即可，
/// 不去匹配会随 locale/版本变化的英文错误文本。
fn pg_error_is_aborted_transaction(message: &str) -> bool {
    message.starts_with("[SQLSTATE 25P02]")
}

fn pg_execute_query(config: &ConnectionConfig, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
    pg_execute_query_with_progress(config, request, &mut |_| {}, &|| false)
}

fn pg_execute_query_with_progress(
    config: &ConnectionConfig, request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary), should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statements = query_statements_for_execution(request);
    if statements.is_empty() { return Err(Error::new(ErrorKind::Query, "查询不能为空")); }
    pg_runtime().block_on(async {
        let database = pg_request_database(config, request.database.as_deref());
        let session = pg_session_acquire(config, request, &database).await?;
        let lock = session.execution.lock();
        tokio::pin!(lock);
        let _guard = loop {
            tokio::select! {
                guard = &mut lock => break guard,
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if should_cancel() { return Err(Error::new(ErrorKind::Cancelled, "已取消排队执行")); }
                }
            }
        };
        if session.driver.0.is_finished() || session.client.is_closed() {
            return Err(Error::new(ErrorKind::Connection, "查询会话已失效，执行结果请先核实；关闭标签后重新连接"));
        }
        let mut execution = QueryExecutionResult { summaries: vec![], results: vec![], rollback_snapshots: vec![] };
        // 显式事务内语句失败后会话进入「中止」态：后续语句发出去也只会收回 25P02。
        // 记下该状态并本地逐条跳过（**不自动回滚**，恢复由用户显式 ROLLBACK 完成）。
        let mut aborted = false;
        // 用户已停止：一旦取消生效，本批次剩下语句一律不再发送（与 should_cancel 是否仍为
        // true 无关，避免取消标志被调用方复位后又继续往下跑）。
        let mut stopped = false;
        for statement in statements {
            if stopped || should_cancel() {
                // 不再向服务端发送语句，逐条标注「未执行」保持与语句列表对齐。
                push_query_summary(&mut execution,
                    failed_query_summary(statement, QueryStatementKind::Command, PG_CANCELLED_NOT_EXECUTED.to_string(), 0), on_summary);
                continue;
            }
            let started = Instant::now();
            if aborted && !pg_is_transaction_recovery(&statement) {
                push_query_summary(&mut execution,
                    failed_query_summary(statement, QueryStatementKind::Command, PG_SKIPPED_NEEDS_ROLLBACK.to_string(), 0), on_summary);
                continue;
            }
            // 恢复语句不能被失败事务中的 SET/prepare 拦住；其余语句交由服务端决定能否执行。
            let result = pg_run_statement(config, &session, &statement, request, should_cancel).await;
            let outcome = match result {
                Ok(outcome) => outcome,
                Err(error) => PgStatementOutcome { completed: vec![], error: Some(error), cancel: None },
            };
            for result in outcome.completed {
                let kind = if result.page.is_some() { QueryStatementKind::ResultSet } else { QueryStatementKind::Command };
                let message = if let Some(page) = &result.page {
                    if page.has_more { format!("返回 {} 行，当前仅展示 {} 行（结果已截断）", result.returned_rows, page.rows.len()) }
                    else { format!("返回 {} 行", result.returned_rows) }
                } else { format!("OK，影响 {} 行", result.affected_rows) };
                push_query_summary(&mut execution, QueryExecutionSummary {
                    sql: statement.clone(), kind, success: true, message,
                    returned_rows: result.returned_rows, affected_rows: result.affected_rows,
                    elapsed_ms: elapsed_ms(started),
                }, on_summary);
                if let Some(page) = result.page { execution.results.push(page); }
            }
            if let Some(error) = outcome.error {
                // 用户取消/查询超时要与真实 SQL 失败分开表达：取消不能报成 SQL 错误，
                // 也不能谎报成功——服务端可能已执行了一部分，统一按「结果待核实」收尾。
                let (message, user_cancelled) = match outcome.cancel {
                    Some(PgCancelReason::User) => (PG_CANCELLED_OUTCOME_UNKNOWN.to_string(), true),
                    Some(PgCancelReason::Timeout) => (PG_TIMEOUT_OUTCOME_UNKNOWN.to_string(), false),
                    None => (error.message.clone(), false),
                };
                // 事务已中止时服务端不会再执行任何语句，除原始 SQLSTATE 外补上恢复方式。
                let aborted_transaction = !user_cancelled && pg_error_is_aborted_transaction(&error.message);
                let message = if aborted_transaction { format!("{message}；事务已中止，需 ROLLBACK 后继续") } else { message };
                let fatal = matches!(error.kind, ErrorKind::Connection | ErrorKind::Cancelled | ErrorKind::Timeout);
                // 失败原因（含服务端 SQLSTATE/错误文本）必须在日志可见，否则排障只能靠重试。
                tracing::warn!(target: "fluxdb_connectors", kind = ?error.kind, user_cancelled, message = %error.message, sql = %statement, "PostgreSQL 执行失败");
                push_query_summary(&mut execution,
                    failed_query_summary(statement, QueryStatementKind::Command, message, elapsed_ms(started)), on_summary);
                if user_cancelled {
                    // 已取消的语句按「结果待核实」收尾，剩余语句交给循环顶部逐条标注「未执行」。
                    stopped = true;
                    continue;
                }
                aborted = aborted || aborted_transaction;
                if fatal || !request.options.continue_on_error { break; }
            } else {
                // 本语句正常收尾；若事务此前已中止，只有恢复语句（ROLLBACK/COMMIT）能走到这里。
                aborted = false;
            }
        }
        Ok(execution)
    })
}

/// 去掉前置注释，事务恢复判断不受注释中的关键字影响。
fn pg_executable_sql(mut sql: &str) -> &str {
    loop {
        sql = sql.trim_start();
        if let Some(rest) = sql.strip_prefix("--") {
            sql = rest.split_once('\n').map_or("", |(_, tail)| tail);
        } else if sql.starts_with("/*") {
            let bytes = sql.as_bytes();
            let mut depth = 1; let mut i = 2;
            while i + 1 < bytes.len() && depth > 0 {
                if &bytes[i..i+2] == b"/*" { depth += 1; i += 2; }
                else if &bytes[i..i+2] == b"*/" { depth -= 1; i += 2; }
                else { i += 1; }
            }
            if depth != 0 { return ""; }
            sql = &sql[i..];
        } else { return sql; }
    }
}

fn pg_is_transaction_recovery(sql: &str) -> bool {
    let word = pg_executable_sql(sql).split(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or("");
    ["ROLLBACK", "ABORT", "COMMIT", "END"].iter().any(|v| word.eq_ignore_ascii_case(v))
}

struct PgCommandResult {
    page: Option<DataPage>,
    returned_rows: u64,
    affected_rows: u64,
}

/// 单条语句的服务端产出：本语句收到的命令结果 + 流上的错误（语句内错误不整体上抛）。
type PgStreamResult = (Vec<PgCommandResult>, Option<Error>);

async fn pg_run_statement(
    config: &ConnectionConfig, session: &PgSession, statement: &str,
    request: &QueryRequest, should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<PgStatementOutcome> {
    let profile = config.postgres_profile.as_ref().ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL 连接档案缺失"))?;
    let run = async {
        if !pg_is_transaction_recovery(statement) { pg_ensure_search_path(session, request.schema.as_deref()).await?; }
        let result = pg_collect_statement(&session.client, statement, request).await;
        // ROLLBACK/SET 会改变 search_path；下一次请求必须重新确认作用域。
        let word = pg_executable_sql(statement).split_whitespace().next().unwrap_or("").trim_end_matches(';');
        if pg_is_transaction_recovery(statement) || word.eq_ignore_ascii_case("SET") || word.eq_ignore_ascii_case("RESET") {
            if let Ok(mut applied) = session.applied_schema.lock() { *applied = None; }
        }
        result
    };
    tokio::pin!(run);
    let started = Instant::now();
    let mut cancel_sent = false;
    let mut cancel_reason: Option<PgCancelReason> = None;
    let mut cancellation_started = None;
    loop {
        tokio::select! {
            result = &mut run => {
                // 取消来源只在语句确实以错误收尾时上抛：若取消没赶上、语句已正常跑完，
                // 结果就是完整的，不能标成「结果待核实」。
                let (completed, error) = result?;
                let cancel = if error.is_some() { cancel_reason } else { None };
                return Ok(PgStatementOutcome { completed, error, cancel });
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                let timed_out = profile.advanced.query_timeout_secs > 0
                    && started.elapsed() >= Duration::from_secs(u64::from(profile.advanced.query_timeout_secs));
                if !cancel_sent && (should_cancel() || timed_out) {
                    cancel_sent = true;
                    cancel_reason = Some(if should_cancel() { PgCancelReason::User } else { PgCancelReason::Timeout });
                    cancellation_started = Some(Instant::now());
                    // 取消等待有界，失败立即废弃会话；不会遗留独立执行任务继续写入。
                    if tokio::time::timeout(profile.connect_timeout(), pg_send_cancel(session, profile)).await
                        .map_or(true, |result| result.is_err()) {
                        session.driver.0.abort();
                        return Err(Error::new(ErrorKind::Connection, "取消请求未确认，会话已关闭；执行结果待核实，请勿直接重试写操作"));
                    }
                }
                if cancellation_started.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) {
                    session.driver.0.abort();
                    return Err(Error::new(ErrorKind::Connection, "取消收尾超时，会话已关闭；执行结果待核实"));
                }
            }
        }
    }
}

async fn pg_send_cancel(session: &PgSession, profile: &fluxdb_core::PostgresConnectionProfile) -> fluxdb_core::Result<()> {
    let stream = pg_transport_stream(profile, session._tunnel.as_deref()).await?;
    let token = session.client.cancel_token();
    match pg_tls_connect(profile)? {
        Some(mut tls) => {
            let ready = tokio_postgres::tls::MakeTlsConnect::<tokio::net::TcpStream>::make_tls_connect(&mut tls, pg_server_name(profile))
                .map_err(|e| Error::new(ErrorKind::Connection, format!("取消 TLS 配置失败: {e}")))?;
            token.cancel_query_raw(stream, ready).await.map_err(pg_error)
        }
        None => token.cancel_query_raw(stream, tokio_postgres::NoTls).await.map_err(pg_error),
    }
}

async fn pg_collect_statement(client: &tokio_postgres::Client, statement: &str, request: &QueryRequest) -> fluxdb_core::Result<PgStreamResult> {
    // 未拆分脚本可能含多语句，不可 prepare。恢复命令在失败事务中也必须直达服务端。
    let described = if request.options.split_statements && !pg_is_transaction_recovery(statement) {
        Some(client.prepare(statement).await.map_err(pg_error)?)
    } else { None };
    let stream = client.simple_query_raw(statement).await.map_err(pg_error)?;
    tokio::pin!(stream);
    let pagination = query_execution_pagination(request.options.page_offset, request.options.page_size);
    let row_capacity = query_execution_row_capacity(pagination.limit);
    let mut completed = Vec::new();
    let mut page: Option<DataPage> = None;
    let mut returned_rows: u64 = 0;
    let mut stored_bytes: usize = 0;
    // 大单元格和很多列也受内存预算约束；展示截断不影响服务端写入是否成功。
    const RESULT_BYTES: usize = 16 * 1024 * 1024;
    while let Some(message) = stream.next().await {
        let message = match message {
            Ok(message) => message,
            Err(error) => return Ok((completed, Some(pg_error(error)))),
        };
        match message {
            tokio_postgres::SimpleQueryMessage::RowDescription(columns) => {
                let columns = columns.iter().enumerate().map(|(index, col)| {
                    let mut column = query_column(col.name(), "text");
                    column.type_name = described.as_ref().and_then(|s| s.columns().get(index)).map(|c| c.type_().name().to_string());
                    column
                }).collect();
                page = Some(DataPage { columns, rows: vec![], offset: pagination.offset, limit: pagination.limit, has_more: false });
                returned_rows = 0;
            }
            tokio_postgres::SimpleQueryMessage::Row(row) => {
                if let Some(page) = &mut page {
                    if returned_rows >= pagination.offset && page.rows.len() < row_capacity {
                        let row_bytes = (0..row.len()).map(|i| row.get(i).map_or(0, str::len)).sum::<usize>();
                        if stored_bytes.saturating_add(row_bytes) <= RESULT_BYTES {
                            let values = page.columns.iter().enumerate().map(|(i, col)| pg_text_value(row.get(i), col)).collect::<fluxdb_core::Result<Vec<_>>>()?;
                            page.rows.push(Row { values });
                            stored_bytes += row_bytes;
                        } else { page.has_more = true; }
                    } else if returned_rows >= pagination.offset { page.has_more = true; }
                }
                returned_rows += 1;
            }
            tokio_postgres::SimpleQueryMessage::CommandComplete(count) => {
                // SELECT 的 command count 是返回行数，不是修改行数。RETURNING 同时保留二者。
                let word = pg_executable_sql(statement).split_whitespace().next().unwrap_or("");
                let affected_rows = if ["SELECT", "SHOW", "TABLE", "VALUES", "EXPLAIN"].iter().any(|s| word.eq_ignore_ascii_case(s)) { 0 } else { count };
                completed.push(PgCommandResult { page: page.take(), returned_rows, affected_rows });
                returned_rows = 0;
            }
            _ => {}
        }
    }
    Ok((completed, None))
}
