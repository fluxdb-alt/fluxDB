// PostgreSQL 拨号与会话管理（T04 / T05 传输与 TLS）。
//
// 设计要点（design 3.3）：
// - 单一共享 tokio runtime（`OnceLock`），任何请求都不重复创建 runtime；
// - 会话注册表（`Mutex<HashMap<SessionKey, PgSession>>`）按「连接 + database/schema + 会话作用域」
//   区分独立 PostgreSQL 连接；不同 key 对应不同 DB 会话，事务天然互不串扰；
// - 显式查询会话（带 `session_id`）复用同一连接，会话内事务/状态跨查询保持；
//   无 `session_id` 的请求走隔离短连接（每请求新建、用后即弃），保证互不串事务。

/// 会话用途：决定复用策略与事务隔离语义。
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum PgSessionPurpose {
    /// 显式查询会话：被 `session_id` 引用，跨请求复用同一条连接（事务跨查询保持）。
    Query(QuerySessionId),
    /// 无会话的隔离执行：每请求新建短连接，用后即弃（互不串事务）。
    Transient,
}

/// 会话注册表键：连接身份 + 配置代际 + 作用域 + 用途。
///
/// `config_generation` 是连接档案有效内容的哈希：改主机/端口/账号/密码/TLS/SSH 后代际变化，
/// 旧会话不再被命中（设计 §3.3），避免编辑配置后继续复用旧凭据的连接。
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct PgSessionKey {
    connection_id: ConnectionId,
    config_generation: u64,
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
    /// 执行锁覆盖整个批次与取消收尾，不能在同一连接上交错事务。
    execution: std::sync::Arc<tokio::sync::Mutex<()>>,
    driver: std::sync::Arc<PgDriver>,
    applied_schema: std::sync::Arc<Mutex<Option<String>>>,
    /// 短连接持有额度至请求结束；查询标签会话不占元数据额度。
    _permit: Option<std::sync::Arc<tokio::sync::OwnedSemaphorePermit>>,

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

/// 查询标签拥有事务、临时表和 SET 状态，只能显式关闭，不能作为普通 TTL 缓存淘汰。
struct PgDriver(tokio::task::JoinHandle<()>);

impl Drop for PgDriver {
    fn drop(&mut self) { self.0.abort(); }
}

/// 按谓词关闭并移除会话：`Client` 与 SSH 隧道随之 drop，连接 future 与桥线程收敛。
/// 返回关闭数量。
fn pg_close_sessions_where(predicate: impl Fn(&PgSessionKey) -> bool) -> usize {
    let Ok(mut sessions) = pg_sessions().lock() else {
        return 0;
    };
    let before = sessions.len();
    sessions.retain(|key, session| {
        if predicate(key) {
            session.driver.0.abort();
            false
        } else { true }
    });
    before - sessions.len()
}

/// 关闭某连接的全部 PostgreSQL 会话（断开连接 / 编辑配置 / 删除连接时调用）。
///
/// 设计 §3.3：连接驱动、转发线程和子进程要在断开/编辑配置/删除连接时一起释放；
/// 只靠空闲 TTL 会让连接与 SSH 桥线程在断开后继续挂着。
pub fn pg_close_connection_sessions(connection_id: ConnectionId) -> usize {
    let closed = pg_close_sessions_where(|key| key.connection_id == connection_id);
    if closed > 0 {
        tracing::info!(
            target: "fluxdb_connectors",
            connection_id = ?connection_id,
            closed,
            "PostgreSQL 会话已随连接释放"
        );
    }
    closed
}

/// 关闭单个查询会话（关闭查询标签页时调用）：未提交事务随连接释放由服务端回滚。
pub fn pg_close_query_session(connection_id: ConnectionId, session_id: QuerySessionId) -> usize {
    pg_close_sessions_where(|key| {
        key.connection_id == connection_id
            && key.purpose == PgSessionPurpose::Query(session_id)
    })
}

/// 连接档案有效内容的代际哈希：主机/端口/库/账号/密码/TLS/SSH/代理/超时任一变化即变化。
///
/// 用 `into_options()`（含解析后的密码等敏感值）而非 `Debug`——`SecretRef` 的 Debug 是打码的，
/// 拿它算哈希会让「只改密码」看起来毫无变化，从而继续复用旧连接。哈希值不回显、不落日志。
fn pg_config_generation(config: &ConnectionConfig) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match config.postgres_profile.as_ref() {
        Some(profile) => {
            for (key, value) in profile.into_options() {
                key.hash(&mut hasher);
                value.hash(&mut hasher);
            }
        }
        // 无结构化档案（历史连接）：退回扁平参数，仍能反映配置变更。
        None => {
            for (key, value) in &config.options {
                key.hash(&mut hasher);
                value.hash(&mut hasher);
            }
        }
    }
    config.credential_ref.hash(&mut hasher);
    hasher.finish()
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
    pg.ssl_mode(match (profile.tls.enabled, profile.tls.ssl_mode) {
        (false, _) | (_, fluxdb_core::PostgresSslMode::Disabled) => tokio_postgres::config::SslMode::Disable,
        (_, fluxdb_core::PostgresSslMode::Prefer) => tokio_postgres::config::SslMode::Prefer,
        _ => tokio_postgres::config::SslMode::Require,
    });
    pg.keepalives(profile.advanced.tcp_keepalive);
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
    pg_connect_session(config, database, true).await
}

async fn pg_connect_session(config: &ConnectionConfig, database: &str, limited: bool) -> fluxdb_core::Result<PgSession> {
    static METADATA_SLOTS: OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let permit = if limited {
        Some(std::sync::Arc::new(METADATA_SLOTS.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(4)))
            .clone().acquire_owned().await.map_err(|_| Error::new(ErrorKind::Connection, "元数据服务已关闭"))?))
    } else { None };
    let profile = config.postgres_profile.as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL 连接档案缺失"))?;
    let pg = pg_config(config, database)?;
    // 建连先落档案默认 schema（空则保留服务器 search_path）；请求级 schema 由会话层按需补 SET。
    let wanted_schema = profile.scope.default_schema.clone();
    let connect_timeout = profile.connect_timeout();

    let (client, tunnel, driver) = tokio::time::timeout(connect_timeout, pg_connect_transport(profile, pg))
        .await
        .map_err(|_| {
            Error::new(ErrorKind::Timeout, "PostgreSQL 建连超时（含传输与 TLS 握手）")
        })??;

    let applied_schema = pg_apply_search_path(&client, &wanted_schema).await?;
    if !profile.advanced.timezone.trim().is_empty() {
        client.query_one("SELECT set_config('TimeZone', $1, false)", &[&profile.advanced.timezone])
            .await.map_err(pg_error)?;
    }

    Ok(PgSession {
        client: std::sync::Arc::new(client),
        _tunnel: tunnel.map(std::sync::Arc::new),
        execution: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        driver,
        _permit: permit,
        applied_schema: std::sync::Arc::new(Mutex::new(applied_schema)),
    })
}

/// 按需设置会话 search_path，返回实际生效的 schema 串（None = 未设置，保留服务器默认）。
///
/// SET 是 utility 语句，不接受 `$n` 参数（服务端会报 syntax error at or near "$1"），
/// 故按标识符转义后拼装；schema 名一律经 `pg_quote_identifier`，不裸拼用户输入。
/// 支持逗号分隔的多段顺序（如 `a,public`）。
async fn pg_apply_search_path(
    client: &tokio_postgres::Client,
    schema: &str,
) -> fluxdb_core::Result<Option<String>> {
    let path = schema
        .split(',')
        .map(str::trim)
        .filter(|schema| !schema.is_empty())
        .map(pg_quote_identifier)
        .collect::<Vec<_>>()
        .join(", ");
    if path.is_empty() {
        return Ok(None);
    }
    client
        .batch_execute(&format!("SET search_path TO {path}"))
        .await
        .map_err(pg_error)?;
    Ok(Some(schema.to_string()))
}

/// 拨号和取消共用裸传输，TLS 名称与实际 TCP 地址独立。
async fn pg_transport_stream(
    profile: &fluxdb_core::PostgresConnectionProfile,
    tunnel: Option<&SshTunnel>,
) -> fluxdb_core::Result<tokio::net::TcpStream> {
    let (host, port) = profile.dial_endpoint();
    if let Some(tunnel) = tunnel {
        return tokio::net::TcpStream::connect(("127.0.0.1", tunnel.local_port)).await
            .map_err(|e| Error::new(ErrorKind::Connection, e.to_string()));
    }
    match profile.transport_layer() {
        fluxdb_core::PostgresTransportLayer::Proxy(proxy) => pg_proxy_connect(&proxy, (&host, port), profile.connect_timeout()).await,
        _ => tokio::net::TcpStream::connect((host.as_str(), port)).await
            .map_err(|e| Error::new(ErrorKind::Connection, e.to_string())),
    }
}

async fn pg_connect_transport(
    profile: &fluxdb_core::PostgresConnectionProfile,
    pg: tokio_postgres::Config,
) -> fluxdb_core::Result<(tokio_postgres::Client, Option<SshTunnel>, std::sync::Arc<PgDriver>)> {
    let tunnel = if let fluxdb_core::PostgresTransportLayer::Ssh(ssh) = profile.transport_layer() {
        let auth = pg_ssh_auth(&ssh);
        let (host, port) = profile.dial_endpoint();
        let options = SshTunnelOptions {
            connect_timeout_secs: if ssh.connect_timeout_secs > 0 { ssh.connect_timeout_secs } else { profile.connect_timeout_secs() },
            keepalive_interval_secs: ssh.keepalive_interval_secs,
            verify_host_key: true,
        };
        Some(tokio::task::spawn_blocking(move || open_tunnel_with((ssh.host.as_str(), ssh.port), &auth, (&host, port), options))
            .await.map_err(|e| Error::new(ErrorKind::Connection, format!("SSH 任务失败: {e}")))??)
    } else { None };
    let stream = pg_transport_stream(profile, tunnel.as_ref()).await?;
    let (client, driver) = match pg_tls_connect(profile)? {
        Some(mut tls) => {
            let ready = tokio_postgres::tls::MakeTlsConnect::<tokio::net::TcpStream>::make_tls_connect(&mut tls, pg_server_name(profile))
                .map_err(|e| Error::new(ErrorKind::Connection, format!("TLS 配置失败: {e}")))?;
            pg_connect_raw_spawn(&pg, stream, ready).await?
        }
        None => pg_connect_raw_spawn(&pg, stream, tokio_postgres::NoTls).await?,
    };
    Ok((client, tunnel, driver))
}

async fn pg_connect_raw_spawn<C, S>(
    pg: &tokio_postgres::Config, stream: S, tls: C,
) -> fluxdb_core::Result<(tokio_postgres::Client, std::sync::Arc<PgDriver>)>
where
    C: tokio_postgres::tls::TlsConnect<S>, C::Stream: Send + 'static,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (client, connection) = pg.connect_raw(stream, tls).await.map_err(pg_error)?;
    let driver = pg_runtime().spawn(async move {
        if let Err(error) = connection.await {
            tracing::warn!(target: "fluxdb_connectors", error = %error, "PostgreSQL 会话驱动退出");
        }
    });
    Ok((client, std::sync::Arc::new(PgDriver(driver))))
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
///
/// 键不含 schema（设计 §3.3）：切换 schema 作用域不应丢掉会话里的事务，改由
/// `pg_ensure_search_path` 在复用到的连接上补发 SET。
fn pg_session_key_for(request: &QueryRequest, database: &str) -> PgSessionKey {
    let purpose = match request.session_id {
        Some(session_id) => PgSessionPurpose::Query(session_id),
        None => PgSessionPurpose::Transient,
    };
    PgSessionKey {
        connection_id: request.connection_id,
        config_generation: 0, // 由 pg_session_acquire 按连接档案填真实代际
        database: Some(database.to_string()),
        schema: None,
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
    let mut key = pg_session_key_for(request, database);
    if matches!(key.purpose, PgSessionPurpose::Transient) {
        return pg_connect(config, database).await;
    }

    key.config_generation = pg_config_generation(config);

    let existing = {
        let sessions = pg_sessions().lock().map_err(pg_lock_error)?;
        sessions.get(&key).cloned()
    };
    if let Some(session) = existing {
        return Ok(session);
    }

    // 未命中：在锁外拨号，避免持 std Mutex 跨 await；随后回填。
    let session = pg_connect_session(config, database, false).await?;
    let mut sessions = pg_sessions().lock().map_err(pg_lock_error)?;
    match sessions.get(&key) {
        // 并发竞态：期间他人已插入可用会话，优先复用他人。
        Some(existing) => Ok(existing.clone()),
        None => {
            // 显式会话有硬上限，避免长期打开标签造成无限连接增长；不偷偷淘汰已有事务。
            if sessions.len() >= 128 {
                return Err(Error::new(ErrorKind::Connection, "PostgreSQL 查询会话已达上限，请关闭不使用的查询标签"));
            }
            sessions.insert(key, session.clone());
            Ok(session)
        }
    }
}

/// 复用到的会话若 schema 作用域与本次请求不一致，则在**同一连接**上补发 SET search_path。
///
/// 这样切换 schema 不会重建连接（保住事务/临时表），也不会让新请求继续用旧 search_path。
/// 请求未指定 schema（None）时保持会话既有作用域，不擅自改回服务器默认。
async fn pg_ensure_search_path(
    session: &PgSession,
    schema: Option<&str>,
) -> fluxdb_core::Result<()> {
    let Some(schema) = schema else {
        return Ok(());
    };
    let already = session
        .applied_schema
        .lock()
        .map(|applied| applied.as_deref() == Some(schema))
        .unwrap_or(false);
    if already {
        return Ok(());
    }
    let applied = pg_apply_search_path(&session.client, schema).await?;
    if let Ok(mut slot) = session.applied_schema.lock() {
        *slot = applied;
    }
    Ok(())
}

fn pg_lock_error(
    _: std::sync::PoisonError<
        std::sync::MutexGuard<'_, HashMap<PgSessionKey, PgSession>>,
    >,
) -> Error {
    Error::new(ErrorKind::Internal, "PostgreSQL 会话注册表锁失效")
}

/// 把 tokio-postgres 错误统一映射为 fluxdb 错误（认证 / 连接 / 查询分类）。
fn pg_error(error: tokio_postgres::Error) -> Error {
    use tokio_postgres::error::SqlState;
    // 错误文本统一带 SQLSTATE 前缀（severity+message+DETAIL+HINT 由 PG to_string 已带）。
    let text = pg_db_error_text(&error);
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
            return Error::new(ErrorKind::Authentication, text);
        }
        // 连接层（08/09/0A/0B …）视为连接失败，其余为查询失败。
        let class_is_connection = code_str.starts_with("08");
        return Error::new(
            if class_is_connection {
                ErrorKind::Connection
            } else {
                ErrorKind::Query
            },
            text,
        );
    }
    // 无 SQLSTATE 的底层 IO/协议错误 → 连接层。
    Error::new(ErrorKind::Connection, text)
}
