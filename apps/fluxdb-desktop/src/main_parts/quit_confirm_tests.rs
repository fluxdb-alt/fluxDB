mod quit_confirm_tests {
    use fluxdb_app::AppController;
    use fluxdb_core::Settings;
    use fluxdb_storage::FileStorage;
    use gpui::{AppContext as _, TestAppContext};
    use gpui_component::Root;

    use crate::{NavicatMain, app_assets_base_path, extend_component_i18n, init_i18n, navicat_main_new};

    /// 退出请求必须穿过 `Root` 找到业务视图，再依据未保存状态弹确认框。
    ///
    /// 回归点：窗口根视图是 `Root`，直接按 `NavicatMain` downcast 会失败并静默退出，
    /// 导致未保存内容随退出丢失。
    #[gpui::test]
    fn unsaved_query_tab_shows_quit_confirm(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().expect("isolated profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );

        let mut controller = AppController::new();
        controller.dispatch(fluxdb_app::AppCommand::SaveSettings(Settings {
            enable_completion_index: false,
            ..Default::default()
        }));
        controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(vec![
            fluxdb_core::ConnectionConfig {
                id: fluxdb_core::ConnectionId(1),
                name: "workspace".into(),
                kind: fluxdb_core::DatabaseKind::Sqlite,
                endpoint: fluxdb_core::Endpoint::SqliteFile {
                    path: temp.path().join("db.sqlite"),
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
            },
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
        cx.run_until_parked();
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
            view.update(cx, |app, _| app.ai_chat.recovery_pending = false);
        });

        cx.update(|_, cx| {
            view.update(cx, |app, cx| {
                app.dispatch(
                    fluxdb_app::AppCommand::OpenQueryEditorInDatabase {
                        connection_id: fluxdb_core::ConnectionId(1),
                        database: None,
                        schema: None,
                    },
                    cx,
                );
                let tab_id = app
                    .controller
                    .state()
                    .tabs
                    .iter()
                    .find(|tab| matches!(tab.kind, fluxdb_app::TabKind::QueryEditor(_)))
                    .expect("query tab")
                    .id;
                app.dispatch(
                    fluxdb_app::AppCommand::UpdateQueryText {
                        tab_id,
                        pane_id: fluxdb_app::QueryPaneId(1),
                        text: "select 1".into(),
                    },
                    cx,
                );
            });
        });

        cx.update(|_, cx| crate::request_app_quit(cx));

        cx.update(|_, cx| {
            assert!(
                view.read(cx).pending_exit_confirm,
                "有未保存 SQL 时退出应弹确认框，而不是直接退出"
            );
        });
    }
}
