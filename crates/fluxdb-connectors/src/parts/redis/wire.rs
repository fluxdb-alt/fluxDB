/// Redis 连接的底层流：明文 TCP，或经 rustls 包一层的 TLS。
/// 两者都是阻塞 IO，上层读写逻辑不需要区分。
enum RedisStream {
    Plain(std::net::TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>),
}

impl std::io::Read for RedisStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            RedisStream::Plain(stream) => stream.read(buf),
            RedisStream::Tls(stream) => stream.read(buf),
        }
    }
}

impl std::io::Write for RedisStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            RedisStream::Plain(stream) => stream.write(buf),
            RedisStream::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            RedisStream::Plain(stream) => stream.flush(),
            RedisStream::Tls(stream) => stream.flush(),
        }
    }
}

impl RedisStream {
    /// 设置底层流的读超时（用于 Pub/Sub 轮询：期望「无消息即超时」）。
    /// Plain（TCP）直接命中底层 socket；TLS 底层被 rustls 包裹，无法再改动超时，
    /// 保持默认 5s（该类连接的 Pub/Sub 推送可达性略降，属已记录限制，不影响功能）。
    fn set_read_timeout(&self, duration: Option<std::time::Duration>) -> std::io::Result<()> {
        match self {
            RedisStream::Plain(stream) => stream.set_read_timeout(duration),
            RedisStream::Tls(_) => Ok(()),
        }
    }
}

struct RedisConnection {
    reader: std::io::BufReader<RedisStream>,
    /// 当前 SELECT 的 DB 序号；复用连接时据此跳过重复 SELECT。
    database: Option<u32>,
    /// 命令出现 IO/协议错误后置位；该连接的读写状态已不可信，归还时直接丢弃而不入池。
    broken: bool,
    /// 连接配置。集群模式下收到 MOVED/ASK 需要按同样的凭据/TLS 参数连到新节点。
    config: Option<ConnectionConfig>,
    /// SSH 隧道句柄（走隧道时持有）。跟随连接存活：连接被池化复用期间隧道不塌，
    /// 连接丢弃时 Drop 关闭。集群重定向 / Sentinel 探测复用同一路径自动继承。
    #[allow(dead_code)] // 仅作为 RAII 生命周期锚点持有，不读取其内容。
    ssh_tunnel: Option<SshTunnel>,
}

/// 集群重定向：MOVED（槽已迁移，后续命令都该发到新节点）/ ASK（仅本次转发）。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisRedirect {
    ask: bool,
    host: String,
    port: u16,
}

/// 解析 `MOVED 3999 127.0.0.1:6381` / `ASK 3999 127.0.0.1:6381` 这类错误回包。
fn redis_parse_redirect(message: &str) -> Option<RedisRedirect> {
    let mut parts = message.split_whitespace();
    let kind = parts.next()?;
    let ask = match kind {
        "MOVED" => false,
        "ASK" => true,
        _ => return None,
    };
    let _slot = parts.next()?;
    let addr = parts.next()?;
    // IPv6 形如 [::1]:6379
    let (host, port) = addr.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some(RedisRedirect {
        ask,
        host: host.to_string(),
        port: port.parse().ok()?,
    })
}

/// 判断是否为「键类型不匹配」错误（WRONGTYPE）：对该键执行了与其实类型不符的命令。
fn redis_is_wrongtype(error: &fluxdb_core::Error) -> bool {
    error.message.contains("WRONGTYPE")
}

/// 把对非 Set 键执行集合命令触发的 WRONGTYPE 翻译成清晰中文提示。
/// 附带 database 与 key 便于核对是否串到了别的库/键（与 List 的处理口径一致）。
fn redis_wrongtype_set_message(
    object: &ObjectPath,
    database: u32,
    error: &fluxdb_core::Error,
) -> fluxdb_core::Error {
    Error::new(
        ErrorKind::Query,
        format!(
            "键「{}」在 DB {} 当前类型不是 Set，可能已被删除/重建为其他类型或已串库（原始: {}）",
            object.name,
            database,
            error.message.trim()
        ),
    )
}

/// 单条命令最多跟随的重定向次数，防止节点间互相指来指去时打转。
const REDIS_MAX_REDIRECTS: usize = 3;

impl RedisConnection {
    /// 批量下发命令（pipeline）：一次性写出全部请求，再按顺序读回全部响应。
    /// 用于「一页 N 个 Key 各要几条元信息」这类场景，把 N 次往返压成 1 次。
    fn command_pipeline(&mut self, commands: &[Vec<String>]) -> fluxdb_core::Result<Vec<RedisValue>> {
        if commands.is_empty() {
            return Ok(Vec::new());
        }
        let mut request = Vec::new();
        for args in commands {
            request.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());
            for arg in args {
                request.extend_from_slice(format!("${}\r\n", arg.as_bytes().len()).as_bytes());
                request.extend_from_slice(arg.as_bytes());
                request.extend_from_slice(b"\r\n");
            }
        }
        use std::io::Write;
        let result = (|| {
            self.reader
                .get_mut()
                .write_all(&request)
                .map_err(redis_io_error)?;
            self.reader.get_mut().flush().map_err(redis_io_error)?;
            let mut values = Vec::with_capacity(commands.len());
            for _ in 0..commands.len() {
                // 错误回包（含集群 MOVED）不能直接中断读取：
                // 剩余响应仍在连接里，必须全部读完，连接才还能继续用。
                match redis_read_value(&mut self.reader) {
                    Ok(value) => values.push(Ok(value)),
                    Err(error) if error.kind == ErrorKind::Query => values.push(Err(error)),
                    // IO / 协议层错误无法恢复，连接作废。
                    Err(error) => return Err(error),
                }
            }
            Ok(values)
        })();
        let values = match result {
            Ok(values) => values,
            Err(error) => {
                self.broken = true;
                return Err(error);
            }
        };

        // 集群下同一批 key 可能分布在不同节点，被 MOVED 的那几条退回单条执行（command 会跟随重定向）。
        let mut resolved = Vec::with_capacity(values.len());
        for (value, args) in values.into_iter().zip(commands) {
            match value {
                Ok(value) => resolved.push(value),
                Err(error) if redis_parse_redirect(&error.to_string()).is_some() => {
                    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
                    resolved.push(self.command(&args)?);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(resolved)
    }

    /// 发送一条命令；集群模式下自动跟随 MOVED/ASK 重定向。
    fn command(&mut self, args: &[&str]) -> fluxdb_core::Result<RedisValue> {
        let mut redirects = 0;
        loop {
            let result = self.command_once(args);
            let Err(error) = &result else {
                return result;
            };
            let Some(redirect) = redis_parse_redirect(&error.to_string()) else {
                return result;
            };
            if redirects >= REDIS_MAX_REDIRECTS {
                return Err(Error::new(
                    ErrorKind::Connection,
                    "Redis 集群重定向次数过多，请检查集群拓扑",
                ));
            }
            redirects += 1;
            self.follow_redirect(&redirect)?;
        }
    }

    /// 切到重定向目标节点：按同样的凭据/TLS 建连并替换当前流。
    /// ASK 只对本次命令有效，按协议要求先发一条 ASKING。
    fn follow_redirect(&mut self, redirect: &RedisRedirect) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.clone() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis 集群重定向缺少连接配置上下文",
            ));
        };
        let mut target = config.clone();
        target.endpoint = match &config.endpoint {
            Endpoint::Tcp { .. } => Endpoint::Tcp {
                host: redirect.host.clone(),
                port: redirect.port,
                database: None,
            },
            _ => {
                return Err(Error::new(
                    ErrorKind::Connection,
                    "Redis 集群重定向需要 TCP 端点",
                ));
            }
        };
        // 重定向目标不能再走哨兵解析，否则会被解析回原主库。
        target.options.remove("sentinel_master");
        let database = self.database;
        let mut connection = redis_new_connection(&target)?;
        if let Some(database) = database {
            // 集群只有 DB 0，这里的 SELECT 仅对单机分片布局有意义，失败不致命。
            let _ = redis_select(&mut connection, database);
        }
        if redirect.ask {
            connection.command_once(&["ASKING"])?;
        }
        self.reader = connection.reader;
        self.database = connection.database;
        self.broken = false;
        Ok(())
    }

    fn command_once(&mut self, args: &[&str]) -> fluxdb_core::Result<RedisValue> {
        let mut request = Vec::new();
        request.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());
        for arg in args {
            request.extend_from_slice(format!("${}\r\n", arg.as_bytes().len()).as_bytes());
            request.extend_from_slice(arg.as_bytes());
            request.extend_from_slice(b"\r\n");
        }
        use std::io::Write;
        // 任一步失败都说明连接状态不可信（半个请求可能已经写出去了），标记为不可复用。
        let result = (|| {
            self.reader
                .get_mut()
                .write_all(&request)
                .map_err(redis_io_error)?;
            self.reader.get_mut().flush().map_err(redis_io_error)?;
            redis_read_value(&mut self.reader)
        })();
        if result.is_err() {
            self.broken = true;
        }
        result
    }
}

/// 连接池分桶键：同一地址 + 同一身份的连接才可互换。
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RedisPoolKey {
    host: String,
    port: u16,
    username: String,
    password: String,
    /// TLS 开关与 SNI、以及 Sentinel 主库名都会改变连接的实际对端，
    /// 必须纳入分桶键，否则会把明文连接复用到 TLS 连接上。
    tls: bool,
    tls_server_name: String,
    sentinel_master: String,
}

/// 单个 key 最多缓存的空闲连接数。Redis 查询都是短命令，少量连接足够，
/// 上限用于防止大量 tab 并发查询后长期占着服务端连接数。
const REDIS_POOL_MAX_IDLE: usize = 4;

/// 空闲连接的最长存活时间。超过后主动丢弃，不再占用服务端连接数，
/// 也避免借出时才发现连接已被服务端 timeout 掉而白跑一次 PING。
const REDIS_POOL_IDLE_TTL: Duration = Duration::from_secs(60);

/// 池中的空闲连接：连接本身 + 归还时刻，用于按 REDIS_POOL_IDLE_TTL 淘汰。
struct RedisIdleConnection {
    connection: RedisConnection,
    idle_since: Instant,
}

fn redis_pool() -> &'static Mutex<HashMap<RedisPoolKey, Vec<RedisIdleConnection>>> {
    static POOL: OnceLock<Mutex<HashMap<RedisPoolKey, Vec<RedisIdleConnection>>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 借出的连接句柄：解引用即原始连接，离开作用域时自动归还到池里（broken 的直接丢弃）。
struct RedisSession {
    key: RedisPoolKey,
    connection: Option<RedisConnection>,
}

impl std::ops::Deref for RedisSession {
    type Target = RedisConnection;

    fn deref(&self) -> &Self::Target {
        self.connection
            .as_ref()
            .expect("Redis 连接在归还后被使用")
    }
}

impl std::ops::DerefMut for RedisSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.connection
            .as_mut()
            .expect("Redis 连接在归还后被使用")
    }
}

impl Drop for RedisSession {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        if connection.broken {
            return;
        }
        let Ok(mut pool) = redis_pool().lock() else {
            return;
        };
        let now = Instant::now();
        // 顺手清掉本桶里已经躺太久的连接，避免只进不出。
        let idle = pool.entry(self.key.clone()).or_default();
        idle.retain(|entry| now.duration_since(entry.idle_since) < REDIS_POOL_IDLE_TTL);
        if idle.len() < REDIS_POOL_MAX_IDLE {
            idle.push(RedisIdleConnection {
                connection,
                idle_since: now,
            });
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum RedisValue {
    Simple(String),
    Int(i64),
    Bulk(Option<Vec<u8>>),
    Array(Vec<RedisValue>),
}

/// 借一条可用连接：优先复用池里的空闲连接（用 PING 探活，1 次往返），
/// 没有可用的再新建 TCP + AUTH。返回的句柄在离开作用域时自动归还。
///
/// 这是所有 Redis 拨号（连接测试、对象浏览、工作台、CLI）的统一入口，
/// 因此在这里做「档案 → 扁平参数」归一化：只要配置携带 `redis_profile`，
/// 就用它派生 endpoint/options，保证 Browser / Workbench / CLI / Overview
/// 拿到的是同一套语义（TLS、哨兵、集群、认证）。
fn redis_connect(config: &ConnectionConfig) -> fluxdb_core::Result<RedisSession> {
    let config = config.redis_resolved();
    let key = redis_pool_key(&config)?;
    while let Some(mut connection) = redis_pool_take(&key) {
        // 服务端可能已按 timeout 断开空闲连接，探活失败就丢弃换新的。
        if connection.command(&["PING"]).is_ok() {
            return Ok(RedisSession {
                key,
                connection: Some(connection),
            });
        }
    }
    Ok(RedisSession {
        connection: Some(redis_new_connection(&config)?),
        key,
    })
}

/// 建立到 `endpoint` 的底层流（可能经 SSH 隧道改写目标）。返回实际流 + 隧道句柄。
///
/// - SSH 未启用：直连 `endpoint`；如启 TLS 则用真实主机名做 SNI/证书校验包裹。
/// - SSH 启用：先在目标端点开隧道，连本地转发端口，TLS 仍以**真实远端主机**做 SNI
///   （隧道本地是 127.0.0.1，直接用它会导致证书主机名/SNI 校验失败）。
/// 返回的 `SshTunnel` 必须随连接存活，故由调用方存入 `RedisConnection`。
fn redis_dial_endpoint(
    config: &ConnectionConfig,
    host: &str,
    port: u16,
) -> fluxdb_core::Result<(RedisStream, Option<SshTunnel>)> {
    let options = &config.options;
    let tunnel = if ssh_enabled(options) {
        let Some(auth) = ssh_auth_from_options(options, true) else {
            return Err(Error::new(ErrorKind::Connection, "SSH 隧道配置缺少认证参数"));
        };
        let jump_host = options.get("ssh_host").cloned().unwrap_or_default();
        let jump_port = options
            .get("ssh_port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(22);
        if jump_host.is_empty() {
            return Err(Error::new(ErrorKind::Connection, "请填写 SSH 跳板机主机"));
        }
        Some(SshTunnel::open((&jump_host, jump_port), &auth, (host, port))?)
    } else {
        None
    };

    // 拨号目标：走隧道则连本地转发端口，否则连远端真实端点。
    let (dial_host, dial_port) = match &tunnel {
        Some(t) => ("127.0.0.1".to_string(), t.local_port),
        None => (host.to_string(), port),
    };
    let stream = redis_tcp_connect(&dial_host, dial_port)?;

    // TLS：SNI / 证书校验的主机名始终用真实远端 Redis 主机，不能是 127.0.0.1。
    let stream = if redis_tls_enabled(config) {
        RedisStream::Tls(Box::new(redis_tls_wrap(config, host, stream)?))
    } else {
        RedisStream::Plain(stream)
    };
    Ok((stream, tunnel))
}

fn redis_pool_key(config: &ConnectionConfig) -> fluxdb_core::Result<RedisPoolKey> {
    let Endpoint::Tcp { host, port, .. } = &config.endpoint else {
        return Err(Error::new(
            ErrorKind::Connection,
            "Redis 连接需要 TCP 主机和端口",
        ));
    };
    Ok(RedisPoolKey {
        host: host.clone(),
        port: *port,
        username: config.options.get("username").cloned().unwrap_or_default(),
        password: config
            .options
            .get(PLAINTEXT_PASSWORD_OPTION)
            .cloned()
            .unwrap_or_default(),
        tls: redis_tls_enabled(config),
        tls_server_name: config
            .options
            .get("tls_server_name")
            .cloned()
            .unwrap_or_default(),
        sentinel_master: config
            .options
            .get("sentinel_master")
            .cloned()
            .unwrap_or_default(),
    })
}

fn redis_pool_take(key: &RedisPoolKey) -> Option<RedisConnection> {
    let mut pool = redis_pool().lock().ok()?;
    let idle = pool.get_mut(key)?;
    let now = Instant::now();
    // 后进先出：栈顶是最近归还的，最不可能已被服务端断开。
    while let Some(entry) = idle.pop() {
        if now.duration_since(entry.idle_since) < REDIS_POOL_IDLE_TTL {
            return Some(entry.connection);
        }
    }
    None
}

fn redis_new_connection(config: &ConnectionConfig) -> fluxdb_core::Result<RedisConnection> {
    let Endpoint::Tcp { host, port, .. } = &config.endpoint else {
        return Err(Error::new(
            ErrorKind::Connection,
            "Redis 连接需要 TCP 主机和端口",
        ));
    };
    // Sentinel 模式下先向哨兵问出当前主库地址，再连过去。
    let (host, port) = redis_resolve_endpoint(config, host, *port)?;
    let (stream, ssh_tunnel) = redis_dial_endpoint(config, &host, port)?;
    let mut connection = RedisConnection {
        reader: std::io::BufReader::new(stream),
        database: None,
        broken: false,
        config: Some(config.clone()),
        ssh_tunnel,
    };
    redis_auth(config, &mut connection)?;
    Ok(connection)
}

fn redis_tcp_connect(host: &str, port: u16) -> fluxdb_core::Result<std::net::TcpStream> {
    use std::net::ToSocketAddrs;
    let mut addrs = (host, port).to_socket_addrs().map_err(redis_io_error)?;
    let addr = addrs
        .next()
        .ok_or_else(|| Error::new(ErrorKind::Connection, "Redis 地址解析失败"))?;
    let stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(redis_io_error)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(redis_io_error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(redis_io_error)?;
    Ok(stream)
}

/// 是否启用 TLS：连接配置里的 `tls` 选项为 true/y/1 时启用（云上的 Redis 基本都强制 TLS）。
fn redis_tls_enabled(config: &ConnectionConfig) -> bool {
    config
        .options
        .get("tls")
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "true" | "y" | "yes" | "1" | "on"))
}

/// 是否跳过证书校验。仅用于自签证书的内网环境，默认关闭。
fn redis_tls_insecure(config: &ConnectionConfig) -> bool {
    config
        .options
        .get("tls_insecure")
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "true" | "y" | "yes" | "1" | "on"))
}

/// 用 rustls 把 TCP 流升级成 TLS。SNI 默认取连接主机名，可用 `tls_server_name` 覆盖
/// （连 IP 但证书签的是域名时需要）。
///
/// 支持自定义信任链与双向认证：
/// - `tls_ca_ref`：CA PEM 文件路径，加载后并入根证书；缺省用系统/内置信任锚。
/// - `tls_client_cert_ref` / `tls_client_key_ref`：客户端证书与私钥 PEM 文件路径，
///   两者都给了才启用 mTLS。
fn redis_tls_wrap(
    config: &ConnectionConfig,
    host: &str,
    stream: std::net::TcpStream,
) -> fluxdb_core::Result<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>> {
    let server_name = config
        .options
        .get("tls_server_name")
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or(host)
        .to_string();

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // 自定义 CA：把这些 PEM 证书追加进根证书。
    if let Some(ca_path) = config.options.get("tls_ca_ref").filter(|p| !p.is_empty()) {
        let certs = redis_load_cert_pem(ca_path)?;
        for cert in certs {
            roots
                .add(cert)
                .map_err(|e| Error::new(ErrorKind::Connection, format!("加载自定义 CA 失败: {e}")))?;
        }
    }

    let client_cert_path = config
        .options
        .get("tls_client_cert_ref")
        .filter(|p| !p.is_empty());
    let client_key_path = config
        .options
        .get("tls_client_key_ref")
        .filter(|p| !p.is_empty());
    let batch_has_mutual_tls = client_cert_path.is_some() && client_key_path.is_some();

    let builder = rustls::ClientConfig::builder().with_root_certificates(roots);
    let client_config = if batch_has_mutual_tls {
        // mTLS：加载客户端证书链与私钥。
        let certs = redis_load_cert_pem(client_cert_path.unwrap())?;
        let key = redis_load_private_key(client_key_path.unwrap())?;
        builder
            .with_client_auth_cert(certs, key)
            .map_err(|e| Error::new(ErrorKind::Connection, format!("加载客户端证书/私钥失败: {e}")))?
    } else {
        // 仅配了证书而没有私钥等不完整组合，视为配置错误而不是静默忽略。
        if client_cert_path.is_some() || client_key_path.is_some() {
            return Err(Error::new(
                ErrorKind::Connection,
                "TLS 客户端证书与私钥需成对配置",
            ));
        }
        builder.with_no_client_auth()
    };
    let mut client_config = client_config;
    if redis_tls_insecure(config) {
        // 显式开关才会走到这里：接受任意证书，等价于关闭中间人防护。
        client_config
            .dangerous()
            .set_certificate_verifier(std::sync::Arc::new(RedisInsecureCertVerifier));
    }

    let name = rustls_pki_types::ServerName::try_from(server_name)
        .map_err(|_| Error::new(ErrorKind::Connection, "TLS 服务器名称非法"))?;
    let connection = rustls::ClientConnection::new(std::sync::Arc::new(client_config), name)
        .map_err(|error| Error::new(ErrorKind::Connection, format!("TLS 握手失败: {error}")))?;
    Ok(rustls::StreamOwned::new(connection, stream))
}

/// 读取 PEM 文件的证书链。
fn redis_load_cert_pem(
    path: &str,
) -> fluxdb_core::Result<Vec<rustls_pki_types::CertificateDer<'static>>> {
    let file = std::fs::File::open(path).map_err(|e| {
        Error::new(
            ErrorKind::Connection,
            format!("无法打开证书文件 {path}: {e}"),
        )
    })?;
    rustls_pemfile::certs(&mut std::io::BufReader::new(file))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| {
            Error::new(
                ErrorKind::Connection,
                format!("解析证书文件 {path} 失败: {e}"),
            )
        })
}

/// 读取 PEM 私钥（支持 PKCS#8 / RSA / EC，未加密）。
fn redis_load_private_key(path: &str) -> fluxdb_core::Result<rustls_pki_types::PrivateKeyDer<'static>> {
    let file = std::fs::File::open(path).map_err(|e| {
        Error::new(
            ErrorKind::Connection,
            format!("无法打开私钥文件 {path}: {e}"),
        )
    })?;
    rustls_pemfile::private_key(&mut std::io::BufReader::new(file))
        .map_err(|e| {
            Error::new(
                ErrorKind::Connection,
                format!("解析私钥文件 {path} 失败: {e}"),
            )
        })?
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Connection,
                format!("私钥文件 {path} 中未找到可用的私钥"),
            )
        })
}

/// 跳过证书校验的验证器，只在 `tls_insecure` 打开时使用。
#[derive(Debug)]
struct RedisInsecureCertVerifier;

impl rustls::client::danger::ServerCertVerifier for RedisInsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Sentinel 支持：配置了 `sentinel_master` 时，把配置里的地址当作哨兵地址，
/// 用 `SENTINEL get-master-addr-by-name <name>` 问出当前主库地址再连过去。
/// 没配置就直连原地址。
fn redis_resolve_endpoint(
    config: &ConnectionConfig,
    host: &str,
    port: u16,
) -> fluxdb_core::Result<(String, u16)> {
    let Some(master) = config
        .options
        .get("sentinel_master")
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
    else {
        return Ok((host.to_string(), port));
    };

    // 哨兵探测同样遵循 SSH/TLS：SSH 场景下哨兵也可能在跳板机后，直连会失败。
    // 隧道句柄在本函数作用域内存活，探测结束即随析构关闭。
    let (stream, _ssh_tunnel) = redis_dial_endpoint(config, host, port)?;
    let mut sentinel = RedisConnection {
        reader: std::io::BufReader::new(stream),
        database: None,
        broken: false,
        config: None,
        ssh_tunnel: _ssh_tunnel,
    };
    // 哨兵也可能要求认证，沿用同一套凭据。
    redis_auth(config, &mut sentinel)?;
    match sentinel.command(&["SENTINEL", "get-master-addr-by-name", master])? {
        RedisValue::Array(items) => match items.as_slice() {
            [master_host, master_port] => {
                let master_host = redis_value_text(master_host.clone());
                let master_port = redis_value_text(master_port.clone())
                    .parse::<u16>()
                    .map_err(|_| Error::new(ErrorKind::Connection, "哨兵返回的主库端口非法"))?;
                Ok((master_host, master_port))
            }
            _ => Err(Error::new(ErrorKind::Connection, "哨兵返回格式异常")),
        },
        // 主库名不存在时哨兵返回 nil
        _ => Err(Error::new(
            ErrorKind::Connection,
            format!("哨兵没有找到名为 {master} 的主库"),
        )),
    }
}

fn redis_auth(
    config: &ConnectionConfig,
    connection: &mut RedisConnection,
) -> fluxdb_core::Result<()> {
    let Some(password) = config
        .options
        .get(PLAINTEXT_PASSWORD_OPTION)
        .map(String::as_str)
        .filter(|password| !password.is_empty())
    else {
        return Ok(());
    };
    let username = config
        .options
        .get("username")
        .map(String::as_str)
        .filter(|username| !username.trim().is_empty());
    let response = if let Some(username) = username {
        connection.command(&["AUTH", username, password])?
    } else {
        connection.command(&["AUTH", password])?
    };
    redis_expect_ok(response)
}
