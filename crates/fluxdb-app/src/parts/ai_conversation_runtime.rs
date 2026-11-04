impl AiRuntimeHandle {
    /// 提交已持久化的会话任务；配置、凭据与工具目录只在 actor 线程读取/装配。
    ///
    /// `relationships` 是 ER 关系能力来源（桌面传 `AppController` 的共享句柄）。
    /// 为 `None` 时关系工具不进入目录：模型既发现不到，也无法绕过发现直接调用。
    pub fn submit_conversation(
        &self,
        storage: fluxdb_storage::FileStorage,
        job: AiConversationJob,
        connections: Vec<fluxdb_core::ConnectionConfig>,
        relationships: Option<Arc<dyn ErRelationshipSource>>,
        ui_actions: Option<Arc<dyn UiActionSink>>,
        resume_approval: Option<String>,
        cancel: CancelFlag,
    ) -> Result<ConversationEventReceiver, RuntimeError> {
        self.submit_conversation_execution(
            storage,
            job,
            cancel,
            move |storage, job, turn, sender| {
                if turn.cancel.is_cancelled() {
                    cancel_ai_conversation(storage, job, sender);
                    return;
                }
                let assembled = (|| {
                    if let Err(error) = purge_expired_result_files(storage) {
                        tracing::warn!(%error, "临时结果清理失败，继续准备工具");
                    }
                    // T050：先清可判定的无引用文件（失败产物快照 + 未登记孤儿），再统计当前占用。
                    // 清理失败只告警，不阻断本轮任务；`ready` 产物（供报告读取）一律保留。
                    if let Err(error) = purge_failed_artifact_files(storage) {
                        tracing::warn!(%error, "失败产物快照清理失败，继续准备工具");
                    }
                    if let Err(error) = purge_orphan_controlled_files(storage) {
                        tracing::warn!(%error, "受控孤儿文件清理失败，继续准备工具");
                    }
                    match ai_storage_usage(storage) {
                        Ok(usage) => tracing::debug!(
                            artifact_files = usage.artifact_files,
                            artifact_bytes = usage.artifact_bytes,
                            result_files = usage.result_files,
                            result_bytes = usage.result_bytes,
                            "AI 受控存储用量"
                        ),
                        Err(error) => tracing::warn!(%error, "AI 受控存储用量统计失败"),
                    }
                    let provider =
                        AiConversationProvider::from_storage(storage, &job.model_profile_id)?;
                    let security = ai_turn_security_context(storage, &job.turn_id)?;
                    let turn = with_ai_storage(storage, |conn| {
                        fluxdb_storage::ai_platform::sessions::load_turn(conn, &job.turn_id)
                    })?;
                    let snapshot: serde_json::Value =
                        serde_json::from_str(&turn.runtime_snapshot_json)
                            .map_err(|_| editor_error("运行快照损坏"))?;
                    let sql_edit_only = snapshot["sql_edit_only"].as_bool() == Some(true);
                    // 页面查询是普通工具路径之一，不改变本轮对已保存连接的最小读取面。
                    let page_connections = connections
                        .iter()
                        .filter(|config| {
                            matches!(
                                config.kind,
                                DatabaseKind::Sqlite
                                    | DatabaseKind::MySql
                                    | DatabaseKind::TiDb
                                    | DatabaseKind::MariaDb
                                    | DatabaseKind::Postgres
                                    | DatabaseKind::Dameng
                                    | DatabaseKind::SqlServer
                            )
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let editor_grant = take_editor_grant(&job.turn_id);
                    let edit_grant = editor_grant
                        .as_ref()
                        .and_then(|grant| grant.sql_edit.clone());
                    if sql_edit_only && (edit_grant.is_none() || ui_actions.is_none()) {
                        return Err(editor_error("sql_edit_unavailable"));
                    }
                    let connection_id = security.connection_id;
                    let pinned = connection_id.and_then(|id| {
                        connections
                            .iter()
                            .find(|config| config.id.0 as i64 == id)
                            .cloned()
                    });
                    let include_write = pinned
                        .as_ref()
                        .is_some_and(|config| !connection_is_read_only(config));
                    let resolver = Arc::new(MapSqlResolver::new(pinned.iter().cloned()));
                    // 关系工具复用同一 resolver（连接授权与对象白名单口径一致）。
                    let relationship_resolver = Arc::clone(&resolver);
                    // 结果仓库按 Turn 绑定归属：ResultRef 只能被本 session/turn 读取；
                    // spill 写在存储根的受控子目录，重启后由恢复流程按保留策略处理。
                    let results = Arc::new(Mutex::new(ResultStore::for_turn(
                        RESULT_STORE_MEMORY_BYTES,
                        &job.session_id,
                        &job.turn_id,
                        Some(storage.root_dir().join("ai-results")),
                    )));
                    let has_sql_dialect = pinned
                        .as_ref()
                        .is_some_and(|config| policy_for(config.kind).is_some());
                    let mut tools = if pinned
                        .as_ref()
                        .is_some_and(|config| config.kind == fluxdb_core::DatabaseKind::Redis)
                    {
                        standard_redis_catalog(Arc::clone(&resolver), include_write)?
                    } else if pinned.is_some() && !has_sql_dialect {
                        // 没有 SQL 策略的连接类型（Redis 之外的文档型/键值型）不装配 db.*，
                        // 避免暴露后每次调用都以 unsupported_database 失败。
                        ToolCatalog::new(Vec::new())
                    } else {
                        standard_sql_catalog_with_results(
                            Arc::clone(&resolver),
                            std::sync::Arc::clone(&results),
                            include_write,
                        )?
                    };
                    // 关系与路径工具（只读）与建议提交（写本地目录）：仅关系型连接可用，
                    // 且只在宿主已接入 ER 模型存储时进入目录。
                    if let Some(relationships) = relationships
                        && !pinned
                            .as_ref()
                            .is_some_and(|config| config.kind == fluxdb_core::DatabaseKind::Redis)
                    {
                        add_er_relationship_read_tools(
                            &mut tools,
                            std::sync::Arc::clone(&relationship_resolver),
                            std::sync::Arc::clone(&relationships),
                        )?;
                        add_er_relationship_propose_tool(
                            &mut tools,
                            std::sync::Arc::clone(&relationship_resolver),
                            relationships,
                        )?;
                    }
                    tools.add_tool(Box::new(AiConnectionsListTool::new(
                        &connections,
                        pinned.as_ref().map(|config| config.id),
                    )))?;
                    if let Some(actions) = ui_actions.as_ref() {
                        add_ai_table_page_tools(
                            &mut tools,
                            &page_connections,
                            Arc::clone(actions),
                        )?;
                    }
                    // 全局停用的工具不进目录：模型既发现不到，也无法绕过发现直接调用。
                    tools = tools.with_hooks(ai_builtin_hooks(storage, job)?);
                    add_ai_file_tools(&mut tools, storage, job)?;
                    add_ai_reference_tools(&mut tools, storage, job)?;
                    // 内置产品文档只读、无需授权：始终进入目录，避免模型凭记忆编操作路径。
                    add_ai_docs_tools(&mut tools)?;
                    // 通用产物工具始终装配；来源兼容性由实际产物能力校验。
                    add_ai_artifact_tools(
                        &mut tools,
                        storage,
                        job,
                        std::sync::Arc::clone(&results),
                    )?;
                    // 编辑器全文由宿主在发送时登记，取走即释放：不写运行快照，
                    // 本轮结束后工具目录随之消失（设计 §13.2 不持久化整份内容）。
                    add_ai_editor_read_tools(&mut tools, editor_grant)?;
                    // 显式修改/修复与“发送时自动绑定选区”都可写回；但后者不限制
                    // 本轮其它只读查询，用户只是在抽屉里发起一次修改。
                    if let (Some(grant), Some(actions)) = (edit_grant, ui_actions.as_ref())
                    {
                        add_ai_sql_editor_edit_tool(&mut tools, grant, Arc::clone(actions))?;
                    }
                    add_ai_page_tools(&mut tools, take_page_data_grant(&job.turn_id), job)?;
                    // MongoDB 文档值同理：发送时登记、装配目录时取走，不落盘、不预填进首条消息。
                    add_ai_mongo_documents_tool(
                        &mut tools,
                        take_mongo_documents_grant(&job.turn_id),
                        job,
                    )?;
                    // 配置读取/提案是全局能力，不依赖当前页面。
                    if let Some(actions) = ui_actions.as_ref() {
                        add_ai_config_tools(&mut tools, storage.clone(), Arc::clone(actions))?;
                    }
                    // 「计划与执行进度」插件：仅在实例开启且当前 Agent 未禁用绑定时
                    // 暴露 `plan.update`；关闭后工具不进入目录，模型发现不到。
                    add_ai_plan_tool(&mut tools, storage, &job.session_id)?;
                    // 诊断读取：解释"刚才为什么失败"。不受遥测插件开关门控，
                    // 仍受全局工具停用与 Agent 白名单收窄（见 ai_diagnostics.rs 文件头）。
                    add_ai_diagnostics_tool(&mut tools, storage, job)?;
                    if tools.results().is_none() {
                        let spec = result_read_page_spec()?;
                        tools = tools.with_results(std::sync::Arc::clone(&results));
                        tools.add_tool_with_shape(
                            Box::new(AiResultReadPageTool::new(std::sync::Arc::clone(&results))?),
                            spec.output_shape,
                        )?;
                    }
                    // 必须放在所有 SQL/关系工具装配之后：这些工具只能对绑定连接执行，
                    // 没有执行目标时暴露给模型只会诱导它反复猜 conn:<id>（dev 会话 5122b1cd）。
                    // 放到目录冻结之前，确保 toolbox 也看不到它们。
                    if pinned.is_none() {
                        without_unbound_sql_tools(&mut tools);
                    }
                    tools.without_tools(&security.disabled_tools);
                    mark_builtin_long_tail_tools(&mut tools)?;
                    // 审批模型与本轮 Agent 用的是同一个模型：Jev 决策不可用时由它回退审批。
                    tools = tools.with_execution_security(ExecutionSecurity::with_connections(
                        storage.clone(),
                        connections.clone(),
                        job.model_profile_id.clone(),
                    ));
                    let read_only = tools.security.as_ref().is_some_and(|security| {
                        security
                            .settings()
                            .map(|(policy, _)| policy.tools_default == ExecutionPolicy::ReadOnly)
                            .unwrap_or(true)
                    });
                    install_ai_toolbox(
                        &mut tools,
                        &security.disabled_tools,
                        &security.agent_allowed_tools,
                        read_only,
                        Some(Arc::new(StorageToolboxSelector {
                            storage: storage.clone(),
                            agent_id: turn_agent_id(storage, &job.session_id),
                            job: job.clone(),
                        })),
                    )?;
                    if sql_edit_only {
                        restrict_sql_editor_edit_tools(&mut tools);
                    }
                    let permission = TurnPermissionContext {
                        agent_allowed_tools: security.agent_allowed_tools,
                        disabled_tools: security.disabled_tools,
                        policy: kernel_policy_for_connection(pinned.as_ref()),
                        plugin_digest: None,
                    };
                    Ok::<_, fluxdb_core::Error>((provider, tools, permission))
                })();
                // 规则集在每轮开始时读一次：规则变更作用于下一个调用边界。
                // 读不出来就 fail closed（审计不可用 = 本轮回绝所有工具），不按"没有规则"放行。
                let assembled = assembled.and_then(|(provider, tools, permission)| {
                    let audit = ConfiguredAuditHook::from_storage(storage)?;
                    Ok((provider, tools, permission, audit))
                });
                match assembled {
                    Ok((provider, tools, permission, audit)) => match resume_approval.as_deref() {
                        Some(approval_id) => resume_ai_approved_turn(
                            storage,
                            job,
                            &turn.cancel,
                            sender,
                            &provider,
                            &tools,
                            &audit,
                            permission,
                            approval_id,
                            Some(turn),
                        ),
                        None => ai_conversation_run_turn(
                            storage,
                            job,
                            &turn.cancel,
                            sender,
                            &provider,
                            None,
                            &tools,
                            &audit,
                            permission,
                            Vec::new(),
                            String::new(),
                            Some(turn),
                        ),
                    },
                    Err(error) => run_ai_conversation_failure(
                        storage,
                        job,
                        sender,
                        FailureStage::Prepare,
                        error,
                    ),
                }
            },
        )
    }

    /// 注入模型事件来源；仍由同一 actor 调度并经过正式 checkpoint 与终态保存。
    pub fn submit_conversation_source(
        &self,
        storage: fluxdb_storage::FileStorage,
        job: AiConversationJob,
        cancel: CancelFlag,
        source: impl FnOnce(
            &CancelFlag,
            &mut dyn FnMut(&fluxdb_core::ModelEvent),
        ) -> fluxdb_core::Result<Vec<fluxdb_core::ModelEvent>>
        + Send
        + 'static,
    ) -> Result<ConversationEventReceiver, RuntimeError> {
        self.submit_conversation_execution(
            storage,
            job,
            cancel,
            move |storage, job, turn, sender| {
                run_ai_conversation_source(storage, job, &turn.cancel, sender, |emit| {
                    if turn.cancel.is_cancelled() {
                        emit(&fluxdb_core::ModelEvent::Finished(FinishReason::Cancelled));
                        Ok(Vec::new())
                    } else {
                        source(&turn.cancel, emit)
                    }
                });
            },
        )
    }

    fn submit_conversation_execution(
        &self,
        storage: fluxdb_storage::FileStorage,
        job: AiConversationJob,
        cancel: CancelFlag,
        execute: impl FnOnce(
            &fluxdb_storage::FileStorage,
            &AiConversationJob,
            &RuntimeTurn,
            &dyn ConversationEventSink,
        ) + Send
        + 'static,
    ) -> Result<ConversationEventReceiver, RuntimeError> {
        let (sender, receiver) = conversation_event_channel();
        self.submit(
            job.turn_id.clone(),
            cancel,
            Box::new(move |turn| {
                // 异常也必须交付终态；不把 panic 载荷拼入公开错误或附加日志。
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    execute(&storage, &job, turn, &sender)
                }));
                clear_current_page_grants(&job.turn_id);
                if result.is_err() {
                    tracing::warn!(turn_id=%job.turn_id, "AI 任务异常，正在收尾");
                    run_ai_conversation_failure(
                        &storage,
                        &job,
                        &sender,
                        FailureStage::Internal,
                        fluxdb_core::Error::new(
                            fluxdb_core::ErrorKind::Internal,
                            "AI 任务异常结束，请核对工具执行状态",
                        ),
                    );
                }
            }),
        )?;
        Ok(receiver)
    }
}
