/// 数据页「编辑即提交」端到端：默认关闭，开启后一次单元格编辑立即写库并清空暂存。
#[gpui::test]
fn auto_commit_writes_each_edit_and_clears_staged_changes(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent, TabKind};
    use fluxdb_core::{
        CellValue, ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let database = temp.path().join("auto-commit.sqlite");
    let connection = fluxdb_storage::AiConnection::open(&database).expect("SQLite database");
    connection
        .execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT);
             INSERT INTO items VALUES (1, 'before');",
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
        name: "auto commit fixture".into(),
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
                    name: "items".into(),
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
                    TabKind::DataEditor(editor) => editor.page.as_ref().map(|page| page.rows.len()),
                    _ => None,
                })
        });
        if loaded == Some(1) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "initial page did not load"
        );
        std::thread::yield_now();
    }
    // 默认关闭：开关只属于已打开的标签。
    cx.update(|_, cx| {
        assert!(!view.read(cx).auto_commit_data_tabs.contains(&tab_id));
    });
    cx.update(|_, cx| {
        view.update(cx, |this, cx| this.set_data_auto_commit(tab_id, true, cx));
    });
    cx.update(|_, cx| {
        view.update(cx, |this, cx| {
            let event = this.dispatch(
                AppCommand::EditDataCell {
                    tab_id,
                    row: 0,
                    column: 1,
                    value: CellValue::Text("after".into()),
                },
                cx,
            );
            assert!(matches!(event, AppEvent::TabActivated(_)), "{event:?}");
        });
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        let stored = fluxdb_storage::AiConnection::open(&database)
            .ok()
            .and_then(|connection| {
                connection
                    .query_row("SELECT name FROM items WHERE id = 1", [], |row| {
                        row.get::<_, String>(0)
                    })
                    .ok()
            });
        let settled = cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .is_some_and(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => {
                        editor.changes.is_none()
                            && editor.page.as_ref().is_some_and(|page| {
                                page.rows.first().is_some_and(|row| {
                                    row.values.get(1)
                                        == Some(&CellValue::Text("after".into()))
                                })
                            })
                    }
                    _ => false,
                })
        });
        if stored.as_deref() == Some("after") && settled {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "edit was not auto-committed: stored={stored:?}, settled={settled}"
        );
        std::thread::yield_now();
    }
}
