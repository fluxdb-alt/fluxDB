/// 复用既有 scope 执行链：单条语句、隔离会话、有界页、可取消。
///
/// `read_only` 为 true 时先过 §21 分类器；分类不是 `ReadOnly` 直接拒绝。
pub fn execute_sql(
    resolver: &dyn SqlExecutionResolver,
    target: &AiSqlTarget,
    sql: &str,
    max_rows: u64,
    read_only: bool,
    should_cancel: &dyn Fn() -> bool,
) -> Result<SqlExecutionOutcome, SqlExecError> {
    execute_sql_with_parameters(
        resolver,
        target,
        sql,
        max_rows,
        &[],
        read_only,
        should_cancel,
    )
}

/// 带精确参数（驱动真实 bind）的受控执行；无参数时等价于 [`execute_sql`]。
pub fn execute_sql_with_parameters(
    resolver: &dyn SqlExecutionResolver,
    target: &AiSqlTarget,
    sql: &str,
    max_rows: u64,
    parameters: &[fluxdb_core::SqlParameter],
    read_only: bool,
    should_cancel: &dyn Fn() -> bool,
) -> Result<SqlExecutionOutcome, SqlExecError> {
    execute_sql_with_parameters_and_progress(
        resolver,
        target,
        SqlExecInput {
            sql,
            max_rows,
            parameters,
        },
        should_cancel,
        &mut |_| {},
        SqlExecRunOptions {
            read_only,
            bounded_write: false,
            explain_only: false,
        },
    )
}

/// 连接器错误 → 工具错误码。
///
/// `OutcomeUnknown` 必须单独成码：它是「提交/回滚未确认」，回执要标成
/// `Unknown` 让 Agent 循环停下来人工核实；笼统归成 `execution_error` 会让
/// 模型把它当普通失败再写一次。
fn sql_exec_error_from_connector(
    error: fluxdb_core::Error,
    prefix: &str,
    unsupported_code: &'static str,
) -> SqlExecError {
    let code = match error.kind {
        fluxdb_core::ErrorKind::OutcomeUnknown => "write_outcome_unknown",
        fluxdb_core::ErrorKind::Unsupported => unsupported_code,
        _ => "execution_error",
    };
    let message = if code == "write_outcome_unknown" {
        "提交或回滚结果未知，禁止自动重试，请人工核实".to_string()
    } else {
        format!("{prefix}: {error}")
    };
    SqlExecError::new(code, message)
}

/// 执行链的可选模式。集中成一个结构，避免继续增加位置参数。
#[derive(Clone, Copy, Debug, Default)]
struct SqlExecRunOptions {
    /// 只读入口：分类器先拦，连接器再用只读事务兜底。
    read_only: bool,
    /// 有界提交（仅具备该能力的连接器）。
    bounded_write: bool,
    /// 请求执行计划；仅 SQL Server 走 SHOWPLAN，其它方言在包装阶段已改写 SQL。
    explain_only: bool,
}

/// 一次执行请求的 SQL、行数上限与参数。
#[derive(Clone, Copy)]
struct SqlExecInput<'a> {
    sql: &'a str,
    max_rows: u64,
    parameters: &'a [fluxdb_core::SqlParameter],
}

fn execute_sql_with_parameters_and_progress(
    resolver: &dyn SqlExecutionResolver,
    target: &AiSqlTarget,
    input: SqlExecInput<'_>,
    should_cancel: &dyn Fn() -> bool,
    on_progress: &mut dyn FnMut(fluxdb_core::ToolProgressEvent),
    options: SqlExecRunOptions,
) -> Result<SqlExecutionOutcome, SqlExecError> {
    let SqlExecInput {
        sql,
        max_rows,
        parameters,
    } = input;
    let config = resolver
        .connection_config(target.connection_id)
        .ok_or_else(|| {
            SqlExecError::new(
                "connection_not_found",
                format!("连接不存在: {}", target.connection_id.0),
            )
        })?;
    let policy = policy_for(config.kind)
        .ok_or_else(|| SqlExecError::new("unsupported_database", "该连接类型不支持 SQL 工具"))?;

    let read_only = options.read_only;
    // 只有声明 Showplan 的策略才走专用会话；其它方言在 apply_explain_mode 里已包装。
    let explain_only = options.explain_only && policy.explain_mode() == ExplainMode::Showplan;
    let risk = analyze_sql(sql, policy);
    // 结构性拒绝（多语句、事务控制）优先给出更具体的原因。
    if let SqlRisk::Rejected { reasons } = &risk {
        return Err(SqlExecError::new(
            "rejected_sql",
            format!("语句被策略拒绝（{}）", reasons.join(",")),
        ));
    }
    if read_only && !risk.is_read_only() {
        return Err(SqlExecError::new(
            "not_read_only",
            format!("只读入口拒绝该语句（{}）", risk.reasons().join(",")),
        ));
    }
    validate_sql_scope(sql, policy, target)?;

    let connector = fluxdb_connectors::connector_for(&config).map_err(|error| {
        SqlExecError::new("connector_error", format!("无法创建连接器: {error}"))
    })?;

    // 取 max_rows+1：多取一行才能区分「结果正好这么多」与「被上限截断」，
    // 否则主动追加的 LIMIT 会让 truncated 永远为 false，模型无法知道还有更多行（设计 §21.3）。
    let fetch_rows = max_rows.saturating_add(1);
    // SQL Server 的 SHOWPLAN 会话里不能再套 TOP，否则返回的是改写后的语句计划。
    let bounded = if explain_only && config.kind == DatabaseKind::SqlServer {
        strip_sql_comments(sql, config.kind)
    } else {
        sql_text_for_execution(sql, fetch_rows, config.kind)
    };
    let request = fluxdb_core::QueryRequest {
        connection_id: target.connection_id,
        database: target.database.clone(),
        schema: target.schema.clone(),
        text: bounded.clone(),
        mode: fluxdb_core::QueryMode::Selection,
        options: fluxdb_core::QueryExecutionOptions {
            // AI 路径不做多语句 split，也不继续执行后续语句。
            continue_on_error: false,
            split_statements: false,
            page_offset: 0,
            page_size: fetch_rows,
            // 只读执行器要求数据库级只读；统一入口中的写分支传 false。
            read_only: read_only || explain_only,
            explain_only,
        },
        // 每个 resolver 对应一个 Agent Turn 的专属会话，不借用编辑器 Tab。
        session_id: Some(resolver.query_session_id()),
    };
    // 本次调用结束（含提前返回）即释放该会话，避免状态残留与会话数累积。
    let _session_guard = AiSqlSessionGuard {
        connection_id: request.connection_id,
        session_id: resolver.query_session_id(),
    };

    let started = Instant::now();
    let mut first_error: Option<String> = None;
    let mut progress = Vec::new();
    // 走有界提交只在连接器确实具备该能力时发生；自动放行的普通写入按原语句执行。
    let execution = if options.bounded_write {
        connector
            .execute_bounded_update(
                &fluxdb_core::ParameterizedQueryRequest {
                    request: request.clone(),
                    parameters: parameters.to_vec(),
                },
                should_cancel,
            )
            .map_err(|error| {
                if error.kind == fluxdb_core::ErrorKind::OutcomeUnknown {
                    SqlExecError::new(
                        "write_outcome_unknown",
                        "提交或回滚结果未知，禁止自动重试，请人工核实",
                    )
                } else {
                    SqlExecError::new(
                        "bounded_write_rejected",
                        "自动审批影响验证失败或已回滚，需要重新人工审核",
                    )
                }
            })?
    } else if parameters.is_empty() {
        connector
            .execute_with_progress(
                &request,
                &mut |summary| {
                    let progress_event = fluxdb_core::ToolProgressEvent {
                        returned_rows: summary.returned_rows,
                        affected_rows: summary.affected_rows,
                        elapsed_ms: summary.elapsed_ms,
                        success: summary.success,
                    };
                    on_progress(progress_event.clone());
                    progress.push(serde_json::json!({
                        "sql": summary.sql,
                        "success": summary.success,
                        "returned_rows": summary.returned_rows,
                        "affected_rows": summary.affected_rows,
                        "elapsed_ms": summary.elapsed_ms,
                    }));
                    if !summary.success && first_error.is_none() {
                        first_error = Some(summary.message.clone());
                    }
                },
                should_cancel,
            )
            .map_err(|error| sql_exec_error_from_connector(error, "执行失败", "execution_error"))?
    } else {
        // 有参数就必须真 bind；不支持绑定的连接器由默认实现明确拒绝（绝不插值）。
        connector
            .execute_parameterized(&fluxdb_core::ParameterizedQueryRequest {
                request: request.clone(),
                parameters: parameters.to_vec(),
            })
            .map_err(|error| sql_exec_error_from_connector(error, "参数化执行失败", "parameters_unsupported"))?
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if should_cancel() {
        // 读操作取消 = 没有副作用，直接报告已取消；
        // 写操作在"取消信号晚到"的竞态下无法确定是否已提交，必须报告 unknown，
        // 交给 §T029 的恢复流程核实，既不能当成功，也不能当已取消。
        return Err(if read_only {
            SqlExecError::new("cancelled", "执行已取消")
        } else {
            SqlExecError::new(
                "cancel_race_unknown",
                "取消信号在写操作返回后才到达，是否已提交无法确定",
            )
        });
    }
    if let Some(message) = first_error {
        return Err(SqlExecError::new("execution_failed", message));
    }

    if !parameters.is_empty() {
        for summary in &execution.summaries {
            on_progress(fluxdb_core::ToolProgressEvent {
                returned_rows: summary.returned_rows,
                affected_rows: summary.affected_rows,
                elapsed_ms: summary.elapsed_ms,
                success: summary.success,
            });
        }
    }
    Ok(project_execution(
        &execution,
        max_rows,
        elapsed_ms,
        resolver.query_session_id(),
        sql,
        &bounded,
        progress,
    ))
}

/// 校验 SQL 内显式 database/schema 限定名不越过 target 作用域。
///
/// 裸名由请求携带的默认 database/schema 解析；显式同名限定允许，显式越界 fail closed。
fn validate_sql_scope(
    sql: &str,
    policy: &dyn SqlPolicy,
    target: &AiSqlTarget,
) -> Result<(), SqlExecError> {
    use sqlparser::ast::{ObjectName, ObjectNamePart, visit_expressions, visit_relations};
    use std::ops::ControlFlow;

    let statements = sqlparser::parser::Parser::parse_sql(policy.parser_dialect(), sql)
        .map_err(|_| SqlExecError::new("unknown_syntax", "SQL 无法解析，无法校验作用域"))?;
    if statements.len() != 1 {
        return Err(SqlExecError::new(
            "rejected_sql",
            format!("多语句无法校验作用域（{} 条）", statements.len()),
        ));
    }
    let statement = &statements[0];
    let allowed_namespace = match policy.namespace_scope() {
        NamespaceScope::Database => target.database.as_deref(),
        NamespaceScope::AttachedDatabase => target.database.as_deref().or(Some("main")),
        NamespaceScope::Schema | NamespaceScope::SchemaAndDatabase => target.schema.as_deref(),
    };
    let namespace_kind = policy.namespace_scope().label();
    let out_of_scope = |name: &ObjectName, kind: &str| -> ControlFlow<()> {
        // 裸名按请求默认 namespace 解析；从两段限定名开始检查倒数第二段。
        if name.0.len() < 2 {
            return ControlFlow::Continue(());
        }
        let qualifier = name.0.get(name.0.len() - 2);
        let Some(Some(qualifier)) = qualifier.map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
            ObjectNamePart::Function(_) => None,
        }) else {
            return ControlFlow::Break(());
        };
        let namespace_ok =
            allowed_namespace.is_some_and(|allowed| qualifier.eq_ignore_ascii_case(allowed));
        // SQL Server 的三段名 `[db].[schema].[table]`：database 也必须等于 target，
        // 否则会跨库读取/写入。两段名没有 database 段，按请求默认库解析。
        let database_ok = if policy.namespace_scope() == NamespaceScope::SchemaAndDatabase
            && name.0.len() >= 3
        {
            let database = name.0.get(name.0.len() - 3).and_then(|part| match part {
                ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
                ObjectNamePart::Function(_) => None,
            });
            database
                .zip(target.database.as_deref())
                .is_some_and(|(database, allowed)| database.eq_ignore_ascii_case(allowed))
        } else {
            true
        };
        if namespace_ok && database_ok {
            ControlFlow::Continue(())
        } else {
            tracing::warn!(
                target: "fluxdb_app",
                kind,
                namespace_kind,
                "AI SQL 显式对象限定名超出 target 作用域"
            );
            ControlFlow::Break(())
        }
    };
    if visit_relations(statement, |name| out_of_scope(name, "relation")).is_break() {
        return Err(SqlExecError::new(
            "scope_mismatch",
            "SQL 引用的 database/schema 超出当前 target 作用域；去掉 SQL 里的库/schema 前缀，或把 target 改写为 conn:<id>/db:<database>[/schema:<schema>]",
        ));
    }
    if visit_expressions(statement, |expr| {
        if let sqlparser::ast::Expr::Function(function) = expr {
            out_of_scope(&function.name, "function")
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
    {
        return Err(SqlExecError::new(
            "scope_mismatch",
            "SQL 调用的函数所在 database/schema 超出当前 target 作用域；去掉函数名前的库/schema 前缀，或把 target 改写为 conn:<id>/db:<database>[/schema:<schema>]",
        ));
    }
    Ok(())
}

/// 把连接器结果投影成有界 JSON：行数/字节双上限，超限只标记 truncated。
fn project_execution(
    execution: &fluxdb_core::QueryExecutionResult,
    max_rows: u64,
    elapsed_ms: u64,
    query_session_id: fluxdb_core::QuerySessionId,
    original_sql: &str,
    executed_sql: &str,
    mut progress: Vec<serde_json::Value>,
) -> SqlExecutionOutcome {
    let mut columns = Vec::new();
    let mut rows: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut truncated = false;
    let mut bytes = 0usize;
    let mut row_count = 0u64;
    let mut statement_kind = "command";
    let mut affected_rows = None;

    if let Some(summary) = execution.summaries.last() {
        statement_kind = match summary.kind {
            fluxdb_core::QueryStatementKind::ResultSet => "result_set",
            fluxdb_core::QueryStatementKind::Command => "command",
        };
        if summary.kind == fluxdb_core::QueryStatementKind::Command {
            affected_rows = Some(summary.affected_rows as i64);
        }
    }
    if progress.is_empty() {
        progress.extend(execution.summaries.iter().map(|summary| {
            serde_json::json!({
                "sql": summary.sql,
                "success": summary.success,
                "returned_rows": summary.returned_rows,
                "affected_rows": summary.affected_rows,
                "elapsed_ms": summary.elapsed_ms,
            })
        }));
    }

    for page in &execution.results {
        if columns.is_empty() {
            columns = page.columns.iter().map(|c| c.name.clone()).collect();
        }
        if page.has_more {
            truncated = true;
        }
        for row in &page.rows {
            if row_count >= max_rows {
                truncated = true;
                break;
            }
            let projected: Vec<serde_json::Value> = row.values.iter().map(cell_to_json).collect();
            let row_bytes = serde_json::to_string(&projected)
                .map(|s| s.len())
                .unwrap_or(0);
            if bytes + row_bytes > SQL_RESULT_MAX_BYTES {
                truncated = true;
                break;
            }
            bytes += row_bytes;
            rows.push(projected);
            row_count += 1;
        }
        if truncated {
            break;
        }
    }

    SqlExecutionOutcome {
        columns,
        rows,
        row_count,
        truncated,
        elapsed_ms,
        query_session_id,
        source: fluxdb_core::CommandExecutionSource::Ai,
        original_sql: original_sql.to_string(),
        executed_sql: executed_sql.to_string(),
        progress,
        affected_rows,
        statement_kind,
    }
}

/// 组装一个真实回执（时间戳 + 载荷/错误码）。
fn receipt_from(
    descriptor: &fluxdb_core::ToolDescriptor,
    call_id: &str,
    started_at_ms: i64,
    outcome: Result<SqlExecutionOutcome, SqlExecError>,
) -> fluxdb_core::ToolReceipt {
    let finished_at_ms = now_ms();
    match outcome {
        Ok(outcome) => fluxdb_core::ToolReceipt {
            call_id: call_id.to_string(),
            tool_id: descriptor.id.clone(),
            status: fluxdb_core::ToolReceiptStatus::Succeeded,
            result_json: outcome.to_json().to_string(),
            error_code: None,
            affected_rows: outcome.affected_rows,
            started_at_ms,
            finished_at_ms,
        },
        Err(error) => fluxdb_core::ToolReceipt {
            call_id: call_id.to_string(),
            tool_id: descriptor.id.clone(),
            status: match error.code {
                "cancelled" => fluxdb_core::ToolReceiptStatus::Cancelled,
                "cancel_race_unknown" | "write_outcome_unknown" => {
                    fluxdb_core::ToolReceiptStatus::Unknown
                }
                _ => fluxdb_core::ToolReceiptStatus::Failed,
            },
            result_json: serde_json::json!({
                "ok": false,
                "code": error.code,
                "message": error.message,
                "retryable": error.retryable,
            })
            .to_string(),
            error_code: Some(error.code.to_string()),
            affected_rows: None,
            started_at_ms,
            finished_at_ms,
        },
    }
}

/// 带 ResultRef 的回执：成功时把句柄装进载荷（T027 分页接线点）。
fn receipt_from_with_ref(
    descriptor: &fluxdb_core::ToolDescriptor,
    call_id: &str,
    started_at_ms: i64,
    outcome: Result<SqlExecutionOutcome, SqlExecError>,
    result_ref: Option<&crate::ResultRef>,
) -> fluxdb_core::ToolReceipt {
    let finished_at_ms = now_ms();
    match outcome {
        Ok(outcome) => fluxdb_core::ToolReceipt {
            call_id: call_id.to_string(),
            tool_id: descriptor.id.clone(),
            status: fluxdb_core::ToolReceiptStatus::Succeeded,
            // 成功回执只带样本行 + ResultRef；完整行留在结果仓库里按需分页读。
            result_json: outcome.to_model_json(result_ref).to_string(),
            error_code: None,
            affected_rows: outcome.affected_rows,
            started_at_ms,
            finished_at_ms,
        },
        Err(error) => receipt_from(descriptor, call_id, started_at_ms, Err(error)),
    }
}

/// 无库上下文时的回退：返回可用库名，供模型拼出完整的 target 再检索。
///
/// 只读、有界；`list_objects(None)` 的契约就是列库，不额外过滤名称。
fn list_databases(
    connector: &dyn Connector,
    target: &AiSqlTarget,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> Result<serde_json::Value, SqlExecError> {
    if should_cancel() {
        return Err(SqlExecError::new("cancelled", "检索已取消"));
    }
    let databases = connector
        .list_objects(None)
        .map_err(|error| SqlExecError::new("metadata_error", format!("库列表获取失败: {error}")))?;
    if should_cancel() {
        return Err(SqlExecError::new("cancelled", "检索已取消"));
    }
    let items: Vec<serde_json::Value> = databases
        .iter()
        .filter(|object| matches!(object.path.kind, ObjectKind::Database))
        .take(limit as usize)
        .map(|object| serde_json::json!({ "name": object.path.name, "kind": "database" }))
        .collect();
    Ok(serde_json::json!({
        "objects": items,
        "count": items.len(),
        "next_step": format!(
            "以上是该连接可见的数据库；检索表请把 target 写成 conn:{}/db:<database>",
            target.connection_id.0
        ),
    }))
}

/// 元数据探索：按名字模糊搜索表/视图（只读、有界、可取消）。
///
/// 复用连接器补全索引（各方言自己实现），不 dump 全库 DDL。
pub fn search_objects(
    resolver: &dyn SqlExecutionResolver,
    target: &AiSqlTarget,
    query: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> Result<serde_json::Value, SqlExecError> {
    let config = resolver
        .connection_config(target.connection_id)
        .ok_or_else(|| {
            SqlExecError::new(
                "connection_not_found",
                format!("连接不存在: {}", target.connection_id.0),
            )
        })?;
    let bounded = limit.clamp(1, 100);
    let connector = fluxdb_connectors::connector_for(&config).map_err(|error| {
        SqlExecError::new("connector_error", format!("无法创建连接器: {error}"))
    })?;
    // MySQL 家族 / SQL Server / MongoDB 的检索必须先有库上下文，连接又没配默认库时
    // 直接返回库清单，让模型先选库，而不是拿到「缺少数据库上下文」这种无法行动的报错。
    if target.database.is_none()
        && default_database(&config).is_none()
        && (config.kind.is_mysql_family()
            || matches!(config.kind, DatabaseKind::SqlServer | DatabaseKind::MongoDb))
    {
        return list_databases(&*connector, target, bounded, should_cancel);
    }
    let tables = connector
        .list_completion_tables_with_cancel(
            target.database.as_deref(),
            target.schema.as_deref(),
            query,
            bounded,
            should_cancel,
        )
        .map_err(|error| SqlExecError::new("metadata_error", format!("元数据检索失败: {error}")))?;
    if should_cancel() {
        return Err(SqlExecError::new("cancelled", "检索已取消"));
    }
    let items: Vec<serde_json::Value> = tables
        .iter()
        .filter(|table| resolver.sql_object_allowed(target, &table.name))
        .map(|table| {
            serde_json::json!({
                "name": table.name,
                "database": table.database,
                "schema": table.schema,
                "kind": format!("{:?}", table.kind).to_lowercase(),
                "comment": table.comment,
            })
        })
        .collect();
    Ok(serde_json::json!({ "objects": items, "count": items.len() }))
}

/// 元数据探索：描述一个已定位的表或视图。
///
/// Connector 负责方言查询；这里仅负责 scope 校验、取消检查和有界 JSON 投影。
pub fn describe_object(
    resolver: &dyn SqlExecutionResolver,
    target: &AiSqlTarget,
    object_ref: &serde_json::Value,
    include: &[String],
    should_cancel: &dyn Fn() -> bool,
) -> Result<serde_json::Value, SqlExecError> {
    let config = resolver
        .connection_config(target.connection_id)
        .ok_or_else(|| {
            SqlExecError::new(
                "connection_not_found",
                format!("连接不存在: {}", target.connection_id.0),
            )
        })?;
    let object = AiSqlObjectRef::parse(object_ref, target)?;
    if !resolver.sql_object_allowed(target, &object.name) {
        return Err(SqlExecError::new(
            "object_not_allowed",
            "对象不在授权范围内",
        ));
    }
    if should_cancel() {
        return Err(SqlExecError::new("cancelled", "元数据描述已取消"));
    }
    let connector = fluxdb_connectors::connector_for(&config).map_err(|error| {
        SqlExecError::new("connector_error", format!("无法创建连接器: {error}"))
    })?;
    let path = object.to_path(target.connection_id);
    let wanted = |name: &str| include.is_empty() || include.iter().any(|item| item == name);

    let mut result = serde_json::Map::new();
    result.insert("object".into(), object.to_json());
    let tables = connector
        .list_completion_tables_with_cancel(
            object.database.as_deref(),
            object.schema.as_deref(),
            &object.name,
            100,
            should_cancel,
        )
        .map_err(|error| SqlExecError::new("metadata_error", format!("对象检索失败: {error}")))?;
    if wanted("comment") {
        result.insert(
            "comment".into(),
            tables
                .iter()
                .find(|table| table.name == object.name && table.kind == object.kind)
                .and_then(|table| table.comment.clone())
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
    }
    if wanted("columns") {
        let columns = connector
            .list_completion_columns_with_cancel(
                object.database.as_deref(),
                object.schema.as_deref(),
                &object.name,
                should_cancel,
            )
            .map_err(|error| {
                SqlExecError::new("metadata_error", format!("列元数据读取失败: {error}"))
            })?;
        result.insert(
            "columns".into(),
            serde_json::Value::Array(
                columns
                    .into_iter()
                    .map(|column| {
                        serde_json::json!({
                            "name": column.name,
                            "type": column.type_name,
                            "nullable": column.nullable,
                            "primary_key": column.primary_key,
                            "comment": column.comment,
                            "stable": column.stable,
                        })
                    })
                    .collect(),
            ),
        );
    }
    if wanted("indexes") {
        let indexes = if object.kind == fluxdb_core::ObjectKind::View {
            Vec::new()
        } else {
            connector.list_indexes(&path).map_err(|error| {
                SqlExecError::new("metadata_error", format!("索引读取失败: {error}"))
            })?
        };
        result.insert(
            "indexes".into(),
            serde_json::Value::Array(
                indexes
                    .into_iter()
                    .map(|index| {
                        serde_json::json!({
                            "name": index.name,
                            "columns": index.columns,
                            "unique": index.is_unique,
                            "primary": index.is_primary,
                            "type": index.index_type,
                            "comment": index.comment,
                        })
                    })
                    .collect(),
            ),
        );
    }
    if wanted("foreign_keys") {
        let foreign_keys = if object.kind == fluxdb_core::ObjectKind::View {
            Vec::new()
        } else {
            connector
                .list_foreign_keys_with_cancel(&path, should_cancel)
                .map_err(|error| {
                    SqlExecError::new("metadata_error", format!("外键读取失败: {error}"))
                })?
        };
        result.insert(
            "foreign_keys".into(),
            serde_json::Value::Array(
                foreign_keys
                    .into_iter()
                    .map(|foreign_key| {
                        serde_json::json!({
                            "name": foreign_key.name,
                            "columns": if foreign_key.columns.is_empty() {
                                vec![foreign_key.column]
                            } else {
                                foreign_key.columns
                            },
                            "ref_schema": foreign_key.ref_schema,
                            "ref_table": foreign_key.ref_table,
                            "ref_columns": if foreign_key.ref_columns.is_empty() {
                                vec![foreign_key.ref_column]
                            } else {
                                foreign_key.ref_columns
                            },
                        })
                    })
                    .collect(),
            ),
        );
    }
    if wanted("ddl") {
        let ddl = connector.table_ddl(&path).map_err(|error| {
            SqlExecError::new("metadata_error", format!("DDL 读取失败: {error}"))
        })?;
        if ddl.len() > SQL_RESULT_MAX_BYTES {
            return Err(SqlExecError::new(
                "metadata_too_large",
                "DDL 超过结果大小上限",
            ));
        }
        result.insert("ddl".into(), serde_json::Value::String(ddl));
    }
    if should_cancel() {
        return Err(SqlExecError::new("cancelled", "元数据描述已取消"));
    }
    Ok(serde_json::Value::Object(result))
}

#[cfg(test)]
mod sqlserver_ai_exec_tests {
    use crate::DatabaseKind;

    /// AI 执行链的纯策略测试：不建连，只验证方言映射与分类结果。
    #[test]
    fn sqlserver_has_ai_dialect_and_classifies_select() {
        let policy = crate::policy_for(DatabaseKind::SqlServer).expect("policy");
        assert!(crate::analyze_sql(
            "SELECT TOP (5) * FROM [dbo].[t]",
            policy
        )
        .is_read_only());
    }

}
