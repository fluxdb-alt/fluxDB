// MySQL/TiDB 专用连接配置模型。
//
// 目标：把原本分散在 `ConnectionConfig.options` 这组扁平字符串里的 TLS /
// SSH 隧道 / 代理 / 超时（connect/query/idle TTL）等连接语义，收敛成结构化、
// 可迁移、可校验的单一模型，作为 `ConnectionConfig` 的补充。
//
// 设计约定（镜像 redis_profile.rs 的既有范式）：
// - 本模型只依赖 `serde` 与 `std`，不引入第三方 URL 解析库。
// - 所有密钥类内容（密码、私钥、证书正文、SSH 口令）都走 [`SecretRef`]：引用
//   （`key`）可落盘，`inline` 受控值标记 `#[serde(skip)]`，绝不序列化到磁盘。
// - 通过 [`MysqlConnectionProfile::from_options`] 承接历史 `options` 扁平参数，
//   通过 [`MysqlConnectionProfile::into_options`] 回写兼容参数供现有消费方复用。
//
// 范围说明：SSH 隧道复用 Redis 的 `SshTunnel` 真正拨号；SOCKS5/HTTP 代理由连接器
// （`transport/proxy.rs`）真正握手后转发，二者互斥。

/// MySQL TLS 模式，与 MySQL 官方 `--ssl-mode` 五档一致。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum MysqlSslMode {
    /// 禁用 TLS。
    Disabled,
    /// 优先 TLS（服务器支持则加密，不支持则明文），默认。
    #[default]
    Preferred,
    /// 强制 TLS：服务器不支持则连接失败。
    Required,
    /// 强制 TLS 并校验证书链，不校验主机名。
    VerifyCa,
    /// 强制 TLS 并校验证书链与主机名。
    VerifyIdentity,
}

impl MysqlSslMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Preferred => "preferred",
            Self::Required => "required",
            Self::VerifyCa => "verify_ca",
            Self::VerifyIdentity => "verify_identity",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "disabled" => Self::Disabled,
            "required" => Self::Required,
            "verify_ca" | "verify-ca" => Self::VerifyCa,
            "verify_identity" | "verify-identity" | "verify_full" | "verify-full" => {
                Self::VerifyIdentity
            }
            _ => Self::Preferred,
        }
    }

    /// 是否需要校验证书链（CA 字段只在这两档下生效）。
    pub fn verifies_certificate(self) -> bool {
        matches!(self, Self::VerifyCa | Self::VerifyIdentity)
    }

    /// 是否需要校验主机名。
    pub fn verifies_hostname(self) -> bool {
        self == Self::VerifyIdentity
    }
}

/// TLS 相关参数。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MysqlTlsOptions {
    /// 是否启用 TLS。
    pub enabled: bool,
    /// TLS 模式。
    pub ssl_mode: MysqlSslMode,
    /// CA 证书文件路径引用。
    pub ca: SecretRef,
    /// 客户端证书文件路径引用。
    pub client_cert: SecretRef,
    /// 客户端私钥文件路径引用。
    pub client_key: SecretRef,
    /// SNI / 校验主机名（默认取连接主机名）。MySQL 驱动不支持独立覆盖，保留供归档。
    pub sni: String,
    /// 历史字段：旧版本用独立开关表达"校验证书"。
    ///
    /// 新逻辑以 `ssl_mode` 为准；此处仅用于把旧配置（`verify=true` +
    /// `preferred/required`）迁移为 `verify_ca`，写回时由模式反推。
    pub verify: bool,
    /// 连接字符集（默认 utf8mb4）。
    pub charset: String,
    /// 排序规则。
    pub collation: String,
}

impl Default for MysqlTlsOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            ssl_mode: MysqlSslMode::Preferred,
            ca: SecretRef::default(),
            client_cert: SecretRef::default(),
            client_key: SecretRef::default(),
            sni: String::new(),
            verify: false,
            charset: "utf8mb4".into(),
            collation: String::new(),
        }
    }
}

impl MysqlTlsOptions {
    /// 实际生效的 TLS 模式：兼容旧配置里的独立校验开关。
    ///
    /// 旧版本把"校验证书"放在 `verify` 布尔位，模式可能仍是 `preferred`；
    /// 升级后统一映射为 `verify_ca`，避免安全级别被悄悄降低。
    ///
    /// 仅在 TLS 启用时迁移：旧版 `Default` 就是 `verify: true`，TLS 关闭时
    /// 该位没有实际含义；无条件迁移会产出「TLS 关 + verify_ca」的非法态，
    /// 校验无法保存且模式下拉在 TLS 关闭时不渲染，用户无法回退。
    pub fn effective_ssl_mode(&self) -> MysqlSslMode {
        match self.ssl_mode {
            MysqlSslMode::Preferred | MysqlSslMode::Required if self.enabled && self.verify => {
                MysqlSslMode::VerifyCa
            }
            mode => mode,
        }
    }
}

/// SSH 认证方式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum MysqlSshAuth {
    #[default]
    Password,
    PrivateKey,
}

/// SSH 隧道参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MysqlSshOptions {
    /// 是否启用 SSH 隧道。
    pub enabled: bool,
    /// 跳板主机。
    pub host: String,
    /// 跳板端口（默认 22）。
    pub port: u16,
    /// 跳板用户名。
    pub username: String,
    /// SSH 认证类型。
    pub auth: MysqlSshAuth,
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
    /// 主机密钥校验策略；默认严格校验 known_hosts。
    pub host_key_policy: SshHostKeyPolicy,
}

/// 代理类型。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum MysqlProxyType {
    #[default]
    Socks5,
    HttpConnect,
}

impl MysqlProxyType {
    /// 该类型代理的默认端口。
    pub fn default_port(&self) -> u16 {
        match self {
            MysqlProxyType::Socks5 => 1080,
            MysqlProxyType::HttpConnect => 8080,
        }
    }
}

/// 代理参数（SOCKS5 / HTTP CONNECT）。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MysqlProxy {
    /// 是否启用代理。
    pub enabled: bool,
    /// 代理类型。
    pub proxy_type: MysqlProxyType,
    /// 代理主机。
    pub host: String,
    /// 代理端口。
    pub port: u16,
    /// 代理用户名（可选）。
    pub username: String,
    /// 代理密码（可选）。
    pub password: SecretRef,
}

/// 传输层：SSH 隧道或代理。`transport: Vec` 支持多跳扩展；两段目前互斥。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MysqlTransportLayer {
    Ssh(MysqlSshOptions),
    Proxy(MysqlProxy),
}

/// 高级连接选项。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MysqlAdvancedOptions {
    /// 建连超时（秒），默认 5。
    pub connect_timeout_secs: u32,
    /// 查询超时（秒），0 表示不设限。
    pub query_timeout_secs: u32,
    /// 底层 TCP 连接保活（`SO_KEEPALIVE`）。
    ///
    /// sqlx 不暴露套接字，开启后连接改经本地转发建立，由转发器设置保活；
    /// 默认关闭以保持直连路径。
    pub tcp_keepalive: bool,
    /// 会话时区（如 `+08:00` 或 `Asia/Shanghai`）；空表示沿用驱动默认。
    pub timezone: String,
    /// 连接建立后执行的初始化语句；按分号拆分后顺序执行（引号/注释内的分号不拆分）。
    pub init_sql: String,
}

impl Default for MysqlAdvancedOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 5,
            query_timeout_secs: 0,
            tcp_keepalive: false,
            timezone: String::new(),
            init_sql: String::new(),
        }
    }
}

/// 基础连接参数。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MysqlBasicOptions {
    /// 目标主机。
    pub host: String,
    /// 目标端口（默认 3306）。
    pub port: u16,
    /// 逻辑库名。
    pub database: String,
    /// 用户名（默认 root）。
    pub username: String,
    /// 认证密码（不安全）。
    pub password: SecretRef,
    /// Unix socket 路径；非空时代替 TCP 主机/端口拨号。
    pub socket_path: String,
}

/// MySQL 完整连接档案：承接 TLS / SSH / 代理 / 高级超时。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MysqlConnectionProfile {
    pub basic: MysqlBasicOptions,
    pub tls: MysqlTlsOptions,
    pub transport: Vec<MysqlTransportLayer>,
    pub advanced: MysqlAdvancedOptions,
}

impl MysqlConnectionProfile {
    /// —— 解析职责 ——

    /// 从历史 `options` 扁平参数迁移出一份档案。
    ///
    /// 只读取已知键，其余任意参数不做透传，避免把垃圾灌进结构化模型。
    pub fn from_options(opts: &std::collections::BTreeMap<String, String>) -> Self {
        let get = |key: &str| opts.get(key).cloned();
        let basic = MysqlBasicOptions {
            host: opts.get("host").cloned().unwrap_or_default(),
            port: opts
                .get("port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(3306),
            database: get("database").unwrap_or_default(),
            username: get("username").unwrap_or_else(|| "root".into()),
            password: SecretRef::inline(get("password").unwrap_or_default()),
            socket_path: get("socket").unwrap_or_default(),
        };
        let tls = MysqlTlsOptions {
            enabled: connection_option_is_true(get("tls").as_ref()),
            ssl_mode: MysqlSslMode::parse(get("ssl_mode").as_deref().unwrap_or("")),
            ca: SecretRef::ref_key(get("tls_ca_ref").unwrap_or_default()),
            client_cert: SecretRef::ref_key(get("tls_client_cert_ref").unwrap_or_default()),
            client_key: SecretRef::ref_key(get("tls_client_key_ref").unwrap_or_default()),
            sni: get("tls_server_name").unwrap_or_default(),
            verify: !connection_option_is_true(get("tls_insecure").as_ref()),
            charset: get("charset")
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| "utf8mb4".into()),
            collation: get("collation").unwrap_or_default(),
        };
        let mut transport = Vec::new();
        let ssh = MysqlSshOptions {
            enabled: connection_option_is_true(get("ssh_enabled").as_ref()),
            host: get("ssh_host").unwrap_or_default(),
            port: get("ssh_port").and_then(|p| p.parse().ok()).unwrap_or(22),
            username: get("ssh_username").unwrap_or_default(),
            auth: match get("ssh_auth").as_deref() {
                Some("key") | Some("private_key") => MysqlSshAuth::PrivateKey,
                _ => MysqlSshAuth::Password,
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
            host_key_policy: SshHostKeyPolicy::parse(
                get("ssh_host_key_policy").as_deref().unwrap_or(""),
            ),
        };
        if ssh.enabled {
            transport.push(MysqlTransportLayer::Ssh(ssh));
        }
        let proxy = MysqlProxy {
            enabled: connection_option_is_true(get("proxy_enabled").as_ref()),
            proxy_type: match get("proxy_type").as_deref() {
                Some("http_connect") => MysqlProxyType::HttpConnect,
                _ => MysqlProxyType::Socks5,
            },
            host: get("proxy_host").unwrap_or_default(),
            port: get("proxy_port")
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(|| MysqlProxyType::Socks5.default_port()),
            username: get("proxy_username").unwrap_or_default(),
            password: SecretRef::inline(get("proxy_password").unwrap_or_default()),
        };
        if proxy.enabled {
            transport.push(MysqlTransportLayer::Proxy(proxy));
        }
        Self {
            basic,
            tls,
            transport,
            advanced: MysqlAdvancedOptions {
                connect_timeout_secs: get("connect_timeout_secs")
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(5),
                query_timeout_secs: get("query_timeout_secs")
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(0),
                tcp_keepalive: connection_option_is_true(get("tcp_keepalive").as_ref()),
                timezone: get("timezone").unwrap_or_default(),
                init_sql: get("init_sql").unwrap_or_default(),
            },
        }
    }

    /// 把档案回写成一组合法的 `options` 兼容参数。
    ///
    /// 供既有扁平参数消费方（如连接测试/保存路径）复用；不写密钥正文，
    /// 密码走引用/占用 `password` 键交给 storage 决定是否落盘。
    pub fn into_options(&self) -> std::collections::BTreeMap<String, String> {
        let mut ops = std::collections::BTreeMap::new();
        if !self.basic.host.is_empty() {
            ops.insert("host".into(), self.basic.host.clone());
        }
        ops.insert("port".into(), self.basic.port.to_string());
        if !self.basic.database.is_empty() {
            ops.insert("database".into(), self.basic.database.clone());
        }
        if !self.basic.username.is_empty() {
            ops.insert("username".into(), self.basic.username.clone());
        }
        if let Some(password) = self.basic.password.value() {
            ops.insert("password".into(), password.to_string());
        }
        if !self.basic.socket_path.trim().is_empty() {
            ops.insert("socket".into(), self.basic.socket_path.trim().into());
        }

        let t = &self.tls;
        let mode = t.effective_ssl_mode();
        ops.insert("tls".into(), t.enabled.to_string());
        ops.insert("ssl_mode".into(), mode.as_str().into());
        if !t.sni.is_empty() {
            ops.insert("tls_server_name".into(), t.sni.clone());
        }
        // 兼容字段：由模式反推，保证旧消费方看到的"是否校验"与新语义一致。
        ops.insert("tls_insecure".into(), (!mode.verifies_certificate()).to_string());
        if !t.ca.key.is_empty() || !t.client_cert.key.is_empty() || !t.client_key.key.is_empty() {
            ops.insert("tls_ca_ref".into(), t.ca.key.clone());
            ops.insert("tls_client_cert_ref".into(), t.client_cert.key.clone());
            ops.insert("tls_client_key_ref".into(), t.client_key.key.clone());
        }
        if t.charset != "utf8mb4" {
            ops.insert("charset".into(), t.charset.clone());
        }
        if !t.collation.is_empty() {
            ops.insert("collation".into(), t.collation.clone());
        }

        for layer in &self.transport {
            match layer {
                MysqlTransportLayer::Ssh(ssh) if ssh.enabled => {
                    ops.insert("ssh_enabled".into(), "true".into());
                    ops.insert("ssh_host".into(), ssh.host.clone());
                    ops.insert("ssh_port".into(), ssh.port.to_string());
                    ops.insert("ssh_username".into(), ssh.username.clone());
                    ops.insert(
                        "ssh_auth".into(),
                        match ssh.auth {
                            MysqlSshAuth::Password => "password".into(),
                            MysqlSshAuth::PrivateKey => "private_key".into(),
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
                    if ssh.host_key_policy != SshHostKeyPolicy::Strict {
                        ops.insert(
                            "ssh_host_key_policy".into(),
                            ssh.host_key_policy.as_str().into(),
                        );
                    }
                }
                MysqlTransportLayer::Proxy(proxy) if proxy.enabled => {
                    ops.insert("proxy_enabled".into(), "true".into());
                    ops.insert(
                        "proxy_type".into(),
                        match proxy.proxy_type {
                            MysqlProxyType::Socks5 => "socks5".into(),
                            MysqlProxyType::HttpConnect => "http_connect".into(),
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
        ops.insert("tcp_keepalive".into(), a.tcp_keepalive.to_string());
        if !a.timezone.trim().is_empty() {
            ops.insert("timezone".into(), a.timezone.trim().into());
        }
        if !a.init_sql.trim().is_empty() {
            ops.insert("init_sql".into(), a.init_sql.clone());
        }

        ops
    }

    /// —— 拨号辅助 ——

    /// 首个启用的 SSH 隧道层（无则 `None`）。
    pub fn ssh(&self) -> Option<&MysqlSshOptions> {
        self.transport.iter().find_map(|layer| match layer {
            MysqlTransportLayer::Ssh(ssh) if ssh.enabled => Some(ssh),
            _ => None,
        })
    }

    /// 首个启用的代理层（无则 `None`）。
    pub fn proxy(&self) -> Option<&MysqlProxy> {
        self.transport.iter().find_map(|layer| match layer {
            MysqlTransportLayer::Proxy(proxy) if proxy.enabled => Some(proxy),
            _ => None,
        })
    }

    /// 实际用于拨号的目标端点（未被 SSH 隧道改写的前提下）。
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

    /// 校验：返回第一条不合法的中文错误；`None` 表示通过。
    pub fn validate(&self) -> Option<String> {
        let socket_path = self.basic.socket_path.trim();
        if socket_path.is_empty() && self.basic.host.is_empty() {
            return Some("请填写主机".into());
        }
        if socket_path.is_empty() && self.basic.port == 0 {
            return Some("端口不合法".into());
        }
        let mode = self.tls.effective_ssl_mode();
        if mode != MysqlSslMode::Disabled && mode != MysqlSslMode::Preferred && !self.tls.enabled {
            return Some("强制 TLS 模式需先启用 TLS".into());
        }
        if !socket_path.is_empty() && (self.ssh().is_some() || self.proxy().is_some()) {
            return Some("Unix Socket 与 SSH 隧道/代理不能同时启用".into());
        }
        if let Some(proxy) = self.proxy() {
            if proxy.host.trim().is_empty() {
                return Some("请填写代理主机".into());
            }
            if proxy.port == 0 {
                return Some("代理端口不合法".into());
            }
            if self.ssh().is_some() {
                return Some("SSH 隧道与代理不能同时启用，请只保留一种".into());
            }
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
            if ssh.auth == MysqlSshAuth::PrivateKey && ssh.private_key.key.is_empty() {
                return Some("请填写 SSH 私钥文件路径".into());
            }
        }
        if self.advanced.connect_timeout_secs > 86400 || self.advanced.query_timeout_secs > 86400 {
            return Some("高级超时值不能超过 86400 秒".into());
        }
        None
    }
}

/// 把初始化 SQL 拆成可逐条执行的语句。
///
/// 分隔符为分号；引号、反引号以及 `--`/`#`/`/* */` 注释内的分号不拆分。
/// 空语句与纯空白会被丢弃，其余原文保留（注释随语句一起下发）。
pub fn split_mysql_init_sql(raw: &str) -> Vec<String> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Normal,
        Single,
        Double,
        Backtick,
        LineComment,
        BlockComment,
    }

    let mut statements = Vec::new();
    let mut current = String::new();
    let mut state = State::Normal;
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match state {
            State::Normal => match ch {
                ';' => {
                    let statement = current.trim();
                    if !statement.is_empty() {
                        statements.push(statement.to_string());
                    }
                    current.clear();
                }
                '\'' => {
                    state = State::Single;
                    current.push(ch);
                }
                '"' => {
                    state = State::Double;
                    current.push(ch);
                }
                '`' => {
                    state = State::Backtick;
                    current.push(ch);
                }
                '#' => {
                    state = State::LineComment;
                    current.push(ch);
                }
                '-' if chars.peek() == Some(&'-') => {
                    current.push(ch);
                    current.push(chars.next().unwrap_or('-'));
                    state = State::LineComment;
                }
                '/' if chars.peek() == Some(&'*') => {
                    current.push(ch);
                    current.push(chars.next().unwrap_or('*'));
                    state = State::BlockComment;
                }
                _ => current.push(ch),
            },
            // 字符串里支持反斜杠转义与 `''` / `""` 双写；两种写法都不结束字符串。
            State::Single | State::Double | State::Backtick => {
                current.push(ch);
                let quote = match state {
                    State::Single => '\'',
                    State::Double => '"',
                    _ => '`',
                };
                if ch == '\\' {
                    if let Some(escaped) = chars.next() {
                        current.push(escaped);
                    }
                } else if ch == quote {
                    if chars.peek() == Some(&quote) {
                        current.push(chars.next().unwrap_or(quote));
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::LineComment => {
                current.push(ch);
                if ch == '\n' {
                    state = State::Normal;
                }
            }
            State::BlockComment => {
                current.push(ch);
                if ch == '*' && chars.peek() == Some(&'/') {
                    current.push(chars.next().unwrap_or('/'));
                    state = State::Normal;
                }
            }
        }
    }
    let statement = current.trim();
    if !statement.is_empty() {
        statements.push(statement.to_string());
    }
    statements
}

#[cfg(test)]
mod mysql_profile_tests {
    use super::*;

    fn options() -> std::collections::BTreeMap<String, String> {
        let mut options = std::collections::BTreeMap::new();
        options.insert("host".into(), "db.example.com".into());
        options.insert("port".into(), "3307".into());
        options.insert("username".into(), "app".into());
        options.insert("database".into(), "shop".into());
        options
    }

    #[test]
    fn effective_mode_migrates_legacy_verify_flag() {
        let legacy = MysqlTlsOptions {
            enabled: true,
            ssl_mode: MysqlSslMode::Preferred,
            verify: true,
            ..MysqlTlsOptions::default()
        };
        assert_eq!(legacy.effective_ssl_mode(), MysqlSslMode::VerifyCa);
        // 新模式不再被历史开关覆盖。
        let explicit = MysqlTlsOptions {
            enabled: true,
            ssl_mode: MysqlSslMode::VerifyIdentity,
            verify: true,
            ..MysqlTlsOptions::default()
        };
        assert_eq!(explicit.effective_ssl_mode(), MysqlSslMode::VerifyIdentity);
    }

    /// 旧版 `Default` 是 `verify: true`，TLS 关闭时不能迁移成 `verify_ca`，
    /// 否则表单回填出「TLS 关 + 强制模式」，校验拒绝保存且无法在界面上改回。
    #[test]
    fn effective_mode_ignores_legacy_verify_when_tls_disabled() {
        let legacy_disabled = MysqlTlsOptions {
            enabled: false,
            ssl_mode: MysqlSslMode::Preferred,
            verify: true,
            ..MysqlTlsOptions::default()
        };
        assert_eq!(
            legacy_disabled.effective_ssl_mode(),
            MysqlSslMode::Preferred
        );

        // 显式选了强制档却没启用 TLS 仍要保留原值，由 validate 拦截。
        let explicit_required = MysqlTlsOptions {
            enabled: false,
            ssl_mode: MysqlSslMode::Required,
            verify: true,
            ..MysqlTlsOptions::default()
        };
        assert_eq!(
            explicit_required.effective_ssl_mode(),
            MysqlSslMode::Required
        );
    }

    #[test]
    fn ssl_mode_parse_and_flags() {
        assert_eq!(MysqlSslMode::parse("verify-ca"), MysqlSslMode::VerifyCa);
        assert_eq!(
            MysqlSslMode::parse("verify_identity"),
            MysqlSslMode::VerifyIdentity
        );
        assert_eq!(MysqlSslMode::parse("bogus"), MysqlSslMode::Preferred);
        assert!(MysqlSslMode::VerifyCa.verifies_certificate());
        assert!(!MysqlSslMode::VerifyCa.verifies_hostname());
        assert!(MysqlSslMode::VerifyIdentity.verifies_hostname());
    }

    #[test]
    fn profile_roundtrip_keeps_new_fields() {
        let mut flat = options();
        flat.insert("ssl_mode".into(), "verify_identity".into());
        flat.insert("collation".into(), "utf8mb4_0900_ai_ci".into());
        flat.insert("socket".into(), String::new());
        flat.insert("timezone".into(), "+08:00".into());
        flat.insert("init_sql".into(), "SET NAMES utf8mb4;SET time_zone='+08:00'".into());
        flat.insert("ssh_enabled".into(), "true".into());
        flat.insert("ssh_host".into(), "jump".into());
        flat.insert("ssh_username".into(), "ops".into());
        flat.insert("ssh_host_key_policy".into(), "ignore".into());

        let profile = MysqlConnectionProfile::from_options(&flat);
        assert_eq!(profile.tls.ssl_mode, MysqlSslMode::VerifyIdentity);
        assert_eq!(profile.tls.collation, "utf8mb4_0900_ai_ci");
        assert_eq!(profile.advanced.timezone, "+08:00");
        assert_eq!(profile.ssh().unwrap().host_key_policy, SshHostKeyPolicy::Ignore);

        let written = profile.into_options();
        assert_eq!(written.get("ssl_mode").map(String::as_str), Some("verify_identity"));
        assert_eq!(written.get("collation").map(String::as_str), Some("utf8mb4_0900_ai_ci"));
        assert_eq!(written.get("timezone").map(String::as_str), Some("+08:00"));
        assert_eq!(
            written.get("ssh_host_key_policy").map(String::as_str),
            Some("ignore")
        );
    }

    #[test]
    fn socket_replaces_host_and_port_validation() {
        let profile = MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                socket_path: "/var/run/mysqld/mysqld.sock".into(),
                port: 0,
                ..MysqlBasicOptions::default()
            },
            ..MysqlConnectionProfile::default()
        };
        assert_eq!(profile.validate(), None);
        // Socket 与 SSH 隧道互斥。
        let profile = MysqlConnectionProfile {
            transport: vec![MysqlTransportLayer::Ssh(MysqlSshOptions {
                enabled: true,
                host: "jump".into(),
                username: "ops".into(),
                ..MysqlSshOptions::default()
            })],
            ..profile
        };
        assert_eq!(
            profile.validate().as_deref(),
            Some("Unix Socket 与 SSH 隧道/代理不能同时启用")
        );
    }

    #[test]
    fn init_sql_splits_only_outside_quotes_and_comments() {
        assert_eq!(
            split_mysql_init_sql("SET NAMES utf8mb4; SET time_zone='+08:00'"),
            vec!["SET NAMES utf8mb4", "SET time_zone='+08:00'"]
        );
        assert_eq!(
            split_mysql_init_sql("INSERT INTO t VALUES ('a;b'); SELECT 1"),
            vec!["INSERT INTO t VALUES ('a;b')", "SELECT 1"]
        );
        assert_eq!(
            split_mysql_init_sql("SELECT 'it\\'s;fine'; SELECT 2"),
            vec!["SELECT 'it\\'s;fine'", "SELECT 2"]
        );
        assert_eq!(
            split_mysql_init_sql("SELECT 1 -- keep; together\n; SELECT 2"),
            vec!["SELECT 1 -- keep; together", "SELECT 2"]
        );
        assert_eq!(split_mysql_init_sql(" ; ; "), Vec::<String>::new());
    }
}
