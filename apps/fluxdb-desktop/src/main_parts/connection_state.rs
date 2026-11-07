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

/// ER 关系面板拖拽类型：独立于连接栏的 `SidebarResizeDrag`。连接栏的 `on_drag_move`
/// 无条件 stop_propagation，若不隔离，关系面板的同类型拖拽事件会被连接栏抢走。
#[derive(Clone)]
struct ErRelationshipResizeDrag;

impl Render for ErRelationshipResizeDrag {
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
struct WorkbenchPanelResizeDrag;

impl Render for WorkbenchPanelResizeDrag {
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
    /// 显示连接信息（仅 MongoDB：拓扑 / 版本 / 成员 / 库大小）。
    MongoInfo,
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
    /// 新建 PostgreSQL schema（仅 PG 数据库显示）。
    NewSchema,
    /// 新建集合（仅 MongoDB 数据库显示）。
    NewCollection,
    /// 打开 MongoDB 命令行终端（仅 MongoDB 数据库显示）。
    MongoCli,
    /// 打开 MongoDB 命令工作台（仅 MongoDB 数据库显示）。
    MongoWorkbench,
}

/// PostgreSQL schema 右键菜单动作：全部 schema 作用域，不与数据库/表动作混淆。
#[derive(Clone, Copy, Debug)]
enum SchemaMenuAction {
    NewQuery,
    NewTable,
    Refresh,
    SetDefault,
}

#[derive(Clone, Copy, Debug)]
enum TableMenuAction {
    TogglePin,
    CopyName,
    ViewData,
    /// 打开以当前表为中心 1 跳的关联 ER 关系图。
    OpenErDiagram,
    Design,
    NewTable,
    Refresh,
    Rename,
    CopyTable,
    ImportData,
    ExportData,
    ExportDatabase,
    CopyStructure,
    /// 打开数据库备份面板（备份当前表所在数据库）。
    Backup,
    Drop,
    Truncate,
    /// 从当前分组中移出该表。
    RemoveFromGroup,
}

/// MongoDB 集合右键菜单动作（视图只读时不会渲染写入类项）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CollectionMenuAction {
    TogglePin,
    CopyName,
    Refresh,
    Rename,
    Import,
    Export,
    Drop,
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
    /// ER 关系图（画布原型；单击打开整库 ER 标签页，不展开子行）。
    ErDiagram,
    /// MongoDB 集合与视图（Mongo 没有表/存储过程分组，集合即第一层对象）。
    Collections,
}

impl ObjectGroup {
    /// SQL 型连接的分组；「集合」只属于 Mongo，不能出现在这里（否则 MySQL 等库也会多出集合分组）。
    const ALL: [Self; 7] = [
        Self::Tables,
        Self::Views,
        Self::Procedures,
        Self::Functions,
        Self::Queries,
        Self::Backup,
        Self::ErDiagram,
    ];

    /// MongoDB 库下只展示「集合」一个分组。
    const MONGO: [Self; 1] = [Self::Collections];

    /// 某类连接的数据库分组顺序：Mongo 与 SQL 型连接的分组语义不同。
    fn for_kind(kind: DatabaseKind) -> &'static [Self] {
        match kind {
            DatabaseKind::MongoDb => &Self::MONGO,
            _ => &Self::ALL,
        }
    }

    fn label(self) -> SharedString {
        tr(match self {
            Self::Queries => "app.connection.kind.queries",
            Self::Tables => "app.connection.kind.tables",
            Self::Views => "app.connection.kind.views",
            Self::Procedures => "app.connection.kind.procedures",
            Self::Functions => "app.connection.kind.functions",
            Self::Backup => "app.connection.kind.backup",
            Self::ErDiagram => "app.connection.kind.er_diagram",
            Self::Collections => "app.connection.kind.collections",
        })
    }

    fn key(self) -> &'static str {
        match self {
            Self::Queries => "queries",
            Self::Tables => "tables",
            Self::Views => "views",
            Self::Procedures => "procedures",
            Self::Functions => "functions",
            Self::Backup => "backup",
            Self::ErDiagram => "er",
            Self::Collections => "collections",
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
            Self::ErDiagram => "er",
            Self::Collections => "tables",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NewConnectionTab {
    Connection,
    /// 网络与安全：TLS/加密与 SSH 隧道合并在一页。
    Tls,
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
    DamengSchema,
    /// 达梦建连超时（秒，默认 5）
    DamengConnectTimeout,
    /// 达梦查询超时（秒，0=不设限）
    DamengQueryTimeout,
    /// 达梦应用名（DM 会话可见）
    DamengApplicationName,
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
    /// TLS 客户端私钥口令
    TlsClientKeyPassphrase,
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
    /// AI Redis 工具的 Key 授权前缀（逗号分隔）
    AiKeyPrefixes,
    /// 连接串导入框
    DiscoveryUri,
    // —— PostgreSQL 专用文本字段 ——
    /// 默认 schema（search_path 首段）
    PgDefaultSchema,
    /// 应用名
    PgApplicationName,
    /// 建连超时（秒）
    PgConnectTimeoutSecs,
    /// 查询超时（秒）
    PgQueryTimeoutSecs,
    /// 会话时区（空表示服务器默认）
    PgTimezone,
    // —— MySQL / TiDB 专用文本字段 ——
    /// 连接字符集
    MysqlCharset,
    /// 排序规则；留空跟随服务器默认
    MysqlCollation,
    /// Unix socket 路径；非空时代替 TCP 拨号
    MysqlSocketPath,
    /// 会话时区（如 +08:00 / Asia/Shanghai）
    MysqlTimezone,
    /// 连接建立后执行的初始化 SQL（分号分隔）
    MysqlInitSql,
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
    // —— SQL Server 专用文本字段 ——
    /// 默认 schema；空表示连接后读取 `SCHEMA_NAME()`，最终回退 `dbo`。
    SqlServerSchema,
    /// 命名实例；非空且端口为默认值时经 SQL Browser（UDP 1434）解析实际端口。
    SqlServerInstance,
    /// 建连超时（秒），默认 5
    SqlServerConnectTimeout,
    /// 查询超时（秒），0 表示不设限
    SqlServerQueryTimeout,
    /// 应用名（服务端 `APP_NAME()` 显示）
    SqlServerApplicationName,
    // —— Oracle 专用文本字段 ——
    /// Service Name（Easy Connect 目标，通常是 PDB 服务名）
    OracleServiceName,
    /// SID（仅 `connect_kind == sid` 时用于拼显式描述符）
    OracleSid,
    /// 登录后切换的默认 Schema；只改名称解析，不授予任何权限。
    OracleSchema,
    /// 建连超时（秒），0 表示不设限
    OracleConnectTimeout,
    /// 语句超时（秒），0 表示不设限
    OracleQueryTimeout,
    /// 客户端程序名（`v$session.program` 可见）
    OracleApplicationName,
    /// Wallet 目录路径；空表示不使用 Wallet
    OracleWalletLocation,
    /// Wallet 口令（掩码输入，落盘走加密凭据表）
    OracleWalletPassword,
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
    /// 强制按单机直连（跳过拓扑自动探测）
    ForceStandalone,
    /// 是否启用 MySQL/TiDB 代理
    MysqlProxyEnabled,
    /// 是否启用 MySQL/TiDB TCP 长连接保活
    MysqlTcpKeepalive,
    /// 是否启用 PostgreSQL TCP 保活
    PgTcpKeepalive,
    /// 是否在对象树显示其他数据库（默认仅维护库/当前库）
    PgShowOtherDatabases,
    /// 是否显示系统 schema（pg_catalog/information_schema 等）
    PgShowSystemSchemas,
    /// MongoDB 是否走 `mongodb+srv://`（SRV 拓扑发现，单 DNS 名）
    MongoSrv,
    /// 连接级只读：开启后禁止通过该连接写入
    ReadOnly,
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
    dameng_schema: String,
    // —— 达梦专用编辑值 ——
    /// 建连超时（秒），默认 5
    dameng_connect_timeout_secs: String,
    /// 查询超时（秒），0 表示不设限
    dameng_query_timeout_secs: String,
    /// 应用名，写入 DM 会话
    dameng_application_name: String,
    /// 服务器字符集提示；空表示驱动握手自动检测。
    /// 界面不暴露（手填错误会乱码），仅在编辑时原样保留，避免保存丢值。
    dameng_encoding: String,
    url_params: String,
    sqlite_path: String,
    mongo_srv: bool,
    /// 连接级只读（落盘为 `options["read_only"]`）
    read_only: bool,
    mongo_auth_db: String,
    mongo_auth_mechanism: String,
    // —— Redis 专用编辑值 ——
    tls_enabled: bool,
    tls_ca: String,
    tls_client_cert: String,
    tls_client_key: String,
    /// 客户端私钥口令（仅 PostgreSQL mTLS 加密私钥使用）
    tls_client_key_passphrase: String,
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
    /// SSH 主机密钥策略："strict" / "accept_new" / "ignore"
    ssh_host_key_policy: String,
    // —— MySQL / TiDB 专用编辑值 ——
    /// TLS 模式："disabled" / "preferred" / "required" / "verify_ca" / "verify_identity"
    mysql_tls_ssl_mode: String,
    /// Unix Socket 路径；非空时替代 TCP 主机/端口拨号
    mysql_socket_path: String,
    /// 连接字符集
    mysql_charset: String,
    /// 连接排序规则（空表示服务端默认）
    mysql_collation: String,
    /// 会话时区（空表示驱动默认）
    mysql_timezone: String,
    /// 连接初始化 SQL（分号分隔，建连后逐条执行）
    mysql_init_sql: String,
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
    /// 是否启用 TCP 长连接保活
    mysql_tcp_keepalive: bool,
    // —— PostgreSQL 专用编辑值 ——
    /// TLS 模式："disable" / "prefer" / "require" / "verify-ca" / "verify-full"
    pg_tls_ssl_mode: String,
    /// 默认 schema 作用域（search_path 首段），空表示库默认
    pg_default_schema: String,
    /// 应用名（服务端 pg_stat_activity.application_name 显示）
    pg_application_name: String,
    /// 建连超时（秒），0 表示不限
    pg_connect_timeout_secs: String,
    /// 查询/语句超时（秒），0 表示不设限
    pg_query_timeout_secs: String,
    /// 会话时区（如 Asia/Shanghai），空表示服务器默认
    pg_timezone: String,
    /// TCP 保活
    pg_tcp_keepalive: bool,
    /// 对象树是否显示其他数据库
    pg_show_other_databases: bool,
    /// 对象树是否显示系统 schema
    pg_show_system_schemas: bool,
    // —— SQL Server 专用编辑值 ——
    /// 加密模式：`disable` / `require` / `trust_server_certificate`
    sqlserver_encryption: String,
    sqlserver_schema: String,
    sqlserver_instance: String,
    sqlserver_connect_timeout_secs: String,
    sqlserver_query_timeout_secs: String,
    sqlserver_application_name: String,
    // —— Oracle 专用编辑值 ——
    /// 监听器寻址方式："service_name" / "sid"
    oracle_connect_kind: String,
    /// 登录身份："normal" / "sysdba" / "sysoper"
    oracle_privilege: String,
    oracle_service_name: String,
    oracle_sid: String,
    oracle_schema: String,
    oracle_connect_timeout_secs: String,
    oracle_query_timeout_secs: String,
    oracle_application_name: String,
    oracle_wallet_location: String,
    oracle_wallet_password: String,
    /// Redis 拓扑：`standalone` / `sentinel` / `cluster`。
    ///
    /// 界面不再让用户选择：连接时由连接器自动探测。该值仅由连接串导入与
    /// 旧档案回填写入，保留用于兼容历史节点列表。
    redis_topology: String,
    /// Sentinel 主库名（可选）。自动探测只能识别「这是哨兵」，多主库时
    /// 需要用户指明连哪个；单主库可留空。
    sentinel_master_name: String,
    /// Sentinel 节点列表，换行分隔（旧档案/导入专用，界面不再展示）
    sentinel_endpoints: String,
    /// Cluster 起始节点列表，换行分隔（旧档案/导入专用，界面不再展示）
    cluster_start_nodes: String,
    cluster_allow_readonly: bool,
    /// 强制按单机直连，跳过拓扑自动探测（代理/端口转发场景的逃生开关）。
    force_standalone: bool,
    /// AI Redis 工具的 Key 授权前缀（逗号分隔），非连接协议参数。
    ai_key_prefixes: String,
    /// Redis 连接串导入框内容
    discovery_uri: String,
    test_status: Option<ConnectionTestStatus>,
}

// AI 配置草稿独立职责，仍保持 include! 的原有作用域。
include!("settings/ai_state.rs");

struct NewConnectionInputs {
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    database: Entity<InputState>,
    dameng_schema: Entity<InputState>,
    dameng_connect_timeout: Entity<InputState>,
    dameng_query_timeout: Entity<InputState>,
    dameng_application_name: Entity<InputState>,
    url_params: Entity<InputState>,
    sqlite_path: Entity<InputState>,
    mongo_auth_db: Entity<InputState>,
    tls_ca: Entity<InputState>,
    tls_client_cert: Entity<InputState>,
    tls_client_key: Entity<InputState>,
    tls_client_key_passphrase: Entity<InputState>,
    tls_sni: Entity<InputState>,
    ssh_host: Entity<InputState>,
    ssh_port: Entity<InputState>,
    ssh_username: Entity<InputState>,
    ssh_password: Entity<InputState>,
    ssh_private_key: Entity<InputState>,
    ssh_passphrase: Entity<InputState>,
    /// MySQL/TiDB SSL 模式下拉（SelectState 实体，避免渲染期读 NavicatMain 触发重入 panic）。
    mysql_ssl_mode_select: Entity<SelectState<SearchableVec<String>>>,
    /// —— PostgreSQL 专用 ——
    /// PG SSL 模式下拉（同上）。
    pg_ssl_mode_select: Entity<SelectState<SearchableVec<String>>>,
    pg_default_schema: Entity<InputState>,
    pg_application_name: Entity<InputState>,
    pg_connect_timeout: Entity<InputState>,
    pg_query_timeout: Entity<InputState>,
    pg_timezone: Entity<InputState>,
    /// —— SQL Server 专用 ——
    sqlserver_encryption_select: Entity<SelectState<SearchableVec<ConnectionSelectOption>>>,
    sqlserver_schema: Entity<InputState>,
    sqlserver_instance: Entity<InputState>,
    sqlserver_connect_timeout: Entity<InputState>,
    sqlserver_query_timeout: Entity<InputState>,
    sqlserver_application_name: Entity<InputState>,
    /// —— Oracle 专用 ——
    oracle_connect_kind_select: Entity<SelectState<SearchableVec<ConnectionSelectOption>>>,
    oracle_privilege_select: Entity<SelectState<SearchableVec<ConnectionSelectOption>>>,
    oracle_service_name: Entity<InputState>,
    oracle_sid: Entity<InputState>,
    oracle_schema: Entity<InputState>,
    oracle_connect_timeout: Entity<InputState>,
    oracle_query_timeout: Entity<InputState>,
    oracle_application_name: Entity<InputState>,
    oracle_wallet_location: Entity<InputState>,
    oracle_wallet_password: Entity<InputState>,
    mysql_charset: Entity<InputState>,
    mysql_socket_path: Entity<InputState>,
    mysql_collation: Entity<InputState>,
    mysql_timezone: Entity<InputState>,
    mysql_init_sql: Entity<InputState>,
    /// SSH 主机密钥策略下拉（共享给所有支持 SSH 隧道的库）。
    ssh_host_key_policy_select: Entity<SelectState<SearchableVec<ConnectionSelectOption>>>,
    /// 代理类型下拉：自由文本会让拼写错误静默退回 SOCKS5。
    mysql_proxy_type_select: Entity<SelectState<SearchableVec<ConnectionSelectOption>>>,
    mysql_ssh_connect_timeout: Entity<InputState>,
    mysql_ssh_keepalive: Entity<InputState>,
    mysql_proxy_host: Entity<InputState>,
    mysql_proxy_port: Entity<InputState>,
    mysql_proxy_username: Entity<InputState>,
    mysql_proxy_password: Entity<InputState>,
    mysql_connect_timeout: Entity<InputState>,
    mysql_query_timeout: Entity<InputState>,
    sentinel_master_name: Entity<InputState>,
    sentinel_endpoints: Entity<InputState>,
    cluster_start_nodes: Entity<InputState>,
    ai_key_prefixes: Entity<InputState>,
    discovery_uri: Entity<InputState>,
    /// 左侧数据库类型搜索框（仅过滤列表，不参与连接配置）。
    kind_search: Entity<InputState>,
    /// —— MongoDB 专用 ——
    /// 认证机制下拉（默认由驱动协商；X509 / AWS 必须显式指定）。
    mongo_auth_mechanism_select: Entity<SelectState<SearchableVec<ConnectionSelectOption>>>,
    /// SSH 认证方式下拉（SelectState 实体，避免渲染期读 NavicatMain 触发重入 panic）。
    ssh_auth_select: Entity<SelectState<SearchableVec<ConnectionSelectOption>>>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
struct ConnectionSelectOption {
    value: String,
    label_key: &'static str,
}

impl SelectItem for ConnectionSelectOption {
    type Value = String;

    fn title(&self) -> SharedString {
        tr(self.label_key)
    }

    fn value(&self) -> &String {
        &self.value
    }
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
            dameng_schema: String::new(),
            dameng_connect_timeout_secs: "5".to_string(),
            dameng_query_timeout_secs: "0".to_string(),
            dameng_application_name: "FluxDB".to_string(),
            dameng_encoding: String::new(),
            url_params: String::new(),
            sqlite_path: String::new(),
            mongo_srv: false,
            read_only: false,
            mongo_auth_db: String::new(),
            // 空串表示交给驱动协商认证机制。
            mongo_auth_mechanism: String::new(),
            tls_enabled: false,
            tls_ca: String::new(),
            tls_client_cert: String::new(),
            tls_client_key: String::new(),
            tls_client_key_passphrase: String::new(),
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
            ssh_host_key_policy: "strict".to_string(),
            mysql_tls_ssl_mode: "preferred".to_string(),
            mysql_socket_path: String::new(),
            mysql_charset: "utf8mb4".to_string(),
            mysql_collation: String::new(),
            mysql_timezone: String::new(),
            mysql_init_sql: String::new(),
            mysql_ssh_connect_timeout_secs: "0".to_string(),
            // 与档案默认值一致：30 秒心跳，避免 NAT/防火墙静默断开长连接。
            mysql_ssh_keepalive_secs: "30".to_string(),
            mysql_proxy_enabled: false,
            mysql_proxy_type: "socks5".to_string(),
            mysql_proxy_host: String::new(),
            mysql_proxy_port: "1080".to_string(),
            mysql_proxy_username: String::new(),
            mysql_proxy_password: String::new(),
            mysql_connect_timeout_secs: "5".to_string(),
            mysql_query_timeout_secs: "0".to_string(),
            mysql_tcp_keepalive: false,
            pg_tls_ssl_mode: "prefer".to_string(),
            pg_default_schema: String::new(),
            pg_application_name: String::new(),
            pg_connect_timeout_secs: "5".to_string(),
            pg_query_timeout_secs: "0".to_string(),
            pg_timezone: String::new(),
            pg_tcp_keepalive: false,
            pg_show_other_databases: false,
            pg_show_system_schemas: false,
            sqlserver_encryption: "trust_server_certificate".to_string(),
            sqlserver_schema: String::new(),
            sqlserver_instance: String::new(),
            sqlserver_connect_timeout_secs: "5".to_string(),
            sqlserver_query_timeout_secs: "0".to_string(),
            sqlserver_application_name: "FluxDB".to_string(),
            oracle_connect_kind: "service_name".to_string(),
            oracle_privilege: "normal".to_string(),
            oracle_service_name: String::new(),
            oracle_sid: String::new(),
            oracle_schema: String::new(),
            oracle_connect_timeout_secs: "8".to_string(),
            oracle_query_timeout_secs: "0".to_string(),
            oracle_application_name: "FluxDB".to_string(),
            oracle_wallet_location: String::new(),
            oracle_wallet_password: String::new(),
            redis_topology: "standalone".to_string(),
            sentinel_master_name: String::new(),
            sentinel_endpoints: String::new(),
            cluster_start_nodes: String::new(),
            cluster_allow_readonly: false,
            force_standalone: false,
            ai_key_prefixes: String::new(),
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
            DatabaseKind::MariaDb => {
                form.name = format!("MariaDB Local {index}");
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
            DatabaseKind::Postgres => {
                form.name = format!("PostgreSQL Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "5432".to_string();
                form.username = "postgres".to_string();
                form.database = "postgres".to_string();
            }
            DatabaseKind::Kingbase => {
                form.name = format!("KingbaseES Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "54321".to_string();
                form.username = "system".to_string();
                form.database = "test".to_string();
            }
            DatabaseKind::Dameng => {
                form.name = format!("DM8 Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "5236".to_string();
                form.username = "SYSDBA".to_string();
                form.database = "DAMENG".to_string();
                form.dameng_schema = "SYSDBA".to_string();
            }
            DatabaseKind::SqlServer => {
                form.name = format!("SQL Server Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "1433".to_string();
                form.username = "sa".to_string();
            }
            DatabaseKind::Oracle => {
                form.name = format!("Oracle Local {index}");
                form.host = "127.0.0.1".to_string();
                form.port = "1521".to_string();
                form.username = "system".to_string();
                form.oracle_service_name = "FREEPDB1".to_string();
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
        form.read_only = config
            .options
            .get("read_only")
            .is_some_and(|value| value == "true")
            || matches!(
                config.endpoint,
                Endpoint::SqliteFile {
                    read_only: true,
                    ..
                }
            );
        form.mongo_auth_db = config.options.get("auth_db").cloned().unwrap_or_default();
        form.mongo_auth_mechanism = config
            .options
            .get("auth_mechanism")
            .cloned()
            .unwrap_or_default();
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

        // 各数据库的档案回填由表单实现负责；新增数据库不再改这里。
        connection_form_spec(config.kind).apply_to_form(config, &mut form);

        form
    }

    /// 用一份 Oracle 档案回填表单。Service Name 与 SID 分别留在各自字段里，
    /// 由 `oracle_connect_kind` 决定哪一个生效，不合并成「数据库名」。
    fn apply_oracle_profile(&mut self, profile: &fluxdb_core::OracleConnectionProfile) {
        self.host = profile.basic.host.clone();
        self.port = profile.basic.port.to_string();
        self.username = profile.basic.username.clone();
        self.password = profile
            .basic
            .password
            .value()
            .unwrap_or_default()
            .to_string();
        self.oracle_connect_kind = profile.basic.connect_kind.as_str().to_string();
        self.oracle_service_name = profile.basic.service_name.clone();
        self.oracle_sid = profile.basic.sid.clone();
        self.oracle_schema = profile.basic.schema.clone();
        self.oracle_privilege = profile.basic.privilege.as_str().to_string();
        self.oracle_connect_timeout_secs = profile.advanced.connect_timeout_secs.to_string();
        self.oracle_query_timeout_secs = profile.advanced.query_timeout_secs.to_string();
        self.oracle_application_name = profile.advanced.application_name.clone();
        self.oracle_wallet_location = profile.advanced.wallet_location.clone();
        self.oracle_wallet_password = profile
            .advanced
            .wallet_password
            .value()
            .unwrap_or_default()
            .to_string();
        self.apply_ssh_tunnel_profile(&profile.ssh);
    }

    /// 用一份 SQL Server 档案回填表单；档案是拨号与保存的事实来源。
    fn apply_sqlserver_profile(&mut self, profile: &fluxdb_core::SqlServerConnectionProfile) {
        self.host = profile.basic.host.clone();
        self.port = profile.basic.port.to_string();
        self.database = profile.basic.database.clone();
        self.sqlserver_schema = profile.basic.schema.clone();
        self.sqlserver_instance = profile.basic.instance.clone();
        self.username = profile.basic.username.clone();
        self.password = profile
            .basic
            .password
            .value()
            .unwrap_or_default()
            .to_string();
        self.sqlserver_encryption = profile.advanced.encryption.as_str().to_string();
        self.sqlserver_connect_timeout_secs = profile.advanced.connect_timeout_secs.to_string();
        self.sqlserver_query_timeout_secs = profile.advanced.query_timeout_secs.to_string();
        self.sqlserver_application_name = profile.advanced.application_name.clone();
        self.apply_ssh_tunnel_profile(&profile.ssh);
    }

    /// 用一份 PostgreSQL 档案回填表单的 PG 专用编辑值。
    fn apply_postgres_profile(&mut self, profile: &fluxdb_core::PostgresConnectionProfile) {
        // TLS：模式与证书路径共同决定拨号行为，必须一起回填。
        self.tls_enabled = profile.tls.enabled;
        self.tls_ca = profile.tls.ca.key.clone();
        self.tls_client_cert = profile.tls.client_cert.key.clone();
        self.tls_client_key = profile.tls.client_key.key.clone();
        self.tls_client_key_passphrase = profile
            .tls
            .client_key_passphrase
            .value()
            .unwrap_or_default()
            .to_string();
        self.tls_sni = profile.tls.server_name.clone();
        self.pg_tls_ssl_mode = match profile.tls.ssl_mode {
            fluxdb_core::PostgresSslMode::Disabled => "disable".to_string(),
            fluxdb_core::PostgresSslMode::Prefer => "prefer".to_string(),
            fluxdb_core::PostgresSslMode::Require => "require".to_string(),
            fluxdb_core::PostgresSslMode::VerifyCa => "verify-ca".to_string(),
            fluxdb_core::PostgresSslMode::VerifyFull => "verify-full".to_string(),
        };
        self.pg_default_schema = profile.scope.default_schema.clone();
        self.pg_application_name = profile.advanced.application_name.clone();
        self.pg_connect_timeout_secs = profile.advanced.connect_timeout_secs.to_string();
        self.pg_query_timeout_secs = profile.advanced.query_timeout_secs.to_string();
        self.pg_timezone = profile.advanced.timezone.clone();
        self.pg_tcp_keepalive = profile.advanced.tcp_keepalive;
        self.pg_show_other_databases = profile.scope.show_other_databases;
        self.pg_show_system_schemas = profile.scope.show_system_schemas;
        // 维护库回填「数据库」输入框（表单与档案保持同一事实来源；留空则显示空）。
        self.database = profile.basic.maintenance_database.clone();

        // 传输层：SSH / 代理复用共享字段（与 MySQL 表单一致的编辑位置）。
        if let Some(ssh) = profile.transport.iter().find_map(|layer| match layer {
            fluxdb_core::PostgresTransportLayer::Ssh(ssh) => Some(ssh),
            _ => None,
        }) {
            self.ssh_enabled = ssh.enabled;
            self.ssh_host = ssh.host.clone();
            self.ssh_port = ssh.port.to_string();
            self.ssh_username = ssh.username.clone();
            self.ssh_auth = match ssh.auth {
                fluxdb_core::PostgresSshAuth::Password => "password".to_string(),
                fluxdb_core::PostgresSshAuth::PrivateKey => "private_key".to_string(),
            };
            if let Some(password) = ssh.password.inline.as_deref() {
                self.ssh_password = password.to_string();
            }
            self.ssh_private_key = ssh.private_key.key.clone();
            if let Some(passphrase) = ssh.passphrase.inline.as_deref() {
                self.ssh_passphrase = passphrase.to_string();
            }
            self.mysql_ssh_connect_timeout_secs = ssh.connect_timeout_secs.to_string();
            self.mysql_ssh_keepalive_secs = ssh.keepalive_interval_secs.to_string();
            self.ssh_host_key_policy = ssh.host_key_policy.as_str().to_string();
        }
        if let Some(proxy) = profile.transport.iter().find_map(|layer| match layer {
            fluxdb_core::PostgresTransportLayer::Proxy(proxy) => Some(proxy),
            _ => None,
        }) {
            self.mysql_proxy_enabled = proxy.enabled;
            self.mysql_proxy_type = match proxy.proxy_type {
                fluxdb_core::PostgresProxyType::Socks5 => "socks5".to_string(),
                fluxdb_core::PostgresProxyType::HttpConnect => "http_connect".to_string(),
            };
            self.mysql_proxy_host = proxy.host.clone();
            self.mysql_proxy_port = proxy.port.to_string();
            self.mysql_proxy_username = proxy.username.clone();
            if let Some(password) = proxy.password.inline.as_deref() {
                self.mysql_proxy_password = password.to_string();
            }
        }
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
        if let Some(password) = profile.basic.password.inline.as_deref() {
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
        if let Some(password) = profile.ssh.password.inline.as_deref() {
            self.ssh_password = password.to_string();
        }
        self.ssh_private_key = profile.ssh.private_key.key.clone();
        if let Some(passphrase) = profile.ssh.passphrase.inline.as_deref() {
            self.ssh_passphrase = passphrase.to_string();
        }
        self.ssh_host_key_policy = profile.ssh.host_key_policy.as_str().to_string();
        // 拓扑：显式模式，URI 导入 / 编辑旧连接时回填到对应选项。
        self.redis_topology = match profile.topology.kind {
            fluxdb_core::RedisTopologyKind::Standalone => "standalone",
            fluxdb_core::RedisTopologyKind::Sentinel => "sentinel",
            fluxdb_core::RedisTopologyKind::Cluster => "cluster",
        }
        .to_string();
        self.sentinel_master_name = profile.topology.sentinel.master_name.clone();
        self.sentinel_endpoints = profile.topology.sentinel.endpoints.join("\n");
        self.cluster_start_nodes = profile.topology.cluster.start_nodes.join("\n");
        self.cluster_allow_readonly = profile.topology.cluster.allow_readonly;
    }

    /// 用一份 MySQL/TiDB 档案回填表单的所有 MySQL 专用编辑值。
    fn apply_mysql_profile(&mut self, profile: &fluxdb_core::MysqlConnectionProfile) {
        // TLS 模式：旧档案的 verify=true 统一展示为 verify_ca，保存后完成迁移。
        let effective_mode = profile.tls.effective_ssl_mode();
        self.mysql_tls_ssl_mode = effective_mode.as_str().to_string();
        self.mysql_socket_path = profile.basic.socket_path.clone();
        self.mysql_charset = profile.tls.charset.clone();
        self.mysql_collation = profile.tls.collation.clone();
        self.mysql_timezone = profile.advanced.timezone.clone();
        self.mysql_init_sql = profile.advanced.init_sql.clone();
        // TLS 开关与证书/校验位必须和 ssl_mode 一起回填：缺少这些赋值时，
        // 「编辑已有连接再保存」会按表单默认值把 TLS 静默重置为关闭。
        self.tls_enabled = profile.tls.enabled;
        self.tls_ca = profile.tls.ca.key.clone();
        self.tls_client_cert = profile.tls.client_cert.key.clone();
        self.tls_client_key = profile.tls.client_key.key.clone();
        self.tls_sni = profile.tls.sni.clone();
        self.tls_verify = profile.tls.verify;
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
            if let Some(password) = ssh.password.inline.as_deref() {
                self.ssh_password = password.to_string();
            }
            self.ssh_private_key = ssh.private_key.key.clone();
            if let Some(passphrase) = ssh.passphrase.inline.as_deref() {
                self.ssh_passphrase = passphrase.to_string();
            }
            self.mysql_ssh_connect_timeout_secs = ssh.connect_timeout_secs.to_string();
            self.mysql_ssh_keepalive_secs = ssh.keepalive_interval_secs.to_string();
            self.ssh_host_key_policy = ssh.host_key_policy.as_str().to_string();
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
            if let Some(password) = proxy.password.inline.as_deref() {
                self.mysql_proxy_password = password.to_string();
            }
        }
        // 高级超时
        self.mysql_connect_timeout_secs = profile.advanced.connect_timeout_secs.to_string();
        self.mysql_query_timeout_secs = profile.advanced.query_timeout_secs.to_string();
        self.mysql_tcp_keepalive = profile.advanced.tcp_keepalive;
    }

    /// 依据当前编辑值组装一份完整的 Redis 连接档案。
    ///
    /// SecretRef 约定（与 `redis_profile` / storage 保持一致）：
    /// - 密码类（基础密码、SSH 密码、SSH 口令）：`key` 留空，值放 `inline`，
    ///   存储层按 `连接 ID + 凭据类别` 写/读 SQLite 加密凭据表，新建与编辑都正确。
    /// - 证书/私钥文件：以文件路径作 `key`、`inline` 为 None（非密码语义，不落 SQLite 加密凭据表）。
    fn build_redis_profile(&self) -> fluxdb_core::RedisConnectionProfile {
        use fluxdb_core::{
            RedisBasicOptions, RedisCloudOptions, RedisClusterOptions, RedisConnectionProfile,
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
            host_key_policy: fluxdb_core::SshHostKeyPolicy::parse(&self.ssh_host_key_policy),
        };

        // 拓扑不再由用户选择：填了主库名才按哨兵处理；旧档案带回的节点列表
        // 继续按对应模式拨号；其余一律交给连接器自动探测。
        let kind = if !self.sentinel_master_name.trim().is_empty() {
            RedisTopologyKind::Sentinel
        } else if !split_endpoints(&self.cluster_start_nodes).is_empty() {
            RedisTopologyKind::Cluster
        } else {
            RedisTopologyKind::Standalone
        };
        let sentinel_endpoints = if kind == RedisTopologyKind::Sentinel {
            split_endpoints(&self.sentinel_endpoints)
        } else {
            Vec::new()
        };
        let cluster_start_nodes = if kind == RedisTopologyKind::Cluster {
            split_endpoints(&self.cluster_start_nodes)
        } else {
            Vec::new()
        };
        let topology = RedisTopology {
            kind,
            sentinel: RedisSentinelOptions {
                master_name: if kind == RedisTopologyKind::Sentinel {
                    self.sentinel_master_name.trim().to_string()
                } else {
                    String::new()
                },
                endpoints: sentinel_endpoints,
                username: Some(self.username.trim().to_string()).filter(|u| !u.is_empty()),
                password: inline_secret(&self.password),
                tls: tls.clone(),
            },
            cluster: RedisClusterOptions {
                start_nodes: cluster_start_nodes,
                allow_readonly: self.cluster_allow_readonly,
            },
            force_standalone: self.force_standalone,
        };
        // 云自动发现需要 Azure / Redis Cloud 的凭据与 API 调用，当前没有接入；
        // 不落任何猜测出来的云元数据，避免界面展示不存在的发现结果。
        let cloud = RedisCloudOptions::default();

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
    /// `连接 ID + 凭据类别` 写/读 SQLite 加密凭据表）：
    /// - 密码类（基础密码、SSH 密码、SSH 口令、代理密码）：`key` 留空，值放 `inline`。
    /// - 证书/私钥文件（TLS CA/客户端证书/客户端密钥、SSH 私钥）：以文件路径作 `key`、
    ///   `inline` 为 None（非密码语义，不落 SQLite 加密凭据表）。
    fn build_mysql_profile(&self) -> fluxdb_core::MysqlConnectionProfile {
        use fluxdb_core::{
            MysqlAdvancedOptions, MysqlBasicOptions, MysqlConnectionProfile, MysqlProxy,
            MysqlProxyType, MysqlSshAuth, MysqlSshOptions, MysqlSslMode, MysqlTlsOptions,
            MysqlTransportLayer, SecretRef,
        };
        let file_ref = |path: &str| SecretRef::ref_key(path.trim().to_string());
        let inline_secret = |value: &str| SecretRef::inline(value.trim().to_string());

        let ssl_mode = MysqlSslMode::parse(&self.mysql_tls_ssl_mode);
        let tls = MysqlTlsOptions {
            enabled: self.tls_enabled,
            ssl_mode,
            ca: file_ref(&self.tls_ca),
            client_cert: file_ref(&self.tls_client_cert),
            client_key: file_ref(&self.tls_client_key),
            sni: self.tls_sni.trim().to_string(),
            // verify 仅为历史字段，由模式反推，保证旧档案迁移后不再出现"模式与开关不一致"。
            verify: ssl_mode.verifies_certificate(),
            charset: self.mysql_charset.trim().to_string(),
            collation: self.mysql_collation.trim().to_string(),
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
                host_key_policy: fluxdb_core::SshHostKeyPolicy::parse(&self.ssh_host_key_policy),
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
                // 非法端口保留为 0，交给档案校验报错；不再静默回落到默认端口。
                port: self.mysql_proxy_port.trim().parse().unwrap_or(0),
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
                socket_path: self.mysql_socket_path.trim().to_string(),
            },
            tls,
            transport,
            advanced: MysqlAdvancedOptions {
                connect_timeout_secs: self.mysql_connect_timeout_secs.trim().parse().unwrap_or(5),
                query_timeout_secs: self.mysql_query_timeout_secs.trim().parse().unwrap_or(0),
                tcp_keepalive: self.mysql_tcp_keepalive,
                timezone: self.mysql_timezone.trim().to_string(),
                init_sql: self.mysql_init_sql.trim().to_string(),
            },
        }
    }

    /// PostgreSQL 结构化档案：表单里的主机/账号 + TLS 模式 + SSH/代理 + scope/advanced。
    ///
    /// SecretRef 约定与 redis/mysql 一致：密码类放 inline（存储层写 SQLite 加密凭据表），
    /// 证书/私钥以文件路径作 key（非密码语义）。
    fn build_postgres_profile(&self) -> fluxdb_core::PostgresConnectionProfile {
        use fluxdb_core::{
            PostgresAdvancedOptions, PostgresBasicOptions, PostgresProxy, PostgresProxyType,
            PostgresScopeOptions, PostgresSshAuth, PostgresSshOptions, PostgresSslMode,
            PostgresTlsOptions, PostgresTransportLayer, SecretRef,
        };
        let file_ref = |path: &str| SecretRef::ref_key(path.trim().to_string());
        let inline_secret = |value: &str| SecretRef::inline(value.trim().to_string());

        let tls = PostgresTlsOptions {
            enabled: self.tls_enabled,
            ssl_mode: match self.pg_tls_ssl_mode.trim().to_ascii_lowercase().as_str() {
                "disable" | "disabled" => PostgresSslMode::Disabled,
                "require" | "required" => PostgresSslMode::Require,
                "verify-ca" => PostgresSslMode::VerifyCa,
                "verify-full" => PostgresSslMode::VerifyFull,
                // 缺省/非法值按 Prefer（本地/内网默认，兼容无 TLS 部署）。
                _ => PostgresSslMode::Prefer,
            },
            ca: file_ref(&self.tls_ca),
            client_cert: file_ref(&self.tls_client_cert),
            client_key: file_ref(&self.tls_client_key),
            // 空口令不留凭据槽位；口令只走 SecretRef.inline（storage 加密落盘）。
            client_key_passphrase: if self.tls_client_key_passphrase.trim().is_empty() {
                SecretRef::default()
            } else {
                inline_secret(&self.tls_client_key_passphrase)
            },
            server_name: self.tls_sni.trim().to_string(),
        };

        // 传输层：SSH 隧道 / 代理按开关压入，缺省直连。
        let mut transport = vec![PostgresTransportLayer::Direct];
        if self.ssh_enabled {
            transport.insert(
                0,
                PostgresTransportLayer::Ssh(PostgresSshOptions {
                    enabled: true,
                    host: self.ssh_host.trim().to_string(),
                    port: self.ssh_port.trim().parse::<u16>().unwrap_or(22),
                    username: self.ssh_username.trim().to_string(),
                    auth: if self.ssh_auth == "private_key" {
                        PostgresSshAuth::PrivateKey
                    } else {
                        PostgresSshAuth::Password
                    },
                    password: inline_secret(&self.ssh_password),
                    private_key: file_ref(&self.ssh_private_key),
                    passphrase: inline_secret(&self.ssh_passphrase),
                    connect_timeout_secs: self
                        .mysql_ssh_connect_timeout_secs
                        .trim()
                        .parse()
                        .unwrap_or(0),
                    keepalive_interval_secs: self
                        .mysql_ssh_keepalive_secs
                        .trim()
                        .parse()
                        .unwrap_or(0),
                    host_key_policy: fluxdb_core::SshHostKeyPolicy::parse(
                        &self.ssh_host_key_policy,
                    ),
                }),
            );
        }
        if self.mysql_proxy_enabled {
            transport.insert(
                0,
                PostgresTransportLayer::Proxy(PostgresProxy {
                    enabled: true,
                    proxy_type: if self.mysql_proxy_type.trim() == "http_connect" {
                        PostgresProxyType::HttpConnect
                    } else {
                        PostgresProxyType::Socks5
                    },
                    host: self.mysql_proxy_host.trim().to_string(),
                    // 非法端口保留为 0，交给档案校验报错。
                    port: self.mysql_proxy_port.trim().parse().unwrap_or(0),
                    username: self.mysql_proxy_username.trim().to_string(),
                    password: inline_secret(&self.mysql_proxy_password),
                }),
            );
        }

        fluxdb_core::PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: self.host.trim().to_string(),
                // 非法/空端口保留为 0，由档案校验报错；不要静默回落到 5432，
                // 因为同一档案构建器也被 Kingbase（默认 54321）复用。
                port: self.port.trim().parse::<u16>().unwrap_or(0),
                // 维护库：保留表单原文（允许留空）。留空时拨号由
                // `PostgresConnectionProfile::maintenance_database()` 回落 postgres 默认。
                maintenance_database: self.database.trim().to_string(),
                username: self.username.trim().to_string(),
                password: inline_secret(&self.password),
            },
            scope: PostgresScopeOptions {
                default_schema: self.pg_default_schema.trim().to_string(),
                show_other_databases: self.pg_show_other_databases,
                show_system_schemas: self.pg_show_system_schemas,
            },
            tls,
            transport,
            advanced: PostgresAdvancedOptions {
                connect_timeout_secs: self.pg_connect_timeout_secs.trim().parse().unwrap_or(5),
                query_timeout_secs: self.pg_query_timeout_secs.trim().parse().unwrap_or(0),
                // PostgreSQL 连接器不复用连接，空闲 TTL 无消费方，固定 0。
                idle_ttl_secs: 0,
                tcp_keepalive: self.pg_tcp_keepalive,
                application_name: self.pg_application_name.trim().to_string(),
                timezone: self.pg_timezone.trim().to_string(),
            },
        }
    }

    /// SSH 隧道编辑值 → 结构化档案（MongoDB / SQL Server / Oracle / 达梦共用）。
    ///
    /// 密码类放 inline（storage 按连接 ID 写入加密凭据表）；私钥只存文件路径。
    fn build_ssh_tunnel_profile(&self) -> fluxdb_core::SshTunnelProfile {
        // 空值不写成 inline("")，避免 storage 为未填写的密码建空凭据槽。
        let secret = |value: &str| {
            let value = value.trim();
            if value.is_empty() {
                fluxdb_core::SecretRef::default()
            } else {
                fluxdb_core::SecretRef::inline(value.to_string())
            }
        };
        fluxdb_core::SshTunnelProfile {
            enabled: self.ssh_enabled,
            host: self.ssh_host.trim().to_string(),
            port: self.ssh_port.trim().parse::<u16>().unwrap_or(22),
            username: self.ssh_username.trim().to_string(),
            auth: fluxdb_core::SshAuth::parse(&self.ssh_auth),
            password: secret(&self.ssh_password),
            private_key: fluxdb_core::SecretRef::ref_key(self.ssh_private_key.trim().to_string()),
            passphrase: secret(&self.ssh_passphrase),
            connect_timeout_secs: self
                .mysql_ssh_connect_timeout_secs
                .trim()
                .parse()
                .unwrap_or(0),
            keepalive_interval_secs: self.mysql_ssh_keepalive_secs.trim().parse().unwrap_or(0),
            host_key_policy: fluxdb_core::SshHostKeyPolicy::parse(&self.ssh_host_key_policy),
        }
    }

    /// SSH 档案 → 表单编辑值；空档案（未启用且无字段）不改动当前输入。
    fn apply_ssh_tunnel_profile(&mut self, ssh: &fluxdb_core::SshTunnelProfile) {
        if !ssh.enabled && ssh.host.trim().is_empty() {
            return;
        }
        self.ssh_enabled = ssh.enabled;
        self.ssh_host = ssh.host.clone();
        self.ssh_port = ssh.port.to_string();
        self.ssh_username = ssh.username.clone();
        self.ssh_auth = ssh.auth.as_str().to_string();
        if let Some(password) = ssh.password.value() {
            self.ssh_password = password.to_string();
        }
        self.ssh_private_key = ssh.private_key.key.clone();
        if let Some(passphrase) = ssh.passphrase.value() {
            self.ssh_passphrase = passphrase.to_string();
        }
        self.mysql_ssh_connect_timeout_secs = ssh.connect_timeout_secs.to_string();
        self.mysql_ssh_keepalive_secs = ssh.keepalive_interval_secs.to_string();
        self.ssh_host_key_policy = ssh.host_key_policy.as_str().to_string();
    }

    fn build_dameng_profile(&self) -> fluxdb_core::DamengConnectionProfile {
        fluxdb_core::DamengConnectionProfile {
            basic: fluxdb_core::DamengBasicOptions {
                host: self.host.trim().to_string(),
                port: self
                    .port
                    .trim()
                    .parse::<u16>()
                    .unwrap_or_else(|_| database_default_port_u16(DatabaseKind::Dameng)),
                database: self.database.trim().to_string(),
                schema: self.dameng_schema.trim().to_string(),
                username: self.username.trim().to_string(),
                password: fluxdb_core::SecretRef::inline(self.password.trim().to_string()),
            },
            advanced: fluxdb_core::DamengAdvancedOptions {
                // 输入合法性由 validate_new_connection 前置校验；此处兜底默认值。
                connect_timeout_secs: self
                    .dameng_connect_timeout_secs
                    .trim()
                    .parse()
                    .unwrap_or(5),
                query_timeout_secs: self
                    .dameng_query_timeout_secs
                    .trim()
                    .parse()
                    .unwrap_or(0),
                encoding: self.dameng_encoding.trim().to_string(),
                application_name: self.dameng_application_name.trim().to_string(),
            },
            ssh: self.build_ssh_tunnel_profile(),
        }
    }

    fn build_sqlserver_profile(&self) -> fluxdb_core::SqlServerConnectionProfile {
        fluxdb_core::SqlServerConnectionProfile {
            basic: fluxdb_core::SqlServerBasicOptions {
                host: self.host.trim().to_string(),
                port: self
                    .port
                    .trim()
                    .parse::<u16>()
                    .unwrap_or_else(|_| database_default_port_u16(DatabaseKind::SqlServer)),
                database: self.database.trim().to_string(),
                schema: self.sqlserver_schema.trim().to_string(),
                instance: self.sqlserver_instance.trim().to_string(),
                username: self.username.trim().to_string(),
                password: fluxdb_core::SecretRef::inline(self.password.clone()),
            },
            advanced: fluxdb_core::SqlServerAdvancedOptions {
                encryption: fluxdb_core::SqlServerEncryption::parse(&self.sqlserver_encryption)
                    .unwrap_or(fluxdb_core::SqlServerEncryption::TrustServerCertificate),
                connect_timeout_secs: self
                    .sqlserver_connect_timeout_secs
                    .trim()
                    .parse()
                    .unwrap_or(5),
                query_timeout_secs: self
                    .sqlserver_query_timeout_secs
                    .trim()
                    .parse()
                    .unwrap_or(0),
                application_name: self.sqlserver_application_name.trim().to_string(),
            },
            ssh: self.build_ssh_tunnel_profile(),
        }
    }

    /// 依据当前编辑值组装一份 MongoDB 连接档案。
    ///
    /// 主机输入框接受 `host1:port1,host2:port2` 形式的种子列表；SRV 开关只允许一个
    /// DNS 名（多主机时由 `validate()` 报错）。密码走 `SecretRef::inline`，存储层
    /// 按连接 ID 写入加密凭据槽，`Endpoint` 不承载凭据。
    fn build_mongo_profile(&self) -> fluxdb_core::MongoConnectionProfile {
        use fluxdb_core::{
            MongoAdvancedOptions, MongoBasicOptions, MongoConnectionProfile, MongoHost,
            MongoTlsOptions, SecretRef,
        };
        // 独立「端口」输入框是默认端口：host 里显式写了 `:port` 时以显式值为准，
        // 否则用端口框，未填才回落 27017。此前端口框被完全忽略，非默认端口连不上。
        let default_port = self.port.trim().parse::<u16>().unwrap_or(27017);
        let hosts: Vec<MongoHost> = self
            .host
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|item| match item.rsplit_once(':') {
                Some((host, port)) if port.parse::<u16>().is_ok() => MongoHost {
                    host: host.trim_matches(['[', ']']).to_string(),
                    port: port.parse().unwrap_or(default_port),
                },
                _ => MongoHost {
                    host: item.trim_matches(['[', ']']).to_string(),
                    port: default_port,
                },
            })
            .collect();
        let file_ref = |path: &str| SecretRef::ref_key(path.trim().to_string());
        MongoConnectionProfile {
            srv: self.mongo_srv,
            basic: MongoBasicOptions {
                hosts,
                username: self.username.trim().to_string(),
                // 空密码不写成 `inline("")`：否则 `validate()` 会误判「已填密码」。
                password: if self.password.is_empty() {
                    SecretRef::default()
                } else {
                    SecretRef::inline(self.password.clone())
                },
                default_database: self.database.trim().to_string(),
                auth_database: self.mongo_auth_db.trim().to_string(),
                auth_mechanism: self.mongo_auth_mechanism.trim().to_string(),
            },
            tls: MongoTlsOptions {
                enabled: self.tls_enabled,
                ca: file_ref(&self.tls_ca),
                client_cert: file_ref(&self.tls_client_cert),
                allow_invalid_certificates: !self.tls_verify,
                allow_invalid_hostnames: false,
                server_name: self.tls_sni.trim().to_string(),
            },
            advanced: MongoAdvancedOptions::default(),
            ssh: self.build_ssh_tunnel_profile(),
        }
    }

    /// 用一份 MongoDB 档案回填表单（连接信息、TLS、认证库为编辑态事实来源）。
    fn apply_mongo_profile(&mut self, profile: &fluxdb_core::MongoConnectionProfile) {
        self.mongo_srv = profile.srv;
        // 种子列表按 `host:port` 拼回单一输入框；非默认端口才带端口，避免噪声。
        self.host = profile
            .basic
            .hosts
            .iter()
            .map(|host| {
                if host.port == 27017 {
                    host.host.clone()
                } else {
                    format!("{}:{}", host.host, host.port)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        if let Some((_, port)) = profile.basic.hosts.first().map(|h| (h.host.clone(), h.port)) {
            self.port = port.to_string();
        }
        self.username = profile.basic.username.clone();
        if let Some(password) = profile.basic.password.inline.as_deref() {
            self.password = password.to_string();
        }
        self.database = profile.basic.default_database.clone();
        self.mongo_auth_db = profile.basic.auth_database.clone();
        self.mongo_auth_mechanism = profile.basic.auth_mechanism.clone();
        self.tls_enabled = profile.tls.enabled;
        self.tls_ca = profile.tls.ca.key.clone();
        self.tls_client_cert = profile.tls.client_cert.key.clone();
        self.tls_sni = profile.tls.server_name.clone();
        self.tls_verify = !profile.tls.allow_invalid_certificates;
        self.apply_ssh_tunnel_profile(&profile.ssh);
    }

    /// 当前生效的 Oracle 寻址目标（Service Name 或 SID）。
    fn oracle_target_name(&self) -> &str {
        match fluxdb_core::OracleConnectKind::parse(&self.oracle_connect_kind) {
            Some(fluxdb_core::OracleConnectKind::Sid) => self.oracle_sid.trim(),
            _ => self.oracle_service_name.trim(),
        }
    }

    fn build_oracle_profile(&self) -> fluxdb_core::OracleConnectionProfile {
        fluxdb_core::OracleConnectionProfile {
            basic: fluxdb_core::OracleBasicOptions {
                host: self.host.trim().to_string(),
                port: self
                    .port
                    .trim()
                    .parse::<u16>()
                    .unwrap_or_else(|_| database_default_port_u16(DatabaseKind::Oracle)),
                connect_kind: fluxdb_core::OracleConnectKind::parse(&self.oracle_connect_kind)
                    .unwrap_or(fluxdb_core::OracleConnectKind::ServiceName),
                service_name: self.oracle_service_name.trim().to_string(),
                sid: self.oracle_sid.trim().to_string(),
                username: self.username.trim().to_string(),
                schema: self.oracle_schema.trim().to_string(),
                privilege: fluxdb_core::OraclePrivilege::parse(&self.oracle_privilege)
                    .unwrap_or(fluxdb_core::OraclePrivilege::Normal),
                password: fluxdb_core::SecretRef::inline(self.password.clone()),
            },
            advanced: fluxdb_core::OracleAdvancedOptions {
                connect_timeout_secs: self
                    .oracle_connect_timeout_secs
                    .trim()
                    .parse()
                    .unwrap_or(8),
                query_timeout_secs: self.oracle_query_timeout_secs.trim().parse().unwrap_or(0),
                application_name: self.oracle_application_name.trim().to_string(),
                wallet_location: self.oracle_wallet_location.trim().to_string(),
                wallet_password: fluxdb_core::SecretRef::inline(
                    self.oracle_wallet_password.clone(),
                ),
            },
            ssh: self.build_ssh_tunnel_profile(),
        }
    }
}

impl NewConnectionInputs {
    /// 输入框 placeholder 在创建时缓存；切换语言或数据库类型后更新，保留已输入内容。
    fn refresh_placeholders(
        &self,
        kind: Option<DatabaseKind>,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) {
        let inputs = [
            (&self.name, ConnectionField::Name),
            (&self.host, ConnectionField::Host),
            (&self.port, ConnectionField::Port),
            (&self.username, ConnectionField::Username),
            (&self.password, ConnectionField::Password),
            (&self.database, ConnectionField::Database),
            (&self.dameng_schema, ConnectionField::DamengSchema),
            (
                &self.dameng_connect_timeout,
                ConnectionField::DamengConnectTimeout,
            ),
            (
                &self.dameng_query_timeout,
                ConnectionField::DamengQueryTimeout,
            ),
            (
                &self.dameng_application_name,
                ConnectionField::DamengApplicationName,
            ),
            (&self.url_params, ConnectionField::UrlParams),
            (&self.sqlite_path, ConnectionField::SqlitePath),
            (&self.mongo_auth_db, ConnectionField::MongoAuthDb),
            (&self.tls_ca, ConnectionField::TlsCa),
            (&self.tls_client_cert, ConnectionField::TlsClientCert),
            (&self.tls_client_key, ConnectionField::TlsClientKey),
            (
                &self.tls_client_key_passphrase,
                ConnectionField::TlsClientKeyPassphrase,
            ),
            (&self.tls_sni, ConnectionField::TlsSni),
            (&self.ssh_host, ConnectionField::SshHost),
            (&self.ssh_port, ConnectionField::SshPort),
            (&self.ssh_username, ConnectionField::SshUsername),
            (&self.ssh_password, ConnectionField::SshPassword),
            (&self.ssh_private_key, ConnectionField::SshPrivateKey),
            (&self.ssh_passphrase, ConnectionField::SshPassphrase),
            (&self.pg_default_schema, ConnectionField::PgDefaultSchema),
            (
                &self.pg_application_name,
                ConnectionField::PgApplicationName,
            ),
            (
                &self.pg_connect_timeout,
                ConnectionField::PgConnectTimeoutSecs,
            ),
            (&self.pg_query_timeout, ConnectionField::PgQueryTimeoutSecs),
            (&self.pg_timezone, ConnectionField::PgTimezone),
            (&self.sqlserver_schema, ConnectionField::SqlServerSchema),
            (
                &self.sqlserver_instance,
                ConnectionField::SqlServerInstance,
            ),
            (
                &self.sqlserver_connect_timeout,
                ConnectionField::SqlServerConnectTimeout,
            ),
            (
                &self.sqlserver_query_timeout,
                ConnectionField::SqlServerQueryTimeout,
            ),
            (
                &self.sqlserver_application_name,
                ConnectionField::SqlServerApplicationName,
            ),
            (
                &self.oracle_service_name,
                ConnectionField::OracleServiceName,
            ),
            (&self.oracle_sid, ConnectionField::OracleSid),
            (&self.oracle_schema, ConnectionField::OracleSchema),
            (
                &self.oracle_connect_timeout,
                ConnectionField::OracleConnectTimeout,
            ),
            (&self.oracle_query_timeout, ConnectionField::OracleQueryTimeout),
            (
                &self.oracle_application_name,
                ConnectionField::OracleApplicationName,
            ),
            (
                &self.oracle_wallet_location,
                ConnectionField::OracleWalletLocation,
            ),
            (
                &self.oracle_wallet_password,
                ConnectionField::OracleWalletPassword,
            ),
            (&self.mysql_charset, ConnectionField::MysqlCharset),
            (&self.mysql_collation, ConnectionField::MysqlCollation),
            (&self.mysql_socket_path, ConnectionField::MysqlSocketPath),
            (&self.mysql_timezone, ConnectionField::MysqlTimezone),
            (&self.mysql_init_sql, ConnectionField::MysqlInitSql),
            (
                &self.mysql_ssh_connect_timeout,
                ConnectionField::MysqlSshConnectTimeout,
            ),
            (
                &self.mysql_ssh_keepalive,
                ConnectionField::MysqlSshKeepalive,
            ),
            (&self.mysql_proxy_host, ConnectionField::MysqlProxyHost),
            (&self.mysql_proxy_port, ConnectionField::MysqlProxyPort),
            (
                &self.mysql_proxy_username,
                ConnectionField::MysqlProxyUsername,
            ),
            (
                &self.mysql_proxy_password,
                ConnectionField::MysqlProxyPassword,
            ),
            (
                &self.mysql_connect_timeout,
                ConnectionField::MysqlConnectTimeout,
            ),
            (
                &self.mysql_query_timeout,
                ConnectionField::MysqlQueryTimeout,
            ),
            (
                &self.sentinel_master_name,
                ConnectionField::SentinelMasterName,
            ),
            (&self.sentinel_endpoints, ConnectionField::SentinelEndpoints),
            (
                &self.cluster_start_nodes,
                ConnectionField::ClusterStartNodes,
            ),
            (&self.ai_key_prefixes, ConnectionField::AiKeyPrefixes),
        ];
        for (input, field) in inputs {
            input.update(cx, |input, cx| {
                input.set_placeholder(connection_field_placeholder(field), window, cx);
            });
        }
        // 连接串示例随数据库类型变化：PostgreSQL 显示 postgres://，不要继续展示 MySQL 示例。
        self.discovery_uri.update(cx, |input, cx| {
            input.set_placeholder(discovery_uri_placeholder(kind), window, cx);
        });
    }

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
        let (dameng_schema, dameng_schema_subscription) =
            input_state(ConnectionField::DamengSchema, false, window, cx);
        // —— 达梦专用文本输入 ——
        let (dameng_connect_timeout, dameng_connect_timeout_subscription) =
            input_state(ConnectionField::DamengConnectTimeout, false, window, cx);
        let (dameng_query_timeout, dameng_query_timeout_subscription) =
            input_state(ConnectionField::DamengQueryTimeout, false, window, cx);
        let (dameng_application_name, dameng_application_name_subscription) =
            input_state(ConnectionField::DamengApplicationName, false, window, cx);
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
        let (tls_client_key_passphrase, tls_client_key_passphrase_subscription) =
            input_state(ConnectionField::TlsClientKeyPassphrase, true, window, cx);
        let (tls_sni, tls_sni_subscription) =
            input_state(ConnectionField::TlsSni, false, window, cx);
        let (ssh_host, ssh_host_subscription) =
            input_state(ConnectionField::SshHost, false, window, cx);
        let (ssh_port, ssh_port_subscription) =
            input_state(ConnectionField::SshPort, false, window, cx);
        let (ssh_username, ssh_username_subscription) =
            input_state(ConnectionField::SshUsername, false, window, cx);
        let (ssh_password, ssh_password_subscription) =
            input_state(ConnectionField::SshPassword, true, window, cx);
        let (ssh_private_key, ssh_private_key_subscription) =
            input_state(ConnectionField::SshPrivateKey, false, window, cx);
        let (ssh_passphrase, ssh_passphrase_subscription) =
            input_state(ConnectionField::SshPassphrase, true, window, cx);
        // —— PostgreSQL 专用文本输入 ——
        let (pg_default_schema, pg_default_schema_subscription) =
            input_state(ConnectionField::PgDefaultSchema, false, window, cx);
        let (pg_application_name, pg_application_name_subscription) =
            input_state(ConnectionField::PgApplicationName, false, window, cx);
        let (pg_connect_timeout, pg_connect_timeout_subscription) =
            input_state(ConnectionField::PgConnectTimeoutSecs, false, window, cx);
        let (pg_query_timeout, pg_query_timeout_subscription) =
            input_state(ConnectionField::PgQueryTimeoutSecs, false, window, cx);
        let (pg_timezone, pg_timezone_subscription) =
            input_state(ConnectionField::PgTimezone, false, window, cx);
        // —— SQL Server 专用文本输入 ——
        let (sqlserver_schema, sqlserver_schema_subscription) =
            input_state(ConnectionField::SqlServerSchema, false, window, cx);
        let (sqlserver_instance, sqlserver_instance_subscription) =
            input_state(ConnectionField::SqlServerInstance, false, window, cx);
        let (sqlserver_connect_timeout, sqlserver_connect_timeout_subscription) =
            input_state(ConnectionField::SqlServerConnectTimeout, false, window, cx);
        let (sqlserver_query_timeout, sqlserver_query_timeout_subscription) =
            input_state(ConnectionField::SqlServerQueryTimeout, false, window, cx);
        let (sqlserver_application_name, sqlserver_application_name_subscription) =
            input_state(ConnectionField::SqlServerApplicationName, false, window, cx);
        // —— Oracle 专用文本输入 ——
        let (oracle_service_name, oracle_service_name_subscription) =
            input_state(ConnectionField::OracleServiceName, false, window, cx);
        let (oracle_sid, oracle_sid_subscription) =
            input_state(ConnectionField::OracleSid, false, window, cx);
        let (oracle_schema, oracle_schema_subscription) =
            input_state(ConnectionField::OracleSchema, false, window, cx);
        let (oracle_connect_timeout, oracle_connect_timeout_subscription) =
            input_state(ConnectionField::OracleConnectTimeout, false, window, cx);
        let (oracle_query_timeout, oracle_query_timeout_subscription) =
            input_state(ConnectionField::OracleQueryTimeout, false, window, cx);
        let (oracle_application_name, oracle_application_name_subscription) =
            input_state(ConnectionField::OracleApplicationName, false, window, cx);
        let (oracle_wallet_location, oracle_wallet_location_subscription) =
            input_state(ConnectionField::OracleWalletLocation, false, window, cx);
        let (oracle_wallet_password, oracle_wallet_password_subscription) =
            input_state(ConnectionField::OracleWalletPassword, true, window, cx);
        // —— MySQL / TiDB 专用文本输入 ——
        let (mysql_charset, mysql_charset_subscription) =
            input_state(ConnectionField::MysqlCharset, false, window, cx);
        let (mysql_collation, mysql_collation_subscription) =
            input_state(ConnectionField::MysqlCollation, false, window, cx);
        let (mysql_socket_path, mysql_socket_path_subscription) =
            input_state(ConnectionField::MysqlSocketPath, false, window, cx);
        let (mysql_timezone, mysql_timezone_subscription) =
            input_state(ConnectionField::MysqlTimezone, false, window, cx);
        let (mysql_init_sql, mysql_init_sql_subscription) =
            input_state(ConnectionField::MysqlInitSql, false, window, cx);
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
        let (sentinel_master_name, sentinel_master_name_subscription) =
            input_state(ConnectionField::SentinelMasterName, false, window, cx);
        let (sentinel_endpoints, sentinel_endpoints_subscription) =
            input_state(ConnectionField::SentinelEndpoints, false, window, cx);
        let (cluster_start_nodes, cluster_start_nodes_subscription) =
            input_state(ConnectionField::ClusterStartNodes, false, window, cx);
        let (ai_key_prefixes, ai_key_prefixes_subscription) =
            input_state(ConnectionField::AiKeyPrefixes, false, window, cx);
        let (discovery_uri, discovery_uri_s) =
            input_state(ConnectionField::DiscoveryUri, false, window, cx);
        // 左侧类型搜索：只影响列表过滤，不进入连接配置，因此不参与表单同步。
        let kind_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("app.connection.search_kind"))
        });
        let kind_search_s = cx.subscribe(&kind_search, |_, _, _: &InputEvent, cx| {
            cx.notify();
        });

        // SSH 认证方式下拉：选项为展示文案，订阅回调里映射回表单字段值。
        let ssh_auth_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ConnectionSelectOption {
                        value: "password".into(),
                        label_key: "app.connection.secret.password",
                    },
                    ConnectionSelectOption {
                        value: "private_key".into(),
                        label_key: "app.connection.secret.private_key",
                    },
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let ssh_auth_select_s = cx.subscribe(
            &ssh_auth_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<ConnectionSelectOption>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.ssh_auth = match value.value().as_str() {
                        "private_key" => "private_key".to_string(),
                        _ => "password".to_string(),
                    };
                    cx.notify();
                }
            },
        );
        // SSH 主机密钥策略：默认严格校验 known_hosts；其余两档需用户显式选择。
        let ssh_host_key_policy_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ConnectionSelectOption {
                        value: "strict".into(),
                        label_key: "app.connection.ssh.host_key_policy_strict",
                    },
                    ConnectionSelectOption {
                        value: "accept_new".into(),
                        label_key: "app.connection.ssh.host_key_policy_accept_new",
                    },
                    ConnectionSelectOption {
                        value: "ignore".into(),
                        label_key: "app.connection.ssh.host_key_policy_ignore",
                    },
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let ssh_host_key_policy_select_s = cx.subscribe(
            &ssh_host_key_policy_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<ConnectionSelectOption>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.ssh_host_key_policy = value.value().clone();
                    cx.notify();
                }
            },
        );
        // PG SSL 模式下拉：选项值即表单值（disable/prefer/require/verify-ca/verify-full）。
        let pg_ssl_mode_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    "disable".to_string(),
                    "prefer".to_string(),
                    "require".to_string(),
                    "verify-ca".to_string(),
                    "verify-full".to_string(),
                ]),
                Some(IndexPath::new(1)),
                window,
                cx,
            )
        });
        let pg_ssl_mode_select_s = cx.subscribe(
            &pg_ssl_mode_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<String>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.pg_tls_ssl_mode = value.clone();
                    cx.notify();
                }
            },
        );
        // MySQL/TiDB SSL 模式下拉：选项值即表单值，与 MySQL 官方 --ssl-mode 五档一致。
        let mysql_ssl_mode_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    "disabled".to_string(),
                    "preferred".to_string(),
                    "required".to_string(),
                    "verify_ca".to_string(),
                    "verify_identity".to_string(),
                ]),
                Some(IndexPath::new(1)),
                window,
                cx,
            )
        });
        let mysql_ssl_mode_select_s = cx.subscribe(
            &mysql_ssl_mode_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<String>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.mysql_tls_ssl_mode = value.clone();
                    cx.notify();
                }
            },
        );
        // MySQL/TiDB 代理类型下拉：只有这两种取值会被连接器识别，
        // 用下拉代替自由文本，避免拼写差异被静默解释成 SOCKS5。
        let mysql_proxy_type_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ConnectionSelectOption {
                        value: "socks5".into(),
                        label_key: "app.connection.mysql.proxy_type_socks5",
                    },
                    ConnectionSelectOption {
                        value: "http_connect".into(),
                        label_key: "app.connection.mysql.proxy_type_http_connect",
                    },
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let mysql_proxy_type_select_s = cx.subscribe(
            &mysql_proxy_type_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<ConnectionSelectOption>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.mysql_proxy_type = value.value().clone();
                    cx.notify();
                }
            },
        );

        // SQL Server 加密模式：默认信任服务器证书（自签部署常见），仍要求 TLS。
        let sqlserver_encryption_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ConnectionSelectOption {
                        value: "disable".into(),
                        label_key: "app.connection.sqlserver.encryption_disable",
                    },
                    ConnectionSelectOption {
                        value: "require".into(),
                        label_key: "app.connection.sqlserver.encryption_require",
                    },
                    ConnectionSelectOption {
                        value: "trust_server_certificate".into(),
                        label_key: "app.connection.sqlserver.encryption_trust",
                    },
                ]),
                Some(IndexPath::new(2)),
                window,
                cx,
            )
        });
        // MongoDB 认证机制：默认交驱动协商；X509/AWS 等必须显式指定。
        let mongo_auth_mechanism_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ConnectionSelectOption {
                        value: String::new(),
                        label_key: "app.connection.mongo.auth_default",
                    },
                    ConnectionSelectOption {
                        value: "SCRAM-SHA-256".into(),
                        label_key: "app.connection.mongo.auth_scram256",
                    },
                    ConnectionSelectOption {
                        value: "SCRAM-SHA-1".into(),
                        label_key: "app.connection.mongo.auth_scram1",
                    },
                    ConnectionSelectOption {
                        value: "MONGODB-X509".into(),
                        label_key: "app.connection.mongo.auth_x509",
                    },
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let mongo_auth_mechanism_select_s = cx.subscribe(
            &mongo_auth_mechanism_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<ConnectionSelectOption>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.mongo_auth_mechanism = value.value().clone();
                    cx.notify();
                }
            },
        );

        let sqlserver_encryption_select_s = cx.subscribe(
            &sqlserver_encryption_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<ConnectionSelectOption>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.sqlserver_encryption = value.value().clone();
                    cx.notify();
                }
            },
        );

        // Oracle 寻址方式：Service Name（Easy Connect）与 SID 互斥，默认前者。
        let oracle_connect_kind_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ConnectionSelectOption {
                        value: "service_name".into(),
                        label_key: "app.connection.oracle.connect_kind_service",
                    },
                    ConnectionSelectOption {
                        value: "sid".into(),
                        label_key: "app.connection.oracle.connect_kind_sid",
                    },
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let oracle_connect_kind_select_s = cx.subscribe(
            &oracle_connect_kind_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<ConnectionSelectOption>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.oracle_connect_kind = value.value().clone();
                    cx.notify();
                }
            },
        );
        // Oracle 登录身份：默认普通连接，特权身份必须显式选择。
        let oracle_privilege_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ConnectionSelectOption {
                        value: "normal".into(),
                        label_key: "app.connection.oracle.privilege_normal",
                    },
                    ConnectionSelectOption {
                        value: "sysdba".into(),
                        label_key: "app.connection.oracle.privilege_sysdba",
                    },
                    ConnectionSelectOption {
                        value: "sysoper".into(),
                        label_key: "app.connection.oracle.privilege_sysoper",
                    },
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let oracle_privilege_select_s = cx.subscribe(
            &oracle_privilege_select,
            move |this: &mut NavicatMain,
                  _select,
                  event: &SelectEvent<SearchableVec<ConnectionSelectOption>>,
                  cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(value) = value {
                    this.new_connection_form.oracle_privilege = value.value().clone();
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
            dameng_schema,
            dameng_connect_timeout,
            dameng_query_timeout,
            dameng_application_name,
            url_params,
            sqlite_path,
            mongo_auth_db,
            tls_ca,
            tls_client_cert,
            tls_client_key,
            tls_client_key_passphrase,
            tls_sni,
            ssh_host,
            ssh_port,
            ssh_username,
            ssh_password,
            ssh_private_key,
            ssh_passphrase,
            pg_ssl_mode_select,
            pg_default_schema,
            pg_application_name,
            pg_connect_timeout,
            pg_query_timeout,
            pg_timezone,
            sqlserver_encryption_select,
            sqlserver_schema,
            sqlserver_instance,
            sqlserver_connect_timeout,
            sqlserver_query_timeout,
            sqlserver_application_name,
            oracle_connect_kind_select,
            oracle_privilege_select,
            oracle_service_name,
            oracle_sid,
            oracle_schema,
            oracle_connect_timeout,
            oracle_query_timeout,
            oracle_application_name,
            oracle_wallet_location,
            oracle_wallet_password,
            mysql_ssl_mode_select,
            mysql_charset,
            mysql_collation,
            mysql_socket_path,
            mysql_timezone,
            mysql_init_sql,
            ssh_host_key_policy_select,
            mysql_proxy_type_select,
            mysql_ssh_connect_timeout,
            mysql_ssh_keepalive,
            mysql_proxy_host,
            mysql_proxy_port,
            mysql_proxy_username,
            mysql_proxy_password,
            mysql_connect_timeout,
            mysql_query_timeout,
            sentinel_master_name,
            sentinel_endpoints,
            cluster_start_nodes,
            ai_key_prefixes,
            discovery_uri,
            kind_search,
            mongo_auth_mechanism_select,
            ssh_auth_select,
            _subscriptions: vec![
                name_subscription,
                host_subscription,
                port_subscription,
                username_subscription,
                password_subscription,
                database_subscription,
                dameng_schema_subscription,
                dameng_connect_timeout_subscription,
                dameng_query_timeout_subscription,
                dameng_application_name_subscription,
                url_params_subscription,
                sqlite_path_subscription,
                mongo_auth_db_subscription,
                tls_ca_subscription,
                tls_client_cert_subscription,
                tls_client_key_subscription,
                tls_client_key_passphrase_subscription,
                tls_sni_subscription,
                ssh_host_subscription,
                ssh_port_subscription,
                ssh_username_subscription,
                ssh_password_subscription,
                ssh_private_key_subscription,
                ssh_passphrase_subscription,
                pg_ssl_mode_select_s,
                pg_default_schema_subscription,
                pg_application_name_subscription,
                pg_connect_timeout_subscription,
                pg_query_timeout_subscription,
                pg_timezone_subscription,
                sqlserver_encryption_select_s,
                sqlserver_schema_subscription,
                sqlserver_instance_subscription,
                sqlserver_connect_timeout_subscription,
                sqlserver_query_timeout_subscription,
                sqlserver_application_name_subscription,
                mysql_ssl_mode_select_s,
                mysql_proxy_type_select_s,
                mysql_charset_subscription,
                mysql_collation_subscription,
                mysql_socket_path_subscription,
                mysql_timezone_subscription,
                mysql_init_sql_subscription,
                ssh_host_key_policy_select_s,
                mysql_ssh_connect_timeout_subscription,
                mysql_ssh_keepalive_subscription,
                mysql_proxy_host_subscription,
                mysql_proxy_port_subscription,
                mysql_proxy_username_subscription,
                mysql_proxy_password_subscription,
                mysql_connect_timeout_subscription,
                mysql_query_timeout_subscription,
                sentinel_master_name_subscription,
                sentinel_endpoints_subscription,
                cluster_start_nodes_subscription,
                ai_key_prefixes_subscription,
                discovery_uri_s,
                kind_search_s,
                mongo_auth_mechanism_select_s,
                ssh_auth_select_s,
                oracle_service_name_subscription,
                oracle_sid_subscription,
                oracle_schema_subscription,
                oracle_connect_timeout_subscription,
                oracle_query_timeout_subscription,
                oracle_application_name_subscription,
                oracle_wallet_location_subscription,
                oracle_wallet_password_subscription,
                oracle_connect_kind_select_s,
                oracle_privilege_select_s,
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
            ConnectionField::DamengSchema => &self.dameng_schema,
            ConnectionField::DamengConnectTimeout => &self.dameng_connect_timeout,
            ConnectionField::DamengQueryTimeout => &self.dameng_query_timeout,
            ConnectionField::DamengApplicationName => &self.dameng_application_name,
            ConnectionField::UrlParams => &self.url_params,
            ConnectionField::SqlitePath => &self.sqlite_path,
            ConnectionField::MongoAuthDb => &self.mongo_auth_db,
            ConnectionField::TlsCa => &self.tls_ca,
            ConnectionField::TlsClientCert => &self.tls_client_cert,
            ConnectionField::TlsClientKey => &self.tls_client_key,
            ConnectionField::TlsClientKeyPassphrase => &self.tls_client_key_passphrase,
            ConnectionField::TlsSni => &self.tls_sni,
            ConnectionField::SshHost => &self.ssh_host,
            ConnectionField::SshPort => &self.ssh_port,
            ConnectionField::SshUsername => &self.ssh_username,
            ConnectionField::SshPassword => &self.ssh_password,
            ConnectionField::SshPrivateKey => &self.ssh_private_key,
            ConnectionField::SshPassphrase => &self.ssh_passphrase,
            ConnectionField::PgDefaultSchema => &self.pg_default_schema,
            ConnectionField::PgApplicationName => &self.pg_application_name,
            ConnectionField::PgConnectTimeoutSecs => &self.pg_connect_timeout,
            ConnectionField::PgQueryTimeoutSecs => &self.pg_query_timeout,
            ConnectionField::PgTimezone => &self.pg_timezone,
            ConnectionField::SqlServerSchema => &self.sqlserver_schema,
            ConnectionField::SqlServerInstance => &self.sqlserver_instance,
            ConnectionField::SqlServerConnectTimeout => &self.sqlserver_connect_timeout,
            ConnectionField::SqlServerQueryTimeout => &self.sqlserver_query_timeout,
            ConnectionField::SqlServerApplicationName => &self.sqlserver_application_name,
            ConnectionField::OracleServiceName => &self.oracle_service_name,
            ConnectionField::OracleSid => &self.oracle_sid,
            ConnectionField::OracleSchema => &self.oracle_schema,
            ConnectionField::OracleConnectTimeout => &self.oracle_connect_timeout,
            ConnectionField::OracleQueryTimeout => &self.oracle_query_timeout,
            ConnectionField::OracleApplicationName => &self.oracle_application_name,
            ConnectionField::OracleWalletLocation => &self.oracle_wallet_location,
            ConnectionField::OracleWalletPassword => &self.oracle_wallet_password,
            ConnectionField::MysqlCharset => &self.mysql_charset,
            ConnectionField::MysqlCollation => &self.mysql_collation,
            ConnectionField::MysqlSocketPath => &self.mysql_socket_path,
            ConnectionField::MysqlTimezone => &self.mysql_timezone,
            ConnectionField::MysqlInitSql => &self.mysql_init_sql,
            ConnectionField::MysqlSshConnectTimeout => &self.mysql_ssh_connect_timeout,
            ConnectionField::MysqlSshKeepalive => &self.mysql_ssh_keepalive,
            ConnectionField::MysqlProxyHost => &self.mysql_proxy_host,
            ConnectionField::MysqlProxyPort => &self.mysql_proxy_port,
            ConnectionField::MysqlProxyUsername => &self.mysql_proxy_username,
            ConnectionField::MysqlProxyPassword => &self.mysql_proxy_password,
            ConnectionField::MysqlConnectTimeout => &self.mysql_connect_timeout,
            ConnectionField::MysqlQueryTimeout => &self.mysql_query_timeout,
            ConnectionField::SentinelMasterName => &self.sentinel_master_name,
            ConnectionField::AiKeyPrefixes => &self.ai_key_prefixes,
            ConnectionField::SentinelEndpoints => &self.sentinel_endpoints,
            ConnectionField::ClusterStartNodes => &self.cluster_start_nodes,
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
        self.set_value(
            ConnectionField::DamengSchema,
            &form.dameng_schema,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::DamengConnectTimeout,
            &form.dameng_connect_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::DamengQueryTimeout,
            &form.dameng_query_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::DamengApplicationName,
            &form.dameng_application_name,
            window,
            cx,
        );
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
        self.set_value(
            ConnectionField::TlsClientCert,
            &form.tls_client_cert,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::TlsClientKey,
            &form.tls_client_key,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::TlsClientKeyPassphrase,
            &form.tls_client_key_passphrase,
            window,
            cx,
        );
        self.set_value(ConnectionField::TlsSni, &form.tls_sni, window, cx);
        self.set_value(ConnectionField::SshHost, &form.ssh_host, window, cx);
        self.set_value(ConnectionField::SshPort, &form.ssh_port, window, cx);
        self.set_value(ConnectionField::SshUsername, &form.ssh_username, window, cx);
        self.set_value(ConnectionField::SshPassword, &form.ssh_password, window, cx);
        self.set_value(
            ConnectionField::SshPrivateKey,
            &form.ssh_private_key,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::SshPassphrase,
            &form.ssh_passphrase,
            window,
            cx,
        );
        // —— PostgreSQL 专用 ——
        self.set_value(
            ConnectionField::PgDefaultSchema,
            &form.pg_default_schema,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::PgApplicationName,
            &form.pg_application_name,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::PgConnectTimeoutSecs,
            &form.pg_connect_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::PgQueryTimeoutSecs,
            &form.pg_query_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::PgTimezone,
            &form.pg_timezone,
            window,
            cx,
        );
        // —— SQL Server 专用 ——
        self.set_value(
            ConnectionField::SqlServerSchema,
            &form.sqlserver_schema,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::SqlServerInstance,
            &form.sqlserver_instance,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::SqlServerConnectTimeout,
            &form.sqlserver_connect_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::SqlServerQueryTimeout,
            &form.sqlserver_query_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::SqlServerApplicationName,
            &form.sqlserver_application_name,
            window,
            cx,
        );
        // —— Oracle 专用 ——
        self.set_value(
            ConnectionField::OracleServiceName,
            &form.oracle_service_name,
            window,
            cx,
        );
        self.set_value(ConnectionField::OracleSid, &form.oracle_sid, window, cx);
        self.set_value(
            ConnectionField::OracleSchema,
            &form.oracle_schema,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::OracleConnectTimeout,
            &form.oracle_connect_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::OracleQueryTimeout,
            &form.oracle_query_timeout_secs,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::OracleApplicationName,
            &form.oracle_application_name,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::OracleWalletLocation,
            &form.oracle_wallet_location,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::OracleWalletPassword,
            &form.oracle_wallet_password,
            window,
            cx,
        );
        // —— MySQL / TiDB 专用 ——
        self.set_value(
            ConnectionField::MysqlCharset,
            &form.mysql_charset,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlCollation,
            &form.mysql_collation,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlSocketPath,
            &form.mysql_socket_path,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlTimezone,
            &form.mysql_timezone,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlInitSql,
            &form.mysql_init_sql,
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
        self.set_value(
            ConnectionField::MysqlProxyHost,
            &form.mysql_proxy_host,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::MysqlProxyPort,
            &form.mysql_proxy_port,
            window,
            cx,
        );
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
            ConnectionField::AiKeyPrefixes,
            &form.ai_key_prefixes,
            window,
            cx,
        );
        self.set_value(
            ConnectionField::DiscoveryUri,
            &form.discovery_uri,
            window,
            cx,
        );
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

#[cfg(test)]
impl NavicatMain {
    /// 语言切换后 placeholder 属于输入框内部状态，E2E 通过该只读探针断言已刷新。
    fn new_connection_placeholders_for_test(&self, cx: &App) -> (SharedString, SharedString) {
        (
            self.new_connection_inputs
                .password
                .read(cx)
                .presentation()
                .placeholder()
                .clone(),
            self.new_connection_inputs
                .database
                .read(cx)
                .presentation()
                .placeholder()
                .clone(),
        )
    }

    /// 连接串导入框的示例必须跟随当前数据库类型，避免 PostgreSQL 弹框展示 mysql:// 示例。
    fn discovery_uri_placeholder_for_test(&self, cx: &App) -> SharedString {
        self.new_connection_inputs
            .discovery_uri
            .read(cx)
            .presentation()
            .placeholder()
            .clone()
    }
}
