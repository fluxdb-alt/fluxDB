fn copy_table_modal(
    form: PendingCopyTable,
    sql_preview: Result<String, String>,
    name_input: Entity<InputState>,
    running: bool,
    focus_handle: FocusHandle,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let no_change = form.object_path.name.trim() == form.new_name.trim();
    let (sql, validation_error) = if no_change {
        (None, Some("新表名不能和原表相同".to_string()))
    } else {
        match sql_preview {
            Ok(sql) => (Some(sql), None),
            Err(message) => (None, Some(message)),
        }
    };
    let error = form.error.clone().or(validation_error);
    let can_submit = !running && sql.is_some() && error.is_none();

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
                this.cancel_copy_table_modal(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(560.))
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
                .key_context("CopyTableModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_copy_table_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(copy_table_header(&form.object_path.name, colors, cx))
                .child(
                    div()
                        .px_5()
                        .pb_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(rename_table_text_field("复制后的表名", name_input, running, colors))
                        .child(copy_table_mode_rows(form.copy_data, running, colors, cx))
                        .when_some(sql, |this, sql| {
                            this.child(rename_table_sql_preview_box(sql, window, colors, cx))
                        })
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
                            Button::new("copy-table-cancel")
                                .label("取消")
                                .w(px(78.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_copy_table_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("copy-table-confirm")
                                .label(if running { "执行中" } else { "确认" })
                                .primary()
                                .w(px(78.))
                                .disabled(!can_submit)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_copy_table(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn copy_table_header(table_name: &str, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
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
                .child(app_icon_box(AppIcon::Copy, 24., 16., rgb(0x2563eb)))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child(format!("复制表：{table_name}")),
                ),
        )
        .child(
            user_admin_icon_button(AppIcon::Close, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.cancel_copy_table_modal(cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn copy_table_mode_rows(
    copy_data: bool,
    running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child("复制内容"),
        )
        .child(copy_table_mode_row("仅结构", !copy_data, false, running, colors, cx))
        .child(copy_table_mode_row("结构和数据", copy_data, true, running, colors, cx))
}

fn copy_table_mode_row(
    label: &'static str,
    checked: bool,
    copy_data: bool,
    running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .h(px(34.))
        .px_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(if checked {
            rgb(0x2563eb)
        } else {
            colors.border_soft
        })
        .bg(if checked {
            colors.tree_selected
        } else {
            colors.panel_bg
        })
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.border_color(colors.border))
        .child(Checkbox::new(("copy-table-mode", copy_data as u64)).checked(checked))
        .child(div().text_size(px(13.)).child(label))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if !running {
                    this.set_copy_table_data_mode(copy_data, cx);
                }
                cx.stop_propagation();
            }),
        )
}
