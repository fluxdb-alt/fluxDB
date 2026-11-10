#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    /// 测试专用：已加载的 SQL Server `database → schema → table` 树直接安装到连接状态。
    #[cfg(test)]
    TestConnectionLoaded(ConnectionConfig),
    AiTableObjectsLoaded {
        connection_id: ConnectionId,
        parent: Option<ObjectPath>,
        objects: Vec<ObjectSummary>,
    },
    AiTableColumnsLoaded(ObjectPath, Vec<Column>),
    /// 表页已开始一次新加载；generation 用于丢弃旧的后台结果。
    DataPageLoadStarted {
        tab_id: TabId,
        generation: u64,
    },
    /// 后台结果或终态属于已被取代的加载，未写入表页。
    DataPageLoadDiscarded {
        tab_id: TabId,
        generation: u64,
    },
    ConfigProposalApplied(ConfigApplyOutcome),
    AiConversationLoaded(AiConversationView),
    /// AI 设置列表已刷新。
    AiSettingsLoaded,
    /// 模型启停状态已变化（id、启用、revision）。
    AiModelEnabled {
        id: String,
        enabled: bool,
        revision: i64,
    },
    /// 内置插件实例启停状态已变化。
    AiPluginInstanceEnabled {
        id: String,
        enabled: bool,
        revision: i64,
    },
    /// 插件配置已保存（新运行设置 revision）。
    AiPluginConfigSaved {
        instance_id: String,
        revision: i64,
    },
    /// 标准工具的全局启停状态已变化。
    AiToolEnabled {
        tool_id: String,
        enabled: bool,
        revision: i64,
    },
    /// AI 设置操作失败（CAS 冲突、存储未初始化等）；UI 用主窗口 show_message 展示。
    AiSettingsError(String),
    /// MCP 配置操作已完成；批量维护只列实际变化，显式接入/移除列出操作目标。
    AiMcpClientsSynced {
        clients: Vec<String>,
    },
    AiMcpClientsLoaded(McpClientsSnapshot),
    /// 模型已保存（id、新 revision）。
    AiModelSaved {
        id: String,
        revision: i64,
    },
    AiModelDeleted {
        id: String,
    },
    /// Agent 已保存（id、新 revision）。
    AiAgentSaved {
        id: String,
        revision: i64,
    },
    AiAgentDeleted {
        id: String,
    },
    /// 对外 MCP Server Profile 已保存。
    AiMcpServerProfileSaved {
        id: String,
        revision: i64,
    },
    /// MCP 数据源已保存。
    AiMcpDatasourceSaved {
        connection_id: i64,
        revision: i64,
    },
    /// MCP 数据源已移除。
    AiMcpDatasourceDeleted {
        connection_id: i64,
    },
    /// 会话已重命名（id、新标题、新 revision）。
    AiSessionRenamed {
        id: String,
        title: String,
        revision: i64,
    },
    AiSessionArchived {
        id: String,
        revision: i64,
    },
    /// 会话历史续读一页：追加条数与是否还有下一页。
    AiSessionsAppended {
        appended: usize,
        has_more: bool,
    },
    AiSessionRestored {
        id: String,
        revision: i64,
    },
    /// 已归档会话及其依赖事实已删除（含受控根内文件清理）。
    AiSessionPurged {
        id: String,
    },
    /// 默认模型选择已保存（运行设置新 revision）。
    AiDefaultModelsSaved {
        revision: i64,
    },
    /// 模型连接探测结果（status 见 `test_ai_model`）。
    AiModelTested {
        id: String,
        status: String,
    },
    /// 已拉取到 Provider 的模型 ID 列表（已排序去重）。
    AiModelsFetched {
        models: Vec<String>,
        capabilities: Vec<AiDiscoveredModel>,
    },
    /// 拉取模型列表失败（message 为脱敏后的可读原因）。
    AiModelsFetchFailed {
        message: String,
    },
    ErRelationshipsLoaded {
        scope_key: String,
        relationships: Vec<ErRelationship>,
    },
    ErRelationshipChanged {
        scope_key: String,
        relationship: ErRelationship,
    },
    ErRelationshipDeleted {
        scope_key: String,
        id: String,
    },
    ErRelationshipProposalsAdopted {
        scope_key: String,
        relationships: Vec<ErRelationship>,
    },
    BackupPrepared(BackupRequest),
    BackupCompleted(BackupManifest),
    RestorePrepared {
        request: RestoreRequest,
        plan: RestorePlan,
    },
    RestoreObjectsProbed(Vec<RestoreObjectProbe>),
    RestoreCompleted(RestoreOutcome),
    ConnectionsLoaded(Vec<ConnectionConfig>),
    SidebarLayoutChanged(SidebarLayout),
    ConnectionCreated(ConnectionConfig),
    ConnectionUpdated(ConnectionConfig),
    ConnectionGroupCreated(ConnectionGroup),
    ConnectionGroupDeleted(ConnectionGroupId),
    ConnectionDeleted(ConnectionId),
    DatabaseDeleted {
        connection_id: ConnectionId,
        database: String,
    },
    ConnectionTested(ConnectionId, fluxdb_core::Result<()>),
    /// 自动发现 / 连接串导入成功，弹框可据此预填一条 Redis 连接草稿。
    RedisConnectionDiscovered(ConnectionDraft),
    /// MongoDB 连接串导入成功，弹框可据此预填一条 MongoDB 连接草稿。
    MongoConnectionDiscovered(ConnectionDraft),
    ObjectsLoaded(Option<ObjectPath>, Vec<ObjectSummary>),
    /// schema 新建成功；UI 据此失效该库 schema 缓存并重取，使新 schema 出现在树。
    SchemaCreated {
        connection_id: ConnectionId,
        schema: String,
    },
    /// PG 角色列表加载完成（T26）。
    PgRolesLoaded(ConnectionId, Vec<fluxdb_core::PgRole>),
    /// PG 角色变更成功（CRUD 后 UI 重取列表）。
    PgRoleChanged(ConnectionId),
    /// PG 对象权限读取完成（对象读模型 + 选中角色生效权限）；Err 为读取失败。
    UserAdminPgObjectGrantsLoaded(
        TabId,
        Result<(PgObjectGrants, Vec<PgEffectivePrivilege>), UserFacingError>,
    ),
    /// PG 对象权限授予/撤销成功（UI 重新读取该对象权限）。
    UserAdminPgGrantsChanged(TabId),
    /// PG 角色列表加载完成回填（改版工作台）。
    UserAdminPgRolesLoaded(TabId, Vec<PgRole>),
    /// PG 成员关系加载完成回填（含服务端是否支持成员级 INHERIT/SET 选项）。
    UserAdminPgMembershipsLoaded(TabId, Vec<PgRoleMembership>, bool),
    /// PG 权限目标列表加载完成回填。
    UserAdminPgGrantTargetsLoaded(TabId, PgGrantTargetLists),
    /// PG 变更计划脱敏预览生成完成。
    UserAdminPgPlanPreview(TabId, Vec<String>),
    /// PG 变更计划应用完成（含计划本体，供 Finish 回填；Err 为失败/结果不确定）。
    UserAdminPgRolePlanFinished(TabId, PgRoleSavePlan, Result<Vec<String>, UserFacingError>),
    /// PG 变更计划应用成功（UI 刷新角色列表并清空草稿）。
    UserAdminPgRolePlanApplied(TabId),
    /// SQL Server 安全快照加载完成。
    SqlServerSecurityDatabasesLoaded(TabId, Result<Vec<String>, UserFacingError>),
    UserAdminSqlServerLoaded(
        TabId,
        Result<fluxdb_core::SqlServerSecuritySnapshot, UserFacingError>,
    ),
    /// SQL Server 指定主体显式权限加载完成。
    UserAdminSqlServerPermissionsLoaded {
        tab_id: TabId,
        principal: fluxdb_core::SqlServerPrincipalRef,
        result: Result<Vec<fluxdb_core::SqlServerPermission>, UserFacingError>,
    },
    /// SQL Server 安全计划脱敏预览完成。
    UserAdminSqlServerPreview {
        tab_id: TabId,
        result: Result<Vec<String>, UserFacingError>,
    },
    /// SQL Server 安全计划应用完成；只返回脱敏 SQL 和主体名，不携带密码草稿。
    UserAdminSqlServerApplied {
        tab_id: TabId,
        principal: String,
        result: Result<Vec<String>, UserFacingError>,
    },
    /// Oracle 用户与权限快照加载完成。
    OracleUsersLoaded(TabId, fluxdb_core::OracleUserAdminSnapshot),
    /// Oracle 变更计划执行完成（只携带脱敏语句）。
    OraclePlanApplied(TabId, Vec<String>),
    DataLoaded(TabId, DataPage),
    /// 惰性补齐无用的事件：元信息已由控制器合并进 `editor.page`，只用于通知 UI 续补下一批可见行。
    RedisKeyMetadataLoaded {
        tab_id: TabId,
    },
    RedisSetMembersLoaded {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        members: Vec<String>,
        next_cursor: String,
        total: usize,
    },
    RedisHashFieldsLoaded {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        fields: Vec<(String, String, String)>,
        next_cursor: String,
        total: usize,
    },
    /// 完整值弹框懒加载结果：对应 [`AppCommand::LoadRedisHashFieldFull`]，返回非截断原始值。
    RedisHashFieldFullValueLoaded {
        tab_id: TabId,
        field: String,
        value: String,
    },
    /// String 详情值加载结果：对应 [`AppCommand::LoadRedisStringValue`]。
    RedisStringValueLoaded {
        tab_id: TabId,
        key: String,
        value: String,
        len: u64,
        loaded_all: bool,
    },
    /// String / JSON 值下载结果：对应 [`AppCommand::DownloadRedisStringValue`]，返回原始字节。
    RedisStringValueDownloaded {
        tab_id: TabId,
        key: String,
        bytes: Vec<u8>,
    },
    RedisZSetMembersLoaded {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        members: Vec<(String, String)>,
        next_cursor: String,
        total: usize,
    },
    RedisListItemsLoaded {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        items: Vec<(usize, String)>,
        next_cursor: String,
        total: usize,
    },
    RedisStreamGroupsLoaded {
        tab_id: TabId,
        key: String,
        /// (组名, 消费者数, 未确认数, 最后投递 ID, [(消费者名, 未确认数, 空闲毫秒)])
        groups: Vec<(String, u64, u64, String, Vec<(String, u64, u64)>)>,
    },
    RedisStreamEntriesLoaded {
        tab_id: TabId,
        key: String,
        cursor: String,
        /// 一页条目：(流 ID, 由 ID 推导的本地时间, 字段列表)，顺序为从新到旧。
        entries: Vec<(String, String, Vec<(String, String)>)>,
        next_cursor: String,
        total: usize,
    },
    /// Redis 连接级运行概览拉取结果，供底栏连接状态摘要展示。
    RedisOverviewLoaded(ConnectionId, ConnectionOverview),
    /// Redis 服务端版本加载结果；`None` 表示非 Redis 连接 / 解析失败 / 命令失败（能力未知）。
    RedisServerVersionLoaded(ConnectionId, Option<RedisServerVersion>),
    /// 新建 Key 成功：`tab_id` 为目标库键列表页，`object` 为目标库（`RedisDb`）路径，
    /// `key` 为新键名，供 UI 关闭抽屉、刷新列表并打开新键详情。
    RedisKeyCreated {
        tab_id: TabId,
        object: ObjectPath,
        key: String,
    },
    BinaryCellDownloaded {
        tab_id: TabId,
        row: usize,
        column: usize,
        bytes: Vec<u8>,
    },
    /// 单元格完整文本已读到（截断 CLOB 的详情/编辑用）。
    CellTextLoaded {
        tab_id: TabId,
        row: usize,
        column: usize,
        text: String,
    },
    /// 进入编辑前需要先读完整文本：UI 据此发起后台加载。
    CellTextLoadRequested {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    /// 大字段下一块已读入（UI 存进自己的窗口状态并重绘）。
    CellContentChunkLoaded {
        tab_id: TabId,
        row: usize,
        column: usize,
        chunk: CellContentChunk,
    },
    /// 大字段完整内容已流式保存到用户选定路径。
    CellContentSaved { tab_id: TabId, bytes: u64 },
    QueryCompletionsLoaded(TabId, u64, QueryCompletionResult),
    CompletionIndexWarmed(ConnectionId, Option<String>),
    QueryFinished(TabId, QueryExecutionResult),
    /// Redis Workbench 后台执行完成：一次运行按「一条命令一条记录」产出多条结果。
    /// 失败也用本事件回传，让实际执行的命令文本随结果一起回到主线程写历史。
    RedisWorkbenchCommandsRan {
        tab_id: TabId,
        text: String,
        result: std::result::Result<Vec<CommandWorkbenchExecution>, UserFacingError>,
    },
    CreateTableApplied(TabId),
    TableRenamed {
        object: ObjectPath,
        new_name: String,
    },
    TableCopied {
        object: ObjectPath,
        new_name: String,
    },
    TableDropped(ObjectPath),
    TableTruncated(ObjectPath),
    TableDataImported {
        object: ObjectPath,
        report: fluxdb_core::TableImportReport,
    },
    CreateTableReferenceColumnsLoaded {
        tab_id: TabId,
        foreign_key_id: u64,
        columns: Vec<String>,
    },
    QueryResultPageRefreshed {
        tab_id: TabId,
        result_index: usize,
        page_index: usize,
        page: DataPage,
    },
    TabOpened(TabId),
    /// 后台设计表元数据读取结果；由 UI 再发 Finish 命令安装标签页。
    DesignTableLoaded(std::result::Result<CreateTableState, UserFacingError>),
    DesignTableDetailsLoaded(fluxdb_core::TableDesignMetadata),
    TabClosed(TabId),
    /// 已置位某标签的查询取消标志（可能仍在等语句收尾）；UI 据此提示「已请求停止」。
    QueryCancelRequested(TabId),
    TabCloseRequested(TabId),
    TabCloseCancelled(TabId),
    TabActivated(TabId),
    /// 服务器面板后台读取完成；由 UI 发 Finish 命令落状态。
    ServerPanelLoaded {
        tab_id: TabId,
        result: std::result::Result<ServerActivity, UserFacingError>,
    },
    /// 终止查询/会话或维护动作完成；成功消息可直接展示，失败沿用 `Failed` 之外的
    /// 专用事件，避免把面板局部失败升级成全局错误。
    ServerPanelActionFinished {
        tab_id: TabId,
        action: String,
        result: std::result::Result<String, UserFacingError>,
    },
    /// Redis 服务器面板后台读取完成；由 UI 发 Finish 命令落状态。
    RedisServerPanelLoaded {
        tab_id: TabId,
        result: std::result::Result<RedisServerActivity, UserFacingError>,
    },
    /// 终止客户端 / 清空慢日志完成。
    RedisServerPanelActionFinished {
        tab_id: TabId,
        action: String,
        result: std::result::Result<String, UserFacingError>,
    },
    /// MongoDB 服务器面板后台读取完成；由 UI 发 Finish 命令落状态。
    MongoServerPanelLoaded {
        tab_id: TabId,
        result: std::result::Result<MongoServerActivity, UserFacingError>,
    },
    /// 终止 MongoDB 操作完成。
    MongoServerPanelActionFinished {
        tab_id: TabId,
        action: String,
        result: std::result::Result<String, UserFacingError>,
    },
    UserAdminUsersLoaded(TabId, Vec<DatabaseUserIdentity>),
    UserAdminGrantsLoaded(TabId, Vec<String>),
    UserAdminMemberGrantsLoaded(TabId, Vec<UserRoleMember>),
    UserAdminSqlApplied(TabId),
    TableInfoLoaded(TabId, TableInfoTab, TableInfoResult),
    TableInfoChanged(TabId, TableInfoTab),
    SettingsSaved,
    /// Redis Workbench 命令历史已变化（新增/删除/清空），供桌面层做持久化 diff 与刷新。
    RedisWorkbenchHistoryChanged,
    /// 一次 MongoDB 文档加载开始（携带本次令牌）。
    MongoLoadStarted {
        tab_id: TabId,
        generation: u64,
    },
    /// 后台结果属于已被取代的加载，未写入集合页。
    MongoLoadDiscarded {
        tab_id: TabId,
        generation: u64,
    },
    /// 文档页已回写。
    MongoDocumentsLoaded(TabId, MongoDocumentsPage),
    /// 一篇文档已整篇替换成功（UI 据此重查本页并退出编辑态）。
    MongoDocumentSaved(TabId),
    /// 一篇文档已删除（UI 据此重查本页并提示）。
    MongoDocumentDeleted(TabId),
    /// 一篇文档已插入，携带服务端生成的新 `_id`（Extended JSON 文本）。
    MongoDocumentInserted(TabId, String),
    /// 文档批量导入结束（`report` 已带成功/失败条数）。
    MongoDocumentsImported {
        object: ObjectPath,
        report: MongoImportReport,
    },
    /// 集合信息已回写。
    MongoCollectionInfoLoaded(TabId, MongoCollectionInfo),
    /// 补全字段目录已采集（按标签页；失败时是空目录，UI 仍可补全运算符）。
    MongoFieldSampleLoaded(TabId, MongoFieldCatalog),
    /// 索引列表已回写。
    MongoIndexesLoaded(TabId, Vec<MongoIndexInfo>),
    /// 一个索引已删除（界面据此重取索引列表并提示）。
    MongoIndexDropped {
        connection_id: ConnectionId,
        database: String,
        collection: String,
        index: String,
    },
    /// 聚合开始（携带本次令牌）。
    MongoAggregationStarted {
        tab_id: TabId,
        generation: u64,
    },
    /// 聚合结果。
    MongoAggregationLoaded(TabId, MongoDocumentsPage),
    /// 单阶段预览结果。`pipeline` 原样回带，调用方据此丢弃过期结果。
    MongoAggregationPreviewLoaded {
        tab_id: TabId,
        pipeline: String,
        result: std::result::Result<MongoDocumentsPage, UserFacingError>,
    },
    /// 连接信息加载完成（`Err` 为读取失败）。
    MongoServerInfoLoaded(
        ConnectionId,
        std::result::Result<MongoServerInfo, UserFacingError>,
    ),
    /// 集合创建成功；UI 据此刷新该库的对象树并提示。
    MongoCollectionCreated {
        connection_id: ConnectionId,
        database: String,
        collection: String,
    },
    /// 集合已删除；UI 关闭对应标签页并刷新对象树。
    MongoCollectionDropped {
        connection_id: ConnectionId,
        database: String,
        collection: String,
    },
    /// 集合重命名成功；UI 据此刷新该库对象树。
    MongoCollectionRenamed {
        connection_id: ConnectionId,
        database: String,
        collection: String,
        new_name: String,
    },
    /// 数据库命令执行成功，`result` 为 Relaxed Extended JSON 文本。
    MongoCommandExecuted {
        connection_id: ConnectionId,
        database: String,
        command: String,
        result: String,
    },
    /// MongoDB 命令工作台后台执行完成。失败也用本事件回传，让实际执行的语句文本
    /// 随结果一起回到主线程写记录。
    MongoWorkbenchRan {
        tab_id: TabId,
        text: String,
        result: std::result::Result<MongoShellResult, UserFacingError>,
    },
    Failed(UserFacingError),
}
