// 统一连接档案的行为边界。
//
// 背景：各数据库的结构化连接档案（MySQL/PostgreSQL/Redis/达梦/SQL Server/
// MongoDB/Oracle）字段布局不同，但存储、校验、迁移对它们做的事是同一套：
// 校验参数、回写扁平 options、给出拨号端点、列出需要加密落盘的密码槽位。
// 此前这些逻辑按数据库类型写了 7 份平行分支，新增或修改时容易漏。
//
// 本 trait 只收敛**行为**，不强行统一字段布局：各档案保留自己的字段，
// 数据库特有的参数（Redis 哨兵/集群、Oracle SID/Service 等）仍由各类型承载。
// 调用方（存储、后续 UI 表单）只依赖 `dyn DatabaseConnection`。

/// 需要写入加密凭据表的受控槽位类别。
///
/// 这里只描述语义，具体凭据表键名由存储层映射，避免 core 依赖存储层常量。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ConnectionSecretSlot {
    /// 数据库登录密码。
    DatabasePassword,
    /// SSH 跳板机密码。
    SshPassword,
    /// SSH 私钥口令。
    SshPassphrase,
    /// 代理认证密码。
    ProxyPassword,
    /// TLS 客户端私钥口令（加密私钥时使用）。
    TlsClientKeyPassphrase,
    /// Oracle Wallet 口令。
    OracleWalletPassword,
}

/// 结构化连接档案的统一行为。
///
/// 实现方只需把已有的 `validate` / `into_options` / `dial_endpoint` 与
/// 密码槽位遍历接到本 trait；不要为满足 trait 改变序列化结构。
pub trait DatabaseConnection: Send + Sync {
    /// 参数是否合法；`None` 表示通过。
    fn validate(&self) -> Option<String>;

    /// 回写成扁平 `options`，供历史消费方与连接器归一化使用。
    fn to_options(&self) -> std::collections::BTreeMap<String, String>;

    /// 档案声明的拨号端点（未被 SSH 隧道改写前）。
    fn dial_endpoint(&self) -> (String, u16);

    /// 需要加密落盘的密码类槽位。
    ///
    /// 证书/私钥路径属于文件引用，不是密码语义，不在此列。
    fn secret_slots(&self) -> Vec<(ConnectionSecretSlot, &SecretRef)>;

    /// [`Self::secret_slots`] 的可变版本，供回填/剥离时原地改写。
    fn secret_slots_mut(&mut self) -> Vec<(ConnectionSecretSlot, &mut SecretRef)>;
}

impl DatabaseConnection for MysqlConnectionProfile {
    fn validate(&self) -> Option<String> {
        MysqlConnectionProfile::validate(self)
    }

    fn to_options(&self) -> std::collections::BTreeMap<String, String> {
        MysqlConnectionProfile::into_options(self)
    }

    fn dial_endpoint(&self) -> (String, u16) {
        MysqlConnectionProfile::dial_endpoint(self)
    }

    fn secret_slots(&self) -> Vec<(ConnectionSecretSlot, &SecretRef)> {
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &self.basic.password,
        )];
        for layer in &self.transport {
            match layer {
                MysqlTransportLayer::Ssh(ssh) if ssh.enabled => {
                    slots.push((ConnectionSecretSlot::SshPassword, &ssh.password));
                    slots.push((ConnectionSecretSlot::SshPassphrase, &ssh.passphrase));
                }
                MysqlTransportLayer::Proxy(proxy) if proxy.enabled => {
                    slots.push((ConnectionSecretSlot::ProxyPassword, &proxy.password));
                }
                _ => {}
            }
        }
        slots
    }

    fn secret_slots_mut(&mut self) -> Vec<(ConnectionSecretSlot, &mut SecretRef)> {
        let MysqlConnectionProfile {
            basic, transport, ..
        } = self;
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &mut basic.password,
        )];
        for layer in transport.iter_mut() {
            match layer {
                MysqlTransportLayer::Ssh(ssh) if ssh.enabled => {
                    slots.push((ConnectionSecretSlot::SshPassword, &mut ssh.password));
                    slots.push((ConnectionSecretSlot::SshPassphrase, &mut ssh.passphrase));
                }
                MysqlTransportLayer::Proxy(proxy) if proxy.enabled => {
                    slots.push((ConnectionSecretSlot::ProxyPassword, &mut proxy.password));
                }
                _ => {}
            }
        }
        slots
    }
}

impl DatabaseConnection for PostgresConnectionProfile {
    fn validate(&self) -> Option<String> {
        PostgresConnectionProfile::validate(self)
    }

    fn to_options(&self) -> std::collections::BTreeMap<String, String> {
        PostgresConnectionProfile::into_options(self)
    }

    fn dial_endpoint(&self) -> (String, u16) {
        PostgresConnectionProfile::dial_endpoint(self)
    }

    fn secret_slots(&self) -> Vec<(ConnectionSecretSlot, &SecretRef)> {
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &self.basic.password,
        )];
        if self.tls.enabled && !self.tls.client_key.key.is_empty() {
            slots.push((
                ConnectionSecretSlot::TlsClientKeyPassphrase,
                &self.tls.client_key_passphrase,
            ));
        }
        for layer in &self.transport {
            match layer {
                PostgresTransportLayer::Ssh(ssh) if ssh.enabled => {
                    slots.push((ConnectionSecretSlot::SshPassword, &ssh.password));
                    slots.push((ConnectionSecretSlot::SshPassphrase, &ssh.passphrase));
                }
                PostgresTransportLayer::Proxy(proxy) if proxy.enabled => {
                    slots.push((ConnectionSecretSlot::ProxyPassword, &proxy.password));
                }
                _ => {}
            }
        }
        slots
    }

    fn secret_slots_mut(&mut self) -> Vec<(ConnectionSecretSlot, &mut SecretRef)> {
        let PostgresConnectionProfile {
            basic, tls, transport, ..
        } = self;
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &mut basic.password,
        )];
        if tls.enabled && !tls.client_key.key.is_empty() {
            slots.push((
                ConnectionSecretSlot::TlsClientKeyPassphrase,
                &mut tls.client_key_passphrase,
            ));
        }
        for layer in transport.iter_mut() {
            match layer {
                PostgresTransportLayer::Ssh(ssh) if ssh.enabled => {
                    slots.push((ConnectionSecretSlot::SshPassword, &mut ssh.password));
                    slots.push((ConnectionSecretSlot::SshPassphrase, &mut ssh.passphrase));
                }
                PostgresTransportLayer::Proxy(proxy) if proxy.enabled => {
                    slots.push((ConnectionSecretSlot::ProxyPassword, &mut proxy.password));
                }
                _ => {}
            }
        }
        slots
    }
}

impl DatabaseConnection for RedisConnectionProfile {
    fn validate(&self) -> Option<String> {
        RedisConnectionProfile::validate(self)
    }

    fn to_options(&self) -> std::collections::BTreeMap<String, String> {
        RedisConnectionProfile::into_options(self)
    }

    fn dial_endpoint(&self) -> (String, u16) {
        RedisConnectionProfile::dial_endpoint(self)
    }

    fn secret_slots(&self) -> Vec<(ConnectionSecretSlot, &SecretRef)> {
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &self.basic.password,
        )];
        if self.ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &self.ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &self.ssh.passphrase));
        }
        slots
    }

    fn secret_slots_mut(&mut self) -> Vec<(ConnectionSecretSlot, &mut SecretRef)> {
        let RedisConnectionProfile { basic, ssh, .. } = self;
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &mut basic.password,
        )];
        if ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &mut ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &mut ssh.passphrase));
        }
        slots
    }
}

impl DatabaseConnection for DamengConnectionProfile {
    fn validate(&self) -> Option<String> {
        DamengConnectionProfile::validate(self)
    }

    fn to_options(&self) -> std::collections::BTreeMap<String, String> {
        DamengConnectionProfile::into_options(self)
    }

    fn dial_endpoint(&self) -> (String, u16) {
        DamengConnectionProfile::dial_endpoint(self)
    }

    fn secret_slots(&self) -> Vec<(ConnectionSecretSlot, &SecretRef)> {
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &self.basic.password,
        )];
        if self.ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &self.ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &self.ssh.passphrase));
        }
        slots
    }

    fn secret_slots_mut(&mut self) -> Vec<(ConnectionSecretSlot, &mut SecretRef)> {
        let DamengConnectionProfile { basic, ssh, .. } = self;
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &mut basic.password,
        )];
        if ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &mut ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &mut ssh.passphrase));
        }
        slots
    }
}

impl DatabaseConnection for SqlServerConnectionProfile {
    fn validate(&self) -> Option<String> {
        SqlServerConnectionProfile::validate(self)
    }

    fn to_options(&self) -> std::collections::BTreeMap<String, String> {
        SqlServerConnectionProfile::into_options(self)
    }

    fn dial_endpoint(&self) -> (String, u16) {
        SqlServerConnectionProfile::dial_endpoint(self)
    }

    fn secret_slots(&self) -> Vec<(ConnectionSecretSlot, &SecretRef)> {
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &self.basic.password,
        )];
        if self.ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &self.ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &self.ssh.passphrase));
        }
        slots
    }

    fn secret_slots_mut(&mut self) -> Vec<(ConnectionSecretSlot, &mut SecretRef)> {
        let SqlServerConnectionProfile { basic, ssh, .. } = self;
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &mut basic.password,
        )];
        if ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &mut ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &mut ssh.passphrase));
        }
        slots
    }
}

impl DatabaseConnection for MongoConnectionProfile {
    fn validate(&self) -> Option<String> {
        MongoConnectionProfile::validate(self)
    }

    fn to_options(&self) -> std::collections::BTreeMap<String, String> {
        MongoConnectionProfile::into_options(self)
    }

    fn dial_endpoint(&self) -> (String, u16) {
        MongoConnectionProfile::dial_endpoint(self)
    }

    fn secret_slots(&self) -> Vec<(ConnectionSecretSlot, &SecretRef)> {
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &self.basic.password,
        )];
        if self.ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &self.ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &self.ssh.passphrase));
        }
        slots
    }

    fn secret_slots_mut(&mut self) -> Vec<(ConnectionSecretSlot, &mut SecretRef)> {
        let MongoConnectionProfile { basic, ssh, .. } = self;
        let mut slots = vec![(
            ConnectionSecretSlot::DatabasePassword,
            &mut basic.password,
        )];
        if ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &mut ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &mut ssh.passphrase));
        }
        slots
    }
}

impl DatabaseConnection for OracleConnectionProfile {
    fn validate(&self) -> Option<String> {
        OracleConnectionProfile::validate(self)
    }

    fn to_options(&self) -> std::collections::BTreeMap<String, String> {
        OracleConnectionProfile::into_options(self)
    }

    fn dial_endpoint(&self) -> (String, u16) {
        OracleConnectionProfile::dial_endpoint(self)
    }

    fn secret_slots(&self) -> Vec<(ConnectionSecretSlot, &SecretRef)> {
        let mut slots = vec![
            (
                ConnectionSecretSlot::DatabasePassword,
                &self.basic.password,
            ),
            (
                ConnectionSecretSlot::OracleWalletPassword,
                &self.advanced.wallet_password,
            ),
        ];
        if self.ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &self.ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &self.ssh.passphrase));
        }
        slots
    }

    fn secret_slots_mut(&mut self) -> Vec<(ConnectionSecretSlot, &mut SecretRef)> {
        let OracleConnectionProfile {
            basic,
            advanced,
            ssh,
            ..
        } = self;
        let mut slots = vec![
            (
                ConnectionSecretSlot::DatabasePassword,
                &mut basic.password,
            ),
            (
                ConnectionSecretSlot::OracleWalletPassword,
                &mut advanced.wallet_password,
            ),
        ];
        if ssh.enabled {
            slots.push((ConnectionSecretSlot::SshPassword, &mut ssh.password));
            slots.push((ConnectionSecretSlot::SshPassphrase, &mut ssh.passphrase));
        }
        slots
    }
}

#[cfg(test)]
mod database_connection_tests {
    use super::*;

    /// MySQL 的 SSH 与代理密码必须都被识别为受控槽位，避免漏存或明文落盘。
    #[test]
    fn mysql_profile_reports_transport_secret_slots() {
        let profile = MysqlConnectionProfile {
            transport: vec![
                MysqlTransportLayer::Ssh(MysqlSshOptions {
                    enabled: true,
                    password: SecretRef::inline("ssh-pass"),
                    passphrase: SecretRef::inline("ssh-passphrase"),
                    ..Default::default()
                }),
                MysqlTransportLayer::Proxy(MysqlProxy {
                    enabled: true,
                    password: SecretRef::inline("proxy-pass"),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        };

        let slots = DatabaseConnection::secret_slots(&profile);
        let kinds: Vec<ConnectionSecretSlot> = slots.iter().map(|(kind, _)| *kind).collect();

        assert_eq!(
            kinds,
            vec![
                ConnectionSecretSlot::DatabasePassword,
                ConnectionSecretSlot::SshPassword,
                ConnectionSecretSlot::SshPassphrase,
                ConnectionSecretSlot::ProxyPassword,
            ]
        );
        assert_eq!(
            slots[3].1.value(),
            Some("proxy-pass"),
            "代理密码必须取到内联值"
        );
    }
}

impl ConnectionConfig {
    /// 当前连接的结构化档案，按数据库类型取；SQLite 与历史扁平连接返回 `None`。
    pub fn connection_profile(&self) -> Option<&dyn DatabaseConnection> {
        match self.kind {
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => self
                .mysql_profile
                .as_ref()
                .map(|profile| profile as &dyn DatabaseConnection),
            DatabaseKind::Postgres | DatabaseKind::Kingbase => self
                .postgres_profile
                .as_ref()
                .map(|profile| profile as &dyn DatabaseConnection),
            DatabaseKind::Redis => self
                .redis_profile
                .as_ref()
                .map(|profile| profile as &dyn DatabaseConnection),
            DatabaseKind::Dameng => self
                .dameng_profile
                .as_ref()
                .map(|profile| profile as &dyn DatabaseConnection),
            DatabaseKind::SqlServer => self
                .sqlserver_profile
                .as_ref()
                .map(|profile| profile as &dyn DatabaseConnection),
            DatabaseKind::MongoDb => self
                .mongo_profile
                .as_ref()
                .map(|profile| profile as &dyn DatabaseConnection),
            DatabaseKind::Oracle => self
                .oracle_profile
                .as_ref()
                .map(|profile| profile as &dyn DatabaseConnection),
            DatabaseKind::Sqlite => None,
        }
    }

    /// [`Self::connection_profile`] 的可变版本。
    pub fn connection_profile_mut(&mut self) -> Option<&mut dyn DatabaseConnection> {
        match self.kind {
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => self
                .mysql_profile
                .as_mut()
                .map(|profile| profile as &mut dyn DatabaseConnection),
            DatabaseKind::Postgres | DatabaseKind::Kingbase => self
                .postgres_profile
                .as_mut()
                .map(|profile| profile as &mut dyn DatabaseConnection),
            DatabaseKind::Redis => self
                .redis_profile
                .as_mut()
                .map(|profile| profile as &mut dyn DatabaseConnection),
            DatabaseKind::Dameng => self
                .dameng_profile
                .as_mut()
                .map(|profile| profile as &mut dyn DatabaseConnection),
            DatabaseKind::SqlServer => self
                .sqlserver_profile
                .as_mut()
                .map(|profile| profile as &mut dyn DatabaseConnection),
            DatabaseKind::MongoDb => self
                .mongo_profile
                .as_mut()
                .map(|profile| profile as &mut dyn DatabaseConnection),
            DatabaseKind::Oracle => self
                .oracle_profile
                .as_mut()
                .map(|profile| profile as &mut dyn DatabaseConnection),
            DatabaseKind::Sqlite => None,
        }
    }
}
