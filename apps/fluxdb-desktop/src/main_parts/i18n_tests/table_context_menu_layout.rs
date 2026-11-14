#[gpui::test]
fn table_context_menu_fits_at_bottom_of_viewport(cx: &mut TestAppContext) {
    use crate::TableContextMenu;
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
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
    let connection_id = ConnectionId(1);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "context menu layout".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("menu.sqlite"),
            read_only: false,
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
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(620.)));
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
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.show_table_context_menu(
                TableContextMenu {
                    object_path: ObjectPath {
                        connection_id,
                        database: None,
                        schema: None,
                        name: "ITEMS".into(),
                        kind: ObjectKind::Table,
                    },
                    position: gpui::point(gpui::px(100.), gpui::px(600.)),
                    submenu: None,
                },
                window,
                cx,
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let menu = cx
        .debug_bounds("table-context-menu")
        .expect("table context menu");
    let viewport_bottom = gpui::px(620.) - gpui::px(8.);
    assert!(
        menu.origin.y + menu.size.height <= viewport_bottom,
        "table context menu must keep its destructive actions inside the viewport"
    );
}

#[gpui::test]
fn table_import_dialog_renders_with_disabled_create_option(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
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
    let connection_id = ConnectionId(1);
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: connection_id,
        name: "import dialog".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("import.sqlite"),
            read_only: false,
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
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(620.)));
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
    let _ = view.update(cx, |this, cx| {
        // 直接构造导入表单，验证对话框在真实生产视图里可渲染且默认追加。
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: None,
            schema: None,
            name: "items".into(),
            kind: ObjectKind::Table,
        };
        // 新流程：先弹框，文件在框内选择；此处验证未选文件时的初始状态。
        let form = crate::TableDataImportForm::new(object);
        assert_eq!(form.options.mode, fluxdb_core::TableImportMode::Append);
        assert_eq!(form.target, crate::TableDataImportTarget::Existing);
        assert!(form.source.is_none());
        assert!(form.format.is_none());
        // 与导出弹框一致：默认常规页，日志页初始为空且未开始。
        assert_eq!(form.tab, crate::ImportDialogTab::General);
        assert!(form.logs.is_empty());
        assert!(form.started_at.is_none());
        this.pending_data_import = Some(form);
        cx.notify();
    });
    // 触发一次真实重绘：对话框若存在布局或状态错误会在这里暴露。
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
fn table_import_failure_log_shows_reason_once(cx: &mut TestAppContext) {
    use fluxdb_app::AppCommand;
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
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
    controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
        id: ConnectionId(1),
        name: "import failure".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: temp.path().join("failure.sqlite"),
            read_only: false,
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
    cx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(620.)));
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
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            let mut form = crate::TableDataImportForm::new(ObjectPath {
                connection_id: ConnectionId(1),
                database: None,
                schema: None,
                name: "items".into(),
                kind: ObjectKind::Table,
            });
            form.tab = crate::ImportDialogTab::Log;
            form.error = Some("SQL Server 驱动错误：测试失败".into());
            form.logs.push(crate::ImportDialogLogEntry {
                stage: crate::tr("app.data.data_import.stage_failed").to_string(),
                message: "SQL Server 驱动错误：测试失败".into(),
                success: false,
                elapsed_ms: 1,
            });
            this.pending_data_import = Some(form);
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("data-import-notice").is_none(),
        "failure already rendered in the stage log must not repeat at the top"
    );
    cx.update(|_, cx| {
        let form = view.read(cx).pending_data_import.as_ref().expect("import form");
        assert_eq!(form.logs.len(), 1);
        assert_eq!(form.logs[0].message, "SQL Server 驱动错误：测试失败");
        assert!(!form.logs[0].success);
    });
}

#[gpui::test]
fn long_column_comment_wraps_inside_header_tooltip(cx: &mut TestAppContext) {
    use crate::{DataPageTableDelegate, NavicatMain, TabId};
    use fluxdb_core::{CellValue, Column, DataPage, Row};
    use gpui_component::table::{DataTable, TableState};

    struct TooltipHarness {
        _app: gpui::Entity<NavicatMain>,
        table: gpui::Entity<TableState<DataPageTableDelegate>>,
    }

    impl gpui::Render for TooltipHarness {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let _ = &self._app;
            gpui::div()
                .size_full()
                .child(DataTable::new(&self.table))
        }
    }

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    cx.update(|cx| {
        extend_component_i18n();
        gpui_component::init(cx);
    });
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let app = cx.new(|cx| {
            navicat_main_new(
                window,
                cx,
                AppController::new(),
                storage,
                gpui_component::ThemeMode::Light,
            )
        });
        let page = std::sync::Arc::new(DataPage {
            columns: vec![Column {
                name: "mom_kwh".into(),
                type_name: Some("numeric(14,2)".into()),
                nullable: true,
                primary_key: false,
                comment: Some("环比电量（上月环比累计电量 kvh）".into()),
            }],
            rows: vec![Row {
                values: vec![CellValue::Null],
            }],
            offset: 0,
            limit: 1,
            has_more: false,
        incomplete: None,
        });
        let edit_input = app.read(cx).data_cell_edit_input.clone();
        let delegate = DataPageTableDelegate::from_page_with_rule(
            app.downgrade(),
            TabId(1),
            crate::DATA_EDITOR_PANE,
            page,
            None,
            None,
            &Default::default(),
            None,
            None,
            None,
            false,
            None,
            None,
            edit_input.clone(),
            None,
            edit_input,
            None,
            true,
            None,
            std::sync::Arc::new(|_, _, _, _| {}),
        );
        let table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .sortable(false)
                .col_movable(false)
                .col_resizable(true)
                .col_selectable(false)
        });
        TooltipHarness {
            _app: app,
            table,
        }
    });
    cx.simulate_resize(gpui::size(gpui::px(500.), gpui::px(300.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let header = cx
        .debug_bounds("data-table-header-text:mom_kwh")
        .expect("column header");
    cx.simulate_mouse_move(header.center(), None, gpui::Modifiers::none());
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let value = cx
        .debug_bounds("data-table-header-tooltip-value:Comment")
        .expect("column comment tooltip");
    assert!(
        value.size.width <= gpui::px(150.) && value.size.height > gpui::px(20.),
        "long comment must wrap inside the tooltip card: {value:?}"
    );
}
