    /// PostgreSQL 连接串导入：sslmode 必须打开 TLS，会话参数落到表单与输入框。
    ///
    /// 通用 SQL 解析会丢弃 PG 的 query 参数，本用例守住专用解析链路：
    /// `sslmode` / `application_name` / `connect_timeout` / `options` 全部可见。
    #[gpui::test]
    fn postgres_uri_import_fills_tls_and_session_fields(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let (cx, view, _temp) = redis_e2e_view(cx);
        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-kind-postgres");
        let placeholder = cx.update(|_, cx| view.read(cx).discovery_uri_placeholder_for_test(cx));
        assert!(
            placeholder.as_ref().contains("postgres://"),
            "PostgreSQL 导入框示例不能继续展示 MySQL: {placeholder}"
        );

        let uri = "postgres://app_user:s3cret@db.example.test:5544/appdb\
                   ?sslmode=require&application_name=flux-e2e&connect_timeout=7\
                   &options=-c%20search_path%3Dapp%20-c%20timezone%3DAsia%2FShanghai%20-c%20statement_timeout%3D5s";
        redis_e2e_fill(cx, "connection-discovery-uri", uri);
        redis_e2e_click(cx, "connection-uri-import");

        cx.update(|_, cx| {
            let form = &view.read(cx).new_connection_form;
            assert_eq!(form.host, "db.example.test");
            assert_eq!(form.port, "5544");
            assert_eq!(form.username, "app_user");
            assert_eq!(form.password, "s3cret");
            assert_eq!(form.database, "appdb");
            assert!(form.tls_enabled, "sslmode=require 必须启用 TLS");
            assert_eq!(form.pg_tls_ssl_mode, "require");
            assert_eq!(form.pg_default_schema, "app");
            assert_eq!(form.pg_application_name, "flux-e2e");
            assert_eq!(form.pg_connect_timeout_secs, "7");
            assert_eq!(form.pg_query_timeout_secs, "5");
            assert_eq!(form.pg_timezone, "Asia/Shanghai");
        });
        // 导入后输入框实体必须与表单一致，否则用户看到的是旧值。
        cx.update(|_, cx| {
            let inputs = &view.read(cx).new_connection_inputs;
            let value = |input: &gpui::Entity<gpui_component::input::InputState>| {
                input.read(cx).value().to_string()
            };
            assert_eq!(value(&inputs.pg_default_schema), "app");
            assert_eq!(value(&inputs.pg_application_name), "flux-e2e");
            assert_eq!(value(&inputs.pg_connect_timeout), "7");
            assert_eq!(value(&inputs.pg_query_timeout), "5");
            assert_eq!(value(&inputs.pg_timezone), "Asia/Shanghai");
        });

        // TLS 页应暴露私钥口令；高级页应暴露会话时区与代理入口。
        redis_e2e_click(cx, "new-connection-tab-1");
        assert!(
            cx.debug_bounds("connection-tls-client-key-passphrase").is_some(),
            "PostgreSQL TLS 需提供客户端私钥口令"
        );
        redis_e2e_click(cx, "new-connection-tab-2");
        redis_e2e_scroll(cx, -12.);
        assert!(
            cx.debug_bounds("connection-pg-timezone").is_some(),
            "PostgreSQL 高级页需提供会话时区"
        );
        assert!(
            cx.debug_bounds("new-connection-toggle-mysql-proxy").is_some(),
            "PostgreSQL 高级页需提供代理入口"
        );
    }

    /// PostgreSQL 真实库端到端：填表 → 高级页设置会话时区 → 保存并连接。
    ///
    /// 由 `scripts/postgres-connection-e2e.sh` 提供 `FLUXDB_PG_E2E`，缺失时跳过。
    /// 连接成功即证明表单 → 档案 → 驱动链路可用；档案回读确认时区随连接保存。
    #[gpui::test]
    fn postgres_connection_form_connects_to_live_server(cx: &mut TestAppContext) {
        let Some(settings) = postgres_e2e_settings() else {
            return;
        };
        let _guard = locale_guard();
        let (cx, view, _temp) = redis_e2e_view(cx);
        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-kind-postgres");

        // 弹框固定高度，表单内部滚动：靠下字段先滚进可视区再点击。
        let save_band = cx
            .debug_bounds("new-connection-save")
            .map(|bounds| bounds.origin.y.as_f32() - 8.)
            .unwrap_or(1120.);
        let scroll_into_view = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            for _ in 0..40 {
                if let Some(bounds) = cx.debug_bounds(selector) {
                    let top = bounds.origin.y.as_f32();
                    let bottom = top + bounds.size.height.as_f32();
                    if bottom <= save_band {
                        return;
                    }
                }
                redis_e2e_scroll(cx, -4.);
            }
            panic!("无法滚动到可视区: {selector}");
        };

        redis_e2e_fill(cx, "connection-name", "PostgreSQL E2E");
        redis_e2e_fill(cx, "connection-host", &settings.host);
        redis_e2e_fill(cx, "connection-port", &settings.port.to_string());
        scroll_into_view(cx, "connection-username");
        redis_e2e_fill(cx, "connection-username", &settings.user);
        scroll_into_view(cx, "connection-password");
        redis_e2e_fill(cx, "connection-password", &settings.password);
        scroll_into_view(cx, "connection-database");
        redis_e2e_fill(cx, "connection-database", &settings.database);

        // 高级页设置会话时区：连接成功说明驱动真的执行了 set_config。
        redis_e2e_click(cx, "new-connection-tab-2");
        scroll_into_view(cx, "connection-pg-timezone");
        redis_e2e_fill(cx, "connection-pg-timezone", "Asia/Shanghai");

        cx.update(|_, cx| {
            let form = &view.read(cx).new_connection_form;
            assert_eq!(form.host, settings.host, "host");
            assert_eq!(form.port, settings.port.to_string(), "port");
            assert_eq!(form.username, settings.user, "user");
            assert_eq!(form.password, settings.password, "password");
            assert_eq!(form.database, settings.database, "database");
            assert_eq!(form.pg_timezone, "Asia/Shanghai", "timezone");
        });

        redis_e2e_click(cx, "new-connection-save");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            let (saved, failure) = cx.update(|_, cx| {
                let this = view.read(cx);
                (
                    this.controller.state().connections.len(),
                    match this.new_connection_form.test_status.as_ref() {
                        Some(crate::ConnectionTestStatus::Error(text)) => Some(text.clone()),
                        _ => None,
                    },
                )
            });
            if saved == 1 {
                break;
            }
            if let Some(text) = failure {
                panic!("PostgreSQL 保存失败: {text}");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "等待 PostgreSQL 连接超时"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        cx.update(|_, cx| {
            let this = view.read(cx);
            let connection = &this.controller.state().connections[0];
            assert_eq!(connection.config.kind, fluxdb_core::DatabaseKind::Postgres);
            let profile = connection
                .config
                .postgres_profile
                .as_ref()
                .expect("PostgreSQL profile");
            assert_eq!(profile.basic.host, settings.host);
            assert_eq!(profile.basic.port, settings.port);
            assert_eq!(profile.basic.username, settings.user);
            assert_eq!(
                profile.basic.maintenance_database, settings.database,
                "数据库名应作为维护库保存"
            );
            assert_eq!(profile.advanced.timezone, "Asia/Shanghai");
            assert!(this.new_connection_kind.is_none(), "连接成功后弹框应关闭");
        });
    }

    /// PostgreSQL E2E 参数：`FLUXDB_PG_E2E=host:port:user:password:db`，缺失时跳过。
    struct PostgresE2eSettings {
        host: String,
        port: u16,
        user: String,
        password: String,
        database: String,
    }

    fn postgres_e2e_settings() -> Option<PostgresE2eSettings> {
        let raw = std::env::var("FLUXDB_PG_E2E").ok()?;
        let parts = raw.splitn(5, ':').collect::<Vec<_>>();
        if parts.len() != 5 {
            return None;
        }
        Some(PostgresE2eSettings {
            host: parts[0].to_string(),
            port: parts[1].parse().ok()?,
            user: parts[2].to_string(),
            password: parts[3].to_string(),
            database: parts[4].to_string(),
        })
    }
