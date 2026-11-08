// 结构对比/数据传输：真实生产视图中右键连接 → 打开菜单 → 打开弹框 → 执行对比。
// 对比与传输的数据库行为由 fluxdb-app 的 SQLite 往返测试覆盖。

fn compare_transfer_redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// 在 SQLite 连接上执行建表语句，为真实视图准备结构差异数据。
fn compare_transfer_execute_sql(config: &fluxdb_core::ConnectionConfig, sql: &str) {
    use fluxdb_connectors::connector_for;
    use fluxdb_core::{QueryExecutionOptions, QueryMode, QueryRequest};

    let result = connector_for(config)
        .expect("sqlite connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: Some("main".into()),
            schema: None,
            session_id: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .expect("sqlite execute");
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "SQL 执行失败: {sql}"
    );
}

#[gpui::test]
fn compare_transfer_opens_from_connection_menu_and_compares(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    let connection = |id: u64, name: &str, file: &str| ConnectionConfig {
        id: ConnectionId(id),
        name: name.into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join(file),
            read_only: false,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    };
    let source = connection(1, "compare source", "source.db");
    let target = connection(2, "compare target", "target.db");
    // 源/目标同一张表但列不同，确保弹框出现可选项和两侧 DDL；
    // 列数刻意写多，让 DDL 高度超过预览面板，覆盖滚动路径。
    let columns: String = (1..=40)
        .map(|index| format!(",\n  c{index:02} TEXT NOT NULL DEFAULT ''"))
        .collect();
    compare_transfer_execute_sql(
        &source,
        &format!(
            "CREATE TABLE cmp_orders (\n  id INTEGER PRIMARY KEY,\n  customer_id INTEGER NOT NULL,\n  amount REAL NOT NULL,\n  status TEXT NOT NULL,\n  note TEXT,\n  created_at TEXT NOT NULL,\n  updated_at TEXT NOT NULL{columns}\n)"
        ),
    );
    compare_transfer_execute_sql(
        &target,
        &format!(
            "CREATE TABLE cmp_orders (\n  id INTEGER PRIMARY KEY,\n  customer_id INTEGER NOT NULL,\n  amount REAL NOT NULL,\n  note TEXT NOT NULL,\n  created_at TEXT NOT NULL,\n  updated_at TEXT NOT NULL{columns}\n)"
        ),
    );
    controller.dispatch(AppCommand::ReplaceConnections(vec![source, target]));
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
    compare_transfer_redraw(cx);

    // 右键连接行，打开连接菜单，再点「结构对比与同步」。
    let row = cx.debug_bounds("connection-row:1").expect("connection row");
    cx.simulate_event(gpui::MouseDownEvent {
        position: row.center(),
        modifiers: Modifiers::default(),
        button: gpui::MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    compare_transfer_redraw(cx);
    let entry = cx
        .debug_bounds("connection-menu-item:Structure compare & sync")
        .expect("compare entry in connection menu");
    cx.simulate_click(entry.center(), Modifiers::default());
    compare_transfer_redraw(cx);

    assert!(
        cx.debug_bounds("compare-transfer-modal").is_some(),
        "modal must open from the real connection menu"
    );
    assert!(cx.debug_bounds("compare-transfer-tab:structure").is_some());
    assert!(cx.debug_bounds("compare-transfer-tab:data").is_some());

    // 下拉候选项来自真实对象树：SQLite 的 main 库应被读回并选中。
    cx.run_until_parked();
    let database = cx.update(|_, cx| {
        view.read(cx)
            .compare_transfer_inputs
            .source_database
            .read(cx)
            .selected_value()
            .cloned()
    });
    assert_eq!(database.as_deref(), Some("main"));
    // 目标连接也走下拉：默认选中第一个可用的非源连接。
    let target = cx.update(|_, cx| {
        view.read(cx)
            .compare_transfer_inputs
            .target_connection
            .read(cx)
            .selected_value()
            .cloned()
    });
    assert_eq!(target.as_deref(), Some("compare target"));
    assert!(
        cx.debug_bounds("compare-transfer-results").is_none(),
        "results appear only after compare runs"
    );

    // 点击「开始对比」，后台读取两端元数据后应渲染结果区域。
    let compare = cx
        .debug_bounds("compare-transfer-compare")
        .expect("compare button");
    cx.simulate_click(compare.center(), Modifiers::default());
    cx.run_until_parked();
    compare_transfer_redraw(cx);
    assert!(
        cx.debug_bounds("compare-transfer-results").is_some(),
        "compare result area must render after the background read"
    );
    // 左栏表单 + 右侧结果：主操作按钮整体位于结果区左侧，确认新布局生效。
    let compare_button = cx
        .debug_bounds("compare-transfer-compare")
        .expect("compare button after compare");
    let results = cx
        .debug_bounds("compare-transfer-results")
        .expect("results bounds");
    assert!(
        compare_button.origin.x + compare_button.size.width <= results.origin.x,
        "compare button should sit in the left rail: button={:?} results={:?}",
        compare_button,
        results
    );

    // 只有「结构不同」且未勾「重建结构不同表」：同步没有可执行动作，按钮必须禁用。
    let sync = cx
        .debug_bounds("compare-transfer-sync")
        .expect("sync button");
    cx.simulate_click(sync.center(), Modifiers::default());
    cx.run_until_parked();
    let sync_running = cx.update(|_, cx| {
        view.read(cx)
            .compare_transfer
            .as_ref()
            .expect("compare state")
            .sync_running
    });
    assert!(
        !sync_running,
        "sync must stay disabled when no selected table is covered by enabled actions"
    );

    // 选中差异表后显示两侧 DDL；DDL 超出面板高度时必须能滚动。
    let row = cx
        .debug_bounds("compare-transfer-row:cmp_orders")
        .expect("diff row for cmp_orders");
    cx.simulate_click(row.center(), Modifiers::default());
    compare_transfer_redraw(cx);
    let pane = cx
        .debug_bounds("compare-transfer-ddl-pane")
        .expect("ddl pane");
    let before = cx
        .debug_bounds("compare-transfer-ddl-content")
        .expect("ddl content");
    assert!(
        before.size.height > pane.size.height,
        "DDL 内容应超出面板高度: content={:?} pane={:?}",
        before.size,
        pane.size
    );
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: pane.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-80.))),
        ..Default::default()
    });
    compare_transfer_redraw(cx);
    let after = cx
        .debug_bounds("compare-transfer-ddl-content")
        .expect("ddl content after scroll");
    assert!(
        after.origin.y < before.origin.y,
        "DDL 区域应响应滚轮: before={:?} after={:?}",
        before.origin,
        after.origin
    );

    // 切到数据传输页：左栏字段更多，主按钮仍须完整落在栏内（不被裁掉）。
    let data_tab = cx
        .debug_bounds("compare-transfer-tab:data")
        .expect("data tab");
    cx.simulate_click(data_tab.center(), Modifiers::default());
    compare_transfer_redraw(cx);
    let rail = cx.debug_bounds("compare-transfer-rail").expect("left rail");
    let start = cx
        .debug_bounds("compare-transfer-start")
        .expect("start transfer button");
    assert!(
        start.origin.y + start.size.height <= rail.origin.y + rail.size.height,
        "传输按钮应完整落在左栏内: start={:?} rail={:?}",
        start,
        rail
    );
    assert!(
        start.origin.x >= rail.origin.x,
        "传输按钮应位于左栏内: start={:?} rail={:?}",
        start,
        rail
    );
}
