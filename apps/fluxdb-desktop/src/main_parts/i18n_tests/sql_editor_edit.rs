#[gpui::test]
fn ai_sql_selection_writes_back_without_execution_and_can_undo(cx: &mut TestAppContext) {
    sql_editor_edit_journey(cx, false);
}

#[gpui::test]
fn ai_sql_error_repair_writes_back_only_the_failed_statement(cx: &mut TestAppContext) {
    sql_editor_edit_journey(cx, true);
}

fn sql_editor_edit_journey(cx: &mut TestAppContext, repair: bool) {
    use fluxdb_app::{AiModelSaveInput, AppCommand, AppEvent, TabKind};
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};
    let _guard = locale_guard();
    cx.background_executor.allow_parking();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let settings = Settings {
        locale: "en".into(),
        show_sidebar: false,
        ..Default::default()
    };
    storage.save_settings(&settings).expect("settings");
    let database = temp.path().join("untouched.sqlite");
    fs::write(&database, []).expect("database placeholder");
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
        name: "SQL edit target".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: database.clone(),
            read_only: false,
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
        display_name: "offline SQL edit fixture".into(),
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
    let AppEvent::TabOpened(tab_id) = controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
        connection_id: ConnectionId(1),
        database: None,
        schema: None,
    }) else {
        panic!("query tab")
    };
    let original = "SELECT '中文';\nSELEC 1;\nSELECT 3;";
    controller.dispatch(AppCommand::UpdateQueryText {
        tab_id,
        pane_id: fluxdb_app::QueryPaneId(1),
        text: original.into(),
    });
    if repair {
        controller.dispatch(AppCommand::FinishQueryExecution {
            tab_id,
            pane_id: fluxdb_app::QueryPaneId(1),
            result: Ok(fluxdb_core::QueryExecutionResult {
                summaries: vec![fluxdb_core::QueryExecutionSummary {
                    // 模拟执行器追加默认 LIMIT 后的实际 SQL；来源快照仍指向编辑器原文。
                    sql: "SELEC 1 LIMIT 100;".into(),
                    kind: fluxdb_core::QueryStatementKind::ResultSet,
                    success: false,
                    message: "near SELEC: syntax error".into(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 1,
                }],
                results: Vec::new(),
                rollback_snapshots: Vec::new(),
            }),
        });
    }
    cx.update(|cx| {
        extend_component_i18n();
        gpui_component::init(cx);
        crate::editor_component::register_editor_shortcuts(cx);
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
            app.ai_chat.scripted_fixture = true;
            app.ai_chat.scripted_edit_sql = Some("SELECT 2;".into());
            app
        });
        Root::new(view, window, cx)
    });
    cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(900.)));
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
    if repair {
        let statements = crate::sql_editor_adapter::build_statement_runs(
            original,
            fluxdb_core::DatabaseKind::MySql,
        );
        cx.update(|_, cx| {
            view.update(cx, |app, _| {
                app.query_execution_sources
                    .insert(fluxdb_app::QueryPaneKey::new(tab_id, fluxdb_app::QueryPaneId(1)), vec![statements[1].clone()]);
            });
        });
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let editor = cx.update(|_, cx| view.read(cx).query_editors[&fluxdb_app::QueryPaneKey::new(tab_id, fluxdb_app::QueryPaneId(1))].clone());
    if !repair {
        // 初始选区属于场景准备；被断言的修改、发送和撤销均由真实控件事件驱动。
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let start = original.find("SELEC 1;").expect("selection");
                editor.select_range(fluxdb_editor_core::Range::new(start, start + 8), cx);
            })
        });
        assert!(
            cx.debug_bounds("query-ai-edit").is_none(),
            "SQL edit must be initiated from the AI drawer only"
        );
        let ask_ai = cx.debug_bounds("ask-ai").expect("ask ai button");
        cx.simulate_click(ask_ai.center(), Modifiers::default());
        cx.update(|window, cx| {
            assert!(view.read(cx).ai_chat.open);
            window.draw(cx).clear(cx);
        });
        let input_bounds = cx.debug_bounds("ai-chat-input").expect("chat input");
        cx.simulate_click(input_bounds.center(), Modifiers::default());
        cx.simulate_input("Fix the selected SQL and write it back.");
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let send = cx.debug_bounds("ai-chat-send").expect("send button");
        cx.simulate_click(send.center(), Modifiers::default());
    } else {
        let button = cx.debug_bounds("query-ai-fix").expect("repair button");
        cx.simulate_click(button.center(), Modifiers::default());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        let finished = cx.update(|_, cx| {
            let app = view.read(cx);
            assert!(app._query_execute_tasks.is_empty(), "must not execute SQL");
            app.ai_chat.running.is_none() && app.ai_chat._io.is_none() && !app.ai_chat.loading
        });
        if finished {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "SQL edit did not finish"
        );
        std::thread::yield_now();
    }
    cx.update(|_, cx| {
        let app = view.read(cx);
        assert_eq!(
            editor.read(cx).text(),
            "SELECT '中文';\nSELECT 2;\nSELECT 3;",
            "error={:?} msg={:?} pending={}",
            app.ai_chat.error,
            app.app_message.as_ref().map(|m| m.text.clone()),
            app.ai_chat.pending_sql_edit.is_some(),
        );
        let TabKind::QueryEditor(query) = &app
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .expect("query tab")
            .kind
        else {
            panic!("query")
        };
        assert_eq!(query.text, editor.read(cx).text());
        assert!(!query.running);
        let message = app.app_message.as_ref().expect("visible feedback");
        assert_eq!(message.kind, crate::AppMessageKind::Success);
        assert_eq!(
            message.text,
            crate::tr("app.ai.sql_edit.applied").to_string()
        );
    });
    assert_eq!(
        fs::read(&database).expect("database unchanged"),
        Vec::<u8>::new()
    );
    let session_id = cx.update(|_, cx| {
        view.read(cx)
            .controller
            .state()
            .ai_conversation
            .session_id
            .clone()
            .expect("session")
    });
    storage
        .with_ai_connection(|conn| {
            let turns = fluxdb_storage::ai_platform::sessions::list_turns(conn, &session_id)?;
            let snapshot: serde_json::Value =
                serde_json::from_str(&turns.last().expect("turn").runtime_snapshot_json)
                    .expect("snapshot");
            // 选区自动绑定用于写回，但不把本轮降成“只能修改、禁止一切查询”的模式；
            // 失败修复仍保持独占断言（见 repair 分支后续用例）。
            let context: serde_json::Value =
                serde_json::from_str(snapshot["ui_context"].as_str().expect("page context"))
                    .expect("context");
            assert!(
                context["capabilities"]
                    .as_array()
                    .expect("capabilities")
                    .contains(&serde_json::json!("editor.apply_edit"))
            );
            Ok(())
        })
        .expect("persisted edit-only scope");
    cx.update(|window, cx| {
        let focus = editor.read(cx).focus_handle.clone();
        focus.focus(window, cx);
        window.draw(cx).clear(cx);
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-z"
    } else {
        "ctrl-z"
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(editor.read(cx).text(), original));
}
