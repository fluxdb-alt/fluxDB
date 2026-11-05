    #[gpui::test]
    fn sqlserver_table_context_menu_exposes_all_actions_without_unsupported_dialect_options(
        cx: &mut TestAppContext,
    ) {
        use crate::{DangerTableAction, PendingDangerTableAction};
        use fluxdb_app::AppCommand;
        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
            SecretRef, SqlServerAdvancedOptions, SqlServerBasicOptions,
            SqlServerConnectionProfile,
        };

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("isolated profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "en",
        );
        let mut controller = AppController::new();
        let connection_id = ConnectionId(41);
        controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
            id: connection_id,
            name: "SQL Server context menu".into(),
            kind: DatabaseKind::SqlServer,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1433,
                database: Some("fluxdb_smoke".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            mongo_profile: None,
            sqlserver_profile: Some(SqlServerConnectionProfile {
                basic: SqlServerBasicOptions {
                    host: "127.0.0.1".into(),
                    port: 1433,
                    database: "fluxdb_smoke".into(),
                    schema: "dbo".into(),
                    instance: String::new(),
                    username: "sa".into(),
                    password: SecretRef::inline("not-used"),
                },
                advanced: SqlServerAdvancedOptions::default(),
                ssh: Default::default(),
            }),
            oracle_profile: None,
        }]));
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
        cx.simulate_resize(gpui::size(gpui::px(1440.), gpui::px(1000.)));
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
        let object_path = ObjectPath {
            connection_id,
            database: Some("fluxdb_smoke".into()),
            schema: Some("dbo".into()),
            name: "FLUXDB_CONTEXT_ITEMS".into(),
            kind: ObjectKind::Table,
        };
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        // The live SQL Server object tree is exercised by the ignored database test. This
        // headless case covers the production menu surface and its dialect-specific options.
        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.show_table_context_menu(
                    crate::TableContextMenu {
                        object_path: object_path.clone(),
                        position: gpui::point(gpui::px(80.), gpui::px(120.)),
                        submenu: None,
                    },
                    window,
                    cx,
                );
            });
            window.draw(cx).clear(cx);
        });

        for selector in [
            "table-menu-item:Pin",
            "table-menu-item:Copy name",
            "table-menu-item:View data",
            "table-menu-item:Open ER diagram",
            "table-menu-item:Design table",
            "table-menu-item:New table",
            "table-menu-item:Refresh",
            "table-menu-item:Rename",
            "table-menu-item:Copy table",
            "table-menu-item:Copy table structure",
            "table-menu-item:Backup",
            "table-menu-item:Import data",
            "table-menu-item:Manage groups",
            "table-menu-item:Export",
            "table-menu-item:Drop table",
            "table-menu-item:Truncate table",
        ] {
            assert!(
                cx.debug_bounds(selector).is_some(),
                "missing SQL Server table menu item: {selector}"
            );
        }

        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.pending_danger_table_action = Some(PendingDangerTableAction {
                    object_path: object_path.clone(),
                    action: DangerTableAction::Truncate,
                    foreign_key_check: crate::ForeignKeyCheckMode::Default,
                    restart_identity: false,
                    acknowledged: true,
                    error: None,
                });
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        assert!(
            cx.debug_bounds("danger-table-fk-row").is_none(),
            "SQL Server must not expose MySQL's connection-level foreign key toggle"
        );
        assert!(
            cx.debug_bounds("danger-table-restart-identity").is_none(),
            "SQL Server must not expose PostgreSQL's RESTART IDENTITY option"
        );
    }
