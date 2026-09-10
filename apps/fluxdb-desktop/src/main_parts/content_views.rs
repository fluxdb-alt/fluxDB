// 结果区高度模型（上下分栏）：最小 = 编辑器高 × `RESULT_MIN_EDITOR_RATIO`（结果最小仍有意义），
// 最大 = 分栏高 × 各自的 `_MAX_SPLIT_RATIO`（结果可拉得较大，但为编辑器保留空间）。
// min：result = 0.20·(split−result) ⇒ split/6。
const RESULT_MIN_EDITOR_RATIO: f32 = 0.20;
// SQL 结果区最大 = 分栏 × 0.70（编辑器至少留 30%）；Workbench 结果区最大 = 分栏 × 0.80。
const QUERY_RESULT_MAX_SPLIT_RATIO: f32 = 0.70;
const REDIS_WB_RESULT_MAX_SPLIT_RATIO: f32 = 0.80;
// 默认结果区高：split×0.375（≈ min 与 max 之间，适中）。
const RESULT_DEFAULT_SPLIT_RATIO: f32 = 0.375;
const QUERY_OUTPUT_DEFAULT_WIDTH: f32 = 560.;
const QUERY_OUTPUT_MIN_WIDTH: f32 = 320.;
const SQL_EDITOR_MIN_WIDTH: f32 = 360.;
const QUERY_TOOLBAR_HEIGHT: f32 = 40.;
// Redis Workbench 上下分栏：编辑器区（上方）与结果区（下方）可拖动调整高度（同一结果区模型）。
const REDIS_WB_TOOLBAR_HEIGHT: f32 = 40.;
// 单条执行记录卡片的结果区最大高度：超长 reply 在卡片内滚动，避免把整张卡片 / 外层列表撑高。
const REDIS_WB_RECORD_BODY_MAX_HEIGHT: f32 = 320.;

/// 垂直分栏（编辑器在结果区上方）可用高度估算：视口高度扣除顶部工具条。
fn redis_workbench_split_height(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) - REDIS_WB_TOOLBAR_HEIGHT).max(0.)
}

/// 结果区高度（像素）夹到 [split/6, 0.70·split]（SQL）。
/// min：result=0.20·editor ⇒ split/6；max：直接占分栏 `QUERY_RESULT_MAX_SPLIT_RATIO`。
fn query_output_clamp_height(result_height: f32, split_height: f32) -> f32 {
    let min_result = split_height / (1. + 1. / RESULT_MIN_EDITOR_RATIO);
    let max_result = split_height * QUERY_RESULT_MAX_SPLIT_RATIO;
    if min_result >= max_result {
        split_height / 4.
    } else {
        result_height.clamp(min_result, max_result)
    }
}

/// 把编辑器区高度（像素）夹到「结果区 ∈ [split/6, 0.80·split]」对应的范围（Workbench）。
/// 结果最小 ⇒ editor 最大 = split − split/6 = 5·split/6；结果最大(0.80·split) ⇒ editor 最小 = 0.20·split。
/// 可用高过小时退化为对半，避免 `clamp` 因 min>max 触发 panic。
fn redis_workbench_clamp_editor_height(height: f32, split_height: f32) -> f32 {
    let min_editor = split_height * (1. - REDIS_WB_RESULT_MAX_SPLIT_RATIO);
    let max_editor = split_height * (1. - 1. / (1. + 1. / RESULT_MIN_EDITOR_RATIO));
    if min_editor >= max_editor {
        split_height / 2.
    } else {
        height.clamp(min_editor, max_editor)
    }
}

/// 由全局分栏占比换算编辑器区高度（像素）。
fn redis_workbench_editor_height(ratio: f32, split_height: f32) -> f32 {
    redis_workbench_clamp_editor_height(split_height * ratio, split_height)
}

fn content(
    state: &AppState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    match state.active_tab() {
        Some(tab) if matches!(tab.kind, TabKind::DataEditor(_)) => {
            let TabKind::DataEditor(editor) = &tab.kind else {
                unreachable!();
            };
            data_editor_content(tab.id, editor, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::QueryEditor(_)) => {
            let TabKind::QueryEditor(editor) = &tab.kind else {
                unreachable!();
            };
            query_editor_content(tab.id, editor, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::RedisWorkbench(_)) => {
            let TabKind::RedisWorkbench(workbench) = &tab.kind else {
                unreachable!();
            };
            redis_workbench_content(tab.id, workbench, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::RedisCli(_)) => {
            let TabKind::RedisCli(cli) = &tab.kind else {
                unreachable!();
            };
            redis_cli_content(tab.id, cli, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::RedisPubSub(_)) => {
            let TabKind::RedisPubSub(pubsub) = &tab.kind else {
                unreachable!();
            };
            redis_pubsub_content(tab.id, pubsub, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::CreateTable(_)) => {
            let TabKind::CreateTable(create) = &tab.kind else {
                unreachable!();
            };
            create_table_content(tab.id, create, this, window, colors, cx)
        }

        Some(tab) if matches!(tab.kind, TabKind::ObjectList(_)) => {
            object_list_content(state, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::BackupList(_)) => {
            let TabKind::BackupList(list) = &tab.kind else {
                unreachable!();
            };
            backup_list_content(state, this, list, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::UserAdmin(_)) => {
            let TabKind::UserAdmin(admin) = &tab.kind else {
                unreachable!();
            };
            user_admin_content(state, tab.id, admin, this, window, colors, cx)
        }
        Some(tab) if matches!(tab.kind, TabKind::Settings) => {
            settings_content(
                state,
                this.theme_mode,
                this.settings_panel_section,
                colors,
                this.settings_editor_draft.clone(),
                this.settings_font_size_slider.clone(),
                this.settings_line_height_input.clone(),
                this.settings_radius_input.clone(),
                window,
                cx,
            )
        }
        _ => workbench_home(state, window, colors, cx),
    }
}

fn workbench_home(
    state: &AppState,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let connected_count = state
        .connections
        .iter()
        .filter(|connection| connection.connected)
        .count();
    let kind_count = database_kind_count(state);
    let width = window.viewport_size().width;
    let compact_home = width < px(1180.);
    let stat_cols = if width < px(880.) {
        1
    } else if width < px(1280.) {
        2
    } else {
        3
    };

    div()
        .flex_1()
        .bg(colors.content_bg)
        .border_r_1()
        .border_color(colors.border)
        .overflow_hidden()
        .child(
            div().size_full().p_6().flex().justify_center().child(
                div()
                    .w_full()
                    .max_w(px(1180.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .grid()
                            .grid_cols(stat_cols)
                            .gap_3()
                            .child(home_stat_card(
                                "连接",
                                state.connections.len(),
                                "已保存连接",
                                colors,
                            ))
                            .child(home_stat_card(
                                "已连接",
                                connected_count,
                                "当前会话",
                                colors,
                            ))
                            .child(home_stat_card(
                                "数据库类型",
                                kind_count,
                                "已配置类型",
                                colors,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .when(compact_home, |this| this.flex_col())
                            .gap_4()
                            .child(recent_connections_panel(state, colors, cx).flex_1())
                            .child(
                                common_actions_panel(state, window, colors, cx)
                                    .when(!compact_home, |this| this.w(px(360.))),
                            ),
                    ),
            ),
        )
}

fn settings_content(
    state: &AppState,
    theme_mode: ThemeMode,
    active_section: SettingsPanelSection,
    colors: UiColors,
    editor_draft: Settings,
    font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    radius_input: Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .bg(colors.content_bg)
        .border_r_1()
        .border_color(colors.border)
        .overflow_hidden()
        .child(
            div()
                .size_full()
                .pt_2()
                .flex()
                .bg(colors.panel_bg)
                .overflow_hidden()
                .child(settings_sidebar(active_section, colors, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(settings_panel_header(
                            state,
                            active_section,
                            colors,
                            editor_draft.clone(),
                            font_size_slider.clone(),
                            line_height_input.clone(),
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .overflow_y_scrollbar()
                                .px_4()
                                .py_3()
                                .child(settings_panel_body(
                                    state,
                                    theme_mode,
                                    active_section,
                                    colors,
                                    editor_draft,
                                    font_size_slider,
                                    line_height_input,
                                    radius_input,
                                    window,
                                    cx,
                                )),
                        ),
                ),
        )
}

fn settings_sidebar(
    active_section: SettingsPanelSection,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Sidebar<SidebarMenu> {
    Sidebar::new("settings-sidebar")
        .w(px(176.))
        .flex_none()
        .h_full()
        .border_r_1()
        .border_color(colors.border)
        .bg(colors.sidebar_bg)
        .px_3()
        .py_2()
        .collapsible(false)
        .collapsed(false)
        .child(
            SidebarMenu::new().children(settings_panel_sections().into_iter().map(|section| {
                SidebarMenuItem::new(settings_section_label(section))
                    .active(section == active_section)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_panel_section = section;
                        cx.stop_propagation();
                        cx.notify();
                    }))
            })),
        )
}

fn settings_panel_sections() -> [SettingsPanelSection; 8] {
    [
        SettingsPanelSection::Editor,
        SettingsPanelSection::Shortcuts,
        SettingsPanelSection::Appearance,
        SettingsPanelSection::System,
        SettingsPanelSection::Data,
        SettingsPanelSection::ConnectionSecurity,
        SettingsPanelSection::DatabaseSupport,
        SettingsPanelSection::About,
    ]
}

fn settings_panel_header(
    state: &AppState,
    active_section: SettingsPanelSection,
    colors: UiColors,
    editor_draft: Settings,
    font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let reset_font_size_slider = font_size_slider.clone();
    let reset_line_height_input = line_height_input.clone();
    let save_line_height_input = line_height_input;
    div()
        .h(px(48.))
        .flex_none()
        .border_b_1()
        .border_color(colors.border)
        .px_4()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(settings_section_label(active_section)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(settings_section_description(active_section)),
                ),
        )
        .when(active_section == SettingsPanelSection::Editor, |this| {
            let dirty = settings_editor_changed(&state.settings, &editor_draft);
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("settings-editor-reset")
                            .label("恢复默认设置")
                            .small()
                            .rounded(colors.radius)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.settings_editor_draft =
                                    settings_default_editor_draft(&this.controller.state().settings);
                                let font_size = this.settings_editor_draft.editor_font_size;
                                reset_font_size_slider.update(cx, |slider, cx| {
                                    slider.set_value(font_size as f32, window, cx)
                                });
                                let value = this
                                    .settings_editor_draft
                                    .editor_line_height
                                    .to_string();
                                reset_line_height_input.update(cx, |input, cx| {
                                    input.set_value(value, window, cx)
                                });
                                this.show_message(
                                    "已恢复默认设置，点击保存后生效",
                                    AppMessageKind::Info,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("settings-editor-save")
                            .label("保存")
                            .small()
                            .rounded(colors.radius)
                            .primary()
                            .disabled(!dirty)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let Ok(line_height) = save_line_height_input
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .parse::<u32>()
                                else {
                                    this.show_message(
                                        "行高请输入 13-28 的数字",
                                        AppMessageKind::Warning,
                                        cx,
                                    );
                                    return;
                                };
                                let line_height = line_height.clamp(13, 28);
                                let mut settings = this.controller.state().settings.clone();
                                settings_apply_editor_fields(
                                    &mut settings,
                                    &this.settings_editor_draft,
                                );
                                settings.editor_line_height = line_height;
                                save_settings_from_ui(this, settings, "已保存编辑器设置", cx);
                                save_line_height_input.update(cx, |input, cx| {
                                    input.set_value(line_height.to_string(), window, cx)
                                });
                            })),
                    ),
            )
        })
        .when(active_section == SettingsPanelSection::Appearance, |this| {
            this.child(settings_appearance_header_actions(
                &state.settings,
                &editor_draft,
                cx,
            ))
        })
        .when(active_section == SettingsPanelSection::Shortcuts, |this| {
            this.child(settings_section_header_actions(
                SettingsPanelSection::Shortcuts,
                &state.settings,
                &editor_draft,
                cx,
            ))
        })
        .when(active_section == SettingsPanelSection::System, |this| {
            this.child(settings_section_header_actions(
                SettingsPanelSection::System,
                &state.settings,
                &editor_draft,
                cx,
            ))
        })
        .when(
            matches!(
                active_section,
                SettingsPanelSection::Data
                    | SettingsPanelSection::ConnectionSecurity
                    | SettingsPanelSection::DatabaseSupport
                    | SettingsPanelSection::About
            ),
            |this| this.child(settings_section_header_actions_disabled(colors)),
        )
}

fn settings_panel_body(
    _state: &AppState,
    theme_mode: ThemeMode,
    active_section: SettingsPanelSection,
    colors: UiColors,
    editor_draft: Settings,
    font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    radius_input: Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    match active_section {
        SettingsPanelSection::Editor => {
            settings_editor_panel(
                editor_draft,
                font_size_slider,
                colors,
                line_height_input,
                window,
                cx,
            )
        }
        SettingsPanelSection::Shortcuts => {
            settings_shortcuts_panel(&editor_draft, colors, window, cx)
        }
        SettingsPanelSection::Appearance => {
            settings_appearance_panel(
                &editor_draft,
                theme_mode,
                colors,
                radius_input,
                window,
                cx,
            )
        }
        SettingsPanelSection::System => {
            settings_system_panel(&editor_draft, colors, cx)
        }
        SettingsPanelSection::Data => settings_data_panel(&editor_draft, colors, cx),
        SettingsPanelSection::ConnectionSecurity => settings_connection_security_panel(colors),
        SettingsPanelSection::DatabaseSupport => {
            div().child(settings_database_support_panel(colors))
        }
        SettingsPanelSection::About => settings_about_panel(colors),
    }
}

fn settings_editor_panel(
    settings: Settings,
    font_size_slider: Entity<SliderState>,
    colors: UiColors,
    line_height_input: Entity<InputState>,
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
                    "DROP、TRUNCATE、无 WHERE 的 UPDATE/DELETE 会先确认",
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
                )),
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
) -> Div {
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

    div()
        .min_h(px(52.))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
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

fn settings_appearance_panel(
    settings: &Settings,
    theme_mode: ThemeMode,
    colors: UiColors,
    radius_input: Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let accent: gpui::Rgba = ComponentTheme::global(cx).primary.into();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group("主题", colors)
                .child(settings_theme_mode_row(theme_mode, colors, cx))
                .child(settings_theme_palette(settings, theme_mode, colors, cx)),
        )
        .child(
            settings_panel_group("形状与层次", colors)
                .child(settings_radius_row(settings, radius_input, colors, window, cx))
                .child(
                    settings_checkbox_row(
                        "显示阴影",
                        "控制按钮、输入框、弹层和通知等组件的阴影",
                        AppIcon::Square,
                        "settings-show-shadows",
                        settings.show_shadows,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.settings_editor_draft.show_shadows =
                                !this.settings_editor_draft.show_shadows;
                            this.preview_settings(cx);
                        }),
                    ),
                ),
        )
        .child(
            settings_panel_group("布局", colors)
                .child(settings_choice_row(
                    "界面密度",
                    "调整组件文字和相对间距的紧凑程度",
                    AppIcon::PanelBottom,
                    density_value(settings.ui_density),
                    &[("Compact", 0), ("Standard", 1), ("Comfortable", 2)],
                    |settings, value| {
                        settings.ui_density = match value {
                            0 => UiDensity::Compact,
                            2 => UiDensity::Comfortable,
                            _ => UiDensity::Standard,
                        }
                    },
                    colors,
                    true,
                    cx,
                ))
                .child(
                    settings_checkbox_row(
                        "显示状态栏",
                        "控制窗口底部状态栏是否显示",
                        AppIcon::PanelBottom,
                        "settings-show-status-bar",
                        settings.show_status_bar,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.settings_editor_draft.show_status_bar =
                                !this.settings_editor_draft.show_status_bar;
                            cx.notify();
                        }),
                    ),
                )
                .child(settings_scrollbar_mode_row(settings.scrollbar_mode, colors, cx)),
        )
        .child(
            settings_panel_group("无障碍", colors)
                .child(
                    settings_checkbox_row(
                        "显示焦点环",
                        "为键盘操作中的控件显示清晰的焦点提示",
                        AppIcon::CircleSlash,
                        "settings-focus-ring",
                        settings.focus_ring,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.settings_editor_draft.focus_ring =
                                !this.settings_editor_draft.focus_ring;
                            this.preview_settings(cx);
                        }),
                    ),
                )
                .child(
                    settings_checkbox_row(
                        "减少动效",
                        "关闭组件过渡动画，减少视觉刺激",
                        AppIcon::Minus,
                        "settings-reduce-motion",
                        settings.reduce_motion,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.settings_editor_draft.reduce_motion =
                                !this.settings_editor_draft.reduce_motion;
                            this.preview_settings(cx);
                        }),
                    ),
                ),
        )
        .child(
            settings_panel_group("全局字体", colors)
                .child(settings_font_family_row(
                    &settings.global_font_family,
                    colors,
                    cx,
                )),
        )
        .child(settings_appearance_preview(colors, accent))
}

fn settings_appearance_header_actions(
    saved: &Settings,
    draft: &Settings,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let dirty = settings_section_changed(
        SettingsPanelSection::Appearance,
        saved,
        draft,
    );
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new("settings-appearance-reset")
                .label("恢复默认设置")
                .small()
                .rounded(ComponentTheme::global(cx).radius)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reset_appearance_settings(cx);
                })),
        )
        .child(
            Button::new("settings-appearance-save")
                .label("保存")
                .small()
                .rounded(ComponentTheme::global(cx).radius)
                .primary()
                .disabled(!dirty)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_appearance_settings_saved(cx);
                })),
        )
}

fn settings_section_header_actions(
    section: SettingsPanelSection,
    saved: &Settings,
    draft: &Settings,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let dirty = settings_section_changed(section, saved, draft);
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new(SharedString::from(format!("settings-{section:?}-reset")))
                .label("恢复默认设置")
                .small()
                .rounded(ComponentTheme::global(cx).radius)
                .on_click(cx.listener(move |this, _, _, cx| {
                    reset_settings_section(this, section, cx);
                })),
        )
        .child(
            Button::new(SharedString::from(format!("settings-{section:?}-save")))
                .label("保存")
                .small()
                .rounded(ComponentTheme::global(cx).radius)
                .primary()
                .disabled(!dirty)
                .on_click(cx.listener(move |this, _, _, cx| {
                    save_settings_section_from_ui(this, section, cx);
                })),
        )
}

fn settings_section_header_actions_disabled(colors: UiColors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new("settings-disabled-reset")
                .label("恢复默认设置")
                .small()
                .rounded(colors.radius)
                .disabled(true),
        )
        .child(
            Button::new("settings-disabled-save")
                .label("保存")
                .small()
                .rounded(colors.radius)
                .primary()
                .disabled(true),
        )
}

fn settings_theme_mode_row(
    theme_mode: ThemeMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    settings_action_row(
        "主题模式",
        "控制当前应用使用浅色或深色外观",
        AppIcon::Settings,
        colors,
    )
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .child(
        div()
            .flex()
            .items_center()
            .gap_1()
            .p_1()
            .rounded(colors.radius)
            .bg(colors.panel_alt)
            .child(settings_theme_mode_button(
                "浅色",
                theme_mode == ThemeMode::Light,
                ThemeMode::Light,
                colors,
                cx,
            ))
            .child(settings_theme_mode_button(
                "深色",
                theme_mode == ThemeMode::Dark,
                ThemeMode::Dark,
                colors,
                cx,
            ))
            .child(
                Button::new("settings-theme-system")
                    .label("系统")
                    .small()
                    .rounded(colors.radius)
                    .disabled(true),
            ),
    )
}

fn settings_theme_palette(
    settings: &fluxdb_core::Settings,
    theme_mode: ThemeMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected = if theme_mode == ThemeMode::Dark {
        settings.dark_theme.as_str()
    } else {
        settings.light_theme.as_str()
    };
    let mut palette = div().flex().flex_wrap().gap_2();
    for config in theme_configs_for_mode(theme_mode, cx) {
        let is_selected = config.name == selected;
        let name = config.name.to_string();
        let background = config
            .colors
            .background
            .as_ref()
            .and_then(|value| parse_theme_hex_color(value).map(Hsla::from))
            .unwrap_or_else(|| colors.panel_bg.into());
        let foreground = config
            .colors
            .foreground
            .as_ref()
            .and_then(|value| parse_theme_hex_color(value).map(Hsla::from))
            .unwrap_or_else(|| colors.text.into());
        let accent = config
            .colors
            .primary
            .as_ref()
            .and_then(|value| parse_theme_hex_color(value).map(Hsla::from))
            .unwrap_or_else(|| colors.hover.into());
        palette = palette.child(
            Button::new(SharedString::from(format!("theme-palette-{}", config.name)))
                .w(px(132.))
                .h(px(52.))
                .p_2()
                .rounded(colors.radius)
                .compact()
                .border(px(if is_selected { 2. } else { 1. }))
                .border_color(if is_selected { colors.text } else { colors.border })
                .bg(background)
                .text_color(foreground)
                .hover(|style| style.opacity(0.88))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_theme_name(theme_mode, name.clone(), cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().size(px(10.)).rounded_full().bg(accent))
                        .child(div().text_xs().truncate().child(config.name.clone())),
                ),
        );
    }
    palette
}

fn settings_radius_row(
    settings: &Settings,
    radius_input: Entity<InputState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    settings_action_row(
        "圆角风格",
        "统一调整按钮、输入框、弹层和通知的圆角",
        AppIcon::Square,
        colors,
    )
    .child(
        div()
            .flex()
            .items_center()
            .gap_1()
                .children(
                [
                    ("Square", (0_u8, 0_u8)),
                    ("Standard", (6_u8, 8_u8)),
                    ("Rounded", (8_u8, 12_u8)),
                    ("Soft", (10_u8, 16_u8)),
                ]
                    .into_iter()
                    .map(|(label, (radius, large_radius))| {
                        Button::new(("settings-radius", settings_status_id("圆角风格", label)))
                            .label(label)
                            .small()
                            .rounded(colors.radius)
                            .when(
                                settings.button_radius == radius
                                    && settings.large_radius == large_radius,
                                |button| button.primary(),
                            )
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                this.settings_editor_draft.button_radius = radius;
                                this.settings_editor_draft.large_radius = large_radius;
                                this.preview_settings(cx);
                                cx.stop_propagation();
                            }))
                    }),
            )
            .child(settings_radius_input(settings.button_radius, &radius_input, colors, window, cx)),
    )
}

fn settings_radius_input(
    current: u8,
    input: &Entity<InputState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let expected = current.min(24).to_string();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !focused && input.read(cx).value().to_string() != expected {
        input.update(cx, |input, cx| input.set_value(expected.clone(), window, cx));
    }
    let focus_border = if colors.is_dark { rgb(0x8ab4ff) } else { rgb(0x111111) };
    div()
        .w(px(86.))
        .h(px(34.))
        .rounded(colors.radius)
        .border_1()
        .border_color(if focused { focus_border } else { colors.border })
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .overflow_hidden()
        .cursor_text()
        .child(
            Input::new(input)
                .appearance(false)
                .focus_bordered(false)
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .px_2()
                .text_size(px(13.))
                .text_color(colors.text),
        )
        .child(div().mr_2().text_size(px(11.)).text_color(colors.muted).child("px"))
}

fn density_value(density: UiDensity) -> u64 {
    match density {
        UiDensity::Compact => 0,
        UiDensity::Standard => 1,
        UiDensity::Comfortable => 2,
    }
}

fn settings_scrollbar_mode_row(
    current: ScrollbarMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    settings_action_row(
        "滚动条显示",
        "控制可滚动区域显示滚动条的时机",
        AppIcon::List,
        colors,
    )
    .child(
        div().flex().items_center().gap_1().children(
            [
                ("Auto", ScrollbarMode::Scrolling),
                ("Hover", ScrollbarMode::Hover),
                ("Always", ScrollbarMode::Always),
            ]
            .into_iter()
            .map(|(label, mode)| {
                Button::new(("settings-scrollbar", settings_status_id("滚动条显示", label)))
                    .label(label)
                    .small()
                    .rounded(colors.radius)
                    .when(current == mode, |button| button.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_editor_draft.scrollbar_mode = mode;
                        this.preview_settings(cx);
                    }))
            }),
        ),
    )
}

fn settings_font_family_row(
    current: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    settings_action_row(
        "全局字体",
        "应用界面文字字体，不影响 SQL 编辑器等宽字体",
        AppIcon::Text,
        colors,
    )
    .child(
        div().flex().items_center().gap_1().children(
            [("System", ""), ("Inter", "Inter"), ("SF Pro", "SF Pro")]
                .into_iter()
                .map(|(label, family)| {
                    Button::new(("settings-font", settings_status_id("全局字体", label)))
                        .label(label)
                        .small()
                        .rounded(colors.radius)
                        .when(current == family, |button| button.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings_editor_draft.global_font_family = family.to_string();
                            this.preview_settings(cx);
                        }))
                }),
        ),
    )
}

fn settings_theme_mode_button(
    label: &'static str,
    active: bool,
    mode: ThemeMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Button {
    Button::new(SharedString::from(format!("settings-theme-mode-{label}")))
        .label(label)
        .small()
        .rounded(colors.radius)
        .when(active, |button| button.primary())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_theme_mode(mode, cx);
            this.show_message(
                if mode == ThemeMode::Dark {
                    "已预览深色主题，点击保存后保留"
                } else {
                    "已预览浅色主题，点击保存后保留"
                },
                AppMessageKind::Success,
                cx,
            );
        }))
}

fn settings_appearance_preview(colors: UiColors, accent: gpui::Rgba) -> GroupBox {
    settings_panel_group("预览", colors).child(
        div()
            .border_t_1()
            .border_color(colors.border_soft)
            .p_3()
            .flex()
            .gap_3()
            .child(
                div()
                    .w(px(150.))
                    .h(px(176.))
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border_soft)
                    .bg(colors.panel_alt)
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.muted)
                            .child("连接"),
                    )
                    .child(settings_preview_tree_item("Local MySQL", true, accent, colors))
                    .child(settings_preview_tree_item("orders", false, accent, colors))
                    .child(settings_preview_tree_item("users", false, accent, colors)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(176.))
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.border_soft)
                    .bg(colors.panel_bg)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(div().h(px(4.)).w_full().bg(accent))
                    .child(
                        div()
                            .h(px(38.))
                            .px_3()
                            .border_b_1()
                            .border_color(colors.border_soft)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(colors.text)
                                    .child("orders"),
                            )
                            .child(
                                div()
                                    .h(px(24.))
                                    .px_2()
                                    .rounded(colors.radius)
                                    .bg(accent)
                                    .flex()
                                    .items_center()
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xffffff))
                                    .child("筛选"),
                            ),
                    )
                    .child(settings_preview_table(colors)),
            ),
    )
}

fn settings_preview_tree_item(
    label: &'static str,
    active: bool,
    accent: gpui::Rgba,
    colors: UiColors,
) -> Div {
    div()
        .h(px(28.))
        .rounded(colors.radius)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .bg(if active {
            colors.tree_selected
        } else {
            colors.panel_alt
        })
        .child(
            div()
                .size(px(7.))
                .rounded_full()
                .bg(if active { accent } else { colors.muted }),
        )
        .child(
            div()
                .text_size(px(12.))
                .font_weight(if active {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if active { colors.text } else { colors.muted })
                .child(label),
        )
}

fn settings_preview_table(colors: UiColors) -> Div {
    let rows = [
        ("1001", "Fusuwei", "paid", "$120.00"),
        ("1002", "Cloud", "pending", "$88.40"),
        ("1003", "Studio", "paid", "$310.20"),
    ];
    let mut table = div().flex().flex_col().p_3();
    table = table.child(settings_preview_table_row(
        ("id", "customer", "status", "total"),
        true,
        colors,
    ));
    for row in rows {
        table = table.child(settings_preview_table_row(row, false, colors));
    }
    table
}

fn settings_preview_table_row(
    row: (&'static str, &'static str, &'static str, &'static str),
    header: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(if header { 30. } else { 28. }))
        .border_1()
        .border_color(colors.border_soft)
        .bg(if header { colors.panel_alt } else { colors.panel_bg })
        .flex()
        .items_center()
        .text_size(px(11.))
        .text_color(if header { colors.muted } else { colors.text })
        .child(settings_preview_table_cell(row.0, header, colors))
        .child(settings_preview_table_cell(row.1, header, colors))
        .child(settings_preview_table_cell(row.2, header, colors))
        .child(settings_preview_table_cell(row.3, header, colors))
}

fn settings_preview_table_cell(text: &'static str, header: bool, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .px_2()
        .font_family(if header { "JetBrains Mono" } else { "Inter" })
        .font_weight(if header {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(if header { colors.muted } else { colors.text })
        .child(text)
}

fn settings_data_panel(
    settings: &Settings,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group("数据表", colors)
                .child(settings_preference_row(
                    "默认分页行数",
                    "后续用于控制新打开数据表的默认加载行数",
                    AppIcon::List,
                    "待接入",
                    colors,
                ))
                .child(settings_preference_row(
                    "单元格内容显示",
                    "后续支持在省略显示和自动换行之间选择",
                    AppIcon::Table,
                    "待接入",
                    colors,
                ))
                .child(settings_preference_row(
                    "查询结果默认布局",
                    "后续用于设置查询结果默认显示在底部或右侧",
                    AppIcon::PanelBottom,
                    "待接入",
                    colors,
                )),
        )
        .child(
            settings_panel_group("备份", colors)
                .child(settings_path_row(
                    "备份目录",
                    "数据库备份文件默认保存目录，保存后生效",
                    AppIcon::Folder,
                    &settings.backup_dir,
                    "settings-backup-dir",
                    true,
                    colors,
                    cx,
                ))
                .child(settings_path_row(
                    "mysqldump 路径",
                    "MySQL/TiDB 原生备份工具路径，留空时使用系统 PATH 中的 mysqldump",
                    AppIcon::Query,
                    &settings.mysqldump_path,
                    "settings-backup-mysqldump",
                    false,
                    colors,
                    cx,
                ))
                .child(settings_path_row(
                    "sqlite3 路径",
                    "SQLite 原生备份工具路径，留空时使用系统 PATH 中的 sqlite3",
                    AppIcon::Database,
                    &settings.sqlite3_path,
                    "settings-backup-sqlite3",
                    false,
                    colors,
                    cx,
                )),
        )
}

/// 设置面板中一条可选择的路径行（目录或文件）。选择结果写入 settings_editor_draft 对应字段。
fn settings_path_row(
    title: &'static str,
    detail: &'static str,
    icon: AppIcon,
    value: &str,
    button_id: &'static str,
    directory: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    settings_action_row(title, detail, icon, colors)
        .child(
            h_flex()
                .min_w(px(0.))
                .max_w(px(380.))
                .gap_2()
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .truncate()
                        .child(if value.trim().is_empty() {
                            "（未设置）".to_string()
                        } else {
                            value.to_string()
                        }),
                )
                .child(
                    Button::new(button_id)
                        .label("选择")
                        .small()
                        .rounded(colors.radius)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            settings_choose_backup_path(this, button_id, directory, window, cx);
                        })),
                ),
        )
}

fn settings_choose_backup_path(
    this: &mut NavicatMain,
    button_id: &'static str,
    directory: bool,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: !directory,
        directories: directory,
        multiple: false,
        prompt: Some("选择路径".into()),
    });
    this._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
        let result = receiver.await;
        let _ = cx.update(|_window, cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            view.update(cx, |this, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        let value = path.display().to_string();
                        match button_id {
                            "settings-backup-dir" => this.settings_editor_draft.backup_dir = value,
                            "settings-backup-mysqldump" => {
                                this.settings_editor_draft.mysqldump_path = value
                            }
                            "settings-backup-sqlite3" => {
                                this.settings_editor_draft.sqlite3_path = value
                            }
                            _ => {}
                        }
                        cx.notify();
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => this.show_message(
                    format!("选择路径失败：{error}"),
                    AppMessageKind::Error,
                    cx,
                ),
                Err(error) => this.show_message(
                    format!("选择路径失败：{error}"),
                    AppMessageKind::Error,
                    cx,
                ),
            });
        });
    }));
}

fn settings_connection_security_panel(colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group("连接", colors)
                .child(settings_preference_row(
                    "连接超时",
                    "后续用于配置连接测试和数据库访问的默认超时时间",
                    AppIcon::Plug,
                    "待接入",
                    colors,
                )),
        )
        .child(
            settings_panel_group("安全", colors).child(settings_preference_row(
                "敏感信息保存",
                "连接配置继续只保存 credential_ref 或非敏感选项",
                AppIcon::Check,
                "已启用",
                colors,
            )),
        )
}

fn settings_database_support_panel(colors: UiColors) -> GroupBox {
    settings_panel_group("内置连接能力", colors).children(
        database_support_entries()
            .into_iter()
            .map(|entry| settings_database_support_row(entry, colors)),
    )
}

fn settings_about_panel(colors: UiColors) -> Div {
    div().flex().flex_col().gap_3().child(
        settings_panel_group("关于", colors)
            .child(settings_preference_row(
                "FluxDB Desktop",
                "本地数据库管理工具，连接能力以内置 Rust connector 为主",
                AppIcon::Database,
                "开发中",
                colors,
            ))
            .child(settings_preference_row(
                "配置模型",
                "后续新增偏好项时会进入 fluxdb-core / fluxdb-storage 的持久化模型",
                AppIcon::Workflow,
                "待接入",
                colors,
            )),
    )
    .child(
        settings_panel_group("更新", colors)
            .child(settings_preference_row(
                "自动检查更新",
                "启动应用时检查是否有新的 FluxDB 版本",
                AppIcon::Refresh,
                "待接入",
                colors,
            ))
            .child(settings_preference_row(
                "检查更新",
                "手动检查当前应用是否有可用更新",
                AppIcon::Refresh,
                "待接入",
                colors,
            )),
    )
}

fn settings_system_panel(
    settings: &Settings,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group("启动与窗口", colors)
                .child(settings_preference_row(
                    "恢复窗口大小和位置",
                    "启动时恢复上次关闭应用时的窗口状态",
                    AppIcon::PanelBottom,
                    "待接入",
                    colors,
                ))
                .child(settings_preference_row(
                    "启动时最小化",
                    "应用启动后保持最小化状态",
                    AppIcon::Minus,
                    "待接入",
                    colors,
                ))
                .child(settings_preference_row(
                    "关闭未保存内容时确认",
                    "关闭包含未保存编辑内容的标签页前显示确认",
                    AppIcon::CircleSlash,
                    "待接入",
                    colors,
                )),
        )
        .child(
            settings_panel_group("语言", colors).child(settings_preference_row(
                "界面语言",
                "国际化支持完成后可切换应用显示语言",
                AppIcon::Workflow,
                "暂不可用",
                colors,
            )),
        )
        .child(
            settings_panel_group("隐私与诊断", colors)
                .child(settings_performance_diagnostics_row(
                    settings.performance_diagnostics,
                    colors,
                    cx,
                ))
                .child(settings_log_level_row(settings.log_level, colors, cx))
                .child(settings_log_path_row(&settings.log_path, colors, cx)),
        )
        .child(
            settings_panel_group("本地数据", colors)
                .child(settings_preference_row(
                    "查看数据目录",
                    "打开应用配置、历史记录和缓存所在的本地目录",
                    AppIcon::Folder,
                    "待接入",
                    colors,
                ))
                .child(settings_preference_row(
                    "清理查询历史",
                    "删除本地保存的 SQL 查询历史记录",
                    AppIcon::Trash,
                    "待接入",
                    colors,
                ))
                .child(settings_preference_row(
                    "清理 Redis 搜索历史",
                    "删除本地保存的 Redis Key 搜索记录",
                    AppIcon::Trash,
                    "待接入",
                    colors,
                ))
                .child(settings_preference_row(
                    "清理 SQL 补全缓存",
                    "删除本地缓存的数据库对象补全索引",
                    AppIcon::Trash,
                    "待接入",
                    colors,
                ))
                .child(settings_preference_row(
                    "导出诊断数据",
                    "导出排查问题所需的日志和运行信息，不包含密码",
                    AppIcon::Refresh,
                    "待接入",
                    colors,
                )),
        )
        .child(
            settings_panel_group("高级", colors).child(settings_preference_row(
                "全局代理",
                "为需要网络访问的应用功能配置 HTTP 或 SOCKS 代理",
                AppIcon::Plug,
                "待接入",
                colors,
            )),
        )
}

fn settings_panel_group(title: &'static str, _colors: UiColors) -> GroupBox {
    GroupBox::new()
        .with_variant(GroupBoxVariant::Outline)
        .title(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
}

fn settings_section_label(section: SettingsPanelSection) -> &'static str {
    match section {
        SettingsPanelSection::Editor => "编辑器",
        SettingsPanelSection::Shortcuts => "快捷键",
        SettingsPanelSection::Appearance => "外观",
        SettingsPanelSection::System => "系统",
        SettingsPanelSection::Data => "数据",
        SettingsPanelSection::ConnectionSecurity => "连接与安全",
        SettingsPanelSection::DatabaseSupport => "数据库支持",
        SettingsPanelSection::About => "关于",
    }
}

fn settings_section_description(section: SettingsPanelSection) -> &'static str {
    match section {
        SettingsPanelSection::Editor => "SQL 编辑器和执行相关偏好",
        SettingsPanelSection::Shortcuts => "查看应用和编辑器快捷键",
        SettingsPanelSection::Appearance => "主题和应用外观偏好",
        SettingsPanelSection::System => "启动、诊断、本地数据和高级选项",
        SettingsPanelSection::Data => "数据表和查询结果显示偏好",
        SettingsPanelSection::ConnectionSecurity => "连接行为和敏感信息策略",
        SettingsPanelSection::DatabaseSupport => "当前内置连接器和数据库能力状态",
        SettingsPanelSection::About => "应用信息和配置说明",
    }
}

#[derive(Clone, Copy)]
struct DatabaseSupportEntry {
    kind: DatabaseKind,
    status: DatabaseSupportStatus,
    source: &'static str,
    detail: &'static str,
}

#[derive(Clone, Copy)]
enum DatabaseSupportStatus {
    BuiltIn,
    Mock,
}

fn database_support_entries() -> [DatabaseSupportEntry; 5] {
    [
        DatabaseSupportEntry {
            kind: DatabaseKind::MySql,
            status: DatabaseSupportStatus::BuiltIn,
            source: "内置 SQLx MySQL 连接器",
            detail: "支持连接测试、对象浏览、数据查看、SQL 执行、表结构和用户管理",
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::TiDb,
            status: DatabaseSupportStatus::BuiltIn,
            source: "复用 MySQL 协议连接器",
            detail: "支持连接测试、对象浏览、数据查看、SQL 执行和表结构能力",
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::Sqlite,
            status: DatabaseSupportStatus::BuiltIn,
            source: "内置 SQLx SQLite 连接器",
            detail: "支持本地文件连接、对象浏览、数据查看、SQL 执行和表结构能力",
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::MongoDb,
            status: DatabaseSupportStatus::Mock,
            source: "配置入口已就绪",
            detail: "当前仍使用模拟连接器，真实对象浏览和数据操作能力待补齐",
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::Redis,
            status: DatabaseSupportStatus::Mock,
            source: "配置入口已就绪",
            detail: "当前仍使用模拟连接器，真实键浏览和数据操作能力待补齐",
        },
    ]
}

fn settings_database_support_row(entry: DatabaseSupportEntry, colors: UiColors) -> Div {
    div()
        .min_h(px(58.))
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
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(
                    div()
                        .size(px(30.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border_soft)
                        .bg(colors.panel_alt)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(img(database_kind_icon_path(entry.kind)).size(px(20.))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(colors.text)
                                        .child(database_kind_name(entry.kind)),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(colors.muted)
                                        .child(entry.source),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(entry.detail),
                        ),
                ),
        )
        .child(settings_database_support_status(entry.status, colors))
}

fn settings_database_support_status(
    status: DatabaseSupportStatus,
    colors: UiColors,
) -> impl IntoElement {
    let label = match status {
        DatabaseSupportStatus::BuiltIn => "已支持",
        DatabaseSupportStatus::Mock => "待补齐",
    };

    settings_status_chip(label, label, colors)
}

fn settings_row_label(
    title: &'static str,
    detail: &'static str,
    icon: AppIcon,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_w(px(0.))
        .child(app_icon_box(icon, 24., 13., colors.muted).flex_none())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(detail),
                ),
        )
}

fn settings_action_row(
    title: &'static str,
    detail: &'static str,
    icon: AppIcon,
    colors: UiColors,
) -> Div {
    div()
        .h(px(52.))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(app_icon_box(icon, 24., 13., colors.muted).flex_none())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(detail),
                        ),
                ),
        )
}

/// 系统设置-性能诊断：右上角 FPS / 帧耗时 / CPU / GPU / 内存 悬浮 HUD 开关。
fn settings_performance_diagnostics_row(
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // Switch 的 on_click 不带 `Context<NavicatMain>`，只能拿到 `&mut App`；
    // 因此通过 self 的弱实体在 App 上下文中更新 draft，走与其它设置一致的保存流。
    let this = cx.entity().downgrade();
    settings_action_row(
        "性能诊断",
        "显示 FPS、帧耗时、CPU、GPU 和内存",
        AppIcon::Activity,
        colors,
    )
    .child(
        Switch::new("settings-performance-diagnostics")
            .checked(checked)
            .on_click(move |checked, _, cx| {
                // render 期间 self 必然存活，失败仅可能因视图临时不可达；忽略即可。
                let _ = this.update(cx, |this, cx| {
                    this.settings_editor_draft.performance_diagnostics = *checked;
                    cx.notify();
                });
            }),
    )
}

fn settings_log_level_row(
    current: LogLevel,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    settings_action_row(
        "日志级别",
        "控制应用记录的日志详细程度，保存后重启应用生效",
        AppIcon::List,
        colors,
    )
    .child(
        div()
            .flex()
            .items_center()
            .gap_1()
            .children(
                [
                    ("Error", LogLevel::Error),
                    ("Warn", LogLevel::Warn),
                    ("Info", LogLevel::Info),
                    ("Debug", LogLevel::Debug),
                    ("Trace", LogLevel::Trace),
                ]
                .into_iter()
                .map(|(label, level)| {
                    Button::new(("settings-log-level", settings_status_id("日志级别", label)))
                        .label(label)
                        .small()
                        .rounded(colors.radius)
                        .when(current == level, |button| button.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings_editor_draft.log_level = level;
                            cx.notify();
                        }))
                }),
            ),
    )
}

fn settings_log_path_row(
    log_path: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let display_path = configured_log_dir(log_path).display().to_string();
    settings_action_row(
        "日志路径",
        "查看或修改应用日志文件保存目录，保存后重启应用生效",
        AppIcon::Folder,
        colors,
    )
    .child(
        h_flex()
            .min_w(px(0.))
            .max_w(px(380.))
            .gap_2()
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .truncate()
                    .child(display_path),
            )
            .child(
                Button::new("settings-log-path")
                    .label("选择")
                    .small()
                    .rounded(colors.radius)
                    .on_click(cx.listener(|this, _, window, cx| {
                        settings_choose_log_directory(this, window, cx);
                    })),
            ),
    )
}

fn settings_choose_log_directory(
    this: &mut NavicatMain,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("选择日志目录".into()),
    });
    this._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
        let result = receiver.await;
        let _ = cx.update(|_window, cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            view.update(cx, |this, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.settings_editor_draft.log_path = path.display().to_string();
                        cx.notify();
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => this.show_message(
                    format!("选择日志目录失败：{error}"),
                    AppMessageKind::Error,
                    cx,
                ),
                Err(error) => this.show_message(
                    format!("选择日志目录失败：{error}"),
                    AppMessageKind::Error,
                    cx,
                ),
            });
        });
    }));
}

fn settings_choice_row(
    title: &'static str,
    detail: &'static str,
    icon: AppIcon,
    current: u64,
    choices: &'static [(&'static str, u64)],
    apply: fn(&mut Settings, u64),
    colors: UiColors,
    preview: bool,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .min_h(px(52.))
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
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(app_icon_box(icon, 24., 13., colors.muted).flex_none())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(detail),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(choices.iter().map(move |(label, value)| {
                    let active = current == *value;
                    Button::new(("settings-choice", settings_status_id(title, *label)))
                        .label(*label)
                        .small()
                        .rounded(colors.radius)
                        .when(active, |button| button.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            apply(&mut this.settings_editor_draft, *value);
                            if preview {
                                this.preview_settings(cx);
                            }
                            cx.notify();
                        }))
                })),
        )
}

fn settings_font_size_slider_row(
    title: &'static str,
    detail: &'static str,
    icon: AppIcon,
    current: u32,
    slider: Entity<SliderState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let current = current.clamp(10, 24);
    if (slider.read(cx).value().end() - current as f32).abs() > f32::EPSILON {
        slider.update(cx, |state, cx| state.set_value(current as f32, window, cx));
    }

    div()
        .min_h(px(68.))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(settings_row_label(title, detail, icon, colors))
        .child(
            div()
                .w(px(260.))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child("10px")
                        .child(format!("{}px", current))
                        .child("24px"),
                )
                .child(Slider::new(&slider).horizontal()),
        )
}

fn settings_input_row(
    title: &'static str,
    detail: &'static str,
    icon: AppIcon,
    current: u32,
    input: &Entity<InputState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let expected = current.clamp(13, 28).to_string();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !focused && input.read(cx).value().to_string() != expected {
        input.update(cx, |input, cx| input.set_value(expected.clone(), window, cx));
    }
    let focus_border = if colors.is_dark {
        rgb(0x8ab4ff)
    } else {
        rgb(0x111111)
    };

    div()
        .min_h(px(52.))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(settings_row_label(title, detail, icon, colors))
        .child(
            div()
                .w(px(86.))
                .h(px(34.))
                .rounded(colors.radius)
                .border_1()
                .border_color(if focused { focus_border } else { colors.border })
                .bg(colors.input_bg)
                .flex()
                .items_center()
                .overflow_hidden()
                .cursor_text()
                .child(
                    Input::new(input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .px_2()
                        .text_size(px(13.))
                        .text_color(colors.text),
                ),
        )
}

fn settings_checkbox_row(
    title: &'static str,
    detail: &'static str,
    icon: AppIcon,
    id: &'static str,
    checked: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(52.))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(settings_row_label(title, detail, icon, colors))
        .child(Checkbox::new(id).checked(checked))
}

fn settings_editor_changed(saved: &Settings, draft: &Settings) -> bool {
    saved.page_size != draft.page_size
        || saved.confirm_dangerous_sql != draft.confirm_dangerous_sql
        || saved.confirm_dangerous_redis != draft.confirm_dangerous_redis
        || saved.editor_font_size != draft.editor_font_size
        || saved.editor_line_height != draft.editor_line_height
        || saved.editor_tab_width != draft.editor_tab_width
        || saved.editor_word_wrap != draft.editor_word_wrap
}

fn settings_changed(saved: &Settings, draft: &Settings) -> bool {
    saved != draft
}

fn settings_section_changed(
    section: SettingsPanelSection,
    saved: &Settings,
    draft: &Settings,
) -> bool {
    match section {
        SettingsPanelSection::Editor => settings_editor_changed(saved, draft),
        SettingsPanelSection::Shortcuts => saved.custom_keybindings != draft.custom_keybindings,
        SettingsPanelSection::Appearance => {
            saved.theme != draft.theme
                || saved.light_theme != draft.light_theme
                || saved.dark_theme != draft.dark_theme
                || saved.button_radius != draft.button_radius
                || saved.large_radius != draft.large_radius
                || saved.show_shadows != draft.show_shadows
                || saved.focus_ring != draft.focus_ring
                || saved.scrollbar_mode != draft.scrollbar_mode
                || saved.ui_density != draft.ui_density
                || saved.show_status_bar != draft.show_status_bar
                || saved.reduce_motion != draft.reduce_motion
                || saved.global_font_family != draft.global_font_family
        }
        SettingsPanelSection::System => {
            saved.log_level != draft.log_level
                || saved.log_path != draft.log_path
                || saved.performance_diagnostics != draft.performance_diagnostics
        }
        SettingsPanelSection::Data => {
            saved.backup_dir != draft.backup_dir
                || saved.mysqldump_path != draft.mysqldump_path
                || saved.sqlite3_path != draft.sqlite3_path
        }
        SettingsPanelSection::ConnectionSecurity
        | SettingsPanelSection::DatabaseSupport
        | SettingsPanelSection::About => false,
    }
}

fn settings_apply_appearance_fields(settings: &mut Settings, draft: &Settings) {
    settings.theme = draft.theme;
    settings.light_theme = draft.light_theme.clone();
    settings.dark_theme = draft.dark_theme.clone();
    settings.button_radius = draft.button_radius;
    settings.large_radius = draft.large_radius;
    settings.show_shadows = draft.show_shadows;
    settings.focus_ring = draft.focus_ring;
    settings.scrollbar_mode = draft.scrollbar_mode;
    settings.ui_density = draft.ui_density;
    settings.show_status_bar = draft.show_status_bar;
    settings.reduce_motion = draft.reduce_motion;
    settings.global_font_family = draft.global_font_family.clone();
}

fn settings_apply_system_fields(settings: &mut Settings, draft: &Settings) {
    settings.log_level = draft.log_level;
    settings.log_path = draft.log_path.clone();
    settings.performance_diagnostics = draft.performance_diagnostics;
}

fn settings_apply_section_fields(
    settings: &mut Settings,
    draft: &Settings,
    section: SettingsPanelSection,
) {
    match section {
        SettingsPanelSection::Editor => settings_apply_editor_fields(settings, draft),
        SettingsPanelSection::Shortcuts => {
            settings.custom_keybindings = draft.custom_keybindings.clone();
        }
        SettingsPanelSection::Appearance => settings_apply_appearance_fields(settings, draft),
        SettingsPanelSection::System => settings_apply_system_fields(settings, draft),
        SettingsPanelSection::Data => settings_apply_backup_fields(settings, draft),
        SettingsPanelSection::ConnectionSecurity
        | SettingsPanelSection::DatabaseSupport
        | SettingsPanelSection::About => {}
    }
}

fn settings_apply_backup_fields(settings: &mut Settings, draft: &Settings) {
    settings.backup_dir = draft.backup_dir.clone();
    settings.mysqldump_path = draft.mysqldump_path.clone();
    settings.sqlite3_path = draft.sqlite3_path.clone();
}

fn reset_settings_section(
    this: &mut NavicatMain,
    section: SettingsPanelSection,
    cx: &mut Context<NavicatMain>,
) {
    match section {
        SettingsPanelSection::Shortcuts => {
            this.settings_editor_draft.custom_keybindings.clear();
            this.show_message("已恢复默认快捷键，点击保存后生效", AppMessageKind::Info, cx);
            cx.notify();
        }
        SettingsPanelSection::System => {
            let defaults = Settings::default();
            this.settings_editor_draft.log_level = defaults.log_level;
            this.settings_editor_draft.log_path = defaults.log_path;
            this.settings_editor_draft.performance_diagnostics =
                defaults.performance_diagnostics;
            this.show_message("已恢复默认系统设置，点击保存后生效", AppMessageKind::Info, cx);
            cx.notify();
        }
        SettingsPanelSection::Appearance => this.reset_appearance_settings(cx),
        _ => {}
    }
}

fn save_settings_section_from_ui(
    this: &mut NavicatMain,
    section: SettingsPanelSection,
    cx: &mut Context<NavicatMain>,
) {
    let saved = this.controller.state().settings.clone();
    let draft = this.settings_editor_draft.clone();
    let mut settings = saved.clone();
    settings_apply_section_fields(&mut settings, &draft, section);

    if section == SettingsPanelSection::Shortcuts {
        for definition in SHORTCUT_DEFINITIONS {
            let old = current_shortcut(&saved, definition);
            let new = current_shortcut(&draft, definition);
            if old != new {
                shadow_shortcut(cx, &old, definition.context);
                bind_shortcut(cx, &new, definition.action, definition.context);
            }
        }
    }

    let _ = this.controller.dispatch(AppCommand::SaveSettings(settings));
    let _ = this.storage.save_settings(&this.controller.state().settings);
    this.sync_settings_tab_dirty();
    this.show_message(
        match section {
            SettingsPanelSection::Shortcuts => "快捷键已保存",
            SettingsPanelSection::System => "系统设置已保存，重启应用后生效",
            _ => "设置已保存",
        },
        AppMessageKind::Success,
        cx,
    );
    cx.notify();
}

fn settings_apply_editor_fields(settings: &mut Settings, draft: &Settings) {
    settings.page_size = draft.page_size;
    settings.confirm_dangerous_sql = draft.confirm_dangerous_sql;
    settings.confirm_dangerous_redis = draft.confirm_dangerous_redis;
    settings.editor_font_size = draft.editor_font_size;
    settings.editor_line_height = draft.editor_line_height;
    settings.editor_tab_width = draft.editor_tab_width;
    settings.editor_word_wrap = draft.editor_word_wrap;
}

fn settings_default_editor_draft(saved: &Settings) -> Settings {
    let defaults = Settings::default();
    let mut draft = saved.clone();
    settings_apply_editor_fields(&mut draft, &defaults);
    draft
}

fn save_settings_from_ui(
    this: &mut NavicatMain,
    settings: Settings,
    message: &'static str,
    cx: &mut Context<NavicatMain>,
) {
    let _ = this.controller.dispatch(AppCommand::SaveSettings(settings));
    let _ = this.storage.save_settings(&this.controller.state().settings);
    // 热更新：把新设置推送到所有已打开的编辑器，无需重启。
    let new_settings = this.controller.state().settings.clone();
    let font_size = new_settings.editor_font_size.clamp(10, 24) as f32;
    let line_height = new_settings.editor_line_height.clamp(13, 28) as f32;
    let soft_wrap = new_settings.editor_word_wrap;
    for sql_editor in this.query_editors.values() {
        sql_editor.update(cx, |editor, _cx| {
            editor.apply_settings(font_size, line_height, soft_wrap, new_settings.editor_tab_width as usize);
        });
    }
    this.settings_editor_draft = this.controller.state().settings.clone();
    this.preview_settings(cx);
    this.sync_settings_tab_dirty();
    this.show_message(message, AppMessageKind::Success, cx);
}

fn settings_preference_row(
    title: &'static str,
    detail: &'static str,
    icon: AppIcon,
    status: &'static str,
    colors: UiColors,
) -> Div {
    div()
        .h(px(52.))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(app_icon_box(icon, 24., 13., colors.muted).flex_none())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(detail),
                        ),
                ),
        )
        .child(settings_status_chip(title, status, colors))
}

fn settings_status_chip(
    title: &'static str,
    label: &'static str,
    colors: UiColors,
) -> impl IntoElement {
    Button::new(("settings-status", settings_status_id(title, label)))
        .label(label)
        .small()
        .rounded(colors.radius)
        .disabled(true)
}

fn settings_status_id(title: &'static str, label: &'static str) -> u64 {
    match (title, label) {
        ("编辑器字号", "待接入") => 1,
        ("执行模式", "待接入") => 2,
        ("执行危险 SQL 前弹出确认", "待接入") => 3,
        ("跟随系统外观", "待接入") => 4,
        ("默认分页行数", "待接入") => 5,
        ("单元格内容显示", "待接入") => 6,
        ("查询结果默认布局", "待接入") => 7,
        ("连接超时", "待接入") => 8,
        ("敏感信息保存", "已启用") => 10,
        ("FluxDB Desktop", "开发中") => 11,
        ("配置模型", "待接入") => 12,
        ("已支持", "已支持") => 13,
        ("待补齐", "待补齐") => 14,
        _ => {
            let mut hasher = DefaultHasher::new();
            title.hash(&mut hasher);
            label.hash(&mut hasher);
            hasher.finish()
        }
    }
}

fn home_stat_card(label: &'static str, value: usize, hint: &'static str, colors: UiColors) -> Div {
    div()
        .h(px(92.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .p_4()
        .flex()
        .flex_col()
        .justify_between()
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .child(
                    div()
                        .text_size(px(30.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(value.to_string()),
                )
                .child(
                    div()
                        .pb_1()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(hint),
                ),
        )
}

fn recent_connections_panel(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div().flex().flex_col();
    for connection in state.connections.iter().take(5) {
        list = list.child(recent_connection_row(connection, colors, cx));
    }

    if state.connections.is_empty() {
        list = list.child(
            div()
                .h(px(96.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.))
                .text_color(colors.muted)
                .child("还没有连接，先新建一个数据库连接"),
        );
    }

    home_panel("快速开始", list, colors)
}

fn recent_connection_row(
    connection: &ConnectionState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let connection_id = connection.config.id;
    let color = connection_config_color(&connection.config.options);
    div()
        .h(px(68.))
        .border_b_1()
        .border_color(colors.border_soft)
        .px_4()
        .flex()
        .items_center()
        .justify_between()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::OpenConnection(connection_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(database_kind_badge(connection.config.kind, colors))
                .child(div().w(px(4.)).h(px(28.)).rounded_full().bg(color))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(connection.config.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(colors.muted)
                                .child(connection_summary(&connection.config)),
                        ),
                ),
        )
        .child(
            div()
                .text_size(px(18.))
                .text_color(if connection.connected {
                    rgb(0x16a34a)
                } else {
                    rgb(0xa0a7b2)
                })
                .child(if connection.connected { "●" } else { "○" }),
        )
}

fn common_actions_panel(
    state: &AppState,
    _window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_connection = !state.connections.is_empty();
    home_panel(
        "常用操作",
        div()
            .flex()
            .flex_col()
            .p_2()
            .gap_1()
            .child(home_action_row("新建连接", "＋", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.show_new_connection(window, cx);
                    cx.stop_propagation();
                }),
            ))
            .child(home_action_row("新建查询", "SQL", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if has_connection {
                        this.open_new_query(None, cx);
                    }
                    cx.stop_propagation();
                }),
            ))
            .child(home_action_row("刷新连接树", "⟳", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.refresh_connection_tree(cx);
                    cx.stop_propagation();
                }),
            ))
            .child(home_icon_action_row("设置", AppIcon::Settings, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dispatch(AppCommand::OpenSettings, cx);
                    cx.stop_propagation();
                }),
            ))
            .child(
                div()
                    .mt_2()
                    .rounded(colors.radius_lg)
                    .bg(colors.panel_alt)
                    .px_3()
                    .py_3()
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child("提示：也可以在左侧展开连接，双击表进入数据编辑器。"),
            ),
        colors,
    )
}

fn home_panel(title: &'static str, body: Div, colors: UiColors) -> Div {
    div()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .child(
            div()
                .h(px(44.))
                .border_b_1()
                .border_color(colors.border_soft)
                .px_4()
                .flex()
                .items_center()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(title),
        )
        .child(body)
}

fn home_icon_action_row(label: &'static str, icon: AppIcon, colors: UiColors) -> Div {
    div()
        .h(px(44.))
        .rounded(colors.radius)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(label),
        )
        .child(app_icon_box(icon, 28., 15., colors.muted))
}

fn home_action_row(label: &'static str, glyph: &'static str, colors: UiColors) -> Div {
    div()
        .h(px(44.))
        .rounded(colors.radius_lg)
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .text_size(px(14.))
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .w(px(30.))
                .h(px(24.))
                .rounded(colors.radius_lg)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(glyph),
        )
        .child(label)
}

fn database_kind_badge(kind: DatabaseKind, colors: UiColors) -> impl IntoElement {
    div()
        .size(px(30.))
        .rounded(colors.radius_lg)
        .bg(rgb(0x24272d))
        .flex()
        .items_center()
        .justify_center()
        .child(img(database_kind_icon_path(kind)).size(px(20.)))
}

fn object_list_content(state: &AppState, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    let mut content = div()
        .flex_1()
        .bg(colors.panel_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .child(object_toolbar(colors))
        .child(table_header(colors));

    for object in visible_objects(state) {
        content = content.child(object_row(object, colors, cx));
    }

    content
}

fn data_editor_content(
    tab_id: TabId,
    editor: &DataEditorState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let is_redis_editor = matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey);
    let show_table_tools = !is_redis_editor;
    let filter_open = show_table_tools && this.data_filter_panels.contains(&tab_id);
    let search_open = show_table_tools && this.data_search_panels.contains(&tab_id);
    let field_filter_open = show_table_tools && this.field_filter_popover == Some(tab_id);
    if filter_open {
        this.sync_data_filter_text_inputs(tab_id, window, cx);
    }
    if search_open {
        let expected = this
            .data_search_queries
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        let input_focused = this
            .data_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !input_focused && this.data_search_input.read(cx).value().to_string() != expected {
            this.data_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
        }
    }
    let filter_rules = this
        .data_filter_draft_rules
        .get(&tab_id)
        .cloned()
        .unwrap_or_default();
    let sort_rules = this
        .data_sort_draft_rules
        .get(&tab_id)
        .cloned()
        .unwrap_or_default();
    let filter_popover = this.data_filter_popover;
    let field_filter_search = this.field_filter_search.clone();
    let field_filter_search_input = this.field_filter_search_input.clone();
    let value_search = this.data_filter_value_search.clone();
    let value_search_loading = this.data_filter_value_search_loading_until.is_some();
    let data_filter_value_input = this.data_filter_value_input.clone();
    let local_filter_value_input = this.local_filter_value_input.clone();
    let local_filter_search_input = this.local_filter_search_input.clone();
    let local_filter_popover = this.local_filter_popover.clone();
    let local_filter_manager_open = this.local_filter_manager_popover == Some(tab_id);
    let local_filter_manager_field = this.local_filter_manager_field.clone();
    let local_filter_manager_draft_filters = this.local_filter_manager_draft_filters.clone();
    let local_filter_manager_field_open = this.local_filter_manager_field_open;
    let local_filter_manager_values_open = this.local_filter_manager_values_open;
    let local_filter_value = this.local_filter_value.clone();
    let local_filter_search = this.local_filter_search.clone();
    let local_filter_draft_values = this.local_filter_draft_values.clone();
    let data_filter_search_input = this.data_filter_search_input.clone();
    let data_filter_text_input = this.data_filter_text_input.clone();
    let data_sort_text_input = this.data_sort_text_input.clone();
    let data_sql_panel_input = this.data_sql_panel_input.clone();
    let data_page_input = this.data_page_input.clone();
    let data_search_input = this.data_search_input.clone();
    let data_sql_footer_selection = this.data_sql_footer_selection.clone();
    let filter_applying = this.data_filter_applying_tabs.contains(&tab_id);
    let filter_mode = this
        .data_filter_modes
        .get(&tab_id)
        .copied()
        .unwrap_or(DataFilterMode::Builder);
    let default_filter_panel_height =
        data_filter_panel_height(filter_rules.as_slice(), filter_mode);
    let filter_panel_height = this
        .data_filter_panel_heights
        .get(&tab_id)
        .copied()
        .map(clamp_data_filter_panel_height)
        .unwrap_or(default_filter_panel_height);
    let filter_text = this
        .data_filter_texts
        .get(&tab_id)
        .cloned()
        .unwrap_or_default();
    let sort_text = this
        .data_sort_texts
        .get(&tab_id)
        .cloned()
        .unwrap_or_default();
    let content = div()
        .relative()
        .flex_1()
        .bg(colors.panel_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .when(show_table_tools, |builder| {
            builder.child(data_editor_toolbar(
                tab_id,
                filter_open,
                search_open,
                field_filter_open,
                None,
                None,
                show_table_tools,
                colors,
                cx,
            ))
        })
        .when(filter_open, |builder| builder);

    if editor.loading {
        return content.child(center_loading_message("正在加载数据...", colors));
    }

    let Some(page) = &editor.page else {
        if let Some(error) = &editor.error {
            // 加载失败：常驻 Alert + 重试。重试复用工具栏「刷新」的既有入口，
            // 按当前筛选/排序在后台重新拉取，不会阻塞 UI；也不再是盖住整个表格区的居中文本。
            let retry = Button::new(("retry-data-page", tab_id.0))
                .label("重试")
                .small()
                .outline()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.request_data_editor_refresh(tab_id, cx);
                }))
                .into_any_element();
            return content.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .p_6()
                    .child(page_error_alert(
                        gpui::ElementId::Name(
                            format!("data-editor-error-{}", tab_id.0).into(),
                        ),
                        &error.title,
                        &error.message,
                        error.detail.as_deref(),
                        Some(retry),
                        cx,
                        colors,
                    )),
            );
        }
        return content.child(center_message("暂无数据", colors));
    };
    if is_redis_editor {
        this.sync_redis_search_controls(tab_id, window, cx);
    }
    // 先把 Redis 搜索/类型筛选后的展示页算出来，喂给下方列表与表格（与既有旧版一致）；
    // 非 Redis 编辑器则原样借用，不影响其它数据页。
    let display_page = this.data_page_for_display(tab_id, page);
    let page = display_page.as_ref();
    let sql = data_editor_sql_preview(
        &editor.object,
        filter_rules.as_slice(),
        sort_rules.as_slice(),
        filter_mode,
        filter_text.as_str(),
        sort_text.as_str(),
        page.offset,
        page.limit,
    );
    let all_field_names = page
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let visible_field_names = this.visible_fields_for_page(tab_id, page);
    let field_count = Some((visible_field_names.len(), all_field_names.len()));
    let table_state = this.data_table_state(tab_id, page, window, cx);
    let (search_matches, active_search_match, selected_source_row) = {
        let table = table_state.read(cx);
        let delegate = table.delegate();
        (
            delegate.search_matches.clone(),
            delegate.active_search_match,
            delegate
                .selected_row
                .or_else(|| delegate.selected_cell.map(|(row, _)| row))
                .and_then(|row| delegate.source_row_indexes.get(row).copied()),
        )
    };
    let search_highlight_all = this.data_search_highlight_all_tabs.contains(&tab_id);
    let change_count = editor
        .changes
        .as_ref()
        .filter(|changes| !changes.is_empty())
        .map(data_change_item_count);
    let change_sql_preview = editor
        .changes
        .as_ref()
        .filter(|changes| !changes.is_empty())
        .map(|changes| data_change_sql_preview(page, changes));
    let change_sql_preview_open = this.data_change_sql_preview_tabs.contains(&tab_id);
    // min_h(0)：flex 项默认最小尺寸为内容高度，缺这行时整列会被内容撑出窗口，
    // 下游（如 Redis Set 成员列表）拿到的永远是内容高度而非可用高度，滚动区因此永不溢出
    let content = div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .bg(colors.panel_bg)
        .border_r_1()
        .border_color(colors.border)
        .flex()
        .flex_col();
    let content = content.when(show_table_tools, |builder| {
        builder.child(data_editor_toolbar(
            tab_id,
            filter_open,
            search_open,
            field_filter_open,
            field_count,
            Some(editor.table_info.open),
            show_table_tools,
            colors,
            cx,
        ))
    });
    let content = content.when(filter_open, |builder| {
        builder.child(data_filter_panel(
            tab_id,
            sql.clone(),
            data_sql_panel_input,
            page,
            filter_rules.as_slice(),
            sort_rules.as_slice(),
            filter_mode,
            filter_panel_height,
            filter_applying,
            data_filter_text_input,
            data_sort_text_input,
            colors,
            window,
            cx,
        ))
    });
    let redis_search_active = is_redis_editor
        && (!this.redis_search_query(tab_id).is_empty()
            || this.redis_type_filter(tab_id) != REDIS_TYPE_FILTER_ALL);
    let redis_refreshing = is_redis_editor
        && (filter_applying
            || this._data_filter_apply_tasks.contains_key(&tab_id.0)
            || this._data_load_tasks.contains_key(&tab_id.0));
    let history_open = this.redis_key_search_history_open
        && tab_id == this.active_redis_data_tab_id().unwrap_or(tab_id);
    let content = content.when(is_redis_editor, |builder| {
        let search_bar = redis_key_search_bar(
            tab_id,
            this.redis_type_select.clone(),
            this.redis_search_input.clone(),
            redis_search_active,
            redis_refreshing,
            colors,
            cx,
        );
        // 全屏遮罩层捕捉外部点击关闭下拉；渲染在搜索栏前（下层），避免遮挡搜索栏与下拉菜单的交互。
        builder
            .when(history_open, |builder| {
                builder.child(redis_key_search_history_occlude(colors, cx))
            })
            .child(search_bar)
    });
    if is_redis_editor {
        let refreshed_at = this.redis_data_refresh_times.get(&tab_id).copied();
        let mut with_view = content.child(redis_split_view(
            tab_id,
            page,
            &table_state,
            refreshed_at,
            redis_refreshing,
            colors,
            this,
            window,
            cx,
        ));
        // 搜索历史下拉挂到 Redis 内容顶层、`redis_split_view` 之后：gpui 无 z-index，兄弟节点按
        // 绘制顺序叠放（后者在上），表格为不透明背景会把前置于它的子节点盖住，故下拉必须后置才能浮于表格之上。
        if history_open {
            with_view = with_view.child(redis_key_search_history_overlay(
                tab_id,
                this.redis_key_search_history_for(tab_id),
                colors,
                cx,
            ));
        }
        // 「新增 Key」抽屉作为全屏遮罩挂到 Redis 内容顶层（对齐 RedisInsight AddKey）；
        // 仅当前标签页有打开中的建 Key 抽屉时挂载。壳内三段式：公共字段 + 当前类型子表单。
        let add_key_open = this
            .redis_add_key_drawer
            .as_ref()
            .is_some_and(|drawer| drawer.tab_id == tab_id);
        let add_key_applying = this.redis_add_key_applying;
        return with_view.when(add_key_open, |builder| {
            // 按当前选中的 Key 类型渲染对应的子表单组件（类型切换由控制器订阅重置字段）。
            let kind = this.redis_add_key_kind(cx);
            let active_form = match kind {
                RedisAddKeyKind::String => {
                    redis_add_key_string_form(&this.redis_add_key_string_input, colors)
                }
                RedisAddKeyKind::Json => {
                    redis_add_key_json_form(
                        &this.redis_add_key_json_input,
                        add_key_applying,
                        colors,
                        cx,
                    )
                }
                RedisAddKeyKind::Hash => {
                    redis_add_key_hash_form(&this.redis_add_key_hash_rows, add_key_applying, colors, cx)
                }
                RedisAddKeyKind::ZSet => {
                    redis_add_key_zset_form(&this.redis_add_key_zset_rows, add_key_applying, colors, cx)
                }
                RedisAddKeyKind::Set => {
                    redis_add_key_set_form(&this.redis_add_key_set_rows, add_key_applying, colors, cx)
                }
                RedisAddKeyKind::List => redis_add_key_list_form(
                    &this.redis_add_key_list_rows,
                    this.redis_add_key_list_direction,
                    add_key_applying,
                    colors,
                    cx,
                ),
                RedisAddKeyKind::Stream => redis_add_key_stream_form(
                    &this.redis_add_key_stream_id_input,
                    &this.redis_add_key_stream_rows,
                    add_key_applying,
                    colors,
                    cx,
                ),
            };
            builder.child(redis_add_key_drawer(
                tab_id,
                &this.redis_add_key_type_select,
                &this.redis_add_key_name_input,
                &this.redis_add_key_ttl_input,
                active_form,
                this.redis_add_key_scroll.clone(),
                add_key_applying,
                colors,
                window,
                cx,
            ))
        });
    }

    let cell_detail_input = this.cell_detail_input.clone();
    let cell_detail_drawer_height = this
        .cell_detail_drawer_heights
        .get(&tab_id)
        .copied()
        .map(clamp_cell_detail_drawer_height)
        .unwrap_or(CELL_DETAIL_DRAWER_DEFAULT_HEIGHT);
    let cell_detail_drawer = editor.cell_detail_panel.open.then(|| {
        this.sync_cell_detail_input(
            &cell_detail_input,
            &editor.cell_detail_panel.edit_value,
            window,
            cx,
        );
        cell_detail_panel(
            tab_id,
            editor,
            page,
            cell_detail_drawer_height,
            cell_detail_input,
            this.temporal_part_input.clone(),
            this.temporal_part_editing,
            colors,
            window,
            cx,
        )
    });
    let drawer_height = cell_detail_drawer_height;
    let footer = (!is_redis_editor).then(|| {
        data_editor_footer(
            tab_id,
            page,
            sql,
            data_sql_footer_selection,
            data_page_input,
            change_count,
            change_sql_preview_open,
            selected_source_row,
            colors,
            window,
            cx,
        )
    });
    let table_area = data_table_area(
        &table_state,
        filter_applying,
        "正在应用筛选...",
        search_open.then(|| {
            data_search_bar(
                tab_id,
                data_search_input,
                search_matches.as_slice(),
                active_search_match,
                search_highlight_all,
                colors,
                cx,
            )
        }),
        change_sql_preview.as_ref().and_then(|preview| {
            change_sql_preview_open
                .then(|| data_change_sql_preview_drawer(tab_id, preview, colors, cx))
        }),
        cell_detail_drawer,
        drawer_height,
        footer,
        colors,
        cx,
    );
    let content = content.child(if editor.table_info.open {
        // DDL 页签用底层编辑器渲染，需要宿主当前主题派生的编辑器配色（见 table_info_ddl_text）。
        let editor_theme = this.editor_theme_for(cx);
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .overflow_hidden()
            .child(table_area)
            .child(table_info_panel(
                tab_id,
                editor,
                page,
                editor_theme,
                window,
                colors,
                cx,
            ))
    } else {
        table_area
    });
    let content = if field_filter_open {
        content.child(field_filter_popover_layer(
            tab_id,
            all_field_names.as_slice(),
            &visible_field_names,
            field_filter_search.as_str(),
            field_filter_search_input,
            colors,
            cx,
        ))
    } else {
        content
    };

    if filter_open {
        if let Some(popover) = filter_popover.filter(|popover| popover.tab_id == tab_id) {
            return content.child(data_filter_popover_layer(
                tab_id,
                page,
                filter_rules.as_slice(),
                sort_rules.as_slice(),
                popover,
                data_filter_value_input,
                data_filter_search_input,
                value_search.as_str(),
                value_search_loading,
                colors,
                cx,
            ));
        }
    }

    if let Some(popover) = local_filter_popover.filter(|popover| popover.tab_id == tab_id) {
        return content.child(local_filter_popover_layer(
            page,
            popover,
            local_filter_draft_values,
            local_filter_value_input,
            local_filter_search_input,
            local_filter_search.as_str(),
            colors,
            cx,
        ));
    }

    if local_filter_manager_open {
        return content.child(local_filter_manager_layer(
            tab_id,
            page,
            local_filter_manager_draft_filters,
            local_filter_manager_field_open,
            local_filter_manager_values_open,
            local_filter_manager_field,
            local_filter_search_input,
            local_filter_search.as_str(),
            local_filter_value.as_str(),
            colors,
            cx,
        ));
    }

    content
}

/// 新编辑器承载面板：键鼠/滚动/滚动条/动作分发已全部下沉到 Editor 自身
/// `Editor::render()` 根元素（参考 zed）。宿主这里只保留布局定位 + 配色，
/// 以及 SQL 专属的 `ExecuteQueryShortcut` 绑定（按祖先链从内层 Editor 冒泡到此处）。
fn sql_editor_panel(
    _tab_id: TabId,
    editor: gpui::Entity<editor_component::Editor>,
    colors: UiColors,
    window: &mut Window,
    _cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let _ = window;
    div()
        .id(("sql-editor", editor.entity_id()))
        .flex_1()
        .min_h(px(0.))
        .relative()
        .bg(colors.input_bg)
        // 键上下文仅供 SQL 执行快捷键（ExecuteQueryShortcut）在聚焦编辑器上派发。
        .key_context(editor_component::CONTEXT)
        // SQL 执行快捷键（Run/Select/Explain）分派：读取 `ExecuteQueryShortcut.mode`
        // 原样传给编辑器执行入口，避免被按文本探测的模式替换（整改 6.1/6.2）。
        .on_action({
            let editor = editor.clone();
            move |action: &sql_editor_adapter::ExecuteQueryShortcut,
                  _window: &mut Window,
                  app: &mut gpui::App| {
                editor.update(app, |editor, cx| editor.request_execution(action.mode, cx));
            }
        })
        .child(editor)
}

fn query_editor_content(
    tab_id: TabId,
    editor: &QueryEditorState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql_editor = this.query_editor_state(tab_id, editor, window, cx);
    // 新建连接弹框打开时，焦点属于弹框输入框；不要在延迟回调里抢回 SQL 编辑器焦点。
    if this.new_connection_kind.is_none() && !sql_editor.read(cx).is_focused(window) {
        // 这里不能直接在 render 里抢焦点；等本帧挂树完成后再聚焦，才能稳定点亮光标。
        let sql_editor = sql_editor.clone();
        cx.defer_in(window, move |this, window, cx| {
            if this.new_connection_kind.is_none() {
                sql_editor.update(cx, |editor, _cx| editor.focus(window));
            }
        });
    }
    let connection = connection_name(this.controller.state(), editor.connection_id);
    let database = editor.database.as_deref().unwrap_or("默认库").to_string();
    let accent = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == editor.connection_id)
        .map(|connection| connection_config_color_hex(&connection.config.options))
        .map(connection_color_rgba)
        .unwrap_or_else(|| connection_color_rgba(DEFAULT_CONNECTION_COLOR));
    let output_placement = this.query_output_placement;
    let workspace = match output_placement {
        QueryOutputPlacement::Bottom => div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(sql_editor_panel(tab_id, sql_editor.clone(), colors, window, cx))
            .child(query_output_panel(
                tab_id,
                editor,
                output_placement,
                this,
                window,
                colors,
                cx,
            )),
        QueryOutputPlacement::Right => div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(sql_editor_panel(tab_id, sql_editor.clone(), colors, window, cx)),
            )
            .child(query_output_panel(
                tab_id,
                editor,
                output_placement,
                this,
                window,
                colors,
                cx,
            )),
    };

    div()
        .flex_1()
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, {
            let sql_editor = sql_editor.clone();
            move |_, _, cx| {
                sql_editor.update(cx, |editor, cx| editor.hide_completion(cx));
            }
        })
        .on_mouse_down(MouseButton::Right, {
            let sql_editor = sql_editor.clone();
            move |_, _, cx| {
                sql_editor.update(cx, |editor, cx| editor.hide_completion(cx));
            }
        })
        .child(query_toolbar(
            this,
            tab_id,
            editor,
            sql_editor.clone(),
            connection,
            database,
            accent,
            output_placement,
            colors,
            cx,
        ))
        .child(workspace)
}

/// Redis Workbench 面板：命令输入 + 执行按钮 + 结果列表。
///
/// 不复用 SQL 查询壳的表格结果模型，独立渲染每条 Redis 命令的回复。
fn redis_workbench_content(
    tab_id: TabId,
    workbench: &RedisWorkbenchState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let input = this.redis_workbench_state(tab_id, workbench, window, cx);
    let connection = connection_name(this.controller.state(), workbench.connection_id);
    let accent = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == workbench.connection_id)
        .map(|connection| connection_config_color_hex(&connection.config.options))
        .map(connection_color_rgba)
        .unwrap_or_else(|| connection_color_rgba(DEFAULT_CONNECTION_COLOR));

    // 上下分栏：编辑器区高度 = 全局占比 × 可用高度（记住并恢复大小）；结果区占据剩余空间。
    let split_height = redis_workbench_split_height(window);
    let editor_ratio =
        f32::from(this.controller.state().settings.redis_workbench_editor_ratio) / 100.0;
    let editor_height = redis_workbench_editor_height(editor_ratio, split_height);
    div()
        .relative()
        .flex_1()
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(redis_workbench_toolbar(
            tab_id,
            workbench,
            connection,
            format!("DB {}", workbench.database),
            accent,
            colors,
            cx,
        ))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .child(redis_workbench_input_panel(
                    input.clone(),
                    editor_height,
                    colors,
                    window,
                    cx,
                ))
                .child(redis_workbench_split_handle(editor_height, colors, cx))
                .child(redis_workbench_results(tab_id, workbench, this, colors, cx)),
        )
}

/// Redis CLI 终端内容：一个真实 PTY 会话（redis-cli）的终端 surface。
///
/// 与 Redis Workbench 不同，输入/输出都发生在终端 surface 内（不走底部输入框 + 结果卡片）。
/// 顶部仅一条连接/库的收窄信息条，真正交互区由 TerminalComponent 全权负责（渲染网格、光标、
/// 状态栏与危险命令确认条）。会话按 tab 懒建并复用，见 NavicatMain::terminal_component_for。
fn redis_cli_content(
    tab_id: TabId,
    cli: &fluxdb_app::RedisCliState,
    this: &mut NavicatMain,
    _window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let component = this.terminal_component_for(tab_id, cli, cx);
    let connection = connection_name(this.controller.state(), cli.connection_id);
    div()
        .relative()
        .flex_1()
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(28.))
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .px_3()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(format!("{connection} · DB {} · redis-cli", cli.database)),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                // Entity<TerminalComponent> 自身即完整终端表面（Render 带回 key context / 焦点 / 自绘 canvas）。
                .child(component),
        )
}

/// Redis Pub/Sub 页面：订阅 / 发布实时消息流。
///
/// 会话与消息流由 `navicat_main.pubsub_sessions` 持有（见 pubsub.rs），此处只负责：
/// 顶部工具栏（连接库上下文 + 订阅输入 + 状态）、中央消息流表格（时间 / 通道 / 消息三列
/// + 分页）、底部发布面板。布局借鉴 RedisInsight 的「订阅置顶、消息流表格为主区域、发布
/// 置底」结构，但全部使用 gdb 的 `UiColors` 主题 token（亮 / 暗色均可用）。连接失败 /
/// 连接不存在时呈现明确错误态，不静默失败。
fn redis_pubsub_content(
    tab_id: TabId,
    pubsub: &fluxdb_app::RedisPubSubState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 先确保会话模型与输入框存在；失败（连接不存在）→ 呈现明确错误态。
    let has_session = this.pubsub_session_for(tab_id, pubsub, cx).is_some();
    let connection = connection_name(this.controller.state(), pubsub.connection_id);
    if !has_session {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.muted)
            .text_size(px(13.))
            .child("无法打开 Pub/Sub：连接不存在或尚未连接");
    }
    this.ensure_pubsub_inputs(tab_id, window, cx);

    let accent = rgb(0x20c76a);
    let model = this.pubsub_sessions.get(&tab_id).expect("Pub/Sub 会话已建立");
    let status_text = if model.connected {
        "已连接".to_string()
    } else if model.connecting {
        "连接中…".to_string()
    } else {
        "连接失败".to_string()
    };
    let status_color = if model.connected {
        accent
    } else if model.connecting {
        colors.muted
    } else {
        rgb(0xff5c5c)
    };
    let error = model.error.clone();
    let subscribed = model.subscribed.clone();
    let messages = model.messages.clone();
    let total = messages.len();
    let page_size = model.message_page_size;
    let page = model.message_page;
    let page_count = total.div_ceil(page_size);
    let subscribe_entity = model.subscribe_input_entity.clone();
    let publish_channel_entity = model.publish_channel_input_entity.clone();
    let publish_message_entity = model.publish_message_input_entity.clone();
    let has_messages = total > 0;

    // ---------- 顶部工具栏：连接上下文 + 状态 + 订阅入口 + 清空 ----------
    // 订阅入口与状态并排置于顶部（RedisInsight 的 header 结构），保持紧凑。
    let header = div()
        .h(px(38.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(colors.panel_alt)
        .child(query_context_box(
            format!("{connection} · DB {}", pubsub.database),
            accent,
            colors,
        ))
        .child(
            div()
                .text_size(px(12.))
                .text_color(status_color)
                .child(status_text),
        )
        .child(div().flex_1())
        // 订阅输入 + 订阅按钮。
        .child(
            div()
                .w(px(180.))
                .child(
                    subscribe_entity
                        .map(|entity| Input::new(&entity).into_any_element())
                        .unwrap_or_else(|| div().into_any_element()),
                ),
        )
        .child(
            query_toolbar_icon_button("订阅", AppIcon::Broadcast, true, false, accent, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.pubsub_subscribe_clicked(tab_id, window, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        // 清空消息。
        .child(
            query_toolbar_icon_button("清空", AppIcon::Trash, has_messages, false, rgb(0xff5c5c), colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.pubsub_clear_messages(tab_id);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                ),
        );

    // ---------- 已订阅通道条：紧凑横排，点击即取消订阅 ----------
    let mut subscribed_row = div()
        .h(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .gap_1p5()
        .px_3()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(format!("已订阅 {} ·", subscribed.len())),
        );
    for (index, ch) in subscribed.iter().enumerate() {
        let ch = ch.clone();
        let channel_for_close = ch.clone();
        subscribed_row = subscribed_row.child(
            div()
                .id(SharedString::from(format!("pubsub-subscribed-{ch}-{index}")))
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .px_1p5()
                .py_0p5()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .hover(|style| style.bg(colors.hover))
                .tooltip(move |window, cx| Tooltip::new("点击取消订阅").build(window, cx))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.pubsub_unsubscribe(tab_id, &channel_for_close);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .max_w(px(120.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.))
                        .text_color(colors.text)
                        .child(ch.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child("×"),
                ),
        );
    }

    // ---------- 中央：消息流表格（时间 / 通道 / 消息 三列，条纹行，紧凑行高） ----------
    // 表头：与数据表一致的三列结构，便于对齐分属关系。
    let table_header = div()
        .h(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .px_2()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(pubsub_column_time("时间".to_string()))
        .child(pubsub_column_channel("通道".to_string()))
        .child(pubsub_column_message(format!("消息（{total}）")));

    // 当前页对应的消息（倒序分页：最新消息在第一页首行）。
    let newest_first: Vec<&crate::PubSubMessageView> = messages.iter().rev().collect();
    let page_messages: Vec<&crate::PubSubMessageView> = newest_first
        .iter()
        .skip(page * page_size)
        .take(page_size)
        .copied()
        .collect();

    let mut table_body = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .overflow_x_hidden();
    for (row_index, msg) in page_messages.iter().enumerate() {
        let channel = msg.channel.clone();
        let pattern = msg.pattern.clone();
        let payload = msg.payload.clone();
        let time_label = redis_workbench_time_label(msg.received_at as u64);
        // 条纹行：交替底色增强行辨识，行内按列固定宽度对齐。
        let row_bg = if row_index % 2 == 0 {
            colors.panel_bg
        } else {
            colors.panel_alt
        };
        table_body = table_body.child(
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .px_2()
                .border_b_1()
                .border_color(colors.border)
                .bg(row_bg)
                .hover(|style| style.bg(colors.hover))
                .child(pubsub_column_time(time_label).text_color(colors.muted))
                .child(
                    pubsub_column_channel(if let Some(p) = pattern {
                        format!("[{p}] {channel}")
                    } else {
                        channel
                    })
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(accent),
                )
                .child(
                    pubsub_column_message(if payload.is_empty() {
                        "（空消息）".to_string()
                    } else {
                        payload
                    })
                    .text_size(px(12.))
                    .text_color(colors.text),
                ),
        );
    }

    // ---------- 分页栏：首页 / 上一页 / 页指示 / 下一页 / 末页 + 每页条数 + 总数 ----------
    let pagination = pubsub_pagination_bar(
        tab_id,
        total,
        page,
        page_count,
        page_size,
        colors,
        cx,
    );

    // ---------- 底部：发布面板（通道 + 内容 + 发布按钮），RedisInsight 置底结构 ----------
    let publish_footer = div()
        .h(px(40.))
        .flex_none()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("发布"),
        )
        .child(
            div()
                .w(px(160.))
                .child(
                    publish_channel_entity
                        .map(|entity| Input::new(&entity).into_any_element())
                        .unwrap_or_else(|| div().into_any_element()),
                ),
        )
        .child(
            div()
                .flex_1()
                .child(
                    publish_message_entity
                        .map(|entity| Input::new(&entity).into_any_element())
                        .unwrap_or_else(|| div().into_any_element()),
                ),
        )
        .child(
            query_toolbar_icon_button("发布", AppIcon::Play, true, false, accent, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.pubsub_publish_clicked(tab_id, window, cx);
                        cx.stop_propagation();
                    }),
                ),
        );

    // 错误态条（连接失败时不静默）。
    let error_bar = error.map(|message| {
        div()
            .h(px(30.))
            .flex_none()
            .flex()
            .items_center()
            .px_3()
            .text_size(px(12.))
            .text_color(rgb(0xff5c5c))
            .bg(rgb(0x2a1415))
            .child(format!("Pub/Sub 错误: {message}"))
    });

    div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        .bg(colors.panel_bg)
        .flex()
        .flex_col()
        .child(header)
        .child(subscribed_row)
        .when_some(error_bar, |this, bar| this.child(bar))
        // 消息流表格为页面主区域，占满剩余空间；其下接分页栏与发布面板。
        .child(table_header)
        .child(table_body)
        .child(pagination)
        .child(publish_footer)
}

/// 表头 / 行内「时间」列容器：固定宽度，右对齐便于数字时间等比阅读。
fn pubsub_column_time(label: String) -> Div {
    div()
        .w(px(150.))
        .flex_none()
        .flex()
        .items_center()
        .pr_2()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(px(11.))
        .child(label)
}

/// 表头 / 行内「通道」列容器：固定宽度，文本溢出省略。
fn pubsub_column_channel(label: String) -> Div {
    div()
        .w(px(200.))
        .flex_none()
        .flex()
        .items_center()
        .pr_2()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(label)
}

/// 表头 / 行内「消息」列容器：占据剩余宽度，单行省略，hover 可完整展示（tooltip 由调用侧给）。
fn pubsub_column_message(label: String) -> Div {
    div()
        .w(px(200.))
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(label)
}

/// 消息流分页栏：首页 / 上一页 / 页指示 / 下一页 / 末页 + 每页条数 + 总数。
///
/// 全部使用 gdb 主题 token（按钮底、边框、文字），并沿用 `query_toolbar_icon_button` 的
/// 图标按钮风格；每页条数用一组紧凑分段按钮快速切换，避免引入下拉实体状态。
fn pubsub_pagination_bar(
    tab_id: TabId,
    total: usize,
    page: usize,
    page_count: usize,
    page_size: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_prev = page > 0 && page_count > 0;
    let can_next = page + 1 < page_count;
    let page_sizes = [25usize, 50, 100, 200];

    // 单页指示文本：第 X / Y 页。
    let page_indicator = if page_count == 0 {
        "0 / 0".to_string()
    } else {
        format!("{} / {}", page + 1, page_count)
    };

    // 复用查询工具栏图标按钮风格构建分页小按钮。
    let pager_button = |label: &'static str,
                        icon: AppIcon,
                        enabled: bool,
                        action: fn(usize, usize) -> usize|
     -> Stateful<Div> {
        let target = action(page, page_count);
        query_toolbar_icon_button(label, icon, enabled, false, rgb(0x1687ff), colors)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(m) = this.pubsub_sessions.get_mut(&tab_id) {
                        m.message_page = target;
                    }
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
    };

    let first = pager_button("首页", AppIcon::ChevronsLeft, can_prev, |_, _| 0);
    // prev: 若已在边界则不动，否则减一。
    let prev = pager_button(
        "上一页",
        AppIcon::ChevronLeft,
        can_prev,
        |page, _| page.saturating_sub(1),
    );
    let next = pager_button(
        "下一页",
        AppIcon::ChevronRight,
        can_next,
        |page, _| page + 1,
    );
    let last = pager_button(
        "末页",
        AppIcon::ChevronsRight,
        can_next,
        |_, page_count| page_count.saturating_sub(1),
    );

    div()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("每页"),
        )
        // 每页条数分段选择。
        .children(page_sizes.iter().map(|size| {
            let size_val = *size;
            let active = size_val == page_size;
            let tap_id = tab_id;
            div()
                .id(SharedString::from(format!("pubsub-page-size-{size_val}")))
                .size(px(22.))
                .rounded(colors.radius)
                .border_1()
                .border_color(if active {
                    rgb(0x1687ff)
                } else {
                    colors.border
                })
                .bg(if active { rgb(0x1687ff) } else { colors.panel_bg })
                .cursor_pointer()
                .hover(|style| style.bg(if active { rgb(0x1687ff) } else { colors.hover }))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .text_color(if active { rgb(0xffffff) } else { colors.text })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if let Some(m) = this.pubsub_sessions.get_mut(&tap_id) {
                            m.message_page_size = size_val;
                            m.message_page = 0;
                        }
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(size_val.to_string())
        }))
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(format!("共 {total} 条")),
        )
        .child(div().flex_1())
        // 首页 / 上一页。
        .child(first)
        .child(prev)
        .child(
            div()
                .min_w(px(58.))
                .text_size(px(11.))
                .text_color(colors.text)
                .child(page_indicator),
        )
        .child(next)
        .child(last)
}

/// Redis Workbench 顶部工具栏：连接 / 数据库上下文 + 运行按钮（带 loading）。
fn redis_workbench_toolbar(
    tab_id: TabId,
    workbench: &RedisWorkbenchState,
    connection: String,
    database: String,
    accent: gpui::Rgba,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_text = !workbench.text.trim().is_empty();
    let running = workbench.running;
    let has_results = !workbench.executions.is_empty();
    // 运行前可能触发危险命令二次确认，需要把待执行文本提前取出供闭包判断。
    let pending_text = workbench.text.clone();
    div()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(colors.panel_alt)
        .child(query_context_box(connection, accent, colors))
        .child(query_context_box(database, accent, colors))
        .child(
            query_toolbar_icon_button(
                if running { "执行中" } else { "运行" },
                AppIcon::Play,
                has_text && !running,
                running,
                rgb(0x20c76a),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if has_text && !running {
                        // 含破坏性命令时先弹确认框；确认后才派发执行。
                        if this.request_redis_dangerous_confirmation(tab_id, &pending_text, cx) {
                            cx.stop_propagation();
                        } else {
                            this.dispatch(AppCommand::ExecuteRedisWorkbench(tab_id), cx);
                            cx.stop_propagation();
                        }
                    }
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                "清除结果",
                AppIcon::Trash,
                has_results && !running,
                false,
                rgb(0xff5c5c),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(AppCommand::ClearRedisWorkbenchResults(tab_id), cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(if workbench.running { "执行中…" } else { "Redis 命令执行器" }),
        )
}


/// 命令输入区：多行代码输入框，高度由分栏占比决定（可上下拖动调整）。
///
/// 布局要点：这里不能只靠外层的 `h(px(height))` 撑高。gpui 的 `Input` 是多行时
/// 默认 `h_auto()` 自动增长、按 `rows(N)` 取整行内容高度（见 input.rs render 的
/// `.h_auto()` 分支），因此即使外层容器变大，输入框自身仍会缩成 `rows(8)` 的“小框”。
/// 要让它真正填满可用高度，必须对 `Input` 显式 `.h_full()`（内部等价 `height =
/// relative(1.)`，会覆盖自动增长，让编辑器随外层容器一起伸缩），并在外层容器
/// `overflow_y_scrollbar()` 保证内容超长时可滚动、不会把结果区顶下去。这和三方 SQL
/// 编辑器用自定义 Canvas + `.flex_1()` 填满的思路一致，只是复用内置 `Input`。
fn redis_workbench_input_panel(
    editor: Entity<editor_component::Editor>,
    height: f32,
    colors: UiColors,
    _window: &mut Window,
    _cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 键鼠/滚动/滚动条/动作分发已下沉到 Editor::render()；此处仅保留布局定位 + 配色。
    div()
        .flex_none()
        .h(px(height))
        .min_h(px(0.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .child(editor)
}

/// Redis Workbench 上下分栏拖动手柄：居中一排 3 个圆点（对齐 RedisInsight 的 grip 观感）。
/// 拖动改变全局分栏占比并即时写回持久化设置，重开 Workbench 仍可恢复。
fn redis_workbench_split_handle(
    editor_height: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    const HANDLE_H: f32 = 7.;
    let dot = colors.muted;
    div()
        .id("redis-workbench-split-handle")
        .flex_none()
        .h(px(HANDLE_H))
        .bg(colors.panel_bg)
        .cursor_ns_resize()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        // 3 个 grip 圆点：常态用弱色，手柄 hover 时整条高亮。
        .child(div().size(px(2.)).rounded_full().bg(dot))
        .child(div().size(px(2.)).rounded_full().bg(dot))
        .child(div().size(px(2.)).rounded_full().bg(dot))
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.redis_workbench_panel_resize_start =
                    Some(RedisWorkbenchPanelResizeStart {
                        y: f32::from(event.position.y),
                        editor_height,
                    });
                cx.stop_propagation();
            }),
        )
        .on_drag(RedisWorkbenchPanelResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<RedisWorkbenchPanelResizeDrag>, window, cx| {
                let Some(start) = this.redis_workbench_panel_resize_start else {
                    return;
                };
                // 拖拽增量 = 当前鼠标 Y − 按下时 Y（与侧边栏/建表面板的
                // `start.x + current.x - start.x` 换算方向一致）：
                // 手柄向下拖动（鼠标 Y 增大）→ 编辑器区变高，向上拖动 → 变小。
                let delta = f32::from(event.event.position.y) - start.y;
                let current_split = redis_workbench_split_height(window);
                let new_height =
                    redis_workbench_clamp_editor_height(start.editor_height + delta, current_split);
                // 换算回整数占比并写入内存设置，触发重渲染即时反馈；落盘推迟到松开鼠标。
                let ratio_pct = (new_height / current_split * 100.0).round().clamp(0.0, 100.0) as u8;
                let mut settings = this.controller.state().settings.clone();
                settings.redis_workbench_editor_ratio = ratio_pct;
                let _ = this.controller.dispatch(AppCommand::SaveSettings(settings));
                cx.notify();
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                if this.redis_workbench_panel_resize_start.take().is_some() {
                    // 松开时把最终占比落盘，重开 Workbench / 重启后仍能恢复。
                    let _ = this.storage.save_settings(&this.controller.state().settings);
                }
                cx.stop_propagation();
            }),
        )
}

/// 结果区：空态 / 执行中 / 错误态，或按「一执行一条记录」展示的执行记录列表。
/// `this` 用于读取 NavicatMain 自身的 hover 记录态；不能在 render 期间通过
/// `cx.entity().read(cx)` 自读（render 时 NavicatMain 已被租用，会触发 double-lease panic）。
fn redis_workbench_results(
    tab_id: TabId,
    workbench: &RedisWorkbenchState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let body: gpui::AnyElement = if workbench.executions.is_empty() {
        if workbench.running {
            redis_workbench_loading_state(colors).into_any_element()
        } else if let Some(error) = workbench.error.as_ref() {
            // 重试与工具栏「运行」同路：含破坏性命令时先弹二次确认。
            let pending_text = workbench.text.clone();
            let retry = Button::new(("retry-redis-workbench", tab_id.0))
                .label("重试")
                .small()
                .outline()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.request_redis_dangerous_confirmation(tab_id, &pending_text, cx) {
                        this.dispatch(AppCommand::ExecuteRedisWorkbench(tab_id), cx);
                    }
                }))
                .into_any_element();
            query_output_error_state(tab_id, error, Some(retry), cx, colors).into_any_element()
        } else {
            redis_workbench_empty_state(colors).into_any_element()
        }
    } else {
        // 已有执行记录时始终展示记录列表；新执行完成后把新记录追加到列表末尾。
        redis_workbench_records(tab_id, workbench, this, colors, cx).into_any_element()
    };

    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .child(body)
        .into_any_element()
}

/// 按执行顺序逐条渲染执行记录。每条记录：折叠 / 展开 + 状态 + 命令文本 + 时间 + 耗时 + 右侧 Run / Delete。
fn redis_workbench_records(
    tab_id: TabId,
    workbench: &RedisWorkbenchState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let running = workbench.running;
    // 结果列表铺在更深的结果区底上，卡片之间留出纵向间距，形成清晰的“黑底嵌卡片”层次。
    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .bg(colors.status_bg);
    for execution in &workbench.executions {
        let collapsed = workbench.collapsed.contains(&execution.id);
        list = list.child(redis_workbench_record_row(
            tab_id,
            execution,
            running,
            collapsed,
            &workbench.json_views,
            this,
            colors,
            cx,
        ));
    }
    list
}

/// Redis Workbench 记录头部轻量纯图标按钮：无外边框、小尺寸（18px 命中区 / 14px 图标），
/// hover 仅轻微背景高亮（透明 → 主题 hover 底），明暗主题经 `colors.hover` 适配。
/// 返回 `Stateful<Div>`，调用方可继续链式绑定 `on_mouse_down`（各自 stop_propagation）。
fn redis_workbench_icon_button(
    execution_id: u64,
    tooltip_label: &'static str,
    icon: AppIcon,
    icon_color: gpui::Rgba,
    colors: UiColors,
    enabled: bool,
) -> Stateful<Div> {
    // 禁用（运行中）时图标降为 muted，且不显示手型 / tooltip。
    let icon_color = if enabled { icon_color } else { colors.muted };
    div()
        .id((tooltip_label, execution_id))
        .size(px(18.))
        .flex_none()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .tooltip(move |window, cx| Tooltip::new(tooltip_label).build(window, cx))
        })
        .child(app_icon(icon, 14., icon_color))
}

/// 单条执行记录卡片：一次提交中的一条命令 = 一张卡（执行单元 = 单条命令）。
/// 卡片头部整栏可点击切换折叠/展开；左侧箭头仅是折叠状态指示图标（非按钮）。
/// 右侧提供独立的 Run（重跑）与 Delete（只删当前）图标，点击不触发整栏折叠。
/// 身体展示该命令的 reply，结果区点击不触发折叠。折叠后只显示头部；运行期间禁用交互。
fn redis_workbench_record_row(
    tab_id: TabId,
    execution: &CommandWorkbenchExecution,
    running: bool,
    collapsed: bool,
    json_views: &BTreeSet<(u64, usize)>,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let execution_id = execution.id;
    let workbench_json_views = json_views;
    let command_text = execution.text.clone();
    // 记录层 JSON 判定与视图态：每条执行记录恰好承载一条命令（command_index 恒为 0），
    // 因而把 `Text / JSON` 切换放在"记录头部运行图标之前"，状态沿用 `json_views`，不引入重复源。
    let is_json_cmd = execution
        .commands
        .first()
        .map(redis_workbench_is_json_command)
        .unwrap_or(false);
    let is_json_view = json_views.contains(&(execution_id, 0));
    // 供 Popover 内容闭包内 dispatch 命令使用（NavicatMain 弱引用），复用既有状态源。
    let navicat_view = cx.entity().downgrade();
    let success = execution.summary.failed == 0;
    let status_color = if success {
        rgb(0x20c76a)
    } else {
        rgb(0xff5c5c)
    };
    let status_label = if success { "成功" } else { "失败" };
    // 耗时（毫秒级，RedisInsight 风格 `N msec`）与执行时刻（完整日期时间，来自 started_at_unix_secs）双展示。
    // 后端仅存整数毫秒（`elapsed_ms: u64`），如实展示整数 `msec`，不拼接假的小数精度。
    let elapsed_label = format!("{} msec", execution.elapsed_ms);
    let time_label = redis_workbench_time_label(execution.started_at_unix_secs);
    // 折叠箭头：展开用 ChevronDown，折叠用 ChevronRight。
    let collapse_icon = if collapsed {
        AppIcon::ChevronRight
    } else {
        AppIcon::ChevronDown
    };
    // 记录下的每条子命令独立展示 reply，复用原有 command row 渲染（折叠时整段不渲染）。
    // 身体直接铺在卡片基色的深色底上（见下方外层卡片），与头部横条形成明显分层。
    // 结果区设置最大高度：超长 reply / 长列表在卡片内部纵向滚动查看，只滚动结果正文本身，
    // 不影响外层记录列表与整页布局。
    // 注意：这里用原生 `overflow_y_scroll()` 而非 `overflow_y_scrollbar()`（Scrollable 封装）。
    // Scrollable 的 render 会给结果区套 `size_full()`（height:100%）并让内部滚动区 `flex_1()`，
    // 在 auto 高度的卡片 flex 列里会压垮自身内容高度 → 结果内容较短时整卡被压矮。
    // 改为原生 overflow_y_scroll 后，body 保持内容感知（flex_none：0 0 auto，不做高度收缩），
    // 内容短时即自然高度、卡片不变矮；内容超过 max_h 阈值时封顶在阈值内纵向滚动。
    // 也无需 `min_h(px(0.))`：body 不参与外部收缩，不会因 max-height 约束而失效。
    // `overflow_y_scroll()` 是 gpui `StatefulInteractiveElement` 的 trait 方法，需在带 `.id()` 的
    // 有状态元素上调用（id 需记录级唯一）——这里也要保留唯一 id 以便 `.id()` 调用合法。
    let mut body = div()
        .id(("redis-wb-record-body", execution_id))
        .flex_none()
        .max_h(px(REDIS_WB_RECORD_BODY_MAX_HEIGHT))
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .overflow_y_scrollbar();
    for (command_index, item) in execution.commands.iter().enumerate() {
        let is_json_view = workbench_json_views.contains(&(execution_id, command_index));
        body = body.child(redis_workbench_command_row(item, is_json_view, colors));
    }

    // RedisInsight 式卡片：外层圆角卡片（深黑底）+ 头部深灰横条 + 下方更深的黑底结果区；
    // `overflow_hidden` 裁切圆角，头部与结果区分层明显。
    // 头部为三段式均衡布局，从左到右依次：左侧命令（整栏点击折叠，箭头仅作折叠状态指示）、
    // 中间 meta 状态/时间/耗时、右侧紧凑操作（Text/JSON 下拉 + Run + Delete）。
    // 左侧与右侧动作区均为 `flex_1`，使中间 meta 时间区在可用宽度内居中、三段分布更均衡；
    // 动作区 `justify_end` 保证按钮始终贴右，按钮间用 `gap_2` 保持合适间距。
    // 右侧按钮各自 stop_propagation，点击不触发整栏折叠。
    // 直接读 NavicatMain 自身字段判断 hover 态：不能经 `cx.entity().read(cx)` 自读，
    // render 期间 NavicatMain 已被租用（更新中），自读会触发
    // 「cannot read NavicatMain while it is already being updated」double-lease panic。
    let row_hovered = this.redis_workbench_hovered_record == Some((tab_id, execution_id));
    // 复制命令用独立的克隆，供 hover 浮现的复制按钮闭包使用，避免 move 走下方动作区要用的 `command_text`。
    let copy_command_text = command_text.clone();
    // 头部带唯一 id，使元素为有状态（`Stateful<Div>`），才可绑定 `on_hover`；
    // 也用作「复制命令」hover 浮现的记录级唯一标识。
    let header = div()
        .id(("redis-wb-record-header", execution_id))
        .h(px(34.))
        .px_2()
        .gap_2()
        .bg(colors.panel_alt)
        .cursor_pointer()
        .flex()
        .items_center()
        // hover 态记录：仅用于让「复制命令」按钮在本行 hover 时浮现（不常驻抢标题行空间），
        // 不改变按钮 / 折叠语义。hover 离开即清除。
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.redis_workbench_hovered_record = if *hovered {
                Some((tab_id, execution_id))
            } else {
                None
            };
            cx.notify();
        }))
        // 头部整栏点击切换折叠/展开；运行期间禁用。右侧按钮各自 stop_propagation，不会冒泡到此处。
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if running {
                    return;
                }
                this.dispatch(
                    AppCommand::ToggleRedisWorkbenchRecordCollapse {
                        tab_id,
                        execution_id,
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        // 左侧：折叠状态指示箭头 + 命令文本（可收缩、过长截断省略），整体 flex_1 占据剩余宽度。
        // 「复制命令」仅在整行 hover 时浮现，避免常驻占位。
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap_1()
                .child(app_icon_box(collapse_icon, 24., 12., colors.muted).flex_none())
                .child(
                    div()
                        .min_w(px(0.))
                        .flex_shrink(1.)
                        .text_size(px(12.))
                        .font_family("Menlo")
                        .text_color(colors.text)
                        .overflow_x_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(single_line_summary_text(command_text.clone())),
                )
                .when(row_hovered && !running, |this| {
                    // 复制命令：hover 浮现的轻量纯图标，点击复制本记录完整命令文本；stop_propagation 不触发折叠。
                    this.child(
                        redis_workbench_icon_button(
                            execution_id,
                            "复制命令",
                            AppIcon::Copy,
                            colors.muted,
                            colors,
                            true,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener({
                                let copy_command_text = copy_command_text.clone();
                                move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_command_text.clone(),
                                    ));
                                    this.show_message("已复制命令", AppMessageKind::Success, cx);
                                    cx.stop_propagation();
                                }
                            }),
                        ),
                    )
                }),
        )
        // 中间 meta 区：运行状态 + 执行时间 + 耗时，状态紧挨时间/耗时之前、紧凑成一组，靠右不抢动作区。
        .child(redis_workbench_record_meta(
            status_label,
            status_color,
            &time_label,
            &elapsed_label,
            colors,
        ))
        // 右侧动作区：Text/JSON 下拉（仅 JSON 命令）+ Run（重跑）+ Delete（删除），紧凑排列。
        .child(redis_workbench_record_actions(
            tab_id,
            execution_id,
            command_text,
            running,
            is_json_cmd,
            is_json_view,
            navicat_view,
            colors,
            cx,
        ));

    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        // 卡片基色用更深的结果区底色，头部横条（panel_alt）叠在上方形成“黑底嵌卡”层次。
        .bg(colors.input_bg)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(header)
        .when(!collapsed, |this| this.child(body))
}

/// 记录头部中间 meta 区：运行状态 + 执行时间 + 耗时，紧凑成一组、状态紧挨时间/耗时之前。
/// 状态用成功 / 失败色突出，时间与耗时用弱化 muted；整组 `flex_none` 靠右、不抢动作区。
/// 颜色全走 `UiColors` 与主题绿 / 红固定语义色，明暗主题均适配。
fn redis_workbench_record_meta(
    status_label: &'static str,
    status_color: gpui::Rgba,
    time_label: &str,
    elapsed_label: &str,
    colors: UiColors,
) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(status_color)
                .child(status_label),
        )
        // 完整执行时间 + 毫秒耗时：如 `2026-01-01 14:03:22 · 16 msec`，整段为 ASCII（规避中英文混排 bug）。
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .whitespace_nowrap()
                .child(format!("{time_label} · {elapsed_label}")),
        )
}

/// 记录头部右侧动作区：Text/JSON 下拉（仅 JSON 命令）+ Run（重跑）+ Delete（删除）。
/// 按钮各自 `stop_propagation` 避免触发整栏折叠；运行期间 Run / Delete 禁用。
/// `flex_1 + justify_end` 让动作区从右侧边缘占满整行，配合左侧命令区的 `flex_1`，
/// 使中间 meta 时间区在三段间居中、分布更均衡（贴合 RedisInsight 头部）；
/// 按钮间用 `gap_2` 适度拉开水平间距，避免挤在一起。
/// 复用 `redis_workbench_icon_button` 轻量纯图标方案，明暗主题经 `colors` 适配。
fn redis_workbench_record_actions(
    tab_id: TabId,
    execution_id: u64,
    command_text: String,
    running: bool,
    is_json_cmd: bool,
    is_json_view: bool,
    navicat_view: WeakEntity<NavicatMain>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .children({
            let mut icons: Vec<gpui::AnyElement> = Vec::new();
            // 记录层的 `Text / JSON` 切换下拉：置于运行图标之前（仅 JSON 命令显示）。
            // 切换入口从命令行标题行迁移到"结果头部运行图标前"，与 RedisInsight 结果头部
            // 一致；状态沿用 `json_views`（每条执行记录恰好一条命令，command_index 恒为 0）。
            // 外层包一层 `stop_propagation`：记录头整栏绑定了折叠 on_mouse_down，点击下拉
            // 需终止冒泡，避免同时触发整卡折叠。
            if is_json_cmd {
                icons.push(
                    div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(redis_workbench_json_view_dropdown(
                            tab_id,
                            execution_id,
                            0,
                            is_json_view,
                            navicat_view.clone(),
                            colors,
                        ))
                        .into_any_element(),
                );
            }
            // 单条 Run：重跑当前记录（危险命令二次确认约束，运行期间禁用），轻量纯图标。
            icons.push(
                redis_workbench_icon_button(
                    execution_id,
                    "重跑",
                    AppIcon::Play,
                    colors.muted,
                    colors,
                    !running,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let command_text = command_text.clone();
                        move |this, _, _, cx| {
                            if running {
                                return;
                            }
                            if this.request_redis_record_rerun_confirmation(
                                tab_id,
                                &command_text,
                                execution_id,
                                cx,
                            ) {
                                cx.stop_propagation();
                            } else {
                                this.dispatch(
                                    AppCommand::RerunRedisWorkbenchRecord {
                                        tab_id,
                                        execution_id,
                                    },
                                    cx,
                                );
                                cx.stop_propagation();
                            }
                        }
                    }),
                )
                .into_any_element(),
            );
            // 单条 Delete：只删除当前这条记录（运行期间禁用，避免中断执行），轻量纯图标。
            icons.push(
                redis_workbench_icon_button(
                    execution_id,
                    "删除",
                    AppIcon::Trash,
                    rgb(0xff5c5c),
                    colors,
                    !running,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if running {
                            return;
                        }
                        this.dispatch(
                            AppCommand::DeleteRedisWorkbenchRecord {
                                tab_id,
                                execution_id,
                            },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )
                .into_any_element(),
            );
            icons
        })
}

/// 把 Unix 秒时间戳格式化为本地完整日期时间 `YYYY-MM-DD HH:MM:SS`，
/// 用于每条执行记录头部的时间展示（复用后端 `started_at_unix_secs` 的开始时间）。
/// 合成 / 测试数据可能传 0，此时转空字符串，避免展示纪元时间。
fn redis_workbench_time_label(unix_secs: u64) -> String {
    if unix_secs == 0 {
        return String::new();
    }
    chrono::DateTime::from_timestamp(unix_secs as i64, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// 空态：尚未执行任何命令。
fn redis_workbench_empty_state(colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(colors.muted)
        // 空态提示：整句保持纯中文，避免与英文命令示例混排。
        // gpui 0.2.2 对「中文 + ASCII 命令」同一文本节点内的跨字体 run 切分存在
        // 字节越界 bug（shape_line 处 panics），故不在此行混入 GET/SET 等英文示例。
        .child("输入命令后点击「运行」按钮执行")
}

/// 执行中占位态。
fn redis_workbench_loading_state(colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .child(loading_spinner_with_color(18., colors.muted))
        .child(div().text_size(px(13.)).text_color(colors.muted).child("执行中…"))
}

/// 单条命令的结果行。
fn redis_workbench_command_row(
    item: &CommandExecutionItem,
    is_json_view: bool,
    colors: UiColors,
) -> Div {
    let status_color = match item.status {
        CommandExecutionStatus::Success => rgb(0x20c76a),
        CommandExecutionStatus::Failed => rgb(0xff5c5c),
        CommandExecutionStatus::Skipped => colors.muted,
    };
    let status_label = match item.status {
        CommandExecutionStatus::Success => "OK",
        CommandExecutionStatus::Failed => "错误",
        CommandExecutionStatus::Skipped => "跳过",
    };
    // 模式切换入口已迁移到"记录头部运行图标之前"的紧凑下拉（见 `redis_workbench_json_view_dropdown`，
    // 由外层 `redis_workbench_record_row` 持有），命令行标题行不再重复展示 `Text / JSON` 切换控件。
    // 开启 JSON 视图时用 `JsonComponent::Preview` 只读渲染结构化高亮（非法 JSON 自动回退原文，绝不留空）；
    // 默认 Text 态沿用纯文本展示。
    let is_json_cmd = redis_workbench_is_json_command(item);
    let reply_body = if is_json_cmd {
        let mut component = JsonComponent::new(JsonEditorConfig::default());
        component.load(&command_reply_display(&item.reply));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(if is_json_view {
                component.render_preview(colors).into_any_element()
            } else {
                redis_workbench_reply_row(&item.reply, colors).into_any_element()
            })
    } else {
        redis_workbench_reply_row(&item.reply, colors)
    };
    div()
        .border_b_1()
        .border_color(colors.border_soft)
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(status_color)
                        .child(status_label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(13.))
                        .font_family("Menlo")
                        .text_color(colors.text)
                        .overflow_x_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(item.command.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("{}ms", item.elapsed_ms)),
                )
        )
        .child(reply_body)
}

/// 判断该子命令是否属于 JSON 命令（首个 argv 分词大写后以 `JSON.` 开头）。
///
/// 非 JSON 命令的结果不展示 `Text / JSON` 切换；This 只对 Redis 的 `JSON.*` 系列命令启用。
fn redis_workbench_is_json_command(item: &CommandExecutionItem) -> bool {
    item.argv_preview
        .first()
        .map(|s| s.to_ascii_uppercase().starts_with("JSON."))
        .unwrap_or(false)
}

/// 记录头部运行图标之前的 `Text / JSON` 展示模式下拉（紧凑 RedisInsight 风格）。
///
/// 切换入口从原来的结果内容区分段按钮迁移到结果层头部右侧、运行图标之前：触发按钮常驻显示
/// 当前模式（`Text` / `JSON`）并带下拉箭头，点击弹出只有两项的小菜单，选中项带 check 标记。
/// 状态复用原有 `json_views` 集合（按 `execution_id + command_index` 记忆，记录级此处
/// command_index 恒为 0），点击任一菜单项 dispatch `ToggleRedisWorkbenchJsonView`，由既有状态源
/// 驱动内容区立即刷新，不引入重复状态源。
/// 复用 `gpui_component` 的 `Popover`，触发按钮与菜单样式都用主题 `UiColors`，兼容明暗主题、不写死颜色。
/// 仅 JSON 命令渲染（由调用方 `is_json_cmd` 控制）。
fn redis_workbench_json_view_dropdown(
    tab_id: TabId,
    execution_id: u64,
    command_index: usize,
    is_json_view: bool,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
) -> Popover {
    // 触发按钮当前模式文案；未选中（Text）用 muted、选中（JSON）用主文本色做视觉区分。
    let current_label = if is_json_view { "JSON" } else { "Text" };
    // 触发按钮复用 gpui_component 的 `Button`（满足 Popover 的 `Selectable` 约束），
    // ghost + xsmall 紧凑尺寸；按钮本身不绑定点击，Popover 外层自动处理"点击触发开合"。
    // 用自定义 muted 箭头代替 Button 内建 `dropdown_caret`（后者 hover 会继承主题强调色），保持中性低干扰。
    let trigger = Button::new(SharedString::from(format!(
        "redis-wb-json-view-trigger-{execution_id}-{command_index}"
    )))
    .ghost()
    .xsmall()
    .h(px(20.))
    .px_1p5()
    .gap_1()
    .child(
        div()
            .text_size(px(11.))
            .font_family("Menlo")
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(if is_json_view { colors.text } else { colors.muted })
            .child(current_label),
    )
    .child(app_icon(AppIcon::ChevronDown, 10., colors.muted));

    // 菜单项配置：`(文案, 是否选中, 目标模式)`。
    let items: [(&'static str, bool, bool); 2] = [
        ("Text", !is_json_view, false),
        ("JSON", is_json_view, true),
    ];
    // 构建单个菜单项：左侧 check 标记占位 + 文案。点击时若目标模式与当前不同则 dispatch 切换，
    // 无论是否切换都立即关闭下拉（贴近原生下拉交互）。
    // 先 `view.clone()` / `popover.clone()` 各留一份给内层闭包：让 make_item 捕获不被子闭包 move 走，
    // 从而保持 `Fn` 可被 `.map` 多次调用（内容闭包每次渲染都会重建菜单项）。
    let make_item = move |(label, active, target_json): (&'static str, bool, bool),
                          popover: WeakEntity<PopoverState>| {
        let fg = if active { colors.text } else { colors.muted };
        let view = view.clone();
        let row = div()
            .px_1p5()
            .py_1()
            .rounded(colors.radius * 0.5)
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .hover(move |style| style.bg(colors.hover))
            // 选中态 check 标记占位；未选中留空，保持两行视觉对齐。
            .child(
                div()
                    .w(px(14.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if active {
                        app_icon(AppIcon::Check, 12., colors.text).into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(
                div()
                    .w_full()
                    .text_size(px(12.))
                    .font_family("Menlo")
                    .font_weight(if active {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(fg)
                    .child(label),
            );
        row.on_mouse_down(
            MouseButton::Left,
            move |_, window, cx| {
                // 与旧分段按钮语义一致：仅目标模式与当前不同才触发切换，保持幂等。
                if target_json != is_json_view {
                    let _ = view.update(cx, |this, cx| {
                        this.dispatch(
                            AppCommand::ToggleRedisWorkbenchJsonView {
                                tab_id,
                                execution_id,
                                command_index,
                            },
                            cx,
                        );
                    });
                }
                // 点击任意菜单项后立即关闭下拉：通过 Popover 状态实体调用 `dismiss`，
                // 让视图切换结果即时可见，交互更贴近原生下拉菜单。
                let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
            },
        )
    };

    Popover::new(gpui::ElementId::Name(SharedString::from(format!(
        "redis-wb-json-view-{execution_id}-{command_index}"
    ))))
    // 关闭内置外观：自行用紧凑容器控制面板尺寸，避免默认大卡片式留白。
    .appearance(false)
    .anchor(Anchor::TopRight)
    .trigger(trigger)
    // 内容：小尺寸两行菜单，选中项带 check、加粗、主文本色，hover 高亮衬托可点性。
    // `cx.entity()` 拿到 Popover 自身的 `PopoverState` 弱引用，交给菜单项点击后调用 `dismiss` 关闭下拉。
    .content(move |_, _, cx| {
        let popover = cx.entity().downgrade();
        div()
            .w(px(132.))
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border)
            .bg(colors.panel_bg)
            .shadow_md()
            .p_1()
            .flex()
            .flex_col()
            .gap_0p5()
            // `items` 为 Copy 数组，用 `iter().copied()` 按值传递而不消费，content 闭包（多次调用）可安全复用。
            .children(items.iter().copied().map(|item| make_item(item, popover.clone())))
    })
}

/// 把结构化回复渲染成可读文本。
fn redis_workbench_reply_row(reply: &CommandReply, colors: UiColors) -> Div {
    let (color, label) = match reply {
        CommandReply::Error(msg) => (rgb(0xff5c5c), msg.clone()),
        CommandReply::Nil => (colors.muted, "(nil)".to_string()),
        _ => (colors.text, command_reply_display(reply)),
    };
    div()
        .flex_1()
        .min_w(px(0.))
        .pl_5()
        .text_size(px(12.))
        .font_family("Menlo")
        .text_color(color)
        .whitespace_normal()
        .child(label)
}

/// CommandReply → redis-cli / RedisInsight 风格的展示文本。
///
/// 顶层回复按类型展开：数组递归出多行带序号的文本，整数显示 `(integer) N`、
/// nil 显示 `(nil)`、bulk 与状态直接展示原文本；映射仿 RedisInsight 把键值平铺为数组。
/// 数组元素的字符串一律加引号、嵌套数组按层级缩进，空数组显示 `(empty list or set)`。
/// 该函数同时作为结果区复制内容的唯一来源，保证「所见即所复」。
fn command_reply_display(reply: &CommandReply) -> String {
    match reply {
        CommandReply::Nil => "(nil)".to_string(),
        CommandReply::Status(s) => s.clone(),
        CommandReply::Integer(i) => format!("(integer) {i}"),
        CommandReply::Float(f) => f.to_string(),
        CommandReply::Bulk(bulk) => command_bulk_display(bulk),
        CommandReply::Array(items) => format_redis_array(items, 0),
        CommandReply::Map(pairs) => format_redis_array(&flatten_map(pairs), 0),
        CommandReply::Error(msg) => msg.clone(),
        CommandReply::Text(text) => text.clone(),
    }
}

/// 把键值映射平铺为「键、值、键、值…」列表（RedisInsight 对对象同理拍平）。
fn flatten_map(pairs: &[(CommandReply, CommandReply)]) -> Vec<CommandReply> {
    let mut flat = Vec::with_capacity(pairs.len() * 2);
    for (key, value) in pairs {
        flat.push(key.clone());
        flat.push(value.clone());
    }
    flat
}

/// 递归生成 redis-cli 风格多行数组文本。
///
/// - 空数组返回 `(empty list or set)`。
/// - 每个元素一行，带 `N)` 序号；`level` 为缩进层级（每层 3 空格），
///   仅非首个元素缩进，嵌套数组以 `level+1` 继续展开。
fn format_redis_array(items: &[CommandReply], level: usize) -> String {
    if items.is_empty() {
        return "(empty list or set)".to_string();
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let left_margin = if index > 0 { "   ".repeat(level) } else { String::new() };
            let value = format_redis_element(item, level + 1);
            format!("{left_margin}{}) {value}", index + 1)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 数组内单个元素的展示：嵌套数组 / 映射继续递归，其余按 redis-cli 语义带引号展开。
fn format_redis_element(reply: &CommandReply, level: usize) -> String {
    match reply {
        CommandReply::Nil => "(nil)".to_string(),
        CommandReply::Status(s) => format!("\"{s}\""),
        CommandReply::Integer(i) => format!("(integer) {i}"),
        CommandReply::Float(f) => format!("\"{f}\""),
        CommandReply::Bulk(bulk) => format!("\"{}\"", command_bulk_display(bulk)),
        CommandReply::Array(items) => format_redis_array(items, level),
        CommandReply::Map(pairs) => format_redis_array(&flatten_map(pairs), level),
        CommandReply::Error(msg) => format!("\"{msg}\""),
        CommandReply::Text(text) => format!("\"{text}\""),
    }
}

/// CommandBulk → 展示文本（优先 UTF-8，否则 hex 预览）。
fn command_bulk_display(bulk: &CommandBulk) -> String {
    if let Some(text) = bulk.text.as_ref() {
        text.clone()
    } else if let Some(hex) = bulk.bytes_preview_hex.as_ref() {
        format!("<{} bytes> {}", bulk.byte_len, hex)
    } else {
        format!("<{} bytes>", bulk.byte_len)
    }
}

fn query_output_panel(
    tab_id: TabId,
    editor: &QueryEditorState,
    placement: QueryOutputPlacement,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_output = !editor.results.is_empty() || !editor.summaries.is_empty();
    let has_error = editor.error.is_some();
    if !has_output && !editor.running && !has_error {
        return div();
    }

    if (has_output || has_error) && this.collapsed_query_outputs.contains(&tab_id) {
        return collapsed_query_output_button(tab_id, placement, colors, cx);
    }

    let selected = this
        .query_output_tabs
        .get(&tab_id)
        .copied()
        .filter(|tab| query_output_tab_exists(*tab, editor))
        .unwrap_or_else(|| {
            if query_result_entry_count(editor) > 0 {
                QueryOutputTab::Result(0)
            } else {
                QueryOutputTab::Summary
            }
        });
    let size = match placement {
        QueryOutputPlacement::Bottom => query_output_height(tab_id, this, window),
        QueryOutputPlacement::Right => query_output_width(tab_id, this, window),
    };
    let body = if let Some(error) = editor.error.as_ref() {
        let retry = Button::new(("retry-query-output", tab_id.0))
            .label("重试")
            .small()
            .outline()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.start_query_execution(tab_id, window, cx);
            }))
            .into_any_element();
        query_output_error_state(tab_id, error, Some(retry), cx, colors).into_any_element()
    } else if !has_output {
        query_output_empty_state(colors).into_any_element()
    } else {
        match selected {
            QueryOutputTab::Result(index) => query_result_view(
                tab_id,
                editor,
                index,
                this,
                window,
                colors,
                cx,
            )
            .into_any_element(),
            QueryOutputTab::Summary => {
                query_execution_summary_view(&editor.summaries, colors, cx).into_any_element()
            }
        }
    };

    let content = div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(query_output_tabs(tab_id, editor, selected, colors, cx))
        .child(
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(body)
                .when(editor.running, |this| this.child(query_output_loading_overlay(colors))),
        );
    let panel = div()
        .flex_none()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .flex();
    match placement {
        QueryOutputPlacement::Bottom => panel
            .h(px(size))
            .border_t_1()
            .flex_col()
            .child(query_output_resize_handle(tab_id, placement, size, colors, cx))
            .child(content),
        QueryOutputPlacement::Right => panel
            .w(px(size))
            .h_full()
            .border_l_1()
            .child(query_output_resize_handle(tab_id, placement, size, colors, cx))
            .child(content),
    }
}

fn query_output_loading_overlay(colors: UiColors) -> Div {
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .occlude()
        .bg(if colors.is_dark {
            hsla(216. / 360., 0.16, 0.10, 0.76)
        } else {
            hsla(210. / 360., 0.20, 0.98, 0.72)
        })
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(loading_spinner_with_color(18., colors.muted))
        .child("正在执行 SQL...")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
}

/// SQL 结果区可用高度估算：视口高度扣除顶部工具条。
fn query_split_height(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) - QUERY_TOOLBAR_HEIGHT).max(0.)
}

/// 结果区高度：默认占分栏 `RESULT_DEFAULT_SPLIT_RATIO`，
/// 再夹到 [split/6, 0.80·split]（min 相对编辑器 20%，max 占分栏 80%）。
fn query_output_height(tab_id: TabId, this: &NavicatMain, window: &Window) -> f32 {
    let split = query_split_height(window);
    let default_height = split * RESULT_DEFAULT_SPLIT_RATIO;
    let raw = this.query_output_heights.get(&tab_id).copied().unwrap_or(default_height);
    query_output_clamp_height(raw, split)
}
fn query_output_width(tab_id: TabId, this: &NavicatMain, window: &Window) -> f32 {
    let max_width = query_output_max_width(window);
    this.query_output_widths
        .get(&tab_id)
        .copied()
        .unwrap_or(QUERY_OUTPUT_DEFAULT_WIDTH)
        .clamp(QUERY_OUTPUT_MIN_WIDTH, max_width)
}

fn query_output_max_width(window: &Window) -> f32 {
    (f32::from(window.viewport_size().width) - SQL_EDITOR_MIN_WIDTH).max(QUERY_OUTPUT_MIN_WIDTH)
}

fn query_output_resize_handle(
    tab_id: TabId,
    placement: QueryOutputPlacement,
    size: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let handle = div()
        .id("query-output-resize-handle")
        .flex_none()
        .bg(colors.panel_bg)
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.query_output_resize_start = Some(QueryOutputResizeStart {
                    tab_id,
                    placement,
                    x: f32::from(event.position.x),
                    y: f32::from(event.position.y),
                    size,
                });
                cx.stop_propagation();
            }),
        )
        .on_drag(QueryOutputResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<QueryOutputResizeDrag>, window, cx| {
                let Some(start) = this.query_output_resize_start else {
                    return;
                };
                if start.tab_id != tab_id {
                    return;
                }
                match start.placement {
                    QueryOutputPlacement::Bottom => {
                        let delta = start.y - f32::from(event.event.position.y);
                        let split = query_split_height(window);
                        let height = query_output_clamp_height(start.size + delta, split);
                        this.query_output_heights.insert(tab_id, height);
                    }
                    QueryOutputPlacement::Right => {
                        let delta = start.x - f32::from(event.event.position.x);
                        let width = (start.size + delta)
                            .clamp(QUERY_OUTPUT_MIN_WIDTH, query_output_max_width(window));
                        this.query_output_widths.insert(tab_id, width);
                    }
                }
                cx.notify();
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                if this
                    .query_output_resize_start
                    .is_some_and(|start| start.tab_id == tab_id)
                {
                    this.query_output_resize_start = None;
                    cx.stop_propagation();
                }
            }),
        );
    match placement {
        QueryOutputPlacement::Bottom => handle.h(px(7.)).cursor_ns_resize(),
        QueryOutputPlacement::Right => handle.w(px(7.)).h_full().cursor_col_resize(),
    }
}

fn collapsed_query_output_button(
    tab_id: TabId,
    placement: QueryOutputPlacement,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let container = div().flex_none().relative();
    let button = div()
        .absolute()
        .right(px(18.))
        .bottom(px(18.))
        .h(px(42.))
        .px_5()
        .rounded_full()
        .cursor_pointer()
        .bg(if colors.is_dark { rgb(0x141414) } else { rgb(0xffffff) })
        .border_1()
        .border_color(colors.border)
        .shadow_lg()
        .flex()
        .items_center()
        .gap_2()
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon(AppIcon::ChevronUp, 16., colors.text))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("显示结果"),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.collapsed_query_outputs.remove(&tab_id);
                cx.notify();
                cx.stop_propagation();
            }),
        );
    match placement {
        QueryOutputPlacement::Bottom => container.h(px(0.)).child(button),
        QueryOutputPlacement::Right => container.w(px(0.)).h_full().child(button),
    }
}

fn query_output_tab_exists(tab: QueryOutputTab, editor: &QueryEditorState) -> bool {
    match tab {
        QueryOutputTab::Result(index) => index < query_result_entry_count(editor),
        QueryOutputTab::Summary => !editor.summaries.is_empty() || editor.error.is_some(),
    }
}

fn data_cell_edit_should_commit_before_query_output_tab_change(
    current: Option<DataCellEditState>,
    tab_id: TabId,
    selected: bool,
) -> bool {
    !selected && current.is_some_and(|editing| editing.tab_id == tab_id)
}

fn query_output_tabs(
    tab_id: TabId,
    editor: &QueryEditorState,
    selected: QueryOutputTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let refresh_sql = selected
        .result_index()
        .and_then(|index| query_result_sql(editor, index));
    let mut tabs = div()
        .h(px(34.))
        .flex_none()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .gap_2()
        .px_3();

    for index in 0..query_result_entry_count(editor) {
        tabs = tabs.child(query_output_tab_button(
            tab_id,
            QueryOutputTab::Result(index),
            format!("结果 {}", index + 1),
            None,
            selected == QueryOutputTab::Result(index),
            colors,
            cx,
        ));
    }

    tabs = tabs.child(query_output_tab_button(
        tab_id,
        QueryOutputTab::Summary,
        "执行摘要".to_string(),
        Some(AppIcon::List),
        selected == QueryOutputTab::Summary,
        colors,
        cx,
    ));

    tabs.child(div().flex_1())
        .when(
            selected
                .result_index()
                .and_then(|index| query_result_page_index(editor, index))
                .is_some_and(|page_index| editor.result_editors.contains_key(&page_index)),
            |this| this.child(query_result_editable_badge(colors)),
        )
        .child(query_output_toolbar_button(
            tab_id,
            "刷新",
            AppIcon::Refresh,
            colors,
            cx,
            move |this, tab_id, window, cx| {
                if let Some(sql) = refresh_sql.clone() {
                    this.start_query_text_execution(tab_id, sql, window, cx);
                } else {
                    this.start_query_execution(tab_id, window, cx);
                }
            },
        ))
    .child(query_output_toolbar_button(
        tab_id,
        "收起结果",
        AppIcon::ChevronDown,
        colors,
        cx,
        |this, tab_id, _, cx| {
            this.collapsed_query_outputs.insert(tab_id);
            cx.notify();
        },
    ))
}

fn query_result_editable_badge(colors: UiColors) -> Div {
    div()
        .h(px(26.))
        .px_2()
        .flex()
        .items_center()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if colors.is_dark {
            rgb(0x7ee787)
        } else {
            rgb(0x188038)
        })
        .child("可编辑")
}

fn query_output_tab_button(
    tab_id: TabId,
    tab: QueryOutputTab,
    label: String,
    icon: Option<AppIcon>,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(26.))
        .px_2()
        .rounded(colors.radius_lg)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .bg(if selected {
            if colors.is_dark {
                rgb(0x2a2e36)
            } else {
                rgb(0xffffff)
            }
        } else {
            colors.panel_bg
        })
        .text_color(if selected { colors.text } else { colors.muted })
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .when_some(icon, |this, icon| {
            this.child(app_icon(icon, 15., if selected { colors.text } else { colors.muted }))
        })
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _window, cx| {
                if selected {
                    cx.stop_propagation();
                    return;
                }
                if data_cell_edit_should_commit_before_query_output_tab_change(
                    this.data_cell_editing,
                    tab_id,
                    selected,
                ) && !this.commit_data_cell_edit_with_refresh(false, cx)
                {
                    cx.stop_propagation();
                    return;
                }
                this.query_output_tabs.insert(tab_id, tab);
                let page_index = tab.result_index().and_then(|index| {
                    this.controller
                        .state()
                        .tabs
                        .iter()
                        .find(|state_tab| state_tab.id == tab_id)
                        .and_then(|state_tab| match &state_tab.kind {
                            TabKind::QueryEditor(editor) => query_result_page_index(editor, index),
                            _ => None,
                        })
                });
                this.dispatch(
                    AppCommand::ActivateQueryResultEditor { tab_id, page_index },
                    cx,
                );
                if let Some(statement) = tab
                    .result_index()
                    .and_then(|index| query_result_statement_ordinal_for_tab(
                        this.controller.state(),
                        tab_id,
                        index,
                    ))
                    .and_then(|ordinal| {
                        // 新编辑器：依据全文切分语句并按 ordinal 取目标语句。
                        this.query_editors.get(&tab_id).and_then(|editor| {
                            let text = editor.read(cx).text();
                            sql_editor_adapter::build_statement_runs(&text)
                                .get(ordinal)
                                .cloned()
                        })
                    })
                    && let Some(sql_editor) = this.query_editors.get(&tab_id).cloned()
                {
                    sql_editor.update(cx, |editor, cx| {
                        editor.reveal_offset(statement.range.start, cx);
                    });
                }
                cx.notify();
                cx.stop_propagation();
            }),
        )
}

fn query_output_toolbar_button(
    tab_id: TabId,
    label: &'static str,
    icon: AppIcon,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
    action: impl Fn(&mut NavicatMain, TabId, &mut Window, &mut Context<NavicatMain>) + Clone + 'static,
) -> Div {
    div()
        .h(px(26.))
        .px_2()
        .rounded(colors.radius_lg)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .text_color(colors.muted)
        .hover(move |style| style.bg(colors.hover).text_color(colors.text))
        .child(app_icon(icon, 14., colors.muted))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                action(this, tab_id, window, cx);
                cx.stop_propagation();
            }),
        )
}

fn query_output_empty_state(colors: UiColors) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child("执行 SQL 后显示结果和摘要")
}

/// 查询 / Redis 输出面板的错误态：常驻 gpui-component `Alert`（不参与自动消失），
/// 顶部对齐放在面板里；`retry` 由调用方自备（面板自带运行按钮时可以不传）。
fn query_output_error_state(
    tab_id: TabId,
    error: &fluxdb_core::UserFacingError,
    retry: Option<gpui::AnyElement>,
    cx: &App,
    colors: UiColors,
) -> Div {
    div().flex_1().min_h(px(0.)).child(
        div().overflow_y_scrollbar().p_6().child(page_error_alert(
            gpui::ElementId::Name(format!("query-output-error-{}", tab_id.0).into()),
            &error.title,
            &error.message,
            error.detail.as_deref(),
            retry,
            cx,
            colors,
        )),
    )
}

#[derive(Clone, Copy)]
enum QueryResultEntry<'a> {
    Page {
        page: &'a DataPage,
        page_index: usize,
    },
    Failure {
        summary: &'a QueryExecutionSummary,
    },
}

fn query_result_summary_has_result_tab(summary: &QueryExecutionSummary) -> bool {
    summary.kind == fluxdb_core::QueryStatementKind::ResultSet || !summary.success
}

fn query_result_entry_count(editor: &QueryEditorState) -> usize {
    query_execution_result_entry_count(editor.results.len(), &editor.summaries)
}

fn query_execution_result_entry_count(
    result_count: usize,
    summaries: &[QueryExecutionSummary],
) -> usize {
    if summaries.is_empty() {
        return result_count;
    }

    summaries
        .iter()
        .filter(|summary| query_result_summary_has_result_tab(summary))
        .count()
        .max(result_count)
}

fn query_result_entry<'a>(
    editor: &'a QueryEditorState,
    result_index: usize,
) -> Option<QueryResultEntry<'a>> {
    if editor.summaries.is_empty() {
        return editor.results.get(result_index).map(|page| QueryResultEntry::Page {
            page,
            page_index: result_index,
        });
    }

    let mut entry_index = 0usize;
    let mut page_index = 0usize;
    for summary in &editor.summaries {
        if !query_result_summary_has_result_tab(summary) {
            continue;
        }

        if !summary.success {
            if entry_index == result_index {
                return Some(QueryResultEntry::Failure { summary });
            }
            entry_index += 1;
            continue;
        }

        let Some(page) = editor.results.get(page_index) else {
            continue;
        };
        if entry_index == result_index {
            return Some(QueryResultEntry::Page { page, page_index });
        }
        entry_index += 1;
        page_index += 1;
    }

    editor.results.get(page_index).map(|page| QueryResultEntry::Page {
        page,
        page_index,
    })
}

fn query_result_page_index(editor: &QueryEditorState, result_index: usize) -> Option<usize> {
    match query_result_entry(editor, result_index)? {
        QueryResultEntry::Page { page_index, .. } => Some(page_index),
        QueryResultEntry::Failure { .. } => None,
    }
}

fn query_result_summary_index(editor: &QueryEditorState, result_index: usize) -> Option<usize> {
    if editor.summaries.is_empty() {
        return Some(result_index);
    }

    let mut entry_index = 0usize;
    for (summary_index, summary) in editor.summaries.iter().enumerate() {
        if !query_result_summary_has_result_tab(summary) {
            continue;
        }
        if entry_index == result_index {
            return Some(summary_index);
        }
        entry_index += 1;
    }
    None
}

fn query_result_statement_ordinal_for_tab(
    state: &AppState,
    tab_id: TabId,
    result_index: usize,
) -> Option<usize> {
    state
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .and_then(|tab| match &tab.kind {
            TabKind::QueryEditor(editor) => query_result_summary_index(editor, result_index),
            _ => None,
        })
}

fn query_result_sql(editor: &QueryEditorState, result_index: usize) -> Option<String> {
    if editor.summaries.is_empty() {
        return None;
    }

    let mut entry_index = 0usize;
    for summary in &editor.summaries {
        if !query_result_summary_has_result_tab(summary) {
            continue;
        }

        if entry_index == result_index {
            return Some(summary.sql.clone());
        }
        entry_index += 1;
    }

    None
}

fn query_result_view(
    tab_id: TabId,
    editor: &QueryEditorState,
    result_index: usize,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(entry) = query_result_entry(editor, result_index) else {
        return query_output_empty_state(colors);
    };
    let (page, page_index) = match entry {
        QueryResultEntry::Page { page, page_index } => (page, page_index),
        QueryResultEntry::Failure { summary } => {
            return query_result_error_state(tab_id, result_index, summary, colors, cx);
        }
    };
    let result_editor = editor.result_editors.get(&page_index);
    let result_page = result_editor
        .and_then(|editor| editor.page.as_ref())
        .unwrap_or(page);
    let sort_rules = this
        .query_result_sort_rules
        .get(&QueryResultSortKey {
            tab_id,
            result_index,
        })
        .cloned()
        .unwrap_or_default();
    let sorted_page =
        this.query_result_display_page(tab_id, result_index, page_index, result_page, &sort_rules, false);
    let page = &sorted_page.page;
    let search_open = this.data_search_panels.contains(&tab_id);
    if search_open {
        let expected = this
            .data_search_queries
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        let input_focused = this
            .data_search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !input_focused && this.data_search_input.read(cx).value().to_string() != expected {
            this.data_search_input.update(cx, |input, cx| {
                input.set_value(expected, window, cx);
            });
        }
    }
    if page.rows.is_empty() {
        return div()
            .size_full()
            .child(
                div()
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .h(px(48.))
                            .flex()
                            .items_center()
                            .px_3()
                            .text_size(px(13.))
                            .text_color(colors.muted)
                            .child("没有返回数据"),
                    ),
            );
    }

    let readonly_detail = result_editor
        .is_none()
        .then(|| this.query_result_cell_detail.get(&tab_id).copied())
        .flatten();
    let editable_detail_open = result_editor.is_some_and(|editor| editor.cell_detail_panel.open);
    let detail_height = if readonly_detail
        .is_some_and(|detail| query_result_cell_detail_exists(result_page, detail))
        || editable_detail_open
    {
            this.cell_detail_drawer_heights
                .get(&tab_id)
                .copied()
                .map(clamp_cell_detail_drawer_height)
                .unwrap_or(CELL_DETAIL_DRAWER_DEFAULT_HEIGHT)
    } else {
        0.
    };
    let table_state = this.query_result_table_state_from_sorted_page(
        tab_id,
        &sorted_page.page,
        page_index,
        &sort_rules,
        &sorted_page.source_row_indexes,
        result_editor,
        window,
        cx,
    );
    let (search_matches, active_search_match) = {
        let table = table_state.read(cx);
        let delegate = table.delegate();
        (delegate.search_matches.clone(), delegate.active_search_match)
    };
    let search_highlight_all = this.data_search_highlight_all_tabs.contains(&tab_id);
    let change_count = result_editor
        .and_then(|editor| editor.changes.as_ref())
        .filter(|changes| !changes.is_empty())
        .map(data_change_item_count);
    let change_sql_preview = result_editor
        .and_then(|_| change_count)
        .and_then(|_| result_editor.and_then(|editor| editor.changes.as_ref()))
        .map(|changes| data_change_sql_preview(result_page, changes));
    let change_sql_preview_open = this.data_change_sql_preview_tabs.contains(&tab_id);
    let cell_detail_drawer = result_editor
        .filter(|editor| editor.cell_detail_panel.open)
        .map(|editor| {
            let input = this.cell_detail_input.clone();
            this.sync_cell_detail_input(&input, &editor.cell_detail_panel.edit_value, window, cx);
            cell_detail_panel(
                tab_id,
                editor,
                result_page,
                detail_height,
                input,
                this.temporal_part_input.clone(),
                this.temporal_part_editing,
                colors,
                window,
                cx,
            )
        });
    let search_height = if search_open { 30. } else { 0. };
    let change_footer_height = if change_count.is_some() { 30. } else { 0. };
    let sql_preview_height = if change_sql_preview_open && change_sql_preview.is_some() {
        160.
    } else {
        0.
    };
    let page_footer_height = if page.offset > 0 || page.has_more {
        36.
    } else {
        0.
    };
    let data_search_input = this.data_search_input.clone();
    let data_page_input = this.data_page_input.clone();
    let pagination_sql = query_result_sql(editor, result_index).unwrap_or_default();
    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom(px(
                    detail_height
                        + change_footer_height
                        + sql_preview_height
                        + page_footer_height
                        + search_height,
                ))
                .left_0()
                .overflow_hidden()
                .child(component_data_table(&table_state, cx)),
        )
        .when(search_open, |this| {
            this.child(
                data_search_bar(
                    tab_id,
                    data_search_input,
                    search_matches.as_slice(),
                    active_search_match,
                    search_highlight_all,
                    colors,
                    cx,
                )
                .absolute()
                .right_0()
                .bottom(px(
                    detail_height + change_footer_height + sql_preview_height + page_footer_height,
                ))
                .left_0(),
            )
        })
        .when_some(change_sql_preview.as_ref(), |this, preview| {
            this.when(change_sql_preview_open, |this| {
                this.child(
                    data_change_sql_preview_drawer(tab_id, preview, colors, cx)
                        .absolute()
                        .right_0()
                        .bottom(px(if detail_height > 0. { detail_height + 30. } else { 30. }))
                        .left_0()
                        .h(px(160.)),
                )
            })
        })
        .when_some(readonly_detail, |this, detail| {
            this.when_some(
                query_result_cell_detail_panel(
                    tab_id,
                    result_page,
                    detail,
                    detail_height,
                    colors,
                    cx,
                ),
                |this, panel| this.child(panel.absolute().right_0().bottom_0().left_0()),
            )
        })
        .when_some(cell_detail_drawer, |this, panel| {
            this.child(panel.absolute().right_0().bottom_0().left_0().h(px(detail_height)))
        })
        .when_some(change_count, |this, count| {
            this.child(
                query_result_change_footer(tab_id, count, change_sql_preview_open, colors, cx)
                    .absolute()
                    .right_0()
                    .bottom(px(detail_height))
                    .left_0(),
            )
        })
        .when(page_footer_height > 0., |this| {
            this.child(
                query_result_page_footer(
                    tab_id,
                    result_index,
                    page_index,
                    page,
                    pagination_sql,
                    data_page_input,
                    colors,
                    window,
                    cx,
                )
                .absolute()
                .right_0()
                .bottom(px(detail_height + change_footer_height + sql_preview_height))
                .left_0(),
            )
        })
}

fn query_result_page_footer(
    tab_id: TabId,
    result_index: usize,
    page_index: usize,
    page: &DataPage,
    sql: String,
    page_input: Entity<InputState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let page_size = data_page_limit(page.limit);
    let page_no = data_page_number(page.offset, page_size);
    let can_previous = page.offset > 0;
    let can_next = page.has_more;
    let previous_offset = page.offset.saturating_sub(page_size);
    let next_offset = page.offset.saturating_add(page_size);

    div()
        .h(px(36.))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .px_3()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .child(
            query_result_page_button(AppIcon::ChevronsLeft, "第一页", can_previous, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql = sql.clone();
                        move |this, _, window, cx| {
                            if can_previous {
                                this.sync_data_page_input_value(0, page_size, window, cx);
                                this.start_query_result_page_refresh(
                                    QueryResultRefreshRequest {
                                        tab_id,
                                        result_index,
                                        page_index,
                                        sql: sql.clone(),
                                        offset: 0,
                                        limit: page_size,
                                    },
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }
                    }),
                ),
        )
        .child(
            query_result_page_button(AppIcon::ChevronLeft, "上一页", can_previous, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql = sql.clone();
                        move |this, _, window, cx| {
                            if can_previous {
                                this.sync_data_page_input_value(
                                    previous_offset,
                                    page_size,
                                    window,
                                    cx,
                                );
                                this.start_query_result_page_refresh(
                                    QueryResultRefreshRequest {
                                        tab_id,
                                        result_index,
                                        page_index,
                                        sql: sql.clone(),
                                        offset: previous_offset,
                                        limit: page_size,
                                    },
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }
                    }),
                ),
        )
        .child(data_editor_page_input(
            page_no, page_input, colors, window, cx,
        ))
        .child(
            query_result_page_button(AppIcon::ChevronRight, "下一页", can_next, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql = sql.clone();
                        move |this, _, window, cx| {
                            if can_next {
                                this.sync_data_page_input_value(next_offset, page_size, window, cx);
                                this.start_query_result_page_refresh(
                                    QueryResultRefreshRequest {
                                        tab_id,
                                        result_index,
                                        page_index,
                                        sql: sql.clone(),
                                        offset: next_offset,
                                        limit: page_size,
                                    },
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }
                    }),
                ),
        )
        .child(
            query_result_page_button(AppIcon::ChevronsRight, "无法确定最后一页", false, colors)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
        )
}

fn query_result_page_button(
    icon: AppIcon,
    tooltip: &'static str,
    enabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(tooltip)
        .size(px(24.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .opacity(if enabled { 1.0 } else { 0.45 })
        .hover(move |style| {
            if enabled {
                style.bg(colors.hover)
            } else {
                style
            }
        })
        .when(enabled, |this| this.cursor_pointer())
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(app_icon(icon, 15., if enabled { colors.text } else { colors.muted }))
}

fn query_result_error_state(
    tab_id: TabId,
    result_index: usize,
    summary: &QueryExecutionSummary,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let error_color = rgb(0xff5c5c);
    let copy_text = query_result_error_copy_text(summary);
    let sql_line = single_line_summary_text(summary.sql.clone());

    // 操作区：重试（复用工具栏「运行」入口重跑整条查询）+ 复制错误 + 用 AI 修复（暂未开放）。
    let actions = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            Button::new(("retry-query-result", tab_id.0))
                .label("重试")
                .small()
                .outline()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start_query_execution(tab_id, window, cx);
                })),
        )
        .child(
            div()
                .id(gpui::ElementId::Name(
                    format!(
                        "copy-query-result-error-{}-{}",
                        tab_id.0, result_index
                    )
                    .into(),
                ))
                .h(px(30.))
                .px_3()
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .cursor_pointer()
                .flex()
                .items_center()
                .gap_2()
                .text_color(colors.text)
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Copy, 14., colors.muted))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("复制错误"),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        this.show_message("已复制错误信息", AppMessageKind::Success, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            div()
                .id(gpui::ElementId::Name(
                    format!(
                        "ai-fix-query-result-error-{}-{}",
                        tab_id.0, result_index
                    )
                    .into(),
                ))
                .h(px(30.))
                .px_3()
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .cursor_default()
                .opacity(0.5)
                .flex()
                .items_center()
                .gap_2()
                .text_color(error_color)
                .child(app_icon(AppIcon::Bot, 14., error_color))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("用 AI 修复"),
                )
                .tooltip(|window, cx| Tooltip::new("暂不支持 AI 修复").build(window, cx)),
        );

    div().flex_1().min_h(px(0.)).child(
        div().overflow_y_scrollbar().p_6().child(page_error_alert(
            gpui::ElementId::Name(
                format!("query-result-error-{}-{}", tab_id.0, result_index).into(),
            ),
            "查询出错",
            &summary.message,
            Some(sql_line.as_str()),
            Some(actions.into_any_element()),
            cx,
            colors,
        )),
    )
}

fn query_result_error_copy_text(summary: &QueryExecutionSummary) -> String {
    format!(
        "SQL:\n{}\n\n错误:\n{}",
        summary.sql.trim(),
        summary.message.trim()
    )
}

fn query_result_cell_detail_exists(page: &DataPage, detail: QueryResultCellDetail) -> bool {
    page.rows
        .get(detail.row)
        .and_then(|row| row.values.get(detail.column))
        .is_some()
        && page.columns.get(detail.column).is_some()
}

fn query_result_change_footer(
    tab_id: TabId,
    change_count: usize,
    change_sql_preview_open: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(30.))
        .w_full()
        .flex_none()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .pl_1()
        .pr_2()
        .flex()
        .items_center()
        .gap_1()
        .child(
            data_editor_footer_icon_button(AppIcon::Check, "提交更改", true, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.request_apply_data_changes(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            data_editor_footer_icon_button(AppIcon::Close, "取消更改", true, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(AppCommand::DiscardDataChanges(tab_id), cx);
                        this.data_change_sql_preview_tabs.remove(&tab_id);
                        this.refresh_active_data_table(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(data_editor_change_status(
            tab_id,
            change_count,
            change_sql_preview_open,
            colors,
            cx,
        ))
}

fn query_result_cell_detail_panel(
    tab_id: TabId,
    page: &DataPage,
    detail: QueryResultCellDetail,
    height: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Option<Div> {
    let column = page.columns.get(detail.column)?;
    let value = page.rows.get(detail.row)?.values.get(detail.column)?.clone();
    let value_text = cell_value_label(&value);
    let type_name = column
        .type_name
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let type_color = data_type_color(&type_name).unwrap_or(colors.text);
    let drawer_bg = if colors.is_dark {
        rgb(0x171b21)
    } else {
        rgb(0xfbfcfe)
    };
    let card_bg = if colors.is_dark {
        rgb(0x20252d)
    } else {
        rgb(0xffffff)
    };
    let soft_border = if colors.is_dark {
        rgb(0x2f3642)
    } else {
        colors.border_soft
    };

    Some(
        div()
            .h(px(height))
            .w_full()
            .flex_none()
            .border_t_1()
            .border_color(soft_border)
            .bg(drawer_bg)
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(cell_detail_drawer_resize_handle(
                tab_id,
                height,
                colors,
                cx,
            ))
            .child(
                div()
                    .h(px(54.))
                    .w_full()
                    .rounded(colors.radius * 0.5)
                    .border_1()
                    .border_color(soft_border)
                    .bg(card_bg)
                    .flex()
                    .overflow_hidden()
                    .child(cell_detail_meta_item("列名", column.name.clone(), colors))
                    .child(cell_detail_meta_item(
                        "行号",
                        (detail.row + 1).to_string(),
                        colors,
                    ))
                    .child(cell_detail_meta_item_colored(
                        "类型", type_name, type_color, colors,
                    ))
                    .child(cell_detail_meta_item(
                        "NULL",
                        matches!(value, CellValue::Null).to_string(),
                        colors,
                    ))
                    .child(cell_detail_meta_item("长度", cell_value_length(&value).to_string(), colors))
                    .child(
                        div()
                            .w(px(42.))
                            .h_full()
                            .flex_none()
                            .flex()
                            .items_start()
                            .justify_center()
                            .pt_2()
                            .child(
                                cell_detail_icon_button(AppIcon::Close, "关闭", colors)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.query_result_cell_detail.remove(&tab_id);
                                            cx.notify();
                                            cx.stop_propagation();
                                        }),
                                    ),
                            ),
                    ),
            )
            .child(query_result_cell_detail_value_header(
                value_text, colors,
            ))
            .child(cell_detail_value_preview(tab_id, &value, colors, cx)),
    )
}

fn query_result_cell_detail_value_header(value_text: String, colors: UiColors) -> Div {
    div()
        .h(px(22.))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child("值"),
        )
        .child(cell_detail_icon_button(AppIcon::Copy, "复制", colors).on_mouse_down(
            MouseButton::Left,
            move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(value_text.clone()));
                cx.stop_propagation();
            },
        ))
}

#[derive(Clone)]
struct SortedQueryResultPage {
    page: DataPage,
    source_row_indexes: Vec<usize>,
}

fn sorted_query_result_page(page: &DataPage, rules: &[DataSortRule]) -> SortedQueryResultPage {
    let active = rules
        .iter()
        .filter(|rule| rule.enabled)
        .filter_map(|rule| {
            page.columns
                .iter()
                .position(|column| column.name == rule.field)
                .map(|index| (index, rule.ascending))
        })
        .collect::<Vec<_>>();
    if active.is_empty() {
        return SortedQueryResultPage {
            page: page.clone(),
            source_row_indexes: (0..page.rows.len()).collect(),
        };
    }

    let mut rows = page
        .rows
        .iter()
        .cloned()
        .enumerate()
        .collect::<Vec<_>>();
    rows.sort_by(|(_, left), (_, right)| {
        for (index, ascending) in &active {
            let ordering =
                compare_query_result_cells(left.values.get(*index), right.values.get(*index));
            if ordering != std::cmp::Ordering::Equal {
                return if *ascending { ordering } else { ordering.reverse() };
            }
        }
        std::cmp::Ordering::Equal
    });
    let (source_row_indexes, rows): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    SortedQueryResultPage {
        page: DataPage {
            rows,
            ..page.clone()
        },
        source_row_indexes,
    }
}

fn compare_query_result_cells(
    left: Option<&CellValue>,
    right: Option<&CellValue>,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(CellValue::Null), Some(CellValue::Null)) => std::cmp::Ordering::Equal,
        (Some(CellValue::Null), _) => std::cmp::Ordering::Less,
        (_, Some(CellValue::Null)) => std::cmp::Ordering::Greater,
        (Some(CellValue::I64(left)), Some(CellValue::I64(right))) => left.cmp(right),
        (Some(CellValue::F64(left)), Some(CellValue::F64(right))) => {
            left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Some(left), Some(right)) => left.display_label().cmp(&right.display_label()),
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
    }
}

fn query_execution_summary_view(
    summaries: &[QueryExecutionSummary],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut table = div().min_w(px(900.)).flex().flex_col();
    table = table.child(query_summary_header(colors));
    for (row_index, summary) in summaries.iter().enumerate() {
        table = table.child(query_summary_row(row_index, summary, colors, cx));
    }
    if summaries.is_empty() {
        table = table.child(
            div()
                .h(px(48.))
                .flex()
                .items_center()
                .px_3()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("暂无执行摘要"),
        );
    }
    div()
        .size_full()
        .overflow_hidden()
        .child(div().size_full().overflow_scrollbar().child(table))
}

fn query_summary_header(colors: UiColors) -> Div {
    div()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .child(query_summary_cell("语句", 1., true, colors))
        .child(query_summary_cell("类型", 0.42, true, colors))
        .child(query_summary_cell("返回行", 0.22, true, colors))
        .child(query_summary_cell("影响行", 0.22, true, colors))
        .child(query_summary_cell("耗时", 0.2, true, colors))
        .child(div().w(px(42.)).flex_none())
}

fn query_summary_row(
    row_index: usize,
    summary: &QueryExecutionSummary,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql = summary.sql.clone();
    div()
        .h(px(40.))
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(colors.border_soft)
        .text_color(colors.text)
        .child(query_summary_cell(summary.sql.clone(), 1., false, colors))
        .child(query_summary_type_cell(summary, colors))
        .child(query_summary_cell(summary.returned_rows.to_string(), 0.22, false, colors))
        .child(query_summary_cell(summary.affected_rows.to_string(), 0.22, false, colors))
        .child(query_summary_cell(format!("{}ms", summary.elapsed_ms), 0.2, false, colors))
        .child(
            div()
                .w(px(42.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id(("copy-query-summary-sql", row_index))
                        .size(px(26.))
                        .rounded(colors.radius)
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |style| style.bg(colors.hover))
                        .tooltip(|window, cx| Tooltip::new("复制 SQL").build(window, cx))
                        .child(app_icon(AppIcon::Copy, 14., colors.muted))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()));
                                this.show_message("已复制 SQL", AppMessageKind::Success, cx);
                                cx.stop_propagation();
                            }),
                        ),
                ),
        )
}

fn query_summary_type_cell(summary: &QueryExecutionSummary, colors: UiColors) -> Div {
    let badge_bg = if summary.success {
        if colors.is_dark {
            rgb(0x053f31)
        } else {
            rgb(0xdff8ec)
        }
    } else if colors.is_dark {
        rgb(0x552126)
    } else {
        rgb(0xffe4e6)
    };
    let badge_text = if summary.success {
        rgb(0x20c76a)
    } else {
        rgb(0xff5c5c)
    };
    div()
        .flex_1()
        .flex_basis(px(0.))
        .min_w(px(0.))
        .h_full()
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .px_2()
                .h(px(20.))
                .rounded_full()
                .bg(badge_bg)
                .flex()
                .items_center()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(badge_text)
                .child(if summary.success { "成功" } else { "失败" }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(colors.text)
                .child(summary.message.clone()),
        )
}

fn query_summary_cell(text: impl Into<String>, flex: f32, header: bool, colors: UiColors) -> Div {
    let text = single_line_summary_text(text.into());

    div()
        .flex_1()
        .flex_basis(px(0.))
        .min_w(px(0.))
        .h_full()
        .px_2()
        .flex()
        .items_center()
        .overflow_hidden()
        .child(
            div()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .font_weight(if header {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if header { colors.muted } else { colors.text })
                .child(text),
        )
        .when(flex < 0.5, |this| this.flex_none().w(px(110. * flex.max(0.2))))
}

fn single_line_summary_text(text: String) -> String {
    if !text.contains('\n') && !text.contains('\r') {
        return text;
    }

    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn query_toolbar(
    this: &mut NavicatMain,
    tab_id: TabId,
    editor: &QueryEditorState,
    sql_editor: Entity<editor_component::Editor>,
    connection: String,
    database: String,
    accent: gpui::Rgba,
    output_placement: QueryOutputPlacement,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_text = !editor.text.trim().is_empty();
    let soft_wrap = sql_editor.read(cx).soft_wrap();
    let can_explain = !editor.running && this.selected_explain_sql_text(tab_id, cx).is_some();
    div()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .bg(colors.panel_alt)
        .child(query_context_box(connection, accent, colors))
        .child(query_context_box(database, accent, colors))
        .child(
            query_toolbar_icon_button(
                if editor.running { "执行中" } else { "运行" },
                AppIcon::Play,
                has_text && !editor.running,
                editor.running,
                rgb(0x20c76a),
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql_editor = sql_editor.clone();
                        move |this, _, window, cx| {
                            let text = sql_editor.read(cx).text();
                            if !text.trim().is_empty() {
                                this.dispatch(AppCommand::UpdateQueryText { tab_id, text }, cx);
                                this.start_query_execution(tab_id, window, cx);
                                cx.stop_propagation();
                            }
                        }
                    }),
                ),
        )
        .child(
            query_toolbar_icon_button(
                "美化 SQL",
                AppIcon::AlignLeft,
                has_text,
                false,
                rgb(0xa45cff),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let sql_editor = sql_editor.clone();
                    move |this, _, _, cx| {
                        if has_text && this.format_query_editor_sql(tab_id, sql_editor.clone(), cx)
                        {
                            cx.stop_propagation();
                        }
                    }
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                "压缩 SQL",
                AppIcon::List,
                has_text,
                false,
                rgb(0x38bdf8),
                colors,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let sql_editor = sql_editor.clone();
                        move |this, _, _, cx| {
                            if has_text
                                && this.compress_query_editor_sql(tab_id, sql_editor.clone(), cx)
                            {
                                cx.stop_propagation();
                            }
                        }
                    }),
                ),
        )
        .child(query_toolbar_icon_button(
            "停止",
            AppIcon::Square,
            editor.running,
            false,
            rgb(0xff5c5c),
            colors,
        ))
        .child(query_toolbar_icon_button(
            "解释",
            AppIcon::FileSearch,
            can_explain,
            false,
            rgb(0x45a3ff),
            colors,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener({
                let sql_editor = sql_editor.clone();
                move |this, _, window, cx| {
                    if this.start_query_explain_execution(tab_id, sql_editor.clone(), window, cx) {
                        cx.stop_propagation();
                    }
                }
            }),
        ))
        .child(div().flex_1())
        .child(query_toolbar_icon_button(
            "保存",
            AppIcon::Save,
            true,
            false,
            rgb(0x4b8dff),
            colors,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.save_active_query(window, cx);
                cx.stop_propagation();
            }),
        ))
        .child(query_toolbar_icon_button(
            "查询创建工具",
            AppIcon::Workflow,
            false,
            false,
            rgb(0xf0b400),
            colors,
        ))
        .child(query_toolbar_icon_button(
            "代码段",
            AppIcon::FileSql,
            false,
            false,
            rgb(0x28c7d7),
            colors,
        ))
        .child(
            query_toolbar_icon_button(
                "历史记录",
                AppIcon::CalendarClock,
                true,
                false,
                rgb(0xf59e0b),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.show_query_history_quick_search(window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                query_editor_wrap_toggle_label(soft_wrap),
                query_editor_wrap_toggle_icon(soft_wrap),
                true,
                false,
                rgb(0x5f8ff7),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let sql_editor = sql_editor.clone();
                    move |this, _, _, cx| {
                        let mut settings = this.controller.state().settings.clone();
                        settings.editor_word_wrap = !soft_wrap;
                        save_settings_from_ui(this, settings, "已更新自动换行", cx);
                        let line_height = this.controller.state().settings.editor_line_height.clamp(13, 28) as f32;
                        sql_editor.update(cx, |editor, _editor_cx| {
                            editor.apply_settings(editor.font_size, line_height, !soft_wrap, 0);
                        });
                        cx.stop_propagation();
                    }
                }),
            ),
        )
        .child(
            query_toolbar_icon_button(
                query_output_layout_toggle_label(output_placement),
                query_output_layout_toggle_icon(output_placement),
                true,
                false,
                rgb(0x45a3ff),
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.query_output_placement = this.query_output_placement.toggled();
                    this.query_output_resize_start = None;
                    cx.notify();
                    cx.stop_propagation();
                }),
            ),
        )
        .child(query_toolbar_icon_button(
            "询问 AI",
            AppIcon::Bot,
            true,
            false,
            rgb(0xb26cff),
            colors,
        ))
}

fn query_editor_wrap_toggle_icon(soft_wrap: bool) -> AppIcon {
    if soft_wrap {
        AppIcon::Text
    } else {
        AppIcon::WrapText
    }
}

fn query_editor_wrap_toggle_label(soft_wrap: bool) -> &'static str {
    if soft_wrap {
        "不换行"
    } else {
        "自动换行"
    }
}

fn query_output_layout_toggle_icon(placement: QueryOutputPlacement) -> AppIcon {
    match placement {
        QueryOutputPlacement::Bottom => AppIcon::PanelBottom,
        QueryOutputPlacement::Right => AppIcon::PanelRight,
    }
}

fn query_output_layout_toggle_label(placement: QueryOutputPlacement) -> &'static str {
    match placement {
        QueryOutputPlacement::Bottom => "结果在下方",
        QueryOutputPlacement::Right => "结果在右侧",
    }
}

fn query_toolbar_icon_button(
    label: &'static str,
    icon: AppIcon,
    enabled: bool,
    loading: bool,
    accent: gpui::Rgba,
    colors: UiColors,
) -> Stateful<Div> {
    let bg = colors.panel_bg;
    let icon_color = if enabled { accent } else { colors.muted };
    div()
        .id(label)
        .size(px(30.))
        .rounded(colors.radius)
        .border_1()
        .border_color(if enabled {
            if colors.is_dark {
                rgb(0x3a414d)
            } else {
                rgb(0xd7dde8)
            }
        } else {
            colors.border
        })
        .bg(if enabled { bg } else { colors.panel_alt })
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| {
                style.bg(if colors.is_dark {
                    rgb(0x2a3039)
                } else {
                    rgb(0xf4f7fb)
                })
            })
        })
        .child(if loading {
            loading_spinner_with_color(17., icon_color).into_any_element()
        } else {
            app_icon_box(icon, 20., 17., icon_color).into_any_element()
        })
}

fn query_context_box(label: String, accent: gpui::Rgba, colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .min_w(px(118.))
        .max_w(px(220.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .overflow_hidden()
        .child(div().w(px(4.)).h_full().bg(accent))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .px_2()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .text_color(colors.text)
                .child(label),
        )
}

fn object_toolbar(colors: UiColors) -> impl IntoElement {
    div()
        .h(px(36.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(17.))
                .text_color(rgb(0x1687ff))
                .child(app_icon_box(AppIcon::Square, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Edit, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Plus, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Minus, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Undo, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Redo, 22., 17., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Refresh, 22., 17., rgb(0x1687ff))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon_box(AppIcon::List, 22., 18., rgb(0x1687ff)))
                .child(app_icon_box(AppIcon::Table, 22., 18., colors.muted))
                .child(
                    div()
                        .w(px(180.))
                        .h(px(28.))
                        .rounded(colors.radius_lg)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(app_icon(AppIcon::Search, 13., colors.muted))
                        .child("搜索"),
                ),
        )
}

/// 展示模式双态 segmented control（平铺 / Folder）：位于左侧列表顶部状态栏最左侧，纯图标无文字。
/// 未选中段使用普通底 + hover；选中段填充高亮色（亮/暗主题均取 UiColors 适配色）。
/// 展示仅用 lucide 风格语义图标：`AppIcon::List`（平铺）/ `AppIcon::Folder`（Folder），不出现文字。
fn redis_key_mode_segmented(
    tab_id: TabId,
    mode: RedisKeyListMode,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(("redis-key-mode-switch", tab_id.0))
        .h(px(32.))
        .w(px(104.))
        .flex_none()
        .p(px(3.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .gap(px(3.))
        .child(
            redis_key_mode_segment(
                tab_id,
                "flat",
                RedisKeyListMode::Flat,
                AppIcon::List,
                mode,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_redis_key_list_mode(tab_id, RedisKeyListMode::Flat, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            redis_key_mode_segment(
                tab_id,
                "folder",
                RedisKeyListMode::Folder,
                AppIcon::Folder,
                mode,
                colors,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_redis_key_list_mode(tab_id, RedisKeyListMode::Folder, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn redis_key_mode_segment(
    tab_id: TabId,
    tag: &'static str,
    seg_mode: RedisKeyListMode,
    icon: AppIcon,
    current: RedisKeyListMode,
    colors: UiColors,
) -> Stateful<Div> {
    let active = current == seg_mode;
    // 选中段高亮：亮暗主题共用品牌蓝，白色图标保证可读性。
    let accent: Hsla = rgb(0x0c5fd5).into();
    div()
        .id(SharedString::from(format!(
            "redis-key-mode-seg-{}-{}",
            tab_id.0, tag
        )))
        .flex_1()
        .h_full()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(active, |this| this.bg(accent))
        .when(!active, |this| this.hover(move |style| style.bg(colors.hover)))
        .child(app_icon(
            icon,
            14.,
            if active { rgb(0xffffff) } else { colors.muted },
        ))
}

fn redis_key_search_bar(
    tab_id: TabId,
    type_select: Entity<SelectState<SearchableVec<String>>>,
    input: Entity<InputState>,
    active: bool,
    refreshing: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 搜索栏（`relative`，高 40px）作为顶层历史下拉重建前缀的参照：历史下拉不再作为本节点的子节点
    // 渲染，而是移到 `redis_split_view` 之后（见 render 根）以保证绘制在表格之上。
    // 展示模式双态切换、结果数 / 已扫描 / Scan more 已迁移到左侧列表顶部状态栏（`redis_key_list_status_bar`）。
    div()
        .relative()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .child(redis_search_type_select_box(type_select))
        .child(redis_search_input_frame(tab_id, input, colors, cx))
        .when(active, |this| {
            this.child(
                div()
                    .ml_4()
                    .text_size(px(13.))
                    .text_color(colors.text)
                    .child("上次搜索: 现在"),
            )
        })
        .child(div().flex_1().min_w(px(0.)))
        .child(redis_refresh_key_list_button(refreshing, colors).when(!refreshing, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.request_data_editor_refresh(tab_id, cx);
                    cx.stop_propagation();
                }),
            )
        }))
        .child(redis_add_key_button(colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.open_redis_add_key_drawer(tab_id, window, cx);
                cx.stop_propagation();
            }),
        ))
}

fn redis_search_type_select_box(
    select: Entity<SelectState<SearchableVec<String>>>,
) -> Div {
    div()
        .h(px(34.))
        .w(px(142.))
        .flex_none()
        .child(
            Select::new(&select)
                .small()
                .title_prefix("类型: ")
                .w_full()
                .h_full()
                .menu_width(px(132.)),
        )
}

fn redis_search_input_frame(
    tab_id: TabId,
    input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_text = !input.read(cx).value().is_empty();
    // 输入框宽度规则（`Fraction(0.5)`/`min_w(360)`）被顶层历史下拉锚点复用，
    // 故本处不受影响、可正常保持 `relative`；历史下拉实际渲染逻辑见 `redis_key_search_history_overlay`。
    // 不能加 `overflow_hidden`，否则绝对定位的下拉会被裁剪在输入框高度内无法展开；
    // 输入框圆角裁剪由内层 `rounded_lg` + 各子元素自带裁剪保证。
    div()
        .relative()
        .h(px(34.))
        .w(gpui::DefiniteLength::Fraction(0.5))
        .min_w(px(360.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .flex_1()
                .min_w(px(0.))
                .w_full()
                .h_full()
                .px_2()
                .text_size(px(13.)),
        )
        .when(has_text, |this| {
            this.child(redis_search_icon_button(AppIcon::Close, "清除", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.clear_redis_search(tab_id, window, cx);
                    cx.stop_propagation();
                }),
            ))
        })
        .child(
            redis_search_icon_button(AppIcon::ArrowUpDown, "搜索历史", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_redis_key_search_history(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(redis_search_icon_button(AppIcon::Search, "搜索", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.apply_redis_search(tab_id, cx);
                cx.stop_propagation();
            }),
        ))
}

fn redis_refresh_key_list_button(refreshing: bool, colors: UiColors) -> Stateful<Div> {
    div()
        .id("redis-refresh-keys")
        .h(px(34.))
        .px_3()
        .flex_none()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .gap_2()
        .when(!refreshing, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .tooltip(|window, cx| Tooltip::new("刷新").build(window, cx))
        })
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if refreshing { colors.muted } else { colors.text })
        .child(if refreshing {
            loading_spinner_with_color(14., colors.muted).into_any_element()
        } else {
            app_icon(AppIcon::Refresh, 14., colors.muted).into_any_element()
        })
        .child("刷新")
}

fn redis_add_key_button(colors: UiColors) -> Stateful<Div> {
    div()
        .id("redis-add-key")
        .h(px(34.))
        .px_3()
        .flex_none()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .tooltip(|window, cx| Tooltip::new("新增键").build(window, cx))
        .child(app_icon(AppIcon::Plus, 14., rgb(0x1687ff)))
        .child("新增键")
}

fn redis_search_icon_button(
    icon: AppIcon,
    tooltip: &'static str,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(tooltip)
        .size(px(26.))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(app_icon(icon, 14., colors.muted))
}

/// 搜索历史下拉的顶层锚点：挂到 `redis_split_view` 之后渲染，保证下拉浮于表格之上
/// （gpui 无 z-index，兄弟节点按绘制顺序叠放，后置者在上）。
/// 这里重建搜索栏前缀（px_2 + 类型选择 142 + gap + 输入框 `Fraction(0.5)`/`min_w(360)`），
/// 复用 taffy 对输入框宽度的精确计算，使下拉左边缘对齐输入框左边缘、宽度严格等于输入框。
/// 该锚点自身与搜索栏区域重叠但不含任何鼠标监听，故不会拦截下方搜索栏的交互。
fn redis_key_search_history_overlay(
    tab_id: TabId,
    history: Vec<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 仅占搜索栏高度一行；下拉为绝对定位，会从该行下方展开，故此处无需 `overflow_hidden`。
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(40.))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        // 类型选择框占位，保持与 `redis_key_search_bar` 一致的横向偏移
        // （展示模式 segmented control 已迁到左侧列表顶部状态栏，不再占用搜索栏宽度）。
        .child(div().w(px(142.)).flex_none())
        .child(
            // 输入框宽度锚点：与 `redis_search_input_frame` 相同的宽度规则。
            div()
                .relative()
                .h(px(34.))
                .w(gpui::DefiniteLength::Fraction(0.5))
                .min_w(px(360.))
                .child(redis_key_search_history_dropdown(tab_id, history, colors, cx)),
        )
}

/// Key 搜索历史下拉菜单：锚定在输入框（`relative`）正下方，左边缘对齐输入框左边缘并撑满输入框宽度，
/// 对齐 RedisInsight 中下拉归属于搜索组件的做法。历史词点击即回填并应用，「清除历史」按当前连接 + DB 清空。
/// 外部点击关闭由 `redis_key_search_history_occlude` 全屏遮罩层负责，故菜单自身拦截点击避免穿透。
fn redis_key_search_history_dropdown(
    tab_id: TabId,
    history: Vec<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let menu = if history.is_empty() {
        // 空历史：仅展示空态提示，不渲染清除按钮。
        div()
            .h(px(36.))
            .px_3()
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(colors.muted)
            .child("暂无搜索历史")
    } else {
        let mut list = div().flex().flex_col();
        for text in history.iter() {
            let item_text = text.clone();
            list = list.child(
                div()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .text_size(px(13.))
                    .text_color(colors.text)
                    .child(app_icon(AppIcon::Search, 14., colors.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(item_text.clone()),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.apply_redis_key_search_history(
                                tab_id,
                                item_text.clone(),
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        div().max_h(px(280.)).child(list.overflow_y_scrollbar())
    };

    let dropdown = div()
        .absolute()
        .top(px(38.))
        // 左边缘对齐搜索输入框左边缘；宽度取输入框实际宽度的 100%（`w_full` = 定位包含块即输入框的宽度），
        // 使下拉紧贴输入框正下方、宽度严格等于输入框；不使用 `right` 侧锚定，避免跑到右侧空白区域。
        .left(px(0.))
        .w_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow_lg()
        .flex()
        .flex_col()
        .overflow_hidden()
        // 标题行
        .child(
            div()
                .h(px(32.))
                .px_3()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(colors.border_soft)
                .bg(colors.panel_alt)
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("搜索历史"),
        )
        .child(menu)
        .when(!history.is_empty(), |this| {
            // 清除历史：按当前连接 + DB 清空
            this.child(
                div()
                    .h(px(32.))
                    .border_t_1()
                    .border_color(colors.border_soft)
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .text_size(px(13.))
                    .text_color(rgb(0xe5484d))
                    .child(app_icon(AppIcon::Trash, 14., rgb(0xe5484d)))
                    .child("清除历史")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.clear_redis_key_search_history(tab_id);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
        });

    // 菜单自身拦截滚轮/移动/左键，避免事件穿透到下层全屏遮罩或表格。
    dropdown
        .on_mouse_move(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
}

/// 全屏遮罩层：捕捉搜索历史下拉之外的外部点击用于关闭，避免与菜单点击冲突。
fn redis_key_search_history_occlude(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    let _ = colors;
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .occlude()
        .on_mouse_move(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_redis_key_search_history(cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
}

fn data_editor_toolbar(
    tab_id: TabId,
    filter_open: bool,
    search_open: bool,
    field_filter_open: bool,
    field_count: Option<(usize, usize)>,
    table_info_open: Option<bool>,
    show_table_tools: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .when(show_table_tools, |this| this.child(
                    data_editor_action("筛选 & 排序", colors)
                        .when(filter_open, |this| {
                            this.border_color(rgb(0x7db4ff)).bg(if colors.is_dark {
                                rgb(0x16345f)
                            } else {
                                rgb(0xe8f2ff)
                            })
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.toggle_data_filter_panel(tab_id, window, cx);
                                cx.stop_propagation();
                            }),
                        ),
                ))
                .when(show_table_tools, |this| this.child(
                    data_editor_action_state("搜索", search_open, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.toggle_data_search_panel(tab_id, window, cx);
                            cx.stop_propagation();
                        }),
                    ),
                ))
                .when(show_table_tools, |this| this.child(data_editor_action("数据分析", colors)))
                .when(show_table_tools, |this| this.child(data_editor_action("导入", colors)))
                .when(show_table_tools, |this| this.child(data_editor_action("导出", colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.open_table_data_export(tab_id, cx);
                        cx.stop_propagation();
                    }),
                )))
                .when(show_table_tools, |this| this.child(data_editor_action("数据生成", colors))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .when_some(field_count, |this, (visible, total)| {
                    this.child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.muted)
                            .child(if visible == total {
                                format!("{total} 字段")
                            } else {
                                format!("{visible}/{total} 字段")
                        }),
                    )
                })
                .when(show_table_tools, |this| this.child(
                    data_editor_action("字段筛选", colors)
                        .when(field_filter_open, |this| {
                            this.border_color(rgb(0x7db4ff)).bg(if colors.is_dark {
                                rgb(0x16345f)
                            } else {
                                rgb(0xe8f2ff)
                            })
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_field_filter_popover(tab_id, cx);
                                cx.stop_propagation();
                            }),
                        ),
                ))
                .when(show_table_tools, |this| this.when_some(table_info_open, |this, active| {
                    this.child(
                        data_editor_action_state("表属性", active, colors).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.dispatch(
                                    AppCommand::ToggleTableInfo {
                                        tab_id,
                                        tab: TableInfoTab::Columns,
                                    },
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ),
                    )
                })),
        )
}

fn data_editor_action(label: &'static str, colors: UiColors) -> Div {
    div()
        .h(px(28.))
        .px_3()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(px(13.))
        .text_color(colors.text)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn data_editor_action_state(label: &'static str, active: bool, colors: UiColors) -> Div {
    data_editor_action(label, colors).when(active, |this| {
        this.border_color(rgb(0x7db4ff)).bg(if colors.is_dark {
            rgb(0x16345f)
        } else {
            rgb(0xe8f2ff)
        })
    })
}

#[cfg(test)]
mod redis_workbench_reply_tests {
    use super::*;

    /// 构造一个 UTF-8 文本的 bulk 回复。
    fn bulk(text: &str) -> CommandReply {
        CommandReply::Bulk(CommandBulk {
            text: Some(text.to_string()),
            bytes_preview_hex: None,
            byte_len: text.len() as u64,
            binary: false,
        })
    }

    #[test]
    fn array_lists_each_element_on_own_line_with_index() {
        let reply = CommandReply::Array(vec![bulk("333"), bulk("332")]);
        assert_eq!(command_reply_display(&reply), "1) \"333\"\n2) \"332\"");
    }

    #[test]
    fn empty_array_shows_empty_list_or_set() {
        let reply = CommandReply::Array(vec![]);
        assert_eq!(command_reply_display(&reply), "(empty list or set)");
    }

    #[test]
    fn nil_shows_nil() {
        assert_eq!(command_reply_display(&CommandReply::Nil), "(nil)");
    }

    #[test]
    fn integer_shows_integer_prefix() {
        assert_eq!(command_reply_display(&CommandReply::Integer(42)), "(integer) 42");
    }

    #[test]
    fn nested_array_indents_by_level() {
        let reply = CommandReply::Array(vec![
            CommandReply::Array(vec![bulk("a"), bulk("b")]),
            bulk("c"),
        ]);
        assert_eq!(
            command_reply_display(&reply),
            "1) 1) \"a\"\n   2) \"b\"\n2) \"c\""
        );
    }
}
