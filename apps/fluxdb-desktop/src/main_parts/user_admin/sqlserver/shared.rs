fn sqlserver_section_title(text: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(text.into())
}

fn sqlserver_empty_panel(text: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .text_size(ui_text_size(13.))
        .text_color(colors.muted)
        .child(text.into())
}

fn sqlserver_status_bar(state: &SqlServerSecurityState, colors: UiColors) -> Div {
    let snapshot = state.snapshot.as_ref();
    let logins = snapshot.map(|snapshot| snapshot.logins.len()).unwrap_or(0);
    let users = snapshot.map(|snapshot| snapshot.users.len()).unwrap_or(0);
    let roles = snapshot
        .map(|snapshot| snapshot.database_roles.len())
        .unwrap_or(0);
    h_flex()
        .items_center()
        .gap_3()
        .px_3()
        .h(px(28.))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .text_size(ui_text_size(11.))
        .text_color(colors.muted)
        .child(
            tr("app.sqlserver_security.status_counts")
                .replace("%{logins}", &logins.to_string())
                .replace("%{users}", &users.to_string())
                .replace("%{roles}", &roles.to_string()),
        )
        .child(div().flex_1())
        .when(state.has_changes(), |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(rgb(0xff9f0a))
                    .child(app_icon(AppIcon::Save, 12., rgb(0xff9f0a)))
                    .child(tr("app.sqlserver_security.pending_changes")),
            )
        })
}

fn sqlserver_delete_modal(
    tab_id: TabId,
    principal: fluxdb_core::SqlServerPrincipalRef,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let focus_handle_for_overlay = focus_handle.clone();
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(gpui::black().opacity(0.35))
        .flex()
        .items_center()
        .justify_center()
        .track_focus(&focus_handle_for_overlay)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::CancelDeleteSqlServerPrincipal(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(440.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow_lg()
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .track_focus(&focus_handle)
                .key_context("SqlServerDeletePrincipalModal")
                .on_action(cx.listener(move |this, _: &CancelDialog, _, cx| {
                    this.dispatch(AppCommand::CancelDeleteSqlServerPrincipal(tab_id), cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(16.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(tr("app.sqlserver_security.delete_title")),
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
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.dispatch(
                                            AppCommand::CancelDeleteSqlServerPrincipal(tab_id),
                                            cx,
                                        );
                                        cx.stop_propagation();
                                    }),
                                )
                                .child(app_icon(AppIcon::Close, 15., colors.muted)),
                        ),
                )
                .child(
                    div().text_size(ui_text_size(13.)).text_color(colors.muted).child(
                        tr("app.sqlserver_security.delete_body")
                            .replace(
                                "%{kind}",
                                sqlserver_principal_kind_label(principal.kind).as_ref(),
                            )
                            .replace("%{name}", &principal.name),
                    ),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            user_admin_button(
                                tr("app.common.cancel"),
                                AppIcon::Close,
                                false,
                                false,
                                colors,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(
                                        AppCommand::CancelDeleteSqlServerPrincipal(tab_id),
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            user_admin_button(
                                tr("app.common.delete"),
                                AppIcon::Trash,
                                true,
                                false,
                                colors,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(
                                        AppCommand::ConfirmDeleteSqlServerPrincipal(tab_id),
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}

fn sqlserver_principal_kind_label(kind: SqlServerPrincipalKind) -> SharedString {
    match kind {
        SqlServerPrincipalKind::Login => tr("app.sqlserver_security.kind.login"),
        SqlServerPrincipalKind::User => tr("app.sqlserver_security.kind.user"),
        SqlServerPrincipalKind::Role => tr("app.sqlserver_security.kind.role"),
    }
}

fn sqlserver_login_type_label(login_type: &str) -> SharedString {
    match login_type {
        "SQL_LOGIN" => tr("app.sqlserver_security.login_type_sql"),
        "WINDOWS_LOGIN" => tr("app.sqlserver_security.login_type_windows"),
        "WINDOWS_GROUP" => tr("app.sqlserver_security.login_type_group"),
        _ => tr("app.sqlserver_security.login_type_other"),
    }
}
