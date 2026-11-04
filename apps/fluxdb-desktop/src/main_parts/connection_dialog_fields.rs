fn field_row_light(
    label: impl Into<SharedString>,
    field_id: ConnectionField,
    inputs: &NewConnectionInputs,
    secure: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    let hint = connection_field_hint(field_id);
    let mut row = field().label(label.into()).items_center();
    if !hint.is_empty() {
        row = row.description_fn(move |_, _| {
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(hint.clone())
        });
    }
    row.child(input_box_light(
        field_id, inputs, secure, colors, window, cx,
    ))
}

fn connection_field_hint(field: ConnectionField) -> SharedString {
    match field {
        ConnectionField::MysqlSshConnectTimeout => tr("app.connection.ssh.connect_timeout_hint"),
        ConnectionField::MysqlSshKeepalive => tr("app.connection.ssh.keepalive_hint"),
        ConnectionField::PgConnectTimeoutSecs => tr("app.connection.postgres.connect_timeout_hint"),
        ConnectionField::PgQueryTimeoutSecs => tr("app.connection.postgres.query_timeout_hint"),
        ConnectionField::MysqlConnectTimeout => tr("app.connection.mysql.connect_timeout_hint"),
        ConnectionField::MysqlQueryTimeout => tr("app.connection.mysql.query_timeout_hint"),
        ConnectionField::MysqlIdleTtl => tr("app.connection.mysql.idle_ttl_hint"),
        ConnectionField::SqlServerInstance => tr("app.connection.sqlserver.instance_hint"),
        ConnectionField::SqlServerConnectTimeout => {
            tr("app.connection.sqlserver.connect_timeout_hint")
        }
        ConnectionField::SqlServerQueryTimeout => tr("app.connection.sqlserver.query_timeout_hint"),
        ConnectionField::SqlServerApplicationName => {
            tr("app.connection.sqlserver.application_name_hint")
        }
        ConnectionField::OracleServiceName => tr("app.connection.oracle.service_name_hint"),
        ConnectionField::OracleSid => tr("app.connection.oracle.sid_hint"),
        ConnectionField::OracleSchema => tr("app.connection.oracle.schema_hint"),
        ConnectionField::OracleConnectTimeout => {
            tr("app.connection.oracle.connect_timeout_hint")
        }
        ConnectionField::OracleQueryTimeout => tr("app.connection.oracle.query_timeout_hint"),
        ConnectionField::OracleApplicationName => {
            tr("app.connection.oracle.application_name_hint")
        }
        ConnectionField::OracleWalletLocation => tr("app.connection.oracle.wallet_location_hint"),
        ConnectionField::OracleWalletPassword => tr("app.connection.oracle.wallet_password_hint"),
        _ => SharedString::default(),
    }
}

fn host_port_row_light(
    label: impl Into<SharedString>,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().label(label.into()).items_center().child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                input_box_light(ConnectionField::Host, inputs, false, colors, window, cx)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(
                input_box_light(ConnectionField::Port, inputs, false, colors, window, cx)
                    .w(px(96.))
                    .flex_none(),
            ),
    )
}

fn sqlite_file_row_light(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field()
        .label(tr("app.connection.field.file_path"))
        .items_center()
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    input_box_light(
                        ConnectionField::SqlitePath,
                        inputs,
                        false,
                        colors,
                        window,
                        cx,
                    )
                    .flex_1()
                    .min_w(px(0.)),
                )
                .child(folder_button_light(colors, cx).flex_none()),
        )
}


fn color_row_light(selected_color: &str, colors: UiColors, cx: &mut Context<NavicatMain>) -> Field {
    let mut swatches = div().flex().items_center().gap_2();
    for &(hex, value) in CONNECTION_COLOR_PALETTE {
        swatches = swatches.child(color_swatch_light(
            hex,
            rgb(value),
            selected_color == hex,
            colors,
            cx,
        ));
    }

    field()
        .label(tr("app.connection.field.color"))
        .items_center()
        .child(swatches)
}

fn color_swatch_light(
    hex: &'static str,
    color: gpui::Rgba,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .size(px(26.))
        .rounded_full()
        .border_2()
        .border_color(if selected {
            if colors.is_dark {
                rgb(0xd1d5db)
            } else {
                rgb(0x858b96)
            }
        } else {
            colors.border
        })
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover).border_color(color))
        .child(
            div()
                .size(px(18.))
                .rounded_full()
                .bg(color)
                .border_1()
                .border_color(colors.border),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.set_new_connection_color(hex, cx);
                cx.stop_propagation();
            }),
        )
}

fn connection_test_status(status: &ConnectionTestStatus) -> Stateful<Div> {
    let (text, fg) = match status {
        ConnectionTestStatus::Success(text) => (text.clone(), rgb(0x16a34a)),
        ConnectionTestStatus::Error(text) => (text.clone(), rgb(0xff0000)),
        ConnectionTestStatus::Pending(text) => (text.clone(), rgb(0x667085)),
    };

    // 测试连接结果按 status 语义暴露给辅助功能与 UI 自动化：
    // 文案作为可访问名称，Role::Status 让读屏与断言都能区分进行中/成功/失败。
    div()
        .id("new-connection-test-status")
        .role(Role::Status)
        .aria_label(text.clone())
        .h(px(36.))
        .flex()
        .items_center()
        .overflow_hidden()
        .truncate()
        .text_size(px(14.))
        .text_color(fg)
        .child(text)
}

fn input_box_light(
    field: ConnectionField,
    inputs: &NewConnectionInputs,
    secure: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = inputs.for_field(field).focus_handle(cx).is_focused(window);
    // 稳定 ElementId 按字段业务身份派生：同一字段在不同页签/数据库类型下仍是同一个控件，
    // UI 自动化与无障碍树都不必依赖 placeholder 文案或列表下标。
    let input = Input::new(inputs.for_field(field))
        .id(connection_field_element_id(field))
        .aria_label(connection_field_label(field))
        .small()
        .appearance(false)
        .focus_bordered(false);

    div()
        .debug_selector(move || connection_field_element_id(field).to_string())
        .w_full()
        .h(px(34.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(if focused {
            if colors.is_dark {
                rgb(0x8ab4ff)
            } else {
                rgb(0x111111)
            }
        } else {
            colors.border
        })
        .bg(colors.input_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(1.),
            px(4.),
            px(0.),
            hsla(0., 0., 0., if focused { 0.08 } else { 0.11 }),
        )])
        .flex()
        .items_center()
        .overflow_hidden()
        .child(input.w_full().h_full().px_3().text_size(px(14.)))
        .when(secure, |this| this.child(password_eye_button(colors, cx)))
}

fn password_eye_button(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .size(px(30.))
        .mr_1()
        .rounded(colors.radius)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.muted)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .relative()
                .w(px(16.))
                .h(px(10.))
                .rounded_full()
                .border_1()
                .border_color(colors.muted)
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(4.)).rounded_full().bg(colors.muted)),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.toggle_new_connection_password_visibility(window, cx);
                cx.stop_propagation();
            }),
        )
}

fn folder_button_light(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .size(px(38.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover).border_color(rgb(0xaeb7c2)))
        .child(
            div()
                .relative()
                .w(px(18.))
                .h(px(14.))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(2.))
                        .w(px(8.))
                        .h(px(4.))
                        .rounded(colors.radius * 0.5)
                        .bg(colors.muted),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w(px(18.))
                        .h(px(12.))
                        .rounded(colors.radius * 0.5)
                        .border_2()
                        .border_color(colors.muted)
                        .bg(colors.input_bg),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.choose_sqlite_file(window, cx);
                cx.stop_propagation();
            }),
        )
}

fn database_default_port(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql | DatabaseKind::MariaDb => "3306",
        DatabaseKind::TiDb => "4000",
        DatabaseKind::MongoDb => "27017",
        DatabaseKind::Redis => "6379",
        DatabaseKind::Postgres => "5432",
        DatabaseKind::Dameng => "5236",
        DatabaseKind::SqlServer => "1433",
        DatabaseKind::Oracle => "1521",
        DatabaseKind::Sqlite => "",
    }
}

fn database_default_port_u16(kind: DatabaseKind) -> u16 {
    database_default_port(kind).parse().unwrap_or(0)
}

fn parse_port(port: &str) -> Result<u16, String> {
    let port = port.trim();
    if port.is_empty() {
        return Err(tr("app.connection.error.port_required").to_string());
    }
    port.parse::<u16>()
        .map_err(|_| tr("app.connection.error.port_invalid").to_string())
}

fn non_empty_option(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn first_connection_field(kind: DatabaseKind) -> ConnectionField {
    connection_fields(kind)[0]
}

fn connection_fields(kind: DatabaseKind) -> &'static [ConnectionField] {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
            ConnectionField::UrlParams,
        ],
        DatabaseKind::Sqlite => &[ConnectionField::Name, ConnectionField::SqlitePath],
        DatabaseKind::MongoDb => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::MongoDefaultDb,
            ConnectionField::MongoAuthDb,
        ],
        DatabaseKind::Redis => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::UrlParams,
        ],
        DatabaseKind::Postgres => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
            ConnectionField::UrlParams,
        ],
        DatabaseKind::Dameng => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
            ConnectionField::DamengSchema,
        ],
        DatabaseKind::SqlServer => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
            ConnectionField::SqlServerSchema,
            ConnectionField::SqlServerInstance,
        ],
        // Service Name 与 SID 都列出来：由「连接方式」下拉决定哪一个生效，
        // 隐藏另一半反而让用户不清楚自己填的是哪一种寻址。
        DatabaseKind::Oracle => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::OracleServiceName,
            ConnectionField::OracleSid,
            ConnectionField::OracleSchema,
        ],
    }
}

/// 文本字段输入框的稳定 `ElementId`，按字段业务身份命名（如 `connection-name`）。
///
/// 与 placeholder 文案解耦：文案面向用户可随时调整，ID 面向控件定位必须稳定。
fn connection_field_element_id(field: ConnectionField) -> &'static str {
    match field {
        ConnectionField::Name => "connection-name",
        ConnectionField::Host => "connection-host",
        ConnectionField::Port => "connection-port",
        ConnectionField::Username => "connection-username",
        ConnectionField::Password => "connection-password",
        ConnectionField::Database => "connection-database",
        ConnectionField::DamengSchema => "connection-dameng-schema",
        ConnectionField::UrlParams => "connection-url-params",
        ConnectionField::SqlitePath => "connection-sqlite-path",
        ConnectionField::MongoDefaultDb => "connection-mongo-default-db",
        ConnectionField::MongoAuthDb => "connection-mongo-auth-db",
        ConnectionField::TlsCa => "connection-tls-ca",
        ConnectionField::TlsClientCert => "connection-tls-client-cert",
        ConnectionField::TlsClientKey => "connection-tls-client-key",
        ConnectionField::TlsSni => "connection-tls-sni",
        ConnectionField::SshHost => "connection-ssh-host",
        ConnectionField::SshPort => "connection-ssh-port",
        ConnectionField::SshUsername => "connection-ssh-username",
        ConnectionField::SshPassword => "connection-ssh-password",
        ConnectionField::SshPrivateKey => "connection-ssh-private-key",
        ConnectionField::SshPassphrase => "connection-ssh-passphrase",
        ConnectionField::SentinelMasterName => "connection-sentinel-master-name",
        ConnectionField::SentinelEndpoints => "connection-sentinel-endpoints",
        ConnectionField::ClusterStartNodes => "connection-cluster-start-nodes",
        ConnectionField::CloudSubscription => "connection-cloud-subscription",
        ConnectionField::CloudResource => "connection-cloud-resource",
        ConnectionField::DiscoveryUri => "connection-discovery-uri",
        ConnectionField::PgDefaultSchema => "connection-pg-default-schema",
        ConnectionField::PgApplicationName => "connection-pg-application-name",
        ConnectionField::PgConnectTimeoutSecs => "connection-pg-connect-timeout-secs",
        ConnectionField::PgQueryTimeoutSecs => "connection-pg-query-timeout-secs",
        ConnectionField::MysqlCharset => "connection-mysql-charset",
        ConnectionField::MysqlProxyType => "connection-mysql-proxy-type",
        ConnectionField::MysqlSshConnectTimeout => "connection-mysql-ssh-connect-timeout",
        ConnectionField::MysqlSshKeepalive => "connection-mysql-ssh-keepalive",
        ConnectionField::MysqlProxyHost => "connection-mysql-proxy-host",
        ConnectionField::MysqlProxyPort => "connection-mysql-proxy-port",
        ConnectionField::MysqlProxyUsername => "connection-mysql-proxy-username",
        ConnectionField::MysqlProxyPassword => "connection-mysql-proxy-password",
        ConnectionField::MysqlConnectTimeout => "connection-mysql-connect-timeout",
        ConnectionField::MysqlQueryTimeout => "connection-mysql-query-timeout",
        ConnectionField::MysqlIdleTtl => "connection-mysql-idle-ttl",
        ConnectionField::SqlServerSchema => "connection-sqlserver-schema",
        ConnectionField::SqlServerInstance => "connection-sqlserver-instance",
        ConnectionField::SqlServerConnectTimeout => "connection-sqlserver-connect-timeout",
        ConnectionField::SqlServerQueryTimeout => "connection-sqlserver-query-timeout",
        ConnectionField::SqlServerApplicationName => "connection-sqlserver-application-name",
        ConnectionField::OracleServiceName => "connection-oracle-service-name",
        ConnectionField::OracleSid => "connection-oracle-sid",
        ConnectionField::OracleSchema => "connection-oracle-schema",
        ConnectionField::OracleConnectTimeout => "connection-oracle-connect-timeout",
        ConnectionField::OracleQueryTimeout => "connection-oracle-query-timeout",
        ConnectionField::OracleApplicationName => "connection-oracle-application-name",
        ConnectionField::OracleWalletLocation => "connection-oracle-wallet-location",
        ConnectionField::OracleWalletPassword => "connection-oracle-wallet-password",
    }
}

/// 文本字段输入框的可访问名称。
///
/// 密码类字段不暴露标签以外的信息，`Input` 的 masked 语义已保证取值不进无障碍树；
/// 这里只给字段名，不拼接当前值。
fn connection_field_label(field: ConnectionField) -> SharedString {
    match field {
        ConnectionField::Name => tr("app.connection.field.name"),
        ConnectionField::Host => tr("app.connection.field.host"),
        ConnectionField::Port => tr("app.connection.field.port"),
        ConnectionField::Username => tr("app.connection.field.username"),
        ConnectionField::Password => tr("app.connection.optional"),
        ConnectionField::Database => tr("app.connection.field.database"),
        ConnectionField::DamengSchema => tr("app.connection.dameng.schema"),
        ConnectionField::UrlParams => tr("app.connection.field.url_params"),
        ConnectionField::SqlitePath => tr("app.connection.field.file_path"),
        ConnectionField::MongoDefaultDb => tr("app.connection.field.default_db"),
        ConnectionField::MongoAuthDb => tr("app.connection.field.auth_db"),
        ConnectionField::TlsCa => tr("app.connection.tls.ca"),
        ConnectionField::TlsClientCert => tr("app.connection.tls.client_cert"),
        ConnectionField::TlsClientKey => tr("app.connection.tls.client_key"),
        ConnectionField::TlsSni => tr("app.connection.tls.sni"),
        ConnectionField::SshHost => tr("app.connection.ssh.host"),
        ConnectionField::SshPort => tr("app.connection.ssh.port"),
        ConnectionField::SshUsername => tr("app.connection.ssh.username"),
        ConnectionField::SshPassword => tr("app.connection.ssh.password"),
        ConnectionField::SshPrivateKey => tr("app.connection.ssh.private_key"),
        ConnectionField::SshPassphrase => tr("app.connection.ssh.passphrase"),
        ConnectionField::SentinelMasterName => tr("app.connection.redis.sentinel_master"),
        ConnectionField::SentinelEndpoints => tr("app.connection.redis.sentinel_endpoints"),
        ConnectionField::ClusterStartNodes => tr("app.connection.redis.cluster_nodes"),
        ConnectionField::CloudSubscription => tr("app.connection.redis.cloud_subscription"),
        ConnectionField::CloudResource => tr("app.connection.redis.cloud_resource"),
        ConnectionField::DiscoveryUri => tr("app.connection.redis.uri"),
        ConnectionField::PgDefaultSchema => tr("app.connection.postgres.default_schema"),
        ConnectionField::PgApplicationName => tr("app.connection.postgres.application_name"),
        ConnectionField::PgConnectTimeoutSecs => tr("app.connection.postgres.connect_timeout"),
        ConnectionField::PgQueryTimeoutSecs => tr("app.connection.postgres.query_timeout"),
        ConnectionField::MysqlCharset => tr("app.connection.tls.charset"),
        ConnectionField::MysqlProxyType => tr("app.connection.mysql.proxy_type"),
        ConnectionField::MysqlSshConnectTimeout => tr("app.connection.ssh.connect_timeout"),
        ConnectionField::MysqlSshKeepalive => tr("app.connection.ssh.keepalive"),
        ConnectionField::MysqlProxyHost => tr("app.connection.field.host"),
        ConnectionField::MysqlProxyPort => tr("app.connection.field.port"),
        ConnectionField::MysqlProxyUsername => tr("app.connection.mysql.proxy_username"),
        ConnectionField::MysqlProxyPassword => tr("app.connection.mysql.proxy_password"),
        ConnectionField::MysqlConnectTimeout => tr("app.connection.mysql.connect_timeout"),
        ConnectionField::MysqlQueryTimeout => tr("app.connection.mysql.query_timeout"),
        ConnectionField::MysqlIdleTtl => tr("app.connection.mysql.idle_ttl"),
        ConnectionField::SqlServerSchema => tr("app.connection.sqlserver.schema"),
        ConnectionField::SqlServerInstance => tr("app.connection.sqlserver.instance"),
        ConnectionField::SqlServerConnectTimeout => tr("app.connection.sqlserver.connect_timeout"),
        ConnectionField::SqlServerQueryTimeout => tr("app.connection.sqlserver.query_timeout"),
        ConnectionField::SqlServerApplicationName => {
            tr("app.connection.sqlserver.application_name")
        }
        ConnectionField::OracleServiceName => tr("app.connection.oracle.service_name"),
        ConnectionField::OracleSid => tr("app.connection.oracle.sid"),
        ConnectionField::OracleSchema => tr("app.connection.oracle.schema"),
        ConnectionField::OracleConnectTimeout => tr("app.connection.oracle.connect_timeout"),
        ConnectionField::OracleQueryTimeout => tr("app.connection.oracle.query_timeout"),
        ConnectionField::OracleApplicationName => tr("app.connection.oracle.application_name"),
        ConnectionField::OracleWalletLocation => tr("app.connection.oracle.wallet_location"),
        ConnectionField::OracleWalletPassword => tr("app.connection.oracle.wallet_password"),
    }
}

fn connection_field_placeholder(field: ConnectionField) -> SharedString {
    match field {
        ConnectionField::Name => tr("app.connection.field.name"),
        ConnectionField::Host => "127.0.0.1".into(),
        ConnectionField::Port => tr("app.connection.field.port"),
        ConnectionField::Username => tr("app.connection.field.username"),
        ConnectionField::Password => tr("app.connection.optional"),
        ConnectionField::Database => tr("app.connection.field.database"),
        ConnectionField::DamengSchema => tr("app.connection.dameng.schema_optional"),
        ConnectionField::MongoDefaultDb => tr("app.connection.optional"),
        ConnectionField::UrlParams => "key=value&key2=value2".into(),
        ConnectionField::SqlitePath => "/path/to/database.db or :memory:".into(),
        ConnectionField::MongoAuthDb => tr("app.connection.field.auth_db_optional"),
        ConnectionField::TlsCa => tr("app.connection.tls.ca"),
        ConnectionField::TlsClientCert => tr("app.connection.tls.client_cert"),
        ConnectionField::TlsClientKey => tr("app.connection.tls.client_key"),
        ConnectionField::TlsSni => tr("app.connection.tls.sni_optional"),
        ConnectionField::SshHost => tr("app.connection.ssh.host"),
        ConnectionField::SshPort => "22".into(),
        ConnectionField::SshUsername => tr("app.connection.ssh.username"),
        ConnectionField::SshPassword => tr("app.connection.ssh.password"),
        ConnectionField::SshPrivateKey => tr("app.connection.ssh.private_key"),
        ConnectionField::SshPassphrase => tr("app.connection.ssh.passphrase_optional"),
        // —— PostgreSQL ——
        ConnectionField::PgDefaultSchema => tr("app.connection.postgres.default_schema_optional"),
        ConnectionField::PgApplicationName => {
            tr("app.connection.postgres.application_name_optional")
        }
        ConnectionField::PgConnectTimeoutSecs => "5".into(),
        ConnectionField::PgQueryTimeoutSecs => tr("app.connection.postgres.query_timeout_hint"),
        // —— MySQL / TiDB ——
        ConnectionField::MysqlCharset => "utf8mb4".into(),
        ConnectionField::MysqlProxyType => "socks5 (socks5/http_connect)".into(),
        ConnectionField::MysqlSshConnectTimeout => tr("app.connection.ssh.connect_timeout_hint"),
        ConnectionField::MysqlSshKeepalive => tr("app.connection.ssh.keepalive_hint"),
        ConnectionField::MysqlProxyHost => tr("app.connection.field.host"),
        ConnectionField::MysqlProxyPort => tr("app.connection.field.port"),
        ConnectionField::MysqlProxyUsername => tr("app.connection.optional"),
        ConnectionField::MysqlProxyPassword => tr("app.connection.optional"),
        ConnectionField::MysqlConnectTimeout => "5".into(),
        ConnectionField::MysqlQueryTimeout => tr("app.connection.mysql.query_timeout_hint"),
        ConnectionField::MysqlIdleTtl => tr("app.connection.mysql.idle_ttl_hint"),
        ConnectionField::SentinelMasterName => tr("app.connection.redis.sentinel_master"),
        ConnectionField::SentinelEndpoints => "host:port, host:port".into(),
        ConnectionField::ClusterStartNodes => "host:port, host:port".into(),
        ConnectionField::CloudSubscription => tr("app.connection.redis.cloud_subscription"),
        ConnectionField::CloudResource => tr("app.connection.redis.cloud_resource"),
        ConnectionField::DiscoveryUri => tr("app.connection.redis.uri"),
        ConnectionField::SqlServerSchema => tr("app.connection.sqlserver.schema_optional"),
        ConnectionField::SqlServerInstance => tr("app.connection.sqlserver.instance_optional"),
        ConnectionField::SqlServerConnectTimeout => "5".into(),
        ConnectionField::SqlServerQueryTimeout => tr("app.connection.sqlserver.query_timeout_hint"),
        ConnectionField::SqlServerApplicationName => {
            tr("app.connection.sqlserver.application_name_optional")
        }
        ConnectionField::OracleServiceName => "FREEPDB1 / ORCLPDB1".into(),
        ConnectionField::OracleSid => "ORCL".into(),
        ConnectionField::OracleSchema => tr("app.connection.oracle.schema_optional"),
        ConnectionField::OracleConnectTimeout => "8".into(),
        ConnectionField::OracleQueryTimeout => tr("app.connection.oracle.query_timeout_hint"),
        ConnectionField::OracleApplicationName => {
            tr("app.connection.oracle.application_name_optional")
        }
        ConnectionField::OracleWalletLocation => tr("app.connection.oracle.wallet_location_optional"),
        ConnectionField::OracleWalletPassword => tr("app.connection.optional"),
    }
}
