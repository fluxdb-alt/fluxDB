// PostgreSQL 执行使用服务端结果描述与 CommandComplete，不按首关键字猜测结果。
// 文本协议只执行一次；prepare 仅补充类型。流始终排空，内存只保存当前页。
use futures_util::StreamExt;

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
        for statement in statements {
            if should_cancel() { break; }
            let started = Instant::now();
            // 恢复语句不能被失败事务中的 SET/prepare 拦住；其余语句交由服务端决定能否执行。
            let result = pg_run_statement(config, &session, &statement, request, should_cancel).await;
            let (completed, error) = match result {
                Ok(result) => result,
                Err(error) => (vec![], Some(error)),
            };
            for result in completed {
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
            if let Some(error) = error {
                let fatal = matches!(error.kind, ErrorKind::Connection | ErrorKind::Cancelled | ErrorKind::Timeout);
                // 失败原因（含服务端 SQLSTATE/错误文本）必须在日志可见，否则排障只能靠重试。
                tracing::warn!(target: "fluxdb_connectors", kind = ?error.kind, message = %error.message, sql = %statement, "PostgreSQL 执行失败");
                push_query_summary(&mut execution,
                    failed_query_summary(statement, QueryStatementKind::Command, error.message, elapsed_ms(started)), on_summary);
                if fatal || !request.options.continue_on_error { break; }
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

type PgStreamResult = (Vec<PgCommandResult>, Option<Error>);

async fn pg_run_statement(
    config: &ConnectionConfig, session: &PgSession, statement: &str,
    request: &QueryRequest, should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<PgStreamResult> {
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
    let mut cancellation_started = None;
    loop {
        tokio::select! {
            result = &mut run => return result,
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                let timed_out = profile.advanced.query_timeout_secs > 0
                    && started.elapsed() >= Duration::from_secs(u64::from(profile.advanced.query_timeout_secs));
                if !cancel_sent && (should_cancel() || timed_out) {
                    cancel_sent = true;
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
    let pagination = Pagination::new(request.options.page_offset, request.options.page_size);
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
                    if returned_rows >= pagination.offset && page.rows.len() < pagination.limit as usize {
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
