// Oracle SQL 执行入口：编辑器查询、分步进度、参数化执行。
//
// 只读入口是两层防线，缺一不可：
// 1. 语句执行前按驱动解析出的类型拒绝非查询语句。Oracle 的 DDL 会隐式提交、
//    PL/SQL 可带自治事务，执行后再回滚也撤销不了，只能根本不执行。
// 2. 连接上先 `SET TRANSACTION READ ONLY`。它挡不住 DDL（隐式提交会结束只读事务
//    并另开读写事务），但能在服务端挡住绕过分类器漏进来的 DML 与 `FOR UPDATE`，
//    报 `ORA-01456` 而不是先改行再回滚。
// 序列 `NEXTVAL`、自治事务、dblink 写入不受只读事务约束，只能靠上层 AST 策略拒绝。

/// 取消命中在途语句：服务端执行到哪一步未知，既不能报成功，也不能报成确定失败。
const ORACLE_CANCELLED_OUTCOME_UNKNOWN: &str = "已取消：结果待核实";
/// 取消之后的语句根本没发出去，确定未执行。
const ORACLE_CANCELLED_NOT_EXECUTED: &str = "已取消：未执行";
/// 查询超时：驱动发 INTERRUPT 打断语句，改了哪些行无从得知。
const ORACLE_TIMEOUT_OUTCOME_UNKNOWN: &str = "已超时：结果待核实";

fn oracle_execute_query(
    config: &ConnectionConfig,
    request: &QueryRequest,
) -> fluxdb_core::Result<QueryExecutionResult> {
    oracle_execute_query_with_progress(config, request, &mut |_| {}, &|| false)
}

fn oracle_execute_query_with_progress(
    config: &ConnectionConfig,
    request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statements = oracle_statements_for_execution(request);
    if statements.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }
    // 请求级 schema 落到会话上，否则编辑器里选的 schema 只影响补全、不影响执行。
    let connection = oracle_connect_with_schema(config, request.schema.as_deref())?;
    if request.options.read_only {
        oracle_begin_read_only(&connection)?;
    }
    let pagination =
        query_execution_pagination(request.options.page_offset, request.options.page_size);
    let mut execution = QueryExecutionResult {
        summaries: Vec::new(),
        results: Vec::new(),
        rollback_snapshots: Vec::new(),
    };
    // 用户已停止：一旦取消生效，本批次剩余语句一律不再发送。不看 `should_cancel()`
    // 当前值——调用方可能在取消后复位标志，那样后面的语句会被静默继续执行。
    let mut stopped = false;
    for statement in statements {
        if stopped || should_cancel() {
            push_query_summary(
                &mut execution,
                failed_query_summary(
                    statement,
                    QueryStatementKind::Command,
                    ORACLE_CANCELLED_NOT_EXECUTED.into(),
                    0,
                ),
                on_summary,
            );
            continue;
        }
        let started = std::time::Instant::now();
        let owner =
            fluxdb_core::result_content_owner(request.session_id, request.connection_id);
        match oracle_execute_statement(
            &connection,
            &statement,
            &[],
            pagination,
            request.options.read_only,
            &owner,
            should_cancel,
        ) {
            Ok(OracleStatementOutcome::Rows(page, returned)) => {
                // 页满即止之后 `returned` 只是「读到多少行」，不再是结果集总行数：
                // 说「返回 N 行」会把没读完的部分谎报成不存在。
                let message = if let Some(incomplete) = &page.incomplete {
                    format!(
                        "已读取 {returned} 行，当前仅展示 {} 行（{}）",
                        page.rows.len(),
                        incomplete.reason
                    )
                } else if page.has_more {
                    format!(
                        "已读取 {returned} 行，当前仅展示 {} 行（结果已截断）",
                        page.rows.len()
                    )
                } else {
                    format!("返回 {returned} 行")
                };
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: QueryStatementKind::ResultSet,
                        success: true,
                        message,
                        returned_rows: returned,
                        affected_rows: 0,
                        elapsed_ms: elapsed_ms(started),
                    },
                    on_summary,
                );
                execution.results.push(page);
            }
            Ok(OracleStatementOutcome::Cancelled) => {
                // 语句已经发到服务端并开始返回行，收到的是哪一段不确定——不能当成功，
                // 也不能当确定失败。剩余语句由循环顶部逐条标注「未执行」。
                tracing::warn!(
                    target: "fluxdb_connectors",
                    sql = %statement,
                    "Oracle 查询在取行途中被取消"
                );
                push_query_summary(
                    &mut execution,
                    failed_query_summary(
                        statement,
                        QueryStatementKind::ResultSet,
                        ORACLE_CANCELLED_OUTCOME_UNKNOWN.into(),
                        elapsed_ms(started),
                    ),
                    on_summary,
                );
                stopped = true;
            }
            Ok(OracleStatementOutcome::Affected(affected, is_ddl)) => {
                // 只读入口到不了这里：非查询语句在执行前就被拒绝了。
                let (success, message) = if is_ddl {
                    (true, "OK（DDL 隐式提交）".to_string())
                } else {
                    (true, format!("OK，影响 {affected} 行"))
                };
                push_query_summary(
                    &mut execution,
                    QueryExecutionSummary {
                        sql: statement,
                        kind: QueryStatementKind::Command,
                        success,
                        message,
                        returned_rows: 0,
                        affected_rows: affected,
                        elapsed_ms: elapsed_ms(started),
                    },
                    on_summary,
                );
            }
            Err(error) => {
                // 驱动在 call timeout 时会先向服务端发 INTERRUPT 再重置会话：语句确实被打断了，
                // 但已经改了几行无从得知。带上「结果待核实」关键字，查询历史才会标成未知状态
                // （`query_history` 按这个关键字判断），不会被后续操作当成干净的失败。
                let message = if error.kind == ErrorKind::Timeout {
                    format!("{ORACLE_TIMEOUT_OUTCOME_UNKNOWN}（{}）", error.message)
                } else {
                    error.message.clone()
                };
                tracing::warn!(
                    target: "fluxdb_connectors",
                    kind = ?error.kind,
                    message = %error.message,
                    sql = %statement,
                    "Oracle 执行失败"
                );
                push_query_summary(
                    &mut execution,
                    failed_query_summary(
                        statement,
                        QueryStatementKind::Command,
                        message,
                        elapsed_ms(started),
                    ),
                    on_summary,
                );
                if matches!(error.kind, ErrorKind::Timeout | ErrorKind::Cancelled) {
                    // 超时/取消意味着驱动已经 INTERRUPT 过服务端，会话状态不再干净：
                    // 剩余语句一律不再发送，但仍逐条标注「未执行」保持与语句列表对齐。
                    stopped = true;
                    continue;
                }
                if !request.options.continue_on_error {
                    break;
                }
            }
        }
    }
    oracle_finish_session(&connection, request.options.read_only)?;
    Ok(execution)
}

/// 会话收尾。驱动不会自动提交 DML，连接关闭时未提交的更改会被丢弃，
/// 所以两条路径都必须显式 commit/rollback。
///
/// 收尾失败直接返回错误：只读入口的「未写入」结论依赖回滚真的成功，
/// 失败还回 `Ok` 会让调用方把未知状态当成成功。写入路径的提交失败还要标成
/// 「结果未知」——提交途中断网时服务端可能已经提交。
fn oracle_finish_session(
    connection: &OracleConnection,
    read_only: bool,
) -> fluxdb_core::Result<()> {
    let finish = if read_only {
        connection
            .rollback()
            .map_err(|error| oracle_error("回滚 Oracle 会话失败", error))
    } else {
        connection
            .commit()
            .map_err(|error| oracle_error("提交 Oracle 会话失败", error))
            .map_err(|error| error.mark_outcome_unknown("提交 Oracle 事务"))
    };
    finish.inspect_err(|error| {
        tracing::warn!(target: "fluxdb_connectors", message = %error.message, "Oracle 会话收尾失败");
    })
}

enum OracleStatementOutcome {
    Rows(DataPage, u64),
    Affected(u64, bool),
    /// 取行途中被取消：语句已在服务端执行过，收了哪些行不确定。
    Cancelled,
}

/// 执行一条语句。
///
/// `should_cancel` 只在**安全点**检查：驱动是同步阻塞实现，没有取消/中断 API，
/// 检查点只能落在两次取行之间。因此停止对「正在服务端计算」的语句不生效，
/// 只能等它返回或配置的查询超时触发；对持续吐行的语句则在一个取行批次内生效。
fn oracle_execute_statement(
    connection: &OracleConnection,
    statement: &str,
    binds: &[OracleBindValue],
    pagination: Pagination,
    read_only: bool,
    owner: &str,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<OracleStatementOutcome> {
    let refs = oracle_bind_refs(binds);
    let mut prepared = connection
        .statement(statement)
        .map_err(|error| oracle_error("Oracle 语句解析失败", error))?
        .exclude_from_cache()
        // 编辑器结果同样要按 LOB 取：否则驱动会把 CLOB 改写成 LONG 并整块物化，
        // `SELECT` 一个长 CLOB 就等于把全部内容拉进内存。理由同 `oracle_query`。
        .fetch_lobs()
        .build()
        .map_err(|error| oracle_error("Oracle 语句解析失败", error))?;
    // 只读入口必须在执行前拒绝，不能靠事后回滚：DDL 隐式提交撤销不了，
    // PL/SQL 块还可能带自治事务。驱动在 build 之后就能给出语句类型，
    // 这里只放行真正的查询（`SELECT` / `WITH ... SELECT`）。
    if read_only && !prepared.is_query() {
        return Err(Error::new(
            ErrorKind::Permission,
            "只读入口拒绝执行：该语句不是只读查询（未执行）",
        ));
    }
    // 先看驱动自己判定的语句类型：不要把 `INSERT ... RETURNING`、匿名 PL/SQL 块
    // 当作查询去取游标，也不要反过来把 `WITH ... SELECT` 当命令执行。
    if prepared.is_query() {
        let columns = prepared
            .out_metadata()
            .iter()
            .map(|metadata| oracle_column(metadata, false))
            .collect::<Vec<_>>();
        let cursor = prepared
            .query(&refs)
            .map_err(|error| oracle_error("Oracle 查询失败", error))?;
        let columns = if columns.is_empty() {
            cursor
                .columns()
                .iter()
                .map(|metadata| oracle_column(metadata, false))
                .collect::<Vec<_>>()
        } else {
            columns
        };
        let mut returned_rows = 0u64;
        let capacity = query_execution_row_capacity(pagination.limit);
        let mut rows = Vec::new();
        let mut has_more = false;
        for item in cursor {
            // 驱动的取行是阻塞调用，取消只能在两次取行之间生效；这里是唯一的安全点。
            if should_cancel() {
                return Ok(OracleStatementOutcome::Cancelled);
            }
            let mut row = item.map_err(|error| oracle_error("Oracle 读取结果失败", error))?;
            returned_rows += 1;
            if (returned_rows as usize) <= pagination.offset as usize {
                continue;
            }
            // 页满即止：继续把游标抽干只是为了让「共多少行」好看，代价是把整表扫完。
            if rows.len() >= capacity {
                has_more = true;
                break;
            }
            let mut row = oracle_row(&mut row, &columns);
            spill_large_cells(owner, &mut row.values)?;
            rows.push(row);
        }
        return Ok(OracleStatementOutcome::Rows(
            DataPage {
                columns,
                rows,
                offset: pagination.offset,
                limit: pagination.limit,
                has_more,
                incomplete: None,
            },
            returned_rows,
        ));
    }
    let is_ddl = prepared.is_ddl();
    let result = prepared
        .execute(&refs)
        .map_err(|error| oracle_error("Oracle 语句执行失败", error))?;
    Ok(OracleStatementOutcome::Affected(
        result.rows_affected(),
        is_ddl,
    ))
}

/// 语句切分。词法与块边界由 [`crate::script_syntax`] 的 Oracle 规则处理：
/// `q'[...]'` 走 `OracleDialect` 词法器，`BEGIN ... END;` 靠块深度计数，
/// 包体靠独占一行的 `/` 收尾。SQL*Plus 的 `/` 表示「执行缓冲区」，
/// 通过驱动发送时必须去掉，所以这里仍要剥掉行尾残留的 `/`。
fn oracle_statements_for_execution(request: &QueryRequest) -> Vec<String> {
    query_statements_for_execution(request, DatabaseKind::Oracle)
        .into_iter()
        .map(|statement| {
            // OCI 不接受语句开头的注释行（ORA-00900）；SQL*Plus 和编辑器都允许，
            // 所以在发送前剥掉前导注释，注释内容不参与执行语义。
            strip_leading_sql_comments(statement.trim().trim_end_matches('/').trim())
                .to_string()
        })
        .filter(|statement| !statement.is_empty())
        .collect()
}

/// 去掉语句最前面的空白、`--` 行注释与 `/* ... */` 块注释。
fn strip_leading_sql_comments(mut sql: &str) -> &str {
    loop {
        sql = sql.trim_start();
        if let Some(rest) = sql.strip_prefix("--") {
            sql = rest.split_once('\n').map_or("", |(_, rest)| rest);
        } else if let Some(rest) = sql.strip_prefix("/*") {
            sql = rest.split_once("*/").map_or("", |(_, rest)| rest);
        } else {
            return sql;
        }
    }
}

fn oracle_execute_parameterized(
    config: &ConnectionConfig,
    request: &QueryRequest,
    parameters: &[fluxdb_core::SqlParameter],
) -> fluxdb_core::Result<QueryExecutionResult> {
    let statement = request.text.trim().to_string();
    // 与批量入口一致：OCI 不接受前导注释，发送前剥掉，摘要里仍保留原文。
    let executable = strip_leading_sql_comments(&statement).to_string();
    if executable.is_empty() {
        return Err(Error::new(ErrorKind::Query, "查询不能为空"));
    }
    let binds = parameters
        .iter()
        .map(oracle_bind_parameter)
        .collect::<fluxdb_core::Result<Vec<_>>>()?;
    let connection = oracle_connect_with_schema(config, request.schema.as_deref())?;
    if request.options.read_only {
        oracle_begin_read_only(&connection)?;
    }
    let started = std::time::Instant::now();
    let pagination =
        query_execution_pagination(request.options.page_offset, request.options.page_size);
    // 参数化入口没有取消通道（AI 写操作路径走 `execute_bounded_update`），
    // 传恒 false 保持原行为；上限与字节预算同样生效。
    let owner = fluxdb_core::result_content_owner(request.session_id, request.connection_id);
    let outcome = oracle_execute_statement(
        &connection,
        &executable,
        &binds,
        pagination,
        request.options.read_only,
        &owner,
        &|| false,
    );
    let finish = oracle_finish_session(&connection, request.options.read_only);
    // 语句错误优先：它比收尾失败更能说明问题。
    let outcome = outcome?;
    finish?;
    match outcome {
        OracleStatementOutcome::Cancelled => Err(Error::new(
            ErrorKind::Cancelled,
            ORACLE_CANCELLED_OUTCOME_UNKNOWN,
        )),
        OracleStatementOutcome::Rows(page, returned) => Ok(QueryExecutionResult {
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
        }),
        OracleStatementOutcome::Affected(affected, is_ddl) => {
            // 只读入口到不了这里：非查询语句在执行前就被拒绝了。
            let (success, message) = if is_ddl {
                (true, "OK（DDL 隐式提交）".to_string())
            } else {
                (true, format!("OK，影响 {affected} 行"))
            };
            Ok(QueryExecutionResult {
                summaries: vec![QueryExecutionSummary {
                    sql: statement,
                    kind: QueryStatementKind::Command,
                    success,
                    message,
                    returned_rows: 0,
                    affected_rows: affected,
                    elapsed_ms: elapsed_ms(started),
                }],
                results: Vec::new(),
                rollback_snapshots: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::strip_leading_sql_comments;

    #[test]
    fn strips_leading_comments_before_oracle_execution() {
        assert_eq!(
            strip_leading_sql_comments("-- 中文注释\nSELECT 1 FROM dual;"),
            "SELECT 1 FROM dual;"
        );
        assert_eq!(
            strip_leading_sql_comments("/* block */ -- line\nSELECT 1 FROM dual"),
            "SELECT 1 FROM dual"
        );
        assert_eq!(strip_leading_sql_comments("  SELECT 1"), "SELECT 1");
        assert_eq!(strip_leading_sql_comments("-- only comment"), "");
    }
}
