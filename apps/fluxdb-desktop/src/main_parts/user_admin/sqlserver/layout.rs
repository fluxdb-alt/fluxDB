/// SQL Server 面板：Login / User / Role 两层安全模型，与 MySQL 的 provider 流程无关。
struct SqlServerUserAdminPanel;

impl UserAdminPanel for SqlServerUserAdminPanel {
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
        sqlserver_security_content(
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

fn sqlserver_security_content(
    tab_id: TabId,
    admin: &UserAdminState,
    connection_name: String,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(state) = admin.sqlserver.as_ref() else {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(tr("app.sqlserver_security.state_missing"));
    };
    this.sync_sqlserver_security_inputs(state, window, cx);
    let mut root = div()
        .relative()
        .debug_selector(|| "sqlserver-security-page".into())
        .flex_1()
        .bg(colors.content_bg)
        .flex()
        .flex_col()
        .child(sqlserver_security_toolbar(
            tab_id,
            state,
            connection_name,
            this,
            window,
            colors,
            cx,
        ))
        .child(
            div()
                .flex()
                .min_h(px(0.))
                .flex_1()
                .child(
                    sqlserver_principal_list(tab_id, state, this, colors, cx)
                        .w(px(320.))
                        .flex_none(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(sqlserver_tab_strip(
                            tab_id,
                            admin.active_detail_tab,
                            colors,
                            cx,
                        ))
                        .child(match admin.active_detail_tab {
                            UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Roles) => {
                                sqlserver_roles_panel(tab_id, state, colors, cx)
                            }
                            UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Permissions) => {
                                sqlserver_permissions_panel(tab_id, state, this, window, colors, cx)
                            }
                            UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::SqlPreview) => {
                                sqlserver_preview_panel(tab_id, state, this, colors, cx)
                            }
                            // 常规页，以及其它库的页签（不会出现）：都按常规页渲染。
                            _ => sqlserver_general_panel(tab_id, state, this, window, colors, cx),
                        }),
                ),
        )
        .child(sqlserver_status_bar(state, colors));
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
    if let Some(principal) = state.pending_delete.clone() {
        root = root.child(sqlserver_delete_modal(
            tab_id,
            principal,
            this.focus_handle.clone(),
            colors,
            cx,
        ));
    }
    root
}

fn sqlserver_security_toolbar(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    connection_name: String,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let busy =
        state.loading || state.loading_permissions || state.preview_loading || state.applying;
    // 空白新建表单没有字段变更，但仍需“取消”关闭，因此单独识别新建/授权表单打开态。
    let form_open = matches!(
        state.form.kind,
        SqlServerSecurityFormKind::CreateLogin
            | SqlServerSecurityFormKind::CreateUser
            | SqlServerSecurityFormKind::CreateRole
            | SqlServerSecurityFormKind::GrantPermission
    );
    let verify = state.save_status == SqlServerSecuritySaveStatus::NeedsVerify;
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
                .child(tr("app.sqlserver_security.title")),
        )
        .child(user_admin_badge(connection_name, colors))
        .child(user_admin_badge("SQL Server".to_string(), colors))
        .child(sqlserver_security_database_control(
            state, this, window, cx,
        ))
        .when(verify, |row| {
            row.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.warning)
                    .child(tr("app.sqlserver_security.needs_verify")),
            )
        })
        .child(div().flex_1())
        // 与 MySQL/PG 工具栏共用 user_admin_button，保证尺寸、图标和删除危险色一致。
        .child(
            user_admin_button(
                if state.applying {
                    tr("app.sqlserver_security.saving")
                } else {
                    tr("app.common.save")
                },
                AppIcon::Save,
                false,
                busy || verify || !state.has_changes(),
                colors,
            )
            .debug_selector(|| "sqlserver-security-save".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_sqlserver_security_apply(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button(
                tr("app.common.cancel"),
                AppIcon::Undo,
                false,
                busy || verify || (!state.has_changes() && !form_open),
                colors,
            )
            .debug_selector(|| "sqlserver-security-discard".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::DiscardSqlServerSecurityChanges(tab_id), cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button(
                if state.loading {
                    tr("app.sqlserver_security.loading")
                } else {
                    tr("app.common.refresh")
                },
                AppIcon::Refresh,
                false,
                busy || (state.has_changes() && !verify),
                colors,
            )
            .debug_selector(|| "sqlserver-security-refresh".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_sqlserver_security_load(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button(
                tr("app.common.delete"),
                AppIcon::Trash,
                true,
                busy || verify || state.selected.is_none() || state.has_changes(),
                colors,
            )
            .debug_selector(|| "sqlserver-security-delete".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::BeginDeleteSqlServerPrincipal(tab_id), cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn sqlserver_tab_strip(
    tab_id: TabId,
    active_tab: UserAdminDetailTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    user_admin_detail_tab_strip(
        vec![
            (
                UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::General),
                sqlserver_detail_tab_id(UserAdminDetailTab::SqlServer(
                    SqlServerUserAdminTab::General,
                )),
                tr("app.sqlserver_security.tab.general"),
            ),
            (
                UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Roles),
                sqlserver_detail_tab_id(UserAdminDetailTab::SqlServer(
                    SqlServerUserAdminTab::Roles,
                )),
                tr("app.sqlserver_security.tab.roles"),
            ),
            (
                UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Permissions),
                sqlserver_detail_tab_id(UserAdminDetailTab::SqlServer(
                    SqlServerUserAdminTab::Permissions,
                )),
                tr("app.sqlserver_security.tab.permissions"),
            ),
            (
                UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::SqlPreview),
                sqlserver_detail_tab_id(UserAdminDetailTab::SqlServer(
                    SqlServerUserAdminTab::SqlPreview,
                )),
                tr("app.sqlserver_security.tab.sql_preview"),
            ),
        ],
        active_tab,
        "sqlserver-security-tab",
        colors,
        cx,
        move |this, detail_tab, cx| {
            if sqlserver_tab_stages_form(detail_tab)
                && !this.stage_sqlserver_security_form(tab_id, cx)
            {
                return;
            }
            this.dispatch(
                AppCommand::SelectUserAdminDetailTab { tab_id, detail_tab },
                cx,
            );
            if detail_tab.is_sqlserver(SqlServerUserAdminTab::Permissions)
                && let Some(principal) = this
                    .user_admin_state_for(tab_id)
                    .and_then(|admin| admin.sqlserver)
                    .and_then(|state| state.selected)
            {
                this.start_sqlserver_permissions_load(tab_id, principal, cx);
            }
            if detail_tab.is_sqlserver(SqlServerUserAdminTab::SqlPreview) {
                let has_preview = this
                    .user_admin_state_for(tab_id)
                    .and_then(|admin| admin.sqlserver)
                    .is_some_and(|state| state.preview.is_some());
                if !has_preview {
                    this.start_sqlserver_security_preview(tab_id, cx);
                }
            }
        },
    )
}

/// 页签的稳定英文 key（`debug_selector` 后缀，不进语言包）。
fn sqlserver_detail_tab_id(tab: UserAdminDetailTab) -> &'static str {
    match tab {
        UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Roles) => "roles",
        UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Permissions) => "permissions",
        UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::SqlPreview) => "sql-preview",
        _ => "general",
    }
}

/// 切到这些页签前要先把表单落进待执行变更，避免填了一半的编辑被切页丢掉。
fn sqlserver_tab_stages_form(tab: UserAdminDetailTab) -> bool {
    matches!(
        tab,
        UserAdminDetailTab::SqlServer(
            SqlServerUserAdminTab::Roles
                | SqlServerUserAdminTab::Permissions
                | SqlServerUserAdminTab::SqlPreview
        )
    )
}
