// 通用 SSH 隧道参数（MongoDB / SQL Server / Oracle / 达梦共用）。
//
// 与 MySQL / PostgreSQL / Redis 各自的历史 SSH 类型保持同一套扁平 `ssh_*` 键，
// 便于旧连接迁移与 storage 凭据槽复用；密码类（密码、私钥口令）是 SecretRef，
// 私钥只存文件路径，不落盘明文。

/// SSH 认证方式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SshAuth {
    #[default]
    Password,
    PrivateKey,
}

impl SshAuth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PrivateKey => "private_key",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "key" | "private_key" | "private-key" => Self::PrivateKey,
            _ => Self::Password,
        }
    }
}

/// SSH 跳板机与隧道行为参数。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SshTunnelProfile {
    /// 是否启用 SSH 隧道。
    pub enabled: bool,
    /// 跳板主机。
    pub host: String,
    /// 跳板端口（默认 22）。
    pub port: u16,
    /// 跳板用户名。
    pub username: String,
    /// 认证方式。
    pub auth: SshAuth,
    /// 密码（password 认证）；只存内存内联值，落盘前由 storage 剥离。
    pub password: SecretRef,
    /// 私钥文件路径（private_key 认证）。
    pub private_key: SecretRef,
    /// 私钥口令（可选）。
    pub passphrase: SecretRef,
    /// SSH 建连超时（秒）；0 表示用默认 5 秒。
    pub connect_timeout_secs: u32,
    /// SSH 心跳间隔（秒）；0 表示不发送。
    pub keepalive_interval_secs: u32,
}

impl Default for SshTunnelProfile {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 22,
            username: String::new(),
            auth: SshAuth::Password,
            password: SecretRef::default(),
            private_key: SecretRef::default(),
            passphrase: SecretRef::default(),
            connect_timeout_secs: 0,
            keepalive_interval_secs: 0,
        }
    }
}

impl SshTunnelProfile {
    /// 从扁平 `ssh_*` 参数还原（兼容旧连接与本仓库既有 MySQL 键名）。
    pub fn from_options(options: &std::collections::BTreeMap<String, String>) -> Self {
        let get = |key: &str| options.get(key).cloned();
        let password = get("ssh_password").unwrap_or_default();
        let passphrase = get("ssh_passphrase").unwrap_or_default();
        Self {
            enabled: connection_option_is_true(get("ssh_enabled").as_ref()),
            host: get("ssh_host").unwrap_or_default(),
            port: get("ssh_port")
                .and_then(|value| value.parse().ok())
                .unwrap_or(22),
            username: get("ssh_username").unwrap_or_default(),
            auth: SshAuth::parse(get("ssh_auth").as_deref().unwrap_or("")),
            password: if password.is_empty() {
                SecretRef::default()
            } else {
                SecretRef::inline(password)
            },
            private_key: SecretRef::ref_key(get("ssh_private_key_ref").unwrap_or_default()),
            passphrase: if passphrase.is_empty() {
                SecretRef::default()
            } else {
                SecretRef::inline(passphrase)
            },
            connect_timeout_secs: get("ssh_connect_timeout_secs")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            keepalive_interval_secs: get("ssh_keepalive_interval_secs")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        }
    }

    /// 回写成扁平 `ssh_*` 参数；未启用时不写，避免旧连接出现无意义的空键。
    pub fn into_options(&self) -> std::collections::BTreeMap<String, String> {
        let mut options = std::collections::BTreeMap::new();
        if !self.enabled {
            return options;
        }
        options.insert("ssh_enabled".into(), "true".into());
        options.insert("ssh_host".into(), self.host.clone());
        options.insert("ssh_port".into(), self.port.to_string());
        options.insert("ssh_username".into(), self.username.clone());
        options.insert("ssh_auth".into(), self.auth.as_str().into());
        if let Some(password) = self.password.value() {
            options.insert("ssh_password".into(), password.to_string());
        }
        if !self.private_key.key.is_empty() {
            options.insert("ssh_private_key_ref".into(), self.private_key.key.clone());
        }
        if let Some(passphrase) = self.passphrase.value() {
            options.insert("ssh_passphrase".into(), passphrase.to_string());
        }
        if self.connect_timeout_secs > 0 {
            options.insert(
                "ssh_connect_timeout_secs".into(),
                self.connect_timeout_secs.to_string(),
            );
        }
        if self.keepalive_interval_secs > 0 {
            options.insert(
                "ssh_keepalive_interval_secs".into(),
                self.keepalive_interval_secs.to_string(),
            );
        }
        options
    }

    /// 启用时校验必填项；错误文案与 MySQL/PostgreSQL 路径保持一致。
    pub fn validate(&self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        if self.host.trim().is_empty() {
            return Some("请填写 SSH 隧道主机".into());
        }
        if self.port == 0 {
            return Some("SSH 端口不合法".into());
        }
        if self.username.trim().is_empty() {
            return Some("请填写 SSH 用户名".into());
        }
        if self.auth == SshAuth::PrivateKey && self.private_key.key.trim().is_empty() {
            return Some("请填写 SSH 私钥文件路径".into());
        }
        None
    }
}

#[cfg(test)]
mod ssh_profile_tests {
    use super::*;

    #[test]
    fn options_roundtrip_keeps_flat_ssh_keys() {
        let mut options = std::collections::BTreeMap::new();
        options.insert("ssh_enabled".into(), "true".into());
        options.insert("ssh_host".into(), "jump.example.com".into());
        options.insert("ssh_port".into(), "2222".into());
        options.insert("ssh_username".into(), "ops".into());
        options.insert("ssh_auth".into(), "private_key".into());
        options.insert("ssh_private_key_ref".into(), "/tmp/id_ed25519".into());
        options.insert("ssh_passphrase".into(), "secret".into());

        let profile = SshTunnelProfile::from_options(&options);
        assert!(profile.enabled);
        assert_eq!(profile.host, "jump.example.com");
        assert_eq!(profile.port, 2222);
        assert_eq!(profile.auth, SshAuth::PrivateKey);
        assert_eq!(profile.validate(), None);

        let written = profile.into_options();
        assert_eq!(written.get("ssh_host").map(String::as_str), Some("jump.example.com"));
        assert_eq!(written.get("ssh_private_key_ref").map(String::as_str), Some("/tmp/id_ed25519"));
    }

    #[test]
    fn disabled_profile_never_blocks_and_writes_no_keys() {
        let profile = SshTunnelProfile::default();
        assert_eq!(profile.validate(), None);
        assert!(profile.into_options().is_empty());
    }

    #[test]
    fn enabled_profile_requires_jump_host_and_username() {
        let profile = SshTunnelProfile {
            enabled: true,
            ..SshTunnelProfile::default()
        };
        assert_eq!(profile.validate().as_deref(), Some("请填写 SSH 隧道主机"));
        let profile = SshTunnelProfile {
            host: "jump".into(),
            username: "ops".into(),
            ..profile
        };
        assert_eq!(profile.validate(), None);
    }
}
