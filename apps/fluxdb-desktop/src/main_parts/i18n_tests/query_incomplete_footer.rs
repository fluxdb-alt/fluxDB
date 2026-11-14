#[gpui::test]
fn first_incomplete_query_page_shows_footer_reason(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent};
    use fluxdb_core::{
        Column, ConnectionConfig, ConnectionId, DataPage, DatabaseKind, Endpoint, PageIncomplete,
        QueryExecutionResult, Row,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let settings = Settings {
        locale: "en".into(),
        show_sidebar: false,
        ..Default::default()
    };
    storage.save_settings(&settings).expect("settings");
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::SaveSettings(settings));
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "Incomplete page".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("incomplete.sqlite"),
            read_only: true,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
        mongo_profile: None,
    }]));
    let AppEvent::TabOpened(tab_id) = controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
        connection_id: ConnectionId(1),
        database: None,
        schema: None,
    }) else {
        panic!("query tab");
    };
    // 通过真实查询完成回桥发布一个首页不完整页，不直接调用页脚布局函数。
    controller.dispatch(AppCommand::FinishQueryExecution {
        tab_id,
        pane_id: fluxdb_app::QueryPaneId(1),
        result: Ok(QueryExecutionResult {
            summaries: Vec::new(),
            results: vec![DataPage {
                columns: vec![Column {
                    name: "value".into(),
                    type_name: Some("text".into()),
                    nullable: true,
                    primary_key: false,
                    comment: None,
                }],
                rows: vec![Row {
                    values: vec![fluxdb_core::CellValue::I64(1)],
                }],
                offset: 0,
                limit: 100,
                has_more: false,
                incomplete: Some(PageIncomplete {
                    reason: "storage limit reached".into(),
                }),
            }],
            rollback_snapshots: Vec::new(),
        }),
    });
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
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(900.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();

    let footer = cx
        .debug_bounds("query-result-incomplete")
        .expect("首页 incomplete 必须渲染原因");
    assert!(footer.size.height > gpui::px(0.));
}
