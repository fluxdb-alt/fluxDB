fn sqlserver_roles_panel(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let Some(principal) = state.selected.as_ref() else {
        return sqlserver_empty_panel(tr("app.sqlserver_security.select_principal"), colors)
            .into_any_element();
    };
    let Some(snapshot) = state.snapshot.as_ref() else {
        return center_loading_message(tr("app.sqlserver_security.loading"), colors)
            .into_any_element();
    };
    let mut panel = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .p_4()
        .flex()
        .flex_col()
        .gap_2();
    match principal.kind {
        SqlServerPrincipalKind::Login => {
            let editable = state.selected_login_is_editable();
            panel = panel.child(sqlserver_section_title(
                tr("app.sqlserver_security.server_roles"),
                colors,
            ));
            for role in &snapshot.server_roles {
                if role.name.eq_ignore_ascii_case("public") {
                    continue;
                }
                let checked = state.effective_role_membership(
                    fluxdb_core::SqlServerRoleScope::Server,
                    &role.name,
                    &principal.name,
                );
                panel = panel.child(sqlserver_role_checkbox(
                    SqlServerRoleCheckbox {
                        tab_id,
                        id: format!("sqlserver-server-role-{}-{}", principal.name, role.name),
                        label: role.name.clone(),
                        checked,
                        scope: fluxdb_core::SqlServerRoleScope::Server,
                        role: role.name.clone(),
                        member: principal.name.clone(),
                        disabled: !editable,
                        colors,
                    },
                    cx,
                ));
            }
        }
        SqlServerPrincipalKind::User => {
            let editable = state.selected_user_is_editable();
            panel = panel.child(sqlserver_section_title(
                tr("app.sqlserver_security.database_roles"),
                colors,
            ));
            for role in &snapshot.database_roles {
                if role.name.eq_ignore_ascii_case("public") {
                    continue;
                }
                let checked = state.effective_role_membership(
                    fluxdb_core::SqlServerRoleScope::Database,
                    &role.name,
                    &principal.name,
                );
                panel = panel.child(sqlserver_role_checkbox(
                    SqlServerRoleCheckbox {
                        tab_id,
                        id: format!("sqlserver-db-role-{}-{}", principal.name, role.name),
                        label: role.name.clone(),
                        checked,
                        scope: fluxdb_core::SqlServerRoleScope::Database,
                        role: role.name.clone(),
                        member: principal.name.clone(),
                        disabled: !editable,
                        colors,
                    },
                    cx,
                ));
            }
        }
        SqlServerPrincipalKind::Role => {
            panel = panel.child(sqlserver_section_title(
                tr("app.sqlserver_security.role_members"),
                colors,
            ));
            let mut candidates = snapshot
                .users
                .iter()
                .map(|user| user.name.clone())
                .collect::<Vec<_>>();
            candidates.extend(
                snapshot
                    .database_roles
                    .iter()
                    .filter(|role| {
                        !role.is_fixed && !role.name.eq_ignore_ascii_case(&principal.name)
                    })
                    .map(|role| role.name.clone()),
            );
            candidates.sort();
            candidates.dedup();
            for member in candidates {
                let checked = state.effective_role_membership(
                    fluxdb_core::SqlServerRoleScope::Database,
                    &principal.name,
                    &member,
                );
                panel = panel.child(sqlserver_role_checkbox(
                    SqlServerRoleCheckbox {
                        tab_id,
                        id: format!("sqlserver-role-member-{}-{member}", principal.name),
                        label: member.clone(),
                        checked,
                        scope: fluxdb_core::SqlServerRoleScope::Database,
                        role: principal.name.clone(),
                        member,
                        disabled: false,
                        colors,
                    },
                    cx,
                ));
            }
        }
    }
    panel.into_any_element()
}

struct SqlServerRoleCheckbox {
    tab_id: TabId,
    id: String,
    label: String,
    checked: bool,
    scope: fluxdb_core::SqlServerRoleScope,
    role: String,
    member: String,
    disabled: bool,
    colors: UiColors,
}

fn sqlserver_role_checkbox(checkbox: SqlServerRoleCheckbox, cx: &mut Context<NavicatMain>) -> Div {
    let SqlServerRoleCheckbox {
        tab_id,
        id,
        label,
        checked,
        scope,
        role,
        member,
        disabled,
        colors,
    } = checkbox;
    let debug_selector = id.clone();
    let mut row = h_flex()
        .h(px(36.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .debug_selector(move || debug_selector.clone())
        .items_center()
        .gap_2()
        .child(
            Checkbox::new(id)
                .checked(checked)
                .disabled(disabled)
                .on_click(cx.listener(move |this, checked, _, cx| {
                    if *checked || !disabled {
                        this.dispatch(
                            AppCommand::ToggleSqlServerRoleMembership {
                                tab_id,
                                scope,
                                role: role.clone(),
                                member: member.clone(),
                            },
                            cx,
                        );
                    }
                })),
        )
        .child(
            div()
                .text_size(ui_text_size(13.))
                .text_color(if disabled { colors.muted } else { colors.text })
                .child(label),
        );
    if disabled {
        row = row.opacity(0.6);
    }
    row
}
