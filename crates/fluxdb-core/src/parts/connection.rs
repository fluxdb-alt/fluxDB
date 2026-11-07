#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DatabaseKind {
    MySql,
    TiDb,
    /// MariaDB：与 MySQL 共用 wire protocol、反引号方言与 SQLx 连接器，
    /// 但在客户端工具、系统库与服务端能力上独立识别。
    MariaDb,
    Sqlite,
    MongoDb,
    Redis,
    Postgres,
    /// 人大金仓 KingbaseES：在 PG 兼容模式下复用 PostgreSQL wire protocol、
    /// `pg_catalog` 与双引号方言，连接器、方言与补全按 PostgreSQL 家族处理。
    /// Oracle 兼容模式下的行为差异不在支持范围内。
    Kingbase,
    Dameng,
    SqlServer,
    Oracle,
}

impl DatabaseKind {
    /// 是否属于 MySQL 协议家族（MySQL / TiDB / MariaDB）。
    ///
    /// 这三者共享 wire protocol、反引号标识符与 `LIMIT/OFFSET` 分页；
    /// 连接器、方言、补全与备份链路应统一用它判断，避免新增分支时漏掉 MariaDB。
    pub fn is_mysql_family(self) -> bool {
        matches!(
            self,
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb
        )
    }

    /// 是否属于 PostgreSQL 协议家族（PostgreSQL / KingbaseES PG 兼容模式）。
    ///
    /// 二者共享 wire protocol、`pg_catalog` 与双引号标识符；新增与 PG 相关的
    /// 方言/补全/过滤分支时应统一用它判断，避免只写 `DatabaseKind::Postgres`
    /// 把 Kingbase 漏在支持范围外。
    pub fn is_postgres_family(self) -> bool {
        matches!(self, DatabaseKind::Postgres | DatabaseKind::Kingbase)
    }

    /// 是否支持「按块追加写入」大字段（上传替换的流式路径）。
    ///
    /// 只有实现了逐块追加的连接器才返回 true。Oracle/达梦暂时为 false；
    /// Kingbase 也未在真实实例上验证逐块写入，先按 false 让上层明确拒绝超阈值上传。
    /// 它们的真流式路径依赖 LOB 定位符与驱动侧写接口，尚未在本仓库验证过，
    /// 与其给出一个可能写坏数据的路径，不如让上层明确拒绝超阈值上传。
    ///
    /// 判定必须在上传暂存**之前**使用，避免先落几百 MiB 再在提交时失败。
    pub fn supports_streamed_lob_write(self) -> bool {
        matches!(
            self,
            DatabaseKind::MySql
                | DatabaseKind::TiDb
                | DatabaseKind::MariaDb
                | DatabaseKind::Sqlite
                | DatabaseKind::Postgres
                | DatabaseKind::SqlServer
        )
    }
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
    pub options: BTreeMap<String, String>,
    /// Redis 专用结构化连接档案（TLS / SSH / Sentinel / Cluster / Cloud / URI）。
    /// 仅为 Redis 连接填充；历史连接缺省为 `None`，兼容既有落盘数据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_profile: Option<RedisConnectionProfile>,
    /// MySQL / MariaDB / TiDB 专用结构化连接档案（TLS / SSH / 代理 / 超时）。
    /// 仅为 MySQL 家族连接填充；历史连接缺省为 `None`，兼容既有落盘数据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mysql_profile: Option<MysqlConnectionProfile>,
    /// PostgreSQL 专用结构化连接档案（TLS / SSH / 代理 / 作用域 / 超时）。
    /// 仅为 Postgres 连接填充；历史连接缺省为 `None`，兼容既有落盘数据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgres_profile: Option<PostgresConnectionProfile>,
    /// 达梦 DM8 专用结构化连接档案；历史连接缺省为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dameng_profile: Option<DamengConnectionProfile>,
    /// SQL Server 专用结构化连接档案；历史连接缺省为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlserver_profile: Option<SqlServerConnectionProfile>,
    /// MongoDB 专用结构化连接档案（种子列表 / SRV / authSource / TLS）；历史连接缺省为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mongo_profile: Option<MongoConnectionProfile>,
    /// Oracle 专用结构化连接档案；历史连接缺省为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_profile: Option<OracleConnectionProfile>,
}

impl ConnectionConfig {
    /// 连接是否被标记为只读：SQLite 走 endpoint 标记，其它类型走显式 `options["read_only"]`。
    ///
    /// 连接器与业务层共用这一判定，避免各层对只读的定义漂移。
    pub fn is_read_only(&self) -> bool {
        matches!(self.endpoint, Endpoint::SqliteFile { read_only: true, .. })
            || self
                .options
                .get("read_only")
                .is_some_and(|value| value == "true")
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

    /// 若携带 PostgreSQL 档案，则把档案归一化进扁平的 `endpoint` / `options`，
    /// 产出一份可直接拨号的配置；无档案时原样返回（兼容历史连接）。
    ///
    /// 归一化规则同 [`ConnectionConfig::mysql_resolved`]：档案是事实来源，
    /// 其派生的扁平参数覆盖同名 `options` 键，其余参数保留。
    pub fn postgres_resolved(&self) -> Self {
        let Some(profile) = self.postgres_profile.clone() else {
            return self.clone();
        };
        let mut config = self.clone();
        let (host, port) = profile.dial_endpoint();
        config.endpoint = Endpoint::Tcp {
            host,
            port,
            database: Some(profile.maintenance_database().to_string()),
        };
        for (key, value) in profile.into_options() {
            config.options.insert(key, value);
        }
        config
    }

    /// 若携带达梦档案，则把档案归一化进 `endpoint` / `options`。
    pub fn dameng_resolved(&self) -> Self {
        let Some(profile) = self.dameng_profile.clone() else {
            return self.clone();
        };
        let mut config = self.clone();
        let (host, port) = profile.dial_endpoint();
        config.endpoint = Endpoint::Tcp {
            host,
            port,
            database: non_empty_string(&profile.basic.database),
        };
        config.options.extend(profile.into_options());
        config
    }

    /// 若携带 SQL Server 档案，则把档案归一化进 `endpoint` / `options`。
    pub fn sqlserver_resolved(&self) -> Self {
        let Some(profile) = self.sqlserver_profile.clone() else {
            return self.clone();
        };
        let mut config = self.clone();
        let (host, port) = profile.dial_endpoint();
        config.endpoint = Endpoint::Tcp {
            host,
            port,
            database: non_empty_string(&profile.basic.database),
        };
        config.options.extend(profile.into_options());
        config
    }

    /// 若携带 Oracle 档案，则把档案归一化进 `endpoint` / `options`。
    ///
    /// `endpoint.database` 记录当前生效的 Service Name 或 SID，供对象树与补全复用；
    /// 具体是哪一种由 `options["connect_kind"]` 区分。
    pub fn oracle_resolved(&self) -> Self {
        let Some(profile) = self.oracle_profile.clone() else {
            return self.clone();
        };
        let mut config = self.clone();
        let (host, port) = profile.dial_endpoint();
        config.endpoint = Endpoint::Tcp {
            host,
            port,
            database: non_empty_string(profile.target_name()),
        };
        config.options.extend(profile.into_options());
        config
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
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
    pub options: BTreeMap<String, String>,
    /// Redis 专用结构化连接档案；见 [`ConnectionConfig::redis_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_profile: Option<RedisConnectionProfile>,
    /// MySQL/TiDB 专用结构化连接档案；见 [`ConnectionConfig::mysql_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mysql_profile: Option<MysqlConnectionProfile>,
    /// PostgreSQL 专用结构化连接档案；见 [`ConnectionConfig::postgres_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgres_profile: Option<PostgresConnectionProfile>,
    /// 达梦 DM8 专用结构化连接档案；见 [`ConnectionConfig::dameng_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dameng_profile: Option<DamengConnectionProfile>,
    /// SQL Server 专用结构化连接档案；见 [`ConnectionConfig::sqlserver_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlserver_profile: Option<SqlServerConnectionProfile>,
    /// MongoDB 专用结构化连接档案；见 [`ConnectionConfig::mongo_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mongo_profile: Option<MongoConnectionProfile>,
    /// Oracle 专用结构化连接档案；见 [`ConnectionConfig::oracle_profile`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_profile: Option<OracleConnectionProfile>,
}

impl ConnectionDraft {
    pub fn into_config(self, id: ConnectionId) -> ConnectionConfig {
        ConnectionConfig {
            id,
            name: self.name,
            kind: self.kind,
            endpoint: self.endpoint,
            options: self.options,
            redis_profile: self.redis_profile,
            mysql_profile: self.mysql_profile,
            postgres_profile: self.postgres_profile,
            dameng_profile: self.dameng_profile,
            sqlserver_profile: self.sqlserver_profile,
            mongo_profile: self.mongo_profile,
            oracle_profile: self.oracle_profile,
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

/// 解析连接选项中的布尔开关；缺失值与未识别文本均视为关闭。
fn connection_option_is_true(value: Option<&String>) -> bool {
    value
        .map(|v| v.trim().to_ascii_lowercase())
        .is_some_and(|v| matches!(v.as_str(), "true" | "y" | "yes" | "1" | "on"))
}

