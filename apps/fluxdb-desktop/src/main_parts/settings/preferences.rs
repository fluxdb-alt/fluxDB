fn settings_editor_changed(saved: &Settings, draft: &Settings) -> bool {
    saved.page_size != draft.page_size
        || saved.results_placement != draft.results_placement
        || saved.confirm_dangerous_sql != draft.confirm_dangerous_sql
        || saved.dangerous_sql_actions != draft.dangerous_sql_actions
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
            saved.data_table_page_size != draft.data_table_page_size
                || saved.backup_dir != draft.backup_dir
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
    settings.data_table_page_size = draft.data_table_page_size;
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
    settings.results_placement = draft.results_placement;
    settings.confirm_dangerous_sql = draft.confirm_dangerous_sql;
    settings.dangerous_sql_actions = draft.dangerous_sql_actions.clone();
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
    // 「查询结果默认布局」改了应当立刻生效，不必重启 —— 否则旁边的工具栏按钮能即时切换、
    // 设置项却要重启，是个 UX 陷阱。只在**该项本身变了**时才动运行时布局：
    // 工具栏切换是仅本次会话的临时覆盖，若无条件同步，用户改个字号一保存就会把布局莫名拉回。
    // 必须在 dispatch 之前比较 —— 那一步会把 state.settings 换成新值。
    if settings.results_placement != this.controller.state().settings.results_placement {
        this.results_placement = settings.results_placement;
    }

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
) -> Stateful<Div> {
    settings_row_container(title, colors)
        .h(px(52.))
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

