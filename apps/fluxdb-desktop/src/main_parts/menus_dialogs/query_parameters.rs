fn query_parameter_prompt_modal(
    pending: PendingQueryParameterPrompt,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    query_parameter_modal_backdrop(colors, cx).child(
        query_parameter_modal_panel(focus_handle, colors, cx)
            .child(query_parameter_modal_header("运行参数", colors, cx))
            .child(query_parameter_mode_tabs(pending.active_mode, colors, cx))
            .child(query_parameter_modal_body(pending, colors))
            .child(query_parameter_modal_footer(colors, cx)),
    )
}

fn query_parameter_mode_tabs(
    active_mode: QueryParameterInputMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .px_5()
        .pb_4()
        .flex_none()
        .child(
            div()
                .h(px(38.))
                .flex()
                .items_end()
                .gap_5()
                .child(query_parameter_mode_tab(
                    QueryParameterInputMode::Fields,
                    "逐个输入",
                    active_mode,
                    colors,
                    cx,
                ))
                .child(query_parameter_mode_tab(
                    QueryParameterInputMode::Array,
                    "数组输入",
                    active_mode,
                    colors,
                    cx,
                )),
        )
}

fn query_parameter_mode_tab(
    mode: QueryParameterInputMode,
    label: &'static str,
    active_mode: QueryParameterInputMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected = mode == active_mode;
    div()
        .h_full()
        .px_1()
        .pb_1()
        .border_b_2()
        .border_color(if selected { colors.text } else { rgba_with_alpha(colors.text, 0.) })
        .cursor_pointer()
        .flex()
        .items_center()
        .text_size(px(14.))
        .font_weight(if selected {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::MEDIUM
        })
        .text_color(if selected { colors.text } else { colors.muted })
        .hover(move |style| style.text_color(colors.text))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.switch_query_parameter_input_mode(mode, window, cx);
                cx.stop_propagation();
            }),
        )
}

fn query_parameter_modal_body(pending: PendingQueryParameterPrompt, colors: UiColors) -> Div {
    match pending.active_mode {
        QueryParameterInputMode::Fields => query_parameter_fields_body(pending.parameters, colors),
        QueryParameterInputMode::Array => {
            query_parameter_array_body(pending.bulk_input, pending.parameters, colors)
        }
    }
}

fn query_parameter_fields_body(parameters: Vec<QueryParameterInput>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .size_full()
                .overflow_y_scrollbar()
                .px_5()
                .pb_5()
                .flex()
                .flex_col()
                .gap_3()
                .children(
                    parameters
                        .into_iter()
                        .map(|parameter| query_parameter_input_row(parameter, colors)),
                ),
        )
}

fn query_parameter_array_body(
    bulk_input: Entity<InputState>,
    _parameters: Vec<QueryParameterInput>,
    colors: UiColors,
) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .px_5()
        .pb_5()
        .child(
            div()
                .h(px(220.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_hidden()
                .child(
                    Input::new(&bulk_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(13.)),
                ),
        )
}

fn query_parameter_input_row(parameter: QueryParameterInput, colors: UiColors) -> Div {
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(86.))
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(format!("{}：", parameter.label)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(34.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .px_3()
                .child(
                    Input::new(&parameter.input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(14.)),
                ),
        )
}

fn query_parameter_modal_footer(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .flex_none()
        .h(px(60.))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_5()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .child(
            Button::new("query-parameters-cancel")
                .label("取消")
                .w(px(78.))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_query_parameter_prompt(cx);
                    cx.stop_propagation();
                })),
        )
        .child(
            Button::new("query-parameters-apply")
                .label("应用并运行")
                .primary()
                .w(px(112.))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.confirm_query_parameter_prompt(window, cx);
                    cx.stop_propagation();
                })),
        )
}

fn query_parameter_modal_backdrop(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
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
                this.cancel_query_parameter_prompt(cx);
                cx.stop_propagation();
            }),
        )
}

fn query_parameter_modal_panel(
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w(px(460.))
        .max_h(px(520.))
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
        .flex()
        .flex_col()
        .track_focus(&focus_handle)
        .key_context("QueryParameterModal")
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            this.cancel_query_parameter_prompt(cx);
            cx.stop_propagation();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn query_parameter_modal_header(
    title: &'static str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_none()
        .px_5()
        .pt_4()
        .pb_3()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(17.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
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
                    cx.listener(|this, _, _, cx| {
                        this.cancel_query_parameter_prompt(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}
