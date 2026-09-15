// PostgreSQL 用户与角色工作台（改版）。
//
// 结构对标 MySQL 用户与权限页：统一工具栏 + 左侧角色列表 + 右侧五页签
// （常规/高级/成员关系/权限/SQL 预览）。所有编辑进入 App 层草稿（PgRoleDraft +
// 成员/授权草稿变更），顶部「保存」经 connector 以单事务一次提交；任何开关/勾选
// 在保存前都不写库。本模块只做渲染与命令分发，不拼 SQL、不直接访问数据库。

fn pg_valid_until_mode_options() -> Vec<String> {
    vec![
        "保持不变".to_string(),
        "清除截止时间（永不过期）".to_string(),
        "自定义截止时间…".to_string(),
    ]
}

/// 密码操作选项：新建为「不设置密码/设置新密码」，既有角色为「保持不变/设置新密码/清除密码」。
fn pg_password_op_options(create: bool) -> Vec<String> {
    if create {
        vec!["不设置密码".to_string(), "设置新密码".to_string()]
    } else {
        vec![
            "保持不变".to_string(),
            "设置新密码".to_string(),
            "清除密码".to_string(),
        ]
    }
}

/// 角色次行标识：LOGIN→可登录；NOLOGIN→不可登录；pg_* 预定义角色加「预定义」前缀。
fn pg_role_identity_label(role: &PgRole) -> String {
    let login_label = if role.can_login { "可登录" } else { "不可登录" };
    if UserAdminState::pg_is_predefined_role(&role.name) {
        format!("预定义 · {login_label}")
    } else {
        login_label.to_string()
    }
}

/// PG 工作台根布局：工具栏 +（列表 | 页签详情）+ 底部状态条 + 确认弹框。
fn pg_role_admin_content(
    tab_id: TabId,
    admin: &UserAdminState,
    connection_name: String,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    this.sync_pg_inputs(admin, window, cx);
    let connection_kind_label = "PostgreSQL";

    let mut root = div()
        .relative()
        .flex_1()
        .bg(colors.content_bg)
        .flex()
        .flex_col()
        .child(pg_user_admin_toolbar(
            tab_id,
            admin,
            connection_name,
            connection_kind_label,
            this,
            colors,
            cx,
        ))
        .child(
            div()
                .flex()
                .min_h(px(0.))
                .flex_1()
                .child(
                    pg_user_admin_role_list(tab_id, admin, this, colors, cx)
                        .w(px(320.))
                        .flex_none(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(pg_user_admin_tab_strip(tab_id, admin.active_detail_tab, colors, cx))
                        .child(match admin.active_detail_tab {
                            UserAdminDetailTab::General => {
                                pg_general_panel(tab_id, admin, this, window, colors, cx)
                            }
                            UserAdminDetailTab::Advanced => {
                                pg_advanced_panel(tab_id, admin, this, window, colors, cx)
                            }
                            // PG 成员关系页同时展示「所属角色」与「此角色的成员」两向。
                            UserAdminDetailTab::MemberOf | UserAdminDetailTab::Members => {
                                pg_membership_panel(tab_id, admin, this, colors, cx)
                            }
                            UserAdminDetailTab::Privileges => {
                                pg_privileges_panel(tab_id, admin, this, window, colors, cx)
                            }
                            UserAdminDetailTab::SqlPreview => {
                                pg_sql_preview_panel(tab_id, admin, this, colors, cx)
                            }
                        }),
                ),
        )
        .child(pg_user_admin_status_bar(admin, colors));

    // 删除确认框（仅已存在、非预定义角色）。
    if let Some(name) = admin.pg_pending_delete.clone() {
        root = root.child(pg_delete_role_modal(tab_id, name, this.focus_handle.clone(), colors, cx));
    }
    // 有草稿时切换角色确认框：确认=丢弃草稿并切换；取消=留在当前角色。
    if let Some(target) = admin.pg_pending_switch.clone() {
        root = root.child(pg_switch_role_modal(tab_id, target, this.focus_handle.clone(), colors, cx));
    }
    root
}

/// 顶部工具栏：左侧「用户与角色 + 连接 + PostgreSQL」，右侧保存/刷新/删除。
fn pg_user_admin_toolbar(
    tab_id: TabId,
    admin: &UserAdminState,
    connection_name: String,
    kind_label: &'static str,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let saving = admin.pg_save_status == PgRoleSaveStatus::Saving;
    let save_disabled = saving || !this.pg_save_enabled(tab_id);
    let deletable = admin
        .pg_selected_role
        .as_deref()
        .is_some_and(|name| !UserAdminState::pg_is_predefined_role(name))
        && admin.pg_draft.as_ref().is_none_or(|d| !d.create);
    h_flex()
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
                .text_size(px(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("用户与角色"),
        )
        .child(user_admin_badge(connection_name, colors))
        .child(user_admin_badge(kind_label.to_string(), colors))
        .when(
            admin.pg_save_status == PgRoleSaveStatus::NeedsVerify,
            |this| {
                this.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(0xff9f0a))
                        .child("上次提交结果待核实，请刷新确认"),
                )
            },
        )
        .child(div().flex_1())
        .child(
            user_admin_button(
                if saving { "保存中" } else { "保存" },
                AppIcon::Save,
                false,
                save_disabled,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_pg_plan_apply(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button(
                if admin.loading_users { "加载中" } else { "刷新" },
                AppIcon::Refresh,
                false,
                admin.loading_users,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_user_admin_pg_roles_load(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            user_admin_button("删除", AppIcon::Trash, true, !deletable, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::PgBeginDeleteRole(tab_id), cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

/// 左侧角色列表：搜索 + 筛选 + 角色行（主行角色名，次行 LOGIN/NOLOGIN/预定义）。
fn pg_user_admin_role_list(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let query = admin.search.trim().to_lowercase();
    let roles: Vec<PgRole> = admin
        .pg_roles
        .iter()
        .filter(|role| match admin.pg_role_filter.as_str() {
            "login" => role.can_login,
            "nologin" => !role.can_login,
            "predefined" => UserAdminState::pg_is_predefined_role(&role.name),
            _ => true,
        })
        .filter(|role| {
            query.is_empty() || role.name.to_lowercase().contains(&query)
        })
        .cloned()
        .collect();

    let mut list = div().flex_1().min_h(px(0.)).overflow_y_scrollbar();
    if admin.loading_users {
        list = list.child(user_admin_empty_row("正在加载角色...", colors));
    } else if let Some(error) = &admin.pg_roles_error {
        list = list.child(user_admin_error_row(&error.message, colors)).child(
            user_admin_button("重试", AppIcon::Refresh, false, false, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.start_user_admin_pg_roles_load(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        );
    } else if roles.is_empty() && admin.pg_draft.is_none() {
        list = list.child(
            user_admin_empty_row("没有匹配的角色。调整搜索或筛选；系统预定义角色始终包含在「全部」中。", colors),
        );
    } else {
        for role in &roles {
            let active = admin.pg_selected_role.as_deref() == Some(role.name.as_str())
                && admin.pg_draft.as_ref().is_none_or(|d| !d.create);
            let dirty = admin.pg_selected_role.as_deref() == Some(role.name.as_str())
                && admin.pg_has_draft_changes();
            list = list.child(pg_user_admin_role_row(tab_id, role, active, dirty, colors, cx));
        }
        // 新建草稿行（角色名留空 → 「新角色 · 待创建」）。
        if let Some(draft) = &admin.pg_draft
            && draft.create
        {
            list = list.child(pg_draft_role_row(draft, colors));
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
                        .child(user_admin_input_box(this.user_admin_search_input.clone(), colors).flex_1())
                        .child(
                            user_admin_add_user_button(admin.loading_users, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(admin) = this.user_admin_state_for(tab_id)
                                        && !admin.loading_users
                                        // 已有未保存的新建草稿时不重复进入新建态。
                                        && !admin.pg_has_draft_changes()
                                    {
                                        this.dispatch(AppCommand::PgBeginCreateRole(tab_id), cx);
                                    }
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                )
                .child(pg_role_filter_row(tab_id, admin, colors, cx)),
        )
        .child(list)
}

/// 紧凑筛选次行：全部 / 可登录 / 不可登录 / 预定义。
fn pg_role_filter_row(
    tab_id: TabId,
    admin: &UserAdminState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let filters = [
        ("all", "全部"),
        ("login", "可登录"),
        ("nologin", "不可登录"),
        ("predefined", "预定义"),
    ];
    let mut row = h_flex().items_center().gap_1().pb_1();
    for (value, label) in filters {
        let active = admin.pg_role_filter == value;
        row = row.child(
            div()
                .id(SharedString::from(format!("pg-role-filter-{value}")))
                .px_2()
                .h(px(22.))
                .rounded(colors.radius)
                .cursor_pointer()
                .bg(if active { colors.tree_selected } else { colors.panel_bg })
                .hover(move |style| style.bg(if active { colors.tree_selected } else { colors.hover }))
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(if active { colors.text } else { colors.muted })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(
                            AppCommand::SetPgRoleFilter {
                                tab_id,
                                filter: value.to_string(),
                            },
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

/// 角色列表行：主行角色名（原样显示，含 @ 等字符），次行身份弱化文本。
fn pg_user_admin_role_row(
    tab_id: TabId,
    role: &PgRole,
    active: bool,
    dirty: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let name = role.name.clone();
    let name_tooltip = name.clone();
    let identity = pg_role_identity_label(role);
    div()
        .id(SharedString::from(format!("pg-role-row-{}", role.name)))
        .h(px(52.))
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .cursor_pointer()
        .bg(if active { colors.tree_selected } else { colors.panel_bg })
        .hover(move |style| style.bg(if active { colors.tree_selected } else { colors.hover }))
        .flex()
        .items_center()
        .gap_2()
        .tooltip(move |window, cx| Tooltip::new(name_tooltip.clone()).build(window, cx))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.pg_select_role(tab_id, name.clone(), cx);
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
                        .child(role.name.clone()),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(identity),
                        )
                        .when(dirty, |this| {
                            this.child(
                                div()
                                    .px_1()
                                    .rounded(colors.radius * 0.5)
                                    .text_size(px(10.))
                                    .text_color(rgb(0xff9f0a))
                                    .child("未保存"),
                            )
                        }),
                ),
        )
}

/// 新建草稿在列表中的占位行。
fn pg_draft_role_row(draft: &PgRoleDraft, colors: UiColors) -> Div {
    let title = if draft.name.trim().is_empty() {
        "新角色 · 待创建".to_string()
    } else {
        format!("{} · 待创建", draft.name.trim())
    };
    div()
        .h(px(56.))
        .mx_2()
        .my_2()
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(rgb(0x2563eb))
        .bg(if colors.is_dark { rgb(0x16345d) } else { rgb(0xeaf2ff) })
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
                        .child(if draft.can_login { "待创建 · 可登录" } else { "待创建 · 不可登录" }),
                ),
        )
}

/// 右侧页签条：常规 / 高级 / 成员关系 / 权限 / SQL 预览（固定五个）。
fn pg_user_admin_tab_strip(
    tab_id: TabId,
    active_tab: UserAdminDetailTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // PG 使用固定五页签；MySQL 的 Members 页在 PG 并入「成员关系」双向展示。
    let tabs = [
        (UserAdminDetailTab::General, "常规"),
        (UserAdminDetailTab::Advanced, "高级"),
        (UserAdminDetailTab::MemberOf, "成员关系"),
        (UserAdminDetailTab::Privileges, "权限"),
        (UserAdminDetailTab::SqlPreview, "SQL 预览"),
    ];
    let mut strip = div()
        .h(px(36.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_end();
    for (detail_tab, label) in tabs {
        let active = active_tab == detail_tab;
        strip = strip.child(
            div()
                .id(SharedString::from(format!("pg-user-admin-tab-{label}")))
                .h(px(36.))
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
                        // 切换页签不触发确认（草稿保留）；SQL 预览页进入时自动生成预览。
                        this.dispatch(
                            AppCommand::SelectUserAdminDetailTab { tab_id, detail_tab },
                            cx,
                        );
                        if detail_tab == UserAdminDetailTab::SqlPreview {
                            this.start_pg_plan_preview(tab_id, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(label),
        );
    }
    strip.child(div().flex_1())
}

/// 底部状态条：角色统计 + 未保存更改标记。
fn pg_user_admin_status_bar(admin: &UserAdminState, colors: UiColors) -> Div {
    let total = admin.pg_roles.len();
    let login_count = admin.pg_roles.iter().filter(|role| role.can_login).count();
    h_flex()
        .items_center()
        .gap_3()
        .px_3()
        .h(px(28.))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(format!("{total} 个角色 · {login_count} 个可登录")),
        )
        .child(div().flex_1())
        .when(admin.pg_has_draft_changes(), |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.))
                    .text_color(rgb(0xff9f0a))
                    .child(app_icon(AppIcon::Save, 12., rgb(0xff9f0a)))
                    .child("未保存的更改"),
            )
        })
}

/// 删除角色确认框：仅报告风险，不做 DROP OWNED/CASCADE；服务端拒绝时展示依赖详情。
fn pg_delete_role_modal(
    tab_id: TabId,
    name: String,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
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
                this.dispatch(AppCommand::PgCancelDeleteRole(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(440.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("PgDeleteRoleModal")
                .on_action(cx.listener(move |this, _: &CancelDialog, _, cx| {
                    this.dispatch(AppCommand::PgCancelDeleteRole(tab_id), cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("删除角色"),
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
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.dispatch(AppCommand::PgCancelDeleteRole(tab_id), cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(format!(
                            "确定要删除角色「{name}」吗？该角色是集群级身份，删除影响所有数据库。\
                             如果它仍拥有对象或包含成员，服务器将拒绝删除并报告依赖；本工具不会自动清理对象。"
                        )),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("pg-delete-cancel")
                                .label("取消")
                                .rounded_md()
                                .w(px(78.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::PgCancelDeleteRole(tab_id), cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("pg-delete-confirm")
                                .label("删除角色")
                                .danger()
                                .rounded_md()
                                .w(px(104.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::PgCancelDeleteRole(tab_id), cx);
                                    this.pg_drop_confirmed(tab_id, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

/// 有草稿时切换角色确认框：确认=丢弃草稿并切换；取消=留在当前角色。
fn pg_switch_role_modal(
    tab_id: TabId,
    target: String,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
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
                this.dispatch(AppCommand::PgCancelDeleteRole(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(440.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("PgSwitchRoleModal")
                .on_action(cx.listener(move |this, _: &CancelDialog, _, cx| {
                    this.clear_pg_pending_switch(tab_id, cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("放弃未保存的更改？"),
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
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.clear_pg_pending_switch(tab_id, cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(format!(
                            "当前角色有未保存的更改。切换到「{target}」将丢弃这些更改（仅本地草稿，数据库未改动）。"
                        )),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("pg-switch-cancel")
                                .label("留在此页")
                                .rounded_md()
                                .w(px(90.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.clear_pg_pending_switch(tab_id, cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("pg-switch-confirm")
                                .label("放弃并切换")
                                .danger()
                                .rounded_md()
                                .w(px(110.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dispatch(
                                        AppCommand::DiscardPgDraftAndSelect {
                                            tab_id,
                                            name: target.clone(),
                                        },
                                        cx,
                                    );
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

impl NavicatMain {
    /// 渲染前同步草稿 → 输入控件/下拉选项；按指纹避免每帧重建 select items。
    fn sync_pg_inputs(
        &mut self,
        admin: &UserAdminState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = admin.pg_draft.clone() else {
            return;
        };
        sync_input_value(&self.pg_role_name_input, &draft.name, window, cx);
        sync_input_value(
            &self.pg_connection_limit_input,
            &draft.connection_limit_text,
            window,
            cx,
        );
        let valid_until_text = match &draft.valid_until {
            PgValidUntilOp::At(text) => text.clone(),
            _ => String::new(),
        };
        sync_input_value(&self.pg_valid_until_input, &valid_until_text, window, cx);
        // 密码输入：进入「设置新密码」模式后不回写（避免打字被草稿同步打断），
        // 仅在模式切换时清空。
        if !matches!(&draft.password, PgPasswordOp::Set(_)) {
            sync_input_value(&self.pg_password_input, "", window, cx);
            sync_input_value(&self.pg_password_confirm_input, "", window, cx);
        }
        // 下拉选项与选中值（带指纹）。
        let _selected = admin.pg_effective_grantee_name();
        let targets = admin.pg_grant_targets.clone().unwrap_or_default();
        let object_options: Vec<String> = match admin.pg_grant_kind {
            PgGrantObjectKind::Table => targets.tables.clone(),
            PgGrantObjectKind::View => targets.views.clone(),
            PgGrantObjectKind::Sequence => targets.sequences.clone(),
            PgGrantObjectKind::Routine => targets.routines.clone(),
            PgGrantObjectKind::Schema => targets.schemas.clone(),
            PgGrantObjectKind::Database => targets.databases.clone(),
        };
        let object_selected = if admin.pg_grant_object.is_empty() {
            String::new()
        } else {
            format!("{}.{}", admin.pg_grant_schema, admin.pg_grant_object)
        };
        let fingerprint = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            draft.create,
            draft.password_label(),
            draft.valid_until_mode_label(),
            targets.databases.len(),
            targets.schemas.len(),
            object_options.len(),
            admin.pg_grant_database,
            admin.pg_grant_schema,
            object_selected,
            // 角色名全集（非数量）：改名/重建等数量不变的变化也必须刷新选项。
            admin
                .pg_roles
                .iter()
                .map(|role| role.name.as_str())
                .collect::<Vec<_>>()
                .join(","),
        );
        if self.pg_select_items_fingerprint != fingerprint {
            self.pg_select_items_fingerprint = fingerprint;
            let member_items: Vec<String> = admin
                .pg_roles
                .iter()
                .map(|role| role.name.clone())
                .filter(|name| Some(name.as_str()) != admin.pg_selected_role.as_deref())
                .collect();
            self.pg_grant_db_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(targets.databases.clone()), window, cx);
            });
            self.pg_grant_schema_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(targets.schemas.clone()), window, cx);
            });
            self.pg_grant_object_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(object_options), window, cx);
            });
            self.pg_member_of_role_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(member_items), window, cx);
            });
        }
        // 选中值每次渲染都同步（set_items 会重置选择，不能只靠指纹门控）。
        self.pg_grant_db_select.update(cx, |select, cx| {
            select.set_selected_value(&admin.pg_grant_database.clone(), window, cx);
        });
        self.pg_grant_schema_select.update(cx, |select, cx| {
            select.set_selected_value(&admin.pg_grant_schema.clone(), window, cx);
        });
        self.pg_grant_object_select.update(cx, |select, cx| {
            select.set_selected_value(&object_selected, window, cx);
        });
        // 成员下拉是「选择要授予的组角色」选择器：不回填当前角色（其已被排除出选项），
        // set_items 重置后显示占位符。
        // 密码操作/有效期模式选项随 create 变化，选中值随草稿回写。
        self.pg_password_op_select.update(cx, |select, cx| {
            let items = SearchableVec::new(pg_password_op_options(draft.create));
            select.set_items(items, window, cx);
            select.set_selected_value(&draft.password_label(), window, cx);
        });
        self.pg_valid_until_mode_select.update(cx, |select, cx| {
            select.set_selected_value(&draft.valid_until_mode_label(), window, cx);
        });
    }

    /// 切换数据库后加载权限目标列表（一期同批只允许一个数据库）。
    fn start_pg_grant_targets_load_for(
        &mut self,
        tab_id: TabId,
        database: String,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            AppCommand::SetPgGrantDatabase {
                tab_id,
                database: database.clone(),
            },
            cx,
        );
        self.dispatch(AppCommand::StartPgGrantTargetsLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadPgGrantTargets { tab_id, database }) {
                        AppEvent::UserAdminPgGrantTargetsLoaded(_, lists) => Ok(lists),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载权限目标失败".to_string(),
                            message: "权限目标列表没有返回结果".to_string(),
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
                        AppCommand::FinishPgGrantTargetsLoad { tab_id, result },
                        cx,
                    );
                    cx.notify();
                });
            });
        });
        let _ = task;
    }

    /// 请求选择角色；有未保存草稿时先弹确认（确认后走 DiscardPgDraftAndSelect）。
    fn pg_select_role(&mut self, tab_id: TabId, name: String, cx: &mut Context<Self>) {
        let has_draft = self
            .user_admin_state_for(tab_id)
            .is_some_and(|admin| admin.pg_has_draft_changes());
        if has_draft {
            self.dispatch(
                AppCommand::SetPgRoleSwitchPending { tab_id, target: name },
                cx,
            );
        } else {
            self.dispatch(AppCommand::SelectPgRole { tab_id, name }, cx);
            self.start_pg_memberships_load(tab_id, cx);
        }
        cx.notify();
    }

    fn clear_pg_pending_switch(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.dispatch(AppCommand::PgCancelSwitchRole(tab_id), cx);
        cx.notify();
    }

    /// 确认删除角色：DROP ROLE（默认 RESTRICT，服务端拒绝时报告依赖）。
    fn pg_drop_confirmed(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return;
        };
        let Some(name) = admin.pg_selected_role.clone() else {
            return;
        };
        let connection_id = admin.connection_id;
        let event = self.dispatch(
            AppCommand::DropPgRole {
                connection_id,
                name: name.clone(),
            },
            cx,
        );
        match event {
            AppEvent::PgRoleChanged(_) => {
                self.show_message(format!("已删除角色 {name}"), AppMessageKind::Success, cx);
                self.start_user_admin_pg_roles_load(tab_id, cx);
            }
            AppEvent::Failed(error) => {
                self.show_message(
                    format!("删除角色失败：{}（未自动清理其对象）", error.message),
                    AppMessageKind::Error,
                    cx,
                );
            }
            _ => {}
        }
    }

    /// 后台加载 PG 角色列表（权威 PgRole 数据，替代空 host 身份）。
    fn start_user_admin_pg_roles_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self._user_admin_users_tasks.contains_key(&tab_id) {
            return;
        }
        self.dispatch(AppCommand::StartUserAdminPgRolesLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadUserAdminPgRoles(tab_id)) {
                        AppEvent::UserAdminPgRolesLoaded(_, roles) => Ok(roles),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载角色失败".to_string(),
                            message: "角色列表加载没有返回结果".to_string(),
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
                    let error_msg = result.as_ref().err().map(|error| error.message.clone());
                    this.dispatch(
                        AppCommand::FinishUserAdminPgRolesLoad {
                            tab_id,
                            result: result.clone(),
                        },
                        cx,
                    );
                    if let Some(error_msg) = error_msg {
                        this.show_message(
                            format!("刷新角色列表失败：{error_msg}"),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    // 角色列表就绪后加载成员关系（双向展示）。
                    this.start_pg_memberships_load(tab_id, cx);
                    cx.notify();
                });
            });
        });
        self._user_admin_users_tasks.insert(tab_id, task);
    }

    /// 后台加载全量成员关系（含服务端成员选项版本探测）。
    fn start_pg_memberships_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self._user_admin_pg_membership_tasks.contains_key(&tab_id) {
            return;
        }
        self.dispatch(AppCommand::StartPgMembershipsLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadPgMemberships(tab_id)) {
                        AppEvent::UserAdminPgMembershipsLoaded(_, memberships, supported) => {
                            Ok((memberships, supported))
                        }
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载成员关系失败".to_string(),
                            message: "成员关系加载没有返回结果".to_string(),
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
                    this._user_admin_pg_membership_tasks.remove(&tab_id);
                    let (memberships, supported) = match result {
                        Ok((memberships, supported)) => (Ok(memberships), supported),
                        Err(error) => (Err(error), false),
                    };
                    this.dispatch(
                        AppCommand::FinishPgMembershipsLoad {
                            tab_id,
                            result: memberships,
                            member_options_supported: supported,
                        },
                        cx,
                    );
                    cx.notify();
                });
            });
        });
        // GPUI Task 被丢弃会取消异步工作；必须持有到回写完成，否则成员页始终显示空态。
        self._user_admin_pg_membership_tasks.insert(tab_id, task);
    }

    /// 保存：App 构建（校验）变更计划并单事务应用；失败保留草稿。
    fn start_pg_plan_apply(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self._user_admin_apply_tasks.contains_key(&tab_id) {
            return;
        }
        // 保存前本地校验：设置新密码时两次输入必须一致。
        let password_mismatch = self
            .user_admin_state_for(tab_id)
            .and_then(|admin| admin.pg_draft)
            .is_some_and(|draft| matches!(&draft.password, PgPasswordOp::Set(_)))
            && self.pg_password_input.read(cx).value().as_ref()
                != self.pg_password_confirm_input.read(cx).value().as_ref();
        if password_mismatch {
            self.show_message("两次输入的密码不一致", AppMessageKind::Warning, cx);
            return;
        }
        self.dispatch(AppCommand::StartPgPlanApply(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::ApplyPgRolePlan(tab_id)) {
                        AppEvent::UserAdminPgRolePlanFinished(_, plan, result) => (plan, result),
                        AppEvent::Failed(error) => (
                            fluxdb_core::PgRoleSavePlan {
                                database: None,
                                role_name: String::new(),
                                changes: Vec::new(),
                            },
                            Err(error),
                        ),
                        _ => (
                            fluxdb_core::PgRoleSavePlan {
                                database: None,
                                role_name: String::new(),
                                changes: Vec::new(),
                            },
                            Err(fluxdb_core::UserFacingError {
                                title: "保存失败".to_string(),
                                message: "角色变更提交没有返回结果".to_string(),
                                detail: None,
                                retryable: true,
                            }),
                        ),
                    }
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._user_admin_apply_tasks.remove(&tab_id);
                    let (plan, result) = result;
                    let success = result.is_ok();
                    let error_msg = result.as_ref().err().map(|error| error.message.clone());
                    this.dispatch(
                        AppCommand::FinishPgRolePlanApply { tab_id, plan, result },
                        cx,
                    );
                    if success {
                        this.show_message("已保存全部更改", AppMessageKind::Success, cx);
                    } else if let Some(error_msg) = error_msg {
                        this.show_message(
                            format!("保存失败（草稿已保留）：{error_msg}"),
                            AppMessageKind::Error,
                            cx,
                        );
                    }
                    // 无论成败都重新读取，核实服务端实际状态。
                    this.start_user_admin_pg_roles_load(tab_id, cx);
                    cx.notify();
                });
            });
        });
        self._user_admin_apply_tasks.insert(tab_id, task);
    }

    /// 生成脱敏 SQL 预览（与执行共用 connector 渲染规则）。
    fn start_pg_plan_preview(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self._user_admin_preview_tasks.contains_key(&tab_id) {
            return;
        }
        tracing::warn!(target: "pg_user_admin", "preview: task start");
        self.dispatch(AppCommand::StartPgPlanPreview(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadPgPlanPreview(tab_id)) {
                        AppEvent::UserAdminPgPlanPreview(_, stmts) => {
                            tracing::warn!(target: "pg_user_admin", stmts = stmts.len(), "preview: rendered");
                            Ok(stmts)
                        }
                        AppEvent::Failed(error) => {
                            tracing::warn!(target: "pg_user_admin", error = %error.message, "preview: dispatch failed");
                            Err(error)
                        }
                        other => {
                            tracing::warn!(target: "pg_user_admin", event = ?std::mem::discriminant(&other), "preview: unexpected event");
                            Err(fluxdb_core::UserFacingError {
                                title: "生成预览失败".to_string(),
                                message: "SQL 预览没有返回结果".to_string(),
                                detail: None,
                                retryable: true,
                            })
                        }
                    }
                })
                .await;
            tracing::warn!(target: "pg_user_admin", ok = result.is_ok(), "preview: task finished");
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._user_admin_preview_tasks.remove(&tab_id);
                    this.dispatch(
                        AppCommand::FinishPgPlanPreview { tab_id, result },
                        cx,
                    );
                    cx.notify();
                });
            });
        });
        // GPUI Task 被丢弃会取消异步工作；持有到回调完成，否则预览会一直停在 loading。
        self._user_admin_preview_tasks.insert(tab_id, task);
    }

    /// 保存按钮可用性：无草稿变更/保存中/新建无角色名时禁用。
    /// （密码一致性在保存入口校验，因需要读取输入控件值。）
    fn pg_save_enabled(&self, tab_id: TabId) -> bool {
        let Some(admin) = self.user_admin_state_for(tab_id) else {
            return false;
        };
        let name_ok = admin
            .pg_draft
            .as_ref()
            .is_none_or(|draft| !draft.create || !draft.name.trim().is_empty());
        name_ok && admin.pg_has_draft_changes() && admin.pg_save_status != PgRoleSaveStatus::Saving
    }

    /// 密码输入变化后刷新保存按钮状态（仅触发重渲染）。
    fn sync_pg_save_enabled(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }
}
