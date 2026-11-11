// PG 用户与角色工作台：SQL 预览页。
//
// 显示本次所有页签草稿的差异 SQL（connector 渲染的完整语句）；无改动时显示
// 「暂无待执行变更」。预览与执行同源，口令在这里如实显示，与 MySQL 一致；
// 但不会进入 SQL 历史、日志或 AI 上下文。

fn pg_sql_preview_panel(
    tab_id: TabId,
    admin: &UserAdminState,
    this: &mut NavicatMain,
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

    // 预览在进入页签时自动生成，头部只保留变更摘要。
    let role_name = admin.pg_effective_grantee_name();
    let database_label = if admin.pg_grant_database.is_empty() {
        tr("app.pg_role.preview.no_object_grants").to_string()
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
                    .text_size(ui_text_size(12.))
                    .text_color(colors.muted)
                    .child(tr("app.pg_role.preview.summary")
                        .replace(
                            "%{role}",
                            if role_name.is_empty() { "—" } else { &role_name },
                        )
                        .replace("%{databases}", &database_label)
                        .replace("%{changes}", &stmts.len().to_string())),
            )
            .child(div().flex_1()),
    );

    if let Some(error) = &admin.pg_plan_error {
        body = body.child(user_admin_error_row(&error.message, colors));
    }

    if !has_changes {
        body = body.child(user_admin_empty_row(tr("app.pg_role.preview.no_changes"), colors));
    } else if admin.pg_preview_loading {
        body = body.child(user_admin_empty_row(tr("app.pg_role.preview.generating"), colors));
    } else if sql.is_empty() {
        body = body.child(user_admin_empty_row(tr("app.pg_role.preview.failed_hint"), colors));
    } else {
        // 复用 MySQL 页的 SQL 预览代码视图（只读、SQL 高亮）。
        body = body.child(
            div()
                .flex_1()
                .min_h(px(0.))
                .child(user_admin_sql_preview_code_view(
                    SharedString::from(format!("pg-user-admin-sql-preview-{}", tab_id.0)),
                    &sql,
                    colors,
                )),
        );
    }
    body.into_any_element()
}
