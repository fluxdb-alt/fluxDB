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
/// 模式直接采用 MySQL 官方五档语义（`disabled/preferred/required/verify_ca/
/// verify_identity`），刷新了旧版"独立校验开关 + 三档模式"的覆盖关系：
/// - 未启用 TLS 或 `ssl_mode=Disabled` → `DISABLED`。不注入时 sqlx 默认
///   Preferred 仍会尝试加密，用户在界面上关闭 TLS 却可能仍建立 TLS 会话；
/// - 其余按档案模式原样下发；旧档案里 `verify=true` + `preferred/required`
///   由 `effective_ssl_mode()` 迁移为 `VERIFY_CA`，安全级别不降；
/// - `verify_identity` 在 SSH 隧道/代理下降级为 `VERIFY_CA`：驱动看到的目标主机
///   已被改写为 127.0.0.1，sqlx 又不支持独立校验名，继续按主机名校验只会误拒。
///
/// 客户端证书/私钥按配置注入（mTLS）；CA 仅在真正校验的模式下带上。
fn mysql_tls_url_params(config: &ConnectionConfig) -> String {
    let Some(profile) = config.mysql_profile.as_ref() else {
        return String::new();
    };
    let tls = &profile.tls;
    if !tls.enabled || tls.effective_ssl_mode() == fluxdb_core::MysqlSslMode::Disabled {
        return "ssl-mode=DISABLED".into();
    }
    let mut mode = tls.effective_ssl_mode();
    // 隧道/代理会把拨号目标改写为本地回环，主机名校验只能拿到 127.0.0.1。
    if mode == fluxdb_core::MysqlSslMode::VerifyIdentity
        && (profile.ssh().is_some() || profile.proxy().is_some())
    {
        tracing::warn!(
            target: "fluxdb_connectors",
            "MySQL TLS verify_identity 在 SSH 隧道/代理下不可用，已降级为 verify_ca"
        );
        mode = fluxdb_core::MysqlSslMode::VerifyCa;
    }
    let mode = match mode {
        fluxdb_core::MysqlSslMode::Preferred => "PREFERRED",
        fluxdb_core::MysqlSslMode::Required => "REQUIRED",
        fluxdb_core::MysqlSslMode::VerifyCa => "VERIFY_CA",
        fluxdb_core::MysqlSslMode::VerifyIdentity => "VERIFY_IDENTITY",
        fluxdb_core::MysqlSslMode::Disabled => "DISABLED",
    };
    let mut params = vec![format!("ssl-mode={mode}")];
    if tls.effective_ssl_mode().verifies_certificate()
        && let Some(ca) = tls.ca.value().map(str::trim).filter(|p| !p.is_empty())
    {
        params.push(format!("ssl-ca={}", percent_encode(ca)));
    }
    for (prefix, secret) in [("ssl-cert", &tls.client_cert), ("ssl-key", &tls.client_key)] {
        if let Some(path) = secret.value().map(str::trim).filter(|p| !p.is_empty()) {
            params.push(format!("{prefix}={}", percent_encode(path)));
        }
    }
    params.join("&")
}

/// MySQL 拨号计划：归一化后的 sqlx 选项、超时与 SSH 隧道。
///
/// 所有连接路径（查询、浏览、元数据、补全、测试）都经此拨号，保证 SSH 隧道、
/// 建连超时在真实操作中同样生效，而不只是“测试连接”时生效。
///
/// `tunnel` 随本对象存活，Drop 时关闭本地转发：必须在连接关闭之后再丢弃，
/// 因此调用方一律把本对象持有到操作结束。
struct MySqlConnectionPlan {
    options: MySqlConnectOptions,
    connect_timeout: Duration,
    /// `None` 表示不限制查询时长（档案里 0）。
    query_timeout: Option<Duration>,
    /// 会话级超时 SQL。浏览/元数据等非用户 SQL 也受同一上限约束；
    /// 客户端侧只包了用户 SQL 与 ping，这里补服务端兜底。
    session_timeout_sql: Option<String>,
    /// 连接建立后按顺序执行的初始化语句（每行一条）。
    init_sql: Vec<String>,
    /// 仅用于保活：Drop 时关闭本地转发，字段本身不被读取。
    _tunnel: Option<SshTunnel>,
    /// 代理 / TCP 保活需要本地转发承接 sqlx 自建连接；首次拨号时懒启动。
    forward: Option<ForwardDial>,
    forwarder: tokio::sync::OnceCell<LocalForwarder>,
}

impl MySqlConnectionPlan {
    /// 拨号：先做经典协议探测，再在档案建连超时内完成 TCP + 握手 + 认证。
    async fn connect(&self) -> fluxdb_core::Result<MySqlConnection> {
        let options = self.options_for_connect().await?;
        // 探测是阻塞操作：放到阻塞线程，避免在单 worker 运行时里挡住转发器 accept。
        // Unix socket 不走 TCP，做经典协议探测没有意义。
        if options.get_socket().is_none() {
            let probe_host = options.get_host().to_string();
            let probe_port = options.get_port();
            let probe_timeout = mysql_greeting_probe_timeout(self.connect_timeout);
            tokio::task::spawn_blocking(move || {
                ensure_mysql_classic_greeting(&probe_host, probe_port, probe_timeout)
            })
            .await
            .map_err(|error| Error::new(ErrorKind::Internal, format!("握手探测线程异常: {error}")))??;
        }
        match tokio::time::timeout(self.connect_timeout, options.connect()).await {
            Ok(Ok(mut connection)) => {
                self.apply_session_timeout(&mut connection).await;
                self.apply_init_sql(&mut connection).await?;
                Ok(connection)
            }
            Ok(Err(error)) => Err(self.forward_error_or(mysql_error(error))),
            Err(_) => Err(self.forward_error_or(Error::new(ErrorKind::Connection, "连接超时"))),
        }
    }

    /// 下发会话级查询超时。
    ///
    /// MySQL/TiDB 用 `max_execution_time`（毫秒，只作用于只读 SELECT），
    /// MariaDB 用 `max_statement_time`（秒）。老版本服务端不支持时记录日志并放行，
    /// 仍由用户 SQL 路径的客户端超时兜底。
    async fn apply_session_timeout(&self, connection: &mut MySqlConnection) {
        let Some(sql) = self.session_timeout_sql.as_deref() else {
            return;
        };
        if let Err(error) = sqlx::query(sql).execute(&mut *connection).await {
            tracing::debug!(
                target: "fluxdb_connectors",
                %error,
                sql,
                "MySQL 会话级查询超时设置失败，退回客户端超时"
            );
        }
    }

    /// 执行连接初始化语句。
    ///
    /// 失败即断开并回报具体行号：初始化 SQL 通常承载 `sql_mode`、审计标记等
    /// 会话前置条件，静默跳过会让后续操作跑在错误会话状态上。
    async fn apply_init_sql(&self, connection: &mut MySqlConnection) -> fluxdb_core::Result<()> {
        for (index, sql) in self.init_sql.iter().enumerate() {
            if let Err(error) = sqlx::query(sql).execute(&mut *connection).await {
                return Err(Error::new(
                    ErrorKind::Connection,
                    format!("初始化 SQL 第 {} 条执行失败: {error}", index + 1),
                ));
            }
        }
        Ok(())
    }

    /// 转发拨号失败时驱动只看到连接关闭，用转发器记录的原始原因替换，避免
    /// 把「代理不可达/认证失败」报成泛化的数据库连接错误。
    fn forward_error_or(&self, fallback: Error) -> Error {
        self.forwarder
            .get()
            .and_then(LocalForwarder::take_last_error)
            .map(|message| Error::new(ErrorKind::Connection, message))
            .unwrap_or(fallback)
    }

    /// 实际拨号选项：需要转发时把主机改成本地监听端口。
    async fn options_for_connect(&self) -> fluxdb_core::Result<MySqlConnectOptions> {
        let Some(dial) = self.forward.as_ref() else {
            return Ok(self.options.clone());
        };
        let forwarder = self
            .forwarder
            .get_or_try_init(|| start_local_forwarder(dial.clone(), self.connect_timeout))
            .await?;
        Ok(self
            .options
            .clone()
            .host("127.0.0.1")
            .port(forwarder.local_port))
    }

    fn query_timeout(&self) -> Option<Duration> {
        self.query_timeout
    }
}

/// 构建拨号计划：档案 → sqlx 选项 + SSH 隧道 + 超时。
///
/// 复用 Redis 的 `SshTunnel`（crate 根 `pub(crate)`）与 `ssh_*` 扁平键；
/// SSH 建连超时/心跳取自档案（0 表示用默认）。
fn mysql_connection_plan(config: &ConnectionConfig) -> fluxdb_core::Result<MySqlConnectionPlan> {
    // 依赖树里 rustls 同时启用了 ring（本 crate 直连）与 aws-lc-rs（sqlx），
    // 不显式安装进程级 provider 时首次 TLS 握手会 panic。与 PostgreSQL 路径保持一致。
    let _ = rustls::crypto::ring::default_provider().install_default();

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
    // 字符集/排序规则直接交给驱动：sqlx 在握手后按这两个值设置会话字符集。
    if let Some(charset) = resolved
        .options
        .get("charset")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        options = options.charset(charset);
    }
    if let Some(collation) = resolved
        .options
        .get("collation")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        options = options.collation(collation);
    }
    let socket_path = resolved
        .options
        .get("socket")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    if let Some(socket) = socket_path {
        // sqlx 收到 socket 后按 Unix 域套接字拨号，忽略 host/port。
        options = options.socket(socket);
    }
    if let Some(timezone) = resolved
        .options
        .get("timezone")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        options = options.timezone(Some(timezone.to_string()));
    }

    let proxy = mysql_proxy_spec(&resolved)?;
    let ssh_enabled = ssh_enabled(&resolved.options);
    if socket_path.is_some() && (ssh_enabled || proxy.is_some()) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "MySQL Unix Socket 与 SSH 隧道/代理不能同时启用",
        ));
    }
    if ssh_enabled && proxy.is_some() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "MySQL 暂不支持同时启用 SSH 隧道与代理，请只保留一种",
        ));
    }
    let tcp_keepalive = resolved.options.get("tcp_keepalive").is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "y"
        )
    });
    let target_host = host.clone();
    let target_port = *port;

    let tunnel = if ssh_enabled {
        let Some(auth) = ssh_auth_from_options(&resolved.options, true) else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SSH 隧道配置缺少认证参数",
            ));
        };
        let jump_host = resolved
            .options
            .get("ssh_host")
            .cloned()
            .unwrap_or_default();
        let jump_port = resolved
            .options
            .get("ssh_port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(22);
        if jump_host.is_empty() {
            return Err(Error::new(ErrorKind::Connection, "请填写 SSH 跳板机主机"));
        }
        let ssh_options = SshTunnelOptions {
            connect_timeout_secs: resolved
                .options
                .get("ssh_connect_timeout_secs")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0),
            keepalive_interval_secs: resolved
                .options
                .get("ssh_keepalive_interval_secs")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0),
            // 主机密钥策略与其它数据库共用一套扁平键，缺省严格校验。
            host_key_policy: fluxdb_core::SshHostKeyPolicy::parse(
                resolved
                    .options
                    .get("ssh_host_key_policy")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
        };
        Some(open_tunnel_with(
            (&jump_host, jump_port),
            &auth,
            (host, *port),
            ssh_options,
        )?)
    } else {
        None
    };
    if let Some(tunnel) = &tunnel {
        options = options.host("127.0.0.1").port(tunnel.local_port);
    }

    let connect_timeout_secs = resolved
        .options
        .get("connect_timeout_secs")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(5);
    let query_timeout = resolved
        .options
        .get("query_timeout_secs")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs);
    // 服务端会话超时：MySQL/TiDB 毫秒，MariaDB 秒；只读 SELECT 生效。
    let session_timeout_sql = query_timeout.map(|limit| {
        if resolved.kind == fluxdb_core::DatabaseKind::MariaDb {
            format!("SET SESSION max_statement_time = {}", limit.as_secs())
        } else {
            format!("SET SESSION max_execution_time = {}", limit.as_millis())
        }
    });
    // sqlx 只能自己建连：代理与 TCP 保活都通过本地转发器落地（SSH 已有自己的转发）。
    let forward = if tunnel.is_some() {
        None
    } else if let Some(proxy) = proxy {
        Some(ForwardDial::Proxy {
            spec: proxy,
            target: (target_host, target_port),
            keepalive: tcp_keepalive,
        })
    } else if tcp_keepalive {
        Some(ForwardDial::Direct {
            host: target_host,
            port: target_port,
            keepalive: true,
        })
    } else {
        None
    };
    // 初始化 SQL：按分号拆分，引号/注释内的分号不切分（见 core::split_mysql_init_sql）。
    let init_sql = resolved
        .options
        .get("init_sql")
        .map(|value| fluxdb_core::split_mysql_init_sql(value))
        .unwrap_or_default();
    Ok(MySqlConnectionPlan {
        options,
        connect_timeout: Duration::from_secs(connect_timeout_secs),
        query_timeout,
        session_timeout_sql,
        init_sql,
        _tunnel: tunnel,
        forward,
        forwarder: tokio::sync::OnceCell::new(),
    })
}

/// 从扁平参数读出代理配置；未启用返回 `None`。
///
/// 启用后主机为空属于配置错误，直接报错而不是退回直连。
fn mysql_proxy_spec(config: &ConnectionConfig) -> fluxdb_core::Result<Option<ProxySpec>> {
    let enabled = config
        .options
        .get("proxy_enabled")
        .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "y"));
    if !enabled {
        return Ok(None);
    }
    let host = config
        .options
        .get("proxy_host")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    if host.is_empty() {
        return Err(Error::new(ErrorKind::Connection, "请填写代理主机"));
    }
    let kind = match config.options.get("proxy_type").map(String::as_str) {
        Some("http_connect") => ProxyKind::HttpConnect,
        _ => ProxyKind::Socks5,
    };
    let port = config
        .options
        .get("proxy_port")
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(0);
    Ok(Some(ProxySpec {
        kind,
        host,
        port,
        username: config
            .options
            .get("proxy_username")
            .cloned()
            .unwrap_or_default(),
        password: config
            .options
            .get("proxy_password")
            .cloned()
            .unwrap_or_default(),
    }))
}

/// 正常握手包的 payload 首字节（protocol_version = 10）。
const MYSQL_HANDSHAKE_PROTOCOL_VERSION: u8 = 0x0a;
/// 服务端直接回错误包（如 host 被封、连接数已满）时的 payload 首字节。
const MYSQL_ERR_PACKET: u8 = 0xff;

/// 拨号前确认 `host:port` 说的是 MySQL 经典协议。
///
/// 经典 MySQL 服务端在 TCP 建连后立即下发握手包，payload 首字节是协议版本 10；直接回错误时
/// 是 ERR 包。把端口填成 MySQL X 协议端口（默认 33060）时，对端回的帧头会被 sqlx 当作握手包
/// 解析，而 sqlx 0.8.6 的 `Handshake::decode_with` 不校验 protocol_version，会在残包上继续读
/// 4 字节 connection_id，最终在空缓冲上 panic（`bytes` 的 advance 越界）。UI 线程驱动该 future
/// 时 panic 无法穿过 macOS 的 C 回调栈帧，整个进程会 abort，所以在交给驱动之前先挡掉。
///
/// 只做协议判定：解析地址、TCP 建连、读包超时或提前关闭都放行，由驱动给出标准错误；
/// 因此探测本身不会让任何原本能成功的连接失败，最坏只多等一个探测预算。
fn ensure_mysql_classic_greeting(
    host: &str,
    port: u16,
    timeout: Duration,
) -> fluxdb_core::Result<()> {
    use std::io::Read;
    use std::net::{TcpStream, ToSocketAddrs};

    let Ok(mut addresses) = (host, port).to_socket_addrs() else {
        return Ok(());
    };
    let Some(address) = addresses.next() else {
        return Ok(());
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
        return Ok(());
    };
    if stream.set_read_timeout(Some(timeout)).is_err() {
        return Ok(());
    }
    // 经典包头是 3 字节小端长度 + 1 字节序号，第 5 字节才是 payload 首字节。
    let mut header = [0u8; 5];
    if stream.read_exact(&mut header).is_err() {
        return Ok(());
    }
    if matches!(
        header[4],
        MYSQL_HANDSHAKE_PROTOCOL_VERSION | MYSQL_ERR_PACKET
    ) {
        return Ok(());
    }

    let message = format!(
        "{host}:{port} 未返回 MySQL 握手包（首字节 0x{:02x}），请确认填写的是 MySQL 服务端口（默认 3306）；\
         MySQL X 协议端口（默认 33060）不能用于数据库连接",
        header[4]
    );
    tracing::warn!(
        target: "fluxdb_connectors",
        host,
        port,
        first_byte = header[4],
        "MySQL 端口协议探测失败"
    );
    Err(Error::new(ErrorKind::Connection, message))
}

/// 探测预算：够本地/正常链路拿到握手包，又不至于拖慢静默端口的建连（超时即放行给驱动）。
const MYSQL_GREETING_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

fn mysql_greeting_probe_timeout(connect_timeout: Duration) -> Duration {
    connect_timeout.min(MYSQL_GREETING_PROBE_TIMEOUT)
}

