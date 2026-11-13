/// 统一命令面板：快捷键打开、scope 切换、搜索已保存查询，并把结果真正打开成标签页。
#[gpui::test]
fn unified_command_palette_opens_and_runs_saved_query(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent, TabKind};
    use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, SavedQuery};
    use gpui::KeyBinding;

    use crate::{
        CommandPalette, CommandPaletteCancel, CommandPaletteConfirm, CommandPaletteNext,
        CommandPaletteNextScope, CommandPalettePrevious, CommandPalettePreviousScope,
        CommandPaletteScope, SettingsPanelSection,
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
        name: "palette-sqlite".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("palette.sqlite"),
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
    let AppEvent::TabOpened(_) =
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
    cx.update(|_, cx| {
        view.update(cx, |app, _| {
            app.saved_queries.push(SavedQuery {
                id: 7,
                connection_id: ConnectionId(1),
                database: Some("main".into()),
                schema: None,
                name: "Palette Target Query".into(),
                text: "SELECT 1;".into(),
            });
            for index in 0..12 {
                app.saved_queries.push(SavedQuery {
                    id: 100 + index,
                    connection_id: ConnectionId(1),
                    database: Some("main".into()),
                    schema: None,
                    name: format!("Palette Filler Query {index}"),
                    text: format!("SELECT {index};"),
                });
            }
        });
    });

    let shortcut = if cfg!(target_os = "macos") {
        "cmd-shift-p"
    } else {
        "ctrl-shift-p"
    };
    cx.update(|_, cx| {
        cx.bind_keys([
            KeyBinding::new(shortcut, CommandPalette, None),
            KeyBinding::new("tab", CommandPaletteNextScope, Some("CommandPalette")),
            KeyBinding::new(
                "shift-tab",
                CommandPalettePreviousScope,
                Some("CommandPalette"),
            ),
            KeyBinding::new("down", CommandPaletteNext, Some("CommandPalette")),
            KeyBinding::new("up", CommandPalettePrevious, Some("CommandPalette")),
            KeyBinding::new("enter", CommandPaletteConfirm, Some("CommandPalette")),
            KeyBinding::new("escape", CommandPaletteCancel, Some("CommandPalette")),
        ]);
    });

    cx.update(|window, cx| {
        view.update(cx, |app, cx| app.focus_handle.focus(window, cx));
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(shortcut);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("command-palette").is_some());

    cx.simulate_keystrokes("down");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| view.read(cx).command_palette.selected),
        1
    );
    cx.simulate_keystrokes("up");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| view.read(cx).command_palette.selected),
        0
    );

    // 选中项越过列表可视区后，滚动容器必须跟随。
    cx.simulate_keystrokes("down down down down down down down down down down");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    let offset = cx.update(|_, cx| view.read(cx).command_palette.scroll.offset());
    assert_ne!(
        offset.y,
        gpui::px(0.),
        "keyboard selection must scroll the palette list"
    );

    cx.simulate_keystrokes("tab");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| view.read(cx).command_palette.scope),
        CommandPaletteScope::Commands
    );
    cx.simulate_keystrokes("shift-tab");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| view.read(cx).command_palette.scope),
        CommandPaletteScope::All
    );

    cx.simulate_keystrokes("t a r g e t");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("command-palette-row:queries:0").is_some());
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("command-palette").is_none());

    let active = cx.update(|_, cx| {
        view.read(cx)
            .controller
            .state()
            .active_tab()
            .map(|tab| (tab.title.clone(), matches!(tab.kind, TabKind::QueryEditor(_))))
    });
    assert_eq!(
        active,
        Some(("Palette Target Query".into(), true)),
        "the selected saved query must open as a query tab"
    );

    cx.update(|window, cx| {
        view.update(cx, |app, cx| app.focus_handle.focus(window, cx));
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(shortcut);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("command-palette").is_some());
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("command-palette").is_none());

    // Esc 关闭后焦点回到主视图，下一次全局快捷键仍能打开。
    cx.simulate_keystrokes(shortcut);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("command-palette").is_some());
    cx.simulate_keystrokes("escape");

    cx.update(|_, cx| {
        view.update(cx, |app, cx| {
            app.dispatch(AppCommand::OpenSettings, cx);
            app.settings_panel_section = SettingsPanelSection::Shortcuts;
            cx.notify();
        });
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("settings-shortcut:app.command_palette").is_some(),
        "the command palette shortcut must be listed in the application shortcut group"
    );
}
