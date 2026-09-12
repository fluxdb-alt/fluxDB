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
                    .child(pg_role_detail(admin, colors)),
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

/// 右侧：选中 PG 角色详情（角色标识 + 成员关系）。
fn pg_role_detail(admin: &UserAdminState, colors: UiColors) -> Div {
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
    body
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
