// PostgreSQL 拨号与会话管理（T04）。
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

/// 一个 PostgreSQL 会话 = 一条活连接（`tokio_postgres::Client`）。
///
/// 底层 `Connection` future 在共享 runtime 上独立 spawn 持续驱动，
/// `Client` 是 `Send + Sync`，用 `Arc` 包裹以便从注册表复用句柄。
#[derive(Clone)]
struct PgSession {
    /// `tokio_postgres::Client` 非 Clone，用 `Arc` 包裹以便从注册表复用句柄。
    client: std::sync::Arc<tokio_postgres::Client>,
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

/// 从连接档案构建 `tokio_postgres::Config`（T04 仅直连；TLS/SSH/代理见 T05）。
fn pg_config(
    config: &ConnectionConfig,
    database: &str,
) -> fluxdb_core::Result<tokio_postgres::Config> {
    let profile = config
        .postgres_profile
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL 连接档案缺失"))?;

    // T04 仅支持直连同居（无隧道）；SSH/代理留待 T05。
    if profile.ssh().is_some() || profile.proxy().is_some() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "PostgreSQL SSH 隧道 / 代理拨号尚未接入（T05）",
        ));
    }
    // T04 仅支持明文；TLS 加密留待 T05。
    if profile.tls.enabled {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "PostgreSQL TLS 拨号尚未接入（T05）",
        ));
    }

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
async fn pg_connect(config: &ConnectionConfig, database: &str) -> fluxdb_core::Result<PgSession> {
    let pg = pg_config(config, database)?;
    let default_schema = config
        .postgres_profile
        .as_ref()
        .map(|profile| profile.scope.default_schema.clone())
        .unwrap_or_default();

    let (client, connection) = pg
        .connect(tokio_postgres::NoTls)
        .await
        .map_err(pg_error)?;
    // 连接 future 持续驱动：detach 即可，任务随 runtime 常驻，无需持有 JoinHandle。
    pg_runtime().spawn(connection);

    if !default_schema.is_empty() {
        client
            .execute("SET search_path TO $1", &[&default_schema])
            .await
            .map_err(pg_error)?;
    }

    Ok(PgSession {
        client: std::sync::Arc::new(client),
        last_used: std::time::Instant::now(),
    })
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

/// 把 tokio-postgres 错误统一映射为 fluxdb 错误（认证 / 连接 / 查询分类）。
fn pg_error(error: tokio_postgres::Error) -> Error {
    use tokio_postgres::error::SqlState;
    // 认证失败：effective_connection_limit / 认证 / 授权类错误。
    if let Some(code) = error.code() {
        let class = code.code().chars().next().unwrap_or('5');
        return match class {
            '2' => Error::new(ErrorKind::Authentication, error.to_string()),
            '3' | '0' => {
                // 0P000 无效角色名、28000 无效授权、28P01 密码错误等。
                if matches!(
                    code,
                    &SqlState::INVALID_AUTHORIZATION_SPECIFICATION
                        | &SqlState::INVALID_PASSWORD
                ) {
                    Error::new(ErrorKind::Authentication, error.to_string())
                } else {
                    Error::new(ErrorKind::Connection, error.to_string())
                }
            }
            _ => Error::new(ErrorKind::Query, error.to_string()),
        };
    }
    // 无 SQLSTATE 的底层 IO/协议错误 → 连接层。
    Error::new(ErrorKind::Connection, error.to_string())
}
