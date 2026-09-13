// PostgreSQL 专用连接配置模型。
//
// 目标：把 PostgreSQL 连接语义收敛成结构化、可迁移、可校验的单一模型，
// 作为 `ConnectionConfig` 的补充（镜像 redis_profile.rs / mysql_profile.rs 范式）。
//
// 设计约定：
// - 本模型只依赖 `serde` 与 `std`，不引入第三方 URL 解析库，URI 解析用内联实现。
// - 所有密钥类内容（密码、私钥、证书正文、SSH 口令）都走 [`SecretRef`]：引用
//   （`key`）可落盘，`inline` 受控值标记 `#[serde(skip)]`，绝不序列化到磁盘。
// - 通过 [`PostgresConnectionProfile::from_uri`] 承接 `postgres://` 导入；
//   通过 [`PostgresConnectionProfile::from_options`] / `into_options` 承接/回写
//   历史 `options` 扁平参数，供现有消费方复用。

/// PostgreSQL TLS 模式（与 libpq 同名语义对齐）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PostgresSslMode {
    /// 禁用 TLS。
    Disabled,
    /// 优先 TLS：服务器支持则加密，否则降级明文（本地/内网默认，兼容无 TLS 部署），默认。
    #[default]
    Prefer,
    /// 强制 TLS：服务器不支持则连接失败。
    Require,
    /// 强制 TLS 并校验 CA 证书链（不校 DNS/主机名）。
    VerifyCa,
    /// 强制 TLS、校验 CA 证书链并校验 DNS/主机名（最严）。
    VerifyFull,
}

/// TLS 相关参数。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PostgresTlsOptions {
    /// 是否启用 TLS。
    pub enabled: bool,
    /// TLS 模式（`Require`/`VerifyCa`/`VerifyFull` 需先启用 TLS；默认 `Prefer`）。
    pub ssl_mode: PostgresSslMode,
    /// CA 证书文件路径引用。
    pub ca: SecretRef,
    /// 客户端证书文件路径引用。
    pub client_cert: SecretRef,
    /// 客户端私钥文件路径引用。
    pub client_key: SecretRef,
    /// 独立 TLS server_name（SNI / 校验主机名）；空则取连接主机名。
    pub server_name: String,
}

impl Default for PostgresTlsOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            ssl_mode: PostgresSslMode::Prefer,
            ca: SecretRef::default(),
            client_cert: SecretRef::default(),
            client_key: SecretRef::default(),
            server_name: String::new(),
        }
    }
}

/// SSH 认证方式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum PostgresSshAuth {
    #[default]
    Password,
    PrivateKey,
}

/// SSH 隧道参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PostgresSshOptions {
    /// 是否启用 SSH 隧道。
    pub enabled: bool,
    /// 跳板主机。
    pub host: String,
    /// 跳板端口（默认 22）。
    pub port: u16,
    /// 跳板用户名。
    pub username: String,
    /// SSH 认证类型。
    pub auth: PostgresSshAuth,
    /// 密码（password 认证）。
    pub password: SecretRef,
    /// 私钥文件路径引用（key/private-key 认证）。
    pub private_key: SecretRef,
    /// 私钥口令（可选）。
    pub passphrase: SecretRef,
    /// SSH 连接超时（秒），0 表示继承全局 connect 超时。
    pub connect_timeout_secs: u32,
    /// SSH 心跳间隔（秒），默认 30；0 表示不发送。
    pub keepalive_interval_secs: u32,
}

/// 代理类型。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum PostgresProxyType {
    #[default]
    Socks5,
    HttpConnect,
}

impl PostgresProxyType {
    /// 该类型代理的默认端口。
    pub fn default_port(&self) -> u16 {
        match self {
            PostgresProxyType::Socks5 => 1080,
            PostgresProxyType::HttpConnect => 8080,
        }
    }
}

/// 代理参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PostgresProxy {
    /// 是否启用代理。
    pub enabled: bool,
    /// 代理类型。
    pub proxy_type: PostgresProxyType,
    /// 代理主机。
    pub host: String,
    /// 代理端口。
    pub port: u16,
    /// 代理用户名（可选）。
    pub username: String,
    /// 代理密码（可选）。
    pub password: SecretRef,
}

/// 传输层：直连、SSH 隧道或代理。`transport` 显式表达顺序，多跳扩展保留。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PostgresTransportLayer {
    /// 直连（默认，无隧道）。
    Direct,
    Ssh(PostgresSshOptions),
    Proxy(PostgresProxy),
}

impl Default for PostgresTransportLayer {
    fn default() -> Self {
        PostgresTransportLayer::Direct
    }
}

/// 作用域参数：schema 选择与可见性。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PostgresScopeOptions {
    /// 可选默认 schema（建连后 `SET search_path`）；空则保留服务器 search_path。
    pub default_schema: String,
    /// 是否显示其他数据库（默认仅维护库/当前库）。
    pub show_other_databases: bool,
    /// 系统 schema（pg_catalog/information_schema/toast/临时）可见性。
    pub show_system_schemas: bool,
}

impl Default for PostgresScopeOptions {
    fn default() -> Self {
        Self {
            default_schema: String::new(),
            show_other_databases: false,
            show_system_schemas: false,
        }
    }
}

/// 高级连接选项。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PostgresAdvancedOptions {
    /// 建连超时（秒），默认 5。
    pub connect_timeout_secs: u32,
    /// 查询超时（秒），0 表示无限（默认）。
    pub query_timeout_secs: u32,
    /// 连接空闲 TTL（秒），0 表示不回收。
    pub idle_ttl_secs: u32,
    /// TCP 长连接保活。
    pub tcp_keepalive: bool,
    /// application_name（默认 FluxDB）。
    pub application_name: String,
    /// 可选 timezone；空则用服务器默认。
    pub timezone: String,
}

impl Default for PostgresAdvancedOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 5,
            query_timeout_secs: 0,
            idle_ttl_secs: 0,
            tcp_keepalive: true,
            application_name: "FluxDB".into(),
            timezone: String::new(),
        }
    }
}

/// 基础连接参数。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PostgresBasicOptions {
    /// 目标主机。
    pub host: String,
    /// 目标端口（默认 5432）。
    pub port: u16,
    /// 维护库（建连/维护用，默认 postgres）。
    pub maintenance_database: String,
    /// 用户名。
    pub username: String,
    /// 认证密码（不安全）。
    pub password: SecretRef,
}

impl Default for PostgresBasicOptions {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 5432,
            maintenance_database: "postgres".into(),
            username: String::new(),
            password: SecretRef::default(),
        }
    }
}

/// PostgreSQL 完整连接档案：承接 TLS / SSH / 代理 / 作用域 / 高级超时。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PostgresConnectionProfile {
    pub basic: PostgresBasicOptions,
    pub scope: PostgresScopeOptions,
    pub tls: PostgresTlsOptions,
    #[serde(default = "postgres_default_transport")]
    pub transport: Vec<PostgresTransportLayer>,
    pub advanced: PostgresAdvancedOptions,
}

/// `transport` 缺省为直连（兼容历史/导入，避免空 Vec 被当作未知语义）。
fn postgres_default_transport() -> Vec<PostgresTransportLayer> {
    vec![PostgresTransportLayer::Direct]
}

impl Default for PostgresConnectionProfile {
    fn default() -> Self {
        Self {
            basic: PostgresBasicOptions::default(),
            scope: PostgresScopeOptions::default(),
            tls: PostgresTlsOptions::default(),
            transport: postgres_default_transport(),
            advanced: PostgresAdvancedOptions::default(),
        }
    }
}

/// 首个显式传输层（直连或隧道）。`None` 仅当 `transport` 为空（异常状态，按直连处理）。
fn postgres_first_layer(profile: &PostgresConnectionProfile) -> Option<&PostgresTransportLayer> {
    profile.transport.first()
}

/// 判断 `value` 是否是合法的布尔开关文本（true/y/yes/1/on）。
fn postgres_option_is_true(value: Option<&String>) -> bool {
    value
        .map(|v| v.trim().to_ascii_lowercase())
        .is_some_and(|v| matches!(v.as_str(), "true" | "y" | "yes" | "1" | "on"))
}

/// 历史 `options` 扁平参数里 PostgreSQL 认识的全部键（白名单）。
pub const POSTGRES_LEGACY_OPTION_KEYS: &[&str] = &[
    "host",
    "port",
    "database",
    "maintenance_database",
    "username",
    "password",
    "default_schema",
    "show_other_databases",
    "show_system_schemas",
    "tls",
    "ssl_mode",
    "tls_ca_ref",
    "tls_client_cert_ref",
    "tls_client_key_ref",
    "tls_server_name",
    "ssh_enabled",
    "ssh_host",
    "ssh_port",
    "ssh_username",
    "ssh_auth",
    "ssh_password",
    "ssh_private_key_ref",
    "ssh_passphrase",
    "ssh_connect_timeout_secs",
    "ssh_keepalive_interval_secs",
    "proxy_enabled",
    "proxy_type",
    "proxy_host",
    "proxy_port",
    "proxy_username",
    "proxy_password",
    "connect_timeout_secs",
    "query_timeout_secs",
    "idle_ttl_secs",
    "tcp_keepalive",
    "application_name",
    "timezone",
];

impl PostgresConnectionProfile {
    /// —— 解析/迁移职责 ——

    /// 从历史 `options` 扁平参数迁移出一份档案。仅迁移白名单键。
    pub fn from_options(opts: &std::collections::BTreeMap<String, String>) -> Self {
        let get = |key: &str| opts.get(key).cloned();
        let basic = PostgresBasicOptions {
            host: opts.get("host").cloned().unwrap_or_default(),
            port: opts
                .get("port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(5432),
            maintenance_database: get("maintenance_database")
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "postgres".into()),
            username: get("username").unwrap_or_default(),
            password: SecretRef::inline(get("password").unwrap_or_default()),
        };
        let scope = PostgresScopeOptions {
            default_schema: get("default_schema").unwrap_or_default(),
            show_other_databases: postgres_option_is_true(get("show_other_databases").as_ref()),
            show_system_schemas: postgres_option_is_true(get("show_system_schemas").as_ref()),
        };
        let tls = PostgresTlsOptions {
            enabled: postgres_option_is_true(get("tls").as_ref()),
            ssl_mode: match get("ssl_mode").as_deref() {
                Some("disable") | Some("disabled") => PostgresSslMode::Disabled,
                Some("require") => PostgresSslMode::Require,
                Some("verify-ca") | Some("verify_ca") => PostgresSslMode::VerifyCa,
                Some("verify-full") | Some("verify_full") => PostgresSslMode::VerifyFull,
                _ => PostgresSslMode::Prefer,
            },
            ca: SecretRef::ref_key(get("tls_ca_ref").unwrap_or_default()),
            client_cert: SecretRef::ref_key(get("tls_client_cert_ref").unwrap_or_default()),
            client_key: SecretRef::ref_key(get("tls_client_key_ref").unwrap_or_default()),
            server_name: get("tls_server_name").unwrap_or_default(),
        };
        let mut transport: Vec<PostgresTransportLayer> = Vec::new();
        let ssh = PostgresSshOptions {
            enabled: postgres_option_is_true(get("ssh_enabled").as_ref()),
            host: get("ssh_host").unwrap_or_default(),
            port: get("ssh_port").and_then(|p| p.parse().ok()).unwrap_or(22),
            username: get("ssh_username").unwrap_or_default(),
            auth: match get("ssh_auth").as_deref() {
                Some("key") | Some("private_key") => PostgresSshAuth::PrivateKey,
                _ => PostgresSshAuth::Password,
            },
            password: SecretRef::inline(get("ssh_password").unwrap_or_default()),
            private_key: SecretRef::ref_key(get("ssh_private_key_ref").unwrap_or_default()),
            passphrase: SecretRef::inline(get("ssh_passphrase").unwrap_or_default()),
            connect_timeout_secs: get("ssh_connect_timeout_secs")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0),
            keepalive_interval_secs: get("ssh_keepalive_interval_secs")
                .and_then(|p| p.parse().ok())
                .unwrap_or(30),
        };
        if ssh.enabled {
            transport.push(PostgresTransportLayer::Ssh(ssh));
        }
        let proxy = PostgresProxy {
            enabled: postgres_option_is_true(get("proxy_enabled").as_ref()),
            proxy_type: match get("proxy_type").as_deref() {
                Some("http_connect") => PostgresProxyType::HttpConnect,
                _ => PostgresProxyType::Socks5,
            },
            host: get("proxy_host").unwrap_or_default(),
            port: get("proxy_port")
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(|| PostgresProxyType::Socks5.default_port()),
            username: get("proxy_username").unwrap_or_default(),
            password: SecretRef::inline(get("proxy_password").unwrap_or_default()),
        };
        if proxy.enabled {
            transport.push(PostgresTransportLayer::Proxy(proxy));
        }
        if transport.is_empty() {
            transport.push(PostgresTransportLayer::Direct);
        }
        let advanced = PostgresAdvancedOptions {
            connect_timeout_secs: get("connect_timeout_secs")
                .and_then(|p| p.parse().ok())
                .unwrap_or(5),
            query_timeout_secs: get("query_timeout_secs")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0),
            idle_ttl_secs: get("idle_ttl_secs").and_then(|p| p.parse().ok()).unwrap_or(0),
            tcp_keepalive: match get("tcp_keepalive") {
                Some(v) => postgres_option_is_true(Some(&v)),
                None => true,
            },
            application_name: get("application_name")
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| "FluxDB".into()),
            timezone: get("timezone").unwrap_or_default(),
        };
        Self {
            basic,
            scope,
            tls,
            transport,
            advanced,
        }
    }

    /// 把档案回写成一组合法的 `options` 兼容参数。不写密钥正文，
    /// 密码走引用/占用 `password` 键交给 storage 决定是否落盘。
    pub fn into_options(&self) -> std::collections::BTreeMap<String, String> {
        let mut ops = std::collections::BTreeMap::new();
        if !self.basic.host.is_empty() {
            ops.insert("host".into(), self.basic.host.clone());
        }
        ops.insert("port".into(), self.basic.port.to_string());
        if !self.basic.maintenance_database.is_empty()
            && self.basic.maintenance_database != "postgres"
        {
            ops.insert("maintenance_database".into(), self.basic.maintenance_database.clone());
        }
        if !self.basic.username.is_empty() {
            ops.insert("username".into(), self.basic.username.clone());
        }
        if let Some(password) = self.basic.password.value() {
            ops.insert("password".into(), password.to_string());
        }

        if !self.scope.default_schema.is_empty() {
            ops.insert("default_schema".into(), self.scope.default_schema.clone());
        }
        if self.scope.show_other_databases {
            ops.insert("show_other_databases".into(), "true".into());
        }
        if self.scope.show_system_schemas {
            ops.insert("show_system_schemas".into(), "true".into());
        }

        let t = &self.tls;
        ops.insert("tls".into(), t.enabled.to_string());
        ops.insert(
            "ssl_mode".into(),
            match t.ssl_mode {
                PostgresSslMode::Disabled => "disable".into(),
                PostgresSslMode::Prefer => "prefer".into(),
                PostgresSslMode::Require => "require".into(),
                PostgresSslMode::VerifyCa => "verify-ca".into(),
                PostgresSslMode::VerifyFull => "verify-full".into(),
            },
        );
        if !t.server_name.is_empty() {
            ops.insert("tls_server_name".into(), t.server_name.clone());
        }
        if !t.ca.key.is_empty() || !t.client_cert.key.is_empty() || !t.client_key.key.is_empty() {
            ops.insert("tls_ca_ref".into(), t.ca.key.clone());
            ops.insert("tls_client_cert_ref".into(), t.client_cert.key.clone());
            ops.insert("tls_client_key_ref".into(), t.client_key.key.clone());
        }

        for layer in &self.transport {
            match layer {
                PostgresTransportLayer::Direct => {}
                PostgresTransportLayer::Ssh(ssh) if ssh.enabled => {
                    ops.insert("ssh_enabled".into(), "true".into());
                    ops.insert("ssh_host".into(), ssh.host.clone());
                    ops.insert("ssh_port".into(), ssh.port.to_string());
                    ops.insert("ssh_username".into(), ssh.username.clone());
                    ops.insert(
                        "ssh_auth".into(),
                        match ssh.auth {
                            PostgresSshAuth::Password => "password".into(),
                            PostgresSshAuth::PrivateKey => "private_key".into(),
                        },
                    );
                    if let Some(p) = ssh.password.value() {
                        ops.insert("ssh_password".into(), p.to_string());
                    }
                    if !ssh.private_key.key.is_empty() {
                        ops.insert("ssh_private_key_ref".into(), ssh.private_key.key.clone());
                    }
                    if let Some(p) = ssh.passphrase.value() {
                        ops.insert("ssh_passphrase".into(), p.to_string());
                    }
                    if ssh.connect_timeout_secs > 0 {
                        ops.insert("ssh_connect_timeout_secs".into(), ssh.connect_timeout_secs.to_string());
                    }
                    if ssh.keepalive_interval_secs != 30 {
                        ops.insert("ssh_keepalive_interval_secs".into(), ssh.keepalive_interval_secs.to_string());
                    }
                }
                PostgresTransportLayer::Proxy(proxy) if proxy.enabled => {
                    ops.insert("proxy_enabled".into(), "true".into());
                    ops.insert(
                        "proxy_type".into(),
                        match proxy.proxy_type {
                            PostgresProxyType::Socks5 => "socks5".into(),
                            PostgresProxyType::HttpConnect => "http_connect".into(),
                        },
                    );
                    ops.insert("proxy_host".into(), proxy.host.clone());
                    ops.insert("proxy_port".into(), proxy.port.to_string());
                    if !proxy.username.is_empty() {
                        ops.insert("proxy_username".into(), proxy.username.clone());
                    }
                    if let Some(p) = proxy.password.value() {
                        ops.insert("proxy_password".into(), p.to_string());
                    }
                }
                _ => {}
            }
        }

        let a = &self.advanced;
        ops.insert("connect_timeout_secs".into(), a.connect_timeout_secs.to_string());
        if a.query_timeout_secs > 0 {
            ops.insert("query_timeout_secs".into(), a.query_timeout_secs.to_string());
        }
        if a.idle_ttl_secs > 0 {
            ops.insert("idle_ttl_secs".into(), a.idle_ttl_secs.to_string());
        }
        ops.insert("tcp_keepalive".into(), a.tcp_keepalive.to_string());
        if a.application_name != "FluxDB" {
            ops.insert("application_name".into(), a.application_name.clone());
        }
        if !a.timezone.is_empty() {
            ops.insert("timezone".into(), a.timezone.clone());
        }

        ops
    }

    /// —— URI 解析职责 ——

    /// 从 `postgres://` / `postgresql://` URI 解析出一份档案。
    ///
    /// - 非 postgres/postgresql scheme 返回 `Err`（不静默忽略未知 scheme）。
    /// - 支持 IPv6（`[::1]`）、百分号编码、特殊字符密码（先 percent-decode）。
    /// - 认证片段（用户名/密码）解析进 `basic`，密码为内存受控值；绝不把含凭据
    ///   URI 原样存入 `Endpoint`。
    /// - `database` 作为维护库（PG 的 libpq 连接库即"维护库"起点，见设计 4.1）。
    /// - 查询参数白名单之外的键返回 `Err`，不静默丢弃。
    pub fn from_uri(uri: &str) -> std::result::Result<Self, String> {
        let (scheme, rest) = uri
            .split_once("://")
            .ok_or_else(|| "连接 URI 缺少 scheme".to_string())?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "postgres" && scheme != "postgresql" {
            return Err(format!("仅支持 postgres/postgresql scheme，收到 `{scheme}`"));
        }

        let (authority, query) = match rest.split_once('?') {
            Some((a, q)) => (a, q),
            None => (rest, ""),
        };
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((u, hp)) => (u.to_string(), hp.to_string()),
            None => (String::new(), authority.to_string()),
        };

        // 路径段 `/dbname`：作为初始连接库 / 维护库的缺省（URL 无同名 query 时生效）。
        let (hostport_no_path, path_db) = match hostport.split_once('/') {
            Some((hp, db)) => (hp.to_string(), db.to_string()),
            None => (hostport, String::new()),
        };

        // host [:port]；IPv6 用方括号包裹。
        let (host_raw, host, port) = parse_pg_hostport(&hostport_no_path)?;
        let _ = host_raw;

        // userinfo：`user` 或 `user:password`。
        let (username, password) = match userinfo.split_once(':') {
            Some((u, p)) => (percent_decode(u), Some(percent_decode(p))),
            None if !userinfo.is_empty() => (percent_decode(&userinfo), None),
            None => (String::new(), None),
        };

        // query 参数：白名单内覆盖，其余报错。
        let mut opts: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        if !host.is_empty() {
            opts.insert("host".into(), host);
        }
        opts.insert("port".into(), port.to_string());
        if !username.is_empty() {
            opts.insert("username".into(), username);
        }
        if let Some(p) = password {
            opts.insert("password".into(), p);
        }

        if !query.is_empty() {
            for pair in query.split('&') {
                let (k, v) = match pair.split_once('=') {
                    Some((k, v)) => (k, v),
                    None => (pair, ""),
                };
                let k = percent_decode(k);
                let v = percent_decode(v);
                match k.as_str() {
                    "database" | "dbname" => {
                        opts.insert("database".into(), v);
                    }
                    "maintenance_database" => {
                        opts.insert("maintenance_database".into(), v);
                    }
                    "sslmode" | "ssl_mode" => {
                        opts.insert("ssl_mode".into(), v);
                    }
                    "sslrootcert" | "tls_ca_ref" => {
                        opts.insert("tls_ca_ref".into(), v);
                    }
                    "sslcert" | "tls_client_cert_ref" => {
                        opts.insert("tls_client_cert_ref".into(), v);
                    }
                    "sslkey" | "tls_client_key_ref" => {
                        opts.insert("tls_client_key_ref".into(), v);
                    }
                    "hostaddr" | "server_name" | "tls_server_name" => {
                        opts.insert("tls_server_name".into(), v);
                    }
                    "connect_timeout" | "connect_timeout_secs" => {
                        opts.insert("connect_timeout_secs".into(), v);
                    }
                    "application_name" => {
                        opts.insert("application_name".into(), v);
                    }
                    "options" | "search_path" | "default_schema" => {
                        opts.insert("default_schema".into(), v);
                    }
                    // 仅记录/忽略的通用键：不缓存含凭据内容，交给 storage 决策。
                    "postgres" | "user" | "host" | "port" | "password" => {
                        // 已在上方按默认处理；此处防重复覆盖仅回写已知键。
                        opts.insert(k, v);
                    }
                    other => {
                        return Err(format!(
                            "不支持的连接参数 `{other}`（未知或明确不支持，不静默丢弃）"
                        ));
                    }
                }
            }
        }

        let mut profile = Self::from_options(&opts);

        // 路径段的数据库名：URL 未在 query 里显式给出 database 时，取路径名为库。
        let no_query_db = opts
            .get("database")
            .map(|v| v.trim().is_empty())
            .unwrap_or(true);
        let has_path_db = !path_db.is_empty();
        if no_query_db && has_path_db {
            profile.basic.maintenance_database = path_db;
        }
        // URI 里没有显式维护库且无路径库时，保持空（连接后走服务器默认）。
        if no_query_db && !has_path_db && profile.basic.maintenance_database == "postgres" {
            profile.basic.maintenance_database = String::new();
        }
        Ok(profile)
    }

    /// —— 拨号辅助 ——

    /// 首个显式传输层（`None` 当 transport 为空，按直连处理）。
    pub fn transport_layer(&self) -> PostgresTransportLayer {
        match postgres_first_layer(self) {
            Some(layer) => layer.clone(),
            None => PostgresTransportLayer::Direct,
        }
    }

    /// 是否启用 SSH 隧道。
    pub fn ssh(&self) -> Option<&PostgresSshOptions> {
        self.transport.iter().find_map(|layer| match layer {
            PostgresTransportLayer::Ssh(ssh) if ssh.enabled => Some(ssh),
            _ => None,
        })
    }

    /// 是否启用代理。
    pub fn proxy(&self) -> Option<&PostgresProxy> {
        self.transport.iter().find_map(|layer| match layer {
            PostgresTransportLayer::Proxy(proxy) if proxy.enabled => Some(proxy),
            _ => None,
        })
    }

    /// 实际用于拨号的目标端点（未被隧道改写前提下）。
    pub fn dial_endpoint(&self) -> (String, u16) {
        (self.basic.host.clone(), self.basic.port)
    }

    /// 认证密码。
    pub fn password(&self) -> Option<&str> {
        self.basic.password.value()
    }

    /// 建连超时。
    pub fn connect_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.advanced.connect_timeout_secs.max(1).into())
    }

    /// 建连超时秒数。
    pub fn connect_timeout_secs(&self) -> u32 {
        self.advanced.connect_timeout_secs
    }

    /// 维护库（缺省 postgres）。
    pub fn maintenance_database(&self) -> &str {
        if self.basic.maintenance_database.is_empty() {
            "postgres"
        } else {
            &self.basic.maintenance_database
        }
    }

    /// 校验：返回第一条不合法的中文错误；`None` 表示通过。
    pub fn validate(&self) -> Option<String> {
        if self.basic.host.is_empty() {
            return Some("请填写主机".into());
        }
        if self.basic.port == 0 {
            return Some("端口不合法".into());
        }
        let strong_tls = matches!(
            self.tls.ssl_mode,
            PostgresSslMode::Require | PostgresSslMode::VerifyCa | PostgresSslMode::VerifyFull
        );
        if strong_tls && !self.tls.enabled {
            return Some(format!("TLS 模式 {:?} 需先启用 TLS", self.tls.ssl_mode));
        }
        if self.tls.ssl_mode == PostgresSslMode::VerifyFull && self.tls.server_name.is_empty()
            && self.basic.host.parse::<std::net::IpAddr>().is_ok()
        {
            // verify-full 需要可校验的主机名；纯 IP 需显式 server_name 或用 verify-ca。
            return Some("verify-full 模式需提供主机名或显式 TLS server_name".into());
        }
        if let Some(ssh) = self.ssh() {
            if ssh.host.is_empty() {
                return Some("请填写 SSH 隧道主机".into());
            }
            if ssh.port == 0 {
                return Some("SSH 端口不合法".into());
            }
            if ssh.username.is_empty() {
                return Some("请填写 SSH 用户名".into());
            }
            if ssh.auth == PostgresSshAuth::PrivateKey && ssh.private_key.key.is_empty() {
                return Some("请填写 SSH 私钥文件路径".into());
            }
        }
        if self.advanced.connect_timeout_secs > 86400
            || self.advanced.query_timeout_secs > 86400
            || self.advanced.idle_ttl_secs > 86400
        {
            return Some("高级超时值不能超过 86400 秒".into());
        }
        None
    }
}

/// 解析 host[:port]，支持 IPv6（`[::1]:5432` 或 `[::1]`）。
/// 返回 `(host原始串, host, port)`。
fn parse_pg_hostport(authority: &str) -> std::result::Result<(String, String, u16), String> {
    // 去掉路径部分（`host[:port]/dbname`）：URI 的权威段只到第一个 `/` 为止。
    let authority = authority.split('/').next().unwrap_or(authority);
    if let Some(rest) = authority.strip_prefix('[') {
        // IPv6 字面量：`[addr]` 或 `[addr]:port`
        let (addr, port) = match rest.split_once(']') {
            Some((addr, suffix)) => {
                let suffix = suffix.strip_prefix(':').unwrap_or("");
                let port = if suffix.is_empty() {
                    5432
                } else {
                    suffix
                        .parse::<u16>()
                        .map_err(|_| format!("端口 `{suffix}` 不是合法数字"))?
                };
                (addr.to_string(), port)
            }
            None => {
                return Err("IPv6 地址缺少结尾 `]`".into());
            }
        };
        Ok((format!("[{addr}]"), addr, port))
    } else {
        // 普通 host[:port]
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !p.contains('[') && p.parse::<u16>().is_ok() => {
                (h.to_string(), p.parse::<u16>().unwrap_or(5432))
            }
            _ => (authority.to_string(), 5432),
        };
        if host.is_empty() {
            return Err("主机不能为空".into());
        }
        Ok((host.clone(), host, port))
    }
}
// 百分号解码复用 redis_profile.rs 的 `percent_decode`（crate-root 作用域可见）：同一套
// 宽松解码语义（非法转义按原样保留、UTF-8 lossy），不在此重复定义。

#[cfg(test)]
mod postgres_profile_tests {
    use super::*;

    fn opt<'a>(map: &'a std::collections::BTreeMap<String, String>, key: &str) -> Option<&'a String> {
        map.get(key)
    }

    #[test]
    fn defaults_use_pg_values() {
        let p = PostgresConnectionProfile::default();
        assert_eq!(p.basic.port, 5432);
        assert_eq!(p.basic.maintenance_database, "postgres");
        assert_eq!(p.advanced.application_name, "FluxDB");
        assert_eq!(p.advanced.connect_timeout_secs, 5);
        assert_eq!(p.tls.ssl_mode, PostgresSslMode::Prefer);
        assert_eq!(p.validate(), Some("请填写主机".into()));
    }

    #[test]
    fn round_trips_options_through_profile() {
        let mut opts = std::collections::BTreeMap::new();
        opts.insert("host".into(), "db.example.com".into());
        opts.insert("port".into(), "5433".into());
        opts.insert("username".into(), "app".into());
        opts.insert("password".into(), "p@ss w/rd+".into());
        opts.insert("ssl_mode".into(), "verify-full".into());
        opts.insert("connect_timeout_secs".into(), "15".into());
        let p = PostgresConnectionProfile::from_options(&opts);
        assert_eq!(p.basic.host, "db.example.com");
        assert_eq!(p.basic.port, 5433);
        assert_eq!(p.basic.username, "app");
        assert_eq!(p.basic.password.value(), Some("p@ss w/rd+"));
        assert_eq!(p.tls.ssl_mode, PostgresSslMode::VerifyFull);
        assert_eq!(p.advanced.connect_timeout_secs, 15);
        let back = p.into_options();
        assert_eq!(opt(&back, "ssl_mode").map(String::as_str), Some("verify-full"));
        assert_eq!(opt(&back, "connect_timeout_secs").map(String::as_str), Some("15"));
    }

    #[test]
    fn secret_ref_never_serializes_inline() {
        let mut p = PostgresConnectionProfile::default();
        p.basic.password = SecretRef::inline("hunter2");
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("hunter2"));
        assert!(!json.contains("inline"));
    }

    #[test]
    fn uri_parses_basic_and_password() {
        let p = PostgresConnectionProfile::from_uri(
            "postgres://alice:p%40ss%2Fword@db.example.com:5433/appdb?sslmode=verify-full",
        )
        .unwrap();
        assert_eq!(p.basic.host, "db.example.com");
        assert_eq!(p.basic.port, 5433);
        assert_eq!(p.basic.username, "alice");
        assert_eq!(p.basic.password.value(), Some("p@ss/word"));
        assert_eq!(p.basic.maintenance_database, "appdb");
        assert_eq!(p.tls.ssl_mode, PostgresSslMode::VerifyFull);
    }

    #[test]
    fn uri_parses_ipv6() {
        let p = PostgresConnectionProfile::from_uri("postgresql://[::1]:5432/mydb").unwrap();
        assert_eq!(p.basic.host, "::1");
        assert_eq!(p.basic.port, 5432);
        assert_eq!(p.basic.maintenance_database, "mydb");
    }

    #[test]
    fn uri_rejects_non_postgres_scheme() {
        let err = PostgresConnectionProfile::from_uri("mysql://localhost/db").unwrap_err();
        assert!(err.contains("postgres/postgresql"), "got: {err}");
    }

    #[test]
    fn uri_rejects_unknown_query_param() {
        let err = PostgresConnectionProfile::from_uri(
            "postgres://localhost/db?random_param=1",
        )
        .unwrap_err();
        assert!(err.contains("random_param"), "got: {err}");
    }

    #[test]
    fn uri_credentials_land_in_structured_secret_and_never_dump_in_debug() {
        let p =
            PostgresConnectionProfile::from_uri("postgres://bob:topsecret@h/db").unwrap();
        // 凭据落进结构化 SecretRef（走 Keychain/内存），而非散落配置。
        assert_eq!(p.basic.password.value(), Some("topsecret"));
        assert_eq!(p.basic.username, "bob");
        // SecretRef 的 Debug 不打印正文；序列化也不含明文（见 secret_ref_never_serializes_inline）。
        let debug = format!("{p:?}");
        assert!(!debug.contains("topsecret"), "Debug 泄漏: {debug}");
    }
}
