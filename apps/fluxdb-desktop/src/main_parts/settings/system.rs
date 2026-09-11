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
) -> Stateful<Div> {
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
        // GroupBox Outline 默认给内容区 p_4()(16px)+gap_4()(16px)，会在每行下方叠出明显留白。
        // 改由各设置行自身的行高/顶部分隔线(如 h(52.) + border_t_1) 统一控制间距，
        // 让所有设置项与分隔线之间间距紧凑一致。
        .content_style(gpui::StyleRefinement::default().p_0().gap_0())
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

