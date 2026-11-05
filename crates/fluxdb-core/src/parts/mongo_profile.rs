// MongoDB 专用连接档案。
//
// 目标：把 MongoDB 连接语义（种子列表 / SRV / 认证库 / TLS / 驱动参数）收敛成
// 结构化、可校验、可回写 URI 的单一模型，镜像 redis_profile.rs / postgres_profile.rs 范式。
//
// 设计约定：
// - 只依赖 `serde` 与 `std`，URI 解析用内联实现，不引第三方 URL 库。
// - 密码走 [`SecretRef`]：`inline` 受控值带 `#[serde(skip)]`，绝不落盘；
//   拨号时由 [`MongoConnectionProfile::to_uri`] 现场拼出连接串，避免把含凭据 URI 存进 `Endpoint`。
// - `mongodb+srv://` 的 SRV 语义原样保留（`srv` 标志 + 单个 DNS 名），不强行拆成单主机普通连接。

/// 一个种子节点。SRV 导入时 `host` 是 SRV 记录名（`xxx.mongodb.net`）。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MongoHost {
    /// 主机名或 IP。
    pub host: String,
    /// 端口，默认 27017。
    pub port: u16,
}

impl MongoHost {
    /// 缺省端口的节点。
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 27017,
        }
    }
}

impl Default for MongoHost {
    fn default() -> Self {
        Self::new(String::new())
    }
}

/// 基础连接参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MongoBasicOptions {
    /// 种子节点列表（非 SRV 可多主机；SRV 固定一个 DNS 名）。
    pub hosts: Vec<MongoHost>,
    /// 用户名；空表示匿名连接。
    pub username: String,
    /// 认证密码。
    pub password: SecretRef,
    /// 默认数据库（URI 路径段）；空表示连接后不预选库。
    pub default_database: String,
    /// 认证库（`authSource`）；空表示沿用默认库或 `admin`。
    pub auth_database: String,
    /// 认证机制；空表示由驱动按服务端协商（SCRAM）。
    pub auth_mechanism: String,
}

/// TLS 参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MongoTlsOptions {
    /// 是否启用 TLS（`mongodb+srv://` 默认启用）。
    pub enabled: bool,
    /// CA 证书文件路径。
    pub ca: SecretRef,
    /// 客户端证书 / 私钥合并文件路径（`tlsCertificateKeyFile`）。
    pub client_cert: SecretRef,
    /// 跳过证书链校验（`tlsAllowInvalidCertificates`）。仅用于自签测试环境。
    pub allow_invalid_certificates: bool,
    /// 跳过主机名校验（`tlsAllowInvalidHostnames`）。
    pub allow_invalid_hostnames: bool,
    /// 独立 SNI / 校验主机名；空则取首个主机名。
    pub server_name: String,
}

/// 高级连接选项。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MongoAdvancedOptions {
    /// 建连超时（秒），默认 10。
    pub connect_timeout_secs: u32,
    /// 选主超时（秒），默认 30。
    pub server_selection_timeout_secs: u32,
    /// 直连模式（`directConnection=true`），仅连单个节点不做拓扑发现。
    pub direct_connection: bool,
    /// 副本集名（`replicaSet`）；空表示自动发现。
    pub replica_set: String,
    /// 应用名（`appName`），用于服务端日志区分来源。
    pub app_name: String,
    /// 驱动参数原样透传（白名单见 [`mongo_passthrough_param`]），不含凭据。
    pub params: std::collections::BTreeMap<String, String>,
}

impl Default for MongoAdvancedOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 10,
            server_selection_timeout_secs: 30,
            direct_connection: false,
            replica_set: String::new(),
            app_name: "FluxDB".into(),
            params: std::collections::BTreeMap::new(),
        }
    }
}

/// MongoDB 完整连接档案。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MongoConnectionProfile {
    /// 是否使用 `mongodb+srv://`（SRV 拓扑发现）。
    pub srv: bool,
    pub basic: MongoBasicOptions,
    pub tls: MongoTlsOptions,
    pub advanced: MongoAdvancedOptions,
    /// SSH 隧道；启用后驱动改为连接本地转发端口。
    #[serde(default)]
    pub ssh: SshTunnelProfile,
}

/// 允许原样透传给驱动的连接串参数（非凭据类）。
///
/// 白名单而非黑名单：未列出的参数在导入时报错，避免用户以为生效却静默丢弃。
fn mongo_passthrough_param(key: &str) -> Option<&'static str> {
    Some(match key {
        "retryWrites" => "retryWrites",
        "retryReads" => "retryReads",
        "w" => "w",
        "journal" => "journal",
        "readConcernLevel" => "readConcernLevel",
        "readPreference" => "readPreference",
        "maxPoolSize" => "maxPoolSize",
        "minPoolSize" => "minPoolSize",
        "compressors" => "compressors",
        "zlibCompressionLevel" => "zlibCompressionLevel",
        "socketTimeoutMS" => "socketTimeoutMS",
        "maxIdleTimeMS" => "maxIdleTimeMS",
        "waitQueueTimeoutMS" => "waitQueueTimeoutMS",
        _ => return None,
    })
}

impl MongoConnectionProfile {
    /// 从历史 `options` 扁平参数迁移出一份档案。
    ///
    /// 只读取已知键，其余不做透传；`hosts` 支持 `host1:port1,host2:port2`。
    pub fn from_options(opts: &std::collections::BTreeMap<String, String>) -> Self {
        let get = |key: &str| opts.get(key).cloned();
        let hosts = get("hosts")
            .or_else(|| get("host"))
            .map(|raw| mongo_parse_host_list(&raw))
            .filter(|hosts| !hosts.is_empty())
            .unwrap_or_else(|| vec![MongoHost::default()]);
        Self {
            srv: connection_option_is_true(get("srv").as_ref()),
            basic: MongoBasicOptions {
                hosts,
                username: get("username").unwrap_or_default(),
                password: {
                    let password = get("password").unwrap_or_default();
                    if password.is_empty() {
                        SecretRef::default()
                    } else {
                        SecretRef::inline(password)
                    }
                },
                default_database: get("database").unwrap_or_default(),
                auth_database: get("auth_db").unwrap_or_default(),
                auth_mechanism: get("auth_mechanism").unwrap_or_default(),
            },
            tls: MongoTlsOptions {
                enabled: connection_option_is_true(get("tls").as_ref()),
                ca: SecretRef::ref_key(get("tls_ca_ref").unwrap_or_default()),
                client_cert: SecretRef::ref_key(get("tls_client_cert_ref").unwrap_or_default()),
                allow_invalid_certificates: connection_option_is_true(
                    get("tls_allow_invalid_certificates").as_ref(),
                ),
                allow_invalid_hostnames: connection_option_is_true(
                    get("tls_allow_invalid_hostnames").as_ref(),
                ),
                server_name: get("tls_server_name").unwrap_or_default(),
            },
            advanced: MongoAdvancedOptions {
                connect_timeout_secs: get("connect_timeout_secs")
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(10),
                server_selection_timeout_secs: get("server_selection_timeout_secs")
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(30),
                direct_connection: connection_option_is_true(get("direct_connection").as_ref()),
                replica_set: get("replica_set").unwrap_or_default(),
                app_name: get("app_name").unwrap_or_else(|| "FluxDB".into()),
                params: mongo_passthrough_params(opts),
            },
            ssh: SshTunnelProfile::from_options(opts),
        }
    }

    /// 回写成扁平 `options`，供既有消费方与存储兼容路径使用。
    ///
    /// 不写密码正文之外的密钥字段；密码键交给 storage 的凭据槽处理。
    pub fn into_options(&self) -> std::collections::BTreeMap<String, String> {
        let mut ops = std::collections::BTreeMap::new();
        let hosts = self
            .basic
            .hosts
            .iter()
            .filter(|h| !h.host.trim().is_empty())
            .map(|h| format!("{}:{}", h.host, h.port))
            .collect::<Vec<_>>()
            .join(",");
        if !hosts.is_empty() {
            ops.insert("hosts".into(), hosts.clone());
            // 单主机时同时写 `host`，兼容只认单主机的历史读取路径。
            if self.basic.hosts.iter().filter(|h| !h.host.trim().is_empty()).count() == 1 {
                ops.insert("host".into(), self.basic.hosts[0].host.clone());
            }
        }
        if self.srv {
            ops.insert("srv".into(), "true".into());
        }
        if !self.basic.username.is_empty() {
            ops.insert("username".into(), self.basic.username.clone());
        }
        if let Some(password) = self.basic.password.value() {
            ops.insert("password".into(), password.to_string());
        }
        if !self.basic.default_database.is_empty() {
            ops.insert("database".into(), self.basic.default_database.clone());
        }
        if !self.basic.auth_database.is_empty() {
            ops.insert("auth_db".into(), self.basic.auth_database.clone());
        }
        if !self.basic.auth_mechanism.is_empty() {
            ops.insert("auth_mechanism".into(), self.basic.auth_mechanism.clone());
        }
        if self.tls.enabled {
            ops.insert("tls".into(), "true".into());
        }
        if !self.tls.ca.key.is_empty() {
            ops.insert("tls_ca_ref".into(), self.tls.ca.key.clone());
        }
        if !self.tls.client_cert.key.is_empty() {
            ops.insert("tls_client_cert_ref".into(), self.tls.client_cert.key.clone());
        }
        if self.tls.allow_invalid_certificates {
            ops.insert("tls_allow_invalid_certificates".into(), "true".into());
        }
        if self.tls.allow_invalid_hostnames {
            ops.insert("tls_allow_invalid_hostnames".into(), "true".into());
        }
        if !self.tls.server_name.is_empty() {
            ops.insert("tls_server_name".into(), self.tls.server_name.clone());
        }
        let a = &self.advanced;
        ops.insert("connect_timeout_secs".into(), a.connect_timeout_secs.to_string());
        ops.insert(
            "server_selection_timeout_secs".into(),
            a.server_selection_timeout_secs.to_string(),
        );
        if a.direct_connection {
            ops.insert("direct_connection".into(), "true".into());
        }
        if !a.replica_set.is_empty() {
            ops.insert("replica_set".into(), a.replica_set.clone());
        }
        if a.app_name != "FluxDB" && !a.app_name.is_empty() {
            ops.insert("app_name".into(), a.app_name.clone());
        }
        for (key, value) in &a.params {
            ops.insert(key.clone(), value.clone());
        }
        ops.extend(self.ssh.into_options());
        ops
    }

    /// 隧道内的连接串：目标改写为本地转发端口，并强制直连。
    ///
    /// 经隧道只能到达跳板机可达的那一个节点，拓扑发现拿到的主机名在本地不可达，
    /// 因此 SSH 模式下不使用 SRV/副本集自动发现。
    pub fn to_tunnel_uri(&self, local_host: &str, local_port: u16) -> String {
        let mut profile = self.clone();
        profile.srv = false;
        profile.basic.hosts = vec![MongoHost {
            host: local_host.to_string(),
            port: local_port,
        }];
        profile.advanced.direct_connection = true;
        profile.to_uri()
    }

    /// 拨号端点：首个种子节点的 `(host, port)`，供 UI 与连接信息展示。
    pub fn dial_endpoint(&self) -> (String, u16) {
        self.basic
            .hosts
            .first()
            .map(|h| (h.host.clone(), h.port))
            .unwrap_or_else(|| (String::new(), 27017))
    }

    /// 校验：返回第一条不合法的中文错误；`None` 表示通过。
    pub fn validate(&self) -> Option<String> {
        let hosts: Vec<&MongoHost> = self
            .basic
            .hosts
            .iter()
            .filter(|h| !h.host.trim().is_empty())
            .collect();
        if hosts.is_empty() {
            return Some("请填写主机地址".to_string());
        }
        if self.srv && hosts.len() > 1 {
            // SRV 只能有一个记录名：多主机时语义不成立，明确拒绝而不是丢弃。
            return Some("SRV 连接只能填写一个主机名".to_string());
        }
        if self.advanced.direct_connection && hosts.len() > 1 {
            return Some("直连模式只能填写一个主机".to_string());
        }
        if let Some(message) = self.ssh.validate() {
            return Some(message);
        }
        if self.ssh.enabled && (self.srv || hosts.len() > 1) {
            return Some("SSH 隧道仅支持单个 MongoDB 主机（不支持 SRV/多主机）".to_string());
        }
        if !self.basic.username.is_empty() && self.basic.password.value().is_none() {
            return Some("请填写密码".to_string());
        }
        None
    }

    /// 现场拼出驱动可用的连接串；密码在此处百分号编码，不进入持久化结构。
    pub fn to_uri(&self) -> String {
        let scheme = if self.srv { "mongodb+srv" } else { "mongodb" };
        let mut out = format!("{scheme}://");

        if !self.basic.username.is_empty() {
            out.push_str(&mongo_percent_encode(&self.basic.username));
            if let Some(password) = self.basic.password.value() {
                out.push(':');
                out.push_str(&mongo_percent_encode(password));
            }
            out.push('@');
        }

        let hosts = self
            .basic
            .hosts
            .iter()
            .filter(|h| !h.host.trim().is_empty())
            .map(|h| {
                // SRV 记录不带端口（端口由 SRV 记录给出），显式带端口会被驱动拒绝。
                if self.srv || h.port == 27017 {
                    h.host.clone()
                } else {
                    format!("{}:{}", h.host, h.port)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&hosts);

        if !self.basic.default_database.is_empty() {
            out.push('/');
            out.push_str(&mongo_percent_encode(&self.basic.default_database));
        }

        let mut query: Vec<String> = Vec::new();
        if !self.basic.auth_database.is_empty() {
            query.push(format!(
                "authSource={}",
                mongo_percent_encode(&self.basic.auth_database)
            ));
        }
        if !self.basic.auth_mechanism.is_empty() {
            query.push(format!(
                "authMechanism={}",
                mongo_percent_encode(&self.basic.auth_mechanism)
            ));
        }
        if self.tls.enabled || self.srv {
            query.push("tls=true".into());
        }
        if !self.tls.ca.key.is_empty() {
            query.push(format!("tlsCAFile={}", mongo_percent_encode(&self.tls.ca.key)));
        }
        if !self.tls.client_cert.key.is_empty() {
            query.push(format!(
                "tlsCertificateKeyFile={}",
                mongo_percent_encode(&self.tls.client_cert.key)
            ));
        }
        if self.tls.allow_invalid_certificates {
            query.push("tlsAllowInvalidCertificates=true".into());
        }
        if self.tls.allow_invalid_hostnames {
            query.push("tlsAllowInvalidHostnames=true".into());
        }
        if self.advanced.direct_connection {
            query.push("directConnection=true".into());
        }
        if !self.advanced.replica_set.is_empty() {
            query.push(format!(
                "replicaSet={}",
                mongo_percent_encode(&self.advanced.replica_set)
            ));
        }
        if !self.advanced.app_name.is_empty() {
            query.push(format!("appName={}", mongo_percent_encode(&self.advanced.app_name)));
        }
        query.push(format!(
            "connectTimeoutMS={}",
            self.advanced.connect_timeout_secs.saturating_mul(1000)
        ));
        query.push(format!(
            "serverSelectionTimeoutMS={}",
            self.advanced.server_selection_timeout_secs.saturating_mul(1000)
        ));
        for (key, value) in &self.advanced.params {
            query.push(format!("{key}={}", mongo_percent_encode(value)));
        }

        if !query.is_empty() {
            out.push('?');
            out.push_str(&query.join("&"));
        }
        out
    }

    /// 从 `mongodb://` / `mongodb+srv://` URI 解析出一份档案。
    ///
    /// - 非 mongo scheme 返回 `Err`（不静默忽略）。
    /// - 支持多主机种子列表、IPv6（`[::1]`）、百分号编码（先解码再入库）。
    /// - 末尾不带 `/`、带 `/?`、带 `/:db` 三种写法都接受。
    /// - `mongodb+srv://` 保留 SRV 语义：单个 DNS 名 + `srv=true`，不拆成单主机。
    /// - 查询参数按白名单消费；未知参数报错而不是丢弃。
    pub fn from_uri(uri: &str) -> std::result::Result<Self, String> {
        let uri = uri.trim();
        let (scheme, rest) = uri
            .split_once("://")
            .ok_or_else(|| "连接 URI 缺少 scheme（应为 mongodb:// 或 mongodb+srv://）".to_string())?;
        let scheme = scheme.to_ascii_lowercase();
        let srv = match scheme.as_str() {
            "mongodb" => false,
            "mongodb+srv" => true,
            other => {
                return Err(format!(
                    "仅支持 mongodb:// 或 mongodb+srv:// scheme，收到 `{other}`"
                ));
            }
        };

        // 先切 query，再切 path，最后剩 authority。
        let (rest, query) = match rest.split_once('?') {
            Some((head, query)) => (head, query),
            None => (rest, ""),
        };
        let (authority, path_db) = match rest.split_once('/') {
            Some((authority, path)) => (authority, path.trim_end_matches('/').to_string()),
            None => (rest, String::new()),
        };

        let (userinfo, hostlist) = match authority.rsplit_once('@') {
            Some((userinfo, hostlist)) => (userinfo, hostlist),
            None => ("", authority),
        };

        // 用户名/密码：首个 `:` 分隔；用户名字面量里的 `:` 需百分号编码，符合 URI 语义。
        let (username, password) = match userinfo.split_once(':') {
            Some((user, pass)) => (mongo_percent_decode(user), Some(mongo_percent_decode(pass))),
            None => (mongo_percent_decode(userinfo), None),
        };

        if hostlist.trim().is_empty() {
            return Err("连接 URI 缺少主机地址".to_string());
        }
        let hosts = mongo_parse_host_list(hostlist);
        if hosts.is_empty() {
            return Err("连接 URI 缺少主机地址".to_string());
        }
        if srv {
            if hosts.len() > 1 {
                return Err("mongodb+srv:// 只能包含一个主机名".to_string());
            }
            // SRV 连接串不允许出现端口（含显式的 27017）：端口由 SRV 记录给出。
            // 只看解析后的 port 值不够——显式 `:27017` 会与默认值撞车而漏判。
            if hostlist.contains(':') || hostlist.contains('[') {
                return Err("mongodb+srv:// 不允许在主机名后指定端口".to_string());
            }
        }

        // 认证：只有 URI 真的带了口令才写入受控值，避免 `Some("")` 被当成"已填密码"。
        let password = password.unwrap_or_default();
        let mut profile = Self {
            srv,
            basic: MongoBasicOptions {
                hosts,
                username,
                password: if password.is_empty() {
                    SecretRef::default()
                } else {
                    SecretRef::inline(password)
                },
                default_database: mongo_percent_decode(&path_db),
                auth_database: String::new(),
                auth_mechanism: String::new(),
            },
            // SRV 语义默认走 TLS（Atlas 等托管服务都要求）；普通 URI 由参数决定。
            tls: MongoTlsOptions {
                enabled: srv,
                ..Default::default()
            },
            advanced: MongoAdvancedOptions::default(),
            ssh: SshTunnelProfile::default(),
        };

        if query.is_empty() {
            return Ok(profile);
        }
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = match pair.split_once('=') {
                Some((key, value)) => (mongo_percent_decode(key), mongo_percent_decode(value)),
                None => (mongo_percent_decode(pair), String::new()),
            };
            match key.as_str() {
                // 认证
                "authSource" => profile.basic.auth_database = value,
                "authMechanism" => profile.basic.auth_mechanism = value,
                // 认证机制属性（SCRAM / X509 之外暂不支持，明确拒绝而非静默丢弃）
                "authMechanismProperties" => {
                    return Err(
                        "暂不支持 authMechanismProperties（如 AWS/GSSAPI 认证参数）".to_string()
                    );
                }
                // TLS：`ssl` 是 `tls` 的历史别名
                "tls" | "ssl" => profile.tls.enabled = mongo_uri_flag(&value)?,
                "tlsCAFile" | "sslCAFile" => profile.tls.ca = SecretRef::ref_key(value),
                "tlsCertificateKeyFile" | "sslCertificateKeyFile" => {
                    profile.tls.client_cert = SecretRef::ref_key(value)
                }
                "tlsAllowInvalidCertificates" | "sslAllowInvalidCertificates" => {
                    profile.tls.allow_invalid_certificates = mongo_uri_flag(&value)?
                }
                "tlsAllowInvalidHostnames" | "sslAllowInvalidHostnames" => {
                    profile.tls.allow_invalid_hostnames = mongo_uri_flag(&value)?
                }
                "tlsInsecure" | "sslInsecure" => {
                    // 一个开关同时放宽链与主机名校验，与驱动语义一致。
                    let insecure = mongo_uri_flag(&value)?;
                    profile.tls.allow_invalid_certificates = insecure;
                    profile.tls.allow_invalid_hostnames = insecure;
                }
                // 拓扑
                "replicaSet" => profile.advanced.replica_set = value,
                "directConnection" => profile.advanced.direct_connection = mongo_uri_flag(&value)?,
                "appName" => profile.advanced.app_name = value,
                "connectTimeoutMS" => {
                    profile.advanced.connect_timeout_secs = mongo_uri_millis_secs(&key, &value)?
                }
                "serverSelectionTimeoutMS" => {
                    profile.advanced.server_selection_timeout_secs =
                        mongo_uri_millis_secs(&key, &value)?
                }
                // 白名单透传参数
                other => match mongo_passthrough_param(other) {
                    Some(canonical) => {
                        profile
                            .advanced
                            .params
                            .insert(canonical.to_string(), value);
                    }
                    None => {
                        return Err(format!(
                            "不支持的连接参数 `{other}`（未知或明确不支持，不静默丢弃）"
                        ));
                    }
                },
            }
        }

        // SRV 主机名不带端口；非 SRV 未写端口时保持默认 27017。
        Ok(profile)
    }
}

/// 解析 `host[:port][,host[:port]]...`，支持 IPv6 `[::1]:27017`。空项被忽略。
fn mongo_parse_host_list(input: &str) -> Vec<MongoHost> {
    input
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| match parse_host_port(item) {
            Some((host, port)) => MongoHost { host, port },
            // 无合法端口段时保留主机字面量，交给校验/驱动报错，不静默丢弃用户输入。
            None => MongoHost::new(item.trim_matches(['[', ']'])),
        })
        .collect()
}

/// 提取 `options` 里白名单驱动参数（`from_options` 用）。
fn mongo_passthrough_params(
    opts: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    opts.iter()
        .filter_map(|(key, value)| {
            mongo_passthrough_param(key).map(|canonical| (canonical.to_string(), value.clone()))
        })
        .collect()
}

/// 解析连接串里的布尔开关；未识别的文本报错，不静默当 false。
fn mongo_uri_flag(value: &str) -> std::result::Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" | "" => Ok(false),
        other => Err(format!("连接参数取值应为 true/false，收到 `{other}`")),
    }
}

/// 毫秒参数转秒，向上取整（0 或非法值按 1 秒兜底，避免 0 被解释成"无限等待"）。
fn mongo_uri_millis_secs(key: &str, value: &str) -> std::result::Result<u32, String> {
    let millis: u64 = value
        .trim()
        .parse()
        .map_err(|_| format!("连接参数 `{key}` 应为毫秒整数，收到 `{value}`"))?;
    Ok(((millis + 999) / 1000).max(1) as u32)
}

/// 百分号编码一个 URI 组件（RFC 3986 unreserved 之外全部转义）。
fn mongo_percent_encode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            other => {
                out.push('%');
                out.push(HEX[(other >> 4) as usize] as char);
                out.push(HEX[(other & 0x0f) as usize] as char);
            }
        }
    }
    out
}

/// 宽松百分号解码：非法转义按原样保留（与 Redis/PG 档案的解析行为一致）。
fn mongo_percent_decode(input: &str) -> String {
    percent_decode(input)
}

#[cfg(test)]
mod mongo_profile_tests {
    use super::*;

    fn profile(uri: &str) -> MongoConnectionProfile {
        MongoConnectionProfile::from_uri(uri).expect("URI 应能解析")
    }

    #[test]
    fn parses_srv_uri_without_trailing_slash() {
        // 占位凭据：真实 Atlas 连接串只从环境变量读取，不入仓库。
        let p = profile("mongodb+srv://demo_user:demo_pass@cluster0.h0l2m0b.mongodb.net");
        assert!(p.srv);
        assert_eq!(p.basic.hosts.len(), 1);
        assert_eq!(p.basic.hosts[0].host, "cluster0.h0l2m0b.mongodb.net");
        assert_eq!(p.basic.hosts[0].port, 27017);
        assert_eq!(p.basic.username, "demo_user");
        assert_eq!(p.basic.password.value(), Some("demo_pass"));
        assert!(p.basic.default_database.is_empty());
        // SRV 默认启 TLS
        assert!(p.tls.enabled);
        assert_eq!(p.validate(), None);
    }

    #[test]
    fn parses_standard_uri_with_seed_list_and_auth_source() {
        let p = profile(
            "mongodb://user:p%40ss@h1:27018,h2:27019/fluxdb_demo?authSource=admin&replicaSet=rs0&tls=true",
        );
        assert!(!p.srv);
        assert_eq!(
            p.basic.hosts,
            vec![
                MongoHost { host: "h1".into(), port: 27018 },
                MongoHost { host: "h2".into(), port: 27019 },
            ]
        );
        assert_eq!(p.basic.password.value(), Some("p@ss"), "密码需百分号解码");
        assert_eq!(p.basic.default_database, "fluxdb_demo");
        assert_eq!(p.basic.auth_database, "admin");
        assert_eq!(p.advanced.replica_set, "rs0");
        assert!(p.tls.enabled);
        assert_eq!(p.validate(), None);
    }

    #[test]
    fn keeps_srv_semantics_and_round_trips_uri() {
        let p = profile(
            "mongodb+srv://u:p@cluster0.h0l2m0b.mongodb.net/fluxdb_demo?authSource=admin",
        );
        let uri = p.to_uri();
        assert!(uri.starts_with("mongodb+srv://"), "SRV 不能被降级成普通连接: {uri}");
        assert!(uri.contains("cluster0.h0l2m0b.mongodb.net"), "{uri}");
        assert!(!uri.contains(":27017"), "SRV 连接串不应带端口: {uri}");
        assert!(uri.contains("authSource=admin"), "{uri}");
        let back = MongoConnectionProfile::from_uri(&uri).expect("回写 URI 应可再解析");
        assert_eq!(back.basic.hosts, p.basic.hosts);
        assert_eq!(back.basic.default_database, p.basic.default_database);
        assert_eq!(back.basic.auth_database, p.basic.auth_database);
    }

    #[test]
    fn rejects_unknown_param_instead_of_dropping() {
        let err = MongoConnectionProfile::from_uri("mongodb://h/db?tlsFoo=1").unwrap_err();
        assert!(err.contains("tlsFoo"), "{err}");
    }

    #[test]
    fn rejects_srv_with_port_or_multiple_hosts() {
        assert!(MongoConnectionProfile::from_uri("mongodb+srv://h:27017/db").is_err());
        assert!(MongoConnectionProfile::from_uri("mongodb+srv://a,b/db").is_err());
    }

    #[test]
    fn rejects_non_mongo_scheme() {
        assert!(MongoConnectionProfile::from_uri("postgres://h/db").is_err());
        assert!(MongoConnectionProfile::from_uri("cluster0.mongodb.net").is_err());
    }

    #[test]
    fn accepts_trailing_slash_and_empty_query() {
        let p = profile("mongodb://h/");
        assert!(p.basic.default_database.is_empty());
        let p = profile("mongodb://h?");
        assert!(p.basic.default_database.is_empty());
        let p = profile("mongodb://h/db/?authSource=admin");
        assert_eq!(p.basic.default_database, "db");
        assert_eq!(p.basic.auth_database, "admin");
    }

    #[test]
    fn options_round_trip_keeps_seed_list() {
        let p = profile("mongodb://u:p@h1:27018,h2/fluxdb_demo?authSource=admin");
        let opts = p.into_options();
        assert_eq!(opts.get("hosts").map(String::as_str), Some("h1:27018,h2:27017"));
        assert_eq!(opts.get("auth_db").map(String::as_str), Some("admin"));
        let back = MongoConnectionProfile::from_options(&opts);
        assert_eq!(back.basic.hosts, p.basic.hosts);
        assert_eq!(back.basic.password.value(), Some("p"));
    }

    #[test]
    fn validate_requires_password_when_username_set() {
        let p = profile("mongodb://user@h/db");
        assert!(p.validate().is_some());
    }
}
