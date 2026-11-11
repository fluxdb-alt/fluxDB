fn sqlserver_general_panel(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    if state.form.is_open() {
        return sqlserver_form_panel(tab_id, state, this, window, colors, cx);
    }
    let Some(principal) = state.selected.as_ref() else {
        return sqlserver_empty_panel(tr("app.sqlserver_security.select_principal"), colors)
            .into_any_element();
    };
    let mut form = div().max_w(px(620.)).flex().flex_col().gap_3().child(
        h_flex()
            .mb_1()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_size(ui_text_size(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child(principal.name.clone()),
            )
            .child(user_admin_badge(
                sqlserver_principal_kind_label(principal.kind).to_string(),
                colors,
            )),
    );
    match principal.kind {
        SqlServerPrincipalKind::Role => {
            if let Some(role) = state.selected_role() {
                form = form
                    .child(
                        sqlserver_form_row(tr("app.sqlserver_security.role_name"), colors).child(
                            div()
                                .text_color(colors.text)
                                .text_size(ui_text_size(13.))
                                .child(role.name.clone()),
                        ),
                    )
                    .child(
                        sqlserver_form_row(tr("app.sqlserver_security.role_owner"), colors).child(
                            div()
                                .text_color(colors.text)
                                .text_size(ui_text_size(13.))
                                .child(role.owner.clone().unwrap_or_else(|| "-".into())),
                        ),
                    )
                    .child(sqlserver_form_hint(
                        if role.is_fixed {
                            tr("app.sqlserver_security.fixed_role_hint")
                        } else {
                            tr("app.sqlserver_security.role_help")
                        },
                        colors,
                    ));
            }
        }
        SqlServerPrincipalKind::Login => {
            if let Some(login) = state.selected_login() {
                form = form
                    .child(sqlserver_readonly_value_row(
                        tr("app.sqlserver_security.login_name"),
                        login.name.clone(),
                        colors,
                    ))
                    .child(sqlserver_readonly_value_row(
                        tr("app.sqlserver_security.login_type"),
                        sqlserver_login_type_label(&login.login_type).to_string(),
                        colors,
                    ))
                    .child(sqlserver_readonly_value_row(
                        tr("app.sqlserver_security.default_database"),
                        login.default_database.clone().unwrap_or_else(|| "-".into()),
                        colors,
                    ));
            }
            form = form.child(sqlserver_form_hint(
                tr("app.sqlserver_security.readonly_hint"),
                colors,
            ));
        }
        SqlServerPrincipalKind::User => {
            if let Some(user) = state.selected_user() {
                form = form
                    .child(sqlserver_readonly_value_row(
                        tr("app.sqlserver_security.user_name"),
                        user.name.clone(),
                        colors,
                    ))
                    .child(sqlserver_readonly_value_row(
                        tr("app.sqlserver_security.mapped_login"),
                        user.login_name.clone().unwrap_or_else(|| "-".into()),
                        colors,
                    ))
                    .child(sqlserver_readonly_value_row(
                        tr("app.sqlserver_security.default_schema"),
                        user.default_schema.clone().unwrap_or_else(|| "-".into()),
                        colors,
                    ));
            }
            form = form.child(sqlserver_form_hint(
                tr("app.sqlserver_security.readonly_hint"),
                colors,
            ));
        }
    }
    if state.pending_creations().iter().any(|pending| pending == principal) {
        form = form.child(sqlserver_form_hint(
            tr("app.sqlserver_security.pending_creation_help"),
            colors,
        ));
    }
    if let Some(error) = &state.apply_error {
        form = form.child(user_admin_error_row(&error.message, colors));
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .bg(colors.panel_bg)
        .p_4()
        .child(form)
        .into_any_element()
}

fn sqlserver_readonly_value_row(label: SharedString, value: String, colors: UiColors) -> Div {
    sqlserver_form_row(label, colors).child(
        div()
            .w(px(360.))
            .text_color(colors.text)
            .text_size(ui_text_size(13.))
            .child(value),
    )
}
