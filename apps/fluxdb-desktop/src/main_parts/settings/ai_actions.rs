// AI 设置职责拆分；由 ai.rs include! 引入，保持原有作用域。

impl NavicatMain {
    /// 后台重载 AI 设置页的全部列表（模型/Agent/插件/MCP/会话/安全审计）。
    ///
    /// 读库在克隆控制器上进行以免阻塞 UI；后台任务连同加载后的控制器一起回传，
    /// 成功后必须经 `merge_ai_settings_from` 合并回页面持有的控制器，
    /// 否则页面仍显示刷新前的旧记录。
    fn reload_ai_settings(&mut self, cx: &mut Context<Self>) {
        if self.ai_settings_loading {
            return;
        }
        self.ai_settings_loading = true;
        self.ai_settings_error = None;
        cx.notify();
        let mut controller = self.controller.clone();
        cx.spawn(async move |view, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::RefreshAiSettings);
                    (event, controller)
                })
                .await;
            let (event, controller) = loaded;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.ai_settings_loading = false;
                    match &event {
                        // 只有成功才回搬：失败时旧数据继续可见，错误另行提示。
                        AppEvent::AiSettingsLoaded => {
                            this.controller.merge_ai_settings_from(&controller);
                        }
                        AppEvent::AiSettingsError(message) => {
                            this.ai_settings_error = Some(message.clone());
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// 续读会话历史下一页：后台读库并追加；失败保留错误提示，不伪装成功。
    ///
    /// 与工具启停同一模式：只有一次本地 SQLite 短读，不触网也不连用户数据库，
    /// 因此同步 dispatch 到主控制器，列表立即反映且不产生“克隆控制器状态”丢失。
    fn load_more_ai_sessions(&mut self, cx: &mut Context<Self>) {
        let state = self.controller.state();
        if self.ai_settings_loading || state.ai_sessions_loading_more || !state.ai_sessions_has_more {
            return;
        }
        match self.controller.dispatch(AppCommand::LoadMoreAiSessions) {
            AppEvent::AiSettingsError(message) => self.ai_settings_error = Some(message),
            _ => {}
        }
        cx.notify();
    }

    /// 切换标准工具的全局启停；全局开关收窄实际授权，不清空 Agent 白名单。
    ///
    /// 与模型/插件启停一致走同步 dispatch：这里只有一次本地 SQLite 短事务，
    /// 不触网也不连用户数据库，没有需要移到后台线程的长任务。
    fn set_ai_tool_enabled(
        &mut self,
        tool_id: String,
        enabled: bool,
        expected_revision: i64,
        cx: &mut Context<Self>,
    ) {
        if self.ai_settings_loading {
            return;
        }
        self.ai_settings_error = None;
        let event = self.controller.dispatch(AppCommand::SetAiToolEnabled {
            tool_id: tool_id.clone(),
            enabled,
            expected_revision,
        });
        match event {
            AppEvent::AiToolEnabled { .. } => {
                let message = if enabled {
                    tr("app.ai.tools.enabled_message").replace("%{tool}", &tool_id)
                } else {
                    tr("app.ai.tools.disabled_message").replace("%{tool}", &tool_id)
                };
                self.show_message(message, AppMessageKind::Success, cx);
            }
            AppEvent::AiSettingsError(message) => {
                self.ai_settings_error = Some(message.clone());
                self.show_message(message, AppMessageKind::Error, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// 切换审计列表的判定筛选：点同一张卡回到"全部"。
    fn set_ai_audit_filter(
        &mut self,
        filter: Option<fluxdb_app::AiAuditDecision>,
        cx: &mut Context<Self>,
    ) {
        // 再次点击同一筛选等价于取消，回到全部。
        self.ai_audit_filter = if self.ai_audit_filter == filter {
            None
        } else {
            filter
        };
        self.ai_audit_expanded = None;
        cx.notify();
    }

    /// 清除审计列表的筛选与搜索文本。
    fn clear_ai_audit_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai_audit_filter = None;
        self.ai_audit_filter_text.clear();
        self.ai_audit_filter_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.ai_audit_expanded = None;
        cx.notify();
    }

    /// 展开/收起一条审计记录的详情。
    fn toggle_ai_audit_detail(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ai_audit_expanded = if self.ai_audit_expanded.as_deref() == Some(id) {
            None
        } else {
            Some(id.to_string())
        };
        cx.notify();
    }

    /// 测试模型连接：真实网络 I/O 放后台线程；失败走 Alert，不写进列表。
    fn start_ai_model_test(&mut self, id: String, cx: &mut Context<Self>) {
        if self.ai_model_testing_id.is_some() {
            return;
        }
        self.ai_model_testing_id = Some(id.clone());
        cx.notify();
        let id_for_task = id.clone();
        let mut controller = self.controller.clone();
        cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::TestAiModel { id: id_for_task })
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.ai_model_testing_id = None;
                    match event {
                        AppEvent::AiModelTested { status, .. } => {
                            if let Some(message) = status.strip_prefix("failed:") {
                                // 失败详情只在 Alert 里出现，不落在模型行上。
                                this.show_message(
                                    format!("{}: {message}", tr("app.ai.connection.failed")),
                                    AppMessageKind::Error,
                                    cx,
                                );
                            } else {
                                match status.as_str() {
                                    "ok" => {
                                        this.show_message(
                                            tr("app.ai.connection.ok"),
                                            AppMessageKind::Success,
                                            cx,
                                        )
                                    }
                                    "missing_key" => this.show_message(
                                        tr("app.ai.error.missing_api_key"),
                                        AppMessageKind::Warning,
                                        cx,
                                    ),
                                    "unsupported" => this.show_message(
                                        tr("app.ai.error.provider_test_unsupported"),
                                        AppMessageKind::Warning,
                                        cx,
                                    ),
                                    _ => {}
                                }
                            }
                            // 连接状态已落库，重新加载以刷新行内状态标签。
                            let _ = this.controller.dispatch(AppCommand::RefreshAiSettings);
                        }
                        AppEvent::AiSettingsError(message) => {
                            this.show_message(message, AppMessageKind::Error, cx);
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// 打开 Agent 编辑器：`Some(id)` 选中已有 Agent，`None` 新建草稿。
    fn open_ai_agent_editor(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = id.as_ref().and_then(|id| {
            self.controller
                .state()
                .ai_agents
                .iter()
                .find(|a| &a.id == id)
                .cloned()
        });
        if id.is_some() && existing.is_none() {
            self.show_message(
                tr("app.ai.agents.not_found"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let inputs = AiAgentEditorInputs::new(window, cx);
        inputs
            .compression_percent
            .update(cx, |state, cx| state.set_value("80", window, cx));
        inputs.max_iterations.update(cx, |state, cx| {
            state.set_value(
                fluxdb_storage::ai_platform::agents::DEFAULT_MAX_ITERATIONS.to_string(),
                window,
                cx,
            )
        });
        if existing.is_none() {
            inputs.system_prompt.update(cx, |state, cx| {
                state.set_value(
                    fluxdb_storage::ai_platform::agents::DEFAULT_SYSTEM_PROMPT,
                    window,
                    cx,
                )
            });
        }
        // 调试用（仅 debug）：直接打开指定分区，便于截图核对。
        let debug_tab = if cfg!(debug_assertions)
            && std::env::var("FLUXDB_DEBUG_AI_AGENT_TAB").is_ok_and(|v| v == "tools")
        {
            AiAgentTab::Tools
        } else {
            AiAgentTab::Basic
        };
        let mut editor = AiAgentEditorState {
            deleting: false,
            agent_id: None,
            expected_revision: 0,
            enabled: true,
            tab: debug_tab,
            selected_model_id: None,
            model_menu_open: false,
            // 新建默认全选（= 不限制）；编辑时按已存白名单，空表示不限制。
            selected_tools: AI_AGENT_TOOL_CATALOG
                .iter()
                .map(|(id, ..)| id.to_string())
                .collect(),
            error: None,
            inputs,
        };
        if let Some(agent) = &existing {
            editor.agent_id = Some(agent.id.clone());
            editor.inputs.compression_percent.update(cx, |state, cx| {
                state.set_value(agent.compression_trigger_percent.to_string(), window, cx)
            });
            editor.inputs.max_iterations.update(cx, |state, cx| {
                state.set_value(agent.max_iterations.to_string(), window, cx)
            });
            editor.expected_revision = agent.revision;
            editor.enabled = agent.enabled;
            editor.selected_model_id = agent.model_profile_id.clone();
            if !agent.allowed_tools.is_empty() {
                editor.selected_tools = agent.allowed_tools.iter().cloned().collect();
            }
            let set = |input: &Entity<InputState>,
                       value: String,
                       window: &mut Window,
                       cx: &mut Context<Self>| {
                input.update(cx, |state, cx| state.set_value(value, window, cx));
            };
            set(
                &editor.inputs.display_name,
                agent.display_name.clone(),
                window,
                cx,
            );
            set(
                &editor.inputs.description,
                agent.description.clone(),
                window,
                cx,
            );
            editor.inputs.system_prompt.update(cx, |state, cx| {
                state.set_value(agent.system_prompt.clone(), window, cx)
            });
        }
        self.ai_agent_editor = Some(editor);
        cx.notify();
    }

    /// 提交 Agent 编辑器：先做 UI 校验，再派发保存命令。
    fn submit_ai_agent_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.ai_agent_editor.clone() else {
            return;
        };
        let display_name = ai_input_text(&editor.inputs.display_name, cx);
        if display_name.is_empty() {
            return self.set_ai_agent_editor_error(tr("app.ai.agents.name_required"), cx);
        }
        // 全选时保存空列表（内核默认全集不限制）；部分选中则保存白名单。
        let all_tools: Vec<String> = AI_AGENT_TOOL_CATALOG
            .iter()
            .map(|(id, ..)| id.to_string())
            .collect();
        let allowed_tools = if all_tools
            .iter()
            .all(|id| editor.selected_tools.contains(id))
        {
            Vec::new()
        } else {
            // 保留不可编辑的历史条目，避免收窄白名单被清空后误变为“不限制”。
            let mut list: Vec<String> = editor.selected_tools.iter().cloned().collect();
            list.sort();
            list
        };
        let percent = match ai_input_text(&editor.inputs.compression_percent, cx).parse::<u8>() {
            Ok(p) if (10..=95).contains(&p) => p,
            _ => {
                return self.set_ai_agent_editor_error(
                    tr("app.ai.agents.compression_range"),
                    cx,
                );
            }
        };
        // 不设上限，只要求为正整数。
        let max_iterations =
            match ai_input_text(&editor.inputs.max_iterations, cx).parse::<i64>() {
                Ok(v) if v >= 1 => v,
                _ => {
                    return self.set_ai_agent_editor_error(
                        tr("app.ai.agents.max_iterations_invalid"),
                        cx,
                    );
                }
            };
        let input = fluxdb_app::AiAgentSaveInput {
            compression_trigger_percent: percent,
            max_iterations,
            id: editor.agent_id.clone(),
            display_name,
            description: ai_input_text(&editor.inputs.description, cx),
            system_prompt: editor
                .inputs
                .system_prompt
                .read(cx)
                .value()
                .trim()
                .to_string(),
            model_profile_id: editor.selected_model_id.clone(),
            allowed_tools,
            enabled: editor.enabled,
            expected_revision: editor.expected_revision,
        };
        match self.controller.dispatch(AppCommand::SaveAiAgent(input)) {
            AppEvent::AiAgentSaved { id, revision } => {
                if let Some(state) = self.ai_agent_editor.as_mut() {
                    state.agent_id = Some(id);
                    state.expected_revision = revision;
                    state.error = None;
                }
                self.show_message(tr("app.ai.agents.saved"), AppMessageKind::Success, cx);
            }
            // CAS 冲突等失败：保留草稿，错误内联可见（T014 验收）。
            AppEvent::AiSettingsError(message) => {
                self.set_ai_agent_editor_error(message, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// 把失败原因写进 Agent 编辑器的内联错误区（草稿保留）。
    fn set_ai_agent_editor_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        if let Some(editor) = self.ai_agent_editor.as_mut() {
            editor.error = Some(message.into());
        }
        cx.notify();
    }

    /// 打开模型新建/编辑弹框：`None` 表示新建。
    fn open_ai_model_dialog(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_ai_model_dialog_with_draft(id, None, window, cx);
    }

    fn open_ai_model_dialog_with_draft(
        &mut self,
        id: Option<String>,
        draft: Option<&fluxdb_app::ModelCreateProposal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = id.as_ref().and_then(|id| {
            self.controller
                .state()
                .ai_models
                .iter()
                .find(|m| &m.id == id)
                .cloned()
        });
        if id.is_some() && existing.is_none() {
            self.show_message(
                tr("app.ai.models.not_found"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let mut dialog = AiModelDialog::new(window, cx);
        // 每次打开都清掉上一次的候选列表与搜索词，避免串到别的模型。
        dialog.model_options.clear();
        dialog.model_options_open = false;
        self.ai_model_dialog_filter = String::new();
        self.ai_model_dialog_filter_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        if let Some(model) = &existing {
            dialog.model_id = Some(model.id.clone());
            dialog.expected_revision = model.revision;
            dialog.enabled = model.enabled;
            dialog.egress = model.egress;
            dialog.endpoint_is_full_url = model.endpoint_is_full_url;
            fill_ai_model_inputs(&mut dialog, model, window, cx);
            // 编辑态把已存 Key 预填进掩码输入框（显示黑点，点眼睛看真实值）。
            // 明文只活在这个 InputState 里：不进事件、不进快照、不落日志。
            if model.credential_status == fluxdb_app::AiCredentialStatus::Configured {
                if let Ok(secret) = self.controller.reveal_ai_model_secret(&model.id) {
                    dialog.original_api_key = Some(secret.clone());
                    dialog.inputs.api_key.update(cx, |state, cx| {
                        state.set_value(secret, window, cx);
                    });
                }
            }
        }
        if let Some(draft) = draft {
            let provider = fluxdb_app::ModelProvider::from_stored(&draft.provider).label();
            dialog.inputs.provider.update(cx, |select, cx| {
                select.set_selected_value(&provider.to_string(), window, cx);
            });
            let mut set =
                |input: &Entity<InputState>, value: String, cx: &mut Context<NavicatMain>| {
                input.update(cx, |state, cx| state.set_value(value, window, cx));
                };
            set(&dialog.inputs.display_name, draft.display_name.clone(), cx);
            set(&dialog.inputs.model_key, draft.model.clone(), cx);
            set(
                &dialog.inputs.endpoint,
                draft.endpoint.clone().unwrap_or_default(),
                cx,
            );
            let price = |micros: Option<i64>| {
                micros
                    .map(|micros| format!("{}", micros as f64 / 1_000_000.0))
                    .unwrap_or_default()
            };
            set(
                &dialog.inputs.price_input,
                price(draft.input_price_micros),
                cx,
            );
            set(
                &dialog.inputs.price_output,
                price(draft.output_price_micros),
                cx,
            );
            set(
                &dialog.inputs.price_cache,
                price(draft.cached_input_price_micros),
                cx,
            );
            fill_ai_capacity_field(
                draft.context_window_tokens, &dialog.inputs.context_window, &dialog.inputs.context_unit,
                &mut dialog.capacity_units[0], window, cx,
            );
            fill_ai_capacity_field(
                draft.max_output_tokens, &dialog.inputs.max_output, &dialog.inputs.output_unit,
                &mut dialog.capacity_units[1], window, cx,
            );
            dialog.proposed_capacities = [draft.context_window_tokens, draft.max_output_tokens];
            dialog.enabled = draft.enabled;
            dialog.egress = draft.egress;
            dialog.endpoint_is_full_url = draft.endpoint_is_full_url;
        }
        // 调试用（仅 debug）：预置候选，便于截图核对浮层样式。
        if cfg!(debug_assertions) {
            if let Ok(raw) = std::env::var("FLUXDB_DEBUG_AI_MODEL_OPTIONS") {
                if raw == "1" {
                    dialog.model_options = vec![
                        "Space Bunny Alpha".into(),
                        "deepseek-v4.1-flash".into(),
                        "deepseek-v4.1-flash:free".into(),
                        "glm-5.3-flash".into(),
                        "qwen3.8-flash".into(),
                        "inclusionai/ling-3.1-flash".into(),
                        "deepseek/deepseek-v4.1-flash-fast".into(),
                    ];
                    dialog.model_options_open = true;
                }
            }
        }
        let mut subscriptions = Vec::new();
        for (index, select) in [&dialog.inputs.context_unit, &dialog.inputs.output_unit]
            .into_iter()
            .enumerate()
        {
            subscriptions.push(cx.subscribe_in(
                select,
                window,
                move |this: &mut NavicatMain,
                      _,
                      event: &gpui_component::select::SelectEvent<SearchableVec<String>>,
                      window,
                      cx| {
                    if let gpui_component::select::SelectEvent::Confirm(Some(label)) = event {
                        this.change_ai_token_unit(
                            index,
                            AiTokenUnit::from_label(label),
                            window,
                            cx,
                        );
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &dialog.inputs.model_key,
            window,
            |this: &mut NavicatMain, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_ai_model_capacity(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &dialog.inputs.provider,
            window,
            |this: &mut NavicatMain,
             _,
             event: &gpui_component::select::SelectEvent<SearchableVec<String>>,
             window,
             cx| {
                if matches!(event, gpui_component::select::SelectEvent::Confirm(_)) {
                    this.invalidate_ai_model_catalog(window, cx);
                }
            },
        ));
        for input in [&dialog.inputs.endpoint, &dialog.inputs.api_key] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut NavicatMain, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.invalidate_ai_model_catalog(window, cx);
                    }
                },
            ));
        }
        dialog.subscriptions = std::rc::Rc::new(subscriptions);
        self.ai_model_dialog = Some(dialog);
        cx.notify();
    }

    /// 提交模型弹框：先在 UI 侧做校验，再派发专用保存命令。
    ///
    /// 明文 Key 只进这一条命令；成功后弹框立即关闭。
    fn submit_ai_model_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.ai_model_dialog.clone() else {
            return;
        };
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let display_name = read(&dialog.inputs.display_name);
        let model = read(&dialog.inputs.model_key);
        // 校验失败 → 弹框内联提示（不关框、保留已填草稿）。
        if display_name.is_empty() || model.is_empty() {
            return self.set_ai_model_dialog_error(
                tr("app.ai.model_dialog.name_and_id_required"),
                cx,
            );
        }
        let context_window = match ai_parse_proposed_token_count(
            &ai_input_text(&dialog.inputs.context_window, cx),
            dialog.capacity_units[0], dialog.proposed_capacities[0],
        ) {
            Ok(v) => v,
            Err(msg) => return self.set_ai_model_dialog_error(msg, cx),
        };
        let max_output = match ai_parse_proposed_token_count(
            &ai_input_text(&dialog.inputs.max_output, cx),
            dialog.capacity_units[1], dialog.proposed_capacities[1],
        ) {
            Ok(v) => v,
            Err(msg) => return self.set_ai_model_dialog_error(msg, cx),
        };
        let (price_in, price_out, price_cache) = match (
            ai_input_price_micros(&dialog.inputs.price_input, cx),
            ai_input_price_micros(&dialog.inputs.price_output, cx),
            ai_input_price_micros(&dialog.inputs.price_cache, cx),
        ) {
            (Ok(a), Ok(b), Ok(c)) => (a, b, c),
            (Err(msg), _, _) | (_, Err(msg), _) | (_, _, Err(msg)) => {
                return self.set_ai_model_dialog_error(msg, cx);
            }
        };
        let endpoint = read(&dialog.inputs.endpoint);
        // Key 输入是掩码字段：用户没有实际输入时为空，表示保留原凭据。
        let api_key_value = read(&dialog.inputs.api_key);
        // 与预填值相同 → 视为 Keep（不递增 secret_revision）；只有真正改过才 Set。
        let keep_existing_key = match &dialog.original_api_key {
            Some(original) => original == &api_key_value,
            None => api_key_value.is_empty(),
        };
        let api_key =
            (!keep_existing_key && !api_key_value.is_empty()).then_some(api_key_value.clone());
        // Provider 下拉的值是展示名，保存路径按稳定标识解析：这里必须显式换算，
        // 否则下拉里换 Provider 会被存储标识解析的兜底分支静默吞掉，保存后仍是原值。
        let provider = match dialog.inputs.provider.read(cx).selected_value().cloned() {
            Some(label) => match fluxdb_app::ModelProvider::from_label(&label) {
                Some(provider) => provider,
                None => {
                    return self.set_ai_model_dialog_error(
                        tr("app.ai.model_dialog.provider_invalid_with_value")
                            .replace("%{value}", &label),
                        cx,
                    );
                }
            },
            None => fluxdb_app::ModelProvider::OpenAiChat,
        };
        let input = AiModelSaveInput {
            id: dialog.model_id.clone(),
            display_name,
            // Provider 是模型类型与协议的唯一来源（不再单独选「模型用途」）。
            provider: provider.stored_id().to_string(),
            model,
            endpoint: (!endpoint.is_empty()).then_some(endpoint),
            endpoint_is_full_url: dialog.endpoint_is_full_url,
            enabled: dialog.enabled,
            egress: dialog.egress,
            context_window_tokens: context_window,
            max_output_tokens: max_output,
            input_price_micros: price_in,
            output_price_micros: price_out,
            cached_input_price_micros: price_cache,
            api_key,
            clear_key: false,
            expected_revision: dialog.model_id.as_ref().map(|_| dialog.expected_revision),
        };
        // 先提交、后关框：只有保存成功才关闭；失败（含 CAS 冲突）保留输入并内联提示。
        match self.controller.dispatch(AppCommand::SaveAiModel(input)) {
            AppEvent::AiModelSaved { .. } => {
                self.ai_model_dialog = None;
            }
            AppEvent::AiSettingsError(message) => {
                self.set_ai_model_dialog_error(message, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// 把失败原因写进模型弹框的内联错误区（保持弹框打开，草稿不丢）。
    fn set_ai_model_dialog_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        if let Some(dialog) = self.ai_model_dialog.as_mut() {
            dialog.validation_error = Some(message.into());
        }
        cx.notify();
    }
}

/// 用已有模型档案回填弹框输入。
///
/// 只回填非敏感字段；API Key 不回显（掩码框留空表示保留原凭据）。
fn fill_ai_model_inputs(
    dialog: &mut AiModelDialog,
    model: &fluxdb_app::AiModelItem,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    // 用普通函数而不是闭包：闭包会同时可变借用 window，无法重复调用。
    fn set(
        input: &Entity<InputState>,
        value: String,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) {
        input.update(cx, |state, cx| state.set_value(value, window, cx));
    }
    let inputs = &dialog.inputs;
    // Provider 下拉回填：编辑时必须显示已存 Provider，否则保存会把类型改掉。
    let expected = fluxdb_app::ModelProvider::from_stored(&model.provider).label();
    let current = inputs
        .provider
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_default();
    if current != expected {
        inputs.provider.update(cx, |select, cx| {
            select.set_selected_value(&expected.to_string(), window, cx);
        });
    }
    set(&inputs.display_name, model.display_name.clone(), window, cx);
    set(&inputs.model_key, model.model.clone(), window, cx);
    // Endpoint 必须回填：否则「保存了再打开就没了」（曾漏掉这一行）。
    set(
        &inputs.endpoint,
        model.endpoint.clone().unwrap_or_default(),
        window,
        cx,
    );
    // 容量字段的单位按数量级回填：1M 显示为「1 M」而不是「1000 K」。
    fill_ai_capacity_field(
        model.context_window_tokens,
        &inputs.context_window,
        &inputs.context_unit,
        &mut dialog.capacity_units[0],
        window,
        cx,
    );
    fill_ai_capacity_field(
        model.max_output_tokens,
        &inputs.max_output,
        &inputs.output_unit,
        &mut dialog.capacity_units[1],
        window,
        cx,
    );
    let price = |micros: Option<i64>| {
        micros
            .map(|v| format!("{}", v as f64 / 1_000_000.0))
            .unwrap_or_default()
    };
    set(
        &inputs.price_input,
        price(model.input_price_micros),
        window,
        cx,
    );
    set(
        &inputs.price_output,
        price(model.output_price_micros),
        window,
        cx,
    );
    set(
        &inputs.price_cache,
        price(model.cached_input_price_micros),
        window,
        cx,
    );
}

// ---- Agent 配置 ----

impl NavicatMain {
    fn confirm_ai_model_delete(
        &mut self,
        id: String,
        name: String,
        revision: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_model_deleting_id.is_some() || self.ai_model_testing_id.as_deref() == Some(&id) {
            return;
        }
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let id = id.clone();
            dialog
                .title(tr("app.ai.models.delete_title"))
                .close_button(true)
                .overlay_closable(true)
                .keyboard(true)
                .child(div()
                    .text_size(ui_text_size(13.))
                    .child(format!(
                        "{} {name}? {}",
                        tr("app.ai.models.delete_confirm_prefix"),
                        tr("app.ai.models.delete_confirm_detail")
                    )))
                .footer(div().flex().justify_end().gap_2()
                    .child(Button::new("ai-delete-cancel").debug_selector(|| "ai-delete-cancel".into())
                        .cursor_pointer().child(tr("app.common.cancel"))
                        .on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("ai-delete-confirm").debug_selector(|| "ai-delete-confirm".into())
                        .danger().cursor_pointer().child(tr("app.ai.models.delete_action"))
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(gpui_component::dialog::Confirm { secondary: false }), cx))))
                .on_ok(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| this.start_ai_model_delete(id.clone(), revision, cx));
                    true
                })
        });
    }

    fn start_ai_model_delete(&mut self, id: String, revision: i64, cx: &mut Context<Self>) {
        if self.ai_model_deleting_id.is_some() || self.ai_model_testing_id.as_deref() == Some(&id) {
            return;
        }
        self.ai_model_deleting_id = Some(id.clone());
        cx.notify();
        let mut controller = self.controller.clone();
        cx.spawn(async move |view, cx| {
            let task_id = id.clone();
            let event = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::DeleteAiModel {
                        id: task_id,
                        expected_revision: revision,
                    })
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.ai_model_deleting_id = None;
                    let result = match event {
                        AppEvent::AiModelDeleted { .. } => Ok(()),
                        AppEvent::AiSettingsError(message) => Err(message),
                        _ => Err(tr("app.ai.models.delete_failed").to_string()),
                    };
                    match this
                        .controller
                        .dispatch(AppCommand::CompleteAiModelDelete { id, result })
                    {
                        AppEvent::AiModelDeleted { .. } => {
                            this.show_message(
                                tr("app.ai.models.deleted"),
                                AppMessageKind::Success,
                                cx,
                            )
                        }
                        AppEvent::AiSettingsError(message) => {
                            this.show_message(message, AppMessageKind::Error, cx)
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }
}
