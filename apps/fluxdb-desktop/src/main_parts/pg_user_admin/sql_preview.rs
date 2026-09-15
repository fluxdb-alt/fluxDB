// PG 用户与角色工作台：SQL 预览页。
//
// 显示本次所有页签草稿的差异 SQL（connector 渲染的脱敏结果）；无改动时显示
// 「暂无待执行变更」。密码语句已替换为占位符并标注「脱敏预览，不能直接执行」，
// 复制不含明文密码；实际执行从内存草稿取值，不反向执行预览文本。

fn pg_sql_preview_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let has_changes = admin.pg_has_draft_changes();
    // 预览是草稿快照：进入页签由页签点击触发生成；此处兜底状态恢复/首次
    // 渲染等没有点击事件的路径（有变更、从未生成、非生成中才触发，避免重复）。
    if has_changes
        && admin.pg_plan_preview.is_none()
        && admin.pg_plan_error.is_none()
        && !admin.pg_preview_loading
    {
        tracing::warn!(target: "pg_user_admin", "preview: auto-generate dispatched");
        this.start_pg_plan_preview(tab_id, cx);
    }
    let stmts = admin.pg_plan_preview.clone().unwrap_or_default();
    let sql = stmts.join("\n");

    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .p_4()
        .flex()
        .flex_col()
        .gap_2();

    // 头部：变更摘要 + 生成/刷新预览。
    let role_name = admin.pg_effective_grantee_name();
    let database_label = if admin.pg_grant_database.is_empty() {
        "（无对象授权）".to_string()
    } else {
        admin.pg_grant_database.clone()
    };
    body = body.child(
        h_flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.muted)
                    .child(format!(
                        "影响角色：{} · 对象授权库：{} · 变更 {} 条",
                        if role_name.is_empty() { "—" } else { &role_name },
                        database_label,
                        stmts.len(),
                    )),
            )
            .child(div().flex_1())
            .child(
                user_admin_button("生成/刷新预览", AppIcon::Refresh, false, !has_changes, colors)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.start_pg_plan_preview(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
            ),
    );

    if admin.pg_plan_preview_masked && !sql.is_empty() {
        body = body.child(
            div()
                .text_size(px(12.))
                .text_color(rgb(0xff9f0a))
                .child("脱敏预览，不能直接执行：密码语句已替换为占位符；实际执行使用内存中的草稿值。"),
        );
    }
    if let Some(error) = &admin.pg_plan_error {
        body = body.child(user_admin_error_row(&error.message, colors));
    }

    if !has_changes {
        body = body.child(user_admin_empty_row("暂无待执行变更。", colors));
    } else if admin.pg_preview_loading {
        body = body.child(user_admin_empty_row("正在生成预览…", colors));
    } else if sql.is_empty() {
        body = body.child(user_admin_empty_row("未能生成预览（草稿可能尚未通过校验）。", colors));
    } else {
        // 复用 MySQL 页的 SQL 预览代码视图（只读、SQL 高亮）。
        body = body.child(
            div()
                .flex_1()
                .min_h(px(0.))
                .child(user_admin_sql_preview_code_view(
                    SharedString::from(format!("pg-user-admin-sql-preview-{}", tab_id.0)),
                    &sql,
                    window,
                    colors,
                    cx,
                )),
        );
    }
    body.into_any_element()
}
