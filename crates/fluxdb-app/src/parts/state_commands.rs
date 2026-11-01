#[derive(Clone, Debug, PartialEq)]
pub enum AppCommand {
    /// 测试专用：以给定连接配置构造一条已连接的 SQL Server 对象树。
    #[cfg(test)]
    LoadTestConnectionTree(ConnectionConfig),
    BrowseAiTableObjects {
        connection_id: ConnectionId,
        parent: Option<ObjectPath>,
    },
    FinishAiTableObjects {
        connection_id: ConnectionId,
        parent: Option<ObjectPath>,
        objects: Vec<ObjectSummary>,
    },
    LoadAiTableColumns(ObjectPath),
    RememberAiTableObject(ObjectPath),
    BeginAiTableQuery {
        tab_id: TabId,
        limit: u64,
    },
    LoadAiTablePage {
        tab_id: TabId,
        generation: u64,
        object: ObjectPath,
        query: AiTableQuery,
    },
    AiConversation(AiConversationCommand),
    PrepareBackup {
        request: BackupRequest,
        method: BackupMethod,
    },
    RunBackup(BackupRequest),
    PrepareRestore(RestoreRequest),
    ProbeRestore(RestoreRequest),
    RunRestore {
        request: RestoreRequest,
        plan: RestorePlan,
    },
    LoadConnections,
    ReplaceConnections(Vec<ConnectionConfig>),
    ReplaceSidebarLayout(SidebarLayout),
    CreateConnection(ConnectionDraft),
    UpdateConnection(ConnectionConfig),
    CreateConnectionGroup(String),
    RenameConnectionGroup {
        group_id: ConnectionGroupId,
        name: String,
    },
    ToggleConnectionGroup(ConnectionGroupId),
    DeleteConnectionGroup(ConnectionGroupId),
    MoveConnectionToGroup {
        connection_id: ConnectionId,
        group_id: ConnectionGroupId,
    },
    MoveConnectionToTopLevel(ConnectionId),
    TestConnection(ConnectionConfig),
    /// 从「连接串 / 云资源标识」自动发现一条 Redis 连接（URI 导入 / 云自动发现统一入口）。
    /// `provider` 为云提供商标识（`azure` / `redis-cloud` / 空表示普通连接串导入）。
    DiscoverRedisConnection {
        provider: String,
        connection_string: String,
    },
    /// 从 `mongodb://` / `mongodb+srv://` 连接串导入一条 MongoDB 连接草稿。
    DiscoverMongoConnection {
        connection_string: String,
    },
    OpenConnection(ConnectionId),
    ToggleConnectionExpanded(ConnectionId),
    DisconnectConnection(ConnectionId),
    DisconnectDatabase {
        connection_id: ConnectionId,
        database: String,
    },
    DeleteConnection(ConnectionId),
    CreateDatabase(CreateDatabaseRequest),
    CreateSchema {
        connection_id: ConnectionId,
        /// 目标数据库（PG 的 schema 属于库）；空串表示回退连接维护库。
        database: String,
        schema: String,
    },
    /// 角色管理（T26）：列表 + CRUD（PG 集群级角色）。
    LoadPgRoles(ConnectionId),
    CreatePgRole {
        connection_id: ConnectionId,
        name: String,
        can_login: bool,
        password: Option<String>,
    },
    AlterPgRolePassword {
        connection_id: ConnectionId,
        name: String,
        password: String,
    },
    RenamePgRole {
        connection_id: ConnectionId,
        old_name: String,
        new_name: String,
    },
    DropPgRole {
        connection_id: ConnectionId,
        name: String,
    },
    DeleteDatabase {
        connection_id: ConnectionId,
        database: String,
    },
    LoadObjectChildren(ObjectPath),
    RefreshObject(Option<ObjectPath>),
    /// 侧边栏「刷新连接树」：只重拉**已展开**连接的第一层对象（库 / Schema / RedisDb）。
    /// 不写 `connected` / `expanded`，也不触碰已加载的表 / 视图行（详见 `replace_connection_level0`）。
    RefreshConnectionTree,
    OpenObjectList(Option<ObjectPath>),
    OpenDataEditor(ObjectPath),
    /// 打开某数据库的备份列表 tab（传入数据库 ObjectPath）。
    OpenBackupList(ObjectPath),
    /// 打开某数据库的 ER 关系图 tab（画布原型；Path 定位到库/表所在连接与 schema）。
    OpenErDiagram(ObjectPath),
    /// ER 逻辑关系目录操作；实际文件读写由桌面端后台任务调用 dispatch。
    LoadErRelationships {
        scope_key: String,
    },
    CreateErRelationship {
        scope_key: String,
        relationship: ErRelationship,
    },
    UpdateErRelationship {
        scope_key: String,
        relationship: ErRelationship,
        expected_revision: u64,
    },
    /// 仅结构身份机械重绑，服务层原子保留原确认；普通编辑仍走 Update 并失效确认。
    RebindErRelationship {
        scope_key: String,
        relationship: ErRelationship,
        expected_revision: u64,
    },
    ConfirmErRelationship {
        scope_key: String,
        id: String,
        expected_revision: u64,
        by: String,
    },
    RejectErRelationship {
        scope_key: String,
        id: String,
        expected_revision: u64,
    },
    /// 用户在 AI 关系建议弹框中明确选择后采纳：宿主重新校验结构并原子写入已确认关系。
    AdoptErRelationshipProposals {
        scope_key: String,
        connection_id: ConnectionId,
        database: String,
        schema: Option<String>,
        proposals: Vec<ErRelationship>,
        message_part_id: String,
        batch_id: String,
    },
    DeleteErRelationship {
        scope_key: String,
        id: String,
        expected_revision: u64,
    },
    CopyTable {
        object: ObjectPath,
        new_name: String,
        copy_data: bool,
    },
    /// 从 TXT/CSV/JSON/XML 文件追加导入表数据。
    ImportTableData {
        object: ObjectPath,
        source: std::path::PathBuf,
        options: fluxdb_core::TableImportOptions,
        column_mapping: Vec<fluxdb_core::TableImportColumnMapping>,
    },
    DropTable {
        object: ObjectPath,
        foreign_key_check: ForeignKeyCheckMode,
    },
    TruncateTable {
        object: ObjectPath,
        foreign_key_check: ForeignKeyCheckMode,
        /// PG：是否 RESTART IDENTITY（默认 false = CONTINUE IDENTITY）。
        restart_identity: bool,
    },
    LoadDataPage(TabId),
    /// 开始一次表页加载：递增该标签的加载令牌，返回旧后台结果应丢弃的依据。
    BeginDataPageLoad(TabId),
    SetDataPagePagination {
        tab_id: TabId,
        offset: u64,
        limit: u64,
    },
    LoadDataPageWithSort {
        tab_id: TabId,
        generation: u64,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
        expression: Option<FilterExpression>,
    },
    LoadRedisKey {
        tab_id: TabId,
        key: String,
    },
    /// 惰性补齐 Redis Key 列表元信息：给定当前可见的一批键名，批量取回类型/值/大小/TTL。
    /// 对应 [`AppEvent::RedisKeyMetadataLoaded`]；空数组时直接成功返回空结果（不再发请求）。
    LoadRedisKeyMetadata {
        tab_id: TabId,
        keys: Vec<String>,
    },
    /// 拉取 Redis 连接级运行概览（版本 / 内存 / CPU），刷新底栏连接状态摘要。
    LoadRedisOverview(ConnectionId),
    /// 拉取 Redis 服务端版本，作为字段级 TTL 编辑等能力开关。
    /// 结果经 [`AppEvent::RedisServerVersionLoaded`] 回传；解析/命令失败统一回 `None`。
    LoadRedisServerVersion(ConnectionId),
    FinishRedisKeyRefresh {
        tab_id: TabId,
        key: String,
        result: std::result::Result<Row, UserFacingError>,
    },
    ApplyRedisKeyValue {
        tab_id: TabId,
        key: String,
        new_key: String,
        ttl: Option<String>,
        value: Option<String>,
    },
    AddRedisStreamEntry {
        tab_id: TabId,
        key: String,
        id: String,
        fields: Vec<(String, String)>,
        /// 追加后按 `MAXLEN ~ n` 近似裁剪；None 表示不裁剪。
        maxlen: Option<u64>,
    },
    DeleteRedisStreamEntry {
        tab_id: TabId,
        key: String,
        entry_id: String,
    },
    DeleteRedisSetMember {
        tab_id: TabId,
        key: String,
        member: String,
    },
    AddRedisSetMember {
        tab_id: TabId,
        key: String,
        member: String,
    },
    LoadRedisSetMembers {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    },
    LoadRedisHashFields {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    },
    SetRedisHashField {
        tab_id: TabId,
        key: String,
        field: String,
        value: String,
        /// 该字段 TTL 的处置方式：保留原值 / 清除 / 设为指定秒数。
        ttl: RedisHashFieldTtl,
    },
    /// 完整值弹框保存：携带完整原始值（可能 >1MB 被截断标记），放行截断 guard。
    /// 行内编辑走 [`AppCommand::SetRedisHashField`]（default 拒截断）。
    SetRedisHashFieldRaw {
        tab_id: TabId,
        key: String,
        field: String,
        value: String,
        ttl: RedisHashFieldTtl,
    },
    /// 仅更新 Hash 字段 TTL，不触碰字段值（纯 TTL 命令，Redis 7.4+ 字段级 TTL）。
    /// 用于大字段（>1MB 被截断）的 TTL 编辑：行内 `SetRedisHashField` 会重写 value，
    /// 对截断片段回写是危险的，必须走纯 TTL 路径。
    SetRedisHashFieldTtl {
        tab_id: TabId,
        key: String,
        field: String,
        ttl: RedisHashFieldTtl,
    },
    /// 完整值弹框懒加载：读取 hash 字段完整原始值（不截断），
    /// 结果经 [`AppEvent::RedisHashFieldFullValueLoaded`] 回传。
    LoadRedisHashFieldFull {
        tab_id: TabId,
        key: String,
        field: String,
    },
    /// String 详情值加载：`full=false` 取前 4999 字节预览（STRLEN+GETRANGE），
    /// `full=true` 取完整值（GET）。结果经 [`AppEvent::RedisStringValueLoaded`] 回传。
    LoadRedisStringValue {
        tab_id: TabId,
        key: String,
        full: bool,
    },
    /// String / JSON 值下载：取原始字节供导出文件。结果经 [`AppEvent::RedisStringValueDownloaded`] 回传。
    DownloadRedisStringValue {
        tab_id: TabId,
        key: String,
    },
    RenameRedisHashField {
        tab_id: TabId,
        key: String,
        old_field: String,
        new_field: String,
        value: String,
    },
    DeleteRedisHashField {
        tab_id: TabId,
        key: String,
        field: String,
    },
    LoadRedisZSetMembers {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    },
    AddRedisZSetMember {
        tab_id: TabId,
        key: String,
        member: String,
        score: String,
    },
    UpdateRedisZSetScore {
        tab_id: TabId,
        key: String,
        member: String,
        score: String,
    },
    DeleteRedisZSetMember {
        tab_id: TabId,
        key: String,
        member: String,
    },
    LoadRedisListItems {
        tab_id: TabId,
        key: String,
        /// 按「包含」过滤元素；空串表示不过滤。
        query: String,
        cursor: String,
    },
    /// 分页拉取 Stream 条目：cursor 为空取最新一页，非空表示「加载更多」续页。
    LoadRedisStreamEntries {
        tab_id: TabId,
        key: String,
        /// 可选的时间范围（毫秒时间戳），下推给 XREVRANGE 由服务端过滤。
        since_ms: Option<u64>,
        until_ms: Option<u64>,
        cursor: String,
    },
    /// 读取 Stream 的消费者组概览（只读）。
    LoadRedisStreamGroups {
        tab_id: TabId,
        key: String,
    },
    PushRedisListItems {
        tab_id: TabId,
        key: String,
        items: Vec<String>,
        head: bool,
    },
    /// 新建一个 Redis Key（对齐 RedisInsight AddKey）。`request` 携带类型化字段，
    /// 后端据类型执行 SET/HSET/SADD/ZADD/RPUSH/XADD/JSON.SET 并应用可选 TTL。
    /// 成功后回传 [`AppEvent::RedisKeyCreated`]。
    CreateRedisKey {
        tab_id: TabId,
        request: RedisAddKeyRequest,
    },
    SetRedisListItem {
        tab_id: TabId,
        key: String,
        index: usize,
        expected_old: Option<String>,
        value: String,
    },
    DeleteRedisListItem {
        tab_id: TabId,
        key: String,
        index: usize,
        expected_old: Option<String>,
    },
    /// 从 List 头部/尾部按数量弹出元素（LPOP/RPOP，对齐 RedisInsight Remove elements）。
    PopRedisListItems {
        tab_id: TabId,
        key: String,
        /// true 从头弹出（LPOP），false 从尾弹出（RPOP）。
        head: bool,
        /// 弹出数量；1 全版本可用，>1 需 Redis ≥ 6.2。
        count: usize,
    },
    FinishDataPageLoad {
        tab_id: TabId,
        generation: u64,
        result: std::result::Result<DataPage, UserFacingError>,
    },
    ToggleTableInfo {
        tab_id: TabId,
        tab: TableInfoTab,
    },
    CloseTableInfo(TabId),
    SelectTableInfoTab {
        tab_id: TabId,
        tab: TableInfoTab,
    },
    LoadTableInfo {
        tab_id: TabId,
        tab: TableInfoTab,
    },
    FinishTableInfoLoad {
        tab_id: TabId,
        tab: TableInfoTab,
        result: std::result::Result<TableInfoResult, UserFacingError>,
    },
    SetTableInfoSearch {
        tab_id: TabId,
        search: String,
    },
    SetTableInfoWidth {
        tab_id: TabId,
        width: f32,
    },
    ToggleDdlWrap(TabId),
    HighlightDataColumn {
        tab_id: TabId,
        column: String,
    },
    EditDataCell {
        tab_id: TabId,
        row: usize,
        column: usize,
        value: CellValue,
    },
    /// 按需读取单元格完整文本（截断的 CLOB）。数据库 I/O 在后台执行，
    /// 结果经 [`AppCommand::FinishCellTextLoad`] 写回，避免阻塞 UI。
    LoadCellText {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    FinishCellTextLoad {
        tab_id: TabId,
        row: usize,
        column: usize,
        result: std::result::Result<String, UserFacingError>,
    },
    /// 读取大字段内容引用的下一块（详情面板分段只读）。
    ///
    /// 块从 `offset` 开始、上限 [`fluxdb_core::CELL_CHUNK_BYTES`]；同一份不可变内容，
    /// 不重查数据库。只传不透明引用，不传路径。结果经
    /// [`AppEvent::CellContentChunkLoaded`] 回到 UI。
    LoadCellContentChunk {
        tab_id: TabId,
        row: usize,
        column: usize,
        content_id: String,
        kind: fluxdb_core::LargeValueKind,
        offset: u64,
    },
    /// 把大字段完整内容流式写到用户选定的文件（UI 先弹保存对话框）。
    SaveCellContentToFile {
        tab_id: TabId,
        content_id: String,
        path: PathBuf,
    },
    DownloadBinaryCell {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    UpdateBinaryCell {
        tab_id: TabId,
        row: usize,
        column: usize,
        payload: UploadPayload,
    },
    SetBinaryCellNull {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    ReplaceBinaryCellFromFile {
        tab_id: TabId,
        row: usize,
        column: usize,
        path: PathBuf,
    },
    OpenCellDetail {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    CloseCellDetail(TabId),
    StartCellDetailEdit(TabId),
    UpdateCellDetailEditValue {
        tab_id: TabId,
        value: String,
    },
    CancelCellDetailEdit(TabId),
    SaveCellDetailEdit(TabId),
    SetCellDetailNull(TabId),
    RestoreCellDetailOriginalValue(TabId),
    InsertDataRow {
        tab_id: TabId,
        result_index: Option<usize>,
        after_row: Option<usize>,
    },
    CloneDataRow {
        tab_id: TabId,
        result_index: Option<usize>,
        row: usize,
        after_row: Option<usize>,
    },
    DeleteDataRow {
        tab_id: TabId,
        result_index: Option<usize>,
        row: usize,
    },
    DiscardDataChanges(TabId),
    /// 提交数据改动的准备步：只置运行态，让 UI 立刻显示提交中并拦截重复提交。
    /// 不做数据库 I/O：真正的写入与提交后重载在后台线程执行。
    StartApplyDataChanges(TabId),
    /// 后台线程执行提交：只做数据库 I/O，不改 `AppState`
    /// （`AppController` 的 state 是按克隆隔离的，必须由回桥命令落状态）。
    RunApplyDataChanges {
        tab_id: TabId,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
    },
    /// 后台提交结果回桥：写回编辑器状态、清理改动集、复位 dirty。
    FinishApplyDataChanges {
        tab_id: TabId,
        result: std::result::Result<DataApplyResult, UserFacingError>,
    },
    OpenQueryEditor(ConnectionId),
    OpenUserAdmin(ConnectionId),
    OpenSettings,
    OpenAiChatTab,
    OpenQueryEditorInDatabase {
        connection_id: ConnectionId,
        database: Option<String>,
        /// 编辑器初始 schema 作用域（PG）；MySQL/TiDB 恒为 None。
        schema: Option<String>,
    },
    OpenRedisWorkbench {
        connection_id: ConnectionId,
        database: u32,
    },
    OpenRedisCli {
        connection_id: ConnectionId,
        database: u32,
    },
    /// 打开（或复用）MongoDB 命令行终端标签页。
    OpenMongoCli {
        connection_id: ConnectionId,
        database: String,
    },
    /// 打开（或复用）MongoDB 命令工作台标签页（库级、跨集合）。
    OpenMongoWorkbench {
        connection_id: ConnectionId,
        database: String,
    },
    OpenRedisPubSub {
        connection_id: ConnectionId,
        database: u32,
    },
    OpenCreateTable {
        connection_id: ConnectionId,
        database: Option<String>,
        /// schema 作用域（PG）；None/空表示用连接默认 search_path。
        schema: Option<String>,
    },
    OpenDesignTable(ObjectPath),
    /// 在后台读取设计表快照；只返回结果，不改控制器状态。
    LoadDesignTable(ObjectPath),
    /// 桌面端首屏：只读字段/主键等快速数据，详情随后补齐。
    LoadDesignTableBase(ObjectPath),
    /// 首屏加载完详情尚未返回时，后台补齐设计表完成值。
    LoadDesignTableDetails(ObjectPath),
    /// 把后台读取结果安装为设计表标签页。
    FinishOpenDesignTable {
        result: std::result::Result<CreateTableState, UserFacingError>,
    },
    FinishDesignTableDetails {
        tab_id: TabId,
        metadata: fluxdb_core::TableDesignMetadata,
    },
    RenameTable {
        object: ObjectPath,
        new_name: String,
    },
    StartCreateTableApply(TabId),
    ApplyCreateTable(TabId),
    FinishCreateTableApply {
        tab_id: TabId,
        result: std::result::Result<(), UserFacingError>,
    },
    SetCreateTableField {
        tab_id: TabId,
        field: CreateTableField,
        value: String,
    },
    SetCreateTableOptionField {
        tab_id: TabId,
        field: CreateTableOptionField,
        value: String,
    },
    ToggleCreateTablePartitionEnabled(TabId),
    SetCreateTablePartitionField {
        tab_id: TabId,
        field: CreateTablePartitionField,
        value: String,
    },
    SelectCreateTableTab {
        tab_id: TabId,
        create_tab: CreateTableTab,
    },
    SelectCreateTableColumn {
        tab_id: TabId,
        column_id: u64,
    },
    AddCreateTableColumn(TabId),
    MoveCreateTableColumnUp {
        tab_id: TabId,
        column_id: u64,
    },
    MoveCreateTableColumnDown {
        tab_id: TabId,
        column_id: u64,
    },
    RemoveCreateTableColumn {
        tab_id: TabId,
        column_id: u64,
    },
    SetCreateTableColumnField {
        tab_id: TabId,
        column_id: u64,
        field: CreateTableColumnField,
        value: String,
    },
    AddCreateTableColumnEnumValue {
        tab_id: TabId,
        column_id: u64,
    },
    SetCreateTableColumnEnumValue {
        tab_id: TabId,
        column_id: u64,
        index: usize,
        value: String,
    },
    RemoveCreateTableColumnEnumValue {
        tab_id: TabId,
        column_id: u64,
        index: usize,
    },
    MoveCreateTableColumnEnumValueUp {
        tab_id: TabId,
        column_id: u64,
        index: usize,
    },
    MoveCreateTableColumnEnumValueDown {
        tab_id: TabId,
        column_id: u64,
        index: usize,
    },
    ToggleCreateTableColumnFlag {
        tab_id: TabId,
        column_id: u64,
        flag: CreateTableColumnFlag,
    },
    SelectCreateTableIndex {
        tab_id: TabId,
        index_id: u64,
    },
    AddCreateTableIndex(TabId),
    MoveCreateTableIndexUp {
        tab_id: TabId,
        index_id: u64,
    },
    MoveCreateTableIndexDown {
        tab_id: TabId,
        index_id: u64,
    },
    RemoveCreateTableIndex {
        tab_id: TabId,
        index_id: u64,
    },
    SetCreateTableIndexField {
        tab_id: TabId,
        index_id: u64,
        field: CreateTableIndexField,
        value: String,
    },
    AddCreateTableIndexColumn {
        tab_id: TabId,
        index_id: u64,
    },
    MoveCreateTableIndexColumnUp {
        tab_id: TabId,
        index_id: u64,
        column_index: usize,
    },
    MoveCreateTableIndexColumnDown {
        tab_id: TabId,
        index_id: u64,
        column_index: usize,
    },
    RemoveCreateTableIndexColumn {
        tab_id: TabId,
        index_id: u64,
        column_index: usize,
    },
    SetCreateTableIndexColumnField {
        tab_id: TabId,
        index_id: u64,
        column_index: usize,
        field: CreateTableIndexColumnField,
        value: String,
    },
    SelectCreateTableCheck {
        tab_id: TabId,
        check_id: u64,
    },
    AddCreateTableCheck(TabId),
    MoveCreateTableCheckUp {
        tab_id: TabId,
        check_id: u64,
    },
    MoveCreateTableCheckDown {
        tab_id: TabId,
        check_id: u64,
    },
    RemoveCreateTableCheck {
        tab_id: TabId,
        check_id: u64,
    },
    SetCreateTableCheckField {
        tab_id: TabId,
        check_id: u64,
        field: CreateTableCheckField,
        value: String,
    },
    ToggleCreateTableCheckNotEnforced {
        tab_id: TabId,
        check_id: u64,
    },
    SelectCreateTableTrigger {
        tab_id: TabId,
        trigger_id: u64,
    },
    AddCreateTableTrigger(TabId),
    MoveCreateTableTriggerUp {
        tab_id: TabId,
        trigger_id: u64,
    },
    MoveCreateTableTriggerDown {
        tab_id: TabId,
        trigger_id: u64,
    },
    RemoveCreateTableTrigger {
        tab_id: TabId,
        trigger_id: u64,
    },
    SetCreateTableTriggerField {
        tab_id: TabId,
        trigger_id: u64,
        field: CreateTableTriggerField,
        value: String,
    },
    SetCreateTableTriggerEvent {
        tab_id: TabId,
        trigger_id: u64,
        event: CreateTableTriggerEvent,
    },
    SelectCreateTableForeignKey {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    AddCreateTableForeignKey(TabId),
    MoveCreateTableForeignKeyUp {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    MoveCreateTableForeignKeyDown {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    RemoveCreateTableForeignKey {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    SetCreateTableForeignKeyField {
        tab_id: TabId,
        foreign_key_id: u64,
        field: CreateTableForeignKeyField,
        value: String,
    },
    StartCreateTableReferenceColumnsLoad {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    LoadCreateTableReferenceColumns {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    FinishCreateTableReferenceColumnsLoad {
        tab_id: TabId,
        foreign_key_id: u64,
        result: std::result::Result<Vec<String>, UserFacingError>,
    },
    AddCreateTableForeignKeyColumn {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    MoveCreateTableForeignKeyColumnUp {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    MoveCreateTableForeignKeyColumnDown {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    RemoveCreateTableForeignKeyColumn {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    SetCreateTableForeignKeyColumn {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
        value: String,
    },
    AddCreateTableForeignKeyReferencedColumn {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    MoveCreateTableForeignKeyReferencedColumnUp {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    MoveCreateTableForeignKeyReferencedColumnDown {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    RemoveCreateTableForeignKeyReferencedColumn {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    SetCreateTableForeignKeyReferencedColumn {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
        value: String,
    },

    UpdateQueryText {
        tab_id: TabId,
        text: String,
    },
    RequestQueryCompletions {
        tab_id: TabId,
        request_seq: u64,
        cursor: usize,
        explicit: bool,
    },
    WarmCompletionIndex {
        connection_id: ConnectionId,
        database: Option<String>,
    },
    FormatQuerySql(TabId),
    CompressQuerySql(TabId),
    MarkQuerySaved {
        tab_id: TabId,
        title: String,
        origin: QueryOrigin,
    },
    StartQueryExecution(TabId),
    /// 请求停止当前标签正在执行的查询（工具栏「停止」）。置位该标签的取消标志，
    /// 执行线程在下一次检查点向服务端发送取消（PG CancelToken），不会强杀执行线程。
    CancelQueryExecution(TabId),
    ExecuteQuery(TabId),
    ExecuteQueryText {
        tab_id: TabId,
        text: String,
    },
    ExecuteQueryTextWithOptions {
        tab_id: TabId,
        text: String,
        options: QueryExecutionOptions,
    },
    FinishQueryExecution {
        tab_id: TabId,
        result: std::result::Result<QueryExecutionResult, UserFacingError>,
    },
    UpdateRedisWorkbenchText {
        tab_id: TabId,
        text: String,
    },
    /// Redis Workbench 执行的准备步：只置运行态并按需清空顶部草稿，不做网络 I/O。
    ///
    /// `execution_id` 给定时重跑结果区该条记录（保留顶部草稿），为 None 时执行顶部草稿。
    /// UI 紧接着在后台线程派发 [`AppCommand::RunRedisWorkbench`]，再用
    /// [`AppCommand::FinishRedisWorkbenchExecution`] 把结果落回主线程。
    BeginRedisWorkbenchExecution {
        tab_id: TabId,
        execution_id: Option<u64>,
    },
    /// 执行 Redis 命令：在后台线程的控制器副本上跑，只读标签页作用域，不改动任何标签页状态。
    RunRedisWorkbench {
        tab_id: TabId,
        execution_id: Option<u64>,
    },
    FinishRedisWorkbenchExecution {
        tab_id: TabId,
        /// 本次实际执行的命令文本：失败入历史时要用，不能取执行期间的草稿。
        text: String,
        /// 一次运行按「一条命令一条记录」产出多条执行记录。
        result: std::result::Result<Vec<CommandWorkbenchExecution>, UserFacingError>,
    },
    /// 删除结果区某条执行记录，只删当前一条。
    DeleteRedisWorkbenchRecord {
        tab_id: TabId,
        execution_id: u64,
    },
    ClearRedisWorkbenchResults(TabId),
    /// 切换结果区某条执行记录的折叠/展开（仅 UI 展示态，会话内有效）。
    ToggleRedisWorkbenchRecordCollapse {
        tab_id: TabId,
        execution_id: u64,
    },
    /// 切换结果区某条子命令的 `Text / JSON` 展示（仅 JSON 命令结果显示切换；按执行项维度记忆）。
    ToggleRedisWorkbenchJsonView {
        tab_id: TabId,
        execution_id: u64,
        command_index: usize,
    },
    /// 删除 Redis Workbench「历史」抽屉中指定 scope 的一条历史记录。
    DeleteRedisWorkbenchHistory {
        scope: WorkbenchHistoryScope,
        id: u64,
    },
    /// 清空 Redis Workbench「历史」抽屉中指定 scope 的全部历史。
    ClearRedisWorkbenchHistory {
        scope: WorkbenchHistoryScope,
    },
    FinishQueryResultPageRefresh {
        tab_id: TabId,
        result_index: usize,
        page_index: usize,
        result: std::result::Result<QueryExecutionResult, UserFacingError>,
    },
    ActivateQueryResultEditor {
        tab_id: TabId,
        page_index: Option<usize>,
    },
    LoadUserAdminUsers(TabId),
    StartUserAdminUsersLoad(TabId),
    FinishUserAdminUsersLoad {
        tab_id: TabId,
        result: std::result::Result<Vec<DatabaseUserIdentity>, UserFacingError>,
    },
    LoadUserAdminGrants {
        tab_id: TabId,
        user: DatabaseUserIdentity,
    },
    StartUserAdminGrantsLoad {
        tab_id: TabId,
        user: DatabaseUserIdentity,
    },
    FinishUserAdminGrantsLoad {
        tab_id: TabId,
        user: DatabaseUserIdentity,
        result: std::result::Result<Vec<String>, UserFacingError>,
    },
    LoadUserAdminMemberGrants {
        tab_id: TabId,
        role: DatabaseUserIdentity,
    },
    StartUserAdminMemberGrantsLoad {
        tab_id: TabId,
        role: DatabaseUserIdentity,
    },
    FinishUserAdminMemberGrantsLoad {
        tab_id: TabId,
        role: DatabaseUserIdentity,
        result: std::result::Result<Vec<UserRoleMember>, UserFacingError>,
    },
    SelectUserAdminUser {
        tab_id: TabId,
        user: DatabaseUserIdentity,
    },
    BeginUserAdminCreateUser(TabId),
    /// 结束「新建」态（取消新建或提交完成），回到选中既有对象。
    EndUserAdminCreateUser(TabId),
    /// 放弃 MySQL/TiDB 本地草稿，回到当前选中用户的服务端基线。
    DiscardUserAdminEdits(TabId),
    // ===== PG 用户与角色工作台（改版）=====
    /// 切换选中 PG 角色（UI 在有草稿时先弹确认，确认后走 DiscardPgDraftAndSelect）。
    SelectPgRole {
        tab_id: TabId,
        name: String,
    },
    /// 挂起「切换角色」等待用户确认丢弃草稿。
    SetPgRoleSwitchPending {
        tab_id: TabId,
        target: String,
    },
    /// 取消切换（留在当前角色，草稿保留）。
    PgCancelSwitchRole(TabId),
    /// 丢弃当前草稿并切换到目标角色（切换确认框确认后）。
    DiscardPgDraftAndSelect {
        tab_id: TabId,
        name: String,
    },
    /// 开始「新建角色」草稿（角色名留空待填，不自动误建）。
    PgBeginCreateRole(TabId),
    /// 取消/关闭当前草稿（仅清理本地草稿，不写库）。
    PgCancelDraft(TabId),
    /// 工具栏「取消」：编辑既有角色回到服务端基线，新建草稿直接丢弃。
    DiscardPgDraftEdits(TabId),
    SetPgDraftName {
        tab_id: TabId,
        name: String,
    },
    SetPgDraftCanLogin {
        tab_id: TabId,
        can_login: bool,
    },
    /// 布尔属性草稿切换（SUPERUSER/CREATEDB/CREATEROLE/INHERIT/REPLICATION/BYPASSRLS）。
    SetPgDraftAttr {
        tab_id: TabId,
        field: PgDraftAttrField,
        value: bool,
    },
    SetPgDraftConnectionLimit {
        tab_id: TabId,
        value: String,
    },
    /// 密码有效期操作：Keep/Clear(infinity)/At(绝对时间)。
    SetPgDraftValidUntil {
        tab_id: TabId,
        op: PgValidUntilOp,
    },
    /// 密码操作：Keep/Set/Clear。
    SetPgDraftPasswordOp {
        tab_id: TabId,
        op: PgPasswordOp,
    },
    /// 录入新密码明文（仅内存；写入 Set 操作）。
    SetPgDraftPassword {
        tab_id: TabId,
        password: String,
    },
    /// 开始加载 PG 角色列表（权威 pg_roles 数据）。
    StartUserAdminPgRolesLoad(TabId),
    LoadUserAdminPgRoles(TabId),
    FinishUserAdminPgRolesLoad {
        tab_id: TabId,
        result: Result<Vec<PgRole>, UserFacingError>,
    },
    /// 加载全量成员关系（双向展示用；同时探测服务端成员选项版本支持）。
    StartPgMembershipsLoad(TabId),
    LoadPgMemberships(TabId),
    FinishPgMembershipsLoad {
        tab_id: TabId,
        result: Result<Vec<PgRoleMembership>, UserFacingError>,
        member_options_supported: bool,
    },
    /// 成员关系草稿：授予 role → member（含 ADMIN/INHERIT/SET 选项）。
    PgMembershipGrant {
        tab_id: TabId,
        role: String,
        member: String,
        admin: bool,
        inherit: bool,
        set: bool,
    },
    /// 成员关系草稿：撤销 role ← member。
    PgMembershipRevoke {
        tab_id: TabId,
        role: String,
        member: String,
    },
    /// 移除一条成员关系草稿变更。
    PgMembershipRemoveEdit {
        tab_id: TabId,
        index: usize,
    },
    /// 权限页：切换目标数据库（一期同批只允许一个数据库）。
    SetPgGrantDatabase {
        tab_id: TabId,
        database: String,
    },
    /// 开始加载权限页目标列表（数据库/schema/对象/函数）。
    StartPgGrantTargetsLoad(TabId),
    LoadPgGrantTargets {
        tab_id: TabId,
        database: String,
    },
    FinishPgGrantTargetsLoad {
        tab_id: TabId,
        result: Result<PgGrantTargetLists, UserFacingError>,
    },
    /// 对象授权草稿：授予/撤销/仅取消可再授权。
    PgToggleGrant {
        tab_id: TabId,
        privilege: String,
        scope: PgObjectGrantScope,
        /// Grant（可带 grant_option）/ Revoke / RevokeGrantOption。
        op: PgGrantEditOp,
        grant_option: bool,
    },
    /// 移除一条对象授权草稿变更。
    PgRemoveGrantEdit {
        tab_id: TabId,
        index: usize,
    },
    /// 生成 SQL 预览（渲染与执行共用 connector 规则，口令按草稿如实显示）。
    StartPgPlanPreview(TabId),
    LoadPgPlanPreview(TabId),
    FinishPgPlanPreview {
        tab_id: TabId,
        result: Result<Vec<String>, UserFacingError>,
    },
    /// 保存：以单事务应用全部草稿变更（App 内部构建计划并执行）。
    StartPgPlanApply(TabId),
    ApplyPgRolePlan(TabId),
    FinishPgRolePlanApply {
        tab_id: TabId,
        plan: PgRoleSavePlan,
        result: Result<Vec<String>, UserFacingError>,
    },
    /// SQL Server 安全快照：启动、加载、回填。
    LoadSqlServerSecurityDatabases(TabId),
    FinishSqlServerSecurityDatabases {
        tab_id: TabId,
        result: Result<Vec<String>, UserFacingError>,
    },
    SelectSqlServerSecurityDatabase {
        tab_id: TabId,
        database: String,
    },
    StartSqlServerSecurityLoad(TabId),
    LoadSqlServerSecurity(TabId),
    FinishSqlServerSecurityLoad {
        tab_id: TabId,
        result: Result<fluxdb_core::SqlServerSecuritySnapshot, UserFacingError>,
    },
    /// SQL Server 指定主体的显式权限：启动、加载、回填。
    StartSqlServerPermissionsLoad {
        tab_id: TabId,
        principal: fluxdb_core::SqlServerPrincipalRef,
    },
    LoadSqlServerPermissions {
        tab_id: TabId,
        principal: fluxdb_core::SqlServerPrincipalRef,
    },
    FinishSqlServerPermissionsLoad {
        tab_id: TabId,
        principal: fluxdb_core::SqlServerPrincipalRef,
        result: Result<Vec<fluxdb_core::SqlServerPermission>, UserFacingError>,
    },
    SelectSqlServerPrincipal {
        tab_id: TabId,
        principal: fluxdb_core::SqlServerPrincipalRef,
    },
    SetSqlServerSecurityFilter {
        tab_id: TabId,
        filter: SqlServerSecurityListFilter,
    },
    /// 新建/编辑表单只存在于内存；提交时转成结构化安全变更。
    OpenSqlServerSecurityForm {
        tab_id: TabId,
        kind: SqlServerSecurityFormKind,
    },
    CloseSqlServerPermissionComposer(TabId),
    SetSqlServerSecurityFormField {
        tab_id: TabId,
        field: SqlServerSecurityFormField,
        value: String,
    },
    SetSqlServerSecurityFormFlag {
        tab_id: TabId,
        flag: SqlServerSecurityFormFlag,
        value: bool,
    },
    DiscardSqlServerSecurityChanges(TabId),
    SubmitSqlServerSecurityForm(TabId),
    ToggleSqlServerRoleMembership {
        tab_id: TabId,
        scope: fluxdb_core::SqlServerRoleScope,
        role: String,
        member: String,
    },
    SetSqlServerSelectedLoginEnabled {
        tab_id: TabId,
        enabled: bool,
    },
    RevokeSqlServerPermission {
        tab_id: TabId,
        permission_index: usize,
    },
    /// 安全计划脱敏预览与单事务应用。
    StartSqlServerSecurityPreview(TabId),
    LoadSqlServerSecurityPreview(TabId),
    FinishSqlServerSecurityPreview {
        tab_id: TabId,
        revision: u64,
        result: Result<Vec<String>, UserFacingError>,
    },
    StartSqlServerSecurityApply(TabId),
    ApplySqlServerSecurityPlan(TabId),
    FinishSqlServerSecurityApply {
        tab_id: TabId,
        principal: String,
        result: Result<Vec<String>, UserFacingError>,
    },
    BeginDeleteSqlServerPrincipal(TabId),
    ConfirmDeleteSqlServerPrincipal(TabId),
    CancelDeleteSqlServerPrincipal(TabId),
    /// Oracle 用户与权限：置 loading，后台读取账号与当前授权快照。
    StartOracleUsersLoad(TabId),
    LoadOracleUsers(TabId),
    FinishOracleUsersLoad {
        tab_id: TabId,
        result: Result<fluxdb_core::OracleUserAdminSnapshot, UserFacingError>,
    },
    SelectOracleUser {
        tab_id: TabId,
        name: String,
    },
    SetOracleGrantDraftField {
        tab_id: TabId,
        field: OracleGrantDraftField,
        value: String,
    },
    SetOracleGrantDraftOption {
        tab_id: TabId,
        with_option: bool,
    },
    /// 把授权编辑器里填好的一条加入待执行列表（新建账号时可在一次保存里带多条授权）。
    ///
    /// `kind` 由所在页签决定（角色 / 系统权限 / 对象权限），编辑器不再自带类型选择。
    QueueOracleGrantDraft {
        tab_id: TabId,
        kind: OracleGrantKind,
    },
    /// 从待执行授权列表移除一条（按索引）。
    RemoveOraclePendingGrant {
        tab_id: TabId,
        index: usize,
    },
    /// 撤销当前账号的一条直接角色授权。
    RevokeOracleRole {
        tab_id: TabId,
        role: String,
    },
    /// 撤销当前账号的一条直接系统权限。
    RevokeOracleSystemPrivilege {
        tab_id: TabId,
        privilege: String,
    },
    /// 撤销当前账号的一条直接对象权限。
    RevokeOracleObjectPrivilege {
        tab_id: TabId,
        owner: String,
        object: String,
        privilege: String,
    },
    BeginOracleCreateUser(TabId),
    CancelOracleCreateUser(TabId),
    SetOracleCreateDraftField {
        tab_id: TabId,
        field: OracleCreateDraftField,
        value: String,
    },
    SetOracleCreateDraftFlag {
        tab_id: TabId,
        grant_connect_role: bool,
    },
    SetOracleUserPassword {
        tab_id: TabId,
        password: String,
    },
    SetOracleUserLocked {
        tab_id: TabId,
        locked: bool,
    },
    DiscardOracleUserChanges(TabId),
    /// 点保存：把草稿渲染成预览 SQL 放进确认框（不执行）。
    PreviewOracleApply(TabId),
    /// 确认框点「执行」：合成计划并置位执行中，随后由 `ApplyOraclePlan` 执行。
    StartOracleApply(TabId),
    ApplyOraclePlan(TabId),
    FinishOracleApply {
        tab_id: TabId,
        result: Result<Vec<String>, UserFacingError>,
    },
    BeginDeleteOracleUser(TabId),
    CancelDeleteOracleUser(TabId),
    SetOracleDeleteCascade {
        tab_id: TabId,
        cascade: bool,
    },
    ConfirmDeleteOracleUser(TabId),
    /// 删除角色确认框。
    PgBeginDeleteRole(TabId),
    PgCancelDeleteRole(TabId),
    /// 角色列表筛选：all / login / nologin / predefined。
    SetPgRoleFilter {
        tab_id: TabId,
        filter: String,
    },
    /// PostgreSQL 对象权限：设置授权目标（种类/schema/对象/签名）。
    SetUserAdminPgGrantTarget {
        tab_id: TabId,
        kind: PgGrantObjectKind,
        schema: String,
        object: String,
        signature: String,
    },
    /// 读取当前授权目标的对象权限（owner/默认/显式条目 + 选中角色生效权限）。
    LoadUserAdminPgObjectGrants(TabId),
    /// 置对象权限面板为「加载中」。
    StartUserAdminPgObjectGrantsLoad(TabId),
    /// 对象权限读取完成回填（Ok 为对象读模型+生效权限；Err 为失败）。
    FinishUserAdminPgObjectGrantsLoad {
        tab_id: TabId,
        target_fingerprint: String,
        result: Result<(PgObjectGrants, Vec<PgEffectivePrivilege>), UserFacingError>,
    },
    SelectUserAdminDetailTab {
        tab_id: TabId,
        detail_tab: UserAdminDetailTab,
    },
    SetUserAdminSearch {
        tab_id: TabId,
        search: String,
    },
    SetUserAdminPrivilegeDatabase {
        tab_id: TabId,
        database: String,
    },
    SetUserAdminPrivilegeTable {
        tab_id: TabId,
        table: String,
    },
    ToggleUserAdminPrivilege {
        tab_id: TabId,
        privilege: String,
    },
    SetUserAdminGrantOption {
        tab_id: TabId,
        enabled: bool,
    },
    AddUserAdminPrivilegeRow {
        tab_id: TabId,
        database: String,
    },
    SetUserAdminPrivilegeRowDatabase {
        tab_id: TabId,
        row_id: u64,
        database: String,
    },
    ToggleUserAdminPrivilegeRowPrivilege {
        tab_id: TabId,
        row_id: u64,
        privilege: String,
    },
    SetUserAdminPrivilegeRowGrantOption {
        tab_id: TabId,
        row_id: u64,
        enabled: bool,
    },
    SetUserAdminCreateUser {
        tab_id: TabId,
        user: String,
    },
    SetUserAdminCreateHost {
        tab_id: TabId,
        host: String,
    },
    SetUserAdminAuthPlugin {
        tab_id: TabId,
        plugin: String,
    },
    SetUserAdminPasswordExpiryPolicy {
        tab_id: TabId,
        policy: String,
    },
    SetUserAdminCreatePassword {
        tab_id: TabId,
        password: String,
    },
    SetUserAdminNewPassword {
        tab_id: TabId,
        password: String,
    },
    SetUserAdminMaxQueriesPerHour {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminMaxUpdatesPerHour {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminMaxConnectionsPerHour {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminMaxUserConnections {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminSslType {
        tab_id: TabId,
        ssl_type: String,
    },
    SetUserAdminSslCipher {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminSslIssuer {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminSslSubject {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminRoleMembershipGranted {
        tab_id: TabId,
        role: DatabaseUserIdentity,
        granted: bool,
    },
    SetUserAdminRoleMembershipDefault {
        tab_id: TabId,
        role: DatabaseUserIdentity,
        default_role: bool,
    },
    SetUserAdminRoleMemberGranted {
        tab_id: TabId,
        member: DatabaseUserIdentity,
        granted: bool,
    },
    /// 「保存」第一步：把本次草稿渲染出的 SQL 放进确认弹框（五种库共用）。
    PreviewUserAdminSql {
        tab_id: TabId,
        sql: String,
        danger: bool,
        /// 确认后走哪条执行路径（各库计划构建/提交方式不同）。
        kind: UserAdminPendingKind,
    },
    /// 「保存」但语句要读服务端才能渲染（PG / SQL Server）：先记下这次预览是给
    /// 确认框用的，预览完成后再弹框。MySQL / Oracle 本地就能渲染，不走这条。
    BeginUserAdminSqlConfirm(TabId),
    /// 关闭确认弹框（取消，不执行）。
    ClearUserAdminPendingSql(TabId),
    /// MySQL：打开「删除用户」确认弹框（针对当前选中的已有用户）。
    BeginUserAdminDeleteUser(TabId),
    /// MySQL：关闭「删除用户」确认弹框（取消）。
    CancelUserAdminDeleteUser(TabId),
    StartUserAdminSqlApply(TabId),
    ApplyUserAdminSql {
        tab_id: TabId,
        sql: String,
    },
    FinishUserAdminSqlApply {
        tab_id: TabId,
        result: std::result::Result<(), UserFacingError>,
    },
    CloseTab(TabId),
    CloseTabs(Vec<TabId>),
    ConfirmCloseDirtyTab(TabId),
    CancelCloseDirtyTab(TabId),
    ActivateTab(TabId),
    /// 取消激活当前标签（active_tab 置空 → 显示首页/启动页）。已打开的标签保留在标签栏，
    /// 仅取消聚焦，非破坏性；与「关闭全部标签」(CloseTabs) 语义不同。
    DeactivateTab,
    PinTab(TabId),
    ReplaceQueryHistory(Vec<QueryHistoryEntry>),
    SaveSettings(Settings),
    /// 仅宿主用户确认后在后台派发；不作为 AI 工具直接开放。
    ApplyConfirmedConfig {
        proposal: Box<ConfigProposal>,
        binary_path: String,
    },
    /// 刷新设置页「大模型 / 工具与MCP / 会话历史」列表（模型、Agent、插件实例、MCP Client、会话）。
    RefreshAiSettings,
    /// 续读设置页会话历史下一页（稳定游标分页；已加载部分不重复）。
    LoadMoreAiSessions,
    /// 重命名会话（对象级 CAS；只改标题，不动消息与审计事实）。
    RenameAiSession {
        id: String,
        title: String,
        expected_revision: i64,
    },
    /// 归档会话（对象级 CAS）：从活跃历史移出，不删除消息与审计事实。
    ArchiveAiSession {
        id: String,
        expected_revision: i64,
    },
    /// 恢复已归档会话到活跃历史（对象级 CAS）。
    RestoreAiSession {
        id: String,
        expected_revision: i64,
    },
    /// 删除已归档会话及其全部依赖事实（仅归档会话可删；活跃 Turn 拒绝）。
    /// 对象级 CAS 校验 revision；成功后清理受控根内的 spill/产物文件。
    PurgeAiSession {
        id: String,
        expected_revision: i64,
    },
    /// 后台删除会话结果回桥（在主窗口控制器上移除列表行）。
    CompleteAiSessionPurge {
        id: String,
        result: std::result::Result<(), String>,
    },
    /// 后台重命名结果回桥（在主窗口控制器上合并列表，不覆盖并发变化）。
    CompleteAiSessionRename {
        id: String,
        title: String,
        revision: i64,
        result: std::result::Result<(), String>,
    },
    /// 后台归档/恢复结果回桥。
    CompleteAiSessionStatus {
        id: String,
        archived: bool,
        revision: i64,
        result: std::result::Result<(), String>,
    },
    /// 新建或保存模型档案（含 Key 的 Keep/Set/Clear）。
    SaveAiModel(AiModelSaveInput),
    /// 删除无引用的模型（对象级 CAS，UI 在后台执行）。
    DeleteAiModel {
        id: String,
        expected_revision: i64,
    },
    /// 将后台删除结果合并到主窗口状态，不覆盖其他页面的并发变化。
    CompleteAiModelDelete {
        id: String,
        result: std::result::Result<(), String>,
    },
    /// 新建或保存 Agent 配置（提示词直接随 Agent 保存）。
    SaveAiAgent(AiAgentSaveInput),
    /// 新建或保存对外 MCP Server Profile（T060，对象级 CAS；新建默认禁用/只读）。
    SaveAiMcpServerProfile(AiMcpServerProfileSaveInput),
    SaveExecutionPolicy {
        settings: fluxdb_core::ExecutionPolicySettings,
        expected_revision: i64,
    },
    EnsureMcpControl,
    /// 新增或更新 MCP 数据源（对象级 CAS）。
    SaveAiMcpDatasource(AiMcpDatasourceSaveInput),
    /// 从对外服务移除 MCP 数据源。
    DeleteAiMcpDatasource {
        profile_id: String,
        connection_id: i64,
    },
    RefreshMcpClients {
        binary_path: String,
    },
    SetMcpClientRegistration {
        client_key: String,
        registered: bool,
        binary_path: String,
    },
    /// 只维护已显式选择且配置归属未改变的客户端。
    SyncMcpClients {
        /// Server 可执行文件绝对路径；有待维护的接入记录时必须可用。
        binary_path: String,
    },
    DeleteAiAgent {
        id: String,
        expected_revision: i64,
    },
    CompleteAiAgentDelete {
        id: String,
        runtime_revision: i64,
        result: std::result::Result<(), String>,
    },
    /// 切换模型启用状态（对象级 CAS）。
    SetAiModelEnabled {
        id: String,
        enabled: bool,
        expected_revision: i64,
    },
    /// 切换内置插件实例启停（对象级 CAS）。
    SetAiPluginInstanceEnabled {
        id: String,
        enabled: bool,
        expected_revision: i64,
    },
    /// 保存插件配置（「插件管理」弹框提交，运行设置单例 CAS）。
    ///
    /// 一次提交插件的所有字段：选择落到宿主为该需求声明的角色槽位，`resource_id=None`
    /// 表示清空；插件随即回到「未配置」，不会退回自动挑选。
    SaveAiPluginConfig {
        instance_id: String,
        selections: Vec<AiPluginFieldSelection>,
        expected_revision: i64,
    },
    /// 切换标准工具的全局启停（对象级 CAS）。
    ///
    /// 全局关闭的工具不下发给模型，且 Agent 白名单不能重新打开它。
    SetAiToolEnabled {
        tool_id: String,
        enabled: bool,
        expected_revision: i64,
    },
    /// 保存默认主模型选择（运行设置单例 CAS）。
    ///
    /// 只写主模型槽位：决策模型由「Jev 决策优化」插件的管理弹框写入同一行的决策槽位，
    /// 两者互不覆盖。
    SetAiDefaultModel {
        model_id: Option<String>,
        expected_revision: i64,
    },
    /// 测试模型连接（真实协议请求并记录结果，不改配置 revision）。
    TestAiModel {
        id: String,
    },
    /// 拉取 Provider 的可用模型列表（模型 ID 下拉用）。
    ///
    /// `api_key` 仅为本次请求的内存载荷：不落库、不写日志、不回显，命令返回即丢弃。
    FetchAiModels {
        provider: String,
        endpoint: Option<String>,
        endpoint_is_full_url: bool,
        api_key: String,
    },
    SetTabDirty {
        tab_id: TabId,
        dirty: bool,
    },
    ClearCompletionIndexCache,

    // —— MongoDB 集合工作台 ——
    /// 打开（或激活已打开的）集合工作台标签页。`read_only` 由视图决定，写入入口据此禁用。
    OpenMongoCollection {
        object: ObjectPath,
        read_only: bool,
    },
    /// 开始一次文档加载：递增令牌，旧后台结果据此丢弃。
    BeginMongoLoad(TabId),
    /// 执行查询（后台线程调用）。`query` 为空时用标签页里已生效的查询。
    LoadMongoDocuments {
        tab_id: TabId,
        generation: u64,
        query: MongoQueryDraft,
    },
    /// 文档加载完成回写（UI 线程）。
    FinishMongoDocuments {
        tab_id: TabId,
        generation: u64,
        result: std::result::Result<MongoDocumentsPage, UserFacingError>,
    },
    /// 读取集合信息（后台）。
    LoadMongoCollectionInfo(TabId),
    FinishMongoCollectionInfo {
        tab_id: TabId,
        result: std::result::Result<MongoCollectionInfo, UserFacingError>,
    },
    /// 抽样文档以采集补全字段（后台，失败静默降级）。
    LoadMongoFieldSample {
        tab_id: TabId,
        object: ObjectPath,
    },
    /// 读取集合索引（后台）。
    LoadMongoIndexes(TabId),
    FinishMongoIndexes {
        tab_id: TabId,
        result: std::result::Result<Vec<MongoIndexInfo>, UserFacingError>,
    },
    /// 读取连接信息（「显示连接信息」弹框）。
    LoadMongoServerInfo(ConnectionId),
    /// 在库内创建集合（建库/建集合共用；MongoDB 的库由首个集合隐式创建）。
    CreateMongoCollection {
        connection_id: ConnectionId,
        database: String,
        collection: String,
    },
    /// 删除集合。
    DropMongoCollection {
        connection_id: ConnectionId,
        database: String,
        collection: String,
    },
    /// 重命名集合（同库内）；成功后重取该连接对象树并改写已打开标签页。
    RenameMongoCollection {
        connection_id: ConnectionId,
        database: String,
        collection: String,
        new_name: String,
    },
    /// 执行任意数据库命令（命令行终端逐行提交）。
    MongoRunCommand {
        connection_id: ConnectionId,
        database: String,
        command: String,
    },
    /// 切换集合工作台分节（文档 / 聚合 / 索引 / 集合信息）。
    SelectMongoSection {
        tab_id: TabId,
        section: MongoSection,
    },
    /// 切换结果展示方式（文档 / 表格 / JSON）。
    SelectMongoDisplayMode {
        tab_id: TabId,
        mode: MongoDisplayMode,
    },
    /// 选中/取消选中右侧详情抽屉的文档（按本页下标）。
    SelectMongoDocument {
        tab_id: TabId,
        index: Option<usize>,
    },
    /// 整篇替换一篇文档（后台）；`before_json` 只用于取原始 `_id` 作定位条件。
    SaveMongoDocument {
        tab_id: TabId,
        before_json: String,
        after_json: String,
    },
    /// 删除一篇文档（后台）；`document_json` 只用于取原始 `_id` 作定位条件。
    DeleteMongoDocument {
        tab_id: TabId,
        document_json: String,
    },
    /// 往集合里插入一篇新文档（后台）；`document_json` 不得自带 `_id`。
    InsertMongoDocument {
        tab_id: TabId,
        document_json: String,
    },
    /// 从文件批量导入文档（后台，带取消与进度）。
    ImportMongoDocuments {
        object: ObjectPath,
        source: PathBuf,
        options: MongoImportOptions,
    },
    /// 设置翻页偏移（下一页 / 上一页 / 新查询回到第一页）。
    SetMongoOffset {
        tab_id: TabId,
        offset: u64,
    },
    /// 回写聚合管道草稿（Extended JSON 数组文本），不执行。
    UpdateMongoAggregation {
        tab_id: TabId,
        pipeline: String,
    },
    /// 开始一次聚合（分配令牌），随后由 `RunMongoAggregation` 执行。
    BeginMongoAggregation(TabId),
    RunMongoAggregation {
        tab_id: TabId,
        generation: u64,
        pipeline: String,
    },
    FinishMongoAggregation {
        tab_id: TabId,
        generation: u64,
        result: std::result::Result<MongoDocumentsPage, UserFacingError>,
    },

    // —— MongoDB 命令工作台 ——
    /// 回写工作台顶部输入框草稿，不执行。
    UpdateMongoWorkbenchText {
        tab_id: TabId,
        text: String,
    },
    /// 准备一次执行：主线程同步置运行态并按需清空草稿。
    /// `execution_id` 为 `Some` 表示重跑结果区里的某条记录。
    BeginMongoWorkbenchExecution {
        tab_id: TabId,
        execution_id: Option<u64>,
    },
    /// 后台线程执行语句（不写标签页状态），结果经 `FinishMongoWorkbenchExecution` 回写。
    RunMongoWorkbench {
        tab_id: TabId,
        execution_id: Option<u64>,
    },
    /// 主线程落执行结果；`result` 携带实际执行的语句文本，失败也要能写进记录。
    FinishMongoWorkbenchExecution {
        tab_id: TabId,
        text: String,
        result: std::result::Result<MongoShellResult, UserFacingError>,
    },
    /// 删除一条执行记录。
    DeleteMongoWorkbenchRecord {
        tab_id: TabId,
        execution_id: u64,
    },
    /// 清空结果区全部记录（不影响历史与草稿）。
    ClearMongoWorkbenchResults(TabId),
    /// 切换单条记录的折叠态。
    ToggleMongoWorkbenchRecordCollapse {
        tab_id: TabId,
        execution_id: u64,
    },
}
