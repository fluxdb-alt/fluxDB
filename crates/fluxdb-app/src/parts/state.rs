/// Provider 公布的模型能力，缺失字段不伪造默认窗口。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiDiscoveredModel {
    pub id: String,
    pub context_window_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

/// 单次上传替换的文件上限（字节）。
///
/// 与暂存归属配额（`CELL_CONTENT_OWNER_QUOTA_BYTES`，256 MiB）取齐：暂存内容
/// 必须落在受控目录里等提交，超过配额就在暂存阶段失败，报错点会离用户操作很远。
/// 上限设在配额上，用户看到的是「文件太大」，而不是「结果内容配额不足」。
///
/// 这个上限不由流式能力决定——超过阈值的文件本来就按块写入，内存占用与大小无关；
/// 真正的天花板是各库的类型上限与追加代价（MySQL/SQLite/PG 追加会重写整值，
/// 见 `BLOB_STREAM_CHUNK_BYTES` 的说明）。
pub const BINARY_FILE_UPLOAD_LIMIT: u64 = 256 * 1024 * 1024;
const COMPLETION_METADATA_LIMIT: u64 = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiModelKind {
    #[default]
    Generation,
    Decision,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiCredentialStatus {
    #[default]
    NotRequired,
    Missing,
    Configured,
    Unavailable,
}

/// 设置页「AI」分区缓存的模型列表（公开视图，不含任何凭据）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiModelItem {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub model: String,
    /// 自定义 API base（未配置时为 None，运行期按 Provider 默认值补全）。
    pub endpoint: Option<String>,
    /// 完整请求地址，不追加协议路由。
    pub endpoint_is_full_url: bool,
    pub kind: AiModelKind,
    pub enabled: bool,
    /// 数据出站许可：false 表示该模型不再接收会话数据（设计 §29/§31.1）。
    pub egress: bool,
    /// 能力标签（当前为长上下文）：由真实上下文窗口推导，不按模型名或价格硬编码。
    pub capability_labels: Vec<String>,
    pub context_window_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    /// 计价：micro-USD / 百万 Token；只用于编辑弹框回填，不参与列表展示、标签或计量。
    pub input_price_micros: Option<i64>,
    pub output_price_micros: Option<i64>,
    pub cached_input_price_micros: Option<i64>,
    pub credential_status: AiCredentialStatus,
    /// 最近一次连接探测结果：`unknown` / `ok` / `failed`。
    pub connection_status: String,
    /// 最近一次连接探测时间戳（毫秒）；从未探测为 `None`。
    pub connection_checked_at_ms: Option<i64>,
    pub revision: i64,
}

/// 模型新建/编辑的提交内容（T013）。
///
/// 明文 Key 只在 `api_key` 里短暂存在，经专用命令进入存储；不进 AppState、日志或事件。
///
/// 刻意不派生 PartialEq/Debug 之外的比较能力：命令枚举的比较不应牵扯明文凭据。
#[derive(Clone)]
pub struct AiModelSaveInput {
    /// `None` 表示新建。
    pub id: Option<String>,
    pub display_name: String,
    pub provider: String,
    pub model: String,
    pub endpoint: Option<String>,
    /// 完整请求地址，不追加协议路由。
    pub endpoint_is_full_url: bool,
    pub enabled: bool,
    /// 数据出站许可：新建默认允许；关闭后该模型不作为路由候选。
    pub egress: bool,
    pub context_window_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    /// 计价单位：micro-USD / 百万 Token；只做保存与再次编辑回填。
    pub input_price_micros: Option<i64>,
    pub output_price_micros: Option<i64>,
    pub cached_input_price_micros: Option<i64>,
    /// `Some` 表示替换 Key；`None` 且 `clear_key=false` 表示 Keep。
    pub api_key: Option<String>,
    pub clear_key: bool,
    /// 新建为 `None`；编辑必须带上打开弹框时的 revision 做 CAS。
    pub expected_revision: Option<i64>,
}

impl Default for AiModelSaveInput {
    /// 显式实现而不是派生：`egress` 是"是否允许数据出站"，`bool` 的派省默认值是 `false`，
    /// 会让"没填这个字段"变成"禁止出站"，与设计默认（配置模型即授权）相反。
    fn default() -> Self {
        Self {
            id: None,
            display_name: String::new(),
            provider: String::new(),
            model: String::new(),
            endpoint: None,
            endpoint_is_full_url: false,
            enabled: false,
            egress: true,
            context_window_tokens: None,
            max_output_tokens: None,
            input_price_micros: None,
            output_price_micros: None,
            cached_input_price_micros: None,
            api_key: None,
            clear_key: false,
            expected_revision: None,
        }
    }
}

impl std::fmt::Debug for AiModelSaveInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiModelSaveInput")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("clear_key", &self.clear_key)
            .field("expected_revision", &self.expected_revision)
            .finish_non_exhaustive()
    }
}

impl PartialEq for AiModelSaveInput {
    /// 比较刻意忽略 `api_key`：明文不参与相等性判断，也不应出现在断言输出里。
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.display_name == other.display_name
            && self.provider == other.provider
            && self.model == other.model
            && self.endpoint == other.endpoint
            && self.endpoint_is_full_url == other.endpoint_is_full_url
            && self.enabled == other.enabled
            && self.context_window_tokens == other.context_window_tokens
            && self.max_output_tokens == other.max_output_tokens
            && self.input_price_micros == other.input_price_micros
            && self.output_price_micros == other.output_price_micros
            && self.cached_input_price_micros == other.cached_input_price_micros
            && self.api_key.is_some() == other.api_key.is_some()
            && self.clear_key == other.clear_key
            && self.expected_revision == other.expected_revision
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiAgentItem {
    pub compression_trigger_percent: u8,
    /// 单个 Turn 内最多几次主模型迭代。
    pub max_iterations: i64,
    pub id: String,
    pub display_name: String,
    pub description: String,
    /// 直接编辑的 Agent 提示词（随 Agent 行 CAS，不做模板/多版本）。
    pub system_prompt: String,
    /// 默认生成模型档案 id（`ai_agents.default_model_profile_id`）。
    pub model_profile_id: Option<String>,
    /// 允许的工具 ID 白名单；空表示不限制（内核默认全集）。
    pub allowed_tools: Vec<String>,
    pub enabled: bool,
    pub revision: i64,
}

/// Agent 新建/编辑的提交内容（T013/T014）。
///
/// 提示词直接随 Agent 保存；`id` 为 `None` 表示新建，编辑用 `expected_revision` 做 CAS。
#[derive(Clone, Debug, PartialEq)]
pub struct AiAgentSaveInput {
    pub compression_trigger_percent: u8,
    /// 单个 Turn 内最多几次主模型迭代（1–1000）。
    pub max_iterations: i64,
    /// `None` 表示新建。
    pub id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub system_prompt: String,
    pub model_profile_id: Option<String>,
    /// 允许的工具 ID 白名单；空 = 不限制。
    pub allowed_tools: Vec<String>,
    pub enabled: bool,
    /// 新建忽略；编辑必须带上打开时的 revision 做 CAS。
    pub expected_revision: i64,
}

impl Default for AiAgentSaveInput {
    fn default() -> Self {
        Self {
            id: None,
            display_name: String::new(),
            description: String::new(),
            system_prompt: String::new(),
            model_profile_id: None,
            allowed_tools: Vec::new(),
            enabled: true,
            expected_revision: 0,
            compression_trigger_percent: 80,
            max_iterations: 100,
        }
    }
}

/// 保存对外 MCP Server Profile 的输入。
///
/// `id` 为 `None` 时新建：存储层强制 `enabled=false`，保存配置本身不会意外对外开放服务。
#[derive(Clone, Debug, PartialEq)]
pub struct AiMcpServerProfileSaveInput {
    pub id: Option<String>,
    pub display_name: String,
    pub enabled: bool,
    /// 新建忽略；编辑必须带上打开时的 revision 做对象级 CAS。
    pub expected_revision: Option<i64>,
}

impl Default for AiMcpServerProfileSaveInput {
    fn default() -> Self {
        Self {
            id: None,
            display_name: String::new(),
            enabled: false,
            expected_revision: None,
        }
    }
}

/// 保存 MCP 数据源写入策略的输入。
///
/// `write_policy` 为 `default`（跟随全局）/ `read_only` / `read_write`。
#[derive(Clone, Debug, PartialEq)]
pub struct AiMcpDatasourceSaveInput {
    pub profile_id: String,
    pub connection_id: i64,
    pub write_policy: String,
    /// 新建忽略；编辑必须带上打开时的 revision 做对象级 CAS。
    pub expected_revision: Option<i64>,
}

impl Default for AiMcpDatasourceSaveInput {
    fn default() -> Self {
        Self {
            profile_id: String::new(),
            connection_id: 0,
            write_policy: "default".to_string(),
            expected_revision: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiPluginItem {
    pub id: String,
    pub display_name: String,
    pub package_key: String,
    /// 内置能力登记表里的「负责内容」；未知实例为空串，不猜语义。
    pub summary: String,
    pub enabled: bool,
    /// 内核强制（当前界面与 @ 上下文、调用前安全审计），UI 需禁用开关并展示"常驻能力"。
    pub mandatory: bool,
    /// 没有实现或已下线的目录占位行：UI 置灰整行、禁用开关并显示"待实现"。
    pub retired: bool,
    /// 插件声明的可配置字段（协议见 [`PluginConfigDeclaration`]）。
    ///
    /// 为空表示该插件不需要配置，UI 因此不显示「管理」入口；这也是内置与外置插件
    /// 共用的一条判断，不按插件 id 特判。
    pub config_fields: Vec<AiPluginConfigFieldItem>,
    /// 声明解析失败的原因（外置插件 manifest 写错时可见，不静默当成"无配置"）。
    pub config_error: Option<String>,
    pub revision: i64,
}

/// 插件配置字段的种类；决定弹框渲染哪种控件与回填什么值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiPluginConfigFieldKind {
    /// 资源选择（模型角色槽位）。
    #[default]
    Resource,
    /// 开关。
    Switch,
    /// 单行文本。
    Text,
    /// 多行文本，一行一个条目。
    TextList,
    /// 枚举选择。
    Select,
}

/// 枚举字段的一个候选项。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiPluginConfigOptionItem {
    pub value: String,
    pub label: String,
}

/// 插件配置字段的展示与候选快照；数据全部来自宿主解析，不含凭据材料。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiPluginConfigFieldItem {
    /// 稳定机器键：保存时原样回传给应用层定位需求。
    pub key: String,
    /// 字段种类：决定控件形态与提交的值类型。
    pub kind: AiPluginConfigFieldKind,
    /// 可选分组名：同一分组的字段在弹框里合并成一块，缺省表示不分组。
    pub group: Option<String>,
    pub label: String,
    /// 插件自己写的补充说明（可空）：只讲插件特有的后果，不描述宿主的过滤规则。
    pub help: Option<String>,
    /// 宿主按声明生成的候选口径说明（例如「只列出…的决策模型」）；标量字段为空。
    pub hint: String,
    /// 标量字段的输入提示（可空）。
    pub placeholder: Option<String>,
    pub required: bool,
    /// 当前值：资源字段是资源 ID，标量字段是文本值（`text_list` 为多行文本）。
    /// `None` 表示未配置。
    pub selection: Option<String>,
    /// 当前选择是否仍满足插件要求；选择失效时保留原值并提示重选。
    /// 标量字段只要通过自身类型校验即为 `true`。
    pub selection_usable: bool,
    /// 资源字段的候选项；标量字段为空。
    pub choices: Vec<AiPluginConfigChoiceItem>,
    /// 枚举字段的候选项。
    pub options: Vec<AiPluginConfigOptionItem>,
    /// 没有候选项且原因与协议有关时的可见说明。
    pub rejection: Option<String>,
}

/// 一个资源候选项（只含 ID 与公开标签）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiPluginConfigChoiceItem {
    pub id: String,
    pub label: String,
    pub detail: String,
}

/// 一次插件配置提交里的单个字段值。
///
/// 资源字段用 `resource_id`（模型档案 ID），标量字段用 `value`（文本原值，
/// `text_list` 传多行文本，由应用层按声明规范化）；两者都为空表示清空该字段。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiPluginFieldSelection {
    pub key: String,
    pub resource_id: Option<String>,
    pub value: Option<String>,
}

/// 审计列表的判定（与存储层 `AuditDecision` 对应，UI 只读）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiAuditDecision {
    Allow,
    RequireApproval,
    Deny,
}

impl AiAuditDecision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Allow => "允许",
            Self::RequireApproval => "需审批",
            Self::Deny => "拒绝",
        }
    }

    /// 机器键，供筛选与测试断言使用。
    pub fn key(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::RequireApproval => "approval",
            Self::Deny => "deny",
        }
    }
}

/// 审计记录页的一行：审计事实的只读投影。
///
/// 不含凭据、密文或未脱敏参数；`statement` 来自入库前已脱敏的参数。
#[derive(Clone, Debug, PartialEq)]
pub struct AiAuditRow {
    pub id: String,
    pub tool_id: String,
    /// 来源：内部 Agent 或对外 MCP。
    pub origin: String,
    /// 目标的可读投影（连接/库/schema）。
    pub target: String,
    pub decision: AiAuditDecision,
    /// 命中的规则（机器键）；未命中为空。
    pub rule_ids: Vec<String>,
    /// 原因码（脱敏，只含规则标签与对象名）。
    pub reason_codes: Vec<String>,
    /// 已脱敏的语句/命令，供详情展示。
    pub statement: String,
    pub ruleset_version: String,
    pub plan_hash: String,
    pub created_at_ms: i64,
    /// 执行前 / 重新校验。
    pub stage: String,
}

/// 审计记录页的筛选统计（当前快照内）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiAuditCounts {
    pub all: usize,
    pub allow: usize,
    pub approval: usize,
    pub deny: usize,
}

/// 设置页「工具与 MCP → 工具」的一行：标准工具的全局启停状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiToolSettingItem {
    pub tool_id: String,
    pub enabled: bool,
    pub revision: i64,
}

/// 设置页「会话历史」列表行：公开的会话身份与派生统计，不含任何凭据或正文。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiSessionItem {
    pub id: String,
    pub title: String,
    /// 该会话使用的 Agent id（UI 按 `state.ai_agents` 显示名称）。
    pub agent_id: String,
    /// 会话状态（`active` / `archived`）。
    pub status: String,
    /// 最后一个 Turn 的状态（`ai_session_turn_status` 口径）；无 Turn 时为 `None`。
    pub last_turn_status: Option<String>,
    /// 最后一个 Turn 的执行目标 JSON（脱敏后的公开目标）；无 Turn 时为 `None`。
    pub last_target_json: Option<String>,
    /// 已持久化的消息条数。
    pub message_count: i64,
    /// Turn 总数。
    pub turn_count: i64,
    /// 最近更新时间（毫秒时间戳）。
    pub updated_at_ms: i64,
    /// 对象级 CAS 期望 revision。
    pub revision: i64,
}

/// 设置页「MCP Server Profile」列表行：对外服务配置，不含任何秘密。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiMcpServerProfileItem {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub revision: i64,
}

/// 设置页「MCP 数据源」列表行：对外可见的连接 + 写入策略。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiMcpDatasourceItem {
    pub profile_id: String,
    pub connection_id: i64,
    pub connection_name: String,
    /// `sql` / `redis`，由连接类型决定（决定展示的资源种类）。
    pub resource_kind: String,
    /// `default` / `read_only` / `read_write`。
    pub write_policy: String,
    pub revision: i64,
}

/// 内置 Jev 的装配/可用状态（设计 §18 状态表；这是状态显示，不是一排错误 toast）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JevStatusKind {
    /// 没有合格决策模型：零请求、零等待。
    #[default]
    NotConfigured,
    /// 内置能力被用户关闭：零请求、零等待。
    Disabled,
    /// 已显式选择决策模型，运行时可以发建议。
    Available,
    /// 有合格候选但用户还没选：等用户选，绝不擅自挑一个。
    NeedsChoice,
    /// 已配置但当前不可用（协议不兼容/默认模型失效/熔断中），附脱敏原因。
    Unavailable,
}

impl JevStatusKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JevStatusKind::NotConfigured => "not_configured",
            JevStatusKind::Disabled => "disabled",
            JevStatusKind::Available => "available",
            JevStatusKind::NeedsChoice => "needs_choice",
            JevStatusKind::Unavailable => "unavailable",
        }
    }
}

/// Jev 状态快照：设置页与运行期共用同一判定，避免 UI 与实际调用不一致。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JevStatus {
    pub kind: JevStatusKind,
    /// 当前生效的决策模型档案 id（`Available`/`Unavailable` 可能有值）。
    pub model_profile_id: Option<String>,
    /// 脱敏原因：不含 URL、响应正文或任何凭据材料。
    pub detail: Option<String>,
    /// 熔断截止（毫秒时间戳）；未熔断为 `None`。
    pub circuit_until_ms: Option<i64>,
}

impl JevStatus {
    /// 状态标签文案（插件页与大模型页共用）。
    pub fn label(&self) -> &'static str {
        match self.kind {
            JevStatusKind::NotConfigured => "未配置",
            JevStatusKind::Disabled => "已禁用",
            JevStatusKind::Available => "可用",
            JevStatusKind::NeedsChoice => "待选择模型",
            JevStatusKind::Unavailable => "暂不可用",
        }
    }

    /// 是否可以发决策请求；熔断期由调用方另行检查截止时间。
    pub fn is_usable(&self) -> bool {
        self.kind == JevStatusKind::Available && self.model_profile_id.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppState {
    pub ai_conversation: AiConversationView,
    pub connections: Vec<ConnectionState>,
    pub tabs: Vec<TabState>,
    pub active_tab: Option<TabId>,
    pub pending_dirty_tab_close: Option<TabId>,
    pub settings: Settings,
    pub sidebar_layout: SidebarLayout,
    /// 设置页「AI」分区缓存（模型/Agent/插件/MCP），加载失败时保持空列表。
    pub ai_models: Vec<AiModelItem>,
    pub ai_agents: Vec<AiAgentItem>,
    pub ai_plugin_instances: Vec<AiPluginItem>,
    /// 标准工具的全局启停；列表项缺失表示该工具从未被显式关闭（默认启用）。
    pub ai_tool_settings: Vec<AiToolSettingItem>,
    pub ai_execution_policy: fluxdb_core::ExecutionPolicySettings,
    /// 设置页「本地 MCP」页签的对外 Server Profile 列表。
    pub ai_mcp_server_profiles: Vec<AiMcpServerProfileItem>,
    /// 各 Profile 的数据源（对外可见连接 + 写入策略）；按 profile_id 归属，UI 分组展示。
    pub ai_mcp_datasources: Vec<AiMcpDatasourceItem>,
    /// 设置页「安全审计」列表：最近的审计事实（只读），判定统计由它推导。
    pub ai_audit_rows: Vec<AiAuditRow>,
    /// 审计列表是否加载失败；失败时页面给出可见原因，不伪装成"暂无记录"。
    pub ai_audit_error: Option<String>,
    /// 设置页「会话历史」列表（活跃 + 已归档，最近更新优先）。
    pub ai_sessions: Vec<AiSessionItem>,
    /// 会话历史分页：总数（分页后不能靠列表长度推断）。
    pub ai_sessions_total: i64,
    /// 会话历史分页：下一页游标；`None` 表示已读完。
    pub ai_sessions_cursor: Option<fluxdb_storage::ai_platform::sessions::SessionListCursor>,
    pub ai_sessions_has_more: bool,
    /// 正在加载下一页（按钮 loading/disabled，拦重复请求）。
    pub ai_sessions_loading_more: bool,
    /// 默认主模型档案 id（`ai_runtime_settings.default_model_profile_id`）。
    pub ai_default_model_id: Option<String>,
    /// 内置 Jev 的装配/可用状态（设置页插件页与大模型页共用，实时反映熔断）。
    pub ai_jev_status: JevStatus,
    /// 运行设置行 revision（默认模型选择的 CAS 期望值）。
    pub ai_runtime_revision: i64,
    pub tasks: Vec<TaskState>,
    pub query_history: Vec<QueryHistoryEntry>,
    /// Redis Workbench 命令历史（全量扁平，按连接 + 逻辑库过滤展示），持久化到
    /// `workbench-history.toml`。与 `query_history` 同层级：App 层写入、桌面层持久化。
    pub redis_workbench_history: Vec<RedisWorkbenchHistoryEntry>,
    /// 为 `redis_workbench_history` 分配历史记录 ID 的单调递增计数器。
    pub next_redis_workbench_history_id: u64,
    pub last_error: Option<UserFacingError>,
}

impl AppState {
    pub fn active_tab(&self) -> Option<&TabState> {
        self.active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            ai_conversation: AiConversationView::default(),
            connections: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            pending_dirty_tab_close: None,
            settings: Settings::default(),
            sidebar_layout: SidebarLayout::default(),
            ai_models: Vec::new(),
            ai_agents: Vec::new(),
            ai_plugin_instances: Vec::new(),
            ai_tool_settings: Vec::new(),
            ai_execution_policy: Default::default(),
            ai_mcp_server_profiles: Vec::new(),
            ai_mcp_datasources: Vec::new(),
            ai_audit_rows: Vec::new(),
            ai_audit_error: None,
            ai_sessions: Vec::new(),
            ai_sessions_total: 0,
            ai_sessions_cursor: None,
            ai_sessions_has_more: false,
            ai_sessions_loading_more: false,
            ai_default_model_id: None,
            ai_jev_status: JevStatus::default(),
            ai_runtime_revision: 0,
            tasks: Vec::new(),
            query_history: Vec::new(),
            redis_workbench_history: Vec::new(),
            next_redis_workbench_history_id: 0,
            last_error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionState {
    pub config: ConnectionConfig,
    pub connected: bool,
    pub expanded: bool,
    pub objects: Vec<ObjectSummary>,
    /// Redis 连接级运行概览（版本 / 内存 / CPU），只在连接为 Redis 时有意义。
    pub redis_overview: RedisConnectionOverview,
}

/// Redis 连接级运行概览，供底栏连接状态摘要展示。
/// 参考 RedisInsight 数据库概览语义，与 key 详情无关。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisConnectionOverview {
    /// Redis 服务端版本，如 `7.2.4`。
    pub version: String,
    /// Redis 实例占用内存字节数。
    pub used_memory_bytes: u64,
    /// CPU 使用率百分比（两次采样增量计算）。尚无第二次采样（或服务端重启）时为 None。
    pub cpu_usage_percent: Option<f64>,
    /// 上一次采样的累计 CPU 秒数（sys+user），用于下一次增量推导缓存。
    prev_cpu_seconds: f64,
    /// 上一次采样时的 uptime 秒数。
    prev_uptime_seconds: f64,
}

impl Default for RedisConnectionOverview {
    fn default() -> Self {
        Self {
            version: String::new(),
            used_memory_bytes: 0,
            cpu_usage_percent: None,
            prev_cpu_seconds: 0.0,
            prev_uptime_seconds: 0.0,
        }
    }
}

impl RedisConnectionOverview {
    /// 合入一次新的原始快照，并用两次采样增量推导 CPU 使用率（对齐 RedisInsight）。
    /// 只有 uptime 严格递增时才能可靠换算出百分比，否则保留 None 等下一次采样。
    pub fn apply(&mut self, overview: ConnectionOverview) {
        self.version = overview.version;
        self.used_memory_bytes = overview.used_memory_bytes;
        if let Some(cpu) = overview.cpu {
            let current_cumulative = cpu.sys_seconds + cpu.user_seconds;
            if self.prev_uptime_seconds > 0.0 && cpu.uptime_seconds > self.prev_uptime_seconds {
                let time_delta = cpu.uptime_seconds - self.prev_uptime_seconds;
                let cpu_delta = current_cumulative - self.prev_cpu_seconds;
                self.cpu_usage_percent = Some(((cpu_delta / time_delta) * 100.0).max(0.0));
            } else {
                self.cpu_usage_percent = None;
            }
            self.prev_cpu_seconds = current_cumulative;
            self.prev_uptime_seconds = cpu.uptime_seconds;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TabId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct TabState {
    pub id: TabId,
    pub title: String,
    pub kind: TabKind,
    pub dirty: bool,
}

impl TabState {
    /// 返回标签页所属的「连接 + 库」工作区；设置页与首页全局标签没有归属。
    pub fn workspace(&self) -> Option<TabWorkspace> {
        match &self.kind {
            TabKind::DataEditor(editor) => Some(TabWorkspace {
                connection_id: editor.object.connection_id,
                database: editor
                    .object
                    .database
                    .clone()
                    .unwrap_or_else(|| "main".to_string()),
            }),
            TabKind::QueryEditor(editor) => Some(TabWorkspace {
                connection_id: editor.connection_id,
                database: editor
                    .database
                    .clone()
                    .unwrap_or_else(|| "默认库".to_string()),
            }),
            TabKind::RedisWorkbench(workbench) => Some(TabWorkspace {
                connection_id: workbench.connection_id,
                database: workbench.database.to_string(),
            }),
            TabKind::RedisCli(cli) => Some(TabWorkspace {
                connection_id: cli.connection_id,
                database: cli.database.to_string(),
            }),
            TabKind::MongoCli(cli) => Some(TabWorkspace {
                connection_id: cli.connection_id,
                database: cli.database.clone(),
            }),
            TabKind::RedisPubSub(pubsub) => Some(TabWorkspace {
                connection_id: pubsub.connection_id,
                database: pubsub.database.to_string(),
            }),
            TabKind::CreateTable(create) => Some(TabWorkspace {
                connection_id: create.connection_id,
                database: create
                    .database
                    .clone()
                    .unwrap_or_else(|| "默认库".to_string()),
            }),
            TabKind::ObjectList(list) => list.parent.as_ref().map(|parent| TabWorkspace {
                connection_id: parent.connection_id,
                database: parent
                    .database
                    .clone()
                    .unwrap_or_else(|| parent.name.clone()),
            }),
            TabKind::UserAdmin(admin) => Some(TabWorkspace {
                connection_id: admin.connection_id,
                database: "默认库".to_string(),
            }),
            TabKind::BackupList(list) => Some(TabWorkspace {
                connection_id: list.connection_id,
                database: list.database.clone(),
            }),
            TabKind::ErDiagram(er) => Some(TabWorkspace {
                connection_id: er.connection_id,
                database: er.database.clone(),
            }),
            TabKind::MongoCollection(collection) => Some(TabWorkspace {
                connection_id: collection.object.connection_id,
                database: collection
                    .object
                    .database
                    .clone()
                    .unwrap_or_else(|| "main".to_string()),
            }),
            TabKind::Settings(_) => None,
            TabKind::AiChat { workspace } => workspace.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TabWorkspace {
    pub connection_id: ConnectionId,
    pub database: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsTabState;

#[derive(Clone, Debug, PartialEq)]
pub enum TabKind {
    /// 单例 AI 会话的显示宿主；会话本身仍由 ai_conversation 管理。
    AiChat { workspace: Option<TabWorkspace> },
    ObjectList(ObjectListState),
    DataEditor(DataEditorState),
    QueryEditor(QueryEditorState),
    RedisWorkbench(RedisWorkbenchState),
    RedisCli(RedisCliState),
    /// MongoDB 命令行终端（库内执行 shell 语句的 REPL）。
    MongoCli(MongoCliState),
    RedisPubSub(RedisPubSubState),
    CreateTable(CreateTableState),
    UserAdmin(UserAdminState),
    Settings(SettingsTabState),
    /// 数据库备份列表 tab（侧边栏「备份」节点单击打开，按库一个）。
    BackupList(BackupListState),
    /// ER 关系图 tab（侧边栏「ER 图」节点单击打开，按库一个；画布原型）。
    ErDiagram(ErDiagramState),
    /// MongoDB 集合工作台（文档 / 聚合 / 索引 / 集合信息）。
    MongoCollection(MongoCollectionState),
}

/// 备份列表 tab 的标识状态：仅记录连接 + 库；行数据由 UI 侧渲染时
/// 扫描备份目录（含 .meta.json 元数据）与内存运行任务合成，无需存入 controller。
#[derive(Clone, Debug, PartialEq)]
pub struct BackupListState {
    pub connection_id: ConnectionId,
    pub database: String,
}

/// ER 图 tab 的标识状态：仅记录连接 + 库 + 可选 schema + 可选中心表；图数据由
/// desktop 侧后台加载并按 tab 缓存（与备份列表一样，controller 不持有行数据）。
/// `schema` 为 None 表示整库范围（MySQL/SQLite 单库、PG 全部 schema）。
/// `center_table` 为 Some 时表示「当前表关联 ER」：以该表为中心默认 1 跳绘制邻域；
/// 为 None 时是整库 ER。
/// 中心表用结构化身份（含 schema，PG 跨 schema 同名表不误配，含点标识符安全），
/// 不再用裸表名或按 `.` 拆展示名。
#[derive(Clone, Debug, PartialEq)]
pub struct ErDiagramState {
    pub connection_id: ConnectionId,
    pub database: String,
    pub schema: Option<String>,
    pub center_table: Option<fluxdb_core::ErTableRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserAdminState {
    pub connection_id: ConnectionId,
    /// SQL Server 使用独立的 Login/User/Role 安全模型，避免与 MySQL/PG 字段混用。
    pub sqlserver: Option<SqlServerSecurityState>,
    /// Oracle 用户与权限：账号即 schema，权限模型独立，不复用 MySQL/SQL Server 字段。
    pub oracle: Option<OracleUserAdminState>,
    pub active_detail_tab: UserAdminDetailTab,
    pub users: Vec<DatabaseUserIdentity>,
    pub selected_user: Option<DatabaseUserIdentity>,
    /// 保存成功后下一次用户列表回填应选中的身份（新用户刷新后仍保持选中）。
    pub pending_selected_user: Option<DatabaseUserIdentity>,
    pub creating_user: bool,
    pub grants: Vec<String>,
    pub grants_loaded_user: Option<DatabaseUserIdentity>,
    pub member_grants: Vec<UserRoleMember>,
    pub member_grants_loaded_role: Option<DatabaseUserIdentity>,
    pub search: String,
    pub loading_users: bool,
    pub loading_grants: bool,
    pub loading_member_grants: bool,
    pub applying: bool,
    pub users_error: Option<UserFacingError>,
    pub grants_error: Option<UserFacingError>,
    pub member_grants_error: Option<UserFacingError>,
    pub apply_error: Option<UserFacingError>,
    pub privilege_scope: PrivilegeScope,
    pub privilege_database: String,
    pub privilege_table: String,
    pub privilege_role: String,
    pub grant_option: bool,
    pub selected_privileges: Vec<String>,
    pub privilege_rows: Vec<UserAdminPrivilegeRow>,
    pub base_privilege_rows: Vec<UserAdminPrivilegeRow>,
    pub next_privilege_row_id: u64,
    pub create_user: String,
    pub create_host: String,
    pub auth_plugin: String,
    pub password_expiry_policy: String,
    pub create_password: String,
    pub new_password: String,
    pub max_queries_per_hour: String,
    pub max_updates_per_hour: String,
    pub max_connections_per_hour: String,
    pub max_user_connections: String,
    pub ssl_type: String,
    pub ssl_cipher: String,
    pub ssl_issuer: String,
    pub ssl_subject: String,
    pub role_membership_edits: Vec<UserRoleMembership>,
    pub member_grant_edits: Vec<UserRoleMember>,
    /// 「保存」后弹出的 SQL 确认框内容（五种库共用）。
    ///
    /// 保存是两段式：先把本次草稿渲染成的 SQL 放进这里并弹框展示，用户点「执行」
    /// 才真正提交。`None` 表示没有待确认的 SQL。
    pub pending_sql: Option<UserAdminPendingSql>,
    /// 本次预览是「点保存」触发的：预览完成时直接弹确认框，而不是只填预览页。
    ///
    /// PG / SQL Server 的预览要读服务端（版本探测等），只能异步生成；没有这个标记
    /// 就分不清「用户切到预览页」和「用户点了保存」。Oracle 与 MySQL 同步渲染，不用它。
    pub pending_sql_awaits_preview: bool,
    /// 待确认删除的用户（MySQL 用户与权限删除确认弹框）。
    pub pending_delete_user: Option<DatabaseUserIdentity>,
    /// PostgreSQL 对象权限（T27）：授权目标种类 + schema/对象/函数签名。
    pub pg_grant_kind: PgGrantObjectKind,
    pub pg_grant_schema: String,
    pub pg_grant_object: String,
    pub pg_grant_signature: String,
    /// 选中对象权限的读取结果（owner/默认/显式条目）与角色生效权限（直接 vs 继承）。
    pub pg_object_grants: Option<PgObjectGrants>,
    pub pg_effective_grants: Vec<PgEffectivePrivilege>,
    pub loading_pg_grants: bool,
    pub pg_grants_error: Option<UserFacingError>,
    // ===== PG 用户与角色工作台（改版）：角色列表 / 草稿 / 成员 / 授权草稿 / 保存状态 =====
    /// 全量角色列表（权威数据，替代 DatabaseUserIdentity[host=""] 承载）。
    pub pg_roles: Vec<PgRole>,
    /// 角色列表加载错误（失败不显示为「无角色」）。
    pub pg_roles_error: Option<UserFacingError>,
    /// 当前选中角色名。
    pub pg_selected_role: Option<String>,
    /// 保存成功后下一次角色列表回填应选中的角色名（新建/改名后的最终身份）。
    pub pending_selected_pg_role: Option<String>,
    /// 当前编辑/新建草稿；None 表示无未保存编辑会话。
    pub pg_draft: Option<PgRoleDraft>,
    /// 全量成员关系（pg_auth_members），供「所属角色 / 此角色的成员」双向展示。
    pub pg_memberships: Vec<PgRoleMembership>,
    /// 成员关系是否已加载完成（区分「无成员」与「未加载」）。
    pub pg_memberships_loaded: bool,
    /// 成员关系草稿变更（Grant/Revoke），保存时并入变更计划。
    pub pg_membership_edits: Vec<PgRoleChange>,
    /// 权限页目标数据库（一期同批只允许一个数据库的对象授权）。
    pub pg_grant_database: String,
    /// 权限页可选目标（数据库/schema/表·视图·序列/函数）。
    pub pg_grant_targets: Option<PgGrantTargetLists>,
    pub pg_loading_targets: bool,
    pub pg_targets_error: Option<UserFacingError>,
    /// 对象授权草稿变更（Grant/Revoke/RevokeGrantOption）。
    pub pg_grant_edits: Vec<PgRoleChange>,
    /// 保存状态：空闲 / 提交中 / 结果待核实（提交时断线等不确定结果）。
    pub pg_save_status: PgRoleSaveStatus,
    /// 变更计划构建/渲染错误。
    pub pg_plan_error: Option<UserFacingError>,
    /// SQL 预览页内容（与执行同源的完整语句，含口令明文本；不进历史/日志/AI 上下文）；None 表示尚未生成。
    pub pg_plan_preview: Option<Vec<String>>,
    /// 预览生成中（后台任务未返回）。
    pub pg_preview_loading: bool,
    /// 角色列表筛选：all / login / nologin / predefined。
    pub pg_role_filter: String,
    /// 待确认删除的角色（PG 删除确认弹框）。
    pub pg_pending_delete: Option<String>,
    /// 有草稿时切换角色：待确认的目标角色（确认后丢弃草稿并切换）。
    pub pg_pending_switch: Option<String>,
    /// 服务端是否支持成员级 INHERIT/SET 选项（PG16+；None = 未探测）。
    pub pg_member_options_supported: Option<bool>,
    /// 最近一次对象权限读取的目标指纹（kind|schema|object|signature），用于过期检测。
    pub pg_loaded_target: String,
}

/// PG 变更计划保存状态。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PgRoleSaveStatus {
    #[default]
    Idle,
    /// 提交中（禁用重复保存）。
    Saving,
    /// 提交结果不确定（如提交时断线）：先重新读取核实，不直接重试。
    NeedsVerify,
}

/// PG 草稿布尔属性字段（高级页开关行）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PgDraftAttrField {
    IsSuperuser,
    CanCreateDb,
    CanCreateRole,
    Inherit,
    IsReplication,
    BypassRls,
}

/// PG 对象授权草稿操作。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PgGrantEditOp {
    /// 授予权限（grant_option 决定是否带 GRANT OPTION）。
    Grant,
    /// 撤销基础权限。
    Revoke,
    /// 仅取消可再授权（REVOKE GRANT OPTION FOR），不动基础权限。
    RevokeGrantOption,
}

/// PgRoleDraft 的 UI 扩展（app 层专属展示辅助；外部类型不能写 inherent impl，用 trait 扩展）。
pub trait PgRoleDraftUiExt {
    /// 读取布尔属性字段（UI 开关行通用取值）。
    fn attr(&self, field: PgDraftAttrField) -> bool;
    /// 密码操作下拉的当前标签（与 UI 选项一致）。
    fn password_label(&self) -> String;
    /// 密码有效期模式下拉的当前标签。
    fn valid_until_mode_label(&self) -> String;
}

impl PgRoleDraftUiExt for fluxdb_core::PgRoleDraft {
    fn attr(&self, field: PgDraftAttrField) -> bool {
        match field {
            PgDraftAttrField::IsSuperuser => self.is_superuser,
            PgDraftAttrField::CanCreateDb => self.can_create_db,
            PgDraftAttrField::CanCreateRole => self.can_create_role,
            PgDraftAttrField::Inherit => self.inherit,
            PgDraftAttrField::IsReplication => self.is_replication,
            PgDraftAttrField::BypassRls => self.bypass_rls,
        }
    }

    fn password_label(&self) -> String {
        if self.create {
            match &self.password {
                PgPasswordOp::Set(_) => "设置新密码".to_string(),
                _ => "不设置密码".to_string(),
            }
        } else {
            match &self.password {
                PgPasswordOp::Set(_) => "设置新密码".to_string(),
                PgPasswordOp::Clear => "清除密码".to_string(),
                PgPasswordOp::Keep => "保持不变".to_string(),
            }
        }
    }

    fn valid_until_mode_label(&self) -> String {
        match &self.valid_until {
            PgValidUntilOp::Clear => "清除截止时间（永不过期）".to_string(),
            PgValidUntilOp::At(_) => "自定义截止时间…".to_string(),
            PgValidUntilOp::Keep => "保持不变".to_string(),
        }
    }
}

/// PG 对象权限授权目标种类（UI 选择器用；映射到 `PgObjectGrantScope`）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PgGrantObjectKind {
    #[default]
    Table,
    View,
    Sequence,
    Schema,
    Database,
    Routine,
}

impl PgGrantObjectKind {
    pub fn label(self) -> &'static str {
        match self {
            PgGrantObjectKind::Table => "表",
            PgGrantObjectKind::View => "视图",
            PgGrantObjectKind::Sequence => "序列",
            PgGrantObjectKind::Schema => "schema",
            PgGrantObjectKind::Database => "数据库",
            PgGrantObjectKind::Routine => "函数",
        }
    }

    pub fn all() -> [PgGrantObjectKind; 6] {
        [
            PgGrantObjectKind::Table,
            PgGrantObjectKind::View,
            PgGrantObjectKind::Sequence,
            PgGrantObjectKind::Schema,
            PgGrantObjectKind::Database,
            PgGrantObjectKind::Routine,
        ]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserAdminPrivilegeRow {
    pub id: u64,
    pub database: String,
    pub privileges: Vec<String>,
    pub grant_option: bool,
}

/// 用户与权限右栏页签。每个库一套，互不影响：
///
/// MySQL 一侧是「用户」，PG 一侧是「角色」，SQL Server 与 Oracle 是「账号/主体」，
/// 页签集合本来就不同。曾经五库共用一个枚举，导致任何一库加页签都要改其它库的
/// 穷尽 `match`；现在按库分开，新增页签只动自己那一支。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserAdminDetailTab {
    /// MySQL / TiDB：常规 / 高级 / 所属角色 / 角色成员 / 权限 / SQL 预览。
    MySql(UserAdminMySqlTab),
    /// PostgreSQL：常规 / 高级 / 成员关系 / 权限 / SQL 预览。
    Postgres(PgUserAdminTab),
    /// SQL Server：常规 / 角色 / 权限 / SQL 预览。
    SqlServer(SqlServerUserAdminTab),
    /// Oracle / 达梦：常规 / 角色 / 系统权限 / 对象权限 / SQL 预览。
    Oracle(OracleUserAdminTab),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UserAdminMySqlTab {
    #[default]
    General,
    Advanced,
    MemberOf,
    Members,
    Privileges,
    SqlPreview,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PgUserAdminTab {
    #[default]
    General,
    Advanced,
    MemberOf,
    Privileges,
    SqlPreview,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SqlServerUserAdminTab {
    #[default]
    General,
    Roles,
    Permissions,
    SqlPreview,
}

/// Oracle / 达梦的授权模型是三份独立清单，页签直接对应，不再挤在「权限」一个页里选类型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OracleUserAdminTab {
    #[default]
    General,
    Roles,
    SystemPrivileges,
    ObjectPrivileges,
    SqlPreview,
}

impl UserAdminDetailTab {
    /// 新建标签页时的默认页签：按连接类型给出对应那一套的「常规」。
    pub fn initial_for(scope: PrivilegeScope) -> Self {
        match scope {
            PrivilegeScope::Postgres => Self::Postgres(PgUserAdminTab::General),
            PrivilegeScope::SqlServer => Self::SqlServer(SqlServerUserAdminTab::General),
            PrivilegeScope::Oracle => Self::Oracle(OracleUserAdminTab::General),
            _ => Self::MySql(UserAdminMySqlTab::General),
        }
    }

    /// 回到「常规」页：各库各自的常规页。
    pub fn general(self) -> Self {
        match self {
            Self::MySql(_) => Self::MySql(UserAdminMySqlTab::General),
            Self::Postgres(_) => Self::Postgres(PgUserAdminTab::General),
            Self::SqlServer(_) => Self::SqlServer(SqlServerUserAdminTab::General),
            Self::Oracle(_) => Self::Oracle(OracleUserAdminTab::General),
        }
    }

    /// MySQL 页签判定：非 MySQL 一律 `false`，调用方无需先判断库类型。
    pub fn is_mysql(self, tab: UserAdminMySqlTab) -> bool {
        self == Self::MySql(tab)
    }

    pub fn is_postgres(self, tab: PgUserAdminTab) -> bool {
        self == Self::Postgres(tab)
    }

    pub fn is_sqlserver(self, tab: SqlServerUserAdminTab) -> bool {
        self == Self::SqlServer(tab)
    }

    pub fn is_oracle(self, tab: OracleUserAdminTab) -> bool {
        self == Self::Oracle(tab)
    }
}

impl UserAdminState {
    pub fn new(
        connection_id: ConnectionId,
        database: Option<String>,
        scope: PrivilegeScope,
    ) -> Self {
        Self {
            connection_id,
            sqlserver: None,
            oracle: None,
            active_detail_tab: UserAdminDetailTab::initial_for(scope),
            users: Vec::new(),
            selected_user: None,
            pending_selected_user: None,
            creating_user: false,
            grants: Vec::new(),
            grants_loaded_user: None,
            member_grants: Vec::new(),
            member_grants_loaded_role: None,
            search: String::new(),
            loading_users: false,
            loading_grants: false,
            loading_member_grants: false,
            applying: false,
            users_error: None,
            grants_error: None,
            member_grants_error: None,
            apply_error: None,
            privilege_scope: scope,
            privilege_database: database.unwrap_or_else(|| "*".to_string()),
            privilege_table: "*".to_string(),
            privilege_role: String::new(),
            grant_option: false,
            selected_privileges: vec!["SELECT".to_string()],
            privilege_rows: Vec::new(),
            base_privilege_rows: Vec::new(),
            next_privilege_row_id: 1,
            create_user: "app_user".to_string(),
            create_host: "%".to_string(),
            auth_plugin: "caching_sha2_password".to_string(),
            password_expiry_policy: "DEFAULT".to_string(),
            create_password: String::new(),
            new_password: String::new(),
            max_queries_per_hour: "0".to_string(),
            max_updates_per_hour: "0".to_string(),
            max_connections_per_hour: "0".to_string(),
            max_user_connections: "0".to_string(),
            ssl_type: "NONE".to_string(),
            ssl_cipher: String::new(),
            ssl_issuer: String::new(),
            ssl_subject: String::new(),
            role_membership_edits: Vec::new(),
            member_grant_edits: Vec::new(),
            pending_sql: None,
            pending_sql_awaits_preview: false,
            pending_delete_user: None,
            pg_grant_kind: PgGrantObjectKind::default(),
            pg_grant_schema: String::new(),
            pg_grant_object: String::new(),
            pg_grant_signature: String::new(),
            pg_object_grants: None,
            pg_effective_grants: Vec::new(),
            loading_pg_grants: false,
            pg_grants_error: None,
            pg_roles: Vec::new(),
            pg_roles_error: None,
            pg_selected_role: None,
            pending_selected_pg_role: None,
            pg_draft: None,
            pg_memberships: Vec::new(),
            pg_memberships_loaded: false,
            pg_membership_edits: Vec::new(),
            pg_grant_database: String::new(),
            pg_grant_targets: None,
            pg_loading_targets: false,
            pg_targets_error: None,
            pg_grant_edits: Vec::new(),
            pg_save_status: PgRoleSaveStatus::Idle,
            pg_plan_error: None,
            pg_plan_preview: None,
            pg_preview_loading: false,
            pg_role_filter: "all".to_string(),
            pg_pending_delete: None,
            pg_pending_switch: None,
            pg_member_options_supported: None,
            pg_loaded_target: String::new(),
        }
    }

    pub fn draft_user_identity(&self) -> DatabaseUserIdentity {
        DatabaseUserIdentity {
            user: self.create_user.clone(),
            host: self.create_host.clone(),
            plugin: Some(self.auth_plugin.clone()),
        }
    }

    pub fn reset_advanced_defaults(&mut self) {
        self.max_queries_per_hour = "0".to_string();
        self.max_updates_per_hour = "0".to_string();
        self.max_connections_per_hour = "0".to_string();
        self.max_user_connections = "0".to_string();
        self.ssl_type = "NONE".to_string();
        self.ssl_cipher.clear();
        self.ssl_issuer.clear();
        self.ssl_subject.clear();
    }

    /// MySQL/TiDB 工具栏「取消」：丢弃本地草稿，回到当前选中用户的服务端基线。
    ///
    /// 新建态没有基线，取消即结束新建并回到已选中用户（没有则清空表单）；对象权限行
    /// 回到基线副本，成员/授权草稿与密码字段全部丢弃。
    pub fn discard_mysql_edits(&mut self) {
        self.creating_user = false;
        self.pending_selected_user = None;
        self.active_detail_tab = self.active_detail_tab.general();
        self.pending_sql = None;
        self.apply_error = None;
        self.grants.clear();
        self.grants_loaded_user = None;
        self.grants_error = None;
        self.member_grants.clear();
        self.member_grants_loaded_role = None;
        self.member_grants_error = None;
        self.role_membership_edits.clear();
        self.member_grant_edits.clear();
        self.privilege_rows = self.base_privilege_rows.clone();
        self.new_password.clear();
        self.create_password.clear();
        self.password_expiry_policy = "DEFAULT".to_string();
        self.reset_advanced_defaults();
        match self.selected_user.clone() {
            Some(selected) => {
                self.create_user = selected.user.clone();
                self.create_host = selected.host.clone();
                self.auth_plugin = selected
                    .plugin
                    .clone()
                    .unwrap_or_else(|| "caching_sha2_password".to_string());
            }
            None => {
                self.create_user.clear();
                self.create_host.clear();
                self.auth_plugin.clear();
            }
        }
    }
    // ===== PG 用户与角色工作台辅助（改版）=====

    /// 选中角色的基线数据（来自 pg_roles）；新建草稿时为 None。
    pub fn pg_baseline_role(&self) -> Option<&PgRole> {
        let selected = self.pg_selected_role.as_deref()?;
        self.pg_roles.iter().find(|role| role.name == selected)
    }

    /// 从基线派生编辑草稿（选中角色后右侧面板以草稿渲染）。
    /// 干净草稿与基线 diff 为空，不计入未保存变更。
    pub fn pg_reset_draft_from_baseline(&mut self) {
        self.pg_draft = self.pg_baseline_role().map(PgRoleDraft::from_role);
    }

    /// 工具栏「取消」：丢弃当前角色的全部本地编辑，回到服务端基线。
    ///
    /// 新建草稿没有服务端基线，取消即结束编辑会话；编辑既有角色则重建干净草稿并清空
    /// 成员/授权草稿与预览，选中角色和角色列表保持不变。
    pub fn discard_pg_edits(&mut self) {
        self.pending_selected_pg_role = None;
        let creating = self.pg_draft.as_ref().is_some_and(|draft| draft.create);
        self.pg_membership_edits.clear();
        self.pg_grant_edits.clear();
        self.pg_plan_preview = None;
        self.pg_preview_loading = false;
        self.pg_plan_error = None;
        self.pg_pending_switch = None;
        self.pg_reset_grant_target_session();
        if creating {
            self.pg_selected_role = None;
            self.pg_draft = None;
        } else {
            self.pg_reset_draft_from_baseline();
        }
        self.active_detail_tab = self.active_detail_tab.general();
    }
    /// 清空权限页目标选择及其读取结果；数据库/Schema/对象都必须由用户重新显式选择。
    pub fn pg_reset_grant_target_session(&mut self) {
        self.pg_grant_kind = PgGrantObjectKind::default();
        self.pg_grant_database.clear();
        self.pg_grant_schema.clear();
        self.pg_grant_object.clear();
        self.pg_grant_signature.clear();
        self.pg_grant_targets = None;
        self.pg_loading_targets = false;
        self.pg_targets_error = None;
        self.pg_object_grants = None;
        self.pg_effective_grants.clear();
        self.loading_pg_grants = false;
        self.pg_grants_error = None;
        self.pg_grant_edits.clear();
        self.pg_loaded_target.clear();
    }

    /// 右侧角色编辑会话回到初始态：选中角色/角色列表保留，目标、成员、草稿编辑和预览清空。
    pub fn pg_reset_role_editor_session(&mut self) {
        self.pg_membership_edits.clear();
        self.pg_memberships.clear();
        self.pg_memberships_loaded = false;
        self.pg_reset_grant_target_session();
        self.pg_pending_switch = None;
        self.pg_plan_preview = None;
        self.pg_preview_loading = false;
        self.pg_plan_error = None;
        self.active_detail_tab = self.active_detail_tab.general();
    }

    /// 预定义角色（pg_ 前缀）：不提供属性编辑/删除，仅可查看权限与按授权能力管理成员。
    pub fn pg_is_predefined_role(name: &str) -> bool {
        name.starts_with("pg_")
    }

    /// 草稿角色名（新建待填时为空串）。
    pub fn pg_draft_name(&self) -> Option<&str> {
        self.pg_draft.as_ref().map(|draft| draft.name.trim())
    }

    /// 授权/成员变更的受影响角色名：新建草稿用草稿名（保存时先建角色再授权），
    /// 编辑既有角色用选中名。
    pub fn pg_effective_grantee_name(&self) -> String {
        self.pg_draft_name()
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .or_else(|| self.pg_selected_role.clone())
            .unwrap_or_default()
    }

    /// 是否存在未保存的草稿变更（常规/高级/成员/授权任一）。
    ///
    /// 纯函数判断：与基线 diff + 密码/有效期操作 + 成员与授权草稿列表非空。
    pub fn pg_has_draft_changes(&self) -> bool {
        let Some(draft) = &self.pg_draft else {
            return false;
        };
        if !self.pg_membership_edits.is_empty() || !self.pg_grant_edits.is_empty() {
            return true;
        }
        if draft.password != PgPasswordOp::Keep || draft.valid_until != PgValidUntilOp::Keep {
            return true;
        }
        if draft.create {
            return true;
        }
        let Some(baseline) = self.pg_baseline_role() else {
            return !draft.name.trim().is_empty();
        };
        draft.name.trim() != baseline.name
            || fluxdb_core::pg_role_attributes_diff(Some(baseline), draft).is_some()
    }

    /// 「所属角色」：当前角色直接所在的组角色（读取 pg_auth_members 过滤）。
    ///
    /// 新建草稿在服务端还没有角色行，返回空 —— 新角色的成员关系只能靠追加。
    pub fn pg_member_of_roles(&self) -> Vec<&PgRoleMembership> {
        if self.pg_draft.as_ref().is_some_and(|draft| draft.create) {
            return Vec::new();
        }
        let selected = self.pg_selected_role.as_deref();
        self.pg_memberships
            .iter()
            .filter(|m| Some(m.member.as_str()) == selected)
            .collect()
    }

    /// 「此角色的成员」：把当前角色授予了哪些成员。
    pub fn pg_members_of_role(&self) -> Vec<&PgRoleMembership> {
        if self.pg_draft.as_ref().is_some_and(|draft| draft.create) {
            return Vec::new();
        }
        let selected = self.pg_selected_role.as_deref();
        self.pg_memberships
            .iter()
            .filter(|m| Some(m.grantee.as_str()) == selected)
            .collect()
    }

    /// 成员关系草稿是否已包含对该 (role, member) 的变更（避免重复条目）。
    pub fn pg_membership_edit_index(&self, role: &str, member: &str) -> Option<usize> {
        self.pg_membership_edits.iter().position(|edit| match edit {
            PgRoleChange::GrantMembership {
                role: r, member: m, ..
            }
            | PgRoleChange::RevokeMembership { role: r, member: m } => r == role && m == member,
            _ => false,
        })
    }

    /// 对象授权草稿定位：同 privilege + 同 scope 的既有变更条目。
    pub fn pg_grant_edit_index(
        &self,
        privilege: &str,
        scope: &PgObjectGrantScope,
    ) -> Option<usize> {
        self.pg_grant_edits.iter().position(|edit| match edit {
            PgRoleChange::GrantObject {
                privilege: p,
                scope: s,
                ..
            }
            | PgRoleChange::RevokeObject {
                privilege: p,
                scope: s,
                ..
            }
            | PgRoleChange::RevokeGrantOption {
                privilege: p,
                scope: s,
                ..
            } => p == privilege && s == scope,
            _ => false,
        })
    }

    pub fn effective_role_memberships(&self) -> Vec<UserRoleMembership> {
        // 新建草稿也走这里：草稿用户没有服务端授权，基线天然全为「未授予」，
        // 勾选只会产生 GRANT，随创建语句在同一次保存里提交。
        let Some(user) = self.selected_user.as_ref() else {
            return Vec::new();
        };
        let mut memberships = role_memberships_from_grants(&self.users, &self.grants, user);
        for edit in &self.role_membership_edits {
            if let Some(membership) = memberships
                .iter_mut()
                .find(|membership| membership.role == edit.role)
            {
                membership.granted = edit.granted;
                membership.default_role = edit.default_role;
            }
        }
        memberships
    }

    pub fn role_membership_dirty(&self) -> bool {
        let Some(user) = self.selected_user.as_ref().filter(|_| !self.creating_user) else {
            return false;
        };
        let base = role_memberships_from_grants(&self.users, &self.grants, user);
        let effective = self.effective_role_memberships();
        effective.iter().any(|membership| {
            base.iter()
                .find(|base| base.role == membership.role)
                .is_some_and(|base| {
                    base.granted != membership.granted
                        || base.default_role != membership.default_role
                })
        })
    }

    pub fn grants_loaded_for_selected_user(&self) -> bool {
        self.selected_user.as_ref().is_some_and(|selected| {
            self.grants_loaded_user
                .as_ref()
                .is_some_and(|loaded| loaded == selected)
        })
    }

    pub fn effective_role_members(&self) -> Vec<UserRoleMember> {
        let Some(role) = self.selected_user.as_ref() else {
            return Vec::new();
        };
        // 新建草稿不在服务端，没有可读的成员基线：候选取现有账号列表，全部按
        // 「未授予」起算，勾选只会生成 GRANT。
        if self.creating_user {
            return self
                .users
                .iter()
                .filter(|user| *user != role)
                .map(|user| UserRoleMember {
                    member: user.clone(),
                    granted: self
                        .member_grant_edits
                        .iter()
                        .find(|edit| edit.member == *user)
                        .is_some_and(|edit| edit.granted),
                })
                .collect();
        }
        let mut members = self
            .member_grants
            .iter()
            .filter(|member| member.member != *role)
            .cloned()
            .collect::<Vec<_>>();
        for edit in &self.member_grant_edits {
            if let Some(member) = members
                .iter_mut()
                .find(|member| member.member == edit.member)
            {
                member.granted = edit.granted;
            }
        }
        members
    }

    pub fn role_members_dirty(&self) -> bool {
        if !self.member_grants_loaded_for_selected_role() {
            return false;
        }
        let base = &self.member_grants;
        self.effective_role_members().iter().any(|member| {
            base.iter()
                .find(|base| base.member == member.member)
                .is_some_and(|base| base.granted != member.granted)
        })
    }

    pub fn member_grants_loaded_for_selected_role(&self) -> bool {
        self.selected_user.as_ref().is_some_and(|selected| {
            self.member_grants_loaded_role
                .as_ref()
                .is_some_and(|loaded| loaded == selected)
        })
    }

    pub fn add_privilege_row(&mut self, database: String) {
        let id = self.next_privilege_row_id;
        self.next_privilege_row_id += 1;
        self.privilege_rows.push(UserAdminPrivilegeRow {
            id,
            database,
            privileges: Vec::new(),
            grant_option: false,
        });
    }

    pub fn set_privilege_row_database(&mut self, row_id: u64, database: String) {
        if let Some(row) = self.privilege_rows.iter_mut().find(|row| row.id == row_id) {
            row.database = database;
        }
    }

    pub fn toggle_privilege_row_privilege(&mut self, row_id: u64, privilege: String) {
        let Some(row) = self.privilege_rows.iter_mut().find(|row| row.id == row_id) else {
            return;
        };
        if let Some(index) = row.privileges.iter().position(|item| item == &privilege) {
            row.privileges.remove(index);
        } else {
            row.privileges.push(privilege);
        }
    }

    pub fn set_privilege_row_grant_option(&mut self, row_id: u64, enabled: bool) {
        if let Some(row) = self.privilege_rows.iter_mut().find(|row| row.id == row_id) {
            row.grant_option = enabled;
        }
    }

    pub fn privileges_dirty(&self) -> bool {
        normalized_user_admin_privilege_rows(&self.privilege_rows)
            != normalized_user_admin_privilege_rows(&self.base_privilege_rows)
    }

    pub fn set_privilege_grants(&mut self, grants: Vec<DatabasePrivilegeGrant>) {
        self.next_privilege_row_id = 1;
        self.privilege_rows = grants
            .into_iter()
            .map(|grant| {
                let id = self.next_privilege_row_id;
                self.next_privilege_row_id += 1;
                UserAdminPrivilegeRow {
                    id,
                    database: grant.database,
                    privileges: grant.privileges,
                    grant_option: grant.grant_option,
                }
            })
            .collect();
        self.base_privilege_rows = self.privilege_rows.clone();
    }

    pub fn set_role_membership_granted(&mut self, role: DatabaseUserIdentity, granted: bool) {
        let current = self
            .effective_role_memberships()
            .into_iter()
            .find(|membership| membership.role == role);
        let default_role = current
            .as_ref()
            .is_some_and(|membership| membership.default_role);
        self.set_role_membership_edit(role, granted, default_role);
    }

    pub fn set_role_membership_default(&mut self, role: DatabaseUserIdentity, default_role: bool) {
        let current = self
            .effective_role_memberships()
            .into_iter()
            .find(|membership| membership.role == role);
        let granted = current
            .as_ref()
            .is_some_and(|membership| membership.granted);
        self.set_role_membership_edit(role, granted, default_role);
    }

    pub fn set_role_member_granted(&mut self, member: DatabaseUserIdentity, granted: bool) {
        if let Some(edit) = self
            .member_grant_edits
            .iter_mut()
            .find(|edit| edit.member == member)
        {
            edit.granted = granted;
        } else {
            self.member_grant_edits
                .push(UserRoleMember { member, granted });
        }
    }

    fn set_role_membership_edit(
        &mut self,
        role: DatabaseUserIdentity,
        granted: bool,
        default_role: bool,
    ) {
        if let Some(edit) = self
            .role_membership_edits
            .iter_mut()
            .find(|edit| edit.role == role)
        {
            edit.granted = granted;
            edit.default_role = default_role;
        } else {
            self.role_membership_edits.push(UserRoleMembership {
                role,
                granted,
                default_role,
            });
        }
    }
}

/// 「保存」确认框里待执行的 SQL（五种库共用同一弹框）。
///
/// 只存在内存里：不落历史、日志或 AI 上下文；口令按草稿原样显示，与「SQL 预览」页一致。
#[derive(Clone, Debug, PartialEq)]
pub struct UserAdminPendingSql {
    pub sql: String,
    /// 是否包含破坏性语句（DROP / REVOKE 等），决定确认按钮用危险色。
    pub danger: bool,
    /// 确认后走哪条执行路径：各库的计划构建与提交方式不同，不能按 SQL 文本猜。
    pub kind: UserAdminPendingKind,
}

/// 确认框确认后要调用的执行路径。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserAdminPendingKind {
    /// MySQL / TiDB：直接执行渲染出的语句。
    MySql,
    /// PostgreSQL：重新构建角色计划并单事务提交。
    Postgres,
    /// SQL Server：重新构建安全计划并事务提交。
    SqlServer,
    /// Oracle / 达梦：重新合成计划后逐条提交（不可整体回滚）。
    Oracle,
}

fn normalized_user_admin_privilege_rows(
    rows: &[UserAdminPrivilegeRow],
) -> Vec<(String, Vec<String>, bool)> {
    let mut rows = rows
        .iter()
        .filter(|row| !row.privileges.is_empty())
        .map(|row| {
            let mut privileges = row.privileges.clone();
            privileges.sort();
            privileges.dedup();
            (row.database.clone(), privileges, row.grant_option)
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectListState {
    pub parent: Option<ObjectPath>,
    pub objects: Vec<ObjectSummary>,
    pub loading: bool,
    pub error: Option<UserFacingError>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DataEditorState {
    pub object: ObjectPath,
    pub page: Option<DataPage>,
    pub original_page: Option<DataPage>,
    pub pagination: Pagination,
    /// 表页数据加载令牌：每次用户筛选/刷新或 AI 查询都会递增。
    /// 后台结果只允许写回与当前令牌一致的加载，避免旧结果覆盖新操作。
    pub load_generation: u64,
    pub changes: Option<DataChangeSet>,
    pub editing_cell: Option<CellPosition>,
    pub cell_detail_panel: CellDetailPanelState,
    pub table_info: TableInfoState,
    pub loading: bool,
    /// 改动正在提交到数据库：表格保持可见（与 `loading` 的整页替换不同），
    /// 但提交类操作 disabled，避免用户在写入期间继续改动同一批数据。
    pub submitting: bool,
    pub error: Option<UserFacingError>,
}

impl DataEditorState {
    /// 开始一次新的表页加载，返回本次令牌。
    pub fn begin_load(&mut self) -> u64 {
        self.load_generation = self.load_generation.wrapping_add(1);
        self.loading = true;
        self.error = None;
        self.load_generation
    }

    /// 只有最新一次加载可以写回数据或终态。
    pub fn accepts_load(&self, generation: u64) -> bool {
        self.load_generation == generation
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableInfoState {
    pub open: bool,
    pub active_tab: TableInfoTab,
    pub width: f32,
    pub search: String,
    pub highlighted_column: Option<String>,
    pub ddl_wrap: bool,
    pub indexes: LoadState<Vec<IndexInfo>>,
    pub foreign_keys: LoadState<Vec<ForeignKeyInfo>>,
    pub triggers: LoadState<Vec<TriggerInfo>>,
    pub ddl: LoadState<String>,
}

/// 后台提交数据改动的产物：`ApplyDataChanges*` 的数据库 I/O 结果 + 提交后重载的数据页。
///
/// 拆成独立类型是因为写入与重载都在后台线程完成，状态写回必须由 UI 线程的回桥命令负责。
#[derive(Debug, Clone, PartialEq)]
pub struct DataApplyResult {
    /// 来自「查询结果」页签（与数据编辑器标签的落状态方式不同：结果区按页索引回写）。
    pub query_result: bool,
    /// 提交后按当前筛选/排序重新加载的数据页。
    pub page: DataPage,
    /// 写入回执，用于历史记录与补偿显示。
    pub outcome: AppliedChangeOutcome,
}

impl Default for TableInfoState {
    fn default() -> Self {
        Self {
            open: false,
            active_tab: TableInfoTab::Columns,
            // 默认给字段名、类型和注释留出稳定的三列空间，避免右侧预览一打开就换行。
            width: 460.,
            search: String::new(),
            highlighted_column: None,
            ddl_wrap: false,
            indexes: LoadState::NotLoaded,
            foreign_keys: LoadState::NotLoaded,
            triggers: LoadState::NotLoaded,
            ddl: LoadState::NotLoaded,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum TableInfoTab {
    Columns,
    Indexes,
    ForeignKeys,
    Triggers,
    Ddl,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState<T> {
    NotLoaded,
    Loading,
    Loaded(T),
    Failed(UserFacingError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TableInfoResult {
    Indexes(Vec<IndexInfo>),
    ForeignKeys(Vec<ForeignKeyInfo>),
    Triggers(Vec<TriggerInfo>),
    Ddl(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellPosition {
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellDetailPanelState {
    pub open: bool,
    pub active_cell: Option<CellPosition>,
    pub mode: CellDetailMode,
    pub edit_value: String,
    /// 按需读到的完整文本；`None` 表示面板里只有数据页带的预览。
    ///
    /// 预览是片段，不能当完整值保存，因此编辑前必须先有这里的完整值。
    pub full_value: Option<String>,
    /// 用户已点「编辑」，但完整文本还在加载：读完后直接进编辑态，不用再点一次。
    pub pending_edit: bool,
}

impl Default for CellDetailPanelState {
    fn default() -> Self {
        Self {
            open: false,
            active_cell: None,
            mode: CellDetailMode::View,
            edit_value: String::new(),
            full_value: None,
            pending_edit: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellDetailMode {
    View,
    Edit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryEditorState {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// 编辑器当前 schema 作用域（PG）；MySQL/TiDB/Redis 恒为 None。
    pub schema: Option<String>,
    pub text: String,
    pub origin: Option<QueryOrigin>,
    pub saved_fingerprint: Option<QueryFingerprint>,
    pub running: bool,
    pub results: Vec<DataPage>,
    pub result_editors: BTreeMap<usize, DataEditorState>,
    pub active_result_editor: Option<usize>,
    pub summaries: Vec<QueryExecutionSummary>,
    pub error: Option<UserFacingError>,
}

/// Redis 命令执行器（Workbench）面板状态。
///
/// 与 SQL 查询面板分离：结果不是表格数据（DataPage），而是通用的命令回复
/// （CommandWorkbenchExecution），由 RedisConnector::execute_command_workbench 产出。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisWorkbenchState {
    pub connection_id: ConnectionId,
    /// 目标逻辑数据库编号（0-15）。
    pub database: u32,
    /// 顶部输入框的草稿文本。点击 Run 后立刻清空，命令文本进入 `executions` 记录。
    pub text: String,
    pub running: bool,
    /// 当前会话内按执行顺序追加的命令执行记录（一执行一条记录）。
    /// 每条都能单独重跑（Run）或删除（Delete），不跨会话持久化。
    pub executions: Vec<CommandWorkbenchExecution>,
    pub error: Option<UserFacingError>,
    /// 最近一次已执行命令文本的指纹；用于让「未保存（未执行改动）」标记在标题上实时更新：
    /// 输入后标记脏，执行成功后清除脏标记，再次修改后又复现。
    /// 点击 Run 清空输入时置为空文本指纹，避免空输入仍被标脏。
    pub saved_fingerprint: Option<QueryFingerprint>,
    /// 用于为 `executions` 记录分配 `CommandWorkbenchExecution.id` 的单调递增计数器。
    pub next_execution_id: u64,
    /// 当前处于折叠态的执行记录 ID 集合（仅 UI 展示用，会话内有效，不持久化）。
    /// 折叠后只显示记录头部（命令文本 + 状态 + 耗时 + 操作），隐藏其下各子命令的 reply。
    pub collapsed: BTreeSet<u64>,
    /// 以 JSON 视图展示的结果项集合，键为 `(execution_id, command_index)`。
    ///
    /// 仅当某条命令属于 JSON 命令（如 `JSON.GET`）时才允许加入；非 JSON 命令不显示
    /// `Text / JSON` 切换，也就不会出现在此集合。按执行项维度记忆，与会话内有效、不持久化，
    /// 保证一次执行里的多条命令可以各自独立选择展示模式。
    pub json_views: BTreeSet<(u64, usize)>,
}

impl RedisWorkbenchState {
    /// 是否有未执行（未保存）的命令改动。已有执行指纹时与最新文本比对；
    /// 从未执行过（无指纹）则只要有非空文本即视为脏。
    pub fn has_unsaved_text(&self) -> bool {
        match self.saved_fingerprint {
            Some(saved) => saved != QueryFingerprint::for_text(&self.text),
            None => !self.text.trim().is_empty(),
        }
    }
}

/// Redis CLI 终端标签页：只定位到「某个连接的某个库」，具体交互（PTY 进程 / 网格 / 交互）
/// 由桌面端 `TerminalSessionModel` 承担，state 侧仅作为 tab 定位与复用 key。
/// 与 RedisWorkbench 并存：Workbench 走输入框 + 结果面板，Redis CLI 走真实终端 surface。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisCliState {
    pub connection_id: ConnectionId,
    /// 目标逻辑数据库编号（0-15）。
    pub database: u32,
}

impl RedisCliState {
    /// tab 复用 key：同一连接 + 同一库在右侧菜单 / 双击打开时复用已有标签页。
    pub fn session_key(&self) -> fluxdb_core::terminal::TerminalSessionKey {
        fluxdb_core::terminal::TerminalSessionKey::Redis {
            connection_id: self.connection_id.0,
            database: self.database,
        }
    }
}

/// MongoDB 命令行标签页：只定位到「某个连接的某个库」。真正的 REPL（行缓冲、历史、
/// 命令执行）由桌面端终端 surface + `terminal_mongo` 的 adapter 承担，与 Redis CLI 同构。
#[derive(Clone, Debug, PartialEq)]
pub struct MongoCliState {
    pub connection_id: ConnectionId,
    pub database: String,
}

impl MongoCliState {
    /// tab 复用 key：同一连接 + 同一库只保留一个命令行标签页。
    pub fn session_key(&self) -> fluxdb_core::terminal::TerminalSessionKey {
        fluxdb_core::terminal::TerminalSessionKey::Mongo {
            connection_id: self.connection_id.0,
            database: self.database.clone(),
        }
    }
}

/// Redis Pub/Sub 标签页：只定位到「某个连接的某个库」，具体的订阅连接、消息流与交互
/// 由桌面端 `PubSubSessionModel` 承担，state 侧仅作为 tab 定位与复用 key。
/// 与 RedisCli 并列：CLI 走真实终端 surface，Pub/Sub 走上「订阅 + 实时消息流」专用页面。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisPubSubState {
    pub connection_id: ConnectionId,
    /// 目标逻辑数据库编号（0-15）。
    pub database: u32,
}

impl QueryEditorState {
    pub fn has_unsaved_sql(&self) -> bool {
        match (&self.origin, self.saved_fingerprint) {
            (None, _) => !self.text.trim().is_empty(),
            (Some(_), Some(saved)) => saved != QueryFingerprint::for_text(&self.text),
            (Some(_), None) => !self.text.trim().is_empty(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QueryOrigin {
    Connection { query_id: u64 },
    File { path: PathBuf },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryFingerprint {
    pub len: usize,
    pub hash: u64,
}

impl QueryFingerprint {
    pub fn for_text(text: &str) -> Self {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in text.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        Self {
            len: text.len(),
            hash,
        }
    }
}

include!("query_history_state.rs");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryHistoryKind {
    Query,
    DataChange,
    SchemaChange,
}

/// Redis Workbench 命令历史单条记录（App 层内存态，与 `QueryHistoryEntry` 同层级）。
///
/// 与 RedisInsight 对齐：命令文本 + 结果摘要 + 时间 + 来源，按连接 + 逻辑库隔离；
/// 成功 / 失败都会入历史，保留排障信息。底层持久化字段见 fluxdb-storage 的
/// `RedisWorkbenchHistoryRecord`。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisWorkbenchHistoryEntry {
    /// scope 内去重的记录 ID（用于删除定位）。
    pub id: u64,
    pub connection_id: ConnectionId,
    /// Redis 逻辑数据库编号（0-15）。
    pub database: u32,
    /// 命令文本，可回填到 Workbench 输入框。
    pub text: String,
    /// 执行是否成功。
    pub success: bool,
    /// 执行 Unix 秒级时间戳。
    pub executed_at_unix_secs: u64,
    /// 人类可读的结果摘要。
    pub summary: String,
    /// 来源类型（Workbench / HistoryRerun / KeyShortcut）。
    pub source: CommandExecutionSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskState {
    pub label: String,
    pub running: bool,
}

include!("state_commands.rs");
include!("state_events.rs");

#[derive(Clone, Debug)]
struct CompletionCacheEntry<T> {
    value: T,
    fetched_at: u64,
}

impl<T> CompletionCacheEntry<T> {
    fn is_fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.fetched_at) <= COMPLETION_INDEX_TTL_SECONDS
    }
}

#[derive(Debug, Default)]
struct CompletionCache {
    tables: BTreeMap<CompletionTablesKey, CompletionCacheEntry<Vec<CompletionTable>>>,
    all_schema_tables:
        BTreeMap<CompletionAllSchemaTablesKey, CompletionCacheEntry<Vec<CompletionTable>>>,
    columns: BTreeMap<CompletionColumnsKey, CompletionCacheEntry<Vec<CompletionColumn>>>,
    routines: BTreeMap<CompletionRoutinesKey, CompletionCacheEntry<Vec<CompletionRoutine>>>,
    triggers: BTreeMap<CompletionTriggersKey, CompletionCacheEntry<Vec<CompletionTrigger>>>,
    /// 按表缓存的真实外键元数据（P2.13 FK JOIN）。
    foreign_keys:
        BTreeMap<CompletionColumnsKey, CompletionCacheEntry<Vec<fluxdb_core::ForeignKeyInfo>>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionTablesKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionAllSchemaTablesKey {
    connection_id: ConnectionId,
    database: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionColumnsKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
    table: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionRoutinesKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionTriggersKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
}

#[derive(Clone)]
pub struct AppController {
    state: AppState,
    next_connection_id: u64,
    next_group_id: u64,
    next_tab_id: u64,
    completion_cache: Arc<Mutex<CompletionCache>>,
    completion_index: Arc<Mutex<CompletionIndex>>,
    completion_index_storage: Option<fluxdb_storage::FileStorage>,
    /// AI 平台存储：设置页「大模型 / 工具与MCP」读写模型、Agent、插件与MCP 配置。
    /// UI 只通过 AppCommand 访问，不直接持有。
    ai_storage: Option<fluxdb_storage::FileStorage>,
    /// ER 逻辑关系服务按作用域复用；生命周期由应用层管理，UI 不直接持有文件 store。
    er_model_storage: Option<fluxdb_storage::FileStorage>,
    er_model_services: Arc<Mutex<BTreeMap<String, Arc<ErModelService>>>>,
    /// T071/F004：可关闭的轻量个性化（recency/frequency）。默认关闭，
    /// 关闭时排序与确定性基线一致。只记匿名 label，不记完整 SQL/敏感值。
    recency: Arc<Mutex<RecencyFrequency>>,
    /// 查询取消标志（按标签）。`StartQueryExecution` 登记新标志，后台执行线程克隆 `Arc`
    /// 后轮询，「停止」按钮经 `CancelQueryExecution` 置位。必须放在共享 `Arc` 里而不是
    /// `AppState`：UI 线程与后台执行线程各持一份 `AppController` 副本，只有共享 `Arc`
    /// 才能让两边看到同一个标志。
    query_cancel_flags: Arc<Mutex<BTreeMap<TabId, Arc<std::sync::atomic::AtomicBool>>>>,
    /// ER 元数据共享缓存（字段按需 + 关系索引 + 连接修订）。UI 线程与后台执行线程
    /// 各持一份 `AppController` 副本，`Arc<Mutex>` 让两边共享同一缓存与并发协调。
    /// 见 parts/er_catalog.rs。
    er_catalog: Arc<Mutex<crate::ErCatalogCache>>,
}
