// MongoDB 命令工作台的文档结果卡片：真实生产视图的无头点击驱动。
//
// 不连库：直接写入一条带结构化文档的执行记录，验证卡头 / 字段列表 / 文档↔JSON 切换
// 与折叠交互都真的画出来，而不是只编译通过。

#[gpui::test]
fn mongo_workbench_renders_document_cards_with_collapsible_json(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MongoDocument, MongoShellResult,
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
    let connection_id = ConnectionId(22);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "mongo workbench".into(),
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
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(700.)));
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
        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::OpenMongoWorkbench {
                    connection_id,
                    database: "shop".into(),
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    let tab_id = cx.update(|_, cx| {
        view.read(cx)
            .controller
            .state()
            .active_tab
            .expect("mongo workbench tab is active")
    });
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.dispatch(
                AppCommand::FinishMongoWorkbenchExecution {
                    tab_id,
                    text: "db.orders.find()".into(),
                    result: Ok(MongoShellResult {
                        output: "{}\n".into(),
                        document_count: Some(1),
                        documents: vec![MongoDocument {
                            id: "66e000000000000000000001".into(),
                            json: r#"{"_id":{"$oid":"66e000000000000000000001"},"name":"A-1","nested":{"qty":2}}"#.into(),
                        }],
                        ..Default::default()
                    }),
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    assert!(
        cx.debug_bounds("mongo-workbench-documents:1").is_some(),
        "document results render as a card list"
    );
    assert!(
        cx.debug_bounds("mongo-workbench-document:1:0").is_some(),
        "one document means one card"
    );
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_some(),
        "the card body is the field list"
    );
    assert!(
        cx.debug_bounds("mongo-wb-json:1-json:0").is_none(),
        "JSON stays collapsed until requested"
    );

    let view_toggle = cx
        .debug_bounds("mongo-workbench-doc-view:1:0")
        .expect("view toggle renders");
    cx.simulate_click(view_toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-wb-json:1-json:0").is_some(),
        "switching to JSON shows the foldable JSON block"
    );
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_none(),
        "JSON mode replaces the field list"
    );

    let view_toggle = cx
        .debug_bounds("mongo-workbench-doc-view:1:0")
        .expect("view toggle stays in the header");
    cx.simulate_click(view_toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-wb-json:1-json:0").is_none(),
        "switching back hides the JSON block"
    );
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_some(),
        "switching back restores the field list"
    );
    assert!(
        cx.debug_bounds("mongo-workbench-doc-copy:1:0").is_some(),
        "the copy button renders next to the view toggle"
    );

    let document_toggle = cx
        .debug_bounds("mongo-workbench-doc-toggle:1:0")
        .expect("document collapse toggle renders");
    cx.simulate_click(document_toggle.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("mongo-document-fields:0").is_none(),
        "collapsing the document drops its field list"
    );
    assert!(
        cx.debug_bounds("mongo-workbench-document:1:0").is_some(),
        "the card header stays after collapsing"
    );
}
