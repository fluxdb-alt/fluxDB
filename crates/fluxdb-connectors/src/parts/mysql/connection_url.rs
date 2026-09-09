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
    let params = config
        .options
        .get(URL_PARAMS_OPTION)
        .map(|params| params.trim().trim_start_matches('?'))
        .filter(|params| !params.is_empty())
        .map(|params| format!("?{params}"))
        .unwrap_or_default();

    Ok(format!("mysql://{auth}@{host}:{port}{database}{params}"))
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

