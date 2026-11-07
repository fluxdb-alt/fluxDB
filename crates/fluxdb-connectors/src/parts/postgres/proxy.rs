// PostgreSQL 代理拨号（T05）：SOCKS5 / HTTP CONNECT，把代理后的裸 TCP 流交给 `connect_raw`。
//
// 握手与拨号复用传输层共享实现；认证信息来自 profile（`SecretRef`），正文不落日志；
// 建连受整体超时约束。

/// 拨代理拿到通往目标的裸 TCP 流。
///
/// `target`：目标主机 + 端口（隧道内直连目标）。`timeout`：代理建连 + 握手整体超时。
async fn pg_proxy_connect(
    proxy: &fluxdb_core::PostgresProxy,
    target: (&str, u16),
    timeout: std::time::Duration,
) -> fluxdb_core::Result<tokio::net::TcpStream> {
    let spec = ProxySpec {
        kind: match proxy.proxy_type {
            fluxdb_core::PostgresProxyType::Socks5 => ProxyKind::Socks5,
            fluxdb_core::PostgresProxyType::HttpConnect => ProxyKind::HttpConnect,
        },
        host: proxy.host.clone(),
        port: proxy.port,
        username: proxy.username.clone(),
        password: proxy.password.value().unwrap_or_default().to_string(),
    };
    proxy_tcp_stream(&spec, target, timeout, false).await
}
