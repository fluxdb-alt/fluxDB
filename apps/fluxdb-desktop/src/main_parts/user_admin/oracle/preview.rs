// 「SQL 预览」页：与 MySQL / PostgreSQL 一样，直接展示当前草稿会渲染出的 SQL。
//
// 语句来自纯本地渲染（`compose_preview`），口令按草稿原样显示。

fn oracle_preview_panel(
    _tab_id: TabId,
    state: &OracleUserAdminState,
    _this: &mut NavicatMain,
    colors: UiColors,
    _cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let rendered = if state.has_changes() {
        state.compose_preview()
    } else {
        Ok(Vec::new())
    };
    // 代码框是 `flex_1`，必须有确定高度才能撑开；`oracle_panel_shell` 是滚动容器，
    // 内部高度由内容自然撑开，放进去会让代码框坍缩成 0 高。这里与 SQL Server 预览页
    // 一样自己组成受高度约束的列容器。
    let mut body = div()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(oracle_section_title(
            tr("app.oracle_security.sql_preview"),
            colors,
        ));
    match rendered {
        Err(error) => {
            body = body.child(user_admin_error_row(&error.message, colors));
        }
        Ok(statements) if statements.is_empty() => {
            body = body.child(user_admin_empty_row(
                tr("app.oracle_security.no_pending_changes"),
                colors,
            ));
        }
        Ok(statements) => {
            body = body
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .debug_selector(|| "oracle-security-preview-code".into())
                        .child(user_admin_sql_preview_code_view(
                            SharedString::from(format!(
                                "oracle-security-preview-{}",
                                state.selected.as_deref().unwrap_or("-")
                            )),
                            &statements.join("\n"),
                            colors,
                        )),
                );
        }
    }
    body.into_any_element()
}

/// 删除账号确认框。口令不会出现，这里只涉及 `DROP USER`。
fn oracle_delete_modal(
    tab_id: TabId,
    cascade: bool,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let overlay_focus = focus_handle.clone();
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(gpui::black().opacity(0.35))
        .flex()
        .items_center()
        .justify_center()
        .track_focus(&overlay_focus)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::CancelDeleteOracleUser(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(460.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow_lg()
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .track_focus(&focus_handle)
                .key_context("OracleDeleteUserModal")
                .on_action(cx.listener(move |this, _: &CancelDialog, _, cx| {
                    this.dispatch(AppCommand::CancelDeleteOracleUser(tab_id), cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_size(ui_text_size(16.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.oracle_security.delete_title")),
                )
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(tr("app.oracle_security.delete_body")),
                )
                .child(oracle_form_row(
                    tr("app.oracle_security.delete_cascade"),
                    colors,
                ).child(
                    Switch::new("oracle-security-delete-cascade")
                        .checked(cascade)
                        .on_click(cx.listener(move |this, checked, _, cx| {
                            this.dispatch(
                                AppCommand::SetOracleDeleteCascade { tab_id, cascade: *checked },
                                cx,
                            );
                        })),
                ))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            user_admin_button(
                                tr("app.common.cancel"),
                                AppIcon::Close,
                                false,
                                false,
                                colors,
                            )
                            .debug_selector(|| "oracle-security-delete-cancel".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::CancelDeleteOracleUser(tab_id), cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            user_admin_button(
                                tr("app.common.delete"),
                                AppIcon::Trash,
                                true,
                                false,
                                colors,
                            )
                            .debug_selector(|| "oracle-security-delete-confirm".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::ConfirmDeleteOracleUser(tab_id), cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}
