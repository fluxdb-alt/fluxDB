#[gpui::test]
fn ai_page_query_opens_sidebar_table_fills_conditions_and_shows_rows(cx: &mut TestAppContext) {
    use fluxdb_app::{AiModelSaveInput, AppCommand, AppEvent, TabKind};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
    };
    let _guard = locale_guard();
    cx.background_executor.allow_parking();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let database = temp.path().join("page-query.sqlite");
    let connection = fluxdb_storage::AiConnection::open(&database).expect("SQLite database");
    connection
        .execute_batch(
            "CREATE TABLE records (id INTEGER PRIMARY KEY, name TEXT, status TEXT);
         INSERT INTO records VALUES (1, 'alpha', 'failed'), (2, 'beta', 'ok'),
                                    (3, 'gamma', 'failed');",
        )
        .expect("fixture rows");
    drop(connection);
    let settings = Settings {
        locale: "en".into(),
        show_sidebar: true,
        ..Default::default()
    };
    storage.save_settings(&settings).expect("settings");
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    controller.set_ai_storage(storage.clone());
    controller.dispatch(AppCommand::SaveSettings(settings));
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "Page query fixture".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: database.clone(),
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
    }]));
    let event = controller.dispatch(AppCommand::SaveAiModel(AiModelSaveInput {
        display_name: "offline page query fixture".into(),
        provider: "openai".into(),
        model: "fixture".into(),
        endpoint: Some("https://models.invalid/v1".into()),
        enabled: true,
        ..Default::default()
    }));
    let AppEvent::AiModelSaved { id, .. } = event else {
        panic!("{event:?}")
    };
    let revision = storage
        .with_ai_connection(fluxdb_storage::ai_platform::settings::load_runtime_settings)
        .expect("runtime")
        .revision;
    controller.dispatch(AppCommand::SetAiDefaultModel {
        model_id: Some(id),
        expected_revision: revision,
    });
    cx.update(|cx| {
        extend_component_i18n();
        gpui_component::init(cx);
    });
    let profile = storage.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut app = navicat_main_new(
                window,
                cx,
                controller,
                profile,
                gpui_component::ThemeMode::Light,
            );
            app.ai_chat.recovery_pending = false;
            app.ai_chat.loaded = true;
            app.ai_chat.open = true;
            app.ai_chat.scripted_fixture = true;
            app.ai_chat.scripted_table_query = Some((
                ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".into()),
                    schema: None,
                    name: "records".into(),
                    kind: ObjectKind::Table,
                },
                serde_json::from_value(serde_json::json!({
                    "page_ref":"fixture", "execute":true, "limit":100,
                    "filters":[{"field":"status","op":"eq","values":["failed"]}],
                    "sort":[{"field":"id","descending":true}]
                }))
                .expect("table query"),
            ));
            app
        });
        Root::new(view, window, cx)
    });
    cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(950.)));
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
    assert!(
        cx.debug_bounds("ai-table-query-mode").is_none(),
        "page query mode button must not be rendered"
    );
    let input_bounds = cx.debug_bounds("ai-chat-input").expect("chat input");
    cx.simulate_click(input_bounds.center(), Modifiers::default());
    cx.simulate_input("Find failed records, newest first.");
    cx.update(|window, cx| {
        assert_eq!(
            view.read(cx).ai_chat.input.read(cx).value(),
            "Find failed records, newest first."
        );
        window.draw(cx).clear(cx);
    });
    let send = cx.debug_bounds("ai-chat-send").expect("send button");
    cx.simulate_click(send.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert!(
            app.ai_chat.running.is_some() || app.ai_chat.loading || app.ai_chat._io.is_some(),
            "send did not start: mode={}, input={:?}, error={:?}",
            false,
            app.ai_chat.input.read(cx).value(),
            app.ai_chat.error,
        );
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        let finished = cx.update(|_, cx| {
            let app = view.read(cx);
            app.ai_chat.running.is_none()
                && app.ai_chat._io.is_none()
                && app.ai_chat.table_page_task.is_none()
                && !app.ai_chat.loading
        });
        if finished {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "page query did not finish"
        );
        std::thread::yield_now();
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let tab_id = cx.update(|_, cx| {
        let app = view.read(cx);
        let (tab_id, editor) = app
            .controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if editor.object.name == "records" => {
                    Some((tab.id, editor))
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "opened table page: chat_error={:?}, message={:?}, tabs={:?}, objects={:?}",
                    app.ai_chat.error,
                    app.app_message.as_ref().map(|m| m.text.clone()),
                    app.controller
                        .state()
                        .tabs
                        .iter()
                        .map(|t| t.title.clone())
                        .collect::<Vec<_>>(),
                    app.controller
                        .state()
                        .connections
                        .iter()
                        .flat_map(|c| c.objects.iter().map(|o| o.path.clone()))
                        .collect::<Vec<_>>(),
                )
            });
        assert!(app.data_filter_panels.contains(&tab_id));
        assert!(app.show_connection_browser);
        let bindings = &app
            .ai_chat
            .table_page_binding
            .as_ref()
            .expect("page binding");
        assert_eq!(bindings.filters[0].field, "status");
        assert_eq!(bindings.sort[0].field, "id");
        let page = editor.page.as_ref().expect("query result");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            page.rows[0].values[0],
            fluxdb_core::CellValue::I64(3)
        );
        let session = app
            .controller
            .state()
            .ai_conversation
            .session_id
            .as_deref()
            .expect("session");
        storage
            .with_ai_connection(|conn| {
                let turns = fluxdb_storage::ai_platform::sessions::list_turns(conn, session)?;
                let snapshot: serde_json::Value =
                    serde_json::from_str(&turns.last().expect("turn").runtime_snapshot_json)
                        .expect("snapshot");
                assert!(
                    snapshot["ui_context"]
                        .as_str()
                        .expect("context")
                        .contains("current_page")
                );
                assert!(
                    !snapshot["ui_context"]
                        .as_str()
                        .expect("context")
                        .contains("alpha")
                );
                Ok(())
            })
            .expect("bounded persisted context");
        let message = app.app_message.as_ref().expect("visible completion");
        assert_eq!(
            message.text,
            crate::tr("app.ai.page_query.queried").to_string()
        );
        tab_id
    });

    assert!(cx.debug_bounds("data-filter-mode-builder").is_some());
    assert!(cx.debug_bounds("data-filter-mode-input").is_some());
    assert!(cx.debug_bounds("data-filter-condition-editor").is_none());
    cx.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.ai_chat.open = false;
            cx.notify();
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let input_mode = cx
        .debug_bounds("data-filter-mode-input")
        .expect("input mode button");
    cx.simulate_click(input_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.data_filter_modes.get(&tab_id),
            Some(&crate::DataFilterMode::Text)
        );
        assert_eq!(
            app.data_filter_text_input.read(cx).value().to_string(),
            "\"status\" = 'failed'"
        );
    });
    assert!(cx.debug_bounds("data-filter-condition-editor").is_some());
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.data_sort_draft_rules.get(&tab_id).map(Vec::len),
            Some(1),
            "sort form must remain independent from filter input mode"
        );
    });

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_texts
                .insert(tab_id, "\"status\" = 'ok'".to_string());
            this.data_filter_text_input.update(cx, |editor, cx| {
                editor.set_value("\"status\" = 'ok'", window, cx);
            });
        });
    });
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.set_data_filter_mode(
                tab_id,
                crate::DataFilterMode::Builder,
                crate::sql_editor_adapter::SqlDialect::Sqlite,
                window,
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|_, cx| {
        let app = view.read(cx);
        let rules = app
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("input must flow back into the form");
        assert_eq!(
            rules.len(),
            1,
            "structured input must replace the form contents"
        );
        assert_eq!(rules[0].field.as_deref(), Some("status"));
        assert_eq!(rules[0].operator, crate::DataFilterOperator::Eq);
        assert_eq!(rules[0].values, vec!["ok".to_string()]);
        assert_eq!(
            app.data_filter_modes.get(&tab_id),
            Some(&crate::DataFilterMode::Builder)
        );
    });
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.set_data_filter_mode(
                tab_id,
                crate::DataFilterMode::Text,
                crate::sql_editor_adapter::SqlDialect::Sqlite,
                window,
                cx,
            );
        });
    });
    assert_eq!(
        cx.update(|_, cx| view.read(cx).data_filter_text_input.read(cx).value().to_string()),
        "\"status\" = 'ok'"
    );

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_texts.insert(tab_id, "\"id\" > 1".to_string());
            this.data_filter_text_input
                .update(cx, |editor, cx| editor.set_value("\"id\" > 1", window, cx));
            this.set_data_filter_mode(
                tab_id,
                crate::DataFilterMode::Builder,
                crate::sql_editor_adapter::SqlDialect::Sqlite,
                window,
                cx,
            );
        });
    });
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("typed form");
        assert_eq!(
            rules[0].typed_values,
            Some(vec![fluxdb_core::FilterValue::Integer(1)])
        );
    });
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.set_data_filter_mode(
                tab_id,
                crate::DataFilterMode::Text,
                crate::sql_editor_adapter::SqlDialect::Sqlite,
                window,
                cx,
            );
        });
    });
    assert_eq!(
        cx.update(|_, cx| view.read(cx).data_filter_text_input.read(cx).value().to_string()),
        "\"id\" > 1",
        "numeric input must keep its type when round-tripping through the form"
    );

    cx.update(|_, cx| {
        view.update(cx, |this, _| {
            this.data_filter_texts
                .insert(tab_id, "lower(\"status\") = 'ok'".to_string());
        });
    });
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.set_data_filter_mode(
                tab_id,
                crate::DataFilterMode::Builder,
                crate::sql_editor_adapter::SqlDialect::Sqlite,
                window,
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("advanced rule");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].advanced_expression.as_deref(),
            Some("lower(\"status\") = 'ok'")
        );
    });
    assert!(cx.debug_bounds("data-filter-advanced-rule-0").is_some());

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_texts.remove(&tab_id);
            this.set_data_filter_mode(
                tab_id,
                crate::DataFilterMode::Text,
                crate::sql_editor_adapter::SqlDialect::Sqlite,
                window,
                cx,
            );
            this.data_filter_texts.remove(&tab_id);
            this.set_data_filter_mode(
                tab_id,
                crate::DataFilterMode::Builder,
                crate::sql_editor_adapter::SqlDialect::Sqlite,
                window,
                cx,
            );
        });
    });
    assert!(
        cx.update(|_, cx| !view.read(cx).data_filter_draft_rules.contains_key(&tab_id)),
        "empty input must clear the previous form"
    );

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_panels.insert(tab_id);
            this.data_filter_modes
                .insert(tab_id, crate::DataFilterMode::Builder);
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("status".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["failed".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 0,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("status".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["ok".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 0,
                        group_end: 0,
                    },
                ],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let connector = cx
        .debug_bounds("data-filter-connector-0")
        .expect("row connector");
    cx.simulate_click(connector.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        let rules = app
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("connector rules");
        assert_eq!(
            rules[0].connector,
            fluxdb_core::FilterCombinator::Or
        );
        assert!(app
            .data_filter_texts
            .get(&tab_id)
            .is_some_and(|text| text.contains(" OR ")));
    });
    let connector = cx
        .debug_bounds("data-filter-connector-0")
        .expect("switched connector");
    cx.simulate_click(connector.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("connector rules");
        assert_eq!(
            rules[0].connector,
            fluxdb_core::FilterCombinator::And
        );
    });
}

fn wait_for_filtered_ids(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<NavicatMain>,
    tab_id: fluxdb_app::TabId,
    expected: &[&str],
) {
    use fluxdb_app::TabKind;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let mut actual = cx.update(|_, cx| {
            let app = view.read(cx);
            let TabKind::DataEditor(editor) = &app
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .expect("data tab")
                .kind
            else {
                panic!("data editor");
            };
            editor
                .page
                .as_ref()
                .expect("filtered page")
                .rows
                .iter()
                .filter_map(|row| row.values.first().map(fluxdb_core::CellValue::display_label))
                .collect::<Vec<_>>()
        });
        actual.sort();
        let mut expected = expected.iter().map(|value| (*value).to_string()).collect::<Vec<_>>();
        expected.sort();
        if actual == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "filtered rows expected {expected:?}, got {actual:?}"
        );
        std::thread::yield_now();
    }
}

#[gpui::test]
fn typed_and_like_form_filters_change_query_results(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent, TabKind};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let database = temp.path().join("numeric-filter.sqlite");
    let connection = fluxdb_storage::AiConnection::open(&database).expect("SQLite database");
    connection
        .execute_batch(
            "CREATE TABLE records (id INTEGER PRIMARY KEY, amount NUMERIC);
             INSERT INTO records VALUES (1, 4), (2, 2), (3, 3), (4, 4), (5, 4);",
        )
        .expect("fixture rows");
    drop(connection);
    let settings = Settings {
        locale: "en".into(),
        ..Default::default()
    };
    storage.save_settings(&settings).expect("settings");
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "numeric filter fixture".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: database,
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
    }]));
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
    let tab_id = cx.update(|_, cx| {
        let event = view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::OpenDataEditor(ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("main".into()),
                schema: None,
                name: "records".into(),
                kind: ObjectKind::Table,
                }),
                cx,
            )
        });
        let AppEvent::TabOpened(tab_id) = event else {
            panic!("open data editor: {event:?}");
        };
        tab_id
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        let loaded = cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => editor.page.as_ref(),
                    _ => None,
                })
                .map(|page| page.rows.len())
        });
        if loaded == Some(5) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "initial page did not load");
        std::thread::yield_now();
    }
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_panels.insert(tab_id);
            this.data_filter_modes
                .insert(tab_id, crate::DataFilterMode::Builder);
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![crate::DataFilterRule {
                    enabled: true,
                    field: Some("id".into()),
                    operator: crate::DataFilterOperator::Contains,
                    values: vec!["2".into()],
                    advanced_expression: None,
                    typed_values: None,
                    connector: fluxdb_core::FilterCombinator::And,
                    group_depth: 0,
                        group_end: 0,
                }],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let panel = cx
        .debug_bounds("data-filter-panel")
        .expect("filter panel");
    let apply = cx
        .debug_bounds("data-filter-apply")
        .expect("builder apply button");
    assert!(
        apply.top() >= panel.top() && apply.bottom() <= panel.bottom(),
        "builder controls must stay inside the filter panel"
    );
    let input_mode = cx
        .debug_bounds("data-filter-mode-input")
        .expect("input mode button");
    cx.simulate_click(input_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.data_filter_modes.get(&tab_id),
            Some(&crate::DataFilterMode::Text)
        );
        assert_eq!(
            app.data_filter_text_input.read(cx).value().to_string(),
            "\"id\" LIKE '%2%'"
        );
    });
    let panel = cx
        .debug_bounds("data-filter-panel")
        .expect("input filter panel");
    let apply = cx
        .debug_bounds("data-filter-apply")
        .expect("input apply button");
    assert!(
        apply.top() >= panel.top() && apply.bottom() <= panel.bottom(),
        "input controls and apply button must stay inside the filter panel"
    );
    let builder_mode = cx
        .debug_bounds("data-filter-mode-builder")
        .expect("builder mode button");
    cx.simulate_click(builder_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.data_filter_modes.get(&tab_id),
            Some(&crate::DataFilterMode::Builder)
        );
        let rules = app
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("LIKE round-trip rules");
        assert_eq!(rules[0].field.as_deref(), Some("id"));
        assert_eq!(rules[0].operator, crate::DataFilterOperator::Contains);
        assert_eq!(rules[0].values, vec!["2".to_string()]);
        assert_eq!(
            rules[0].typed_values,
            Some(vec![fluxdb_core::FilterValue::Text("2".into())])
        );
    });
    let apply = cx
        .debug_bounds("data-filter-apply")
        .expect("apply filter button");
    cx.simulate_click(apply.center(), Modifiers::default());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let filtered = cx.update(|_, cx| {
            let app = view.read(cx);
            let TabKind::DataEditor(editor) = &app
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .expect("data tab")
                .kind
            else {
                panic!("data editor");
            };
            editor.page.as_ref().is_some_and(|page| {
                page.rows.len() == 1
                    && page.rows[0].values.first()
                        == Some(&fluxdb_core::CellValue::I64(2))
            })
        });
        if filtered {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "filtered page did not load"
        );
        std::thread::yield_now();
    }
    cx.update(|_, cx| {
        let app = view.read(cx);
        let TabKind::DataEditor(editor) = &app
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .expect("data tab")
            .kind
        else {
            panic!("data editor");
        };
        let page = editor.page.as_ref().expect("filtered page");
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].values[0], fluxdb_core::CellValue::I64(2));
    });

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![crate::DataFilterRule {
                    enabled: true,
                    field: Some("amount".into()),
                    operator: crate::DataFilterOperator::Eq,
                    values: vec!["3".into()],
                    advanced_expression: None,
                    typed_values: None,
                    connector: fluxdb_core::FilterCombinator::And,
                    group_depth: 0,
                        group_end: 0,
                }],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let apply = cx
        .debug_bounds("data-filter-apply")
        .expect("apply numeric filter button");
    cx.simulate_click(apply.center(), Modifiers::default());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let filtered = cx.update(|_, cx| {
            let app = view.read(cx);
            let TabKind::DataEditor(editor) = &app
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .expect("data tab")
                .kind
            else {
                panic!("data editor");
            };
            editor.page.as_ref().is_some_and(|page| {
                page.rows.len() == 1
                    && page.rows[0].values.first()
                        == Some(&fluxdb_core::CellValue::I64(3))
            })
        });
        if filtered {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "numeric filtered page did not load"
        );
        std::thread::yield_now();
    }
    cx.update(|_, cx| {
        let app = view.read(cx);
        let TabKind::DataEditor(editor) = &app
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .expect("data tab")
            .kind
        else {
            panic!("data editor");
        };
        let page = editor.page.as_ref().expect("numeric filtered page");
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].values[0], fluxdb_core::CellValue::I64(3));
    });

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("id".into()),
                        operator: crate::DataFilterOperator::Gt,
                        values: vec!["1".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 0,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("amount".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["2".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::Or,
                        group_depth: 0,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("amount".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["4".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 0,
                        group_end: 0,
                    },
                ],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let apply = cx
        .debug_bounds("data-filter-apply")
        .expect("apply mixed filter button");
    cx.simulate_click(apply.center(), Modifiers::default());
    wait_for_filtered_ids(cx, &view, tab_id, &["1", "2", "4", "5"]);
    cx.update(|_, cx| {
        let text = view
            .read(cx)
            .data_filter_texts
            .get(&tab_id)
            .expect("mixed filter text");
        assert!(text.contains(" OR "), "{text}");
    });

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("id".into()),
                        operator: crate::DataFilterOperator::Gt,
                        values: vec!["1".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 0,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("amount".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["2".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::Or,
                        group_depth: 1,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("amount".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["4".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 1,
                        group_end: 0,
                    },
                ],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let apply = cx
        .debug_bounds("data-filter-apply")
        .expect("apply grouped filter button");
    cx.simulate_click(apply.center(), Modifiers::default());
    wait_for_filtered_ids(cx, &view, tab_id, &["2", "4", "5"]);

    let input_mode = cx
        .debug_bounds("data-filter-mode-input")
        .expect("grouped input mode button");
    cx.simulate_click(input_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let text = view.read(cx).data_filter_text_input.read(cx).value().to_string();
        assert!(text.contains(" AND ("), "{text}");
        assert!(text.contains(" OR "), "{text}");
    });
    let builder_mode = cx
        .debug_bounds("data-filter-mode-builder")
        .expect("grouped builder mode button");
    cx.simulate_click(builder_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        let rules = app
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("grouped round-trip rules");
        assert_eq!(rules.len(), 3);
        assert!(rules.iter().all(|rule| rule.advanced_expression.is_none()));
        assert_eq!(
            rules.iter().map(|rule| rule.group_depth).collect::<Vec<_>>(),
            vec![0, 1, 1]
        );
        assert_eq!(
            rules.iter().map(|rule| rule.connector).collect::<Vec<_>>(),
            vec![
                fluxdb_core::FilterCombinator::And,
                fluxdb_core::FilterCombinator::Or,
                fluxdb_core::FilterCombinator::And,
            ]
        );
    });

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("id".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["1".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::Or,
                        group_depth: 1,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("amount".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["4".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 1,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("amount".into()),
                        operator: crate::DataFilterOperator::Ge,
                        values: vec!["2".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 0,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("id".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["4".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::Or,
                        group_depth: 1,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("id".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["1".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 1,
                        group_end: 0,
                    },
                ],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let apply = cx
        .debug_bounds("data-filter-apply")
        .expect("apply two-group filter button");
    cx.simulate_click(apply.center(), Modifiers::default());
    wait_for_filtered_ids(cx, &view, tab_id, &["1", "4"]);

    let input_mode = cx
        .debug_bounds("data-filter-mode-input")
        .expect("two-group input mode button");
    cx.simulate_click(input_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let builder_mode = cx
        .debug_bounds("data-filter-mode-builder")
        .expect("two-group builder mode button");
    cx.simulate_click(builder_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("two-group round-trip rules");
        assert_eq!(rules.len(), 5);
        assert!(rules.iter().all(|rule| rule.advanced_expression.is_none()));
        assert_eq!(
            rules.iter().map(|rule| rule.group_depth).collect::<Vec<_>>(),
            vec![1, 1, 0, 1, 1]
        );
    });

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![crate::DataFilterRule {
                    enabled: true,
                    field: Some("id".into()),
                    operator: crate::DataFilterOperator::Contains,
                    values: vec!["1".into()],
                    advanced_expression: None,
                    typed_values: None,
                    connector: fluxdb_core::FilterCombinator::And,
                    group_depth: 0,
                        group_end: 0,
                }],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let add_group = cx
        .debug_bounds("data-filter-add-group-0")
        .expect("top-level add group button");
    cx.simulate_click(add_group.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("first nested group");
        assert_eq!(
            rules.iter().map(|rule| rule.group_depth).collect::<Vec<_>>(),
            vec![0, 1]
        );
    });
    let add_sibling = cx
        .debug_bounds("data-filter-add-rule-1")
        .expect("nested add sibling button");
    cx.simulate_click(add_sibling.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("nested sibling");
        assert_eq!(
            rules.iter().map(|rule| rule.group_depth).collect::<Vec<_>>(),
            vec![0, 1, 1]
        );
    });
    let add_nested_group = cx
        .debug_bounds("data-filter-add-group-2")
        .expect("nested add group button");
    cx.simulate_click(add_nested_group.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("deeply nested group");
        assert_eq!(
            rules.iter().map(|rule| rule.group_depth).collect::<Vec<_>>(),
            vec![0, 1, 1, 2]
        );
    });
}

fn data_filter_mode_round_trip_for_database(
    cx: &mut TestAppContext,
    kind: fluxdb_core::DatabaseKind,
    expected_input: &str,
) {
    use fluxdb_app::{AppCommand, AppEvent, TabKind};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, Endpoint, ObjectKind, ObjectPath,
    };

    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let settings = Settings {
        locale: "en".into(),
        ..Default::default()
    };
    storage.save_settings(&settings).expect("settings");
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    let endpoint = if kind == fluxdb_core::DatabaseKind::Sqlite {
        Endpoint::SqliteFile {
            path: temp.path().join("demo-filter.sqlite"),
            read_only: true,
        }
    } else {
        Endpoint::Tcp {
            host: "demo.invalid".into(),
            port: 1,
            database: Some("main".into()),
        }
    };
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: format!("{kind:?} filter fixture"),
        kind,
        endpoint,
        options: std::collections::BTreeMap::from([("demo".into(), "true".into())]),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }]));
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
    let tab_id = cx.update(|_, cx| {
        let event = view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::OpenDataEditor(ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".into()),
                    schema: None,
                    name: "Product".into(),
                    kind: ObjectKind::Table,
                }),
                cx,
            )
        });
        let AppEvent::TabOpened(tab_id) = event else {
            panic!("open demo data editor: {event:?}");
        };
        tab_id
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        let loaded = cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .is_some_and(|tab| {
                    matches!(
                        &tab.kind,
                        TabKind::DataEditor(editor) if editor.page.is_some()
                    )
                })
        });
        if loaded {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{kind:?} demo page did not load"
        );
        std::thread::yield_now();
    }
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let toggle = cx
        .debug_bounds("data-filter-sort-toggle")
        .expect("filter toggle");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_modes
                .insert(tab_id, crate::DataFilterMode::Builder);
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![crate::DataFilterRule {
                    enabled: true,
                    field: Some("id".into()),
                    operator: crate::DataFilterOperator::Contains,
                    values: vec!["1".into()],
                    advanced_expression: None,
                    typed_values: None,
                    connector: fluxdb_core::FilterCombinator::And,
                    group_depth: 0,
                        group_end: 0,
                }],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let input = cx
        .debug_bounds("data-filter-mode-input")
        .expect("input mode button");
    cx.simulate_click(input.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.data_filter_modes.get(&tab_id),
            Some(&crate::DataFilterMode::Text),
            "{kind:?}"
        );
        assert_eq!(
            app.data_filter_text_input.read(cx).value().to_string(),
            expected_input,
            "{kind:?}"
        );
    });
    let builder = cx
        .debug_bounds("data-filter-mode-builder")
        .expect("builder mode button");
    cx.simulate_click(builder.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.data_filter_modes.get(&tab_id),
            Some(&crate::DataFilterMode::Builder),
            "{kind:?}"
        );
        let rule = &app
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("round-trip rule")[0];
        assert_eq!(rule.field.as_deref(), Some("id"), "{kind:?}");
        assert_eq!(
            rule.operator,
            crate::DataFilterOperator::Contains,
            "{kind:?}"
        );
        assert_eq!(rule.values, vec!["1".to_string()], "{kind:?}");
    });
}

fn data_editor_with_filter_panel(
    cx: &mut TestAppContext,
) -> (
    gpui::Entity<NavicatMain>,
    &mut gpui::VisualTestContext,
    fluxdb_app::TabId,
) {
    use fluxdb_app::{AppCommand, AppEvent};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
    };

    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let settings = Settings {
        locale: "en".into(),
        ..Default::default()
    };
    storage.save_settings(&settings).expect("settings");
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "filter layout fixture".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("filter-layout.sqlite"),
            read_only: true,
        },
        options: std::collections::BTreeMap::from([("demo".into(), "true".into())]),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }]));
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
    let tab_id = cx.update(|_, cx| {
        let event = view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::OpenDataEditor(ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some("main".into()),
                    schema: None,
                    name: "Product".into(),
                    kind: ObjectKind::Table,
                }),
                cx,
            )
        });
        let AppEvent::TabOpened(tab_id) = event else {
            panic!("open data editor: {event:?}");
        };
        tab_id
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        let loaded = cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .is_some_and(|tab| {
                    matches!(
                        &tab.kind,
                        fluxdb_app::TabKind::DataEditor(editor) if editor.page.is_some()
                    )
                })
        });
        if loaded {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "demo data editor did not load"
        );
        std::thread::yield_now();
    }
    (view, cx, tab_id)
}

#[gpui::test]
fn filter_layout_caps_height_keeps_actions_fixed_and_preserves_collapsed_state(
    cx: &mut TestAppContext,
) {
    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(700.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let toggle = cx
        .debug_bounds("data-filter-sort-toggle")
        .expect("filter toggle");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("data-filter-panel").is_some());
    assert!(cx.debug_bounds("data-filter-toolbar-mode-switch").is_some());

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_draft_rules.insert(
                tab_id,
                (0..24)
                    .map(|index| crate::DataFilterRule {
                        enabled: true,
                        field: Some("id".into()),
                        operator: crate::DataFilterOperator::Gt,
                        values: vec![index.to_string()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 0,
                        group_end: 0,
                    })
                    .collect(),
            );
            this.data_filter_modes
                .insert(tab_id, crate::DataFilterMode::Text);
            this.data_filter_texts
                .insert(tab_id, "\"id\" > 1".into());
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let panel = cx.debug_bounds("data-filter-panel").expect("filter panel");
    let fixed = cx
        .debug_bounds("data-filter-fixed-actions")
        .expect("fixed actions region");
    let apply_row = cx
        .debug_bounds("data-filter-apply-row")
        .expect("apply row");
    let scroll = cx
        .debug_bounds("data-filter-scroll-area")
        .expect("condition scroll area");
    assert!(
        scroll.bottom() <= fixed.bottom() + gpui::px(1.),
        "condition area must end before fixed actions"
    );
    assert!(
        apply_row.top() >= fixed.top() && apply_row.bottom() <= panel.bottom(),
        "sort + apply rows must stay inside the panel"
    );
    assert!(
        panel.size.height <= gpui::px(0.35 * f32::from(gpui::px(700.)) + 1.),
        "filter panel exceeded 35% of the data area: {:?}",
        panel.size.height
    );

    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("data-filter-panel").is_none());
    assert!(cx.debug_bounds("data-filter-toolbar-mode-switch").is_none());
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.data_filter_modes.get(&tab_id),
            Some(&crate::DataFilterMode::Text)
        );
        assert_eq!(
            app.data_filter_texts.get(&tab_id).map(String::as_str),
            Some("\"id\" > 1")
        );
        assert_eq!(
            app.data_filter_draft_rules.get(&tab_id).map(Vec::len),
            Some(24)
        );
    });

    let toggle = cx
        .debug_bounds("data-filter-sort-toggle")
        .expect("filter toggle after collapse");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("data-filter-panel").is_some());
    assert!(cx.debug_bounds("data-filter-toolbar-mode-switch").is_some());
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            app.data_filter_modes.get(&tab_id),
            Some(&crate::DataFilterMode::Text)
        );
        assert_eq!(
            app.data_filter_draft_rules.get(&tab_id).map(Vec::len),
            Some(24)
        );
    });
}

/// 已有数据时重新加载（应用筛选、翻页、刷新）只盖表格区域，
/// 工具栏、筛选面板与 SQL 预览保持可见，不再整块替换成居中加载态。
#[gpui::test]
fn reloading_with_existing_rows_only_covers_table_area(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;

    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(700.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let toggle = cx
        .debug_bounds("data-filter-sort-toggle")
        .expect("filter toggle");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let panel_before = cx.debug_bounds("data-filter-panel").expect("filter panel");

    cx.update(|_, cx| {
        let event = view.update(cx, |this, cx| {
            this.dispatch(AppCommand::BeginDataPageLoad(tab_id), cx)
        });
        assert!(
            matches!(event, fluxdb_app::AppEvent::DataPageLoadStarted { .. }),
            "begin reload: {event:?}"
        );
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(
        cx.debug_bounds("data-page-loading").is_none(),
        "已有数据时不应整块替换成居中加载态"
    );
    assert_eq!(
        cx.debug_bounds("data-filter-panel"),
        Some(panel_before),
        "筛选面板必须保持可见"
    );
    let overlay = cx
        .debug_bounds("data-table-loading-overlay")
        .expect("table loading overlay");
    assert!(
        overlay.top() >= panel_before.bottom() - gpui::px(1.),
        "加载遮罩只能覆盖表格区域，不能盖住筛选面板: {overlay:?} vs {panel_before:?}"
    );
}

#[gpui::test]
fn multibyte_nested_filter_sql_round_trips_through_input_mode(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let toggle = cx
        .debug_bounds("data-filter-sort-toggle")
        .expect("filter toggle");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let sql = crate::data_filter_rules_sql_pretty(
        &[
            crate::DataFilterRule {
                enabled: true,
                field: Some("id".into()),
                operator: crate::DataFilterOperator::Eq,
                values: vec!["1".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::Or,
                group_depth: 0,
                        group_end: 0,
            },
            crate::DataFilterRule {
                enabled: true,
                field: Some("signup_date".into()),
                operator: crate::DataFilterOperator::Lt,
                values: vec!["2026-01-05".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::And,
                group_depth: 1,
                        group_end: 0,
            },
            crate::DataFilterRule {
                enabled: true,
                field: Some("signup_date".into()),
                operator: crate::DataFilterOperator::Ge,
                values: vec!["2026-04-19".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::Or,
                group_depth: 1,
                        group_end: 0,
            },
            crate::DataFilterRule {
                enabled: true,
                field: Some("name".into()),
                operator: crate::DataFilterOperator::Contains,
                values: vec!["等等".into()],
                advanced_expression: None,
                typed_values: None,
                connector: fluxdb_core::FilterCombinator::And,
                group_depth: 2,
                        group_end: 0,
            },
        ],
        fluxdb_core::DatabaseKind::MySql,
        &[
            fluxdb_core::Column {
                name: "id".into(),
                type_name: Some("integer".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            },
            fluxdb_core::Column {
                name: "signup_date".into(),
                type_name: Some("date".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
            fluxdb_core::Column {
                name: "name".into(),
                type_name: Some("varchar(64)".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            },
        ],
    );
    assert!(sql.contains("'%等等%'"), "{sql}");

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("id".into()),
                        operator: crate::DataFilterOperator::Eq,
                        values: vec!["1".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::Or,
                        group_depth: 0,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("signup_date".into()),
                        operator: crate::DataFilterOperator::Lt,
                        values: vec!["2026-01-05".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 1,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("signup_date".into()),
                        operator: crate::DataFilterOperator::Ge,
                        values: vec!["2026-04-19".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::Or,
                        group_depth: 1,
                        group_end: 0,
                    },
                    crate::DataFilterRule {
                        enabled: true,
                        field: Some("name".into()),
                        operator: crate::DataFilterOperator::Contains,
                        values: vec!["等等".into()],
                        advanced_expression: None,
                        typed_values: None,
                        connector: fluxdb_core::FilterCombinator::And,
                        group_depth: 2,
                        group_end: 0,
                    },
                ],
            );
            this.data_filter_modes
                .insert(tab_id, crate::DataFilterMode::Builder);
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let input_mode = cx
        .debug_bounds("data-filter-mode-input")
        .expect("input mode button");
    cx.simulate_click(input_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        assert!(
            view.read(cx)
                .data_filter_text_input
                .read(cx)
                .value()
                .contains("等等")
        );
    });
    let builder_mode = cx
        .debug_bounds("data-filter-mode-builder")
        .expect("builder mode button");
    cx.simulate_click(builder_mode.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("round-trip rules");
        assert_eq!(rules.len(), 4);
        assert_eq!(rules[3].values, vec!["等等".to_string()]);
    });
}

#[gpui::test]
fn complex_parenthesized_filter_sql_restores_editable_form_rules(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let toggle = cx
        .debug_bounds("data-filter-sort-toggle")
        .expect("filter toggle");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let sql = "(`id` = 1 OR ((`signup_date` < '2026-01-05' AND `signup_date` >= '2026-04-19') OR (`name` LIKE '%等等%' AND `id` = 1)))";
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_modes
                .insert(tab_id, crate::DataFilterMode::Text);
            this.data_filter_texts.insert(tab_id, sql.into());
            this.data_filter_text_input
                .update(cx, |input, cx| input.set_value(sql, window, cx));
        });
        window.draw(cx).clear(cx);
    });
    let builder = cx
        .debug_bounds("data-filter-mode-builder")
        .expect("builder mode button");
    cx.simulate_click(builder.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx
        .debug_bounds("data-filter-advanced-rule-0")
        .is_none());
    cx.update(|_, cx| {
        let rules = view
            .read(cx)
            .data_filter_draft_rules
            .get(&tab_id)
            .expect("restored form rules");
        assert_eq!(rules.len(), 5);
        assert!(rules.iter().all(|rule| rule.advanced_expression.is_none()));
        assert_eq!(
            rules.iter().map(|rule| rule.field.as_deref()).collect::<Vec<_>>(),
            vec![
                Some("id"),
                Some("signup_date"),
                Some("signup_date"),
                Some("name"),
                Some("id"),
            ]
        );
    });
}

#[gpui::test]
fn compact_sort_pills_toggle_remove_clear_and_undo(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let toggle = cx
        .debug_bounds("data-filter-sort-toggle")
        .expect("filter toggle");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_sort_draft_rules.insert(
                tab_id,
                vec![
                    crate::DataSortRule {
                        enabled: true,
                        field: "id".into(),
                        ascending: true,
                    },
                    crate::DataSortRule {
                        enabled: true,
                        field: "name".into(),
                        ascending: false,
                    },
                ],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("data-sort-pill-0").is_some());
    assert!(cx.debug_bounds("data-sort-handle-0").is_some());
    assert!(cx.debug_bounds("data-sort-field-1").is_some());
    assert!(cx.debug_bounds("data-sort-direction-1").is_some());
    assert!(cx.debug_bounds("data-sort-remove-1").is_some());

    let direction = cx
        .debug_bounds("data-sort-direction-1")
        .expect("sort direction");
    cx.simulate_click(direction.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        assert!(view.read(cx).data_sort_draft_rules[&tab_id][1].ascending);
    });
    let remove = cx
        .debug_bounds("data-sort-remove-1")
        .expect("remove sort rule");
    cx.simulate_click(remove.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        assert_eq!(view.read(cx).data_sort_draft_rules[&tab_id].len(), 1);
    });
    let clear = cx
        .debug_bounds("data-sort-clear")
        .expect("clear sort rules");
    cx.simulate_click(clear.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert!(!app.data_sort_draft_rules.contains_key(&tab_id));
        assert!(app.data_sort_clear_undo.contains_key(&tab_id));
        assert!(!app.data_sort_texts.contains_key(&tab_id));
    });
    let undo = cx
        .debug_bounds("data-sort-undo-clear")
        .expect("undo clear sort rules");
    cx.simulate_click(undo.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(app.data_sort_draft_rules[&tab_id].len(), 1);
        assert!(!app.data_sort_clear_undo.contains_key(&tab_id));
    });
}

/// 字段弹层必须贴着被点的字段显示，不能浮在写死的固定坐标上。
#[gpui::test]
fn filter_field_popover_anchors_to_clicked_field(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(700.)));
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_panels.insert(tab_id);
            this.data_filter_modes
                .insert(tab_id, crate::DataFilterMode::Builder);
            this.data_filter_draft_rules.insert(
                tab_id,
                vec![crate::DataFilterRule {
                    enabled: true,
                    field: Some("id".into()),
                    operator: crate::DataFilterOperator::Eq,
                    values: vec!["1".into()],
                    advanced_expression: None,
                    typed_values: None,
                    connector: fluxdb_core::FilterCombinator::And,
                    group_depth: 0,
                    group_end: 0,
                }],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });

    let field = cx
        .debug_bounds("data-filter-field-0")
        .expect("field cell");
    let click_at = field.center();
    cx.simulate_click(click_at, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let menu = cx
        .debug_bounds("data-filter-popover-menu")
        .expect("field popover must open");
    // 弹层左上角对齐点击点（纵向再下移 4px 让开被点的文字）。
    assert!(
        (f32::from(menu.left()) - f32::from(click_at.x)).abs() <= 2.,
        "popover left must follow the click: menu={menu:?} click={click_at:?}"
    );
    assert!(
        (f32::from(menu.top()) - f32::from(click_at.y) - 4.).abs() <= 2.,
        "popover top must follow the click: menu={menu:?} click={click_at:?}"
    );
    assert!(
        menu.right() <= gpui::px(1000.) && menu.bottom() <= gpui::px(700.),
        "popover must stay inside the window: {menu:?}"
    );
}

#[gpui::test]
fn sort_field_popover_anchors_to_clicked_pill(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(700.)));
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_panels.insert(tab_id);
            this.data_sort_draft_rules.insert(
                tab_id,
                vec![crate::DataSortRule {
                    enabled: true,
                    field: "id".into(),
                    ascending: true,
                }],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });

    let field = cx
        .debug_bounds("data-sort-field-0")
        .expect("sort field cell");
    let click_at = field.center();
    cx.simulate_click(click_at, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let menu = cx
        .debug_bounds("data-filter-popover-menu")
        .expect("sort field popover must open");
    assert!(
        (f32::from(menu.left()) - f32::from(click_at.x)).abs() <= 2.
            && (f32::from(menu.top()) - f32::from(click_at.y) - 4.).abs() <= 2.,
        "sort popover must follow the click: menu={menu:?} click={click_at:?}"
    );
}

#[gpui::test]
fn sort_rule_move_updates_priority_order(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_sort_draft_rules.insert(
                tab_id,
                vec![
                    crate::DataSortRule {
                        enabled: true,
                        field: "id".into(),
                        ascending: true,
                    },
                    crate::DataSortRule {
                        enabled: true,
                        field: "name".into(),
                        ascending: false,
                    },
                    crate::DataSortRule {
                        enabled: true,
                        field: "created_at".into(),
                        ascending: true,
                    },
                ],
            );
            this.move_data_sort_rule(tab_id, 0, 1, cx);
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|_, cx| {
        let rules = &view.read(cx).data_sort_draft_rules[&tab_id];
        assert_eq!(
            rules.iter().map(|rule| rule.field.as_str()).collect::<Vec<_>>(),
            vec!["name", "id", "created_at"]
        );
    });
}

#[gpui::test]
fn dragging_sort_pill_handle_reorders_rules(cx: &mut TestAppContext) {
    use gpui::MouseButton;

    let _guard = locale_guard();
    let (view, cx, tab_id) = data_editor_with_filter_panel(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let toggle = cx
        .debug_bounds("data-filter-sort-toggle")
        .expect("filter toggle");
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.data_filter_panels.insert(tab_id);
            this.data_sort_draft_rules.insert(
                tab_id,
                vec![
                    crate::DataSortRule {
                        enabled: true,
                        field: "id".into(),
                        ascending: true,
                    },
                    crate::DataSortRule {
                        enabled: true,
                        field: "name".into(),
                        ascending: false,
                    },
                    crate::DataSortRule {
                        enabled: true,
                        field: "created_at".into(),
                        ascending: true,
                    },
                ],
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let handle = cx
        .debug_bounds("data-sort-handle-0")
        .expect("sort drag handle");
    let target = cx
        .debug_bounds("data-sort-pill-1")
        .expect("sort drop target");
    let start = handle.center();
    let end = target.center();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        start + gpui::point(gpui::px(3.), gpui::px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        gpui::point(start.x + gpui::px(8.), (start.y + end.y) / 2.),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        let rules = &view.read(cx).data_sort_draft_rules[&tab_id];
        assert_eq!(
            rules
                .iter()
                .map(|rule| rule.field.as_str())
                .collect::<Vec<_>>(),
            vec!["name", "id", "created_at"]
        );
    });
}

macro_rules! data_filter_mode_round_trip_tests {
    ($($name:ident: $kind:expr => $expected:literal),+ $(,)?) => {
        $(
            #[gpui::test]
            fn $name(cx: &mut TestAppContext) {
                let _guard = locale_guard();
                data_filter_mode_round_trip_for_database(cx, $kind, $expected);
            }
        )+
    };
}

data_filter_mode_round_trip_tests! {
    mysql_filter_form_input_round_trip: fluxdb_core::DatabaseKind::MySql => "`id` LIKE '%1%'",
    tidb_filter_form_input_round_trip: fluxdb_core::DatabaseKind::TiDb => "`id` LIKE '%1%'",
    postgres_filter_form_input_round_trip: fluxdb_core::DatabaseKind::Postgres => "CAST(\"id\" AS TEXT) LIKE '%1%'",
    sqlite_filter_form_input_round_trip: fluxdb_core::DatabaseKind::Sqlite => "\"id\" LIKE '%1%'",
    sqlserver_filter_form_input_round_trip: fluxdb_core::DatabaseKind::SqlServer => "CAST([id] AS NVARCHAR(MAX)) LIKE '%1%'",
    dameng_filter_form_input_round_trip: fluxdb_core::DatabaseKind::Dameng => "CAST(\"id\" AS VARCHAR(4000)) LIKE '%1%'",
}
