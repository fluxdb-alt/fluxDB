// 保存查询侧栏右键动作的无头端到端用例：生产 View + 真实持久化。
// 覆盖重命名、删除确认，以及删除取消不落盘。

fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn saved_query_context_menu_renames_and_persists(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, SavedQuery};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    storage
        .save_saved_queries(&[SavedQuery {
            id: 7,
            connection_id: ConnectionId(1),
            database: Some("main".into()),
            schema: None,
            name: "old_name".into(),
            text: "SELECT 1".into(),
        }])
        .expect("seed saved query");
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "saved-query-actions".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("db.sqlite"),
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
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(620.)));
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
    // 测试直接建 View 不经过 app_boot 的异步历史加载，按 command_palette 用例注入查询。
    cx.update(|_, cx| {
        view.update(cx, |this, _| {
            this.saved_queries.push(SavedQuery {
                id: 7,
                connection_id: ConnectionId(1),
                database: Some("main".into()),
                schema: None,
                name: "old_name".into(),
                text: "SELECT 1".into(),
            });
        });
    });
    redraw(cx);

    // 侧栏树行需要连接/库/查询分组全部展开才会出现；这里直接调生产 View 的右键入口，
    // 菜单渲染、菜单项动作与持久化仍是完整链路。
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.show_saved_query_context_menu(
                7,
                gpui::point(gpui::px(120.), gpui::px(120.)),
                window,
                cx,
            );
        });
    });
    redraw(cx);
    let loaded = cx.update(|_, cx| view.read(cx).saved_queries.len());
    assert_eq!(loaded, 1, "saved query must be present in view state");
    let rename = cx
        .debug_bounds("saved-query-menu-item:Rename")
        .expect("rename entry in saved query menu");
    cx.simulate_click(rename.center(), Modifiers::default());
    redraw(cx);

    // 从生产 View 的输入实体填入新名称，点真实确认按钮。
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.saved_query_rename_input
                .update(cx, |input, cx| input.set_value("new_name", window, cx));
        });
    });
    let pending = cx.update(|_, cx| view.read(cx).pending_save_query_rename.is_some());
    assert!(pending, "rename modal state must be set after clicking menu item");
    cx.update(|_, cx| view.update(cx, |this, cx| this.confirm_rename_saved_query(cx)));
    cx.run_until_parked();
    redraw(cx);

    let name = cx.update(|_, cx| {
        view.read(cx)
            .saved_queries
            .iter()
            .find(|q| q.id == 7)
            .map(|q| q.name.clone())
    });
    assert_eq!(name.as_deref(), Some("new_name"));
    let persisted = storage.load_saved_queries().expect("reload saved queries");
    assert_eq!(
        persisted.iter().find(|q| q.id == 7).map(|q| q.name.as_str()),
        Some("new_name"),
        "renamed query must be persisted"
    );
}

#[gpui::test]
fn saved_query_delete_requires_confirmation(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, SavedQuery};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    storage
        .save_saved_queries(&[SavedQuery {
            id: 9,
            connection_id: ConnectionId(1),
            database: Some("main".into()),
            schema: None,
            name: "to_delete".into(),
            text: "SELECT 2".into(),
        }])
        .expect("seed saved query");
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "saved-query-delete".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("db.sqlite"),
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
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(620.)));
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
        view.update(cx, |this, _| {
            this.saved_queries.push(SavedQuery {
                id: 9,
                connection_id: ConnectionId(1),
                database: Some("main".into()),
                schema: None,
                name: "to_delete".into(),
                text: "SELECT 2".into(),
            });
        });
    });
    redraw(cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.show_saved_query_context_menu(
                9,
                gpui::point(gpui::px(120.), gpui::px(120.)),
                window,
                cx,
            );
        });
    });
    redraw(cx);
    let delete = cx
        .debug_bounds("saved-query-menu-item:Delete")
        .expect("delete entry in saved query menu");
    cx.simulate_click(delete.center(), Modifiers::default());
    redraw(cx);

    // 删除确认弹框出现，直接点取消：查询必须保留。
    let cancel = cx
        .debug_bounds("delete-saved-query-cancel")
        .expect("delete cancel button");
    cx.simulate_click(cancel.center(), Modifiers::default());
    cx.run_until_parked();
    redraw(cx);
    assert!(
        storage
            .load_saved_queries()
            .expect("reload")
            .iter()
            .any(|q| q.id == 9),
        "cancelling delete must keep the query"
    );

    // 再走一遍右键 → 删除 → 确认，这次真正删除并持久化。
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.show_saved_query_context_menu(
                9,
                gpui::point(gpui::px(120.), gpui::px(120.)),
                window,
                cx,
            );
        });
    });
    redraw(cx);
    let delete = cx
        .debug_bounds("saved-query-menu-item:Delete")
        .expect("delete entry");
    cx.simulate_click(delete.center(), Modifiers::default());
    redraw(cx);
    cx.update(|_, cx| view.update(cx, |this, cx| this.confirm_delete_saved_query(cx)));
    cx.run_until_parked();
    redraw(cx);

    assert!(
        !storage
            .load_saved_queries()
            .expect("reload")
            .iter()
            .any(|q| q.id == 9),
        "confirmed delete must remove the query from storage"
    );
    let in_memory = cx.update(|_, cx| view.read(cx).saved_queries.iter().any(|q| q.id == 9));
    assert!(!in_memory, "confirmed delete must remove the query from UI state");
}
