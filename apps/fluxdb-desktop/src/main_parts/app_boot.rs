fn main() {
    let storage = FileStorage::default();
    let saved_settings = storage.load_settings().unwrap_or_default();
    // 最先初始化日志系统；guard 持有到 main 结束，保证日志 worker 线程存活。
    let _log_guard = init_logging(saved_settings.log_level, &saved_settings.log_path);
    tracing::info!(
        target: "fluxdb_desktop",
        "FluxDB 启动，日志已初始化（目录: {}）",
        configured_log_dir(&saved_settings.log_path).display()
    );

    gpui_platform::application()
        .with_assets(Assets {
            base: app_assets_base_path(),
        })
        .run(move |cx: &mut App| {
            set_dock_icon();
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
            let mut controller = AppController::with_mock_data();
            controller.set_completion_index_storage(storage.clone());
            let theme_mode = theme_mode_from_app_theme(saved_settings.theme);
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
                    titlebar: Some(TitlebarOptions {
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(17.), px(12.))),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    install_close_to_tray(window, cx);
                    let view = cx.new(|cx| {
                        let rename_group_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("分组名称"));
                        let rename_group_subscription = cx.subscribe(
                            &rename_group_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(
                                    event,
                                    InputEvent::Change | InputEvent::PressEnter { .. }
                                ) {
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
                            cx.new(|cx| InputState::new(window, cx).placeholder("分组名称"));
                        let table_folder_rename_subscription = cx.subscribe(
                            &table_folder_rename_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(
                                    event,
                                    InputEvent::Change | InputEvent::PressEnter { .. }
                                ) {
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
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索数据库..."));
                        let display_database_search_subscription = cx.subscribe(
                            &display_database_search_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.display_database_search =
                                        input.read(cx).value().to_string();
                                    cx.notify();
                                }
                            },
                        );
                        let create_database_name_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("数据库名称"));
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
                        let danger_table_foreign_key_check_select = cx.new(|cx| {
                            SelectState::new(
                                SearchableVec::new(danger_table_foreign_key_check_options()),
                                danger_table_foreign_key_check_index(ForeignKeyCheckMode::Default),
                                window,
                                cx,
                            )
                        });
                        let rename_table_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("表名"));
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
                        let copy_table_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("表名"));
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
                            cx.new(|cx| InputState::new(window, cx).placeholder("success"));
                        let column_choice_label_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("成功"));
                        let query_save_name_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("查询名称"));
                        let sidebar_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索表"));
                        let sidebar_search_subscription = cx.subscribe(
                            &sidebar_search_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.sidebar_search = input.read(cx).value().to_string();
                                    cx.notify();
                                }
                            },
                        );
                        let field_filter_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索字段..."));
                        let field_filter_search_subscription = cx.subscribe(
                            &field_filter_search_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.update_field_filter_search(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
                                }
                            },
                        );
                        let data_filter_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索"));
                        let data_filter_value_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("输入筛选值"));
                        let local_filter_value_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("输入筛选值"));
                        let local_filter_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索"));
                        let data_filter_text_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .multi_line(true)
                                .placeholder("例如：`question` LIKE '%0.648238%'")
                        });
                        let data_sort_text_input = cx.new(|cx| {
                            InputState::new(window, cx).placeholder("<Field> ASC, <Field> DESC")
                        });
                        let data_sql_panel_input = cx.new(|cx| InputState::new(window, cx));
                        let data_page_input = cx.new(|cx| InputState::new(window, cx));
                        let settings_line_height_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("14"));
                        let settings_radius_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("6"));
                        let data_cell_edit_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("输入值"));
                        let temporal_part_input = cx.new(|cx| InputState::new(window, cx));
                        let cell_detail_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .code_editor("json")
                                .line_number(false)
                                .rows(10)
                                .placeholder("编辑单元格值")
                        });
                        let data_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("输入搜索内容"));
                        let redis_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索"));
                        let redis_key_value_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder("值")
                                .multi_line(true)
                                .rows(12)
                        });
                        let redis_json_key_value_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .code_editor(JSON_HIGHLIGHT_LANGUAGE)
                                .line_number(false)
                                .placeholder("值")
                                .rows(12)
                        });
                        let redis_key_name_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("键名称"));
                        let redis_key_ttl_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("TTL"));
                        let redis_stream_entry_id_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("*"));
                        let redis_stream_entry_field_rows =
                            vec![new_redis_stream_entry_field_inputs(window, cx)];
                        let redis_set_member_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索"));
                        let redis_set_member_search_subscription = cx.subscribe_in(
                            &redis_set_member_search_input,
                            window,
                            |this: &mut NavicatMain,
                             input,
                             event: &InputEvent,
                             _window,
                             cx| {
                                // 程序化 set_value 触发的 Change（syncing=true）不算用户输入，
                                // 避免与初始打开/Enter 重复发起搜索。
                                if matches!(event, InputEvent::Change)
                                    && !this.redis_set_member_search_syncing
                                {
                                    let _ = this
                                        .active_redis_selected_key(cx)
                                        .map(|(tab_id, key)| {
                                            // 切换 key 时可能残留携带旧 key 的 Change 事件：
                                            // 只当捕获 key 仍是当前 panel 的 key 时才排入防抖，
                                            // 避免 350ms 后对已不展示的旧键误发首屏搜索（WRONGTYPE）。
                                            if this.redis_set_member_search_active.as_ref()
                                                != Some(&(tab_id, key.clone()))
                                            {
                                                return;
                                            }
                                            let query = input.read(cx).value().to_string();
                                            this.redis_set_member_search_queries
                                                .insert((tab_id, key.clone()), query.clone());
                                            this.schedule_redis_set_member_search(
                                                tab_id, key, query, cx,
                                            );
                                        });
                                }
                                // Enter：立即发起搜索（取消在飞防抖）。
                                if matches!(event, InputEvent::PressEnter { .. }) {
                                    if let Some((tab_id, key)) =
                                        this.active_redis_selected_key(cx)
                                    {
                                        if this.redis_set_member_search_active.as_ref()
                                            != Some(&(tab_id, key.clone()))
                                        {
                                            return;
                                        }
                                        this.redis_set_member_search_debounce = None;
                                        this.redis_set_member_search_debounce_until = None;
                                        let query = input.read(cx).value().to_string();
                                        this.redis_set_member_search_queries
                                            .insert((tab_id, key.clone()), query.clone());
                                        this.request_redis_set_member_search(
                                            tab_id, key, query, String::new(), cx,
                                        );
                                    }
                                }
                            },
                        );
                        let redis_hash_field_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索字段"));
                        let redis_hash_value_edit_input =
                            cx.new(|cx| {
                                InputState::new(window, cx)
                                    .placeholder("Value")
                                    .multi_line(true)
                                    .rows(3)
                                    .legacy_soft_wrap(true)
                            });
                        let redis_hash_ttl_edit_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("TTL"));
                        let redis_hash_field_search_subscription = cx.subscribe_in(
                            &redis_hash_field_search_input,
                            window,
                            |this: &mut NavicatMain,
                             input,
                             event: &InputEvent,
                             _window,
                             cx| {
                                if matches!(event, InputEvent::Change)
                                    && !this.redis_hash_field_search_syncing
                                {
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
                                    this.schedule_redis_hash_field_search(
                                        tab_id, key, query, cx,
                                    );
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
                                    this.request_redis_hash_field_search(
                                        tab_id, key, query, String::new(), cx,
                                    );
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
                            |this: &mut NavicatMain,
                             input,
                             event: &InputEvent,
                             window,
                             cx| {
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
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索成员"));
                        let redis_zset_member_search_subscription = cx.subscribe_in(
                            &redis_zset_member_search_input,
                            window,
                            |this: &mut NavicatMain,
                             input,
                             event: &InputEvent,
                             _window,
                             cx| {
                                if matches!(event, InputEvent::Change)
                                    && !this.redis_zset_member_search_syncing
                                {
                                    let _ = this
                                        .active_redis_selected_key(cx)
                                        .map(|(tab_id, key)| {
                                            let query = input.read(cx).value().to_string();
                                            this.redis_zset_member_search_queries
                                                .insert((tab_id, key.clone()), query.clone());
                                            this.schedule_redis_zset_member_search(
                                                tab_id, key, query, cx,
                                            );
                                        });
                                }
                                if matches!(event, InputEvent::PressEnter { .. }) {
                                    if let Some((tab_id, key)) = this.active_redis_selected_key(cx) {
                                        this.redis_zset_member_search_debounce = None;
                                        this.redis_zset_member_search_debounce_until = None;
                                        let query = input.read(cx).value().to_string();
                                        this.redis_zset_member_search_queries
                                            .insert((tab_id, key.clone()), query.clone());
                                        this.request_redis_zset_member_search(
                                            tab_id, key, query, String::new(), cx,
                                        );
                                    }
                                }
                            },
                        );
                        let redis_list_item_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("按索引搜索"));
                        let redis_list_item_search_subscription = cx.subscribe_in(
                            &redis_list_item_search_input,
                            window,
                            |this: &mut NavicatMain,
                             input,
                             event: &InputEvent,
                             window,
                             cx| {
                                // 输入框只允许数字：Change 时即时剔除非数字字符（重写为数字串）。
                                // 搜索不做键入自动触发，仅在 Enter 时「按索引跳转」（对齐 RedisInsight）。
                                if matches!(event, InputEvent::Change)
                                    && !this.redis_list_item_search_syncing
                                {
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
                                        this.redis_list_item_search_active =
                                            Some((tab_id, key.clone()));
                                        this.request_redis_list_item_search(
                                            tab_id,
                                            key,
                                            query,
                                            String::new(),
                                            cx,
                                        );
                                    }
                                }
                            },
                        );
                        let redis_list_value_edit_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder("Value")
                                .multi_line(true)
                                .rows(3)
                                .legacy_soft_wrap(true)
                        });
                        let redis_list_value_edit_subscription = cx.subscribe_in(
                            &redis_list_value_edit_input,
                            window,
                            |this: &mut NavicatMain,
                             _input,
                             event: &InputEvent,
                             _window,
                             cx| {
                                // 保持编辑输入与进行中的编辑行同步：切 key/标签页时由
                                // sync 逻辑清理；按下 Enter 确认。取消由编辑弹层内的
                                // CancelDialog action 处理（全局 Escape 键绑定）。
                                if matches!(event, InputEvent::PressEnter { .. }) {
                                    this.confirm_redis_list_item_edit(cx);
                                }
                            },
                        );
                        let redis_list_item_remove_count_input = cx.new(|cx| {
                            InputState::new(window, cx).placeholder("请输入数量").default_value("1")
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
                             _event: &SelectEvent<SearchableVec<String>>,
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
                            |this: &mut NavicatMain,
                             input,
                             event: &InputEvent,
                             window,
                             cx| {
                                // 数量只允许数字：Change 时即时剔除非数字字符（复用 TTL 数字清洗）。
                                if matches!(event, InputEvent::Change)
                                    && !this.redis_list_item_search_syncing
                                {
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
                        let redis_add_key_name_input = cx
                            .new(|cx| InputState::new(window, cx).placeholder("Key 名称"));
                        let redis_add_key_name_subscription = cx.subscribe_in(
                            &redis_add_key_name_input,
                            window,
                            |this: &mut NavicatMain,
                             _input,
                             _event: &InputEvent,
                             _window,
                             _cx| {
                                this.redis_add_key_applying = false;
                            },
                        );
                        let redis_add_key_ttl_input = cx
                            .new(|cx| InputState::new(window, cx).placeholder("TTL（秒，可选）"));
                        let redis_add_key_ttl_subscription = cx.subscribe_in(
                            &redis_add_key_ttl_input,
                            window,
                            |this: &mut NavicatMain,
                             input,
                             event: &InputEvent,
                             window,
                             cx| {
                                // TTL 只允许数字：Change 时即时剔除非数字字符（复用 TTL 数字清洗）。
                                if matches!(event, InputEvent::Change)
                                    && !this.redis_list_item_search_syncing
                                {
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
                        let redis_add_key_string_input = cx.new(|cx| {
                            InputState::new(window, cx).placeholder("值")
                        });
                        let redis_add_key_string_subscription = cx.subscribe_in(
                            &redis_add_key_string_input,
                            window,
                            |this: &mut NavicatMain, _input, _event: &InputEvent, _window, _cx| {
                                this.redis_add_key_applying = false;
                            },
                        );
                        let redis_add_key_json_input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .code_editor(JSON_HIGHLIGHT_LANGUAGE)
                                .line_number(false)
                                .placeholder("{}")
                        });
                        let redis_add_key_json_subscription = cx.subscribe_in(
                            &redis_add_key_json_input,
                            window,
                            |this: &mut NavicatMain, _input, _event: &InputEvent, _window, _cx| {
                                this.redis_add_key_applying = false;
                            },
                        );
                        let redis_add_key_stream_id_input = cx.new(|cx| {
                            InputState::new(window, cx).placeholder("留空自动生成")
                        });
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
                            InputState::new(window, cx).placeholder("搜索字段名或值...")
                        });
                        let tab_switcher_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索标签页"));
                        let query_history_search_input = cx.new(|cx| {
                            InputState::new(window, cx).placeholder("搜索 SQL / 连接 / 库 / 表")
                        });
                        let query_history_quick_search_input = cx.new(|cx| {
                            InputState::new(window, cx).placeholder("搜索执行过的 SQL...")
                        });
                        let redis_history_search_input = cx.new(|cx| {
                            InputState::new(window, cx).placeholder("搜索命令 / 连接 / DB...")
                        });
                        let query_history_connection_select = cx.new(|cx| {
                            SelectState::new(
                                SearchableVec::new(query_history_connection_filter_items(
                                    controller.state(),
                                )),
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
                                        .map(|connection_id| {
                                            sql_file_database_items(controller.state(), connection_id)
                                        })
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
                            cx.new(|cx| InputState::new(window, cx).placeholder("选择 SQL 文件"));
                        let backup_file_name_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("默认：库名_时间戳.sql"));
                        let backup_object_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索表"));
                        // 新建备份弹框：备注输入（成功后写入 .meta.json）。
                        let backup_note_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("备注（可空）"));
                        // 备份 tab：备注编辑弹框输入。
                        let backup_note_edit_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("备注（留空表示清除）"));
                        let user_admin_search_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("搜索用户或 Host"));
                        let user_admin_create_user_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("用户名"));
                        let user_admin_create_host_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("Host"));
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
                                .placeholder("确认密码")
                                .masked(true)
                        });
                        let user_admin_new_password_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("密码").masked(true));
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
                            cx.new(|cx| InputState::new(window, cx).placeholder("Cipher"));
                        let user_admin_ssl_issuer_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("证书发行者"));
                        let user_admin_ssl_subject_input =
                            cx.new(|cx| InputState::new(window, cx).placeholder("证书主旨"));
                        let data_sql_panel_subscription = cx.subscribe(
                            &data_sql_panel_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.update_data_editor_sql_text(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
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
                                    this.update_data_search_query(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
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
                                    this.apply_data_page_input(
                                        input.read(cx).value().to_string(),
                                        window,
                                        cx,
                                    );
                                }
                            },
                        );
                        let settings_line_height_subscription = cx.subscribe_in(
                            &settings_line_height_input,
                            window,
                            |this: &mut NavicatMain, input, event: &InputEvent, _window, cx| {
                                if !matches!(event, InputEvent::Change | InputEvent::PressEnter { .. })
                                {
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
                            |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
                                match event {
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
                                }
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
                                let is_editing_detail = this.controller.state().active_tab().is_some_and(|tab| {
                                    match &tab.kind {
                                        TabKind::DataEditor(editor) => {
                                            editor.cell_detail_panel.mode == CellDetailMode::Edit
                                        }
                                        TabKind::QueryEditor(editor) => {
                                            active_query_result_editor_state(editor).is_some_and(|editor| {
                                                editor.cell_detail_panel.mode == CellDetailMode::Edit
                                            })
                                        }
                                        _ => false,
                                    }
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
                                    this.query_history_search =
                                        input.read(cx).value().to_string();
                                    cx.notify();
                                }
                            },
                        );
                        let query_history_quick_search_subscription = cx.subscribe(
                            &query_history_quick_search_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                match event {
                                    InputEvent::Change => {
                                        this.query_history_quick_search =
                                            input.read(cx).value().to_string();
                                        this.query_history_quick_selected = 0;
                                        cx.notify();
                                    }
                                    InputEvent::PressEnter { .. } => {
                                        this.confirm_query_history_quick_search(cx);
                                    }
                                    InputEvent::Focus | InputEvent::Blur => {}
                                }
                            },
                        );
                        let redis_history_search_subscription = cx.subscribe(
                            &redis_history_search_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.redis_history_search =
                                        input.read(cx).value().to_string();
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
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                match event {
                                    InputEvent::Change => {
                                        if this.redis_key_meta_syncing {
                                            return;
                                        }
                                        let Some((tab_id, key)) = this.active_redis_selected_key(cx)
                                        else {
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
                                }
                            },
                        );
                        let redis_key_ttl_subscription = cx.subscribe_in(
                            &redis_key_ttl_input,
                            window,
                            |this: &mut NavicatMain, input, event: &InputEvent, window, cx| {
                                match event {
                                    InputEvent::Change => {
                                        if this.redis_key_meta_syncing {
                                            return;
                                        }
                                        let Some((tab_id, key)) = this.active_redis_selected_key(cx)
                                        else {
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
                                }
                            },
                        );
                        let redis_type_select_subscription = cx.subscribe_in(
                            &redis_type_select,
                            window,
                            |this: &mut NavicatMain,
                             _select,
                             event: &SelectEvent<SearchableVec<String>>,
                             _window,
                             cx| {
                                let SelectEvent::Confirm(value) = event;
                                if let (Some(tab_id), Some(value)) =
                                    (this.active_redis_data_tab_id(), value)
                                {
                                    this.set_redis_type_filter(tab_id, value.clone(), cx);
                                }
                            },
                        );
                        let query_history_connection_select_subscription = cx.subscribe_in(
                            &query_history_connection_select,
                            window,
                            |this: &mut NavicatMain,
                             _select,
                             event: &SelectEvent<
                                SearchableVec<QueryHistoryConnectionFilterItem>,
                            >,
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
                                                .find(|connection| {
                                                    connection.config.id == *connection_id
                                                })
                                                .and_then(|connection| {
                                                    connection_default_database(&connection.config)
                                                });
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
                                if let (Some(tab_id), Some(plugin)) =
                                    (this.active_user_admin_tab_id(), value)
                                {
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
                                if let (Some(tab_id), Some(policy)) =
                                    (this.active_user_admin_tab_id(), value)
                                {
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
                                if let (Some(tab_id), Some(ssl_type)) =
                                    (this.active_user_admin_tab_id(), value)
                                {
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
                                    this.select_create_database_collation(
                                        collation.as_str(),
                                        window,
                                        cx,
                                    );
                                }
                            },
                        );
                        let danger_table_foreign_key_check_select_subscription = cx.subscribe_in(
                            &danger_table_foreign_key_check_select,
                            window,
                            |this: &mut NavicatMain,
                             _select,
                             event: &SelectEvent<SearchableVec<String>>,
                             _window,
                             cx| {
                                let SelectEvent::Confirm(value) = event;
                                if let (Some(value), Some(form)) =
                                    (value, &mut this.pending_danger_table_action)
                                {
                                    form.foreign_key_check =
                                        danger_table_foreign_key_check_from_label(value);
                                    form.error = None;
                                    cx.notify();
                                }
                            },
                        );
                        let data_filter_value_input_subscription = cx.subscribe(
                            &data_filter_value_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.update_data_filter_manual_value(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
                                }
                            },
                        );
                        let local_filter_value_input_subscription = cx.subscribe(
                            &local_filter_value_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.update_local_filter_value(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
                                }
                            },
                        );
                        let local_filter_search_subscription = cx.subscribe(
                            &local_filter_search_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.update_local_filter_search(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
                                }
                            },
                        );
                        let data_filter_text_subscription = cx.subscribe(
                            &data_filter_text_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.update_data_filter_text(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
                                }
                            },
                        );
                        let data_sort_text_subscription = cx.subscribe(
                            &data_sort_text_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.update_data_sort_text(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
                                }
                            },
                        );
                        let data_filter_search_subscription = cx.subscribe(
                            &data_filter_search_input,
                            |this: &mut NavicatMain, input, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    this.update_data_filter_value_search(
                                        input.read(cx).value().to_string(),
                                        cx,
                                    );
                                }
                            },
                        );
                        let table_folders =
                            controller.state().sidebar_layout.table_folders.clone();
                        let table_folder_assignments = controller
                            .state()
                            .sidebar_layout
                            .table_folder_assignments
                            .clone();
                        let settings_editor_draft = controller.state().settings.clone();
                        settings_radius_input.update(cx, |input, cx| {
                            input.set_value(
                                settings_editor_draft.button_radius.to_string(),
                                window,
                                cx,
                            )
                        });
                        let settings_font_size_slider = cx.new(|_| {
                            SliderState::new()
                                .min(10.)
                                .max(24.)
                                .step(1.)
                                .default_value(
                                    settings_editor_draft.editor_font_size.clamp(10, 24) as f32,
                                )
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
                        NavicatMain {
                            focus_handle: cx.focus_handle(),
                            controller,
                            storage,
                            theme_mode,
                            new_connection_kind: None,
                            new_connection_tab: NewConnectionTab::Connection,
                            new_connection_form: NewConnectionForm::for_kind(
                                DatabaseKind::MySql,
                                1,
                            ),
                            new_connection_inputs: NewConnectionInputs::new(window, cx),
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
                            _data_filter_value_input_subscription:
                                data_filter_value_input_subscription,
                            local_filter_value_input,
                            _local_filter_value_input_subscription:
                                local_filter_value_input_subscription,
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
                            redis_key_list_hovered_tab: None,
                            redis_workbench_hovered_record: None,
                            redis_stream_table_state,
                            redis_hash_table_state,
                            redis_list_table_state,
                            redis_key_value_input,
                            _redis_key_value_subscription: redis_key_value_subscription,
                            redis_json_key_value_input,
                            _redis_json_key_value_subscription: redis_json_key_value_subscription,
                            redis_json_editor: JsonEditorState::new(JsonEditorConfig::default()),
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
                            _redis_set_member_search_subscription:
                                redis_set_member_search_subscription,
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
                            _redis_hash_field_search_subscription:
                                redis_hash_field_search_subscription,
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
                            redis_hash_full_value_viewer: Rc::new(RefCell::new(None)),
                            pending_redis_hash_field_drawer: None,
                            redis_hash_field_drawer_rows: Vec::new(),
                            redis_hash_field_drawer_scroll: ScrollHandle::new(),
                            pending_redis_hash_field_delete: None,
                            redis_zset_member_search_input,
                            _redis_zset_member_search_subscription:
                                redis_zset_member_search_subscription,
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
                            _redis_list_item_search_subscription:
                                redis_list_item_search_subscription,
                            redis_stream_entry_active: None,
                            redis_stream_since_input: cx.new(|cx| {
                                InputState::new(window, cx).placeholder("起始时间 2026-08-09 12:00:00")
                            }),
                            redis_stream_until_input: cx.new(|cx| {
                                InputState::new(window, cx).placeholder("结束时间")
                            }),
                            redis_stream_ranges: BTreeMap::new(),
                            redis_stream_maxlen_input: cx
                                .new(|cx| InputState::new(window, cx).placeholder("MAXLEN")),
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
                            _redis_list_item_remove_count_subscription:
                                redis_list_item_remove_count_subscription,
                            redis_list_item_remove_select,
                            _redis_list_item_remove_select_subscription:
                                redis_list_item_remove_select_subscription,
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
                            _redis_add_key_stream_id_subscription:
                                redis_add_key_stream_id_subscription,
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
                            _query_history_quick_search_subscription:
                                query_history_quick_search_subscription,
                            query_history_quick_search: String::new(),
                            redis_history_search_input,
                            _redis_history_search_subscription: redis_history_search_subscription,
                            redis_history_search: String::new(),
                            query_history_quick_open: false,
                            query_history_quick_selected: 0,
                            query_history_quick_kind_filter: QueryHistoryKindFilter::All,
                            query_history_connection_select,
                            _query_history_connection_select_subscription:
                                query_history_connection_select_subscription,
                            query_history_database_select,
                            _query_history_database_select_subscription:
                                query_history_database_select_subscription,
                            query_history_table_select,
                            _query_history_table_select_subscription:
                            query_history_table_select_subscription,
                            sql_file_connection_select,
                            _sql_file_connection_select_subscription:
                                sql_file_connection_select_subscription,
                            sql_file_database_select,
                            _sql_file_database_select_subscription:
                                sql_file_database_select_subscription,
                            sql_file_encoding_select,
                            _sql_file_encoding_select_subscription:
                                sql_file_encoding_select_subscription,
                            sql_file_path_input,
                            _sql_file_path_subscription: sql_file_path_subscription,
                            backup_file_name_input,
                            _backup_file_name_subscription: backup_file_name_subscription,
                            backup_object_search_input,
                            _backup_object_search_subscription: backup_object_search_subscription,
                            backup_note_input,
                            _backup_note_subscription: backup_note_subscription,
                            backup_tables_modal: None,
                            backup_note_modal_path: None,
                            backup_note_edit_input,
                            backup_pending_metas: BTreeMap::new(),
                            pending_delete_backup: None,
                            user_admin_search_input,
                            _user_admin_search_subscription: user_admin_search_subscription,
                            user_admin_create_user_input,
                            _user_admin_create_user_subscription:
                                user_admin_create_user_subscription,
                            user_admin_create_host_input,
                            _user_admin_create_host_subscription:
                                user_admin_create_host_subscription,
                            user_admin_auth_plugin_select,
                            _user_admin_auth_plugin_select_subscription:
                                user_admin_auth_plugin_select_subscription,
                            user_admin_password_expiry_select,
                            _user_admin_password_expiry_select_subscription:
                                user_admin_password_expiry_select_subscription,
                            user_admin_create_password_input,
                            _user_admin_create_password_subscription:
                                user_admin_create_password_subscription,
                            user_admin_new_password_input,
                            _user_admin_new_password_subscription:
                                user_admin_new_password_subscription,
                            user_admin_max_queries_input,
                            _user_admin_max_queries_subscription:
                                user_admin_max_queries_subscription,
                            user_admin_max_updates_input,
                            _user_admin_max_updates_subscription:
                                user_admin_max_updates_subscription,
                            user_admin_max_connections_input,
                            _user_admin_max_connections_subscription:
                                user_admin_max_connections_subscription,
                            user_admin_max_user_connections_input,
                            _user_admin_max_user_connections_subscription:
                                user_admin_max_user_connections_subscription,
                            user_admin_ssl_type_select,
                            _user_admin_ssl_type_select_subscription:
                                user_admin_ssl_type_select_subscription,
                            user_admin_ssl_cipher_input,
                            _user_admin_ssl_cipher_subscription:
                                user_admin_ssl_cipher_subscription,
                            user_admin_ssl_issuer_input,
                            _user_admin_ssl_issuer_subscription:
                                user_admin_ssl_issuer_subscription,
                            user_admin_ssl_subject_input,
                            _user_admin_ssl_subject_subscription:
                                user_admin_ssl_subject_subscription,
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
                            hovered_database_tab: None,
                            display_database_search_input,
                            _display_database_search_subscription:
                                display_database_search_subscription,
                            create_database_name_input,
                            _create_database_name_subscription: create_database_name_subscription,
                            create_database_charset_select,
                            _create_database_charset_select_subscription:
                                create_database_charset_select_subscription,
                            create_database_collation_select,
                            _create_database_collation_select_subscription:
                                create_database_collation_select_subscription,
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
                            _connection_tasks: BTreeMap::new(),
                            _database_tasks: BTreeMap::new(),
                            _data_load_tasks: BTreeMap::new(),
                            _redis_key_value_apply_tasks: BTreeMap::new(),
                            _query_execute_tasks: BTreeMap::new(),
                            _sql_file_execute_tasks: BTreeMap::new(),
                            _sql_file_cancel_flags: BTreeMap::new(),
                            _query_completion_tasks: BTreeMap::new(),
                            _completion_index_tasks: BTreeMap::new(),
                            _table_info_tasks: BTreeMap::new(),
                            _user_admin_users_tasks: BTreeMap::new(),
                            _user_admin_grants_tasks: BTreeMap::new(),
                            _user_admin_member_grants_tasks: BTreeMap::new(),
                            _user_admin_apply_tasks: BTreeMap::new(),
                            _create_table_apply_tasks: BTreeMap::new(),
                            _create_table_reference_columns_tasks: BTreeMap::new(),
                            _cell_binary_download_tasks: BTreeMap::new(),
                            _data_export_tasks: BTreeMap::new(),
                            _data_export_cancel_flags: BTreeMap::new(),
                            _backup_tasks: BTreeMap::new(),
                            _backup_cancel_flags: BTreeMap::new(),
                            _create_database_tasks: BTreeMap::new(),
                            _delete_database_tasks: BTreeMap::new(),
                            _rename_table_task: None,
                            _copy_table_task: None,
                            _copy_table_ddl_task: None,
                            _copy_table_structure_task: None,
                            _danger_table_task: None,
                            data_export_task_seq: 0,
                            _test_connection_task: None,
                            _redis_discover_task: None,
                            redis_discovery_pending_sync: false,
                            connection_context_menu: None,
                            database_context_menu: None,
                            table_context_menu: None,
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
                            pending_disconnect_connection: None,
                            pending_close_workspace: None,
                            pending_new_query_connection: None,
                            pending_delete_data_row: None,
                            data_row_viewer: None,
                            pending_dirty_data_action: None,
                            pending_apply_data_changes: None,
                            pending_query_parameters: None,
                            pending_dangerous_query: None,
                            pending_dangerous_redis_command: None,
                            sql_file_modal: Rc::new(std::cell::RefCell::new(
                                SqlFileModalData::default(),
                            )),
                            pending_data_export: None,
                            data_export_custom_conditions_open: false,
                            data_export_preview: None,
                            data_export_preview_seq: 0,
                            _data_export_preview_task: None,
                            pending_table_data_export_after_load: None,
                            pending_rename_table: None,
                            pending_copy_table: None,
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
                            create_table_selects: BTreeMap::new(),
                            _create_table_select_subscriptions: BTreeMap::new(),
                            data_export_object_selects: BTreeMap::new(),
                            _data_export_object_select_subscriptions: BTreeMap::new(),
                            query_editors: BTreeMap::new(),
                            query_statement_statuses: BTreeMap::new(),
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
                            query_output_placement: QueryOutputPlacement::Bottom,
                            query_result_display_pages: BTreeMap::new(),
                            settings_panel_section: SettingsPanelSection::Editor,
                            settings_editor_draft,
                            settings_font_size_slider,
                            _settings_font_size_slider_subscription:
                                settings_font_size_slider_subscription,
                            settings_line_height_input,
                            _settings_line_height_subscription: settings_line_height_subscription,
                            settings_radius_input,
                            _settings_radius_subscription: settings_radius_subscription,
                            query_output_heights: BTreeMap::new(),
                            query_output_widths: BTreeMap::new(),
                            query_output_resize_start: None,
                            query_result_sort_rules: BTreeMap::new(),
                            query_result_cell_detail: BTreeMap::new(),
                            data_change_sql_preview_tabs: BTreeSet::new(),
                            data_filter_panels: BTreeSet::new(),
                            data_filter_rules: BTreeMap::new(),
                            data_sort_rules: BTreeMap::new(),
                            data_filter_draft_rules: BTreeMap::new(),
                            data_sort_draft_rules: BTreeMap::new(),
                            data_filter_grouped_tabs: BTreeSet::new(),
                            data_filter_modes: BTreeMap::new(),
                            data_filter_texts: BTreeMap::new(),
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
                            data_filter_applying_tabs: BTreeSet::new(),
                            _data_filter_apply_tasks: BTreeMap::new(),
                            app_message: None,
                            _app_message_task: None,
                            table_hover_color: None,
                            table_hover_blocked_by_overlay: false,
                            show_connection_browser: true,
                            connection_browser_width: CONNECTION_BROWSER_DEFAULT_WIDTH,
                            connection_browser_resize_start: None,
                            table_info_resize_start: None,
                        }
                    });
                    // Tab 拦截已迁移到通用 Editor 的键绑定体系，不再需要全局拦截。
                    // 首帧后异步恢复查询/工作台历史与保存的查询（不阻塞建窗；
                    // saved_queries 经内部 notify 补显，详见 load_persisted_history_in_background）。
                    view.update(cx, |this, cx| {
                        this.load_persisted_history_in_background(cx);
                    });
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .unwrap();
            cx.activate(true);
        });
}

fn set_app_menus(cx: &mut App) {
    cx.set_menus(vec![Menu {
        name: "FluxDB".into(),
        disabled: true,
        items: vec![],
    }]);
}

/// List 删除元素 - 位置下拉选项（对齐 RedisInsight）。
/// 第一个「从尾部移除」为默认选中项（head=false → RPOP）。
const REDIS_LIST_REMOVE_FROM_TAIL: &str = "从尾部移除";
const REDIS_LIST_REMOVE_FROM_HEAD: &str = "从头部移除";

/// 删除位置选项列表；下标 0 对应从尾删除（默认）。
fn redis_list_remove_position_options() -> Vec<String> {
    [REDIS_LIST_REMOVE_FROM_TAIL, REDIS_LIST_REMOVE_FROM_HEAD]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn query_history_record_to_entry(record: QueryHistoryRecord) -> QueryHistoryEntry {
    let kind = query_history_kind_from_storage(&record.kind);
    let success = record.success;
    let text = record.text;
    QueryHistoryEntry {
        connection_id: record.connection_id,
        database: record.database,
        text: text.clone(),
        tables: record.tables,
        kind,
        success,
        summary: QueryExecutionSummary {
            sql: text.clone(),
            kind: query_statement_kind_from_history(kind),
            success,
            message: record
                .message
                .unwrap_or_else(|| if success { "OK" } else { "执行失败" }.to_string()),
            returned_rows: record.returned_rows,
            affected_rows: record.affected_rows,
            elapsed_ms: record.elapsed_ms,
        },
        executed_at_unix_secs: record.executed_at_unix_secs,
        object: record.object,
        rollback_snapshot: record.rollback_snapshot,
    }
}

fn query_history_entry_to_record(entry: &QueryHistoryEntry) -> QueryHistoryRecord {
    QueryHistoryRecord {
        connection_id: entry.connection_id,
        database: entry.database.clone(),
        text: entry.text.clone(),
        tables: entry.tables.clone(),
        kind: query_history_kind_to_storage(entry.kind).to_string(),
        success: entry.success,
        executed_at_unix_secs: entry.executed_at_unix_secs,
        object: entry.object.clone(),
        rollback_sql: None,
        rollback_snapshot: entry.rollback_snapshot.clone(),
        message: Some(entry.summary.message.clone()),
        returned_rows: entry.summary.returned_rows,
        affected_rows: entry.summary.affected_rows,
        elapsed_ms: entry.summary.elapsed_ms,
    }
}

/// 把 App 层 Redis 历史记录转成持久化记录（source 序列化为枚举名）。
fn redis_workbench_entry_to_record(
    entry: &fluxdb_app::RedisWorkbenchHistoryEntry,
) -> RedisWorkbenchHistoryRecord {
    RedisWorkbenchHistoryRecord {
        id: entry.id,
        connection_id: entry.connection_id,
        database: entry.database,
        text: entry.text.clone(),
        success: entry.success,
        executed_at_unix_secs: entry.executed_at_unix_secs,
        summary: entry.summary.clone(),
        source: format!("{:?}", entry.source),
    }
}

/// 把 source 字符串还原成 `CommandExecutionSource`（枚举名匹配，未知回退 KeyShortcut）。
fn redis_workbench_source_from_str(source: &str) -> fluxdb_core::CommandExecutionSource {
    match source {
        "Workbench" => fluxdb_core::CommandExecutionSource::Workbench,
        "HistoryRerun" => fluxdb_core::CommandExecutionSource::HistoryRerun,
        _ => fluxdb_core::CommandExecutionSource::KeyShortcut,
    }
}

fn query_statement_kind_from_history(kind: QueryHistoryKind) -> fluxdb_core::QueryStatementKind {
    match kind {
        QueryHistoryKind::Query => fluxdb_core::QueryStatementKind::ResultSet,
        QueryHistoryKind::DataChange | QueryHistoryKind::SchemaChange => {
            fluxdb_core::QueryStatementKind::Command
        }
    }
}

fn query_history_kind_from_storage(kind: &str) -> QueryHistoryKind {
    match kind {
        "data_change" => QueryHistoryKind::DataChange,
        "schema_change" => QueryHistoryKind::SchemaChange,
        _ => QueryHistoryKind::Query,
    }
}

fn query_history_kind_to_storage(kind: QueryHistoryKind) -> &'static str {
    match kind {
        QueryHistoryKind::Query => "query",
        QueryHistoryKind::DataChange => "data_change",
        QueryHistoryKind::SchemaChange => "schema_change",
    }
}

fn app_assets_base_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(contents_dir) = exe_path.parent().and_then(|macos_dir| macos_dir.parent()) {
                let bundled_assets = contents_dir.join("Resources").join("assets");
                if bundled_assets.exists() {
                    return bundled_assets;
                }
            }
        }
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

fn register_shortcuts(cx: &mut App, settings: &Settings) {
    // 新的通用编辑器与 SQL 连接层：注册通用编辑器快捷键 + SQL 执行快捷键。
    editor_component::register_editor_shortcuts(cx);
    sql_editor_adapter::register_sql_execute_shortcuts(cx);
    // 旧 SQL 编辑器快捷键已随旧 sql_editor 模块移除，不再注册，避免与新的
    // EditorComponent 快捷键绑定冲突。
    register_terminal_shortcuts(cx);

    // 应用级退出快捷键绑定在无上下文层级，确保编辑器或表格聚焦时也能退出。
    cx.on_action(|_: &Quit, cx| {
        tracing::info!(target: "fluxdb_desktop", "收到退出快捷键，退出应用");
        cx.quit();
    });
    cx.bind_keys([KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-q"
        } else {
            "ctrl-q"
        },
        Quit,
        None,
    )]);

    for definition in SHORTCUT_DEFINITIONS {
        bind_shortcut(
            cx,
            &current_shortcut(settings, definition),
            definition.action,
            definition.context,
        );
    }
    cx.bind_keys([
        KeyBinding::new(
            "up",
            QueryHistoryQuickSearchPrevious,
            Some("QueryHistoryQuickSearch"),
        ),
        KeyBinding::new(
            "down",
            QueryHistoryQuickSearchNext,
            Some("QueryHistoryQuickSearch"),
        ),
        KeyBinding::new(
            "enter",
            QueryHistoryQuickSearchConfirm,
            Some("QueryHistoryQuickSearch"),
        ),
        KeyBinding::new("escape", CancelDialog, None),
        KeyBinding::new(
            "delete",
            DeleteConnectionShortcut,
            Some("ConnectionContextMenu"),
        ),
        KeyBinding::new(
            "backspace",
            DeleteConnectionShortcut,
            Some("ConnectionContextMenu"),
        ),
        KeyBinding::new(
            "delete",
            DeleteConnectionShortcut,
            Some("DeleteConnectionModal"),
        ),
        KeyBinding::new(
            "backspace",
            DeleteConnectionShortcut,
            Some("DeleteConnectionModal"),
        ),
        KeyBinding::new(
            "enter",
            DeleteConnectionShortcut,
            Some("DeleteConnectionModal"),
        ),
    ]);
}
