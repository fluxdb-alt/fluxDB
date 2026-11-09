/// 结构化 EXPLAIN 的 JSON 往往有几十上百行；「表格」视图必须能纵向滚动，
/// 而不是被结果区分栏裁掉。
#[gpui::test]
fn query_plan_raw_json_scrolls_when_taller_than_output_panel(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent};
    use fluxdb_core::{
        CellValue, Column, ConnectionConfig, ConnectionId, DataPage, DatabaseKind, Endpoint,
        QueryExecutionResult, Row,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let settings = Settings {
        locale: "en".into(),
        show_sidebar: false,
        // 只验证计划视图滚动；关闭补全索引，避免后台索引任务越过用例边界。
        enable_completion_index: false,
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
        name: "Query plan scroll".into(),
        kind: DatabaseKind::MySql,
        // 本用例不打开连接，任何意外拨号都不得命中真实实例。
        endpoint: Endpoint::Tcp {
            host: "plan-scroll.invalid".into(),
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
    let AppEvent::TabOpened(tab_id) = controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
        connection_id: ConnectionId(1),
        database: None,
        schema: None,
    }) else {
        panic!("query tab");
    };

    // 额外塞一段长数组：解析器忽略未知字段，但 JSON 视图会把它完整画出来。
    let long_tail = (0..200)
        .map(|index| format!("\"probe-{index}\","))
        .collect::<String>();
    let explain_json = format!(
        r#"{{"query_block":{{"select_id":1,"cost_info":{{"query_cost":"1.00"}},"table":{{"table_name":"users","access_type":"ALL","rows_examined_per_scan":1000,"rows_produced_per_join":1000,"cost_info":{{"read_cost":"0.25","eval_cost":"0.10","prefix_cost":"0.35","data_read_per_join":"8K"}},"used_columns":["id"],"probes":[{long_tail}0]}}}}}}"#
    );
    controller.dispatch(AppCommand::FinishQueryExecution {
        tab_id,
        result: Ok(QueryExecutionResult {
            summaries: Vec::new(),
            results: vec![DataPage {
                columns: vec![Column {
                    name: "EXPLAIN".into(),
                    type_name: Some("json".into()),
                    nullable: false,
                    primary_key: false,
                    comment: None,
                }],
                rows: vec![Row {
                    values: vec![CellValue::Json(explain_json)],
                }],
                offset: 0,
                limit: 1,
                has_more: false,
                incomplete: None,
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
            let mut app = navicat_main_new(
                window,
                cx,
                controller,
                storage,
                gpui_component::ThemeMode::Light,
            );
            app.ai_chat.recovery_pending = false;
            app
        });
        Root::new(view, window, cx)
    });
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(900.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();

    let plan_tab = cx
        .debug_bounds("query-output-tab:plan")
        .expect("plan tab must appear for a parsed EXPLAIN");
    cx.simulate_click(plan_tab.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let table_toggle = cx
        .debug_bounds("query-plan-toggle:table")
        .expect("table toggle");
    cx.simulate_click(table_toggle.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    // 选择器落在滚动内容上（高度 = 内容高），滚动后它的原点会上移。
    let content = cx
        .debug_bounds("query-plan-raw-scroll")
        .expect("raw json content");
    assert!(
        content.size.height > gpui::px(600.),
        "the JSON must overflow the output panel: {content:?}"
    );
    let before = content.origin.y;
    // 视口顶部在内容原点上方的内边距之外；滚轮落在可见区域内。
    let viewport_top = plan_tab.origin.y + plan_tab.size.height;
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(content.origin.x + gpui::px(40.), viewport_top + gpui::px(60.)),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., -8.)),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    let after = cx
        .debug_bounds("query-plan-raw-scroll")
        .expect("raw json content stays laid out")
        .origin
        .y;
    assert!(
        after < before,
        "vertical scroll must move the JSON content: {before} -> {after}"
    );
}
