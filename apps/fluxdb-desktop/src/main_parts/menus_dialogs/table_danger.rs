#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DangerTableCapabilities {
    foreign_key_check: bool,
    restart_identity: bool,
}

fn danger_table_capabilities(database_kind: DatabaseKind) -> DangerTableCapabilities {
    DangerTableCapabilities {
        foreign_key_check: matches!(database_kind, DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb),
        restart_identity: database_kind == DatabaseKind::Postgres,
    }
}

fn danger_table_modal(
    form: PendingDangerTableAction,
    sql_preview: Result<String, String>,
    foreign_key_check_select: Entity<SelectState<SearchableVec<ForeignKeyCheckOption>>>,
    running: bool,
    capabilities: DangerTableCapabilities,
    focus_handle: FocusHandle,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let (sql, validation_error) = match sql_preview {
        Ok(sql) => (Some(sql), None),
        Err(message) => (None, Some(message)),
    };
    let error = form.error.clone().or(validation_error);
    let can_submit = !running && form.acknowledged && sql.is_some() && error.is_none();
    // 注意：不能在 builder 内 `cx.entity().read(cx)` 自读判断连接类型——此 builder 在
    // NavicatMain::render 期间被同步执行，自读会触发 double-lease panic。能力由调用方读取后传入。
    let is_truncate = form.action == DangerTableAction::Truncate;
    let restart_identity = capabilities.restart_identity && is_truncate;

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
            cx.listener(|this, _, _, cx| {
                this.cancel_danger_table_modal(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(500.))
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
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("DangerTableModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_danger_table_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(danger_table_header(form.action, colors, cx))
                .child(
                    div()
                        .px_5()
                        .pb_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(danger_table_prompt(&form))
                        .when(capabilities.foreign_key_check, |this| {
                            this.child(danger_table_fk_row(
                                foreign_key_check_select,
                                running,
                                colors,
                            ))
                        })
                        .when(restart_identity, |this| {
                            this.child(danger_table_restart_identity_row(
                                &form,
                                running,
                                colors,
                                cx,
                            ))
                        })
                        .when_some(sql, |this, sql| {
                            this.child(table_sql_preview_box(
                                form.action.sql_preview_key(),
                                sql,
                                window,
                                colors,
                                cx,
                            ))
                        })
                        .child(danger_table_ack_row(&form, running, colors, cx))
                        .when_some(error, |this, error| {
                            this.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(0xe5484d))
                                    .child(error),
                            )
                        }),
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
                            Button::new("danger-table-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_danger_table_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("danger-table-confirm")
                                .label(if running {
                                    tr("app.common.running")
                                } else {
                                    form.action.confirm_label().into()
                                })
                                .danger()
                                .w(px(78.))
                                .disabled(!can_submit)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_danger_table_action(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn danger_table_header(
    action: DangerTableAction,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .px_5()
        .pt_4()
        .pb_4()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon_box(AppIcon::Trash, 24., 16., rgb(0xe5484d)))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child(action.title()),
                ),
        )
        .child(
            user_admin_icon_button(AppIcon::Close, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.cancel_danger_table_modal(cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn danger_table_prompt(form: &PendingDangerTableAction) -> impl IntoElement {
    div()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child({
            let action = form.action.prompt();
            tr("app.table.danger.title")
                .replace("%{action}", action.as_ref())
                .replace("%{table}", &form.object_path.name)
        })
}

fn danger_table_fk_row(
    select: Entity<SelectState<SearchableVec<ForeignKeyCheckOption>>>,
    running: bool,
    colors: UiColors,
) -> impl IntoElement {
    div()
        .debug_selector(|| "danger-table-fk-row".into())
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(76.))
                .text_size(px(13.))
                .text_color(colors.muted)
                .child(tr("app.table.danger.foreign_key_check")),
        )
        .child(
            div()
                .debug_selector(|| "danger-table-fk-select".into())
                .flex_1()
                .child(
                    Select::new(&select)
                        .placeholder(tr("app.table.danger.default"))
                        .disabled(running)
                        .w_full()
                        .h(px(34.))
                        .menu_width(px(160.)),
                ),
        )
}

fn danger_table_ack_row(
    form: &PendingDangerTableAction,
    running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let acknowledged = form.acknowledged;
    div()
        .h(px(34.))
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(colors.hover))
        .child(Checkbox::new("danger-table-ack").checked(acknowledged))
        .child(
            div()
                .text_size(px(13.))
                .child(tr("app.table.danger.acknowledge")),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if !running {
                    this.set_danger_table_acknowledged(!acknowledged, cx);
                }
                cx.stop_propagation();
            }),
        )
}

/// PG 清空表：重置自增序列（RESTART IDENTITY vs CONTINUE IDENTITY）显式选项。
fn danger_table_restart_identity_row(
    form: &PendingDangerTableAction,
    running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let restart = form.restart_identity;
    div()
        .h(px(34.))
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(colors.hover))
        .child(Checkbox::new("danger-table-restart-identity").checked(restart))
        .child(
            div()
                .text_size(px(13.))
                .child(tr("app.table.danger.restart_identity")),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if !running {
                    this.set_danger_table_restart_identity(!restart, cx);
                }
                cx.stop_propagation();
            }),
        )
}
