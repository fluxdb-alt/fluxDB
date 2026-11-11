// 服务器面板的终止/维护二次确认弹框。

fn server_kill_modal(
    confirm: ServerKillConfirm,
    panel: &ServerPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let (title, message, ack) = match confirm.kind {
        ServerKillKind::Query => (
            tr("app.server_panel.kill.query_title").to_string(),
            tr("app.server_panel.kill.query_message")
                .replace("%{id}", &confirm.session_id),
            tr("app.server_panel.kill.ack_query").to_string(),
        ),
        ServerKillKind::Session => (
            tr("app.server_panel.kill.session_title").to_string(),
            tr("app.server_panel.kill.session_message")
                .replace("%{id}", &confirm.session_id),
            tr("app.server_panel.kill.ack_session").to_string(),
        ),
    };
    let sql = panel
        .activity
        .as_ref()
        .and_then(|activity| {
            activity
                .sessions
                .iter()
                .find(|session| session.id == confirm.session_id)
        })
        .map(|session| server_sql_preview(&session.sql, 200))
        .unwrap_or_default();
    let checked = confirm.checked;
    server_modal_shell(
        "server-panel-kill-modal",
        colors,
        div()
            .w(px(460.))
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
                                    this.server_kill_confirm = None;
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .px_5()
                    .pb_3()
                    .text_size(ui_text_size(13.))
                    .text_color(colors.muted)
                    .child(message),
            )
            .when(!sql.is_empty(), |this| {
                this.child(
                    div()
                        .mx_5()
                        .mb_3()
                        .max_h(px(120.))
                        .overflow_y_scrollbar()
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.panel_alt)
                        .p_2()
                        .text_size(ui_text_size(12.))
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_color(colors.text)
                        .child(sql),
                )
            })
            .child(
                div()
                    .mx_5()
                    .mb_3()
                    .child(server_check_row(ack, checked, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if let Some(confirm) = this.server_kill_confirm.as_mut() {
                                confirm.checked = !confirm.checked;
                            }
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    )),
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
                    .child(server_dialog_button(
                        tr("app.server_panel.kill.cancel").to_string(),
                        false,
                        true,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.server_kill_confirm = None;
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ))
                    .child(server_dialog_button(
                        tr("app.server_panel.kill.confirm").to_string(),
                        true,
                        checked,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.confirm_server_kill(cx);
                            cx.stop_propagation();
                        }),
                    )),
            ),
    )
}

fn server_maintenance_modal(
    confirm: ServerMaintenanceConfirm,
    panel: &ServerPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let database = if panel.database.trim().is_empty() {
        tr("app.server_panel.no_database").to_string()
    } else {
        panel.database.clone()
    };
    let message = tr("app.server_panel.maintenance.message")
        .replace("%{database}", &database)
        .replace("%{action}", confirm.action.as_str());
    server_modal_shell(
        "server-panel-maintenance-modal",
        colors,
        div()
            .w(px(430.))
            .child(
                div()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .text_size(ui_text_size(17.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(tr("app.server_panel.maintenance.title")),
            )
            .child(
                div()
                    .px_5()
                    .pb_4()
                    .text_size(ui_text_size(13.))
                    .text_color(colors.muted)
                    .child(message),
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
                    .child(server_dialog_button(
                        tr("app.server_panel.kill.cancel").to_string(),
                        false,
                        true,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.server_maintenance_confirm = None;
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ))
                    .child(server_dialog_button(
                        tr("app.server_panel.maintenance.confirm").to_string(),
                        true,
                        true,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.confirm_server_maintenance(cx);
                            cx.stop_propagation();
                        }),
                    )),
            ),
    )
}

fn server_modal_shell(id: &'static str, colors: UiColors, card: Div) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
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
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
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
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(card),
        )
}
