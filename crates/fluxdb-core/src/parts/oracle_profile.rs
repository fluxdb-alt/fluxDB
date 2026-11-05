// Oracle 专用连接配置模型。
//
// Oracle 的 Service Name 与 SID 是两种互斥的监听器寻址方式，不能合成一个“数据库名”；
// 登录用户、默认 Schema（`ALTER SESSION SET CURRENT_SCHEMA`，只改名称解析、不增加权限）
// 与对象 Owner 是三件事，这里分别建模。密码与 Wallet 口令必须走受控引用，
// 不能进入普通 DTO、日志、导出或 AI 上下文。

/// Oracle 监听器寻址方式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum OracleConnectKind {
    /// Easy Connect：`host:port/service_name`，对应 PDB 或非 CDB 数据库的服务。
    #[default]
    ServiceName,
    /// SID：老式 `(CONNECT_DATA=(SID=…))`，只用于明确知道 SID 的场景。
    Sid,
}

impl OracleConnectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ServiceName => "service_name",
            Self::Sid => "sid",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "service_name" | "service" | "service name" => Some(Self::ServiceName),
            "sid" => Some(Self::Sid),
            _ => None,
        }
    }
}

/// Oracle 登录身份。普通连接默认 `Normal`；SYSDBA/SYSOPER 必须由用户显式选择，
/// 不能从用户名（如 `sys`）自动推断。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum OraclePrivilege {
    #[default]
    Normal,
    SysDba,
    SysOper,
}

impl OraclePrivilege {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::SysDba => "sysdba",
            Self::SysOper => "sysoper",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "normal" | "default" => Some(Self::Normal),
            "sysdba" | "sys_dba" => Some(Self::SysDba),
            "sysoper" | "sys_oper" => Some(Self::SysOper),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "",
            Self::SysDba => "SYSDBA",
            Self::SysOper => "SYSOPER",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct OracleConnectionProfile {
    pub basic: OracleBasicOptions,
    pub advanced: OracleAdvancedOptions,
    /// SSH 隧道；启用后拨号目标改写为本地转发端口。
    #[serde(default)]
    pub ssh: SshTunnelProfile,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OracleBasicOptions {
    pub host: String,
    pub port: u16,
    /// 监听器寻址方式；决定 `service_name` 与 `sid` 哪个生效。
    pub connect_kind: OracleConnectKind,
    /// Service Name（PDB 服务名或非 CDB 数据库服务名）。
    pub service_name: String,
    /// SID；仅 `connect_kind == Sid` 时使用。
    pub sid: String,
    /// 登录用户；Oracle 未加引号的用户名折叠为大写。
    pub username: String,
    /// 连接后切换的默认 Schema（只影响未限定名的解析，不代表权限）。
    pub schema: String,
    /// 登录身份；默认普通连接，SYSDBA/SYSOPER 需显式选择。
    pub privilege: OraclePrivilege,
    /// 密码走受控引用；使用 Wallet 时可以为空。
    pub password: SecretRef,
}

impl Default for OracleBasicOptions {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 1521,
            connect_kind: OracleConnectKind::ServiceName,
            service_name: String::new(),
            sid: String::new(),
            username: "system".into(),
            schema: String::new(),
            privilege: OraclePrivilege::Normal,
            password: SecretRef::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OracleAdvancedOptions {
    pub connect_timeout_secs: u32,
    /// 0 表示不设置语句超时；>0 时由驱动在客户端侧中止调用。
    pub query_timeout_secs: u32,
    pub application_name: String,
    /// Wallet 目录（本地路径）；空表示不使用 Wallet。目录不是凭据，但口令不是。
    pub wallet_location: String,
    /// Wallet 口令；只在 `wallet_location` 非空时有意义，不得落盘明文。
    pub wallet_password: SecretRef,
}

impl Default for OracleAdvancedOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 8,
            query_timeout_secs: 0,
            application_name: "FluxDB".into(),
            wallet_location: String::new(),
            wallet_password: SecretRef::default(),
        }
    }
}

impl OracleConnectionProfile {
    pub fn from_options(opts: &std::collections::BTreeMap<String, String>) -> Self {
        let mut profile = Self::default();
        if let Some(host) = opts.get("host") {
            profile.basic.host = host.clone();
        }
        if let Some(port) = opts.get("port").and_then(|value| value.parse().ok()) {
            profile.basic.port = port;
        }
        if let Some(kind) = opts
            .get("connect_kind")
            .and_then(|value| OracleConnectKind::parse(value))
        {
            profile.basic.connect_kind = kind;
        }
        if let Some(service) = opts.get("service_name") {
            profile.basic.service_name = service.clone();
        }
        if let Some(sid) = opts.get("sid") {
            profile.basic.sid = sid.clone();
        }
        if let Some(username) = opts.get("username") {
            profile.basic.username = username.clone();
        }
        if let Some(schema) = opts.get("schema") {
            profile.basic.schema = schema.clone();
        }
        if let Some(privilege) = opts
            .get("privilege")
            .and_then(|value| OraclePrivilege::parse(value))
        {
            profile.basic.privilege = privilege;
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
        if let Some(application_name) = opts.get("application_name") {
            profile.advanced.application_name = application_name.clone();
        }
        if let Some(location) = opts.get("wallet_location") {
            profile.advanced.wallet_location = location.clone();
        }
        if let Some(password) = opts.get("wallet_password") {
            profile.advanced.wallet_password = SecretRef::inline(password.clone());
        }
        profile.ssh = SshTunnelProfile::from_options(opts);
        profile
    }

    /// 回写扁平兼容参数（storage 会剥离其中的密码、Wallet 口令内联值）。
    pub fn into_options(&self) -> std::collections::BTreeMap<String, String> {
        let mut options = std::collections::BTreeMap::new();
        options.insert("host".into(), self.basic.host.clone());
        options.insert("port".into(), self.basic.port.to_string());
        options.insert(
            "connect_kind".into(),
            self.basic.connect_kind.as_str().to_string(),
        );
        options.insert("service_name".into(), self.basic.service_name.clone());
        options.insert("sid".into(), self.basic.sid.clone());
        options.insert("username".into(), self.basic.username.clone());
        options.insert("schema".into(), self.basic.schema.clone());
        options.insert(
            "privilege".into(),
            self.basic.privilege.as_str().to_string(),
        );
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
        options.insert(
            "application_name".into(),
            self.advanced.application_name.clone(),
        );
        options.insert(
            "wallet_location".into(),
            self.advanced.wallet_location.clone(),
        );
        if let Some(password) = self.advanced.wallet_password.value() {
            options.insert("wallet_password".into(), password.to_string());
        }
        options.extend(self.ssh.into_options());
        options
    }

    pub fn dial_endpoint(&self) -> (String, u16) {
        (self.basic.host.clone(), self.basic.port)
    }

    pub fn password(&self) -> Option<&str> {
        self.basic.password.value()
    }

    pub fn wallet_password(&self) -> Option<&str> {
        self.advanced.wallet_password.value()
    }

    /// 当前生效的寻址目标（Service Name 或 SID）。空字符串表示未填写。
    pub fn target_name(&self) -> &str {
        match self.basic.connect_kind {
            OracleConnectKind::ServiceName => self.basic.service_name.trim(),
            OracleConnectKind::Sid => self.basic.sid.trim(),
        }
    }

    /// 生成驱动 connect string。Service Name 走 Easy Connect；
    /// SID 走显式描述符，避免把 SID 误当服务名去监听器上查找。
    pub fn connect_string(&self) -> String {
        let host = self.basic.host.trim();
        let port = self.basic.port;
        match self.basic.connect_kind {
            OracleConnectKind::ServiceName => {
                format!("{host}:{port}/{}", self.basic.service_name.trim())
            }
            OracleConnectKind::Sid => format!(
                "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST={host})(PORT={port}))\
                 (CONNECT_DATA=(SID={})))",
                self.basic.sid.trim()
            ),
        }
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
        match self.basic.connect_kind {
            OracleConnectKind::ServiceName if self.basic.service_name.trim().is_empty() => {
                return Some("请填写 Service Name".into());
            }
            OracleConnectKind::Sid if self.basic.sid.trim().is_empty() => {
                return Some("请填写 SID".into());
            }
            _ => {}
        }
        if self.advanced.connect_timeout_secs > 86400
            || self.advanced.query_timeout_secs > 86400
        {
            return Some("高级超时值不能超过 86400 秒".into());
        }
        self.ssh.validate()
    }
}

#[cfg(test)]
mod oracle_profile_tests {
    use super::*;

    #[test]
    fn service_name_and_sid_produce_different_connect_strings() {
        let mut profile = OracleConnectionProfile::default();
        profile.basic.host = "db.example.com".into();
        profile.basic.service_name = "FREEPDB1".into();
        assert_eq!(
            profile.connect_string(),
            "db.example.com:1521/FREEPDB1"
        );

        profile.basic.connect_kind = OracleConnectKind::Sid;
        profile.basic.sid = "ORCL".into();
        let sid = profile.connect_string();
        assert!(sid.contains("(SID=ORCL)"), "{sid}");
        assert!(!sid.contains("FREEPDB1"), "{sid}");
    }

    #[test]
    fn validate_requires_the_active_target_only() {
        let mut profile = OracleConnectionProfile::default();
        profile.basic.host = "h".into();
        assert_eq!(profile.validate().as_deref(), Some("请填写 Service Name"));
        profile.basic.service_name = "svc".into();
        assert_eq!(profile.validate(), None);
        profile.basic.connect_kind = OracleConnectKind::Sid;
        assert_eq!(profile.validate().as_deref(), Some("请填写 SID"));
    }

    #[test]
    fn privilege_defaults_to_normal_and_never_inferred_from_username() {
        let profile = OracleConnectionProfile::from_options(&std::collections::BTreeMap::from([(
            "username".to_string(),
            "sys".to_string(),
        )]));
        assert_eq!(profile.basic.privilege, OraclePrivilege::Normal);
        assert_eq!(OraclePrivilege::parse("SYSDBA"), Some(OraclePrivilege::SysDba));
    }
}
