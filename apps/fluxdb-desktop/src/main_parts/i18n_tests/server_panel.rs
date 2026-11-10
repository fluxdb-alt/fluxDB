// 服务器面板：真实生产视图渲染 + 会话选择 + 终止确认 + 锁/状态页签。
//
// 不连接真实数据库：活动快照直接注入，验证界面装配、确认门槛与只读禁用逻辑。

#[gpui::test]
fn server_panel_renders_sessions_locks_and_confirm_flow(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent, TabKind};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ServerActivity, ServerLockWait,
        ServerMetric, ServerSession, ServerStatus,
    };

    use crate::ServerPanelTab;

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let mut controller = AppController::new();
    let mysql_config = |id: u64, name: &str, read_only: bool| ConnectionConfig {
        id: ConnectionId(id),
        name: name.into(),
        kind: DatabaseKind::MySql,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 3306,
            database: Some("demo".into()),
        },
        options: read_only
            .then(|| ("read_only".to_string(), "true".to_string()))
            .into_iter()
            .collect(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    };
    controller.dispatch(AppCommand::ReplaceConnections(vec![
        mysql_config(41, "server panel", false),
        mysql_config(42, "server panel read only", true),
    ]));
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
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
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

    let tab_id = cx.update(|window, cx| {
        let tab_id = view.update(cx, |this, cx| {
            let tab_id = match this.dispatch(AppCommand::OpenServerPanel(ConnectionId(41)), cx) {
                AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => tab_id,
                other => panic!("unexpected open event: {other:?}"),
            };
            let activity = ServerActivity {
                version: "8.0.36".into(),
                current_user: "app@localhost".into(),
                sessions: vec![
                    ServerSession {
                        id: "11".into(),
                        user: "app".into(),
                        host: "127.0.0.1:50001".into(),
                        database: "demo".into(),
                        command: "Sleep".into(),
                        elapsed_secs: 12,
                        wait_event: String::new(),
                        sql: String::new(),
                        is_self: true,
                    },
                    ServerSession {
                        id: "22".into(),
                        user: "other".into(),
                        host: "10.0.0.8:50002".into(),
                        database: "demo".into(),
                        command: "Query".into(),
                        elapsed_secs: 30,
                        wait_event: "updating".into(),
                        sql: "update orders set status = 'paid' where id = 1".into(),
                        is_self: false,
                    },
                ],
                locks: vec![ServerLockWait {
                    waiting_session: "22".into(),
                    blocking_session: "11".into(),
                    object: "demo.orders".into(),
                    lock_type: "RECORD".into(),
                    wait_secs: 18,
                    state: "waiting".into(),
                    sql: "update orders set status = 'paid' where id = 1".into(),
                }],
                locks_error: None,
                status: ServerStatus {
                    uptime_secs: Some(3600),
                    metrics: vec![ServerMetric {
                        key: "Threads_connected".into(),
                        value: "2".into(),
                        warning: false,
                    }],
                },
                status_error: None,
            };
            this.dispatch(
                AppCommand::FinishServerPanelLoad {
                    tab_id,
                    result: Ok(activity),
                },
                cx,
            );
            tab_id
        });
        window.draw(cx).clear(cx);
        tab_id
    });

    cx.update(|_, cx| {
        let state = view.read(cx).controller.state();
        let active = state.active_tab().map(|tab| tab.id);
        assert_eq!(active, Some(tab_id), "server panel must stay active");
        assert!(
            matches!(
                state.active_tab().map(|tab| &tab.kind),
                Some(TabKind::ServerPanel(_))
            ),
            "active tab must be the server panel"
        );
    });
    assert!(
        cx.debug_bounds("server-panel").is_some(),
        "panel root must render"
    );
    assert!(
        cx.debug_bounds("server-panel-refresh").is_some(),
        "panel toolbar must render"
    );
    assert!(
        cx.debug_bounds("server-panel-sessions").is_some(),
        "sessions table must render"
    );
    let row_id: &'static str =
        Box::leak(format!("server-session-row-{}-22", tab_id.0).into_boxed_str());
    let row = cx
        .debug_bounds(row_id)
        .expect("waiting session row");
    cx.run_until_parked();
    cx.simulate_click(
        gpui::point(row.origin.x + px(24.), row.center().y),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(
        cx.debug_bounds("server-panel-detail").is_some(),
        "selected session must open the inspector"
    );

    // 终止会话必须先勾选确认；未勾选时确认按钮不关闭弹框，也不发命令。
    let kill = cx
        .debug_bounds("server-panel-kill-session")
        .expect("kill session action");
    cx.simulate_click(kill.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("server-panel-kill-modal").is_some(),
        "kill confirmation must open"
    );
    let confirm = cx
        .debug_bounds("server-dialog-button-Terminate")
        .expect("terminate button");
    cx.simulate_click(confirm.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("server-panel-kill-modal").is_some(),
        "unchecked confirmation must not issue the kill"
    );
    let ack = cx
        .debug_bounds("server-panel-kill-ack")
        .expect("ack checkbox");
    cx.simulate_click(ack.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let cancel = cx
        .debug_bounds("server-dialog-button-Cancel")
        .expect("cancel button");
    cx.simulate_click(cancel.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("server-panel-kill-modal").is_none(),
        "cancel must close the confirmation"
    );

    // 锁等待页签保留等待边；只读/自身会话保护不影响查看。
    let locks_tab_id: &'static str =
        Box::leak(format!("server-panel-subtab-{}-1", tab_id.0).into_boxed_str());
    let locks_tab = cx
        .debug_bounds(locks_tab_id)
        .expect("lock tab");
    cx.simulate_click(locks_tab.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("server-panel-locks").is_some(),
        "lock waits table must render"
    );

    // 状态页签的维护入口需要二次确认。
    let status_tab_id: &'static str =
        Box::leak(format!("server-panel-subtab-{}-2", tab_id.0).into_boxed_str());
    let status_tab = cx
        .debug_bounds(status_tab_id)
        .expect("status tab");
    cx.simulate_click(status_tab.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("server-panel-status").is_some(),
        "status metrics must render"
    );
    let optimize_id: &'static str = Box::leak(
        format!("server-panel-maintenance-{}-optimize", tab_id.0).into_boxed_str(),
    );
    let optimize = cx
        .debug_bounds(optimize_id)
        .expect("MySQL maintenance action");
    cx.simulate_click(optimize.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("server-panel-maintenance-modal").is_some(),
        "maintenance confirmation must open"
    );

    // 只读连接：终止入口置灰，点击也不会打开确认框。
    let read_only_tab = cx.update(|window, cx| {
        let tab_id = view.update(cx, |this, cx| {
            let tab_id = match this.dispatch(AppCommand::OpenServerPanel(ConnectionId(42)), cx) {
                AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => tab_id,
                other => panic!("unexpected open event: {other:?}"),
            };
            this.dispatch(
                AppCommand::FinishServerPanelLoad {
                    tab_id,
                    result: Ok(ServerActivity {
                        version: "8.0.36".into(),
                        current_user: "app@localhost".into(),
                        sessions: vec![ServerSession {
                            id: "31".into(),
                            user: "app".into(),
                            host: "127.0.0.1:50003".into(),
                            database: "demo".into(),
                            command: "Query".into(),
                            elapsed_secs: 5,
                            wait_event: String::new(),
                            sql: "select 1".into(),
                            is_self: false,
                        }],
                        ..ServerActivity::default()
                    }),
                },
                cx,
            );
            tab_id
        });
        window.draw(cx).clear(cx);
        tab_id
    });
    assert!(
        cx.debug_bounds("server-panel-read-only").is_some(),
        "read-only badge must render"
    );
    let ro_row_id: &'static str =
        Box::leak(format!("server-session-row-{}-31", read_only_tab.0).into_boxed_str());
    let ro_row = cx.debug_bounds(ro_row_id).expect("read-only session row");
    cx.simulate_click(
        gpui::point(ro_row.origin.x + px(24.), ro_row.center().y),
        Modifiers::default(),
    );
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let ro_kill = cx
        .debug_bounds("server-panel-kill-session")
        .expect("disabled kill entry");
    cx.simulate_click(ro_kill.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds("server-panel-kill-modal").is_none(),
        "read-only connection must not open kill confirmation"
    );

    // 面板状态仍由 app 层持有：页签类型与活动页签未被 UI 复制。
    let state = cx.update(|_, cx| view.read(cx).controller.state().clone());
    assert!(state
        .tabs
        .iter()
        .any(|tab| tab.id == tab_id && matches!(&tab.kind, TabKind::ServerPanel(panel) if panel.active_tab == ServerPanelTab::Status)));
}
