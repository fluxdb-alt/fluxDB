// 仅共用渲染：同名处理、复制选项和提交回调按操作区分，业务状态仍由各入口维护。
enum TableNameForm {
    Rename(PendingRenameTable),
    Copy(PendingCopyTable),
}

fn table_name_form_modal(
    form: TableNameForm,
    sql_preview: Result<String, String>,
    name_input: Entity<InputState>,
    running: bool,
    focus_handle: FocusHandle,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let is_copy = matches!(&form, TableNameForm::Copy(_));
    let (object_path, new_name, form_error, copy_data) = match form {
        TableNameForm::Rename(form) => (form.object_path, form.new_name, form.error, None),
        TableNameForm::Copy(form) => (
            form.object_path,
            form.new_name,
            form.error,
            Some(form.copy_data),
        ),
    };
    let no_change = object_path.name.trim() == new_name.trim();
    let (sql, validation_error) = if no_change {
        if is_copy {
            (None, Some(tr("app.table.error.same_name").to_string()))
        } else {
            (None, None)
        }
    } else {
        match sql_preview {
            Ok(sql) => (Some(sql), None),
            Err(message) => (None, Some(message)),
        }
    };
    let decision_error = form_error.as_ref().or(validation_error.as_ref());
    let can_submit = !running && sql.is_some() && decision_error.is_none();
    // 复制失败已由主窗口 Alert 展示；弹窗内不再重复同一段文字，但仍阻止提交。
    let visible_error = if is_copy && form_error.is_some() {
        None
    } else {
        form_error
    };
    let error = visible_error.or(validation_error);
    let (title, field_label, key_context, cancel_id, confirm_id, input_id, preview_key, icon) =
        if is_copy {
            (
                tr("app.table.copy.title"),
                tr("app.table.copy.new_name"),
                "CopyTableModal",
                "copy-table-cancel",
                "copy-table-confirm",
                "copy-table-name",
                "copy-table-sql-preview",
                AppIcon::Copy,
            )
        } else {
            (
                tr("app.table.rename.title"),
                tr("app.table.rename.new_name"),
                "RenameTableModal",
                "rename-table-cancel",
                "rename-table-confirm",
                "rename-table-name",
                "rename-table-sql-preview",
                AppIcon::Edit,
            )
        };
    let cancel: fn(&mut NavicatMain, &mut Context<NavicatMain>) = if is_copy {
        NavicatMain::cancel_copy_table_modal
    } else {
        NavicatMain::cancel_rename_table_modal
    };
    let confirm: fn(&mut NavicatMain, &mut Window, &mut Context<NavicatMain>) = if is_copy {
        NavicatMain::confirm_copy_table
    } else {
        NavicatMain::confirm_rename_table
    };

    div()
        .id(key_context)
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
                cancel(this, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .id((key_context, 0u64))
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
                .key_context(key_context)
                .on_action(cx.listener(move |this, _: &CancelDialog, _, cx| {
                    cancel(this, cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
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
                                .child(app_icon_box(icon, 24., 16., rgb(0x2563eb)))
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_size(ui_text_size(15.))
                                        .child(format!("{title}：{}", object_path.name)),
                                ),
                        )
                        .child(
                            user_admin_icon_button(AppIcon::Close, colors)
                                .id((key_context, 1u64))
                                .aria_label(tr("app.common.close"))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        cancel(this, cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(table_name_field(
                            input_id,
                            field_label,
                            name_input,
                            running,
                            colors,
                        ))
                        .when_some(copy_data, |this, copy_data| {
                            this.child(copy_table_mode_rows(copy_data, running, colors, cx))
                        })
                        .when_some(sql, |this, sql| {
                            this.child(table_sql_preview_box(preview_key, sql, window, colors, cx))
                        })
                        .when_some(error, |this, error| {
                            this.child(
                                div()
                                    .id((key_context, 2u64))
                                    .debug_selector(|| format!("{key_context}-error"))
                                    .aria_label(error.clone())
                                    .text_size(ui_text_size(12.))
                                    .text_color(colors.danger)
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
                            Button::new(cancel_id)
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .cursor_pointer()
                                .disabled(running)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cancel(this, cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new(confirm_id)
                                .debug_selector(|| format!("{key_context}-confirm"))
                                .label(if running {
                                    tr("app.common.running")
                                } else if is_copy {
                                    tr("app.common.confirm")
                                } else {
                                    tr("app.common.execute")
                                })
                                .primary()
                                .w(px(78.))
                                .cursor_pointer()
                                .disabled(!can_submit)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    confirm(this, window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn table_name_field(
    id: &'static str,
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    running: bool,
    colors: UiColors,
) -> impl IntoElement {
    let label: SharedString = label.into();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label.clone()),
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
                        .id(id)
                        .aria_label(label.clone())
                        .appearance(false)
                        .focus_bordered(false)
                        .disabled(running)
                        .w_full()
                        .h_full()
                        .text_size(ui_text_size(13.)),
                ),
        )
}

fn table_sql_preview_box(
    key: &'static str,
    sql: String,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // DDL 是多行文本；单行 Input 会删掉换行，导致每次渲染都判断为未同步并重置内容。
    let editor = window.use_keyed_state(key, cx, {
        let sql = sql.clone();
        move |window, cx| {
            gpui_component::input::TextareaState::new(window, cx)
                .default_value(sql)
        }
    });
    editor.update(cx, |state, cx| {
        if state.value().to_string() != sql {
            state.set_value(sql.clone(), window, cx);
        }
    });

    div()
        .debug_selector(move || key.to_string())
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(11.))
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
                    gpui_component::input::Textarea::new(&editor)
                        .appearance(false)
                        .bordered(false)
                        .readonly(true)
                        .text_size(ui_text_size(12.))
                        .font_family(EDITOR_FONT)
                        .p_2()
                        .size_full(),
                ),
        )
}
