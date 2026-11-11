const CONFIG_PROPOSAL_TTL_MS: u64 = 10 * 60 * 1000;

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn config_proposal_modal(
    proposal: fluxdb_app::ConfigProposal,
    settings: &Settings,
    busy: bool,
    delete_confirmed: bool,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let destructive = config_proposal_destructive(&proposal.kind);
    let rows = config_proposal_rows(settings, &proposal);
    let action = config_proposal_action_label(&proposal.kind);
    let apply_label = if matches!(
        &proposal.kind,
        fluxdb_app::ConfigProposalKind::ModelCreate(_)
            | fluxdb_app::ConfigProposalKind::ModelUpdate { .. }
    ) {
        tr("app.ai.config_proposal.continue")
    } else {
        tr("app.ai.config_proposal.apply")
    };
    div()
        .id("ConfigProposalModal")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.cancel_config_proposal(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .id("config-proposal-panel")
                .w(px(560.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("ConfigProposalModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_config_proposal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.ai.config_proposal.title")),
                        )
                        .child(
                            div()
                                .id("config-proposal-close")
                                .aria_label(tr("app.common.close"))
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.cancel_config_proposal(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .text_size(ui_text_size(13.))
                        .child(
                            div()
                                .text_color(colors.muted)
                                .child(tr("app.ai.config_proposal.description")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(colors.muted)
                                .child(action),
                        )
                        .child(
                            div()
                                .id("config-proposal-changes")
                                .max_h(px(360.))
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .children(rows.into_iter().map(|(field, before, after)| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(190.))
                                                .text_color(colors.muted)
                                                .child(config_field_label(&field)),
                                        )
                                        .when(!before.is_empty(), |this| {
                                            this.child(
                                                div()
                                                    .min_w(px(0.))
                                                    .flex_1()
                                                    .whitespace_normal()
                                                    .child(display_diff_value(&before)),
                                            )
                                            .child(div().text_color(colors.muted).child("→"))
                                        })
                                        .child(
                                            div()
                                                .min_w(px(0.))
                                                .flex_1()
                                                .whitespace_normal()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .child(display_diff_value(&after)),
                                        )
                                })),
                        ),
                )
                .when(destructive, |this| {
                    this.child(
                        div().px_5().pb_3().child(
                            gpui_component::checkbox::Checkbox::new("config-proposal-ack")
                                .label(tr("app.ai.config_proposal.acknowledge"))
                                .checked(delete_confirmed)
                                .disabled(busy)
                                .on_click(cx.listener(|this, checked, _, cx| {
                                    this.config_delete_confirmed = *checked;
                                    cx.notify();
                                })),
                        ),
                    )
                })
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            div()
                                .debug_selector(|| "config-proposal-cancel".into())
                                .child(
                                    Button::new("config-proposal-cancel")
                                        .label(tr("app.common.cancel"))
                                        .disabled(busy)
                                        .w(px(86.))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_config_proposal(cx);
                                            cx.stop_propagation();
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "config-proposal-apply".into())
                                .child(
                                    Button::new("config-proposal-apply")
                                        .label(apply_label)
                                        .loading(busy)
                                        .disabled(busy || (destructive && !delete_confirmed))
                                        .primary()
                                        .w(px(120.))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm_config_proposal(window, cx);
                                            cx.stop_propagation();
                                        })),
                                ),
                        ),
                ),
        )
}

fn config_proposal_action_label(kind: &fluxdb_app::ConfigProposalKind) -> SharedString {
    match kind {
        fluxdb_app::ConfigProposalKind::SettingUpdate(prefill) => {
            settings_section_label(settings_panel_section_from_ai(prefill.section))
        }
        fluxdb_app::ConfigProposalKind::ModelUpdate { .. } => {
            tr("app.ai.config_proposal.action_model_update")
        }
        fluxdb_app::ConfigProposalKind::ModelDelete { .. } => {
            tr("app.ai.config_proposal.action_model_delete")
        }
        fluxdb_app::ConfigProposalKind::AgentUpdate { .. } => {
            tr("app.ai.config_proposal.action_agent_update")
        }
        fluxdb_app::ConfigProposalKind::AgentDelete { .. } => {
            tr("app.ai.config_proposal.action_agent_delete")
        }
        fluxdb_app::ConfigProposalKind::ToolSetEnabled { .. } => {
            tr("app.ai.config_proposal.action_tool")
        }
        fluxdb_app::ConfigProposalKind::AgentCreate(_) => tr("app.ai.config_proposal.action_agent"),
        fluxdb_app::ConfigProposalKind::SessionArchive { .. } => {
            tr("app.ai.config_proposal.action_archive")
        }
        fluxdb_app::ConfigProposalKind::SessionPurge { .. } => {
            tr("app.ai.config_proposal.action_purge")
        }
        fluxdb_app::ConfigProposalKind::ModelCreate(_) => tr("app.ai.config_proposal.action_model"),
        fluxdb_app::ConfigProposalKind::McpServerSetEnabled { .. } => {
            tr("app.ai.config_proposal.action_mcp")
        }
        fluxdb_app::ConfigProposalKind::McpSyncClient { .. } => {
            tr("app.ai.config_proposal.action_mcp_client")
        }
    }
}

fn config_proposal_rows(
    settings: &Settings,
    proposal: &fluxdb_app::ConfigProposal,
) -> Vec<(String, String, String)> {
    match &proposal.kind {
        fluxdb_app::ConfigProposalKind::SettingUpdate(prefill) => {
            fluxdb_app::settings_prefill_diff(settings, prefill)
        }
        fluxdb_app::ConfigProposalKind::ModelUpdate { id, changes }
        | fluxdb_app::ConfigProposalKind::AgentUpdate { id, changes } => {
            let mut rows = vec![("id".into(), String::new(), id.clone())];
            rows.extend(changes.iter().map(|(field, value)| {
                (
                    field.clone(),
                    String::new(),
                    value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string()),
                )
            }));
            rows
        }
        fluxdb_app::ConfigProposalKind::ModelDelete { id }
        | fluxdb_app::ConfigProposalKind::AgentDelete { id } => {
            vec![("id".into(), String::new(), id.clone())]
        }
        fluxdb_app::ConfigProposalKind::ToolSetEnabled { tool_id, enabled } => vec![
            ("tool_id".into(), String::new(), tool_id.clone()),
            ("enabled".into(), String::new(), enabled.to_string()),
        ],
        fluxdb_app::ConfigProposalKind::AgentCreate(agent) => vec![
            (
                "display_name".into(),
                String::new(),
                agent.display_name.clone(),
            ),
            (
                "model_profile_id".into(),
                String::new(),
                agent.model_profile_id.clone().unwrap_or_default(),
            ),
            ("enabled".into(), String::new(), agent.enabled.to_string()),
            (
                "system_prompt".into(),
                String::new(),
                agent.system_prompt.clone(),
            ),
            (
                "description".into(),
                String::new(),
                agent.description.clone(),
            ),
            (
                "allowed_tools".into(),
                String::new(),
                if agent.allowed_tools.is_empty() {
                    tr("app.ai.config_proposal.unrestricted").to_string()
                } else {
                    agent.allowed_tools.join(", ")
                },
            ),
        ],
        fluxdb_app::ConfigProposalKind::SessionArchive { id } => vec![
            ("session_id".into(), String::new(), id.clone()),
            ("action".into(), String::new(), "archive".into()),
        ],
        fluxdb_app::ConfigProposalKind::SessionPurge { id } => vec![
            ("session_id".into(), String::new(), id.clone()),
            ("action".into(), String::new(), "purge".into()),
        ],
        fluxdb_app::ConfigProposalKind::ModelCreate(model) => vec![
            (
                "display_name".into(),
                String::new(),
                model.display_name.clone(),
            ),
            ("provider".into(), String::new(), model.provider.clone()),
            ("model".into(), String::new(), model.model.clone()),
            (
                "endpoint".into(),
                String::new(),
                model.endpoint.clone().unwrap_or_default(),
            ),
            ("enabled".into(), String::new(), model.enabled.to_string()),
            ("egress".into(), String::new(), model.egress.to_string()),
            (
                "context_window_tokens".into(),
                String::new(),
                model
                    .context_window_tokens
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            (
                "max_output_tokens".into(),
                String::new(),
                model
                    .max_output_tokens
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            (
                "input_price_micros".into(),
                String::new(),
                model
                    .input_price_micros
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            (
                "output_price_micros".into(),
                String::new(),
                model
                    .output_price_micros
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
            (
                "cached_input_price_micros".into(),
                String::new(),
                model
                    .cached_input_price_micros
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ),
        ],
        fluxdb_app::ConfigProposalKind::McpServerSetEnabled { id, enabled } => vec![
            ("server_id".into(), String::new(), id.clone()),
            ("enabled".into(), String::new(), enabled.to_string()),
        ],
        fluxdb_app::ConfigProposalKind::McpSyncClient {
            client_key,
            registered,
        } => vec![
            ("client".into(), String::new(), client_key.clone()),
            ("registered".into(), String::new(), registered.to_string()),
        ],
    }
}

fn display_diff_value(value: &str) -> SharedString {
    if value.is_empty() {
        SharedString::from("—")
    } else {
        SharedString::from(value.to_string())
    }
}

impl NavicatMain {
    fn cancel_config_proposal(&mut self, cx: &mut Context<Self>) {
        if self.config_proposal_busy {
            return;
        }
        self.pending_config_proposal = None;
        self.config_delete_confirmed = false;
        cx.notify();
    }

    fn confirm_config_proposal(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.config_proposal_busy {
            return;
        }
        let Some(proposal) = self.pending_config_proposal.clone() else {
            return;
        };
        if config_proposal_destructive(&proposal.kind) && !self.config_delete_confirmed {
            return;
        }
        if now_millis().saturating_sub(proposal.created_at_ms) > CONFIG_PROPOSAL_TTL_MS as i64 {
            self.pending_config_proposal = None;
            self.show_message(
                tr("app.ai.config_proposal.expired"),
                AppMessageKind::Warning,
                cx,
            );
            cx.notify();
            return;
        }
        if let fluxdb_app::ConfigProposalKind::SettingUpdate(prefill) = &proposal.kind {
            let target = match fluxdb_app::settings_after_prefill(
                &self.controller.state().settings,
                prefill,
            ) {
                Ok(target) => target,
                Err(_) => {
                    self.show_message(
                        tr("app.ai.config_proposal.invalid"),
                        AppMessageKind::Error,
                        cx,
                    );
                    return;
                }
            };
            if self.settings_editor_draft != self.controller.state().settings
                && self.settings_editor_draft != target
            {
                self.show_message(
                    tr("app.ai.config_proposal.settings_draft_dirty"),
                    AppMessageKind::Warning,
                    cx,
                );
                return;
            }
        }
        self.config_proposal_busy = true;
        let refresh_clients = matches!(
            &proposal.kind,
            fluxdb_app::ConfigProposalKind::McpSyncClient { .. }
        );
        let draft_before = self.settings_editor_draft.clone();
        let settings_before = self.controller.state().settings.clone();
        let binary_path = self.mcp_binary_path();
        let mut controller = self.controller.clone();
        cx.notify();
        cx.spawn_in(_window, async move |view, cx| {
            let (event, controller, refresh_error) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::ApplyConfirmedConfig {
                        proposal: Box::new(proposal),
                        binary_path,
                    });
                    let refresh_error = if matches!(event, AppEvent::ConfigProposalApplied(_)) {
                        match controller.dispatch(AppCommand::RefreshAiSettings) {
                            AppEvent::AiSettingsError(error) => Some(error),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    (event, controller, refresh_error)
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.config_proposal_busy = false;
                this.pending_config_proposal = None;
                this.config_delete_confirmed = false;
                match event {
                    AppEvent::ConfigProposalApplied(outcome) => {
                        if refresh_error.is_none() {
                            this.controller.merge_ai_settings_from(&controller);
                        }
                        match outcome {
                            fluxdb_app::ConfigApplyOutcome::ModelEditor { id, draft } => {
                                if this.ai_model_dialog.is_some() {
                                    this.show_message(
                                        tr("app.ai.config_proposal.dialog_busy"),
                                        AppMessageKind::Warning,
                                        cx,
                                    );
                                } else if id.is_some() && refresh_error.is_some() {
                                    this.show_message(
                                        tr("app.ai.config_proposal.refresh_failed"),
                                        AppMessageKind::Warning,
                                        cx,
                                    );
                                } else {
                                    this.open_ai_model_dialog_with_draft(
                                        id,
                                        Some(&draft),
                                        window,
                                        cx,
                                    );
                                    this.show_message(
                                        tr("app.ai.config_proposal.model_prefilled"),
                                        AppMessageKind::Info,
                                        cx,
                                    );
                                }
                            }
                            fluxdb_app::ConfigApplyOutcome::Settings(settings) => {
                                let settings = *settings;
                                // 写盘成功后才发布 live Settings；用户并发编辑的草稿不覆盖。
                                this.controller
                                    .dispatch(AppCommand::SaveSettings(settings.clone()));
                                if this.settings_editor_draft == draft_before {
                                    this.settings_editor_draft = settings.clone();
                                }
                                this.apply_saved_settings_runtime(&settings_before, cx);
                                this.show_message(
                                    tr("app.ai.config_proposal.applied"),
                                    AppMessageKind::Success,
                                    cx,
                                );
                            }
                            fluxdb_app::ConfigApplyOutcome::Applied => {
                                if refresh_clients {
                                    this.refresh_mcp_install_status(cx);
                                }
                                this.show_message(
                                    tr("app.ai.config_proposal.applied"),
                                    AppMessageKind::Success,
                                    cx,
                                );
                            }
                        }
                        if refresh_error.is_some() {
                            this.show_message(
                                tr("app.ai.config_proposal.refresh_failed"),
                                AppMessageKind::Warning,
                                cx,
                            );
                        }
                    }
                    AppEvent::AiSettingsError(message) => {
                        let error = localize_backend_error_text(&message);
                        this.show_message(
                            tr("app.ai.config_proposal.failed").replace("%{error}", &error),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    _ => this.show_message(
                        tr("app.ai.config_proposal.invalid"),
                        AppMessageKind::Error,
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn config_proposal_destructive(kind: &fluxdb_app::ConfigProposalKind) -> bool {
    matches!(
        kind,
        fluxdb_app::ConfigProposalKind::ModelDelete { .. }
            | fluxdb_app::ConfigProposalKind::AgentDelete { .. }
            | fluxdb_app::ConfigProposalKind::SessionPurge { .. }
            | fluxdb_app::ConfigProposalKind::McpSyncClient { .. }
    )
}

fn config_field_label(field: &str) -> SharedString {
    match field {
        "display_name" => tr("app.ai.config_proposal.field_display_name"),
        "enabled" => tr("app.ai.config_proposal.field_enabled"),
        "system_prompt" => tr("app.ai.config_proposal.field_system_prompt"),
        "description" => tr("app.ai.config_proposal.field_description"),
        "allowed_tools" => tr("app.ai.config_proposal.field_allowed_tools"),
        "model_profile_id" => tr("app.ai.config_proposal.field_model_profile_id"),
        "provider" => tr("app.ai.config_proposal.field_provider"),
        "model" => tr("app.ai.config_proposal.field_model"),
        "endpoint" => tr("app.ai.config_proposal.field_endpoint"),
        "egress" => tr("app.ai.config_proposal.field_egress"),
        "editor_font_size" => tr("app.ai.config_proposal.field_editor_font_size"),
        "editor_line_height" => tr("app.ai.config_proposal.field_editor_line_height"),
        "editor_font_family" => tr("app.ai.config_proposal.field_editor_font_family"),
        "editor_tab_width" => tr("app.ai.config_proposal.field_editor_tab_width"),
        "editor_word_wrap" => tr("app.ai.config_proposal.field_editor_word_wrap"),
        "page_size" => tr("app.ai.config_proposal.field_page_size"),
        "data_table_page_size" => tr("app.ai.config_proposal.field_data_table_page_size"),
        "results_placement" => tr("app.ai.config_proposal.field_results_placement"),
        "theme" => tr("app.ai.config_proposal.field_theme"),
        "id" => tr("app.ai.config_proposal.field_id"),
        "tool_id" => tr("app.ai.config_proposal.field_tool_id"),
        "session_id" => tr("app.ai.config_proposal.field_session_id"),
        "server_id" => tr("app.ai.config_proposal.field_server_id"),
        "client" => tr("app.ai.config_proposal.field_client"),
        "registered" => tr("app.ai.config_proposal.field_registered"),
        "action" => tr("app.ai.config_proposal.field_action"),
        "context_window_tokens" => tr("app.ai.config_proposal.field_context_window_tokens"),
        "max_output_tokens" => tr("app.ai.config_proposal.field_max_output_tokens"),
        "input_price_micros" => tr("app.ai.config_proposal.field_input_price_micros"),
        "output_price_micros" => tr("app.ai.config_proposal.field_output_price_micros"),
        "cached_input_price_micros" => tr("app.ai.config_proposal.field_cached_input_price_micros"),
        "endpoint_is_full_url" => tr("app.ai.config_proposal.field_endpoint_is_full_url"),
        "compression_trigger_percent" => {
            tr("app.ai.config_proposal.field_compression_trigger_percent")
        }
        "max_iterations" => tr("app.ai.config_proposal.field_max_iterations"),
        "show_shadows" => tr("app.ai.config_proposal.field_show_shadows"),
        "focus_ring" => tr("app.ai.config_proposal.field_focus_ring"),
        "scrollbar_mode" => tr("app.ai.config_proposal.field_scrollbar_mode"),
        "ui_density" => tr("app.ai.config_proposal.field_ui_density"),
        "show_status_bar" => tr("app.ai.config_proposal.field_show_status_bar"),
        "reduce_motion" => tr("app.ai.config_proposal.field_reduce_motion"),
        "global_font_family" => tr("app.ai.config_proposal.field_global_font_family"),
        "button_radius" => tr("app.ai.config_proposal.field_button_radius"),
        "large_radius" => tr("app.ai.config_proposal.field_large_radius"),
        "performance_diagnostics" => tr("app.ai.config_proposal.field_performance_diagnostics"),
        "log_level" => tr("app.ai.config_proposal.field_log_level"),
        _ => field.to_string().into(),
    }
}
