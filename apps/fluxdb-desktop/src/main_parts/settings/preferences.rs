fn settings_editor_changed(saved: &Settings, draft: &Settings) -> bool {
    saved.page_size != draft.page_size
        || saved.results_placement != draft.results_placement
        || saved.confirm_dangerous_sql != draft.confirm_dangerous_sql
        || saved.dangerous_sql_actions != draft.dangerous_sql_actions
        || saved.confirm_dangerous_redis != draft.confirm_dangerous_redis
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
        SettingsPanelSection::AiModels
        | SettingsPanelSection::AiToolsMcp
        | SettingsPanelSection::AiAgents
        | SettingsPanelSection::AiSessions => false,
        SettingsPanelSection::Editor => settings_editor_changed(saved, draft),
        SettingsPanelSection::Shortcuts => saved.custom_keybindings != draft.custom_keybindings,
        SettingsPanelSection::Appearance => {
            saved.theme != draft.theme
                || saved.locale != draft.locale
                || saved.light_theme != draft.light_theme
                || saved.dark_theme != draft.dark_theme
                // 字体设置现在由外观页统一管理；形状/布局/动效已从 UI 移除，
                // 不再参与脏检查，避免保存外观时覆盖历史配置。
                || saved.global_font_family != draft.global_font_family
                || saved.ui_font_size != draft.ui_font_size
                || saved.editor_font_family != draft.editor_font_family
                || saved.editor_font_size != draft.editor_font_size
                || saved.editor_line_height != draft.editor_line_height
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
        || saved.mysql_client_dir != draft.mysql_client_dir
        || saved.mysql_client_download_source != draft.mysql_client_download_source
        || saved.sqlite3_path != draft.sqlite3_path
        || saved.pg_dump_path != draft.pg_dump_path
        || saved.pg_client_dir != draft.pg_client_dir
        || saved.pg_client_download_source != draft.pg_client_download_source
        }
        SettingsPanelSection::ConnectionSecurity
        | SettingsPanelSection::DatabaseSupport => false,
        SettingsPanelSection::About => saved.auto_check_updates != draft.auto_check_updates,
    }
}

fn settings_apply_appearance_fields(settings: &mut Settings, draft: &Settings) {
    settings.theme = draft.theme;
    settings.locale = draft.locale.clone();
    settings.light_theme = draft.light_theme.clone();
    settings.dark_theme = draft.dark_theme.clone();
    settings.global_font_family = draft.global_font_family.clone();
    settings.ui_font_size = draft.ui_font_size;
    settings.editor_font_family = draft.editor_font_family.clone();
    settings.editor_font_size = draft.editor_font_size;
    settings.editor_line_height = draft.editor_line_height;
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
        SettingsPanelSection::AiModels
        | SettingsPanelSection::AiToolsMcp
        | SettingsPanelSection::AiAgents
        | SettingsPanelSection::AiSessions => {}
        SettingsPanelSection::Editor => settings_apply_editor_fields(settings, draft),
        SettingsPanelSection::Shortcuts => {
            settings.custom_keybindings = draft.custom_keybindings.clone();
            settings.table_shortcut_groups = draft.table_shortcut_groups.clone();
        }
        SettingsPanelSection::Appearance => settings_apply_appearance_fields(settings, draft),
        SettingsPanelSection::System => settings_apply_system_fields(settings, draft),
        SettingsPanelSection::Data => settings_apply_backup_fields(settings, draft),
        SettingsPanelSection::About => {
            settings.auto_check_updates = draft.auto_check_updates;
        }
        SettingsPanelSection::ConnectionSecurity
        | SettingsPanelSection::DatabaseSupport => {}
    }
}

fn settings_apply_backup_fields(settings: &mut Settings, draft: &Settings) {
    settings.data_table_page_size = draft.data_table_page_size;
    settings.backup_dir = draft.backup_dir.clone();
    settings.mysqldump_path = draft.mysqldump_path.clone();
    settings.mysql_client_dir = draft.mysql_client_dir.clone();
    settings.mysql_client_download_source = draft.mysql_client_download_source.clone();
    settings.sqlite3_path = draft.sqlite3_path.clone();
    settings.pg_dump_path = draft.pg_dump_path.clone();
    settings.pg_client_dir = draft.pg_client_dir.clone();
    settings.pg_client_download_source = draft.pg_client_download_source.clone();
}

fn reset_settings_section(
    this: &mut NavicatMain,
    section: SettingsPanelSection,
    cx: &mut Context<NavicatMain>,
) {
    match section {
        SettingsPanelSection::Shortcuts => {
            this.settings_editor_draft.custom_keybindings.clear();
            this.show_message(
                tr("app.settings.message.shortcuts_reset"),
                AppMessageKind::Info,
                cx,
            );
            cx.notify();
        }
        SettingsPanelSection::System => {
            let defaults = Settings::default();
            this.settings_editor_draft.log_level = defaults.log_level;
            this.settings_editor_draft.log_path = defaults.log_path;
            this.settings_editor_draft.performance_diagnostics =
                defaults.performance_diagnostics;
            this.show_message(
                tr("app.settings.message.system_reset"),
                AppMessageKind::Info,
                cx,
            );
            cx.notify();
        }
        SettingsPanelSection::About => {
            this.settings_editor_draft.auto_check_updates = Settings::default().auto_check_updates;
            this.show_message(
                tr("app.settings.message.update_reset"),
                AppMessageKind::Info,
                cx,
            );
            cx.notify();
        }
        SettingsPanelSection::Data => {
            let defaults = Settings::default();
            this.settings_editor_draft.data_table_page_size = defaults.data_table_page_size;
            this.settings_editor_draft.backup_dir = defaults.backup_dir;
            this.settings_editor_draft.mysqldump_path = defaults.mysqldump_path;
            this.settings_editor_draft.mysql_client_dir = defaults.mysql_client_dir;
            this.settings_editor_draft.mysql_client_download_source =
                defaults.mysql_client_download_source;
            this.settings_editor_draft.sqlite3_path = defaults.sqlite3_path;
            this.settings_editor_draft.pg_dump_path = defaults.pg_dump_path;
            this.settings_editor_draft.pg_client_dir = defaults.pg_client_dir;
            this.settings_editor_draft.pg_client_download_source =
                defaults.pg_client_download_source;
            this.show_message(
                tr("app.settings.message.data_reset"),
                AppMessageKind::Info,
                cx,
            );
            cx.notify();
        }
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
        rebind_table_shortcut_groups(cx, &saved, &settings);
    }

    let _ = this.controller.dispatch(AppCommand::SaveSettings(settings));
    let _ = this.storage.save_settings(&this.controller.state().settings);
    let locale = fluxdb_i18n::effective_locale(&this.controller.state().settings.locale);
    apply_ui_locale(&locale, cx);
    // 保存后按已落盘的草稿重新应用主题、界面字体与代码字体/字号/行高，
    // 不依赖选择控件是否已触发预览。
    if section == SettingsPanelSection::Appearance {
        this.preview_settings(cx);
    }
    this.sync_settings_tab_dirty();
    this.show_message(
        match section {
            SettingsPanelSection::Shortcuts => tr("app.settings.message.shortcuts_saved"),
            SettingsPanelSection::System => tr("app.settings.message.system_saved"),
            _ => tr("app.settings.message.saved"),
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
    settings.editor_tab_width = draft.editor_tab_width;
    settings.editor_word_wrap = draft.editor_word_wrap;
}

fn settings_default_editor_draft(saved: &Settings) -> Settings {
    let defaults = Settings::default();
    let mut draft = saved.clone();
    settings_apply_editor_fields(&mut draft, &defaults);
    draft
}

impl NavicatMain {
    /// 消费 AI 的结构化动作；配置提案仍需确认，SQL 修改走独立的范围校验。
    fn apply_ai_ui_action(
        &mut self,
        action: fluxdb_app::UiAction,
        cx: &mut Context<NavicatMain>,
    ) {
        match action {
            fluxdb_app::UiAction::TablePage(request) => self.apply_ai_table_page(request, cx),
            fluxdb_app::UiAction::EditSqlEditor(request) => {
                self.apply_ai_sql_editor_edit(request, cx);
            }
            fluxdb_app::UiAction::ProposeConfig(proposal) => {
                self.pending_config_proposal = Some(proposal);
                self.config_delete_confirmed = false;
                self.show_message(
                    tr("app.ai.config_proposal.pending").as_ref(),
                    AppMessageKind::Info,
                    cx,
                );
                cx.notify();
            }
        }
    }
}

fn settings_panel_section_from_ai(
    section: fluxdb_app::SettingsSection,
) -> SettingsPanelSection {
    match section {
        fluxdb_app::SettingsSection::Editor => SettingsPanelSection::Editor,
        fluxdb_app::SettingsSection::Appearance => SettingsPanelSection::Appearance,
        fluxdb_app::SettingsSection::Data => SettingsPanelSection::Data,
        fluxdb_app::SettingsSection::System => SettingsPanelSection::System,
    }
}

fn save_settings_from_ui(
    this: &mut NavicatMain,
    settings: Settings,
    message: impl Into<String>,
    cx: &mut Context<NavicatMain>,
) {
    let settings_before = this.controller.state().settings.clone();
    // 「查询结果默认布局」改了应当立刻生效，不必重启 —— 否则旁边的工具栏按钮能即时切换、
    // 设置项却要重启，是个 UX 陷阱。只在**该项本身变了**时才动运行时布局：
    // 工具栏切换是仅本次会话的临时覆盖，若无条件同步，用户改个字号一保存就会把布局莫名拉回。
    // 必须在 dispatch 之前比较 —— 那一步会把 state.settings 换成新值。
    let _ = this.controller.dispatch(AppCommand::SaveSettings(settings));
    let _ = this.storage.save_settings(&this.controller.state().settings);
    this.settings_editor_draft = this.controller.state().settings.clone();
    this.apply_saved_settings_runtime(&settings_before, cx);
    this.show_message(message, AppMessageKind::Success, cx);
}

impl NavicatMain {
    /// 只更新运行时投影，不写盘；持久化提案成功后复用，避免二次保存。
    fn apply_saved_settings_runtime(&mut self, before: &Settings, cx: &mut Context<Self>) {
        if before.results_placement != self.controller.state().settings.results_placement {
            self.results_placement = self.controller.state().settings.results_placement;
        }
        let locale = fluxdb_i18n::effective_locale(&self.controller.state().settings.locale);
        apply_ui_locale(&locale, cx);
        let new_settings = self.controller.state().settings.clone();
        for sql_editor in self.query_editors.values() {
            sql_editor.update(cx, |editor, cx| {
                apply_code_editor_settings(editor, &new_settings, cx);
            });
        }
        self.preview_settings(cx);
        self.sync_settings_tab_dirty();
    }
}

fn settings_preference_row(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon: AppIcon,
    status: impl Into<SharedString>,
    colors: UiColors,
) -> Stateful<Div> {
    let title = title.into();
    let detail = detail.into();
    let status = status.into();
    settings_row_container_dynamic(
        format!("settings-preference:{title}:{status}"),
        colors,
    )
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
                                .text_size(ui_text_size(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(title.clone()),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(detail),
                        ),
                ),
        )
        .child(settings_status_chip(title, status, colors))
}

fn settings_status_chip(
    title: impl Into<SharedString>,
    label: impl Into<SharedString>,
    colors: UiColors,
) -> impl IntoElement {
    let title = title.into();
    let label = label.into();
    Button::new(SharedString::from(format!("settings-status:{title}:{label}")))
        .label(label)
        .small()
        .rounded(colors.radius)
        .disabled(true)
}

fn settings_status_id(title: &str, label: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    title.hash(&mut hasher);
    label.hash(&mut hasher);
    hasher.finish()
}
