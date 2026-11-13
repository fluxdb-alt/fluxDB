/// Tab 切换：工作区顺序循环、可搜索切换器、键盘选择和滚动跟随。
#[gpui::test]
fn tab_switching_cycles_and_picks_a_visible_tab(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent};
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};
    use gpui::{Focusable as _, InputEvent as _, KeyBinding, KeyUpEvent, Keystroke};

    use crate::{
        NextTab, OpenTabSwitcher, PreviousTab, TabSwitcherCancel, TabSwitcherConfirm,
        TabSwitcherNext, TabSwitcherPrevious,
    };

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
        name: "tabs-sqlite".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("tabs.sqlite"),
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

    let mut tab_ids = Vec::new();
    for _ in 0..10 {
        let AppEvent::TabOpened(tab_id) =
            controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
                connection_id: ConnectionId(1),
                database: Some("main".into()),
                schema: None,
            })
        else {
            panic!("query tab");
        };
        tab_ids.push(tab_id);
    }

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

    let switcher_shortcut = if cfg!(target_os = "macos") {
        "cmd-shift-e"
    } else {
        "ctrl-shift-e"
    };
    cx.update(|_, cx| {
        cx.bind_keys([
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
            KeyBinding::new(switcher_shortcut, OpenTabSwitcher, None),
            KeyBinding::new("up", TabSwitcherPrevious, Some("TabSwitcher")),
            KeyBinding::new("down", TabSwitcherNext, Some("TabSwitcher")),
            KeyBinding::new("enter", TabSwitcherConfirm, Some("TabSwitcher")),
            KeyBinding::new("escape", TabSwitcherCancel, Some("TabSwitcher")),
        ]);
    });
    cx.update(|window, cx| {
        view.update(cx, |app, cx| app.focus_handle.focus(window, cx));
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    let active_tab = |cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).controller.state().active_tab)
    };
    assert_eq!(active_tab(cx), Some(*tab_ids.last().unwrap()));

    cx.simulate_keystrokes("ctrl-tab");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("tab-switcher").is_some());
    assert_eq!(
        cx.update(|_, cx| view.read(cx).tab_switcher_selected),
        0
    );
    // 松开 Ctrl 才提交，和 VS Code 的 quick navigate 一致。
    cx.update(|window, cx| {
        window.dispatch_event(
            gpui::ModifiersChangedEvent {
                modifiers: gpui::Modifiers::default(),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(active_tab(cx), Some(tab_ids[0]));

    cx.simulate_keystrokes("ctrl-shift-tab");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    let (open, selected) = cx.update(|_, cx| {
        let app = view.read(cx);
        (app.tab_switcher.is_some(), app.tab_switcher_selected)
    });
    assert!(open, "Ctrl+Shift+Tab must open the switcher");
    assert_eq!(selected, 9);
    cx.update(|window, cx| {
        window.dispatch_event(
            KeyUpEvent {
                keystroke: Keystroke::parse("control").expect("control key"),
            }
            .to_platform_input(),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(active_tab(cx), Some(*tab_ids.last().unwrap()));

    cx.simulate_keystrokes(switcher_shortcut);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("tab-switcher").is_some());
    let (selected, offset, max_offset, children) = cx.update(|_, cx| {
        let app = view.read(cx);
        (
            app.tab_switcher_selected,
            app.tab_switcher_scroll.offset(),
            app.tab_switcher_scroll.max_offset(),
            app.tab_switcher_scroll.children_count(),
        )
    });
    assert_eq!(selected, 9);
    assert!(
        max_offset.y > gpui::px(0.),
        "tab switcher must have scroll headroom: offset={offset:?}, max={max_offset:?}, children={children}"
    );
    assert_ne!(
        offset.y,
        gpui::px(0.),
        "opening the switcher on the last tab must scroll it into view"
    );

    // 从最后一项向上移动 5 项，确认后应打开第 5 个标签页。
    cx.simulate_keystrokes("up up up up up");
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert_eq!(active_tab(cx), Some(tab_ids[4]));
    assert!(cx.debug_bounds("tab-switcher").is_none());

    // 焦点在输入框里时也必须能触发应用级 Tab 切换，而不是被输入框吞掉。
    cx.update(|window, cx| {
        view.update(cx, |app, cx| {
            app.sidebar_search_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        });
    });
    cx.simulate_keystrokes("ctrl-tab");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.dispatch_event(
            KeyUpEvent {
                keystroke: Keystroke::parse("control").expect("control key"),
            }
            .to_platform_input(),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(active_tab(cx), Some(tab_ids[5]));
}
