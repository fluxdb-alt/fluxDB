// AI 设置职责拆分；由 ai.rs include! 引入，保持原有作用域。

/// 表头：模型 / 能力与上下文 / 凭据 / 默认 / 启用 / 操作（对齐六列宽度）。
fn ai_model_table_header(colors: UiColors) -> impl IntoElement {
    let head = move |label: &str, w: f32| {
        div()
            .w(px(w))
            .flex_shrink_0()
            .debug_selector(|| format!("ai-head:{label}"))
            .text_size(ui_text_size(11.))
            .text_color(colors.muted)
            .child(label.to_string())
    };
    div()
        .border_t_1()
        .border_color(colors.border_soft)
        .px_4()
        .py_2()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.ai.models.header.model")),
        )
        .child(head(&tr("app.ai.models.header.capability"), 150.))
        .child(head(&tr("app.ai.models.header.credential"), 80.))
        .child(head(&tr("app.ai.models.header.default"), 56.))
        .child(head(&tr("app.ai.models.header.enabled"), 48.))
        .child(head(&tr("app.ai.models.header.actions"), 148.))
}

/// 大模型列表行：对照交互稿六列（模型 / 能力与上下文 / 凭据 / 默认 / 启用 / 操作）。
///
/// 价格只在编辑弹框里填写与回读，不参与列表展示、标签推导或任何用量统计。
fn ai_model_row(
    model: &fluxdb_app::AiModelItem,
    is_default: bool,
    testing: bool,
    deleting: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let this = cx.entity().downgrade();
    let id = model.id.clone();
    let edit_id = model.id.clone();
    let test_id = model.id.clone();
    let delete_id = model.id.clone();
    let default_id = model.id.clone();
    let delete_name = model.display_name.clone();
    let enabled = model.enabled;
    let revision = model.revision;
    let decision = model.kind == fluxdb_app::AiModelKind::Decision;
    // 「默认」单选在渲染闭包里派发命令，这里单独克隆一份弱引用。
    let default_owner = cx.entity().downgrade();

    let tokens = |v: Option<i64>| match v {
        Some(n) if n >= 1000 => format!("{}k", n / 1000),
        Some(n) => n.to_string(),
        None => "—".into(),
    };
    let capability_note = if decision {
        tr("app.ai.models.decision_note").to_string()
    } else {
        // 能力标签只由模型上下文窗口推导（app 层 `model_capability_labels`），
        // 价格不参与标签或列表展示。
        let long_context = if model
            .capability_labels
            .iter()
            .any(|l| l == "long_context")
        {
            tr("app.ai.models.long_context_suffix").to_string()
        } else {
            String::new()
        };
        format!(
            "{} ctx / {} out{}",
            tokens(model.context_window_tokens),
            tokens(model.max_output_tokens),
            long_context
        )
    };
    let egress = model.egress;

    div()
        .border_t_1()
        .border_color(colors.border_soft)
        .px_4()
        .py_2()
        .min_h(px(60.))
        .flex()
        .items_center()
        .gap_3()
        .child(
            // 列 1：模型
            div()
                .flex_1()
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(app_icon(
                    if decision {
                        AppIcon::Workflow
                    } else {
                        AppIcon::Bot
                    },
                    16.,
                    colors.muted,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .truncate()
                                .child(model.display_name.clone()),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .truncate()
                                .child(format!(
                            "{} · {}",
                            fluxdb_app::ModelProvider::from_stored(&model.provider).label(),
                            model.model
                        )),
                        ),
                ),
        )
        .child(
            // 列 2：能力与上下文
            div()
                .w(px(150.))
                .flex_shrink_0()
                .debug_selector(|| "ai-model-capability".into())
                .flex()
                .flex_col()
                .items_start()
                .gap_1()
                .child(ai_capability_tag(&model.kind, colors))
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(capability_note),
                )
                .child(ai_egress_pill(egress, colors)),
        )
        .child(
            // 列 3：凭据
            div()
                .w(px(80.))
                .flex_shrink_0()
                .debug_selector(|| "ai-model-credential".into())
                .flex()
                .items_start()
                .child(ai_credential_tag(&model.credential_status, colors)),
        )
        .child(
            // 列 4：默认主模型（单选）
            div()
                .w(px(56.))
                .flex_shrink_0()
                .debug_selector(|| "ai-model-default".into())
                .flex()
                .items_center()
                .when(decision, |column| {
                    column.child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child("—"),
                    )
                })
                .when(!decision, |column| {
                    column.child(
                        Radio::new(SharedString::from(format!("ai-model-default:{id}")))
                            .debug_selector(|| "ai-model-default-radio".into())
                            .checked(is_default)
                            .disabled(deleting)
                            .tooltip(if is_default {
                                tr("app.ai.models.default_current")
                            } else {
                                tr("app.ai.models.default_set")
                            })
                            .on_change(move |checked, _, cx| {
                                if !*checked {
                                    return;
                                }
                                let _ = default_owner.update(cx, |this, cx| {
                                    let expected_revision =
                                        this.controller.state().ai_runtime_revision;
                                    let _ = this
                                        .controller
                                        .dispatch(AppCommand::SetAiDefaultModel {
                                            model_id: Some(default_id.clone()),
                                            expected_revision,
                                        });
                                    cx.notify();
                                });
                            }),
                    )
                }),
        )
        .child(
            // 列 5：启用
            div()
                .w(px(48.))
                .flex_shrink_0()
                .debug_selector(|| "ai-model-enable".into())
                .child(
                    Switch::new(SharedString::from(format!("ai-model:{id}")))
                        .disabled(deleting)
                        .checked(enabled)
                        .on_click(move |checked, _, cx| {
                            let _ = this.update(cx, |this, cx| {
                                let _ = this.controller.dispatch(AppCommand::SetAiModelEnabled {
                                    id: id.clone(),
                                    enabled: *checked,
                                    expected_revision: revision,
                                });
                                cx.notify();
                            });
                        }),
                ),
        )
        .child(
            // 列 6：状态和图标操作，与表头使用同一宽度。
            div()
                .w(px(148.))
                .flex_shrink_0()
                .debug_selector(|| "ai-model-actions".into())
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(ai_connection_tag(&model.connection_status, colors))
                .child(
                    Button::new(SharedString::from(format!("ai-model-test:{test_id}")))
                        .small()
                        .ghost()
                        .disabled(testing || deleting)
                        .loading(testing)
                        .cursor_pointer()
                        .icon(
                            gpui_component::Icon::default().path(app_icon_path(AppIcon::Activity)),
                        )
                        .tooltip(if testing {
                            tr("app.ai.models.testing")
                        } else {
                            tr("app.ai.models.test_connection")
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_ai_model_test(test_id.clone(), cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("ai-model-edit:{edit_id}")))
                        .debug_selector(|| "ai-model-edit".into())
                        .disabled(deleting)
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .icon(gpui_component::Icon::default().path(app_icon_path(AppIcon::Edit)))
                        .tooltip(tr("app.ai.models.edit"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_ai_model_dialog(Some(edit_id.clone()), window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("ai-model-delete:{delete_id}")))
                        .debug_selector(|| "ai-model-delete".into())
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .disabled(testing || deleting)
                        .loading(deleting)
                        .icon(
                            gpui_component::Icon::default()
                                .path(app_icon_path(AppIcon::Trash))
                                .text_color(colors.danger),
                        )
                        .tooltip(tr("app.ai.models.delete"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_ai_model_delete(
                                delete_id.clone(),
                                delete_name.clone(),
                                revision,
                                window,
                                cx,
                            );
                        })),
                ),
        )
}

/// 单模型数据出站许可的展示标签：关闭表示该模型不会接收会话数据（§29/§31.1）。
fn ai_egress_pill(enabled: bool, colors: UiColors) -> impl IntoElement {
    if enabled {
        ai_pill(tr("app.ai.models.egress_allowed"), colors.success, colors)
    } else {
        ai_pill(tr("app.ai.models.egress_blocked"), colors.danger, colors)
    }
}

/// 「大模型」分区。
fn settings_ai_models_panel(
    state: &AppState,
    panel: AiSettingsPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let filter = panel.filter.trim().to_lowercase();
    let filtered: Vec<&fluxdb_app::AiModelItem> = state
        .ai_models
        .iter()
        .filter(|m| {
            filter.is_empty()
                || m.display_name.to_lowercase().contains(&filter)
                || m.provider.to_lowercase().contains(&filter)
                || m.model.to_lowercase().contains(&filter)
                || tr(match m.kind {
                    fluxdb_app::AiModelKind::Generation => "app.ai.capability.generation",
                    fluxdb_app::AiModelKind::Decision => "app.ai.capability.decision",
                })
                .to_lowercase()
                .contains(&filter)
        })
        .collect();

    let rows: Vec<gpui::AnyElement> = if panel.settings_loading {
        vec![inline_loading_row(tr("app.ai.models.loading"), colors).into_any_element()]
    } else if let Some(message) = panel.settings_error.as_ref() {
        // 错误态必须与「空库」区分：显示原因 + 重试，而不是伪装成空列表（设计 §39.5）。
        vec![ai_settings_error_block(message, colors, cx)]
    } else if state.ai_models.is_empty() {
        vec![ai_empty_state(&tr("app.ai.models.empty"), colors)]
    } else if filtered.is_empty() {
        vec![ai_empty_state(&tr("app.ai.models.no_match"), colors)]
    } else {
        filtered
            .iter()
            .map(|m| {
                let testing = panel.testing_id.as_deref() == Some(m.id.as_str());
                ai_model_row(
                    m,
                    state.ai_default_model_id.as_deref() == Some(m.id.as_str()),
                    testing,
                    panel.deleting_id.as_deref() == Some(m.id.as_str()),
                    colors,
                    cx,
                )
                .into_any_element()
            })
            .collect()
    };

    div().flex().flex_col().gap_3().child(
        settings_panel_group(tr("app.ai.models.title"), colors)
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
                                .child(tr("app.ai.models.description")),
                        )
                        .child(
                            Button::new("ai-model-new")
                                .small()
                                .primary()
                                .child(tr("app.ai.models.new"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_ai_model_dialog(None, window, cx);
                                })),
                        ),
                )
                // 工具栏：搜索框 + 刷新。
                .child(
                    div()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .px_4()
                        .py_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(ai_model_search_field(&panel.filter_input, colors, cx))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Button::new("ai-refresh-models")
                                        .small()
                                        .ghost()
                                        .disabled(panel.settings_loading)
                                        .child(if panel.settings_loading {
                                            tr("app.ai.models.refreshing")
                                        } else {
                                            tr("app.common.refresh")
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reload_ai_settings(cx);
                                        })),
                                ),
                        ),
                )
                // 窄窗口：六列保持最小宽度并横向滚动，而不是把列压扁/裁切（T013 窄窗口验收）。
                .child(
                    div().w_full().overflow_x_scrollbar().child(
                        div()
                            .min_w(px(880.))
                            .flex()
                            .flex_col()
                            .child(ai_model_table_header(colors))
                            .child(div().children(rows)),
                    ),
                ),
        )
}

/// 大模型搜索框（对照交互稿的圆角搜索条）。
fn ai_model_search_field(
    input: &Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let input = input.clone();
    let has_value = !input.read(cx).value().is_empty();
    let input_for_clear = input.clone();
    div()
        .w(px(300.))
        .h(px(32.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .text_color(colors.muted)
        .child(app_icon(AppIcon::Search, 15., colors.muted))
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .text_size(ui_text_size(12.)),
        )
        .when(has_value, |this| {
            this.child(
                div()
                    .size(px(18.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .child(app_icon(AppIcon::Close, 12., colors.muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            input_for_clear.update(cx, |input, cx| {
                                input.set_value("", window, cx);
                            });
                            this.ai_model_filter = String::new();
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            )
        })
}
