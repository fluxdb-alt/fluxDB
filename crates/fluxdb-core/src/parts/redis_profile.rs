// Redis 专用连接配置模型。
//
// 目标：把原本分散在 `ConnectionConfig.options` 这组扁平字符串里的 TLS / SSH /
// Sentinel / Cluster / Cloud-Azure / URI 导入等连接语义，收敛成结构化、可迁移、
// 可校验的单一模型，作为 `ConnectionConfig` 的补充（而不是 UI 临时状态）。
//
// 设计约定：
// - 本模型只依赖 `serde` 与 `std`，不引入第三方 URL 解析库，URI 解析用内联实现。
// - 所有密钥类内容（密码、私钥、证书正文）都走 [`SecretRef`]：引用（`key`）可落盘，
//   `inline` 受控值标记 `#[serde(skip)]`，绝不序列化到磁盘。
// - 通过 [`RedisConnectionProfile::from_options`] 承接历史 `options` 扁平参数，
//   通过 [`RedisConnectionProfile::into_options`] 回写兼容参数供现有消费方复用。

/// Redis 连接拓扑类型。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum RedisTopologyKind {
    /// 单点连接（默认，含通过 SSH 隧道 / TLS 的直连）。
    #[default]
    Standalone,
    /// Redis Sentinel 高可用：先问哨兵取当前主库地址再连。
    Sentinel,
    /// Redis Cluster：连任一节点后做槽位 / 节点发现与重定向。
    Cluster,
}

/// 一条可拨号的不安全密钥引用。
///
/// - `key`：受控存储里的引用名（如 macOS Keychain 的 account），可安全落盘。
/// - `inline`：仅在内存中出现的一次性值（本次拨号使用），`#[serde(skip)]` 保证绝不写盘。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SecretRef {
    /// 受控存储中的引用名；空表示没有引用。
    pub key: String,
    /// 内存中的受控值；落盘前由 storage 剥离，本字段不参与序列化。
    #[serde(skip)]
    pub inline: Option<String>,
}

impl SecretRef {
    /// 构造一个持有内联受控值的引用（仅内存）。
    pub fn inline(value: impl Into<String>) -> Self {
        Self {
            key: String::new(),
            inline: Some(value.into()),
        }
    }

    /// 构造一个指向受控存储槽位的引用。
    pub fn ref_key(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            inline: None,
        }
    }

    /// 取用于拨号的值：优先内联受控值，其次引用名（引用通常在拨号前被 storage 解析回填）。
    pub fn value(&self) -> Option<&str> {
        self.inline.as_deref().or(if self.key.is_empty() { None } else { Some(self.key.as_str()) })
    }
}

/// 基础连接参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedisBasicOptions {
    /// 目标主机（或哨兵 / 集群起始节点主机）。
    pub host: String,
    /// 目标端口。
    pub port: u16,
    /// 逻辑库序号（`None` 视为 0）。
    pub database: Option<String>,
    /// ACL 用户名（AUTH user pass）。
    pub username: Option<String>,
    /// 认证密码（不安全）。
    pub password: SecretRef,
}

/// TLS 相关参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedisTlsOptions {
    /// 是否启用 TLS。
    pub enabled: bool,
    /// CA 证书引用。
    pub ca: SecretRef,
    /// 客户端证书引用。
    pub client_cert: SecretRef,
    /// 客户端私钥引用。
    pub client_key: SecretRef,
    /// SNI / 校验主机名（默认取连接主机名）。
    pub sni: String,
    /// 是否校验证书。关闭仅用于自签环境。
    pub verify: bool,
}

/// SSH 隧道参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedisSshOptions {
    /// 是否启用 SSH 隧道。
    pub enabled: bool,
    /// 跳板主机。
    pub host: String,
    /// 跳板端口（默认 22）。
    pub port: u16,
    /// 跳板用户名。
    pub username: String,
    /// SSH 认证类型。
    pub auth: RedisSshAuth,
    /// 密码（password 认证）。
    pub password: SecretRef,
    /// 私钥引用（key/private-key 认证）。
    pub private_key: SecretRef,
    /// 私钥口令（可选）。
    pub passphrase: SecretRef,
}

/// SSH 认证方式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum RedisSshAuth {
    #[default]
    Password,
    PrivateKey,
}

/// Sentinel 拓扑参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedisSentinelOptions {
    /// 哨兵主库名（master name）。
    pub master_name: String,
    /// 哨兵节点列表（host:port）。为空时回退到基础连接的 host/port。
    pub endpoints: Vec<String>,
    /// 哨兵侧认证（通常与 Redis 相同，`password` 复用基础连接）。
    pub username: Option<String>,
    /// 哨兵侧密码（为空时复用基础连接的密码）。
    pub password: SecretRef,
    pub tls: RedisTlsOptions,
}

/// Cluster 拓扑参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedisClusterOptions {
    /// 起始节点列表（host:port）。为空时回退到基础连接的 host/port。
    pub start_nodes: Vec<String>,
    /// 是否允许把命令重定向到从节点（readonly 策略）。
    pub allow_readonly: bool,
}

/// 拓扑统一承载。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedisTopology {
    pub kind: RedisTopologyKind,
    pub sentinel: RedisSentinelOptions,
    pub cluster: RedisClusterOptions,
}

/// 云 / 托管发现来源信息（Azure、Redis Cloud 等）。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedisCloudOptions {
    /// 云提供商标识（`azure` / `redis-cloud` / 空表示非云导入）。
    pub provider: String,
    /// 订阅 / 账号标识。
    pub subscription: String,
    /// 资源 / 数据库标识。
    pub resource: String,
    /// 导入后的显示名覆盖。
    pub imported_name: String,
}

/// Redis 完整连接档案：承接 TLS / SSH / Sentinel / Cluster / Cloud / URI 导入。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RedisConnectionProfile {
    pub basic: RedisBasicOptions,
    pub tls: RedisTlsOptions,
    pub ssh: RedisSshOptions,
    pub topology: RedisTopology,
    pub cloud: RedisCloudOptions,
}

/// 从「云 provider + 目标主机」推断云端 resource / subscription 元数据。
///
/// 用于「连接串导入 + 云 provider 选择」这一真实可用云发现主路径：让 `RedisCloudOptions`
/// 不只存 provider，还能从托管主机名里解析出资源标识，供弹框与后续日志/展示使用。
/// 解析不出具体账号/订阅时保持为空（诚实地不臆造，仅填能确定的部分）。
pub fn infer_cloud_from_host(provider: &str, host: &str) -> (String, String) {
    let host = host.trim();
    let first_label = host.split('.').next().unwrap_or("").to_string();
    match provider {
        // Azure：`mycache.redis.cache.windows.net` → resource = 缓存名 mycache。
        "azure" => (first_label, String::new()),
        // Redis Cloud：`redis-12345.c263...redns.redis-cloud.com` → resource = 库标识 redis-12345。
        "redis-cloud" => (first_label, String::new()),
        // 其余/未知：resource 用整段主机，subscription 留空待用户补充。
        _ => (host.to_string(), String::new()),
    }
}

/// 判断一个字符串键是否是密钥/证书类键（落盘前应剔除）。
/// fluxdb-storage 在执行 `strip_plaintext_secrets` 时复用本判定，保证剥离口径一致。
pub fn is_secret_option_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("password")
        || key.contains("secret")
        || key.contains("private_key")
        || key.contains("passphrase")
        || key.contains("ca_cert")
        || key.contains("client_cert")
        || key.contains("client_key")
}

/// 历史 `options` 扁平参数里 Redis 认识的全部键（白名单）。
/// 用于把旧连接迁移进结构化模型，也用于 UI 回填。
pub const REDIS_LEGACY_OPTION_KEYS: &[&str] = &[
    "username",
    "password",
    "database",
    "tls",
    "tls_insecure",
    "tls_server_name",
    "tls_ca_ref",
    "tls_client_cert_ref",
    "tls_client_key_ref",
    "sentinel_master",
    "sentinel_endpoints",
    "cluster_start_nodes",
    "cluster_readonly",
    "ssh_enabled",
    "ssh_host",
    "ssh_port",
    "ssh_username",
    "ssh_auth",
    "ssh_password",
    "ssh_private_key_ref",
    "ssh_passphrase",
    "cloud_provider",
    "cloud_subscription",
    "cloud_resource",
];

fn option_is_true(value: Option<&String>) -> bool {
    value
        .map(|v| v.trim().to_ascii_lowercase())
        .is_some_and(|v| matches!(v.as_str(), "true" | "y" | "yes" | "1" | "on"))
}

impl RedisConnectionProfile {
    /// —— 解析职责 ——

    /// 从历史 `options` 扁平参数迁移出一份档案。
    ///
    /// 仅迁移白名单键，其余任意参数不做透传，避免把垃圾灌进结构化模型。
    /// 迁移保留原始 `options` 亦无碍：模型是 `ConnectionConfig` 的可选补充。
    pub fn from_options(opts: &std::collections::BTreeMap<String, String>) -> Self {
        let get = |key: &str| opts.get(key).cloned();
        let tls = RedisTlsOptions {
            enabled: option_is_true(get("tls").as_ref()),
            ca: SecretRef::ref_key(get("tls_ca_ref").unwrap_or_default()),
            client_cert: SecretRef::ref_key(get("tls_client_cert_ref").unwrap_or_default()),
            client_key: SecretRef::ref_key(get("tls_client_key_ref").unwrap_or_default()),
            sni: get("tls_server_name").unwrap_or_default(),
            verify: !option_is_true(get("tls_insecure").as_ref()),
        };
        let ssh = RedisSshOptions {
            enabled: option_is_true(get("ssh_enabled").as_ref()),
            host: get("ssh_host").unwrap_or_default(),
            port: get("ssh_port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(22),
            username: get("ssh_username").unwrap_or_default(),
            auth: match get("ssh_auth").as_deref() {
                Some("key") | Some("private_key") => RedisSshAuth::PrivateKey,
                _ => RedisSshAuth::Password,
            },
            password: SecretRef::inline(get("ssh_password").unwrap_or_default()),
            private_key: SecretRef::ref_key(get("ssh_private_key_ref").unwrap_or_default()),
            passphrase: SecretRef::inline(get("ssh_passphrase").unwrap_or_default()),
        };
        let topology = RedisTopology {
            kind: if get("sentinel_master").is_some() {
                RedisTopologyKind::Sentinel
            } else if option_is_true(get("cluster_enabled").as_ref())
                || get("cluster_start_nodes").is_some()
            {
                RedisTopologyKind::Cluster
            } else {
                RedisTopologyKind::Standalone
            },
            sentinel: RedisSentinelOptions {
                master_name: get("sentinel_master").unwrap_or_default(),
                endpoints: split_host_port_list(get("sentinel_endpoints").as_deref()),
                username: get("username").clone(),
                password: SecretRef::inline(get("password").unwrap_or_default()),
                tls: tls.clone(),
            },
            cluster: RedisClusterOptions {
                start_nodes: split_host_port_list(get("cluster_start_nodes").as_deref()),
                allow_readonly: option_is_true(get("cluster_readonly").as_ref()),
            },
        };
        Self {
            basic: RedisBasicOptions {
                host: opts
                    .get("host")
                    .cloned()
                    .unwrap_or_default(),
                port: opts
                    .get("port")
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(6379),
                database: opts.get("database").cloned().filter(|d| !d.is_empty()),
                username: opts.get("username").cloned().filter(|u| !u.is_empty()),
                password: SecretRef::inline(get("password").unwrap_or_default()),
            },
            tls,
            ssh,
            topology,
            cloud: RedisCloudOptions {
                provider: get("cloud_provider").unwrap_or_default(),
                subscription: get("cloud_subscription").unwrap_or_default(),
                resource: get("cloud_resource").unwrap_or_default(),
                imported_name: get("cloud_imported_name").unwrap_or_default(),
            },
        }
    }

    /// 把档案回写成一组合法的 `options` 兼容参数。
    ///
    /// 仅供「需要扁平参数的既有消费方」使用（如 redis-cli terminal adapter），
    /// 不写密钥正文：密码走引用/占用 `password` 键交给 storage 决定是否落盘。
    pub fn into_options(&self) -> std::collections::BTreeMap<String, String> {
        let mut ops = std::collections::BTreeMap::new();
        if !self.basic.host.is_empty() {
            ops.insert("host".into(), self.basic.host.clone());
        }
        ops.insert("port".into(), self.basic.port.to_string());
        if let Some(db) = &self.basic.database {
            ops.insert("database".into(), db.clone());
        }
        if let Some(user) = &self.basic.username {
            ops.insert("username".into(), user.clone());
        }
        // 用「生效密码」而不是只看基础密码：Sentinel 场景哨兵/主库通常共用同一密码，
        // 仅配置哨兵密码而基础密码为空时也能正确鉴权（哨兵探测与主库 AUTH 都走该键）。
        if let Some(password) = self.password() {
            ops.insert("password".into(), password.to_string());
        }

        let t = &self.tls;
        ops.insert("tls".into(), t.enabled.to_string());
        if !t.sni.is_empty() {
            ops.insert("tls_server_name".into(), t.sni.clone());
        }
        ops.insert("tls_insecure".into(), (!t.verify).to_string());
        if !t.ca.key.is_empty() || !t.client_cert.key.is_empty() || !t.client_key.key.is_empty() {
            ops.insert("tls_ca_ref".into(), t.ca.key.clone());
            ops.insert("tls_client_cert_ref".into(), t.client_cert.key.clone());
            ops.insert("tls_client_key_ref".into(), t.client_key.key.clone());
        }

        if self.ssh.enabled {
            ops.insert("ssh_enabled".into(), "true".into());
            ops.insert("ssh_host".into(), self.ssh.host.clone());
            ops.insert("ssh_port".into(), self.ssh.port.to_string());
            ops.insert("ssh_username".into(), self.ssh.username.clone());
            ops.insert(
                "ssh_auth".into(),
                match self.ssh.auth {
                    RedisSshAuth::Password => "password".into(),
                    RedisSshAuth::PrivateKey => "private_key".into(),
                },
            );
            if let Some(p) = self.ssh.password.value() {
                ops.insert("ssh_password".into(), p.to_string());
            }
            if !self.ssh.private_key.key.is_empty() {
                ops.insert("ssh_private_key_ref".into(), self.ssh.private_key.key.clone());
            }
        }

        match self.topology.kind {
            RedisTopologyKind::Standalone => {}
            RedisTopologyKind::Sentinel => {
                ops.insert("sentinel_master".into(), self.topology.sentinel.master_name.clone());
                if !self.topology.sentinel.endpoints.is_empty() {
                    ops.insert(
                        "sentinel_endpoints".into(),
                        self.topology.sentinel.endpoints.join(","),
                    );
                }
            }
            RedisTopologyKind::Cluster => {
                ops.insert("cluster_enabled".into(), "true".into());
                if !self.topology.cluster.start_nodes.is_empty() {
                    ops.insert(
                        "cluster_start_nodes".into(),
                        self.topology.cluster.start_nodes.join(","),
                    );
                }
                ops.insert("cluster_readonly".into(), self.topology.cluster.allow_readonly.to_string());
            }
        }

        if !self.cloud.provider.is_empty() {
            ops.insert("cloud_provider".into(), self.cloud.provider.clone());
            if !self.cloud.subscription.is_empty() {
                ops.insert("cloud_subscription".into(), self.cloud.subscription.clone());
            }
            if !self.cloud.resource.is_empty() {
                ops.insert("cloud_resource".into(), self.cloud.resource.clone());
            }
        }

        ops
    }

    /// —— 拨号辅助 ——

    /// 实际用于拨号的目标端点（未被 SSH 隧道改写的前提下）。
    /// 对于 Sentinel/Cluster，返回对应拓扑里配置的主地址；否则返回基础地址。
    pub fn dial_endpoint(&self) -> (String, u16) {
        match self.topology.kind {
            RedisTopologyKind::Sentinel => {
                if let Some((h, p)) = self
                    .topology
                    .sentinel
                    .endpoints
                    .first()
                    .and_then(|e| parse_host_port(e))
                {
                    (h, p)
                } else {
                    (self.basic.host.clone(), self.basic.port)
                }
            }
            RedisTopologyKind::Cluster => {
                if let Some((h, p)) = self
                    .topology
                    .cluster
                    .start_nodes
                    .first()
                    .and_then(|e| parse_host_port(e))
                {
                    (h, p)
                } else {
                    (self.basic.host.clone(), self.basic.port)
                }
            }
            RedisTopologyKind::Standalone => (self.basic.host.clone(), self.basic.port),
        }
    }

    /// 认证密码：优先拓扑侧，其次基础连接。
    pub fn password(&self) -> Option<&str> {
        if self.topology.kind == RedisTopologyKind::Sentinel {
            if let Some(p) = self.topology.sentinel.password.value() {
                return Some(p);
            }
        }
        self.basic.password.value()
    }

    /// URI 归一化：目标 profile（或录入一个候选草稿，供 UI 回填）。
    pub fn from_uri(uri: &str) -> std::result::Result<Self, String> {
        parse_redis_uri(uri)
    }

    /// 校验：返回第一条不合法的中文错误；`None` 表示通过。
    pub fn validate(&self) -> Option<String> {
        match self.topology.kind {
            RedisTopologyKind::Standalone => {
                if self.basic.host.is_empty() {
                    return Some("请填写主机".to_string());
                }
            }
            RedisTopologyKind::Sentinel => {
                if self.topology.sentinel.master_name.is_empty() {
                    return Some("请填写 Sentinel 主库名".to_string());
                }
                if self.topology.sentinel.endpoints.is_empty() && self.basic.host.is_empty() {
                    return Some("请至少填写一个 Sentinel 节点".to_string());
                }
            }
            RedisTopologyKind::Cluster => {
                if self.topology.cluster.start_nodes.is_empty() && self.basic.host.is_empty() {
                    return Some("请至少填写一个 Cluster 节点".to_string());
                }
            }
        }
        if self.ssh.enabled && self.ssh.host.is_empty() {
            return Some("请填写 SSH 隧道主机".to_string());
        }
        None
    }
}

/// —— URI / host:port 文本解析（依赖无关）——

/// 解析 `host:port`；支持 IPv6 `\[::1\]:6379`。失败返回 None。
fn parse_host_port(input: &str) -> Option<(String, u16)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let (host, port) = if let Some(rest) = input.strip_prefix('[') {
        // IPv6 字面量：`[::1]:6379`
        let close = rest.find(']')?;
        let host = rest[..close].to_string();
        let port = rest[close + 1..]
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())?;
        (host, port)
    } else {
        let idx = input.rfind(':')?;
        let host = input[..idx].trim_matches(['[', ']']).to_string();
        let port = input[idx + 1..].parse().ok()?;
        if host.is_empty() {
            return None;
        }
        (host, port)
    };
    Some((host, port))
}

/// 把 `a:1,b:2` 拆成 `["a:1","b:2"]`，丢弃空项。
fn split_host_port_list(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 百分号解码一段 URI 组件。不是合法转义时按原样保留（宽松解码）。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 解析 redis 系列 URI 到档案。
///
/// 支持 scheme：
/// - `redis://` 明文
/// - `rediss://` TLS
/// - `redis+sentinel://` 哨兵
/// - `redis-cluster://` 集群
///
/// authority：`[user[:password]@]host[:port]`；path 首个段为 db 序号。
/// query：`?tls_ca_ref=...&db=...` 等可映射到档案的参数。
fn parse_redis_uri(uri: &str) -> std::result::Result<RedisConnectionProfile, String> {
    let uri = uri.trim();
    let (host_part, scheme_tail) = split_scheme(uri)?;

    let mut tls = false;
    let mut sentinel = false;
    let mut cluster = false;
    match scheme_tail.as_str() {
        "redis" => {}
        "rediss" => tls = true,
        "redis+sentinel" => {
            sentinel = true;
            tls = true; // sentinel 默认走 TLS，可被 query 关闭
        }
        "redis+sentinel+tls" => {
            sentinel = true;
            tls = true;
        }
        "redis-cluster" => cluster = true,
        other => return Err(format!("不支持的 Redis URI 协议: {other}")),
    }

    // 拆 authority / path / query / fragment
    let (rest, _fragment) = host_part.split_once('#').unwrap_or((host_part, ""));
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));

    let (host, port, user, password) = parse_authority(authority)?;
    let database = if path.is_empty() {
        None
    } else {
        Some(path.split('/').next().unwrap_or("").to_string())
    };

    // query 参数（宽松），覆盖默认值。
    let mut query_map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            query_map
                .insert(percent_decode(k), percent_decode(v));
        }
    }

    let mut tls_opts = RedisTlsOptions {
        enabled: tls,
        ca: SecretRef::ref_key(query_map.remove("tls_ca_ref").unwrap_or_default()),
        client_cert: SecretRef::ref_key(query_map.remove("tls_client_cert_ref").unwrap_or_default()),
        client_key: SecretRef::ref_key(query_map.remove("tls_client_key_ref").unwrap_or_default()),
        sni: query_map.remove("tls_server_name").unwrap_or_default(),
        verify: !option_is_true(query_map.get("tls_insecure")),
    };
    if let Some(v) = query_map.remove("tls") {
        tls_opts.enabled = option_is_true(Some(&v));
    }
    if let Some(v) = query_map.remove("tls_insecure") {
        tls_opts.verify = !option_is_true(Some(&v));
    }

    let master_name = query_map.remove("sentinel_master");
    let mut endpoints = split_host_port_list(query_map.remove("sentinel_endpoints").as_deref());
    let mut start_nodes = split_host_port_list(query_map.remove("cluster_start_nodes").as_deref());
    let allow_readonly = option_is_true(query_map.get("cluster_readonly"));

    let mut topology_kind = if sentinel {
        RedisTopologyKind::Sentinel
    } else if cluster {
        RedisTopologyKind::Cluster
    } else {
        RedisTopologyKind::Standalone
    };
    // path 里带 db 时，数据库号即逻辑库（哨兵 path 语义也可能不带 db）。
    let database = query_map
        .remove("db")
        .or(database);
    if master_name.is_some() {
        topology_kind = RedisTopologyKind::Sentinel;
    }

    let username = user.or_else(|| query_map.remove("username")).filter(|u| !u.is_empty());
    let password = password
        .or_else(|| query_map.remove("password"))
        .map(SecretRef::inline)
        .unwrap_or_default();

    // 哨兵端点未在 query 显式给时，用 authority 的 host:port 作首哨兵。
    if topology_kind == RedisTopologyKind::Sentinel && endpoints.is_empty() && !host.is_empty() {
        endpoints.push(format!("{host}:{port}"));
    }
    // 集群起始节点同理。
    if topology_kind == RedisTopologyKind::Cluster && start_nodes.is_empty() && !host.is_empty() {
        start_nodes.push(format!("{host}:{port}"));
    }

    let profile = RedisConnectionProfile {
        basic: RedisBasicOptions {
            host,
            port,
            database,
            username: username.clone(),
            password,
        },
        tls: tls_opts.clone(),
        ssh: RedisSshOptions::default(),
        topology: RedisTopology {
            kind: topology_kind,
            sentinel: RedisSentinelOptions {
                master_name: master_name.unwrap_or_default(),
                endpoints,
                username: username.clone(),
                password: SecretRef::default(),
                tls: tls_opts,
            },
            cluster: RedisClusterOptions {
                start_nodes,
                allow_readonly,
            },
        },
        cloud: RedisCloudOptions::default(),
    };
    let _ = scheme_tail;
    Ok(profile)
}

/// 拆出 scheme 与剩余部分。返回 `(剩余部分, scheme)`。
fn split_scheme(uri: &str) -> std::result::Result<(&str, String), String> {
    let Some(colon) = uri.find(':') else {
        return Err("无法识别的连接字符串".to_string());
    };
    let scheme = uri[..colon].to_ascii_lowercase();
    let rest = uri[colon + 1..].strip_prefix("//").unwrap_or(&uri[colon + 1..]);
    Ok((rest, scheme))
}

/// 解析 authority 段的 user / pass / host / port。
fn parse_authority(authority: &str) -> std::result::Result<(String, u16, Option<String>, Option<String>), String> {
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((ui, hp)) => (Some(ui), hp),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(ui) => match ui.split_once(':') {
            Some((u, p)) => (
                Some(percent_decode(u)),
                Some(percent_decode(p)).filter(|p| !p.is_empty()),
            ),
            None => (Some(percent_decode(ui)), None),
        },
        None => (None, None),
    };
    let (host, port) = parse_host_port(hostport).unwrap_or_else(|| {
        // 无端口时默认 6379
        (hostport.trim_matches(['[', ']']).to_string(), 6379_u16)
    });
    if host.is_empty() {
        return Err("缺少主机地址".to_string());
    }
    Ok((host, port, user, password))
}

/// 抹掉连接串里的口令（`:password@` → 仅保留 `user@`），
/// 供导入元数据 `imported_name` 记录来源时使用，避免明文口令随元数据落盘。
pub fn redact_uri_password(uri: &str) -> String {
    // 先切开 scheme(`://`) 前缀，仅在 authority 的 userinfo 段内抹掉口令，
    // 避免把 `://` 的斜杠误判为 user/pass 的分隔符。
    let (prefix, rest) = match uri.find("://") {
        Some(idx) => (&uri[..idx + 3], &uri[idx + 3..]),
        None => ("", uri),
    };
    match rest.rsplit_once('@') {
        Some((userinfo, hostport)) => {
            // 只保留 userinfo 中的用户名，丢弃 `:密码` 段；无密码时不改动。
            let user = userinfo.split_once(':').map(|(u, _)| u).unwrap_or(userinfo);
            format!("{prefix}{user}@{hostport}")
        }
        None => uri.to_string(),
    }
}

#[cfg(test)]
mod redis_profile_tests {
    use super::*;

    fn opt<'a>(
        map: &'a std::collections::BTreeMap<String, String>,
        key: &str,
    ) -> Option<&'a String> {
        map.get(key)
    }

    #[test]
    fn parses_plain_redis_uri() {
        let p = RedisConnectionProfile::from_uri("redis://localhost:6379").unwrap();
        assert_eq!(p.basic.host, "localhost");
        assert_eq!(p.basic.port, 6379);
        assert!(!p.tls.enabled);
        assert_eq!(p.topology.kind, RedisTopologyKind::Standalone);
    }

    #[test]
    fn parses_tls_uri_with_auth_and_db() {
        let p = RedisConnectionProfile::from_uri("rediss://user:pass@example.com:6380/3").unwrap();
        assert!(p.tls.enabled);
        assert_eq!(p.basic.host, "example.com");
        assert_eq!(p.basic.port, 6380);
        assert_eq!(p.basic.username.as_deref(), Some("user"));
        assert_eq!(p.basic.password.value(), Some("pass"));
        assert_eq!(p.basic.database.as_deref(), Some("3"));
    }

    #[test]
    fn parses_uri_with_encoded_special_chars() {
        let p = RedisConnectionProfile::from_uri(
            "redis://user%20x:p%40ss@127.0.0.1:6379/1",
        )
        .unwrap();
        assert_eq!(p.basic.username.as_deref(), Some("user x"));
        assert_eq!(p.basic.password.value(), Some("p@ss"));
        assert_eq!(p.basic.database.as_deref(), Some("1"));
    }

    #[test]
    fn redact_uri_password_drops_credential_but_keeps_audit() {
        // 导入元数据不应携带明文口令；用户名/主机仍保留以作来源审计。
        assert_eq!(
            redact_uri_password("rediss://:pass@mycache.redis.cache.windows.net:6380/0"),
            "rediss://@mycache.redis.cache.windows.net:6380/0"
        );
        assert_eq!(
            redact_uri_password("redis://user:secret@example.com:6379"),
            "redis://user@example.com:6379"
        );
        // 无口令时原样保留。
        assert_eq!(
            redact_uri_password("redis://localhost:6379"),
            "redis://localhost:6379"
        );
        assert_eq!(
            redact_uri_password("redis://user@example.com:6379"),
            "redis://user@example.com:6379"
        );
    }

    #[test]
    fn parses_sentinel_uri() {
        let p = RedisConnectionProfile::from_uri(
            "redis+sentinel://:pass@sentinel0:26379?sentinel_master=mymaster",
        )
        .unwrap();
        assert_eq!(p.topology.kind, RedisTopologyKind::Sentinel);
        assert_eq!(p.topology.sentinel.master_name, "mymaster");
        assert_eq!(p.topology.sentinel.endpoints, vec!["sentinel0:26379"]);
    }

    #[test]
    fn parses_cluster_uri() {
        let p = RedisConnectionProfile::from_uri(
            "redis-cluster://node1:7000?cluster_start_nodes=node1:7000,node2:7000&cluster_readonly=true",
        )
        .unwrap();
        assert_eq!(p.topology.kind, RedisTopologyKind::Cluster);
        assert_eq!(
            p.topology.cluster.start_nodes,
            vec!["node1:7000".to_string(), "node2:7000".to_string()]
        );
        assert!(p.topology.cluster.allow_readonly);
    }

    #[test]
    fn rejects_unknown_scheme() {
        assert!(RedisConnectionProfile::from_uri("http://x").is_err());
    }

    #[test]
    fn round_trips_options_through_profile() {
        let mut opts = std::collections::BTreeMap::new();
        opts.insert("tls".to_string(), "true".to_string());
        opts.insert("username".to_string(), "admin".to_string());
        opts.insert("password".to_string(), "secret".to_string());
        opts.insert("sentinel_master".to_string(), "mymaster".to_string());
        let p = RedisConnectionProfile::from_options(&opts);
        assert_eq!(p.topology.kind, RedisTopologyKind::Sentinel);
        assert_eq!(p.basic.username.as_deref(), Some("admin"));
        // 回写后关键开关仍在
        let back = p.into_options();
        assert_eq!(opt(&back, "tls").map(String::as_str), Some("true"));
        assert_eq!(opt(&back, "sentinel_master").map(String::as_str), Some("mymaster"));
    }

    #[test]
    fn sentinel_password_flows_into_resolved_options() {
        // 哨兵侧设置了密码、基础密码为空：回写后 password 键应取哨兵密码，
        // 保证哨兵探测与主库 AUTH 能正确鉴权（共用密码的常见哨兵部署）。
        let p = RedisConnectionProfile {
            basic: RedisBasicOptions {
                host: "s1".into(),
                port: 26379,
                password: SecretRef::default(),
                ..Default::default()
            },
            topology: RedisTopology {
                kind: RedisTopologyKind::Sentinel,
                sentinel: RedisSentinelOptions {
                    master_name: "mymaster".into(),
                    endpoints: vec!["s1:26379".into()],
                    password: SecretRef::inline("sentinel-pass"),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let back = p.into_options();
        assert_eq!(
            opt(&back, "password").map(String::as_str),
            Some("sentinel-pass")
        );
    }

    #[test]
    fn secret_ref_never_serializes_inline() {
        let s = RedisConnectionProfile {
            basic: RedisBasicOptions {
                password: SecretRef::inline("hunter2"),
                ..Default::default()
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("hunter2"));
        assert!(!json.contains("inline"));
    }

    #[test]
    fn parses_ipv6_host_port() {
        let (host, port) = parse_host_port("[::1]:6379").unwrap();
        assert_eq!(host, "::1");
        assert_eq!(port, 6379);
    }

    #[test]
    fn validate_reports_missing_host() {
        let p = RedisConnectionProfile::default();
        assert_eq!(p.validate(), Some("请填写主机".to_string()));
    }

    #[test]
    fn infers_azure_resource_from_host() {
        let (resource, sub) =
            infer_cloud_from_host("azure", "mycache.redis.cache.windows.net");
        assert_eq!(resource, "mycache");
        assert!(sub.is_empty());
    }

    #[test]
    fn infers_redis_cloud_resource_from_host() {
        let (resource, _) =
            infer_cloud_from_host("redis-cloud", "redis-12345.c263.us-east-1-mz.ec2.redns.redis-cloud.com");
        assert_eq!(resource, "redis-12345");
    }

    #[test]
    fn unknown_provider_uses_full_host() {
        let (resource, _) = infer_cloud_from_host("", "some.host:9000");
        // dial 出的 host 是纯主机名，此处传入即整段作为 resource。
        assert_eq!(resource, "some.host:9000");
    }
}
