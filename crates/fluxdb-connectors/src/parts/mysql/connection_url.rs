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

// 探测逻辑的离线测试放在本文件，避免继续膨胀 `parts/tests.rs`。
#[cfg(test)]
mod greeting_probe_tests {
    use super::*;
    use std::io::Write;
    use std::net::{Shutdown, TcpListener};

    enum FakeEndpoint {
        /// 建连后立刻回一段字节（模拟对端协议的第一个包）。
        Replies(&'static [u8]),
        /// 建连后什么都不发（多数非 MySQL 服务的行为）。
        Silent,
        /// 建连后直接关闭。
        Closes,
    }

    /// 起一个只服务一次探测连接的假端口，返回其端口号。
    fn fake_endpoint(mode: FakeEndpoint) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("假端口应可监听");
        let port = listener.local_addr().expect("假端口应有本地地址").port();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            match mode {
                FakeEndpoint::Replies(bytes) => {
                    // 先 shutdown(Write) 发 FIN，保证数据送达后再释放 socket。
                    let _ = stream.write_all(bytes);
                    let _ = stream.shutdown(Shutdown::Write);
                }
                FakeEndpoint::Silent => std::thread::sleep(Duration::from_secs(3)),
                FakeEndpoint::Closes => {}
            }
        });
        port
    }

    fn probe(port: u16, timeout: Duration) -> fluxdb_core::Result<()> {
        ensure_mysql_classic_greeting("127.0.0.1", port, timeout)
    }

    #[test]
    fn x_protocol_endpoint_is_rejected_before_driver_handshake() {
        // mysqlx 建连后立即回的帧头：交给 sqlx 会被当握手包解析并在空缓冲上 panic。
        let port = fake_endpoint(FakeEndpoint::Replies(&[
            0x05, 0x00, 0x00, 0x00, 0x0b, 0x08, 0x05, 0x1a, 0x00,
        ]));

        let error = probe(port, Duration::from_secs(2)).expect_err("X 协议端口应被探测拦下");

        assert_eq!(error.kind, ErrorKind::Connection);
        assert!(
            error.message.contains("未返回 MySQL 握手包"),
            "错误信息应说明协议不符: {}",
            error.message
        );
        assert!(
            error.message.contains("33060"),
            "错误信息应提示 X 协议端口: {}",
            error.message
        );
    }

    #[test]
    fn classic_greeting_and_error_packet_pass_probe() {
        // 8.0 服务端握手包：长度 0x4a + 序号 0 + payload 首字节 0x0a（协议版本 10）。
        let greeting = fake_endpoint(FakeEndpoint::Replies(&[
            0x4a, 0x00, 0x00, 0x00, 0x0a, b'8', b'.', b'0', 0x00,
        ]));
        // host 被封等场景服务端直接回 ERR 包（payload 首字节 0xff），交由驱动报错。
        let err_packet = fake_endpoint(FakeEndpoint::Replies(&[
            0x10, 0x00, 0x00, 0x00, 0xff, 0x28, 0x00, 0x00, 0x00,
        ]));

        assert!(probe(greeting, Duration::from_secs(2)).is_ok());
        assert!(probe(err_packet, Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn undecidable_endpoints_pass_probe_to_driver() {
        // 静默 / 建连即关 / 无监听：探测不裁定，由驱动给出标准连接错误。
        let silent = fake_endpoint(FakeEndpoint::Silent);
        let closed = fake_endpoint(FakeEndpoint::Closes);

        assert!(probe(silent, Duration::from_millis(300)).is_ok());
        assert!(probe(closed, Duration::from_secs(2)).is_ok());
        assert!(probe(1, Duration::from_millis(300)).is_ok());
    }

    #[test]
    fn probe_budget_is_capped_below_connect_timeout() {
        assert_eq!(
            mysql_greeting_probe_timeout(Duration::from_secs(30)),
            Duration::from_secs(2)
        );
        assert_eq!(
            mysql_greeting_probe_timeout(Duration::from_millis(500)),
            Duration::from_millis(500)
        );
    }
}

