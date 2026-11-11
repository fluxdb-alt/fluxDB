fn settings_content(
    state: &AppState,
    update: &UpdateUiState,
    ai_settings_panel: AiSettingsPanelState,
    theme_mode: ThemeMode,
    active_section: SettingsPanelSection,
    colors: UiColors,
    editor_draft: Settings,
    custom_themes: &BTreeSet<String>,
    font_size_slider: Entity<SliderState>,
    ui_font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    ui_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    code_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    clients: [NativeClientPanelState; 2],
    dangerous_actions_collapsed: bool,
    data_groups: SettingsDataGroupsCollapsed,
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
                            ui_font_size_slider.clone(),
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
                                    update,
                                    ai_settings_panel,
                                    theme_mode,
                                    active_section,
                                    colors,
                                    editor_draft,
                                    custom_themes,
                                    font_size_slider,
                                    ui_font_size_slider,
                                    line_height_input,
                                    ui_font_select,
                                    code_font_select,
                                    clients,
                                    dangerous_actions_collapsed,
                                    data_groups,
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
) -> Sidebar<SidebarGroup<SidebarMenuItem>> {
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
        .children(settings_sidebar_groups(active_section, cx))
}

/// 设置菜单分组：通用 / 数据与连接 / AI 助手 / 应用。
fn settings_sidebar_groups(
    active_section: SettingsPanelSection,
    cx: &mut Context<NavicatMain>,
) -> Vec<SidebarGroup<SidebarMenuItem>> {
    let item = |section: SettingsPanelSection| {
        SidebarMenuItem::new(settings_section_label(section))
            .active(section == active_section)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.settings_panel_section = section;
                if section == SettingsPanelSection::AiAgents
                    && this.ai_agent_editor.is_none()
                    && !this.controller.state().ai_agents.is_empty()
                {
                    let first = this.controller.state().ai_agents[0].id.clone();
                    this.open_ai_agent_editor(Some(first), window, cx);
                }
                cx.stop_propagation();
                cx.notify();
            }))
    };

    vec![
        SidebarGroup::new(tr("app.settings.group.general")).children([
            item(SettingsPanelSection::Editor),
            item(SettingsPanelSection::Shortcuts),
            item(SettingsPanelSection::Appearance),
        ]),
        SidebarGroup::new(tr("app.settings.group.data_connections")).children([
            item(SettingsPanelSection::Data),
            item(SettingsPanelSection::ConnectionSecurity),
            item(SettingsPanelSection::DatabaseSupport),
        ]),
        SidebarGroup::new(tr("app.settings.group.ai_assistant")).children([
            item(SettingsPanelSection::AiModels),
            item(SettingsPanelSection::AiToolsMcp),
            item(SettingsPanelSection::AiAgents),
            item(SettingsPanelSection::AiSessions),
        ]),
        SidebarGroup::new(tr("app.settings.group.application")).children([
            item(SettingsPanelSection::System),
            item(SettingsPanelSection::About),
        ]),
    ]
}

fn settings_panel_header(
    state: &AppState,
    active_section: SettingsPanelSection,
    colors: UiColors,
    editor_draft: Settings,
    font_size_slider: Entity<SliderState>,
    ui_font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let appearance_font_size_slider = font_size_slider;
    let appearance_ui_font_size_slider = ui_font_size_slider;
    let appearance_line_height_input = line_height_input;
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
                        .text_size(ui_text_size(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(settings_section_label(active_section)),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
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
                            .label(tr("app.settings.reset_defaults"))
                            .small()
                            .rounded(colors.radius)
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.settings_editor_draft = settings_default_editor_draft(
                                    &this.controller.state().settings,
                                );
                                this.show_message(
                                    tr("app.settings.message.editor_reset"),
                                    AppMessageKind::Info,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("settings-editor-save")
                                    .label(tr("app.common.save"))
                            .small()
                            .rounded(colors.radius)
                            .primary()
                            .disabled(!dirty)
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                let mut settings = this.controller.state().settings.clone();
                                settings_apply_editor_fields(
                                    &mut settings,
                                    &this.settings_editor_draft,
                                );
                                save_settings_from_ui(
                                    this,
                                    settings,
                                    tr("app.settings.message.editor_saved"),
                                    cx,
                                );
                            })),
                    ),
            )
        })
        .when(active_section == SettingsPanelSection::Appearance, |this| {
            this.child(settings_appearance_header_actions(
                &state.settings,
                &editor_draft,
                appearance_font_size_slider,
                appearance_ui_font_size_slider,
                appearance_line_height_input,
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
        .when(active_section == SettingsPanelSection::About, |this| {
            this.child(settings_section_header_actions(
                SettingsPanelSection::About,
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
            ),
            |this| this.child(settings_section_header_actions_disabled(colors)),
        )
}

fn settings_panel_body(
    state: &AppState,
    update: &UpdateUiState,
    ai_settings_panel: AiSettingsPanelState,
    theme_mode: ThemeMode,
    active_section: SettingsPanelSection,
    colors: UiColors,
    editor_draft: Settings,
    custom_themes: &BTreeSet<String>,
    font_size_slider: Entity<SliderState>,
    ui_font_size_slider: Entity<SliderState>,
    line_height_input: Entity<InputState>,
    ui_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    code_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    clients: [NativeClientPanelState; 2],
    dangerous_actions_collapsed: bool,
    data_groups: SettingsDataGroupsCollapsed,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    match active_section {
        SettingsPanelSection::Editor => settings_editor_panel(
            editor_draft,
            colors,
            dangerous_actions_collapsed,
            cx,
        ),
        SettingsPanelSection::Shortcuts => {
            settings_shortcuts_panel(&editor_draft, colors, window, cx)
        }
        SettingsPanelSection::Appearance => settings_appearance_panel(
            &editor_draft,
            theme_mode,
            custom_themes,
            colors,
            font_size_slider,
            ui_font_size_slider,
            line_height_input,
            ui_font_select,
            code_font_select,
            window,
            cx,
        ),
        SettingsPanelSection::System => settings_system_panel(&editor_draft, colors, cx),
        SettingsPanelSection::Data => {
            settings_data_panel(&editor_draft, clients, data_groups, colors, window, cx)
        }
        SettingsPanelSection::ConnectionSecurity => settings_connection_security_panel(colors),
        SettingsPanelSection::DatabaseSupport => {
            div().child(settings_database_support_panel(colors))
        }
        SettingsPanelSection::AiModels => {
            settings_ai_models_panel(state, ai_settings_panel, colors, cx)
        }
        SettingsPanelSection::AiToolsMcp => {
            settings_ai_tools_mcp_panel(state, &ai_settings_panel, colors, cx)
        }
        SettingsPanelSection::AiAgents => {
            settings_ai_agents_panel(state, &ai_settings_panel, colors, cx)
        }
        SettingsPanelSection::AiSessions => {
            settings_ai_sessions_panel(state, &ai_settings_panel, colors, window, cx)
        }
        SettingsPanelSection::About => settings_about_panel(&editor_draft, update, colors, cx),
    }
}
