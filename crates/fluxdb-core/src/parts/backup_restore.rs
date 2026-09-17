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
#[derive(Clone, Debug, PartialEq)]
pub struct BackupRequest {
    pub config: ConnectionConfig,
    pub database: String,
    pub output: PathBuf,
    pub execution: BackupExecution,
    pub tool: PathBuf,
    pub tool_version: Option<String>,
    pub tables: Vec<String>,
    pub include_views: bool,
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

/// 单表恢复动作。
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum RestoreTableAction {
    /// 覆盖：DROP 后重建再导入（目标有同名表时会清掉重来）。
    Overwrite,
    /// 追加：只导入数据，不改结构（目标需已有同名表）。
    Append,
    /// 跳过：不处理该表。
    Skip,
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
}
#[derive(Clone, Debug, PartialEq)]
pub struct DatabaseTaskProgress {
    pub stage: String,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RestoreOutcome {
    pub verification: String,
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
    fn restore(
        &self,
        request: &RestoreRequest,
        plan: &RestorePlan,
        cancel: &std::sync::atomic::AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> Result<RestoreOutcome>;
}
