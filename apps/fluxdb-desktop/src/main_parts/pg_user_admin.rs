// PostgreSQL 用户/角色管理面板（T27）。
//
// PG 角色是集群级身份（无 host/plugin/资源限制），不套 MySQL 的 SQL 预览/apply 流程。
// 角色 CRUD 经 AppCommand→连接器（CreatePgRole/DropPgRole/AlterPgRolePassword），成功后
// start_user_admin_users_load 刷新；成员关系经 list_role_membership（admin.grants）。
// 复用既有输入句柄（user_admin_create_user_input 等，已订阅写入 admin.create_user/password）
// 与既有列表/输入行 helper。MySQL user_admin 完整保留。

/// PG 角色管理面板：工具栏 + 可选新建表单 + 左侧角色列表 + 右侧角色详情（成员关系）。
fn pg_role_admin_content(
    tab_id: TabId,
    admin: &UserAdminState,
    connection_name: String,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut root = div()
        .relative()
        .flex_1()
        .bg(colors.content_bg)
        .flex()
        .flex_col()
        .child(pg_role_toolbar(tab_id, admin, connection_name, colors, cx));
    if admin.creating_user {
        root = root.child(pg_create_role_form(tab_id, admin, this, window, colors, cx));
    }
    if admin.pg_edit_mode != fluxdb_app::PgRoleEditMode::None {
        root = root.child(pg_role_edit_form(tab_id, admin, this, window, colors, cx));
    }
    root.child(
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
                    .child(pg_role_detail(tab_id, admin, this, window, colors, cx)),
            ),
    )
}

fn pg_role_toolbar(
    tab_id: TabId,
    admin: &UserAdminState,
    connection_name: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 内置超级/维护角色（postgres、pg_*）不提供删除入口，避免误删集群关键角色。
    let selected_role = admin.selected_user.as_ref().map(|u| u.user.clone());
    let deletable = selected_role
        .as_deref()
        .is_some_and(|name| name != "postgres" && !name.starts_with("pg_"));
    // 与删除一致：postgres/pg_* 不提供改密/重命名入口（避免改坏集群关键角色）。
    let editable = deletable;
    h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .child(
            div()
                .text_size(px(14.))
                .text_color(colors.text)
                .child(format!("PostgreSQL 角色 · {connection_name}")),
        )
        .child(
            Button::new("pg-role-create")
                .label("新建角色")
                .small()
                .rounded_md()
                .disabled(admin.creating_user)
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.dispatch(AppCommand::BeginUserAdminCreateUser(tab_id), cx);
                    cx.notify();
                })),
        )
        .when(deletable && !admin.creating_user, |this| {
            this.child(
                Button::new("pg-role-drop")
                    .label("删除角色")
                    .small()
                    .rounded_md()
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.pg_drop_selected_role(tab_id, cx);
                    })),
            )
        })
        .when(editable && !admin.creating_user && admin.pg_edit_mode == fluxdb_app::PgRoleEditMode::None, |this| {
            this.child(
                Button::new("pg-role-rename")
                    .label("重命名")
                    .small()
                    .rounded_md()
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.dispatch(AppCommand::BeginUserAdminPgRename(tab_id), cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("pg-role-password")
                    .label("改密")
                    .small()
                    .rounded_md()
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.dispatch(AppCommand::BeginUserAdminPgPassword(tab_id), cx);
                        cx.notify();
                    })),
            )
        })
        .child(
            div()
                .ml_auto()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("集群级角色（LOGIN/NOLOGIN），非 MySQL user@host"),
        )
}

/// 新建 PG 角色表单：角色名 + 可登录(LOGIN) + 密码 → CreatePgRole → 刷新。复用已订阅的输入句柄。
fn pg_create_role_form(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_login = admin.pg_can_login;
    h_flex()
        .items_center()
        .flex_wrap()
        .gap_3()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(user_admin_text_input_row(
            "角色名:",
            this.user_admin_create_user_input.clone(),
            window,
            colors,
            cx,
        ))
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("可登录"),
                )
                .child(
                    Switch::new("pg-role-login")
                        .checked(can_login)
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            let next = this
                                .user_admin_state_for(tab_id)
                                .map(|admin| !admin.pg_can_login)
                                .unwrap_or(false);
                            this.dispatch(
                                AppCommand::SetUserAdminPgCanLogin {
                                    tab_id,
                                    can_login: next,
                                },
                                cx,
                            );
                            cx.notify();
                        })),
                ),
        )
        .child(user_admin_password_row(
            "密码:",
            this.user_admin_new_password_input.clone(),
            this.user_admin_password_visible,
            window,
            colors,
            cx,
        ))
        .child(
            Button::new("pg-role-create-confirm")
                .label("创建")
                .small()
                .rounded_md()
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.pg_create_role(tab_id, cx);
                })),
        )
        .child(
            Button::new("pg-role-create-cancel")
                .label("取消")
                .small()
                .rounded_md()
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.pg_cancel_create_role(tab_id, cx);
                })),
        )
}

/// PG 角色内联编辑表单（重命名 / 改密）：
/// - Rename：新角色名输入（复用 user_admin_pg_rename_input）→ RenamePgRole；
/// - Password：新密码输入（复用 user_admin_new_password_input）→ AlterPgRolePassword。
/// 两模式共用「确定 / 取消」，Esc/外点由既有弹框策略覆盖（此处直接渲染为工具条下表单行）。
fn pg_role_edit_form(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let role = admin
        .selected_user
        .as_ref()
        .map(|u| u.user.clone())
        .unwrap_or_default();
    match admin.pg_edit_mode {
        fluxdb_app::PgRoleEditMode::Rename => h_flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_alt)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.muted)
                    .child(format!("重命名 {role} 为:")),
            )
            .child(user_admin_form_input_box(
                this.user_admin_pg_rename_input.clone(),
                window,
                colors,
                cx,
            ))
            .child(
                Button::new("pg-rename-confirm")
                    .label("确定")
                    .small()
                    .rounded_md()
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.pg_rename_role(tab_id, cx);
                    })),
            )
            .child(
                Button::new("pg-rename-cancel")
                    .label("取消")
                    .small()
                    .rounded_md()
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.dispatch(AppCommand::EndUserAdminPgEdit(tab_id), cx);
                        this.start_user_admin_users_load(tab_id, cx);
                        cx.notify();
                    })),
            ),
        fluxdb_app::PgRoleEditMode::Password => h_flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_alt)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.muted)
                    .child(format!("修改 {role} 密码:")),
            )
            .child(user_admin_password_row(
                "新密码:",
                this.user_admin_new_password_input.clone(),
                this.user_admin_password_visible,
                window,
                colors,
                cx,
            ))
            .child(
                Button::new("pg-password-confirm")
                    .label("确定")
                    .small()
                    .rounded_md()
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.pg_change_password(tab_id, cx);
                    })),
            )
            .child(
                Button::new("pg-password-cancel")
                    .label("取消")
                    .small()
                    .rounded_md()
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.dispatch(AppCommand::EndUserAdminPgEdit(tab_id), cx);
                        this.start_user_admin_users_load(tab_id, cx);
                        cx.notify();
                    })),
            ),
        fluxdb_app::PgRoleEditMode::None => div(),
    }
}

/// 角色选项：可登录（LOGIN）切换。postgres/pg_* 只读（与删除/改密/重命名一致）。
fn pg_role_login_toggle(
    tab_id: TabId,
    admin: &UserAdminState,
    role: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let editable = role != "postgres" && !role.starts_with("pg_");
    let role_owned = role.to_string(); // move 闭包需要归属所有
    let can_login = admin
        .pg_role_login
        .get(role)
        .copied()
        .unwrap_or(true); // 未知角色默认按 LOGIN 展示
    h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("可登录 (LOGIN)"),
        )
        .child(
            Switch::new(format!("pg-role-login-{role}"))
                .checked(can_login)
                .when(!editable, |s| s.disabled(true))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    let role = role_owned.clone();
                    let connection_id = this
                        .user_admin_state_for(tab_id)
                        .map(|a| a.connection_id);
                    let Some(connection_id) = connection_id else {
                        return;
                    };
                    let cur = this
                        .user_admin_state_for(tab_id)
                        .and_then(|a| a.pg_role_login.get(&role).copied())
                        .unwrap_or(true);
                    this.dispatch(
                        AppCommand::SetPgRoleLogin {
                            connection_id,
                            name: role.clone(),
                            can_login: !cur,
                        },
                        cx,
                    );
                    this.start_user_admin_users_load(tab_id, cx);
                    cx.notify();
                })),
        )
        .when(!editable, |d| {
            d.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child("（内置角色只读）"),
            )
        })
}

/// 右侧：选中 PG 角色详情（角色标识 + 成员关系 + 对象权限）。
fn pg_role_detail(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(selected) = admin.selected_user.clone() else {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.muted)
            .child("请选择角色");
    };

    let mut body = div()
        .flex_1()
        .min_w(px(0.))
        .px_4()
        .py_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_size(px(15.))
                        .text_color(colors.text)
                        .child(format!("角色：{}", selected.user)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("集群级身份"),
                ),
        )
        // 角色选项：可登录 LOGIN 切换（postgres/pg_* 只读，避免改坏集群关键角色）。
        .child(pg_role_login_toggle(tab_id, admin, &selected.user, colors, cx))
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("作为成员所属的组角色（读取自 pg_auth_members）："),
        );

    let loaded_for_selected = admin
        .grants_loaded_user
        .as_ref()
        .is_some_and(|u| u == &selected);
    if admin.loading_grants || !loaded_for_selected {
        body = body.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("正在加载成员关系…"),
        );
    } else if admin.grants.is_empty() {
        body = body.child(
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("（无组角色成员关系）"),
        );
    } else {
        let groups: Vec<String> = admin.grants.clone();
        for group in groups {
            body = body.child(
                div()
                    .text_size(px(13.))
                    .text_color(colors.text)
                    .child(group),
            );
        }
    }

    body.child(pg_role_privileges_panel(tab_id, admin, this, window, colors, cx))
}

/// 对象权限面板：目标选择（种类/schema/对象/签名）+ 读取 + 逐权限显示直接/继承与授权撤销。
fn pg_role_privileges_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let active_kind = admin.pg_grant_kind;

    // 对象种类选择（紧凑按钮组）。
    let mut kinds = h_flex().items_center().gap_1();
    for kind in fluxdb_app::PgGrantObjectKind::all() {
        let selected = kind == active_kind;
        kinds = kinds.child(
            Button::new(format!("pg-grant-kind-{}", kind.label()))
                .label(kind.label())
                .small()
                .rounded_md()
                .when(selected, |b| b.on_click(cx.listener(move |this, _, _window, cx| {
                    this.dispatch(
                        AppCommand::SetUserAdminPgGrantTarget {
                            tab_id,
                            kind,
                            schema: this
                                .user_admin_state_for(tab_id)
                                .map(|a| a.pg_grant_schema)
                                .unwrap_or_default(),
                            object: this
                                .user_admin_state_for(tab_id)
                                .map(|a| a.pg_grant_object)
                                .unwrap_or_default(),
                            signature: this
                                .user_admin_state_for(tab_id)
                                .map(|a| a.pg_grant_signature)
                                .unwrap_or_default(),
                        },
                        cx,
                    );
                    cx.notify();
                }))),
        );
    }

    let mut panel = div()
        .mt_2()
        .pt_3()
        .border_t_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.text)
                .child("对象权限（无需手写 SQL）"),
        )
        .child(kinds)
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .flex_wrap()
                .child(user_admin_form_input_box(
                    this.user_admin_pg_grant_schema_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_form_input_box(
                    this.user_admin_pg_grant_object_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(user_admin_form_input_box(
                    this.user_admin_pg_grant_signature_input.clone(),
                    window,
                    colors,
                    cx,
                ))
                .child(
                    Button::new("pg-grant-load")
                        .label("读取权限")
                        .small()
                        .rounded_md()
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            this.start_pg_object_grants_load(tab_id, cx);
                        })),
                ),
        );

    if admin.loading_pg_grants {
        panel = panel.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("正在读取权限…"),
        );
    } else if let Some(error) = &admin.pg_grants_error {
        panel = panel.child(
            div()
                .text_size(px(12.))
                .text_color(rgb(0xff3b45))
                .child(format!("读取失败：{}", error.message)),
        );
    } else if admin.pg_object_grants.is_none() {
        panel = panel.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("选择目标后点「读取权限」。"),
        );
    } else {
        // owner/默认权限概览（ACL 为 NULL 即默认权限，不是「无权限」）。
        if let Some(grants) = &admin.pg_object_grants {
            let summary = if grants.acl_is_null {
                format!("属主 {}（默认权限：属主全权，其余无显式授权）", grants.owner)
            } else {
                format!("属主 {}", grants.owner)
            };
            panel = panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.muted)
                    .child(summary),
            );
        }
        for grant in &admin.pg_effective_grants {
            let (status, status_color) = if grant.direct {
                ("直接授权（可撤销）", colors.text)
            } else if grant.effective {
                ("继承/PUBLIC/属主（不可直接撤销）", colors.muted)
            } else {
                ("无", colors.muted)
            };
            let privilege = grant.privilege.clone();
            let direct = grant.direct;
            let mut row = h_flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .w(px(120.))
                        .text_size(px(13.))
                        .text_color(colors.text)
                        .child(privilege.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .text_color(status_color)
                        .child(status),
                );
            if direct {
                // 已直接授权：提供撤销（只撤销该角色直接授权，不动继承/属主）。
                row = row.child(
                    Button::new(format!("pg-revoke-{privilege}"))
                        .label("撤销")
                        .small()
                        .rounded_md()
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            this.pg_revoke_privilege(tab_id, privilege.clone(), cx);
                        })),
                );
            } else {
                // 未直接授权：提供授予（带/不带 GRANT OPTION）。闭包各持一份克隆避免移动冲突。
                let grant_plain = privilege.clone();
                let grant_option = privilege.clone();
                row = row
                    .child(
                        Button::new(format!("pg-grant-{privilege}"))
                            .label("授予")
                            .small()
                            .rounded_md()
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.pg_grant_privilege(tab_id, grant_plain.clone(), false, cx);
                            })),
                    )
                    .child(
                        Button::new(format!("pg-grant-opt-{privilege}"))
                            .label("授予+可再授")
                            .small()
                            .rounded_md()
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.pg_grant_privilege(tab_id, grant_option.clone(), true, cx);
                            })),
                    );
            }
            panel = panel.child(row);
        }
    }
    panel
}

impl NavicatMain {
    /// 创建 PG 角色：读取表单 → CreatePgRole → 成功刷新角色列表。
    fn pg_create_role(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return;
        };
        let connection_id = admin.connection_id;
        let can_login = admin.pg_can_login;
        let name = admin.create_user.trim().to_string();
        let password = match admin.new_password.trim() {
            "" => None,
            value => Some(value.to_string()),
        };
        if name.is_empty() {
            self.show_message("角色名不能为空", AppMessageKind::Warning, cx);
            return;
        }
        let event = self.dispatch(
            AppCommand::CreatePgRole {
                connection_id,
                name: name.clone(),
                can_login,
                password,
            },
            cx,
        );
        match event {
            AppEvent::PgRoleChanged(_) => {
                self.show_message(format!("已创建角色 {name}"), AppMessageKind::Success, cx);
                self.dispatch(AppCommand::EndUserAdminCreateUser(tab_id), cx);
                self.start_user_admin_users_load(tab_id, cx);
            }
            AppEvent::Failed(error) => {
                self.show_message(
                    format!("创建角色失败：{}", error.message),
                    AppMessageKind::Error,
                    cx,
                );
            }
            _ => {}
        }
    }

    /// 取消新建 PG 角色：结束新建态并刷新。
    fn pg_cancel_create_role(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.dispatch(AppCommand::EndUserAdminCreateUser(tab_id), cx);
        self.start_user_admin_users_load(tab_id, cx);
        cx.notify();
    }

    /// 重命名当前选中 PG 角色 → RenamePgRole → 刷新成员/角色列表。
    fn pg_rename_role(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return;
        };
        let Some(role) = admin.selected_user.clone() else {
            return;
        };
        let connection_id = admin.connection_id;
        let new_name = admin.pg_rename_new.trim().to_string();
        if new_name.is_empty() {
            self.show_message("新角色名不能为空", AppMessageKind::Warning, cx);
            return;
        }
        let old_name = role.user.clone();
        let event = self.dispatch(
            AppCommand::RenamePgRole {
                connection_id,
                old_name: old_name.clone(),
                new_name: new_name.clone(),
            },
            cx,
        );
        match event {
            AppEvent::PgRoleChanged(_) => {
                self.show_message(
                    format!("已重命名 {old_name} → {new_name}"),
                    AppMessageKind::Success,
                    cx,
                );
                self.dispatch(AppCommand::EndUserAdminPgEdit(tab_id), cx);
                self.start_user_admin_users_load(tab_id, cx);
            }
            AppEvent::Failed(error) => {
                self.show_message(
                    format!("重命名失败：{}", error.message),
                    AppMessageKind::Error,
                    cx,
                );
            }
            _ => {}
        }
    }

    /// 修改当前选中 PG 角色密码 → AlterPgRolePassword → 结束编辑态。
    fn pg_change_password(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return;
        };
        let Some(role) = admin.selected_user.clone() else {
            return;
        };
        let connection_id = admin.connection_id;
        let password = admin.new_password.trim().to_string();
        if password.is_empty() {
            self.show_message("密码不能为空", AppMessageKind::Warning, cx);
            return;
        }
        let role_name = role.user.clone();
        let event = self.dispatch(
            AppCommand::AlterPgRolePassword {
                connection_id,
                name: role_name.clone(),
                password,
            },
            cx,
        );
        match event {
            AppEvent::PgRoleChanged(_) => {
                self.show_message(
                    format!("已修改 {role_name} 密码"),
                    AppMessageKind::Success,
                    cx,
                );
                self.dispatch(AppCommand::EndUserAdminPgEdit(tab_id), cx);
                self.start_user_admin_users_load(tab_id, cx);
            }
            AppEvent::Failed(error) => {
                self.show_message(
                    format!("修改密码失败：{}", error.message),
                    AppMessageKind::Error,
                    cx,
                );
            }
            _ => {}
        }
    }

    /// 删除当前选中 PG 角色（连接器默认 RESTRICT：有依赖时服务端拒绝，不自动 DROP OWNED/CASCADE）。
    fn pg_drop_selected_role(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return;
        };
        let Some(role) = admin.selected_user.clone() else {
            return;
        };
        let connection_id = admin.connection_id;
        let role_name = role.user.clone();
        let event = self.dispatch(
            AppCommand::DropPgRole {
                connection_id,
                name: role_name.clone(),
            },
            cx,
        );
        match event {
            AppEvent::PgRoleChanged(_) => {
                self.show_message(format!("已删除角色 {role_name}"), AppMessageKind::Success, cx);
                self.start_user_admin_users_load(tab_id, cx);
            }
            AppEvent::Failed(error) => {
                self.show_message(
                    format!("删除角色失败：{}", error.message),
                    AppMessageKind::Error,
                    cx,
                );
            }
            _ => {}
        }
    }
}

impl NavicatMain {
    /// 读取当前目标的对象权限（后台线程 → 回填 pg_object_grants/pg_effective_grants）。
    fn start_pg_object_grants_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.dispatch(AppCommand::StartUserAdminPgObjectGrantsLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadUserAdminPgObjectGrants(tab_id)) {
                        AppEvent::UserAdminPgObjectGrantsLoaded(_, result) => result,
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "读取对象权限失败".to_string(),
                            message: "对象权限读取没有返回结果".to_string(),
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
                    this.dispatch(
                        AppCommand::FinishUserAdminPgObjectGrantsLoad { tab_id, result },
                        cx,
                    );
                    cx.notify();
                });
            });
        });
        let _ = task;
    }

    /// 授予选中角色某权限（可选 GRANT OPTION），成功后重新读取。
    fn pg_grant_privilege(
        &mut self,
        tab_id: TabId,
        privilege: String,
        grant_option: bool,
        cx: &mut Context<Self>,
    ) {
        let event = self.dispatch(
            AppCommand::GrantUserAdminPgPrivilege {
                tab_id,
                privilege,
                grant_option,
            },
            cx,
        );
        match event {
            AppEvent::UserAdminPgGrantsChanged(_) => {
                self.show_message("已授权", AppMessageKind::Success, cx);
                self.start_pg_object_grants_load(tab_id, cx);
            }
            AppEvent::Failed(error) => {
                self.show_message(
                    format!("授权失败：{}", error.message),
                    AppMessageKind::Error,
                    cx,
                );
            }
            _ => {}
        }
    }

    /// 撤销选中角色某权限（仅撤销直接授权），成功后重新读取。
    fn pg_revoke_privilege(&mut self, tab_id: TabId, privilege: String, cx: &mut Context<Self>) {
        let event = self.dispatch(AppCommand::RevokeUserAdminPgPrivilege { tab_id, privilege }, cx);
        match event {
            AppEvent::UserAdminPgGrantsChanged(_) => {
                self.show_message("已撤销", AppMessageKind::Success, cx);
                self.start_pg_object_grants_load(tab_id, cx);
            }
            AppEvent::Failed(error) => {
                self.show_message(
                    format!("撤销失败：{}", error.message),
                    AppMessageKind::Error,
                    cx,
                );
            }
            _ => {}
        }
    }
}
