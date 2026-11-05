use fluxdb_core::{SqlServerEncryption, SqlServerConnectionProfile};
use futures_util::TryStreamExt as _;
use tiberius::{AuthMethod, Client, Config as TdsConfig, EncryptionLevel};
use tokio::net::TcpStream as TokioTcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt as _};

/// SQL Server 连接句柄：驱动 client + 可能存在的 SSH 隧道。
///
/// 隧道必须与 client 同生命周期存活（Drop 即关闭转发端口），Deref 让既有调用点
/// 继续按 `Client<Compat<TokioTcpStream>>` 使用。
struct SqlServerClient {
    client: Client<Compat<TokioTcpStream>>,
    _tunnel: Option<SshTunnel>,
}

impl std::ops::Deref for SqlServerClient {
    type Target = Client<Compat<TokioTcpStream>>;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl std::ops::DerefMut for SqlServerClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct SqlServerSessionKey {
    connection_id: ConnectionId,
    config_generation: u64,
    database: String,
    session_id: QuerySessionId,
}

struct SqlServerSession {
    client: std::sync::Arc<Mutex<SqlServerClient>>,
}

fn sqlserver_sessions(
) -> &'static Mutex<HashMap<SqlServerSessionKey, SqlServerSession>> {
    static SESSIONS: OnceLock<
        Mutex<HashMap<SqlServerSessionKey, SqlServerSession>>,
    > = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 全进程复用一个多线程 runtime，避免每次 Connector 同步调用都创建线程池。
fn sqlserver_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("创建 SQL Server tokio runtime 失败")
    })
}

fn sqlserver_profile(config: &ConnectionConfig) -> fluxdb_core::Result<SqlServerConnectionProfile> {
    if config.kind != DatabaseKind::SqlServer {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }
    Ok(config
        .sqlserver_profile
        .clone()
        .unwrap_or_else(|| SqlServerConnectionProfile::from_options(&config.options)))
}

fn sqlserver_database(config: &ConnectionConfig, profile: &SqlServerConnectionProfile) -> Option<String> {
    let profile_database = profile.basic.database.trim();
    (!profile_database.is_empty())
        .then(|| profile_database.to_string())
        .or_else(|| match &config.endpoint {
            Endpoint::Tcp { database, .. } => database
                .as_deref()
                .map(str::trim)
                .filter(|database| !database.is_empty())
                .map(str::to_string),
            _ => None,
        })
}

fn sqlserver_config_generation(profile: &SqlServerConnectionProfile) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (key, value) in profile.into_options() {
        key.hash(&mut hasher);
        value.hash(&mut hasher);
    }
    hasher.finish()
}

fn sqlserver_session_key(
    config: &ConnectionConfig,
    database: &str,
    session_id: QuerySessionId,
) -> fluxdb_core::Result<SqlServerSessionKey> {
    let profile = sqlserver_profile(config)?;
    Ok(SqlServerSessionKey {
        connection_id: config.id,
        config_generation: sqlserver_config_generation(&profile),
        database: database.to_string(),
        session_id,
    })
}

/// Query-editor sessions persist transactions, temporary tables and `SET` state.
/// Metadata and data-page requests do not pass a session id and keep using
/// isolated short connections.
fn sqlserver_session_client(
    config: &ConnectionConfig,
    database: &str,
    session_id: QuerySessionId,
) -> fluxdb_core::Result<std::sync::Arc<Mutex<SqlServerClient>>> {
    let key = sqlserver_session_key(config, database, session_id)?;
    if let Ok(sessions) = sqlserver_sessions().lock()
        && let Some(session) = sessions.get(&key)
    {
        return Ok(session.client.clone());
    }

    let mut profile = sqlserver_profile(config)?;
    profile.basic.database = database.to_string();
    let client = sqlserver_connect_profile(profile)?;
    let session = SqlServerSession {
        client: std::sync::Arc::new(Mutex::new(client)),
    };
    let mut sessions = sqlserver_sessions()
        .lock()
        .map_err(|_| Error::new(ErrorKind::Internal, "SQL Server 会话注册表锁失效"))?;
    if let Some(existing) = sessions.get(&key) {
        return Ok(existing.client.clone());
    }
    if sessions.len() >= 128 {
        return Err(Error::new(
            ErrorKind::Connection,
            "SQL Server 查询会话已达上限，请关闭不使用的查询标签",
        ));
    }
    let client = session.client.clone();
    sessions.insert(key, session);
    Ok(client)
}

fn sqlserver_close_sessions_where(
    predicate: impl Fn(&SqlServerSessionKey) -> bool,
) -> usize {
    let Ok(mut sessions) = sqlserver_sessions().lock() else {
        return 0;
    };
    let before = sessions.len();
    sessions.retain(|key, _| !predicate(key));
    before - sessions.len()
}

pub fn sqlserver_close_connection_sessions(connection_id: ConnectionId) -> usize {
    sqlserver_close_sessions_where(|key| key.connection_id == connection_id)
}

pub fn sqlserver_close_query_session(
    connection_id: ConnectionId,
    session_id: QuerySessionId,
) -> usize {
    sqlserver_close_sessions_where(|key| {
        key.connection_id == connection_id && key.session_id == session_id
    })
}

fn sqlserver_tiberius_config(
    profile: &SqlServerConnectionProfile,
) -> fluxdb_core::Result<TdsConfig> {
    if !profile.basic.instance.trim().is_empty() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "SQL Server 命名实例首版需要填写实际 TCP 端口；不支持 SQL Browser 自动解析",
        ));
    }
    if let Some(message) = profile.validate() {
        return Err(Error::new(ErrorKind::Connection, message));
    }

    let mut config = TdsConfig::new();
    config.host(profile.basic.host.trim());
    config.port(profile.basic.port);
    if let Some(database) = sqlserver_database(
        // profile 已独立解析；这里传空配置只为复用同名 endpoint 取值逻辑。
        &ConnectionConfig {
            id: ConnectionId(0),
            name: String::new(),
            kind: DatabaseKind::SqlServer,
            endpoint: Endpoint::Tcp {
                host: profile.basic.host.clone(),
                port: profile.basic.port,
                database: None,
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: Some(profile.clone()),
            mongo_profile: None,
            oracle_profile: None,
        },
        profile,
    ) {
        config.database(database);
    }
    config.authentication(AuthMethod::sql_server(
        profile.basic.username.trim(),
        profile.password().unwrap_or(""),
    ));
    config.application_name(if profile.advanced.application_name.trim().is_empty() {
        "FluxDB"
    } else {
        profile.advanced.application_name.trim()
    });
    config.encryption(match profile.advanced.encryption {
        SqlServerEncryption::Disable => EncryptionLevel::Off,
        SqlServerEncryption::Require | SqlServerEncryption::TrustServerCertificate => {
            EncryptionLevel::Required
        }
    });
    if profile.advanced.encryption == SqlServerEncryption::TrustServerCertificate {
        // 只在用户明确选择信任自签证书时关闭链校验；Require 保持系统根证书校验。
        config.trust_cert();
    }
    Ok(config)
}

/// 建立一条 SQL Server 会话。connect timeout 覆盖 TCP + TLS + TDS login 全过程；
/// Azure 路由重定向最多跟随两次，超限直接报错，避免无限重连。
async fn sqlserver_connect_raw(
    profile: &SqlServerConnectionProfile,
    tunneled: bool,
) -> fluxdb_core::Result<Client<Compat<TokioTcpStream>>> {
    let mut config = sqlserver_tiberius_config(profile)?;
    let timeout = Duration::from_secs(u64::from(profile.advanced.connect_timeout_secs.max(1)));
    for redirects in 0..=2 {
        let address = config.get_addr();
        let connect = async {
            let tcp = TokioTcpStream::connect(&address)
                .await
                .map_err(|error| sqlserver_io_error("连接 SQL Server", error))?;
            tcp.set_nodelay(true)
                .map_err(|error| sqlserver_io_error("配置 SQL Server TCP", error))?;
            Client::connect(config.clone(), tcp.compat_write())
                .await
                .map_err(sqlserver_driver_error)
        };
        match tokio::time::timeout(timeout, connect).await {
            Ok(Ok(client)) => return Ok(client),
            Ok(Err(Error {
                kind: ErrorKind::Connection,
                message,
                ..
            })) if message.contains("alternative address") && redirects < 2 => {
                // 隧道只能到跳板机可达的那一个目标；服务端给出的替代地址本地不可达，
                // 继续直连会绕过隧道，明确拒绝而非静默改走公网。
                if tunneled {
                    return Err(Error::new(
                        ErrorKind::Connection,
                        "SQL Server 路由重定向不支持 SSH 隧道，请改用直连或固定端口",
                    ));
                }
                let (host, port) = parse_sqlserver_routing_error(&message)?;
                config.host(host);
                config.port(port);
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(Error::new(
                    ErrorKind::Timeout,
                    format!("连接 SQL Server 超时（{} 秒）", timeout.as_secs()),
                ));
            }
        }
    }
    Err(Error::new(
        ErrorKind::Connection,
        "SQL Server 路由重定向次数超过上限",
    ))
}

fn parse_sqlserver_routing_error(message: &str) -> fluxdb_core::Result<(String, u16)> {
    // `tiberius::Error::Routing` 的 Display 含 ``host:port``；这里解析自身错误文本，
    // 不依赖私有结构，解析失败会明确报错而不是沿用旧地址重试。
    let endpoint = message
        .rsplit_once('`')
        .and_then(|(_, rest)| rest.split_once('`'))
        .map(|(endpoint, _)| endpoint)
        .or_else(|| message.split("`").nth(1))
        .ok_or_else(|| Error::new(ErrorKind::Connection, "SQL Server 路由响应缺少目标地址"))?;
    let (host, port) = endpoint
        .rsplit_once(':')
        .ok_or_else(|| Error::new(ErrorKind::Connection, "SQL Server 路由目标端口缺失"))?;
    Ok((
        host.trim_matches(['`', '[', ']']).to_string(),
        port.parse().map_err(|_| {
            Error::new(ErrorKind::Connection, "SQL Server 路由目标端口不合法")
        })?,
    ))
}

/// 同步 Connector API 的统一阻塞入口。
///
/// 连接器本身是同步 trait，但内部元数据读取/写入辅助函数都运行在同一个 runtime 上，
/// 会再次调用本函数。若当前线程已经在该 multi-thread runtime 内，必须用
/// `block_in_place` 把它移出 worker，否则 tokio 会直接 panic
/// “Cannot start a runtime from within a runtime”。
fn sqlserver_block_on<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        Ok(_) => {
            // 单线程 runtime 不允许 block_in_place；连接器 API 只从同步工作线程调用，
            // 这里显式报错，避免悄悄嵌套运行导致协议流错乱。
            panic!("SQL Server 连接器不能在单线程 tokio runtime 内同步调用");
        }
        Err(_) => sqlserver_runtime().block_on(future),
    }
}

/// Runtime::block_on 在调用线程上轮询根 future，并不会把 TLS 握手移到工作线程。
/// macOS GCD 小栈上的业务调用链叠加 rustls 握手会溢出；连接 future 必须由共享
/// runtime 调度，调用方只轮询轻量 JoinHandle。连接超时与证书校验仍在原握手中执行。
fn sqlserver_connect_profile(
    mut profile: SqlServerConnectionProfile,
) -> fluxdb_core::Result<SqlServerClient> {
    // SSH 隧道：先建本地转发，再把拨号目标改成 127.0.0.1:local_port。
    // 隧道由返回句柄持有，client 关闭/丢弃时才断开。
    let tunnel = if profile.ssh.enabled {
        let target = (profile.basic.host.clone(), profile.basic.port);
        let tunnel = open_tunnel_for_profile(
            &profile.ssh,
            (&target.0, target.1),
            profile.advanced.connect_timeout_secs,
        )?;
        profile.basic.host = "127.0.0.1".into();
        profile.basic.port = tunnel.local_port;
        Some(tunnel)
    } else {
        None
    };
    let tunneled = tunnel.is_some();
    let connection = sqlserver_runtime().spawn(async move {
        sqlserver_connect_raw(&profile, tunneled).await
    });
    let client = sqlserver_block_on(connection).map_err(|error| {
        tracing::error!(
            target: "fluxdb_connectors",
            error = %error,
            "SQL Server connection task failed"
        );
        Error::new(ErrorKind::Connection, "SQL Server 连接任务异常结束")
    })??;
    Ok(SqlServerClient {
        client,
        _tunnel: tunnel,
    })
}

fn sqlserver_connect(config: &ConnectionConfig) -> fluxdb_core::Result<SqlServerClient> {
    let profile = sqlserver_profile(config)?;
    sqlserver_connect_profile(profile)
}

async fn sqlserver_close(client: SqlServerClient) {
    // `Client::close` 消费 client；隧道由 wrapper 持有的引用在函数结束时释放。
    let _ = client.client.close().await;
}

fn sqlserver_io_error(prefix: &str, error: std::io::Error) -> Error {
    let kind = match error.kind() {
        std::io::ErrorKind::TimedOut => ErrorKind::Timeout,
        _ => ErrorKind::Connection,
    };
    Error::new(kind, format!("{prefix}：{error}"))
}

fn sqlserver_driver_error(error: tiberius::error::Error) -> Error {
    let kind = match &error {
        tiberius::error::Error::Io { kind, .. } if *kind == std::io::ErrorKind::TimedOut => {
            ErrorKind::Timeout
        }
        tiberius::error::Error::Io { .. } | tiberius::error::Error::Tls(_) => ErrorKind::Connection,
        tiberius::error::Error::Server(token) if token.code() == 18456 => ErrorKind::Authentication,
        tiberius::error::Error::Server(token) if token.code() == 229 => ErrorKind::Permission,
        tiberius::error::Error::Routing { .. } => ErrorKind::Connection,
        _ => ErrorKind::Query,
    };
    Error::new(kind, format!("SQL Server 驱动错误：{error}"))
}

/// 参数化查询统一使用 `@P1..@Pn`；tiberius 的 Query 对象按顺序绑定，
/// 这里只生成占位符，不把任何值拼进 SQL。
fn sqlserver_placeholder(index: usize) -> String {
    format!("@P{index}")
}

async fn sqlserver_run_batch(
    client: &mut SqlServerClient,
    sql: &str,
) -> fluxdb_core::Result<SqlServerBatchResult> {
    sqlserver_run_batch_with(client, sql, None).await
}

/// `owner` 为 `Some` 时超预算的行先把大字段外置成内容引用再收，绝不整行丢弃。
///
/// 元数据/管理命令的行很小，走 `None` 保持原样；只有面向用户的数据页与查询
/// 结果才需要「不丢行」这条路。
async fn sqlserver_run_batch_with(
    client: &mut SqlServerClient,
    sql: &str,
    owner: Option<&str>,
) -> fluxdb_core::Result<SqlServerBatchResult> {
    let mut stream = client.simple_query(sql).await.map_err(sqlserver_driver_error)?;
    let mut result = SqlServerBatchResult::default();
    while let Some(item) = stream.try_next().await.map_err(sqlserver_driver_error)? {
        match item {
            tiberius::QueryItem::Metadata(meta) => {
                result.result_sets.push(SqlServerResultSet {
                    columns: meta.columns().iter().map(sqlserver_query_column).collect(),
                    rows: Vec::new(),
                });
            }
            tiberius::QueryItem::Row(row) => {
                sqlserver_store_row(
                    &mut result,
                    sqlserver_row(row),
                    owner,
                )?;
            }
        }
    }
    Ok(result)
}

async fn sqlserver_run_query(
    client: &mut SqlServerClient,
    sql: &str,
    parameters: &[fluxdb_core::SqlParameter],
) -> fluxdb_core::Result<SqlServerBatchResult> {
    sqlserver_run_query_with(client, sql, parameters, None).await
}

async fn sqlserver_run_query_with(
    client: &mut SqlServerClient,
    sql: &str,
    parameters: &[fluxdb_core::SqlParameter],
    owner: Option<&str>,
) -> fluxdb_core::Result<SqlServerBatchResult> {
    if parameters.is_empty() {
        return sqlserver_run_batch_with(client, sql, owner).await;
    }
    let mut query = tiberius::Query::new(sql);
    for parameter in parameters {
        bind_sqlserver_parameter(&mut query, parameter);
    }
    let mut stream = query.query(client).await.map_err(sqlserver_driver_error)?;
    let mut result = SqlServerBatchResult::default();
    while let Some(item) = stream.try_next().await.map_err(sqlserver_driver_error)? {
        match item {
            tiberius::QueryItem::Metadata(meta) => {
                result.result_sets.push(SqlServerResultSet {
                    columns: meta.columns().iter().map(sqlserver_query_column).collect(),
                    rows: Vec::new(),
                });
            }
            tiberius::QueryItem::Row(row) => {
                sqlserver_store_row(
                    &mut result,
                    sqlserver_row(row),
                    owner,
                )?;
            }
        }
    }
    Ok(result)
}

/// 值参数按精确类型绑定。Decimal 走 Numeric 会受 scale<38 限制，因此大精度
/// decimal 保留为字符串，由 SQL Server 按列类型隐式转换。
fn bind_sqlserver_parameter(query: &mut tiberius::Query<'_>, parameter: &fluxdb_core::SqlParameter) {
    match parameter {
        fluxdb_core::SqlParameter::Null => query.bind(Option::<&str>::None),
        fluxdb_core::SqlParameter::Bool(value) => query.bind(*value),
        fluxdb_core::SqlParameter::I64(value) | fluxdb_core::SqlParameter::U64(value) => {
            if let Ok(value) = value.parse::<i64>() {
                query.bind(value);
            } else {
                query.bind(value.clone());
            }
        }
        fluxdb_core::SqlParameter::Decimal(value) => {
            if let Some(numeric) = parse_sqlserver_numeric(value) {
                query.bind(numeric);
            } else {
                query.bind(value.clone());
            }
        }
        fluxdb_core::SqlParameter::F64(value) => query.bind(*value),
        fluxdb_core::SqlParameter::Text(value) => query.bind(value.clone()),
        fluxdb_core::SqlParameter::Bytes(value) => query.bind(value.clone()),
    }
}

fn parse_sqlserver_numeric(value: &str) -> Option<tiberius::numeric::Numeric> {
    let (integer, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() >= 38 {
        return None;
    }
    let negative = integer.starts_with('-');
    let digits = format!(
        "{}{}",
        integer.trim_start_matches('-'),
        fraction
    )
    .parse::<i128>()
    .ok()?;
    let digits = if negative { -digits } else { digits };
    Some(tiberius::numeric::Numeric::new_with_scale(
        digits,
        fraction.len() as u8,
    ))
}

fn sqlserver_request_database(
    config: &ConnectionConfig,
    requested: Option<&str>,
) -> fluxdb_core::Result<String> {
    requested
        .map(str::trim)
        .filter(|database| !database.is_empty())
        .map(str::to_string)
        .or_else(|| {
            config
                .sqlserver_profile
                .as_ref()
                .map(|profile| profile.basic.database.trim().to_string())
                .filter(|database| !database.is_empty())
        })
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少 SQL Server 数据库上下文"))
}

fn sqlserver_request_schema(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<String> {
    path.schema
        .clone()
        .filter(|schema| !schema.trim().is_empty())
        .or_else(|| {
            config
                .sqlserver_profile
                .as_ref()
                .map(|profile| profile.basic.schema.clone())
                .filter(|schema| !schema.trim().is_empty())
        })
        .or_else(|| {
            config
                .options
                .get("schema")
                .map(|schema| schema.trim().to_string())
                .filter(|schema| !schema.is_empty())
        })
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少 SQL Server schema 上下文"))
}

fn sqlserver_qualified_table(schema: &str, table: &str) -> String {
    format!(
        "{}.{}",
        fluxdb_core::SqlServerDialect.quote_identifier(schema),
        fluxdb_core::SqlServerDialect.quote_identifier(table)
    )
}

#[derive(Default)]
struct SqlServerBatchResult {
    result_sets: Vec<SqlServerResultSet>,
}

fn sqlserver_store_row(
    result: &mut SqlServerBatchResult,
    mut row: Row,
    owner: Option<&str>,
) -> fluxdb_core::Result<()> {
    // 单格超阈值无条件外置；整页不再按累计字节截断。
    if let Some(owner) = owner {
        spill_large_cells(owner, &mut row.values)?;
    }
    if let Some(set) = result.result_sets.last_mut() {
            set.rows.push(row);
    }
    Ok(())
}

#[cfg(test)]
mod sqlserver_result_collection_tests {
    use super::*;
    use fluxdb_core::{CellContentMeta, CellContentStore, LargeValueKind};
    use std::sync::Arc;

    /// 只记录落盘长度、不真写文件的存储桩：这里验证的是「行有没有丢」。
    struct MemoryStore;

    impl CellContentStore for MemoryStore {
        fn put(
            &self,
            owner: &str,
            _kind: LargeValueKind,
            bytes: &[u8],
        ) -> fluxdb_core::Result<String> {
            Ok(format!("{owner}-{}", bytes.len()))
        }

        fn stat(&self, _content_id: &str) -> Option<CellContentMeta> {
            None
        }

        fn read(
            &self,
            _content_id: &str,
            _offset: u64,
            _len: usize,
        ) -> fluxdb_core::Result<Vec<u8>> {
            Ok(Vec::new())
        }

        fn release(&self, _content_id: &str) {}

        fn release_owner(&self, _owner: &str) {}
    }

    fn install_store() {
        fluxdb_core::register_cell_content_store(Arc::new(MemoryStore));
    }

    fn empty_result() -> SqlServerBatchResult {
        SqlServerBatchResult {
            result_sets: vec![SqlServerResultSet {
                columns: Vec::new(),
                rows: Vec::new(),
            }],
        }
    }

    #[test]
    fn small_rows_are_not_truncated_by_total_bytes() {
        let mut result = empty_result();
        for value in ["123456", "abcdef"] {
            sqlserver_store_row(
                &mut result,
                Row {
                    values: vec![CellValue::Text(value.into())],
                },
                None,
            )
            .expect("store row");
        }
        assert_eq!(result.result_sets[0].rows.len(), 2);
    }

    /// 有大字段的行不能因为超预算被丢掉：外置后仍要出现在页里，且不带正文。
    #[test]
    fn oversized_row_is_spilled_instead_of_dropped() {
        install_store();
        let mut result = empty_result();
        let oversized = vec![7u8; fluxdb_core::CELL_SPILL_THRESHOLD_BYTES + 1];
        sqlserver_store_row(
            &mut result,
            Row {
                values: vec![CellValue::Bytes(oversized.clone())],
            },
            Some("data-1-db.dbo.t"),
        )
        .expect("spill large cell");

        assert_eq!(result.result_sets[0].rows.len(), 1, "外置成功后行必须留在页里");
        let value = &result.result_sets[0].rows[0].values[0];
        assert!(
            matches!(value, CellValue::LargeValue(_)),
            "大字段必须换成内容引用：{value:?}"
            );
        }

    /// 没有归属的管理查询也保留大字段，不再按累计字节截断。
    #[test]
    fn oversized_row_without_owner_is_kept() {
        let mut result = empty_result();
        let oversized = vec![7u8; fluxdb_core::CELL_SPILL_THRESHOLD_BYTES + 1];
        sqlserver_store_row(
            &mut result,
            Row {
                values: vec![CellValue::Bytes(oversized)],
            },
            None,
        )
        .expect("store row");
        assert_eq!(result.result_sets[0].rows.len(), 1);
    }
}

struct SqlServerResultSet {
    columns: Vec<Column>,
    rows: Vec<Row>,
}
