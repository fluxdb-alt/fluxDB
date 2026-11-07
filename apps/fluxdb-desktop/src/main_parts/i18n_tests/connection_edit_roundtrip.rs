    /// MySQL 连接弹框全字段无头 E2E：新增的 socket / 排序规则 / 时区 / 初始化 SQL
    /// 与 SSH 主机密钥策略都要能通过真实 UI 事件写入并原样落档。
    #[gpui::test]
    fn mysql_connection_dialog_exposes_all_supported_fields(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let (cx, view, _temp) = redis_e2e_view(cx);
        // 可点击区受滚动视口裁剪，不是整窗：靠下字段的绘制坐标可能落在弹框
        // 底部按钮之下，直接点会被静默丢弃。以底部按钮上沿作为视口下界。
        let dialog_band = cx
            .debug_bounds("new-connection-save")
            .map(|bounds| (0., bounds.origin.y.as_f32() - 8.))
            .unwrap_or((0., 1120.));
        let scroll_into_view = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            for _ in 0..40 {
                if let Some(bounds) = cx.debug_bounds(selector) {
                    let top = bounds.origin.y.as_f32();
                    let bottom = top + bounds.size.height.as_f32();
                    if top >= dialog_band.0 && bottom <= dialog_band.1 {
                        return;
                    }
                }
                redis_e2e_scroll(cx, -4.);
            }
            panic!("无法滚动到可视区: {selector}");
        };

        redis_e2e_click(cx, "home-new-connection");
        // 连接信息页保留高频字段：Unix socket；排序规则属于低频，移到高级页。
        scroll_into_view(cx, "connection-mysql-socket-path");
        assert!(
            cx.debug_bounds("connection-mysql-collation").is_none(),
            "排序规则不应挤占连接信息首屏"
        );

        // 网络与安全页：MySQL 不渲染独立「校验证书」开关，校验收敛到 SSL 模式。
        redis_e2e_click(cx, "new-connection-tab-1");
        assert!(
            cx.debug_bounds("new-connection-toggle-tls").is_some(),
            "MySQL 应提供 TLS 开关"
        );
        assert!(
            cx.debug_bounds("new-connection-toggle-tls-verify").is_none(),
            "MySQL 的证书校验必须由 SSL 模式表达，不能再出现独立开关"
        );
        scroll_into_view(cx, "new-connection-toggle-ssh");
        redis_e2e_click(cx, "new-connection-toggle-ssh");
        scroll_into_view(cx, "connection-ssh-host");
        redis_e2e_fill(cx, "connection-ssh-host", "jump.internal");
        scroll_into_view(cx, "connection-ssh-port");
        redis_e2e_fill(cx, "connection-ssh-port", "2222");
        scroll_into_view(cx, "connection-ssh-username");
        redis_e2e_fill(cx, "connection-ssh-username", "ops");
        scroll_into_view(cx, "connection-ssh-password");
        redis_e2e_fill(cx, "connection-ssh-password", "jump-pass");

        // 高级页：连接参数（排序规则）、会话时区与初始化 SQL。
        redis_e2e_click(cx, "new-connection-tab-2");
        scroll_into_view(cx, "connection-mysql-collation");
        redis_e2e_fill(cx, "connection-mysql-collation", "utf8mb4_unicode_ci");
        scroll_into_view(cx, "connection-mysql-timezone");
        redis_e2e_fill(cx, "connection-mysql-timezone", "+08:00");
        scroll_into_view(cx, "connection-mysql-init-sql");
        redis_e2e_fill(
            cx,
            "connection-mysql-init-sql",
            "SET NAMES utf8mb4; SET time_zone='+08:00'",
        );

        cx.update(|_, cx| {
            let form = &view.read(cx).new_connection_form;
            assert_eq!(form.mysql_collation, "utf8mb4_unicode_ci");
            assert_eq!(form.mysql_timezone, "+08:00");
            assert_eq!(
                form.mysql_init_sql,
                "SET NAMES utf8mb4; SET time_zone='+08:00'"
            );
            assert_eq!(form.ssh_host, "jump.internal");
            assert_eq!(form.ssh_host_key_policy, "strict");
            assert!(form.ssh_enabled);
        });

        redis_e2e_click(cx, "new-connection-save-only");
        cx.update(|_, cx| {
            let state = view.read(cx).controller.state();
            let config = &state.connections[0].config;
            let profile = config.mysql_profile.as_ref().expect("MySQL 档案");
            assert_eq!(profile.tls.collation, "utf8mb4_unicode_ci");
            assert_eq!(profile.advanced.timezone, "+08:00");
            assert_eq!(
                profile.advanced.init_sql,
                "SET NAMES utf8mb4; SET time_zone='+08:00'"
            );
            assert_eq!(
                profile.ssh().map(|ssh| ssh.host_key_policy),
                Some(fluxdb_core::SshHostKeyPolicy::Strict)
            );
        });
    }

    /// URL 参数入口只在 MySQL 系出现，且输入值会落进连接配置。
    ///
    /// 回归场景：字段原本建了输入实体但从不渲染，默认参数与导入参数
    /// 对用户完全不可见。
    #[gpui::test]
    fn mysql_url_params_are_visible_and_saved(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let (cx, view, _temp) = redis_e2e_view(cx);

        redis_e2e_click(cx, "home-new-connection");
        assert!(
            cx.debug_bounds("connection-url-params").is_none(),
            "URL 参数属于低频配置，不应挤占连接信息首屏"
        );
        redis_e2e_click(cx, "new-connection-tab-2");
        assert!(
            cx.debug_bounds("connection-url-params").is_some(),
            "MySQL 高级页应提供 URL 参数入口"
        );
        redis_e2e_fill(
            cx,
            "connection-url-params",
            "charset=utf8mb4&parseTime=true",
        );
        redis_e2e_click(cx, "new-connection-save-only");

        cx.update(|_, cx| {
            let state = view.read(cx).controller.state();
            assert_eq!(state.connections.len(), 1, "仅保存应落库一条连接");
            assert_eq!(
                state.connections[0]
                    .config
                    .options
                    .get("url_params")
                    .map(String::as_str),
                Some("charset=utf8mb4&parseTime=true")
            );
        });

        // 其他类型不消费 URL 参数，不渲染入口，避免无效配置。
        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-kind-postgres");
        assert!(
            cx.debug_bounds("connection-url-params").is_none(),
            "PostgreSQL 不消费 URL 参数，不应提供入口"
        );
    }

    /// MySQL 连接编辑回填：TLS 开关、证书路径、SNI 与校验位都必须从档案恢复。
    ///
    /// 回归场景：打开一条已启用 TLS 的 MySQL 连接，改任意字段后保存，
    /// 若回填遗漏这些字段，档案会被重建成「TLS 关闭 + 证书清空」。
    #[test]
    fn mysql_tls_profile_survives_edit_roundtrip() {
        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlConnectionProfile,
            MysqlSslMode, MysqlTlsOptions, SecretRef,
        };

        let mut profile = MysqlConnectionProfile::default();
        profile.basic.host = "db.internal".into();
        profile.basic.port = 3307;
        profile.basic.database = "app".into();
        profile.basic.username = "app".into();
        profile.tls = MysqlTlsOptions {
            enabled: true,
            ssl_mode: MysqlSslMode::Required,
            ca: SecretRef::ref_key("/certs/ca.pem"),
            client_cert: SecretRef::ref_key("/certs/client.pem"),
            client_key: SecretRef::ref_key("/certs/client.key"),
            sni: "db.internal".into(),
            verify: false,
            charset: "utf8mb4".into(),
            collation: String::new(),
        };
        let config = ConnectionConfig {
            id: ConnectionId(1),
            name: "mysql-tls".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "db.internal".into(),
                port: 3307,
                database: Some("app".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: Some(profile.clone()),
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };

        let form = crate::NewConnectionForm::from_config(&config);
        assert!(form.tls_enabled, "TLS 开关应回填为启用");
        assert_eq!(form.tls_ca, "/certs/ca.pem");
        assert_eq!(form.tls_client_cert, "/certs/client.pem");
        assert_eq!(form.tls_client_key, "/certs/client.key");
        assert_eq!(form.tls_sni, "db.internal");
        assert!(!form.tls_verify, "校验证书开关应回填为关闭");
        assert_eq!(form.mysql_tls_ssl_mode, "required");

        let rebuilt = form.build_mysql_profile();
        assert_eq!(rebuilt.tls, profile.tls, "编辑后保存应原样保留 TLS 档案");
    }

    /// MySQL 新增会话/拨号参数编辑回填：socket、排序规则、时区、初始化 SQL 与 SSH 主机密钥策略。
    #[test]
    fn mysql_session_and_socket_profile_survives_edit_roundtrip() {
        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlAdvancedOptions,
            MysqlConnectionProfile, MysqlSslMode, MysqlTlsOptions, SshHostKeyPolicy,
        };

        let mut profile = MysqlConnectionProfile::default();
        profile.basic.host = "db.internal".into();
        profile.basic.port = 3307;
        profile.basic.username = "app".into();
        profile.basic.socket_path = "/var/run/mysqld/mysqld.sock".into();
        profile.tls = MysqlTlsOptions {
            enabled: true,
            ssl_mode: MysqlSslMode::VerifyCa,
            collation: "utf8mb4_unicode_ci".into(),
            verify: true,
            ..MysqlTlsOptions::default()
        };
        profile.transport = Vec::new();
        profile.advanced = MysqlAdvancedOptions {
            timezone: "+08:00".into(),
            init_sql: "SET NAMES utf8mb4; SET time_zone='+08:00'".into(),
            ..MysqlAdvancedOptions::default()
        };
        let config = ConnectionConfig {
            id: ConnectionId(7),
            name: "mysql-session".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "db.internal".into(),
                port: 3307,
                database: Some("app".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: Some(profile.clone()),
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };

        let form = crate::NewConnectionForm::from_config(&config);
        assert_eq!(form.mysql_socket_path, "/var/run/mysqld/mysqld.sock");
        assert_eq!(form.mysql_collation, "utf8mb4_unicode_ci");
        assert_eq!(form.mysql_timezone, "+08:00");
        assert_eq!(
            form.mysql_init_sql,
            "SET NAMES utf8mb4; SET time_zone='+08:00'"
        );
        assert_eq!(form.mysql_tls_ssl_mode, "verify_ca");
        assert_eq!(form.ssh_host_key_policy, SshHostKeyPolicy::Strict.as_str());

        let rebuilt = form.build_mysql_profile();
        assert_eq!(rebuilt.basic.socket_path, profile.basic.socket_path);
        assert_eq!(rebuilt.tls.collation, profile.tls.collation);
        assert_eq!(rebuilt.advanced.timezone, profile.advanced.timezone);
        assert_eq!(rebuilt.advanced.init_sql, profile.advanced.init_sql);
        assert_eq!(rebuilt.tls.ssl_mode, MysqlSslMode::VerifyCa);
    }

    /// 旧档案 `verify=true` + `preferred`：打开后必须显示 verify_ca 并原样保存新档位。
    #[test]
    fn mysql_legacy_verify_flag_migrates_to_verify_ca_on_save() {
        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlConnectionProfile,
            MysqlSslMode, MysqlTlsOptions,
        };

        let mut profile = MysqlConnectionProfile::default();
        profile.tls = MysqlTlsOptions {
            enabled: true,
            ssl_mode: MysqlSslMode::Preferred,
            verify: true,
            ..MysqlTlsOptions::default()
        };
        let config = ConnectionConfig {
            id: ConnectionId(8),
            name: "mysql-legacy".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "db".into(),
                port: 3306,
                database: None,
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: Some(profile),
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };

        let form = crate::NewConnectionForm::from_config(&config);
        assert_eq!(form.mysql_tls_ssl_mode, "verify_ca");
        let rebuilt = form.build_mysql_profile();
        assert_eq!(rebuilt.tls.ssl_mode, MysqlSslMode::VerifyCa);
        assert!(rebuilt.tls.verify);
        assert_eq!(rebuilt.tls.effective_ssl_mode(), MysqlSslMode::VerifyCa);
    }

    /// 达梦高级项编辑回填：超时、应用名与隐藏的 encoding 都必须原样保留。
    #[test]
    fn dameng_advanced_profile_survives_edit_roundtrip() {
        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DamengAdvancedOptions, DamengConnectionProfile,
            DatabaseKind, Endpoint,
        };

        let mut profile = DamengConnectionProfile::default();
        profile.basic.host = "dm.internal".into();
        profile.basic.username = "SYSDBA".into();
        profile.advanced = DamengAdvancedOptions {
            connect_timeout_secs: 12,
            query_timeout_secs: 30,
            encoding: "UTF8".into(),
            application_name: "fluxdb-dm".into(),
        };
        let config = ConnectionConfig {
            id: ConnectionId(3),
            name: "dameng-advanced".into(),
            kind: DatabaseKind::Dameng,
            endpoint: Endpoint::Tcp {
                host: "dm.internal".into(),
                port: 5236,
                database: Some("DAMENG".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: Some(profile.clone()),
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };

        let form = crate::NewConnectionForm::from_config(&config);
        assert_eq!(form.dameng_connect_timeout_secs, "12");
        assert_eq!(form.dameng_query_timeout_secs, "30");
        assert_eq!(form.dameng_application_name, "fluxdb-dm");
        assert_eq!(form.dameng_encoding, "UTF8");

        let rebuilt = form.build_dameng_profile();
        assert_eq!(
            rebuilt.advanced, profile.advanced,
            "编辑后保存应原样保留达梦高级参数"
        );
    }

    /// Oracle 编辑回填：寻址方式与登录身份下拉必须跟随档案。
    ///
    /// 回归场景：SID + SYSDBA 的连接打开后，下拉曾停留在默认的
    /// 「Service Name」「normal」，与实际输入框和档案互相矛盾。
    #[gpui::test]
    fn oracle_selects_follow_saved_profile_on_edit(cx: &mut TestAppContext) {
        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, OracleBasicOptions,
            OracleConnectKind, OracleConnectionProfile, OraclePrivilege, SecretRef,
        };

        let _guard = locale_guard();
        let (cx, view, _temp) = redis_e2e_view(cx);

        let config = ConnectionConfig {
            id: ConnectionId(9),
            name: "oracle-sid".into(),
            kind: DatabaseKind::Oracle,
            endpoint: Endpoint::Tcp {
                host: "10.0.0.5".into(),
                port: 1521,
                database: Some("ORCL".into()),
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: Some(OracleConnectionProfile {
                basic: OracleBasicOptions {
                    host: "10.0.0.5".into(),
                    port: 1521,
                    connect_kind: OracleConnectKind::Sid,
                    sid: "ORCL".into(),
                    username: "sys".into(),
                    privilege: OraclePrivilege::SysDba,
                    password: SecretRef::inline("secret"),
                    ..OracleBasicOptions::default()
                },
                ..OracleConnectionProfile::default()
            }),
        };

        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.new_connection_form = crate::NewConnectionForm::from_config(&config);
                this.new_connection_inputs
                    .sync_from_form(&this.new_connection_form, window, cx);
                this.sync_new_connection_selects(window, cx);
            });
        });

        let (connect_kind, privilege) = cx.update(|_, cx| {
            let inputs = &view.read(cx).new_connection_inputs;
            (
                inputs
                    .oracle_connect_kind_select
                    .read(cx)
                    .selected_value()
                    .cloned(),
                inputs
                    .oracle_privilege_select
                    .read(cx)
                    .selected_value()
                    .cloned(),
            )
        });
        assert_eq!(connect_kind.as_deref(), Some("sid"), "寻址方式下拉应回填 SID");
        assert_eq!(
            privilege.as_deref(),
            Some("sysdba"),
            "登录身份下拉应回填 SYSDBA"
        );
    }

    /// MySQL 代理类型改为下拉后，编辑已有 HTTP CONNECT 代理连接必须回填。
    ///
    /// 回归场景：自由文本输入时，任何拼写差异都会被连接器静默解释成 SOCKS5；
    /// 改成下拉后仍需保证档案里的类型能同步到独立的光标状态。
    #[gpui::test]
    fn mysql_proxy_type_select_follows_saved_profile(cx: &mut TestAppContext) {
        use fluxdb_core::{
            ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlConnectionProfile,
            MysqlProxy, MysqlProxyType, MysqlTransportLayer, SecretRef,
        };

        let _guard = locale_guard();
        let (cx, view, _temp) = redis_e2e_view(cx);

        let mut profile = MysqlConnectionProfile::default();
        profile.basic.host = "db.internal".into();
        profile.basic.username = "app".into();
        profile.transport.push(MysqlTransportLayer::Proxy(MysqlProxy {
            enabled: true,
            proxy_type: MysqlProxyType::HttpConnect,
            host: "proxy.internal".into(),
            port: 3128,
            username: "proxy-user".into(),
            password: SecretRef::inline("secret"),
        }));
        let config = ConnectionConfig {
            id: ConnectionId(11),
            name: "mysql-proxy".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "db.internal".into(),
                port: 3306,
                database: None,
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: Some(profile),
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };

        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.new_connection_form = crate::NewConnectionForm::from_config(&config);
                this.new_connection_inputs
                    .sync_from_form(&this.new_connection_form, window, cx);
                this.sync_new_connection_selects(window, cx);
            });
        });

        let selected = cx.update(|_, cx| {
            view.read(cx)
                .new_connection_inputs
                .mysql_proxy_type_select
                .read(cx)
                .selected_value()
                .cloned()
        });
        assert_eq!(
            selected.as_deref(),
            Some("http_connect"),
            "代理类型下拉应回填档案中的 HTTP CONNECT"
        );
    }

    /// TLS 关闭时参数区必须不可编辑，而不是仅置灰。
    ///
    /// 回归场景：未启用 TLS 时证书与 SSL 模式仍可填写，但连接器按
    /// DISABLED / Disable 拨号，用户以为配置生效、实际被整体丢弃。
    #[gpui::test]
    fn tls_parameters_hidden_until_enabled(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let (cx, _view, _temp) = redis_e2e_view(cx);

        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-tab-1");
        assert!(
            cx.debug_bounds("connection-tls-ca").is_none(),
            "TLS 未启用时不应渲染证书配置"
        );
        assert!(
            cx.debug_bounds("new-connection-toggle-tls-verify").is_none(),
            "TLS 未启用时不应渲染证书校验开关"
        );

        redis_e2e_click(cx, "new-connection-toggle-tls");
        // MySQL 的证书校验由 SSL 模式表达（默认 preferred 不校验），
        // 独立开关在 MySQL 上始终不渲染，避免模式与开关两套事实来源。
        redis_e2e_scroll(cx, -10.);
        assert!(
            cx.debug_bounds("new-connection-toggle-tls-verify").is_none(),
            "MySQL 未选择 verify_ca/verify_identity 时不应出现证书校验开关"
        );
        // preferred 不加载 CA，填写也会被连接器忽略，因此不渲染 CA 输入框。
        assert!(
            cx.debug_bounds("connection-tls-ca").is_none(),
            "preferred 模式下 CA 不生效，不应渲染"
        );
    }

    /// SSH / 代理参数块同样只在启用后渲染，避免「置灰仍可编辑」。
    ///
    /// 回归场景：未启用 SSH/代理时参数仍可填写，但档案根本不会参与拨号，
    /// 用户看到「已配置」却完全无效。
    #[gpui::test]
    fn ssh_and_proxy_parameters_hidden_until_enabled(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let (cx, _view, _temp) = redis_e2e_view(cx);

        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-tab-1");
        assert!(
            cx.debug_bounds("connection-ssh-host").is_none(),
            "SSH 未启用时不应渲染隧道参数"
        );
        redis_e2e_click(cx, "new-connection-toggle-ssh");
        assert!(
            cx.debug_bounds("connection-ssh-host").is_some(),
            "启用 SSH 后应显示隧道参数"
        );

        redis_e2e_click(cx, "new-connection-tab-2");
        assert!(
            cx.debug_bounds("connection-mysql-proxy-host").is_none(),
            "代理未启用时不应渲染代理参数"
        );
        redis_e2e_click(cx, "new-connection-toggle-mysql-proxy");
        assert!(
            cx.debug_bounds("connection-mysql-proxy-host").is_some(),
            "启用代理后应显示代理参数"
        );
    }

    /// 代理端口非法时必须报错，不能静默回落到默认端口。
    #[gpui::test]
    fn invalid_proxy_port_is_rejected(cx: &mut TestAppContext) {
        let _guard = locale_guard();
        let (cx, view, _temp) = redis_e2e_view(cx);

        redis_e2e_click(cx, "home-new-connection");
        redis_e2e_click(cx, "new-connection-tab-2");
        redis_e2e_click(cx, "new-connection-toggle-mysql-proxy");
        redis_e2e_fill(cx, "connection-mysql-proxy-host", "proxy.internal");
        // 端口行可能在滚动视口外，先滚动避免点击/输入被裁剪后静默丢弃。
        redis_e2e_scroll(cx, -6.);
        redis_e2e_fill(cx, "connection-mysql-proxy-port", "70000");
        redis_e2e_click(cx, "new-connection-save-only");

        cx.update(|_, cx| {
            let this = view.read(cx);
            assert_eq!(
                this.controller.state().connections.len(),
                0,
                "非法代理端口不应落库"
            );
            let error = match this.new_connection_form.test_status.as_ref() {
                Some(crate::ConnectionTestStatus::Error(text)) => text.clone(),
                other => panic!("应提示代理端口错误，实际：{other:?}"),
            };
            assert!(
                error.contains("代理端口"),
                "错误信息应指出代理端口: {error}"
            );
        });
    }

