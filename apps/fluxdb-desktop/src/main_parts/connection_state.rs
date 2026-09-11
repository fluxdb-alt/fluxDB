#[derive(Clone, Debug)]
struct DraggedConnection {
    connection_id: ConnectionId,
    name: String,
}

impl Render for DraggedConnection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        drag_preview(self.name.clone(), ComponentTheme::global(cx).radius_lg)
    }
}

fn drag_preview(label: String, radius: gpui::Pixels) -> impl IntoElement {
    div()
        .h(px(30.))
        .px_3()
        .rounded(radius)
        .border_1()
        .border_color(rgb(0xf5a400))
        .bg(rgb(0x232832))
        .shadow(vec![box_shadow(
            px(0.),
            px(10.),
            px(24.),
            px(0.),
            hsla(0., 0., 0., 0.22),
        )])
        .flex()
        .items_center()
        .text_size(px(13.))
        .text_color(rgb(0xf4f6f8))
        .child(label)
}

#[derive(Clone)]
struct SidebarResizeDrag;

impl Render for SidebarResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct DataFilterPanelResizeDrag;

impl Render for DataFilterPanelResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct CellDetailDrawerResizeDrag;

impl Render for CellDetailDrawerResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct QueryOutputResizeDrag;

impl Render for QueryOutputResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct RedisWorkbenchPanelResizeDrag;

impl Render for RedisWorkbenchPanelResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}


#[derive(Clone)]
struct TableInfoResizeDrag;

impl Render for TableInfoResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone, Copy, Debug)]
enum ConnectionMenuAction {
    Open,
    Disconnect,
    NewQuery,
    UserAdmin,
    ExecuteSqlFile,
    NewDatabase,
    Edit,
    Copy,
    Refresh,
    SelectDatabases,
    Delete,
    MoveToNewGroup,
    MoveToGroup(ConnectionGroupId),
    Ungroup,
}

#[derive(Clone, Copy, Debug)]
enum DatabaseMenuAction {
    TogglePin,
    ToggleOpen,
    SetDefault,
    NewTable,
    RunSqlFile,
    FindInDatabase,
    Refresh,
    Delete,
    NewQuery,
    /// 打开数据库备份面板。
    Backup,
    /// 打开 Redis CLI 终端（仅 Redis 数据库显示）。
    RedisCli,
    /// 打开 Redis Pub/Sub 会话（仅 Redis 数据库显示）。
    PubSub,
}

#[derive(Clone, Copy, Debug)]
enum TableMenuAction {
    TogglePin,
    CopyName,
    ViewData,
    Design,
    NewTable,
    Refresh,
    Rename,
    CopyTable,
    ExportData,
    CopyStructure,
    /// 打开数据库备份面板（备份当前表所在数据库）。
    Backup,
    Drop,
    Truncate,
    /// 从当前分组中移出该表。
    RemoveFromGroup,
}

#[derive(Clone, Copy, Debug)]
enum TableGroupMenuAction {
    NewTable,
    NewGroup,
    Refresh,
}

#[derive(Clone, Copy, Debug)]
enum TableFolderMenuAction {
    MoveUp,
    MoveDown,
    Rename,
    Delete,
}

#[derive(Clone, Copy, Debug)]
enum TabMenuAction {
    CopyTableName,
    Pin,
    Close,
    CloseOthers,
    CloseAll,
}

#[derive(Clone, Copy, Debug)]
enum GroupMenuAction {
    NewConnection,
    Rename,
    Delete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObjectGroup {
    Queries,
    Tables,
    Views,
    Procedures,
    Functions,
    Backup,
}

impl ObjectGroup {
    const ALL: [Self; 6] = [
        Self::Tables,
        Self::Views,
        Self::Procedures,
        Self::Functions,
        Self::Queries,
        Self::Backup,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Queries => "查询",
            Self::Tables => "表",
            Self::Views => "视图",
            Self::Procedures => "存储过程",
            Self::Functions => "函数",
            Self::Backup => "备份",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Queries => "queries",
            Self::Tables => "tables",
            Self::Views => "views",
            Self::Procedures => "procedures",
            Self::Functions => "functions",
            Self::Backup => "backup",
        }
    }

    fn icon_key(self) -> &'static str {
        match self {
            Self::Queries => "queries",
            Self::Tables => "tables",
            Self::Views => "views",
            Self::Procedures => "procedures",
            Self::Functions => "functions",
            Self::Backup => "backup",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NewConnectionTab {
    Connection,
    Tls,
    Ssh,
    Advanced,
}

/// 文本输入型字段 ID：用于 `set_connection_field_value` 与输入框绑定。
/// 布尔开关、认证方式下拉等非文本字段不走这里（见 `set_connection_toggle_field` 等）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionField {
    Name,
    Host,
    Port,
    Username,
    Password,
    Database,
    UrlParams,
    SqlitePath,
    MongoDefaultDb,
    MongoAuthDb,
    // —— Redis 专用文本字段 ——
    /// TLS CA 证书文件路径
    TlsCa,
    /// TLS 客户端证书文件路径
    TlsClientCert,
    /// TLS 客户端私钥文件路径
    TlsClientKey,
    /// TLS SNI / 校验主机名
    TlsSni,
    /// SSH 隧道主机
    SshHost,
    /// SSH 隧道端口
    SshPort,
    /// SSH 隧道用户名
    SshUsername,
    /// SSH 认证密码（password 认证）
    SshPassword,
    /// SSH 私钥文件路径（private_key 认证）
    SshPrivateKey,
    /// SSH 私钥口令
    SshPassphrase,
    /// Sentinel 主库名
    SentinelMasterName,
    /// Sentinel 节点列表（换行分隔）
    SentinelEndpoints,
    /// Cluster 起始节点列表（换行分隔）
    ClusterStartNodes,
    /// 云订阅/账号标识
    CloudSubscription,
    /// 云资源/数据库标识
    CloudResource,
    /// 连接串导入框
    DiscoveryUri,
    // —— MySQL / TiDB 专用文本字段 ——
    /// TLS 模式："disabled" / "preferred" / "required"
    MysqlTlsSslMode,
    /// 连接字符集
    MysqlCharset,
    /// 代理类型："socks5" / "http_connect"
    MysqlProxyType,
    /// SSH 连接超时（秒），0 表示继承全局
    MysqlSshConnectTimeout,
    /// SSH 心跳间隔（秒），0 表示不发送
    MysqlSshKeepalive,
    /// 代理主机
    MysqlProxyHost,
    /// 代理端口
    MysqlProxyPort,
    /// 代理用户名（可选）
    MysqlProxyUsername,
    /// 代理密码（可选）
    MysqlProxyPassword,
    /// 建连超时（秒），默认 5
    MysqlConnectTimeout,
    /// 查询超时（秒），0 表示不设限
    MysqlQueryTimeout,
    /// 连接空闲 TTL（秒），0 表示不回收
    MysqlIdleTtl,
}

/// 布尔开关类字段 ID（非文本输入，走 `set_connection_toggle_field`）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionToggleField {
    /// 是否启用 TLS
    TlsEnabled,
    /// 是否校验证书
    TlsVerify,
    /// 是否启用 SSH 隧道
    SshEnabled,
    /// Cluster 是否允许命令重定向到从节点
    ClusterAllowReadonly,
    /// 是否启用 MySQL/TiDB 代理
    MysqlProxyEnabled,
    /// 是否启用 MySQL/TiDB TCP 长连接保活
    MysqlTcpKeepalive,
}

#[derive(Clone, Debug)]
struct NewConnectionForm {
    name: String,
    host: String,
    port: String,
    username: String,
    password: String,
    color: String,
    database: String,
    url_params: String,
    sqlite_path: String,
    mongo_srv: bool,
    mongo_auth_db: String,
    mongo_auth_mechanism: String,
    // —— Redis 专用编辑值 ——
    tls_enabled: bool,
    tls_ca: String,
    tls_client_cert: String,
    tls_client_key: String,
    tls_sni: String,
    tls_verify: bool,
    ssh_enabled: bool,
    ssh_host: String,
    ssh_port: String,
    ssh_username: String,
    /// SSH 认证方式："password" / "private_key"
    ssh_auth: String,
    ssh_password: String,
    ssh_private_key: String,
    ssh_passphrase: String,
    // —— MySQL / TiDB 专用编辑值 ——
    /// TLS 模式："disabled" / "preferred" / "required"
    mysql_tls_ssl_mode: String,
    /// 连接字符集
    mysql_charset: String,
    /// SSH 连接超时（秒），0 表示继承全局
    mysql_ssh_connect_timeout_secs: String,
    /// SSH 心跳间隔（秒），0 表示不发送
    mysql_ssh_keepalive_secs: String,
    /// 是否启用代理
    mysql_proxy_enabled: bool,
    /// 代理类型："socks5" / "http_connect"
    mysql_proxy_type: String,
    mysql_proxy_host: String,
    mysql_proxy_port: String,
    mysql_proxy_username: String,
    mysql_proxy_password: String,
    /// 建连超时（秒），默认 5
    mysql_connect_timeout_secs: String,
    /// 查询超时（秒），0 表示不设限
    mysql_query_timeout_secs: String,
    /// 连接空闲 TTL（秒），0 表示不回收
    mysql_idle_ttl_secs: String,
    /// 是否启用 TCP 长连接保活
    mysql_tcp_keepalive: bool,
    sentinel_master_name: String,
    /// Sentinel 节点列表，换行分隔
    sentinel_endpoints: String,
    /// Cluster 起始节点列表，换行分隔
    cluster_start_nodes: String,
    cluster_allow_readonly: bool,
    /// 云提供商标识："azure" / "redis-cloud" / ""（非云）
    cloud_provider: String,
    cloud_subscription: String,
    cloud_resource: String,
    /// Redis 连接串导入框内容
    discovery_uri: String,
    test_status: Option<ConnectionTestStatus>,
}

struct NewConnectionInputs {
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    database: Entity<InputState>,
    url_params: Entity<InputState>,
    sqlite_path: Entity<InputState>,
    mongo_auth_db: Entity<InputState>,
    tls_ca: Entity<InputState>,
    tls_client_cert: Entity<InputState>,
    tls_client_key: Entity<InputState>,
    tls_sni: Entity<InputState>,
    ssh_host: Entity<InputState>,
    ssh_port: Entity<InputState>,
    ssh_username: Entity<InputState>,
    ssh_password: Entity<InputState>,
    ssh_private_key: Entity<InputState>,
    ssh_passphrase: Entity<InputState>,
    mysql_tls_ssl_mode: Entity<InputState>,
    mysql_charset: Entity<InputState>,
    mysql_proxy_type: Entity<InputState>,
    mysql_ssh_connect_timeout: Entity<InputState>,
    mysql_ssh_keepalive: Entity<InputState>,
    mysql_proxy_host: Entity<InputState>,
    mysql_proxy_port: Entity<InputState>,
    mysql_proxy_username: Entity<InputState>,
    mysql_proxy_password: Entity<InputState>,
    mysql_connect_timeout: Entity<InputState>,
    mysql_query_timeout: Entity<InputState>,
    mysql_idle_ttl: Entity<InputState>,
    sentinel_master_name: Entity<InputState>,
    sentinel_endpoints: Entity<InputState>,
    cluster_start_nodes: Entity<InputState>,
    cloud_subscription: Entity<InputState>,
    cloud_resource: Entity<InputState>,
    discovery_uri: Entity<InputState>,
    /// SSH 认证方式下拉（SelectState 实体，避免渲染期读 NavicatMain 触发重入 panic）。
    ssh_auth_select: Entity<SelectState<SearchableVec<String>>>,
    /// 云提供方下拉（同上）。
    cloud_provider_select: Entity<SelectState<SearchableVec<String>>>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
enum ConnectionTestStatus {
    Success(String),
    Error(String),
    Pending(String),
}

impl NewConnectionForm {
    fn for_kind(kind: DatabaseKind, index: usize) -> Self {
        // 先铺一套通用默认值（含 Redis 专用字段的默认），再按类型覆盖主机/端口/账号等。
        let mut form = Self {
            name: format!("New Connection {index}"),
            host: String::new(),
            port: String::new(),
            username: String::new(),
            password: String::new(),
            color: DEFAULT_CONNECTION_COLOR.to_string(),
            database: String::new(),
            url_params: String::new(),
            sqlite_path: String::new(),
            mongo_srv: false,
            mongo_auth_db: String::new(),
            mongo_auth_mechanism: "默认".to_string(),
            tls_enabled: false,
            tls_ca: String::new(),
            tls_client_cert: String::new(),
            tls_client_key: String::new(),
            tls_sni: String::new(),
            tls_verify: true,
            ssh_enabled: false,
            ssh_host: String::new(),
            ssh_port: "22".to_string(),
            ssh_username: String::new(),
            ssh_auth: "password".to_string(),
            ssh_password: String::new(),
            ssh_private_key: String::new(),
            ssh_passphrase: String::new(),
            mysql_tls_ssl_mode: "preferred".to_string(),
            mysql_charset: "utf8mb4".to_string(),
            mysql_ssh_connect_timeout_secs: "0".to_string(),
            mysql_ssh_keepalive_secs: "0".to_string(),
            mysql_proxy_enabled: false,
            mysql_proxy_type: "socks5".to_string(),
            mysql_proxy_host: String::new(),
            mysql_proxy_port: "1080".to_string(),
            mysql_proxy_username: String::new(),
            mysql_proxy_password: String::new(),
            mysql_connect_timeout_secs: "5".to_string(),
            mysql_query_timeout_secs: "0".to_string(),
            mysql_idle_ttl_secs: "0".to_string(),
            mysql_tcp_keepalive: true,
            sentinel_master_name: String::new(),
            sentinel_endpoints: String::new(),
            cluster_start_nodes: String::new(),
            cluster_allow_readonly: false,
            cloud_provider: String::new(),
            cloud_subscription: String::new(),
            cloud_resource: String::new(),
            discovery_uri: String::new(),
            test_status: None,
        };
        match kind {
            DatabaseKind::MySql => {
                form.name = format!("MySQL Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "3306".to_string();
                form.username = "root".to_string();
                form.url_params = "charset=utf8mb4&parseTime=true".to_string();
            }
            DatabaseKind::TiDb => {
                form.name = format!("TiDB Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "4000".to_string();
                form.username = "root".to_string();
                form.url_params = "ssl-mode=DISABLED".to_string();
            }
            DatabaseKind::Sqlite => {
                form.name = format!("SQLite Local {index}");
                form.sqlite_path = format!("local-{index}.db");
            }
            DatabaseKind::MongoDb => {
                form.name = format!("MongoDB Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "27017".to_string();
            }
            DatabaseKind::Redis => {
                form.name = format!("Redis Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "6379".to_string();
                form.username = "default".to_string();
                form.database = "0".to_string();
            }
        }
        form
    }

    fn from_config(config: &ConnectionConfig) -> Self {
        let mut form = Self::for_kind(config.kind, 1);
        form.name = config.name.clone();
        form.color = config
            .options
            .get(CONNECTION_COLOR_OPTION)
            .cloned()
            .unwrap_or_else(|| DEFAULT_CONNECTION_COLOR.to_string());
        form.username = config.options.get("username").cloned().unwrap_or_default();
        form.password = config.options.get("password").cloned().unwrap_or_default();
        form.url_params = config
            .options
            .get("url_params")
            .cloned()
            .unwrap_or_default();
        form.mongo_srv = config
            .options
            .get("srv")
            .is_some_and(|value| value == "true");
        form.mongo_auth_db = config.options.get("auth_db").cloned().unwrap_or_default();
        form.mongo_auth_mechanism = config
            .options
            .get("auth_mechanism")
            .cloned()
            .unwrap_or_else(|| "默认".to_string());
        form.test_status = None;

        match &config.endpoint {
            Endpoint::Tcp {
                host,
                port,
                database,
            } => {
                form.host = host.clone();
                form.port = port.to_string();
                form.database = database.clone().unwrap_or_default();
            }
            Endpoint::SqliteFile { path, .. } => {
                form.sqlite_path = path.display().to_string();
            }
            Endpoint::Uri { uri } => {
                form.url_params = uri.clone();
            }
        }

        // Redis 档案回填：优先结构化档案；缺省时用历史扁平参数迁移（兼容旧连接）。
        if config.kind == DatabaseKind::Redis {
            let profile = config
                .redis_profile
                .clone()
                .or_else(|| Some(fluxdb_core::RedisConnectionProfile::from_options(&config.options)));
            if let Some(profile) = profile {
                form.apply_profile(&profile);
                // 档案是事实来源，其派生的 host/port/账号覆盖上面的扁平 endpoint 值。
                if !profile.basic.host.is_empty() {
                    form.host = profile.basic.host.clone();
                }
                form.port = profile.basic.port.to_string();
            }
        }

        // MySQL / TiDB 档案回填：同样优先结构化档案，缺省时用历史扁平参数迁移。
        if matches!(config.kind, DatabaseKind::MySql | DatabaseKind::TiDb) {
            let profile = config
                .mysql_profile
                .clone()
                .or_else(|| Some(fluxdb_core::MysqlConnectionProfile::from_options(&config.options)));
            if let Some(profile) = profile {
                form.apply_mysql_profile(&profile);
            }
        }

        form
    }

    /// 用一份 Redis 档案（结构化或历史扁平迁移而来）回填表单的所有编辑值。
    fn apply_profile(&mut self, profile: &fluxdb_core::RedisConnectionProfile) {
        // 基础连接参数
        if !profile.basic.host.is_empty() {
            self.host = profile.basic.host.clone();
        }
        self.port = profile.basic.port.to_string();
        if let Some(db) = &profile.basic.database {
            self.database = db.clone();
        }
        if let Some(user) = &profile.basic.username {
            self.username = user.clone();
        }
        if let Some(password) = profile.basic.password.value() {
            self.password = password.to_string();
        }
        // TLS
        self.tls_enabled = profile.tls.enabled;
        self.tls_ca = profile.tls.ca.key.clone();
        self.tls_client_cert = profile.tls.client_cert.key.clone();
        self.tls_client_key = profile.tls.client_key.key.clone();
        self.tls_sni = profile.tls.sni.clone();
        self.tls_verify = profile.tls.verify;
        // SSH 隧道
        self.ssh_enabled = profile.ssh.enabled;
        self.ssh_host = profile.ssh.host.clone();
        self.ssh_port = profile.ssh.port.to_string();
        self.ssh_username = profile.ssh.username.clone();
        self.ssh_auth = match profile.ssh.auth {
            fluxdb_core::RedisSshAuth::Password => "password".to_string(),
            fluxdb_core::RedisSshAuth::PrivateKey => "private_key".to_string(),
        };
        if let Some(password) = profile.ssh.password.value() {
            self.ssh_password = password.to_string();
        }
        self.ssh_private_key = profile.ssh.private_key.key.clone();
        if let Some(passphrase) = profile.ssh.passphrase.value() {
            self.ssh_passphrase = passphrase.to_string();
        }
        // 拓扑：Sentinel / Cluster
        self.sentinel_master_name = profile.topology.sentinel.master_name.clone();
        self.sentinel_endpoints = profile.topology.sentinel.endpoints.join("\n");
        self.cluster_start_nodes = profile.topology.cluster.start_nodes.join("\n");
        self.cluster_allow_readonly = profile.topology.cluster.allow_readonly;
        // 云自动发现来源
        self.cloud_provider = profile.cloud.provider.clone();
        self.cloud_subscription = profile.cloud.subscription.clone();
        self.cloud_resource = profile.cloud.resource.clone();
    }

    /// 用一份 MySQL/TiDB 档案回填表单的所有 MySQL 专用编辑值。
    fn apply_mysql_profile(&mut self, profile: &fluxdb_core::MysqlConnectionProfile) {
        // TLS 模式
        self.mysql_tls_ssl_mode = match profile.tls.ssl_mode {
            fluxdb_core::MysqlSslMode::Disabled => "disabled".to_string(),
            fluxdb_core::MysqlSslMode::Preferred => "preferred".to_string(),
            fluxdb_core::MysqlSslMode::Required => "required".to_string(),
        };
        self.mysql_charset = profile.tls.charset.clone();
        // SSH 隧道（复用共享字段，另补 MySQL 专用超时）
        if let Some(ssh) = profile.ssh() {
            self.ssh_enabled = ssh.enabled;
            self.ssh_host = ssh.host.clone();
            self.ssh_port = ssh.port.to_string();
            self.ssh_username = ssh.username.clone();
            self.ssh_auth = match ssh.auth {
                fluxdb_core::MysqlSshAuth::Password => "password".to_string(),
                fluxdb_core::MysqlSshAuth::PrivateKey => "private_key".to_string(),
            };
            if let Some(password) = ssh.password.value() {
                self.ssh_password = password.to_string();
            }
            self.ssh_private_key = ssh.private_key.key.clone();
            if let Some(passphrase) = ssh.passphrase.value() {
                self.ssh_passphrase = passphrase.to_string();
            }
            self.mysql_ssh_connect_timeout_secs = ssh.connect_timeout_secs.to_string();
            self.mysql_ssh_keepalive_secs = ssh.keepalive_interval_secs.to_string();
        }
        // 代理
        if let Some(proxy) = profile.proxy() {
            self.mysql_proxy_enabled = proxy.enabled;
            self.mysql_proxy_type = match proxy.proxy_type {
                fluxdb_core::MysqlProxyType::Socks5 => "socks5".to_string(),
                fluxdb_core::MysqlProxyType::HttpConnect => "http_connect".to_string(),
            };
            self.mysql_proxy_host = proxy.host.clone();
            self.mysql_proxy_port = proxy.port.to_string();
            self.mysql_proxy_username = proxy.username.clone();
            if let Some(password) = proxy.password.value() {
                self.mysql_proxy_password = password.to_string();
            }
        }
        // 高级超时
        self.mysql_connect_timeout_secs = profile.advanced.connect_timeout_secs.to_string();
        self.mysql_query_timeout_secs = profile.advanced.query_timeout_secs.to_string();
        self.mysql_idle_ttl_secs = profile.advanced.idle_ttl_secs.to_string();
        self.mysql_tcp_keepalive = profile.advanced.tcp_keepalive;
    }

    /// 依据当前编辑值组装一份完整的 Redis 连接档案。
    ///
    /// SecretRef 约定（与 `redis_profile` / storage 保持一致）：
    /// - 密码类（基础密码、SSH 密码、SSH 口令）：`key` 留空，值放 `inline`，
    ///   存储层按 `credential_ref + 槽后缀` 写/读 Keychain，新建与编辑都正确。
    /// - 证书/私钥文件：以文件路径作 `key`、`inline` 为 None（非密码语义，不落 Keychain）。
    fn build_redis_profile(&self) -> fluxdb_core::RedisConnectionProfile {
        use fluxdb_core::{
            RedisBasicOptions, RedisClusterOptions, RedisCloudOptions, RedisConnectionProfile,
            RedisSentinelOptions, RedisSshAuth, RedisSshOptions, RedisTlsOptions, RedisTopology,
            RedisTopologyKind, SecretRef,
        };
        // 换行或逗号分隔的 host:port 列表统一拆分。
        let split_endpoints = |raw: &str| -> Vec<String> {
            raw.split([',', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };
        let file_ref = |path: &str| SecretRef::ref_key(path.trim().to_string());
        let inline_secret = |value: &str| SecretRef::inline(value.trim().to_string());

        let tls = RedisTlsOptions {
            enabled: self.tls_enabled,
            ca: file_ref(&self.tls_ca),
            client_cert: file_ref(&self.tls_client_cert),
            client_key: file_ref(&self.tls_client_key),
            sni: self.tls_sni.trim().to_string(),
            verify: self.tls_verify,
        };
        let ssh = RedisSshOptions {
            enabled: self.ssh_enabled,
            host: self.ssh_host.trim().to_string(),
            port: self.ssh_port.trim().parse::<u16>().unwrap_or(22),
            username: self.ssh_username.trim().to_string(),
            auth: if self.ssh_auth == "private_key" {
                RedisSshAuth::PrivateKey
            } else {
                RedisSshAuth::Password
            },
            password: inline_secret(&self.ssh_password),
            private_key: file_ref(&self.ssh_private_key),
            passphrase: inline_secret(&self.ssh_passphrase),
        };

        let sentinel_endpoints = split_endpoints(&self.sentinel_endpoints);
        let cluster_start_nodes = split_endpoints(&self.cluster_start_nodes);
        // 依据已填写的字段推断拓扑类型：优先 Sentinel，其次 Cluster，否则单点。
        let kind = if !self.sentinel_master_name.trim().is_empty() || !sentinel_endpoints.is_empty() {
            RedisTopologyKind::Sentinel
        } else if !cluster_start_nodes.is_empty() {
            RedisTopologyKind::Cluster
        } else {
            RedisTopologyKind::Standalone
        };
        let topology = RedisTopology {
            kind,
            sentinel: RedisSentinelOptions {
                master_name: self.sentinel_master_name.trim().to_string(),
                endpoints: sentinel_endpoints,
                username: Some(self.username.trim().to_string()).filter(|u| !u.is_empty()),
                password: inline_secret(&self.password),
                tls: tls.clone(),
            },
            cluster: RedisClusterOptions {
                start_nodes: cluster_start_nodes,
                allow_readonly: self.cluster_allow_readonly,
            },
        };
        let cloud = RedisCloudOptions {
            provider: self.cloud_provider.trim().to_string(),
            subscription: self.cloud_subscription.trim().to_string(),
            resource: self.cloud_resource.trim().to_string(),
            imported_name: String::new(),
        };

        RedisConnectionProfile {
            basic: RedisBasicOptions {
                host: self.host.trim().to_string(),
                port: self.port.trim().parse::<u16>().unwrap_or(6379),
                database: non_empty_option(&self.database),
                username: Some(self.username.trim().to_string()).filter(|u| !u.is_empty()),
                password: inline_secret(&self.password),
            },
            tls,
            ssh,
            topology,
            cloud,
        }
    }

    /// 依据当前编辑值组装一份完整的 MySQL/TiDB 连接档案。
    ///
    /// SecretRef 约定（与 `build_redis_profile` 完全一致，storage 按
    /// `credential_ref + 槽后缀` 写/读 Keychain）：
    /// - 密码类（基础密码、SSH 密码、SSH 口令、代理密码）：`key` 留空，值放 `inline`。
    /// - 证书/私钥文件（TLS CA/客户端证书/客户端密钥、SSH 私钥）：以文件路径作 `key`、
    ///   `inline` 为 None（非密码语义，不落 Keychain）。
    fn build_mysql_profile(&self) -> fluxdb_core::MysqlConnectionProfile {
        use fluxdb_core::{
            MysqlAdvancedOptions, MysqlBasicOptions, MysqlConnectionProfile, MysqlProxy,
            MysqlProxyType, MysqlSshAuth, MysqlSshOptions, MysqlSslMode, MysqlTlsOptions,
            MysqlTransportLayer, SecretRef,
        };
        let file_ref = |path: &str| SecretRef::ref_key(path.trim().to_string());
        let inline_secret = |value: &str| SecretRef::inline(value.trim().to_string());

        let tls = MysqlTlsOptions {
            enabled: self.tls_enabled,
            ssl_mode: match self.mysql_tls_ssl_mode.trim() {
                "disabled" => MysqlSslMode::Disabled,
                "required" => MysqlSslMode::Required,
                // 缺省/非法值统一按 Preferred 处理。
                _ => MysqlSslMode::Preferred,
            },
            ca: file_ref(&self.tls_ca),
            client_cert: file_ref(&self.tls_client_cert),
            client_key: file_ref(&self.tls_client_key),
            sni: self.tls_sni.trim().to_string(),
            verify: self.tls_verify,
            charset: self.mysql_charset.trim().to_string(),
            collation: String::new(),
        };

        // 传输层列表：仅把「已启用的 SSH 隧道 / 代理」压入。
        let mut transport = Vec::new();
        if self.ssh_enabled {
            transport.push(MysqlTransportLayer::Ssh(MysqlSshOptions {
                enabled: true,
                host: self.ssh_host.trim().to_string(),
                port: self.ssh_port.trim().parse::<u16>().unwrap_or(22),
                username: self.ssh_username.trim().to_string(),
                auth: if self.ssh_auth == "private_key" {
                    MysqlSshAuth::PrivateKey
                } else {
                    MysqlSshAuth::Password
                },
                password: inline_secret(&self.ssh_password),
                private_key: file_ref(&self.ssh_private_key),
                passphrase: inline_secret(&self.ssh_passphrase),
                connect_timeout_secs: self
                    .mysql_ssh_connect_timeout_secs
                    .trim()
                    .parse()
                    .unwrap_or(0),
                keepalive_interval_secs: self.mysql_ssh_keepalive_secs.trim().parse().unwrap_or(0),
            }));
        }
        if self.mysql_proxy_enabled {
            let proxy_type = if self.mysql_proxy_type.trim() == "http_connect" {
                MysqlProxyType::HttpConnect
            } else {
                MysqlProxyType::Socks5
            };
            transport.push(MysqlTransportLayer::Proxy(MysqlProxy {
                enabled: true,
                proxy_type,
                host: self.mysql_proxy_host.trim().to_string(),
                port: self
                    .mysql_proxy_port
                    .trim()
                    .parse()
                    .unwrap_or(proxy_type.default_port()),
                username: self.mysql_proxy_username.trim().to_string(),
                password: inline_secret(&self.mysql_proxy_password),
            }));
        }

        MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: self.host.trim().to_string(),
                port: self.port.trim().parse::<u16>().unwrap_or(3306),
                database: non_empty_option(&self.database).unwrap_or_default(),
                username: self.username.trim().to_string(),
                password: inline_secret(&self.password),
            },
            tls,
            transport,
            advanced: MysqlAdvancedOptions {
                connect_timeout_secs: self
                    .mysql_connect_timeout_secs
                    .trim()
                    .parse()
                    .unwrap_or(5),
                query_timeout_secs: self.mysql_query_timeout_secs.trim().parse().unwrap_or(0),
                idle_ttl_secs: self.mysql_idle_ttl_secs.trim().parse().unwrap_or(0),
                tcp_keepalive: self.mysql_tcp_keepalive,
            },
        }
    }
}

impl NewConnectionInputs {
    fn new(window: &mut Window, cx: &mut Context<NavicatMain>) -> Self {
        fn input_state(
            field: ConnectionField,
            masked: bool,
            window: &mut Window,
            cx: &mut Context<NavicatMain>,
        ) -> (Entity<InputState>, Subscription) {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(connection_field_placeholder(field))
                    .masked(masked)
            });
            let subscription = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.set_connection_field_value(field, input.read(cx).value().to_string(), cx);
                }
            });

            (input, subscription)
        }

        let (name, name_subscription) = input_state(ConnectionField::Name, false, window, cx);
        let (host, host_subscription) = input_state(ConnectionField::Host, false, window, cx);
        let (port, port_subscription) = input_state(ConnectionField::Port, false, window, cx);
        let (username, username_subscription) =
            input_state(ConnectionField::Username, false, window, cx);
        let (password, password_subscription) =
            input_state(ConnectionField::Password, true, window, cx);
        let (database, database_subscription) =
            input_state(ConnectionField::Database, false, window, cx);
        let (url_params, url_params_subscription) =
            input_state(ConnectionField::UrlParams, false, window, cx);
        let (sqlite_path, sqlite_path_subscription) =
            input_state(ConnectionField::SqlitePath, false, window, cx);
        let (mongo_auth_db, mongo_auth_db_subscription) =
            input_state(ConnectionField::MongoAuthDb, false, window, cx);
        // —— Redis 专用文本输入 ——
        let (tls_ca, tls_ca_subscription) = input_state(ConnectionField::TlsCa, false, window, cx);
        let (tls_client_cert, tls_client_cert_subscription) =
            input_state(ConnectionField::TlsClientCert, false, window, cx);
        let (tls_client_key, tls_client_key_subscription) =
            input_state(ConnectionField::TlsClientKey, false, window, cx);
        let (tls_sni, tls_sni_subscription) = input_state(ConnectionField::TlsSni, false, window, cx);
        let (ssh_host, ssh_host_subscription) = input_state(ConnectionField::SshHost, false, window, cx);
        let (ssh_port, ssh_port_subscription) = input_state(ConnectionField::SshPort, false, window, cx);
        let (ssh_username, ssh_username_subscription) =
            input_state(ConnectionField::SshUsername, false, window, cx);
        let (ssh_password, ssh_password_subscription) =
            input_state(ConnectionField::SshPassword, true, window, cx);
        let (ssh_private_key, ssh_private_key_subscription) =
            input_state(ConnectionField::SshPrivateKey, false, window, cx);
        let (ssh_passphrase, ssh_passphrase_subscription) =
            input_state(ConnectionField::SshPassphrase, true, window, cx);
        // —— MySQL / TiDB 专用文本输入 ——
        let (mysql_tls_ssl_mode, mysql_tls_ssl_mode_subscription) =
            input_state(ConnectionField::MysqlTlsSslMode, false, window, cx);
        let (mysql_charset, mysql_charset_subscription) =
            input_state(ConnectionField::MysqlCharset, false, window, cx);
        let (mysql_proxy_type, mysql_proxy_type_subscription) =
            input_state(ConnectionField::MysqlProxyType, false, window, cx);
        let (mysql_ssh_connect_timeout, mysql_ssh_connect_timeout_subscription) =
            input_state(ConnectionField::MysqlSshConnectTimeout, false, window, cx);
        let (mysql_ssh_keepalive, mysql_ssh_keepalive_subscription) =
            input_state(ConnectionField::MysqlSshKeepalive, false, window, cx);
        let (mysql_proxy_host, mysql_proxy_host_subscription) =
            input_state(ConnectionField::MysqlProxyHost, false, window, cx);
        let (mysql_proxy_port, mysql_proxy_port_subscription) =
            input_state(ConnectionField::MysqlProxyPort, false, window, cx);
        let (mysql_proxy_username, mysql_proxy_username_subscription) =
            input_state(ConnectionField::MysqlProxyUsername, false, window, cx);
        let (mysql_proxy_password, mysql_proxy_password_subscription) =
            input_state(ConnectionField::MysqlProxyPassword, true, window, cx);
        let (mysql_connect_timeout, mysql_connect_timeout_subscription) =
            input_state(ConnectionField::MysqlConnectTimeout, false, window, cx);
        let (mysql_query_timeout, mysql_query_timeout_subscription) =
            input_state(ConnectionField::MysqlQueryTimeout, false, window, cx);
        let (mysql_idle_ttl, mysql_idle_ttl_subscription) =
            input_state(ConnectionField::MysqlIdleTtl, false, window, cx);
        let (sentinel_master_name, sentinel_master_name_subscription) =
            input_state(ConnectionField::SentinelMasterName, false, window, cx);
        let (sentinel_endpoints, sentinel_endpoints_subscription) =
            input_state(ConnectionField::SentinelEndpoints, false, window, cx);
        let (cluster_start_nodes, cluster_start_nodes_subscription) =
            input_state(ConnectionField::ClusterStartNodes, false, window, cx);
        let (cloud_subscription, cloud_subscription_s) =
            input_state(ConnectionField::CloudSubscription, false, window, cx);
        let (cloud_resource, cloud_resource_s) =
            input_state(ConnectionField::CloudResource, false, window, cx);
        let (discovery_uri, discovery_uri_s) =
            input_state(ConnectionField::DiscoveryUri, false, window, cx);

        // SSH 认证方式下拉：选项为展示文案，订阅回调里映射回表单字段值。
        let ssh_auth_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec!["密码".to_string(), "私钥".to_string()]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let ssh_auth_select_s = cx.subscribe(
            &ssh_auth_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<String>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    // 展示文案 → 表单字段值。
                    this.new_connection_form.ssh_auth = match value.as_str() {
                        "私钥" => "private_key".to_string(),
                        _ => "password".to_string(),
                    };
                    cx.notify();
                }
            },
        );
        // 云提供方下拉：同上。
        let cloud_provider_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    "不使用云".to_string(),
                    "Azure".to_string(),
                    "Redis Cloud".to_string(),
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let cloud_provider_select_s = cx.subscribe(
            &cloud_provider_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<String>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    // 展示文案 → 表单字段值。
                    this.new_connection_form.cloud_provider = match value.as_str() {
                        "Azure" => "azure".to_string(),
                        "Redis Cloud" => "redis-cloud".to_string(),
                        _ => String::new(),
                    };
                    cx.notify();
                }
            },
        );

        Self {
            name,
            host,
            port,
            username,
            password,
            database,
            url_params,
            sqlite_path,
            mongo_auth_db,
            tls_ca,
            tls_client_cert,
            tls_client_key,
            tls_sni,
            ssh_host,
            ssh_port,
            ssh_username,
            ssh_password,
            ssh_private_key,
            ssh_passphrase,
            mysql_tls_ssl_mode,
            mysql_charset,
            mysql_proxy_type,
            mysql_ssh_connect_timeout,
            mysql_ssh_keepalive,
            mysql_proxy_host,
            mysql_proxy_port,
            mysql_proxy_username,
            mysql_proxy_password,
            mysql_connect_timeout,
            mysql_query_timeout,
            mysql_idle_ttl,
            sentinel_master_name,
            sentinel_endpoints,
            cluster_start_nodes,
            cloud_subscription,
            cloud_resource,
            discovery_uri,
            ssh_auth_select,
            cloud_provider_select,
            _subscriptions: vec![
                name_subscription,
                host_subscription,
                port_subscription,
                username_subscription,
                password_subscription,
                database_subscription,
                url_params_subscription,
                sqlite_path_subscription,
                mongo_auth_db_subscription,
                tls_ca_subscription,
                tls_client_cert_subscription,
                tls_client_key_subscription,
                tls_sni_subscription,
                ssh_host_subscription,
                ssh_port_subscription,
                ssh_username_subscription,
                ssh_password_subscription,
                ssh_private_key_subscription,
                ssh_passphrase_subscription,
                mysql_tls_ssl_mode_subscription,
                mysql_charset_subscription,
                mysql_proxy_type_subscription,
                mysql_ssh_connect_timeout_subscription,
                mysql_ssh_keepalive_subscription,
                mysql_proxy_host_subscription,
                mysql_proxy_port_subscription,
                mysql_proxy_username_subscription,
                mysql_proxy_password_subscription,
                mysql_connect_timeout_subscription,
                mysql_query_timeout_subscription,
                mysql_idle_ttl_subscription,
                sentinel_master_name_subscription,
                sentinel_endpoints_subscription,
                cluster_start_nodes_subscription,
                cloud_subscription_s,
                cloud_resource_s,
                discovery_uri_s,
                ssh_auth_select_s,
                cloud_provider_select_s,
            ],
        }
    }

    fn for_field(&self, field: ConnectionField) -> &Entity<InputState> {
        match field {
            ConnectionField::Name => &self.name,
            ConnectionField::Host => &self.host,
            ConnectionField::Port => &self.port,
            ConnectionField::Username => &self.username,
            ConnectionField::Password => &self.password,
            ConnectionField::Database | ConnectionField::MongoDefaultDb => &self.database,
            ConnectionField::UrlParams => &self.url_params,
            ConnectionField::SqlitePath => &self.sqlite_path,
            ConnectionField::MongoAuthDb => &self.mongo_auth_db,
            ConnectionField::TlsCa => &self.tls_ca,
            ConnectionField::TlsClientCert => &self.tls_client_cert,
            ConnectionField::TlsClientKey => &self.tls_client_key,
            ConnectionField::TlsSni => &self.tls_sni,
            ConnectionField::SshHost => &self.ssh_host,
            ConnectionField::SshPort => &self.ssh_port,
            ConnectionField::SshUsername => &self.ssh_username,
            ConnectionField::SshPassword => &self.ssh_password,
            ConnectionField::SshPrivateKey => &self.ssh_private_key,
            ConnectionField::SshPassphrase => &self.ssh_passphrase,
            ConnectionField::MysqlTlsSslMode => &self.mysql_tls_ssl_mode,
            ConnectionField::MysqlCharset => &self.mysql_charset,
            ConnectionField::MysqlProxyType => &self.mysql_proxy_type,
            ConnectionField::MysqlSshConnectTimeout => &self.mysql_ssh_connect_timeout,
            ConnectionField::MysqlSshKeepalive => &self.mysql_ssh_keepalive,
            ConnectionField::MysqlProxyHost => &self.mysql_proxy_host,
            ConnectionField::MysqlProxyPort => &self.mysql_proxy_port,
            ConnectionField::MysqlProxyUsername => &self.mysql_proxy_username,
            ConnectionField::MysqlProxyPassword => &self.mysql_proxy_password,
            ConnectionField::MysqlConnectTimeout => &self.mysql_connect_timeout,
            ConnectionField::MysqlQueryTimeout => &self.mysql_query_timeout,
            ConnectionField::MysqlIdleTtl => &self.mysql_idle_ttl,
            ConnectionField::SentinelMasterName => &self.sentinel_master_name,
            ConnectionField::SentinelEndpoints => &self.sentinel_endpoints,
            ConnectionField::ClusterStartNodes => &self.cluster_start_nodes,
            ConnectionField::CloudSubscription => &self.cloud_subscription,
            ConnectionField::CloudResource => &self.cloud_resource,
            ConnectionField::DiscoveryUri => &self.discovery_uri,
        }
    }

    fn sync_from_form(
        &self,
        form: &NewConnectionForm,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) {
        self.set_value(ConnectionField::Name, &form.name, window, cx);
        self.set_value(ConnectionField::Host, &form.host, window, cx);
        self.set_value(ConnectionField::Port, &form.port, window, cx);
        self.set_value(ConnectionField::Username, &form.username, window, cx);
        self.set_value(ConnectionField::Password, &form.password, window, cx);
        self.set_value(ConnectionField::Database, &form.database, window, cx);
        self.set_value(ConnectionField::UrlParams, &form.url_params, window, cx);
        self.set_value(ConnectionField::SqlitePath, &form.sqlite_path, window, cx);
        self.set_value(
            ConnectionField::MongoAuthDb,
            &form.mongo_auth_db,
            window,
            cx,
        );
        // —— Redis 专用 ——
        self.set_value(ConnectionField::TlsCa, &form.tls_ca, window, cx);
        self.set_value(ConnectionField::TlsClientCert, &form.tls_client_cert, window, cx);
        self.set_value(ConnectionField::TlsClientKey, &form.tls_client_key, window, cx);
        self.set_value(ConnectionField::TlsSni, &form.tls_sni, window, cx);
        self.set_value(ConnectionField::SshHost, &form.ssh_host, window, cx);
        self.set_value(ConnectionField::SshPort, &form.ssh_port, window, cx);
        self.set_value(ConnectionField::SshUsername, &form.ssh_username, window, cx);
        self.set_value(ConnectionField::SshPassword, &form.ssh_password, window, cx);
        self.set_value(ConnectionField::SshPrivateKey, &form.ssh_private_key, window, cx);
        self.set_value(ConnectionField::SshPassphrase, &form.ssh_passphrase, window, cx);
        // —— MySQL / TiDB 专用 ——
        self.set_value(
            ConnectionField::MysqlTlsSslMode,
            &form.mysql_tls_ssl_mode,
            window,
            cx,
        );
        self.set_value(ConnectionField::MysqlCharset, &form.mysql_charset, window, cx);
        self.set_value(
            ConnectionField::MysqlProxyType,
            &form.mysql_proxy_type,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlSshConnectTimeout,
            &form.mysql_ssh_connect_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlSshKeepalive,
            &form.mysql_ssh_keepalive_secs,
            window,
            cx,
        );
        self.set_value(ConnectionField::MysqlProxyHost, &form.mysql_proxy_host, window, cx);
        self.set_value(ConnectionField::MysqlProxyPort, &form.mysql_proxy_port, window, cx);
        self.set_value(
            ConnectionField::MysqlProxyUsername,
            &form.mysql_proxy_username,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlProxyPassword,
            &form.mysql_proxy_password,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlConnectTimeout,
            &form.mysql_connect_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlQueryTimeout,
            &form.mysql_query_timeout_secs,
            window,
            cx,
        );
        self.set_value(ConnectionField::MysqlIdleTtl, &form.mysql_idle_ttl_secs, window, cx);
        self.set_value(
            ConnectionField::SentinelMasterName,
            &form.sentinel_master_name,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::SentinelEndpoints,
            &form.sentinel_endpoints,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::ClusterStartNodes,
            &form.cluster_start_nodes,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::CloudSubscription,
            &form.cloud_subscription,
            window,
            cx,
        );
        self.set_value(ConnectionField::CloudResource, &form.cloud_resource, window, cx);
        self.set_value(ConnectionField::DiscoveryUri, &form.discovery_uri, window, cx);
    }


    fn set_value(
        &self,
        field: ConnectionField,
        value: &str,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) {
        let value = value.to_string();
        self.for_field(field).update(cx, |input, cx| {
            input.set_value(value, window, cx);
        });
    }

    fn focus_field(
        &self,
        field: ConnectionField,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) {
        self.for_field(field).update(cx, |input, cx| {
            input.focus(window, cx);
        });
    }
}
