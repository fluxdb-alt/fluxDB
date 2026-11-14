#[gpui::test]
fn query_summary_columns_align_and_stay_readable_when_narrow(cx: &mut TestAppContext) {
    use crate::QUERY_SUMMARY_TABLE_MIN_WIDTH;
    use fluxdb_app::{AppCommand, AppEvent};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, QueryExecutionResult,
        QueryExecutionSummary, QueryStatementKind,
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
        name: "Query summary layout".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("summary.sqlite"),
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
    let AppEvent::TabOpened(tab_id) = controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
        connection_id: ConnectionId(1),
        database: None,
        schema: None,
    }) else {
        panic!("query tab");
    };
    controller.dispatch(AppCommand::FinishQueryExecution {
        tab_id,
        pane_id: fluxdb_app::QueryPaneId(1),
        result: Ok(QueryExecutionResult {
            summaries: vec![QueryExecutionSummary {
                sql: "db.orders.createIndex({ customer_id: 1, created_at: -1 })".into(),
                kind: QueryStatementKind::Command,
                success: true,
                message: "OK, 0 rows affected".into(),
                returned_rows: 0,
                affected_rows: 0,
                elapsed_ms: 12,
            }],
            results: Vec::new(),
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
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert_query_summary_columns(cx);
    assert!(
        cx.debug_bounds("query-summary-table")
            .expect("summary table")
            .size
            .width
            >= gpui::px(QUERY_SUMMARY_TABLE_MIN_WIDTH)
    );

    cx.simulate_resize(gpui::size(gpui::px(600.), gpui::px(700.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert_query_summary_columns(cx);
    let table = cx.debug_bounds("query-summary-table").expect("summary table");
    assert!(
        table.size.width >= gpui::px(QUERY_SUMMARY_TABLE_MIN_WIDTH),
        "narrow layouts must scroll instead of shrinking columns: {table:?}"
    );
    assert!(cx.update(|_, cx| {
        view.read(cx).controller.state().tabs.iter().any(
            |tab| tab.id == tab_id && matches!(&tab.kind, fluxdb_app::TabKind::QueryEditor(editor) if editor.summaries.len() == 1)
        )
    }));
}

/// 批量执行会产生任意多行摘要；超出结果区高度时必须能纵向滚动，而不是把行挤出面板。
#[gpui::test]
fn query_summary_rows_scroll_when_they_overflow_the_output_panel(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, QueryExecutionResult,
        QueryExecutionSummary, QueryStatementKind,
    };

    const ROWS: usize = 40;
    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let settings = Settings {
        locale: "en".into(),
        show_sidebar: false,
        // 只验证摘要滚动；关闭补全索引，避免后台索引任务越过用例边界。
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
        name: "Query summary scroll".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("summary.sqlite"),
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
    controller.dispatch(AppCommand::FinishQueryExecution {
        tab_id,
        pane_id: fluxdb_app::QueryPaneId(1),
        result: Ok(QueryExecutionResult {
            summaries: (0..ROWS)
                .map(|index| QueryExecutionSummary {
                    sql: format!("CREATE TABLE batch_{index} (id INT PRIMARY KEY)"),
                    kind: QueryStatementKind::Command,
                    success: true,
                    message: "OK".into(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 1,
                })
                .collect(),
            results: Vec::new(),
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
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();

    // 选择器落在滚动内容上（高度=内容高），不能当视口用；只取一个内容坐标做滚动前后对比。
    let last = "query-summary-row:39:statement";
    let before = cx
        .debug_bounds(last)
        .unwrap_or_else(|| panic!("{last} must be laid out"))
        .origin
        .y;
    let panel_viewport = cx.debug_bounds("query-summary-scroll").expect("scroll region");

    cx.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(
            panel_viewport.origin.x + gpui::px(20.),
            panel_viewport.origin.y + gpui::px(20.),
        ),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., -8.)),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    let after = cx
        .debug_bounds(last)
        .unwrap_or_else(|| panic!("{last} must still be laid out after scrolling"))
        .origin
        .y;
    assert!(after < before, "vertical scroll must move summary rows: {before} -> {after}");
}

fn assert_query_summary_columns(cx: &mut gpui::VisualTestContext) {
    use crate::{
        QUERY_SUMMARY_AFFECTED_WIDTH, QUERY_SUMMARY_ELAPSED_WIDTH, QUERY_SUMMARY_RETURNED_WIDTH,
        QUERY_SUMMARY_STATEMENT_MIN_WIDTH, QUERY_SUMMARY_TYPE_WIDTH,
    };

    for (header_selector, row_selector, minimum_width) in [
        (
            "query-summary-header:statement",
            "query-summary-row:0:statement",
            QUERY_SUMMARY_STATEMENT_MIN_WIDTH,
        ),
        (
            "query-summary-header:type",
            "query-summary-row:0:type",
            QUERY_SUMMARY_TYPE_WIDTH,
        ),
        (
            "query-summary-header:returned",
            "query-summary-row:0:returned",
            QUERY_SUMMARY_RETURNED_WIDTH,
        ),
        (
            "query-summary-header:affected",
            "query-summary-row:0:affected",
            QUERY_SUMMARY_AFFECTED_WIDTH,
        ),
        (
            "query-summary-header:elapsed",
            "query-summary-row:0:elapsed",
            QUERY_SUMMARY_ELAPSED_WIDTH,
        ),
    ] {
        let header = cx.debug_bounds(header_selector).expect(header_selector);
        let row = cx.debug_bounds(row_selector).expect(row_selector);
        assert!(
            (f32::from(header.origin.x) - f32::from(row.origin.x)).abs() < 0.5,
            "header and row must start at the same x: {header:?} vs {row:?}"
        );
        assert!(
            (f32::from(header.size.width) - f32::from(row.size.width)).abs() < 0.5,
            "header and row must have the same width: {header:?} vs {row:?}"
        );
        assert!(
            header.size.width >= gpui::px(minimum_width),
            "column must keep its readable minimum width: {header:?}"
        );
    }
}
