fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--mcp-server") {
        // stdio 子进程不能进入桌面单实例守卫、日志初始化或 GPUI。
        std::process::exit(fluxdb_mcp::run_cli(args));
    }

    // 实例标识最先校验：单实例守卫与数据目录解析都依赖它，非法取值必须在此明确
    // 报错退出，否则用户只会看到后续步骤的"无法解析应用数据目录"而不知根因。
    if let Err(err) = fluxdb_storage::instance_profile() {
        eprintln!("FluxDB 启动终止: {err}");
        std::process::exit(1);
    }

    // 单实例守卫最先执行（方案 §12.1 方案 A）：避免第二实例与首实例并发写同一
    // SQLite/配置。第二实例明确提示并退出；锁基础设施失败时降级放行（见 single_instance.rs）。
    if !acquire_single_instance() {
        notify_second_instance_and_exit();
    }

    // 应用数据目录解析失败时明确报错退出（AI-01，方案 §4.1/§10.4-4：启动失败必须有证据），
    // 不再静默回退到当前目录。日志系统此时未初始化，先以 stderr 输出；
    // Windows GUI 子系统下 stderr 不可见的兜底可见性属后续任务（方案 §12.8）。
    let storage = match FileStorage::try_default() {
        Ok(storage) => storage,
        Err(err) => {
            eprintln!("FluxDB 无法解析应用数据目录，启动终止: {err}");
            std::process::exit(1);
        }
    };
    let storage = if cfg!(debug_assertions) {
        std::env::var_os("FLUXDB_AI_TEST_ROOT")
            .map(|p| FileStorage::new(std::path::PathBuf::from(p)))
            .unwrap_or(storage)
    } else {
        storage
    };
    let saved_settings = storage.load_settings().unwrap_or_default();
    // 最先初始化日志系统；guard 持有到 main 结束，保证日志 worker 线程存活。
    let _log_guard = init_logging(saved_settings.log_level, &saved_settings.log_path);
    tracing::info!(
        target: "fluxdb_desktop",
        "FluxDB 启动，日志已初始化（目录: {}）",
        configured_log_dir(&saved_settings.log_path).display()
    );

    let assets_base = app_assets_base_path();
    init_i18n(
        &assets_base.join("locales"),
        &storage.root_dir().join("locales"),
        &saved_settings.locale,
    );

    gpui_platform::application()
        .with_assets(Assets { base: assets_base })
        .run(move |cx: &mut App| {
            set_dock_icon();
            // 组件库必须在首次查找组件文案前完成命名空间扩展。
            extend_component_i18n();
            // Linux 首版无托盘；tray_icon.rs 仅在非 Linux 编译（见 main.rs include 与 Cargo.toml）。
            #[cfg(not(target_os = "linux"))]
            install_tray_icon(cx);
            gpui_component::init(cx);
            register_sql_highlighter();
            register_mysql_ddl_highlighter();
            register_json_highlighter();

            let bounds = startup_window_bounds(cx);
            let storage = storage.clone();
            let saved_settings = saved_settings.clone();
            register_shortcuts(cx, &saved_settings);
            set_app_menus(cx);
            // 从空连接启动：首启不注入 demo 假连接，侧边栏真实连接完全来自持久化存储，
            // 无持久连接时保持为空（空连接不造假数据显示）。
            let mut controller = AppController::new();
            controller.set_completion_index_storage(storage.clone());
            controller.set_ai_storage(storage.clone());
            // 大字段完整内容的受控存储：连接器外置与详情分段读取共用。
            controller.set_cell_content_storage(storage.clone());
            // 首帧后加载 AI 设置列表，避免阻塞开窗；空列表由设置页给出空状态。
            let _ = controller.dispatch(AppCommand::RefreshAiSettings);
            controller.set_er_model_storage(storage.clone());
            let theme_mode = debug_theme_mode_override()
                .unwrap_or_else(|| theme_mode_from_app_theme(saved_settings.theme));
            let _ = controller.dispatch(AppCommand::SaveSettings(saved_settings));
            apply_registered_theme(&controller.state().settings, theme_mode, cx);
            apply_component_theme_colors(theme_mode, cx);
            watch_bundled_themes(controller.state().settings.clone(), theme_mode, cx);
            if let Ok(connections) =
                storage.import_connections_if_empty(&controller.connection_configs())
            {
                let _ = controller.dispatch(AppCommand::ReplaceConnections(connections));
            }
            if let Ok(layout) = storage.load_sidebar_layout(&controller.connection_configs()) {
                let _ = controller.dispatch(AppCommand::ReplaceSidebarLayout(layout));
            }
            // 查询/工作台历史与保存的查询已改为首帧后异步恢复（NavicatMain 的
            // `load_persisted_history_in_background`），不再阻塞 open_window 前主线程建窗。

            cx.open_window(
                WindowOptions {
                    focus: true,
                    is_resizable: true,
                    titlebar: Some(platform_titlebar_options()),
                    // 与 .desktop ID 保持一致，Wayland app_id / X11 WM_CLASS 才能关联启动器图标。
                    #[cfg(target_os = "linux")]
                    app_id: Some("com.shining3d.fluxdb".to_string()),
                    // Linux 请求客户端装饰：撤掉系统标题栏，避免与顶栏自绘窗口按钮出现两套关闭；
                    // X11 无合成器时 gpui 自动回退 Server，CSD 阴影与拖边缩放由 gpui-component window_border 提供。
                    #[cfg(target_os = "linux")]
                    window_decorations: Some(gpui::WindowDecorations::Client),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    // Windows/Linux 默认正常关闭。当前只有 macOS 的隐藏/恢复行为经过验证。
                    #[cfg(target_os = "macos")]
                    install_close_to_tray(window, cx);
                    let view =
                        cx.new(|cx| navicat_main_new(window, cx, controller, storage, theme_mode));
                    // Tab 拦截已迁移到通用 Editor 的键绑定体系，不再需要全局拦截。
                    // 首帧后异步恢复查询/工作台历史与保存的查询（不阻塞建窗；
                    // saved_queries 经内部 notify 补显，详见 load_persisted_history_in_background）。
                    view.update(cx, |this, cx| {
                        this.load_persisted_history_in_background(cx);
                        // 只维护用户明确接入的客户端，不自动接管检测到的配置。
                        this.sync_mcp_clients(cx);
                        // 调试入口：打开设置页的 AI 分区（仅 debug 构建，用于 UI 截图验证）。
                        if debug_ai_settings_requested() {
                            let _ = this.controller.dispatch(AppCommand::OpenSettings);
                            cx.notify();
                        }
                        // 调试入口：直接打开模型编辑弹框（仅 debug 构建）。
                        if debug_ai_model_dialog_requested() {
                            let _ = this.controller.dispatch(AppCommand::OpenSettings);
                        }
                    });
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .unwrap();
            cx.activate(true);
        });
}

/// 调试用：从环境变量读取要打开的 AI 设置分区。
///
/// 仅 debug 构建生效，用于 UI 截图/交互验证；发布构建始终返回编辑器分区。
/// 例：`FLUXDB_DEBUG_AI_SETTINGS=models cargo run -p fluxdb-desktop`
/// 可用值：models / tools / agents / sessions / appearance
fn debug_ai_settings_section() -> SettingsPanelSection {
    if !cfg!(debug_assertions) {
        return SettingsPanelSection::Editor;
    }
    match std::env::var("FLUXDB_DEBUG_AI_SETTINGS").as_deref() {
        Ok("models") => SettingsPanelSection::AiModels,
        Ok("tools") => SettingsPanelSection::AiToolsMcp,
        Ok("agents") => SettingsPanelSection::AiAgents,
        Ok("sessions") => SettingsPanelSection::AiSessions,
        // 外观页用于明暗主题/窄窗截图核对，以及与设置相关的渲染回归定位。
        Ok("appearance") => SettingsPanelSection::Appearance,
        _ => SettingsPanelSection::Editor,
    }
}

/// 调试用：从环境变量选择「工具与 MCP」初始页签（仅 debug 构建）。
///
/// 例：`FLUXDB_DEBUG_AI_SETTINGS=tools FLUXDB_DEBUG_AI_TAB=plugins cargo run -p fluxdb-desktop`
fn debug_ai_tools_tab() -> AiToolsTab {
    if !cfg!(debug_assertions) {
        return AiToolsTab::Tools;
    }
    match std::env::var("FLUXDB_DEBUG_AI_TAB").as_deref() {
        Ok("mcp") => AiToolsTab::Mcp,
        Ok("plugins") => AiToolsTab::Plugins,
        Ok("audit") => AiToolsTab::Audit,
        _ => AiToolsTab::Tools,
    }
}

/// 调试用：是否请求打开 AI 设置分区。
fn debug_ai_settings_requested() -> bool {
    cfg!(debug_assertions) && std::env::var("FLUXDB_DEBUG_AI_SETTINGS").is_ok()
}

/// 调试用：覆写主题（仅 debug 构建），用于明暗主题截图核对。
///
/// 例：`FLUXDB_DEBUG_THEME=dark cargo run -p fluxdb-desktop`
fn debug_theme_mode_override() -> Option<ThemeMode> {
    if !cfg!(debug_assertions) {
        return None;
    }
    match std::env::var("FLUXDB_DEBUG_THEME").as_deref() {
        Ok("dark") => Some(ThemeMode::Dark),
        Ok("light") => Some(ThemeMode::Light),
        _ => None,
    }
}

/// 调试用：是否请求首帧自动测试第一个模型的连接（用于截图核对 Alert/状态标签）。
fn debug_ai_model_test_requested() -> bool {
    cfg!(debug_assertions) && std::env::var("FLUXDB_DEBUG_AI_TEST_MODEL").is_ok_and(|v| v == "1")
}

/// 调试用：是否请求直接打开 Agent 新建草稿。
fn debug_ai_agent_draft_requested() -> bool {
    cfg!(debug_assertions) && std::env::var("FLUXDB_DEBUG_AI_AGENT_DRAFT").is_ok_and(|v| v == "1")
}

/// 调试用：是否请求直接打开模型编辑弹框。
fn debug_ai_model_dialog_requested() -> bool {
    cfg!(debug_assertions)
        && std::env::var("FLUXDB_DEBUG_AI_MODEL_DIALOG")
            .is_ok_and(|v| v == "1" || v == "edit" || v == "invalid")
}

/// 调试用：请求直接打开某个可配置插件的「管理」弹框（截图核对用）。
///
/// `FLUXDB_DEBUG_AI_PLUGIN_CONFIG=1` 打开第一个可配置插件；
/// 设为具体实例 id（如 `builtin.security-audit`）则打开该实例。
fn debug_ai_plugin_config_target() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    match std::env::var("FLUXDB_DEBUG_AI_PLUGIN_CONFIG").as_deref() {
        Ok("1") => Some(String::new()),
        Ok(id) if !id.is_empty() => Some(id.to_string()),
        _ => None,
    }
}

/// 构造主视图 `NavicatMain`。
///
/// 从 `main()` 里约 3000 行的内联结构体字面量抽出：生产启动与 `#[gpui::test]` 交互测试
/// 走**同一条**构造路径，测试不能靠给孤立组件塞数据绕过生产装配（AGENTS.md / 设计 §39.5）。
fn navicat_main_new(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    controller: AppController,
    storage: FileStorage,
    theme_mode: ThemeMode,
) -> NavicatMain {
    let auto_check_updates = controller.state().settings.auto_check_updates;
    let rename_group_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.group_name")));
    let ai_session_filter_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_session")));
    let ai_session_filter_subscription = cx.subscribe(
        &ai_session_filter_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.ai_session_filter = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let ai_audit_filter_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_audit")));
    let ai_audit_filter_subscription = cx.subscribe(
        &ai_audit_filter_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.ai_audit_filter_text = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let ai_session_rename_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.session_name")));
    let ai_session_rename_subscription = cx.subscribe(
        &ai_session_rename_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                let value = input.read(cx).value().to_string();
                if let Some(pending) = &mut this.pending_ai_session_rename {
                    pending.title = value;
                }
            }
        },
    );
    let rename_group_subscription = cx.subscribe(
        &rename_group_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                let value = input.read(cx).value().to_string();
                if let Some(pending) = &mut this.pending_rename_group {
                    pending.name = value;
                }
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_rename_group(cx);
            }
        },
    );
    let table_folder_rename_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.group_name")));
    let table_folder_rename_subscription = cx.subscribe(
        &table_folder_rename_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                let value = input.read(cx).value().to_string();
                if let Some(pending) = &mut this.pending_rename_table_folder {
                    pending.name = value;
                }
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_rename_table_folder(cx);
            }
        },
    );
    let display_database_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_database")));
    let display_database_search_subscription = cx.subscribe(
        &display_database_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.display_database_search = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let create_database_name_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.database_name")));
    let create_database_name_subscription = cx.subscribe_in(
        &create_database_name_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_create_database
            {
                form.database_name = input.read(cx).value().to_string();
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_create_database(window, cx);
            }
        },
    );
    let create_database_charset_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(create_database_charset_options()),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let create_database_collation_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(create_database_collation_options("utf8mb4")),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let create_database_first_collection_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr("app.mongo.create_database.first_collection_placeholder"))
    });
    let create_database_first_collection_subscription = cx.subscribe_in(
        &create_database_first_collection_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_create_database
            {
                form.first_collection = input.read(cx).value().to_string();
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_create_database(window, cx);
            }
        },
    );
    let create_collection_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.mongo.create_collection.placeholder"))
    });
    let create_collection_subscription = cx.subscribe_in(
        &create_collection_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_create_collection
            {
                form.collection = input.read(cx).value().to_string();
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_create_collection(window, cx);
            }
        },
    );
    let create_schema_name_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.schema_name")));
    let create_schema_name_subscription = cx.subscribe_in(
        &create_schema_name_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(pending) = &mut this.pending_create_schema
            {
                pending.2 = input.read(cx).value().to_string();
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_create_schema(window, cx);
            }
        },
    );
    let create_database_owner_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.owner_example")));
    let create_database_owner_subscription = cx.subscribe_in(
        &create_database_owner_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_create_database
            {
                form.owner = input.read(cx).value().to_string();
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_create_database(window, cx);
            }
        },
    );
    let create_database_template_input = cx
        .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.template_example")));
    let create_database_template_subscription = cx.subscribe_in(
        &create_database_template_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_create_database
            {
                form.template = input.read(cx).value().to_string();
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_create_database(window, cx);
            }
        },
    );
    let danger_table_foreign_key_check_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(danger_table_foreign_key_check_options()),
            danger_table_foreign_key_check_index(ForeignKeyCheckMode::Default),
            window,
            cx,
        )
    });
    let rename_table_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.table_name")));
    let rename_table_subscription = cx.subscribe_in(
        &rename_table_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_rename_table
            {
                form.new_name = input.read(cx).value().to_string();
                form.error = None;
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_rename_table(window, cx);
            }
        },
    );
    let rename_collection_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.mongo.rename.field_placeholder"))
    });
    let rename_collection_subscription = cx.subscribe_in(
        &rename_collection_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_rename_collection
            {
                form.new_name = input.read(cx).value().to_string();
                form.error = None;
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_rename_collection(window, cx);
            }
        },
    );
    let copy_table_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.table_name")));
    let copy_table_subscription = cx.subscribe_in(
        &copy_table_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_copy_table
            {
                form.new_name = input.read(cx).value().to_string();
                form.error = None;
                cx.notify();
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_copy_table(window, cx);
            }
        },
    );
    let column_choice_value_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.choice_value")));
    let column_choice_label_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.choice_label")));
    let query_save_name_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.query_name")));
    let sidebar_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_table")));
    let sidebar_search_subscription = cx.subscribe(
        &sidebar_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.sidebar_search = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let ai_model_dialog_filter_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_model")));
    let ai_model_dialog_filter_subscription = cx.subscribe(
        &ai_model_dialog_filter_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.ai_model_dialog_filter = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let ai_model_filter_input = cx
        .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_model_name")));
    let ai_model_filter_subscription = cx.subscribe(
        &ai_model_filter_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.ai_model_filter = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let field_filter_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_field")));
    let field_filter_search_subscription = cx.subscribe(
        &field_filter_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_field_filter_search(input.read(cx).value().to_string(), cx);
            }
        },
    );
    let data_filter_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search")));
    let data_filter_value_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.filter_value")));
    let data_filter_batch_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.filter_batch")));
    let local_filter_value_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.filter_value")));
    let local_filter_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search")));
    let data_filter_text_input = cx.new(|cx| {
        gpui_component::input::EditorState::new(window, cx)
            .language(SQL_HIGHLIGHT_LANGUAGE)
            .placeholder(tr("app.placeholder.filter_expression"))
    });
    let data_sort_text_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.sort_fields")));
    let data_sql_panel_input = cx.new(|cx| InputState::new(window, cx));
    let data_page_input = cx.new(|cx| InputState::new(window, cx));
    let settings_line_height_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.placeholder.line_height_example"))
    });
    let settings_radius_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.radius_example")));
    // 留空即使用内置官方源，占位符直接展示默认模板，便于用户改成内网镜像。
    let settings_pg_client_source_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(fluxdb_app::PG_CLIENT_DEFAULT_SOURCE));
    // 留空即使用内置官方源，占位符直接展示默认模板，便于用户改成内网镜像。
    let settings_mysql_client_source_input = cx
        .new(|cx| InputState::new(window, cx).placeholder(fluxdb_app::MYSQL_CLIENT_DEFAULT_SOURCE));
    let data_cell_edit_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.input_value")));
    let temporal_part_input = cx.new(|cx| InputState::new(window, cx));
    let cell_detail_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.edit_cell")));
    let data_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_content")));
    let redis_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search")));
    let redis_key_value_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.value")));
    // Redis JSON 值组件：Edit 模式使用组件内部的多行编辑框。
    let redis_json_editor =
        JsonComponent::new_with_mode(JsonEditorConfig::default(), JsonMode::Preview, window, cx);
    let redis_json_key_value_input = redis_json_editor
        .input()
        .expect("JSON 组件应持有编辑框")
        .clone();
    let redis_key_name_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.key_name")));
    let redis_key_ttl_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.redis.form.ttl")));
    let redis_stream_entry_id_input = cx.new(|cx| InputState::new(window, cx).placeholder("*"));
    let redis_stream_entry_field_rows = vec![new_redis_stream_entry_field_inputs(window, cx)];
    let redis_set_member_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search")));
    let redis_set_member_search_subscription = cx.subscribe_in(
        &redis_set_member_search_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            // 程序化 set_value 触发的 Change（syncing=true）不算用户输入，
            // 避免与初始打开/Enter 重复发起搜索。
            if matches!(event, InputEvent::Change) && !this.redis_set_member_search_syncing {
                let _ = this.active_redis_selected_key(cx).map(|(tab_id, key)| {
                    // 切换 key 时可能残留携带旧 key 的 Change 事件：
                    // 只当捕获 key 仍是当前 panel 的 key 时才排入防抖，
                    // 避免 350ms 后对已不展示的旧键误发首屏搜索（WRONGTYPE）。
                    if this.redis_set_member_search_active.as_ref() != Some(&(tab_id, key.clone()))
                    {
                        return;
                    }
                    let query = input.read(cx).value().to_string();
                    this.redis_set_member_search_queries
                        .insert((tab_id, key.clone()), query.clone());
                    this.schedule_redis_set_member_search(tab_id, key, query, cx);
                });
            }
            // Enter：立即发起搜索（取消在飞防抖）。
            if matches!(event, InputEvent::PressEnter { .. }) {
                if let Some((tab_id, key)) = this.active_redis_selected_key(cx) {
                    if this.redis_set_member_search_active.as_ref() != Some(&(tab_id, key.clone()))
                    {
                        return;
                    }
                    this.redis_set_member_search_debounce = None;
                    this.redis_set_member_search_debounce_until = None;
                    let query = input.read(cx).value().to_string();
                    this.redis_set_member_search_queries
                        .insert((tab_id, key.clone()), query.clone());
                    this.request_redis_set_member_search(tab_id, key, query, String::new(), cx);
                }
            }
        },
    );
    let redis_hash_field_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_field")));
    let redis_hash_value_edit_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.redis.form.value")));
    let redis_hash_ttl_edit_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.redis.form.ttl")));
    let redis_hash_field_search_subscription = cx.subscribe_in(
        &redis_hash_field_search_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) && !this.redis_hash_field_search_syncing {
                // 搜索字段变化时自动关闭完整值内嵌面板
                this.close_redis_hash_full_value_viewer(cx);
                // 优先用面板同步出来的 active key，避免选中行切换时读到旧行。
                let active_key = this
                    .redis_hash_field_search_active
                    .clone()
                    .or_else(|| this.active_redis_selected_key(cx));
                let Some((tab_id, key)) = active_key else {
                    return;
                };
                let query = input.read(cx).value().to_string();
                this.redis_hash_field_search_queries
                    .insert((tab_id, key.clone()), query.clone());
                this.schedule_redis_hash_field_search(tab_id, key, query, cx);
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                let active_key = this
                    .redis_hash_field_search_active
                    .clone()
                    .or_else(|| this.active_redis_selected_key(cx));
                let Some((tab_id, key)) = active_key else {
                    return;
                };
                this.redis_hash_field_search_debounce = None;
                this.redis_hash_field_search_debounce_until = None;
                let query = input.read(cx).value().to_string();
                this.redis_hash_field_search_queries
                    .insert((tab_id, key.clone()), query.clone());
                this.request_redis_hash_field_search(tab_id, key, query, String::new(), cx);
            }
        },
    );
    let redis_hash_value_edit_subscription = cx.subscribe_in(
        &redis_hash_value_edit_input,
        window,
        |_this: &mut NavicatMain, _input, _event: &InputEvent, _window, _cx| {},
    );
    let redis_hash_ttl_edit_subscription = cx.subscribe_in(
        &redis_hash_ttl_edit_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                let current = input.read(cx).value().to_string();
                let expected = redis_ttl_input_value(&current);
                if current != expected {
                    input.update(cx, |input, cx| {
                        input.set_value(expected, window, cx);
                    });
                }
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_redis_hash_field_edit(cx);
            }
        },
    );
    let redis_zset_member_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_member")));
    let redis_zset_member_search_subscription = cx.subscribe_in(
        &redis_zset_member_search_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) && !this.redis_zset_member_search_syncing {
                let _ = this.active_redis_selected_key(cx).map(|(tab_id, key)| {
                    let query = input.read(cx).value().to_string();
                    this.redis_zset_member_search_queries
                        .insert((tab_id, key.clone()), query.clone());
                    this.schedule_redis_zset_member_search(tab_id, key, query, cx);
                });
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                if let Some((tab_id, key)) = this.active_redis_selected_key(cx) {
                    this.redis_zset_member_search_debounce = None;
                    this.redis_zset_member_search_debounce_until = None;
                    let query = input.read(cx).value().to_string();
                    this.redis_zset_member_search_queries
                        .insert((tab_id, key.clone()), query.clone());
                    this.request_redis_zset_member_search(tab_id, key, query, String::new(), cx);
                }
            }
        },
    );
    let redis_list_item_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.index_search")));
    let redis_list_item_search_subscription = cx.subscribe_in(
        &redis_list_item_search_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            // 输入框只允许数字：Change 时即时剔除非数字字符（重写为数字串）。
            // 搜索不做键入自动触发，仅在 Enter 时「按索引跳转」（对齐 RedisInsight）。
            if matches!(event, InputEvent::Change) && !this.redis_list_item_search_syncing {
                let current = input.read(cx).value().to_string();
                let sanitized = redis_ttl_input_value(&current);
                if current != sanitized {
                    input.update(cx, |input, cx| {
                        input.set_value(sanitized, window, cx);
                    });
                }
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                if let Some((tab_id, key)) = this.active_redis_selected_key(cx) {
                    // 按索引跳转会改变当前页行集，先取消进行中的行内编辑避免错位
                    this.cancel_redis_list_item_edit(cx);
                    let query = input.read(cx).value().to_string();
                    this.redis_list_item_search_queries
                        .insert((tab_id, key.clone()), query.clone());
                    this.redis_list_item_search_active = Some((tab_id, key.clone()));
                    this.request_redis_list_item_search(tab_id, key, query, String::new(), cx);
                }
            }
        },
    );
    let redis_list_value_edit_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.redis.form.value")));
    let redis_list_value_edit_subscription = cx.subscribe_in(
        &redis_list_value_edit_input,
        window,
        |this: &mut NavicatMain, _input, event: &InputEvent, _window, cx| {
            // 保持编辑输入与进行中的编辑行同步：切 key/标签页时由
            // sync 逻辑清理；按下 Enter 确认。取消由编辑弹层内的
            // CancelDialog action 处理（全局 Escape 键绑定）。
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_redis_list_item_edit(cx);
            }
        },
    );
    let redis_list_item_remove_count_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr("app.placeholder.quantity"))
            .default_value("1")
    });
    // List 删除位置下拉（对齐 RedisInsight）：默认选中第一个「Remove from tail」。
    let redis_list_item_remove_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(redis_list_remove_position_options()),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let redis_list_item_remove_select_subscription = cx.subscribe_in(
        &redis_list_item_remove_select,
        window,
        |this: &mut NavicatMain,
         _select,
         _event: &SelectEvent<SearchableVec<RedisListRemovePositionOption>>,
         _window,
         cx| {
            // 位置状态由 Select 自身持有，确认时直接读取 selected_value；这里仅触发重绘。
            this.redis_list_item_remove_drawer
                .as_ref()
                .map(|_| cx.notify());
        },
    );
    let redis_list_item_remove_count_subscription = cx.subscribe_in(
        &redis_list_item_remove_count_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            // 数量只允许数字：Change 时即时剔除非数字字符（复用 TTL 数字清洗）。
            if matches!(event, InputEvent::Change) && !this.redis_list_item_search_syncing {
                let current = input.read(cx).value().to_string();
                let sanitized = redis_ttl_input_value(&current);
                if current != sanitized {
                    input.update(cx, |input, cx| {
                        input.set_value(sanitized, window, cx);
                    });
                }
                // 数量变化后撤销悬空的删除二次确认，避免确认弹出时与实际输入不一致。
                this.redis_list_item_remove_confirm = None;
            }
        },
    );
    // 「新增 Key」抽屉（对齐 RedisInsight AddKey）的输入与类型下拉。
    let redis_add_key_type_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(
                REDIS_ADD_KEY_TYPES
                    .iter()
                    .map(|item| item.to_string())
                    .collect::<Vec<String>>(),
            ),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let redis_add_key_type_select_subscription = cx.subscribe_in(
        &redis_add_key_type_select,
        window,
        |this: &mut NavicatMain,
         _select,
         _event: &SelectEvent<SearchableVec<String>>,
         window,
         cx| {
            // 类型切换：仅抽屉开启时重置切换到的新类型的子表单依赖字段。
            if this.redis_add_key_drawer.is_some() {
                this.reset_redis_add_key_active_form(window, cx);
                cx.notify();
            }
        },
    );
    let redis_add_key_name_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.redis_key_name")));
    let redis_add_key_name_subscription = cx.subscribe_in(
        &redis_add_key_name_input,
        window,
        |this: &mut NavicatMain, _input, _event: &InputEvent, _window, _cx| {
            this.redis_add_key_applying = false;
        },
    );
    let redis_add_key_ttl_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.ttl")));
    let redis_add_key_ttl_subscription = cx.subscribe_in(
        &redis_add_key_ttl_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            // TTL 只允许数字：Change 时即时剔除非数字字符（复用 TTL 数字清洗）。
            if matches!(event, InputEvent::Change) && !this.redis_list_item_search_syncing {
                let current = input.read(cx).value().to_string();
                let sanitized = redis_ttl_input_value(&current);
                if current != sanitized {
                    input.update(cx, |input, cx| {
                        input.set_value(sanitized, window, cx);
                    });
                }
            }
        },
    );
    // 按类型子表单输入：String 单值 / JSON 大文本 / Stream entry id（多行成员走动态行）。
    let redis_add_key_string_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.value")));
    let redis_add_key_string_subscription = cx.subscribe_in(
        &redis_add_key_string_input,
        window,
        |this: &mut NavicatMain, _input, _event: &InputEvent, _window, _cx| {
            this.redis_add_key_applying = false;
        },
    );
    let redis_add_key_json_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.json_object")));
    let redis_add_key_json_subscription = cx.subscribe_in(
        &redis_add_key_json_input,
        window,
        |this: &mut NavicatMain, _input, _event: &InputEvent, _window, _cx| {
            this.redis_add_key_applying = false;
        },
    );
    let redis_add_key_stream_id_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.auto_generate")));
    let redis_add_key_stream_id_subscription = cx.subscribe_in(
        &redis_add_key_stream_id_input,
        window,
        |this: &mut NavicatMain, _input, _event: &InputEvent, _window, _cx| {
            this.redis_add_key_applying = false;
        },
    );
    let navicat_view = cx.entity().downgrade();
    let redis_stream_table_state = cx.new(|cx| {
        let mut state = TableState::new(
            RedisStreamTableDelegate::new(navicat_view.clone()),
            window,
            cx,
        );
        state.row_selectable = false;
        state.col_selectable = false;
        state
    });
    // gpui-component 的 TableState 在点击/键盘导航时不会检查 row_selectable，
    // Stream 表格只做展示与行内删除，不允许出现行/列选中态。
    // 选中事件是在 TableState 自身的 update（lease）回调里同步派发的，
    // 此时再 update 同一个表格会触发 double-lease panic，
    // 因此用 defer 延迟到本帧 effect 结束后再清空选中。
    cx.subscribe(
        &redis_stream_table_state,
        |_, table, event: &TableEvent, cx| {
            if matches!(
                event,
                TableEvent::SelectRow(_) | TableEvent::SelectColumn(_)
            ) {
                let table = table.downgrade();
                cx.defer(move |cx| {
                    if let Some(table) = table.upgrade() {
                        table.update(cx, |table, cx| {
                            table.clear_selection(cx);
                        });
                    }
                });
            }
        },
    )
    .detach();
    let redis_hash_table_state = cx.new(|cx| {
        let mut state = TableState::new(
            RedisHashTableDelegate::new(navicat_view.clone()),
            window,
            cx,
        );
        state.row_selectable = false;
        state.col_selectable = false;
        state
    });
    // Hash 明细表只做展示与行内编辑，不允许出现行/列选中态，
    // 清理方式与 Stream 表格一致（见上方注释）。
    cx.subscribe(
        &redis_hash_table_state,
        |_, table, event: &TableEvent, cx| {
            if matches!(
                event,
                TableEvent::SelectRow(_) | TableEvent::SelectColumn(_)
            ) {
                let table = table.downgrade();
                cx.defer(move |cx| {
                    if let Some(table) = table.upgrade() {
                        table.update(cx, |table, cx| {
                            table.clear_selection(cx);
                        });
                    }
                });
            }
        },
    )
    .detach();
    let redis_list_table_state = cx.new(|cx| {
        let mut state = TableState::new(
            RedisListTableDelegate::new(navicat_view.clone()),
            window,
            cx,
        );
        state.row_selectable = false;
        state.col_selectable = false;
        state
    });
    // List 明细表只做展示与行内编辑，不允许出现行/列选中态，
    // 清理方式与 Hash 表格一致（见上方注释）。
    cx.subscribe(
        &redis_list_table_state,
        |_, table, event: &TableEvent, cx| {
            if matches!(
                event,
                TableEvent::SelectRow(_) | TableEvent::SelectColumn(_)
            ) {
                let table = table.downgrade();
                cx.defer(move |cx| {
                    if let Some(table) = table.upgrade() {
                        table.update(cx, |table, cx| {
                            table.clear_selection(cx);
                        });
                    }
                });
            }
        },
    )
    .detach();
    let redis_type_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(redis_type_filter_options()),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let row_detail_search_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.placeholder.search_field_value"))
    });
    let tab_switcher_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_tab")));
    let query_history_search_input = cx
        .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_sql_scope")));
    let query_history_quick_search_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.placeholder.search_executed_sql"))
    });
    let redis_history_search_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.placeholder.search_command_scope"))
    });
    let query_history_connection_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(query_history_connection_filter_items(controller.state())),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
        .searchable(true)
    });
    let query_history_database_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(query_history_database_filter_items(
                controller.state(),
                None,
            )),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
        .searchable(true)
    });
    let query_history_table_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(query_history_table_filter_items(
                controller.state(),
                None,
                None,
            )),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
        .searchable(true)
    });
    let first_sql_file_connection = controller
        .state()
        .connections
        .first()
        .map(|connection| connection.config.id);
    let sql_file_connection_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(sql_file_connection_items(controller.state())),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
        .searchable(true)
    });
    let sql_file_database_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(
                first_sql_file_connection
                    .map(|connection_id| sql_file_database_items(controller.state(), connection_id))
                    .unwrap_or_default(),
            ),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
        .searchable(true)
    });
    let sql_file_encoding_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(sql_file_encoding_items()),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let sql_file_path_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.choose_sql_file")));
    let backup_file_name_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.placeholder.backup_filename_default"))
    });
    // 备份目录输入：打开弹框时填入设置 backup_dir，可手动更换。
    let backup_target_dir_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.placeholder.backup_dir_default"))
    });
    let backup_object_search_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_table")));
    // 新建备份弹框：备注输入（成功后写入 .meta.json）。
    let backup_note_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.note_optional")));
    // 备份 tab：备注编辑弹框输入。
    let backup_note_edit_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.note_clear")));
    let user_admin_search_input = cx
        .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.search_user_host")));
    let user_admin_create_user_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.username")));
    let user_admin_create_host_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.user_admin.advanced.host")));
    let user_admin_auth_plugin_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(user_admin_auth_plugin_options()),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
        .searchable(true)
    });
    let user_admin_password_expiry_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(user_admin_password_expiry_options()),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let user_admin_create_password_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr("app.placeholder.confirm_password"))
            .masked(true)
    });
    let user_admin_new_password_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr("app.placeholder.password"))
            .masked(true)
    });
    // PG 用户与角色工作台（改版）：常规/高级/权限页输入与选择器。
    let pg_role_name_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.role_name")));
    let pg_password_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr("app.placeholder.new_password"))
            .masked(true)
    });
    let pg_password_confirm_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr("app.placeholder.confirm_new_password"))
            .masked(true)
    });
    let pg_connection_limit_input = cx.new(|cx| InputState::new(window, cx).placeholder("-1"));
    let pg_valid_until_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.placeholder.valid_until_example"))
    });
    let pg_valid_until_mode_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(pg_valid_until_mode_options()),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let pg_password_op_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(pg_password_op_options(true)),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let pg_grant_db_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    });
    let pg_grant_schema_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    });
    let pg_grant_object_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    });
    let pg_member_of_role_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    });
    let sqlserver_security_database_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    });
    let sqlserver_security_name_input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("app.sqlserver_security.name_placeholder"))
    });
    let sqlserver_security_password_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr("app.placeholder.password"))
            .masked(true)
    });
    let sqlserver_security_confirm_password_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr("app.placeholder.confirm_password"))
            .masked(true)
    });
    let sqlserver_security_login_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    });
    let sqlserver_security_default_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
    });
                        let sqlserver_security_permission_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.placeholder.permission_name"))
                        });
                        let sqlserver_security_permission_target_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.placeholder.permission_target"))
                        });
                        // Oracle 用户与权限：新建账号草稿字段。
                        let oracle_security_name_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.name_placeholder"))
                        });
                        let oracle_security_password_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.placeholder.password"))
                                .masked(true)
                        });
                        let oracle_security_confirm_password_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.placeholder.confirm_password"))
                                .masked(true)
                        });
                        // 改密码独立于新建草稿，避免同步把正在输入的口令清掉。
                        let oracle_security_new_password_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.new_password_placeholder"))
                                .masked(true)
                        });
                        let oracle_security_new_password_confirm_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.placeholder.confirm_password"))
                                .masked(true)
                        });
                        let oracle_security_default_ts_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.tablespace_placeholder"))
                        });
                        let oracle_security_temp_ts_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.tablespace_placeholder"))
                        });
                        let oracle_security_profile_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.profile_placeholder"))
                        });
                        let oracle_security_quota_ts_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.tablespace_placeholder"))
                        });
                        let oracle_security_quota_bytes_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.quota_placeholder"))
                        });
                        let oracle_security_grant_name_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.grant_name_placeholder"))
                        });
                        let oracle_security_grant_owner_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.grant_owner_placeholder"))
                        });
                        let oracle_security_grant_object_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(tr("app.oracle_security.grant_object_placeholder"))
                        });
                        let user_admin_max_queries_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("0"));
                        let user_admin_max_updates_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("0"));
    let user_admin_max_connections_input =
        cx.new(|cx| InputState::new(window, cx).placeholder("0"));
    let user_admin_max_user_connections_input =
        cx.new(|cx| InputState::new(window, cx).placeholder("0"));
    let user_admin_ssl_type_select = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(user_admin_ssl_type_options()),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
    });
    let user_admin_ssl_cipher_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.user_admin.advanced.cipher")));
    let user_admin_ssl_issuer_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.cert_issuer")));
    let user_admin_ssl_subject_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.cert_subject")));
    let data_sql_panel_subscription = cx.subscribe(
        &data_sql_panel_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_data_editor_sql_text(input.read(cx).value().to_string(), cx);
            }
            if matches!(event, InputEvent::PressEnter { .. })
                && let Some(tab_id) = this.active_data_filter_tab_id()
            {
                this.apply_data_filter_and_sort(tab_id, cx);
            }
        },
    );
    let data_search_subscription = cx.subscribe(
        &data_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_data_search_query(input.read(cx).value().to_string(), cx);
            }
            if matches!(event, InputEvent::PressEnter { .. })
                && let Some(tab_id) = this.active_data_editor_tab_id()
            {
                this.select_next_data_search_match(tab_id, cx);
            }
        },
    );
    let row_detail_search_subscription = cx.subscribe(
        &row_detail_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.row_detail_search = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let data_page_subscription = cx.subscribe_in(
        &data_page_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.apply_data_page_input(input.read(cx).value().to_string(), window, cx);
            }
        },
    );
    let settings_line_height_subscription = cx.subscribe_in(
        &settings_line_height_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            if !matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                return;
            }
            let Some(value) = input
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .ok()
                .map(|value| value.clamp(13, 28))
            else {
                return;
            };
            this.settings_editor_draft.editor_line_height = value;
            cx.notify();
        },
    );
    let settings_radius_subscription = cx.subscribe_in(
        &settings_radius_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            if !matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                return;
            }
            let Ok(value) = input.read(cx).value().trim().parse::<u8>() else {
                return;
            };
            let value = value.min(24);
            this.settings_editor_draft.button_radius = value;
            this.settings_editor_draft.large_radius = if value == 0 {
                0
            } else {
                value.saturating_add(2).min(32)
            };
            this.preview_settings(cx);
        },
    );
    // 下载源是纯文本设置，随输入写入草稿，由「保存」统一落盘。
    let settings_pg_client_source_subscription = cx.subscribe_in(
        &settings_pg_client_source_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            if !matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                return;
            }
            this.settings_editor_draft.pg_client_download_source =
                input.read(cx).value().trim().to_string();
            cx.notify();
        },
    );
    // 下载源是纯文本设置，随输入写入草稿，由「保存」统一落盘。
    let settings_mysql_client_source_subscription = cx.subscribe_in(
        &settings_mysql_client_source_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
            if !matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                return;
            }
            this.settings_editor_draft.mysql_client_download_source =
                input.read(cx).value().trim().to_string();
            cx.notify();
        },
    );
    let data_cell_edit_subscription = cx.subscribe_in(
        &data_cell_edit_input,
        window,
        |this: &mut NavicatMain, _input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(editing) = this.data_cell_editing
                && editing.temporal_kind.is_some()
            {
                let tab_id = editing.tab_id;
                cx.defer_in(window, move |this, _, cx| {
                    this.refresh_active_data_table(tab_id, cx);
                });
            }
            if data_cell_edit_event_should_commit(event) {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_data_cell_edit(cx);
                } else if let Some(editing) = this.data_cell_editing
                    && let Some(meta) = this.data_cell_meta_for_edit(editing)
                    && data_cell_blur_should_commit(&meta, editing)
                {
                    this.commit_data_cell_edit(cx);
                }
            }
        },
    );
    let temporal_part_subscription = cx.subscribe_in(
        &temporal_part_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                this.apply_temporal_part_input(
                    input.read(cx).value().to_string().as_str(),
                    window,
                    cx,
                );
            }
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                this.temporal_part_editing = None;
                cx.notify();
            }
            InputEvent::Focus => {}
        },
    );
    let cell_detail_subscription = cx.subscribe(
        &cell_detail_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let Some(tab_id) = this.controller.state().active_tab else {
                return;
            };
            let is_editing_detail =
                this.controller
                    .state()
                    .active_tab()
                    .is_some_and(|tab| match &tab.kind {
                        TabKind::DataEditor(editor) => {
                            editor.cell_detail_panel.mode == CellDetailMode::Edit
                        }
                        TabKind::QueryEditor(editor) => active_query_result_editor_state(editor)
                            .is_some_and(|editor| {
                                editor.cell_detail_panel.mode == CellDetailMode::Edit
                            }),
                        _ => false,
                    });
            if is_editing_detail {
                this.dispatch(
                    AppCommand::UpdateCellDetailEditValue {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let tab_switcher_search_subscription = cx.subscribe(
        &tab_switcher_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.tab_switcher_search = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let query_history_search_subscription = cx.subscribe(
        &query_history_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.query_history_search = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let query_history_quick_search_subscription = cx.subscribe(
        &query_history_quick_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                this.query_history_quick_search = input.read(cx).value().to_string();
                this.query_history_quick_selected = 0;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                this.confirm_query_history_quick_search(cx);
            }
            InputEvent::Focus | InputEvent::Blur => {}
        },
    );
    let redis_history_search_subscription = cx.subscribe(
        &redis_history_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.redis_history_search = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let redis_search_subscription = cx.subscribe(
        &redis_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if let Some(tab_id) = this.active_redis_data_tab_id() {
                match event {
                    InputEvent::Change => {
                        this.redis_search_drafts
                            .insert(tab_id, input.read(cx).value().to_string());
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        this.apply_redis_search(tab_id, cx);
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                }
            }
        },
    );
    let redis_key_value_subscription = cx.subscribe(
        &redis_key_value_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            if this.redis_key_value_syncing {
                return;
            }
            let Some((tab_id, key)) = this.active_redis_selected_key(cx) else {
                return;
            };
            this.redis_key_value_drafts
                .insert((tab_id, key), input.read(cx).value().to_string());
            cx.notify();
        },
    );
    let redis_json_key_value_subscription = cx.subscribe(
        &redis_json_key_value_input,
        |this: &mut NavicatMain, _input, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            if this.redis_key_value_syncing {
                return;
            }
            let Some((tab_id, key)) = this.active_redis_selected_key(cx) else {
                return;
            };
            this.on_redis_json_input_change(tab_id, &key, cx);
        },
    );
    let redis_key_name_subscription = cx.subscribe(
        &redis_key_name_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                if this.redis_key_meta_syncing {
                    return;
                }
                let Some((tab_id, key)) = this.active_redis_selected_key(cx) else {
                    return;
                };
                this.redis_key_name_drafts
                    .insert((tab_id, key), input.read(cx).value().to_string());
                cx.notify();
            }
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                this.redis_key_meta_editing = None;
                cx.notify();
            }
            InputEvent::Focus => {}
        },
    );
    let redis_key_ttl_subscription = cx.subscribe_in(
        &redis_key_ttl_input,
        window,
        |this: &mut NavicatMain, input, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                if this.redis_key_meta_syncing {
                    return;
                }
                let Some((tab_id, key)) = this.active_redis_selected_key(cx) else {
                    return;
                };
                let raw = input.read(cx).value().to_string();
                let value = raw
                    .chars()
                    .filter(|ch| ch.is_ascii_digit())
                    .collect::<String>();
                if value != raw {
                    this.redis_key_meta_syncing = true;
                    input.update(cx, |input, cx| {
                        input.set_value(value.clone(), window, cx);
                    });
                    this.redis_key_meta_syncing = false;
                }
                this.redis_key_ttl_drafts.insert((tab_id, key), value);
                cx.notify();
            }
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                this.redis_key_meta_editing = None;
                cx.notify();
            }
            InputEvent::Focus => {}
        },
    );
    let redis_type_select_subscription = cx.subscribe_in(
        &redis_type_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<RedisTypeFilterOption>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(value)) = (this.active_redis_data_tab_id(), value) {
                this.set_redis_type_filter(tab_id, value.value().clone(), cx);
            }
        },
    );
    let query_history_connection_select_subscription = cx.subscribe_in(
        &query_history_connection_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<QueryHistoryConnectionFilterItem>>,
         window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            this.query_history_connection_filter = value.flatten();
            this.query_history_database_filter = None;
            this.query_history_table_filter = None;
            this.refresh_query_history_filter_selects(window, cx);
        },
    );
    let query_history_database_select_subscription = cx.subscribe_in(
        &query_history_database_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<QueryHistoryTextFilterItem>>,
         window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            this.query_history_database_filter = value.clone().flatten();
            this.query_history_table_filter = None;
            this.refresh_query_history_filter_selects(window, cx);
        },
    );
    let query_history_table_select_subscription = cx.subscribe_in(
        &query_history_table_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<QueryHistoryTextFilterItem>>,
         window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            this.query_history_table_filter = value.clone().flatten();
            this.refresh_query_history_filter_selects(window, cx);
        },
    );
    let sql_file_connection_select_subscription = cx.subscribe_in(
        &sql_file_connection_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<SqlFileConnectionItem>>,
         window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let Some(connection_id) = value {
                // 先写入并释放 RefCell 借用，再调用 refresh（内部会再次借用）
                let matched = {
                    let mut data = this.sql_file_modal.borrow_mut();
                    if let Some(form) = &mut data.form {
                        form.connection_id = *connection_id;
                        form.database = this
                            .controller
                            .state()
                            .connections
                            .iter()
                            .find(|connection| connection.config.id == *connection_id)
                            .and_then(|connection| connection_default_database(&connection.config));
                        true
                    } else {
                        false
                    }
                };
                if matched {
                    this.refresh_sql_file_selects(window, cx);
                }
            }
        },
    );
    let sql_file_database_select_subscription = cx.subscribe_in(
        &sql_file_database_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<SqlFileDatabaseItem>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let Some(form) = &mut this.sql_file_modal.borrow_mut().form {
                form.database = value.clone().flatten();
                cx.notify();
            }
        },
    );
    let sql_file_encoding_select_subscription = cx.subscribe_in(
        &sql_file_encoding_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<SqlFileEncodingItem>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(encoding), Some(form)) =
                (value, &mut this.sql_file_modal.borrow_mut().form)
            {
                form.encoding = *encoding;
                cx.notify();
            }
        },
    );
    let sql_file_path_subscription = cx.subscribe(
        &sql_file_path_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = input.read(cx).value().trim().to_string();
                if let Some(form) = &mut this.sql_file_modal.borrow_mut().form {
                    form.path = if value.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(value))
                    };
                    cx.notify();
                }
            }
        },
    );
    let backup_file_name_subscription = cx.subscribe(
        &backup_file_name_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_backup_modal
            {
                form.file_name = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    // 备份目录输入 → form.target_dir（最终用于备份输出路径）。
    let backup_target_dir_subscription = cx.subscribe(
        &backup_target_dir_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_backup_modal
            {
                form.target_dir = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let backup_object_search_subscription = cx.subscribe(
        &backup_object_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_backup_modal
            {
                form.object_search = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let backup_note_subscription = cx.subscribe(
        &backup_note_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(form) = &mut this.pending_backup_modal
            {
                form.note = input.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    let user_admin_search_subscription = cx.subscribe(
        &user_admin_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminSearch {
                        tab_id,
                        search: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_create_user_subscription = cx.subscribe(
        &user_admin_create_user_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminCreateUser {
                        tab_id,
                        user: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_create_host_subscription = cx.subscribe(
        &user_admin_create_host_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminCreateHost {
                        tab_id,
                        host: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_auth_plugin_select_subscription = cx.subscribe_in(
        &user_admin_auth_plugin_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(plugin)) = (this.active_user_admin_tab_id(), value) {
                this.dispatch(
                    AppCommand::SetUserAdminAuthPlugin {
                        tab_id,
                        plugin: plugin.clone(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_password_expiry_select_subscription = cx.subscribe_in(
        &user_admin_password_expiry_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(policy)) = (this.active_user_admin_tab_id(), value) {
                this.dispatch(
                    AppCommand::SetUserAdminPasswordExpiryPolicy {
                        tab_id,
                        policy: policy.clone(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_create_password_subscription = cx.subscribe(
        &user_admin_create_password_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminCreatePassword {
                        tab_id,
                        password: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    // ===== PG 用户与角色工作台（改版）订阅 =====
    let pg_role_name_subscription = cx.subscribe(
        &pg_role_name_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetPgDraftName {
                        tab_id,
                        name: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    // 密码明文仅进入内存中的草稿（Set 操作），不写日志/历史。
    let pg_password_subscription = cx.subscribe(
        &pg_password_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetPgDraftPassword {
                        tab_id,
                        password: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    // 确认密码只留在桌面层做一致性校验，不进入 App 状态。
    let pg_password_confirm_subscription = cx.subscribe(
        &pg_password_confirm_input,
        |this: &mut NavicatMain, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.sync_pg_save_enabled(cx);
            }
        },
    );
    let pg_connection_limit_subscription = cx.subscribe(
        &pg_connection_limit_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetPgDraftConnectionLimit {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let pg_valid_until_subscription = cx.subscribe(
        &pg_valid_until_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetPgDraftValidUntil {
                        tab_id,
                        op: PgValidUntilOp::At(input.read(cx).value().to_string()),
                    },
                    cx,
                );
            }
        },
    );
    let valid_until_input_ref = pg_valid_until_input.clone();
    let pg_valid_until_mode_select_subscription = cx.subscribe_in(
        &pg_valid_until_mode_select,
        window,
        move |this: &mut NavicatMain,
              _select,
              event: &SelectEvent<SearchableVec<String>>,
              _window,
              cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(value)) = (this.active_user_admin_tab_id(), value) {
                let op = match value.as_str() {
                    label if pg_valid_until_label(&PgValidUntilOp::Clear) == label => {
                        PgValidUntilOp::Clear
                    }
                    // 切到自定义模式时以输入框当前文本作为截止时间。
                    label if pg_valid_until_label(&PgValidUntilOp::At(String::new())) == label => {
                        PgValidUntilOp::At(valid_until_input_ref.read(cx).value().to_string())
                    }
                    _ => PgValidUntilOp::Keep,
                };
                this.dispatch(AppCommand::SetPgDraftValidUntil { tab_id, op }, cx);
            }
        },
    );
    let pg_password_op_select_subscription = cx.subscribe_in(
        &pg_password_op_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(value)) = (this.active_user_admin_tab_id(), value) {
                let op = match value.as_str() {
                    label
                        if pg_password_op_label(true, &PgPasswordOp::Set(String::new()))
                            == label =>
                    {
                        PgPasswordOp::Set(String::new())
                    }
                    label if pg_password_op_label(false, &PgPasswordOp::Clear) == label => {
                        PgPasswordOp::Clear
                    }
                    // 「不设置密码」/「保持不变」= 不下发 PASSWORD 子句（Keep）。
                    _ => PgPasswordOp::Keep,
                };
                this.dispatch(AppCommand::SetPgDraftPasswordOp { tab_id, op }, cx);
            }
        },
    );
    let pg_grant_db_select_subscription = cx.subscribe_in(
        &pg_grant_db_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(db)) = (this.active_user_admin_tab_id(), value) {
                this.set_pg_grant_database(tab_id, db.clone(), cx);
            }
        },
    );
    let pg_grant_schema_select_subscription = cx.subscribe_in(
        &pg_grant_schema_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(schema)) = (this.active_user_admin_tab_id(), value) {
                this.set_pg_grant_schema(tab_id, schema.clone(), cx);
            }
        },
    );
    let pg_grant_object_select_subscription = cx.subscribe_in(
        &pg_grant_object_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(object)) = (this.active_user_admin_tab_id(), value) {
                this.set_pg_grant_object(tab_id, object.clone(), cx);
            }
        },
    );
    let pg_member_of_role_select_subscription = cx.subscribe_in(
        &pg_member_of_role_select,
        window,
        |_this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         _window,
         cx| {
            let SelectEvent::Confirm(_) = event;
            cx.notify();
        },
    );
    let sqlserver_security_database_subscription = cx.subscribe_in(
        &sqlserver_security_database_select,
        window,
        |this: &mut NavicatMain, _, event: &SelectEvent<SearchableVec<String>>, _, cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(database)) = (this.active_user_admin_tab_id(), value) {
                this.select_sqlserver_security_database(tab_id, database.clone(), cx);
            }
        },
    );
    let sqlserver_security_name_subscription = cx.subscribe(
        &sqlserver_security_name_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetSqlServerSecurityFormField {
                        tab_id,
                        field: SqlServerSecurityFormField::Name,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let sqlserver_security_password_subscription = cx.subscribe(
        &sqlserver_security_password_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetSqlServerSecurityFormField {
                        tab_id,
                        field: SqlServerSecurityFormField::Password,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let sqlserver_security_confirm_password_subscription = cx.subscribe(
        &sqlserver_security_confirm_password_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetSqlServerSecurityFormField {
                        tab_id,
                        field: SqlServerSecurityFormField::ConfirmPassword,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let sqlserver_security_login_subscription = cx.subscribe_in(
        &sqlserver_security_login_select,
        window,
        |this: &mut NavicatMain, _, event: &SelectEvent<SearchableVec<String>>, _, cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(value)) = (this.active_user_admin_tab_id(), value) {
                this.dispatch(
                    AppCommand::SetSqlServerSecurityFormField {
                        tab_id,
                        field: SqlServerSecurityFormField::Login,
                        value: value.clone(),
                    },
                    cx,
                );
            }
        },
    );
    let sqlserver_security_default_subscription = cx.subscribe_in(
        &sqlserver_security_default_select,
        window,
        |this: &mut NavicatMain, _, event: &SelectEvent<SearchableVec<String>>, _, cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(value)) = (this.active_user_admin_tab_id(), value) {
                this.dispatch(
                    AppCommand::SetSqlServerSecurityFormField {
                        tab_id,
                        field: SqlServerSecurityFormField::DefaultValue,
                        value: value.clone(),
                    },
                    cx,
                );
            }
        },
    );
    let sqlserver_security_permission_subscription = cx.subscribe(
        &sqlserver_security_permission_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetSqlServerSecurityFormField {
                        tab_id,
                        field: SqlServerSecurityFormField::Permission,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let sqlserver_security_permission_target_subscription = cx.subscribe(
        &sqlserver_security_permission_target_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetSqlServerSecurityFormField {
                        tab_id,
                        field: SqlServerSecurityFormField::PermissionTarget,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
                        // Oracle 输入订阅：口令只进草稿，不回显到任何快照。
                        let oracle_security_name_subscription = cx.subscribe(
                            &oracle_security_name_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_create_field_changed(
                                    input, event, fluxdb_app::OracleCreateDraftField::Name, cx,
                                );
                            },
                        );
                        let oracle_security_password_subscription = cx.subscribe(
                            &oracle_security_password_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_create_field_changed(
                                    input, event, fluxdb_app::OracleCreateDraftField::Password, cx,
                                );
                            },
                        );
                        let oracle_security_confirm_password_subscription = cx.subscribe(
                            &oracle_security_confirm_password_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_create_field_changed(
                                    input, event, fluxdb_app::OracleCreateDraftField::ConfirmPassword, cx,
                                );
                            },
                        );
                        let oracle_security_default_ts_subscription = cx.subscribe(
                            &oracle_security_default_ts_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_create_field_changed(
                                    input, event, fluxdb_app::OracleCreateDraftField::DefaultTablespace, cx,
                                );
                            },
                        );
                        let oracle_security_temp_ts_subscription = cx.subscribe(
                            &oracle_security_temp_ts_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_create_field_changed(
                                    input, event, fluxdb_app::OracleCreateDraftField::TemporaryTablespace, cx,
                                );
                            },
                        );
                        let oracle_security_profile_subscription = cx.subscribe(
                            &oracle_security_profile_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_create_field_changed(
                                    input, event, fluxdb_app::OracleCreateDraftField::Profile, cx,
                                );
                            },
                        );
                        let oracle_security_quota_ts_subscription = cx.subscribe(
                            &oracle_security_quota_ts_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_create_field_changed(
                                    input, event, fluxdb_app::OracleCreateDraftField::QuotaTablespace, cx,
                                );
                            },
                        );
                        let oracle_security_quota_bytes_subscription = cx.subscribe(
                            &oracle_security_quota_bytes_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_create_field_changed(
                                    input, event, fluxdb_app::OracleCreateDraftField::QuotaBytes, cx,
                                );
                            },
                        );
                        // 改密口令：明文只进内存草稿，保存时随计划一起执行。
                        let oracle_security_new_password_subscription = cx.subscribe(
                            &oracle_security_new_password_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change)
                                    && let Some(tab_id) = this.active_oracle_admin_tab_id()
                                {
                                    this.dispatch(
                                        AppCommand::SetOracleUserPassword {
                                            tab_id,
                                            password: input.read(cx).value().to_string(),
                                        },
                                        cx,
                                    );
                                }
                            },
                        );
                        // 确认框不入状态，只需重绘以刷新「两次口令不一致」提示。
                        let oracle_security_new_password_confirm_subscription = cx.subscribe(
                            &oracle_security_new_password_confirm_input,
                            |_this: &mut NavicatMain, _, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    cx.notify();
                                }
                            },
                        );
                        let oracle_security_grant_name_subscription = cx.subscribe(
                            &oracle_security_grant_name_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_grant_field_changed(
                                    input, event, fluxdb_app::OracleGrantDraftField::Name, cx,
                                );
                            },
                        );
                        let oracle_security_grant_owner_subscription = cx.subscribe(
                            &oracle_security_grant_owner_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_grant_field_changed(
                                    input, event, fluxdb_app::OracleGrantDraftField::Owner, cx,
                                );
                            },
                        );
                        let oracle_security_grant_object_subscription = cx.subscribe(
                            &oracle_security_grant_object_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                this.oracle_grant_field_changed(
                                    input, event, fluxdb_app::OracleGrantDraftField::Object, cx,
                                );
                            },
                        );
    let user_admin_new_password_subscription = cx.subscribe(
        &user_admin_new_password_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminNewPassword {
                        tab_id,
                        password: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_max_queries_subscription = cx.subscribe(
        &user_admin_max_queries_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminMaxQueriesPerHour {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_max_updates_subscription = cx.subscribe(
        &user_admin_max_updates_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminMaxUpdatesPerHour {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_max_connections_subscription = cx.subscribe(
        &user_admin_max_connections_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminMaxConnectionsPerHour {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_max_user_connections_subscription = cx.subscribe(
        &user_admin_max_user_connections_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminMaxUserConnections {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_ssl_type_select_subscription = cx.subscribe_in(
        &user_admin_ssl_type_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(tab_id), Some(ssl_type)) = (this.active_user_admin_tab_id(), value) {
                this.dispatch(
                    AppCommand::SetUserAdminSslType {
                        tab_id,
                        ssl_type: ssl_type.clone(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_ssl_cipher_subscription = cx.subscribe(
        &user_admin_ssl_cipher_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminSslCipher {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_ssl_issuer_subscription = cx.subscribe(
        &user_admin_ssl_issuer_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminSslIssuer {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let user_admin_ssl_subject_subscription = cx.subscribe(
        &user_admin_ssl_subject_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab_id) = this.active_user_admin_tab_id()
            {
                this.dispatch(
                    AppCommand::SetUserAdminSslSubject {
                        tab_id,
                        value: input.read(cx).value().to_string(),
                    },
                    cx,
                );
            }
        },
    );
    let create_database_charset_select_subscription = cx.subscribe_in(
        &create_database_charset_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let Some(charset) = value {
                this.select_create_database_charset(charset.as_str(), window, cx);
            }
        },
    );
    let create_database_collation_select_subscription = cx.subscribe_in(
        &create_database_collation_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<String>>,
         window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let Some(collation) = value {
                this.select_create_database_collation(collation.as_str(), window, cx);
            }
        },
    );
    let danger_table_foreign_key_check_select_subscription = cx.subscribe_in(
        &danger_table_foreign_key_check_select,
        window,
        |this: &mut NavicatMain,
         _select,
         event: &SelectEvent<SearchableVec<ForeignKeyCheckOption>>,
         _window,
         cx| {
            let SelectEvent::Confirm(value) = event;
            if let (Some(value), Some(form)) = (value, &mut this.pending_danger_table_action) {
                form.foreign_key_check = *value;
                form.error = None;
                cx.notify();
            }
        },
    );
    let data_filter_value_input_subscription = cx.subscribe(
        &data_filter_value_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_data_filter_manual_value(input.read(cx).value().to_string(), cx);
            }
            // Enter：单值弹层直接确定写回；多值弹层把输入内容追加为新值，继续连续输入。
            // 中文输入法组合确认阶段的 Enter 由 Input 内部消费为标记
            // 文本提交，不会触发 PressEnter，因此不会在组合输入时误添加。
            if matches!(event, InputEvent::PressEnter { .. }) {
                let value = input.read(cx).value().to_string();
                this.commit_data_filter_value_input(value, cx);
                cx.stop_propagation();
            }
        },
    );
    let data_filter_batch_input_subscription = cx.subscribe(
        &data_filter_batch_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(draft) = this.data_filter_value_draft.as_mut() {
                    draft.batch_text = input.read(cx).value().to_string();
                    cx.notify();
                }
            }
        },
    );
    let local_filter_value_input_subscription = cx.subscribe(
        &local_filter_value_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_local_filter_value(input.read(cx).value().to_string(), cx);
            }
        },
    );
    let local_filter_search_subscription = cx.subscribe(
        &local_filter_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_local_filter_search(input.read(cx).value().to_string(), cx);
            }
        },
    );
    let data_filter_text_subscription = cx.subscribe(
        &data_filter_text_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_data_filter_condition_text(input.read(cx).value().to_string(), cx);
            }
        },
    );
    let data_sort_text_subscription = cx.subscribe(
        &data_sort_text_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_data_sort_text(input.read(cx).value().to_string(), cx);
            }
        },
    );
    let data_filter_search_subscription = cx.subscribe(
        &data_filter_search_input,
        |this: &mut NavicatMain, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_data_filter_value_search(input.read(cx).value().to_string(), cx);
            }
        },
    );
    let table_folders = controller.state().sidebar_layout.table_folders.clone();
    let table_folder_assignments = controller
        .state()
        .sidebar_layout
        .table_folder_assignments
        .clone();
    let settings_editor_draft = controller.state().settings.clone();
    settings_radius_input.update(cx, |input, cx| {
        input.set_value(settings_editor_draft.button_radius.to_string(), window, cx)
    });
    settings_pg_client_source_input.update(cx, |input, cx| {
        input.set_value(
            settings_editor_draft.pg_client_download_source.clone(),
            window,
            cx,
        )
    });
    settings_mysql_client_source_input.update(cx, |input, cx| {
        input.set_value(
            settings_editor_draft.mysql_client_download_source.clone(),
            window,
            cx,
        )
    });
    let settings_font_size_slider = cx.new(|_| {
        SliderState::new()
            .min(10.)
            .max(24.)
            .step(1.)
            .default_value(settings_editor_draft.editor_font_size.clamp(10, 24) as f32)
    });
    let settings_font_size_slider_subscription = cx.subscribe(
        &settings_font_size_slider,
        |this: &mut NavicatMain, _slider, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                let font_size = value.end().round().clamp(10., 24.) as u32;
                this.settings_editor_draft.editor_font_size = font_size;
                cx.notify();
            }
        },
    );
    // ER 画布视图状态：Rc 共享，实体（er_view）只做重绘通知（§perf 8）。
    let er_canvas = std::rc::Rc::new(std::cell::RefCell::new(ErCanvasShared {
        er_viewports: BTreeMap::new(),
        er_viewport_drag: None,
        er_canvas_sizes: BTreeMap::new(),
        er_canvas_origins: BTreeMap::new(),
        er_node_scroll_px: BTreeMap::new(),
        er_scroll_drag: None,
        er_selected_table: BTreeMap::new(),
        er_scene_positions: BTreeMap::new(),
        er_pinned: BTreeMap::new(),
        er_node_drag: None,
        er_link_drag: None,
        er_row_hover: BTreeMap::new(),
        er_port_hover: BTreeMap::new(),
        er_frame_nodes: BTreeMap::new(),
        er_frame_edges: BTreeMap::new(),
        er_field_highlights: BTreeMap::new(),
        er_minimap_rect: BTreeMap::new(),
    }));
    NavicatMain {
        ai_chat: AiChatUi::new(window, cx),
        focus_handle: cx.focus_handle(),
        controller,
        storage,
        theme_mode,
        update_ui: UpdateUiState::default(),
        update_auto_check_pending: auto_check_updates,
        update_cancel: Arc::new(AtomicBool::new(false)),
        update_downloaded_bytes: Arc::new(AtomicU64::new(0)),
        update_total_bytes: Arc::new(AtomicU64::new(0)),
        _update_check_task: None,
        _update_download_task: None,
        _update_progress_task: None,
        _update_install_task: None,
        new_connection_kind: None,
        new_connection_tab: NewConnectionTab::Connection,
        new_connection_form: NewConnectionForm::for_kind(DatabaseKind::MySql, 1),
        new_connection_inputs: NewConnectionInputs::new(window, cx),
        ai_model_dialog: None,
        ai_plugin_config_dialog: None,
        new_connection_password_visible: false,
        editing_connection_id: None,
        rename_group_input,
        _rename_group_subscription: rename_group_subscription,
        table_folder_rename_input,
        _table_folder_rename_subscription: table_folder_rename_subscription,
        sidebar_search_input,
        _sidebar_search_subscription: sidebar_search_subscription,
        sidebar_search: String::new(),
        sidebar_tree_scroll: VirtualListScrollHandle::new(),
        sidebar_tree_cache: std::cell::RefCell::new(None),
        field_filter_search_input,
        _field_filter_search_subscription: field_filter_search_subscription,
        field_filter_search: String::new(),
        field_filter_popover: None,
        visible_table_fields: BTreeMap::new(),
        data_filter_value_input,
        _data_filter_value_input_subscription: data_filter_value_input_subscription,
        data_filter_batch_input,
        _data_filter_batch_input_subscription: data_filter_batch_input_subscription,
        local_filter_value_input,
        _local_filter_value_input_subscription: local_filter_value_input_subscription,
        local_filter_search_input,
        _local_filter_search_subscription: local_filter_search_subscription,
        data_filter_search_input,
        _data_filter_search_subscription: data_filter_search_subscription,
        data_filter_text_input,
        _data_filter_text_subscription: data_filter_text_subscription,
        data_sort_text_input,
        _data_sort_text_subscription: data_sort_text_subscription,
        data_sql_panel_input,
        _data_sql_panel_subscription: data_sql_panel_subscription,
        data_sql_footer_selection: SqlTextSelection::default(),
        data_page_input,
        _data_page_subscription: data_page_subscription,
        data_cell_edit_input,
        _data_cell_edit_subscription: data_cell_edit_subscription,
        data_cell_editing: None,
        temporal_part_input,
        _temporal_part_subscription: temporal_part_subscription,
        temporal_part_editing: None,
        cell_detail_input,
        _cell_detail_subscription: cell_detail_subscription,
        data_search_input,
        _data_search_subscription: data_search_subscription,
        row_detail_search_input,
        _row_detail_search_subscription: row_detail_search_subscription,
        row_detail_search: String::new(),
        data_search_panels: BTreeSet::new(),
        data_search_queries: BTreeMap::new(),
        data_search_active_matches: BTreeMap::new(),
        data_search_highlight_all_tabs: BTreeSet::new(),
        redis_search_input,
        _redis_search_subscription: redis_search_subscription,
        redis_type_select,
        _redis_type_select_subscription: redis_type_select_subscription,
        redis_search_drafts: BTreeMap::new(),
        redis_search_queries: BTreeMap::new(),
        redis_type_filters: BTreeMap::new(),
        redis_key_list_modes: BTreeMap::new(),
        redis_key_list_expanded: BTreeMap::new(),
        redis_key_list_selected_leaf: BTreeMap::new(),
        redis_key_list_folder_hovered: BTreeMap::new(),
        redis_key_list_uniform_scroll: BTreeMap::new(),
        redis_key_folder_visible_cache: BTreeMap::new(),
        redis_key_list_loaded: BTreeMap::new(),
        redis_key_metadata_pending: BTreeMap::new(),
        redis_key_search_history: BTreeMap::new(),
        redis_key_search_history_loaded: BTreeMap::new(),
        redis_key_search_history_open: false,
        redis_data_refresh_times: BTreeMap::new(),
        redis_refresh_time_task: None,
        redis_overview_refresh_task: None,
        redis_overview_refresh_tasks: BTreeMap::new(),
        redis_server_versions: BTreeMap::new(),
        redis_server_version_tasks: BTreeMap::new(),
        er_graphs: BTreeMap::new(),
        er_full_tables: BTreeMap::new(),
        er_errors: BTreeMap::new(),
        er_load_tasks: BTreeMap::new(),
        er_generation: BTreeMap::new(),
        er_relation_tasks: BTreeMap::new(),
        er_depths: BTreeMap::new(),
        er_expanded: BTreeMap::new(),
        er_scenes: BTreeMap::new(),
        er_canvas,
        er_user_interacted: BTreeSet::new(),
        er_layout_applied: BTreeSet::new(),
        er_auto_placed: BTreeSet::new(),
        er_fit_done: BTreeSet::new(),
        er_all_edges: BTreeMap::new(),
        er_center_refs: BTreeMap::new(),
        er_last_updated: BTreeMap::new(),
        er_refreshing: BTreeSet::new(),
        er_pending_columns: BTreeMap::new(),
        er_last_import: BTreeMap::new(),
        er_column_debounce_tasks: BTreeMap::new(),
        er_search_input: BTreeMap::new(),
        er_search_subs: BTreeMap::new(),
        er_search_query: BTreeMap::new(),
        er_search_sel: BTreeMap::new(),
        er_group: BTreeMap::new(),
        er_custom_groups: BTreeMap::new(),
        er_group_inputs: BTreeMap::new(),
        er_group_built: BTreeMap::new(),
        er_export_open: BTreeSet::new(),
        er_export_sel: BTreeMap::new(),
        er_export_focus: BTreeMap::new(),
        er_view_restored: BTreeSet::new(),
        er_view_save_failed: BTreeSet::new(),
        er_previous_layout: BTreeMap::new(),
        er_scope_keys: BTreeMap::new(),
        er_relationship_scope_keys: BTreeMap::new(),
        er_relationships: BTreeMap::new(),
        er_relationship_loading: BTreeSet::new(),
        er_relationship_tasks: BTreeMap::new(),
        er_relationship_errors: BTreeMap::new(),
        er_relationship_panel_open: BTreeSet::new(),
        er_relationship_panel_focus: BTreeMap::new(),
        er_relationship_form_open: BTreeSet::new(),
        er_relationship_form_editing: BTreeMap::new(),
        er_relationship_form_role_inputs: BTreeMap::new(),
        er_relationship_form_description_inputs: BTreeMap::new(),
        er_relationship_form_left_tables: BTreeMap::new(),
        er_relationship_form_right_tables: BTreeMap::new(),
        er_relationship_form_pairs: BTreeMap::new(),
        er_relationship_form_filters: BTreeMap::new(),
        er_relationship_form_cardinality: BTreeMap::new(),
        er_relationship_panel_width: BTreeMap::new(),
        er_relationship_panel_selected: BTreeMap::new(),
        er_rebind_pending: BTreeMap::new(),
        er_rebind_auto: BTreeMap::new(),
        er_rebind_tasks: BTreeMap::new(),
        er_rebuild_review_tasks: BTreeMap::new(),
        er_relationship_panel_resize_start: None,
        er_relationship_form_subscriptions: BTreeMap::new(),
        er_relationship_delete_pending: BTreeMap::new(),
        er_relationship_delete_undo: BTreeMap::new(),
        er_relationship_form_table_options: BTreeMap::new(),
        redis_key_list_hovered_tab: None,
        redis_workbench_hovered_record: None,
        redis_stream_table_state,
        redis_hash_table_state,
        redis_list_table_state,
        redis_key_value_input,
        _redis_key_value_subscription: redis_key_value_subscription,
        _redis_json_key_value_subscription: redis_json_key_value_subscription,
        redis_json_editor,
        redis_json_editor_active: None,
        redis_json_editor_busy: false,
        redis_key_value_active: None,
        redis_key_value_drafts: BTreeMap::new(),
        redis_key_value_syncing: false,
        redis_string_values: BTreeMap::new(),
        redis_string_format: BTreeMap::new(),
        redis_string_editing: None,
        redis_string_loading: None,
        redis_string_downloading: BTreeSet::new(),
        redis_string_active: None,
        redis_key_name_input,
        _redis_key_name_subscription: redis_key_name_subscription,
        redis_key_ttl_input,
        _redis_key_ttl_subscription: redis_key_ttl_subscription,
        redis_key_meta_active: None,
        redis_key_name_drafts: BTreeMap::new(),
        redis_key_ttl_drafts: BTreeMap::new(),
        redis_key_meta_editing: None,
        redis_key_meta_syncing: false,
        pending_redis_key_delete: None,
        pending_redis_stream_entry_add: None,
        pending_redis_stream_entry_delete: None,
        redis_stream_entry_id_input,
        redis_stream_entry_field_rows,
        redis_stream_entry_drawer_scroll: ScrollHandle::new(),
        redis_set_member_search_input,
        _redis_set_member_search_subscription: redis_set_member_search_subscription,
        redis_set_member_search_active: None,
        redis_set_member_search_queries: BTreeMap::new(),
        redis_set_member_search_pages: BTreeMap::new(),
        redis_set_member_search_loading: None,
        redis_set_member_search_more_loading: None,
        redis_set_member_search_generation: BTreeMap::new(),
        redis_set_member_search_debounce: None,
        redis_set_member_search_debounce_until: None,
        redis_set_member_search_syncing: false,
        redis_set_member_rows: Vec::new(),
        redis_set_member_panel_scroll: ScrollHandle::new(),
        redis_set_member_active: None,
        pending_redis_set_member_drawer: None,
        redis_set_member_drawer_rows: Vec::new(),
        redis_set_member_drawer_scroll: ScrollHandle::new(),
        pending_redis_set_member_delete: None,
        redis_hash_field_search_input,
        _redis_hash_field_search_subscription: redis_hash_field_search_subscription,
        redis_hash_field_search_active: None,
        redis_hash_field_search_queries: BTreeMap::new(),
        redis_hash_field_search_pages: BTreeMap::new(),
        redis_hash_field_search_loading: None,
        redis_hash_field_search_more_loading: None,
        redis_hash_field_search_generation: BTreeMap::new(),
        redis_hash_field_search_debounce: None,
        redis_hash_field_search_debounce_until: None,
        redis_hash_field_search_syncing: false,
        redis_hash_value_edit_input,
        _redis_hash_value_edit_subscription: redis_hash_value_edit_subscription,
        redis_hash_ttl_edit_input,
        _redis_hash_ttl_edit_subscription: redis_hash_ttl_edit_subscription,
        redis_hash_field_rows: Vec::new(),
        redis_hash_field_hovered: None,
        redis_hash_field_editing: None,
        redis_hash_full_value_viewer: Rc::new(std::cell::RefCell::new(None)),
        pending_redis_hash_field_drawer: None,
        redis_hash_field_drawer_rows: Vec::new(),
        redis_hash_field_drawer_scroll: ScrollHandle::new(),
        pending_redis_hash_field_delete: None,
        redis_zset_member_search_input,
        _redis_zset_member_search_subscription: redis_zset_member_search_subscription,
        redis_zset_member_search_active: None,
        redis_zset_member_search_queries: BTreeMap::new(),
        redis_zset_member_search_pages: BTreeMap::new(),
        redis_zset_member_search_loading: None,
        redis_zset_member_search_more_loading: None,
        redis_zset_member_search_generation: BTreeMap::new(),
        redis_zset_member_search_debounce: None,
        redis_zset_member_search_debounce_until: None,
        redis_zset_member_search_syncing: false,
        redis_zset_member_rows: Vec::new(),
        redis_zset_member_panel_scroll: ScrollHandle::new(),
        redis_zset_member_score_hover: None,
        redis_zset_member_score_editing: None,
        redis_zset_member_score_edit_input: None,
        redis_zset_member_score_blur_sub: None,
        pending_redis_zset_member_drawer: None,
        redis_zset_member_drawer_rows: Vec::new(),
        redis_zset_member_drawer_scroll: ScrollHandle::new(),
        pending_redis_zset_member_delete: None,
        redis_list_item_search_input,
        _redis_list_item_search_subscription: redis_list_item_search_subscription,
        redis_stream_entry_active: None,
        redis_stream_since_input: cx
            .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.stream_since"))),
        redis_stream_until_input: cx
            .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.stream_until"))),
        redis_stream_ranges: BTreeMap::new(),
        redis_stream_maxlen_input: cx
            .new(|cx| InputState::new(window, cx).placeholder(tr("app.placeholder.maxlen"))),
        redis_stream_groups: BTreeMap::new(),
        redis_stream_groups_loading: None,
        redis_stream_groups_expanded: false,
        redis_stream_entry_pages: BTreeMap::new(),
        redis_stream_entry_loading: None,
        redis_stream_entry_more_loading: None,
        redis_stream_entry_generation: BTreeMap::new(),
        redis_list_item_search_active: None,
        redis_list_item_search_queries: BTreeMap::new(),
        redis_list_item_search_pages: BTreeMap::new(),
        redis_list_item_search_loading: None,
        redis_list_item_search_more_loading: None,
        redis_list_item_search_generation: BTreeMap::new(),
        redis_list_item_search_syncing: false,
        pending_redis_list_item_drawer: None,
        redis_list_item_drawer_rows: Vec::new(),
        redis_list_item_drawer_scroll: ScrollHandle::new(),
        redis_list_item_remove_drawer: None,
        redis_list_item_remove_confirm: None,
        redis_list_item_remove_count_input,
        _redis_list_item_remove_count_subscription: redis_list_item_remove_count_subscription,
        redis_list_item_remove_select,
        _redis_list_item_remove_select_subscription: redis_list_item_remove_select_subscription,
        redis_add_key_drawer: None,
        redis_add_key_type_select,
        _redis_add_key_type_select_subscription: redis_add_key_type_select_subscription,
        redis_add_key_scroll: ScrollHandle::new(),
        redis_add_key_name_input,
        _redis_add_key_name_subscription: redis_add_key_name_subscription,
        redis_add_key_ttl_input,
        _redis_add_key_ttl_subscription: redis_add_key_ttl_subscription,
        redis_add_key_string_input,
        _redis_add_key_string_subscription: redis_add_key_string_subscription,
        redis_add_key_json_input,
        _redis_add_key_json_subscription: redis_add_key_json_subscription,
        redis_add_key_hash_rows: Vec::new(),
        redis_add_key_zset_rows: Vec::new(),
        redis_add_key_set_rows: Vec::new(),
        redis_add_key_list_rows: Vec::new(),
        redis_add_key_list_direction: RedisListDirection::Tail,
        redis_add_key_stream_id_input,
        _redis_add_key_stream_id_subscription: redis_add_key_stream_id_subscription,
        redis_add_key_stream_rows: Vec::new(),
        redis_add_key_applying: false,
        redis_list_value_edit_input,
        _redis_list_value_edit_subscription: redis_list_value_edit_subscription,
        redis_list_item_hovered: None,
        redis_list_item_editing: None,
        tab_switcher_search_input,
        _tab_switcher_search_subscription: tab_switcher_search_subscription,
        tab_switcher: None,
        tab_switcher_search: String::new(),
        query_history_search_input,
        _query_history_search_subscription: query_history_search_subscription,
        query_history_search: String::new(),
        query_history_quick_search_input,
        _query_history_quick_search_subscription: query_history_quick_search_subscription,
        query_history_quick_search: String::new(),
        redis_history_search_input,
        _redis_history_search_subscription: redis_history_search_subscription,
        redis_history_search: String::new(),
        query_history_quick_open: false,
        query_history_quick_selected: 0,
        query_history_quick_kind_filter: QueryHistoryKindFilter::All,
        query_history_connection_select,
        _query_history_connection_select_subscription: query_history_connection_select_subscription,
        query_history_database_select,
        _query_history_database_select_subscription: query_history_database_select_subscription,
        query_history_table_select,
        _query_history_table_select_subscription: query_history_table_select_subscription,
        sql_file_connection_select,
        _sql_file_connection_select_subscription: sql_file_connection_select_subscription,
        sql_file_database_select,
        _sql_file_database_select_subscription: sql_file_database_select_subscription,
        sql_file_encoding_select,
        _sql_file_encoding_select_subscription: sql_file_encoding_select_subscription,
        sql_file_path_input,
        _sql_file_path_subscription: sql_file_path_subscription,
        backup_file_name_input,
        _backup_file_name_subscription: backup_file_name_subscription,
        backup_target_dir_input,
        _backup_target_dir_subscription: backup_target_dir_subscription,
        backup_object_search_input,
        _backup_object_search_subscription: backup_object_search_subscription,
        backup_note_input,
        _backup_note_subscription: backup_note_subscription,
        backup_tables_modal: None,
        restore_records_modal: None,
        backup_note_modal_path: None,
        backup_note_edit_input,
        backup_pending_metas: BTreeMap::new(),
        pending_delete_backup: None,
        pending_exit_confirm: false,
        user_admin_search_input,
        _user_admin_search_subscription: user_admin_search_subscription,
        user_admin_create_user_input,
        _user_admin_create_user_subscription: user_admin_create_user_subscription,
        user_admin_create_host_input,
        _user_admin_create_host_subscription: user_admin_create_host_subscription,
        user_admin_auth_plugin_select,
        _user_admin_auth_plugin_select_subscription: user_admin_auth_plugin_select_subscription,
        user_admin_password_expiry_select,
        _user_admin_password_expiry_select_subscription:
            user_admin_password_expiry_select_subscription,
        user_admin_create_password_input,
        _user_admin_create_password_subscription: user_admin_create_password_subscription,
        user_admin_new_password_input,
        _user_admin_new_password_subscription: user_admin_new_password_subscription,
        pg_role_name_input,
        _pg_role_name_subscription: pg_role_name_subscription,
        pg_password_input,
        _pg_password_subscription: pg_password_subscription,
        pg_password_confirm_input,
        _pg_password_confirm_subscription: pg_password_confirm_subscription,
        pg_connection_limit_input,
        _pg_connection_limit_subscription: pg_connection_limit_subscription,
        pg_valid_until_input,
        _pg_valid_until_subscription: pg_valid_until_subscription,
        pg_valid_until_mode_select,
        _pg_valid_until_mode_select_subscription: pg_valid_until_mode_select_subscription,
        pg_password_op_select,
        _pg_password_op_select_subscription: pg_password_op_select_subscription,
        pg_grant_db_select,
        _pg_grant_db_select_subscription: pg_grant_db_select_subscription,
        pg_grant_schema_select,
        _pg_grant_schema_select_subscription: pg_grant_schema_select_subscription,
        pg_grant_object_select,
        _pg_grant_object_select_subscription: pg_grant_object_select_subscription,
        pg_member_of_role_select,
        _pg_member_of_role_select_subscription: pg_member_of_role_select_subscription,
        pg_member_new_admin: false,
        pg_select_items_fingerprint: String::new(),
        sqlserver_security_database_select,
        _sqlserver_security_database_subscription: sqlserver_security_database_subscription,
        sqlserver_security_database_items: Vec::new(),
        sqlserver_security_name_input,
        _sqlserver_security_name_subscription: sqlserver_security_name_subscription,
        sqlserver_security_password_input,
        _sqlserver_security_password_subscription: sqlserver_security_password_subscription,
        sqlserver_security_confirm_password_input,
        _sqlserver_security_confirm_password_subscription:
            sqlserver_security_confirm_password_subscription,
        sqlserver_security_login_select,
        _sqlserver_security_login_subscription: sqlserver_security_login_subscription,
        sqlserver_security_default_select,
        sqlserver_security_form_options: (Vec::new(), Vec::new()),
        _sqlserver_security_default_subscription: sqlserver_security_default_subscription,
        sqlserver_security_permission_input,
        _sqlserver_security_permission_subscription: sqlserver_security_permission_subscription,
        sqlserver_security_permission_target_input,
        _sqlserver_security_permission_target_subscription:
            sqlserver_security_permission_target_subscription,
        _sqlserver_security_load_task: None,
        _sqlserver_security_permissions_task: None,
        _sqlserver_security_preview_task: None,
        _sqlserver_security_apply_task: None,
                            oracle_security_name_input,
                            _oracle_security_name_subscription: oracle_security_name_subscription,
                            oracle_security_password_input,
                            _oracle_security_password_subscription:
                                oracle_security_password_subscription,
                            oracle_security_confirm_password_input,
                            _oracle_security_confirm_password_subscription:
                                oracle_security_confirm_password_subscription,
                            oracle_security_new_password_input,
                            _oracle_security_new_password_subscription:
                                oracle_security_new_password_subscription,
                            oracle_security_new_password_confirm_input,
                            _oracle_security_new_password_confirm_subscription:
                                oracle_security_new_password_confirm_subscription,
                            oracle_security_default_ts_input,
                            _oracle_security_default_ts_subscription:
                                oracle_security_default_ts_subscription,
                            oracle_security_temp_ts_input,
                            _oracle_security_temp_ts_subscription:
                                oracle_security_temp_ts_subscription,
                            oracle_security_profile_input,
                            _oracle_security_profile_subscription:
                                oracle_security_profile_subscription,
                            oracle_security_quota_ts_input,
                            _oracle_security_quota_ts_subscription:
                                oracle_security_quota_ts_subscription,
                            oracle_security_quota_bytes_input,
                            _oracle_security_quota_bytes_subscription:
                                oracle_security_quota_bytes_subscription,
                            oracle_security_grant_name_input,
                            _oracle_security_grant_name_subscription:
                                oracle_security_grant_name_subscription,
                            oracle_security_grant_owner_input,
                            _oracle_security_grant_owner_subscription:
                                oracle_security_grant_owner_subscription,
                            oracle_security_grant_object_input,
                            _oracle_security_grant_object_subscription:
                                oracle_security_grant_object_subscription,
                            _oracle_security_load_task: None,
                            _oracle_security_apply_task: None,
        user_admin_max_queries_input,
        _user_admin_max_queries_subscription: user_admin_max_queries_subscription,
        user_admin_max_updates_input,
        _user_admin_max_updates_subscription: user_admin_max_updates_subscription,
        user_admin_max_connections_input,
        _user_admin_max_connections_subscription: user_admin_max_connections_subscription,
        user_admin_max_user_connections_input,
        _user_admin_max_user_connections_subscription: user_admin_max_user_connections_subscription,
        user_admin_ssl_type_select,
        _user_admin_ssl_type_select_subscription: user_admin_ssl_type_select_subscription,
        user_admin_ssl_cipher_input,
        _user_admin_ssl_cipher_subscription: user_admin_ssl_cipher_subscription,
        user_admin_ssl_issuer_input,
        _user_admin_ssl_issuer_subscription: user_admin_ssl_issuer_subscription,
        user_admin_ssl_subject_input,
        _user_admin_ssl_subject_subscription: user_admin_ssl_subject_subscription,
        user_admin_privilege_database_menu: None,
        query_history_connection_filter: None,
        query_history_database_filter: None,
        query_history_table_filter: None,
        query_history_kind_filter: QueryHistoryKindFilter::All,
        query_history_detail: None,
        user_admin_password_visible: false,
        pinned_tabs: BTreeSet::new(),
        tab_order: Vec::new(),
        workspace_tab_order: Vec::new(),
        hovered_tab: None,
        hovered_first_level_tab: None,
        display_database_search_input,
        _display_database_search_subscription: display_database_search_subscription,
        create_database_name_input,
        _create_database_name_subscription: create_database_name_subscription,
        create_database_charset_select,
        _create_database_charset_select_subscription: create_database_charset_select_subscription,
        create_database_collation_select,
        _create_database_collation_select_subscription:
            create_database_collation_select_subscription,
        create_database_owner_input,
        _create_database_owner_subscription: create_database_owner_subscription,
        create_database_template_input,
        _create_database_template_subscription: create_database_template_subscription,
        create_database_first_collection_input,
        _create_database_first_collection_subscription:
            create_database_first_collection_subscription,
        danger_table_foreign_key_check_select,
        _danger_table_foreign_key_check_select_subscription:
            danger_table_foreign_key_check_select_subscription,
        rename_table_input,
        _rename_table_subscription: rename_table_subscription,
        copy_table_input,
        _copy_table_subscription: copy_table_subscription,
        column_choice_value_input,
        column_choice_label_input,
        query_save_name_input,
        _file_picker_task: None,
        query_files: QueryFilesUi::default(),
        _connection_tasks: BTreeMap::new(),
        _tree_refresh_task: None,
        _database_tasks: BTreeMap::new(),
        _data_load_tasks: BTreeMap::new(),
        mongo_query_inputs: BTreeMap::new(),
        mongo_query_options_expanded: BTreeMap::new(),
        mongo_query_advanced_expanded: BTreeMap::new(),
        mongo_query_builder_expanded: BTreeMap::new(),
        mongo_query_builders: BTreeMap::new(),
        mongo_aggregation_builders: BTreeMap::new(),
        mongo_collapsed_documents: BTreeMap::new(),
        mongo_expanded_fields: BTreeMap::new(),
        mongo_json_fold_toggles: BTreeMap::new(),
        mongo_document_cache: BTreeMap::new(),
        mongo_document_editor: None,
        _mongo_document_editor_task: None,
        pending_mongo_document_switch: None,
        pending_mongo_document_delete: None,
        _mongo_document_delete_task: None,
        pending_mongo_document_open: None,
        _mongo_document_insert_task: None,
        pending_mongo_import: None,
        _mongo_import_task: None,
        _mongo_load_tasks: BTreeMap::new(),
        _mongo_info_tasks: BTreeMap::new(),
        _mongo_index_tasks: BTreeMap::new(),
        _mongo_field_sample_tasks: BTreeMap::new(),
        _redis_key_value_apply_tasks: BTreeMap::new(),
        _query_execute_tasks: BTreeMap::new(),
        _redis_workbench_tasks: BTreeMap::new(),
        _sql_file_execute_tasks: BTreeMap::new(),
        _sql_file_cancel_flags: BTreeMap::new(),
        _query_completion_tasks: BTreeMap::new(),
        _completion_index_tasks: BTreeMap::new(),
        _table_info_tasks: BTreeMap::new(),
        _design_table_tasks: BTreeMap::new(),
        _design_table_detail_tasks: BTreeMap::new(),
        _user_admin_users_tasks: BTreeMap::new(),
        _user_admin_pg_membership_tasks: BTreeMap::new(),
        _user_admin_pg_target_tasks: BTreeMap::new(),
        _user_admin_pg_object_grant_tasks: BTreeMap::new(),
        _user_admin_grants_tasks: BTreeMap::new(),
        _user_admin_member_grants_tasks: BTreeMap::new(),
        _user_admin_apply_tasks: BTreeMap::new(),
        _user_admin_preview_tasks: BTreeMap::new(),
        _create_table_apply_tasks: BTreeMap::new(),
        _create_table_reference_columns_tasks: BTreeMap::new(),
        _cell_binary_download_tasks: BTreeMap::new(),
                            _cell_text_load_tasks: BTreeMap::new(),
                            _cell_content_chunk_tasks: BTreeMap::new(),
                            _cell_content_save_tasks: BTreeMap::new(),
        _data_export_tasks: BTreeMap::new(),
        _data_export_cancel_flags: BTreeMap::new(),
        _backup_tasks: BTreeMap::new(),
        _backup_cancel_flags: BTreeMap::new(),
        _create_database_tasks: BTreeMap::new(),
        _delete_database_tasks: BTreeMap::new(),
        _rename_table_task: None,
        _copy_table_task: None,
        _routine_ddl_task: None,
        _copy_table_ddl_task: None,
        _copy_table_structure_task: None,
        _danger_table_task: None,
        _table_import_task: None,
        data_export_task_seq: 0,
        _test_connection_task: None,
        _create_schema_task: None,
        saving_connection: false,
        _redis_discover_task: None,
        redis_discovery_pending_sync: false,
        connection_context_menu: None,
        database_context_menu: None,
        schema_context_menu: None,
        table_context_menu: None,
        collection_context_menu: None,
        table_group_context_menu: None,
        table_folder_context_menu: None,
        tab_context_menu: None,
        data_cell_context_menu: None,
        data_row_context_menu: None,
        group_context_menu: None,
        pending_query_save: None,
        pending_connection_query_save: None,
        pending_rename_group: None,
        pending_rename_table_folder: None,
        pending_delete_connection: None,
        pending_delete_database: None,
        mongo_info_dialog: None,
        pending_create_collection: None,
        create_collection_running: false,
        create_collection_input,
        _create_collection_subscription: create_collection_subscription,
        _create_mongo_collection_task: None,
        pending_disconnect_connection: None,
        pending_close_workspace: None,
        pending_new_query_connection: None,
        pending_delete_data_row: None,
        data_row_viewer: None,
        pending_dirty_data_action: None,
        pending_apply_data_changes: None,
        data_change_apply_tasks: BTreeMap::new(),
        pending_query_parameters: None,
        pending_dangerous_query: None,
        pending_dangerous_redis_command: None,
        pending_config_proposal: None,
        config_proposal_busy: false,
        config_delete_confirmed: false,
        sql_file_modal: Rc::new(std::cell::RefCell::new(SqlFileModalData::default())),
        restore_modal: None,
        pending_data_export: None,
        pending_data_import: None,
        _data_import_task: None,
        data_export_custom_conditions_open: false,
        data_export_preview: None,
        data_export_preview_seq: 0,
        _data_export_preview_task: None,
        pending_table_data_export_after_load: None,
        pending_rename_table: None,
        pending_rename_collection: None,
        rename_collection_input,
        _rename_collection_subscription: rename_collection_subscription,
        _rename_mongo_collection_task: None,
        pending_drop_collection: None,
        _drop_mongo_collection_task: None,
        pending_mongo_export: None,
        _mongo_export_task: None,
        pending_copy_table: None,
        pending_routine_ddl: None,
        pending_column_choices: None,
        pending_danger_table_action: None,
        data_export_log_task: None,
        data_export_tasks: Vec::new(),
        pending_backup_modal: None,
        backup_objects_scroll: VirtualListScrollHandle::new(),
        backup_log_task: None,
        backup_tasks: Vec::new(),
        backup_task_seq: 0,
        query_parameter_history: BTreeMap::new(),
        display_database_connection: None,
        display_database_selection: BTreeSet::new(),
        display_database_search: String::new(),
        display_database_show_system: false,
        pending_create_database: None,
        create_database_running: BTreeSet::new(),
        pending_create_schema: None,
        create_schema_running: false,
        create_schema_name_input,
        _create_schema_name_subscription: create_schema_name_subscription,
        new_connection_target_group: None,
        connecting_connections: BTreeSet::new(),
        loading_databases: BTreeSet::new(),
        loaded_database_children: BTreeSet::new(),
        pinned_databases: BTreeSet::new(),
        pinned_tables: BTreeSet::new(),
        table_folders,
        table_folder_assignments,
        selected_table_folder: None,
        expanded_databases: BTreeMap::new(),
        expanded_object_groups: BTreeMap::new(),
        data_table_states: BTreeMap::new(),
        data_table_column_widths: BTreeMap::new(),
        _data_table_width_subscriptions: BTreeMap::new(),
        create_table_inputs: BTreeMap::new(),
        _create_table_input_subscriptions: BTreeMap::new(),
        create_table_comment_editor_sizes: BTreeMap::new(),
        create_table_check_expression_editor_sizes: BTreeMap::new(),
        create_table_comment_editor_resize_start: None,
        create_table_check_expression_editor_resize_start: None,
        create_table_index_field_dropdown: None,
        create_table_index_field_selection: None,
        create_table_foreign_key_field_selection: None,
        create_table_foreign_key_fields_draft: None,
        create_table_foreign_key_referenced_fields_draft: None,
        create_table_type_selects: BTreeMap::new(),
        _create_table_type_select_subscriptions: BTreeMap::new(),
        create_table_custom_type_editing: None,
        create_table_enum_scrolls: BTreeMap::new(),
        create_table_selects: BTreeMap::new(),
        _create_table_select_subscriptions: BTreeMap::new(),
        data_export_object_selects: BTreeMap::new(),
        _data_export_object_select_subscriptions: BTreeMap::new(),
        query_editors: BTreeMap::new(),
        query_statement_statuses: BTreeMap::new(),
        query_execution_sources: BTreeMap::new(),
        query_save_targets: BTreeMap::new(),
        // 初始为空，首帧后由 load_persisted_history_in_background 异步填充。
        saved_queries: Vec::new(),
        persisted_history_loaded: false,
        _query_editor_subscriptions: BTreeMap::new(),
        query_find_inputs: BTreeMap::new(),
        query_replace_inputs: BTreeMap::new(),
        _query_find_input_subscriptions: BTreeMap::new(),
        _query_replace_input_subscriptions: BTreeMap::new(),
        redis_workbench_inputs: BTreeMap::new(),
        _redis_workbench_input_subscriptions: BTreeMap::new(),
        terminal_sessions: BTreeMap::new(),
        _terminal_session_pump: None,
        pubsub_sessions: BTreeMap::new(),
        _pubsub_pump: None,
        redis_workbench_panel_resize_start: None,
        query_output_tabs: BTreeMap::new(),
        collapsed_query_outputs: BTreeSet::new(),
        // 启动时的结果区布局取自持久化设置（「查询结果默认布局」）；
        // 工具栏上的切换按钮只改这个运行时值，不回写设置。
        results_placement: settings_editor_draft.results_placement,
        query_result_display_pages: BTreeMap::new(),
        settings_panel_section: debug_ai_settings_section(),
        ai_model_dialog_debug_pending: debug_ai_model_dialog_requested(),
        ai_plugin_config_dialog_debug_pending: debug_ai_plugin_config_target(),
        ai_model_dialog_filter_input,
        _ai_model_dialog_filter_subscription: ai_model_dialog_filter_subscription,
        ai_model_dialog_filter: String::new(),
        ai_model_filter_input,
        _ai_model_filter_subscription: ai_model_filter_subscription,
        ai_model_filter: String::new(),
        ai_model_testing_id: None,
        ai_model_deleting_id: None,
        ai_settings_loading: false,
        ai_settings_error: None,
        ai_tools_tab: debug_ai_tools_tab(),
        ai_mcp_install_status: Vec::new(),
        ai_mcp_clients_busy: None,
        ai_mcp_clients_error: None,
        ai_mcp_connection_details: None,
        ai_mcp_connection_error: None,
        ai_mcp_details_open: false,
        ai_mcp_binary_override: None,
        ai_agent_editor: None,
        ai_agent_editor_debug_pending: debug_ai_agent_draft_requested(),
        ai_agent_delete_just_removed: String::new(),
        ai_session_filter_input,
        _ai_session_filter_subscription: ai_session_filter_subscription,
        ai_session_filter: String::new(),
        ai_audit_filter: None,
        ai_audit_filter_input,
        _ai_audit_filter_subscription: ai_audit_filter_subscription,
        ai_audit_filter_text: String::new(),
        ai_audit_expanded: None,
        ai_session_archiving_id: None,
        ai_report_modal: Rc::new(std::cell::RefCell::new(AiReportModalData::default())),
        _ai_report_task: None,
        _ai_report_export_task: None,
        ai_diagnostics_modal: Rc::new(std::cell::RefCell::new(AiDiagnosticsModalData::default())),
        _ai_diagnostics_task: None,
        er_proposal_modal: Rc::new(std::cell::RefCell::new(ErProposalModalData::default())),
        _er_proposal_task: None,
        ai_session_rename_input,
        _ai_session_rename_subscription: ai_session_rename_subscription,
        pending_ai_session_rename: None,
        ai_model_test_debug_pending: debug_ai_model_test_requested(),
        settings_editor_draft,
        settings_dangerous_actions_collapsed: false,
        // 备份默认展开（常用），客户端默认折叠（标题右侧有状态摘要）。
        settings_data_groups_collapsed: SettingsDataGroupsCollapsed {
            backup: false,
            pg_client: true,
            mysql_client: true,
        },
        settings_font_size_slider,
        _settings_font_size_slider_subscription: settings_font_size_slider_subscription,
        settings_line_height_input,
        _settings_line_height_subscription: settings_line_height_subscription,
        settings_radius_input,
        _settings_radius_subscription: settings_radius_subscription,
        pg_client: NativeClientState {
            source_input: settings_pg_client_source_input,
            _source_subscription: settings_pg_client_source_subscription,
            status: None,
            status_missing: false,
            download: None,
            detect_task: None,
            download_task: None,
            progress_task: None,
        },
        mysql_client: NativeClientState {
            source_input: settings_mysql_client_source_input,
            _source_subscription: settings_mysql_client_source_subscription,
            status: None,
            status_missing: false,
            download: None,
            detect_task: None,
            download_task: None,
            progress_task: None,
        },
        backup_pg_client_missing: false,
        backup_mysql_client_missing: false,
        query_output_heights: BTreeMap::new(),
        query_output_widths: BTreeMap::new(),
        query_output_resize_start: None,
        query_result_sort_rules: BTreeMap::new(),
        query_result_cell_detail: BTreeMap::new(),
                            large_value_windows: BTreeMap::new(),
        data_change_sql_preview_tabs: BTreeSet::new(),
        data_filter_panels: BTreeSet::new(),
        data_filter_rules: BTreeMap::new(),
        data_sort_rules: BTreeMap::new(),
        data_filter_draft_rules: BTreeMap::new(),
        data_sort_draft_rules: BTreeMap::new(),
        data_sort_clear_undo: BTreeMap::new(),
        data_filter_grouped_tabs: BTreeSet::new(),
        data_filter_modes: BTreeMap::new(),
        data_filter_texts: BTreeMap::new(),
        data_filter_condition_completion_signatures: BTreeMap::new(),
        data_sort_texts: BTreeMap::new(),
        data_filter_panel_heights: BTreeMap::new(),
        data_filter_panel_resize_start: None,
        cell_detail_drawer_heights: BTreeMap::new(),
        cell_detail_drawer_resize_start: None,
        data_filter_popover: None,
        local_filter_popover: None,
        local_filter_manager_popover: None,
        local_filter_value: String::new(),
        local_filter_search: String::new(),
        local_filter_draft_values: BTreeSet::new(),
        local_filter_manager_field: None,
        local_filter_manager_draft_filters: BTreeMap::new(),
        local_filter_manager_field_open: false,
        local_filter_manager_values_open: false,
        local_table_filters: BTreeMap::new(),
        query_history_open: false,
        redis_history_open: false,
        redis_history_scope: None,
        data_filter_value_input_text: String::new(),
        data_filter_value_search: String::new(),
        data_filter_value_search_loading_until: None,
        data_filter_value_search_task: None,
        data_filter_value_draft: None,
        data_filter_applying_tabs: BTreeSet::new(),
        _data_filter_apply_tasks: BTreeMap::new(),
        _data_filter_apply_generations: BTreeMap::new(),
        app_message: None,
        _app_message_task: None,
        table_hover_color: None,
        table_hover_blocked_by_overlay: false,
        show_connection_browser: true,
        connection_browser_width: CONNECTION_BROWSER_DEFAULT_WIDTH,
        connection_browser_resize_start: None,
        table_info_resize_start: None,
    }
}
