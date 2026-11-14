/// 查询分屏：工具栏拆屏、空屏直接关、有内容时确认后再关。
#[gpui::test]
fn query_pane_split_and_close_confirmation(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent};
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
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
    controller.dispatch(AppCommand::SaveSettings(settings));
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "split-sqlite".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("split.sqlite"),
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
    let AppEvent::TabOpened(tab_id) =
        controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
            connection_id: ConnectionId(1),
            database: Some("main".into()),
            schema: None,
        })
    else {
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

    // 工具栏拆成左右两屏，屏头出现编号 1 / 2。
    let split = cx.debug_bounds("query-split-columns").expect("split button");
    cx.simulate_click(split.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let pane_count = cx.update(|_, cx| {
        let state = view.read(cx).controller.state();
        let tab = state.tabs.iter().find(|tab| tab.id == tab_id).expect("tab");
        let fluxdb_app::TabKind::QueryEditor(editor) = &tab.kind else {
            panic!("query editor");
        };
        editor.pane_count()
    });
    assert_eq!(pane_count, 2);
    assert!(cx.debug_bounds("query-pane-header:1-1").is_some());
    assert!(cx.debug_bounds("query-pane-header:1-2").is_some());

    // 新屏是空的：关闭不弹框。
    let close_second = cx
        .debug_bounds("query-pane-close-2")
        .expect("close second pane");
    cx.simulate_click(close_second.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let pending = cx.update(|_, cx| view.read(cx).pending_close_query_pane.is_some());
    assert!(!pending, "空屏直接关闭，不应弹确认");
    assert!(cx.debug_bounds("query-pane-header:1-2").is_none());

    // 再拆一屏并写入 SQL：关闭必须进入确认流程，而不是直接丢文本。
    let split = cx.debug_bounds("query-split-columns").expect("split button");
    cx.simulate_click(split.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let second_pane = cx.update(|_, cx| {
        let state = view.read(cx).controller.state();
        let tab = state.tabs.iter().find(|tab| tab.id == tab_id).expect("tab");
        let fluxdb_app::TabKind::QueryEditor(editor) = &tab.kind else {
            panic!("query editor");
        };
        editor
            .panes
            .iter()
            .map(|pane| pane.id)
            .max()
            .expect("second pane")
    });
    cx.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.dispatch(
                AppCommand::UpdateQueryText {
                    tab_id,
                    pane_id: second_pane,
                    text: "SELECT 2".into(),
                },
                cx,
            );
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let close_selector: &'static str =
        Box::leak(format!("query-pane-close-{}", second_pane.0).into_boxed_str());
    let close_second = cx
        .debug_bounds(close_selector)
        .expect("close second pane");
    cx.simulate_click(close_second.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.update(|_, cx| view.read(cx).pending_close_query_pane.is_some()),
        "有内容的屏关闭前必须确认"
    );
    assert!(cx.debug_bounds("close-query-pane-discard").is_some());
    let discard = cx
        .debug_bounds("close-query-pane-discard")
        .expect("discard button");
    cx.simulate_click(discard.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let pane_count = cx.update(|_, cx| {
        let state = view.read(cx).controller.state();
        let tab = state.tabs.iter().find(|tab| tab.id == tab_id).expect("tab");
        let fluxdb_app::TabKind::QueryEditor(editor) = &tab.kind else {
            panic!("query editor");
        };
        editor.pane_count()
    });
    assert_eq!(pane_count, 1);
}
