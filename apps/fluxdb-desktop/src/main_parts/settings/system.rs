fn settings_data_panel(
    settings: &Settings,
    clients: [NativeClientPanelState; 2],
    groups: SettingsDataGroupsCollapsed,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 备份组内容较多但属于常用配置，默认展开；客户端组默认折叠，标题右侧带状态摘要。
    let backup_group = settings_collapsible_group(SettingsDataGroup::Backup, None, groups.backup, colors, cx);
    let backup_group = if groups.backup {
        backup_group
    } else {
        backup_group
            .child(settings_path_row(
                tr("app.settings.data.backup_dir"),
                tr("app.settings.data.backup_dir_detail"),
                AppIcon::Folder,
                &settings.backup_dir,
                "settings-backup-dir",
                true,
                colors,
                cx,
            ))
            .child(settings_path_row(
                tr("app.settings.data.sqlite3_path"),
                tr("app.settings.data.sqlite3_path_detail"),
                AppIcon::Database,
                &settings.sqlite3_path,
                "settings-backup-sqlite3",
                false,
                colors,
                cx,
            ))
    };
    let page_size_choices = DATA_TABLE_PAGE_SIZE_CHOICES
        .iter()
        .map(|(_, value)| (value.to_string().into(), *value))
        .collect::<Vec<_>>();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group(tr("app.settings.data.group.tables"), colors)
                .child(settings_choice_row_with_id(
                    "settings-data-page-size",
                    tr("app.settings.data.page_size"),
                    tr("app.settings.data.page_size_detail"),
                    AppIcon::List,
                    settings.data_table_page_size,
                    // 档位与《数据页 SQL 面板的 LIMIT 封顶》同源，见
                    // `fluxdb_core::data_table_page_size_max` 的说明。
                    &page_size_choices,
                    |settings, value| settings.data_table_page_size = value,
                    colors,
                    false,
                    cx,
                )),
        )
        .child(backup_group)
        .children(clients.into_iter().map(|client| {
            // 折叠状态按客户端类型区分，两个客户端组可独立展开/收起。
            let collapsed = match client.kind {
                NativeClientKind::Postgres => groups.pg_client,
                NativeClientKind::MySql => groups.mysql_client,
            };
            settings_native_client_group(settings, client, collapsed, colors, window, cx)
        }))
}

/// 「数据」页各分组的折叠状态。UI 瞬时态，不持久化；备份默认展开，客户端默认折叠。
#[derive(Clone, Copy)]
struct SettingsDataGroupsCollapsed {
    backup: bool,
    pg_client: bool,
    mysql_client: bool,
}

/// 「数据」页里支持折叠的分组标识。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsDataGroup {
    Backup,
    PgClient,
    MySql,
}

impl SettingsDataGroup {
    fn title(self) -> SharedString {
        match self {
            Self::Backup => tr("app.settings.data.group.backup"),
            Self::PgClient => tr("app.settings.data.group.pg_client"),
            Self::MySql => tr("app.settings.data.group.mysql_client"),
        }
    }
}

impl NavicatMain {
    fn toggle_settings_data_group(&mut self, group: SettingsDataGroup) {
        let collapsed = &mut self.settings_data_groups_collapsed;
        match group {
            SettingsDataGroup::Backup => collapsed.backup = !collapsed.backup,
            SettingsDataGroup::PgClient => collapsed.pg_client = !collapsed.pg_client,
            SettingsDataGroup::MySql => collapsed.mysql_client = !collapsed.mysql_client,
        }
    }
}

/// 可折叠的设置分组：标题整行可点击切换折叠，右侧可挂状态摘要（如客户端检测状态）。
/// 交互与编辑器页「危险 SQL 操作清单」一致：Chevron 指示方向 + on_mouse_down 切换。
/// 不用 gpui-component 的 Accordion：其展开状态是组件内部 RenderOnce 临时态，
/// 设置页每次重渲染会重建组件导致状态丢失；这里沿用仓库已有的折叠状态模式。
fn settings_collapsible_group(
    group: SettingsDataGroup,
    status_hint: Option<&str>,
    collapsed: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> GroupBox {
    let mut title = h_flex()
        .w_full()
        .gap_1()
        .cursor_pointer()
        .child(app_icon_box(
            if collapsed {
                AppIcon::ChevronRight
            } else {
                AppIcon::ChevronDown
            },
            24.,
            13.,
            colors.muted,
        ))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(group.title()),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_settings_data_group(group);
                cx.notify();
            }),
        );
    if let Some(hint) = status_hint {
        // 折叠时也能从标题右侧看到关键状态，避免为了看状态必须展开。
        title = title.child(
            div().flex_1().child(
                h_flex()
                    .ml_auto()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(hint.to_string()),
            ),
        );
    }
    settings_panel_group_el(title)
}

/// 设置面板中一条可选择的路径行（目录或文件）。选择结果写入 settings_editor_draft 对应字段。
#[allow(
    clippy::too_many_arguments,
    reason = "路径行参数逐项对应 GPUI 控件契约，打包成结构体反而隐藏调用点差异"
)]
fn settings_path_row(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon: AppIcon,
    value: &str,
    button_id: &'static str,
    directory: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let title = title.into();
    let detail = detail.into();
    settings_path_row_with_placeholder(
        title,
        detail,
        icon,
        value,
        &tr("app.settings.not_set"),
        button_id,
        directory,
        colors,
        cx,
    )
}

/// 同 `settings_path_row`,但未设置时的占位文案可自定义(如显示系统默认搜索路径)。
#[allow(clippy::too_many_arguments)]
fn settings_path_row_with_placeholder(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon: AppIcon,
    value: &str,
    placeholder: &str,
    button_id: &'static str,
    directory: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    settings_action_row_with_id(button_id, title, detail, icon, colors)
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
                            placeholder.to_string()
                        } else {
                            value.to_string()
                        }),
                )
                .child(
                    Button::new(button_id)
                        .label(tr("app.common.choose"))
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
        prompt: Some(tr("app.settings.choose_path")),
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
                            "settings-backup-pg-dump" => {
                                this.settings_editor_draft.pg_dump_path = value
                            }
                            "settings-backup-pg-client-dir" => {
                                this.settings_editor_draft.pg_client_dir = value;
                                // 选完目录立刻复检，省去「保存后才知道选对没有」的来回。
                                this.detect_native_client_tools(NativeClientKind::Postgres, cx);
                            }
                            "settings-backup-mysql-client-dir" => {
                                this.settings_editor_draft.mysql_client_dir = value;
                                this.detect_native_client_tools(NativeClientKind::MySql, cx);
                            }
                            _ => {}
                        }
                        cx.notify();
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => this.show_message(
                    format!("{}: {error}", tr("app.settings.choose_path_failed")),
                    AppMessageKind::Error,
                    cx,
                ),
                Err(error) => this.show_message(
                    format!("{}: {error}", tr("app.settings.choose_path_failed")),
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
            settings_panel_group(tr("app.settings.connection_security.group.connection"), colors)
                .child(settings_preference_row(
                    tr("app.settings.connection_security.timeout"),
                    tr("app.settings.connection_security.timeout_detail"),
                    AppIcon::Plug,
                    tr("app.settings.status.pending"),
                    colors,
                )),
        )
        .child(
            settings_panel_group(tr("app.settings.connection_security.group.security"), colors)
                .child(settings_preference_row(
                tr("app.settings.connection_security.sensitive_storage"),
                tr("app.settings.connection_security.sensitive_storage_detail"),
                AppIcon::Check,
                tr("app.settings.status.enabled"),
                colors,
            )),
        )
}

fn settings_database_support_panel(colors: UiColors) -> GroupBox {
    settings_panel_group(tr("app.settings.database_support.title"), colors).children(
        database_support_entries()
            .into_iter()
            .map(|entry| settings_database_support_row(entry, colors)),
    )
}

fn settings_about_panel(
    settings: &Settings,
    update: &UpdateUiState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let auto_check_draft = settings.auto_check_updates;
    let update_draft = cx.entity().downgrade();
    let busy = matches!(
        update.status,
        UpdateUiStatus::Checking
            | UpdateUiStatus::Downloading
            | UpdateUiStatus::Installing
    );
    let manual_label = match update.status {
        UpdateUiStatus::Checking => tr("app.settings.update.checking"),
        _ => tr("app.settings.update.check_now"),
    };
    let update_hint = if update.status == UpdateUiStatus::Available {
        update
            .update
            .as_ref()
            .map(|available| {
                SharedString::from(
                    tr("app.settings.update.available_version")
                        .replace("%{version}", &available.version),
                )
            })
            .unwrap_or_else(|| tr("app.settings.update.status.available"))
    } else {
        match update.status {
            UpdateUiStatus::Idle => tr("app.settings.update.status.idle"),
            UpdateUiStatus::Checking => tr("app.settings.update.status.checking"),
            UpdateUiStatus::Available => tr("app.settings.update.status.available"),
            UpdateUiStatus::UpToDate => tr("app.settings.update.status.up_to_date"),
            UpdateUiStatus::Downloading => tr("app.settings.update.status.downloading"),
            UpdateUiStatus::ReadyToInstall => tr("app.settings.update.status.ready"),
            UpdateUiStatus::Installing => tr("app.settings.update.status.installing"),
            UpdateUiStatus::Error => tr("app.settings.update.status.error"),
        }
    };
    div().flex().flex_col().gap_3().child(
        settings_panel_group(tr("app.settings.about.group.about"), colors)
            .child(settings_preference_row(
                "FluxDB Desktop",
                format!(
                    "{}  ·  {}",
                    tr("app.settings.update.current_version"),
                    env!("CARGO_PKG_VERSION")
                ),
                AppIcon::Database,
                format!("v{}", env!("CARGO_PKG_VERSION")),
                colors,
            ))
            .child(settings_preference_row(
                tr("app.settings.about.config_model"),
                tr("app.settings.about.config_model_detail"),
                AppIcon::Workflow,
                tr("app.settings.status.pending"),
                colors,
            )),
    )
    .child(
        settings_panel_group(tr("app.settings.about.group.updates"), colors)
            .child(
                settings_action_row_with_id(
                    "settings-auto-check-updates",
                    tr("app.settings.about.auto_update"),
                    tr("app.settings.about.auto_update_detail"),
                    AppIcon::Refresh,
                    colors,
                )
                .child(
                    div()
                        .debug_selector(|| "settings-auto-check-updates-switch".into())
                        .child(
                            Switch::new("settings-auto-check-updates")
                                .checked(auto_check_draft)
                                .on_click(move |checked, _, cx| {
                                    let _ = update_draft.update(cx, |this, cx| {
                                        this.settings_editor_draft.auto_check_updates = *checked;
                                        cx.notify();
                                    });
                                }),
                        ),
                ),
            )
            .child(
                settings_action_row_with_id(
                    "settings-check-updates",
                    tr("app.settings.about.check_updates"),
                    update_hint,
                    AppIcon::Refresh,
                    colors,
                )
                .child(
                    Button::new("settings-check-updates")
                        .label(manual_label)
                        .small()
                        .rounded(colors.radius)
                        .disabled(busy)
                        .debug_selector(|| "settings-check-updates".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_update_check(true, cx);
                        })),
                ),
            ),
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
            settings_panel_group(tr("app.settings.system.group.diagnostics"), colors)
                .child(settings_performance_diagnostics_row(
                    settings.performance_diagnostics,
                    colors,
                    cx,
                ))
                .child(settings_log_level_row(settings.log_level, colors, cx))
                .child(settings_log_path_row(&settings.log_path, colors, cx)),
        )
        .child(
            settings_panel_group(tr("app.settings.system.group.local_data"), colors)
                .child(settings_preference_row(
                    tr("app.settings.system.view_data_dir"),
                    tr("app.settings.system.view_data_dir_detail"),
                    AppIcon::Folder,
                    tr("app.settings.status.pending"),
                    colors,
                ))
                .child(settings_preference_row(
                    tr("app.settings.system.clear_query_history"),
                    tr("app.settings.system.clear_query_history_detail"),
                    AppIcon::Trash,
                    tr("app.settings.status.pending"),
                    colors,
                ))
                .child(settings_preference_row(
                    tr("app.settings.system.clear_redis_history"),
                    tr("app.settings.system.clear_redis_history_detail"),
                    AppIcon::Trash,
                    tr("app.settings.status.pending"),
                    colors,
                ))
                .child(settings_preference_row(
                    tr("app.settings.system.clear_completion_cache"),
                    tr("app.settings.system.clear_completion_cache_detail"),
                    AppIcon::Trash,
                    tr("app.settings.status.pending"),
                    colors,
                ))
                .child(settings_preference_row(
                    tr("app.settings.system.export_diagnostics"),
                    tr("app.settings.system.export_diagnostics_detail"),
                    AppIcon::Refresh,
                    tr("app.settings.status.pending"),
                    colors,
                )),
        )
        .child(
            settings_panel_group(tr("app.settings.system.group.advanced"), colors)
                .child(settings_preference_row(
                tr("app.settings.system.proxy"),
                tr("app.settings.system.proxy_detail"),
                AppIcon::Plug,
                tr("app.settings.status.pending"),
                colors,
            )),
        )
}

fn settings_panel_group(title: impl Into<SharedString>, _colors: UiColors) -> GroupBox {
    let title = title.into();
    settings_panel_group_el(
        div()
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(title),
    )
}

fn settings_panel_group_el(title: impl IntoElement) -> GroupBox {
    GroupBox::new()
        .with_variant(GroupBoxVariant::Outline)
        .title(title)
        // GroupBox Outline 默认给内容区 p_4()(16px)+gap_4()(16px)，会在每行下方叠出明显留白。
        // 改由各设置行自身的行高/顶部分隔线(如 h(52.) + border_t_1) 统一控制间距，
        // 让所有设置项与分隔线之间间距紧凑一致。
        .content_style(gpui::StyleRefinement::default().p_0().gap_0())
}

fn settings_section_label(section: SettingsPanelSection) -> SharedString {
    match section {
        SettingsPanelSection::Editor => tr("app.settings.section.editor"),
        SettingsPanelSection::Shortcuts => tr("app.settings.section.shortcuts"),
        SettingsPanelSection::Appearance => tr("app.settings.section.appearance"),
        SettingsPanelSection::System => tr("app.settings.section.system"),
        SettingsPanelSection::Data => tr("app.settings.section.data"),
        SettingsPanelSection::ConnectionSecurity => tr("app.settings.section.connection_security"),
        SettingsPanelSection::DatabaseSupport => tr("app.settings.section.database_support"),
        SettingsPanelSection::AiModels => tr("app.settings.section.ai_models"),
        SettingsPanelSection::AiToolsMcp => tr("app.settings.section.ai_tools_mcp"),
        SettingsPanelSection::AiAgents => tr("app.settings.section.ai_agents"),
        SettingsPanelSection::AiSessions => tr("app.settings.section.ai_sessions"),
        SettingsPanelSection::About => tr("app.settings.section.about"),
    }
}

fn settings_section_description(section: SettingsPanelSection) -> SharedString {
    match section {
        SettingsPanelSection::Editor => tr("app.settings.description.editor"),
        SettingsPanelSection::Shortcuts => tr("app.settings.description.shortcuts"),
        SettingsPanelSection::Appearance => tr("app.settings.description.appearance"),
        SettingsPanelSection::System => tr("app.settings.description.system"),
        SettingsPanelSection::Data => tr("app.settings.description.data"),
        SettingsPanelSection::ConnectionSecurity => {
            tr("app.settings.description.connection_security")
        }
        SettingsPanelSection::AiModels => tr("app.settings.description.ai_models"),
        SettingsPanelSection::AiToolsMcp => tr("app.settings.description.ai_tools_mcp"),
        SettingsPanelSection::AiAgents => tr("app.settings.description.ai_agents"),
        SettingsPanelSection::AiSessions => tr("app.settings.description.ai_sessions"),
        SettingsPanelSection::DatabaseSupport => {
            tr("app.settings.description.database_support")
        }
        SettingsPanelSection::About => tr("app.settings.description.about"),
    }
}

#[derive(Clone)]
struct DatabaseSupportEntry {
    kind: DatabaseKind,
    source: SharedString,
    detail: SharedString,
}

/// 内置连接器清单，顺序与 `DatabaseKind` 保持一致；新增数据库类型时必须同步补齐。
fn database_support_entries() -> [DatabaseSupportEntry; 10] {
    [
        DatabaseSupportEntry {
            kind: DatabaseKind::MySql,
            source: tr("app.settings.database_support.mysql_source"),
            detail: tr("app.settings.database_support.mysql_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::TiDb,
            source: tr("app.settings.database_support.tidb_source"),
            detail: tr("app.settings.database_support.tidb_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::MariaDb,
            source: tr("app.settings.database_support.mariadb_source"),
            detail: tr("app.settings.database_support.mariadb_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::Sqlite,
            source: tr("app.settings.database_support.sqlite_source"),
            detail: tr("app.settings.database_support.sqlite_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::Postgres,
            source: tr("app.settings.database_support.postgres_source"),
            detail: tr("app.settings.database_support.postgres_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::Dameng,
            source: tr("app.settings.database_support.dameng_source"),
            detail: tr("app.settings.database_support.dameng_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::SqlServer,
            source: tr("app.settings.database_support.sqlserver_source"),
            detail: tr("app.settings.database_support.sqlserver_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::Oracle,
            source: tr("app.settings.database_support.oracle_source"),
            detail: tr("app.settings.database_support.oracle_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::MongoDb,
            source: tr("app.settings.database_support.mongo_source"),
            detail: tr("app.settings.database_support.mongo_detail"),
        },
        DatabaseSupportEntry {
            kind: DatabaseKind::Redis,
            source: tr("app.settings.database_support.redis_source"),
            detail: tr("app.settings.database_support.redis_detail"),
        },
    ]
}
