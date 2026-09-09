fn rename_table_modal(
    form: PendingRenameTable,
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
        (None, None)
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
                this.cancel_rename_table_modal(cx);
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
                .key_context("RenameTableModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_rename_table_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(rename_table_header(&form.object_path.name, colors, cx))
                .child(
                    div()
                        .px_5()
                        .pb_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(rename_table_text_field("新表名", name_input, running, colors))
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
                            Button::new("rename-table-cancel")
                                .label("取消")
                                .w(px(78.))
                                .disabled(running)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_rename_table_modal(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("rename-table-confirm")
                                .label(if running { "执行中" } else { "执行" })
                                .primary()
                                .w(px(78.))
                                .disabled(!can_submit)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_rename_table(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn rename_table_header(table_name: &str, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
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
                .child(app_icon_box(AppIcon::Edit, 24., 16., rgb(0x2563eb)))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child(format!("重命名表：{table_name}")),
                ),
        )
        .child(
            user_admin_icon_button(AppIcon::Close, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.cancel_rename_table_modal(cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn rename_table_text_field(
    label: &'static str,
    input: Entity<InputState>,
    running: bool,
    colors: UiColors,
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
                .child(label),
        )
        .child(
            div()
                .h(px(34.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .child(
                    Input::new(&input)
                        .appearance(false)
                        .focus_bordered(false)
                        .disabled(running)
                        .w_full()
                        .h_full()
                        .text_size(px(13.)),
                ),
        )
}

fn rename_table_sql_preview_box(
    sql: String,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    table_sql_preview_box("rename-table-sql-preview", sql, window, colors, cx)
}

fn table_sql_preview_box(
    key: &'static str,
    sql: String,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let editor = window.use_keyed_state(key, cx, {
        let sql = sql.clone();
        move |window, cx| {
            InputState::new(window, cx)
                .code_editor(SQL_HIGHLIGHT_LANGUAGE)
                .line_number(false)
                .legacy_soft_wrap(false)
                .default_value(sql)
        }
    });
    editor.update(cx, |state, cx| {
        if state.value().to_string() != sql {
            state.set_value(sql.clone(), window, cx);
        }
    });

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child("SQL"),
        )
        .child(
            div()
                .h(px(56.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border_soft)
                .bg(colors.panel_alt)
                .overflow_hidden()
                .child(
                    Input::new(&editor)
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false)
                        .text_size(px(12.))
                        .font_family(EDITOR_FONT)
                        .p_2()
                        .size_full(),
                ),
        )
}
