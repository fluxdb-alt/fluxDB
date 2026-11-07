    #[gpui::test]
    fn dameng_connection_form_uses_localized_dm8_fields_and_saves_profile(
        cx: &mut TestAppContext,
    ) {
        use fluxdb_core::DatabaseKind;

        let _guard = locale_guard();
        let temp = tempfile::tempdir().expect("temp profile");
        let storage = FileStorage::new(temp.path().join("profile"));
        write_user_pack(&storage.root_dir().join("locales"));
        fs::write(
            storage.root_dir().join("locales/ja/app.json"),
            r#"{
                "connection": {
                    "dameng": {
                        "instance": "インスタンス",
                        "schema": "スキーマ"
                    }
                }
            }"#,
        )
        .expect("write DM8 labels");
        let bootstrap = init_i18n(
            &app_assets_base_path().join("locales"),
            &storage.root_dir().join("locales"),
            "ja",
        );
        assert!(bootstrap.issues.is_empty(), "{:?}", bootstrap.issues);

        let controller = AppController::new();
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
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let bounds = cx
            .debug_bounds("home-new-connection")
            .expect("new connection action");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let bounds = cx
            .debug_bounds("new-connection-kind-dameng")
            .expect("DM8 tile");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        assert!(cx.debug_bounds("connection-dameng-schema").is_some());
        assert!(cx.debug_bounds("connection-host").is_some());
        assert!(cx.debug_bounds("connection-port").is_some());
        cx.update(|_, cx| {
            let this = view.read(cx);
            assert_eq!(this.new_connection_kind, Some(DatabaseKind::Dameng));
            assert_eq!(this.new_connection_form.port, "5236");
            assert_eq!(this.new_connection_form.username, "SYSDBA");
            assert_eq!(this.new_connection_form.database, "DAMENG");
            assert_eq!(this.new_connection_form.dameng_schema, "SYSDBA");
            assert_eq!(
                rust_i18n::t!("app.connection.dameng.schema").as_ref(),
                "スキーマ"
            );
        });

        for (selector, text) in [
            ("connection-name", "DM8 E2E"),
            ("connection-host", "dm8-e2e.invalid"),
            ("connection-port", "5236"),
            ("connection-username", "SYSDBA"),
        ] {
            let bounds = cx.debug_bounds(selector).expect("production connection input");
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            cx.simulate_input(text);
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
        }

        // 高级页：达梦高级参数必须可见并写回表单。
        let bounds = cx
            .debug_bounds("new-connection-tab-2")
            .expect("advanced tab");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        for (selector, text) in [
            ("connection-dameng-connect-timeout", "12"),
            ("connection-dameng-query-timeout", "30"),
            ("connection-dameng-application-name", "dm-e2e"),
        ] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing advanced field: {selector}"));
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            cx.simulate_keystrokes("cmd-a");
            cx.simulate_input(text);
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
        }
        cx.update(|_, cx| {
            let form = &view.read(cx).new_connection_form;
            assert_eq!(form.dameng_connect_timeout_secs, "12");
            assert_eq!(form.dameng_query_timeout_secs, "30");
            assert_eq!(form.dameng_application_name, "dm-e2e");
        });

        let bounds = cx
            .debug_bounds("new-connection-save")
            .expect("save and connect");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        cx.update(|_, cx| {
            let this = view.read(cx);
            let status = this
                .new_connection_form
                .test_status
                .as_ref()
                .expect("background connection failure must be observable");
            let text = match status {
                crate::ConnectionTestStatus::Error(text) => text,
                other => panic!("expected DM8 connection error, got {other:?}"),
            };
            assert!(
                text.contains("建立达梦连接失败"),
                "failure must come from the DM8 connector, not form validation: {text}"
            );
            assert_eq!(
                this.controller.state().connections.len(),
                0,
                "failed connection must not be persisted"
            );
            assert!(
                this.new_connection_kind.is_some(),
                "form remains open for correction"
            );
        });
        init_i18n(&app_assets_base_path().join("locales"), temp.path(), "en");
    }
