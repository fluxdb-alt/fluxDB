// Oracle 用户与权限页骨架：工具栏 + 左侧账号列表 + 右侧详情标签页 + 状态栏 + 删除确认。

/// Oracle / 达梦面板：账号即 schema，授权明细只读当前账号自己。
struct OracleUserAdminPanel;

impl UserAdminPanel for OracleUserAdminPanel {
    fn render(
        &self,
        state: &AppState,
        tab_id: TabId,
        admin: &UserAdminState,
        connection_name: String,
        this: &mut NavicatMain,
        window: &mut Window,
        colors: UiColors,
        cx: &mut Context<NavicatMain>,
    ) -> Div {
        let _ = state;
        sys_user_admin_search_placeholder(this, tr("app.sqlserver_security.search"), window, cx);
        oracle_security_content(
            tab_id,
            admin,
            connection_name,
            this,
            window,
            colors,
            cx,
        )
    }
}

fn oracle_security_content(
    tab_id: TabId,
    admin: &UserAdminState,
    connection_name: String,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(state) = admin.oracle.as_ref() else {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(tr("app.oracle_security.state_missing"));
    };
    this.sync_oracle_security_inputs(state, window, cx);
    let mut root = div()
        .relative()
        .flex_1()
        .bg(colors.content_bg)
        .flex()
        .flex_col()
        .child(oracle_security_toolbar(
            tab_id,
            state,
            connection_name,
            colors,
            cx,
        ))
        .child(
            div()
                .flex()
                .min_h(px(0.))
                .flex_1()
                .child(
                    oracle_user_list(tab_id, state, this, colors, cx)
                        .w(px(320.))
                        .flex_none(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(oracle_tab_strip(tab_id, admin.active_detail_tab, colors, cx))
                        .child(match admin.active_detail_tab {
                            UserAdminDetailTab::Oracle(OracleUserAdminTab::Roles) => {
                                oracle_roles_panel(tab_id, state, this, window, colors, cx)
                            }
                            UserAdminDetailTab::Oracle(OracleUserAdminTab::SystemPrivileges) => {
                                oracle_system_privileges_panel(
                                    tab_id, state, this, window, colors, cx,
                                )
                            }
                            UserAdminDetailTab::Oracle(OracleUserAdminTab::ObjectPrivileges) => {
                                oracle_object_privileges_panel(
                                    tab_id, state, this, window, colors, cx,
                                )
                            }
                            UserAdminDetailTab::Oracle(OracleUserAdminTab::SqlPreview) => {
                                oracle_preview_panel(tab_id, state, this, colors, cx)
                            }
                            // 常规页，以及不会出现的其它库页签。
                            _ => oracle_general_panel(tab_id, state, this, window, colors, cx),
                        }),
                ),
        )
        .child(oracle_status_bar(state, colors));
    if let Some(pending) = &admin.pending_sql {
        root = root.child(user_admin_sql_preview_modal(
            tab_id,
            pending,
            state.applying,
            state.apply_error.as_ref(),
            colors,
            cx,
        ));
    }
    if state.pending_delete.is_some() {
        let cascade = state.delete_cascade;
        root = root.child(oracle_delete_modal(
            tab_id,
            cascade,
            this.focus_handle.clone(),
            colors,
            cx,
        ));
    }
    root
}

fn oracle_security_toolbar(
    tab_id: TabId,
    state: &OracleUserAdminState,
    connection_name: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let busy = state.loading || state.applying;
    h_flex()
        .flex_none()
        .items_center()
        .gap_2()
        .px_3()
        .h(px(44.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .text_color(colors.text)
        .child(app_icon_box(AppIcon::Users, 28., 16., rgb(0x2563eb)))
        .child(
            div()
                .text_size(ui_text_size(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(tr("app.oracle_security.title")),
        )
        .child(user_admin_badge(connection_name, colors))
        // 达梦复用本面板，徽标必须如实显示目标库，避免把达梦当成 Oracle。
        .child(user_admin_badge(
            match state.dialect {
                fluxdb_core::UserAdminSqlDialect::Dameng => "DM8",
                fluxdb_core::UserAdminSqlDialect::Oracle => "Oracle",
            }
            .to_string(),
            colors,
        ))
        .when(state.applying, |row| {
            row.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.warning)
                    .child(tr("app.oracle_security.applying")),
            )
        })
        .child(div().flex_1())
        .child(
            user_admin_button(
                if state.applying {
                    tr("app.oracle_security.saving")
                } else {
                    tr("app.common.save")
                },
                AppIcon::Save,
                false,
                busy || !state.has_changes(),
                colors,
            )
            .debug_selector(|| "oracle-security-save".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_oracle_security_apply(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button(
                tr("app.common.cancel"),
                AppIcon::Undo,
                false,
                busy || !state.has_changes(),
                colors,
            )
            .debug_selector(|| "oracle-security-discard".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::DiscardOracleUserChanges(tab_id), cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button(
                if state.loading {
                    tr("app.oracle_security.loading")
                } else {
                    tr("app.common.refresh")
                },
                AppIcon::Refresh,
                false,
                busy || state.has_changes(),
                colors,
            )
            .debug_selector(|| "oracle-security-refresh".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_oracle_security_load(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button(
                tr("app.common.delete"),
                AppIcon::Trash,
                true,
                busy || state.selected.is_none() || state.has_changes(),
                colors,
            )
            .debug_selector(|| "oracle-security-delete".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::BeginDeleteOracleUser(tab_id), cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn oracle_tab_strip(
    tab_id: TabId,
    active_tab: UserAdminDetailTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    user_admin_detail_tab_strip(
        vec![
            (
                UserAdminDetailTab::Oracle(OracleUserAdminTab::General),
                oracle_detail_tab_id(UserAdminDetailTab::Oracle(OracleUserAdminTab::General)),
                tr("app.oracle_security.tab.general"),
            ),
            (
                UserAdminDetailTab::Oracle(OracleUserAdminTab::Roles),
                oracle_detail_tab_id(UserAdminDetailTab::Oracle(OracleUserAdminTab::Roles)),
                tr("app.oracle_security.tab.roles"),
            ),
            (
                UserAdminDetailTab::Oracle(OracleUserAdminTab::SystemPrivileges),
                oracle_detail_tab_id(UserAdminDetailTab::Oracle(
                    OracleUserAdminTab::SystemPrivileges,
                )),
                tr("app.oracle_security.tab.system_privileges"),
            ),
            (
                UserAdminDetailTab::Oracle(OracleUserAdminTab::ObjectPrivileges),
                oracle_detail_tab_id(UserAdminDetailTab::Oracle(
                    OracleUserAdminTab::ObjectPrivileges,
                )),
                tr("app.oracle_security.tab.object_privileges"),
            ),
            (
                UserAdminDetailTab::Oracle(OracleUserAdminTab::SqlPreview),
                oracle_detail_tab_id(UserAdminDetailTab::Oracle(
                    OracleUserAdminTab::SqlPreview,
                )),
                tr("app.oracle_security.tab.sql_preview"),
            ),
        ],
        active_tab,
        "oracle-security-tab",
        colors,
        cx,
        move |this, detail_tab, cx| {
            this.dispatch(
                AppCommand::SelectUserAdminDetailTab { tab_id, detail_tab },
                cx,
            );
        },
    )
}

/// 页签的稳定英文 key（`debug_selector` 后缀，不进语言包）。
fn oracle_detail_tab_id(tab: UserAdminDetailTab) -> &'static str {
    match tab {
        UserAdminDetailTab::Oracle(OracleUserAdminTab::Roles) => "roles",
        UserAdminDetailTab::Oracle(OracleUserAdminTab::SystemPrivileges) => "system-privileges",
        UserAdminDetailTab::Oracle(OracleUserAdminTab::ObjectPrivileges) => "object-privileges",
        UserAdminDetailTab::Oracle(OracleUserAdminTab::SqlPreview) => "sql-preview",
        _ => "general",
    }
}
