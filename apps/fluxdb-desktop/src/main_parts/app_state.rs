#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum CreateTableInputKey {
    TableName(TabId),
    TableComment(TabId),
    ColumnName(TabId, u64),
    ColumnLength(TabId, u64),
    ColumnCustomType(TabId, u64),
    ColumnEnumValue(TabId, u64, usize),
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
    AiModels,
    AiToolsMcp,
    AiAgents,
    AiSessions,
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
        field_input: cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.redis.form.field"))),
        value_input: cx.new(|cx| InputState::new(window, cx).placeholder(tr("app.redis.form.value"))),
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
        let mut input =
            InputState::new(window, cx).placeholder(tr("app.redis.form.enter_member"));
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
        let mut input = InputState::new(window, cx).placeholder(tr("app.redis.form.field"));
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
        let mut input = InputState::new(window, cx).placeholder(tr("app.redis.form.value"));
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
        let mut input = InputState::new(window, cx).placeholder(tr("app.redis.form.ttl"));
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
    placeholder: impl Into<SharedString>,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

/// 「新增 Key」名称=值型行的「值」输入（Hash/Stream=Value，ZSet=Score）。
fn new_redis_add_key_pair_value_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    placeholder: impl Into<SharedString>,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

/// 「新增 Key」单成员型行的成员输入（Set=Member，List=Element）。
fn new_redis_add_key_single_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    placeholder: impl Into<SharedString>,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

fn new_redis_zset_member_input(
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder(tr("app.redis.form.member"));
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
        let mut input = InputState::new(window, cx).placeholder(tr("app.redis.form.score"));
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
        let mut input = InputState::new(window, cx).placeholder(tr("app.redis.form.value"));
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

/// MongoDB「显示连接信息」弹框状态：加载中 / 已加载 / 加载失败。
///
/// 内容全部来自服务端命令（`hello` / `buildInfo` / `listDatabases`），UI 不补造字段。
struct MongoInfoDialog {
    connection_id: ConnectionId,
    state: LoadState<fluxdb_core::MongoServerInfo>,
}

/// MongoDB「创建集合」弹框：`MongoDB` 的库由首个集合隐式创建，故只需连接 + 库 + 集合名。
#[derive(Clone, Debug)]
struct MongoCreateCollectionForm {
    connection_id: ConnectionId,
    database: String,
    collection: String,
}

#[derive(Clone, Debug)]
struct ErRelationshipSelectOption {
    id: String,
    label: String,
}

impl SelectItem for ErRelationshipSelectOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

struct ErRelationshipPairControls {
    left: Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>,
    right: Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>,
    left_options_key: String,
    right_options_key: String,
}

/// 结构刷新重绑待处理项（§5.2/D9，供关系面板列出，人工重绑；不自动改关系）。
/// `kind` 为可读描述，如「实体未找到」「同名重建需确认」「缺列: xxx」。
#[derive(Clone, Debug, Default)]
struct ErRebindPendingItem {
    rel_id: String,
    /// 端点描述（左表 / 右表）。
    endpoint: String,
    kind: String,
}

struct ErRelationshipFilterControls {
    side: Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>,
    column: Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>,
    op: Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>,
    literal: Entity<InputState>,
    column_options_key: String,
}

/// ER 画布视图状态（§perf 8 画布实体隔离，阶段一）：平移/拖动/字段滚动等连续交互的
/// 热路径状态经 `Rc<RefCell>` 共享（`NavicatMain.er_canvas`），app 各 handler 无需 cx
/// 直改。阶段二（把画布渲染/通知下沉到独立实体）设计已定但需 GPUI 真机点验后再落地，
/// 届时才引入实体层；当前仅 Rc 共享（避免阶段一在全仓警醒下铺开实体死代码）。
/// 画布浮层所需的每帧只读数据（端口圆点/临时连线/画布原点）。
/// 由 canvas.rs 从状态读出后传入 er_canvas_view，避免渲染函数直接持状态借用。
struct ErCanvasOverlay {
    /// 悬停字段行（用于浮现端口圆点）。
    row_hover: Option<(String, String)>,
    /// 悬停端口圆点（行→圆点过渡时不闪烁）。
    port_hover: Option<(String, String)>,
    /// 连线拖拽状态（临时线 + 源/目标端口高亮）。
    link_drag: Option<ErLinkDrag>,
}

/// 字段手动连线拖拽状态（§手动连线）。
/// 从字段端口圆点按下开始，移动到目标字段端口时吸附；释放到有效目标则打开关系表单并预填。
#[derive(Clone, Debug)]
struct ErLinkDrag {
    tab: TabId,
    from_table: String,
    from_column: String,
    from_side: ErPortSide,
    /// 光标屏幕坐标（画布内）。
    cursor: (f32, f32),
    /// 当前吸附到的目标端口（表, 字段, 侧别）；未命中有效字段端口时为 None（跟手）。
    target: Option<(String, String, ErPortSide)>,
}

struct ErCanvasShared {
    // ER 画布视口（平移/缩放）：按 tab 隔离。
    er_viewports: BTreeMap<TabId, ErViewport>,
    // ER 画布拖动平移进行中：记录 (tab, 按下时光标, 按下时 pan)，用于拖动更新 pan。
    er_viewport_drag: Option<(TabId, f32, f32, f32, f32)>,
    // ER 画布可见区域尺寸（px）：由画布区 Element 回报，用于虚拟化可见性计算。
    er_canvas_sizes: BTreeMap<TabId, (f32, f32)>,
    // ER 画布区在窗口中的原点（px）：绘制/命中需统一画布原点偏移（§修复：命中遗漏原点）。
    er_canvas_origins: BTreeMap<TabId, (f32, f32)>,
    // ER 节点内字段纵向滚动偏移（px，按 tab+表名）。字段多时在节点内滚动。
    er_node_scroll_px: BTreeMap<(TabId, String), f32>,
    // ER 字段滚动条拖拽进行中：记录 (tab, 表名)。
    er_scroll_drag: Option<(TabId, String)>,
    // ER 画布当前选中的表（按 tab）：表头点击选择，相关连线高亮；空白/Esc 取消。
    er_selected_table: BTreeMap<TabId, Option<String>>,
    // ER 节点自由世界坐标（按 tab + 表名，稳定表身份，不依赖布局下标）。
    // 场景只存拓扑；坐标在此可变（拖动/布局应用更新）。
    er_scene_positions: BTreeMap<TabId, BTreeMap<String, (f32, f32)>>,
    // 手动固定（拖动过）的表：关系布局/重排保留其坐标（§6.3）。
    er_pinned: BTreeMap<TabId, BTreeSet<String>>,
    // 节点拖动进行中：(tab, 表名, 按下光标 x/y, 原坐标 x/y, 是否已超阈值)。
    // 未超阈值释放 = 选择；超阈值 = 拖动（§7）。
    er_node_drag: Option<(TabId, String, f32, f32, f32, f32, bool)>,
    // 字段手动连线拖拽进行中（§手动连线）。
    er_link_drag: Option<ErLinkDrag>,
    // 悬停的字段行（表, 字段）：用于浮现字段端口圆点。行与圆点分开记录，
    // 光标从字段行移到圆点时不因行 hover 丢失而闪烁。
    er_row_hover: BTreeMap<TabId, Option<(String, String)>>,
    er_port_hover: BTreeMap<TabId, Option<(String, String)>>,
    // 本帧可见节点视图缓存（供字段端口命中测试：拖动/释放时按真实几何反查字段行）。
    er_frame_nodes: BTreeMap<TabId, Vec<ErNodeView>>,
    // 本帧可见折线缓存（供空白点击的关系线命中，有界）。
    er_frame_edges: BTreeMap<TabId, Vec<ErEdgeView>>,
    // ER 隐藏字段定位后的高亮（按 tab → (表名, 字段名)）：从汇总端口定位到精确字段后高亮该行。
    er_field_highlights: BTreeMap<TabId, (String, String)>,
    // ER 小地图（§六.25）：小地图在窗口中的 (x,y,w,h)，供点击导航归一化换算（paint 回报）。
    er_minimap_rect: BTreeMap<TabId, (f32, f32, f32, f32)>,
}

include!("navicat_main/state.rs");

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
    loaded_database_children: BTreeSet<String>,
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
    /// 自动探测：原生工具可用则原生，否则逻辑。
    Auto,
}

impl BackupMode {
    fn label(self) -> SharedString {
        match self {
            Self::Auto => tr("app.backup.mode.auto"),
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

    fn label(self) -> SharedString {
        match self {
            Self::General => tr("app.database_backup.tab.general"),
            Self::Objects => tr("app.database_backup.tab.objects"),
            Self::Advanced => tr("app.database_backup.tab.advanced"),
            Self::Log => tr("app.database_backup.tab.log"),
        }
    }
}

/// 恢复配置弹框页签（与备份同款四页结构，设计文档 §14.1）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestoreTab {
    General,
    Objects,
    Advanced,
    Log,
}

impl RestoreTab {
    const ALL: [Self; 4] = [Self::General, Self::Objects, Self::Advanced, Self::Log];

    fn label(self) -> SharedString {
        match self {
            Self::General => tr("app.database_restore.tab.general"),
            Self::Objects => tr("app.database_restore.tab.objects"),
            Self::Advanced => tr("app.database_restore.tab.advanced"),
            Self::Log => tr("app.database_restore.tab.log"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BackupForm {
    database_kind: DatabaseKind,
    connection_id: ConnectionId,
    database: Option<String>,
    mode: BackupMode,
    target_dir: String,
    tab: BackupTab,
    /// 常规：文件名/模板；留空用默认（库名_时间戳.sql）。
    file_name: String,
    /// 对象选择：已勾选的表名集合（`scope_all` 为 false 时生效）。
    selected_tables: BTreeSet<String>,
    /// 对象选择：是否整库备份（全部对象）。为 true 时执行阶段重新枚举全部用户表，
    /// `selected_tables` 被忽略；为 false 时按 `selected_tables` 固定清单导出。
    scope_all: bool,
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
    /// 高级（仅 PostgreSQL 原生 pg_dump）：导出属主（OWNER）；默认 false 传 `--no-owner`。
    pg_include_owner: bool,
    /// 高级（仅 PostgreSQL 原生 pg_dump）：导出 ACL 权限；默认 false 传 `--no-acl`。
    pg_include_acl: bool,
}

/// PostgreSQL 客户端工具下载任务的 UI 状态（设置面板展示 + 取消）。
#[derive(Clone, Debug)]
struct PgClientDownloadState {
    /// 展示给用户的进度文案，如「下载中 42%（138.2 MB / 329.9 MB）」。
    message: String,
    /// 取消标志，后台下载线程按分片检查。
    cancel: Arc<AtomicBool>,
}

/// 备份记录（存 sqlite，真实数据在磁盘）：复用 fluxdb-storage 的 `BackupRecord`。
/// 字段含 connection_id/database（按库筛选）、output_path（真实文件路径）、
/// created_unix/size（备份时间与大小）、tables/include_views/note（表清单与备注）。
use fluxdb_storage::BackupRecord as BackupFileMeta;

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

/// 备份 tab「恢复记录」弹框状态：该备份关联的全部恢复记录（新→旧）。
#[derive(Clone, Debug)]
struct RestoreRecordsModal {
    /// 备份文件名（标题展示）。
    file_name: String,
    records: Vec<fluxdb_storage::RestoreRecord>,
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

/// PostgreSQL schema 节点右键菜单：与数据库菜单隔离，避免把 schema 当作数据库操作。
#[derive(Clone, Debug)]
struct SchemaContextMenu {
    connection_id: ConnectionId,
    database: String,
    schema: String,
    position: Point<Pixels>,
}

#[derive(Clone, Debug)]
struct TableContextMenu {
    object_path: ObjectPath,
    position: Point<Pixels>,
    submenu: Option<TableContextSubmenu>,
}

/// MongoDB 集合/视图右键菜单：项与表菜单不同（无设计/复制/备份等 SQL 语义）。
#[derive(Clone, Debug)]
struct CollectionContextMenu {
    object_path: ObjectPath,
    position: Point<Pixels>,
    /// 视图只读：隐藏重命名/删除/导入等写入入口。
    read_only: bool,
}

#[derive(Clone, Debug)]
struct TableGroupContextMenu {
    connection_id: ConnectionId,
    database_path: ObjectPath,
    database: String,
    /// PG 下为所属 schema；非 schema 分桶数据库为 `None`，刷新时据此选择加载作用域。
    schema: Option<String>,
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

/// MongoDB「重命名集合」弹框。
#[derive(Clone, Debug)]
struct PendingRenameCollection {
    object_path: ObjectPath,
    new_name: String,
    error: Option<String>,
}

/// MongoDB「删除集合」确认弹框。
#[derive(Clone, Debug)]
struct PendingDropCollection {
    object_path: ObjectPath,
    /// 删除会连带丢弃文档，弹框内固定展示这句风险说明。
    document_count: Option<u64>,
}

/// MongoDB 导出弹框：常规页（格式 + 范围 + 条件预览）与消息日志页。
/// 与 Compass 一致，条件不需要单独页签——选「条件」时在常规页直接展示 shell 查询。
#[derive(Clone, Debug)]
struct MongoExportForm {
    object: ObjectPath,
    format: MongoExportFormat,
    scope: MongoExportScope,
    active_tab: MongoExportTab,
    /// 条件导出用的 Extended JSON 文本（全量导出时被忽略）。
    /// 直接存在表单里而不是另开输入框实体：条件只读展示，不需要编辑状态。
    filter: String,
    projection: String,
    sort: String,
    /// 导出进行中：页签可切，「导出」按钮禁用。
    running: bool,
    /// 取消标记；后台分页循环每取一页检查一次，已写入的临时文件随后丢弃。
    cancel: Arc<AtomicBool>,
    exported_documents: u64,
    /// 消息日志页内容（沿用导入弹窗的条目结构）。
    logs: Vec<ImportDialogLogEntry>,
    error: Option<String>,
    finished: bool,
    /// 任务中心记录 id；弹窗关闭后仍可查看本次导出结果。
    task_center_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MongoExportTab {
    General,
    Log,
}

/// 导出格式。CSV 会先扫全量求列并集再写行（见 `mongo_export_csv.rs`），
/// 因此混字段集合不会静默丢列，但导出进度会分「下载 / 写入」两段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MongoExportFormat {
    Json,
    Csv,
    JsonLines,
}

impl MongoExportFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Csv => "CSV",
            Self::JsonLines => "JSONL",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::JsonLines => "jsonl",
        }
    }

    fn all() -> &'static [Self] {
        &[Self::Json, Self::Csv, Self::JsonLines]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MongoExportScope {
    /// 整个集合（空过滤）。
    All,
    /// 条件导出：使用弹框里的 filter/projection/sort。
    Conditions,
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
struct PendingRoutineDdl {
    object_path: ObjectPath,
    ddl: Option<String>,
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
    /// PG 清空表：是否 RESTART IDENTITY（默认 CONTINUE IDENTITY）。
    restart_identity: bool,
    acknowledged: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DangerTableAction {
    Drop,
    Truncate,
}

impl DangerTableAction {
    fn title(self) -> SharedString {
        match self {
            Self::Drop => tr("app.table.danger.drop_title"),
            Self::Truncate => tr("app.table.danger.truncate_title"),
        }
    }

    fn prompt(self) -> SharedString {
        match self {
            Self::Drop => tr("app.table.danger.drop_prompt"),
            Self::Truncate => tr("app.table.danger.truncate_prompt"),
        }
    }

    fn confirm_label(self) -> SharedString {
        match self {
            Self::Drop => tr("app.redis.delete"),
            Self::Truncate => tr("app.table.danger.truncate"),
        }
    }

    fn running_message(self) -> SharedString {
        match self {
            Self::Drop => tr("app.table.danger.dropping"),
            Self::Truncate => tr("app.table.danger.truncating"),
        }
    }

    fn sql_preview_key(self) -> &'static str {
        match self {
            Self::Drop => "drop",
            Self::Truncate => "truncate",
        }
    }
}

#[derive(Clone)]
struct ForeignKeyCheckOption {
    mode: ForeignKeyCheckMode,
}

impl SelectItem for ForeignKeyCheckOption {
    type Value = ForeignKeyCheckMode;

    fn title(&self) -> SharedString {
        match self.mode {
            ForeignKeyCheckMode::Default => tr("app.table.danger.fk_default"),
            ForeignKeyCheckMode::Enable => tr("app.table.danger.fk_enable"),
            ForeignKeyCheckMode::Disable => tr("app.table.danger.fk_disable"),
        }
    }

    fn value(&self) -> &ForeignKeyCheckMode {
        &self.mode
    }
}

fn danger_table_foreign_key_check_options() -> Vec<ForeignKeyCheckOption> {
    [
        ForeignKeyCheckMode::Default,
        ForeignKeyCheckMode::Enable,
        ForeignKeyCheckMode::Disable,
    ]
    .into_iter()
    .map(|mode| ForeignKeyCheckOption { mode })
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
    /// 单元格显示值；截断/超大字段时带「仅预览」标记，不能当权威数据复制。
    value: String,
    /// 该值只是预览（截断的 CLOB 或超大字段）：复制必须拒绝，改走完整读取或流式保存。
    value_truncated: bool,
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
    scope: FirstLevelScope,
    display_name: Option<String>,
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
    /// PostgreSQL OWNER（可选）。
    owner: String,
    /// PostgreSQL TEMPLATE（可选）。
    template: String,
    /// MongoDB 首个集合名：Mongo 的库由首个集合隐式创建，缺了它库不会真正存在。
    first_collection: String,
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
    detail: Option<String>,
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
    scope: FirstLevelScope,
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

/// 上下分栏拖拽的目标工作台：决定松手时把尺寸写回哪个持久化设置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkbenchSplitTarget {
    Redis,
    Mongo,
}

/// Workbench 分栏拖动起点。分栏尺寸是全局设置（非按 tab），
/// 故无需 tab_id（对齐 RedisInsight 的全局 panelSizes 语义）。
///
/// **必须把 `target` / `placement` 一起记下来**，不能在拖动时读实时的全局值：
/// `Div::on_mouse_up` 只在指针仍悬停在手柄上时才触发（gpui 的 hover 判定），
/// 正常拖拽手势在结果区松手，于是起点常常不被清理。若此时布局被工具栏切过，
/// 残留的「高度」会被当成「宽度」用；跨工作台时也会写错设置字段。
/// 连 `x`/`y` 一起记录，可保证起点记录的四元组（目标、轴、基准、尺寸）自洽。
#[derive(Clone, Copy, Debug)]
struct WorkbenchPanelResizeStart {
    target: WorkbenchSplitTarget,
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
    /// 当前显示页数据，与结果页缓存共享同一份 `DataPage`；单元格文本在渲染时按需格式化，
    /// 不再为整页预生成第二份文本拷贝（宽结果集下这是与数据同量级的内存开销）。
    page: Arc<DataPage>,
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
    /// 当前结果页解析出的查询计划；解析失败时页签不出现，保留原始结果表格。
    Plan,
}

impl QueryOutputTab {
    fn result_index(self) -> Option<usize> {
        match self {
            QueryOutputTab::Result(index) => Some(index),
            QueryOutputTab::Summary | QueryOutputTab::Plan => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum QueryPlanViewMode {
    #[default]
    Graph,
    Table,
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

/// 详情面板里大字段的分段只读窗口（UI 侧状态）。
///
/// 完整内容不进 UI 进程：这里只存按块读到的有上限窗口。键与打开中的详情一致，
/// 换单元格、关闭面板、关闭标签都要丢掉，避免把旧窗口错挂到新单元格上。
#[derive(Clone, Debug, PartialEq)]
struct LargeValueWindows {
    /// 窗口所属单元格（可编辑结果用 `row/column`；只读结果同样用它）。
    row: usize,
    column: usize,
    view: fluxdb_app::LargeValueView,
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
