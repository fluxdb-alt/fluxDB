/// 「结果布局」设置项的两个选项目标（显示文案, 下标）。顺序即面板按钮顺序。
fn results_placement_choices() -> Vec<(SharedString, u64)> {
    vec![
        (
            tr("app.settings.editor.results_bottom"),
            ResultsPlacement::Bottom.to_index(),
        ),
        (
            tr("app.settings.editor.results_right"),
            ResultsPlacement::Right.to_index(),
        ),
    ]
}

fn dangerous_sql_action_text(
    action: &fluxdb_core::DangerousSqlAction,
) -> (SharedString, SharedString) {
    match action.key {
        "drop" => (
            tr("app.settings.dangerous_sql.drop"),
            tr("app.settings.dangerous_sql.drop_detail"),
        ),
        "truncate" => (
            tr("app.settings.dangerous_sql.truncate"),
            tr("app.settings.dangerous_sql.truncate_detail"),
        ),
        "update_without_where" => (
            tr("app.settings.dangerous_sql.update_without_where"),
            tr("app.settings.dangerous_sql.update_without_where_detail"),
        ),
        "delete_without_where" => (
            tr("app.settings.dangerous_sql.delete_without_where"),
            tr("app.settings.dangerous_sql.delete_without_where_detail"),
        ),
        "alter_table" => (
            tr("app.settings.dangerous_sql.alter_table"),
            tr("app.settings.dangerous_sql.alter_table_detail"),
        ),
        _ => (action.title.into(), action.description.into()),
    }
}

fn shortcut_text(definition: &ShortcutDefinition) -> (SharedString, SharedString) {
    match definition.id {
        "app.new_query" => (
            tr("app.shortcuts.new_query"),
            tr("app.shortcuts.new_query_detail"),
        ),
        "app.refresh" => (tr("app.shortcuts.refresh"), tr("app.shortcuts.refresh_detail")),
        "app.save_or_apply" => (
            tr("app.shortcuts.save_or_apply"),
            tr("app.shortcuts.save_or_apply_detail"),
        ),
        "app.close_current_tab" => (
            tr("app.shortcuts.close_current_tab"),
            tr("app.shortcuts.close_current_tab_detail"),
        ),
        "app.toggle_connection_browser" => (
            tr("app.shortcuts.toggle_connection_browser"),
            tr("app.shortcuts.toggle_connection_browser_detail"),
        ),
        "query.execute_or_apply" => (
            tr("app.shortcuts.execute_or_apply"),
            tr("app.shortcuts.execute_or_apply_detail"),
        ),
        "query.search_data" => (
            tr("app.shortcuts.search_data"),
            tr("app.shortcuts.search_data_detail"),
        ),
        "query.search_history" => (
            tr("app.shortcuts.search_history"),
            tr("app.shortcuts.search_history_detail"),
        ),
        "app.quick_open_table" => (
            tr("app.shortcuts.quick_open_table"),
            tr("app.shortcuts.quick_open_table_detail"),
        ),
        "query.copy_data" => (
            tr("app.shortcuts.copy_data"),
            tr("app.shortcuts.copy_data_detail"),
        ),
        _ => (definition.title.into(), definition.detail.into()),
    }
}

fn settings_editor_panel(
    settings: Settings,
    colors: UiColors,
    dangerous_actions_collapsed: bool,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group(tr("app.settings.editor.group.sql_execution"), colors)
                .child(settings_choice_row_with_id(
                    "settings-editor-query-limit",
                    tr("app.settings.editor.query_limit"),
                    tr("app.settings.editor.query_limit_detail"),
                    AppIcon::Table,
                    settings.page_size,
                    &[
                        (tr("app.settings.editor.unlimited"), 0),
                        ("100".into(), 100),
                        ("500".into(), 500),
                        ("1000".into(), 1000),
                    ],
                    |settings, value| settings.page_size = value,
                    colors,
                    false,
                    cx,
                ))
                .child(settings_checkbox_row(
                    tr("app.settings.editor.confirm_dangerous_sql"),
                    tr("app.settings.editor.confirm_dangerous_sql_detail"),
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
                            tr("app.settings.editor.dangerous_actions"),
                            tr("app.settings.editor.dangerous_actions_detail"),
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
                        let (title, detail) = dangerous_sql_action_text(action);
                        settings_checkbox_row(
                            title,
                            detail,
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
            // 单独成组：该项同时作用于 SQL 查询结果面板与 Redis Workbench 结果区，
            // 塞进「SQL 执行」或「Redis」任何一边都不贴切。
            settings_panel_group(tr("app.settings.editor.group.results_layout"), colors)
                .child(settings_choice_row_with_id(
                    "settings-editor-results-placement",
                    tr("app.settings.editor.results_placement"),
                    tr("app.settings.editor.results_placement_detail"),
                    AppIcon::PanelBottom,
                    settings.results_placement.to_index(),
                    &results_placement_choices(),
                    |settings, value| settings.results_placement = ResultsPlacement::from_index(value),
                    colors,
                    false,
                    cx,
                )),
        )
        .child(
            settings_panel_group(tr("app.settings.editor.group.redis"), colors)
                .child(settings_checkbox_row(
                    tr("app.settings.editor.confirm_dangerous_redis"),
                    tr("app.settings.editor.confirm_dangerous_redis_detail"),
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
            settings_panel_group(tr("app.settings.editor.group.editing"), colors)
                .child(settings_choice_row_with_id(
                    "settings-editor-tab-width",
                    tr("app.settings.editor.tab_width"),
                    tr("app.settings.editor.tab_width_detail"),
                    AppIcon::AlignLeft,
                    settings.editor_tab_width as u64,
                    &[
                        ("2".into(), 2),
                        ("4".into(), 4),
                        ("8".into(), 8),
                    ],
                    |settings, value| settings.editor_tab_width = value as u32,
                    colors,
                    false,
                    cx,
                ))
                .child(settings_checkbox_row(
                    tr("app.settings.editor.word_wrap"),
                    tr("app.settings.editor.word_wrap_detail"),
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
        (
            tr("app.settings.shortcuts.group.application"),
            &SHORTCUT_DEFINITIONS[..5],
        ),
        (
            tr("app.settings.shortcuts.group.query_data"),
            &SHORTCUT_DEFINITIONS[5..],
        ),
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
        settings_panel_group(tr("app.settings.shortcuts.group.sql_editor"), colors)
            .child(settings_preference_row(
                tr("app.settings.shortcuts.undo"),
                tr("app.settings.shortcuts.undo_detail"),
                AppIcon::Undo,
                "⌘Z / Ctrl+Z",
                colors,
            ))
            .child(settings_preference_row(
                tr("app.settings.shortcuts.redo"),
                tr("app.settings.shortcuts.redo_detail"),
                AppIcon::Redo,
                "⇧⌘Z / Ctrl+Y",
                colors,
            ))
            .child(settings_preference_row(
                tr("app.settings.shortcuts.complete"),
                tr("app.settings.shortcuts.complete_detail"),
                AppIcon::Wand,
                "Ctrl+Space",
                colors,
            ))
            .child(settings_preference_row(
                tr("app.settings.shortcuts.toggle_comment"),
                tr("app.settings.shortcuts.toggle_comment_detail"),
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
    let (title, detail) = shortcut_text(&definition);
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
                        let (conflict_title, _) = shortcut_text(conflict);
                        this.show_message(
                            format!(
                                "{} {}",
                                tr("app.settings.shortcuts.conflict_prefix"),
                                conflict_title
                            ),
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
                        tr("app.settings.shortcuts.invalid_keystroke")
                    } else {
                        tr("app.settings.shortcuts.press_shortcut")
                    }),
            )
            .child(
                Button::new(SharedString::from(format!("shortcut-cancel-{}", definition.id)))
                    .label(tr("app.common.cancel"))
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
                    .tooltip(tr("app.settings.shortcuts.reset_default"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        reset_shortcut(this, definition, &capture_state, cx);
                    })),
            )
            .into_any_element()
    };

    settings_row_container(definition.id, colors)
        .min_h(px(52.))
        .child(settings_row_label(title, detail, definition.icon, colors))
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

