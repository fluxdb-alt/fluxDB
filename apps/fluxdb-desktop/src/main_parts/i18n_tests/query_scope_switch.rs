/// 查询工具栏作用域下拉：Postgres 同时显示数据库与 schema，schema 可切回「默认」。
///
/// 只验证生产视图里的真实点击/键盘事件链路；数据库切换本身由 app 层状态测试覆盖，
/// 这里不依赖真实 PG 连接。
#[gpui::test]
fn query_toolbar_switches_postgres_schema_via_dropdown(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent, QueryEditorState, TabKind};
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, Settings};

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
        name: "Postgres scope".into(),
        kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 5432,
            database: Some("app".into()),
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
        database: Some("app".into()),
        schema: Some("testA".into()),
    }) else {
        panic!("query tab");
    };
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

    // 数据库与 schema 两个下拉都真实出现在工具栏上。
    let database = cx
        .debug_bounds("query-database-select")
        .expect("database select");
    let schema = cx
        .debug_bounds("query-schema-select")
        .expect("schema select");
    assert!(database.center().x < schema.center().x);

    // 选中项初始为 testA；打开菜单后向上移到「默认」并回车确认。
    cx.simulate_click(schema.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("up enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();

    let editor = cx.update(|_, cx| {
        view.read(cx)
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => Some(editor.clone()),
                _ => None,
            })
    });
    let editor: QueryEditorState = editor.expect("query editor");
    assert_eq!(editor.database.as_deref(), Some("app"));
    assert_eq!(editor.schema, None);
}
