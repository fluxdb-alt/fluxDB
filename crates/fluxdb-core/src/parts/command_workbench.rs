// 通用命令执行器 / Workbench 领域模型。
//
// 该模块不依赖 GPUI，也不包含任何 Redis/MySQL 连接实现，只描述命令执行的通用模型：
// 执行目标、执行请求、执行选项、执行结果、Reply 结构与能力开关。
// 参考 RedisInsight command-execution.ts 的字段边界，但 generalized 了 target，
// 不使用 databaseId 作为唯一上下文，以便后续接入 MySQL / System 等 backend。

/// 命令执行的上下文目标。
///
/// `Redis` 是当前唯一开放的后端；`MySql` 与 `System` 作为通用模型预留，
/// 默认不开放入口（系统级命令属于高风险能力，需后续单独设计权限与输出流）。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommandExecutionTarget {
    /// Redis 连接 + 逻辑数据库编号。
    Redis {
        connection_id: ConnectionId,
        database: u32,
    },
    /// MySQL 连接 + 可选 database / schema。
    MySql {
        connection_id: ConnectionId,
        database: Option<String>,
        schema: Option<String>,
    },
    /// 系统级命令行（仅预留模型，默认不开放）。
    System {
        profile_id: Option<String>,
        working_dir: Option<String>,
    },
}

/// 一次携带完整运行选项的命令执行请求。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandWorkbenchRequest {
    /// 执行目标（决定 backend 与上下文）。
    pub target: CommandExecutionTarget,
    /// 用户输入的命令文本（Redis CLI / SQL / shell 原始文本）。
    pub text: String,
    /// 运行模式：文本可读 / 原始 / RESP 协议感。
    pub run_mode: CommandRunMode,
    /// 结果组织模式：默认 / 聚合 / 静默。
    pub results_mode: CommandResultsMode,
    /// 批量执行时每批的命令条数。
    pub batch_size: usize,
    /// 执行中是否在遇到失败后继续执行后续命令。
    pub continue_on_error: bool,
    /// 本次执行的来源（用于标记重跑等场景）。
    pub source: CommandExecutionSource,
}

/// 运行模式。
///
/// RedisInsight 的 `RunQueryMode.ASCII` / `Raw` 对应 `Text` / `Raw`。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommandRunMode {
    /// 类似 redis-cli 的可读文本输出。
    Text,
    /// 尽量原样保留原始字节/文本。
    Raw,
    /// 展示 RESP 协议结构感（-、+、$、* 等前缀）。
    Resp,
}

/// 结果组织模式。
///
/// RedisInsight 的 `ResultsMode.Default` / `GroupMode` / `Silent`。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommandResultsMode {
    /// 每条命令独立一张结果卡片。
    Default,
    /// 一次提交聚合为一个 execution，内部列出所有 item 与 summary。
    Group,
    /// 成功结果默认折叠 / 仅保留 summary，失败结果展开。
    Silent,
}

/// 执行来源。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommandExecutionSource {
    /// 用户在 Workbench 页面直接执行。
    Workbench,
    /// 从历史记录重跑。
    HistoryRerun,
    /// 从 Key 快捷操作预填后执行。
    KeyShortcut,
}

/// 一次完整执行的产物，包含命令列表、summary 与耗时。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandWorkbenchExecution {
    /// 执行 ID（App 层分配，单调递增）。
    pub id: u64,
    /// 执行目标。
    pub target: CommandExecutionTarget,
    /// 用户输入的原始命令文本。
    pub text: String,
    /// 切分并执行后的命令 item 列表。
    pub commands: Vec<CommandExecutionItem>,
    /// 总数 / 成功 / 失败 / 跳过统计。
    pub summary: CommandExecutionSummary,
    /// 执行时的运行模式。
    pub run_mode: CommandRunMode,
    /// 执行时的结果组织模式。
    pub results_mode: CommandResultsMode,
    /// 开始执行的 Unix 秒级时间戳。
    pub started_at_unix_secs: u64,
    /// 整次执行耗时（毫秒）。
    pub elapsed_ms: u64,
}

/// 单条命令的执行结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandExecutionItem {
    /// 命令原始文本（含参数）。
    pub command: String,
    /// 解析后的 argv 预览（用于展示，不参与协议发送）。
    pub argv_preview: Vec<String>,
    /// 执行状态。
    pub status: CommandExecutionStatus,
    /// 命令回复。
    pub reply: CommandReply,
    /// 单条命令耗时（毫秒）。
    pub elapsed_ms: u64,
    /// 是否因超限仅保存了 preview。
    pub size_limit_exceeded: bool,
}

/// 单条命令的执行状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CommandExecutionStatus {
    /// 执行成功。
    Success,
    /// 执行失败（命令错误 / 被 unsupported / blocking 拦截）。
    Failed,
    /// 因上一命令失败且 continue_on_error=false 而跳过。
    Skipped,
}

/// 一次执行的汇总统计。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandExecutionSummary {
    /// 总命令数。
    pub total: usize,
    /// 成功数。
    pub success: usize,
    /// 失败数。
    pub failed: usize,
    /// 跳过数。
    pub skipped: usize,
}

/// 结构化的命令回复，保留原始结构以便 UI 在多种视图间切换。
///
/// Redis RESP2 目前只有 simple / int / bulk / array / error；
/// Map / Float 为 RESP3、MySQL metadata 或系统命令结构化输出预留。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CommandReply {
    /// 空回复（nil）。
    Nil,
    /// 简单字符串（RESP `+` 前缀）。
    Status(String),
    /// 整数（RESP `:` 前缀）。
    Integer(i64),
    /// 浮点数。
    Float(f64),
    /// 二进制 bulk（RESP `$` 前缀）。
    Bulk(CommandBulk),
    /// 数组（RESP `*` 前缀）。
    Array(Vec<CommandReply>),
    /// 键值映射结构。
    Map(Vec<(CommandReply, CommandReply)>),
    /// 错误。
    Error(String),
    /// formatter 产出的纯文本视图（保留原始结构的场景由 backend 决定）。
    Text(String),
}

/// 二进制 bulk 的载体：尽量以 UTF-8 文本呈现，不可解码时提供 hex 预览与字节长度。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandBulk {
    /// UTF-8 可解码的文本内容（不可解码时为 None）。
    pub text: Option<String>,
    /// 不可解码时的十六进制预览。
    pub bytes_preview_hex: Option<String>,
    /// 原始字节长度。
    pub byte_len: u64,
    /// 是否为二进制（含不可打印字符）。
    pub binary: bool,
}

/// Workbench backend 能力开关，供 UI 决定是否展示对应控件。
#[derive(Clone, Debug, PartialEq)]
pub struct CommandWorkbenchCapabilities {
    /// 是否支持 batch 批量执行。
    pub supports_batch: bool,
    /// 是否支持 Group 聚合结果。
    pub supports_group_results: bool,
    /// 是否支持 Raw 模式。
    pub supports_raw_mode: bool,
    /// 是否支持命令补全。
    pub supports_completion: bool,
    /// 是否支持危险命令二次确认。
    pub supports_dangerous_confirmation: bool,
    /// 是否支持执行取消。
    pub supports_cancel: bool,
    /// 单次执行允许的最大 batch 数。
    pub max_batch_size: usize,
    /// 默认 batch 数。
    pub default_batch_size: usize,
}

impl Default for CommandWorkbenchCapabilities {
    fn default() -> Self {
        Self {
            supports_batch: false,
            supports_group_results: false,
            supports_raw_mode: false,
            supports_completion: false,
            supports_dangerous_confirmation: false,
            supports_cancel: false,
            max_batch_size: 0,
            default_batch_size: 0,
        }
    }
}
