// PostgreSQL 拨号与会话管理（T04 / T05 传输与 TLS）。
//
// 设计要点（design 3.3）：
// - 单一共享 tokio runtime（`OnceLock`），任何请求都不重复创建 runtime；
// - 会话注册表（`Mutex<HashMap<SessionKey, PgSession>>`）按「连接 + database/schema + 会话作用域」
//   区分独立 PostgreSQL 连接；不同 key 对应不同 DB 会话，事务天然互不串扰；
// - 显式查询会话（带 `session_id`）复用同一连接，会话内事务/状态跨查询保持；
//   无 `session_id` 的请求走隔离短连接（每请求新建、用后即弃），保证互不串事务。

/// 会话空闲淘汰 TTL：超过此时长的空闲会话被惰性回收（防只进不出）。
const PG_SESSION_IDLE_TTL: Duration = Duration::from_secs(60);

/// 会话用途：决定复用策略与事务隔离语义。
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum PgSessionPurpose {
    /// 显式查询会话：被 `session_id` 引用，跨请求复用同一条连接（事务跨查询保持）。
    Query(QuerySessionId),
    /// 无会话的隔离执行：每请求新建短连接，用后即弃（互不串事务）。
    Transient,
}

/// 会话注册表键：连接身份 + 作用域 + 用途。
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct PgSessionKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
    purpose: PgSessionPurpose,
}

/// 一个 PostgreSQL 会话 = 一条活连接（`tokio_postgres::Client`）+ 可选的 SSH 隧道。
///
/// 底层 `Connection` future 在共享 runtime 上独立 spawn 持续驱动；
/// `Client` 用 `Arc` 包裹以便从注册表复用句柄。SSH 隧道用 `Arc` 包裹：任一会话副本存活期间
/// 隧道常开，全部副本释放（会话淘汰/断开）时监听器关闭、桥线程收敛退出，无线程泄漏（T05 M4）。
#[derive(Clone)]
struct PgSession {
    /// `tokio_postgres::Client` 非 Clone，用 `Arc` 包裹以便从注册表复用句柄。
    client: std::sync::Arc<tokio_postgres::Client>,
    /// SSH 隧道句柄（直连 / 代理路径为 None）。持有即保活。
    _tunnel: Option<std::sync::Arc<SshTunnel>>,
    last_used: std::time::Instant,
}

/// 共享 runtime：全进程唯一，避免每个请求重建。
fn pg_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("创建 PostgreSQL tokio runtime 失败")
    })
}

fn pg_sessions() -> &'static Mutex<HashMap<PgSessionKey, PgSession>> {
    static SESSIONS: OnceLock<Mutex<HashMap<PgSessionKey, PgSession>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 惰性淘汰空闲会话：取用前顺手清理躺太久/超上限的连接，避免只进不出。
fn pg_sweep_idle(now: std::time::Instant) {
    let Ok(mut sessions) = pg_sessions().lock() else {
        return;
    };
    let deadline = now - PG_SESSION_IDLE_TTL;
    let idle_before = sessions.len();
    sessions.retain(|_, session| session.last_used >= deadline);
    if sessions.len() != idle_before {
        tracing::debug!(
            target: "fluxdb_connectors",
            evicted = (idle_before - sessions.len()),
            "PostgreSQL 空闲会话已淘汰"
        );
    }
}

/// 从连接档案构建 `tokio_postgres::Config`。
///
/// 只填认证/库/超时/会话参数；传输（直连/SSH/代理）与 TLS 在 `pg_connect` 按传输层选择。
/// `host` 恒为真实远端主机（作为 TLS 校验主机名）；SSH 路径另设 `hostaddr` 走隧道本地端口。
fn pg_config(
    config: &ConnectionConfig,
    database: &str,
) -> fluxdb_core::Result<tokio_postgres::Config> {
    let profile = config
        .postgres_profile
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL 连接档案缺失"))?;

    let (host, port) = profile.dial_endpoint();
    let mut pg = tokio_postgres::Config::new();
    pg.host(&host);
    pg.port(port);
    pg.user(&profile.basic.username);
    if let Some(password) = profile.password() {
        pg.password(password);
    }
    pg.dbname(database);
    pg.connect_timeout(profile.connect_timeout());
    if profile.advanced.query_timeout_secs > 0 {
        // 会话级 statement_timeout（毫秒），覆盖默认无限等待。
        let ms = u64::from(profile.advanced.query_timeout_secs) * 1000;
        pg.options(&format!("-c statement_timeout={ms}"));
    }
    let app_name = if profile.advanced.application_name.is_empty() {
        "FluxDB"
    } else {
        &profile.advanced.application_name
    };
    pg.application_name(app_name);
    Ok(pg)
}

/// 解析本次请求要执行所连的物理数据库：请求指定优先，否则用维护库。
fn pg_request_database(config: &ConnectionConfig, request_database: Option<&str>) -> String {
    match request_database.filter(|database| !database.is_empty()) {
        Some(database) => database.to_string(),
        None => config
            .postgres_profile
            .as_ref()
            .map(|profile| profile.maintenance_database().to_string())
            .unwrap_or_else(|| "postgres".to_string()),
    }
}

/// 在共享 runtime 上拨号建连（直接传输）。
///
/// 注意：本函数为 `async`——调用方（`test_connection` / `pg_execute_query*`）统一在共享
/// runtime 的 `block_on` 里 `.await` 它，避免在已进入 runtime 的线程上再 `block_on` 导致
/// tokio「Cannot start a runtime from within a runtime」。
/// 建连并返回会话（含 SSH 隧道句柄）。
///
/// 传输按 `transport_layer` 三选一：直连 `connect` / SSH 隧道（`hostaddr` 走隧道本地端口，
/// TLS 身份仍取真实主机）/ 代理（拨代理拿裸流后 `connect_raw`）。TLS 由 `pg_tls_connect` 决定。
/// 整体（传输握手 + TLS 握手 + 启动）统一受 `connect_timeout` 约束。
async fn pg_connect(config: &ConnectionConfig, database: &str) -> fluxdb_core::Result<PgSession> {
    let profile = config
        .postgres_profile
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL 连接档案缺失"))?;
    let pg = pg_config(config, database)?;
    let default_schema = profile.scope.default_schema.clone();
    let connect_timeout = profile.connect_timeout();

    let (client, tunnel) = tokio::time::timeout(connect_timeout, pg_connect_transport(profile, pg))
        .await
        .map_err(|_| {
            Error::new(ErrorKind::Timeout, "PostgreSQL 建连超时（含传输与 TLS 握手）")
        })??;

    if !default_schema.is_empty() {
        // SET 是 utility 语句，不接受 $n 参数（服务端会报 syntax error at or near "$1"），
        // 故按标识符转义后拼装；schema 名一律经 pg_quote_identifier 处理，不裸拼用户输入。
        // 支持逗号分隔的 schema 顺序（如 `a,public`），逐段转义后按序设置。
        let path = default_schema
            .split(',')
            .map(str::trim)
            .filter(|schema| !schema.is_empty())
            .map(pg_quote_identifier)
            .collect::<Vec<_>>()
            .join(", ");
        if !path.is_empty() {
            client
                .batch_execute(&format!("SET search_path TO {path}"))
                .await
                .map_err(pg_error)?;
        }
    }

    Ok(PgSession {
        client: std::sync::Arc::new(client),
        _tunnel: tunnel.map(std::sync::Arc::new),
        last_used: std::time::Instant::now(),
    })
}

/// 按传输层建连并 spawn 连接 future，返回 `Client` 与隧道句柄。
async fn pg_connect_transport(
    profile: &fluxdb_core::PostgresConnectionProfile,
    mut pg: tokio_postgres::Config,
) -> fluxdb_core::Result<(tokio_postgres::Client, Option<SshTunnel>)> {
    let tls = pg_tls_connect(profile)?;
    let connect_timeout = profile.connect_timeout();

    match profile.transport_layer() {
        // SSH：先建带 hostkey 校验的隧道，再向 `127.0.0.1:<local_port>` 拨号；
        // 保持 `host` 为真实主机，TLS 校验仍针对真实远端（hostaddr 分离，R33）。
        fluxdb_core::PostgresTransportLayer::Ssh(ssh) => {
            let auth = pg_ssh_auth(&ssh);
            let (host, port) = profile.dial_endpoint();
            let options = SshTunnelOptions {
                connect_timeout_secs: if ssh.connect_timeout_secs > 0 {
                    ssh.connect_timeout_secs
                } else {
                    profile.connect_timeout_secs()
                },
                keepalive_interval_secs: ssh.keepalive_interval_secs,
                verify_host_key: true, // PG 传输路径强制校验已知主机（错误 hostkey 直接拒绝）。
            };
            let tunnel = open_tunnel_with((ssh.host.as_str(), ssh.port), &auth, (&host, port), options)?;
            pg.hostaddr(std::net::IpAddr::from(std::net::Ipv4Addr::LOCALHOST));
            pg.port(tunnel.local_port);
            let client = match tls {
                Some(t) => pg_connect_spawn(&pg, t).await?,
                None => pg_connect_spawn(&pg, tokio_postgres::NoTls).await?,
            };
            Ok((client, Some(tunnel)))
        }
        // 代理：拨代理（SOCKS5 / HTTP CONNECT）拿裸流，再 `connect_raw`；TLS 校验主机名取 profile。
        fluxdb_core::PostgresTransportLayer::Proxy(proxy) => {
            let (host, port) = profile.dial_endpoint();
            let stream = pg_proxy_connect(&proxy, (&host, port), connect_timeout).await?;
            let client = match tls {
                Some(mut t) => {
                    let server_name = pg_server_name(profile);
                    let ready = tokio_postgres::tls::MakeTlsConnect::<tokio::net::TcpStream>::make_tls_connect(&mut t, server_name)
                        .map_err(|e| {
                            Error::new(ErrorKind::Connection, format!("TLS 连接器构建失败: {e}"))
                        })?;
                    pg_connect_raw_spawn(&pg, stream, ready).await?
                }
                None => pg_connect_raw_spawn(&pg, stream, tokio_postgres::NoTls).await?,
            };
            Ok((client, None))
        }
        // 直连：`connect` 走 host/hostaddr，TLS 身份即配置主机。
        fluxdb_core::PostgresTransportLayer::Direct => {
            let client = match tls {
                Some(t) => pg_connect_spawn(&pg, t).await?,
                None => pg_connect_spawn(&pg, tokio_postgres::NoTls).await?,
            };
            Ok((client, None))
        }
    }
}

/// `connect` 建连并 spawn 连接 future，返回 `Client`。
/// `C::Stream` 需为 `Send` 才能被 tokio runtime 的独立任务驱动。
async fn pg_connect_spawn<C>(
    pg: &tokio_postgres::Config,
    tls: C,
) -> fluxdb_core::Result<tokio_postgres::Client>
where
    C: tokio_postgres::tls::MakeTlsConnect<tokio_postgres::Socket>,
    C::Stream: Send + 'static,
{
    let (client, connection) = pg.connect(tls).await.map_err(pg_error)?;
    pg_runtime().spawn(connection);
    Ok(client)
}

/// `connect_raw`（代理裸流）建连并 spawn 连接 future，返回 `Client`。
/// 连接 future 需 `Send` 才能 spawn 到共享 runtime，故要求流与 TLS 结果流均 `Send + 'static`。
async fn pg_connect_raw_spawn<C, S>(
    pg: &tokio_postgres::Config,
    stream: S,
    tls: C,
) -> fluxdb_core::Result<tokio_postgres::Client>
where
    C: tokio_postgres::tls::TlsConnect<S>,
    C::Stream: Send + 'static,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (client, connection) = pg.connect_raw(stream, tls).await.map_err(pg_error)?;
    pg_runtime().spawn(connection);
    Ok(client)
}

/// 由 SSH options 组装认证参数：密码或私钥任一存在即用，空字段自动忽略。
fn pg_ssh_auth(ssh: &fluxdb_core::PostgresSshOptions) -> SshAuthParams {
    fn nonempty(s: Option<&str>) -> Option<String> {
        s.filter(|v| !v.is_empty()).map(str::to_string)
    }
    SshAuthParams {
        username: ssh.username.clone(),
        password: nonempty(ssh.password.value()),
        private_key_path: ssh.private_key.value().unwrap_or_default().trim().to_string(),
        passphrase: nonempty(ssh.passphrase.value()),
    }
}

/// 获取本次请求的会话键：
/// - 携带 `session_id` → 显式查询会话（复用连接，事务跨查询）；键含 session_id 确保互不串事务；
/// - 无 `session_id` → 隔离短暂（每请求新建连接）。
fn pg_session_key_for(request: &QueryRequest, database: &str) -> PgSessionKey {
    let purpose = match request.session_id {
        Some(session_id) => PgSessionPurpose::Query(session_id),
        None => PgSessionPurpose::Transient,
    };
    PgSessionKey {
        connection_id: request.connection_id,
        database: Some(database.to_string()),
        schema: request.schema.clone(),
        purpose,
    }
}

/// 取回（或新建）一个会话供本次执行使用。
///
/// 显式会话复用注册表里的同一连接；隔离执行直接新拨一条（不进注册表，用后即弃）。
/// `async`：仅在锁外 await 拨号，避免持 std Mutex 跨 await 也避免在 block_on 内再 block_on。
async fn pg_session_acquire(
    config: &ConnectionConfig,
    request: &QueryRequest,
    database: &str,
) -> fluxdb_core::Result<PgSession> {
    let key = pg_session_key_for(request, database);
    let is_transient = matches!(key.purpose, PgSessionPurpose::Transient);

    if is_transient {
        // 隔离执行：新拨一条，不进入注册表，天然互不串事务。
        return pg_connect(config, database).await;
    }

    // 惰性清除全局空闲会话（轻量，约 60s 一次）。
    let now = std::time::Instant::now();
    pg_sweep_idle(now);

    {
        let sessions = pg_sessions().lock().map_err(pg_lock_error)?;
        if let Some(session) = sessions.get(&key) {
            return Ok(session.clone());
        }
    }

    // 未命中：在锁外拨号，避免持 std Mutex 跨 await；随后回填。
    let session = pg_connect(config, database).await?;
    let mut sessions = pg_sessions().lock().map_err(pg_lock_error)?;
    match sessions.get(&key) {
        // 并发竞态：期间他人已插入可用会话，优先复用他人。
        Some(existing) => Ok(existing.clone()),
        None => {
            sessions.insert(key, session.clone());
            Ok(session)
        }
    }
}

fn pg_lock_error(
    _: std::sync::PoisonError<
        std::sync::MutexGuard<'_, HashMap<PgSessionKey, PgSession>>,
    >,
) -> Error {
    Error::new(ErrorKind::Internal, "PostgreSQL 会话注册表锁失效")
}

/// 该错误是否使当前会话进入 aborted 事务态（`25P02 in_failed_sql_transaction`）。
/// 显式事务内任意语句失败后，后续语句在 ROLLBACK 前都不可执行；用于停止 continue_on_error。
fn pg_error_aborts_transaction(error: &tokio_postgres::Error) -> bool {
    use tokio_postgres::error::SqlState;
    matches!(
        error.code(),
        Some(&SqlState::IN_FAILED_SQL_TRANSACTION)
    )
}

/// 把 tokio-postgres 错误统一映射为 fluxdb 错误（认证 / 连接 / 查询分类）。
fn pg_error(error: tokio_postgres::Error) -> Error {
    use tokio_postgres::error::SqlState;
    // 仅认证/授权类错误归为 Authentication；其余 DB 类错误一律 Query。
    // 早期实现把「SQLSTATE 首字符为 2」都当认证，误伤了 22 数据异常/23 完整性/25 事务态
    // （如唯一约束冲突、参数越界）——那些是查询/写入失败而非认证失败。
    if let Some(code) = error.code() {
        let code_str = code.code();
        let is_auth = code_str.starts_with("28")
            || matches!(
                code,
                &SqlState::INVALID_AUTHORIZATION_SPECIFICATION
                    | &SqlState::INVALID_PASSWORD
            )
            || code_str == "0P000";
        if is_auth {
            return Error::new(ErrorKind::Authentication, error.to_string());
        }
        // 连接层（08/09/0A/0B …）视为连接失败，其余为查询失败。
        let class_is_connection = code_str.starts_with('8') || code_str.starts_with("0A0");
        return Error::new(
            if class_is_connection {
                ErrorKind::Connection
            } else {
                ErrorKind::Query
            },
            error.to_string(),
        );
    }
    // 无 SQLSTATE 的底层 IO/协议错误 → 连接层。
    Error::new(ErrorKind::Connection, error.to_string())
}
