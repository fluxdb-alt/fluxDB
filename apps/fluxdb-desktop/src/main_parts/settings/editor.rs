fn settings_editor_panel(
    settings: Settings,
    font_size_slider: Entity<SliderState>,
    colors: UiColors,
    line_height_input: Entity<InputState>,
    dangerous_actions_collapsed: bool,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group("字体", colors)
                .child(settings_font_size_slider_row(
                    "字体大小",
                    "控制 SQL 编辑器文字大小",
                    AppIcon::Text,
                    settings.editor_font_size,
                    font_size_slider,
                    colors,
                    window,
                    cx,
                ))
                .child(settings_input_row(
                    "行高",
                    "控制 SQL 编辑器每行高度",
                    AppIcon::List,
                    settings.editor_line_height,
                    &line_height_input,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(
            settings_panel_group("SQL 执行", colors)
                .child(settings_choice_row(
                    "默认查询限制",
                    "查询结果每页显示行数",
                    AppIcon::Table,
                    settings.page_size,
                    &[
                        ("不限制", 0),
                        ("100", 100),
                        ("500", 500),
                        ("1000", 1000),
                    ],
                    |settings, value| settings.page_size = value,
                    colors,
                    false,
                    cx,
                ))
                .child(settings_checkbox_row(
                    "执行危险 SQL 前弹出确认",
                    "勾选下方操作后，执行命中清单的 SQL 会先确认",
                    AppIcon::CircleSlash,
                    "settings-confirm-dangerous-sql",
                    settings.confirm_dangerous_sql,
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.settings_editor_draft.confirm_dangerous_sql =
                            !this.settings_editor_draft.confirm_dangerous_sql;
                        cx.notify();
                    }),
                ))
                .child(
                    settings_row_container("settings-dangerous-actions-toggle", colors)
                        .h(px(52.))
                        .cursor_pointer()
                        .child(settings_row_label(
                            "危险 SQL 操作清单",
                            "勾选哪些操作算危险（默认 DROP/TRUNCATE/无 WHERE 的 UPDATE/DELETE）",
                            AppIcon::CircleSlash,
                            colors,
                        ))
                        .child(app_icon_box(
                            if dangerous_actions_collapsed {
                                AppIcon::ChevronRight
                            } else {
                                AppIcon::ChevronDown
                            },
                            24.,
                            13.,
                            colors.muted,
                        ))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.settings_dangerous_actions_collapsed =
                                    !this.settings_dangerous_actions_collapsed;
                                cx.notify();
                            }),
                        ),
                )
                .when(!dangerous_actions_collapsed, |this| {
                    this.children(fluxdb_core::DangerousSqlAction::ALL.iter().map(|action| {
                        let key = action.key;
                        settings_checkbox_row(
                            action.title,
                            action.description,
                            AppIcon::CircleSlash,
                            key,
                            settings.dangerous_sql_actions.contains(key),
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                let draft = &mut this.settings_editor_draft.dangerous_sql_actions;
                                if !draft.insert(key.to_string()) {
                                    draft.remove(key);
                                }
                                cx.notify();
                            }),
                        )
                    }))
                }),
        )
        .child(
            settings_panel_group("Redis", colors)
                .child(settings_checkbox_row(
                    "执行危险命令前弹出确认",
                    "FLUSHDB、FLUSHALL、SHUTDOWN 等破坏性命令会先确认",
                    AppIcon::CircleSlash,
                    "settings-confirm-dangerous-redis",
                    settings.confirm_dangerous_redis,
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.settings_editor_draft.confirm_dangerous_redis =
                            !this.settings_editor_draft.confirm_dangerous_redis;
                        cx.notify();
                    }),
                )),
        )
        .child(
            settings_panel_group("编辑行为", colors)
                .child(settings_choice_row(
                    "Tab 宽度",
                    "按 Tab 缩进时插入的空格数",
                    AppIcon::AlignLeft,
                    settings.editor_tab_width as u64,
                    &[
                        ("2", 2),
                        ("4", 4),
                        ("8", 8),
                    ],
                    |settings, value| settings.editor_tab_width = value as u32,
                    colors,
                    false,
                    cx,
                ))
                .child(settings_checkbox_row(
                    "自动换行",
                    "控制 SQL 编辑器长行是否按窗口宽度换行",
                    AppIcon::WrapText,
                    "settings-editor-word-wrap",
                    settings.editor_word_wrap,
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.settings_editor_draft.editor_word_wrap =
                            !this.settings_editor_draft.editor_word_wrap;
                        cx.notify();
                    }),
                )),
        )
}

fn settings_shortcuts_panel(
    settings: &Settings,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let capture_state = window.use_keyed_state("shortcut-capture", cx, |_, cx| {
        ShortcutCaptureState {
            active_id: None,
            invalid: false,
            focus_handle: cx.focus_handle(),
        }
    });
    let mut panel = div().flex().flex_col().gap_3();
    for (title, definitions) in [
        ("应用", &SHORTCUT_DEFINITIONS[..5]),
        ("查询与数据", &SHORTCUT_DEFINITIONS[5..]),
    ] {
        let mut group = settings_panel_group(title, colors);
        for definition in definitions {
            group = group.child(settings_shortcut_row(
                settings,
                *definition,
                capture_state.clone(),
                colors,
                cx,
            ));
        }
        panel = panel.child(group);
    }
    panel.child(
        settings_panel_group("SQL 编辑器", colors)
            .child(settings_preference_row(
                "撤销",
                "撤销最近一次编辑操作",
                AppIcon::Undo,
                "⌘Z / Ctrl+Z",
                colors,
            ))
            .child(settings_preference_row(
                "重做",
                "恢复最近撤销的编辑操作",
                AppIcon::Redo,
                "⇧⌘Z / Ctrl+Y",
                colors,
            ))
            .child(settings_preference_row(
                "触发补全",
                "打开 SQL 关键字、表名和字段补全",
                AppIcon::Wand,
                "Ctrl+Space",
                colors,
            ))
            .child(settings_preference_row(
                "切换行注释",
                "为当前行或选中行添加 / 移除注释",
                AppIcon::Text,
                "⌘/ / Ctrl+/",
                colors,
            )),
    )
}

fn settings_shortcut_row(
    settings: &Settings,
    definition: ShortcutDefinition,
    capture_state: Entity<ShortcutCaptureState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let current = current_shortcut(settings, &definition);
    let editing = capture_state.read(cx).active_id == Some(definition.id);
    let focus_handle = capture_state.read(cx).focus_handle.clone();
    let value = if editing {
        div()
            .track_focus(&focus_handle)
            .on_key_down(cx.listener({
                let capture_state = capture_state.clone();
                move |this, event: &KeyDownEvent, _window, cx| {
                    if event.keystroke.key == "escape" {
                        capture_state.update(cx, |state, cx| {
                            state.active_id = None;
                            state.invalid = false;
                            cx.notify();
                        });
                        cx.stop_propagation();
                        return;
                    }
                    let Some(spec) = shortcut_spec_from_keystroke(&event.keystroke) else {
                        capture_state.update(cx, |state, cx| {
                            state.invalid = true;
                            cx.notify();
                        });
                        cx.stop_propagation();
                        return;
                    };
                    let settings = &this.settings_editor_draft;
                    if let Some(conflict) = SHORTCUT_DEFINITIONS.iter().find(|other| {
                        other.id != definition.id && current_shortcut(settings, other) == spec
                    }) {
                        this.show_message(
                            format!("快捷键已被“{}”使用", conflict.title),
                            AppMessageKind::Warning,
                            cx,
                        );
                        cx.stop_propagation();
                        return;
                    }
                    apply_shortcut_change(this, definition, &spec, &capture_state, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(if capture_state.read(cx).invalid {
                        if colors.is_dark { rgb(0xff7b72) } else { rgb(0xb42318) }
                    } else {
                        colors.border
                    })
                    .text_color(if capture_state.read(cx).invalid {
                        if colors.is_dark { rgb(0xff7b72) } else { rgb(0xb42318) }
                    } else {
                        colors.muted
                    })
                    .child(if capture_state.read(cx).invalid {
                        "请按下有效快捷键"
                    } else {
                        "请按下快捷键"
                    }),
            )
            .child(
                Button::new(SharedString::from(format!("shortcut-cancel-{}", definition.id)))
                    .label("取消")
                    .ghost()
                    .xsmall()
                    .rounded(colors.radius)
                    .on_click({
                        let capture_state = capture_state.clone();
                        move |_, _, cx| {
                            capture_state.update(cx, |state, cx| {
                                state.active_id = None;
                                state.invalid = false;
                                cx.notify();
                            });
                        }
                    }),
            )
            .into_any_element()
    } else {
        h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new(SharedString::from(format!("shortcut-value-{}", definition.id)))
                    .label(shortcut_display(&current))
                    .small()
                    .rounded(colors.radius)
                    .on_click({
                        let capture_state = capture_state.clone();
                        let focus_handle = focus_handle.clone();
                        move |_, window, cx| {
                            capture_state.update(cx, |state, cx| {
                                state.active_id = Some(definition.id);
                                state.invalid = false;
                                cx.notify();
                            });
                            focus_handle.focus(window, cx);
                        }
                    }),
            )
            .child(
                Button::new(SharedString::from(format!("shortcut-reset-{}", definition.id)))
                    .icon(IconName::Redo2)
                    .ghost()
                    .xsmall()
                    .rounded(colors.radius)
                    .tooltip("恢复默认快捷键")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        reset_shortcut(this, definition, &capture_state, cx);
                    })),
            )
            .into_any_element()
    };

    settings_row_container(definition.id, colors)
        .min_h(px(52.))
        .child(settings_row_label(
            definition.title,
            definition.detail,
            definition.icon,
            colors,
        ))
        .child(value)
}

fn shortcut_spec_from_keystroke(keystroke: &Keystroke) -> Option<String> {
    let key = keystroke.key.as_str();
    if matches!(key, "ctrl" | "control" | "alt" | "shift" | "cmd" | "win") {
        return None;
    }
    let mut parts = Vec::new();
    if keystroke.modifiers.control {
        parts.push("ctrl");
    }
    if keystroke.modifiers.alt {
        parts.push("alt");
    }
    if keystroke.modifiers.shift {
        parts.push("shift");
    }
    if keystroke.modifiers.platform {
        parts.push("cmd");
    }
    parts.push(key);
    Some(parts.join("-"))
}

fn apply_shortcut_change(
    this: &mut NavicatMain,
    definition: ShortcutDefinition,
    spec: &str,
    capture_state: &Entity<ShortcutCaptureState>,
    cx: &mut Context<NavicatMain>,
) {
    let mut settings = this.settings_editor_draft.clone();
    if spec == default_shortcut(&definition) {
        settings.custom_keybindings.remove(definition.id);
    } else {
        settings
            .custom_keybindings
            .insert(definition.id.to_string(), spec.to_string());
    }
    this.settings_editor_draft = settings;
    cx.notify();
    capture_state.update(cx, |state, cx| {
        state.active_id = None;
        state.invalid = false;
        cx.notify();
    });
}

fn reset_shortcut(
    this: &mut NavicatMain,
    definition: ShortcutDefinition,
    capture_state: &Entity<ShortcutCaptureState>,
    cx: &mut Context<NavicatMain>,
) {
    let mut settings = this.settings_editor_draft.clone();
    settings.custom_keybindings.remove(definition.id);
    this.settings_editor_draft = settings;
    cx.notify();
    capture_state.update(cx, |state, cx| {
        state.active_id = None;
        state.invalid = false;
        cx.notify();
    });
}

