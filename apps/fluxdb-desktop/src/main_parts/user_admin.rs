impl NavicatMain {
    fn active_user_admin_tab_id(&self) -> Option<TabId> {
        self.controller.state().active_tab().and_then(|tab| {
            matches!(tab.kind, TabKind::UserAdmin(_)).then_some(tab.id)
        })
    }

    fn start_user_admin_users_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self._user_admin_users_tasks.contains_key(&tab_id) {
            return;
        }
        self.dispatch(AppCommand::StartUserAdminUsersLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadUserAdminUsers(tab_id)) {
                        AppEvent::UserAdminUsersLoaded(_, users) => Ok(users),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载用户失败".to_string(),
                            message: "用户列表加载没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._user_admin_users_tasks.remove(&tab_id);
                    this.dispatch(
                        AppCommand::FinishUserAdminUsersLoad {
                            tab_id,
                            result: result.clone(),
                        },
                        cx,
                    );
                    if let Some(admin) = this.user_admin_state_for(tab_id)
                        && matches!(
                            admin.active_detail_tab,
                            UserAdminDetailTab::MemberOf | UserAdminDetailTab::Privileges
                        )
                        && let Some(user) = user_admin_grants_load_user(&admin)
                    {
                        this.start_user_admin_grants_load(tab_id, user, cx);
                    }
                    if let Some(admin) = this.user_admin_state_for(tab_id)
                        && admin.active_detail_tab == UserAdminDetailTab::MemberOf
                        && let Some(role) = user_admin_members_load_role(&admin)
                    {
                        this.start_user_admin_member_grants_load(tab_id, role, cx);
                    }
                    if let Some(admin) = this.user_admin_state_for(tab_id)
                        && admin.active_detail_tab == UserAdminDetailTab::Members
                        && let Some(role) = user_admin_members_load_role(&admin)
                    {
                        this.start_user_admin_member_grants_load(tab_id, role, cx);
                    }
                });
            });
        });
        self._user_admin_users_tasks.insert(tab_id, task);
    }

    fn start_user_admin_grants_load(
        &mut self,
        tab_id: TabId,
        user: DatabaseUserIdentity,
        cx: &mut Context<Self>,
    ) {
        if self._user_admin_grants_tasks.contains_key(&tab_id) {
            return;
        }
        self.dispatch(
            AppCommand::StartUserAdminGrantsLoad {
                tab_id,
                user: user.clone(),
            },
            cx,
        );
        let mut controller = self.controller.clone();
        let load_user = user.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadUserAdminGrants {
                        tab_id,
                        user: load_user,
                    }) {
                        AppEvent::UserAdminGrantsLoaded(_, grants) => Ok(grants),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载角色归属失败".to_string(),
                            message: "角色归属加载没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._user_admin_grants_tasks.remove(&tab_id);
                    this.dispatch(
                        AppCommand::FinishUserAdminGrantsLoad {
                            tab_id,
                            user,
                            result,
                        },
                        cx,
                    );
                });
            });
        });
        self._user_admin_grants_tasks.insert(tab_id, task);
    }

    fn start_user_admin_member_grants_load(
        &mut self,
        tab_id: TabId,
        role: DatabaseUserIdentity,
        cx: &mut Context<Self>,
    ) {
        if self._user_admin_member_grants_tasks.contains_key(&tab_id) {
            return;
        }
        self.dispatch(
            AppCommand::StartUserAdminMemberGrantsLoad {
                tab_id,
                role: role.clone(),
            },
            cx,
        );
        let mut controller = self.controller.clone();
        let load_role = role.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadUserAdminMemberGrants {
                        tab_id,
                        role: load_role,
                    }) {
                        AppEvent::UserAdminMemberGrantsLoaded(_, members) => Ok(members),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载成员失败".to_string(),
                            message: "成员加载没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._user_admin_member_grants_tasks.remove(&tab_id);
                    this.dispatch(
                        AppCommand::FinishUserAdminMemberGrantsLoad {
                            tab_id,
                            role,
                            result,
                        },
                        cx,
                    );
                });
            });
        });
        self._user_admin_member_grants_tasks.insert(tab_id, task);
    }

    fn start_user_admin_sql_apply(&mut self, tab_id: TabId, sql: String, cx: &mut Context<Self>) {
        if self._user_admin_apply_tasks.contains_key(&tab_id) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }
        self.dispatch(AppCommand::StartUserAdminSqlApply(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::ApplyUserAdminSql { tab_id, sql }) {
                        AppEvent::UserAdminSqlApplied(_) => Ok(()),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "执行失败".to_string(),
                            message: "用户权限 SQL 执行没有返回结果".to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._user_admin_apply_tasks.remove(&tab_id);
                    let success = result.is_ok();
                    this.dispatch(
                        AppCommand::FinishUserAdminSqlApply { tab_id, result },
                        cx,
                    );
                    if success {
                        this.show_message("用户权限已更新", AppMessageKind::Success, cx);
                        this.start_user_admin_users_load(tab_id, cx);
                        if let Some(admin) = this.user_admin_state_for(tab_id)
                            && admin.active_detail_tab == UserAdminDetailTab::MemberOf
                            && let Some(user) = user_admin_grants_load_user(&admin)
                        {
                            this.start_user_admin_grants_load(tab_id, user, cx);
                        }
                        if let Some(admin) = this.user_admin_state_for(tab_id)
                            && admin.active_detail_tab == UserAdminDetailTab::MemberOf
                            && let Some(role) = user_admin_members_load_role(&admin)
                        {
                            this.start_user_admin_member_grants_load(tab_id, role, cx);
                        }
                        if let Some(admin) = this.user_admin_state_for(tab_id)
                            && admin.active_detail_tab == UserAdminDetailTab::Members
                            && let Some(role) = user_admin_members_load_role(&admin)
                        {
                            this.start_user_admin_member_grants_load(tab_id, role, cx);
                        }
                    }
                });
            });
        });
        self._user_admin_apply_tasks.insert(tab_id, task);
    }

    fn open_user_admin_for_connection(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        let event = self.dispatch(AppCommand::OpenUserAdmin(connection_id), cx);
        let tab_id = match event {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => Some(tab_id),
            _ => None,
        };
        if let Some(tab_id) = tab_id {
            self.start_user_admin_users_load(tab_id, cx);
            self.start_user_admin_database_options_load(tab_id, cx);
        }
    }

    fn start_user_admin_database_options_load(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return;
        };
        let should_load = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == admin.connection_id)
            .is_some_and(|connection| {
                supports_database_user_admin(connection.config.kind) && connection.objects.is_empty()
            });
        if should_load {
            self.open_connection_from_sidebar(admin.connection_id, cx);
        }
    }

    fn preview_user_admin_sql(
        &mut self,
        tab_id: TabId,
        sql: String,
        danger: bool,
        cx: &mut Context<Self>,
    ) {
        if sql.trim().is_empty() {
            return;
        }
        self.dispatch(AppCommand::PreviewUserAdminSql { tab_id, sql, danger }, cx);
    }

    fn preview_user_admin_all_sql(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(provider) = self.user_admin_provider(tab_id) else {
            return;
        };
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return;
        };
        let draft = user_admin_all_sql_preview(provider, &admin);
        self.preview_user_admin_sql(tab_id, draft.sql(), draft.danger, cx);
    }

    fn user_admin_state_for(&self, tab_id: TabId) -> Option<UserAdminState> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::UserAdmin(admin) => Some(admin.clone()),
                _ => None,
            })
    }

    fn user_admin_provider(&self, tab_id: TabId) -> Option<fluxdb_core::DatabaseUserAdminProvider> {
        let admin = self.user_admin_state_for(tab_id)?;
        let kind = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == admin.connection_id)
            .map(|connection| connection.config.kind)?;
        database_user_admin_provider(kind)
    }

    fn toggle_user_admin_password_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.user_admin_password_visible = !self.user_admin_password_visible;
        let visible = self.user_admin_password_visible;
        self.user_admin_new_password_input.update(cx, |input, cx| {
            input.set_masked(!visible, window, cx);
        });
        self.user_admin_create_password_input.update(cx, |input, cx| {
            input.set_masked(!visible, window, cx);
        });
        cx.notify();
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct UserAdminSqlPreviewDraft {
    statements: Vec<String>,
    danger: bool,
}

impl UserAdminSqlPreviewDraft {
    fn sql(&self) -> String {
        self.statements.join("\n")
    }

    fn extend(&mut self, other: UserAdminSqlPreviewDraft) {
        self.danger |= other.danger;
        self.statements.extend(other.statements);
    }
}

fn user_admin_general_sql_preview(
    provider: fluxdb_core::DatabaseUserAdminProvider,
    admin: &UserAdminState,
) -> Option<UserAdminSqlPreviewDraft> {
    if !user_admin_general_form_valid(admin) {
        return None;
    }
    let mut statements = Vec::new();
    let user = if admin.creating_user {
        let user = admin.draft_user_identity();
        statements.push(provider.create_user_sql(&CreatePrincipalInput {
            user: admin.create_user.trim().to_string(),
            host: admin.create_host.trim().to_string(),
            password: admin.new_password.clone(),
            auth_plugin: Some(admin.auth_plugin.clone()),
        }));
        user
    } else {
        let user = admin.selected_user.clone()?;
        let password_changed = !admin.new_password.is_empty();
        let plugin_changed = user_admin_auth_plugin_dirty(admin, &user);
        if plugin_changed {
            statements.push(provider.alter_auth_plugin_sql(
                &user,
                &admin.auth_plugin,
                password_changed.then_some(admin.new_password.as_str()),
            ));
        } else if password_changed {
            statements.push(provider.alter_password_sql(&user, &admin.new_password));
        }
        user
    };
    if admin.password_expiry_policy != "DEFAULT"
        && let Some(sql) = provider.alter_password_expiry_sql(&user, &admin.password_expiry_policy)
    {
        statements.push(sql);
    }

    (!statements.is_empty()).then_some(UserAdminSqlPreviewDraft {
        statements,
        danger: !admin.creating_user,
    })
}

fn user_admin_advanced_sql_preview(
    provider: fluxdb_core::DatabaseUserAdminProvider,
    admin: &UserAdminState,
) -> UserAdminSqlPreviewDraft {
    let user = if admin.creating_user {
        if !user_admin_general_form_valid(admin) {
            return UserAdminSqlPreviewDraft::default();
        }
        admin.draft_user_identity()
    } else {
        let Some(user) = admin.selected_user.clone() else {
            return UserAdminSqlPreviewDraft::default();
        };
        user
    };
    let mut statements = Vec::new();
    let limits = UserResourceLimits {
        max_queries_per_hour: user_admin_limit_value(&admin.max_queries_per_hour),
        max_updates_per_hour: user_admin_limit_value(&admin.max_updates_per_hour),
        max_connections_per_hour: user_admin_limit_value(&admin.max_connections_per_hour),
        max_user_connections: user_admin_limit_value(&admin.max_user_connections),
    };
    if let Some(sql) = provider.alter_resource_limits_sql(&user, &limits) {
        statements.push(sql);
    }
    if let Some(sql) = provider.alter_ssl_requirement_sql(
        &user,
        &admin.ssl_type,
        &admin.ssl_cipher,
        &admin.ssl_issuer,
        &admin.ssl_subject,
    ) {
        statements.push(sql);
    }
    UserAdminSqlPreviewDraft {
        statements,
        danger: false,
    }
}

fn user_admin_limit_value(value: &str) -> Option<u64> {
    let value = value.trim().parse::<u64>().ok()?;
    (value > 0).then_some(value)
}

fn user_admin_auth_plugin_dirty(admin: &UserAdminState, user: &DatabaseUserIdentity) -> bool {
    let plugin = admin.auth_plugin.trim();
    !plugin.is_empty()
        && user
            .plugin
            .as_deref()
            .unwrap_or("caching_sha2_password")
            != plugin
}

fn user_admin_general_form_valid(admin: &UserAdminState) -> bool {
    let account_ready = !admin.creating_user
        || (!admin.create_user.trim().is_empty() && !admin.create_host.trim().is_empty());
    let password_ready = admin.new_password == admin.create_password;
    account_ready && password_ready && (!admin.creating_user || !admin.new_password.is_empty())
}

fn user_admin_member_relationship_sql_preview(
    provider: fluxdb_core::DatabaseUserAdminProvider,
    admin: &UserAdminState,
) -> UserAdminSqlPreviewDraft {
    let mut draft = user_admin_member_of_sql_preview(provider, admin);
    draft.extend(user_admin_members_sql_preview(provider, admin));
    draft
}

fn user_admin_member_of_sql_preview(
    provider: fluxdb_core::DatabaseUserAdminProvider,
    admin: &UserAdminState,
) -> UserAdminSqlPreviewDraft {
    let Some(user) = user_admin_selected_existing_user(admin) else {
        return UserAdminSqlPreviewDraft::default();
    };
    let base = role_memberships_from_grants(&admin.users, &admin.grants, &user);
    let effective = admin.effective_role_memberships();
    let mut statements = Vec::new();
    for membership in &effective {
        let Some(base_membership) = base
            .iter()
            .find(|base_membership| base_membership.role == membership.role)
        else {
            continue;
        };
        if base_membership.granted != membership.granted {
            if membership.granted {
                statements.push(provider.grant_role_sql(&membership.role, &user));
            } else {
                statements.push(provider.revoke_role_sql(&membership.role, &user));
            }
        }
    }
    let default_changed = effective.iter().any(|membership| {
        base.iter()
            .find(|base_membership| base_membership.role == membership.role)
            .is_some_and(|base_membership| base_membership.default_role != membership.default_role)
    });
    if default_changed {
        let default_roles = effective
            .iter()
            .filter(|membership| membership.default_role)
            .map(|membership| membership.role.clone())
            .collect::<Vec<_>>();
        statements.push(provider.set_default_roles_sql(&user, &default_roles));
    }

    let danger = statements
        .iter()
        .any(|statement| statement.starts_with("REVOKE ") || statement.contains("SET DEFAULT ROLE NONE"));
    UserAdminSqlPreviewDraft { statements, danger }
}

fn user_admin_members_sql_preview(
    provider: fluxdb_core::DatabaseUserAdminProvider,
    admin: &UserAdminState,
) -> UserAdminSqlPreviewDraft {
    let Some(role) = user_admin_selected_existing_user(admin) else {
        return UserAdminSqlPreviewDraft::default();
    };
    let base = admin.member_grants.clone();
    let effective = admin.effective_role_members();
    let mut statements = Vec::new();
    for member in &effective {
        let Some(base_member) = base.iter().find(|base| base.member == member.member) else {
            continue;
        };
        if base_member.granted != member.granted {
            if member.granted {
                statements.push(provider.grant_role_sql(&role, &member.member));
            } else {
                statements.push(provider.revoke_role_sql(&role, &member.member));
            }
        }
    }
    let danger = statements.iter().any(|statement| statement.starts_with("REVOKE "));
    UserAdminSqlPreviewDraft { statements, danger }
}

fn user_admin_all_sql_preview(
    provider: fluxdb_core::DatabaseUserAdminProvider,
    admin: &UserAdminState,
) -> UserAdminSqlPreviewDraft {
    if !user_admin_general_form_valid(admin) {
        return UserAdminSqlPreviewDraft::default();
    }
    let mut draft = UserAdminSqlPreviewDraft::default();
    if let Some(general) = user_admin_general_sql_preview(provider, admin) {
        draft.extend(general);
    }
    draft.extend(user_admin_advanced_sql_preview(provider, admin));
    draft.extend(user_admin_member_relationship_sql_preview(provider, admin));
    draft.extend(user_admin_privileges_sql_preview(provider, admin));
    draft
}

fn user_admin_can_save_all(
    provider: fluxdb_core::DatabaseUserAdminProvider,
    admin: &UserAdminState,
) -> bool {
    !admin.applying && !user_admin_all_sql_preview(provider, admin).statements.is_empty()
}

fn user_admin_content(
    state: &AppState,
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    sync_user_admin_inputs(admin, this, window, cx);
    let connection = state
        .connections
        .iter()
        .find(|connection| connection.config.id == admin.connection_id);
    let provider = connection.and_then(|connection| database_user_admin_provider(connection.config.kind));
    let supported = provider.is_some();
    let connection_name = connection
        .map(|connection| connection.config.name.clone())
        .unwrap_or_else(|| "连接".to_string());

    let mut root = div()
        .relative()
        .flex_1()
        .bg(colors.content_bg)
        .border_r_1()
        .border_color(colors.border)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(user_admin_toolbar(
            tab_id,
            admin,
            connection_name,
            provider,
            colors,
            cx,
        ));

    if !supported {
        return root.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .px_6()
                .text_align(gpui::TextAlign::Center)
                .text_size(px(14.))
                .text_color(colors.muted)
                .child("当前支持 MySQL/TiDB 兼容连接。SQLite、MongoDB、Redis 会按各自权限模型继续扩展。"),
        );
    }

    root = root.child(
        div()
            .flex()
            .min_h(px(0.))
            .flex_1()
            .child(
                user_admin_user_list(tab_id, admin, this, colors, cx)
                    .w(px(320.))
                    .flex_none(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(user_admin_detail(tab_id, state, admin, this, window, colors, cx)),
            ),
    );

    if let Some(pending) = &admin.pending_sql {
        root = root.child(user_admin_sql_preview_modal(
            tab_id,
            pending,
            admin.applying,
            admin.apply_error.as_ref(),
            window,
            colors,
            cx,
        ));
    }

    root
}

fn sync_user_admin_inputs(
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    sync_input_value(&this.user_admin_search_input, &admin.search, window, cx);
    sync_input_value(&this.user_admin_create_user_input, &admin.create_user, window, cx);
    sync_input_value(&this.user_admin_create_host_input, &admin.create_host, window, cx);
    sync_select_value(&this.user_admin_auth_plugin_select, &admin.auth_plugin, window, cx);
    sync_select_value(
        &this.user_admin_password_expiry_select,
        &admin.password_expiry_policy,
        window,
        cx,
    );
    sync_input_value(&this.user_admin_new_password_input, &admin.new_password, window, cx);
    sync_input_value(
        &this.user_admin_create_password_input,
        &admin.create_password,
        window,
        cx,
    );
    sync_input_value(
        &this.user_admin_max_queries_input,
        &admin.max_queries_per_hour,
        window,
        cx,
    );
    sync_input_value(
        &this.user_admin_max_updates_input,
        &admin.max_updates_per_hour,
        window,
        cx,
    );
    sync_input_value(
        &this.user_admin_max_connections_input,
        &admin.max_connections_per_hour,
        window,
        cx,
    );
    sync_input_value(
        &this.user_admin_max_user_connections_input,
        &admin.max_user_connections,
        window,
        cx,
    );
    sync_select_value(&this.user_admin_ssl_type_select, &admin.ssl_type, window, cx);
    sync_input_value(&this.user_admin_ssl_cipher_input, &admin.ssl_cipher, window, cx);
    sync_input_value(&this.user_admin_ssl_issuer_input, &admin.ssl_issuer, window, cx);
    sync_input_value(&this.user_admin_ssl_subject_input, &admin.ssl_subject, window, cx);
}

fn sync_input_value(
    input: &Entity<InputState>,
    expected: &str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    if input.read(cx).value().as_ref() != expected {
        input.update(cx, |input, cx| input.set_value(expected.to_string(), window, cx));
    }
}

fn sync_select_value(
    select: &Entity<SelectState<SearchableVec<String>>>,
    expected: &str,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    if select.read(cx).selected_value().is_some_and(|value| value == expected) {
        return;
    }
    select.update(cx, |select, cx| {
        select.set_selected_value(&expected.to_string(), window, cx);
    });
}

fn user_admin_auth_plugin_options() -> Vec<String> {
    [
        "caching_sha2_password",
        "mysql_native_password",
        "sha256_password",
        "auth_socket",
        "mysql_clear_password",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn user_admin_password_expiry_options() -> Vec<String> {
    ["DEFAULT", "NEVER", "INTERVAL 90 DAY", "EXPIRE NOW"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn user_admin_ssl_type_options() -> Vec<String> {
    ["NONE", "ANY", "SPECIFIED", "X509"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn user_admin_toolbar(
    tab_id: TabId,
    admin: &UserAdminState,
    connection_name: String,
    provider: Option<fluxdb_core::DatabaseUserAdminProvider>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let supported = provider.is_some();
    let can_save = provider.is_some_and(|provider| user_admin_can_save_all(provider, admin));
    div()
        .h(px(44.))
        .px_3()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .gap_2()
        .text_color(colors.text)
        .child(app_icon_box(AppIcon::Users, 28., 16., rgb(0x2563eb)))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("用户与权限"),
        )
        .child(user_admin_badge(connection_name, colors))
        .child(div().flex_1())
        .child(
            user_admin_button(
                if admin.applying { "保存中" } else { "保存" },
                AppIcon::Save,
                false,
                !can_save,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if supported {
                        this.preview_user_admin_all_sql(tab_id, cx);
                    }
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button(
                if admin.loading_users { "加载中" } else { "刷新" },
                AppIcon::Refresh,
                false,
                !supported || admin.loading_users,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if supported {
                        this.start_user_admin_users_load(tab_id, cx);
                    }
                    cx.stop_propagation();
                }),
            ),
        )
}

fn user_admin_badge(text: String, colors: UiColors) -> Div {
    div()
        .max_w(px(180.))
        .h(px(22.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .overflow_hidden()
        .text_size(px(11.))
        .text_color(colors.muted)
        .child(div().truncate().child(text))
}

fn user_admin_user_list(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let query = admin.search.trim().to_lowercase();
    let users = admin
        .users
        .iter()
        .filter(|user| {
            query.is_empty()
                || format!("{}@{}", user.user, user.host)
                    .to_lowercase()
                    .contains(&query)
                || user
                    .plugin
                    .as_ref()
                    .is_some_and(|plugin| plugin.to_lowercase().contains(&query))
        })
        .cloned()
        .collect::<Vec<_>>();
    let draft_user = admin.draft_user_identity();
    let draft_matches = admin.creating_user
        && (query.is_empty()
            || format!("{}@{}", draft_user.user, draft_user.host)
                .to_lowercase()
                .contains(&query)
            || draft_user
                .plugin
                .as_ref()
                .is_some_and(|plugin| plugin.to_lowercase().contains(&query)));
    let mut list = div().flex_1().min_h(px(0.)).overflow_y_scrollbar();
    if admin.loading_users {
        list = list.child(user_admin_empty_row("正在加载用户...", colors));
    } else if let Some(error) = &admin.users_error {
        list = list.child(user_admin_error_row(&error.message, colors));
    } else if users.is_empty() && !draft_matches {
        list = list.child(user_admin_empty_row("暂无用户，或当前账号没有读取用户列表的权限。", colors));
    } else {
        for user in users {
            let active = admin.selected_user.as_ref() == Some(&user);
            list = list.child(user_admin_user_row(tab_id, user, active, colors, cx));
        }
        if draft_matches {
            list = list.child(user_admin_draft_user_row(draft_user, colors));
        }
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
                .h(px(48.))
                .px_2()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .gap_2()
                .child(user_admin_input_box(this.user_admin_search_input.clone(), colors).flex_1())
                .child(
                    user_admin_add_user_button(admin.loading_users, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(admin) = this.user_admin_state_for(tab_id)
                                && !admin.loading_users
                            {
                                this.dispatch(AppCommand::BeginUserAdminCreateUser(tab_id), cx);
                            }
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
        .child(list)
}

fn user_admin_user_row(
    tab_id: TabId,
    user: DatabaseUserIdentity,
    active: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let title = if user.user.is_empty() {
        "匿名用户".to_string()
    } else {
        format!("{}@{}", user.user, user.host)
    };
    let detail = user.plugin.clone().unwrap_or_default();
    div()
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
        .hover(move |style| style.bg(if active { colors.tree_selected } else { colors.hover }))
        .flex()
        .items_center()
        .gap_2()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                let should_load_grants = this.user_admin_state_for(tab_id).is_some_and(|admin| {
                    matches!(
                        admin.active_detail_tab,
                        UserAdminDetailTab::MemberOf | UserAdminDetailTab::Privileges
                    )
                });
                let should_load_members = this.user_admin_state_for(tab_id).is_some_and(|admin| {
                    matches!(
                        admin.active_detail_tab,
                        UserAdminDetailTab::MemberOf | UserAdminDetailTab::Members
                    )
                });
                this.dispatch(
                    AppCommand::SelectUserAdminUser {
                        tab_id,
                        user: user.clone(),
                    },
                    cx,
                );
                if should_load_grants {
                    this.start_user_admin_grants_load(tab_id, user.clone(), cx);
                }
                if should_load_members {
                    this.start_user_admin_member_grants_load(tab_id, user.clone(), cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon_box(AppIcon::Users, 24., 15., colors.muted))
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
                .when(!detail.is_empty(), |this| {
                    this.child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(detail),
                    )
                }),
        )
}

fn user_admin_draft_user_row(user: DatabaseUserIdentity, colors: UiColors) -> Div {
    let title = if user.user.trim().is_empty() {
        "新用户@%".to_string()
    } else {
        format!("{}@{}", user.user, user.host)
    };
    let detail = user
        .plugin
        .clone()
        .unwrap_or_else(|| "caching_sha2_password".to_string());
    div()
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
                        .child(format!("待创建 · {}", detail)),
                ),
        )
}

fn user_admin_detail(
    tab_id: TabId,
    state: &AppState,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let user = if admin.creating_user {
        admin.draft_user_identity()
    } else if let Some(user) = admin.selected_user.clone() {
        user
    } else {
        return div()
            .min_w(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(13.))
            .text_color(colors.muted)
            .child("请选择用户");
    };

    div()
        .min_w(px(0.))
        .size_full()
        .bg(colors.content_bg)
        .flex()
        .flex_col()
        .child(user_admin_tab_strip(tab_id, admin.active_detail_tab, colors, cx))
        .child(match admin.active_detail_tab {
            UserAdminDetailTab::General => {
                user_admin_general_panel(tab_id, admin, user, this, window, colors, cx)
            }
            UserAdminDetailTab::Advanced => {
                user_admin_advanced_panel(tab_id, admin, this, window, colors, cx)
            }
            UserAdminDetailTab::MemberOf => {
                user_admin_member_relationships_panel(tab_id, admin, colors, cx)
            }
            UserAdminDetailTab::Members => user_admin_members_panel(tab_id, admin, colors, cx),
            UserAdminDetailTab::Privileges => {
                user_admin_privileges_panel(tab_id, state, admin, this, colors, cx)
            }
            UserAdminDetailTab::SqlPreview => {
                user_admin_sql_preview_panel(tab_id, state, admin, window, colors, cx)
            }
        })
}

fn user_admin_tab_strip(
    tab_id: TabId,
    active_tab: UserAdminDetailTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tabs = [
        (UserAdminDetailTab::General, "常规"),
        (UserAdminDetailTab::Advanced, "高级"),
        (UserAdminDetailTab::MemberOf, "成员关系"),
        (UserAdminDetailTab::Privileges, "权限"),
        (UserAdminDetailTab::SqlPreview, "SQL 预览"),
    ];
    let mut strip = div()
        .h(px(28.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_end();
    for (detail_tab, label) in tabs {
        strip = strip.child(user_admin_detail_tab(tab_id, detail_tab, label, active_tab == detail_tab, colors, cx));
    }
    strip.child(div().flex_1())
}

fn user_admin_detail_tab(
    tab_id: TabId,
    detail_tab: UserAdminDetailTab,
    label: &'static str,
    active: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(28.))
        .px_3()
        .border_r_1()
        .border_color(colors.border)
        .bg(if active { colors.panel_bg } else { colors.panel_alt })
        .cursor_pointer()
        .hover(move |style| style.bg(if active { colors.panel_bg } else { colors.hover }))
        .flex()
        .items_center()
        .text_size(px(12.))
        .font_weight(if active {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(if active { colors.text } else { colors.muted })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::SelectUserAdminDetailTab { tab_id, detail_tab },
                    cx,
                );
                this.user_admin_privilege_database_menu = None;
                if let Some(admin) = this.user_admin_state_for(tab_id) {
                    if let Some(user) = user_admin_detail_tab_grants_load_user(detail_tab, &admin) {
                        this.start_user_admin_grants_load(tab_id, user, cx);
                    }
                    if let Some(role) = user_admin_detail_tab_members_load_role(detail_tab, &admin)
                    {
                        this.start_user_admin_member_grants_load(tab_id, role, cx);
                    }
                    if detail_tab == UserAdminDetailTab::Privileges {
                        this.start_user_admin_database_options_load(tab_id, cx);
                    }
                }
                cx.stop_propagation();
            }),
        )
        .child(label)
}

fn user_admin_general_panel(
    _tab_id: TabId,
    admin: &UserAdminState,
    _user: DatabaseUserIdentity,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let password_mismatch =
        !admin.new_password.is_empty() && admin.new_password != admin.create_password;
    let account_incomplete =
        admin.creating_user && (admin.create_user.trim().is_empty() || admin.create_host.trim().is_empty());
    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .p_3()
        .child(
            div()
                .max_w(px(620.))
                .flex()
                .flex_col()
                .gap_3()
                .child(user_admin_text_input_row(
                    "用户名:",
                    this.user_admin_create_user_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_text_input_row(
                    "主机:",
                    this.user_admin_create_host_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_select_row(
                    "插件:",
                    this.user_admin_auth_plugin_select.clone(),
                    "选择认证插件",
                    colors,
                ))
                .child(user_admin_password_row(
                    "密码:",
                    this.user_admin_new_password_input.clone(),
                    this.user_admin_password_visible,
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_password_row(
                    "确认密码:",
                    this.user_admin_create_password_input.clone(),
                    this.user_admin_password_visible,
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_select_row(
                    "密码过期策略:",
                    this.user_admin_password_expiry_select.clone(),
                    "选择密码过期策略",
                    colors,
                ))
                .when(password_mismatch, |this| {
                    this.child(
                        div()
                            .ml(px(120.))
                            .text_size(px(12.))
                            .text_color(rgb(0xff3b45))
                            .child("两次输入的密码不一致"),
                    )
                })
                .when(account_incomplete, |this| {
                    this.child(
                        div()
                            .ml(px(120.))
                            .text_size(px(12.))
                            .text_color(rgb(0xff3b45))
                            .child("用户名和 Host 不能为空"),
                    )
                })
        )
}

fn user_admin_selected_existing_user(admin: &UserAdminState) -> Option<DatabaseUserIdentity> {
    (!admin.creating_user)
        .then(|| admin.selected_user.clone())
        .flatten()
}

fn user_admin_grants_load_user(admin: &UserAdminState) -> Option<DatabaseUserIdentity> {
    if admin.loading_grants || admin.grants_loaded_for_selected_user() {
        return None;
    }
    user_admin_selected_existing_user(admin)
}

fn user_admin_detail_tab_grants_load_user(
    detail_tab: UserAdminDetailTab,
    admin: &UserAdminState,
) -> Option<DatabaseUserIdentity> {
    (matches!(
        detail_tab,
        UserAdminDetailTab::MemberOf | UserAdminDetailTab::Privileges
    ))
    .then(|| user_admin_grants_load_user(admin))
    .flatten()
}

fn user_admin_members_load_role(admin: &UserAdminState) -> Option<DatabaseUserIdentity> {
    if admin.loading_member_grants || admin.member_grants_loaded_for_selected_role() {
        return None;
    }
    user_admin_selected_existing_user(admin)
}

fn user_admin_detail_tab_members_load_role(
    detail_tab: UserAdminDetailTab,
    admin: &UserAdminState,
) -> Option<DatabaseUserIdentity> {
    (matches!(
        detail_tab,
        UserAdminDetailTab::MemberOf | UserAdminDetailTab::Members
    ))
    .then(|| user_admin_members_load_role(admin))
    .flatten()
}

fn user_admin_text_input_row(
    label: &'static str,
    input: Entity<InputState>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(user_admin_form_label(label, colors))
        .child(user_admin_form_input_box(input, window, colors, cx))
}

fn user_admin_select_row(
    label: &'static str,
    select: Entity<SelectState<SearchableVec<String>>>,
    placeholder: &'static str,
    colors: UiColors,
) -> Div {
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(user_admin_form_label(label, colors))
        .child(user_admin_select_box(select, placeholder, colors))
}

fn user_admin_select_box(
    select: Entity<SelectState<SearchableVec<String>>>,
    placeholder: &'static str,
    colors: UiColors,
) -> Div {
    div()
        .w(px(360.))
        .h(px(34.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .flex()
        .items_center()
        .hover(move |style| style.border_color(user_admin_input_hover_border_color(false, colors)))
        .child(
            Select::new(&select)
                .placeholder(placeholder)
                .appearance(false)
                .w_full()
                .h_full()
                .menu_width(px(360.)),
        )
}

fn user_admin_advanced_panel(
    _tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let ssl_cipher_input = this.user_admin_ssl_cipher_input.clone();
    let ssl_issuer_input = this.user_admin_ssl_issuer_input.clone();
    let ssl_subject_input = this.user_admin_ssl_subject_input.clone();
    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .p_3()
        .child(
            div()
                .max_w(px(620.))
                .flex()
                .flex_col()
                .gap_3()
                .child(user_admin_text_input_row(
                    "每小时最大查询数:",
                    this.user_admin_max_queries_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_text_input_row(
                    "每小时最大更新数:",
                    this.user_admin_max_updates_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_text_input_row(
                    "每小时最大连接数:",
                    this.user_admin_max_connections_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_text_input_row(
                    "最大用户连接数:",
                    this.user_admin_max_user_connections_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_select_row(
                    "SSL 类型:",
                    this.user_admin_ssl_type_select.clone(),
                    "选择 SSL 类型",
                    colors,
                ))
                .when(admin.ssl_type == "SPECIFIED", |this| {
                    this.child(user_admin_text_input_row(
                        "SSL Cipher:",
                        ssl_cipher_input,
                        window,
                        colors,
                        cx,
                    ))
                    .child(user_admin_text_input_row(
                        "证书发行者:",
                        ssl_issuer_input,
                        window,
                        colors,
                        cx,
                    ))
                    .child(user_admin_text_input_row(
                        "证书主旨:",
                        ssl_subject_input,
                        window,
                        colors,
                        cx,
                    ))
                }),
        )
}

fn user_admin_member_relationships_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if admin.creating_user {
        return user_admin_placeholder_panel_with_text("请先保存新用户，再配置成员关系。", colors);
    }

    let memberships = admin.effective_role_memberships();
    let mut member_of_body = div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .overflow_y_scrollbar();
    if admin.loading_grants {
        member_of_body = member_of_body.child(user_admin_empty_row("正在加载成员属于...", colors));
    } else if let Some(error) = &admin.grants_error {
        member_of_body = member_of_body.child(user_admin_error_row(&error.message, colors));
    } else if memberships.is_empty() {
        member_of_body = member_of_body.child(user_admin_empty_row("暂无可授予的用户或角色。", colors));
    } else {
        for membership in memberships {
            member_of_body =
                member_of_body.child(user_admin_member_of_row(tab_id, membership, colors, cx));
        }
    }

    let members = admin.effective_role_members();
    let mut members_body = div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .overflow_y_scrollbar();
    if admin.loading_member_grants {
        members_body = members_body.child(user_admin_empty_row("正在加载成员...", colors));
    } else if let Some(error) = &admin.member_grants_error {
        members_body = members_body.child(user_admin_error_row(&error.message, colors));
    } else if members.is_empty() {
        members_body = members_body.child(user_admin_empty_row("暂无可授予的成员。", colors));
    } else {
        for member in members {
            members_body = members_body.child(user_admin_member_row(tab_id, member, colors, cx));
        }
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .p_3()
        .child(
            div()
                .size_full()
                .flex()
                .gap_3()
                .child(
                    user_admin_member_table_shell(
                        "成员属于",
                        user_admin_member_of_header(colors),
                        member_of_body,
                        colors,
                    )
                    .flex_1()
                    .min_w(px(0.)),
                )
                .child(
                    user_admin_member_table_shell(
                        "成员",
                        user_admin_members_header(colors),
                        members_body,
                        colors,
                    )
                    .flex_1()
                    .min_w(px(0.)),
                ),
        )
}

fn user_admin_member_table_shell(
    title: &'static str,
    header: Div,
    body: impl IntoElement,
    colors: UiColors,
) -> Div {
    div()
        .h_full()
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(34.))
                .px_3()
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(title),
        )
        .child(header)
        .child(body)
}

fn user_admin_members_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if admin.creating_user {
        return user_admin_placeholder_panel_with_text("请先保存新用户，再配置成员。", colors);
    }
    let members = admin.effective_role_members();
    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .overflow_y_scrollbar();
    if admin.loading_member_grants {
        body = body.child(user_admin_empty_row("正在加载成员...", colors));
    } else if let Some(error) = &admin.member_grants_error {
        body = body.child(user_admin_error_row(&error.message, colors));
    } else if members.is_empty() {
        body = body.child(user_admin_empty_row("暂无可授予的成员。", colors));
    } else {
        for member in members {
            body = body.child(user_admin_member_row(tab_id, member, colors, cx));
        }
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .p_3()
        .child(
            div()
                .max_w(px(560.))
                .h_full()
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .flex()
                .flex_col()
                .child(user_admin_members_header(colors))
                .child(body),
        )
}

fn user_admin_member_of_header(colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(div().flex_1().min_w(px(0.)).px_3().child("用户名"))
        .child(
            div()
                .w(px(72.))
                .border_l_1()
                .border_color(colors.border)
                .flex()
                .justify_center()
                .child("授予"),
        )
        .child(
            div()
                .w(px(72.))
                .border_l_1()
                .border_color(colors.border)
                .flex()
                .justify_center()
                .child("集"),
        )
}

fn user_admin_members_header(colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(div().flex_1().min_w(px(0.)).px_3().child("用户名"))
        .child(
            div()
                .w(px(72.))
                .border_l_1()
                .border_color(colors.border)
                .flex()
                .justify_center()
                .child("授予"),
        )
}

fn user_admin_member_of_row(
    tab_id: TabId,
    membership: fluxdb_core::UserRoleMembership,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let role = membership.role.clone();
    let role_for_grant = role.clone();
    let role_for_default = role.clone();
    let role_label = format!("{}@{}", role.user, role.host);
    let detail = role.plugin.unwrap_or_default();
    let role_id = user_admin_role_row_id(&role_for_grant);
    div()
        .h(px(34.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .hover(move |style| style.bg(colors.hover))
        .flex()
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Users, 14., colors.muted))
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .truncate()
                        .text_size(px(13.))
                        .text_color(colors.text)
                        .child(role_label),
                )
                .when(!detail.is_empty(), |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(detail),
                    )
                }),
        )
        .child(user_admin_member_checkbox_cell(
            "user-admin-member-grant",
            role_id,
            membership.granted,
            colors,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::SetUserAdminRoleMembershipGranted {
                        tab_id,
                        role: role_for_grant.clone(),
                        granted: !membership.granted,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        ))
        .child(user_admin_member_checkbox_cell(
            "user-admin-member-default",
            role_id,
            membership.default_role,
            colors,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::SetUserAdminRoleMembershipDefault {
                        tab_id,
                        role: role_for_default.clone(),
                        default_role: !membership.default_role,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        ))
}

fn user_admin_member_row(
    tab_id: TabId,
    member: fluxdb_core::UserRoleMember,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let user = member.member.clone();
    let user_for_grant = user.clone();
    let user_label = format!("{}@{}", user.user, user.host);
    let detail = user.plugin.unwrap_or_default();
    let user_id = user_admin_role_row_id(&user_for_grant);
    div()
        .h(px(34.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .hover(move |style| style.bg(colors.hover))
        .flex()
        .items_center()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Users, 14., colors.muted))
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .truncate()
                        .text_size(px(13.))
                        .text_color(colors.text)
                        .child(user_label),
                )
                .when(!detail.is_empty(), |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(detail),
                    )
                }),
        )
        .child(user_admin_member_checkbox_cell(
            "user-admin-member-grant-to-role",
            user_id,
            member.granted,
            colors,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    AppCommand::SetUserAdminRoleMemberGranted {
                        tab_id,
                        member: user_for_grant.clone(),
                        granted: !member.granted,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        ))
}

fn user_admin_member_checkbox_cell(
    id_prefix: &'static str,
    id: u64,
    checked: bool,
    colors: UiColors,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id((id_prefix, id))
        .w(px(72.))
        .h_full()
        .border_l_1()
        .border_color(colors.border)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(MouseButton::Left, on_click)
        .flex()
        .items_center()
        .justify_center()
        .child(user_admin_check_box_visual(checked, colors))
}

fn user_admin_role_row_id(role: &DatabaseUserIdentity) -> u64 {
    let mut hasher = DefaultHasher::new();
    role.user.hash(&mut hasher);
    role.host.hash(&mut hasher);
    hasher.finish()
}

fn user_admin_check_box_visual(checked: bool, colors: UiColors) -> Div {
    div()
        .size(px(16.))
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(if checked { rgb(0x2563eb) } else { colors.border })
        .bg(if checked { rgb(0x2563eb) } else { colors.input_bg })
        .shadow(vec![box_shadow(
            px(0.),
            px(1.),
            px(3.),
            px(0.),
            hsla(0., 0., 0., 0.14),
        )])
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |this| this.child(app_icon(AppIcon::Check, 12., rgb(0xffffff))))
}

fn user_admin_placeholder_panel(colors: UiColors) -> Div {
    user_admin_placeholder_panel_with_text("后续实现", colors)
}

fn user_admin_placeholder_panel_with_text(text: &'static str, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(text)
}

fn user_admin_sql_preview_panel(
    tab_id: TabId,
    state: &AppState,
    admin: &UserAdminState,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql = state
        .connections
        .iter()
        .find(|connection| connection.config.id == admin.connection_id)
        .and_then(|connection| database_user_admin_provider(connection.config.kind))
        .map(|provider| user_admin_all_sql_preview(provider, admin).sql())
        .unwrap_or_default();

    div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .p_3()
        .flex()
        .flex_col()
        .child(if sql.trim().is_empty() {
            user_admin_placeholder_panel_with_text("暂无待预览 SQL", colors)
        } else {
            user_admin_sql_preview_code_view(
                SharedString::from(format!("user-admin-sql-preview-tab-{}", tab_id.0)),
                &sql,
                window,
                colors,
                cx,
            )
        })
}

fn user_admin_sql_preview_code_view(
    editor_key: SharedString,
    sql: &str,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let editor = window.use_keyed_state(editor_key, cx, {
        let sql = sql.to_string();
        move |window, cx| {
            InputState::new(window, cx)
                .code_editor(SQL_HIGHLIGHT_LANGUAGE)
                .line_number(false)
                .legacy_soft_wrap(false)
                .default_value(sql)
        }
    });
    editor.update(cx, |state, cx| {
        if state.value().to_string() != sql {
            state.set_value(sql.to_string(), window, cx);
        }
    });

    div()
        .size_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .child(
            Input::new(&editor)
                .appearance(false)
                .bordered(false)
                .focus_bordered(false)
                .disabled(true)
                .text_size(px(12.))
                .font_family(EDITOR_FONT)
                .p_3()
                .size_full(),
        )
}

fn user_admin_form_input_box(
    input: Entity<InputState>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    div()
        .w(px(360.))
        .h(px(34.))
        .rounded(colors.radius)
        .border_1()
        .border_color(user_admin_input_border_color(focused, colors))
        .bg(colors.input_bg)
        .shadow(user_admin_input_shadow(focused))
        .overflow_hidden()
        .flex()
        .items_center()
        .hover(move |style| {
            style.border_color(user_admin_input_hover_border_color(focused, colors))
        })
        .child(
            Input::new(&input)
                .small()
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .px_2()
                .text_size(px(13.))
                .text_color(colors.text),
        )
}

fn user_admin_password_row(
    label: &'static str,
    input: Entity<InputState>,
    visible: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    div()
        .h(px(38.))
        .flex()
        .items_center()
        .gap_3()
        .child(user_admin_form_label(label, colors))
        .child(
            div()
                .w(px(360.))
                .h(px(34.))
                .rounded(colors.radius)
                .border_1()
                .border_color(user_admin_input_border_color(focused, colors))
                .bg(colors.input_bg)
                .shadow(user_admin_input_shadow(focused))
                .overflow_hidden()
                .flex()
                .items_center()
                .hover(move |style| {
                    style.border_color(user_admin_input_hover_border_color(focused, colors))
                })
                .child(
                    Input::new(&input)
                        .small()
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .px_2()
                        .text_size(px(13.))
                        .text_color(colors.text),
                )
                .child(user_admin_password_eye_button(visible, colors, cx)),
        )
}

fn user_admin_input_border_color(focused: bool, colors: UiColors) -> gpui::Rgba {
    if focused {
        if colors.is_dark { rgb(0x8ab4ff) } else { rgb(0x111111) }
    } else {
        colors.border
    }
}

fn user_admin_input_hover_border_color(focused: bool, colors: UiColors) -> gpui::Rgba {
    if focused {
        user_admin_input_border_color(true, colors)
    } else if colors.is_dark {
        rgb(0x5b6675)
    } else {
        rgb(0xb8c0cc)
    }
}

fn user_admin_input_shadow(focused: bool) -> Vec<gpui::BoxShadow> {
    vec![box_shadow(
        px(0.),
        px(1.),
        px(4.),
        px(0.),
        hsla(0., 0., 0., if focused { 0.08 } else { 0.11 }),
    )]
}

fn user_admin_form_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .w(px(108.))
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.text)
        .child(label)
}

fn user_admin_password_eye_button(
    visible: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .size(px(28.))
        .mr_1()
        .rounded(colors.radius * 0.5)
        .cursor_pointer()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.toggle_user_admin_password_visibility(window, cx);
                cx.stop_propagation();
            }),
        )
        .child(app_icon(
            if visible { AppIcon::EyeOff } else { AppIcon::Eye },
            14.,
            colors.muted,
        ))
}

fn user_admin_sql_preview_modal(
    tab_id: TabId,
    pending: &fluxdb_app::UserAdminPendingSql,
    applying: bool,
    error: Option<&fluxdb_core::UserFacingError>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql = pending.sql.clone();
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
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::ClearUserAdminPendingSql(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(680.))
                .max_w(px(680.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., 0.22),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(48.))
                        .px_4()
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(app_icon_box(
                            if pending.danger {
                                AppIcon::Trash
                            } else {
                                AppIcon::FileSql
                            },
                            24.,
                            16.,
                            if pending.danger {
                                rgb(0xff3b45)
                            } else {
                                rgb(0x2563eb)
                            },
                        ))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_size(px(15.))
                                .child("SQL 预览"),
                        )
                        .child(div().flex_1())
                        .child(
                            user_admin_icon_button(AppIcon::Close, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::ClearUserAdminPendingSql(tab_id), cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                )
                .child(
                    div()
                        .m_4()
                        .max_h(px(280.))
                        .h(px(280.))
                        .child(user_admin_sql_preview_code_view(
                            SharedString::from(format!("user-admin-sql-preview-modal-{}", tab_id.0)),
                            &sql,
                            window,
                            colors,
                            cx,
                        )),
                )
                .when_some(error, |this, error| {
                    this.child(
                        div()
                            .px_4()
                            .pb_2()
                            .text_size(px(12.))
                            .text_color(rgb(0xff3b45))
                            .child(error.message.clone()),
                    )
                })
                .child(
                    div()
                        .h(px(58.))
                        .px_4()
                        .border_t_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            user_admin_button("取消", AppIcon::Close, false, applying, colors)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.dispatch(AppCommand::ClearUserAdminPendingSql(tab_id), cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        )
                        .child(
                            user_admin_button(
                                if applying { "执行中" } else { "执行 SQL" },
                                AppIcon::Play,
                                pending.danger,
                                applying,
                                colors,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if !applying {
                                        this.start_user_admin_sql_apply(tab_id, sql.clone(), cx);
                                    }
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}

fn user_admin_icon_button(icon: AppIcon, colors: UiColors) -> Div {
    div()
        .size(px(28.))
        .rounded(colors.radius)
        .cursor_pointer()
        .hover(|style| style.bg(colors.hover))
        .flex()
        .items_center()
        .justify_center()
        .child(app_icon(icon, 16., colors.muted))
}

fn user_admin_add_user_button(disabled: bool, colors: UiColors) -> Stateful<Div> {
    let color = if disabled { colors.muted } else { rgb(0x2563eb) };
    div()
        .size(px(32.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(if disabled { colors.panel_alt } else { colors.input_bg })
        .cursor_pointer()
        .when(disabled, |this| this.opacity(0.55))
        .hover(move |style| if disabled { style } else { style.bg(colors.hover) })
        .flex()
        .items_center()
        .justify_center()
        .id("user-admin-add-user")
        .tooltip(|window, cx| Tooltip::new("新增用户").build(window, cx))
        .child(app_icon(AppIcon::Plus, 16., color))
}

fn user_admin_button(
    label: &'static str,
    icon: AppIcon,
    danger: bool,
    disabled: bool,
    colors: UiColors,
) -> Div {
    let fg = if danger {
        rgb(0xff3b45)
    } else if disabled {
        colors.muted
    } else {
        colors.text
    };
    div()
        .h(px(30.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(if disabled {
            colors.panel_alt
        } else {
            colors.input_bg
        })
        .cursor_pointer()
        .when(disabled, |this| this.opacity(0.55))
        .hover(move |style| if disabled { style } else { style.bg(colors.hover) })
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(fg)
        .child(app_icon_box(icon, 18., 14., fg))
        .child(label)
}

fn user_admin_input_box(input: Entity<InputState>, colors: UiColors) -> Div {
    div()
        .w_full()
        .h(px(32.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .flex()
        .items_center()
        .child(
            Input::new(&input)
                .small()
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h(px(28.))
                .px_2()
                .text_size(px(13.)),
        )
}

fn user_admin_empty_row(text: &'static str, colors: UiColors) -> Div {
    div()
        .p_4()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(text)
}

fn user_admin_error_row(text: &str, _colors: UiColors) -> Div {
    div()
        .p_4()
        .text_size(px(12.))
        .text_color(rgb(0xff3b45))
        .child(text.to_string())
}
