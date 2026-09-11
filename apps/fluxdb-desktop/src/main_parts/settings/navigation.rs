fn settings_content(
    state: &AppState,
    theme_mode: ThemeMode,
    active_section: SettingsPanelSection,
    colors: UiColors,
    editor_draft: Settings,
    font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    radius_input: Entity<InputState>,
    dangerous_actions_collapsed: bool,
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
                                    dangerous_actions_collapsed,
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
    dangerous_actions_collapsed: bool,
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
                dangerous_actions_collapsed,
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

