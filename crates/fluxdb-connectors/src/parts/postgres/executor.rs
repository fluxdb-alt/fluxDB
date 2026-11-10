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
    let statements = query_statements_for_execution(request, config.kind);
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
                    // 资源不足与「还有下一页」必须分开说：前者要告诉用户为什么停在这里，
                    // 以及下一页从哪继续，不能只说「已截断」。
                    if let Some(incomplete) = &page.incomplete {
                        format!(
                            "返回 {} 行，当前展示 {} 行（{}）；下一页从第 {} 行继续",
                            result.returned_rows,
                            page.rows.len(),
                            incomplete.reason,
                            page.next_offset() + 1
                        )
                    } else if page.has_more {
                        format!("返回 {} 行，当前展示 {} 行（还有更多）", result.returned_rows, page.rows.len())
                    } else {
                        format!("返回 {} 行", result.returned_rows)
                    }
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
    // 只读入口：读写工具复用同一条后端会话，不能把连接/会话置成只读（会污染后续写工具），
    // 也不能包成跨请求事务（会留下 idle-in-transaction）。因此逐语句把它包进一个带取消保护的
    // 只读事务：`BEGIN READ ONLY` → 语句 → `ROLLBACK`。即使用户 SQL 自行 BEGIN/COMMIT，
    // 也只会在本只读包装内生效；真正的外部 BEGIN 已被只读入口的策略层拒绝。
    let wrap_read_only = request.options.read_only && !pg_is_transaction_recovery(statement);
    let run = async {
        if !pg_is_transaction_recovery(statement) {
            pg_ensure_search_path(
                session,
                request.schema.as_deref(),
                &profile.scope.default_schema,
            )
            .await?;
        }
        if wrap_read_only {
            session.client.batch_execute("BEGIN READ ONLY").await.map_err(pg_error)?;
            let result = pg_collect_statement(&session.client, statement, request).await;
            // 无论成功与否都结束事务：不回滚会把连接留在 idle-in-transaction，并让后续写工具失败。
            if let Err(error) = session.client.batch_execute("ROLLBACK").await {
                // 回滚没确认：只读事务不会写入数据，但会话仍留在事务里，
                // 必须让调用方知道本次收尾失败，不能把结果当干净地交回去。
                tracing::warn!(target: "fluxdb_connectors", message = %error, "PostgreSQL 只读事务 ROLLBACK 失败");
                return Err(Error::new(ErrorKind::Query, format!("回滚 PostgreSQL 只读事务失败：{error}")));
            }
            return result;
        }
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
    let owner = fluxdb_core::result_content_owner(request.session_id, request.connection_id);
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
                // 多语句脚本：每条结果集各自收集，不能沿用上一条的页。
                page = Some(DataPage { columns, rows: vec![], offset: pagination.offset, limit: pagination.limit, has_more: false, incomplete: None });
                returned_rows = 0;
            }
            tokio_postgres::SimpleQueryMessage::Row(row) => {
                if let Some(page) = &mut page {
                    if returned_rows >= pagination.offset && page.rows.len() < row_capacity {
                        let mut values = page.columns.iter().enumerate().map(|(i, col)| pg_text_value(row.get(i), col)).collect::<fluxdb_core::Result<Vec<_>>>()?;
                        spill_large_cells(&owner, &mut values)?;
                        page.rows.push(Row { values });
                    } else if returned_rows >= pagination.offset {
                        page.has_more = true;
                    }
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

// ---- PostgreSQL numeric 精确编解码（T055）----
//
// 本版本 tokio-postgres/postgres-types 未内置 numeric 的 ToSql/FromSql（需 bigdecimal 集成）。
// 为**不丢精度**地绑定与解码 numeric，这里按 PostgreSQL 二进制 `numeric` 格式实现编解码：
//   int16 ndigits; int16 weight; uint16 sign(0=+,0x4000=-,0xC000=NaN);
//   int16 dscale(小数位); int16 digits[ndigits]（每项 0..9999，base-10000）。
// 全部用十进制字符串运算，不经过 f64。

/// 精确十进制数：符号 + 整数位数字串 + 小数位数字串（已去首尾多余 0，保留 dscale）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PgNumeric {
    negative: bool,
    /// display scale（小数位数）。
    scale: u32,
    /// base-10000 分组（已规范化）。
    digits: Vec<i16>,
    /// 首个分组的权重（10000^weight）。
    weight: i16,
}

impl PgNumeric {
    /// 解析十进制文本（`[+-]?digits[.digits]`）；拒绝 NaN/Inf/科学计数。
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (negative, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err("不是合法十进制文本".into());
        }
        let (int_part, frac_part) = match body.split_once('.') {
            Some((i, f)) => (i, f),
            None => (body, ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return Err("不是合法十进制文本".into());
        }
        let scale = frac_part.len() as u32;
        // 左侧补齐到 4 的倍数，切成 base-10000 分组。
        let mut int_padded = int_part.to_string();
        while int_padded.len() % 4 != 0 {
            int_padded.insert(0, '0');
        }
        let mut frac_padded = frac_part.to_string();
        while frac_padded.len() % 4 != 0 {
            frac_padded.push('0');
        }
        let mut groups: Vec<i16> = Vec::new();
        for chunk in int_padded.as_bytes().chunks(4) {
            groups.push(std::str::from_utf8(chunk).unwrap().parse::<i16>().unwrap());
        }
        for chunk in frac_padded.as_bytes().chunks(4) {
            groups.push(std::str::from_utf8(chunk).unwrap().parse::<i16>().unwrap());
        }
        // weight = 整数分组数 - 1；若整数部分为空（如 ".5"）则补一组 0。
        let int_groups = if int_part.is_empty() { 1 } else { int_part.len().div_ceil(4) };
        let mut weight = int_groups as i16 - 1;
        // 去前导 0 组。
        let mut start = 0;
        while start < groups.len() && groups[start] == 0 {
            start += 1;
            weight -= 1;
        }
        let mut normalized = groups[start..].to_vec();
        // 去尾部 0 组。
        while normalized.last() == Some(&0) {
            normalized.pop();
        }
        let negative = negative && normalized.iter().any(|d| *d != 0);
        Ok(Self {
            negative,
            scale,
            digits: normalized,
            weight,
        })
    }

    /// 编码为 PostgreSQL numeric 二进制。
    fn encode(&self, out: &mut tokio_postgres::types::private::BytesMut) {
        out.extend_from_slice(&(self.digits.len() as i16).to_be_bytes());
        let weight = if self.digits.is_empty() { 0 } else { self.weight };
        out.extend_from_slice(&weight.to_be_bytes());
        let sign: u16 = if self.negative { 0x4000 } else { 0x0000 };
        out.extend_from_slice(&sign.to_be_bytes());
        out.extend_from_slice(&(self.scale as i16).to_be_bytes());
        for digit in &self.digits {
            out.extend_from_slice(&digit.to_be_bytes());
        }
    }

    /// 转为十进制文本（保留 scale）。
    pub(crate) fn to_text(&self) -> CellValue {
        CellValue::Text(self.render())
    }

    /// 由已解析字段渲染十进制文本（保留 scale）。
    fn render(&self) -> String {
        if self.digits.is_empty() {
            // 0 也要保留 scale（如 0.00）。
            return if self.scale == 0 {
                "0".into()
            } else {
                format!("0.{}", "0".repeat(self.scale as usize))
            };
        }
        // 整数分组：weight+1 个分组（不足补 0），前导分组去零。
        let mut int_text = String::new();
        if self.weight >= 0 {
            for i in 0..=self.weight as usize {
                let group = self.digits.get(i).copied().unwrap_or(0) as u32;
                if i == 0 {
                    int_text.push_str(&group.to_string());
                } else {
                    int_text.push_str(&format!("{group:04}"));
                }
            }
        }
        if int_text.is_empty() {
            int_text.push('0');
        }
        let mut frac_text = String::new();
        let mut idx = self.weight + 1;
        while (frac_text.len() as i16) < self.scale as i16 {
            let group = if idx >= 0 { self.digits.get(idx as usize).copied().unwrap_or(0) } else { 0 } as u32;
            frac_text.push_str(&format!("{group:04}"));
            idx += 1;
        }
        frac_text.truncate(self.scale as usize);
        let sign = if self.negative { "-" } else { "" };
        if self.scale > 0 {
            format!("{sign}{int_text}.{frac_text}")
        } else {
            format!("{sign}{int_text}")
        }
    }

    /// 从 PostgreSQL numeric 二进制解码（保留 dscale，不经 f64）。
    fn decode(raw: &[u8]) -> Result<String, String> {
        if raw.len() < 8 {
            return Err("numeric 载荷过短".into());
        }
        let read_i16 = |o: usize| i16::from_be_bytes([raw[o], raw[o + 1]]);
        let ndigits = read_i16(0);
        let weight = read_i16(2);
        let sign = u16::from_be_bytes([raw[4], raw[5]]);
        let dscale = read_i16(6);
        if ndigits < 0 || dscale < 0 {
            return Err("numeric 头部非法".into());
        }
        if sign == 0xC000 {
            return Ok("NaN".into());
        }
        let base = 8usize;
        let mut digits = Vec::with_capacity(ndigits as usize);
        for i in 0..ndigits as usize {
            let offset = base + i * 2;
            if offset + 1 >= raw.len() {
                return Err("numeric 数字段截断".into());
            }
            digits.push(read_i16(offset));
        }
        let value = PgNumeric {
            negative: sign == 0x4000,
            scale: dscale as u32,
            digits,
            weight,
        };
        Ok(value.render())
    }
}


impl tokio_postgres::types::ToSql for PgNumeric {
    fn to_sql(
        &self,
        _ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.encode(out);
        Ok(tokio_postgres::types::IsNull::No)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        ty == &tokio_postgres::types::Type::NUMERIC
    }

    tokio_postgres::types::to_sql_checked!();
}

impl<'a> tokio_postgres::types::FromSql<'a> for PgNumeric {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let text = PgNumeric::decode(raw)?;
        PgNumeric::parse(&text).map_err(Into::into)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        ty == &tokio_postgres::types::Type::NUMERIC
    }
}

/// PostgreSQL 参数化单语句执行（T055）。
///
/// 用**扩展查询协议**：先 `prepare` 取得每个占位符的期望类型，再把 `SqlParameter`
/// 强制转换成该类型后真绑定（`int2/int4/int8/float4/float8/bool/text/varchar/bytea/numeric`），
/// **绝不**把参数拼进 SQL 文本。只接受单条语句。
pub(crate) fn pg_execute_parameterized(
    config: &ConnectionConfig,
    request: &QueryRequest,
    parameters: &[fluxdb_core::SqlParameter],
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statement = request.text.trim().to_string();
    if statement.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }
    if statement.contains(';') && statement.trim_end().trim_end_matches(';').contains(';') {
        return Err(Error::new(
            ErrorKind::Query,
            "参数化执行只接受单条语句；不支持多语句脚本",
        ));
    }
    pg_runtime().block_on(async {
        let database = pg_request_database(config, request.database.as_deref());
        let session = pg_session_acquire(config, request, &database).await?;
        let lock = session.execution.lock();
        tokio::pin!(lock);
        let _guard = lock.await;
        if session.driver.0.is_finished() || session.client.is_closed() {
            return Err(Error::new(ErrorKind::Connection, "查询会话已失效，执行结果请先核实；关闭标签后重新连接"));
        }
        let profile = config
            .postgres_profile
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL 连接档案缺失"))?;
        pg_ensure_search_path(
            &session,
            request.schema.as_deref(),
            &profile.scope.default_schema,
        )
        .await?;
        // prepare 只描述参数类型，不执行用户语句。
        let prepared = session
            .client
            .prepare(statement.as_str())
            .await
            .map_err(pg_error)?;
        let expected = prepared.params();
        if expected.len() != parameters.len() {
            return Err(Error::new(
                ErrorKind::Query,
                format!(
                    "参数个数不匹配：SQL 期望 {} 个，实际传入 {} 个",
                    expected.len(),
                    parameters.len()
                ),
            ));
        }
        let mut bound: Vec<PgBound> = Vec::with_capacity(parameters.len());
        for (index, (parameter, ty)) in parameters.iter().zip(expected.iter()).enumerate() {
            bound.push(PgBound::coerce(parameter, ty).map_err(|reason| {
                Error::new(
                    ErrorKind::Query,
                    format!("参数 {} 与占位符类型 {} 不兼容：{reason}", index + 1, ty.name()),
                )
            })?);
        }
        let bind_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            bound.iter().map(PgBound::as_tosql).collect();
        let started = Instant::now();
        let pagination = query_execution_pagination(request.options.page_offset, request.options.page_size);
        let capacity = query_execution_row_capacity(pagination.limit);
        let has_result_columns = !prepared.columns().is_empty();
        if statement_returns_rows(&statement) || has_result_columns {
            let stream = session
                .client
                .query_raw(&prepared, bind_refs.iter().copied())
                .await
                .map_err(pg_error)?;
            let page = pg_collect_page(
                request,
                stream,
                pg_prepared_columns(&prepared),
                pagination.offset,
                pagination.limit,
                capacity,
            )
            .await?;
            let returned = page.rows.len() as u64;
            return Ok(QueryExecutionResult {
                summaries: vec![QueryExecutionSummary {
                    sql: statement,
                    kind: QueryStatementKind::ResultSet,
                    success: true,
                    message: format!("返回 {returned} 行结果表（参数化绑定）"),
                    returned_rows: returned,
                    affected_rows: 0,
                    elapsed_ms: elapsed_ms(started),
                }],
                results: vec![page],
                rollback_snapshots: Vec::new(),
            });
        }
        let affected = session
            .client
            .execute_raw(&prepared, bind_refs.iter().copied())
            .await
            .map_err(pg_error)?;
        Ok(QueryExecutionResult {
            summaries: vec![QueryExecutionSummary {
                sql: statement,
                kind: QueryStatementKind::Command,
                success: true,
                message: format!("OK，影响 {affected} 行（参数化绑定）"),
                returned_rows: 0,
                affected_rows: affected,
                elapsed_ms: elapsed_ms(started),
            }],
            results: Vec::new(),
            rollback_snapshots: Vec::new(),
        })
    })
}

/// 按服务端期望类型强制转换后的绑定值。
enum PgBound {
    Null,
    NullText,
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    /// 精确 numeric（base-10000 二进制编码，不经 f64）。
    Numeric(PgNumeric),
}

impl PgBound {
    /// 把 tagged 参数强制转换为占位符的期望类型；不兼容时给出明确原因。
    fn coerce(
        parameter: &fluxdb_core::SqlParameter,
        ty: &tokio_postgres::types::Type,
    ) -> Result<Self, String> {
        use fluxdb_core::SqlParameter;
        use tokio_postgres::types::Type;
        if matches!(parameter, SqlParameter::Null) {
            return Ok(match *ty {
                Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                    PgBound::NullText
                }
                _ => PgBound::Null,
            });
        }
        let as_i64 = || -> Result<i64, String> {
            match parameter {
                SqlParameter::I64(t) | SqlParameter::U64(t) => {
                    t.parse::<i64>().map_err(|_| format!("不是整数: {t}"))
                }
                SqlParameter::Bool(b) => Ok(i64::from(*b)),
                SqlParameter::F64(v)
                    if v.fract() == 0.0 && *v >= i64::MIN as f64 && *v <= i64::MAX as f64 =>
                {
                    Ok(*v as i64)
                }
                _ => Err("无法转换为整数".into()),
            }
        };
        let as_f64 = || -> Result<f64, String> {
            match parameter {
                SqlParameter::F64(v) => Ok(*v),
                SqlParameter::I64(t) | SqlParameter::U64(t) | SqlParameter::Decimal(t) => {
                    t.parse::<f64>().map_err(|_| format!("不是数值: {t}"))
                }
                SqlParameter::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
                _ => Err("无法转换为浮点".into()),
            }
        };
        match *ty {
            Type::BOOL => match parameter {
                SqlParameter::Bool(b) => Ok(PgBound::Bool(*b)),
                SqlParameter::I64(t) | SqlParameter::U64(t) => Ok(PgBound::Bool(t != "0")),
                _ => Err("无法转换为 bool".into()),
            },
            Type::INT2 => Ok(PgBound::I16(
                i16::try_from(as_i64()?).map_err(|_| "超出 int2 范围")?,
            )),
            Type::INT4 => Ok(PgBound::I32(
                i32::try_from(as_i64()?).map_err(|_| "超出 int4 范围")?,
            )),
            Type::INT8 => Ok(PgBound::I64(as_i64()?)),
            Type::FLOAT4 => Ok(PgBound::F32(as_f64()? as f32)),
            Type::FLOAT8 => Ok(PgBound::F64(as_f64()?)),
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                Ok(PgBound::Text(value_text(parameter)))
            }
            Type::BYTEA => match parameter {
                SqlParameter::Bytes(b) => Ok(PgBound::Bytes(b.clone())),
                SqlParameter::Text(t) => Ok(PgBound::Bytes(t.clone().into_bytes())),
                _ => Err("无法转换为 bytea".into()),
            },
            // numeric/decimal：以十进制**文本**绑定；调用方 SQL 应用 `$n` 且服务端按 numeric 解析文本。
            // 仍不拼接进 SQL 文本，参数经协议绑定通道下发。
            ref other if other.name() == "numeric" => {
                PgNumeric::parse(&value_text(parameter))
                    .map(PgBound::Numeric)
                    .map_err(|reason| format!("numeric 参数非法：{reason}"))
            }
            ref other => Err(format!("暂不支持的占位符类型 {}", other.name())),
        }
    }

    fn as_tosql(&self) -> &(dyn tokio_postgres::types::ToSql + Sync) {
        match self {
            PgBound::Null => &PG_NULL,
            PgBound::NullText => &PG_NULL_TEXT,
            PgBound::Bool(v) => v,
            PgBound::I16(v) => v,
            PgBound::I32(v) => v,
            PgBound::I64(v) => v,
            PgBound::F32(v) => v,
            PgBound::F64(v) => v,
            PgBound::Text(v) => v,
            PgBound::Bytes(v) => v,
            PgBound::Numeric(v) => v,
        }
    }
}

/// 参数的文本投影（供 TEXT/numeric 类占位符）。
fn value_text(parameter: &fluxdb_core::SqlParameter) -> String {
    use fluxdb_core::SqlParameter;
    match parameter {
        SqlParameter::Text(v)
        | SqlParameter::Decimal(v)
        | SqlParameter::I64(v)
        | SqlParameter::U64(v) => v.clone(),
        SqlParameter::Bool(b) => {
            if *b {
                "t".into()
            } else {
                "f".into()
            }
        }
        SqlParameter::F64(v) => v.to_string(),
        SqlParameter::Null => String::new(),
        SqlParameter::Bytes(_) => String::new(),
    }
}

/// `NULL` 绑定：静态避免引用临时。
static PG_NULL: Option<i32> = None;
static PG_NULL_TEXT: Option<String> = None;

/// 扩展协议的行流边读边收页：先按当前描述建列，再逐行预算 + 单格外置。
///
/// `pg_rows_to_page` 的流式版本：`query_raw` 读一行解一行，不再先攒 `Vec<Row>`
/// （一行 100 MB 乘以行数就是用户看到的那几个 GB）。未读的行留在驱动里，
/// 连接关掉就一起释放；`next_offset()` 保证下一页从第一条未收录行继续。
async fn pg_collect_page(
    request: &QueryRequest,
    stream: impl futures_util::Stream<Item = Result<tokio_postgres::Row, tokio_postgres::Error>>,
    columns: Vec<Column>,
    offset: u64,
    limit: u64,
    capacity: usize,
) -> fluxdb_core::Result<DataPage> {
    let owner = fluxdb_core::result_content_owner(request.session_id, request.connection_id);
    let mut page = DataPage {
        columns: columns.clone(),
        rows: Vec::new(),
        offset,
        limit,
        has_more: false,
        incomplete: None,
    };
    let mut consumed = 0u64;
    futures_util::pin_mut!(stream);
    while let Some(row) = stream.next().await {
        let row = row.map_err(pg_error)?;
        consumed += 1;
        if consumed <= offset {
            continue;
        }
        if page.rows.len() >= capacity {
            page.has_more = true;
            break;
        }
        let mut values = Vec::with_capacity(columns.len());
        for (col_index, column) in columns.iter().enumerate() {
            values.push(pg_row_cell(&row, col_index, column)?);
        }
        spill_large_cells(&owner, &mut values)?;
        page.rows.push(Row { values });
    }
    // 预算截断时后面还有行；这不是「正常还有更多」，两个状态不能同时成立。
    if page.incomplete.is_some() {
        page.has_more = false;
    }
    Ok(page)
}

/// 参数化查询的列定义来自服务端 prepare 结果（零行结果也保留列结构）。
fn pg_prepared_columns(statement: &tokio_postgres::Statement) -> Vec<Column> {
    statement
        .columns()
        .iter()
        .map(|col| {
            let mut column = query_column(col.name(), col.type_().name());
            column.type_name = Some(col.type_().name().to_string());
            column
        })
        .collect()
}

/// numeric 单元格转为受标签文本（保留 scale），避免经 f64 丢精度。
fn numeric_to_text(value: PgNumeric) -> CellValue {
    value.to_text()
}

/// 参数化行取单元格：按列类型的原生解码。
fn pg_row_cell(
    row: &tokio_postgres::Row,
    index: usize,
    column: &Column,
) -> fluxdb_core::Result<CellValue> {
    let type_name = column.type_name.as_deref().unwrap_or("text");
    let value = match type_name {
        "int2" => row.try_get::<_, Option<i16>>(index).ok().flatten().map(|v| CellValue::I64(i64::from(v))),
        "int4" => row.try_get::<_, Option<i32>>(index).ok().flatten().map(|v| CellValue::I64(i64::from(v))),
        "int8" => row.try_get::<_, Option<i64>>(index).ok().flatten().map(CellValue::I64),
        "float4" => row.try_get::<_, Option<f32>>(index).ok().flatten().map(|v| CellValue::F64(f64::from(v))),
        "float8" => row.try_get::<_, Option<f64>>(index).ok().flatten().map(CellValue::F64),
        "bool" => row.try_get::<_, Option<bool>>(index).ok().flatten().map(CellValue::Bool),
        "bytea" => row.try_get::<_, Option<Vec<u8>>>(index).ok().flatten().map(CellValue::Bytes),
        "numeric" => row
            .try_get::<_, Option<PgNumeric>>(index)
            .ok()
            .flatten()
            .map(numeric_to_text),
        _ => row.try_get::<_, Option<String>>(index).ok().flatten().map(CellValue::Text),
    };
    Ok(match value {
        Some(value) => value,
        None => match row.try_get::<_, Option<String>>(index) {
            Ok(None) | Err(_) => CellValue::Null,
            Ok(Some(text)) => CellValue::Text(text),
        },
    })
}

