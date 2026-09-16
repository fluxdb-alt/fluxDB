fn mysql_connection_url(config: &ConnectionConfig) -> fluxdb_core::Result<String> {
    let Endpoint::Tcp {
        host,
        port,
        database,
    } = &config.endpoint
    else {
        return Err(Error::new(
            ErrorKind::Connection,
            "MySQL 连接需要 TCP 主机和端口",
        ));
    };

    let username = config
        .options
        .get("username")
        .map(String::as_str)
        .unwrap_or("root");
    let password = config
        .options
        .get(PLAINTEXT_PASSWORD_OPTION)
        .map(String::as_str)
        .unwrap_or("");
    let auth = if password.is_empty() {
        percent_encode(username)
    } else {
        format!("{}:{}", percent_encode(username), percent_encode(password))
    };
    let database = database
        .as_deref()
        .filter(|database| !database.trim().is_empty())
        .map(|database| format!("/{}", percent_encode(database)))
        .unwrap_or_default();
    let user_params = config
        .options
        .get(URL_PARAMS_OPTION)
        .map(|params| params.trim().trim_start_matches('?'))
        .filter(|params| !params.is_empty())
        .map(str::to_string);
    // 档案 TLS 参数追加在用户自定义参数之后：同名键档案覆盖用户值。
    let tls_params = mysql_tls_url_params(config);
    let mut params = user_params.unwrap_or_default();
    if !tls_params.is_empty() {
        if !params.is_empty() {
            params.push('&');
        }
        params.push_str(&tls_params);
    }
    let query = if params.is_empty() {
        String::new()
    } else {
        format!("?{params}")
    };

    Ok(format!("mysql://{auth}@{host}:{port}{database}{query}"))
}

/// 把结构化 MySQL 档案的 TLS 语义折叠成 sqlx URL 参数。
///
/// 仅档案启用 TLS 时注入；历史扁平连接（无档案）不注入，保持 sqlx 默认 Preferred
/// 行为不变。模式映射：
/// - `Disabled` → `ssl-mode=DISABLED`（明确禁用加密）；
/// - `Preferred`/`Required` 且 `verify=true` 并配置了 CA → 升级 `VERIFY_CA`
///   （sqlx 校验证书链，防中间人；`Preferred+VERIFY_CA` 时服务器不支持 TLS 也会失败，
///   配置了 CA 即视为要求校验）；
/// - 其余注入 `PREFERRED`/`REQUIRED`（sqlx 仅加密、不校验证书，覆盖 `tls_insecure` 语义）。
///
/// 客户端证书/私钥按配置注入（mTLS）。`tls.sni` 独立校验名 sqlx 不支持
/// （主机名固定取连接 host），暂忽略；SSH 隧道下 host 为 127.0.0.1，
/// 因此校验档位最高到 VERIFY_CA（不校验主机名）。
fn mysql_tls_url_params(config: &ConnectionConfig) -> String {
    let Some(profile) = config.mysql_profile.as_ref() else {
        return String::new();
    };
    let tls = &profile.tls;
    if !tls.enabled {
        return String::new();
    }
    let ca = tls.ca.value().map(str::trim).filter(|p| !p.is_empty());
    let verify_ca = tls.verify && ca.is_some();
    let mode = match (tls.ssl_mode, verify_ca) {
        (fluxdb_core::MysqlSslMode::Disabled, _) => "DISABLED",
        (_, true) => "VERIFY_CA",
        (fluxdb_core::MysqlSslMode::Required, false) => "REQUIRED",
        (fluxdb_core::MysqlSslMode::Preferred, false) => "PREFERRED",
    };
    let mut params = vec![format!("ssl-mode={mode}")];
    if verify_ca {
        params.push(format!("ssl-ca={}", percent_encode(ca.unwrap())));
    }
    for (prefix, secret) in [("ssl-cert", &tls.client_cert), ("ssl-key", &tls.client_key)] {
        if let Some(path) = secret.value().map(str::trim).filter(|p| !p.is_empty()) {
            params.push(format!("{prefix}={}", percent_encode(path)));
        }
    }
    params.join("&")
}

/// 拨号 MySQL：解析出可直接 `connect()` 的选项，并按需建立 SSH 隧道。
///
/// 返回 `(options, tunnel)`：
/// - `tunnel` 为 `Some` 时，连接目标已被改写为本地转发端口 `127.0.0.1:<local_port>`，
///   隧道对象需与连接同生命周期存活（`Drop` 时断开转发）。
/// - 复用 Redis 的 `SshTunnel`（crate 根 `pub(crate)`）与 `ssh_*` 扁平键，零可见性改动。
///
/// 仅测试/保存路径经此拨号；其余无状态 ops 仍走 `mysql_connection_url` 直连。
fn mysql_dial(
    config: &ConnectionConfig,
) -> fluxdb_core::Result<(MySqlConnectOptions, Option<SshTunnel>)> {
    let resolved = config.mysql_resolved();
    let Endpoint::Tcp { host, port, .. } = &resolved.endpoint else {
        return Err(Error::new(
            ErrorKind::Connection,
            "MySQL 连接需要 TCP 主机和端口",
        ));
    };
    let mut options = mysql_connection_url(&resolved)?
        .parse::<MySqlConnectOptions>()
        .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?;

    let tunnel = if ssh_enabled(&resolved.options) {
        let Some(auth) = ssh_auth_from_options(&resolved.options, true) else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SSH 隧道配置缺少认证参数",
            ));
        };
        let jump_host = resolved.options.get("ssh_host").cloned().unwrap_or_default();
        let jump_port = resolved
            .options
            .get("ssh_port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(22);
        if jump_host.is_empty() {
            return Err(Error::new(ErrorKind::Connection, "请填写 SSH 跳板机主机"));
        }
        Some(SshTunnel::open((&jump_host, jump_port), &auth, (host, *port))?)
    } else {
        None
    };
    if let Some(tunnel) = &tunnel {
        options = options.host("127.0.0.1").port(tunnel.local_port);
    }
    Ok((options, tunnel))
}

