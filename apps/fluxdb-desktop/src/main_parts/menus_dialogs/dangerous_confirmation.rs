#[derive(Clone, Copy)]
enum DangerousCommandKind {
    Sql,
    Redis,
}

// 共用既有按 pending 状态挂载的弹层；保留主视图焦点与快捷键上下文，不迁移弹框生命周期。
fn dangerous_command_modal(
    kind: DangerousCommandKind,
    text: String,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let preview = single_line_summary_text(text);
    let (title, description, key_context, cancel_id, confirm_id) = match kind {
        DangerousCommandKind::Sql => (
            tr("app.confirm.dangerous_sql.title"),
            tr("app.confirm.dangerous_sql.message"),
            "DangerousQueryModal",
            "dangerous-query-cancel",
            "dangerous-query-confirm",
        ),
        DangerousCommandKind::Redis => (
            tr("app.confirm.dangerous_redis.title"),
            tr("app.confirm.dangerous_redis.message"),
            "DangerousRedisCommandModal",
            "dangerous-redis-command-cancel",
            "dangerous-redis-command-confirm",
        ),
    };
    let cancel: fn(&mut NavicatMain, &mut Context<NavicatMain>) = match kind {
        DangerousCommandKind::Sql => NavicatMain::cancel_dangerous_query,
        DangerousCommandKind::Redis => NavicatMain::cancel_dangerous_redis_command,
    };
    let confirm: fn(&mut NavicatMain, &mut Context<NavicatMain>) = match kind {
        DangerousCommandKind::Sql => NavicatMain::confirm_dangerous_query,
        DangerousCommandKind::Redis => NavicatMain::confirm_dangerous_redis_command,
    };
    div()
        .id(key_context)
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
                cancel(this, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .id((key_context, 0u64))
                .w(px(460.))
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
                .key_context(key_context)
                .on_action(cx.listener(move |this, _: &CancelDialog, _, cx| {
                    cancel(this, cx);
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
                                .child(title),
                        )
                        .child(
                            div()
                                .id((key_context, 1u64))
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
                                        cancel(this, cx);
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
                        .gap_2()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(description)
                        .child(preview),
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
                        .child(Button::new(cancel_id).label(tr("app.common.cancel")).w(px(78.)).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cancel(this, cx);
                                cx.stop_propagation();
                            }),
                        ))
                        .child(
                            Button::new(confirm_id)
                                .label(tr("app.common.execute"))
                                .danger()
                                .w(px(78.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    confirm(this, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

