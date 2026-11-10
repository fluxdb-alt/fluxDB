fn sqlserver_preview_panel(
    tab_id: TabId,
    state: &SqlServerSecurityState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    if state.has_pending_changes()
        && state.preview.is_none()
        && state.preview_error.is_none()
        && !state.preview_loading
    {
        this.start_sqlserver_security_preview(tab_id, cx);
    }
    let mut panel = div().flex_1().min_h(px(0.)).flex().flex_col().p_4().gap_3();
    panel = panel.child(
        h_flex()
            .items_center()
            .gap_2()
            .child(sqlserver_section_title(
                tr("app.sqlserver_security.sql_preview"),
                colors,
            ))
            .child(div().flex_1()),
    );
    if state.preview_loading {
        return panel
            .child(inline_loading_row(tr("app.sqlserver_security.preview_loading"), colors))
            .into_any_element();
    }
    if let Some(error) = &state.preview_error {
        panel = panel.child(user_admin_error_row(&error.message, colors));
    }
    if let Some(error) = &state.apply_error {
        panel = panel.child(user_admin_error_row(&error.message, colors));
    }
    let sql = state.preview_for_display().join("\n");
    panel
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .debug_selector(|| "sqlserver-security-preview-code".into())
                .child(if sql.is_empty() {
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(tr("app.sqlserver_security.preview_empty"))
                } else {
                    user_admin_sql_preview_code_view(
                        SharedString::from(format!("sqlserver-security-preview-{}", tab_id.0)),
                        &sql,
                        colors,
                    )
                }),
        )
        .into_any_element()
}
