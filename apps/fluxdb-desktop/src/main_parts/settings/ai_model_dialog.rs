// AI 设置职责拆分；由 ai.rs include! 引入，保持原有作用域。

/// 读取输入框文本（空串视为缺省）。
fn ai_input_text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().trim().to_string()
}

/// 把「美元 / 百万 Token」换算成 micro-USD（1 USD = 1_000_000 micro）。
fn ai_input_price_micros(input: &Entity<InputState>, cx: &App) -> Result<Option<i64>, String> {
    let raw = ai_input_text(input, cx);
    if raw.is_empty() {
        return Ok(None);
    }
    let value: f64 = raw
        .parse()
        .map_err(|_| format!("{} {raw}", tr("app.ai.model_dialog.invalid_price")))?;
    if !value.is_finite() || value < 0.0 {
        return Err(tr("app.ai.model_dialog.negative_price").to_string());
    }
    Ok(Some((value * 1_000_000.0).round() as i64))
}

/// 模型表单的一个「标签 + 输入」行（无说明文字，视觉以交互稿为准）。
fn ai_field(
    label: impl Into<SharedString>,
    input: &Entity<InputState>,
    colors: UiColors,
) -> impl IntoElement {
    let label = label.into();
    let debug_label = label.clone();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(label),
        )
        .child(
            div()
                .debug_selector(move || format!("ai-field:{debug_label}"))
                .child(Input::new(input).text_size(ui_text_size(13.))),
        )
}

/// API Key 字段：默认掩码 + 组件内置眼睛按钮。
fn ai_secret_field(
    label: impl Into<SharedString>,
    input: &Entity<InputState>,
    colors: UiColors,
) -> impl IntoElement {
    let label = label.into();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(label),
        )
        // mask_toggle 必须加在渲染的 Input 上（组件级开关），只设 state 不会出现眼睛。
        .child(
            div()
                .debug_selector(|| "ai-secret-input".into())
                .child(Input::new(input).mask_toggle().text_size(ui_text_size(13.))),
        )
}

/// Provider 下拉字段。
fn ai_select_field(
    label: &str,
    select: &Entity<SelectState<SearchableVec<String>>>,
    colors: UiColors,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(label.to_string()),
        )
        .child(Select::new(select).w_full().h(px(32.)))
}

use fluxdb_app::AiModelSaveInput;

/// 「模型 ID」字段：可输入，也可点「获取模型列表」拉取后从浮动面板选择。
///
/// 拉取需要 API Key（因此本字段排在 API Key 之后）。候选是输入框下方的浮层：
/// 顶部搜索、按模型 ID 前缀分组、悬停高亮、限高滚动。
/// 浮层不参与表单布局，避免模型数量把弹框持续撑高；超出弹框内容区的部分由弹框裁切。
fn ai_model_id_field(
    dialog: &AiModelDialog,
    filter_input: &Entity<InputState>,
    filter_text: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let fetching = dialog.fetching_models;
    let mut col = div()
        .relative()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(tr("app.ai.model_dialog.model_id")),
                )
                .child(
                    Button::new("ai-model-fetch")
                        .debug_selector(|| "ai-model-fetch".into())
                        .small()
                        .ghost()
                        .disabled(fetching || dialog.endpoint_is_full_url)
                        .child(if fetching {
                            tr("app.ai.model_dialog.fetching")
                        } else {
                            tr("app.ai.model_dialog.fetch_models")
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.fetch_ai_model_list(window, cx);
                        })),
                ),
        )
        .child(
            div()
                .debug_selector(|| "ai-model-id-input".into())
                .child(Input::new(&dialog.inputs.model_key).text_size(ui_text_size(13.))),
        );
    if dialog.model_options_open && !dialog.model_options.is_empty() {
        // 绝对定位覆盖在后续字段上，不参与弹框高度计算；deferred 保证浮层绘制在其上方。
        let options = ai_model_options_panel(dialog, filter_input, filter_text, colors, cx)
            .debug_selector(|| "ai-model-options-overlay".into())
            .occlude()
            .max_h(px(240.));
        col = col.child(
            div()
                .absolute()
                .top(px(54.))
                .left_0()
                .right_0()
                .child(deferred(options)),
        );
    }
    col
}

/// 模型候选浮层：搜索框 + 分组列表。
fn ai_model_options_panel(
    dialog: &AiModelDialog,
    filter_input: &Entity<InputState>,
    filter_text: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let filter = filter_text.trim().to_lowercase();
    // 按「模型 ID 前缀」分组：`inclusionai/ling-3.1-flash` → inclusionai。
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for model in &dialog.model_options {
        if !filter.is_empty() && !model.to_lowercase().contains(&filter) {
            continue;
        }
        let group = model
            .split_once('/')
            .map(|(prefix, _)| prefix.to_string())
            .unwrap_or_default();
        match groups.iter_mut().find(|(key, _)| key == &group) {
            Some((_, items)) => items.push(model.clone()),
            None => groups.push((group, vec![model.clone()])),
        }
    }

    let mut list = div().flex().flex_col().py_1();
    if groups.is_empty() {
        list = list.child(
            div()
                .px_3()
                .py_2()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.ai.model_dialog.no_model_match")),
        );
    }
    // 只有存在带前缀分组时才给无前缀项加「其他」标题，避免平铺列表多一个多余标题。
    let show_plain_header = groups.iter().any(|(key, _)| !key.is_empty());
    for (group, items) in &groups {
        if !group.is_empty() || show_plain_header {
            list = list.child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(if group.is_empty() {
                        tr("app.ai.model_dialog.other").to_string()
                    } else {
                        group.clone()
                    }),
            );
        }
        for model in items {
            let model_owned = model.clone();
            let selected = dialog.inputs.model_key.read(cx).value().trim() == model_owned.as_str();
            list = list.child(
                div()
                    .id(SharedString::from(format!("ai-model-opt:{model}")))
                    .px_3()
                    .py_2()
                    .mx_1()
                    .rounded(colors.radius)
                    .cursor_pointer()
                    .bg(if selected {
                        colors.hover
                    } else {
                        colors.panel_bg.alpha(0.0)
                    })
                    .hover(move |style| style.bg(colors.hover))
                    .text_size(ui_text_size(12.5))
                    .text_color(colors.text)
                    .child(model.clone())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if let Some(dialog) = this.ai_model_dialog.as_mut() {
                                dialog.inputs.model_key.update(cx, |state, cx| {
                                    state.set_value(model_owned.clone(), window, cx);
                                });
                                dialog.model_options_open = false;
                            }
                            this.apply_ai_model_capacity(window, cx);
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
    }

    div()
        .mt_1()
        .w_full()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        // 面板内点击（含搜索框）不外传给弹框根，避免误关浮层。
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .shadow(vec![box_shadow(
            px(0.),
            px(8.),
            px(24.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.34 } else { 0.14 }),
        )])
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_2()
                .text_color(colors.muted)
                .child(app_icon(AppIcon::Search, 14., colors.muted))
                .child(
                    Input::new(filter_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .text_size(ui_text_size(12.5)),
                ),
        )
        .child(
            div()
                .border_t_1()
                .border_color(colors.border_soft)
                .max_h(px(168.))
                .overflow_y_scrollbar()
                .child(list),
        )
}

/// 模型新建/编辑弹框；未打开时不渲染。/// 模型新建/编辑弹框；未打开时不渲染。
fn ai_model_dialog(
    dialog: Option<AiModelDialog>,
    filter_input: Entity<InputState>,
    filter_text: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Option<gpui::AnyElement> {
    let dialog = dialog.as_ref()?;
    let inputs = &dialog.inputs;
    let is_new = dialog.model_id.is_none();
    let egress = dialog.egress;
    let this = cx.entity().downgrade();

    let mask = div()
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
            cx.listener(|this, _, _, cx| {
                this.ai_model_dialog = None;
                cx.notify();
                cx.stop_propagation();
            }),
        );

    let panel = div()
        .w(px(560.))
        .max_h(px(720.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(18.),
            px(48.),
            px(0.),
            hsla(220. / 360., 0.3, 0.1, 0.28),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        // 内部点击不穿透到遮罩；同时关闭模型候选浮层（点空白处收起）。
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if let Some(dialog) = this.ai_model_dialog.as_mut() {
                    dialog.model_options_open = false;
                }
                cx.stop_propagation();
            }),
        )
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            // Esc 等同取消，不写入任何数据。
            this.ai_model_dialog = None;
            cx.notify();
            cx.stop_propagation();
        }))
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
                        .text_size(ui_text_size(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(if is_new {
                            tr("app.ai.models.new")
                        } else {
                            tr("app.ai.model_dialog.edit_title")
                        }),
                )
                .child(
                    Button::new("ai-model-close")
                        .small()
                        .ghost()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ai_model_dialog = None;
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex_1()
                .overflow_y_scrollbar()
                .px_5()
                .pb_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(ai_field(
                    &tr("app.ai.model_dialog.display_name"),
                    &inputs.display_name,
                    colors,
                ))
                .child(ai_select_field("Provider", &inputs.provider, colors))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(div().text_size(ui_text_size(11.)).text_color(colors.text).child(
                                    if dialog.endpoint_is_full_url {
                                        tr("app.ai.model_dialog.full_url_label")
                                    } else {
                                        tr("app.ai.model_dialog.base_url_label")
                                    },
                                ))
                                .child(
                                    div().debug_selector(|| "ai-model-full-url".into()).child(
                                        Switch::new("ai-model-full-url")
                                            .small()
                                            .label(tr("app.ai.model_dialog.full_url"))
                                            .checked(dialog.endpoint_is_full_url)
                                            .disabled(dialog.fetching_models)
                                            .on_click(cx.listener(|this, checked, _, cx| {
                                                if let Some(dialog) = this.ai_model_dialog.as_mut()
                                                {
                                                    dialog.endpoint_is_full_url = *checked;
                                                    dialog.model_options_open = false;
                                                }
                                                cx.notify();
                                            })),
                                    ),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "ai-model-endpoint-input".into())
                                .child(Input::new(&inputs.endpoint).text_size(ui_text_size(13.))),
                        )
                        .child(div().text_size(ui_text_size(11.)).text_color(colors.muted).child(
                            if dialog.endpoint_is_full_url {
                                tr("app.ai.model_dialog.full_url_help")
                            } else {
                                tr("app.ai.model_dialog.base_url_help")
                            },
                        )),
                )
                // 模型 ID 依赖 API Key 才能拉取，因此排在 API Key 之后。
                .child(ai_secret_field("API Key", &inputs.api_key, colors))
                .child(ai_model_id_field(
                    dialog,
                    &filter_input,
                    &filter_text,
                    colors,
                    cx,
                ))
                // 数据出站许可（设计 §29/§31.1）：默认允许；关闭后该模型不再接收会话数据。
                .child(
                    div()
                        .debug_selector(|| "ai-model-egress".into())
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .text_color(colors.text)
                                        .child(tr("app.ai.model_dialog.egress")),
                                )
                                .child(
                                    div()
                                        .text_size(ui_text_size(11.))
                                        .text_color(colors.muted)
                                        .child(
                                            tr("app.ai.model_dialog.egress_help"),
                                        ),
                                ),
                        )
                        .child(
                            div().debug_selector(|| "ai-model-egress-switch".into()).child(
                                Switch::new("ai-model-egress-permission")
                                    .small()
                                    .checked(egress)
                                    .on_click(cx.listener(|this, checked, _, cx| {
                                        if let Some(dialog) = this.ai_model_dialog.as_mut() {
                                            dialog.egress = *checked;
                                        }
                                        cx.notify();
                                    })),
                            ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(div().flex_1().min_w(px(0.)).child(ai_capacity_field(
                            tr("app.ai.model_dialog.context_window"),
                            &inputs.context_window,
                            &inputs.context_unit,
                            colors,
                        )))
                        .child(div().flex_1().min_w(px(0.)).child(ai_capacity_field(
                            tr("app.ai.model_dialog.max_output"),
                            &inputs.max_output,
                            &inputs.output_unit,
                            colors,
                        ))),
                )
                // 计价三项（输入 / 输出 / 缓存读取）固定同一行，避免换行破坏栅格。
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(div().flex_1().min_w(px(0.)).child(ai_field(
                            tr("app.ai.model_dialog.price_input"),
                            &inputs.price_input,
                            colors,
                        )))
                        .child(div().flex_1().min_w(px(0.)).child(ai_field(
                            tr("app.ai.model_dialog.price_output"),
                            &inputs.price_output,
                            colors,
                        )))
                        .child(div().flex_1().min_w(px(0.)).child(ai_field(
                            tr("app.ai.model_dialog.price_cache"),
                            &inputs.price_cache,
                            colors,
                        ))),
                ),
        );

    let footer = div()
        .border_t_1()
        .border_color(colors.border_soft)
        .px_5()
        .py_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("ai-model-cancel")
                        .debug_selector(|| "ai-model-cancel".into())
                        .small()
                        .ghost()
                        .child(tr("app.common.cancel"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ai_model_dialog = None;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("ai-model-save")
                        .debug_selector(|| "ai-model-save".into())
                        .small()
                        .primary()
                        .child(tr("app.ai.model_dialog.save"))
                        .on_click(move |_, _, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.submit_ai_model_dialog(cx);
                            });
                        }),
                ),
        );

    // 内联错误区：校验失败或保存失败（含 CAS 冲突）时显示，弹框保持打开、草稿保留。
    let error_line = div().when_some(dialog.validation_error.clone(), |this, message| {
        this.px_5()
            .pb_3()
            .flex()
            .items_center()
            .gap_2()
            .child(app_icon(AppIcon::CircleSlash, 14., colors.danger))
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.danger)
                    .child(message),
            )
    });

    Some(
        mask.child(panel.child(error_line).child(footer))
            .into_any_element(),
    )
}
