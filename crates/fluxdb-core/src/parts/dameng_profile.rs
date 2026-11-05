// 达梦 DM8 专用连接配置模型。
//
// 达梦以“实例 + schema”组织对象，连接时选中的 database 只用于展示与默认 schema；
// 密码必须走受控引用，不能进入普通 DTO/日志/导出。

/// 达梦连接目标。`database` 是实例名（DM 通常为 DAMENG），`schema` 是登录后的默认模式。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DamengConnectionProfile {
    pub basic: DamengBasicOptions,
    pub advanced: DamengAdvancedOptions,
    /// SSH 隧道；启用后拨号目标改写为本地转发端口。
    #[serde(default)]
    pub ssh: SshTunnelProfile,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DamengBasicOptions {
    pub host: String,
    pub port: u16,
    /// 实例名（展示用，不参与 DM 登录；DM 登录以用户所属 schema 为准）。
    pub database: String,
    /// 默认 schema；空则使用用户同名 schema。
    pub schema: String,
    pub username: String,
    /// 密码走受控引用；`inline` 落盘前由 storage 剥离。
    pub password: SecretRef,
}

impl Default for DamengBasicOptions {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 5236,
            database: "DAMENG".into(),
            schema: String::new(),
            username: "SYSDBA".into(),
            password: SecretRef::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DamengAdvancedOptions {
    pub connect_timeout_secs: u32,
    /// 0 表示不设置语句超时；>0 时在客户端侧中止等待。
    pub query_timeout_secs: u32,
    /// 服务器字符集提示；空表示由驱动握手自动检测。
    pub encoding: String,
    /// 应用名，写入 DM 会话。
    pub application_name: String,
}

impl Default for DamengAdvancedOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 5,
            query_timeout_secs: 0,
            encoding: String::new(),
            application_name: "FluxDB".into(),
        }
    }
}

impl DamengConnectionProfile {
    pub fn from_options(opts: &std::collections::BTreeMap<String, String>) -> Self {
        let mut profile = Self::default();
        if let Some(host) = opts.get("host") {
            profile.basic.host = host.clone();
        }
        if let Some(port) = opts.get("port").and_then(|value| value.parse().ok()) {
            profile.basic.port = port;
        }
        if let Some(database) = opts.get("database").filter(|value| !value.is_empty()) {
            profile.basic.database = database.clone();
        }
        if let Some(schema) = opts.get("schema") {
            profile.basic.schema = schema.clone();
        }
        if let Some(username) = opts.get("username") {
            profile.basic.username = username.clone();
        }
        if let Some(password) = opts.get("password") {
            profile.basic.password = SecretRef::inline(password.clone());
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
        if let Some(encoding) = opts.get("encoding") {
            profile.advanced.encoding = encoding.clone();
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
        options.insert("username".into(), self.basic.username.clone());
        if let Some(password) = self.basic.password.value() {
            options.insert("password".into(), password.to_string());
        }
        options.insert(
            "connect_timeout_secs".into(),
            self.advanced.connect_timeout_secs.to_string(),
        );
        options.insert(
            "query_timeout_secs".into(),
            self.advanced.query_timeout_secs.to_string(),
        );
        options.insert("encoding".into(), self.advanced.encoding.clone());
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
