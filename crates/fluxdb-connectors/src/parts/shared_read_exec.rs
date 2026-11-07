fn mysql_execute_query(
    config: &ConnectionConfig,
    request: &QueryRequest,
) -> fluxdb_core::Result<QueryExecutionResult> {
    mysql_execute_query_with_progress(config, request, &mut |_| {}, &|| false)
}

/// 单条 MySQL 语句的执行结局：服务端产出 + 失败 + 是否用户「停止」取消。
struct MySqlStatementOutcome {
    page: Option<DataPage>,
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
    plan: &MySqlConnectionPlan,
    connection: &mut MySqlConnection,
    conn_id: u64,
    statement: &str,
    pagination: Pagination,
    owner: &str,
    should_cancel: &dyn Fn() -> bool,
) -> MySqlStatementOutcome {
    let returns_rows = statement_returns_rows(statement);
    let run = async {
        if returns_rows {
            // 流式收页：`fetch_all` 会先把整个结果集堆进 `Vec<Row>`，
            // 一条 8 MB 的行乘以行数就是几百 MB，预算在那之后才生效。
            let page = collect_sqlx_page(
                sqlx::query(statement).fetch(&mut *connection),
                Vec::new(),
                pagination,
                pagination.offset,
                owner,
                mysql_query_cell_value,
                mysql_error,
            )
            .await
            .map(|(page, _read_rows)| page);
            match page {
                Ok(page) => MySqlStatementOutcome {
                    page: Some(page),
                    affected_rows: 0,
                    error: None,
                    user_cancelled: false,
                },
                Err(error) => MySqlStatementOutcome {
                    page: None,
                    affected_rows: 0,
                    error: Some(error),
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
                    page: None,
                    affected_rows: result.rows_affected(),
                    error: None,
                    user_cancelled: false,
                });
            match outcome {
                Ok(outcome) => outcome,
                Err(error) => MySqlStatementOutcome {
                    page: None,
                    affected_rows: 0,
                    error: Some(mysql_error(error)),
                    user_cancelled: false,
                },
            }
        }
    };
    tokio::pin!(run);
    let started = Instant::now();
    let query_timeout = plan.query_timeout();
    let mut kill_sent = false;
    let mut kill_succeeded = false;
    let mut timeout_hit = false;
    loop {
        tokio::select! {
            outcome = &mut run => {
                let mut outcome = outcome;
                // 超过档案里的查询超时：语句已被 KILL 打断，统一报「查询超时」，不返回
                // 被打断后的结果。注意 `SLEEP()` 被 KILL 时不会报错，而是返回 `1` 并正常
                // 结束，只看 error 会把超时误判成成功，因此这里以截止时间为准。
                // 语句也可能在 100ms 计时 tick 之前就返回（服务端 max_execution_time 同样
                // 会打断），此时 timeout_hit 还没置位；用截止时间兜底，避免 select 分支
                // 竞争把已超时的语句误报成成功。
                let timed_out = timeout_hit
                    || (!kill_sent
                        && query_timeout.is_some_and(|limit| started.elapsed() >= limit));
                if timed_out {
                    if let Some(error) = outcome.error.take() {
                        tracing::info!(
                            target: "fluxdb_connectors",
                            connection_id = conn_id,
                            timeout_secs = query_timeout.map(|limit| limit.as_secs()).unwrap_or(0),
                            server_message = %error.message,
                            "MySQL 查询超时已中止语句"
                        );
                    }
                    outcome.page = None;
                    outcome.affected_rows = 0;
                    outcome.error = Some(Error::new(
                        ErrorKind::Query,
                        format!(
                            "查询超时（超过 {} 秒）",
                            query_timeout.map(|limit| limit.as_secs()).unwrap_or(0)
                        ),
                    ));
                }
                // 用户主动取消：只有服务端确认接收 KILL，且主连接以错误结束时，才能认定
                // 本语句被取消；若 KILL 没赶上而语句正常完成，仍保留真实成功结果。
                outcome.user_cancelled = kill_succeeded && !timed_out && outcome.error.is_some();
                return outcome;
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                let timed_out = query_timeout.is_some_and(|limit| started.elapsed() >= limit);
                if !kill_sent && (should_cancel() || timed_out) {
                    kill_sent = true;
                    // 用户主动停止优先于超时：两者都命中时按取消语义标注结果待核实。
                    timeout_hit = timed_out && !should_cancel();
                    // KILL QUERY 只中止当前语句，随后主连接会收到该语句超时/中断错误。
                    match mysql_send_kill_query(plan, conn_id).await {
                        Ok(()) => {
                            kill_succeeded = true;
                            tracing::info!(
                                target: "fluxdb_connectors",
                                connection_id = conn_id,
                                timed_out = timeout_hit,
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
///
/// 复用同一条 SSH 隧道与建连超时：隧道模式下 kill 连接同样只能经本地转发端口进入。
async fn mysql_send_kill_query(
    plan: &MySqlConnectionPlan,
    conn_id: u64,
) -> fluxdb_core::Result<()> {
    let mut kill_conn = plan.connect().await?;
    let killed = sqlx::raw_sql(&format!("KILL QUERY {}", conn_id))
        .execute(&mut kill_conn)
        .await
        .map_err(mysql_error);
    let _ = kill_conn.close().await;
    killed.map(|_| ())
}

/// 参数化单语句执行（MySQL）：**必须**预编译 + bind，绝不做字符串插值。
///
/// 只接受单条语句（多语句会破坏"参数属于哪条语句"的绑定语义，直接拒绝）。
pub(crate) fn mysql_execute_parameterized(
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
    // 计划持有 SSH 隧道与超时；必须活到语句执行结束。
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        if let Some(database) = request.database.as_deref().filter(|database| !database.is_empty()) {
            sqlx::raw_sql(&format!("USE {}", fluxdb_core::MySqlDialect.quote_identifier(database)))
                .execute(&mut connection)
                .await
                .map_err(mysql_error)?;
        }
        let run = async {
        let pagination = query_execution_pagination(request.options.page_offset, request.options.page_size);
        let started = std::time::Instant::now();
        if statement_returns_rows(&statement) {
            let mut query = sqlx::query(&statement);
            for parameter in parameters {
                query = mysql_bind_parameter(query, parameter)?;
            }
            let owner =
                fluxdb_core::result_content_owner(request.session_id, request.connection_id);
            let (page, _read_rows) = collect_sqlx_page(
                query.fetch(&mut connection),
                Vec::new(),
                pagination,
                pagination.offset,
                &owner,
                mysql_query_cell_value,
                mysql_error,
            )
            .await?;
            let returned = page.rows.len() as u64;
            let summary = QueryExecutionSummary {
                sql: statement,
                kind: QueryStatementKind::ResultSet,
                success: true,
                message: format!("返回 {returned} 行结果表（参数化绑定）"),
                returned_rows: returned,
                affected_rows: 0,
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
            return Ok(QueryExecutionResult {
                summaries: vec![summary],
                results: vec![page],
                rollback_snapshots: Vec::new(),
            });
        }
        let mut query = sqlx::query(&statement);
        for parameter in parameters {
            query = mysql_bind_parameter(query, parameter)?;
        }
        let outcome = query.execute(&mut connection).await.map_err(mysql_error)?;
        let affected = outcome.rows_affected();
        let summary = QueryExecutionSummary {
            sql: statement,
            kind: QueryStatementKind::Command,
            success: true,
            message: format!("影响 {affected} 行（参数化绑定）"),
            returned_rows: 0,
            affected_rows: affected,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
        Ok(QueryExecutionResult {
            summaries: vec![summary],
            results: Vec::new(),
            rollback_snapshots: Vec::new(),
        })
        };
        // 参数化路径不做 KILL QUERY，超时只放弃客户端等待；默认 0（不限制），
        // 仅为用户显式设置的查询超时兜底，避免单条语句无限挂住执行器。
        let execution = match plan.query_timeout() {
            Some(limit) => tokio::time::timeout(limit, run)
                .await
                .map_err(|_| {
                    Error::new(
                        ErrorKind::Query,
                        format!("查询超时（超过 {} 秒）", limit.as_secs()),
                    )
                })??,
            None => run.await?,
        };
        Ok(execution)
    })
}

/// 精确参数 → MySQL 绑定值。十进制用文本绑定（仍走协议参数，不做 SQL 拼接）。
fn mysql_bind_parameter<'q>(
    query: sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>,
    parameter: &'q fluxdb_core::SqlParameter,
) -> fluxdb_core::Result<sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>> {
    use fluxdb_core::SqlParameter;
    Ok(match parameter {
        SqlParameter::Null => query.bind(None::<String>),
        SqlParameter::Bool(value) => query.bind(*value),
        SqlParameter::I64(text) => query.bind(text.parse::<i64>().map_err(|_| {
            Error::new(ErrorKind::Query, "i64 参数不是合法的十进制文本")
        })?),
        SqlParameter::U64(text) => query.bind(text.parse::<u64>().map_err(|_| {
            Error::new(ErrorKind::Query, "u64 参数不是合法的十进制文本")
        })?),
        SqlParameter::Decimal(text) => query.bind(text.clone()),
        SqlParameter::F64(value) => {
            if !value.is_finite() {
                return Err(Error::new(ErrorKind::Query, "f64 参数必须是有限数值"));
            }
            query.bind(*value)
        }
        SqlParameter::Text(value) => query.bind(value.clone()),
        SqlParameter::Bytes(value) => query.bind(value.clone()),
    })
}

fn mysql_execute_query_with_progress(
    config: &ConnectionConfig,
    request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statements = query_statements_for_execution(request, config.kind);
    if statements.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }

    // 计划持有 SSH 隧道与超时；必须活到本批次所有语句执行结束。
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;

        if let Some(database) = request.database.as_deref().filter(|database| !database.is_empty())
        {
            sqlx::raw_sql(&format!("USE {}", fluxdb_core::MySqlDialect.quote_identifier(database)))
                .execute(&mut connection)
                .await
                .map_err(mysql_error)?;
        }

        // 只读入口：连接为本次调用新建（不复用），故会话级只读是安全的；默认事务因此只读，
        // 未在 denylist 里的 UDF/DML 由服务端拒绝，作为客户端 AST 策略之外的纵深。
        if request.options.read_only {
            sqlx::raw_sql("SET SESSION TRANSACTION READ ONLY")
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
                &plan,
                &mut connection,
                conn_id,
                &statement,
                query_execution_pagination(request.options.page_offset, request.options.page_size),
                &fluxdb_core::result_content_owner(request.session_id, request.connection_id),
                should_cancel,
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
            if let Some(page) = outcome.page {
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
    let statements = query_statements_for_execution(request, config.kind);
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

        // 只读入口：连接为本次调用新建（不复用），故连接级 `PRAGMA query_only=ON` 不会外泄；
        // 即使客户端分类漏掉某个带副作用的函数，SQLite 也会直接拒绝写入。
        if request.options.read_only {
            sqlx::query("PRAGMA query_only = ON")
                .execute(&mut connection)
                .await
                .map_err(sqlite_error)?;
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
                let pagination = query_execution_pagination(
                    request.options.page_offset,
                    request.options.page_size,
                );
                let owner =
                    fluxdb_core::result_content_owner(request.session_id, request.connection_id);
                let mut page = match collect_sqlx_page(
                    sqlx::query(&statement).fetch(&mut connection),
                    Vec::new(),
                    pagination,
                    pagination.offset,
                    &owner,
                    sqlite_query_cell_value,
                    sqlite_error,
                )
                .await
                {
                    Ok((page, _read_rows)) => page,
                    Err(error) => {
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
                if page.columns.is_empty() {
                    // 零行结果不能丢列结构；只准备语句元数据，不重新执行查询取数。
                    let description = sqlx::Executor::describe(&mut connection, &statement)
                        .await.map_err(sqlite_error)?;
                    page.columns = description.columns().iter()
                        .map(|column| query_column(column.name(), column.type_info().name()))
                        .collect();
                }
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

/// 参数化单语句执行（SQLite）：**必须**以 `?` 绑定，绝不做字符串插值。
///
/// 只接受单条语句（参数与语句的对应关系在多语句下不明确，直接拒绝）。
pub(crate) fn sqlite_execute_parameterized(
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
        if request.options.read_only {
            sqlx::query("PRAGMA query_only = ON")
                .execute(&mut connection)
                .await
                .map_err(sqlite_error)?;
        }
        let started = std::time::Instant::now();
        let pagination =
            query_execution_pagination(request.options.page_offset, request.options.page_size);
        if statement_returns_rows(&statement) {
            let mut query = sqlx::query(&statement);
            for parameter in parameters {
                query = sqlite_bind_parameter(query, parameter)?;
            }
            let owner =
                fluxdb_core::result_content_owner(request.session_id, request.connection_id);
            let (mut page, _read_rows) = collect_sqlx_page(
                query.fetch(&mut connection),
                Vec::new(),
                pagination,
                pagination.offset,
                &owner,
                sqlite_query_cell_value,
                sqlite_error,
            )
            .await?;
            if page.columns.is_empty() {
                // 零行结果不能丢列结构：与脚本路径一致，只读语句元数据。
                let description = sqlx::Executor::describe(&mut connection, &statement)
                    .await
                    .map_err(sqlite_error)?;
                page.columns = description
                    .columns()
                    .iter()
                    .map(|column| query_column(column.name(), column.type_info().name()))
                    .collect();
            }
            connection.close().await.map_err(sqlite_error)?;
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
        let mut query = sqlx::query(&statement);
        for parameter in parameters {
            query = sqlite_bind_parameter(query, parameter)?;
        }
        let outcome = query.execute(&mut connection).await.map_err(sqlite_error)?;
        let affected = outcome.rows_affected();
        connection.close().await.map_err(sqlite_error)?;
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

fn sqlite_bind_parameter<'q>(
    query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    parameter: &'q fluxdb_core::SqlParameter,
) -> fluxdb_core::Result<sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>> {
    use fluxdb_core::SqlParameter;
    Ok(match parameter {
        SqlParameter::Null => query.bind(None::<String>),
        SqlParameter::Bool(value) => query.bind(i64::from(*value)),
        SqlParameter::I64(text) => query.bind(text.parse::<i64>().map_err(|_| {
            Error::new(ErrorKind::Query, "i64 参数不是合法的十进制文本")
        })?),
        // SQLite 无原生 u64；超出 i64 的十进制以文本绑定，保留精度。
        SqlParameter::U64(text) => match text.parse::<i64>() {
            Ok(value) => query.bind(value),
            Err(_) => query.bind(text.clone()),
        },
        // decimal 以文本绑定，避免经 f64 丢精度。
        SqlParameter::Decimal(text) => query.bind(text.clone()),
        SqlParameter::F64(value) => {
            if !value.is_finite() {
                return Err(Error::new(ErrorKind::Query, "f64 参数必须是有限数值"));
            }
            query.bind(*value)
        }
        SqlParameter::Text(value) => query.bind(value.clone()),
        SqlParameter::Bytes(value) => query.bind(value.clone()),
    })
}
