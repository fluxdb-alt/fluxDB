#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ObjectKind {
    Database,
    Schema,
    Table,
    View,
    Column,
    Index,
    Collection,
    RedisDb,
    RedisKey,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObjectPath {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: ObjectKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObjectSummary {
    pub path: ObjectPath,
    pub rows: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryRequest {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// 显式 schema 作用域（PG 用，控制 `search_path` / 对象解析）；其它方言忽略。
    pub schema: Option<String>,
    pub text: String,
    pub mode: QueryMode,
    pub options: QueryExecutionOptions,
    /// 可选复用会话的标识；`None` 时用隔离短连接执行。
    pub session_id: Option<QuerySessionId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryExecutionOptions {
    pub continue_on_error: bool,
    pub split_statements: bool,
    pub page_offset: u64,
    pub page_size: u64,
}

impl Default for QueryExecutionOptions {
    fn default() -> Self {
        Self {
            continue_on_error: true,
            split_statements: true,
            page_offset: 0,
            page_size: Pagination::DEFAULT_LIMIT,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryExecutionResult {
    pub summaries: Vec<QueryExecutionSummary>,
    pub results: Vec<DataPage>,
    pub rollback_snapshots: Vec<Option<QueryRollbackSnapshot>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum QueryRollbackSnapshot {
    Insert(QueryInsertRollbackSnapshot),
    Update(QueryUpdateRollbackSnapshot),
    Delete(QueryDeleteRollbackSnapshot),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryInsertRollbackSnapshot {
    pub table: String,
    pub identities: Vec<RowIdentity>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryUpdateRollbackSnapshot {
    pub table: String,
    pub columns: Vec<Column>,
    pub changed_columns: Vec<String>,
    pub rows: Vec<QueryRollbackRowSnapshot>,
    pub fallback_where: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryDeleteRollbackSnapshot {
    pub table: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryRollbackRowSnapshot {
    pub identity: RowIdentity,
    pub values: BTreeMap<String, CellValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryExecutionSummary {
    pub sql: String,
    pub kind: QueryStatementKind,
    pub success: bool,
    pub message: String,
    pub returned_rows: u64,
    pub affected_rows: u64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: u64,
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// schema 作用域（PG）；旧记录缺省为 None。附带 `#[serde(default)]` 兼容加载。
    #[serde(default)]
    pub schema: Option<String>,
    pub name: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryStatementKind {
    ResultSet,
    Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryMode {
    All,
    CurrentStatement,
    Selection,
}

/// 补全索引快照结构版本。变更快照结构（字段含义/新增集合）时必须递增：
/// 旧缓存因版本不匹配被安全拒绝并重建，但连接、查询与历史不受影响（§8.4）。
/// 3：快照新增 routines（含签名）/triggers，用于函数重载索引与触发器持久化。
pub const COMPLETION_INDEX_VERSION: u32 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryCompletionResult {
    pub replace_start: usize,
    pub replace_end: usize,
    pub items: Vec<QueryCompletionItem>,
}

/// 补全项插入格式：PlainText 原样插入，Snippet 解析占位符（`$1`、`${1:placeholder}`）。
///
/// 默认 PlainText 保持向后兼容——SQL 候选中的 `$1`、`${name}` 等文本是实际 SQL 内容，
/// 只有显式声明 Snippet 格式才解析。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InsertTextFormat {
    #[default]
    PlainText,
    Snippet,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryCompletionItem {
    pub label: String,
    pub insert_text: String,
    pub kind: QueryCompletionKind,
    pub detail: Option<String>,
    /// 文档说明（悬浮/选中项底部面板展示）。P1.4 新增。
    pub documentation: Option<String>,
    /// 过滤文本：用于候选匹配，缺省时回退到 label。
    pub filter_text: Option<String>,
    /// 排序文本：用于候选排序，缺省时回退到 label。
    pub sort_text: Option<String>,
    /// 插入格式：默认 PlainText；Snippet 时 editor-core 解析占位符并建立 tabstop 会话。
    pub insert_text_format: InsertTextFormat,
}

impl Default for QueryCompletionItem {
    fn default() -> Self {
        Self {
            label: String::new(),
            insert_text: String::new(),
            kind: QueryCompletionKind::Keyword,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
            insert_text_format: InsertTextFormat::PlainText,
        }
    }
}

impl QueryCompletionItem {
    /// 构造补全项，未指定的字段取默认值（insert_text_format 默认 PlainText）。
    pub fn new(label: impl Into<String>, insert_text: impl Into<String>, kind: QueryCompletionKind) -> Self {
        Self {
            label: label.into(),
            insert_text: insert_text.into(),
            kind,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum QueryCompletionKind {
    Keyword,
    Snippet,
    Schema,
    Table,
    View,
    Column,
    Function,
    Procedure,
    Trigger,
    /// Redis 顶层命令。
    RedisCommand,
    /// Redis 树状命令的子命令。
    RedisSubCommand,
    /// Redis 命令参数占位 / 固定 token。
    RedisArgument,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionTable {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: ObjectKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionColumn {
    /// 所属 database（PG 为物理连接库，MySQL/TiDB 为库名，Redis 无）。
    pub database: Option<String>,
    /// 所属 schema（PG 必备；MySQL/TiDB 为 None）。
    pub schema: Option<String>,
    /// 所属表名（不拼点号；裸 table 匹配只在本结构内，跨 schema 同名表靠 database+schema 区分）。
    pub table: String,
    pub name: String,
    pub type_name: Option<String>,
    pub nullable: bool,
    pub primary_key: bool,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionRoutine {
    pub schema: Option<String>,
    pub name: String,
    pub kind: CompletionRoutineKind,
    /// 签名 / identity arguments（PG `pg_get_function_identity_arguments`）。
    /// 用于区分同 schema 同名重载：同名不同签名是不同候选，不能合并（§8.4）；MySQL 为 None。
    pub signature: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CompletionRoutineKind {
    Function,
    Procedure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionTrigger {
    pub schema: Option<String>,
    pub name: String,
    pub table: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TableRef {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: ObjectKind,
    pub rows: Option<u64>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ColumnRef {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub table: String,
    pub column: String,
    pub type_name: Option<String>,
    pub nullable: bool,
    pub primary_key: bool,
    pub ordinal_position: Option<u32>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutineRef {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: CompletionRoutineKind,
    /// 签名 / identity arguments（PG 的 `pg_get_function_identity_arguments`），
    /// 用于区分同 schema 同名重载；MySQL 可空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TriggerRef {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub table: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompletionIndexMeta {
    pub app_index_version: u32,
    pub db_kind: DatabaseKind,
    pub last_indexed_at: u64,
    pub last_verified_at: u64,
    pub ttl_seconds: u64,
    pub dirty: bool,
    pub table_count: usize,
    pub table_fingerprints: Vec<TableFingerprint>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TableFingerprint {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub table: String,
    pub fingerprint: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompletionIndexSnapshot {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub tables: Vec<TableRef>,
    pub columns: Vec<ColumnRef>,
    pub routines: Vec<RoutineRef>,
    pub triggers: Vec<TriggerRef>,
    pub meta: CompletionIndexMeta,
}
