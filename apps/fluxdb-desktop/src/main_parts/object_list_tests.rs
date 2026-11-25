// 表概览端到端用例：真实 SQLite 连接 + 生产视图，只走用户可见路径。
//
// 双击「表」分组 → 概览页签出现并加载表 → 双击表行 → 打开数据页。

mod object_list_tests {
    use std::collections::BTreeMap;

    use fluxdb_app::{AppCommand, AppController, TabKind};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, QueryExecutionOptions, QueryMode,
        QueryRequest, Settings,
    };
    use fluxdb_storage::{FileStorage, Storage as _};
    use gpui::{
        AppContext as _, Modifiers, MouseButton, MouseDownEvent, TestAppContext, VisualTestContext,
    };
    use gpui_component::Root;

    use crate::{
        NavicatMain, app_assets_base_path, extend_component_i18n, init_i18n, navicat_main_new,
    };

    fn selector(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    fn wait_until<R>(
        cx: &mut VisualTestContext,
        label: &str,
        mut probe: impl FnMut(&mut VisualTestContext) -> Option<R>,
    ) -> R {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(16));
            cx.run_until_parked();
            if let Some(value) = probe(cx) {
                return value;
            }
            assert!(std::time::Instant::now() < deadline, "等待超时：{label}");
            std::thread::yield_now();
        }
    }

    fn redraw_and_bounds(
        cx: &mut VisualTestContext,
        selector: &'static str,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        cx.debug_bounds(selector)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str, click_count: usize) {
        let bounds = redraw_and_bounds(cx, selector).expect("目标行必须渲染");
        cx.simulate_event(MouseDownEvent {
            position: bounds.center(),
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
    }

    fn seed_sqlite(path: &std::path::Path) -> ConnectionConfig {
        let config = ConnectionConfig {
            id: ConnectionId(7_801),
            name: "Overview SQLite".into(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: path.to_path_buf(),
                read_only: false,
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };
        let connector = fluxdb_connectors::connector_for(&config).expect("sqlite connector");
        for text in [
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, total INTEGER)",
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
            "CREATE VIEW recent_orders AS SELECT * FROM orders",
        ] {
            connector
                .execute(&QueryRequest {
                    connection_id: config.id,
                    database: None,
                    schema: None,
                    text: text.into(),
                    mode: QueryMode::All,
                    options: QueryExecutionOptions::default(),
                    session_id: None,
                })
                .expect("seed statement");
        }
        config
    }

    fn launch_app(
        cx: &mut TestAppContext,
        config: ConnectionConfig,
    ) -> (gpui::Entity<NavicatMain>, &mut VisualTestContext) {
        cx.background_executor.allow_parking();
        let temp = tempfile::tempdir().expect("isolated profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        let settings = Settings {
            locale: "zh-CN".into(),
            show_sidebar: true,
            ..Default::default()
        };
        storage.save_settings(&settings).expect("settings");
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "zh-CN",
        );
        let mut controller = AppController::new();
        controller.set_ai_storage(storage.clone());
        controller.dispatch(AppCommand::SaveSettings(settings));
        controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        cx.update(|cx| {
            extend_component_i18n();
            gpui_component::init(cx);
            crate::register_terminal_shortcuts(cx);
        });
        let profile = storage.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut app = navicat_main_new(
                    window,
                    cx,
                    controller,
                    profile,
                    gpui_component::ThemeMode::Light,
                );
                app.ai_chat.recovery_pending = false;
                app
            });
            Root::new(view, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(1500.), gpui::px(950.)));
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
        (view, cx)
    }

    #[gpui::test]
    fn double_click_tables_group_opens_overview_and_table_row_opens_data(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().expect("temp");
        let db_path = temp.path().join("overview.db");
        let config = seed_sqlite(&db_path);
        let connection_id = config.id;
        let (view, cx) = launch_app(cx, config);

        let connection_row =
            selector(format!("connection-row:{}", connection_id.0));
        click(cx, connection_row, 2);
        wait_until(cx, "连接展开", |cx| {
            redraw_and_bounds(cx, selector("database-row:main".into()))
        });

        let group_row = selector("object-group-row:tables:main".into());
        if redraw_and_bounds(cx, group_row).is_none() {
            click(cx, selector("database-row:main".into()), 1);
            wait_until(cx, "库展开出分组", |cx| redraw_and_bounds(cx, group_row));
        }

        click(cx, group_row, 2);
        wait_until(cx, "表概览加载完成", |cx| {
            let loaded = cx.update(|_, cx| {
                view.read(cx).controller.state().tabs.iter().any(|tab| {
                    matches!(&tab.kind, TabKind::ObjectList(list)
                        if !list.loading
                            && list.error.is_none()
                            && list.objects.len() == 2)
                })
            });
            loaded.then_some(())
        });

        let title = cx.update(|_, cx| {
            view.read(cx)
                .controller
                .state()
                .tabs
                .iter()
                .find_map(|tab| {
                    matches!(tab.kind, TabKind::ObjectList(_))
                        .then(|| crate::localized_tab_title(tab))
                })
        });
        assert_eq!(
            title.as_deref(),
            Some("main · 表"),
            "概览页签标题应显示当前库"
        );

        let orders = selector("object-list-row:orders".into());
        wait_until(cx, "orders 行渲染", |cx| redraw_and_bounds(cx, orders));

        let header = redraw_and_bounds(cx, selector("object-list-header".into()))
            .expect("表头必须渲染");
        let row = redraw_and_bounds(cx, orders).expect("表行必须渲染");
        assert!(
            (f32::from(row.size.width) - f32::from(header.size.width)).abs() < 1.0,
            "表头与行必须同宽：header={:?} row={:?}",
            header.size,
            row.size
        );
        assert!(
            redraw_and_bounds(cx, selector("object-list-row:recent_orders".into())).is_none(),
            "概览只展示数据表，不展示视图"
        );

        click(cx, orders, 2);
        wait_until(cx, "双击表打开数据页", |cx| {
            let opened = cx.update(|_, cx| {
                view.read(cx).controller.state().tabs.iter().any(|tab| {
                    matches!(&tab.kind, TabKind::DataEditor(editor)
                        if editor.object.name == "orders")
                })
            });
            opened.then_some(())
        });
    }
}
