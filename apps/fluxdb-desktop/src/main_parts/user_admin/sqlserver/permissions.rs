fn sqlserver_permissions_panel(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let Some(principal) = state.selected.as_ref() else {
        return sqlserver_empty_panel(tr("app.sqlserver_security.select_principal"), colors)
            .into_any_element();
    };
    let editable = match principal.kind {
        SqlServerPrincipalKind::Login => state.selected_login_is_editable(),
        SqlServerPrincipalKind::User => state.selected_user_is_editable(),
        SqlServerPrincipalKind::Role => true,
    };
    let adding = state.permission_form.kind == SqlServerSecurityFormKind::GrantPermission;
    let mut panel = div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .p_4()
        .gap_2();
    if adding {
        panel = panel.child(sqlserver_permission_composer(
            tab_id, state, this, window, colors, cx,
        ));
    }
    panel = panel.child(
        h_flex()
            .items_center()
            .gap_2()
            .child(sqlserver_section_title(
                if adding {
                    tr("app.sqlserver_security.permission_list")
                } else {
                    tr("app.sqlserver_security.explicit_permissions")
                },
                colors,
            ))
            .when(adding, |row| {
                row.child(user_admin_badge(
                    tr("app.sqlserver_security.permission_pending_count").replace(
                        "%{count}",
                        &state
                            .effective_permissions()
                            .iter()
                            .filter(|permission| state.permission_is_pending(permission))
                            .count()
                            .to_string(),
                    ),
                    colors,
                ))
            })
            .child(div().flex_1())
            .child(
                user_admin_button(
                    if adding {
                        tr("app.sqlserver_security.collapse_add_permission")
                    } else {
                        tr("app.sqlserver_security.grant_permission")
                    },
                    if adding { AppIcon::ChevronUp } else { AppIcon::Plus },
                    false,
                    state.loading_permissions || !editable || state.applying,
                    colors,
                )
                .id("sqlserver-security-grant-permission")
                .debug_selector(|| "sqlserver-security-grant-permission".into())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if !editable {
                            return;
                        }
                        if adding {
                            this.dispatch(
                                AppCommand::CloseSqlServerPermissionComposer(tab_id),
                                cx,
                            );
                        } else {
                            this.dispatch(
                                AppCommand::OpenSqlServerSecurityForm {
                                    tab_id,
                                    kind: SqlServerSecurityFormKind::GrantPermission,
                                },
                                cx,
                            );
                        }
                        cx.stop_propagation();
                    }),
                ),
            ),
    );
    let mut list = div().flex_1().min_h(px(0.)).overflow_y_scrollbar();
    if state.loading_permissions {
        list = list.child(user_admin_empty_row(
            tr("app.sqlserver_security.loading_permissions"),
            colors,
        ));
    } else if let Some(error) = &state.permissions_error {
        list = list.child(user_admin_error_row(&error.message, colors));
    } else if state.effective_permissions().is_empty() {
        list = list.child(user_admin_empty_row(
            if state.pending_creations().contains(principal) {
                tr("app.sqlserver_security.pending_permissions_help")
            } else {
                tr("app.sqlserver_security.no_explicit_permissions")
            },
            colors,
        ));
    } else {
        for (index, permission) in state.effective_permissions().iter().enumerate() {
            let debug_selector = format!("sqlserver-security-permission-{index}");
            list = list.child(
                h_flex()
                    .debug_selector(move || debug_selector.clone())
                    .h(px(38.))
                    .px_2()
                    .border_b_1()
                    .border_color(colors.border_soft)
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(180.))
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(colors.text)
                            .child(permission.permission.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .text_color(colors.muted)
                            .child(SqlServerSecurityState::permission_target_label(
                                &permission.target,
                            )),
                    )
                    .child(user_admin_badge(
                        SqlServerSecurityState::permission_state_label(permission.state)
                            .to_string(),
                        colors,
                    ))
                    .when(state.permission_is_pending(permission), |row| {
                        row.child(user_admin_badge(
                            tr("app.sqlserver_security.pending_badge").to_string(),
                            colors,
                        ))
                    })
                    .child(
                        user_admin_button(
                            if state.permission_is_pending(permission) {
                                tr("app.sqlserver_security.remove_pending")
                            } else if permission.state
                                == fluxdb_core::SqlServerPermissionState::GrantWithGrantOption {
                                tr("app.sqlserver_security.revoke_cascade")
                            } else {
                                tr("app.sqlserver_security.revoke")
                            },
                            if state.permission_is_pending(permission) {
                                AppIcon::Undo
                            } else {
                                AppIcon::CircleSlash
                            },
                            true,
                            !editable,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                if !editable {
                                    return;
                                }
                                this.dispatch(
                                    AppCommand::RevokeSqlServerPermission {
                                        tab_id,
                                        permission_index: index,
                                    },
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ),
                    ),
            );
        }
    }
    panel.child(list).into_any_element()
}

fn sqlserver_permission_composer(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let editable = !state.applying
        && !state.loading
        && !state.preview_loading
        && state.save_status != SqlServerSecuritySaveStatus::NeedsVerify;
    let permission = if state.permission_form.permission.trim().is_empty() {
        tr("app.sqlserver_security.permission_not_selected")
    } else {
        SharedString::from(state.permission_form.permission.clone())
    };
    let has_permission = !state.permission_form.permission.trim().is_empty();
    let mut composer = div()
        .debug_selector(|| "sqlserver-security-permission-composer".into())
        .border_1()
        .border_color(colors.border)
        .rounded(colors.radius)
        .bg(colors.panel_alt)
        .p_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.sqlserver_security.add_permission_title")),
                )
                .child(
                    div()
                        .id("sqlserver-permission-composer-close")
                        .cursor_pointer()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.muted)
                        .hover(move |style| style.text_color(colors.text))
                        .child(tr("app.sqlserver_security.collapse_add_permission"))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.dispatch(
                                    AppCommand::CloseSqlServerPermissionComposer(tab_id),
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ),
                ),
        )
        .child(sqlserver_permission_scope_row(tab_id, state, colors, cx));
    if matches!(state.permission_form.permission_scope.as_str(), "SCHEMA" | "OBJECT") {
        composer = composer.child(sqlserver_form_input(
            if state.permission_form.permission_scope == "OBJECT" {
                tr("app.sqlserver_security.permission_object")
            } else {
                tr("app.sqlserver_security.permission_schema")
            },
            this.sqlserver_security_permission_target_input.clone(),
            "sqlserver-security-form-target",
            editable,
            window,
            colors,
            cx,
        ));
    }
    composer = composer
        .child(sqlserver_permission_chips(state, colors, cx))
        .child(sqlserver_permission_state_row(tab_id, state, colors, cx))
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    h_flex()
                        .min_w(px(0.))
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(tr("app.sqlserver_security.selected_permission")),
                        )
                        .child(user_admin_badge(permission.to_string(), colors)),
                )
                .child(
                    user_admin_button(
                        tr("app.sqlserver_security.add_to_pending"),
                        AppIcon::Plus,
                        false,
                        !editable || !has_permission,
                        colors,
                    )
                    .id("sqlserver-permission-add")
                    .debug_selector(|| "sqlserver-permission-add".into())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if !has_permission {
                                return;
                            }
                            let event = this.dispatch(
                                AppCommand::SubmitSqlServerSecurityForm(tab_id),
                                cx,
                            );
                            if matches!(event, AppEvent::Failed(_)) {
                                return;
                            }
                            cx.stop_propagation();
                        }),
                    ),
                ),
        );
    if let Some(error) = &state.apply_error {
        composer = composer.child(user_admin_error_row(&error.message, colors));
    }
    composer
}

fn sqlserver_permission_scope_row(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let kind = state
        .selected_kind()
        .unwrap_or(SqlServerPrincipalKind::User);
    let scopes = if kind == SqlServerPrincipalKind::Login {
        vec!["SERVER"]
    } else {
        vec!["DATABASE", "SCHEMA", "OBJECT"]
    };
    let mut row = h_flex().gap_1().items_center();
    for scope in scopes {
        let selected = state.permission_form.permission_scope == scope;
        row = row.child(
            div()
                .id(SharedString::from(format!(
                    "sqlserver-permission-scope-{scope}"
                )))
                .h(px(28.))
                .px_3()
                .rounded(colors.radius)
                .border_1()
                .border_color(if selected {
                    rgb(0x2563eb)
                } else {
                    colors.border
                })
                .bg(if selected {
                    colors.tree_selected
                } else {
                    colors.input_bg
                })
                .cursor_pointer()
                .flex()
                .items_center()
                .text_size(ui_text_size(12.))
                .text_color(colors.text)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(
                            AppCommand::SetSqlServerSecurityFormField {
                                tab_id,
                                field: SqlServerSecurityFormField::PermissionScope,
                                value: scope.to_string(),
                            },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )
                .child(scope),
        );
    }
    row
}

fn sqlserver_permission_chips(
    state: &SqlServerSecurityState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let target = match state.permission_form.permission_scope.as_str() {
        "SERVER" => fluxdb_core::SqlServerPermissionTarget::Server,
        "SCHEMA" => fluxdb_core::SqlServerPermissionTarget::Schema {
            schema: String::new(),
        },
        "OBJECT" => fluxdb_core::SqlServerPermissionTarget::Object {
            schema: String::new(),
            object: String::new(),
        },
        _ => fluxdb_core::SqlServerPermissionTarget::Database,
    };
    let mut wrap = div().flex().flex_wrap().gap_1();
    for permission in fluxdb_core::sqlserver_permissions_for_target(&target) {
        let selected = state
            .permission_form
            .permission
            .eq_ignore_ascii_case(permission);
        let permission = permission.to_string();
        let value = permission.clone();
        let debug_selector = format!("sqlserver-permission-chip-{permission}");
        wrap = wrap.child(
            div()
                .id(SharedString::from(format!(
                    "sqlserver-permission-chip-{permission}"
                )))
                .debug_selector(move || debug_selector.clone())
                .h(px(26.))
                .px_2()
                .rounded(colors.radius)
                .border_1()
                .border_color(if selected {
                    rgb(0x2563eb)
                } else {
                    colors.border
                })
                .bg(if selected {
                    colors.tree_selected
                } else {
                    colors.panel_alt
                })
                .cursor_pointer()
                .flex()
                .items_center()
                .text_size(ui_text_size(11.))
                .text_color(colors.text)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        let Some(tab_id) = this.active_user_admin_tab_id() else {
                            return;
                        };
                        this.dispatch(
                            AppCommand::SetSqlServerSecurityFormField {
                                tab_id,
                                field: SqlServerSecurityFormField::Permission,
                                value: value.clone(),
                            },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )
                .child(permission),
        );
    }
    wrap
}

fn sqlserver_permission_state_row(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut row = h_flex().gap_3().items_center();
    for (value, label) in [
        ("GRANT", tr("app.sqlserver_security.grant")),
        ("DENY", tr("app.sqlserver_security.deny")),
    ] {
        let selected = state.permission_form.permission_state == value;
        row = row.child(
            div()
                .id(SharedString::from(format!(
                    "sqlserver-permission-state-{value}"
                )))
                .px_2()
                .h(px(26.))
                .rounded(colors.radius)
                .border_1()
                .border_color(if selected {
                    rgb(0x2563eb)
                } else {
                    colors.border
                })
                .cursor_pointer()
                .flex()
                .items_center()
                .text_size(ui_text_size(12.))
                .text_color(colors.text)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(
                            AppCommand::SetSqlServerSecurityFormField {
                                tab_id,
                                field: SqlServerSecurityFormField::PermissionState,
                                value: value.to_string(),
                            },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )
                .child(label),
        );
    }
    row = row.child(
        Checkbox::new("sqlserver-permission-grant-option")
            .checked(state.permission_form.grant_option)
            .label(tr("app.sqlserver_security.with_grant_option"))
            .disabled(state.permission_form.permission_state == "DENY")
            .on_click(cx.listener(move |this, checked, _, cx| {
                this.dispatch(
                    AppCommand::SetSqlServerSecurityFormFlag {
                        tab_id,
                        flag: SqlServerSecurityFormFlag::GrantOption,
                        value: *checked,
                    },
                    cx,
                );
            })),
    );
    row
}
