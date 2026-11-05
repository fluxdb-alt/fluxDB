#[gpui::test]
fn sqlserver_security_forms_keep_pending_state_and_password_preview(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent, SqlServerUserAdminTab, TabKind, UserAdminDetailTab};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, SecretRef,
        SqlServerAdvancedOptions, SqlServerBasicOptions, SqlServerConnectionProfile,
        SqlServerDatabaseRole, SqlServerDatabaseUser, SqlServerLogin, SqlServerSecuritySnapshot,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let controller = AppController::new();
    let connection_id = ConnectionId(51);
    let connection_config = ConnectionConfig {
        id: connection_id,
        name: "SQL Server security".into(),
        kind: DatabaseKind::SqlServer,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 1433,
            database: Some("fluxdb_demo".into()),
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
                database: "fluxdb_demo".into(),
                schema: "dbo".into(),
                instance: String::new(),
                username: "sa".into(),
                password: SecretRef::inline("not-used"),
            },
            advanced: SqlServerAdvancedOptions {
                connect_timeout_secs: 1,
                ..SqlServerAdvancedOptions::default()
            },
            ssh: Default::default(),
        }),
        oracle_profile: None,
    };
    let snapshot = SqlServerSecuritySnapshot {
        schemas: vec!["app".into(), "dbo".into()],
        database: "fluxdb_demo".into(),
        current_login: "sa".into(),
        current_user: "dbo".into(),
        logins: vec![SqlServerLogin {
            name: "sa".into(),
            login_type: "SQL_LOGIN".into(),
            is_disabled: false,
            default_database: Some("master".into()),
            default_language: Some("us_english".into()),
            create_date: None,
        }],
        users: vec![SqlServerDatabaseUser {
            name: "dbo".into(),
            user_type: "SQL_USER".into(),
            login_name: Some("sa".into()),
            default_schema: Some("dbo".into()),
            create_date: None,
        }],
        server_roles: vec![fluxdb_core::SqlServerServerRole {
            name: "sysadmin".into(),
            is_fixed: true,
        }],
        database_roles: vec![SqlServerDatabaseRole {
            name: "db_datareader".into(),
            owner: Some("dbo".into()),
            is_fixed: true,
            create_date: None,
        }],
        memberships: Vec::new(),
    };

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
    cx.run_until_parked();
    let tab_id = cx.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.controller
                .dispatch(AppCommand::ReplaceConnections(vec![connection_config]));
            let tab_id = match this
                .controller
                .dispatch(AppCommand::OpenUserAdmin(connection_id))
            {
                AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => tab_id,
                event => panic!("unexpected open event: {event:?}"),
            };
            this.controller
                .dispatch(AppCommand::FinishSqlServerSecurityDatabases {
                    tab_id,
                    result: Ok(vec!["fluxdb_demo".into(), "master".into()]),
                });
            this.controller
                .dispatch(AppCommand::FinishSqlServerSecurityLoad {
                    tab_id,
                    result: Ok(snapshot),
                });
            cx.notify();
            tab_id
        })
    });
    cx.update(|_, cx| {
        let this = view.read(cx);
        let state = this.controller.state();
        assert_eq!(state.active_tab, Some(tab_id));
        assert!(
            state
                .connections
                .iter()
                .any(|connection| connection.config.id == connection_id)
        );
        assert!(state.tabs.iter().any(|tab| {
            tab.id == tab_id
                && matches!(
                    &tab.kind,
                    TabKind::UserAdmin(admin) if admin.sqlserver.is_some()
                )
        }));
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let workspace_bounds = cx
        .debug_bounds("workspace-page")
        .expect("workspace content must be laid out");
    assert!(
        f32::from(workspace_bounds.size.height) > 1.,
        "workspace content must have height: {workspace_bounds:?}"
    );
    assert!(
        cx.debug_bounds("home-new-connection").is_none(),
        "active SQL Server tab must not render the home page"
    );

    for selector in [
        "sqlserver-security-database",
        "sqlserver-security-save",
        "sqlserver-security-refresh",
        "sqlserver-security-delete",
        "sqlserver-security-filter-0",
        "sqlserver-security-filter-1",
        "sqlserver-security-filter-2",
        "sqlserver-security-filter-3",
        "sqlserver-security-tab-general",
        "sqlserver-security-tab-roles",
        "sqlserver-security-tab-permissions",
        "sqlserver-security-tab-sql-preview",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "SQL Server security control must render: {selector}"
        );
    }

    let roles_tab = cx
        .debug_bounds("sqlserver-security-tab-roles")
        .expect("roles tab");
    cx.simulate_click(roles_tab.center(), Modifiers::default());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("sqlserver-server-role-sa-sysadmin").is_some(),
        "the membership tab must render existing server roles"
    );

    let roles_filter = cx
        .debug_bounds("sqlserver-security-filter-3")
        .expect("roles filter");
    cx.simulate_click(roles_filter.center(), Modifiers::default());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let add = cx
        .debug_bounds("user-admin-add-user")
        .expect("add principal");
    cx.simulate_click(add.center(), Modifiers::default());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("sqlserver-security-draft").is_some(),
        "new role must use the shared draft card"
    );
    assert!(
        cx.debug_bounds("sqlserver-security-form-name").is_some(),
        "role draft form must render"
    );
    let name = cx
        .debug_bounds("sqlserver-security-form-name")
        .expect("role name input");
    cx.simulate_click(name.center(), Modifiers::default());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    cx.simulate_input("fluxdb_role_draft");
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("sqlserver-security-form-submit").is_none(),
        "no intermediate staging action"
    );
    let row = cx
        .debug_bounds("sqlserver-security-form-name-row")
        .expect("horizontal form row");
    assert!(
        name.origin.x - row.origin.x >= gpui::px(120.),
        "label and input must be side by side"
    );
    let preview_tab = cx
        .debug_bounds("sqlserver-security-tab-sql-preview")
        .expect("SQL preview tab");
    cx.simulate_click(preview_tab.center(), Modifiers::default());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        let preview = cx.update(|_, cx| {
            view.read(cx)
                .user_admin_state_for(tab_id)
                .and_then(|admin| admin.sqlserver)
                .and_then(|state| state.preview)
        });
        if let Some(preview) = preview {
            assert!(
                preview
                    .iter()
                    .any(|sql| sql == "CREATE ROLE [fluxdb_role_draft];")
            );
            assert!(
                cx.debug_bounds("sqlserver-security-preview-code").is_some(),
                "preview must use the shared highlighted SQL view"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "save must open a SQL preview without a staging step"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    cx.update(|_, cx| {
        let admin = view
            .read(cx)
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::UserAdmin(admin) => Some(admin),
                _ => None,
            })
            .expect("SQL Server security tab");
        let state = admin.sqlserver.as_ref().expect("SQL Server state");
        assert!(state.has_pending_changes());
        assert!(matches!(
            state.pending_changes.first(),
            Some(fluxdb_core::SqlServerSecurityChange::CreateDatabaseRole { name })
                if name == "fluxdb_role_draft"
        ));
    });

    // 登录名、用户与角色都使用同一保存入口；只生成 SQL，不连接服务器或提交权限写入。
    for scenario in ["login", "user"] {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let cancel = cx
            .debug_bounds("sqlserver-security-discard")
            .expect("toolbar cancel");
        cx.simulate_click(cancel.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let filter = cx
            .debug_bounds(if scenario == "user" {
                "sqlserver-security-filter-2"
            } else {
                "sqlserver-security-filter-1"
            })
            .expect("principal filter");
        cx.simulate_click(filter.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let add = cx
            .debug_bounds("user-admin-add-user")
            .expect("new principal");
        cx.simulate_click(add.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let fields = if scenario == "user" {
            vec![("sqlserver-security-form-name", "test_user")]
        } else {
            vec![
                ("sqlserver-security-form-name", "test_login"),
                ("sqlserver-security-form-password", "FluxDB_Test!123"),
                (
                    "sqlserver-security-form-confirm-password",
                    "FluxDB_Test!123",
                ),
            ]
        };
        for (selector, text) in fields {
            let bounds = cx.debug_bounds(selector).expect("form field");
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.simulate_input(text);
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        let selectors = if scenario == "user" {
            vec![
                ("sqlserver-security-form-login", "sa"),
                ("sqlserver-security-form-default", "app"),
            ]
        } else {
            Vec::new()
        };
        for (selector, text) in selectors {
            let bounds = cx.debug_bounds(selector).expect("form dropdown");
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.simulate_input(text);
            cx.run_until_parked();
            cx.simulate_keystrokes("down enter");
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        let preview_tab = cx
            .debug_bounds("sqlserver-security-tab-sql-preview")
            .expect("SQL preview tab");
        cx.simulate_click(preview_tab.center(), Modifiers::default());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            let state = cx
                .update(|_, cx| {
                    view.read(cx)
                        .user_admin_state_for(tab_id)
                        .and_then(|admin| admin.sqlserver)
                })
                .expect("security");
            if let Some(ref preview) = state.preview {
                let sql = preview.join("\n");
                let display_sql = state.preview_for_display().join("\n");
                if scenario == "user" {
                    assert_eq!(
                        sql,
                        "CREATE USER [test_user] FOR LOGIN [sa] WITH DEFAULT_SCHEMA = [app];"
                    );
                } else {
                    assert!(sql.contains("CREATE LOGIN [test_login]"));
                    assert!(sql.contains("DEFAULT_DATABASE = [fluxdb_demo]"));
                    assert!(sql.contains("********"));
                    assert!(display_sql.contains("FluxDB_Test!123"));
                }
                assert!(cx.debug_bounds("sqlserver-security-preview-code").is_some());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{scenario} preview tab did not produce highlighted SQL"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Selecting Members/Permissions must stage the form first, then show that pending principal.
        let roles_tab = cx
            .debug_bounds("sqlserver-security-tab-roles")
            .expect("roles tab");
        cx.simulate_click(roles_tab.center(), Modifiers::default());
        cx.run_until_parked();
        cx.update(|_, cx| {
            let admin = view
                .read(cx)
                .user_admin_state_for(tab_id)
                .expect("admin");
            let state = admin.sqlserver.as_ref().expect("security");
            assert_eq!(state.selected.as_ref().map(|p| p.name.as_str()), Some(if scenario == "user" { "test_user" } else { "test_login" }));
            assert_eq!(admin.active_detail_tab, UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Roles));
            if scenario == "login" {
                assert_eq!(state.form.kind, fluxdb_app::SqlServerSecurityFormKind::CreateLogin);
                assert_eq!(state.form.password, "FluxDB_Test!123");
            }
        });
        if scenario == "login" {
            let general_tab = cx
                .debug_bounds("sqlserver-security-tab-general")
                .expect("general tab");
            cx.simulate_click(general_tab.center(), Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(
                cx.debug_bounds("sqlserver-security-form-password").is_some(),
                "the staged create-login form must keep its fields after switching tabs"
            );
            let roles_tab = cx
                .debug_bounds("sqlserver-security-tab-roles")
                .expect("roles tab");
            cx.simulate_click(roles_tab.center(), Modifiers::default());
            cx.run_until_parked();
        }
        let permissions_tab = cx
            .debug_bounds("sqlserver-security-tab-permissions")
            .expect("permissions tab");
        cx.simulate_click(permissions_tab.center(), Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|_, cx| {
            let admin = view
                .read(cx)
                .user_admin_state_for(tab_id)
                .expect("admin");
            let state = admin.sqlserver.as_ref().expect("security");
            assert_eq!(admin.active_detail_tab, UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Permissions));
            assert!(!state.loading_permissions);
            assert!(state.selected.is_some());
        });
        assert!(
            cx.debug_bounds("sqlserver-security-grant-permission")
                .is_some(),
            "permissions tab must expose grant controls for the pending principal"
        );
        if scenario == "login" {
            let grant = cx
                .debug_bounds("sqlserver-security-grant-permission")
                .expect("grant permission");
            cx.simulate_click(grant.center(), Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let chip = cx
                .debug_bounds("sqlserver-permission-chip-CONNECT SQL")
                .expect("server permission chip");
            cx.simulate_click(chip.center(), Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(
                cx.debug_bounds("sqlserver-security-permission-composer").is_some(),
                "the add-permission composer must stay open for consecutive edits"
            );
            assert!(
                cx.debug_bounds("sqlserver-security-permission-0").is_none(),
                "an unsubmitted permission must not be treated as a staged row"
            );
            let add_permission = cx
                .debug_bounds("sqlserver-permission-add")
                .expect("add to pending");
            cx.simulate_click(add_permission.center(), Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|_, cx| {
                let admin = view.read(cx).user_admin_state_for(tab_id).expect("admin");
                let state = admin.sqlserver.as_ref().expect("security");
                assert_eq!(
                    admin.active_detail_tab,
                    UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Permissions),
                    "adding permission must keep the user on the permissions tab"
                );
                assert_eq!(
                    state.permission_form.kind,
                    fluxdb_app::SqlServerSecurityFormKind::GrantPermission,
                    "composer must remain open after adding"
                );
                assert_eq!(
                    state.form.kind,
                    fluxdb_app::SqlServerSecurityFormKind::CreateLogin,
                    "permission editing must not overwrite the staged login form"
                );
                assert!(state.effective_permissions().iter().any(|permission| {
                    permission.permission == "CONNECT SQL"
                        && permission.target == fluxdb_core::SqlServerPermissionTarget::Server
                }));
            });
            assert!(
                cx.debug_bounds("sqlserver-security-permission-0").is_some(),
                "a newly added permission must appear immediately without switching tabs"
            );
            let collapse = cx
                .debug_bounds("sqlserver-security-grant-permission")
                .expect("collapse permission composer");
            cx.simulate_click(collapse.center(), Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|_, cx| {
                let state = view
                    .read(cx)
                    .user_admin_state_for(tab_id)
                    .and_then(|admin| admin.sqlserver)
                    .expect("security");
                assert_eq!(
                    state.form.kind,
                    fluxdb_app::SqlServerSecurityFormKind::CreateLogin,
                    "collapsing the permission editor must preserve the staged login form"
                );
                assert_eq!(state.form.name, "test_login");
            });
            let general_tab = cx
                .debug_bounds("sqlserver-security-tab-general")
                .expect("general tab");
            cx.simulate_click(general_tab.center(), Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(
                cx.debug_bounds("sqlserver-security-form-name").is_some(),
                "general tab must still render the editable login form"
            );
            let permissions_tab = cx
                .debug_bounds("sqlserver-security-tab-permissions")
                .expect("permissions tab");
            cx.simulate_click(permissions_tab.center(), Modifiers::default());
            cx.run_until_parked();
            let roles_tab = cx
                .debug_bounds("sqlserver-security-tab-roles")
                .expect("roles tab");
            cx.simulate_click(roles_tab.center(), Modifiers::default());
            cx.run_until_parked();
            let permissions_tab = cx
                .debug_bounds("sqlserver-security-tab-permissions")
                .expect("permissions tab");
            cx.simulate_click(permissions_tab.center(), Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|_, cx| {
                let state = view
                    .read(cx)
                    .user_admin_state_for(tab_id)
                    .and_then(|admin| admin.sqlserver)
                    .expect("security");
                let admin = view.read(cx).user_admin_state_for(tab_id).expect("admin");
                assert_eq!(admin.active_detail_tab, UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::Permissions));
                assert!(state.effective_permissions().iter().any(|permission| {
                    permission.permission == "CONNECT SQL"
                        && permission.target == fluxdb_core::SqlServerPermissionTarget::Server
                }));
            });
            assert!(
                cx.debug_bounds("sqlserver-security-grant-permission").is_some(),
                "permissions panel must be visible after switching tabs"
            );
            let effective_len = cx.update(|_, cx| {
                view.read(cx)
                    .user_admin_state_for(tab_id)
                    .and_then(|admin| admin.sqlserver)
                    .map(|state| state.effective_permissions().len())
                    .unwrap_or(0)
            });
            assert!(
                cx.debug_bounds("sqlserver-security-permission-0").is_some(),
                "pending permissions must remain visible after switching tabs; effective rows: {}",
                effective_len
            );
        }

        // The last scenario verifies Save opens the confirmation modal first (same as PG / MySQL / Oracle).
        if scenario != "user" {
            continue;
        }
        let save = cx
            .debug_bounds("sqlserver-security-save")
            .expect("toolbar save");
        cx.simulate_click(save.center(), Modifiers::default());
        // 预览要过连接器渲染，异步完成；等确认框出现。
        let modal_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            if cx.debug_bounds("user-admin-sql-preview-modal").is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < modal_deadline,
                "{scenario} Save must open the SQL confirmation modal"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // 只弹框、不落库。
        let applying_before_confirm = cx.update(|_, cx| {
            view.read(cx)
                .user_admin_state_for(tab_id)
                .and_then(|admin| admin.sqlserver)
                .is_some_and(|state| state.applying)
        });
        assert!(
            !applying_before_confirm,
            "{scenario} Save must not execute before confirmation"
        );
        let execute = cx
            .debug_bounds("user-admin-sql-preview-execute")
            .expect("modal execute button");
        cx.simulate_click(execute.center(), Modifiers::default());
        let execution_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            let (applying, error) = cx.update(|_, cx| {
                let state = view
                    .read(cx)
                    .user_admin_state_for(tab_id)
                    .and_then(|admin| admin.sqlserver)
                    .expect("security");
                (state.applying, state.apply_error.is_some())
            });
            if applying || error {
                break;
            }
            assert!(
                std::time::Instant::now() < execution_deadline,
                "{scenario} confirmed execution did not start"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Wait for the deterministic offline connection failure, then discard before the next scenario.
        let failure_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            let (applying, error) = cx.update(|_, cx| {
                let state = view
                    .read(cx)
                    .user_admin_state_for(tab_id)
                    .and_then(|admin| admin.sqlserver)
                    .expect("security");
                (state.applying, state.apply_error.is_some())
            });
            if !applying && error {
                break;
            }
            assert!(
                std::time::Instant::now() < failure_deadline,
                "{scenario} direct Save did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

#[gpui::test]
fn failed_create_login_rename_refreshes_preview_without_the_old_identity(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent, SqlServerUserAdminTab, TabKind, UserAdminDetailTab};
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, SecretRef,
        SqlServerAdvancedOptions, SqlServerBasicOptions, SqlServerConnectionProfile,
        SqlServerDatabaseRole, SqlServerDatabaseUser, SqlServerLogin, SqlServerSecuritySnapshot,
    };

    let _guard = locale_guard();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );
    let controller = AppController::new();
    let connection_id = ConnectionId(52);
    let connection_config = ConnectionConfig {
        id: connection_id,
        name: "SQL Server rename".into(),
        kind: DatabaseKind::SqlServer,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 1433,
            database: Some("fluxdb_empty".into()),
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
                database: "fluxdb_empty".into(),
                schema: "dbo".into(),
                instance: String::new(),
                username: "sa".into(),
                password: SecretRef::inline("not-used"),
            },
            advanced: SqlServerAdvancedOptions {
                connect_timeout_secs: 1,
                ..SqlServerAdvancedOptions::default()
            },
            ssh: Default::default(),
        }),
        oracle_profile: None,
    };
    let snapshot = SqlServerSecuritySnapshot {
        schemas: vec!["dbo".into()],
        database: "fluxdb_empty".into(),
        current_login: "sa".into(),
        current_user: "dbo".into(),
        logins: vec![SqlServerLogin {
            name: "sa".into(),
            login_type: "SQL_LOGIN".into(),
            is_disabled: false,
            default_database: Some("master".into()),
            default_language: None,
            create_date: None,
        }],
        users: vec![SqlServerDatabaseUser {
            name: "dbo".into(),
            user_type: "SQL_USER".into(),
            login_name: Some("sa".into()),
            default_schema: Some("dbo".into()),
            create_date: None,
        }],
        server_roles: Vec::new(),
        database_roles: vec![SqlServerDatabaseRole {
            name: "db_datareader".into(),
            owner: Some("dbo".into()),
            is_fixed: true,
            create_date: None,
        }],
        memberships: Vec::new(),
    };
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
    cx.run_until_parked();
    let tab_id = cx.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.controller
                .dispatch(AppCommand::ReplaceConnections(vec![connection_config]));
            let tab_id = match this
                .controller
                .dispatch(AppCommand::OpenUserAdmin(connection_id))
            {
                AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => tab_id,
                event => panic!("unexpected open event: {event:?}"),
            };
            this.controller
                .dispatch(AppCommand::FinishSqlServerSecurityDatabases {
                    tab_id,
                    result: Ok(vec!["fluxdb_empty".into(), "master".into()]),
                });
            this.controller
                .dispatch(AppCommand::FinishSqlServerSecurityLoad {
                    tab_id,
                    result: Ok(snapshot),
                });
            cx.notify();
            tab_id
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let add = cx
        .debug_bounds("user-admin-add-user")
        .expect("add login");
    cx.simulate_click(add.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    for (selector, text) in [
        ("sqlserver-security-form-name", "test"),
        ("sqlserver-security-form-password", "Fsw!123"),
        ("sqlserver-security-form-confirm-password", "Fsw!123"),
    ] {
        let bounds = cx.debug_bounds(selector).expect("login form field");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.simulate_input(text);
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    let original_plan = cx.update(|_, cx| {
        view.update(cx, |this, cx| {
            let state = this
                .user_admin_state_for(tab_id)
                .and_then(|admin| admin.sqlserver)
                .expect("state");
            assert!(state.has_form_changes(), "fixture must have a staged login");
            this.controller
                .dispatch(AppCommand::SubmitSqlServerSecurityForm(tab_id));
            let state = this
                .user_admin_state_for(tab_id)
                .and_then(|admin| admin.sqlserver)
                .expect("state");
            let plan = state.build_plan().expect("original plan");
            this.controller.dispatch(AppCommand::FinishSqlServerSecurityApply {
                tab_id,
                principal: plan.principal,
                result: Err(fluxdb_core::UserFacingError::from(fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Query,
                    "login already exists",
                ))),
            });
            cx.notify();
            plan.changes
        })
    });
    assert!(!original_plan.is_empty(), "fixture must stage an original login");
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let name = cx
        .debug_bounds("sqlserver-security-form-name")
        .expect("restored failed login name");
    cx.simulate_click(name.center(), Modifiers::default());
    cx.simulate_keystrokes("backspace backspace backspace backspace");
    cx.simulate_input("test22");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.controller
                .dispatch(AppCommand::SubmitSqlServerSecurityForm(tab_id));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let preview_tab = cx
        .debug_bounds("sqlserver-security-tab-sql-preview")
        .expect("SQL preview tab");
    cx.simulate_click(preview_tab.center(), Modifiers::default());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        let preview = cx.update(|_, cx| {
            view.read(cx)
                .user_admin_state_for(tab_id)
                .and_then(|admin| admin.sqlserver)
                .and_then(|state| state.preview)
        });
        if let Some(preview) = preview {
            let sql = preview.join("\n");
            assert!(sql.contains("[test22]"), "{sql}");
            assert!(!sql.contains("[test]"), "old identity leaked into preview: {sql}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "renamed login preview did not load"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    cx.update(|_, cx| {
        let admin = view
            .read(cx)
            .user_admin_state_for(tab_id)
            .expect("admin");
        assert!(
            admin.sqlserver.is_some(),
            "SQL Server state must survive preview"
        );
        assert_eq!(admin.active_detail_tab, UserAdminDetailTab::SqlServer(SqlServerUserAdminTab::SqlPreview));
        assert!(view.read(cx).controller.state().tabs.iter().any(|tab| {
            tab.id == tab_id
                && matches!(&tab.kind, TabKind::UserAdmin(admin) if admin.sqlserver.is_some())
        }));
    });
}
