// SSH 隧道基础设施：本地端口转发桥（MySQL / Redis / PostgreSQL 共享）。
//
// 与 `redis/ssh_tunnel.rs` 原实现相比，本提取版本：
// - 中性归属，不再置于 redis 目录；MySQL / Redis 通过 crate 根的 `SshTunnel` / `ssh_auth_from_options`
//   / `ssh_enabled` 原样复用（旧调用接口保持不变，兼容包装）；
// - 支持多连接（桥线程循环 accept，每次新建 direct-tcpip channel 并独立搬运），
//   取代原「只 accept 一次」的限制，让多个会话/取消通道可共享同一隧道 [R18]；
// - 可选 SSH 心跳（`keepalive_interval_secs`）与 SSH 建连超时；
// - 可选 hostkey 校验（`open_verified`）：未知/变更主机密钥在拨号期即拒绝，
//   不默认信任；旧 `open` 保持「首连即接受」，供 MySQL / Redis 兼容路径。
//
// 传输模型：跳板建立 SSH 会话 → 本地 `127.0.0.1:0` 起监听 → 每个入站连接开一条
// direct-tcpip channel 指向目标 host:port → 后台线程把「本地 socket ↔ channel」双向搬运。
// 上层（数据库驱动）连本地转发端口，隧道对象与连接同生命周期存活（Drop 断开发送）。
//
// 安全口径：密码 / 口令在 storage 层已进 Keychain 并以 inline 回填到 profile；私钥以文件路径
// 引用，不落盘明文。日志与错误消息不携带密码/证书正文。

use std::net::{TcpListener, TcpStream, ToSocketAddrs};

/// SSH 认证所需参数（从 resolved 配置解析而来）。
pub(crate) struct SshAuthParams {
    pub(crate) username: String,
    pub(crate) password: Option<String>,
    pub(crate) private_key_path: String,
    pub(crate) passphrase: Option<String>,
}

/// 隧道行为选项（默认兼容旧路径：单连接、无心跳、首连即接受）。
#[derive(Clone, Copy)]
pub(crate) struct SshTunnelOptions {
    /// SSH 建连超时；0 用默认 5s。
    pub(crate) connect_timeout_secs: u32,
    /// 心跳间隔（秒）；0 表示不发送。
    pub(crate) keepalive_interval_secs: u32,
    /// 是否校验远端主机密钥（`~/.ssh/known_hosts`）；false = 首连即接受。
    pub(crate) verify_host_key: bool,
}

impl Default for SshTunnelOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 5,
            keepalive_interval_secs: 0,
            verify_host_key: false,
        }
    }
}

/// 一座把数据库拨号桥接到远端目标端口的 SSH 隧道。
///
/// 实例被丢弃时通知桥线程停止监听、关闭转发 socket，并等待线程释放会话。
pub(crate) struct SshTunnel {
    /// 本地转发端口，上层连接 `127.0.0.1:<local_port>`。
    pub(crate) local_port: u16,
    /// 保持监听器存活；桥线程通过非阻塞轮询观察 shutdown。
    _listener: TcpListener,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    bridge: Option<std::thread::JoinHandle<()>>,
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.shutdown.store(true, std::sync::atomic::Ordering::Release);
        if let Some(bridge) = self.bridge.take() {
            let _ = bridge.join();
        }
    }
}

impl SshTunnel {
    /// 建隧道（不校验 hostkey），供 MySQL / Redis 兼容路径，行为与旧实现一致。
    pub(crate) fn open(
        jump: (&str, u16),
        auth: &SshAuthParams,
        target: (&str, u16),
    ) -> fluxdb_core::Result<SshTunnel> {
        open_tunnel_with(jump, auth, target, SshTunnelOptions::default())
    }
}

/// 建隧道，可指定建连超时 / 心跳 / hostkey 校验（PostgreSQL 传输路径使用）。
pub(crate) fn open_tunnel_with(
    jump: (&str, u16),
    auth: &SshAuthParams,
    target: (&str, u16),
    options: SshTunnelOptions,
) -> fluxdb_core::Result<SshTunnel> {
    let (jump_host, jump_port) = jump;
    let (target_host, target_port) = target;

    // 1. 连跳板机：遵循建连超时口径。
    let connect_timeout = std::time::Duration::from_secs(if options.connect_timeout_secs == 0 {
        5
    } else {
        u64::from(options.connect_timeout_secs)
    });
    let session_sock = {
        let mut addrs = (jump_host, jump_port)
            .to_socket_addrs()
            .map_err(|e| {
                fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Connection,
                    format!("SSH 跳板机地址解析失败: {e}"),
                )
            })?;
        let addr = addrs.next().ok_or_else(|| {
            fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, "SSH 跳板机地址解析失败")
        })?;
        TcpStream::connect_timeout(&addr, connect_timeout).map_err(|e| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("连接 SSH 跳板机失败: {e}"),
            )
        })?
    };

    // 2. SSH 握手与鉴权。
    let mut session = ssh2::Session::new().map_err(|e| {
        fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, format!("创建 SSH 会话失败: {e}"))
    })?;
    session.set_timeout(connect_timeout.as_millis().min(u32::MAX as u128) as u32);
    session.set_tcp_stream(session_sock);
    session.handshake().map_err(|e| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("SSH 握手失败: {e}"),
        )
    })?;

    // 可选 hostkey 校验：未知/变更主机密钥拒绝，不默认信任。
    if options.verify_host_key {
        verify_host_key(&session, jump_host, jump_port)?;
    }

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

    start_ssh_bridge(session, target_host, target_port, options.keepalive_interval_secs)
}

/// 已认证会话的监听与线程生命周期集中管理，调用方只持有 RAII 句柄。
fn start_ssh_bridge(
    session: ssh2::Session,
    target_host: &str,
    target_port: u16,
    keepalive_interval_secs: u32,
) -> fluxdb_core::Result<SshTunnel> {
    // 本地监听 + 桥线程循环 accept（每次开一条 direct-tcpip channel）。
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

    let handle = listener.try_clone().map_err(|e| {
        fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, e.to_string())
    })?;
    handle.set_nonblocking(true).map_err(|e| {
        fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, e.to_string())
    })?;
    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_for_bridge = shutdown.clone();
    let keepalive_interval = std::time::Duration::from_secs(
        u64::from(keepalive_interval_secs),
    );
    // 桥线程需跨线程存活的目标地址（owned）。
    let target_host_owned = target_host.to_string();
    let target_port_owned = target_port;

    // 桥线程接管 session/listener；session 在握手/鉴权后不再被主线程引用。
    let bridge = std::thread::spawn(move || {
        // 整个会话切非阻塞：搬运阶段两个方向要并行、各自短持会话锁，阻塞模式会互相饿死
        // （见 bridge_one 注释）。libssh2 的阻塞标志是会话级的，故在此统一设置一次。
        session.set_blocking(false);
        let mut since_keepalive = std::time::Instant::now();
        let mut connections: Vec<SshBridgeConnection> = Vec::new();
        loop {
            if shutdown_for_bridge.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            connections.retain(|connection| !connection.finished());
            // 心跳：长连接期间桥线程空闲于 accept，可周期发送。
            if !keepalive_interval.is_zero() && since_keepalive.elapsed() >= keepalive_interval {
                let _ = session.keepalive_send();
                since_keepalive = std::time::Instant::now();
            }
            match handle.accept() {
                Ok((local_sock, _)) => {
                    // 每条入站连接独立 channel，各自双向搬运；互不影响。
                    match open_direct_tcpip(&session, &target_host_owned, target_port_owned, &shutdown_for_bridge) {
                        Ok(channel) => {
                            if shutdown_for_bridge.load(std::sync::atomic::Ordering::Acquire) {
                                break;
                            }
                            match bridge_one(local_sock, channel, shutdown_for_bridge.clone()) {
                                Ok(connection) => connections.push(connection),
                                Err(error) => tracing::warn!(%error, "SSH 转发连接初始化失败"),
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "SSH 转发通道建立失败");
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => {
                    tracing::warn!(%error, "SSH 隧道监听退出");
                    break;
                }
            }
        }
        shutdown_for_bridge.store(true, std::sync::atomic::Ordering::Release);
        // 先关闭所有本地 socket 唤醒阻塞读写，再 join 搬运线程，确保会话随句柄释放。
        for connection in &connections {
            let _ = connection.socket.shutdown(std::net::Shutdown::Both);
        }
        drop(connections);
    });

    Ok(SshTunnel {
        local_port,
        _listener: listener,
        shutdown,
        bridge: Some(bridge),
    })
}

/// libssh2 `LIBSSH2_ERROR_EAGAIN`：非阻塞会话下「暂时无数据/还不能写」。
const SSH_EAGAIN: i32 = -37;

/// 该 SSH 错误是否为 EAGAIN（非阻塞重试信号，不是真失败）。
fn ssh_error_is_again(error: &ssh2::Error) -> bool {
    matches!(error.code(), ssh2::ErrorCode::Session(SSH_EAGAIN))
}

/// 打开一条指向远端目标的 direct-tcpip 通道。
///
/// 会话已切非阻塞，通道建立期间可能返回 EAGAIN（libssh2 尚未完成握手往返），
/// 故短退避重试；其它错误立即返回。
fn open_direct_tcpip(
    session: &ssh2::Session,
    target_host: &str,
    target_port: u16,
    shutdown: &std::sync::atomic::AtomicBool,
) -> Result<ssh2::Channel, ssh2::Error> {
    for _ in 0..BRIDGE_RETRY_LIMIT {
        if shutdown.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        match session.channel_direct_tcpip(target_host, target_port, None) {
            Ok(channel) => return Ok(channel),
            Err(error) if ssh_error_is_again(&error) => {
                std::thread::sleep(BRIDGE_RETRY_BACKOFF);
            }
            Err(error) => return Err(error),
        }
    }
    Err(ssh2::Error::from_errno(ssh2::ErrorCode::Session(SSH_EAGAIN)))
}

/// 非阻塞重试上限与退避间隔（通道建立 + 单次读写）。
const BRIDGE_RETRY_LIMIT: u32 = 20_000;
const BRIDGE_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(1);

/// 为一条入站本地连接 + 一条 channel 启动双向搬运线程。
///
/// SSH 会话被切为非阻塞：libssh2 的每次通道读写都要持会话锁，阻塞模式下先开始的方向会
/// 一直持锁到连接结束，另一方向永远拿不到锁（半双工死锁，PG 握手就会卡到建连超时）。
/// 非阻塞后每次调用立刻返回（无数据即 EAGAIN），两个方向各自按调用粒度取锁、交替推进。
/// `ssh2::Channel` 是 `Arc` 包装的共享句柄，可直接 clone 给两个线程。
struct SshBridgeConnection {
    socket: TcpStream,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl SshBridgeConnection {
    fn finished(&self) -> bool {
        self.workers.iter().all(|worker| worker.is_finished())
    }
}

impl Drop for SshBridgeConnection {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn bridge_one(
    local_sock: TcpStream,
    channel: ssh2::Channel,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::io::Result<SshBridgeConnection> {
    let mut sock_a = local_sock.try_clone()?;
    let mut sock_b = local_sock.try_clone()?;
    let mut chan_a = channel.clone();
    let mut chan_b = channel;
    let stop_a = shutdown.clone();
    // 本地 EOF 只发送半关闭，让远端仍可返回最后一段结果。
    let upstream = std::thread::spawn(move || {
        if let Err(error) = pump_local_to_remote(&mut sock_a, &mut chan_a, &stop_a) {
            tracing::debug!(%error, "SSH 上行转发结束");
        }
        // 非阻塞 send_eof 也可能返回 EAGAIN，不能丢失本地半关闭信号。
        for _ in 0..BRIDGE_RETRY_LIMIT {
            if stop_a.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            match chan_a.send_eof() {
                Ok(()) => break,
                Err(error) if ssh_error_is_again(&error) => {
                    std::thread::sleep(BRIDGE_RETRY_BACKOFF);
                }
                Err(error) => {
                    tracing::debug!(%error, "SSH 发送 EOF 失败");
                    break;
                }
            }
        }
    });
    let downstream = std::thread::spawn(move || {
        let _ = pump_remote_to_local(&mut chan_b, &mut sock_b, &shutdown);
        let _ = sock_b.shutdown(std::net::Shutdown::Both);
    });
    Ok(SshBridgeConnection { socket: local_sock, workers: vec![upstream, downstream] })
}

/// 本地 → 远端：读本地 socket（阻塞），写通道（非阻塞，满窗口时退避重试）。
fn pump_local_to_remote(
    sock: &mut TcpStream,
    channel: &mut ssh2::Channel,
    shutdown: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    let mut buffer = [0u8; 32 * 1024];
    loop {
        let read = match std::io::Read::read(sock, &mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let mut offset = 0;
        while offset < read {
            if shutdown.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            match std::io::Write::write(channel, &buffer[offset..read]) {
                Ok(0) => return Ok(()),
                Ok(written) => offset += written,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(BRIDGE_RETRY_BACKOFF);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// 远端 → 本地：读通道（非阻塞，无数据即退避），写本地 socket（阻塞）。
fn pump_remote_to_local(
    channel: &mut ssh2::Channel,
    sock: &mut TcpStream,
    shutdown: &std::sync::atomic::AtomicBool,
) -> std::io::Result<()> {
    let mut buffer = [0u8; 32 * 1024];
    loop {
        if shutdown.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        let read = match std::io::Read::read(channel, &mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(BRIDGE_RETRY_BACKOFF);
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        std::io::Write::write_all(sock, &buffer[..read])?;
    }
}

/// 不回退到当前工作目录，避免找不到用户目录时读取项目内不可信的 known_hosts。
fn ssh_known_hosts_path(
    override_path: Option<std::ffi::OsString>,
    home: Option<std::path::PathBuf>,
) -> fluxdb_core::Result<std::path::PathBuf> {
    if let Some(path) = override_path.filter(|path| !path.is_empty()) {
        return Ok(path.into());
    }
    home.filter(|path| path.is_absolute())
        .map(|path| path.join(".ssh").join("known_hosts"))
        .ok_or_else(|| fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            "无法定位用户目录，请设置 SSH_KNOWN_HOSTS 指向已知主机文件",
        ))
}

/// 校验远端主机密钥：读取 `~/.ssh/known_hosts`；未知或变更即拒绝。
///
/// 匹配口径与多数连接工具一致：按「主机:端口」精确匹配；未收录 → 拒绝（返回含指纹的错误，
/// 由上层决定是否引导用户信任），不默认信任。
fn verify_host_key(
    session: &ssh2::Session,
    host: &str,
    port: u16,
) -> fluxdb_core::Result<()> {
    let key = session
        .host_key()
        .map(|(key, _)| key.to_vec())
        .ok_or_else(|| {
            fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, "SSH 会话未提供主机密钥")
        })?;
    let fingerprint = host_key_fingerprint(session);

    // known_hosts 路径：优先 $SSH_KNOWN_HOSTS（测试可注入），否则 ~/.ssh/known_hosts。
    let file = ssh_known_hosts_path(std::env::var_os("SSH_KNOWN_HOSTS"), dirs::home_dir())?;
    // 仅当存在 known_hosts 文件时校验；无文件按「未知主机」拒绝，避免静默放行。
    if !file.exists() {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("SSH 主机密钥未知主机 {host}:{port}，指纹 {fingerprint}"),
        ));
    }

    let mut kh = session.known_hosts().map_err(|e| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("SSH known_hosts 初始化失败: {e}"),
        )
    })?;
    kh.read_file(&file, ssh2::KnownHostFileKind::OpenSSH).map_err(|error| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("读取 SSH known_hosts 失败（{}）：{error}", file.display()),
        )
    })?;

    match kh.check_port(host, port, &key) {
        ssh2::CheckResult::Match => Ok(()),
        ssh2::CheckResult::Mismatch => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("SSH 主机密钥变更，拒绝连接 {host}:{port}（指纹 {fingerprint}）"),
        )),
        ssh2::CheckResult::NotFound | ssh2::CheckResult::Failure => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("SSH 主机密钥未知主机 {host}:{port}，指纹 {fingerprint}"),
        )),
    }
}

/// 主机密钥指纹（SHA256 base64 形式，OpenSSH 风格）；取不到时给占位说明。
fn host_key_fingerprint(session: &ssh2::Session) -> String {
    session
        .host_key_hash(ssh2::HashType::Sha256)
        .map(|hash| {
            use std::fmt::Write;
            let mut out = String::with_capacity(hash.len() * 2);
            for byte in hash {
                let _ = write!(out, "{byte:02x}");
            }
            format!("sha256:{out}")
        })
        .unwrap_or_else(|| "不可用".to_string())
}

/// 从 resolved 配置的 `ssh_*` 键解析 SSH 认证参数。
pub(crate) fn ssh_auth_from_options(
    options: &std::collections::BTreeMap<String, String>,
    enabled: bool,
) -> Option<SshAuthParams> {
    if !enabled {
        return None;
    }
    let username = options.get("ssh_username").cloned().unwrap_or_default();
    let password = options
        .get("ssh_password")
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    let private_key_path = options.get("ssh_private_key_ref").cloned().unwrap_or_default();
    let passphrase = options
        .get("ssh_passphrase")
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
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

    #[test]
    fn known_hosts_uses_platform_home_without_cwd_fallback() {
        let home = std::env::temp_dir().join("用户 home");
        assert_eq!(ssh_known_hosts_path(None, Some(home.clone())).unwrap(),
            home.join(".ssh").join("known_hosts"));
        assert!(ssh_known_hosts_path(None, None).is_err());
        assert!(ssh_known_hosts_path(None, Some("relative".into())).is_err());
        let explicit = home.join("custom known_hosts");
        assert_eq!(ssh_known_hosts_path(Some(explicit.clone().into_os_string()), None).unwrap(), explicit);
    }

    #[test]
    fn dropping_idle_tunnel_releases_listener_and_bridge_thread() {
        // 不发起 SSH 通道；直接验证生产监听循环的退出，不依赖外部 sshd。
        let tunnel = start_ssh_bridge(ssh2::Session::new().unwrap(), "127.0.0.1", 1, 0).unwrap();
        let port = tunnel.local_port;
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            drop(tunnel);
            done_tx.send(()).unwrap();
        });
        done_rx.recv_timeout(std::time::Duration::from_secs(3)).expect("隧道 Drop 未退出");
        worker.join().unwrap();
        let _rebound = TcpListener::bind(("127.0.0.1", port)).expect("监听器未释放");
    }

    #[test]
    fn dropping_bridge_connection_wakes_blocked_socket_reader() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (socket, _) = listener.accept().unwrap();
        let mut reader = socket.try_clone().unwrap();
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let connection = SshBridgeConnection {
            socket,
            workers: vec![std::thread::spawn(move || {
                let mut buffer = [0u8; 1];
                let _ = std::io::Read::read(&mut reader, &mut buffer);
                read_tx.send(()).unwrap();
            })],
        };
        let (drop_tx, drop_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            drop(connection);
            drop_tx.send(()).unwrap();
        });
        read_rx.recv_timeout(std::time::Duration::from_secs(3)).expect("读线程未被唤醒");
        drop_rx.recv_timeout(std::time::Duration::from_secs(3)).expect("连接 Drop 未退出");
        worker.join().unwrap();
    }

    #[test]
    fn ssh_handshake_obeys_configured_timeout() {
        // TCP 接受连接但永不回复 SSH banner，隔离验证握手超时（不访问真实服务）。
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (_socket, _) = listener.accept().unwrap();
            let _ = release_rx.recv_timeout(std::time::Duration::from_secs(5));
        });
        let start = std::time::Instant::now();
        let result = open_tunnel_with(
            ("127.0.0.1", port),
            &SshAuthParams { username: "fixture".into(), password: None, private_key_path: String::new(), passphrase: None },
            ("127.0.0.1", 1),
            SshTunnelOptions { connect_timeout_secs: 1, ..Default::default() },
        );
        let elapsed = start.elapsed();
        let _ = release_tx.send(());
        server.join().unwrap();
        let error = match result { Ok(_) => panic!("无 SSH 响应不应成功"), Err(error) => error };
        assert!(error.to_string().contains("SSH 握手失败"), "{error}");
        assert!(elapsed < std::time::Duration::from_secs(4), "握手超时未生效: {elapsed:?}");
    }

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

    /// 真库门控：隧道双向搬运（回归「半双工死锁」）。
    ///
    /// 目标选跳板机自身的 sshd 端口：连上后 sshd 会主动发 `SSH-2.0-...` banner，因此
    /// **只要远端→本地这一个方向能回流数据，就证明桥没有死锁**——修复前通道被单方向
    /// 长期持锁，本地读会一直阻塞到超时。
    ///
    /// 环境：`FLUXDB_SSH_SMOKE=host:port:user:password`；未配置则跳过（不谎报通过）。
    #[test]
    fn ssh_tunnel_relays_both_directions() {
        let Some(value) = std::env::var("FLUXDB_SSH_SMOKE").ok() else {
            return;
        };
        let mut parts = value.split(':');
        let host = parts.next().unwrap_or_default().to_string();
        let port: u16 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let user = parts.next().unwrap_or_default().to_string();
        let password = parts.next().unwrap_or_default().to_string();
        if host.is_empty() || port == 0 {
            return;
        }
        let auth = SshAuthParams {
            username: user,
            password: Some(password),
            private_key_path: String::new(),
            passphrase: None,
        };
        // 目标 = 跳板机自己的 sshd（从跳板机视角解析 127.0.0.1）。
        let tunnel = open_tunnel_with(
            (&host, port),
            &auth,
            ("127.0.0.1", port),
            SshTunnelOptions::default(),
        )
        .expect("建隧道应成功");

        let mut stream = TcpStream::connect_timeout(
            &(std::net::Ipv4Addr::LOCALHOST, tunnel.local_port).into(),
            std::time::Duration::from_secs(5),
        )
        .expect("连隧道本地端口应成功");
        // 读 banner：读不到（死锁）就会在此超时失败，而不是静默通过。
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("设置读超时");
        // 逐字节读满版本串（banner 一次读不一定读全）。
        let mut banner = Vec::new();
        while !banner.ends_with(b"\n") {
            let mut byte = [0u8; 1];
            let read = std::io::Read::read(&mut stream, &mut byte).expect("远端→本地应有数据回流");
            assert_ne!(read, 0, "banner 未读完即 EOF");
            banner.push(byte[0]);
        }
        assert!(
            String::from_utf8_lossy(&banner).starts_with("SSH-2.0"),
            "应收到 sshd banner，实收 {:?}",
            String::from_utf8_lossy(&banner)
        );

        // 本地→远端：回自己的版本串完成协议交换，sshd 随后会发 KEXINIT（二进制，远大于
        // banner）。若该方向不通，sshd 收不到版本串，本地读就会超时——以此验证上行。
        std::io::Write::write_all(&mut stream, b"SSH-2.0-FluxDBTunnelProbe\r\n")
            .expect("本地→远端应能写入");
        let mut kexinit = [0u8; 32];
        let read = std::io::Read::read(&mut stream, &mut kexinit)
            .expect("版本串送达后 sshd 应回 KEXINIT（上行不通会在此超时）");
        assert!(read > 0, "应收到 KEXINIT 数据");
    }

    /// 真库门控：SSH 隧道桥转发到**真实 PG**，并走完整 tokio-postgres 建连 + SCRAM 认证 + 查询。
    ///
    /// 环境：`FLUXDB_SSH_PG_SMOKE=jumphost:port:user:pass:pg_host:pg_port:pg_user:pg_pass`；
    /// 未配置则跳过。复用 `open_tunnel_with`（App 同款 libssh2 桥），目标指 PG，然后让
    /// tokio-postgres（App 同款驱动栈）连隧道本地端口做真实握手。这直接复现 App「测试连接」
    /// 的完整路径——若此处报 `error communicating with the server`，即为桥/驱动层缺陷。
    #[test]
    fn ssh_tunnel_relays_to_real_pg() {
        let Some(Ok(value)) = std::env::var_os("FLUXDB_SSH_PG_SMOKE").map(|v| v.into_string()) else {
            return;
        };
        let p: Vec<&str> = value.split(':').collect();
        if p.len() < 8 {
            return;
        }
        let jh = p[0];
        let jp: u16 = p[1].parse().unwrap_or(0);
        let ju = p[2];
        let jpw = p[3];
        let pgh = p[4];
        let pgp: u16 = p[5].parse().unwrap_or(0);
        let pgu = p[6];
        let pgpw = p[7];
        if jh.is_empty() || jp == 0 || pgh.is_empty() || pgp == 0 {
            return;
        }
        let auth = SshAuthParams {
            username: ju.to_string(),
            password: Some(jpw.to_string()),
            private_key_path: String::new(),
            passphrase: None,
        };
        // App PG 路径强制 verify_host_key: true（读取 ~/.ssh/known_hosts），此处对标打开。
        let tunnel = open_tunnel_with(
            (jh, jp),
            &auth,
            (pgh, pgp),
            SshTunnelOptions {
                connect_timeout_secs: 5,
                keepalive_interval_secs: 0,
                verify_host_key: true,
                ..SshTunnelOptions::default()
            },
        )
        .expect("建隧道应成功");

        // App 同款：tokio-postgres 直连隧道本地端口（vv host 字段仅作 TLS SNI）。
        let mut cfg = tokio_postgres::Config::new();
        cfg.host("127.0.0.1")
            .port(tunnel.local_port)
            .user(pgu)
            .password(pgpw)
            .dbname("postgres");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let row: i32 = runtime.block_on(async {
            let (client, conn) = cfg
                .connect(tokio_postgres::NoTls)
                .await
                .expect("经隧道连 PG 并完成认证应成功（App 报 error communicating 的点）");
            // drive conn 在同一 runtime 后台，逐出用后即弃。
            tokio::spawn(conn);
            let row = client.query_one("SELECT 1", &[]).await.expect("查询应成功");
            row.get(0)
        });
        assert_eq!(row, 1);
    }

    /// 集成口径：SSH 开启但缺跳板机主机时，拨号入口应报可读中文错误而不是静默降级为直连。
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
            postgres_profile: None,
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
