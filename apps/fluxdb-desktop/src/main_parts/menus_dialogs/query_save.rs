fn query_save_choice_modal(
    tab_id: TabId,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    query_save_modal_backdrop(colors, cx).child(
        query_save_modal_panel(focus_handle, colors, cx)
            .child(query_save_modal_header("保存 SQL", colors, cx))
            .child(
                div()
                    .px_5()
                    .pb_4()
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child("选择保存位置。"),
            )
            .child(div().h(px(1.)).bg(colors.border_soft))
            .child(
                div()
                    .h(px(64.))
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("query-save-local").label("保存到本地文件...").on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.choose_query_save_local(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ))
                    .child(
                        Button::new("query-save-connection")
                            .label("保存到连接中...")
                            .primary()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_query_save_connection(tab_id, window, cx);
                                cx.stop_propagation();
                            })),
                    ),
            ),
        )
}

fn query_save_connection_modal(
    tab_id: TabId,
    input: Entity<InputState>,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    query_save_modal_backdrop(colors, cx).child(
        query_save_modal_panel(focus_handle, colors, cx)
            .child(query_save_modal_header("保存到连接中", colors, cx))
            .child(
                div()
                    .px_5()
                    .pb_5()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(colors.muted)
                            .child("查询名称"),
                    )
                    .child(
                        div()
                            .h(px(36.))
                            .rounded(colors.radius)
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.input_bg)
                            .px_3()
                            .child(
                                Input::new(&input)
                                    .appearance(false)
                                    .focus_bordered(false)
                                    .w_full()
                                    .h_full()
                                    .text_size(px(14.)),
                            ),
                    ),
            )
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
                        Button::new("query-save-connection-cancel")
                            .label("取消")
                            .w(px(78.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_query_save_modal(cx);
                                cx.stop_propagation();
                            })),
                    )
                    .child(
                        Button::new("query-save-connection-confirm")
                            .label("保存")
                            .primary()
                            .w(px(78.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.confirm_query_save_connection(tab_id, cx);
                                cx.stop_propagation();
                            })),
                    ),
            ),
        )
}

fn query_save_modal_backdrop(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
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
                this.cancel_query_save_modal(cx);
                cx.stop_propagation();
            }),
        )
}

fn query_save_modal_panel(
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w(px(420.))
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
        .key_context("QuerySaveModal")
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            this.cancel_query_save_modal(cx);
            cx.stop_propagation();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn query_save_modal_header(
    title: &'static str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
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
                        this.cancel_query_save_modal(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}
