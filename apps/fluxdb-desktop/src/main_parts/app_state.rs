#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum CreateTableInputKey {
    TableName(TabId),
    TableComment(TabId),
    ColumnName(TabId, u64),
    ColumnLength(TabId, u64),
    ColumnScale(TabId, u64),
    ColumnDefault(TabId, u64),
    ColumnComment(TabId, u64),
    ColumnCommentEditor(TabId, u64),
    ColumnKeyLength(TabId, u64),
    IndexName(TabId, u64),
    IndexComment(TabId, u64),
    IndexColumnSubPart(TabId, u64, usize),
    ForeignKeyName(TabId, u64),
    CheckName(TabId, u64),
    CheckExpression(TabId, u64),
    CheckExpressionEditor(TabId, u64),
    TriggerName(TabId, u64),
    TriggerBody(TabId, u64),
    TableTablespace(TabId),
    TableAvgRowLength(TabId),
    TableMaxRows(TabId),
    TableMinRows(TabId),
    TableKeyBlockSize(TabId),
    TablePartitionExpression(TabId),
    TablePartitionSql(TabId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum CreateTableSelectKey {
    ColumnCharset(TabId, u64),
    ColumnCollation(TabId, u64),
    IndexType(TabId, u64),
    IndexMethod(TabId, u64),
    ForeignKeyReferencedDatabase(TabId, u64),
    ForeignKeyReferencedTable(TabId, u64),
    ForeignKeyOnDelete(TabId, u64),
    ForeignKeyOnUpdate(TabId, u64),
    TriggerTiming(TabId, u64),
    TableEngine(TabId),
    TableCharset(TabId),
    TableCollation(TabId),
    TableRowFormat(TabId),
    TablePartitionMethod(TabId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum DataExportObjectSelectKey {
    Database(TabId),
    Table(TabId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CreateTableIndexFieldDropdownKind {
    Name,
    SortOrder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CreateTableIndexFieldDropdownKey {
    tab_id: TabId,
    index_id: u64,
    column_index: usize,
    kind: CreateTableIndexFieldDropdownKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CreateTableIndexFieldSelectionKey {
    tab_id: TabId,
    index_id: u64,
    column_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CreateTableForeignKeyFieldSelectionKind {
    Local,
    Referenced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CreateTableForeignKeyFieldSelectionKey {
    tab_id: TabId,
    foreign_key_id: u64,
    column_index: usize,
    kind: CreateTableForeignKeyFieldSelectionKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CreateTableForeignKeyFieldsDraft {
    tab_id: TabId,
    foreign_key_id: u64,
    columns: Vec<String>,
    selected_field: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CreateTableForeignKeyReferencedFieldsDraft {
    tab_id: TabId,
    foreign_key_id: u64,
    columns: Vec<String>,
    selected_field: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsPanelSection {
    Editor,
    Shortcuts,
    Appearance,
    System,
    Data,
    ConnectionSecurity,
    DatabaseSupport,
    About,
}

struct ShortcutCaptureState {
    active_id: Option<&'static str>,
    invalid: bool,
    focus_handle: FocusHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedisKeyMetaField {
    KeyName,
    Ttl,
}

/// String 详情已加载的值状态：值文本 + 服务端字节长度 + 是否完整。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisStringValueState {
    value: String,
    len: u64,
    loaded_all: bool,
}

/// String 详情「格式转换」下拉支持的展示格式：仅 Unicode（原文）与 JSON（pretty）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedisStringFormat {
    Unicode,
    Json,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisKeyDeleteConfirm {
    tab_id: TabId,
    source_row: usize,
    key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisStreamEntryAddForm {
    tab_id: TabId,
    key: String,
}

#[derive(Clone, Debug)]
struct RedisStreamEntryFieldInputs {
    field_input: Entity<InputState>,
    value_input: Entity<InputState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisStreamEntryDeleteConfirm {
    tab_id: TabId,
    key: String,
    entry_id: String,
}

/// Set 类型「新增成员」底部抽屉的打开状态，记录目标 Key。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisSetMemberDrawerForm {
    tab_id: TabId,
    key: String,
}

/// Set 成员删除二次确认的目标：面板成员行（`usize` 为行索引）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedisSetMemberDeleteTarget {
    PanelRow(usize),
}

/// Hash 字段删除二次确认的目标：按 tab/key/字段名定位，字段名在表内唯一。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisHashFieldDeleteTarget {
    tab_id: TabId,
    key: String,
    field: String,
}

fn new_redis_stream_entry_field_inputs(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> RedisStreamEntryFieldInputs {
    RedisStreamEntryFieldInputs {
        field_input: cx.new(|cx| InputState::new(window, cx).placeholder("Field")),
        value_input: cx.new(|cx| InputState::new(window, cx).placeholder("Value")),
    }
}

/// Set 成员搜索结果页（桌面层镜像，不直接引用 connector 类型）。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RedisSetMemberPage {
    members: Vec<String>,
    next_cursor: String,
    total: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RedisHashFieldPage {
    fields: Vec<(String, String, String)>,
    next_cursor: String,
    total: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RedisZSetMemberPage {
    members: Vec<(String, String)>,
    next_cursor: String,
    total: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RedisListItemPage {
    items: Vec<(usize, String)>,
    next_cursor: String,
    total: usize,
}

/// 消费者组概览的一行（只读展示用）。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RedisStreamGroupRow {
    name: String,
    consumers: u64,
    pending: u64,
    last_delivered_id: String,
    /// (消费者名, 未确认条目数, 空闲毫秒数)
    consumer_detail: Vec<(String, u64, u64)>,
}

/// Stream 条目分页缓存：按 (tab, key) 累积，「加载更多」沿 next_cursor 向更旧方向追加。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RedisStreamEntryPage {
    entries: Vec<RedisStreamEntryRow>,
    /// 下一页 XREVRANGE 起始 ID；为 "0" 表示已到最旧一条。
    next_cursor: String,
    /// XLEN 得到的条目总数。
    total: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RedisHashFieldRow {
    field: String,
    value: String,
    ttl: String,
}

#[derive(Clone, Debug)]
struct RedisHashFieldDrawerInputs {
    field_input: Entity<InputState>,
    value_input: Entity<InputState>,
    ttl_input: Entity<InputState>,
}

/// 「新增 Key」抽屉里 名称=值 型子表单（Hash/ZSet/Stream）的可增删行：
/// 每行由两个动态 Input 实体组成，`name` 为字段名/成员，`value` 为值/score。
#[derive(Clone, Debug)]
struct RedisAddKeyNameValueRow {
    name: Entity<InputState>,
    value: Entity<InputState>,
}

/// 「新增 Key」抽屉里 Hash 子表单的可增删行：每行 = 字段名 / 值 / 可选字段级 TTL（秒）。
/// 与名称=值行不同，Hash 每个字段都支持独立 TTL（Redis 7.4+ 字段级 TTL）。
#[derive(Clone, Debug)]
struct RedisAddKeyHashRow {
    name: Entity<InputState>,
    value: Entity<InputState>,
    ttl: Entity<InputState>,
}

/// 「新增 Key」抽屉里 单成员 型子表单（Set/List）的可增删行：每行一个成员输入实体。
#[derive(Clone, Debug)]
struct RedisAddKeySingleRow {
    value: Entity<InputState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedisHashFieldCellKind {
    Value,
    Ttl,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisHashFieldEditingState {
    tab_id: TabId,
    key: String,
    row_index: usize,
    kind: RedisHashFieldCellKind,
}

/// 完整值内嵌面板（查看/编辑 >1MB 被截断的 hash 字段完整值）。
/// `full_value` 懒加载：打开时按需 HGET，未加载完为 None。编辑态时文本域接管显示。
/// `error` 表示懒加载失败，面板内展示错误与重试入口。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisHashFullValueViewer {
    tab_id: TabId,
    key: String,
    field: String,
    full_value: Option<String>,
    loading: bool,
    editing: bool,
    saving: bool,
    error: bool,
}

#[derive(Clone, Debug)]
struct RedisZSetMemberInputs {
    member_input: Entity<InputState>,
    score_input: Entity<InputState>,
}

#[derive(Clone, Debug)]
struct RedisListItemInputs {
    value_input: Entity<InputState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisHashFieldDrawerForm {
    tab_id: TabId,
    key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisZSetMemberDrawerForm {
    tab_id: TabId,
    key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisListItemDrawerForm {
    tab_id: TabId,
    key: String,
}

/// List 元素删除抽屉（对齐 RedisInsight Remove elements）的表单态：
/// 从头部还是尾部弹出 + 弹出数量由 `head` 与独立的 count 输入持有。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisListItemRemoveDrawerForm {
    tab_id: TabId,
    key: String,
}

/// List 元素删除的二次确认目标（对齐 RedisInsight 的 ConfirmationPopover）：
/// 在「删除」按钮上先解析出方向 `head` 与数量 `count`，确认后才真正 LPOP/RPOP。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisListItemRemoveConfirmTarget {
    tab_id: TabId,
    key: String,
    /// true 表示从头弹（LPOP），false 表示从尾弹（RPOP）
    head: bool,
    count: usize,
}

/// List 元素的编辑目标（行内编辑 LSET）：用表格行号 `row_index` 定位，
/// 渲染时从当前页 `rows[row_index]` 取绝对下标与当前值。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisListItemEditingState {
    tab_id: TabId,
    key: String,
    row_index: usize,
}

/// 「新增 Key」抽屉（对齐 RedisInsight AddKey）的开启态：
/// 只记录归属标签页 `tab_id`，公共字段与各类型子表单的状态由独立的 Entity / 行 Vec 持有。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisAddKeyDrawerForm {
    tab_id: TabId,
}

/// 新增 Key 的可用类型，顺序即下拉展示顺序（对齐 RedisInsight 核心类型集）。
const REDIS_ADD_KEY_TYPES: &[&str] = &[
    "String", "Hash", "List", "Set", "ZSet", "Stream", "JSON",
];

fn new_redis_set_member_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("Enter Member");
        if let Some(value) = value {
            input = input.default_value(value);
        }
        input
    })
}

fn new_redis_hash_field_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("Field");
        if let Some(value) = value {
            input = input.default_value(value);
        }
        input
    })
}

fn new_redis_hash_value_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("Value");
        if let Some(value) = value {
            input = input.default_value(value);
        }
        input
    })
}

fn new_redis_hash_ttl_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("TTL");
        if let Some(value) = value {
            input = input.default_value(value);
        }
        input
    })
}

/// 「新增 Key」名称=值型行的「名称」输入（Hash=Field / ZSet=Member / Stream=Field）。
fn new_redis_add_key_name_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

/// 「新增 Key」名称=值型行的「值」输入（Hash/Stream=Value，ZSet=Score）。
fn new_redis_add_key_pair_value_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

/// 「新增 Key」单成员型行的成员输入（Set=Member，List=Element）。
fn new_redis_add_key_single_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

fn new_redis_zset_member_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("Member");
        if let Some(value) = value {
            input = input.default_value(value);
        }
        input
    })
}

fn new_redis_zset_score_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("Score");
        if let Some(value) = value {
            input = input.default_value(value);
        }
        input
    })
}

fn new_redis_list_item_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("Value");
        if let Some(value) = value {
            input = input.default_value(value);
        }
        input
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingDeleteDatabase {
    connection_id: ConnectionId,
    database: String,
}

struct NavicatMain {
    focus_handle: FocusHandle,
    controller: AppController,
    storage: FileStorage,
    theme_mode: ThemeMode,
    new_connection_kind: Option<DatabaseKind>,
    new_connection_tab: NewConnectionTab,
    new_connection_form: NewConnectionForm,
    new_connection_inputs: NewConnectionInputs,
    new_connection_password_visible: bool,
    editing_connection_id: Option<ConnectionId>,
    rename_group_input: Entity<InputState>,
    _rename_group_subscription: Subscription,
    table_folder_rename_input: Entity<InputState>,
    _table_folder_rename_subscription: Subscription,
    sidebar_search_input: Entity<InputState>,
    _sidebar_search_subscription: Subscription,
    sidebar_search: String,
    // 连接浏览器树：虚拟化滚动句柄。滚动偏移/视口保留在句柄内跨帧不掉；
    // 可见行列表只在连接树输入变化时重算，表格滚动期间复用纯数据缓存。
    sidebar_tree_scroll: VirtualListScrollHandle,
    sidebar_tree_cache: std::cell::RefCell<Option<SidebarTreeCache>>,
    field_filter_search_input: Entity<InputState>,
    _field_filter_search_subscription: Subscription,
    field_filter_search: String,
    field_filter_popover: Option<TabId>,
    visible_table_fields: BTreeMap<TabId, BTreeSet<String>>,
    data_filter_value_input: Entity<InputState>,
    _data_filter_value_input_subscription: Subscription,
    local_filter_value_input: Entity<InputState>,
    _local_filter_value_input_subscription: Subscription,
    local_filter_search_input: Entity<InputState>,
    _local_filter_search_subscription: Subscription,
    data_filter_search_input: Entity<InputState>,
    _data_filter_search_subscription: Subscription,
    data_filter_text_input: Entity<InputState>,
    _data_filter_text_subscription: Subscription,
    data_sort_text_input: Entity<InputState>,
    _data_sort_text_subscription: Subscription,
    data_sql_panel_input: Entity<InputState>,
    _data_sql_panel_subscription: Subscription,
    data_sql_footer_selection: SqlTextSelection,
    data_page_input: Entity<InputState>,
    _data_page_subscription: Subscription,
    data_cell_edit_input: Entity<InputState>,
    _data_cell_edit_subscription: Subscription,
    data_cell_editing: Option<DataCellEditState>,
    temporal_part_input: Entity<InputState>,
    _temporal_part_subscription: Subscription,
    temporal_part_editing: Option<TemporalPartEditState>,
    cell_detail_input: Entity<InputState>,
    _cell_detail_subscription: Subscription,
    data_search_input: Entity<InputState>,
    _data_search_subscription: Subscription,
    row_detail_search_input: Entity<InputState>,
    _row_detail_search_subscription: Subscription,
    row_detail_search: String,
    data_search_panels: BTreeSet<TabId>,
    data_search_queries: BTreeMap<TabId, String>,
    data_search_active_matches: BTreeMap<TabId, DataSearchMatch>,
    data_search_highlight_all_tabs: BTreeSet<TabId>,
    redis_search_input: Entity<InputState>,
    _redis_search_subscription: Subscription,
    redis_type_select: Entity<SelectState<SearchableVec<String>>>,
    _redis_type_select_subscription: Subscription,
    redis_search_drafts: BTreeMap<TabId, String>,
    redis_search_queries: BTreeMap<TabId, String>,
    redis_type_filters: BTreeMap<TabId, String>,
    // Redis Key 列表展示模式（平铺 / Folder），按 tab 隔离；默认平铺以保持既有行为。
    redis_key_list_modes: BTreeMap<TabId, RedisKeyListMode>,
    // Redis Key 列表 Folder 模式下已展开的 folder 前缀集合，按 tab 隔离；
    // 切回平铺时不清空，仅不使用，切回 Folder 时继续生效。
    redis_key_list_expanded: BTreeMap<TabId, BTreeSet<String>>,
    // Redis Key 列表 Folder 模式下当前选中的叶子键名：详情抽屉跟随叶子键，folder 节点不打开详情。
    redis_key_list_selected_leaf: BTreeMap<TabId, String>,
    // Redis Key 列表 Folder 模式下 hover 的叶子原始行下标（用于浮现删除按钮，避免与表格行下标纠缠）。
    redis_key_list_folder_hovered: BTreeMap<TabId, usize>,
    // Redis Key 平铺列表的虚拟滚动句柄：只保留视口附近的行，避免 Key 数量增长后整页布局。
    redis_key_list_uniform_scroll: BTreeMap<TabId, UniformListScrollHandle>,
    // Redis Key Folder 拍平结果缓存：仅在数据或展开状态变化时重建前缀树。
    redis_key_folder_visible_cache: BTreeMap<TabId, Rc<Vec<RedisKeyVisibleRow>>>,
    // Redis Key 列表当前已加载（已扫描）的键数，按 tab 隔离：Folder 每批拉到 10000，
    // 平铺随滚动触底累加。用于状态栏展示「已扫描 N 个键」并作为每次懒加载的基准。
    redis_key_list_loaded: BTreeMap<TabId, u64>,
    // Redis Key 元信息惰性补全「请求中」的键名集合，按 tab 隔离：用于并发去重，
    // 避免同一批可见行被重复触发补全请求。
    redis_key_metadata_pending: BTreeMap<TabId, BTreeSet<String>>,
    // Redis Key 搜索历史：按 (连接 ID, DB 索引) 隔离，队首最新，上限 200。
    // `redis_key_search_history_loaded` 标记该 (连接, DB) 是否已从存储惰性加载一次，
    // 避免用户从未打开的连接历史被提前读入。
    redis_key_search_history: BTreeMap<(ConnectionId, String), Vec<String>>,
    redis_key_search_history_loaded: BTreeMap<(ConnectionId, String), bool>,
    // 上下按钮触发的搜索历史下拉开关
    redis_key_search_history_open: bool,
    redis_data_refresh_times: BTreeMap<TabId, Instant>,
    redis_refresh_time_task: Option<Task<()>>,
    // Redis 连接级概览（版本/内存/CPU）的定期刷新任务与进行中的单次拉取任务
    redis_overview_refresh_task: Option<Task<()>>,
    redis_overview_refresh_tasks: BTreeMap<u64, Task<()>>,
    // Redis 服务端版本缓存（字段级 TTL 编辑等能力开关）；None 表示未知/探测失败。
    // 惰性填充，连接重连成功后清除、下次打开 Hash 面板自动重新探测。
    redis_server_versions: BTreeMap<ConnectionId, Option<RedisServerVersion>>,
    // Redis 服务端版本探测中的任务（按连接去重，避免重复渲染重复发起）
    redis_server_version_tasks: BTreeMap<ConnectionId, Task<()>>,
    redis_key_list_hovered_tab: Option<TabId>,
    // 当前被 hover 的 Redis Workbench 记录（(tab_id, execution_id)）；用于「复制命令」按钮
    // hover 态浮现（不常驻抢标题行空间）。记录 id 为每标签页各自分配，跨标签可能重号，故按
    // (tab_id, execution_id) 唯一标识。None 表示未 hover 任何记录；会话内 UI 展示用，不持久化。
    redis_workbench_hovered_record: Option<(TabId, u64)>,
    redis_stream_table_state: Entity<TableState<RedisStreamTableDelegate>>,
    // Hash 明细数据表：序列/Field/Value/TTL 四列，比例宽度由 canvas 测量后换算
    redis_hash_table_state: Entity<TableState<RedisHashTableDelegate>>,
    // List 明细数据表：序号/Value 两列（只读展示），比例宽度由 canvas 测量后换算
    redis_list_table_state: Entity<TableState<RedisListTableDelegate>>,
    redis_key_value_input: Entity<InputState>,
    _redis_key_value_subscription: Subscription,
    redis_json_key_value_input: Entity<InputState>,
    _redis_json_key_value_subscription: Subscription,
    // Redis JSON 值编辑器运行态：绑定 `redis_json_key_value_input`，维护权威全文、诊断与折叠状态。
    redis_json_editor: JsonEditorState,
    // JSON 编辑器当前归属的 (tab, key)；切换 key 或关闭抽屉时据此清理并重置折叠/诊断。
    redis_json_editor_active: Option<(TabId, String)>,
    // JSON 编辑器是否为「保存中/加载中」，期间禁用编辑、保存、格式化等修改文本的操作。
    redis_json_editor_busy: bool,
    redis_key_value_active: Option<(TabId, String)>,
    redis_key_value_drafts: BTreeMap<(TabId, String), String>,
    redis_key_value_syncing: bool,
    // String 详情值状态：按 (tab, key) 缓存已加载值（preview 或完整值），
    // 让「值是否完整」脱离列表 200 字符 preview，作为格式化/复制/编辑/保存的统一 gating。
    redis_string_values: BTreeMap<(TabId, String), RedisStringValueState>,
    redis_string_format: BTreeMap<(TabId, String), RedisStringFormat>,
    redis_string_editing: Option<(TabId, String)>,
    // 在飞的 string 值加载：(tab, key, full)；full=true 表示「加载全部」。
    redis_string_loading: Option<(TabId, String, bool)>,
    // 在飞的 string 值下载（去重，同一 (tab, key) 只允许一个进行中的下载）。
    redis_string_downloading: BTreeSet<(TabId, String)>,
    // 当前详情抽屉正在展示的 string key：离开（切换 key / 关闭抽屉）时据此清理缓存，下次进入重新预览。
    redis_string_active: Option<(TabId, String)>,
    redis_key_name_input: Entity<InputState>,
    _redis_key_name_subscription: Subscription,
    redis_key_ttl_input: Entity<InputState>,
    _redis_key_ttl_subscription: Subscription,
    redis_key_meta_active: Option<(TabId, String)>,
    redis_key_name_drafts: BTreeMap<(TabId, String), String>,
    redis_key_ttl_drafts: BTreeMap<(TabId, String), String>,
    redis_key_meta_editing: Option<RedisKeyMetaField>,
    redis_key_meta_syncing: bool,
    pending_redis_key_delete: Option<RedisKeyDeleteConfirm>,
    pending_redis_stream_entry_add: Option<RedisStreamEntryAddForm>,
    pending_redis_stream_entry_delete: Option<RedisStreamEntryDeleteConfirm>,
    redis_stream_entry_id_input: Entity<InputState>,
    redis_stream_entry_field_rows: Vec<RedisStreamEntryFieldInputs>,
    // Stream「新增 Entry」抽屉字段区滚动句柄：滚动区与常显滚动条共用同一偏移
    redis_stream_entry_drawer_scroll: ScrollHandle,
    redis_set_member_search_input: Entity<InputState>,
    _redis_set_member_search_subscription: Subscription,
    redis_set_member_search_active: Option<(TabId, String)>,
    redis_set_member_search_queries: BTreeMap<(TabId, String), String>,
    redis_set_member_search_pages: BTreeMap<(TabId, String, String), RedisSetMemberPage>,
    redis_set_member_search_loading: Option<(TabId, String, String)>,
    redis_set_member_search_more_loading: Option<(TabId, String, String)>,
    redis_set_member_search_generation: BTreeMap<u64, u64>,
    redis_set_member_search_debounce: Option<Task<()>>,
    redis_set_member_search_debounce_until: Option<Instant>,
    redis_set_member_search_syncing: bool,
    redis_set_member_rows: Vec<Entity<InputState>>,
    // Set 详情面板成员列表滚动句柄：供成员列表滚动区与常显滚动条共用同一偏移
    redis_set_member_panel_scroll: ScrollHandle,
    redis_set_member_active: Option<(TabId, String)>,
    pending_redis_set_member_drawer: Option<RedisSetMemberDrawerForm>,
    redis_set_member_drawer_rows: Vec<Entity<InputState>>,
    redis_set_member_drawer_scroll: ScrollHandle,
    pending_redis_set_member_delete: Option<RedisSetMemberDeleteTarget>,
    redis_hash_field_search_input: Entity<InputState>,
    _redis_hash_field_search_subscription: Subscription,
    redis_hash_field_search_active: Option<(TabId, String)>,
    redis_hash_field_search_queries: BTreeMap<(TabId, String), String>,
    redis_hash_field_search_pages: BTreeMap<(TabId, String, String), RedisHashFieldPage>,
    redis_hash_field_search_loading: Option<(TabId, String, String)>,
    redis_hash_field_search_more_loading: Option<(TabId, String, String)>,
    redis_hash_field_search_generation: BTreeMap<u64, u64>,
    redis_hash_field_search_debounce: Option<Task<()>>,
    redis_hash_field_search_debounce_until: Option<Instant>,
    redis_hash_field_search_syncing: bool,
    redis_hash_value_edit_input: Entity<InputState>,
    _redis_hash_value_edit_subscription: Subscription,
    redis_hash_ttl_edit_input: Entity<InputState>,
    _redis_hash_ttl_edit_subscription: Subscription,
    redis_hash_field_rows: Vec<RedisHashFieldRow>,
    redis_hash_field_hovered: Option<RedisHashFieldEditingState>,
    redis_hash_field_editing: Option<RedisHashFieldEditingState>,
    // 完整值弹框（查看/编辑 >1MB 截断 hash 字段）；懒加载 task id 用 redis_hash_field_mutation_task_id。
    // 用 Rc<RefCell> 而非裸字段：gpui-component 的 dialog builder 在 NavicatMain::render 期间被
    // `Root::render_dialog_layer` 同步执行，此时 NavicatMain 处于 lease 状态，builder 里 `view.read(cx)`
    // 会触发 double-lease panic，因此 builder 必须改读共享状态而不是 re-enter 视图本身。
    redis_hash_full_value_viewer: Rc<RefCell<Option<RedisHashFullValueViewer>>>,
    pending_redis_hash_field_drawer: Option<RedisHashFieldDrawerForm>,
    redis_hash_field_drawer_rows: Vec<RedisHashFieldDrawerInputs>,
    redis_hash_field_drawer_scroll: ScrollHandle,
    pending_redis_hash_field_delete: Option<RedisHashFieldDeleteTarget>,
    redis_zset_member_search_input: Entity<InputState>,
    _redis_zset_member_search_subscription: Subscription,
    redis_zset_member_search_active: Option<(TabId, String)>,
    redis_zset_member_search_queries: BTreeMap<(TabId, String), String>,
    redis_zset_member_search_pages: BTreeMap<(TabId, String, String), RedisZSetMemberPage>,
    redis_zset_member_search_loading: Option<(TabId, String, String)>,
    redis_zset_member_search_more_loading: Option<(TabId, String, String)>,
    redis_zset_member_search_generation: BTreeMap<u64, u64>,
    redis_zset_member_search_debounce: Option<Task<()>>,
    redis_zset_member_search_debounce_until: Option<Instant>,
    redis_zset_member_search_syncing: bool,
    redis_zset_member_rows: Vec<RedisZSetMemberInputs>,
    redis_zset_member_panel_scroll: ScrollHandle,
    // ZSet 详情表 score 列「行内编辑」状态：hover 显示编辑图标，点击进入编辑（输入框 + x/勾），
    // 与 member 列（只读纯文本）区分。编辑态独立于 rows 存储，避免同步重建行时丢失编辑焦点。
    redis_zset_member_score_hover: Option<usize>,
    redis_zset_member_score_editing: Option<usize>,
    redis_zset_member_score_edit_input: Option<Entity<InputState>>,
    // 编辑输入失焦订阅：点击编辑框以外区域（焦点转移）时自动取消编辑。
    redis_zset_member_score_blur_sub: Option<Subscription>,
    pending_redis_zset_member_drawer: Option<RedisZSetMemberDrawerForm>,
    redis_zset_member_drawer_rows: Vec<RedisZSetMemberInputs>,
    redis_zset_member_drawer_scroll: ScrollHandle,
    pending_redis_zset_member_delete: Option<usize>,
    redis_list_item_search_input: Entity<InputState>,
    _redis_list_item_search_subscription: Subscription,
    redis_stream_entry_active: Option<(TabId, String)>,
    /// Stream 时间范围过滤的输入框（本地时间，格式 YYYY-MM-DD HH:MM:SS，留空表示不限）。
    redis_stream_since_input: Entity<InputState>,
    redis_stream_until_input: Entity<InputState>,
    /// 生效中的时间范围（毫秒时间戳），按 (tab, key) 记住。
    redis_stream_ranges: BTreeMap<(TabId, String), (Option<u64>, Option<u64>)>,
    /// 新增条目时的 MAXLEN 近似裁剪输入框，留空表示不裁剪。
    redis_stream_maxlen_input: Entity<InputState>,
    /// 消费者组概览：(组名, 消费者数, 未确认数, 最后投递 ID, [(消费者名, 未确认, 空闲毫秒)])
    redis_stream_groups: BTreeMap<(TabId, String), Vec<RedisStreamGroupRow>>,
    redis_stream_groups_loading: Option<(TabId, String)>,
    /// 消费者组面板是否展开。
    redis_stream_groups_expanded: bool,
    redis_stream_entry_pages: BTreeMap<(TabId, String), RedisStreamEntryPage>,
    redis_stream_entry_loading: Option<(TabId, String)>,
    redis_stream_entry_more_loading: Option<(TabId, String)>,
    redis_stream_entry_generation: BTreeMap<u64, u64>,
    redis_list_item_search_active: Option<(TabId, String)>,
    redis_list_item_search_queries: BTreeMap<(TabId, String), String>,
    redis_list_item_search_pages: BTreeMap<(TabId, String, String), RedisListItemPage>,
    redis_list_item_search_loading: Option<(TabId, String, String)>,
    redis_list_item_search_more_loading: Option<(TabId, String, String)>,
    redis_list_item_search_generation: BTreeMap<u64, u64>,
    redis_list_item_search_syncing: bool,
    pending_redis_list_item_drawer: Option<RedisListItemDrawerForm>,
    redis_list_item_drawer_rows: Vec<RedisListItemInputs>,
    redis_list_item_drawer_scroll: ScrollHandle,
    // List 元素删除抽屉（LPOP/RPOP 按数量）：表单态 + 数量输入 + 位置选择（Select）+ 二次确认目标
    redis_list_item_remove_drawer: Option<RedisListItemRemoveDrawerForm>,
    redis_list_item_remove_confirm: Option<RedisListItemRemoveConfirmTarget>,
    redis_list_item_remove_count_input: Entity<InputState>,
    _redis_list_item_remove_count_subscription: Subscription,
    redis_list_item_remove_select: Entity<SelectState<SearchableVec<String>>>,
    _redis_list_item_remove_select_subscription: Subscription,
    // 「新增 Key」抽屉（对齐 RedisInsight AddKey）：公共字段 entity + 按类型子表单状态。
    redis_add_key_drawer: Option<RedisAddKeyDrawerForm>,
    redis_add_key_type_select: Entity<SelectState<SearchableVec<String>>>,
    _redis_add_key_type_select_subscription: Subscription,
    redis_add_key_name_input: Entity<InputState>,
    _redis_add_key_name_subscription: Subscription,
    redis_add_key_ttl_input: Entity<InputState>,
    _redis_add_key_ttl_subscription: Subscription,
    // 「新增 Key」抽屉中间可滚动表单区的滚动句柄。
    redis_add_key_scroll: ScrollHandle,
    // 类型专用子表单：String/JSON 单值多行输入；Hash 每字段带 TTL；ZSet/Stream 名称=值行；Set/List 成员行 + List 方向。
    redis_add_key_string_input: Entity<InputState>,
    _redis_add_key_string_subscription: Subscription,
    redis_add_key_json_input: Entity<InputState>,
    _redis_add_key_json_subscription: Subscription,
    redis_add_key_hash_rows: Vec<RedisAddKeyHashRow>,
    redis_add_key_zset_rows: Vec<RedisAddKeyNameValueRow>,
    redis_add_key_set_rows: Vec<RedisAddKeySingleRow>,
    redis_add_key_list_rows: Vec<RedisAddKeySingleRow>,
    redis_add_key_list_direction: RedisListDirection,
    redis_add_key_stream_id_input: Entity<InputState>,
    _redis_add_key_stream_id_subscription: Subscription,
    redis_add_key_stream_rows: Vec<RedisAddKeyNameValueRow>,
    redis_add_key_applying: bool,
    // List 元素行内编辑（LSET）：编辑输入框、hover 行、正在编辑的行
    redis_list_value_edit_input: Entity<InputState>,
    _redis_list_value_edit_subscription: Subscription,
    redis_list_item_hovered: Option<RedisListItemEditingState>,
    redis_list_item_editing: Option<RedisListItemEditingState>,
    _redis_key_value_apply_tasks: BTreeMap<u64, Task<()>>,
    tab_switcher_search_input: Entity<InputState>,
    _tab_switcher_search_subscription: Subscription,
    tab_switcher: Option<TabSwitcherKind>,
    tab_switcher_search: String,
    query_history_search_input: Entity<InputState>,
    _query_history_search_subscription: Subscription,
    query_history_search: String,
    query_history_quick_search_input: Entity<InputState>,
    _query_history_quick_search_subscription: Subscription,
    query_history_quick_search: String,
    query_history_quick_open: bool,
    query_history_quick_selected: usize,
    query_history_quick_kind_filter: QueryHistoryKindFilter,
    query_history_connection_select: Entity<
        SelectState<SearchableVec<QueryHistoryConnectionFilterItem>>,
    >,
    _query_history_connection_select_subscription: Subscription,
    query_history_database_select: Entity<SelectState<SearchableVec<QueryHistoryTextFilterItem>>>,
    _query_history_database_select_subscription: Subscription,
    query_history_table_select: Entity<SelectState<SearchableVec<QueryHistoryTextFilterItem>>>,
    _query_history_table_select_subscription: Subscription,
    sql_file_connection_select: Entity<SelectState<SearchableVec<SqlFileConnectionItem>>>,
    _sql_file_connection_select_subscription: Subscription,
    sql_file_database_select: Entity<SelectState<SearchableVec<SqlFileDatabaseItem>>>,
    _sql_file_database_select_subscription: Subscription,
    sql_file_encoding_select: Entity<SelectState<SearchableVec<SqlFileEncodingItem>>>,
    _sql_file_encoding_select_subscription: Subscription,
    sql_file_path_input: Entity<InputState>,
    _sql_file_path_subscription: Subscription,
    backup_file_name_input: Entity<InputState>,
    _backup_file_name_subscription: Subscription,
    backup_object_search_input: Entity<InputState>,
    _backup_object_search_subscription: Subscription,
    /// 新建备份弹框：备注输入（保存到 .meta.json）。
    backup_note_input: Entity<InputState>,
    _backup_note_subscription: Subscription,
    /// 备份 tab：查看「备份表」弹框（None = 未打开）。tables 为 None 表示该备份无表清单记录。
    backup_tables_modal: Option<BackupTablesModal>,
    /// 备份 tab：备注编辑弹框对应的备份文件路径（None = 未打开）。
    backup_note_modal_path: Option<PathBuf>,
    /// 备注编辑弹框的输入控件。
    backup_note_edit_input: Entity<InputState>,
    /// 运行中备份任务的元数据暂存：task_id → 表清单/备注；成功后写入 .meta.json 并移除。
    backup_pending_metas: BTreeMap<u64, BackupFileMeta>,
    /// 备份 tab：删除确认弹框对应的备份文件路径（None = 未打开）。
    pending_delete_backup: Option<PathBuf>,
    user_admin_search_input: Entity<InputState>,
    _user_admin_search_subscription: Subscription,
    user_admin_create_user_input: Entity<InputState>,
    _user_admin_create_user_subscription: Subscription,
    user_admin_create_host_input: Entity<InputState>,
    _user_admin_create_host_subscription: Subscription,
    user_admin_auth_plugin_select: Entity<SelectState<SearchableVec<String>>>,
    _user_admin_auth_plugin_select_subscription: Subscription,
    user_admin_password_expiry_select: Entity<SelectState<SearchableVec<String>>>,
    _user_admin_password_expiry_select_subscription: Subscription,
    user_admin_create_password_input: Entity<InputState>,
    _user_admin_create_password_subscription: Subscription,
    user_admin_new_password_input: Entity<InputState>,
    _user_admin_new_password_subscription: Subscription,
    user_admin_max_queries_input: Entity<InputState>,
    _user_admin_max_queries_subscription: Subscription,
    user_admin_max_updates_input: Entity<InputState>,
    _user_admin_max_updates_subscription: Subscription,
    user_admin_max_connections_input: Entity<InputState>,
    _user_admin_max_connections_subscription: Subscription,
    user_admin_max_user_connections_input: Entity<InputState>,
    _user_admin_max_user_connections_subscription: Subscription,
    user_admin_ssl_type_select: Entity<SelectState<SearchableVec<String>>>,
    _user_admin_ssl_type_select_subscription: Subscription,
    user_admin_ssl_cipher_input: Entity<InputState>,
    _user_admin_ssl_cipher_subscription: Subscription,
    user_admin_ssl_issuer_input: Entity<InputState>,
    _user_admin_ssl_issuer_subscription: Subscription,
    user_admin_ssl_subject_input: Entity<InputState>,
    _user_admin_ssl_subject_subscription: Subscription,
    user_admin_privilege_database_menu: Option<(TabId, u64)>,
    query_history_connection_filter: Option<ConnectionId>,
    query_history_database_filter: Option<String>,
    query_history_table_filter: Option<String>,
    query_history_kind_filter: QueryHistoryKindFilter,
    query_history_detail: Option<QueryHistoryEntry>,
    user_admin_password_visible: bool,
    pinned_tabs: BTreeSet<TabId>,
    tab_order: Vec<TabId>,
    workspace_tab_order: Vec<WorkspaceScope>,
    // 当前鼠标悬停的标签页：仅用于浮现标签操作按钮，不持久化。
    hovered_tab: Option<TabId>,
    hovered_database_tab: Option<WorkspaceScope>,
    display_database_search_input: Entity<InputState>,
    _display_database_search_subscription: Subscription,
    create_database_name_input: Entity<InputState>,
    _create_database_name_subscription: Subscription,
    create_database_charset_select: Entity<SelectState<SearchableVec<String>>>,
    _create_database_charset_select_subscription: Subscription,
    create_database_collation_select: Entity<SelectState<SearchableVec<String>>>,
    _create_database_collation_select_subscription: Subscription,
    danger_table_foreign_key_check_select: Entity<SelectState<SearchableVec<String>>>,
    _danger_table_foreign_key_check_select_subscription: Subscription,
    rename_table_input: Entity<InputState>,
    _rename_table_subscription: Subscription,
    copy_table_input: Entity<InputState>,
    _copy_table_subscription: Subscription,
    column_choice_value_input: Entity<InputState>,
    column_choice_label_input: Entity<InputState>,
    query_save_name_input: Entity<InputState>,
    _file_picker_task: Option<Task<()>>,
    _connection_tasks: BTreeMap<u64, Task<()>>,
    /// 侧边栏「刷新连接树」的后台任务。只保留最近一次：连点刷新时旧任务被丢弃即取消，
    /// 避免多轮刷新结果交错回写。
    _tree_refresh_task: Option<Task<()>>,
    _database_tasks: BTreeMap<String, Task<()>>,
    _data_load_tasks: BTreeMap<u64, Task<()>>,
    _query_execute_tasks: BTreeMap<u64, Task<()>>,
    _sql_file_execute_tasks: BTreeMap<u64, Task<()>>,
    _sql_file_cancel_flags: BTreeMap<u64, Arc<AtomicBool>>,
    _query_completion_tasks: BTreeMap<(TabId, u64), Task<()>>,
    _completion_index_tasks: BTreeMap<(ConnectionId, Option<String>), Task<()>>,
    _table_info_tasks: BTreeMap<(TabId, TableInfoTab), Task<()>>,
    _user_admin_users_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_grants_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_member_grants_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_apply_tasks: BTreeMap<TabId, Task<()>>,
    _create_table_apply_tasks: BTreeMap<TabId, Task<()>>,
    _create_table_reference_columns_tasks: BTreeMap<(TabId, u64), Task<()>>,
    _cell_binary_download_tasks: BTreeMap<(TabId, usize, usize), Task<()>>,
    _data_export_tasks: BTreeMap<u64, Task<()>>,
    _data_export_cancel_flags: BTreeMap<u64, Arc<AtomicBool>>,
    _backup_tasks: BTreeMap<u64, Task<()>>,
    _backup_cancel_flags: BTreeMap<u64, Arc<AtomicBool>>,
    _create_database_tasks: BTreeMap<u64, Task<()>>,
    _delete_database_tasks: BTreeMap<u64, Task<()>>,
    _rename_table_task: Option<Task<()>>,
    _copy_table_task: Option<Task<()>>,
    _copy_table_ddl_task: Option<Task<()>>,
    _copy_table_structure_task: Option<Task<()>>,
    _danger_table_task: Option<Task<()>>,
    data_export_task_seq: u64,
    _test_connection_task: Option<Task<()>>,
    /// Redis 连接串导入 / 云自动发现的异步任务。
    _redis_discover_task: Option<Task<()>>,
    /// Redis 连接串导入成功后，输入框实体待与表单重新同步的标记
    /// （异步回调里没有 `Window`，真正同步放在 `render` 消费该标记）。
    redis_discovery_pending_sync: bool,
    connection_context_menu: Option<ConnectionContextMenu>,
    database_context_menu: Option<DatabaseContextMenu>,
    table_context_menu: Option<TableContextMenu>,
    table_group_context_menu: Option<TableGroupContextMenu>,
    table_folder_context_menu: Option<TableFolderContextMenu>,
    tab_context_menu: Option<TabContextMenu>,
    data_cell_context_menu: Option<DataCellContextMenu>,
    data_row_context_menu: Option<DataRowContextMenu>,
    group_context_menu: Option<GroupContextMenu>,
    pending_query_save: Option<TabId>,
    pending_connection_query_save: Option<TabId>,
    pending_rename_group: Option<PendingRenameGroup>,
    pending_rename_table_folder: Option<PendingRenameTableFolder>,
    pending_delete_connection: Option<ConnectionId>,
    pending_delete_database: Option<PendingDeleteDatabase>,
    pending_disconnect_connection: Option<PendingDisconnectConnection>,
    pending_close_workspace: Option<PendingCloseWorkspace>,
    pending_new_query_connection: Option<ConnectionId>,
    pending_delete_data_row: Option<DataCellContextMenu>,
    data_row_viewer: Option<DataRowViewer>,
    pending_dirty_data_action: Option<PendingDirtyDataAction>,
    pending_apply_data_changes: Option<TabId>,
    pending_query_parameters: Option<PendingQueryParameterPrompt>,
    pending_dangerous_query: Option<PendingDangerousQuery>,
    pending_dangerous_redis_command: Option<PendingDangerousRedisCommand>,
    // 「执行 SQL 文件」弹框共享状态：dialog builder 在 NavicatMain::render 期间被同步执行，
    // 此时主视图处于 lease 状态，builder 内不能 view.read(cx)，实时数据统一放 Rc<RefCell<..>>
    // （先例：redis_hash_full_value_viewer）。
    sql_file_modal: Rc<std::cell::RefCell<SqlFileModalData>>,
    pending_data_export: Option<TableDataExportForm>,
    data_export_custom_conditions_open: bool,
    data_export_preview: Option<TableDataExportPreviewState>,
    data_export_preview_seq: u64,
    _data_export_preview_task: Option<Task<()>>,
    pending_table_data_export_after_load: Option<TabId>,
    pending_rename_table: Option<PendingRenameTable>,
    pending_copy_table: Option<PendingCopyTable>,
    pending_column_choices: Option<PendingColumnChoices>,
    pending_danger_table_action: Option<PendingDangerTableAction>,
    data_export_log_task: Option<u64>,
    data_export_tasks: Vec<TableDataExportTaskState>,
    pending_backup_modal: Option<BackupForm>,
    // 对象选择表列表：虚拟化滚动句柄（滚动偏移/视口跨帧保留，仅渲染视口附近的行）。
    backup_objects_scroll: VirtualListScrollHandle,
    backup_log_task: Option<u64>,
    backup_tasks: Vec<BackupTaskState>,
    backup_task_seq: u64,
    query_parameter_history: BTreeMap<String, String>,
    display_database_connection: Option<ConnectionId>,
    display_database_selection: BTreeSet<String>,
    display_database_search: String,
    display_database_show_system: bool,
    pending_create_database: Option<CreateDatabaseForm>,
    create_database_running: BTreeSet<ConnectionId>,
    new_connection_target_group: Option<ConnectionGroupId>,
    connecting_connections: BTreeSet<ConnectionId>,
    loading_databases: BTreeSet<String>,
    loaded_database_children: BTreeSet<String>,
    pinned_databases: BTreeSet<String>,
    pinned_tables: BTreeSet<String>,
    table_folders: BTreeMap<String, Vec<String>>,
    table_folder_assignments: BTreeMap<String, (String, String)>,
    selected_table_folder: Option<(String, String)>,
    expanded_databases: BTreeMap<String, bool>,
    expanded_object_groups: BTreeMap<String, bool>,
    data_table_states: BTreeMap<TabId, Entity<TableState<DataPageTableDelegate>>>,
    data_table_column_widths: BTreeMap<DataTableWidthKey, BTreeMap<String, Pixels>>,
    _data_table_width_subscriptions: BTreeMap<TabId, Subscription>,
    create_table_inputs: BTreeMap<CreateTableInputKey, Entity<InputState>>,
    _create_table_input_subscriptions: BTreeMap<CreateTableInputKey, Subscription>,
    create_table_comment_editor_sizes: BTreeMap<(TabId, u64), (f32, f32)>,
    create_table_check_expression_editor_sizes: BTreeMap<(TabId, u64), (f32, f32)>,
    create_table_index_field_dropdown: Option<CreateTableIndexFieldDropdownKey>,
    create_table_index_field_selection: Option<CreateTableIndexFieldSelectionKey>,
    create_table_foreign_key_field_selection: Option<CreateTableForeignKeyFieldSelectionKey>,
    create_table_foreign_key_fields_draft: Option<CreateTableForeignKeyFieldsDraft>,
    create_table_foreign_key_referenced_fields_draft:
        Option<CreateTableForeignKeyReferencedFieldsDraft>,
    create_table_comment_editor_resize_start: Option<CreateTableCommentEditorResizeStart>,
    create_table_check_expression_editor_resize_start:
        Option<CreateTableCheckExpressionEditorResizeStart>,
    create_table_type_selects: BTreeMap<(TabId, u64), Entity<SelectState<SearchableVec<String>>>>,
    _create_table_type_select_subscriptions: BTreeMap<(TabId, u64), Subscription>,
    create_table_selects: BTreeMap<CreateTableSelectKey, Entity<SelectState<SearchableVec<String>>>>,
    _create_table_select_subscriptions: BTreeMap<CreateTableSelectKey, Subscription>,
    data_export_object_selects:
        BTreeMap<DataExportObjectSelectKey, Entity<SelectState<SearchableVec<String>>>>,
    _data_export_object_select_subscriptions: BTreeMap<DataExportObjectSelectKey, Subscription>,
    query_editors: BTreeMap<TabId, Entity<editor_component::Editor>>,
    /// 查询页语句执行状态仓库（按 tab 缓存；与 SqlAdapter 共享，供装饰还原行背景）。
    query_statement_statuses: BTreeMap<TabId, sql_editor_adapter::SqlStatusStore>,
    query_save_targets: BTreeMap<TabId, QuerySaveTarget>,
    saved_queries: Vec<SavedQuery>,
    /// 启动后是否已完成持久化历史/保存查询的一次性异步恢复（防重复触发）。
    persisted_history_loaded: bool,
    _query_editor_subscriptions: BTreeMap<TabId, Subscription>,
    /// 查询页查找/替换输入框（按 tab 缓存；宿主渲染 find 面板用）。
    #[allow(dead_code)] // 查找面板（table_state::query_find_state 等）尚未接入宿主渲染，保留字段。
    query_find_inputs: BTreeMap<TabId, Entity<InputState>>,
    #[allow(dead_code)] // 同上，替换输入框随查找面板一并预留。
    query_replace_inputs: BTreeMap<TabId, Entity<InputState>>,
    _query_find_input_subscriptions: BTreeMap<TabId, Subscription>,
    _query_replace_input_subscriptions: BTreeMap<TabId, Subscription>,
    /// workbench 命令编辑器（按 tab 缓存）。
    /// 迁移到通用 `editor_component::Editor` 后，命令输入、补全、执行全部经本实体。
    /// `pub(crate)`：供 crate 根作用域读取，用于事件订阅与生命周期管理。
    pub(crate) redis_workbench_inputs: BTreeMap<TabId, Entity<editor_component::Editor>>,
    _redis_workbench_input_subscriptions: BTreeMap<TabId, Subscription>,
    // Redis CLI 终端会话：按 tab 缓存复用同一个 PTY 会话。
    // 仅在通过数据库右键菜单「Redis CLI」打开时创建，生命周期随 tab 关闭而销毁。
    terminal_sessions: BTreeMap<TabId, Entity<TerminalComponent>>,
    // 周期 pumping 各终端 PTY 输出的后台任务（首开终端时懒创建，drop 即停止）。
    _terminal_session_pump: Option<gpui::Task<()>>,
    // Redis Pub/Sub 会话：按 tab 缓存复用同一个订阅连接，生命周期随 tab 关闭而销毁。
    pubsub_sessions: BTreeMap<TabId, PubSubSessionModel>,
    // 周期消费各 Pub/Sub 会话消息并合并进 UI 的后台任务（首开会话时懒创建，drop 即停止）。
    _pubsub_pump: Option<gpui::Task<()>>,
    // Redis Workbench 上下分栏拖动状态（全局，跨 tab 共享同一分栏占比）。
    // 注：Workbench 历史持久化已移除，只保留当前会话态；后期恢复历史时在此
    // 新增「按连接 + DB 分组」的会话历史与持久化入口，不复用 SQL 历史。
    redis_workbench_panel_resize_start: Option<RedisWorkbenchPanelResizeStart>,
    query_output_tabs: BTreeMap<TabId, QueryOutputTab>,
    collapsed_query_outputs: BTreeSet<TabId>,
    results_placement: ResultsPlacement,
    query_result_display_pages: BTreeMap<QueryResultDisplayKey, SortedQueryResultPage>,
    settings_panel_section: SettingsPanelSection,
    settings_editor_draft: Settings,
    /// 设置面板「危险 SQL 操作清单」折叠区是否展开（UI 瞬时状态，不进渲染快照）。
    settings_dangerous_actions_collapsed: bool,
    settings_font_size_slider: Entity<SliderState>,
    _settings_font_size_slider_subscription: Subscription,
    settings_line_height_input: Entity<InputState>,
    _settings_line_height_subscription: Subscription,
    settings_radius_input: Entity<InputState>,
    _settings_radius_subscription: Subscription,
    query_output_heights: BTreeMap<TabId, f32>,
    query_output_widths: BTreeMap<TabId, f32>,
    query_output_resize_start: Option<QueryOutputResizeStart>,
    query_result_sort_rules: BTreeMap<QueryResultSortKey, Vec<DataSortRule>>,
    query_result_cell_detail: BTreeMap<TabId, QueryResultCellDetail>,
    data_change_sql_preview_tabs: BTreeSet<TabId>,
    data_filter_panels: BTreeSet<TabId>,
    data_filter_rules: BTreeMap<TabId, Vec<DataFilterRule>>,
    data_sort_rules: BTreeMap<TabId, Vec<DataSortRule>>,
    data_filter_draft_rules: BTreeMap<TabId, Vec<DataFilterRule>>,
    data_sort_draft_rules: BTreeMap<TabId, Vec<DataSortRule>>,
    data_filter_grouped_tabs: BTreeSet<TabId>,
    data_filter_modes: BTreeMap<TabId, DataFilterMode>,
    data_filter_texts: BTreeMap<TabId, String>,
    data_sort_texts: BTreeMap<TabId, String>,
    data_filter_panel_heights: BTreeMap<TabId, f32>,
    data_filter_panel_resize_start: Option<DataFilterPanelResizeStart>,
    cell_detail_drawer_heights: BTreeMap<TabId, f32>,
    cell_detail_drawer_resize_start: Option<CellDetailDrawerResizeStart>,
    data_filter_popover: Option<DataFilterPopover>,
    local_filter_popover: Option<LocalFilterPopover>,
    local_filter_manager_popover: Option<TabId>,
    local_filter_value: String,
    local_filter_search: String,
    local_filter_draft_values: BTreeSet<String>,
    local_filter_manager_field: Option<String>,
    local_filter_manager_draft_filters: BTreeMap<String, BTreeSet<String>>,
    local_filter_manager_field_open: bool,
    local_filter_manager_values_open: bool,
    local_table_filters: BTreeMap<TabId, BTreeMap<String, BTreeSet<String>>>,
    query_history_open: bool,
    /// Redis Workbench 历史抽屉是否展开。
    pub redis_history_open: bool,
    /// 打开 Redis 历史抽屉时锚定的作用域（连接 + 逻辑库）。
    pub redis_history_scope: Option<WorkbenchHistoryScope>,
    /// Redis 历史抽屉搜索框：输入实体 / 订阅 / 当前搜索文本（仅对已 load 的历史做本地过滤）。
    redis_history_search_input: Entity<InputState>,
    _redis_history_search_subscription: Subscription,
    redis_history_search: String,
    data_filter_value_input_text: String,
    data_filter_value_search: String,
    data_filter_value_search_loading_until: Option<Instant>,
    data_filter_value_search_task: Option<Task<()>>,
    data_filter_applying_tabs: BTreeSet<TabId>,
    _data_filter_apply_tasks: BTreeMap<u64, Task<()>>,
    app_message: Option<AppMessage>,
    _app_message_task: Option<Task<()>>,
    table_hover_color: Option<Hsla>,
    table_hover_blocked_by_overlay: bool,
    show_connection_browser: bool,
    connection_browser_width: f32,
    connection_browser_resize_start: Option<SidebarResizeStart>,
    table_info_resize_start: Option<SidebarResizeStart>,
}

#[derive(Clone)]
struct SidebarTreeCache {
    key: SidebarTreeCacheKey,
    rows: Rc<Vec<SidebarVisibleRow>>,
    item_sizes: Rc<Vec<Size<Pixels>>>,
}

#[derive(Clone, PartialEq)]
struct SidebarTreeCacheKey {
    connections: Vec<ConnectionState>,
    sidebar_layout: SidebarLayout,
    loading_databases: BTreeSet<String>,
    pinned_databases: BTreeSet<String>,
    pinned_tables: BTreeSet<String>,
    table_folders: BTreeMap<String, Vec<String>>,
    table_folder_assignments: BTreeMap<String, (String, String)>,
    expanded_databases: BTreeMap<String, bool>,
    expanded_object_groups: BTreeMap<String, bool>,
    saved_queries: Vec<SavedQuery>,
    search_query: String,
}

#[derive(Clone)]
struct PendingQueryParameterPrompt {
    tab_id: TabId,
    sql: String,
    execution: PendingQueryExecution,
    parameters: Vec<QueryParameterInput>,
    active_mode: QueryParameterInputMode,
    bulk_input: Entity<InputState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueryParameterInputMode {
    Fields,
    Array,
}

#[derive(Clone)]
enum PendingQueryExecution {
    All { statements: Vec<SqlStatementRun> },
    Text,
    Statement { statement: SqlStatementRun },
}

#[derive(Clone)]
struct PendingDangerousQuery {
    tab_id: TabId,
    text: String,
    execution: PendingQueryExecution,
}

/// Redis Workbench 危险命令二次确认目标：确认后才真正执行破坏性命令。
/// `execution_id` 非空表示确认后重跑结果区某条执行记录（而非顶部输入框草稿）。
#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingDangerousRedisCommand {
    tab_id: TabId,
    text: String,
    execution_id: Option<u64>,
}

#[derive(Clone)]
struct QueryParameterInput {
    key: String,
    label: String,
    input: Entity<InputState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SqlFileExecutionTab {
    General,
    Log,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SqlFileExecutionForm {
    connection_id: ConnectionId,
    database: Option<String>,
    path: Option<PathBuf>,
    encoding: SqlFileEncoding,
    continue_on_error: bool,
    split_statements: bool,
    active_tab: SqlFileExecutionTab,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SqlFileExecutionLogEntry {
    index: usize,
    success: bool,
    elapsed_ms: u64,
    message: String,
    sql: String,
}

#[derive(Clone, Debug)]
struct SqlFileExecutionTaskState {
    id: u64,
    file_name: String,
    path: PathBuf,
    connection_id: ConnectionId,
    database: Option<String>,
    tab_id: Option<TabId>,
    total: usize,
    processed: usize,
    errors: usize,
    started_at: Instant,
    finished_at: Option<Instant>,
    logs: Vec<SqlFileExecutionLogEntry>,
    error: Option<String>,
    cancel_requested: bool,
    canceled: bool,
}

impl SqlFileExecutionTaskState {
    fn running(&self) -> bool {
        self.finished_at.is_none()
    }
}

/// 「执行 SQL 文件」gpui-component Dialog 的共享状态。
/// form 与 log_task 互斥：form 有值表示展示「常规」表单，log_task 有值表示展示任务日志。
/// tasks 同时被状态栏任务芯片读取，因此必须放在 Rc 里与弹框共享。
struct SqlFileModalData {
    form: Option<SqlFileExecutionForm>,
    log_task: Option<u64>,
    tasks: Vec<SqlFileExecutionTaskState>,
    task_seq: u64,
    /// 弹框是否已打开（防止重复 open_dialog；关闭路径统一清此标志）。
    dialog_open: bool,
}

impl Default for SqlFileModalData {
    fn default() -> Self {
        Self {
            form: None,
            log_task: None,
            tasks: Vec::new(),
            task_seq: 0,
            dialog_open: false,
        }
    }
}

/// 数据库备份方式。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackupMode {
    /// 原生工具（mysqldump / sqlite3）
    Native,
    /// 逻辑备份（SQL dump，复用现有导出能力）
    Logic,
    /// 自动探测：原生工具可用则原生，否则逻辑。
    Auto,
}

impl BackupMode {
    fn label(self) -> &'static str {
        match self {
            Self::Native => "原生备份（推荐）",
            Self::Logic => "逻辑备份（SQL）",
            Self::Auto => "自动选择",
        }
    }
}

/// 备份配置弹框页签（Navicat 风格）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackupTab {
    General,
    Objects,
    Advanced,
    Log,
}

impl BackupTab {
    const ALL: [Self; 4] = [Self::General, Self::Objects, Self::Advanced, Self::Log];

    fn label(self) -> &'static str {
        match self {
            Self::General => "常规",
            Self::Objects => "对象选择",
            Self::Advanced => "高级",
            Self::Log => "消息日志",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BackupForm {
    connection_id: ConnectionId,
    database: Option<String>,
    mode: BackupMode,
    target_dir: String,
    tab: BackupTab,
    /// 常规：文件名/模板；留空用默认（库名_时间戳.sql）。
    file_name: String,
    /// 对象选择：已勾选的表名集合（空集合 = 全选，由对象页签快速勾选改变）。
    selected_tables: BTreeSet<String>,
    /// 对象选择：当前库下全部表名（弹框打开时快照，用于渲染表列表，避免渲染期读 self）。
    all_table_names: BTreeSet<String>,
    /// 对象选择：当前库下全部视图名（快照，用于「视图」分组展示与统计）。
    all_view_names: BTreeSet<String>,
    /// 对象选择：表列表的搜索关键词（仅过滤显示，不影响勾选集合）。
    object_search: String,
    /// 对象选择：是否包含视图（默认 true）。
    include_views: bool,
    /// 高级：锁定所有表（--lock-all-tables，仅原生 MySQL/TiDB）。
    lock_tables: bool,
    /// 高级：使用单一事务（--single-transaction，默认 true 保持现状）。
    single_transaction: bool,
    /// 高级：包含存储过程/函数（--routines，默认 true）。
    /// MySQL 9.x 客户端 + 老版本服务端时，--routines 会触发查询服务端不存在的
    /// INFORMATION_SCHEMA.LIBRARIES 导致备份失败，此时可取消勾选。
    include_routines: bool,
    /// 高级：包含表结构（逻辑备份 DDL）。
    include_schema: bool,
    /// 高级：包含表数据（逻辑备份数据行）。
    include_data: bool,
    /// 常规：备注（可空）。备份成功后随表清单写入 {文件}.meta.json。
    note: String,
}

/// 备份文件的旁挂元数据（{备份文件}.meta.json）：记录表清单与备注。
/// 仅新备份写入；历史备份无此文件时对应列显示「无记录」。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct BackupFileMeta {
    /// 本次备份勾选的表名（有序数组；Some(空数组) = 整库备份；None = 未记录，如仅编辑备注的老备份）。
    #[serde(default)]
    tables: Option<Vec<String>>,
    /// 是否包含视图。
    #[serde(default)]
    include_views: bool,
    /// 用户备注。
    #[serde(default)]
    note: String,
}

/// 备份 tab「查看备份表」弹框状态。
#[derive(Clone, Debug, Eq, PartialEq)]
struct BackupTablesModal {
    /// 备份文件名（标题展示）。
    file_name: String,
    /// 表清单；None = 该备份无元数据记录（历史备份）。
    tables: Option<Vec<String>>,
    /// 是否包含视图（仅在有元数据时有意义）。
    include_views: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BackupLogEntry {
    index: usize,
    stage: String,
    success: bool,
    message: String,
}

#[derive(Clone, Debug, PartialEq)]
struct BackupTaskState {
    id: u64,
    connection_id: ConnectionId,
    database: Option<String>,
    mode: BackupMode,
    output_path: PathBuf,
    stage: String,
    started_at: Instant,
    finished_at: Option<Instant>,
    logs: Vec<BackupLogEntry>,
    skipped: Vec<String>,
    error: Option<String>,
    cancel_requested: bool,
    canceled: bool,
}

impl BackupTaskState {
    fn running(&self) -> bool {
        self.finished_at.is_none()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableDataExportTab {
    General,
    Fields,
    Conditions,
    Log,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableDataExportFormat {
    Sql,
    Txt,
    Csv,
    Json,
    Xml,
}

impl TableDataExportFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Sql => "SQL",
            Self::Txt => "TXT",
            Self::Csv => "CSV",
            Self::Json => "JSON",
            Self::Xml => "XML",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Sql => "sql",
            Self::Txt => "txt",
            Self::Csv => "csv",
            Self::Json => "json",
            Self::Xml => "xml",
        }
    }

    fn all() -> &'static [Self] {
        &[Self::Sql, Self::Txt, Self::Csv, Self::Json, Self::Xml]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableDataExportScope {
    CurrentConditions,
    AllRows,
    CustomRules,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TableDataExportColumn {
    name: String,
    type_name: Option<String>,
    nullable: bool,
    primary_key: bool,
    comment: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct TableDataExportForm {
    tab_id: TabId,
    object: ObjectPath,
    columns: Vec<TableDataExportColumn>,
    selected_fields: BTreeSet<String>,
    format: TableDataExportFormat,
    scope: TableDataExportScope,
    sort: Vec<SortSpec>,
    filters: Vec<FilterSpec>,
    custom_filter_rules: Vec<DataFilterRule>,
    custom_sort_rules: Vec<DataSortRule>,
    active_tab: TableDataExportTab,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TableDataExportPreviewStatus {
    Loading,
    Ready { sql: String, row_count: u64 },
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TableDataExportPreviewState {
    key: String,
    status: TableDataExportPreviewStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TableDataExportLogEntry {
    index: usize,
    success: bool,
    elapsed_ms: u64,
    message: String,
}

#[derive(Clone, Debug)]
struct TableDataExportTaskState {
    id: u64,
    table_name: String,
    path: PathBuf,
    format: TableDataExportFormat,
    field_count: usize,
    exported_rows: u64,
    batch_count: u64,
    started_at: Instant,
    finished_at: Option<Instant>,
    logs: Vec<TableDataExportLogEntry>,
    error: Option<String>,
    cancel_requested: bool,
    canceled: bool,
}

impl TableDataExportTaskState {
    fn running(&self) -> bool {
        self.finished_at.is_none()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SqlFileEncoding {
    Utf8,
    Utf8Bom,
    Gbk,
    Gb18030,
    Utf16Le,
    Utf16Be,
}

impl SqlFileEncoding {
    fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Gbk => "GBK",
            Self::Gb18030 => "GB18030",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ConnectionContextMenu {
    connection_id: ConnectionId,
    position: Point<Pixels>,
    show_group_submenu: bool,
}

#[derive(Clone, Debug)]
struct DatabaseContextMenu {
    connection_id: ConnectionId,
    database_path: ObjectPath,
    database: String,
    position: Point<Pixels>,
    expanded: bool,
    /// 备份节点右键专用：仅渲染「新建备份」一个菜单项。
    backup_only: bool,
}

#[derive(Clone, Debug)]
struct TableContextMenu {
    object_path: ObjectPath,
    position: Point<Pixels>,
    submenu: Option<TableContextSubmenu>,
}

#[derive(Clone, Debug)]
struct TableGroupContextMenu {
    connection_id: ConnectionId,
    database_path: ObjectPath,
    database: String,
    position: Point<Pixels>,
}

#[derive(Clone, Debug)]
struct TableFolderContextMenu {
    parent_key: String,
    name: String,
    position: Point<Pixels>,
}

#[derive(Clone, Debug)]
struct PendingRenameTableFolder {
    parent_key: String,
    original_name: String,
    name: String,
}

#[derive(Clone, Debug)]
struct PendingRenameTable {
    object_path: ObjectPath,
    new_name: String,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct PendingCopyTable {
    object_path: ObjectPath,
    new_name: String,
    copy_data: bool,
    source_ddl: Option<Result<String, String>>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct PendingColumnChoices {
    tab_id: TabId,
    object_path: ObjectPath,
    column_name: String,
    type_name: String,
    choices: Vec<ColumnChoice>,
    adding: bool,
}

#[derive(Clone, Debug)]
struct PendingDangerTableAction {
    object_path: ObjectPath,
    action: DangerTableAction,
    foreign_key_check: ForeignKeyCheckMode,
    acknowledged: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DangerTableAction {
    Drop,
    Truncate,
}

impl DangerTableAction {
    fn title(self) -> &'static str {
        match self {
            Self::Drop => "确认删除表",
            Self::Truncate => "确认清空表",
        }
    }

    fn prompt(self) -> &'static str {
        match self {
            Self::Drop => "你确定要删除",
            Self::Truncate => "你确定要清空",
        }
    }

    fn confirm_label(self) -> &'static str {
        match self {
            Self::Drop => "删除",
            Self::Truncate => "清空",
        }
    }

    fn running_message(self) -> &'static str {
        match self {
            Self::Drop => "正在删除表",
            Self::Truncate => "正在清空表",
        }
    }

    fn sql_preview_key(self) -> &'static str {
        match self {
            Self::Drop => "drop",
            Self::Truncate => "truncate",
        }
    }
}

fn danger_table_foreign_key_check_options() -> Vec<String> {
    ["默认", "启用", "禁用"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn danger_table_foreign_key_check_index(mode: ForeignKeyCheckMode) -> Option<IndexPath> {
    let index = match mode {
        ForeignKeyCheckMode::Default => 0,
        ForeignKeyCheckMode::Enable => 1,
        ForeignKeyCheckMode::Disable => 2,
    };
    Some(IndexPath::new(index))
}

fn danger_table_foreign_key_check_from_label(label: &str) -> ForeignKeyCheckMode {
    match label {
        "启用" => ForeignKeyCheckMode::Enable,
        "禁用" => ForeignKeyCheckMode::Disable,
        _ => ForeignKeyCheckMode::Default,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableContextSubmenu {
    Export,
    ManageGroup,
}

#[derive(Clone, Copy, Debug)]
struct TabContextMenu {
    tab_id: TabId,
    position: Point<Pixels>,
}

#[derive(Clone, Debug)]
struct DataCellContextMenu {
    tab_id: TabId,
    position: Point<Pixels>,
    source_row: usize,
    query_result_page_index: Option<usize>,
    source_col: usize,
    column_name: String,
    type_name: String,
    nullable: bool,
    value: String,
    submenu: Option<DataCellContextSubmenu>,
    selection_row_count: usize,
    selection_copy_label: Option<String>,
    selection_export_label: Option<String>,
}

#[derive(Clone, Debug)]
struct DataRowContextMenu {
    tab_id: TabId,
    position: Point<Pixels>,
    source_row: usize,
    query_result_page_index: Option<usize>,
    rows_editable: bool,
    row_object_available: bool,
    submenu: Option<DataRowContextSubmenu>,
    selection_row_count: usize,
    selection_copy_label: Option<String>,
    selection_export_label: Option<String>,
}

#[derive(Clone, Debug)]
struct DataRowViewer {
    tab_id: TabId,
    source_row: usize,
    query_result_page_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataCellContextSubmenu {
    Filter,
    Sort,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataRowContextSubmenu {
    Copy,
    Export,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataRowCopyKind {
    Json,
    Insert,
    InsertWithoutPrimaryKey,
    Update,
    Tsv,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataRowExportFormat {
    Csv,
    Json,
    Markdown,
    SqlInsert,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum QuerySaveTarget {
    Local(PathBuf),
    Connection(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingDisconnectConnection {
    connection_id: ConnectionId,
    unsaved_queries: usize,
    running_queries: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingCloseWorkspace {
    scope: WorkspaceScope,
    unsaved_queries: usize,
    running_queries: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CreateDatabaseForm {
    connection_id: ConnectionId,
    database_name: String,
    charset: String,
    database_kind: DatabaseKind,
    collation: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataTableSelectionKind {
    Cells,
    Rows,
}

#[derive(Clone, Debug)]
struct DataTableSelectionExport {
    kind: DataTableSelectionKind,
    text: String,
    rows: usize,
    cells: usize,
}

#[derive(Clone, Debug)]
struct RowFieldSnapshot {
    index: usize,
    name: String,
    type_name: String,
    primary_key: bool,
    comment: Option<String>,
    value: CellValue,
}

#[derive(Clone, Debug)]
struct DataRowSnapshot {
    object: Option<ObjectPath>,
    display_name: String,
    fields: Vec<RowFieldSnapshot>,
}

#[derive(Clone, Debug)]
enum PendingDirtyDataAction {
    ApplyFilterSort(TabId),
    HeaderSort {
        tab_id: TabId,
        field: String,
        direction: Option<DataTableSortDirection>,
    },
    ContextFilter {
        menu: DataCellContextMenu,
        operator: DataFilterOperator,
    },
    ContextSort {
        menu: DataCellContextMenu,
        ascending: bool,
    },
    RemoveContextFilter(DataCellContextMenu),
    RemoveContextSort(DataCellContextMenu),
    ClearFilterSort(TabId),
    SetPagination {
        tab_id: TabId,
        offset: u64,
        limit: u64,
    },
}

impl PendingDirtyDataAction {
    fn tab_id(&self) -> TabId {
        match self {
            Self::ApplyFilterSort(tab_id) | Self::ClearFilterSort(tab_id) => *tab_id,
            Self::SetPagination { tab_id, .. } => *tab_id,
            Self::HeaderSort { tab_id, .. } => *tab_id,
            Self::ContextFilter { menu, .. }
            | Self::ContextSort { menu, .. }
            | Self::RemoveContextFilter(menu)
            | Self::RemoveContextSort(menu) => menu.tab_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TabSwitcherKind {
    Databases,
    Tables,
}

#[derive(Clone, Debug)]
struct TabSwitcherEntry {
    id: TabId,
    title: String,
    active: bool,
    icon: AppIcon,
}

#[derive(Clone, Debug)]
struct DraggedTab {
    tab_id: TabId,
    title: String,
}

#[derive(Clone, Debug)]
struct DraggedWorkspaceTab {
    scope: WorkspaceScope,
    title: String,
}

impl Render for DraggedTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        drag_preview(self.title.clone(), ComponentTheme::global(cx).radius_lg)
    }
}

impl Render for DraggedWorkspaceTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        drag_preview(self.title.clone(), ComponentTheme::global(cx).radius_lg)
    }
}

#[derive(Clone, Copy, Debug)]
struct GroupContextMenu {
    group_id: ConnectionGroupId,
    position: Point<Pixels>,
}

#[derive(Clone, Debug)]
struct PendingRenameGroup {
    group_id: ConnectionGroupId,
    name: String,
}

#[derive(Clone, Copy, Debug)]
struct SidebarResizeStart {
    x: f32,
    width: f32,
}

/// Redis Workbench 分栏拖动起点。分栏尺寸是全局设置（非按 tab），
/// 故无需 tab_id（对齐 RedisInsight 的全局 panelSizes 语义）。
///
/// **必须把 `placement` 一起记下来**，不能在拖动时读实时的全局值：`Div::on_mouse_up`
/// 只在指针仍悬停在手柄上时才触发（gpui 的 hover 判定），正常拖拽手势在结果区松手，
/// 于是起点常常不被清理。若此时布局被工具栏切过，残留的「高度」会被当成「宽度」用。
/// 连 `x`/`y` 一起记录，可保证起点记录的三元组（轴、基准、尺寸）自洽。
#[derive(Clone, Copy, Debug)]
struct RedisWorkbenchPanelResizeStart {
    placement: ResultsPlacement,
    x: f32,
    y: f32,
    size: f32,
}

#[derive(Clone, Copy, Debug)]
struct DataFilterPanelResizeStart {
    tab_id: TabId,
    y: f32,
    height: f32,
}

#[derive(Clone, Copy, Debug)]
struct CellDetailDrawerResizeStart {
    tab_id: TabId,
    y: f32,
    height: f32,
}

#[derive(Clone, Copy, Debug)]
struct QueryOutputResizeStart {
    tab_id: TabId,
    placement: ResultsPlacement,
    x: f32,
    y: f32,
    size: f32,
}

#[derive(Clone, Copy, Debug)]
struct CreateTableCommentEditorResizeStart {
    tab_id: TabId,
    column_id: u64,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy, Debug)]
struct CreateTableCheckExpressionEditorResizeStart {
    tab_id: TabId,
    check_id: u64,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}


#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct QueryResultSortKey {
    tab_id: TabId,
    result_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct QueryResultDisplayKey {
    tab_id: TabId,
    result_index: usize,
    page_index: usize,
}

#[derive(Clone, Debug)]
struct QueryResultRefreshRequest {
    tab_id: TabId,
    result_index: usize,
    page_index: usize,
    sql: String,
    offset: u64,
    limit: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DataTableWidthKey {
    tab_id: TabId,
    query_result_page_index: Option<usize>,
}

#[derive(Clone)]
struct DataPageTableDelegate {
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    columns: Vec<TableColumn>,
    source_column_indexes: Vec<Option<usize>>,
    column_types: Vec<String>,
    column_meta: Vec<DataTableColumnMeta>,
    rows: Vec<Vec<SharedString>>,
    source_row_indexes: Vec<usize>,
    null_cells: BTreeSet<(usize, usize)>,
    dirty_cells: BTreeSet<(usize, usize)>,
    deleted_rows: BTreeSet<usize>,
    sorts: Vec<DataTableSort>,
    selected_cell: Option<(usize, usize)>,
    selected_row: Option<usize>,
    selected_cells: BTreeSet<(usize, usize)>,
    selected_rows: BTreeSet<usize>,
    selection_anchor: Option<DataTableSelectionAnchor>,
    hovered_cell: Option<(usize, usize)>,
    search_matches: Vec<DataSearchMatch>,
    active_search_match: Option<DataSearchMatch>,
    highlight_search_matches: bool,
    highlighted_column: Option<String>,
    query_result_page_index: Option<usize>,
    edit_input: Entity<InputState>,
    editing_cell: Option<DataCellEditState>,
    temporal_part_input: Entity<InputState>,
    temporal_part_editing: Option<TemporalPartEditState>,
    cells_editable: bool,
    header_actions: bool,
    show_row_index: bool,
    redis_page: bool,
    on_sort: DataTableSortHandler,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataTableSelectionAnchor {
    Cell { row: usize, col: usize },
    Row { row: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueryOutputTab {
    Result(usize),
    Summary,
}

impl QueryOutputTab {
    fn result_index(self) -> Option<usize> {
        match self {
            QueryOutputTab::Result(index) => Some(index),
            QueryOutputTab::Summary => None,
        }
    }
}

fn active_query_result_editor_state(editor: &QueryEditorState) -> Option<&DataEditorState> {
    editor
        .active_result_editor
        .and_then(|page_index| editor.result_editors.get(&page_index))
}

type DataTableSortHandler = Arc<dyn Fn(TabId, String, Option<DataTableSortDirection>, &mut App)>;

#[derive(Clone, Debug)]
struct DataTableColumnMeta {
    name: String,
    type_name: String,
    comment: Option<String>,
    nullable: bool,
    primary_key: bool,
    choices: Vec<ColumnChoice>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ColumnChoice {
    value: String,
    #[serde(default)]
    label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DataCellEditState {
    tab_id: TabId,
    query_result_page_index: Option<usize>,
    visible_row: usize,
    source_row: usize,
    col_ix: usize,
    source_col: usize,
    temporal_kind: Option<DataCellTemporalKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TemporalPartEditState {
    target: TemporalEditTarget,
    part: TemporalPart,
    kind: DataCellTemporalKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TemporalEditTarget {
    DataCell(DataCellEditState),
    CellDetail(TabId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QueryResultCellDetail {
    row: usize,
    column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataCellTemporalKind {
    Date,
    Time,
    DateTime,
}

impl DataCellTemporalKind {
    fn has_date(self) -> bool {
        matches!(self, Self::Date | Self::DateTime)
    }

    fn has_time(self) -> bool {
        matches!(self, Self::Time | Self::DateTime)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataCellEditorKind {
    Text,
    Boolean,
    Enum,
    Set,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TemporalPart {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DataTableSort {
    col_ix: usize,
    direction: DataTableSortDirection,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DataSearchMatch {
    row_ix: usize,
    col_ix: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalFilterPopover {
    tab_id: TabId,
    field_name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataTableSortDirection {
    Ascending,
    Descending,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppMessageKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AppMessage {
    id: u64,
    text: String,
    kind: AppMessageKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueryHistoryKindFilter {
    All,
    Query,
    DataChange,
    SchemaChange,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueryHistoryConnectionFilterItem {
    label: String,
    value: Option<ConnectionId>,
}

impl SelectItem for QueryHistoryConnectionFilterItem {
    type Value = Option<ConnectionId>;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueryHistoryTextFilterItem {
    label: String,
    value: Option<String>,
}

impl SelectItem for QueryHistoryTextFilterItem {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SqlFileConnectionItem {
    label: String,
    value: ConnectionId,
}

impl SelectItem for SqlFileConnectionItem {
    type Value = ConnectionId;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SqlFileDatabaseItem {
    label: String,
    value: Option<String>,
}

impl SelectItem for SqlFileDatabaseItem {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SqlFileEncodingItem {
    label: String,
    value: SqlFileEncoding,
}

impl SelectItem for SqlFileEncodingItem {
    type Value = SqlFileEncoding;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}
