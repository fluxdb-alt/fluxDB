#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DatabaseKind {
    MySql,
    TiDb,
    Sqlite,
    MongoDb,
    Redis,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct ConnectionGroupId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: ConnectionId,
    pub name: String,
    pub kind: DatabaseKind,
    pub endpoint: Endpoint,
    pub credential_ref: Option<String>,
    pub options: BTreeMap<String, String>,
    /// Redis 专用结构化连接档案（TLS / SSH / Sentinel / Cluster / Cloud / URI）。
    /// 仅为 Redis 连接填充；历史连接缺省为 `None`，兼容既有落盘数据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_profile: Option<RedisConnectionProfile>,
    /// MySQL/TiDB 专用结构化连接档案（TLS / SSH / 代理 / 超时）。
    /// 仅为 MySQL/TiDB 连接填充；历史连接缺省为 `None`，兼容既有落盘数据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mysql_profile: Option<MysqlConnectionProfile>,
}

impl ConnectionConfig {
    /// 是否存在 Redis 结构化档案。
    pub fn has_redis_profile(&self) -> bool {
        self.redis_profile.is_some()
    }

    /// 若携带 Redis 档案，则把档案归一化进扁平的 `endpoint` / `options`，
    /// 产出一份可直接拨号的配置；无档案时原样返回（兼容历史连接）。
    ///
    /// 归一化规则：档案是事实来源，其派生的扁平参数覆盖同名 `options` 键，
    /// 其余 `options`（如遗留的哨兵/集群参数）保留，保证旧连接升级后行为一致。
    pub fn redis_resolved(&self) -> Self {
        let Some(profile) = self.redis_profile.clone() else {
            return self.clone();
        };
        let mut config = self.clone();
        let (host, port) = profile.dial_endpoint();
        config.endpoint = Endpoint::Tcp {
            host,
            port,
            database: profile.basic.database.clone(),
        };
        for (key, value) in profile.into_options() {
            config.options.insert(key, value);
        }
        config
    }

    /// 是否存在 MySQL/TiDB 结构化档案。
    pub fn has_mysql_profile(&self) -> bool {
        self.mysql_profile.is_some()
    }

    /// 若携带 MySQL 档案，则把档案归一化进扁平的 `endpoint` / `options`，
    /// 产出一份可直接拨号的配置；无档案时原样返回（兼容历史连接）。
    ///
    /// 归一化规则同 [`ConnectionConfig::redis_resolved`]：档案是事实来源，
    /// 其派生的扁平参数覆盖同名 `options` 键，其余参数保留。
    pub fn mysql_resolved(&self) -> Self {
        let Some(profile) = self.mysql_profile.clone() else {
            return self.clone();
        };
        let mut config = self.clone();
        let (host, port) = profile.dial_endpoint();
        config.endpoint = Endpoint::Tcp {
            host,
            port,
            database: Some(profile.basic.database.clone()),
        };
        for (key, value) in profile.into_options() {
            config.options.insert(key, value);
        }
        config
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectionGroup {
    pub id: ConnectionGroupId,
    pub name: String,
    pub collapsed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SidebarOrderEntry {
    #[serde(rename = "connection")]
    Connection { id: ConnectionId },
    #[serde(rename = "group")]
    Group {
        id: ConnectionGroupId,
        connection_ids: Vec<ConnectionId>,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarLayout {
    pub groups: Vec<ConnectionGroup>,
    pub order: Vec<SidebarOrderEntry>,
    #[serde(default)]
    pub table_folders: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub table_folder_assignments: BTreeMap<String, (String, String)>,
}

impl SidebarLayout {
    pub fn for_connections(connections: &[ConnectionConfig]) -> Self {
        Self {
            groups: Vec::new(),
            order: connections
                .iter()
                .map(|connection| SidebarOrderEntry::Connection { id: connection.id })
                .collect(),
            table_folders: BTreeMap::new(),
            table_folder_assignments: BTreeMap::new(),
        }
    }

    pub fn repair(&mut self, connections: &[ConnectionConfig]) {
        let mut valid_connection_ids: Vec<ConnectionId> =
            connections.iter().map(|connection| connection.id).collect();
        valid_connection_ids.sort();
        valid_connection_ids.dedup();

        self.groups.retain(|group| {
            self.order
                .iter()
                .any(|entry| entry.group_id() == Some(group.id))
        });
        let valid_group_ids: Vec<ConnectionGroupId> =
            self.groups.iter().map(|group| group.id).collect();

        let mut seen_connections = Vec::new();
        let mut repaired_order = Vec::new();
        for entry in self.order.drain(..) {
            match entry {
                SidebarOrderEntry::Connection { id } => {
                    if valid_connection_ids.contains(&id) && !seen_connections.contains(&id) {
                        seen_connections.push(id);
                        repaired_order.push(SidebarOrderEntry::Connection { id });
                    }
                }
                SidebarOrderEntry::Group { id, connection_ids } => {
                    if !valid_group_ids.contains(&id) {
                        continue;
                    }

                    let mut repaired_children = Vec::new();
                    for connection_id in connection_ids {
                        if valid_connection_ids.contains(&connection_id)
                            && !seen_connections.contains(&connection_id)
                        {
                            seen_connections.push(connection_id);
                            repaired_children.push(connection_id);
                        }
                    }
                    repaired_order.push(SidebarOrderEntry::Group {
                        id,
                        connection_ids: repaired_children,
                    });
                }
            }
        }

        for connection_id in valid_connection_ids {
            if !seen_connections.contains(&connection_id) {
                repaired_order.push(SidebarOrderEntry::Connection { id: connection_id });
            }
        }

        self.order = repaired_order;
        self.groups.retain(|group| {
            self.order
                .iter()
                .any(|entry| entry.group_id() == Some(group.id))
        });
    }

    pub fn is_connection_grouped(&self, connection_id: ConnectionId) -> bool {
        self.connection_group(connection_id).is_some()
    }

    pub fn connection_group(&self, connection_id: ConnectionId) -> Option<ConnectionGroupId> {
        self.order.iter().find_map(|entry| match entry {
            SidebarOrderEntry::Group { id, connection_ids }
                if connection_ids.contains(&connection_id) =>
            {
                Some(*id)
            }
            _ => None,
        })
    }

    pub fn add_group(&mut self, group: ConnectionGroup) {
        if self.groups.iter().any(|existing| existing.id == group.id) {
            return;
        }
        let id = group.id;
        self.groups.push(group);
        self.order.push(SidebarOrderEntry::Group {
            id,
            connection_ids: Vec::new(),
        });
    }

    pub fn move_connection_to_group(
        &mut self,
        connection_id: ConnectionId,
        group_id: ConnectionGroupId,
    ) {
        self.remove_connection(connection_id);
        if let Some(SidebarOrderEntry::Group { connection_ids, .. }) = self
            .order
            .iter_mut()
            .find(|entry| entry.group_id() == Some(group_id))
        {
            connection_ids.push(connection_id);
        } else {
            self.order
                .push(SidebarOrderEntry::Connection { id: connection_id });
        }
    }

    pub fn move_connection_to_group_after(
        &mut self,
        connection_id: ConnectionId,
        group_id: ConnectionGroupId,
        after_connection_id: Option<ConnectionId>,
    ) {
        if after_connection_id == Some(connection_id) {
            return;
        }

        self.remove_connection(connection_id);
        if let Some(SidebarOrderEntry::Group { connection_ids, .. }) = self
            .order
            .iter_mut()
            .find(|entry| entry.group_id() == Some(group_id))
        {
            if let Some(after_connection_id) = after_connection_id {
                if let Some(index) = connection_ids
                    .iter()
                    .position(|id| *id == after_connection_id)
                {
                    connection_ids.insert(index + 1, connection_id);
                    return;
                }
            }
            connection_ids.push(connection_id);
        } else {
            self.order
                .push(SidebarOrderEntry::Connection { id: connection_id });
        }
    }

    pub fn move_connection_to_top_level(&mut self, connection_id: ConnectionId) {
        self.remove_connection(connection_id);
        self.order
            .push(SidebarOrderEntry::Connection { id: connection_id });
    }

    pub fn move_connection_to_top_level_after(
        &mut self,
        connection_id: ConnectionId,
        after_connection_id: Option<ConnectionId>,
    ) {
        if after_connection_id == Some(connection_id) {
            return;
        }

        self.remove_connection(connection_id);
        let entry = SidebarOrderEntry::Connection { id: connection_id };
        if let Some(after_connection_id) = after_connection_id {
            if let Some(index) = self.order.iter().position(|entry| {
                matches!(entry, SidebarOrderEntry::Connection { id } if *id == after_connection_id)
            }) {
                self.order.insert(index + 1, entry);
                return;
            }
        }
        self.order.push(entry);
    }

    pub fn delete_group(&mut self, group_id: ConnectionGroupId) {
        self.groups.retain(|group| group.id != group_id);
        let mut children = Vec::new();
        self.order.retain(|entry| match entry {
            SidebarOrderEntry::Group { id, connection_ids } if *id == group_id => {
                children.extend(connection_ids.iter().copied());
                false
            }
            _ => true,
        });
        self.order.extend(
            children
                .into_iter()
                .map(|id| SidebarOrderEntry::Connection { id }),
        );
    }

    fn remove_connection(&mut self, connection_id: ConnectionId) {
        self.order.retain(|entry| match entry {
            SidebarOrderEntry::Connection { id } => *id != connection_id,
            SidebarOrderEntry::Group { .. } => true,
        });
        for entry in &mut self.order {
            if let SidebarOrderEntry::Group { connection_ids, .. } = entry {
                connection_ids.retain(|id| *id != connection_id);
            }
        }
    }
}

impl SidebarOrderEntry {
    pub fn group_id(&self) -> Option<ConnectionGroupId> {
        match self {
            SidebarOrderEntry::Group { id, .. } => Some(*id),
            SidebarOrderEntry::Connection { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectionDraft {
    pub name: String,
    pub kind: DatabaseKind,
    pub endpoint: Endpoint,
    pub credential_ref: Option<String>,
    pub options: BTreeMap<String, String>,
    /// Redis 专用结构化连接档案；见 [`ConnectionConfig::redis_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_profile: Option<RedisConnectionProfile>,
    /// MySQL/TiDB 专用结构化连接档案；见 [`ConnectionConfig::mysql_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mysql_profile: Option<MysqlConnectionProfile>,
}

impl ConnectionDraft {
    pub fn into_config(self, id: ConnectionId) -> ConnectionConfig {
        ConnectionConfig {
            id,
            name: self.name,
            kind: self.kind,
            endpoint: self.endpoint,
            credential_ref: self.credential_ref,
            options: self.options,
            redis_profile: self.redis_profile,
            mysql_profile: self.mysql_profile,
        }
    }
}

pub const SQLITE_ATTACHED_DATABASE_OPTION_PREFIX: &str = "sqlite.attach.";

pub fn sqlite_attached_database_option_key(database: &str) -> String {
    format!("{SQLITE_ATTACHED_DATABASE_OPTION_PREFIX}{database}")
}

pub fn sqlite_attached_databases(config: &ConnectionConfig) -> Vec<(String, PathBuf)> {
    config
        .options
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix(SQLITE_ATTACHED_DATABASE_OPTION_PREFIX)
                .filter(|database| !database.is_empty() && !value.is_empty())
                .map(|database| (database.to_string(), PathBuf::from(value)))
        })
        .collect()
}

pub fn sqlite_attached_database_path(
    config: &ConnectionConfig,
    database: &str,
) -> Option<PathBuf> {
    sqlite_attached_databases(config)
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(database))
        .map(|(_, path)| path)
}

pub fn set_sqlite_attached_database(
    config: &mut ConnectionConfig,
    database: &str,
    path: PathBuf,
) {
    config.options.insert(
        sqlite_attached_database_option_key(database),
        path.to_string_lossy().to_string(),
    );
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Endpoint {
    Tcp {
        host: String,
        port: u16,
        database: Option<String>,
    },
    SqliteFile {
        path: PathBuf,
        read_only: bool,
    },
    Uri {
        uri: String,
    },
}

/// 单个 CPU 采样：Redis `INFO cpu` / `INFO server` 中的累计秒数。
/// CPU 百分比需要两次采样做增量计算（参考 RedisInsight），原始值先由连接采集，
/// 百分比再在 app 层根据基线推导。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CpuStats {
    pub sys_seconds: f64,
    pub user_seconds: f64,
    pub uptime_seconds: f64,
}

/// 连接级运行概览（连接状态摘要，参考 RedisInsight 数据库概览语义）。
/// 目前只承载 Redis 的心跳指标：版本、内存占用、CPU 采样。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectionOverview {
    /// Redis 服务端版本，如 `7.2.4`。
    pub version: String,
    /// `INFO memory used_memory`：Redis 实例占用内存字节数。
    pub used_memory_bytes: u64,
    /// 最近一次 CPU 采样（`INFO cpu` + `INFO server`），用于 app 层推导使用率。
    pub cpu: Option<CpuStats>,
}
