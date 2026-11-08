// MongoDB 集合右键菜单与导出弹框：真实生产视图渲染 + 贴底右键不越界。
//
// 只用一个用例：`add_window_view` 会借用 `TestAppContext` 生成 `VisualTestContext`，
// 拆成多个用例就得把整段初始化复制一遍。

#[gpui::test]
fn mongo_collection_menu_and_export_dialog_render_in_production_view(cx: &mut TestAppContext) {
    use crate::{
        CollectionContextMenu, MongoExportForm, MongoExportFormat, MongoExportScope, MongoExportTab,
    };
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
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
    let connection_id = ConnectionId(1);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "mongo menu".into(),
        kind: DatabaseKind::MongoDb,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 27017,
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
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(620.)));
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
    let object = |kind: ObjectKind| ObjectPath {
        connection_id,
        database: Some("shop".into()),
        schema: None,
        name: "orders".into(),
        kind,
    };

    // 贴底右键：破坏性项必须仍在视口内。
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.show_collection_context_menu(
                CollectionContextMenu {
                    object_path: object(ObjectKind::Collection),
                    position: gpui::point(gpui::px(100.), gpui::px(600.)),
                    read_only: false,
                },
                window,
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    let menu = cx
        .debug_bounds("collection-context-menu")
        .expect("collection context menu");
    let viewport_bottom = gpui::px(620.) - gpui::px(8.);
    assert!(
        menu.origin.y + menu.size.height <= viewport_bottom,
        "collection menu must keep destructive actions inside the viewport: {menu:?}"
    );
    assert!(
        cx.debug_bounds("collection-menu-item:Drop collection")
            .is_some(),
        "writable collection must offer drop"
    );

    // 视图只读：写入类入口不渲染，只读项仍在。
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.show_collection_context_menu(
                CollectionContextMenu {
                    object_path: object(ObjectKind::View),
                    position: gpui::point(gpui::px(100.), gpui::px(100.)),
                    read_only: true,
                },
                window,
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("collection-menu-item:Drop collection")
            .is_none()
    );
    assert!(cx.debug_bounds("collection-menu-item:Rename").is_none());
    assert!(
        cx.debug_bounds("collection-menu-item:Import data")
            .is_none()
    );
    assert!(cx.debug_bounds("collection-menu-item:Export").is_some());

    // 导出弹框两个页签都能渲染。
    let mut form = MongoExportForm {
        object: object(ObjectKind::Collection),
        format: MongoExportFormat::JsonLines,
        scope: MongoExportScope::Conditions,
        active_tab: MongoExportTab::General,
        filter: r#"{"status":"paid"}"#.into(),
        projection: r#"{"_id":1,"total":1}"#.into(),
        sort: r#"{"total":-1}"#.into(),
        running: false,
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        exported_documents: 0,
        logs: Vec::new(),
        error: None,
        finished: false,
        task_center_id: None,
    };
    for tab in [MongoExportTab::General, MongoExportTab::Log] {
        form.active_tab = tab;
        let pending = form.clone();
        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.pending_mongo_export = Some(pending);
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        assert!(
            cx.debug_bounds("mongo-export-modal").is_some(),
            "export dialog must render tab {tab:?}"
        );
    }
    // 条件导出直接在常规页展示 shell 查询，不需要单独的「条件」页签。
    // 上面循环最后一轮画的是消息日志页，这里必须重画常规页再断言。
    let draw = |form: MongoExportForm, cx: &mut gpui::VisualTestContext| {
        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.pending_mongo_export = Some(form);
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
    };
    form.active_tab = MongoExportTab::General;
    draw(form.clone(), cx);
    assert!(
        cx.debug_bounds("mongo-export-tab:Conditions").is_none(),
        "conditions must be folded into the general tab"
    );
    assert!(
        cx.debug_bounds("mongo-export-query-preview").is_some(),
        "conditions scope must show the shell query preview"
    );
    assert!(
        cx.debug_bounds("mongo-export-query-copy").is_some(),
        "query preview must be copyable"
    );
    assert!(
        cx.debug_bounds("mongo-export-summary").is_some(),
        "footer must show the export summary"
    );
    let conditions_height = cx
        .debug_bounds("mongo-export-modal")
        .expect("export modal")
        .size
        .height;
    // 与 Compass 一致：全量导出不展示查询预览（它就是 find({})）。
    form.scope = MongoExportScope::All;
    draw(form, cx);
    assert!(
        cx.debug_bounds("mongo-export-query-preview").is_none(),
        "all-documents scope must hide the query preview"
    );
    // 少了预览区就不该留 560 的固定高度，否则弹框下半截全是空白。
    let all_height = cx
        .debug_bounds("mongo-export-modal")
        .expect("export modal")
        .size
        .height;
    assert!(
        all_height < conditions_height,
        "all-documents dialog must shrink: {all_height:?} vs {conditions_height:?}"
    );
}
