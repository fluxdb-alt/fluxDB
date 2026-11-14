    fn dameng_e2e_config() -> fluxdb_core::ConnectionConfig {
        use std::collections::BTreeMap;

        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DamengAdvancedOptions, DamengBasicOptions,
            DamengConnectionProfile, DatabaseKind, Endpoint, SecretRef,
        };

        let raw = std::env::var("FLUXDB_DM_SMOKE")
            .expect("FLUXDB_DM_SMOKE must be host:port:user:password");
        let parts = raw.splitn(4, ':').collect::<Vec<_>>();
        assert_eq!(parts.len(), 4, "FLUXDB_DM_SMOKE must be host:port:user:password");
        let host = parts[0];
        let port = parts[1].parse::<u16>().expect("DM port");
        let user = parts[2];
        let schema = user.to_ascii_uppercase();
        ConnectionConfig {
            id: ConnectionId(77),
            name: "DM8 completion E2E".into(),
            kind: DatabaseKind::Dameng,
            endpoint: Endpoint::Tcp {
                host: host.into(),
                port,
                database: Some("DAMENG".into()),
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: Some(DamengConnectionProfile {
                basic: DamengBasicOptions {
                    host: host.into(),
                    port,
                    database: "DAMENG".into(),
                    schema: schema.clone(),
                    username: user.into(),
                    password: SecretRef::inline(parts[3]),
                },
                advanced: DamengAdvancedOptions {
                    connect_timeout_secs: 10,
                    query_timeout_secs: 30,
                    encoding: String::new(),
                    application_name: "FluxDB E2E".into(),
                },
                ssh: Default::default(),
            }),
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    #[test]
    #[ignore = "需要 FLUXDB_DM_SMOKE 与已创建的 FLUXSALES schema"]
    fn dameng_schema_completion_lists_fluxsales_tables_only_when_qualified() {
        use fluxdb_core::{ConnectionId, ObjectKind, QueryCompletionKind};

        let mut controller = AppController::new();
        let config = dameng_e2e_config();
        let _ = controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(vec![config]));
        let _ = controller.dispatch(fluxdb_app::AppCommand::OpenConnection(ConnectionId(77)));
        let database = controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == ConnectionId(77))
            .and_then(|connection| {
                connection
                    .objects
                    .iter()
                    .find(|object| object.path.kind == ObjectKind::Database)
            })
            .map(|object| object.path.clone())
            .expect("DM database");
        let _ = controller.dispatch(fluxdb_app::AppCommand::LoadObjectChildren(
            database.clone(),
        ));
        let schema_path = controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == ConnectionId(77))
            .and_then(|connection| {
                connection.objects.iter().find(|object| {
                    object.path.kind == ObjectKind::Schema
                        && object.path.name.eq_ignore_ascii_case("FLUXSALES")
                })
            })
            .map(|object| object.path.clone())
            .expect("FLUXSALES schema");
        let _ = controller.dispatch(fluxdb_app::AppCommand::LoadObjectChildren(schema_path));
        // 模拟用户同时展开默认 schema；裸名补全的优先来源是它。
        let default_schema = controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == ConnectionId(77))
            .and_then(|connection| {
                connection.objects.iter().find(|object| {
                    object.path.kind == ObjectKind::Schema
                        && object.path.name.eq_ignore_ascii_case("FLUXDEMO")
                })
            })
            .map(|object| object.path.clone())
            .expect("FLUXDEMO schema");
        let _ = controller.dispatch(fluxdb_app::AppCommand::LoadObjectChildren(default_schema));

        let schema_items = controller
            .query_completions_for_text(
                ConnectionId(77),
                Some("DAMENG".into()),
                None,
                "SELECT * FROM ".into(),
                "SELECT * FROM ".len(),
                true,
            )
            .expect("schema completions")
            .items;
        assert!(
            schema_items.iter().any(|item| {
                item.kind == QueryCompletionKind::Schema
                    && item.label.eq_ignore_ascii_case("FLUXSALES")
            }),
            "{:?}",
            schema_items.iter().map(|item| &item.label).collect::<Vec<_>>()
        );

        let qualified_items = controller
            .query_completions_for_text(
                ConnectionId(77),
                Some("DAMENG".into()),
                None,
                "SELECT * FROM FLUXSALES.".into(),
                "SELECT * FROM FLUXSALES.".len(),
                true,
            )
            .expect("qualified table completions")
            .items;
        let labels = qualified_items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        for table in ["SALES_ORDER", "SALES_ORDER_ITEM", "SALES_REGION"] {
            assert!(labels.contains(&table), "{labels:?}");
        }
        assert!(
            qualified_items.iter().any(|item| {
                item.label == "V_SALES_ORDER_SUMMARY"
                    && item.kind == QueryCompletionKind::View
            }),
            "{labels:?}"
        );
        assert!(
            labels.iter().all(|label| !label.starts_with("SREF_CON_")),
            "{labels:?}"
        );

        let bare_items = controller
            .query_completions_for_text(
                ConnectionId(77),
                Some("DAMENG".into()),
                None,
                "SELECT * FROM CUST".into(),
                "SELECT * FROM CUST".len(),
                true,
            )
            .expect("bare table completions")
            .items;
        assert!(
            bare_items
                .iter()
                .any(|item| item.label == "CUSTOMERS"),
            "default schema completion must stay usable: {:?}",
            bare_items
                .iter()
                .map(|item| &item.label)
                .collect::<Vec<_>>()
        );
        assert!(
            bare_items
                .iter()
                .all(|item| !matches!(
                    item.label.as_str(),
                    "SALES_ORDER"
                        | "SALES_ORDER_ITEM"
                        | "SALES_REGION"
                        | "V_SALES_ORDER_SUMMARY"
                )),
            "FLUXSALES tables must not leak into the default schema: {:?}",
            bare_items
                .iter()
                .map(|item| &item.label)
                .collect::<Vec<_>>()
        );
    }

    /// 达梦补全的无头 E2E：生产 NavicatMain + SQL 编辑器，真实键入、接受候选并执行。
    ///
    /// 依赖物化好的 DM8 演示库（`FLUXDEMO.CUSTOMERS`），仅在显式提供
    /// `FLUXDB_DM_SMOKE=host:port:user:password` 时运行。
    #[gpui::test]
    #[ignore = "需要 FLUXDB_DM_SMOKE 与含 FLUXDEMO.CUSTOMERS 的 DM8"]
    fn dameng_completion_inserts_qualified_table_and_executes(cx: &mut TestAppContext) {
        use std::time::{Duration, Instant};

        use crate::TabKind;
        use fluxdb_core::ConnectionId;

        let config = dameng_e2e_config();

        let _guard = locale_guard();
        // 真实 DM8 客户端会在线程中等待网络 I/O；允许 parking，避免测试调度器
        // 把跨线程唤醒误判为非确定性。该用例本身标记 ignored，不会进入普通单测。
        cx.background_executor.allow_parking();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "zh-CN");
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });

        let mut controller = AppController::new();
        let _ = controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(vec![config]));
        let _ = controller.dispatch(fluxdb_app::AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(77),
            database: Some("DAMENG".into()),
            schema: None,
        });
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
        cx.simulate_resize(gpui::size(gpui::px(1440.), gpui::px(900.)));
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
        cx.run_until_parked();

        let tab_id = cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .active_tab
                .expect("query tab")
        });
        let editor = cx
            .update(|_, cx| view.read(cx).query_editors.get(&fluxdb_app::QueryPaneKey::new(tab_id, fluxdb_app::QueryPaneId(1))).cloned())
            .expect("SQL editor entity");
        let bounds = cx.debug_bounds("query-editor").expect("SQL editor");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        cx.simulate_input("SELECT * FROM CUST");
        cx.update(|_, cx| {
            assert_eq!(editor.read(cx).text(), "SELECT * FROM CUST");
        });
        // 自动补全有 75ms debounce，且表元数据来自真实 DM8 查询。
        cx.background_executor.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let ready = cx.update(|_, cx| {
                let editor = editor.read(cx);
                editor.completion_visible() && !editor.completion_items.is_empty()
            });
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "DM8 completion did not return within 15s"
            );
            std::thread::sleep(Duration::from_millis(50));
            cx.background_executor.advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
        }

        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let labels = editor
                .completion_items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>();
            assert!(labels.contains(&"CUSTOMERS"), "{labels:?}");
            assert!(
                labels.iter().all(|label| !label.starts_with("SREF_CON_")),
                "internal DM tables must not be suggested: {labels:?}"
            );
            let customers = editor
                .completion_items
                .iter()
                .find(|item| item.label == "CUSTOMERS")
                .expect("CUSTOMERS candidate");
            assert_eq!(customers.insert_text, "\"FLUXDEMO\".\"CUSTOMERS\"");
        });

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let row = cx
            .debug_bounds("completion-item:CUSTOMERS")
            .expect("CUSTOMERS completion row");
        cx.simulate_click(row.center(), Modifiers::default());
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).text(),
                "SELECT * FROM \"FLUXDEMO\".\"CUSTOMERS\""
            );
        });

        let run = cx.debug_bounds("query-run").expect("query run button");
        cx.simulate_click(run.center(), Modifiers::default());
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let finished = cx.update(|_, cx| {
                let view = view.read(cx);
                view.controller.state().tabs.iter().any(|tab| {
                    tab.id == tab_id
                        && matches!(&tab.kind, TabKind::QueryEditor(editor) if !editor.running)
                }) && view
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .is_some_and(|tab| match &tab.kind {
                        TabKind::QueryEditor(editor) => !editor.summaries.is_empty(),
                        _ => false,
                    })
            });
            if finished {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "qualified DM query did not finish within 30s"
            );
            std::thread::sleep(Duration::from_millis(50));
            cx.run_until_parked();
        }
        cx.update(|_, cx| {
            let TabKind::QueryEditor(editor) = &view
                .read(cx)
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .expect("query tab")
                .kind
            else {
                panic!("query editor")
            };
            assert!(editor.error.is_none(), "{:?}", editor.error);
            assert!(
                editor.summaries.iter().all(|summary| summary.success),
                "{:?}",
                editor.summaries
            );
        });
    }

    /// PostgreSQL schema 补全：点击 schema 插入带引号的 `"schema".` 后，
    /// 生产编辑器应接续弹出该 schema 的表候选。
    #[gpui::test]
    fn postgres_schema_acceptance_chains_to_table_completion(cx: &mut TestAppContext) {
        use std::time::{Duration, Instant};

        use fluxdb_core::{
            ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath, ObjectSummary,
            PostgresConnectionProfile,
        };

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "zh-CN");
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
        });

        let connection_id = ConnectionId(78);
        let database = "fluxdb_pr46";
        let schema = "fluxdb_pr46_acceptance";
        let mut controller = AppController::new();
        let _ = controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(vec![
            fluxdb_core::ConnectionConfig {
                id: connection_id,
                name: "PG quoted schema completion".into(),
                kind: DatabaseKind::Postgres,
                endpoint: Endpoint::Tcp {
                    host: "127.0.0.1".into(),
                    port: 5432,
                    database: Some(database.into()),
                },
                options: std::collections::BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: Some(PostgresConnectionProfile::default()),
                dameng_profile: None,
                mongo_profile: None,
                sqlserver_profile: None,
                oracle_profile: None,
            },
        ]));
        let _ = controller.dispatch(fluxdb_app::AppCommand::OpenQueryEditorInDatabase {
            connection_id,
            database: Some(database.into()),
            schema: Some(schema.into()),
        });
        controller.merge_loaded_children(
            &ObjectPath {
                connection_id,
                database: Some(database.into()),
                schema: Some(schema.into()),
                name: schema.into(),
                kind: ObjectKind::Schema,
            },
            vec![ObjectSummary {
                path: ObjectPath {
                    connection_id,
                    database: Some(database.into()),
                    schema: Some(schema.into()),
                    name: "pg_acceptance".into(),
                    kind: ObjectKind::Table,
                },
                rows: None,
                size_bytes: None,
                modified_at: None,
                comment: None,
                stable: None,
            }],
        );
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
        cx.simulate_resize(gpui::size(gpui::px(1440.), gpui::px(900.)));
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

        let tab_id = cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .active_tab
                .expect("query tab")
        });
        let editor = cx
            .update(|_, cx| view.read(cx).query_editors.get(&fluxdb_app::QueryPaneKey::new(tab_id, fluxdb_app::QueryPaneId(1))).cloned())
            .expect("SQL editor entity");
        let bounds = cx.debug_bounds("query-editor").expect("SQL editor");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let sql = "SELECT * FROM flu";
        cx.simulate_input(sql);
        cx.update(|_, cx| {
            assert_eq!(editor.read(cx).text(), sql);
        });

        cx.background_executor.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let ready = cx.update(|_, cx| {
                let editor = editor.read(cx);
                editor
                    .completion_items
                    .iter()
                    .any(|item| item.label == schema)
            });
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "schema completion did not open within 5s"
            );
            std::thread::sleep(Duration::from_millis(50));
            cx.background_executor.advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
        }

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let schema_row = cx
            .debug_bounds("completion-item:fluxdb_pr46_acceptance")
            .expect("schema completion row");
        cx.simulate_click(schema_row.center(), Modifiers::default());
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).text(),
                format!(r#"SELECT * FROM "{schema}"."#)
            );
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let ready = cx.update(|_, cx| {
                let editor = editor.read(cx);
                editor.completion_visible()
                    && editor
                        .completion_items
                        .iter()
                        .any(|item| item.label == "pg_acceptance")
            });
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "table completion did not chain after accepting schema"
            );
            std::thread::sleep(Duration::from_millis(50));
            cx.background_executor.advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
        }
        cx.update(|_, cx| {
            assert!(
                editor
                    .read(cx)
                    .completion_items
                    .iter()
                    .any(|item| item.label == "pg_acceptance"),
                "table completion should follow accepted schema"
            );
        });
    }
