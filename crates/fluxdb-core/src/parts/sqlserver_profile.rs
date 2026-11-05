// SQL Server 专用连接配置模型。
//
// SQL Server 的 database 与 schema 是两层：database 是物理库，schema 通常是 dbo。
// 密码必须走受控引用，不能进入普通 DTO、日志或导出。

/// SQL Server 加密模式。`Prefer` 映射到 tiberius 的 `Required`，失败时不自动降级，
/// 避免在用户不知情时把已配置的加密连接降为登录包加密。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SqlServerEncryption {
    /// 不要求全链路加密；登录包仍由 TDS 加密。
    Disable,
    /// 要求加密，并校验服务器证书。
    #[default]
    Require,
    /// 要求加密，但接受服务器自签证书。
    TrustServerCertificate,
}

impl SqlServerEncryption {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Require => "require",
            Self::TrustServerCertificate => "trust_server_certificate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "disable" | "off" | "false" => Some(Self::Disable),
            "require" | "required" | "true" => Some(Self::Require),
            "trust_server_certificate" | "trust" => Some(Self::TrustServerCertificate),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SqlServerConnectionProfile {
    pub basic: SqlServerBasicOptions,
    pub advanced: SqlServerAdvancedOptions,
    /// SSH 隧道；启用后拨号目标改写为本地转发端口。
    #[serde(default)]
    pub ssh: SshTunnelProfile,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SqlServerBasicOptions {
    pub host: String,
    pub port: u16,
    /// 登录默认库；空表示使用登录的默认数据库。
    pub database: String,
    /// 默认 schema；空表示连接后读取 `SCHEMA_NAME()`，读不到时回退 `dbo`。
    pub schema: String,
    /// 命名实例；显式端口非空时由端口优先。
    pub instance: String,
    pub username: String,
    /// 密码走受控引用；`inline` 落盘前由 storage 剥离。
    pub password: SecretRef,
}

impl Default for SqlServerBasicOptions {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 1433,
            database: String::new(),
            schema: String::new(),
            instance: String::new(),
            username: "sa".into(),
            password: SecretRef::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SqlServerAdvancedOptions {
    pub encryption: SqlServerEncryption,
    pub connect_timeout_secs: u32,
    /// 0 表示不设置语句超时；>0 时在客户端侧中止等待并关闭连接。
    pub query_timeout_secs: u32,
    pub application_name: String,
}

impl Default for SqlServerAdvancedOptions {
    fn default() -> Self {
        Self {
            encryption: SqlServerEncryption::TrustServerCertificate,
            connect_timeout_secs: 5,
            query_timeout_secs: 0,
            application_name: "FluxDB".into(),
        }
    }
}

impl SqlServerConnectionProfile {
    pub fn from_options(opts: &std::collections::BTreeMap<String, String>) -> Self {
        let mut profile = Self::default();
        if let Some(host) = opts.get("host") {
            profile.basic.host = host.clone();
        }
        if let Some(port) = opts.get("port").and_then(|value| value.parse().ok()) {
            profile.basic.port = port;
        }
        if let Some(database) = opts.get("database") {
            profile.basic.database = database.clone();
        }
        if let Some(schema) = opts.get("schema") {
            profile.basic.schema = schema.clone();
        }
        if let Some(instance) = opts.get("instance") {
            profile.basic.instance = instance.clone();
        }
        if let Some(username) = opts.get("username") {
            profile.basic.username = username.clone();
        }
        if let Some(password) = opts.get("password") {
            profile.basic.password = SecretRef::inline(password.clone());
        }
        if let Some(encryption) = opts.get("encryption").and_then(|value| SqlServerEncryption::parse(value)) {
            profile.advanced.encryption = encryption;
        }
        if let Some(timeout) = opts
            .get("connect_timeout_secs")
            .and_then(|value| value.parse().ok())
        {
            profile.advanced.connect_timeout_secs = timeout;
        }
        if let Some(timeout) = opts
            .get("query_timeout_secs")
            .and_then(|value| value.parse().ok())
        {
            profile.advanced.query_timeout_secs = timeout;
        }
        if let Some(application_name) = opts.get("application_name") {
            profile.advanced.application_name = application_name.clone();
        }
        profile.ssh = SshTunnelProfile::from_options(opts);
        profile
    }

    /// 回写扁平兼容参数（storage 会剥离其中的密码内联值）。
    pub fn into_options(&self) -> std::collections::BTreeMap<String, String> {
        let mut options = std::collections::BTreeMap::new();
        options.insert("host".into(), self.basic.host.clone());
        options.insert("port".into(), self.basic.port.to_string());
        options.insert("database".into(), self.basic.database.clone());
        options.insert("schema".into(), self.basic.schema.clone());
        options.insert("instance".into(), self.basic.instance.clone());
        options.insert("username".into(), self.basic.username.clone());
        if let Some(password) = self.basic.password.value() {
            options.insert("password".into(), password.to_string());
        }
        options.insert(
            "encryption".into(),
            self.advanced.encryption.as_str().to_string(),
        );
        options.insert(
            "connect_timeout_secs".into(),
            self.advanced.connect_timeout_secs.to_string(),
        );
        options.insert(
            "query_timeout_secs".into(),
            self.advanced.query_timeout_secs.to_string(),
        );
        options.insert(
            "application_name".into(),
            self.advanced.application_name.clone(),
        );
        options.extend(self.ssh.into_options());
        options
    }

    pub fn dial_endpoint(&self) -> (String, u16) {
        (self.basic.host.clone(), self.basic.port)
    }

    pub fn password(&self) -> Option<&str> {
        self.basic.password.value()
    }

    pub fn validate(&self) -> Option<String> {
        if self.basic.host.trim().is_empty() {
            return Some("请填写主机".into());
        }
        if self.basic.port == 0 {
            return Some("端口不合法".into());
        }
        if self.basic.username.trim().is_empty() {
            return Some("请填写用户名".into());
        }
        if self.advanced.connect_timeout_secs > 86400
            || self.advanced.query_timeout_secs > 86400
        {
            return Some("高级超时值不能超过 86400 秒".into());
        }
        self.ssh.validate()
    }
}
