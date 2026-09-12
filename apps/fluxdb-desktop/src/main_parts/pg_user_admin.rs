// PostgreSQL 用户/角色管理面板（T27）。
//
// PG 角色是集群级身份（无 host/plugin/资源限制），不套 MySQL 的 SQL 预览/apply 流程。
// 左侧复用 user_admin_user_list（UserAdminState.users 已由 app 层经连接器 list_roles 装载），
// 右侧展示选中角色的成员关系（pg_auth_members → admin.grants）。角色 CRUD 走 AppCommand→连接器
// （后续增量）；MySQL user_admin 完整保留。

/// PG 角色管理面板：左侧角色列表，右侧选中角色的成员关系概览。
fn pg_role_admin_content(
    tab_id: TabId,
    admin: &UserAdminState,
    connection_name: String,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .relative()
        .flex_1()
        .bg(colors.content_bg)
        .flex()
        .flex_col()
        .child(pg_role_toolbar(connection_name, colors))
        .child(
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

fn pg_role_toolbar(connection_name: String, colors: UiColors) -> Div {
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
            div()
                .ml_auto()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("集群级角色（LOGIN/NOLOGIN），非 MySQL user@host"),
        )
}

/// 右侧：选中 PG 角色详情（角色标识 + 成员关系概览）。
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
            div()
                .text_size(px(15.))
                .text_color(colors.text)
                .child(format!("角色：{}", selected.user)),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("该角色是以下组角色的成员（读取自 pg_auth_members）："),
        );

    if admin.loading_grants {
        body = body.child(user_admin_placeholder_panel_with_text("正在加载成员关系…", colors));
    } else if admin.grants.is_empty() {
        body = body.child(
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("（无组角色成员关系）"),
        );
    } else {
        // 克隆为 owned 本地列表，避免把对 &admin 的借用写进返回的 Div。
        let groups: Vec<String> = admin.grants.clone();
        for group in groups {
            body = body.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(colors.text)
                            .child(group),
                    ),
            );
        }
    }
    body
}
