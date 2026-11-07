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

/// 隧道行为选项。
///
/// 默认校验远端主机密钥：所有数据库共用同一套拨号路径，不能因为某个
/// 连接器漏传选项就静默退化。测试通过 `SSH_KNOWN_HOSTS` 注入已知主机。
#[derive(Clone, Copy)]
pub(crate) struct SshTunnelOptions {
    /// SSH 建连超时；0 用默认 5s。
    pub(crate) connect_timeout_secs: u32,
    /// 心跳间隔（秒）；0 表示不发送。
    pub(crate) keepalive_interval_secs: u32,
    /// 远端主机密钥校验策略：严格 / 首次信任 / 不校验。
    pub(crate) host_key_policy: fluxdb_core::SshHostKeyPolicy,
}

impl Default for SshTunnelOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 5,
            keepalive_interval_secs: 0,
            host_key_policy: fluxdb_core::SshHostKeyPolicy::Strict,
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

    // hostkey 校验：严格档未知/变更即拒绝；首次信任档只放行未知主机；
    // 不校验档跳过（仅调试用途）。
    if options.host_key_policy != fluxdb_core::SshHostKeyPolicy::Ignore {
        verify_host_key(&session, jump_host, jump_port, options.host_key_policy)?;
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
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl SshBridgeConnection {
    fn finished(&self) -> bool {
        self.workers.iter().all(|worker| worker.is_finished())
    }
}

impl Drop for SshBridgeConnection {
    fn drop(&mut self) {
        self.stopped.store(true, std::sync::atomic::Ordering::Release);
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
    // Windows shutdown 不唤醒已阻塞的 recv；两个方向均用非阻塞 I/O 观察取消。
    local_sock.set_nonblocking(true)?;
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped_a = stopped.clone();
    let stopped_b = stopped.clone();
    let mut sock_a = local_sock.try_clone()?;
    let mut sock_b = local_sock.try_clone()?;
    let mut chan_a = channel.clone();
    let mut chan_b = channel;
    let stop_a = shutdown.clone();
    // 本地 EOF 只发送半关闭，让远端仍可返回最后一段结果。
    let upstream = std::thread::spawn(move || {
        let canceled = || stop_a.load(std::sync::atomic::Ordering::Acquire)
            || stopped_a.load(std::sync::atomic::Ordering::Acquire);
        if let Err(error) = pump_ssh_bytes(&mut sock_a, &mut chan_a, canceled) {
            tracing::debug!(%error, "SSH 上行转发结束");
            stopped_a.store(true, std::sync::atomic::Ordering::Release);
            return;
        }
        // 非阻塞 send_eof 也可能返回 EAGAIN，不能丢失本地半关闭信号。
        for _ in 0..BRIDGE_RETRY_LIMIT {
            if canceled() {
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
        let canceled = || shutdown.load(std::sync::atomic::Ordering::Acquire)
            || stopped_b.load(std::sync::atomic::Ordering::Acquire);
        if let Err(error) = pump_ssh_bytes(&mut chan_b, &mut sock_b, canceled) {
            tracing::debug!(%error, "SSH 下行转发结束");
        }
        stopped_b.store(true, std::sync::atomic::Ordering::Release);
        let _ = sock_b.shutdown(std::net::Shutdown::Both);
    });
    Ok(SshBridgeConnection { socket: local_sock, stopped, workers: vec![upstream, downstream] })
}

/// 双向共用非阻塞字节搬运：保留部分写入偏移，背压时重试，并在每次 I/O 前检查取消。
/// Read/Write 使用标准 trait，测试可隔离注入背压、短写和中途取消。
fn pump_ssh_bytes(
    source: &mut impl std::io::Read,
    destination: &mut impl std::io::Write,
    canceled: impl Fn() -> bool,
) -> std::io::Result<()> {
    let mut buffer = [0u8; 32 * 1024];
    loop {
        if canceled() { return Ok(()); }
        let read = match source.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(BRIDGE_RETRY_BACKOFF);
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let mut offset = 0;
        while offset < read {
            if canceled() { return Ok(()); }
            match destination.write(&buffer[offset..read]) {
                Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "SSH 转发写入未推进")),
                Ok(written) => offset += written,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(BRIDGE_RETRY_BACKOFF);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
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

/// 校验远端主机密钥。
///
/// 匹配口径与多数连接工具一致：按「主机:端口」精确匹配。
/// - 严格档：未知主机或指纹变更一律拒绝，错误里带指纹，便于用户手工核对。
/// - 首次信任档：未知主机记录指纹到 known_hosts 后继续；**已记录但指纹变更仍拒绝**，
///   避免把"第一次信任"变成"永远信任"。
/// - 不校验档：在调用方跳过本函数。
fn verify_host_key(
    session: &ssh2::Session,
    host: &str,
    port: u16,
    policy: fluxdb_core::SshHostKeyPolicy,
) -> fluxdb_core::Result<()> {
    let (key, key_type) = session
        .host_key()
        .map(|(key, key_type)| (key.to_vec(), key_type))
        .ok_or_else(|| {
            fluxdb_core::Error::new(fluxdb_core::ErrorKind::Connection, "SSH 会话未提供主机密钥")
        })?;
    let fingerprint = host_key_fingerprint(session);
    let accept_new = policy == fluxdb_core::SshHostKeyPolicy::AcceptNew;

    // known_hosts 路径：优先 $SSH_KNOWN_HOSTS（测试可注入），否则 ~/.ssh/known_hosts。
    let file = ssh_known_hosts_path(std::env::var_os("SSH_KNOWN_HOSTS"), dirs::home_dir())?;
    // 无文件按「未知主机」处理：严格档拒绝，首次信任档创建记录。
    if !file.exists() {
        if accept_new {
            return trust_new_host(session, &file, host, port, &key, key_type, &fingerprint);
        }
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
        ssh2::CheckResult::Failure => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("SSH known_hosts 校验失败 {host}:{port}（指纹 {fingerprint}）"),
        )),
        ssh2::CheckResult::NotFound if accept_new => {
            trust_new_host(session, &file, host, port, &key, key_type, &fingerprint)
        }
        ssh2::CheckResult::NotFound => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("SSH 主机密钥未知主机 {host}:{port}，指纹 {fingerprint}"),
        )),
    }
}

/// 首次信任：把主机密钥写入 known_hosts 后放行。
///
/// 写入失败必须报错而不是静默继续：否则用户以为已信任，下次连接仍会被拦，
/// 且无法判断首次信任是否真正生效。
fn trust_new_host(
    session: &ssh2::Session,
    file: &std::path::Path,
    host: &str,
    port: u16,
    key: &[u8],
    key_type: ssh2::HostKeyType,
    fingerprint: &str,
) -> fluxdb_core::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("创建 SSH 目录失败（{}）：{error}", parent.display()),
            )
        })?;
    }
    let mut kh = session.known_hosts().map_err(|e| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("SSH known_hosts 初始化失败: {e}"),
        )
    })?;
    // 已存在的文件先读入，写回时才不会丢掉其他主机记录。
    if file.exists() {
        kh.read_file(file, ssh2::KnownHostFileKind::OpenSSH)
            .map_err(|error| {
                fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Connection,
                    format!("读取 SSH known_hosts 失败（{}）：{error}", file.display()),
                )
            })?;
    }
    // OpenSSH 非默认端口写作 `[host]:port`，与 check_port 的匹配口径一致。
    let entry = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    kh.add(
        &entry,
        key,
        "fluxdb first-use",
        ssh2::KnownHostKeyFormat::from(key_type),
    )
    .map_err(|error| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("写入 SSH known_hosts 失败（{}）：{error}", file.display()),
        )
    })?;
    kh.write_file(file, ssh2::KnownHostFileKind::OpenSSH)
        .map_err(|error| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("保存 SSH known_hosts 失败（{}）：{error}", file.display()),
            )
        })?;
    tracing::info!(
        target: "fluxdb_connectors",
        host,
        port,
        fingerprint,
        "已按首次信任策略记录 SSH 主机密钥"
    );
    Ok(())
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

/// 按结构化 SSH 档案建隧道（MongoDB / SQL Server / Oracle / 达梦共用）。
///
/// `default_connect_timeout_secs` 在档案未显式配置超时时兜底；hostkey 一律校验，
/// 与 PostgreSQL 传输路径保持一致，避免静默信任未知主机。
pub(crate) fn open_tunnel_for_profile(
    ssh: &fluxdb_core::SshTunnelProfile,
    target: (&str, u16),
    default_connect_timeout_secs: u32,
) -> fluxdb_core::Result<SshTunnel> {
    if !ssh.enabled {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            "SSH 隧道未启用",
        ));
    }
    if ssh.host.trim().is_empty() {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            "请填写 SSH 隧道主机",
        ));
    }
    let auth = SshAuthParams {
        username: ssh.username.clone(),
        password: ssh.password.value().map(str::to_string),
        private_key_path: ssh.private_key.key.clone(),
        passphrase: ssh.passphrase.value().map(str::to_string),
    };
    let options = SshTunnelOptions {
        connect_timeout_secs: if ssh.connect_timeout_secs > 0 {
            ssh.connect_timeout_secs
        } else {
            default_connect_timeout_secs
        },
        keepalive_interval_secs: ssh.keepalive_interval_secs,
        host_key_policy: ssh.host_key_policy,
    };
    open_tunnel_with((ssh.host.trim(), ssh.port), &auth, target, options)
}

/// 判断 resolved 配置是否启用了 SSH 隧道。
pub(crate) fn ssh_enabled(options: &std::collections::BTreeMap<String, String>) -> bool {
    options
        .get("ssh_enabled")
        .map(|v| v.trim().to_ascii_lowercase())
        .is_some_and(|v| matches!(v.as_str(), "true" | "y" | "yes" | "1" | "on"))
}

