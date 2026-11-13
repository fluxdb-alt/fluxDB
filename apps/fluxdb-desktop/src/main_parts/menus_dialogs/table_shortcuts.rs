/// 设置页「表快捷组」区块：列表 + 新增入口。
fn settings_table_shortcut_groups(
    settings: &Settings,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> GroupBox {
    let mut group = settings_panel_group(tr("app.table_shortcuts.group"), colors);
    if settings.table_shortcut_groups.is_empty() {
        group = group.child(
            div()
                .px_3()
                .py_4()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.table_shortcuts.empty")),
        );
    } else {
        for index in 0..settings.table_shortcut_groups.len() {
            group = group.child(settings_table_shortcut_group_row(
                settings, index, colors, window, cx,
            ));
        }
    }
    let at_limit = settings.table_shortcut_groups.len() >= TABLE_SHORTCUT_GROUP_LIMIT;
    group.child(
        div()
            .border_t_1()
            .border_color(colors.border_soft)
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(tr("app.table_shortcuts.group_hint")),
            )
            .child(
                Button::new("table-shortcut-group-add")
                    .small()
                    .ghost()
                    .disabled(at_limit)
                    .child(tr("app.table_shortcuts.add_group"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if at_limit {
                            this.show_message(
                                tr("app.table_shortcuts.group_limit").as_ref(),
                                AppMessageKind::Warning,
                                cx,
                            );
                            return;
                        }
                        this.open_table_shortcut_dialog_new(window, cx);
                    })),
            ),
    )
}

fn settings_table_shortcut_group_row(
    settings: &Settings,
    index: usize,
    colors: UiColors,
    _window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let Some(group) = settings.table_shortcut_groups.get(index) else {
        return div().id("table-shortcut-group-missing");
    };
    let name = table_shortcut_group_name(group);
    let behavior = table_shortcut_behavior_label(group);
    let problem = table_shortcut_group_problem(settings, index);
    settings_row_container_dynamic(format!("table-shortcut-group:{index}"), colors)
        .min_h(px(58.))
        .py_2()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(name),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(behavior),
                )
                .when_some(problem, |this, problem| {
                    this.child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.warning)
                            .child(problem),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_none()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded(colors.radius)
                        .bg(colors.panel_alt)
                        .text_size(ui_text_size(11.))
                        .text_color(if group.keys.is_empty() {
                            colors.muted
                        } else {
                            colors.text
                        })
                        .child(if group.keys.is_empty() {
                            tr("app.table_shortcuts.unset")
                        } else {
                            shortcut_display(&group.keys).into()
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("table-shortcut-edit:{index}")))
                        .small()
                        .ghost()
                        .child(tr("app.table_shortcuts.manage"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_table_shortcut_dialog_at(index, window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("table-shortcut-clear:{index}")))
                        .small()
                        .ghost()
                        .child(tr("app.table_shortcuts.clear"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.clear_table_shortcut_group(index, cx);
                        })),
                ),
        )
}

fn table_shortcut_behavior_label(group: &TableShortcutGroup) -> String {
    match group.targets.len() {
        0 => tr("app.table_shortcuts.behavior_empty").to_string(),
        1 => tr("app.table_shortcuts.behavior_direct").to_string(),
        count => format!(
            "{count} {}",
            tr("app.table_shortcuts.behavior_picker")
        ),
    }
}

/// 组不可绑定时的原因；可绑定时返回 None。
fn table_shortcut_group_problem(settings: &Settings, index: usize) -> Option<String> {
    if table_shortcut_group_bindable(settings, index) {
        return None;
    }
    let group = settings.table_shortcut_groups.get(index)?;
    if group.targets.is_empty() {
        return Some(tr("app.table_shortcuts.no_tables").to_string());
    }
    if validate_table_shortcut_spec(&group.keys).is_err() {
        return Some(tr("app.table_shortcuts.invalid_shortcut").to_string());
    }
    Some(
        table_shortcut_conflict_message(settings, &group.keys, Some(&group.id))
            .unwrap_or_else(|| tr("app.table_shortcuts.conflict_group").to_string()),
    )
}

/// 编辑表快捷组弹窗。
fn table_shortcut_dialog_modal(
    dialog: &TableShortcutDialog,
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_save = !dialog.targets.is_empty()
        && !dialog.keys.is_empty()
        && validate_table_shortcut_spec(&dialog.keys).is_ok()
        && dialog.conflict.is_none();
    let capture_text = if dialog.capturing {
        tr("app.table_shortcuts.press_shortcut")
    } else if dialog.invalid {
        tr("app.table_shortcuts.invalid_shortcut")
    } else if dialog.keys.is_empty() {
        tr("app.table_shortcuts.record")
    } else {
        shortcut_display(&dialog.keys).into()
    };
    let capture_color = if dialog.invalid {
        colors.danger
    } else if dialog.keys.is_empty() {
        colors.muted
    } else {
        colors.text
    };
    let at_target_limit = dialog.targets.len() >= TABLE_SHORTCUT_TARGET_LIMIT;
    let is_editing = dialog.group_index.is_some();

    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.5)
        } else {
            opaque_grey(0.6, 0.28)
        })
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_table_shortcut_dialog(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(560.))
                .max_h(px(680.))
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
                .flex()
                .flex_col()
                .overflow_hidden()
                .text_size(ui_text_size(13.))
                .text_color(colors.text)
                .key_context("TableShortcutDialog")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_4()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(tr("app.table_shortcuts.dialog_title")),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(tr("app.table_shortcuts.dialog_detail")),
                        ),
                )
                .child(
                    div()
                        .px_4()
                        .pb_3()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(86.))
                                        .flex_none()
                                        .text_size(ui_text_size(12.))
                                        .text_color(colors.muted)
                                        .child(tr("app.table_shortcuts.name")),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .child(Input::new(&dialog.name_input).small()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(86.))
                                        .flex_none()
                                        .text_size(ui_text_size(12.))
                                        .text_color(colors.muted)
                                        .child(tr("app.table_shortcuts.shortcut")),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .h(px(38.))
                                        .rounded(colors.radius)
                                        .border_1()
                                        .border_color(if dialog.capturing {
                                            colors.info
                                        } else if dialog.invalid {
                                            colors.danger
                                        } else {
                                            colors.border
                                        })
                                        .bg(colors.input_bg)
                                        .px_3()
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .track_focus(&dialog.focus_handle)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, window, cx| {
                                                if let Some(dialog) =
                                                    this.table_shortcut_dialog.as_mut()
                                                {
                                                    dialog.capturing = true;
                                                    dialog.invalid = false;
                                                }
                                                this.start_table_shortcut_capture(window, cx);
                                                cx.stop_propagation();
                                            }),
                                        )
                                        .on_key_down(cx.listener(
                                            NavicatMain::table_shortcut_dialog_key_down,
                                        ))
                                        .child(
                                            div()
                                                .text_size(ui_text_size(12.))
                                                .text_color(capture_color)
                                                .child(capture_text),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .pl(px(98.))
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(tr("app.table_shortcuts.shortcut_hint")),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .text_color(colors.muted)
                                        .child(format!(
                                            "{} {}/{}",
                                            tr("app.table_shortcuts.tables"),
                                            dialog.targets.len(),
                                            TABLE_SHORTCUT_TARGET_LIMIT
                                        )),
                                )
                                .child(
                                    Button::new("table-shortcut-add-table")
                                        .small()
                                        .ghost()
                                        .disabled(at_target_limit)
                                        .child(tr("app.table_shortcuts.add_table"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if at_target_limit {
                                                this.show_message(
                                                    tr("app.table_shortcuts.target_limit")
                                                        .as_ref(),
                                                    AppMessageKind::Warning,
                                                    cx,
                                                );
                                                return;
                                            }
                                            this.open_quick_open_table_for_shortcut(window, cx);
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .max_h(px(240.))
                                .overflow_y_scrollbar()
                                .rounded(colors.radius)
                                .border_1()
                                .border_color(colors.border_soft)
                                .children(dialog.targets.iter().enumerate().map(
                                    |(index, target)| {
                                        table_shortcut_dialog_target_row(
                                            target, index, state, colors, cx,
                                        )
                                    },
                                ))
                                .when(dialog.targets.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .h(px(72.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(ui_text_size(12.))
                                            .text_color(colors.muted)
                                            .child(tr("app.table_shortcuts.no_tables")),
                                    )
                                }),
                        )
                        .when_some(dialog.conflict.clone(), |this, conflict| {
                            this.child(
                                div()
                                    .rounded(colors.radius)
                                    .border_1()
                                    .border_color(colors.warning)
                                    .bg(colors.panel_alt)
                                    .px_3()
                                    .py_2()
                                    .text_size(ui_text_size(11.))
                                    .text_color(colors.warning)
                                    .child(conflict),
                            )
                        }),
                )
                .child(
                    div()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(is_editing, |this| {
                            this.child(
                                Button::new("table-shortcut-delete-group")
                                    .small()
                                    .ghost()
                                    .child(tr("app.table_shortcuts.delete_group"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.delete_table_shortcut_dialog_group(cx);
                                    })),
                            )
                        })
                        .child(div().flex_1())
                        .child(
                            Button::new("table-shortcut-cancel")
                                .small()
                                .ghost()
                                .child(tr("app.common.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_table_shortcut_dialog(cx);
                                })),
                        )
                        .child(
                            Button::new("table-shortcut-save")
                                .small()
                                .primary()
                                .disabled(!can_save)
                                .child(tr("app.common.save"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_table_shortcut_dialog(cx);
                                })),
                        ),
                ),
        )
}

fn table_shortcut_dialog_target_row(
    target: &TableShortcutTarget,
    index: usize,
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let unavailable = table_shortcut_target_unavailable(state, target);
    let detail = table_shortcut_target_detail(target);
    div()
        .border_t_1()
        .border_color(colors.border_soft)
        .px_3()
        .h(px(48.))
        .flex()
        .items_center()
        .gap_3()
        .child(app_icon(AppIcon::Table, 14., colors.muted))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.text)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(target.name.clone()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(detail),
                ),
        )
        .when_some(unavailable, |this, reason| {
            this.child(
                div()
                    .flex_none()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.warning)
                    .child(reason),
            )
        })
        .child(
            div()
                .size(px(26.))
                .flex_none()
                .rounded(colors.radius)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    this.remove_table_shortcut_dialog_target(index, cx);
                    cx.stop_propagation();
                }))
                .child(app_icon(AppIcon::Close, 14., colors.muted)),
        )
}

/// 多表快捷组选择器。
fn table_shortcut_picker_modal(
    picker: &TableShortcutPicker,
    settings: &Settings,
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(group) = settings.table_shortcut_groups.get(picker.group_index) else {
        return div();
    };
    let selected = picker.selected.min(group.targets.len().saturating_sub(1));
    let selected_bg = if colors.is_dark {
        rgb(0x263244)
    } else {
        rgb(0xe8f2ff)
    };
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.48)
        } else {
            opaque_grey(0.6, 0.24)
        })
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_table_shortcut_picker(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(520.))
                .max_h(px(520.))
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
                .flex()
                .flex_col()
                .overflow_hidden()
                .track_focus(&picker.focus_handle)
                .key_context("TableShortcutPicker")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(52.))
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(table_shortcut_group_name(group)),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(shortcut_display(&group.keys)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .p_2()
                        .children(group.targets.iter().enumerate().map(|(index, target)| {
                            let unavailable = table_shortcut_target_unavailable(state, target);
                            let detail = table_shortcut_target_detail(target);
                            let is_selected = index == selected;
                            div()
                                .h(px(52.))
                                .rounded(colors.radius_lg)
                                .px_3()
                                .flex()
                                .items_center()
                                .gap_3()
                                .cursor_pointer()
                                .bg(if is_selected {
                                    selected_bg
                                } else {
                                    colors.panel_bg
                                })
                                .hover(move |style| {
                                    style.bg(if is_selected {
                                        selected_bg
                                    } else {
                                        colors.hover
                                    })
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        if let Some(picker) =
                                            this.table_shortcut_picker.as_mut()
                                        {
                                            picker.selected = index;
                                        }
                                        this.confirm_table_shortcut_picker(cx);
                                        cx.stop_propagation();
                                    }),
                                )
                                .child(
                                    div()
                                        .size(px(22.))
                                        .flex_none()
                                        .rounded(colors.radius)
                                        .bg(colors.panel_alt)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(ui_text_size(11.))
                                        .text_color(colors.muted)
                                        .child((index + 1).to_string()),
                                )
                                .child(app_icon(AppIcon::Table, 14., colors.muted))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(ui_text_size(13.))
                                                .text_color(colors.text)
                                                .whitespace_nowrap()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(target.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(ui_text_size(11.))
                                                .text_color(colors.muted)
                                                .whitespace_nowrap()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(detail),
                                        ),
                                )
                                .when_some(unavailable, |this, reason| {
                                    this.child(
                                        div()
                                            .flex_none()
                                            .text_size(ui_text_size(11.))
                                            .text_color(colors.warning)
                                            .child(reason),
                                    )
                                })
                        })),
                )
                .child(
                    div()
                        .min_h(px(42.))
                        .py_2()
                        .px_4()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_4()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(tr("app.table_shortcuts.picker_up_down"))
                        .child(tr("app.table_shortcuts.picker_direct"))
                        .child(tr("app.table_shortcuts.picker_enter"))
                        .child(tr("app.table_shortcuts.picker_escape"))
                        .child(tr("app.table_shortcuts.picker_auto_connect")),
                ),
        )
}

fn table_shortcut_target_detail(target: &TableShortcutTarget) -> String {
    let connection = if target.connection_name.is_empty() {
        format!("#{}", target.connection_id.0)
    } else {
        target.connection_name.clone()
    };
    let mut parts = vec![connection];
    if let Some(database) = &target.database {
        parts.push(database.clone());
    }
    if let Some(schema) = &target.schema {
        parts.push(schema.clone());
    }
    parts.join(" / ")
}

/// 目标不可用的原因：连接已删除或未打开。仅用于展示，不自动重连。
fn table_shortcut_target_unavailable(
    state: &AppState,
    target: &TableShortcutTarget,
) -> Option<String> {
    match state
        .connections
        .iter()
        .find(|connection| connection.config.id == target.connection_id)
    {
        None => Some(tr("app.table_shortcuts.connection_missing").to_string()),
        Some(connection) if !connection.connected => {
            Some(tr("app.table_shortcuts.connection_closed").to_string())
        }
        Some(_) => None,
    }
}
