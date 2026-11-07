// 共享代理拨号（SOCKS5 / HTTP CONNECT）与本地转发器。
//
// PostgreSQL 能把代理后的裸流直接交给驱动（`connect_raw`），MySQL 的驱动只能自己
// 建连，因此额外提供 `LocalForwarder`：本地 127.0.0.1 随机端口接住驱动连接，再按
// 代理/直连方式拨到真实目标并双向搬运。两条路径共用同一套代理握手，凭据只在首包
// 出现，不写日志。

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream as ProxyTcpStream};
use tokio::task::JoinHandle;

/// 代理协议类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProxyKind {
    Socks5,
    HttpConnect,
}

impl ProxyKind {
    fn default_port(self) -> u16 {
        match self {
            ProxyKind::Socks5 => 1080,
            ProxyKind::HttpConnect => 8080,
        }
    }
}

/// 代理端点与凭据。`port == 0` 时使用协议默认端口。
///
/// 刻意不派生 `Debug`：含明文密码，避免被顺手打进日志。
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ProxySpec {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl ProxySpec {
    fn effective_port(&self) -> u16 {
        if self.port != 0 {
            self.port
        } else {
            self.kind.default_port()
        }
    }

    fn display_endpoint(&self) -> String {
        format!("{}:{}", self.host, self.effective_port())
    }
}

/// 拨代理，拿到通往 `target` 的裸 TCP 流。
///
/// `timeout` 覆盖「连接代理 + 握手」整体；`keepalive` 只作用于到代理的这一跳。
pub(crate) async fn proxy_tcp_stream(
    spec: &ProxySpec,
    target: (&str, u16),
    timeout: Duration,
    keepalive: bool,
) -> fluxdb_core::Result<ProxyTcpStream> {
    let endpoint = spec.display_endpoint();
    let run = async {
        let mut stream = dial_tcp(&spec.host, spec.effective_port(), keepalive, timeout).await?;
        match spec.kind {
            ProxyKind::Socks5 => socks5_handshake(&mut stream, target, spec).await,
            ProxyKind::HttpConnect => http_connect_handshake(&mut stream, target, spec).await,
        }
        .map_err(|error| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("代理握手失败 ({endpoint}): {error}"),
            )
        })?;
        Ok(stream)
    };
    match tokio::time::timeout(timeout, run).await {
        Ok(result) => result,
        Err(_) => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Timeout,
            format!("代理 {endpoint} 连接超时"),
        )),
    }
}

/// 按目标建一条 TCP 连接；`keepalive` 打开 `SO_KEEPALIVE`。
async fn dial_tcp(
    host: &str,
    port: u16,
    keepalive: bool,
    timeout: Duration,
) -> fluxdb_core::Result<ProxyTcpStream> {
    let mut addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("解析代理地址 {host}:{port} 失败: {error}"),
            )
        })?;
    let address = addresses.next().ok_or_else(|| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("代理地址 {host}:{port} 无可用解析结果"),
        )
    })?;
    let socket = if address.is_ipv4() {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    }
    .map_err(|error| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("创建代理套接字失败: {error}"),
        )
    })?;
    if keepalive {
        let _ = socket.set_keepalive(true);
    }
    match tokio::time::timeout(timeout, socket.connect(address)).await {
        Ok(Ok(stream)) => {
            let _ = stream.set_nodelay(true);
            Ok(stream)
        }
        Ok(Err(error)) => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("连接代理 {host}:{port} 失败: {error}"),
        )),
        Err(_) => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Timeout,
            format!("连接代理 {host}:{port} 超时"),
        )),
    }
}

/// SOCKS5：greeting →（可选 user/pass 认证）→ CONNECT。
async fn socks5_handshake(
    stream: &mut ProxyTcpStream,
    target: (&str, u16),
    spec: &ProxySpec,
) -> std::io::Result<()> {
    let has_creds = !spec.username.is_empty();
    // greeting：NO AUTH(0x00) 与 USER/PASS(0x02)。
    stream
        .write_all(&[0x05, if has_creds { 2 } else { 1 }, 0x00, 0x02])
        .await?;
    let mut greets = [0u8; 2];
    stream.read_exact(&mut greets).await?;
    if greets[0] != 0x05 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SOCKS5 版本不支持",
        ));
    }
    match greets[1] {
        0x00 => {}
        0x02 => {
            let user = spec.username.as_bytes();
            let pass = spec.password.as_bytes();
            let mut auth = Vec::with_capacity(3 + user.len() + pass.len());
            auth.push(0x01);
            auth.push(user.len() as u8);
            auth.extend_from_slice(user);
            auth.push(pass.len() as u8);
            auth.extend_from_slice(pass);
            stream.write_all(&auth).await?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await?;
            if status[0] != 0x01 || status[1] != 0x00 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "SOCKS5 代理认证失败",
                ));
            }
        }
        m => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("SOCKS5 不接受可用认证方法 ({m})"),
            ))
        }
    }

    // CONNECT 请求 + 目标地址。
    let mut req = Vec::new();
    req.push(0x05); // VER
    req.push(0x01); // CONNECT
    req.push(0x00); // RSV
    socks_append_addr(&mut req, target.0)?;
    req.extend_from_slice(&target.1.to_be_bytes());
    stream.write_all(&req).await?;

    // 响应：VER REP RSV ATYP 地址 端口；REP 0 成功。
    let mut rep = [0u8; 4];
    stream.read_exact(&mut rep).await?;
    if rep[0] != 0x05 || rep[1] != 0x00 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("SOCKS5 连接目标失败 (REP={})", rep[1]),
        ));
    }
    let addr_len = match rep[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            len[0] as usize
        }
        at => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("SOCKS5 未知地址类型 ({at})"),
            ))
        }
    };
    let mut _bnd = vec![0u8; addr_len + 2];
    stream.read_exact(&mut _bnd).await?;
    Ok(())
}

/// 按目标地址类型拼 SOCKS5 地址字段（域名 / IPv4 / IPv6）。
fn socks_append_addr(req: &mut Vec<u8>, host: &str) -> std::io::Result<()> {
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        req.push(0x01);
        req.extend_from_slice(&ip.octets());
        return Ok(());
    }
    if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        req.push(0x04);
        req.extend_from_slice(&ip.octets());
        return Ok(());
    }
    if host.len() > 255 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "SOCKS5 目标主机名过长",
        ));
    }
    req.push(0x03);
    req.push(host.len() as u8);
    req.extend_from_slice(host.as_bytes());
    Ok(())
}

/// HTTP CONNECT：发送隧道请求，直到读到响应头，校验 200。
async fn http_connect_handshake(
    stream: &mut ProxyTcpStream,
    target: (&str, u16),
    spec: &ProxySpec,
) -> std::io::Result<()> {
    let mut head = format!(
        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n",
        target.0, target.1, target.0, target.1
    );
    if !spec.username.is_empty() {
        use base64::Engine as _;
        let cred = format!("{}:{}", spec.username, spec.password);
        let token = base64::engine::general_purpose::STANDARD.encode(cred);
        head.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;

    // 读响应头：累加到 \r\n\r\n。
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await?;
        buf.push(byte[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if buf.len() > 8192 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "代理响应头过大",
            ));
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let status_line = text.lines().next().unwrap_or("");
    if !status_line.starts_with("HTTP/1.")
        || !status_line
            .split_whitespace()
            .nth(1)
            .is_some_and(|code| code == "200")
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("HTTP CONNECT 被代理拒绝: {status_line}"),
        ));
    }
    Ok(())
}

/// 本地转发器拨号参数。
#[derive(Clone)]
pub(crate) enum ForwardDial {
    /// 直连目标；`keepalive` 打开 `SO_KEEPALIVE`。
    Direct {
        host: String,
        port: u16,
        keepalive: bool,
    },
    /// 经代理拨目标；代理跳同样可开 `SO_KEEPALIVE`。
    Proxy {
        spec: ProxySpec,
        target: (String, u16),
        keepalive: bool,
    },
}

impl ForwardDial {
    async fn dial(&self, timeout: Duration) -> fluxdb_core::Result<ProxyTcpStream> {
        match self {
            ForwardDial::Direct {
                host,
                port,
                keepalive,
            } => dial_tcp(host, *port, *keepalive, timeout).await,
            ForwardDial::Proxy {
                spec,
                target,
                keepalive,
            } => proxy_tcp_stream(spec, (target.0.as_str(), target.1), timeout, *keepalive).await,
        }
    }
}

/// 只监听 127.0.0.1 的转发器；Drop 时停止监听并断开后续连接。
pub(crate) struct LocalForwarder {
    pub(crate) local_port: u16,
    /// 最近一次拨号失败原因：驱动侧只看到连接被关闭，需要它给出可诊断的错误。
    last_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    task: JoinHandle<()>,
}

impl LocalForwarder {
    /// 取走最近一次拨号失败原因（若有）。
    pub(crate) fn take_last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|mut slot| slot.take())
    }
}

impl Drop for LocalForwarder {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 启动本地转发器：每条进入的连接单独拨目标并双向搬运。
///
/// 失败只影响该条连接（驱动侧看到 EOF/重置），监听器继续服务。
pub(crate) async fn start_local_forwarder(
    dial: ForwardDial,
    timeout: Duration,
) -> fluxdb_core::Result<LocalForwarder> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("启动本地转发端口失败: {error}"),
            )
        })?;
    let local_port = listener
        .local_addr()
        .map_err(|error| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("读取转发端口失败: {error}"),
            )
        })?
        .port();
    let last_error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let error_slot = last_error.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut inbound, _)) = listener.accept().await else {
                continue;
            };
            let dial = dial.clone();
            let last_error = error_slot.clone();
            tokio::spawn(async move {
                match dial.dial(timeout).await {
                    Ok(mut outbound) => {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "fluxdb_connectors",
                            error = %error.message,
                            "本地转发拨号失败"
                        );
                        if let Ok(mut slot) = last_error.lock() {
                            *slot = Some(error.message.clone());
                        }
                    }
                }
            });
        }
    });
    Ok(LocalForwarder {
        local_port,
        last_error,
        task,
    })
}
