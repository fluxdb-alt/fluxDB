// SSH 隧道基础设施：本地端口转发桥。
//
// 设计约束：整个 Redis 连接栈是同步阻塞式（`std::net::TcpStream` + `connect_timeout` + 同步池），
// 且 `RedisStream::Tls` 底层硬编码了 `std::net::TcpStream`。为避免改 rustls 包裹逻辑，
// SSH 隧道采用「本地端口转发桥」：
//   1. 与跳板机建立 SSH 会话（`ssh2`，同步）。
//   2. 在 `127.0.0.1:0` 起一个 `TcpListener`，开一条 direct-tcpip channel 指向目标 host:port。
//   3. Redis 侧连本地转发端口；后台桥线程把「本地 socket ↔ channel」双向搬运。
//
// 这样上层 `redis_tcp_connect` / TLS / 认证 / 连接池全部零改动，Cluster 重定向与 Sentinel 探测
// 只需走同一个拨号入口即可自动复用隧道。
//
// 生命周期：`SshTunnel` 跟随 `RedisConnection` 存活（被连接池复用期间不塌）；Redis 侧连接关闭时
// 桥线程读到 EOF 自动收敛并关闭 channel / 会话，无序手写 join。
//
// 安全口径：密码 / 口令在 storage 层已进 Keychain 并以 `inline` 回填到 profile，拨号时从 resolved
// 配置的 `ssh_*` 键读取；私钥以 `ssh_private_key_ref`（文件路径）引用，不落盘明文。hostkey 校验关闭
// （首连即接受），与多数连接工具默认一致，避免首次连接被未知主机密钥卡死。

use std::net::{TcpListener, TcpStream, ToSocketAddrs};

/// SSH 认证所需参数（从 resolved 配置解析而来）。
pub(crate) struct SshAuthParams {
    pub(crate) username: String,
    pub(crate) password: Option<String>,
    pub(crate) private_key_path: String,
    pub(crate) passphrase: Option<String>,
}

/// 一座用于把 Redis 拨号桥接到远端目标端口的 SSH 隧道。
///
/// 字段全部保持存活即可让隧道常开；实例被丢弃（连接丢弃）时桥线程因本地 EOF 自动收敛。
pub(crate) struct SshTunnel {
    /// 本地转发端口，Redis 侧连接 `127.0.0.1:<local_port>`。
    local_port: u16,
    /// 保持监听器存活；关闭时后续 accept 失败（桥线程已因 EOF 退出）。
    _listener: TcpListener,
    /// 桥线程句柄；Drop 时无需显式 join（线程自行因通道 EOF 退出）。
    _bridge: std::thread::JoinHandle<()>,
}

impl SshTunnel {
    /// 建立到 `jump` 的 SSH 隧道，把流量转发到 `target`。
    /// 返回本地转发端口，由调用方对 `127.0.0.1:port` 发起连接。
    pub(crate) fn open(
        jump: (&str, u16),
        auth: &SshAuthParams,
        target: (&str, u16),
    ) -> fluxdb_core::Result<SshTunnel> {
        let (jump_host, jump_port) = jump;

        // 1. 连跳板机：遵循与 Redis 直连一致的超时口径。
        let session_sock = {
            let mut addrs = (jump_host, jump_port)
                .to_socket_addrs()
                .map_err(|e| fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, format!("SSH 跳板机地址解析失败: {e}")))?;
            let addr = addrs.next().ok_or_else(|| {
                fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, "SSH 跳板机地址解析失败")
            })?;
            let stream = TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(5))
                .map_err(|e| {
                    fluxdb_core::Error::new(
                        fluxdb_core::ErrorKind::Connection,
                        format!("连接 SSH 跳板机失败: {e}"),
                    )
                })?;
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .ok();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(10)))
                .ok();
            stream
        };

        // 2. SSH 握手与鉴权。
        let mut session = ssh2::Session::new().map_err(|e| {
            fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, format!("创建 SSH 会话失败: {e}"))
        })?;
        session.set_tcp_stream(session_sock);
        session.handshake().map_err(|e| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("SSH 握手失败: {e}"),
            )
        })?;

        if !auth.private_key_path.is_empty() {
            session
                .userauth_pubkey_file(
                    &auth.username,
                    None,
                    std::path::Path::new(&auth.private_key_path),
                    auth.passphrase.as_deref(),
                )
                .map_err(|e| {
                    fluxdb_core::Error::new(
                        fluxdb_core::ErrorKind::Connection,
                        format!("SSH 私钥认证失败: {e}"),
                    )
                })?;
        } else {
            let password = auth.password.clone().unwrap_or_default();
            session.userauth_password(&auth.username, &password).map_err(|e| {
                fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Connection,
                    format!("SSH 密码认证失败: {e}"),
                )
            })?;
        }
        if !session.authenticated() {
            return Err(fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                "SSH 认证未通过",
            ));
        }

        // 3. 本地监听 + direct-tcpip channel 指向目标端。
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("SSH 隧道本地端口绑定失败: {e}"),
            )
        })?;
        let local_port = listener.local_addr().map_err(|e| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("SSH 隧道本地端口读取失败: {e}"),
            )
        })?.port();

        let (target_host, target_port) = target;
        let channel = session.channel_direct_tcpip(target_host, target_port, None).map_err(|e| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("SSH 隧道建立到目标 {target_host}:{target_port} 失败: {e}"),
            )
        })?;

        let handle = listener
            .try_clone()
            .map_err(|e| fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, e.to_string()))?;
        // 桥线程接管 session/channel/监听器，接受一次连接后双向搬运。
        let bridge = std::thread::spawn(move || {
            match handle.accept() {
                Ok((redis_sock, _)) => {
                    let channel = std::sync::Arc::new(std::sync::Mutex::new(channel));
                    let (chan_a, chan_b) = (channel.clone(), channel.clone());
                    let (sock_a, sock_b) = (redis_sock.try_clone(), redis_sock);
                    // 方向1: 本地 → 远端
                    let t1 = std::thread::spawn(move || {
                        let mut redis_sock = sock_a.ok()?;
                        let mut channel = chan_a.lock().ok()?;
                        let _ = std::io::copy(&mut redis_sock, &mut *channel);
                        Some(())
                    });
                    // 方向2: 远端 → 本地
                    let t2 = std::thread::spawn(move || {
                        let mut redis_sock = sock_b;
                        let mut channel = chan_b.lock().ok()?;
                        let _ = std::io::copy(&mut *channel, &mut redis_sock);
                        Some(())
                    });
                    // 任一方向结束后，另一方向会因通道/本地关闭而 EOF 退出。
                    let _ = t1.join();
                    let _ = t2.join();
                }
                Err(_) => {
                    // 本地侧先关闭：直接结束。
                }
            }
        });
        // 释放 session 所有权给桥线程。session 在握手/鉴权后不再被主线程引用。
        drop(session);

        Ok(SshTunnel {
            local_port,
            _listener: listener,
            _bridge: bridge,
        })
    }
}

/// 从 resolved 配置的 `ssh_*` 键解析 SSH 认证参数。
///
/// 凭据取值口径：密码/口令优先取内联（storage 已从 Keychain 回填），私钥是文件路径引用。
pub(crate) fn ssh_auth_from_options(options: &std::collections::BTreeMap<String, String>, enabled: bool) -> Option<SshAuthParams> {
    if !enabled {
        return None;
    }
    let username = options.get("ssh_username").cloned().unwrap_or_default();
    let password = options.get("ssh_password").map(String::as_str).filter(|p| !p.is_empty()).map(str::to_string);
    let private_key_path = options.get("ssh_private_key_ref").cloned().unwrap_or_default();
    let passphrase = options.get("ssh_passphrase").map(String::as_str).filter(|p| !p.is_empty()).map(str::to_string);
    Some(SshAuthParams {
        username,
        password,
        private_key_path,
        passphrase,
    })
}

/// 判断 resolved 配置是否启用了 SSH 隧道。
pub(crate) fn ssh_enabled(options: &std::collections::BTreeMap<String, String>) -> bool {
    options
        .get("ssh_enabled")
        .map(|v| v.trim().to_ascii_lowercase())
        .is_some_and(|v| matches!(v.as_str(), "true" | "y" | "yes" | "1" | "on"))
}

#[cfg(test)]
mod ssh_tunnel_tests {
    use super::*;

    fn options(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn detects_ssh_enabled_variants() {
        for v in ["true", "YES", "1", "on", "y"] {
            assert!(ssh_enabled(&options(&[("ssh_enabled", v)])), "expected {v} enabled");
        }
        for v in ["false", "0", "", "no"] {
            assert!(!ssh_enabled(&options(&[("ssh_enabled", v)])), "expected {v} disabled");
        }
        assert!(!ssh_enabled(&options(&[])));
    }

    #[test]
    fn parses_password_auth_params() {
        let o = options(&[
            ("ssh_enabled", "true"),
            ("ssh_username", "jumpuser"),
            ("ssh_password", "p@ss"),
            ("ssh_private_key_ref", ""),
        ]);
        let auth = ssh_auth_from_options(&o, true).unwrap();
        assert_eq!(auth.username, "jumpuser");
        assert_eq!(auth.password.as_deref(), Some("p@ss"));
        assert!(auth.private_key_path.is_empty());
        assert_eq!(auth.passphrase, None);
    }

    #[test]
    fn parses_private_key_params() {
        let o = options(&[
            ("ssh_enabled", "true"),
            ("ssh_username", "jumpuser"),
            ("ssh_private_key_ref", "/Users/x/.ssh/id_ed25519"),
            ("ssh_passphrase", "secret"),
        ]);
        let auth = ssh_auth_from_options(&o, true).unwrap();
        assert_eq!(auth.private_key_path, "/Users/x/.ssh/id_ed25519");
        assert_eq!(auth.passphrase.as_deref(), Some("secret"));
        assert_eq!(auth.password, None);
    }

    #[test]
    fn disabled_ssh_yields_no_auth() {
        assert!(ssh_auth_from_options(&options(&[]), false).is_none());
    }

    /// 集成口径：SSH 开启但缺跳板机主机时，`redis_dial_endpoint` 应报可读中文错误，
    /// 而不是去解析本地地址或静默降级为直连。
    #[test]
    fn dial_endpoint_rejects_empty_jump_host() {
        let config = fluxdb_core::ConnectionConfig {
            id: fluxdb_core::ConnectionId(1),
            name: "t".to_string(),
            kind: fluxdb_core::DatabaseKind::Redis,
            endpoint: fluxdb_core::Endpoint::Tcp {
                host: "redis.internal".to_string(),
                port: 6379,
                database: None,
            },
            credential_ref: None,
            options: options(&[("ssh_enabled", "true"), ("ssh_username", "u")]),
            redis_profile: None,
            mysql_profile: None,
        };
        let result = redis_dial_endpoint(&config, "redis.internal", 6379);
        let Err(err) = result else {
            panic!("expected error for missing jump host");
        };
        assert!(
            err.message.contains("跳板机"),
            "got unexpected error message: {}",
            err.message
        );
    }
}

