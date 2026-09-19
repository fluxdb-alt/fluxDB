/// 输出格式属于领域模型，不能通过文件后缀反推执行方式。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BackupFormat {
    Sql,
    SqliteBinary,
}
impl BackupFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Sql => "sql",
            Self::SqliteBinary => "db",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Sql => "SQL 转储",
            Self::SqliteBinary => "SQLite 二进制备份",
        }
    }
    pub fn is_sql(self) -> bool {
        self == Self::Sql
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BackupExecution {
    MySqlDump,
    PgDump,
    SqlDump,
    SqliteBinary,
}
impl BackupExecution {
    pub fn format(self) -> BackupFormat {
        if self == Self::SqliteBinary {
            BackupFormat::SqliteBinary
        } else {
            BackupFormat::Sql
        }
    }
    pub fn as_record(self) -> &'static str {
        match self {
            Self::MySqlDump => "mysqldump",
            Self::PgDump => "pg_dump",
            Self::SqlDump => "sql_dump",
            Self::SqliteBinary => "sqlite3_backup",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupMethod {
    Auto,
    Native,
    Logical,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BackupObjectKind {
    Table,
    View,
}
impl BackupObjectKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "表",
            Self::View => "视图",
        }
    }
}
/// 备份对象的稳定身份：PG 带 schema，避免跨 schema 同名表被点号拼接后错误拆分。
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BackupObjectRef {
    /// PG schema；MySQL/SQLite 为 None。
    pub schema: Option<String>,
    pub kind: BackupObjectKind,
    pub name: String,
}
impl BackupObjectRef {
    pub fn table(name: impl Into<String>) -> Self {
        Self { schema: None, kind: BackupObjectKind::Table, name: name.into() }
    }
    /// 决策/清单匹配键：PG 非 public 为 `schema.name`，其余为裸名（与逐表分桶器一致）。
    pub fn key(&self) -> String {
        match &self.schema {
            Some(schema) if schema != "public" => format!("{}.{}", schema, self.name),
            _ => self.name.clone(),
        }
    }
}
/// 备份对象范围。`All` 在执行时重新枚举（包含打开弹框后新增的对象），
/// `Objects` 是固定清单；空清单非法，必须在执行前拒绝，不能按整库兜底。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BackupScope {
    /// 执行时枚举全部用户表（`include_views` 时含全部视图）。
    All { include_views: bool },
    /// 固定对象清单（打开弹框时确定）。
    Objects(Vec<BackupObjectRef>),
    /// SQLite 完整一致快照；不支持逐对象选择。
    SqliteSnapshot,
}
#[derive(Clone, Debug, PartialEq)]
pub struct BackupRequest {
    pub config: ConnectionConfig,
    pub database: String,
    pub output: PathBuf,
    pub execution: BackupExecution,
    pub tool: PathBuf,
    pub tool_version: Option<String>,
    pub scope: BackupScope,
    pub include_schema: bool,
    pub include_data: bool,
    pub include_routines: bool,
    pub single_transaction: bool,
    pub lock_tables: bool,
    pub include_owner: bool,
    pub include_acl: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct BackupManifest {
    pub kind: Option<DatabaseKind>,
    pub execution: Option<BackupExecution>,
    pub complete: bool,
    pub include_schema: bool,
    pub include_data: bool,
    pub objects: Vec<String>,
    pub tool_version: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RestoreRequest {
    pub config: ConnectionConfig,
    pub source: PathBuf,
    /// 服务端库名或 SQLite 新文件路径。
    pub target: String,
    pub create_target: bool,
    pub tool: PathBuf,
    pub manifest: Option<BackupManifest>,
    /// 逐表恢复决策（非空 = 逐表模式，放行非空目标并按表过滤重建脚本；空 = 旧整库模式）。
    pub table_decisions: Vec<PerTableDecision>,
    /// 高级执行选项（事务范围、完成验证级别）。执行器按引擎能力兑现，不能兑现时在计划中说明实际范围。
    pub options: RestoreOptions,
}

/// 还原事务范围（设计文档 §6.7）。仅收录执行器能真正兑现的语义，不用一个布尔值概括。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub enum RestoreTransactionMode {
    /// 引擎默认：MySQL 逐语句自动提交且 DDL 隐式提交；PG 由 psql 默认行为决定。
    #[default]
    EngineDefault,
    /// 全任务单事务：失败整体回滚。PG 通过 `psql --single-transaction` 兑现；
    /// MySQL/TiDB 仅在逐表且不含建表/重建（无 DDL 隐式提交）时兑现，否则回退引擎默认并在计划说明。
    SingleTransaction,
}
impl RestoreTransactionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::EngineDefault => "引擎默认",
            Self::SingleTransaction => "单事务（整任务）",
        }
    }
}

/// 完成验证级别（设计文档 §6.7）。基础验证默认开；行数核对更慢但更可信。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub enum RestoreValidation {
    /// 基础：目标可访问 + 预期对象存在性核对。
    #[default]
    Basic,
    /// 行数：在基础之上逐表 `SELECT count(*)` 并回填实际行数（SQLite 快照不适用）。
    RowCount,
}
impl RestoreValidation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Basic => "基础（对象存在）",
            Self::RowCount => "逐表行数",
        }
    }
}

/// 还原高级选项集合。历史记录用 serde default 兼容旧数据。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct RestoreOptions {
    #[serde(default)]
    pub transaction: RestoreTransactionMode,
    #[serde(default)]
    pub validation: RestoreValidation,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestorePlan {
    pub format: BackupFormat,
    pub summary: String,
    pub warnings: Vec<String>,
    pub source_size: u64,
    pub source_modified: Option<std::time::SystemTime>,
    /// 逐表模式下：源表清单与目标存在情况、默认动作，供 UI 回显。
    pub tables: Vec<RestoreTableInfo>,
    /// 是否逐表模式（table_decisions 非空）。
    pub per_table: bool,
}

/// 单表恢复动作（设计文档 §6.4）：新建与重建分离，不把两者都叫“覆盖”。
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RestoreTableAction {
    /// 新建表：目标不存在，按备份结构创建并导入数据。
    Create,
    /// 重建表：DROP 目标后重建再导入；原数据与原定义丢失。
    Recreate,
    /// 清空后导入：保留目标结构，清空全部行后导入。
    TruncateAndLoad,
    /// 追加数据：只导入数据，不改结构（目标需已有同名表）。
    Append,
    /// 不处理该表。
    Skip,
}
impl RestoreTableAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Create => "新建表",
            Self::Recreate => "重建表",
            Self::TruncateAndLoad => "清空后导入",
            Self::Append => "追加数据",
            Self::Skip => "不处理",
        }
    }
    /// 是否会删除目标已有数据/定义（确认页据此要求二次确认）。
    pub fn destroys_target_data(self) -> bool {
        matches!(self, Self::Recreate | Self::TruncateAndLoad)
    }
}

/// 用户对某一源表选定的动作。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerTableDecision {
    pub table: String,
    pub action: RestoreTableAction,
}

/// UI 展示用的单表信息（来源于分桶器 + 目标存在性探测 + 默认动作解析）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreTableInfo {
    pub name: String,
    /// 目标库是否已有同名表。
    pub exists_in_target: bool,
    /// 文件里该表是否有数据语句。
    pub has_data: bool,
    /// 文件里该表是否有 DDL（DROP/CREATE）。
    pub has_ddl: bool,
    /// 默认动作（服务端解析，UI 据此预渲染）。
    pub default_action: RestoreTableAction,
    /// 预检查发现的元数据风险（唯一键冲突可能、外键引用等）；不阻断执行但进入计划与结果。
    pub risks: Vec<String>,
}

/// 对象页探测结果：仅承载事实（存在性 + 备份内容），不做阻断校验、不定动作。
/// 供 UI 在「预检查」之前按存在性/内容给出默认动作与合法候选（设计文档 §6.3/§14.7）；
/// 与最终 `RestoreTableInfo`（含决策校验）分离，探测失败或过期不得当作可执行计划。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreObjectProbe {
    /// 展示名（PG 非 public 为 `schema.name`，其余为裸表名）。
    pub name: String,
    /// 决策匹配键（与备份文件分桶 key 一致）。
    pub key: String,
    /// 目标库是否已有同名对象。
    pub exists_in_target: bool,
    /// 备份是否含可执行结构（DROP/CREATE）。
    pub has_ddl: bool,
    /// 备份是否含数据语句。
    pub has_data: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DatabaseTaskProgress {
    pub stage: String,
    pub message: String,
}
/// 单对象恢复结果状态。“SQL 退出成功”不等于完全一致：Warning 表示导入完成但保留预检查风险。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RestoreObjectStatus {
    Succeeded,
    Warning,
    Failed,
    Skipped,
}
impl RestoreObjectStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "成功",
            Self::Warning => "完成（有警告）",
            Self::Failed => "失败",
            Self::Skipped => "未处理",
        }
    }
}
/// 逐对象结果（设计文档 §15.3）：历史记录用 serde default 字段保存，旧记录无需迁移。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreObjectResult {
    pub name: String,
    #[serde(default)]
    pub action: Option<RestoreTableAction>,
    pub status: RestoreObjectStatus,
    #[serde(default)]
    pub detail: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreOutcome {
    pub verification: String,
    #[serde(default)]
    pub total_objects: usize,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub objects: Vec<RestoreObjectResult>,
}
/// 稳定的数据库备份/恢复能力边界；UI 和工具发现不属于此接口。
pub trait DatabaseBackup: Send + Sync {
    fn kind(&self) -> DatabaseKind;
    fn execution(&self, method: BackupMethod, native_available: bool) -> Result<BackupExecution>;
    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &std::sync::atomic::AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> Result<BackupManifest>;
    fn inspect_restore(
        &self,
        request: &RestoreRequest,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<RestorePlan>;
    /// 对象页进入时的轻量探测：解析备份内容 + 查询目标存在性，返回逐对象事实。
    /// 不做决策阻断校验、不创建库、不选动作；仅供 UI 预渲染默认动作与合法候选。
    fn probe_restore(
        &self,
        request: &RestoreRequest,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<RestoreObjectProbe>>;
    fn restore(
        &self,
        request: &RestoreRequest,
        plan: &RestorePlan,
        cancel: &std::sync::atomic::AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> Result<RestoreOutcome>;
}
