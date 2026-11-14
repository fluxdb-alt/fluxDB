mod i18n_e2e_tests {
    use std::fs;
    use std::path::Path;

    use fluxdb_app::AppController;
    use fluxdb_core::Settings;
    use fluxdb_storage::{FileStorage, Storage as _};
    use gpui::{AppContext as _, Modifiers, ParentElement as _, Styled as _, TestAppContext, px};
    use gpui_component::Root;

    use crate::{
        NavicatMain, SettingsPanelSection, app_assets_base_path, extend_component_i18n,
        init_i18n, navicat_main_new,
    };

    include!("i18n_tests/large_value_window.rs");
    include!("i18n_tests/redis_stream.rs");
    include!("i18n_tests/redis_key_selection.rs");
    include!("i18n_tests/config_proposal.rs");
    include!("i18n_tests/sql_editor_edit.rs");
    include!("i18n_tests/table_page_query.rs");
    include!("i18n_tests/dameng_connection.rs");
    // 生产视图里的 Redis 拓扑显式选择 + 节点列表完整落库。
    include!("i18n_tests/redis_topology.rs");
    // 生产视图里的 PostgreSQL 连接串导入与真实库端到端（Podman 容器 + 环境变量门控）。
    include!("i18n_tests/postgres_connection.rs");
    // 连接表单字段按高频/低频分页展示。
    include!("i18n_tests/connection_layout.rs");
    // 编辑已有连接时档案必须完整回填（MySQL TLS、Oracle 下拉光标）。
    include!("i18n_tests/connection_edit_roundtrip.rs");
    include!("i18n_tests/sqlserver_connection.rs");
    // 生产视图里的 MongoDB SSH 隧道端到端（Podman 容器 + 环境变量门控）。
    include!("i18n_tests/ssh_tunnel_connection.rs");
    include!("i18n_tests/dameng_completion.rs");
    include!("i18n_tests/dameng_table_design.rs");
    include!("i18n_tests/table_design_types.rs");
    include!("i18n_tests/table_copy_error.rs");
    include!("i18n_tests/table_danger_layout.rs");
    include!("i18n_tests/table_context_menu_layout.rs");
    include!("i18n_tests/mongo_collection_menu.rs");
    // 任务中心：状态栏入口、任务列表与右侧详情抽屉。
    include!("i18n_tests/task_center.rs");
    // 服务器面板：会话/锁/状态页签与终止确认门槛。
    include!("i18n_tests/server_panel.rs");
    include!("i18n_tests/compare_transfer.rs");
    include!("i18n_tests/mongo_query_panel.rs");
    include!("i18n_tests/mongo_aggregation_cards.rs");
    include!("i18n_tests/mongo_workbench_cards.rs");
    include!("i18n_tests/mongo_collection_selection.rs");
    include!("i18n_tests/mongo_indexes_page.rs");
    include!("i18n_tests/sqlserver_table_context.rs");
    include!("i18n_tests/user_admin_toolbar.rs");
    // 新建用户一次保存的真实库端到端测试（MySQL/PG/Oracle/达梦，`#[ignore]` + 环境变量门控）。
    include!("i18n_tests/user_admin_one_shot_live.rs");
    include!("i18n_tests/sqlserver_security.rs");
    include!("i18n_tests/oracle_security.rs");
    include!("i18n_tests/oracle_completion.rs");
    include!("i18n_tests/query_summary_layout.rs");
    include!("i18n_tests/query_scope_switch.rs");
    include!("i18n_tests/command_palette.rs");
    include!("i18n_tests/tab_switching.rs");
    include!("i18n_tests/query_pane_split.rs");
    include!("i18n_tests/query_plan_scroll.rs");
    include!("i18n_tests/query_incomplete_footer.rs");
    include!("i18n_tests/sql_completion_mysql_e2e.rs");
    include!("i18n_tests/editor_signature_dismiss.rs");

    /// 进程级 locale 是全局状态；会切换语言的用例必须串行，否则互相污染断言。
    fn locale_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn write_user_pack(root: &std::path::Path) {
        let locale_dir = root.join("ja");
        fs::create_dir_all(&locale_dir).expect("create external ja pack");
        fs::write(
            locale_dir.join("manifest.json"),
            r#"{
                "format_version": 1,
                "locale": "ja",
                "native_name": "日本語",
                "english_name": "Japanese"
            }"#,
        )
        .expect("write ja manifest");
        fs::write(
            locale_dir.join("app.json"),
            r#"{
                "window_title": "FluxDB — データベースクライアント",
                "settings": {
                    "section": { "appearance": "外観" },
                    "appearance": {
                        "language": "言語",
                        "language_detail": "アプリの表示言語を選択します",
                        "follow_system": "システムに従う"
                    }
                },
                "common": { "save": "保存" }
            }"#,
        )
        .expect("write ja app translations");
        fs::write(
            locale_dir.join("gpui_component.json"),
            r#"{
                "DatePicker": { "placeholder": "日付を選択" }
            }"#,
        )
        .expect("write ja component translations");
    }

    fn flatten_json_keys(value: &serde_json::Value, prefix: &str, output: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    let next = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    flatten_json_keys(child, &next, output);
                }
            }
            _ => output.push(format!("app.{prefix}")),
        }
    }

    fn literal_tr_keys(dir: &Path, output: &mut Vec<String>) {
        static KEY_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let key_re = KEY_RE.get_or_init(|| {
            regex::Regex::new(r#"\btr\("([^"]+)"\)"#).expect("literal key regex")
        });
        for entry in fs::read_dir(dir).expect("read source directory") {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                literal_tr_keys(&path, output);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let source = fs::read_to_string(&path).expect("read Rust source");
                for capture in key_re.captures_iter(&source) {
                    let key = capture[1].to_string();
                    if key.starts_with("app.") && !output.contains(&key) {
                        output.push(key);
                    }
                }
            }
        }
    }

    #[test]
    fn every_literal_ui_key_exists_in_bundled_packs() {
        let mut used = Vec::new();
        literal_tr_keys(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut used,
        );
        for locale in ["en", "zh-CN"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/locales")
                .join(locale)
                .join("app.json");
            let value: serde_json::Value = serde_json::from_slice(
                &fs::read(&path).expect("read bundled locale pack"),
            )
            .expect("parse bundled locale pack");
            let mut available = Vec::new();
            flatten_json_keys(&value, "", &mut available);
            let missing = used
                .iter()
                .filter(|key| !available.contains(key))
                .cloned()
                .collect::<Vec<_>>();
            assert!(missing.is_empty(), "{locale} missing keys: {missing:?}");
        }
    }

    #[gpui::test]
    fn ai_chat_tab_is_a_global_first_level_tab(cx: &mut TestAppContext) {
        use crate::{
            FirstLevelScope, WorkspaceTabGroup, first_level_has_second_row, first_level_tab_groups,
            tab_first_level_scope,
        };
        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
        let mut controller = AppController::new();
        // 本用例只验证一级标签分组；关闭补全索引预热点，避免不存在的 SQLite
        // 目标在用例结束后仍留下后台连接任务。
        controller.dispatch(fluxdb_app::AppCommand::SaveSettings(Settings {
            enable_completion_index: false,
            ..Default::default()
        }));
        controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(vec![
            fluxdb_core::ConnectionConfig {
                id: fluxdb_core::ConnectionId(1),
                name: "workspace".into(),
                kind: fluxdb_core::DatabaseKind::Sqlite,
                endpoint: fluxdb_core::Endpoint::SqliteFile {
                    path: temp.path().join("db.sqlite"),
                    read_only: true,
                },
                options: Default::default(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: None,
                mongo_profile: None,
                sqlserver_profile: None,
                oracle_profile: None,
            },
        ]));
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });
        let profile = storage.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                navicat_main_new(
                    window,
                    cx,
                    controller,
                    profile,
                    gpui_component::ThemeMode::Light,
                )
            });
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let view = cx.update(|window, cx| {
            window
                .root::<Root>()
                .expect("root")
                .expect("typed root")
                .read(cx)
                .view()
                .clone()
                .downcast::<NavicatMain>()
                .expect("production view")
        });
        // 只验证标签分组；跳过首帧会话恢复，避免后台存储任务越过用例边界
        // 后向已结束的测试调度器投递唤醒。
        cx.update(|_, cx| {
            view.update(cx, |app, _| {
                app.ai_chat.recovery_pending = false;
            });
        });
        cx.update(|_, cx| {
            view.update(cx, |app, cx| {
                app.dispatch(fluxdb_app::AppCommand::OpenQueryEditorInDatabase {
                    connection_id: fluxdb_core::ConnectionId(1),
                    database: None,
                    schema: None,
                }, cx);
                cx.notify();
            });
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("table-tab-row").is_some());
        cx.update(|_, cx| {
            view.update(cx, |app, cx| {
                app.dispatch(fluxdb_app::AppCommand::OpenAiChatTab, cx);
                let state = app.controller.state();
                let ai = state
                    .tabs
                    .iter()
                    .find(|tab| matches!(tab.kind, fluxdb_app::TabKind::AiChat { .. }))
                    .expect("AI tab");
                assert_eq!(tab_first_level_scope(ai), Some(FirstLevelScope::AiChat));
                assert_eq!(ai.workspace(), None);
                let groups = first_level_tab_groups(state);
                assert!(groups
                    .iter()
                    .any(|group| matches!(group, WorkspaceTabGroup::AiChat { .. })));
                assert!(groups
                    .iter()
                    .any(|group| matches!(group, WorkspaceTabGroup::Database { .. })));
                assert!(!first_level_has_second_row(Some(&FirstLevelScope::AiChat)));
                assert!(!first_level_has_second_row(Some(&FirstLevelScope::Settings)));
                assert!(first_level_has_second_row(Some(&FirstLevelScope::Database(
                    fluxdb_app::TabWorkspace {
                        connection_id: fluxdb_core::ConnectionId(1),
                        database: "main".into(),
                    },
                ))));
            });
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("table-tab-row").is_none());
    }

    #[gpui::test]
    fn redis_databases_render_below_one_connection_tab(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
        let mut controller = AppController::new();
        controller.dispatch(fluxdb_app::AppCommand::SaveSettings(Settings {
            enable_completion_index: false,
            ..Default::default()
        }));
        controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(vec![
            fluxdb_core::ConnectionConfig {
                id: fluxdb_core::ConnectionId(1),
                name: "docker-redis".into(),
                kind: fluxdb_core::DatabaseKind::Redis,
                endpoint: fluxdb_core::Endpoint::Tcp {
                    host: "redis.invalid".into(),
                    port: 6379,
                    database: None,
                },
                options: Default::default(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: None,
                mongo_profile: None,
                sqlserver_profile: None,
                oracle_profile: None,
            },
        ]));
        let fluxdb_app::AppEvent::TabOpened(db0_tab) =
            controller.dispatch(fluxdb_app::AppCommand::OpenRedisWorkbench {
                connection_id: fluxdb_core::ConnectionId(1),
                database: 0,
            })
        else {
            panic!("open Redis DB 0 workbench");
        };
        let fluxdb_app::AppEvent::TabOpened(db1_tab) =
            controller.dispatch(fluxdb_app::AppCommand::OpenRedisWorkbench {
                connection_id: fluxdb_core::ConnectionId(1),
                database: 1,
            })
        else {
            panic!("open Redis DB 1 workbench");
        };

        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });
        let profile = storage.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                navicat_main_new(
                    window,
                    cx,
                    controller,
                    profile,
                    gpui_component::ThemeMode::Light,
                )
            });
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        let view = cx.update(|window, cx| {
            window
                .root::<Root>()
                .expect("root")
                .expect("typed root")
                .read(cx)
                .view()
                .clone()
                .downcast::<NavicatMain>()
                .expect("production view")
        });
        cx.update(|_, cx| {
            view.update(cx, |app, _| {
                app.ai_chat.recovery_pending = false;
            });
        });
        cx.update(|_, cx| {
            view.update(cx, |app, cx| {
                app.dispatch(
                    fluxdb_app::AppCommand::UpdateRedisWorkbenchText {
                        tab_id: db1_tab,
                        text: "PING".into(),
                    },
                    cx,
                );
                let pending = app.close_workspace_warning(
                    crate::FirstLevelScope::RedisConnection(fluxdb_core::ConnectionId(1)),
                );
                assert_eq!(pending.unsaved_queries, 1);
            });
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        assert_eq!((db0_tab.0, db1_tab.0), (1, 2));
        assert!(cx.debug_bounds("first-level-tab:1").is_some());
        assert!(cx.debug_bounds("first-level-tab:2").is_none());
        assert!(cx.debug_bounds("workspace-tab:1").is_some());
        assert!(cx.debug_bounds("workspace-tab:2").is_some());
    }

    #[test]
    fn bundled_packs_have_identical_keys_and_placeholders() {
        fn flatten_leaves(
            value: &serde_json::Value,
            prefix: &str,
            output: &mut std::collections::BTreeMap<String, String>,
        ) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, child) in map {
                        let next = if prefix.is_empty() {
                            key.clone()
                        } else {
                            format!("{prefix}.{key}")
                        };
                        flatten_leaves(child, &next, output);
                    }
                }
                serde_json::Value::String(text) => {
                    output.insert(prefix.to_string(), text.clone());
                }
                _ => {}
            }
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales");
        let mut packs = Vec::new();
        for locale in ["en", "zh-CN"] {
            let value: serde_json::Value = serde_json::from_slice(
                &fs::read(root.join(locale).join("app.json")).expect("read bundled locale pack"),
            )
            .expect("parse bundled locale pack");
            let mut leaves = std::collections::BTreeMap::new();
            flatten_leaves(&value, "", &mut leaves);
            packs.push((locale, leaves));
        }
        let en = &packs[0].1;
        let zh = &packs[1].1;
        assert_eq!(
            en.keys().collect::<Vec<_>>(),
            zh.keys().collect::<Vec<_>>(),
            "bundled packs must contain the same keys"
        );
        let placeholder_re = regex::Regex::new(r"%\{[a-z_]+\}").expect("placeholder regex");
        for (key, en_text) in en {
            let mut en_slots = placeholder_re
                .find_iter(en_text)
                .map(|m| m.as_str())
                .collect::<Vec<_>>();
            let mut zh_slots = placeholder_re
                .find_iter(&zh[key])
                .map(|m| m.as_str())
                .collect::<Vec<_>>();
            en_slots.sort_unstable();
            zh_slots.sort_unstable();
            assert_eq!(en_slots, zh_slots, "{key} placeholder mismatch");
        }
    }

    #[test]
    fn dynamic_translation_tables_reference_existing_keys() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"));
        let sources = [
            ("src/main_parts/backend_errors.rs", "app.backend_error."),
            (
                "src/main_parts/backend_errors_external.rs",
                "app.backend_error.",
            ),
            (
                "src/main_parts/backend_errors_connectors.rs",
                "app.backend_error.",
            ),
            (
                "src/main_parts/backend_errors_storage.rs",
                "app.backend_error.",
            ),
            (
                "src/main_parts/backend_errors_templates.rs",
                "app.backend_error.",
            ),
            ("src/main_parts/ai/approval.rs", "app.ai.guardrail."),
            (
                "src/main_parts/settings/ai_plugin_text.rs",
                "app.ai.plugins.text.",
            ),
        ];
        let key_re = regex::Regex::new(r#""(app\.[a-z0-9_.]+)""#).expect("dynamic key regex");
        let mut used = Vec::new();
        for (relative, prefix) in sources {
            let source = fs::read_to_string(base.join(relative)).expect("read dynamic key source");
            for capture in key_re.captures_iter(&source) {
                let key = capture[1].to_string();
                if key.starts_with(prefix) && !used.contains(&key) {
                    used.push(key);
                }
            }
        }
        assert!(!used.is_empty(), "dynamic key tables must be detected");
        for locale in ["en", "zh-CN"] {
            let value: serde_json::Value = serde_json::from_slice(
                &fs::read(base.join("assets/locales").join(locale).join("app.json"))
                    .expect("read bundled locale pack"),
            )
            .expect("parse bundled locale pack");
            let mut available = Vec::new();
            flatten_json_keys(&value, "", &mut available);
            let missing = used
                .iter()
                .filter(|key| !available.contains(key))
                .cloned()
                .collect::<Vec<_>>();
            assert!(missing.is_empty(), "{locale} missing dynamic keys: {missing:?}");
        }
    }

    #[test]
    fn restore_action_plan_keeps_stable_values() {
        use crate::{RestoreActionChoice, restore_row_action_plan};
        use fluxdb_core::RestoreTableAction;

        let (options, default_index) = restore_row_action_plan(true, true, true);
        assert_eq!(default_index, 0);
        assert_eq!(options[0].choice, RestoreActionChoice::Pending);
        assert_eq!(
            options[1].choice,
            RestoreActionChoice::Action(RestoreTableAction::Recreate)
        );
        assert_eq!(
            options[2].choice,
            RestoreActionChoice::Action(RestoreTableAction::TruncateAndLoad)
        );
        assert_eq!(
            options[3].choice,
            RestoreActionChoice::Action(RestoreTableAction::Append)
        );
        assert_eq!(
            options[4].choice,
            RestoreActionChoice::Action(RestoreTableAction::Skip)
        );

        let (options, _) = restore_row_action_plan(false, true, true);
        assert_eq!(
            options[0].choice,
            RestoreActionChoice::Action(RestoreTableAction::Create)
        );
        assert_eq!(
            options[1].choice,
            RestoreActionChoice::Action(RestoreTableAction::Skip)
        );
    }

    #[test]
    fn pg_draft_option_labels_match_under_switched_locale() {
        use crate::{pg_password_op_label, pg_valid_until_label};
        use fluxdb_core::{PgPasswordOp, PgValidUntilOp};

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        write_user_pack(&storage.root_dir().join("locales"));
        let settings = Settings {
            locale: "ja".to_string(),
            ..Settings::default()
        };
        storage.save_settings(&settings).expect("save settings");
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            &settings.locale,
        );

        // 下拉选中值会回灌业务枚举，标签必须随当前语言变化，
        // 否则按标签匹配的下拉会退化成默认值。
        assert_eq!(
            pg_valid_until_label(&PgValidUntilOp::Clear).as_str(),
            rust_i18n::t!("app.pg_role.valid_until.clear").as_ref(),
        );
        assert_eq!(
            pg_password_op_label(false, &PgPasswordOp::Set(String::new())).as_str(),
            rust_i18n::t!("app.pg_role.password.set_new").as_ref(),
        );
        // 归还默认语言，避免影响后续用例的断言基准。
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
    }

    #[test]
    fn ai_scope_permission_keeps_stable_tiers_under_switched_locale() {
        use crate::AiScopePermission;

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        write_user_pack(&storage.root_dir().join("locales"));
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "zh-CN",
        );

        let target = serde_json::json!({ "database": "app" });
        let none = serde_json::Value::Null;
        // 档位由稳定输入决定：翻译只影响文案，不影响只读/需审批的判定，
        // 否则切换语言后审批卡的图标与颜色会退化成默认值。
        assert_eq!(
            AiScopePermission::from_target(Some(&target), true),
            AiScopePermission::ReadOnly
        );
        assert_eq!(
            AiScopePermission::from_target(Some(&target), false),
            AiScopePermission::WriteNeedsApproval
        );
        assert_eq!(
            // 生产侧对解析不到连接的 target 用 `unwrap_or(true)` 兜底，这里同口径。
            AiScopePermission::from_target(Some(&none), true),
            AiScopePermission::ReadOnly,
            "无连接时按只读兜底，不能因为翻译缺失而放宽"
        );
        assert_eq!(
            AiScopePermission::from_target(None, true),
            AiScopePermission::ExplainAndPlan
        );
        assert_ne!(
            AiScopePermission::ReadOnly.label(),
            AiScopePermission::WriteNeedsApproval.label(),
            "两个档位不能共用同一句文案"
        );

        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
    }

    #[test]
    fn localized_select_options_keep_stable_business_values() {
        use gpui_component::select::SelectItem as _;
        use crate::{
            ConnectionSelectOption, ForeignKeyCheckOption, RedisListRemovePositionOption,
        };

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated locale root");
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "zh-CN");

        let ssh = [
            ConnectionSelectOption {
                value: "password".into(),
                label_key: "app.connection.secret.password",
            },
            ConnectionSelectOption {
                value: "private_key".into(),
                label_key: "app.connection.secret.private_key",
            },
        ];
        assert_eq!(ssh[0].value(), "password");
        assert_eq!(ssh[1].value(), "private_key");
        assert_eq!(ssh[0].title().as_ref(), "密码");
        assert_eq!(ssh[1].title().as_ref(), "私钥");

        let fk = [
            ForeignKeyCheckOption {
                mode: fluxdb_app::ForeignKeyCheckMode::Default,
            },
            ForeignKeyCheckOption {
                mode: fluxdb_app::ForeignKeyCheckMode::Enable,
            },
            ForeignKeyCheckOption {
                mode: fluxdb_app::ForeignKeyCheckMode::Disable,
            },
        ];
        assert_eq!(*fk[1].value(), fluxdb_app::ForeignKeyCheckMode::Enable);
        assert_eq!(fk[1].title().as_ref(), "启用");

        let positions = [
            RedisListRemovePositionOption { head: false },
            RedisListRemovePositionOption { head: true },
        ];
        assert!(!*positions[0].value());
        assert!(*positions[1].value());
        assert_eq!(positions[0].title().as_ref(), "从尾部移除");
        assert_eq!(positions[1].title().as_ref(), "从头部移除");

        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
        assert_eq!(ssh[0].title().as_ref(), "Password");
        assert_eq!(fk[2].title().as_ref(), "Disable");
        assert_eq!(positions[1].title().as_ref(), "Remove from head");
    }

    #[test]
    fn backend_error_messages_translate_known_text_and_keep_unknown() {
        use crate::localize_backend_error_text;

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated locale root");
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
        assert_eq!(
            localize_backend_error_text("连接不存在"),
            "Connection not found"
        );
        assert_eq!(
            localize_backend_error_text("SQL Server 连接任务异常结束"),
            "SQL Server connection task terminated unexpectedly"
        );
        assert_eq!(
            localize_backend_error_text("导入二进制值格式无效"),
            "Invalid binary value; use 0x-prefixed hexadecimal"
        );
        assert_eq!(
            localize_backend_error_text(
                "SQL Server 查询超时，连接已关闭；执行结果待核实"
            ),
            "SQL Server query timed out; the connection was closed and the result must be verified"
        );
        assert_eq!(
            localize_backend_error_text(
                "查询已取消，SQL Server 连接已关闭；执行结果待核实"
            ),
            "Query cancelled; the SQL Server connection was closed and the result must be verified"
        );
        assert_eq!(
            localize_backend_error_text("SQL 无法解析，无法校验作用域"),
            "SQL cannot be parsed, so scope cannot be validated"
        );
        assert_eq!(
            localize_backend_error_text("检索已取消"),
            "Search cancelled"
        );
        assert_eq!(
            localize_backend_error_text("对象不在授权范围内"),
            "Object is outside the authorized scope"
        );
        assert_eq!(
            localize_backend_error_text("该连接有尚未核实的未知写入，禁止继续修改"),
            "This connection has an unverified unknown write; further modifications are blocked"
        );
        assert_eq!(
            localize_backend_error_text("数据库执行失败：duplicate key"),
            "Database execution failed: duplicate key"
        );
        assert_eq!(
            localize_backend_error_text("parameters[0] 不是合法 base64"),
            "parameters[0] is not valid base64"
        );
        assert_eq!(
            localize_backend_error_text("第 2 步标题过长"),
            "Step 2 title is too long"
        );
        assert_eq!(
            localize_backend_error_text("来源 s1 的结果里没有字段 amount"),
            "The result of source s1 has no field amount"
        );
        assert_eq!(
            localize_backend_error_text("表名不能为空"),
            "Table name cannot be empty"
        );
        assert_eq!(
            localize_backend_error_text("ER 存储尚未初始化"),
            "ER storage is not initialized"
        );
        assert_eq!(localize_backend_error_text("查询失败"), "Query failed");
        assert_eq!(
            localize_backend_error_text("正在压缩上下文（达到上下文 80%）…"),
            "Compacting context (reached 80% of context)…"
        );
        assert_eq!(
            localize_backend_error_text("AI 运行时繁忙，请稍后重试"),
            "AI runtime is busy; try again later"
        );
        assert_eq!(
            localize_backend_error_text("网络错误: connection reset"),
            "Network error: connection reset"
        );
        assert_eq!(
            localize_backend_error_text("连接类型不匹配"),
            "Connection type mismatch"
        );
        assert_eq!(
            localize_backend_error_text("Redis Key 不存在"),
            "Redis key not found"
        );
        assert_eq!(
            localize_backend_error_text("配置已被其他窗口修改，请重新加载"),
            "Configuration was modified in another window; reload it"
        );
        assert_eq!(localize_backend_error_text("模型不存在"), "Model not found");
        assert_eq!(
            localize_backend_error_text("模拟查询错误"),
            "Simulated query error"
        );
        assert_eq!(
            localize_backend_error_text("SQLite 文件目录不存在：/tmp/x"),
            "SQLite file directory does not exist: /tmp/x"
        );
        assert_eq!(
            localize_backend_error_text("SSH 握手失败: timeout"),
            "SSH handshake failed: timeout"
        );
        assert_eq!(
            localize_backend_error_text("Agent 不存在或状态已变化，请重新加载"),
            "The agent does not exist or its state changed; reload it"
        );
        assert_eq!(
            localize_backend_error_text("连接 3 缺少端口"),
            "Connection 3 is missing a port"
        );
        assert_eq!(
            localize_backend_error_text("元数据检索失败: boom"),
            "Metadata search failed: boom"
        );
        assert_eq!(
            localize_backend_error_text("导入失败：age 是 int，最多支持 4，当前文件 8"),
            "Import failed: age is int; the maximum is 4, current file 8"
        );
        assert_eq!(
            localize_backend_error_text("CSV 解析失败（第 3 条记录附近）：bad"),
            "CSV parsing failed near record 3: bad"
        );
        assert_eq!(
            localize_backend_error_text("返回 5 行"),
            "Returned 5 rows"
        );
        assert_eq!(
            localize_backend_error_text("有界提交，影响 3 行"),
            "Bounded commit; 3 rows affected"
        );
        assert_eq!(
            localize_backend_error_text("读取密钥配置失败: denied"),
            "Failed to read secret configuration: denied"
        );
        assert_eq!(
            localize_backend_error_text("high → 综合 0.85 / 阈值 0.70"),
            "high → combined 0.85 / threshold 0.70"
        );
        assert_eq!(
            localize_backend_error_text("设计表: users"),
            "Design table: users"
        );
        // 未知错误不能吞掉或改写，便于继续定位原始报文。
        assert_eq!(
            localize_backend_error_text("数据库返回的原始错误: permission denied"),
            "数据库返回的原始错误: permission denied"
        );

        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "zh-CN");
        assert_eq!(localize_backend_error_text("连接不存在"), "连接不存在");
    }

    #[test]
    fn ai_guardrails_translate_known_text_and_keep_unknown() {
        use crate::ai_guardrail_display_text;

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated locale root");
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
        assert_eq!(
            ai_guardrail_display_text("计划风险尚未核实"),
            "Plan risk has not been verified"
        );
        assert_eq!(
            ai_guardrail_display_text("内核策略已拒绝该语句（denied_function:drop）"),
            "Kernel policy rejected the statement (denied_function:drop)"
        );
        assert_eq!(
            ai_guardrail_display_text("custom guardrail"),
            "custom guardrail"
        );

        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "zh-CN");
        assert_eq!(
            ai_guardrail_display_text("计划风险尚未核实"),
            "计划风险尚未核实"
        );
    }

    #[test]
    fn builtin_plugin_text_translates_known_declarations_and_keeps_unknown() {
        use crate::localize_ai_plugin_text;

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated locale root");
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
        assert_eq!(
            localize_ai_plugin_text("BI 与报表输出"),
            "BI and report output"
        );
        assert_eq!(
            localize_ai_plugin_text("写入与结构"),
            "Writes and schema"
        );
        assert_eq!(
            localize_ai_plugin_text("custom plugin text"),
            "custom plugin text"
        );
        assert_eq!(
            localize_ai_plugin_text("所选模型不支持 tool_call 协议"),
            "The selected model does not support the tool_call protocol"
        );
        assert_eq!(
            localize_ai_plugin_text("字段 `model` 取值无效：missing"),
            "Field `model` has an invalid value: missing"
        );
        assert_eq!(
            localize_ai_plugin_text("只列出已启用、允许出站、凭据可用的生成模型。"),
            "Only enabled, outbound-allowed models with usable credentials are listed (Generation model)."
        );

        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "zh-CN");
        assert_eq!(
            localize_ai_plugin_text("BI 与报表输出"),
            "BI 与报表输出"
        );
    }

    #[test]
    fn builtin_tab_titles_follow_locale_without_changing_state() {
        use crate::localized_tab_title;
        use fluxdb_app::{
            BackupListState, ErDiagramState, RedisCliState, SettingsTabState, TabId, TabKind,
            TabState,
        };
        use fluxdb_core::ConnectionId;

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated locale root");
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
        let tab = |id: u64, title: &str, kind: TabKind| TabState {
            id: TabId(id),
            title: title.to_string(),
            kind,
            dirty: false,
        };
        assert_eq!(
            localized_tab_title(&tab(
                1,
                "设置",
                TabKind::Settings(SettingsTabState)
            )),
            "Settings"
        );
        assert_eq!(
            localized_tab_title(&tab(2, "询问 AI", TabKind::AiChat { workspace: None })),
            "Ask AI"
        );
        assert_eq!(
            localized_tab_title(&tab(
                3,
                "Redis CLI - DB 3",
                TabKind::RedisCli(RedisCliState {
                    connection_id: ConnectionId(1),
                    database: 3,
                })
            )),
            "Redis CLI - DB 3"
        );
        assert_eq!(
            localized_tab_title(&tab(
                4,
                "备份-app",
                TabKind::BackupList(BackupListState {
                    connection_id: ConnectionId(1),
                    database: "app".into(),
                })
            )),
            "Backup - app"
        );
        assert_eq!(
            localized_tab_title(&tab(
                5,
                "app · ER",
                TabKind::ErDiagram(ErDiagramState {
                    connection_id: ConnectionId(1),
                    database: "app".into(),
                    schema: None,
                    center_table: None,
                })
            )),
            "app · ER"
        );

        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "zh-CN");
        assert_eq!(
            localized_tab_title(&tab(
                1,
                "设置",
                TabKind::Settings(SettingsTabState)
            )),
            "设置"
        );
    }


    #[test]
    fn create_table_options_keep_sql_values_and_blank_option_meanings() {
        use crate::CreateTableSelectOption;
        use gpui_component::select::SelectItem as _;

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated locale root");
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
        for (value, inherit_default, expected_title) in [
            ("", true, "Inherit database default"),
            ("", false, ""),
            ("InnoDB", true, "InnoDB"),
            ("CASCADE", false, "CASCADE"),
        ] {
            let option = CreateTableSelectOption {
                value: value.into(),
                inherit_default,
            };
            assert_eq!(option.value(), value);
            assert_eq!(option.title().as_str(), expected_title);
        }
    }

    #[gpui::test]
    fn create_table_inheritance_uses_stable_values_from_localized_dropdown(
        cx: &mut TestAppContext,
    ) {
        use fluxdb_app::{AppCommand, AppEvent, CreateTableTab, TabKind};
        use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        write_user_pack(&storage.root_dir().join("locales"));
        // 故意让继承标题与真实引擎名冲突，验证翻译不能改变 SQL 选项。
        fs::write(
            storage.root_dir().join("locales/ja/app.json"),
            r#"{"create_table":{"inherit_database_default":"InnoDB","header":{"name":"名前"}}}"#,
        )
        .expect("write colliding external translation");
        let bootstrap = init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "ja",
        );
        assert!(bootstrap.issues.is_empty(), "{:?}", bootstrap.issues);
        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
            id: ConnectionId(1),
            name: "i18n UI only".into(),
            kind: DatabaseKind::MySql,
            // 本用例不打开连接、不提交建表，任何意外拨号都不得命中用户实例。
            endpoint: Endpoint::Tcp {
                host: "i18n-e2e.invalid".into(),
                port: 1,
                database: None,
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            mongo_profile: None,
            sqlserver_profile: None,
            oracle_profile: None,
        }]));
        controller.dispatch(AppCommand::SaveSettings(Settings {
            locale: "ja".into(),
            show_sidebar: false,
            ..Settings::default()
        }));
        let AppEvent::TabOpened(tab_id) = controller.dispatch(AppCommand::OpenCreateTable {
            connection_id: ConnectionId(1),
            // 不预选数据库，外键页不会自动发起元数据加载。
            database: None,
            schema: None,
        }) else {
            panic!("create table tab must open without connecting")
        };
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                navicat_main_new(
                    window,
                    cx,
                    controller,
                    storage,
                    gpui_component::ThemeMode::Light,
                )
            });
            Root::new(view, window, cx)
        });
        let view = cx.update(|window, cx| {
            window
                .root::<Root>()
                .expect("root")
                .expect("typed root")
                .read(cx)
                .view()
                .clone()
                .downcast::<NavicatMain>()
                .expect("production view")
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let bounds = cx
            .debug_bounds("create-table-tab:Options")
            .expect("options tab");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        for (selector, expected) in [
            ("create-table-option:MyISAM", "MyISAM"),
            ("create-table-option:InnoDB", "InnoDB"),
            ("create-table-option:", ""),
        ] {
            let bounds = cx
                .debug_bounds("create-table-engine")
                .expect("engine select");
            // 行容器会拉满可用宽度，点击 84px 标签之后的真实 Select 区域。
            cx.simulate_click(
                bounds.origin + gpui::point(gpui::px(150.), bounds.size.height / 2.),
                Modifiers::default(),
            );
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing dropdown option {selector}"));
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            // 同一显示标题的两个选项仍能选中不同业务值，确认订阅链没有按标题匹配。
            let title = if expected == "MyISAM" {
                "create-table-selected:MyISAM"
            } else {
                "create-table-selected:InnoDB"
            };
            assert!(
                cx.debug_bounds(title).is_some(),
                "selected title must be rendered"
            );
            cx.update(|_, cx| {
                view.update(cx, |this, _| {
                    let tab = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .expect("table tab");
                    let TabKind::CreateTable(create) = &tab.kind else {
                        panic!("create state")
                    };
                    assert_eq!(create.engine, expected);
                });
            });
        }
        for (tab_selector, add_selector, heading, expected_tab) in [
            (
                "create-table-tab:Indexes",
                "create-table-add:Add index",
                "create-table-heading:Index type",
                CreateTableTab::Indexes,
            ),
            (
                "create-table-tab:ForeignKeys",
                "create-table-add:Add foreign key",
                "create-table-heading:Target database",
                CreateTableTab::ForeignKeys,
            ),
            (
                "create-table-tab:Checks",
                "create-table-add:Add check",
                "create-table-heading:Not enforced",
                CreateTableTab::Checks,
            ),
            (
                "create-table-tab:Triggers",
                "create-table-add:Add trigger",
                "create-table-heading:Timing",
                CreateTableTab::Triggers,
            ),
        ] {
            let bounds = cx.debug_bounds(tab_selector).expect("subpage tab");
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            assert!(
                cx.debug_bounds("create-table-heading:名前").is_some(),
                "the external translation must be rendered in every subpage"
            );
            assert!(
                cx.debug_bounds(heading).is_some(),
                "omitted headings must fall back to English"
            );
            let bounds = cx
                .debug_bounds(add_selector)
                .expect("translated add action");
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            cx.update(|_, cx| {
                view.update(cx, |this, _| {
                    let tab = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .expect("table tab");
                    let TabKind::CreateTable(create) = &tab.kind else {
                        panic!("create state")
                    };
                    assert_eq!(create.active_tab, expected_tab);
                    match expected_tab {
                        CreateTableTab::Indexes => {
                            assert_eq!(create.indexes.len(), 1);
                            assert_eq!(create.indexes[0].index_type, "");
                            assert_eq!(create.indexes[0].index_method, "");
                        }
                        CreateTableTab::ForeignKeys => {
                            assert_eq!(create.foreign_keys.len(), 1);
                            assert_eq!(create.foreign_keys[0].on_delete, "");
                            assert_eq!(create.foreign_keys[0].on_update, "");
                        }
                        CreateTableTab::Checks => {
                            assert_eq!(create.checks.len(), 1);
                            assert!(!create.checks[0].not_enforced);
                            assert_eq!(create.checks[0].expression, "");
                        }
                        CreateTableTab::Triggers => {
                            assert_eq!(create.triggers.len(), 1);
                            assert_eq!(create.triggers[0].timing, "BEFORE");
                            assert_eq!(create.triggers[0].event, "INSERT");
                        }
                        _ => unreachable!("only editable subpages are listed"),
                    }
                });
            });
        }
        let bounds = cx
            .debug_bounds("create-table-tab:Partitions")
            .expect("partitions tab");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("create-table-heading:Method").is_some());
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
    }

    #[gpui::test]
    fn external_language_pack_is_discovered_and_applied_from_settings(
        cx: &mut TestAppContext,
    ) {
        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        write_user_pack(&storage.root_dir().join("locales"));

        let settings = Settings {
            locale: "en".to_string(),
            show_sidebar: false,
            ..Settings::default()
        };
        storage.save_settings(&settings).expect("save settings");

        let bootstrap = init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            &settings.locale,
        );
        assert_eq!(bootstrap.active_locale, "en");
        assert!(
            bootstrap
                .locales
                .iter()
                .any(|locale| locale.locale == "ja"),
            "external pack must appear without a compiled language list"
        );
        assert!(bootstrap.issues.is_empty(), "{:?}", bootstrap.issues);
        assert_eq!(
            fluxdb_i18n::missing_keys("zh-CN", "en"),
            Vec::<String>::new()
        );
        assert_eq!(
            fluxdb_i18n::placeholder_mismatches("zh-CN", "en"),
            Vec::<String>::new()
        );

        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });
        let component_translation =
            gpui_component::_rust_i18n_try_translate("ja", "DatePicker.placeholder")
                .map(|value| value.into_owned());

        let mut controller = AppController::new();
        let _ = controller.dispatch(fluxdb_app::AppCommand::SaveSettings(settings.clone()));
        let storage_for_view = storage.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                navicat_main_new(
                    window,
                    cx,
                    controller,
                    storage_for_view,
                    gpui_component::ThemeMode::Light,
                )
            });
            Root::new(view, window, cx)
        });

        let view = cx.update(|window, cx| {
            window
                .root::<Root>()
                .expect("base window root")
                .expect("typed window root")
                .read(cx)
                .view()
                .clone()
                .downcast::<NavicatMain>()
                .expect("production NavicatMain root view")
        });
        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                // 前置页面选择不替代任何被断言行为；语言切换与保存仍由真实控件事件驱动。
                this.settings_panel_section = SettingsPanelSection::Appearance;
            });
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let settings_bound = cx
            .debug_bounds("home-settings")
            .expect("home settings action must be rendered");
        cx.simulate_click(settings_bound.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let language_bound = cx
            .debug_bounds("settings-language-dropdown")
            .expect("language dropdown is rendered");
        cx.simulate_click(language_bound.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let language_option = cx
            .debug_bounds("settings-language-option:ja")
            .expect("external ja language is discovered into the language dropdown");
        cx.simulate_click(language_option.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        assert_eq!(fluxdb_i18n::active_locale(), "ja");
        let (password_placeholder, database_placeholder) =
            cx.update(|_, cx| view.read(cx).new_connection_placeholders_for_test(cx));
        assert_eq!(
            password_placeholder.as_ref(),
            "Optional",
            "connection placeholders must follow the switched locale"
        );
        assert_eq!(database_placeholder.as_ref(), "Database");
        assert_eq!(
            rust_i18n::t!("app.settings.appearance.group.theme").as_ref(),
            "Theme",
            "keys omitted by a partial user pack fall back to English"
        );
        assert_eq!(
            rust_i18n::t!("app.settings.appearance.code_font_and_size").as_ref(),
            "Code font and size"
        );
        assert_eq!(
            rust_i18n::t!("app.settings.data.backup_dir").as_ref(),
            "Backup directory"
        );
        assert_eq!(
            rust_i18n::t!("app.settings.system.log_level").as_ref(),
            "Log level"
        );
        assert_eq!(
            rust_i18n::t!("app.settings.native_client.detect").as_ref(),
            "Detect"
        );
        assert_eq!(
            rust_i18n::t!("app.connection.new_title").as_ref(),
            "New connection"
        );
        assert_eq!(
            rust_i18n::t!("app.connection.tab.connection").as_ref(),
            "Connection"
        );
        assert_eq!(rust_i18n::t!("app.topbar.settings").as_ref(), "Settings");
        assert_eq!(
            rust_i18n::t!("app.connection.tls.ssl_mode").as_ref(),
            "SSL mode"
        );
        assert_eq!(
            rust_i18n::t!("app.connection.ssh.private_key").as_ref(),
            "Private key file"
        );
        assert_eq!(
            rust_i18n::t!("app.connection.redis.force_standalone").as_ref(),
            "Force standalone connection"
        );
        assert_eq!(
            rust_i18n::t!("app.ai.models.title").as_ref(),
            "Model profiles"
        );
        assert_eq!(
            rust_i18n::t!("app.ai.model_dialog.display_name").as_ref(),
            "Display name"
        );
        assert_eq!(
            rust_i18n::t!("app.ai.tools.catalog_title").as_ref(),
            "Tool catalog"
        );
        assert_eq!(
            rust_i18n::t!("app.ai.sessions.title").as_ref(),
            "Session history"
        );
        assert_eq!(
            rust_i18n::t!("app.ai.mcp.control_title").as_ref(),
            "MCP control"
        );
        assert_eq!(
            rust_i18n::t!("app.ai.policy.title").as_ref(),
            "Safety policy"
        );
        assert_eq!(rust_i18n::t!("app.ai.plugins.required").as_ref(), "Required");
        assert_eq!(
            rust_i18n::t!("app.sidebar.connections").as_ref(),
            "Connections"
        );
        assert_eq!(
            rust_i18n::t!("app.home.quick_start").as_ref(),
            "Quick start"
        );
        assert_eq!(
            rust_i18n::t!("app.query.summary_tab").as_ref(),
            "Execution summary"
        );
        assert_eq!(
            rust_i18n::t!("app.data.toolbar.filter_sort").as_ref(),
            "Filter & sort"
        );
        assert_eq!(
            rust_i18n::t!("app.data_export.title").as_ref(),
            "Export data"
        );
        assert_eq!(
            rust_i18n::t!("app.data_export.tab.conditions").as_ref(),
            "Conditions"
        );
        assert_eq!(
            rust_i18n::t!("app.create_table.tab.fields").as_ref(),
            "Fields"
        );
        assert_eq!(
            rust_i18n::t!("app.create_table.save_and_create").as_ref(),
            "Save and create table"
        );
        assert_eq!(
            rust_i18n::t!("app.sql_file.title").as_ref(),
            "Execute SQL file"
        );
        assert_eq!(
            rust_i18n::t!("app.sql_file.continue_on_error").as_ref(),
            "Continue on error"
        );
        assert_eq!(
            rust_i18n::t!("app.user_admin.title").as_ref(),
            "Users & permissions"
        );
        assert_eq!(
            rust_i18n::t!("app.user_admin.tab.privileges").as_ref(),
            "Privileges"
        );
        assert_eq!(
            rust_i18n::t!("app.user_admin.advanced.max_queries").as_ref(),
            "Max queries per hour:"
        );
        assert_eq!(
            rust_i18n::t!("app.backup.new").as_ref(),
            "New backup"
        );
        assert_eq!(
            rust_i18n::t!("app.database_backup.start").as_ref(),
            "Start backup"
        );
        assert_eq!(
            component_translation.as_deref(),
            Some("日付を選択"),
            "the app pack must extend GPUI Component's own translation backend"
        );
        assert_eq!(cx.window_title().as_deref(), Some("FluxDB — データベースクライアント"));

        let save_bound = cx
            .debug_bounds("settings-appearance-save")
            .expect("appearance save button");
        cx.simulate_click(save_bound.center(), Modifiers::default());
        let saved = storage.load_settings().expect("reload saved settings");
        assert_eq!(saved.locale, "ja");

        let reloaded = init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            &saved.locale,
        );
        assert_eq!(reloaded.active_locale, "ja");
    }

    #[gpui::test]
    fn update_settings_controls_render_and_toggle_in_real_view(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        let settings = Settings {
            locale: "en".to_string(),
            show_sidebar: false,
            ..Settings::default()
        };
        storage.save_settings(&settings).expect("save settings");
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            &settings.locale,
        );
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });

        let mut controller = AppController::new();
        let _ = controller.dispatch(fluxdb_app::AppCommand::SaveSettings(settings.clone()));
        assert!(matches!(
            controller.dispatch(fluxdb_app::AppCommand::OpenSettings),
            fluxdb_app::AppEvent::TabOpened(_)
        ));
        let storage_for_view = storage.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                navicat_main_new(
                    window,
                    cx,
                    controller,
                    storage_for_view,
                    gpui_component::ThemeMode::Light,
                )
            });
            Root::new(view, window, cx)
        });
        let view = cx.update(|window, cx| {
            window
                .root::<Root>()
                .expect("base window root")
                .expect("typed window root")
                .read(cx)
                .view()
                .clone()
                .downcast::<NavicatMain>()
                .expect("production NavicatMain root view")
        });
        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                this.settings_panel_section = SettingsPanelSection::About;
            });
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let switch = cx
            .debug_bounds("settings-auto-check-updates-switch")
            .expect("auto update switch");
        assert!(
            cx.debug_bounds("settings-check-updates").is_some(),
            "manual update button must render"
        );
        cx.simulate_click(switch.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            cx.update(|_, cx| view.read(cx).settings_editor_draft.auto_check_updates),
            "switch must update the settings draft"
        );
        let save = cx
            .debug_bounds("settings-About-save")
            .expect("about save button");
        cx.simulate_click(save.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.update(|_, cx| {
            view.read(cx).controller.state().settings.auto_check_updates
        }));
        assert!(
            storage
                .load_settings()
                .expect("reload settings")
                .auto_check_updates
        );
        assert_eq!(
            rust_i18n::t!("app.settings.update.title").as_ref(),
            "Software Update"
        );

        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                this.update_ui.status = crate::UpdateUiStatus::Available;
                this.update_ui.open = true;
                this.update_ui.locale = "en".to_string();
                this.update_ui.update = Some(fluxdb_app::AvailableUpdate {
                    current_version: "0.1.9".to_string(),
                    version: "0.2.0".to_string(),
                    published_at: None,
                    releases: vec![fluxdb_app::UpdateRelease {
                        version: "0.2.0".to_string(),
                        published_at: None,
                        notes: std::collections::BTreeMap::from([(
                            "en".to_string(),
                            "## Highlights\n\n- In-app updates".to_string(),
                        )]),
                    }],
                    asset: fluxdb_app::UpdateAsset {
                        url: "https://example.com/FluxDB-update.app.tar.gz".to_string(),
                        sha256: "a".repeat(64),
                        size: 128,
                        kind: fluxdb_app::UpdateAssetKind::MacosAppTarGz,
                    },
                    platform: "macos-aarch64".to_string(),
                });
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("update-dialog").is_some());
        let notes = cx
            .debug_bounds("update-dialog-notes")
            .expect("release notes must be visible");
        assert!(
            notes.size.height > px(40.),
            "release notes must not collapse: {notes:?}"
        );
        assert!(cx.debug_bounds("update-dialog-download").is_some());

        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                this.update_ui.status = crate::UpdateUiStatus::Downloading;
                this.update_ui.downloaded_bytes = 64;
                this.update_ui.total_bytes = 128;
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            cx.debug_bounds("update-download-progress").is_some(),
            "download progress bar must be visible"
        );
        assert!(
            cx.debug_bounds("update-download-status").is_some(),
            "download percentage and byte count must be visible"
        );
        assert_eq!(
            rust_i18n::t!("app.settings.update.downloading_percent")
                .replace("%{percent}", "50"),
            "Downloading 50%"
        );
        assert!(cx.debug_bounds("update-dialog-cancel").is_some());

        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                this.update_ui.status = crate::UpdateUiStatus::Available;
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let later = cx.debug_bounds("update-dialog-later").expect("later button");
        cx.simulate_click(later.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            cx.debug_bounds("update-dialog").is_none(),
            "later must close the update dialog"
        );

        // 下载中点击「取消」必须立刻关闭弹框并回到可下载状态，不能等后台线程响应。
        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                this.update_ui.status = crate::UpdateUiStatus::Downloading;
                this.update_ui.open = true;
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let cancel = cx
            .debug_bounds("update-dialog-cancel")
            .expect("cancel button");
        cx.simulate_click(cancel.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            cx.debug_bounds("update-dialog").is_none(),
            "cancel must close the update dialog"
        );
        assert_eq!(
            cx.update(|_, cx| view.read(cx).update_ui.status),
            crate::UpdateUiStatus::Available
        );
    }

    #[gpui::test]
    fn appearance_page_renders_and_persists_font_settings(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        let settings = Settings {
            locale: "en".to_string(),
            ..Settings::default()
        };
        storage.save_settings(&settings).expect("save settings");
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            &settings.locale,
        );
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });
        let mut controller = AppController::new();
        let _ = controller.dispatch(fluxdb_app::AppCommand::SaveSettings(settings.clone()));
        assert!(matches!(
            controller.dispatch(fluxdb_app::AppCommand::OpenSettings),
            fluxdb_app::AppEvent::TabOpened(_)
        ));
        let storage_for_view = storage.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                navicat_main_new(
                    window,
                    cx,
                    controller,
                    storage_for_view,
                    gpui_component::ThemeMode::Light,
                )
            });
            Root::new(view, window, cx)
        });
        let view = cx.update(|window, cx| {
            window
                .root::<Root>()
                .expect("base window root")
                .expect("typed window root")
                .read(cx)
                .view()
                .clone()
                .downcast::<NavicatMain>()
                .expect("production NavicatMain root view")
        });
        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                this.settings_panel_section = SettingsPanelSection::Appearance;
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let save = cx
            .debug_bounds("settings-appearance-save")
            .expect("appearance save button must render");

        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                this.settings_editor_draft.global_font_family = "Menlo".to_string();
                this.settings_editor_draft.ui_font_size = 17;
                this.settings_editor_draft.editor_font_family = "Menlo".to_string();
                this.settings_editor_draft.editor_font_size = 18;
                this.settings_editor_draft.editor_line_height = 22;
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(save.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let saved = storage.load_settings().expect("reload saved settings");
        assert_eq!(saved.global_font_family, "Menlo");
        assert_eq!(saved.ui_font_size, 17);
        assert_eq!(saved.editor_font_family, "Menlo");
        assert_eq!(saved.editor_font_size, 18);
        assert_eq!(saved.editor_line_height, 22);
        cx.update(|_, cx| {
            assert_eq!(
                gpui_component::Theme::global(cx).font_family.to_string(),
                "Menlo"
            );
            assert_eq!(
                gpui_component::Theme::global(cx).mono_font_family.to_string(),
                "Menlo"
            );
            // 界面字号驱动窗口 rem 基准，主题字号必须同步到草稿值。
            assert_eq!(
                f32::from(gpui_component::Theme::global(cx).font_size),
                17.0
            );
        });
    }

    /// 新建主题：弹框渲染 → 填写名称 → 保存 → 落盘并立即应用。
    #[gpui::test]
    fn appearance_custom_theme_dialog_saves_and_applies(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        let settings = Settings {
            locale: "en".to_string(),
            ..Settings::default()
        };
        storage.save_settings(&settings).expect("save settings");
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            &settings.locale,
        );
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });
        let mut controller = AppController::new();
        let _ = controller.dispatch(fluxdb_app::AppCommand::SaveSettings(settings.clone()));
        assert!(matches!(
            controller.dispatch(fluxdb_app::AppCommand::OpenSettings),
            fluxdb_app::AppEvent::TabOpened(_)
        ));
        let storage_for_view = storage.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                navicat_main_new(
                    window,
                    cx,
                    controller,
                    storage_for_view,
                    gpui_component::ThemeMode::Light,
                )
            });
            Root::new(view, window, cx)
        });
        let view = cx.update(|window, cx| {
            window
                .root::<Root>()
                .expect("base window root")
                .expect("typed window root")
                .read(cx)
                .view()
                .clone()
                .downcast::<NavicatMain>()
                .expect("production NavicatMain root view")
        });
        cx.update(|_, cx| {
            view.update(cx, |this, _| {
                this.settings_panel_section = SettingsPanelSection::Appearance;
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let new_theme = cx
            .debug_bounds("settings-theme-new")
            .expect("new theme button must render");
        cx.simulate_click(new_theme.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let name_input = cx
            .debug_bounds("theme-editor-name")
            .expect("theme editor dialog must render");
        // 弹框正文必须真正占高：父容器高度为 auto 时 flex_1 会把表单压成 0 高。
        assert!(
            f32::from(name_input.size.height) >= 20.0,
            "theme editor body must not collapse"
        );

        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                let input = this
                    .theme_editor
                    .as_ref()
                    .expect("theme editor open")
                    .name_input
                    .clone();
                input.update(cx, |input, cx| {
                    input.set_value("Test Theme".to_string(), window, cx)
                });
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let save = cx
            .debug_bounds("theme-editor-save")
            .expect("theme editor save button must render");
        cx.simulate_click(save.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let theme_file = storage.root_dir().join("themes").join("test-theme.json");
        assert!(theme_file.exists(), "theme JSON must be written");
        cx.update(|_, cx| {
            assert!(
                gpui_component::ThemeRegistry::global(cx)
                    .themes()
                    .contains_key("Test Theme"),
                "saved theme must be registered"
            );
            assert!(
                view.read(cx).theme_editor.is_none(),
                "dialog closes after save"
            );
            assert_eq!(
                view.read(cx).settings_editor_draft.light_theme, "Test Theme",
                "new theme is selected for the current mode"
            );
        });
        // 编辑：改名为 Renamed Theme，旧文件必须删除、新名称同步进入注册表。
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let edit = cx
            .debug_bounds("settings-theme-card-edit")
            .expect("hover edit affordance must render on custom theme card");
        cx.simulate_click(edit.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                let dialog = this.theme_editor.as_ref().expect("editor reopened");
                assert_eq!(
                    dialog.name_input.read(cx).value().as_str(),
                    "Test Theme",
                    "editing prefills the existing theme name"
                );
                let input = dialog.name_input.clone();
                input.update(cx, |input, cx| {
                    input.set_value("Renamed Theme".to_string(), window, cx)
                });
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let save = cx
            .debug_bounds("theme-editor-save")
            .expect("save button must render when editing");
        cx.simulate_click(save.center(), Modifiers::default());
        let renamed_file = storage.root_dir().join("themes").join("renamed-theme.json");
        assert!(renamed_file.exists(), "renamed theme JSON must be written");
        assert!(!theme_file.exists(), "stale theme JSON must be removed on rename");
        cx.update(|_, cx| {
            let registry = gpui_component::ThemeRegistry::global(cx);
            assert!(
                registry.themes().contains_key("Renamed Theme"),
                "renamed theme must be registered"
            );
            // 注册表里的旧名称由文件监听异步清理，磁盘名单必须立刻更新。
            assert!(
                !view.read(cx).custom_theme_names.contains("Test Theme"),
                "old theme name must leave the on-disk list"
            );
            assert_eq!(
                view.read(cx).settings_editor_draft.light_theme, "Renamed Theme",
                "edited theme stays selected"
            );
        });
        // 删除：文件移入 .trash，选中回退到该模式默认主题。
        let default_light = cx.update(|_, cx| {
            gpui_component::ThemeRegistry::global(cx)
                .default_light_theme()
                .name
                .to_string()
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let delete = cx
            .debug_bounds("settings-theme-card-delete")
            .expect("hover delete affordance must render on custom theme card");
        cx.simulate_click(delete.center(), Modifiers::default());
        assert!(!renamed_file.exists(), "deleted theme JSON must leave the root dir");
        let trash_entries = std::fs::read_dir(storage.root_dir().join("themes").join(".trash"))
            .expect("trash dir must exist")
            .flatten()
            .count();
        assert_eq!(trash_entries, 1, "deleted theme must be recoverable from .trash");
        cx.update(|_, cx| {
            assert!(
                !view.read(cx).custom_theme_names.contains("Renamed Theme"),
                "deleted theme must leave the on-disk list"
            );
            assert_eq!(
                view.read(cx).settings_editor_draft.light_theme, default_light,
                "selection falls back to the default light theme"
            );
        });
        // 导入 Zed 主题：文件写入、注册表注入并选中。
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                let count = this
                    .import_theme_content(
                        r##"{
                            "name": "Zed One",
                            "themes": [{
                                "name": "Zed Test",
                                "appearance": "light",
                                "style": {
                                    "background": "#fafafa",
                                    "text": "#383a42",
                                    "editor.background": "#fafafa",
                                    "syntax": { "keyword": { "color": "#a626a4" } }
                                }
                            }]
                        }"##,
                        cx,
                    )
                    .expect("import zed theme");
                assert_eq!(count, 1, "one theme imported");
            });
        });
        assert!(
            storage
                .root_dir()
                .join("themes")
                .join("zed-test.json")
                .exists(),
            "imported theme JSON must be written"
        );
        cx.update(|_, cx| {
            assert!(
                gpui_component::ThemeRegistry::global(cx)
                    .themes()
                    .contains_key("Zed Test"),
                "imported theme must be registered"
            );
            assert_eq!(
                view.read(cx).settings_editor_draft.light_theme, "Zed Test",
                "imported theme is selected"
            );
        });
        // 导入 VS Code 主题：同一入口按结构识别，真实主题文件（One Dark Pro）落盘并选中。
        cx.update(|_, cx| {
            view.update(cx, |this, cx| {
                let count = this
                    .import_theme_content(
                        include_str!("settings/testdata/one-dark-pro.json"),
                        cx,
                    )
                    .expect("import vscode theme");
                assert_eq!(count, 1, "one vscode theme imported");
            });
        });
        assert!(
            storage
                .root_dir()
                .join("themes")
                .join("one-dark-pro.json")
                .exists(),
            "imported vscode theme JSON must be written"
        );
        cx.update(|_, cx| {
            assert!(
                gpui_component::ThemeRegistry::global(cx)
                    .themes()
                    .contains_key("One Dark Pro"),
                "imported vscode theme must be registered"
            );
            assert_eq!(
                view.read(cx).settings_editor_draft.dark_theme, "One Dark Pro",
                "imported vscode theme is selected"
            );
            // 深色主题导入后自动切到深色模式，否则配色列表（只列当前模式）看不到它。
            assert_eq!(
                view.read(cx).theme_mode,
                gpui_component::ThemeMode::Dark,
                "import switches to the imported theme mode"
            );
        });
    }

}

