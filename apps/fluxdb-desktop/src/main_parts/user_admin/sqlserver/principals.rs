fn sqlserver_principal_list(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let query = this
        .user_admin_state_for(tab_id)
        .map(|admin| admin.search.to_lowercase())
        .unwrap_or_default();
    let matches = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
    let show_logins = matches_filter(state.list_filter, SqlServerPrincipalKind::Login);
    let show_users = matches_filter(state.list_filter, SqlServerPrincipalKind::User);
    let show_roles = matches_filter(state.list_filter, SqlServerPrincipalKind::Role);
    let snapshot = state.snapshot.as_ref();
    let mut list = div().flex_1().min_h(px(0.)).overflow_y_scrollbar();
    if state.loading {
        list = list.child(inline_loading_row(tr("app.sqlserver_security.loading"), colors));
    } else if let Some(error) = &state.error {
        list = list.child(user_admin_error_row(&error.message, colors));
    } else if let Some(snapshot) = snapshot {
        let mut count = 0usize;
        if show_logins {
            for login in snapshot.logins.iter().filter(|login| matches(&login.name)) {
                count += 1;
                list = list.child(sqlserver_principal_row(
                    tab_id,
                    state,
                    fluxdb_core::SqlServerPrincipalRef {
                        kind: SqlServerPrincipalKind::Login,
                        name: login.name.clone(),
                    },
                    sqlserver_login_type_label(&login.login_type).to_string(),
                    login.is_disabled,
                    colors,
                    cx,
                ));
            }
        }
        if show_users {
            for user in snapshot.users.iter().filter(|user| matches(&user.name)) {
                count += 1;
                list = list.child(sqlserver_principal_row(
                    tab_id,
                    state,
                    fluxdb_core::SqlServerPrincipalRef {
                        kind: SqlServerPrincipalKind::User,
                        name: user.name.clone(),
                    },
                    user.login_name
                        .as_ref()
                        .map(|login| {
                            tr("app.sqlserver_security.user_mapped").replace("%{login}", login)
                        })
                        .unwrap_or_else(|| tr("app.sqlserver_security.user_unmapped").to_string()),
                    false,
                    colors,
                    cx,
                ));
            }
        }
        if show_roles {
            for role in snapshot
                .database_roles
                .iter()
                .filter(|role| matches(&role.name))
            {
                count += 1;
                list = list.child(sqlserver_principal_row(
                    tab_id,
                    state,
                    fluxdb_core::SqlServerPrincipalRef {
                        kind: SqlServerPrincipalKind::Role,
                        name: role.name.clone(),
                    },
                    if role.is_fixed {
                        tr("app.sqlserver_security.fixed_role").to_string()
                    } else {
                        tr("app.sqlserver_security.database_role").to_string()
                    },
                    false,
                    colors,
                    cx,
                ));
            }
        }
        for pending in state.pending_creations() {
            if query.is_empty() || pending.name.to_lowercase().contains(&query) {
                list = list.child(
                    sqlserver_security_draft_row(
                        tr("app.sqlserver_security.pending_creation")
                            .replace("%{name}", &pending.name),
                        sqlserver_draft_subtitle(pending.kind),
                        colors,
                    )
                    .debug_selector(|| "sqlserver-security-pending-creation".into())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.dispatch(
                                AppCommand::SelectSqlServerPrincipal {
                                    tab_id,
                                    principal: pending.clone(),
                                },
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
                );
                count += 1;
            }
        }
        if count == 0 {
            list = list.child(user_admin_empty_row(
                tr("app.sqlserver_security.empty_principals"),
                colors,
            ));
        }
    } else {
        list = list.child(user_admin_empty_row(
            tr("app.sqlserver_security.choose_database"),
            colors,
        ));
    }

    if matches!(
        state.form.kind,
        SqlServerSecurityFormKind::CreateLogin
            | SqlServerSecurityFormKind::CreateUser
            | SqlServerSecurityFormKind::CreateRole
    ) && !state.form_matches_pending_creation()
    {
        list = list.child(
            sqlserver_security_draft_row(
                if state.form.name.is_empty() {
                    tr("app.sqlserver_security.new_draft").to_string()
                } else {
                    state.form.name.clone()
                },
                sqlserver_draft_subtitle(form_kind_to_principal_kind(state.form.kind)),
                colors,
            )
            .debug_selector(|| "sqlserver-security-draft".into()),
        );
    }

    div()
        .min_w(px(0.))
        .border_r_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(
            div()
                .px_2()
                .pt_2()
                .pb_1()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            user_admin_input_box(this.user_admin_search_input.clone(), colors)
                                .flex_1(),
                        )
                        .child(
                            user_admin_add_user_button(
                                state.loading || state.database.is_empty(),
                                colors,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    let database_ready = this
                                        .user_admin_state_for(tab_id)
                                        .and_then(|admin| admin.sqlserver)
                                        .is_some_and(|state| !state.database.is_empty());
                                    if !database_ready {
                                        this.show_message(
                                            tr("app.sqlserver_security.choose_database"),
                                            AppMessageKind::Warning,
                                            cx,
                                        );
                                        cx.stop_propagation();
                                        return;
                                    }
                                    let kind = this
                                        .user_admin_state_for(tab_id)
                                        .and_then(|admin| admin.sqlserver)
                                        .map(|state| match state.list_filter {
                                            SqlServerSecurityListFilter::Users => {
                                                SqlServerSecurityFormKind::CreateUser
                                            }
                                            SqlServerSecurityListFilter::Roles => {
                                                SqlServerSecurityFormKind::CreateRole
                                            }
                                            _ => SqlServerSecurityFormKind::CreateLogin,
                                        })
                                        .unwrap_or(SqlServerSecurityFormKind::CreateLogin);
                                    this.dispatch(
                                        AppCommand::OpenSqlServerSecurityForm { tab_id, kind },
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                )
                .child(sqlserver_filter_row(tab_id, state.list_filter, colors, cx)),
        )
        .child(list)
}

fn sqlserver_security_draft_row(title: String, subtitle: SharedString, colors: UiColors) -> Stateful<Div> {
    div()
        .id("sqlserver-security-draft-row")
        .h(px(56.))
        .mx_2()
        .my_2()
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(rgb(0x2563eb))
        .bg(if colors.is_dark {
            rgb(0x16345d)
        } else {
            rgb(0xeaf2ff)
        })
        .flex()
        .items_center()
        .gap_2()
        .child(app_icon_box(AppIcon::Plus, 24., 15., rgb(0x2563eb)))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(subtitle),
                ),
        )
}

fn sqlserver_draft_subtitle(kind: SqlServerPrincipalKind) -> SharedString {
    match kind {
        SqlServerPrincipalKind::Login => tr("app.sqlserver_security.draft_login"),
        SqlServerPrincipalKind::User => tr("app.sqlserver_security.draft_user"),
        SqlServerPrincipalKind::Role => tr("app.sqlserver_security.draft_role"),
    }
}

fn form_kind_to_principal_kind(kind: SqlServerSecurityFormKind) -> SqlServerPrincipalKind {
    match kind {
        SqlServerSecurityFormKind::CreateLogin => SqlServerPrincipalKind::Login,
        SqlServerSecurityFormKind::CreateUser => SqlServerPrincipalKind::User,
        _ => SqlServerPrincipalKind::Role,
    }
}

fn matches_filter(filter: SqlServerSecurityListFilter, kind: SqlServerPrincipalKind) -> bool {
    matches!(
        (filter, kind),
        (SqlServerSecurityListFilter::All, _)
            | (
                SqlServerSecurityListFilter::Logins,
                SqlServerPrincipalKind::Login
            )
            | (
                SqlServerSecurityListFilter::Users,
                SqlServerPrincipalKind::User
            )
            | (
                SqlServerSecurityListFilter::Roles,
                SqlServerPrincipalKind::Role
            )
    )
}

fn sqlserver_filter_row(
    tab_id: TabId,
    active: SqlServerSecurityListFilter,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let filters = [
        (
            SqlServerSecurityListFilter::All,
            tr("app.sqlserver_security.filter.all"),
        ),
        (
            SqlServerSecurityListFilter::Logins,
            tr("app.sqlserver_security.filter.logins"),
        ),
        (
            SqlServerSecurityListFilter::Users,
            tr("app.sqlserver_security.filter.users"),
        ),
        (
            SqlServerSecurityListFilter::Roles,
            tr("app.sqlserver_security.filter.roles"),
        ),
    ];
    let mut row = h_flex().items_center().gap_1().pb_1();
    for (filter, label) in filters {
        let selected = active == filter;
        let filter_code = filter as u8;
        let debug_selector = format!("sqlserver-security-filter-{filter_code}");
        row = row.child(
            div()
                .id(SharedString::from(format!(
                    "sqlserver-security-filter-{}",
                    filter_code
                )))
                .debug_selector(move || debug_selector.clone())
                .px_2()
                .h(px(22.))
                .rounded(colors.radius)
                .cursor_pointer()
                .bg(if selected {
                    colors.tree_selected
                } else {
                    colors.panel_bg
                })
                .hover(move |style| {
                    style.bg(if selected {
                        colors.tree_selected
                    } else {
                        colors.hover
                    })
                })
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(if selected { colors.text } else { colors.muted })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(
                            AppCommand::SetSqlServerSecurityFilter { tab_id, filter },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )
                .child(label),
        );
    }
    row.child(div().flex_1())
}

fn sqlserver_principal_row(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    principal: fluxdb_core::SqlServerPrincipalRef,
    detail: String,
    warning: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let active = state.selected.as_ref() == Some(&principal);
    let icon = match principal.kind {
        SqlServerPrincipalKind::Login => AppIcon::Key,
        SqlServerPrincipalKind::User => AppIcon::Users,
        SqlServerPrincipalKind::Role => AppIcon::Users,
    };
    let select = principal.clone();
    div()
        .id(SharedString::from(format!(
            "sqlserver-principal-{}-{}",
            principal.kind as u8, principal.name
        )))
        .h(px(52.))
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .cursor_pointer()
        .bg(if active {
            colors.tree_selected
        } else {
            colors.panel_bg
        })
        .hover(move |style| {
            style.bg(if active {
                colors.tree_selected
            } else {
                colors.hover
            })
        })
        .flex()
        .items_center()
        .gap_2()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                let load_permissions = this
                    .user_admin_state_for(tab_id)
                    .is_some_and(|admin| admin.active_detail_tab.is_sqlserver(SqlServerUserAdminTab::Permissions));
                let event = this.dispatch(
                    AppCommand::SelectSqlServerPrincipal {
                        tab_id,
                        principal: select.clone(),
                    },
                    cx,
                );
                if load_permissions && !matches!(event, AppEvent::Failed(_)) {
                    this.start_sqlserver_permissions_load(tab_id, select.clone(), cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon_box(icon, 24., 15., colors.muted))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(principal.name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(if warning { rgb(0xff9f0a) } else { colors.muted })
                        .child(detail),
                ),
        )
}
