// PostgreSQL 代理拨号（T05）：SOCKS5 / HTTP CONNECT，把代理后的裸 TCP 流交给 `connect_raw`。
//
// 认证信息来自 profile（`SecretRef`），正文不落日志；建连受整体超时约束。

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 拨代理拿到通往目标的裸 TCP 流。
///
/// `target`：目标主机 + 端口（隧道内直连目标）。`timeout`：代理建连 + 握手整体超时。
async fn pg_proxy_connect(
    proxy: &fluxdb_core::PostgresProxy,
    target: (&str, u16),
    timeout: std::time::Duration,
) -> fluxdb_core::Result<tokio::net::TcpStream> {
    let port = if proxy.port != 0 {
        proxy.port
    } else {
        proxy.proxy_type.default_port()
    };
    let mut stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect((proxy.host.as_str(), port)))
        .await
        .map_err(|_| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Timeout,
                format!("连接代理 {}:{} 超时", proxy.host, port),
            )
        })?
        .map_err(|e| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("连接代理 {}:{} 失败: {e}", proxy.host, port),
            )
        })?;
    let _ = stream.set_nodelay(true);

    let run = match proxy.proxy_type {
        fluxdb_core::PostgresProxyType::Socks5 => {
            pg_socks5_handshake(&mut stream, target, proxy).await
        }
        fluxdb_core::PostgresProxyType::HttpConnect => {
            pg_http_connect_handshake(&mut stream, target, proxy).await
        }
    };
    run.map_err(|e| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("代理握手失败 ({}:{}): {e}", proxy.host, port),
        )
    })?;
    Ok(stream)
}

/// SOCKS5：greeting →（可选 user/pass 认证）→ CONNECT。
async fn pg_socks5_handshake(
    stream: &mut tokio::net::TcpStream,
    target: (&str, u16),
    proxy: &fluxdb_core::PostgresProxy,
) -> std::io::Result<()> {
    let has_creds = !proxy.username.is_empty();
    // greeting：NO AUTH(0x00) 与 USER/PASS(0x02)。
    stream
        .write_all(&[0x05, if has_creds { 2 } else { 1 }, 0x00, 0x02])
        .await?;
    let mut greets = [0u8; 2];
    stream.read_exact(&mut greets).await?;
    if greets[0] != 0x05 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "SOCKS5 版本不支持"));
    }
    match greets[1] {
        0x00 => {}
        0x02 => {
            let user = proxy.username.as_bytes();
            let pass = proxy.password.value().unwrap_or("").as_bytes();
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
    pg_socks_append_addr(&mut req, target.0)?;
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
fn pg_socks_append_addr(req: &mut Vec<u8>, host: &str) -> std::io::Result<()> {
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

/// HTTP CONNECT：发送隧道请求，直到读到响应头，校验 2xx/200。
async fn pg_http_connect_handshake(
    stream: &mut tokio::net::TcpStream,
    target: (&str, u16),
    proxy: &fluxdb_core::PostgresProxy,
) -> std::io::Result<()> {
    let mut head = format!("CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n", target.0, target.1, target.0, target.1);
    if !proxy.username.is_empty() {
        let cred = format!("{}:{}", proxy.username, proxy.password.value().unwrap_or(""));
        use base64::Engine as _;
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
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "代理响应头过大"));
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let head = text.lines().next().unwrap_or("");
    if !head.starts_with("HTTP/1.") || !head.split_whitespace().nth(1).is_some_and(|code| code == "200") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("HTTP CONNECT 被代理拒绝: {head}"),
        ));
    }
    Ok(())
}
