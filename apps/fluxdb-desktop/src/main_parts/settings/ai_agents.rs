// AI 设置职责拆分；由 ai.rs include! 引入，保持原有作用域。

/// Agent 绑定模型的下拉触发条；展开态归编辑器所有（`AiAgentEditorState::model_menu_open`）。
fn ai_agent_model_trigger(
    current: &str,
    open: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    div()
        .id(SharedString::from("ai-agent-model-trigger"))
        .h(px(34.))
        .px_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(if open { colors.info } else { colors.border })
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .text_size(ui_text_size(12.))
                .text_color(colors.text)
                .child(current.to_string()),
        )
        .child(app_icon(
            if open {
                AppIcon::ChevronUp
            } else {
                AppIcon::ChevronDown
            },
            14.,
            colors.muted,
        ))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if let Some(editor) = this.ai_agent_editor.as_mut() {
                    editor.model_menu_open = !editor.model_menu_open;
                }
                cx.notify();
                cx.stop_propagation();
            }),
        )
}

/// 模型候选列表容器（内联展开，不用浮层以免定位复杂）。
fn ai_model_candidates(colors: UiColors) -> Div {
    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
}

/// Agent 编辑页签按钮。
fn ai_agent_tab_button(
    label: impl Into<SharedString>,
    value: AiAgentTab,
    active: AiAgentTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let label = label.into();
    let debug_label = label.clone();
    let is_active = value == active;
    div()
        .id(SharedString::from(format!("ai-agent-tab:{label}")))
        .debug_selector(move || format!("ai-agent-tab:{debug_label}"))
        .px_3()
        .pt_1()
        .pb_2()
        .cursor_pointer()
        .border_b_2()
        .border_color(if is_active {
            colors.info
        } else {
            colors.info.alpha(0.0)
        })
        .text_size(ui_text_size(12.))
        .text_color(if is_active { colors.text } else { colors.muted })
        .font_weight(if is_active {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .hover(move |style| style.text_color(colors.text))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if let Some(editor) = this.ai_agent_editor.as_mut() {
                    editor.tab = value;
                }
                cx.notify();
                cx.stop_propagation();
            }),
        )
}

/// 多行提示词输入（保留换行）。
fn ai_prompt_field(
    label: impl Into<SharedString>,
    input: &Entity<gpui_component::input::TextareaState>,
    colors: UiColors,
) -> impl IntoElement {
    let label = label.into();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .child(
            div()
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(label),
        )
        .child(
            div()
                .debug_selector(|| "ai-agent-prompt".into())
                .child(gpui_component::input::Textarea::new(input).h(px(180.))),
        )
}

/// Agent 编辑器：页签 + 表单 + 底部保存栏。
fn ai_agent_editor_form(
    state: &AppState,
    editor: &AiAgentEditorState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tabs = div()
        .border_b_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .gap_1()
        .child(ai_agent_tab_button(
            tr("app.ai.agents.tab.basic").to_string(),
            AiAgentTab::Basic,
            editor.tab,
            colors,
            cx,
        ))
        .child(ai_agent_tab_button(
            tr("app.ai.agents.tab.model_prompt").to_string(),
            AiAgentTab::Model,
            editor.tab,
            colors,
            cx,
        ))
        .child(ai_agent_tab_button(
            tr("app.ai.agents.tab.tools").to_string(),
            AiAgentTab::Tools,
            editor.tab,
            colors,
            cx,
        ));

    let body: gpui::AnyElement = match editor.tab {
        AiAgentTab::Basic => {
            let enabled = editor.enabled;
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(ai_field(
                    tr("app.ai.agents.name"),
                    &editor.inputs.display_name,
                    colors,
                ))
                .child(ai_field(
                    tr("app.ai.agents.description"),
                    &editor.inputs.description,
                    colors,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(colors.text)
                                .child(tr("app.ai.agents.enabled")),
                        )
                        .child(
                            Switch::new("ai-agent-enabled")
                                .checked(enabled)
                                .disabled(
                                    editor.agent_id.as_deref()
                                        == Some(
                                            fluxdb_storage::ai_platform::agents::BUILTIN_AGENT_ID,
                                        ),
                                )
                                .on_click(cx.listener(move |this, checked, _, cx| {
                                    if let Some(editor) = this.ai_agent_editor.as_mut() {
                                        editor.enabled = *checked;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element()
        }
        AiAgentTab::Model => {
            let generation: Vec<(String, String)> = state
                .ai_models
                .iter()
                .filter(|m| m.kind == fluxdb_app::AiModelKind::Generation)
                .map(|m| (m.id.clone(), m.display_name.clone()))
                .collect();
            let current_label = editor
                .selected_model_id
                .as_ref()
                .and_then(|id| generation.iter().find(|(i, _)| i == id))
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| tr("app.ai.agents.follow_default_model").to_string());

            let mut col = div().flex().flex_col().gap_3().child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(colors.text)
                            .child(tr("app.ai.agents.default_generation_model")),
                    )
                    .child(ai_agent_model_trigger(&current_label, editor.model_menu_open, colors, cx)),
            );
            if editor.model_menu_open {
                let mut menu = ai_model_candidates(colors);
                for (id, name) in &generation {
                    let id_owned = id.clone();
                    let selected = editor.selected_model_id.as_deref() == Some(id.as_str());
                    menu = menu.child(
                        div()
                            .id(SharedString::from(format!("ai-agent-model-opt:{id}")))
                            .h(px(30.))
                            .px_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .cursor_pointer()
                            .bg(if selected {
                                colors.hover
                            } else {
                                colors.panel_bg
                            })
                            .hover(move |style| style.bg(colors.hover))
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(colors.text)
                                    .child(name.clone()),
                            )
                            .when(selected, |this| {
                                this.child(app_icon(AppIcon::Check, 14., colors.info))
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(editor) = this.ai_agent_editor.as_mut() {
                                        editor.selected_model_id = Some(id_owned.clone());
                                        editor.model_menu_open = false;
                                    }
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                    );
                }
                col = col.child(menu);
            }
            col.child(ai_field(
                tr("app.ai.agents.compression_percent"),
                &editor.inputs.compression_percent,
                colors,
            ))
            .child(ai_field(
                tr("app.ai.agents.max_iterations"),
                &editor.inputs.max_iterations,
                colors,
            ))
            .child(ai_prompt_field(
                tr("app.ai.agents.prompt"),
                &editor.inputs.system_prompt,
                colors,
            ))
            .into_any_element()
        }
        AiAgentTab::Tools => {
            // 全局关闭的工具在这里锁定：Agent 只能在全局允许的工具里再收窄。
            let globally_disabled: std::collections::BTreeSet<String> = state
                .ai_tool_settings
                .iter()
                .filter(|t| !t.enabled)
                .map(|t| t.tool_id.clone())
                .collect();
            let mut list = div().flex().flex_col().gap_1();
            for (id, group, desc, write) in AI_AGENT_TOOL_CATALOG {
                let id_owned = id.to_string();
                let globally_off = globally_disabled.contains(*id);
                let checked = editor.selected_tools.contains(*id) && !globally_off;
                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(200.))
                                .text_size(ui_text_size(12.))
                                .text_color(colors.text)
                                .child(id.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(desc.to_string()),
                        )
                        .child(ai_pill(
                            if *group == "artifact" {
                                tr("app.ai.tools.group.artifact")
                            } else if *write {
                                tr("app.ai.agents.access.write")
                            } else {
                                tr("app.ai.agents.access.read")
                            },
                            if *group == "artifact" {
                                colors.info
                            } else if *write {
                                colors.warning
                            } else {
                                colors.success
                            },
                            colors,
                        ))
                        .child(
                            div()
                                .w(px(60.))
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(group.to_string()),
                        )
                        .child(
                            Switch::new(SharedString::from(format!("ai-agent-tool:{id}")))
                                .checked(checked)
                                .disabled(globally_off)
                                .on_click(cx.listener(move |this, checked, _, cx| {
                                    if let Some(editor) = this.ai_agent_editor.as_mut() {
                                        if *checked {
                                            editor.selected_tools.insert(id_owned.clone());
                                        } else {
                                            editor.selected_tools.remove(&id_owned);
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.ai.agents.tools_available")),
                )
                // 被全局关闭的工具在此说明原因，避免用户以为开关坏了。
                .when(!globally_disabled.is_empty(), |this| {
                    this.child(
                        div()
                            .rounded(colors.radius)
                            .px_3()
                            .py_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .bg(colors.warning.alpha(0.12))
                            .child(app_icon(AppIcon::Info, 14., colors.warning))
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(colors.warning)
                                    .child(tr("app.ai.agents.globally_disabled").replace(
                                        "%{count}",
                                        &globally_disabled.len().to_string(),
                                    )),
                            ),
                    )
                })
                .child(list)
                .into_any_element()
        }
    };

    let is_new = editor.agent_id.is_none();
    // 校验/保存失败（含 CAS 冲突）：内联红条，编辑器保持打开、草稿不丢（T014 验收）。
    let error_banner = div().when_some(editor.error.clone(), |this, message| {
        this.rounded(colors.radius)
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .bg(colors.danger.alpha(0.12))
            .child(app_icon(AppIcon::CircleSlash, 14., colors.danger))
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.danger)
                    .child(message),
            )
    });
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(tabs)
        .child(body)
        .child(error_banner)
        .child(
            div()
                .border_t_1()
                .border_color(colors.border_soft)
                .pt_3()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("ai-agent-cancel")
                        .small()
                        .ghost()
                        .child(tr("app.common.cancel"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ai_agent_editor = None;
                            cx.notify();
                        })),
                )
                .children(
                    editor
                        .agent_id
                        .as_ref()
                        .filter(|id| {
                            id.as_str() != fluxdb_storage::ai_platform::agents::BUILTIN_AGENT_ID
                        })
                        .map(|id| {
                            let id = id.clone();
                            let revision = editor.expected_revision;
                            let name = state
                                .ai_agents
                                .iter()
                                .find(|a| a.id == id)
                                .map(|a| a.display_name.clone())
                                .unwrap_or_default();
                            Button::new("ai-agent-delete")
                                .small()
                                .disabled(editor.deleting)
                                .danger()
                                .cursor_pointer()
                                .debug_selector(|| "ai-agent-delete".into())
                                .child(tr("app.ai.agents.delete"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.confirm_ai_agent_delete(
                                        id.clone(),
                                        name.clone(),
                                        revision,
                                        window,
                                        cx,
                                    )
                                }))
                        }),
                )
                .child(
                    Button::new("ai-agent-save")
                        .small()
                        .disabled(editor.deleting)
                        .primary()
                        .debug_selector(|| "ai-agent-save".into())
                        .child(if is_new {
                            tr("app.ai.agents.create")
                        } else {
                            tr("app.ai.agents.save")
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.submit_ai_agent_editor(cx);
                        })),
                ),
        )
}

/// 「Agent 配置」分区：左侧列表 + 右侧编辑器。
fn settings_ai_agents_panel(
    state: &AppState,
    panel: &AiSettingsPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected_id = panel.agent_editor.as_ref().and_then(|e| e.agent_id.clone());

    let list_rows: Vec<gpui::AnyElement> = if state.ai_agents.is_empty() {
        vec![ai_empty_state(&tr("app.ai.agents.empty"), colors)]
    } else {
        state
            .ai_agents
            .iter()
            .map(|a| {
                let id = a.id.clone();
                let id_for_debug = id.clone();
                let selected = selected_id.as_deref() == Some(a.id.as_str());
                div()
                    .id(SharedString::from(format!("ai-agent-item:{}", a.id)))
                    .debug_selector(move || format!("ai-agent-item:{id_for_debug}"))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .bg(if selected {
                        colors.hover
                    } else {
                        colors.panel_bg
                    })
                    .hover(move |style| style.bg(colors.hover))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.text)
                            .child(a.display_name.clone()),
                    )
                    .child(div().text_size(ui_text_size(11.)).text_color(colors.muted).child(
                        if a.id == fluxdb_storage::ai_platform::agents::BUILTIN_AGENT_ID {
                            tr("app.ai.agents.builtin_editable").to_string()
                        } else if a.enabled {
                            tr("app.ai.status.enabled").to_string()
                        } else {
                            tr("app.ai.status.disabled").to_string()
                        },
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.open_ai_agent_editor(Some(id.clone()), window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .into_any_element()
            })
            .collect()
    };

    let editor_body: gpui::AnyElement = match panel.agent_editor.as_ref() {
        None => ai_empty_state(&tr("app.ai.agents.select_hint"), colors),
        Some(editor) => ai_agent_editor_form(state, editor, colors, cx).into_any_element(),
    };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div().flex().flex_col().gap_1().child(
                        div()
                            .text_size(ui_text_size(14.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.text)
                            .child("Agent"),
                    ),
                )
                .child(
                    Button::new("ai-agent-new")
                        .small()
                        .primary()
                        .child(tr("app.ai.agents.new"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_ai_agent_editor(None, window, cx);
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .gap_4()
                .items_start()
                .child(
                    div()
                        .w(px(240.))
                        .flex_none()
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .children(list_rows),
                )
                .child(div().flex_1().min_w(px(0.)).child(editor_body)),
        )
}

/// 会话状态/最近 Turn 状态的中文标签与颜色（状态同时有文字，不只靠颜色区分）。
fn ai_session_status_label(status: &str, colors: UiColors) -> (SharedString, gpui::Rgba) {
    match status {
        "queued" | "preparing" | "generating" | "executing_tools" | "cancel_requested" => {
            (tr("app.ai.sessions.status.running"), colors.info)
        }
        "waiting_approval" => (tr("app.ai.sessions.status.waiting_approval"), colors.warning),
        "waiting_user" => (tr("app.ai.sessions.status.waiting_user"), colors.warning),
        "cancelled" => (tr("app.ai.sessions.status.cancelled"), colors.muted),
        "failed" => (tr("app.ai.sessions.status.failed"), colors.danger),
        "interrupted" => (tr("app.ai.sessions.status.interrupted"), colors.warning),
        "outcome_unknown" => (tr("app.ai.sessions.status.outcome_unknown"), colors.danger),
        "completed" => (tr("app.ai.sessions.status.completed"), colors.success),
        _ => (tr("app.ai.sessions.status.not_started"), colors.muted),
    }
}

/// 会话时间戳（毫秒）格式化为本地时间 `MM-DD HH:MM`；无效值回落为 `—`。
fn ai_session_timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".to_string())
}

/// 目标摘要：从 Turn 的公开 target JSON 取连接/库/对象名；无目标显示占位。
fn ai_session_target_summary(target_json: Option<&str>) -> String {
    let Some(target_json) = target_json else {
        return tr("app.ai.sessions.no_target").to_string();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(target_json) else {
        return tr("app.ai.sessions.no_target").to_string();
    };
    // target_json 结构由 T021 目标快照决定：优先库/对象，其次 Redis db。
    let parts = [
        value.get("database").and_then(|v| v.as_str()),
        value.get("schema").and_then(|v| v.as_str()),
        value.get("object").and_then(|v| v.as_str()),
        value.get("redis_db").and_then(|v| v.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if parts.is_empty() {
        tr("app.ai.sessions.no_target").to_string()
    } else {
        parts.join(" / ")
    }
}

/// 「会话历史」分区：真实会话列表（标题、Agent/目标、状态、打开/重命名/归档）。
fn settings_ai_sessions_panel(
    state: &AppState,
    panel: &AiSettingsPanelState,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let filter = panel.session_filter.trim().to_lowercase();
    let filtered: Vec<&fluxdb_app::AiSessionItem> = state
        .ai_sessions
        .iter()
        .filter(|s| filter.is_empty() || s.title.to_lowercase().contains(&filter))
        .collect();

    let rows: Vec<gpui::AnyElement> = if panel.settings_loading {
        vec![inline_loading_row(tr("app.ai.sessions.loading"), colors).into_any_element()]
    } else if let Some(message) = panel.settings_error.as_ref() {
        vec![ai_settings_error_block(message, colors, cx)]
    } else if state.ai_sessions.is_empty() {
        vec![ai_empty_state(&tr("app.ai.sessions.empty"), colors)]
    } else if filtered.is_empty() {
        vec![ai_empty_state(&tr("app.ai.sessions.no_match"), colors)]
    } else {
        filtered
            .iter()
            .map(|session| {
                ai_session_row(
                    session,
                    state,
                    panel.session_archiving_id.as_deref() == Some(session.id.as_str()),
                    colors,
                    window,
                    cx,
                )
                .into_any_element()
            })
            .collect()
    };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group(tr("app.ai.sessions.title"), colors)
                .child(
                    div()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(colors.muted)
                                .child(
                                    tr("app.ai.sessions.summary")
                                        .replace("%{total}", &state.ai_sessions_total.to_string())
                                        .replace("%{loaded}", &state.ai_sessions.len().to_string()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(ai_session_search_field(
                                    &panel.session_filter_input,
                                    colors,
                                    cx,
                                ))
                                .child(
                                    Button::new("ai-sessions-refresh")
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .debug_selector(|| "ai-sessions-refresh".into())
                                        .disabled(panel.settings_loading)
                                        .child(if panel.settings_loading {
                                            tr("app.ai.models.refreshing")
                                        } else {
                                            tr("app.common.refresh")
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reload_ai_settings(cx);
                                        })),
                                )
                                // 续读下一页：放在头部便于直接点击（长列表不必先滚到底）；
                                // 加载中 disabled 防重复请求，已到底时不显示。
                                .when(state.ai_sessions_has_more, |row| {
                                    row.child(
                                        Button::new("ai-sessions-load-more")
                                            .small()
                                            .outline()
                                            .cursor_pointer()
                                            .debug_selector(|| "ai-sessions-load-more".into())
                                            .disabled(state.ai_sessions_loading_more)
                                            .child(if state.ai_sessions_loading_more {
                                                tr("app.ai.sessions.loading_more")
                                            } else {
                                                tr("app.ai.sessions.load_more")
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.load_more_ai_sessions(cx);
                                            })),
                                    )
                                }),
                        ),
                )
                .child(
                    div()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .px_4()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(ai_table_head(
                            tr("app.ai.sessions.header.session"),
                            300.,
                            colors,
                        ))
                        .child(ai_table_head(
                            tr("app.ai.sessions.header.agent_target"),
                            240.,
                            colors,
                        ))
                        .child(ai_table_head(
                            tr("app.ai.sessions.header.status"),
                            120.,
                            colors,
                        ))
                        .child(div().flex_1()),
                )
                // 窄窗口：四列保持最小宽度并横向滚动，避免列被压扁/裁切。
                .child(
                    div().w_full().overflow_x_scrollbar().child(
                        div()
                            .min_w(px(760.))
                            .flex()
                            .flex_col()
                            .child(div().children(rows)),
                    ),
                )
        )
}

/// 会话搜索框（复用「大模型」页同款圆角搜索条）。
fn ai_session_search_field(
    input: &Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let input = input.clone();
    let value = input.read(cx).value().to_string();
    div()
        .w(px(200.))
        .h(px(28.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .child(app_icon(AppIcon::Search, 12., colors.muted))
        .child(
            div().flex_1().min_w(px(0.)).child(
                Input::new(&input)
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false)
                    .w_full()
                    .h_full()
                    .text_size(ui_text_size(12.)),
            ),
        )
        .when(!value.is_empty(), |this| {
            this.child(
                div()
                    .id("ai-session-search-clear")
                    .debug_selector(|| "ai-session-search-clear".into())
                    .cursor_pointer()
                    .child(app_icon(AppIcon::Close, 12., colors.muted))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.ai_session_filter_input
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        this.ai_session_filter.clear();
                        cx.notify();
                    })),
            )
        })
}

/// 单个会话行：会话标题/消息数、Agent 与目标、最近状态、行内动作。
fn ai_session_row(
    session: &fluxdb_app::AiSessionItem,
    state: &AppState,
    archiving: bool,
    colors: UiColors,
    _window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let session_id = session.id.clone();
    let agent_name = state
        .ai_agents
        .iter()
        .find(|agent| agent.id == session.agent_id)
        .map(|agent| agent.display_name.clone())
        .unwrap_or_else(|| session.agent_id.clone());
    let (status_label, status_color) = ai_session_status_label(
        session.last_turn_status.as_deref().unwrap_or_default(),
        colors,
    );
    let target = ai_session_target_summary(session.last_target_json.as_deref());
    let open_id = session_id.clone();
    let rename_id = session_id.clone();
    let rename_title = session.title.clone();
    let rename_revision = session.revision;
    let archive_id = session_id.clone();
    let archive_revision = session.revision;
    let purge_id = session_id.clone();
    let purge_revision = session.revision;
    let purge_title = session.title.clone();
    let archived = session.status == "archived";
    let turns = if session.turn_count == 0 {
        tr("app.ai.sessions.not_sent").to_string()
    } else {
        tr("app.ai.sessions.turns").replace("%{count}", &session.turn_count.to_string())
    };
    ai_row_container(&format!("ai-session-row:{session_id}"), colors)
        .debug_selector({
            let session_id = session_id.clone();
            move || format!("ai-session-row:{session_id}")
        })
        .min_h(px(56.))
        .py_2()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .w(px(300.))
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .truncate()
                        .child(session.title.clone()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.ai.sessions.row_summary")
                                .replace("%{messages}", &session.message_count.to_string())
                                .replace("%{turns}", &turns)
                                .replace(
                                    "%{updated}",
                                    &ai_session_timestamp(session.updated_at_ms),
                                ),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .w(px(240.))
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.text)
                        .truncate()
                        .child(agent_name),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .truncate()
                        .child(target),
                ),
        )
        .child(
            div()
                .w(px(120.))
                .flex()
                .items_center()
                .child(ai_pill(status_label, status_color, colors))
                .when(session.status == "archived", |this| {
                    this.child(div().ml_1().child(ai_pill(
                        tr("app.ai.sessions.archived"),
                        colors.muted,
                        colors,
                    )))
                }),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("ai-session-open:{open_id}")))
                        .small()
                        .outline()
                        .cursor_pointer()
                        .debug_selector({
                            let open_id = open_id.clone();
                            move || format!("ai-session-open:{open_id}")
                        })
                        .child(tr("app.ai.sessions.open_panel"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_ai_session_from_settings(open_id.clone(), window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("ai-session-rename:{rename_id}")))
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .tooltip(tr("app.ai.sessions.rename"))
                        .debug_selector({
                            let rename_id = rename_id.clone();
                            move || format!("ai-session-rename:{rename_id}")
                        })
                        .disabled(archiving)
                        .child(app_icon(AppIcon::Edit, 13., colors.muted))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_ai_session_rename(
                                rename_id.clone(),
                                rename_title.clone(),
                                rename_revision,
                                window,
                                cx,
                            );
                        })),
                )
                .child({
                    // 归档与恢复共用同一个后台 CAS 入口，按钮文案随当前状态切换。
                    let archived = session.status == "archived";
                    let action = if archived { "restore" } else { "archive" };
                    Button::new(SharedString::from(format!(
                        "ai-session-{action}:{session_id}"
                    )))
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .tooltip(if archived {
                        tr("app.ai.sessions.restore")
                    } else {
                        tr("app.ai.sessions.archive")
                    })
                    .debug_selector({
                        let archive_id = archive_id.clone();
                        move || {
                            format!(
                                "ai-session-{}:{archive_id}",
                                if archived { "restore" } else { "archive" }
                            )
                        }
                    })
                    .disabled(archiving)
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(if archiving {
                                tr("app.ai.sessions.processing")
                            } else if archived {
                                tr("app.ai.sessions.restore")
                            } else {
                                tr("app.ai.sessions.archive")
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_ai_session_status_change(
                            archive_id.clone(),
                            archive_revision,
                            archived,
                            window,
                            cx,
                        );
                    }))
                })
                // 删除只对已归档会话开放：活跃会话需先归档，避免误删进行中任务。
                .when(archived, |row| {
                    row.child(
                        Button::new(SharedString::from(format!(
                            "ai-session-purge:{purge_id}"
                        )))
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .tooltip(tr("app.ai.sessions.delete_tooltip"))
                        .debug_selector({
                            let purge_id = purge_id.clone();
                            move || format!("ai-session-purge:{purge_id}")
                        })
                        .disabled(archiving)
                        .child(app_icon(AppIcon::Trash, 13., colors.danger))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_ai_session_purge(
                                purge_id.clone(),
                                purge_title.clone(),
                                purge_revision,
                                window,
                                cx,
                            );
                        })),
                    )
                }),
        )
}

// 「会话历史」页的行内动作：在面板打开、重命名（弹框）、归档（后台 CAS）。
impl NavicatMain {
    /// 从设置页「会话历史」把某个会话装进询问 AI 面板并显示。
    ///
    /// 走与面板历史列表相同的生产链路：先保存当前草稿，再发 `Load` 命令，
    /// 由后台读取后回桥；不直接改 `controller.state`。
    fn open_ai_session_from_settings(
        &mut self,
        session_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 设置页已经存在时不再重复开标签，只切回该标签。
        let _ = self.controller.dispatch(AppCommand::OpenSettings);
        if self.ai_chat.open {
            self.save_ai_chat_ui(window, cx);
        }
        self.ai_chat.open = true;
        self.ai_chat.history = false;
        self.ai_chat.error = None;
        self.ai_chat.epoch += 1;
        self.ai_chat_request(
            fluxdb_app::AiConversationCommand::Load {
                session_id: Some(session_id),
            },
            window,
            cx,
        );
    }

    /// 删除已归档会话：确认弹框（Esc/遮罩/取消不写库），后台按依赖清理并删除受控文件。
    fn confirm_ai_session_purge(
        &mut self,
        id: String,
        title: String,
        expected_revision: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_session_archiving_id.is_some() {
            return;
        }
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let id = id.clone();
            dialog
                .title(tr("app.ai.sessions.delete_title"))
                .close_button(true)
                .overlay_closable(true)
                .keyboard(true)
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .child(tr("app.ai.sessions.delete_confirm").replace("%{name}", &title)),
                )
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("ai-session-purge-cancel")
                                .debug_selector(|| "ai-session-purge-cancel".into())
                                .cursor_pointer()
                                .child(tr("app.common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("ai-session-purge-confirm")
                                .debug_selector(|| "ai-session-purge-confirm".into())
                                .danger()
                                .cursor_pointer()
                                .child(tr("app.ai.sessions.delete_action"))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(gpui_component::dialog::Confirm {
                                            secondary: false,
                                        }),
                                        cx,
                                    )
                                }),
                        ),
                )
                .on_ok(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.start_ai_session_purge(id.clone(), expected_revision, cx)
                    });
                    true
                })
        });
    }

    /// 执行删除：后台读依赖事实 + 删库 + 清文件，结果经 Complete 命令合并到主控制器。
    fn start_ai_session_purge(
        &mut self,
        id: String,
        expected_revision: i64,
        cx: &mut Context<Self>,
    ) {
        if self.ai_session_archiving_id.is_some() {
            return;
        }
        self.ai_session_archiving_id = Some(id.clone());
        cx.notify();
        let mut controller = self.controller.clone();
        cx.spawn(async move |view, cx| {
            let (id, result) = cx
                .background_spawn({
                    let id = id.clone();
                    async move {
                        let event = controller.dispatch(AppCommand::PurgeAiSession {
                            id: id.clone(),
                            expected_revision,
                        });
                        let result = match event {
                            AppEvent::AiSessionPurged { .. } => Ok(()),
                            AppEvent::AiSettingsError(message) => Err(message),
                            _ => Err(tr("app.ai.sessions.delete_failed").to_string()),
                        };
                        (id, result)
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.ai_session_archiving_id = None;
                    let event = this.controller.dispatch(AppCommand::CompleteAiSessionPurge {
                        id: id.clone(),
                        result,
                    });
                    match event {
                        AppEvent::AiSessionPurged { .. } => {
                            this.show_message(
                                tr("app.ai.sessions.deleted"),
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

    /// 归档 / 恢复会话：对象级 CAS 后台执行，行内 loading/disabled，失败保留原状态。
    fn start_ai_session_status_change(
        &mut self,
        id: String,
        expected_revision: i64,
        restore: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_session_archiving_id.is_some() {
            return;
        }
        self.ai_session_archiving_id = Some(id.clone());
        cx.notify();
        let mut controller = self.controller.clone();
        cx.spawn(async move |view, cx| {
            // 后台写库走控制器副本，结果经 Complete 命令回桥到主窗口控制器。
            let (id, revision, result) = cx
                .background_spawn({
                    let id = id.clone();
                    async move {
                        let event = if restore {
                            controller.dispatch(AppCommand::RestoreAiSession {
                                id: id.clone(),
                                expected_revision,
                            })
                        } else {
                            controller.dispatch(AppCommand::ArchiveAiSession {
                                id: id.clone(),
                                expected_revision,
                            })
                        };
                        let revision = controller
                            .state()
                            .ai_sessions
                            .iter()
                            .find(|s| s.id == id)
                            .map(|s| s.revision)
                            .unwrap_or(expected_revision + 1);
                        let result = match event {
                            AppEvent::AiSessionArchived { .. }
                            | AppEvent::AiSessionRestored { .. } => Ok(()),
                            AppEvent::AiSettingsError(message) => Err(message),
                            _ => Err(tr("app.ai.sessions.archive_failed").to_string()),
                        };
                        (id, revision, result)
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.ai_session_archiving_id = None;
                    let event = this
                        .controller
                        .dispatch(AppCommand::CompleteAiSessionStatus {
                            id: id.clone(),
                            archived: !restore,
                            revision,
                            result,
                        });
                    match event {
                        AppEvent::AiSessionArchived { .. } => {
                            this.show_message(
                                tr("app.ai.sessions.archived_message"),
                                AppMessageKind::Success,
                                cx,
                            )
                        }
                        AppEvent::AiSessionRestored { .. } => {
                            this.show_message(
                                tr("app.ai.sessions.restored"),
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

