#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ObjectKind {
    Database,
    Schema,
    Table,
    View,
    Procedure,
    Function,
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
    /// 表/物化视图占用空间（数据 + 索引，字节），供对象树快速识别大表。
    ///
    /// 取不到（视图、无权限、SQLite 等）或数据库只能给统计估算时按实际能力返回；
    /// MySQL/TiDB 来自 `information_schema.tables` 统计值，PostgreSQL 为 catalog
    /// `relpages` 估算，均可能滞后于真实占用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    pub comment: Option<String>,
    /// 数据库稳定对象标识：PG 为 `pg_class.oid`（表/视图）。MySQL/SQLite 无公开稳定号，
    /// 保持 `None`（不伪造）。用于 ER 结构刷新重绑：改名但同对象可据此自动接回关系。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable: Option<u64>,
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
    /// 只读执行（AI/受限只读入口）。置位时驱动以数据库级只读方式执行
    /// （PG 只读事务、MySQL `SET SESSION TRANSACTION READ ONLY`、SQLite `PRAGMA query_only`、
    /// Oracle `SET TRANSACTION READ ONLY` + 执行前拒绝非查询语句），
    /// 作为客户端 AST 策略之外的纵深防线：即使某个未在 denylist 里的函数/语法带副作用，
    /// 服务端也会拒绝写入。默认 `false`，普通编辑器路径不受影响。
    pub read_only: bool,
    /// 只取执行计划、不执行语句（AI `db.explain`）。
    ///
    /// 目前只有 SQL Server 需要：它没有 `EXPLAIN` 前缀，计划由
    /// `SET SHOWPLAN_ALL ON` 会话开关产生。其它方言继续用文本包装，
    /// 该标志保持 `false`。
    pub explain_only: bool,
}

impl Default for QueryExecutionOptions {
    fn default() -> Self {
        Self {
            continue_on_error: true,
            split_statements: true,
            page_offset: 0,
            page_size: Pagination::DEFAULT_LIMIT,
            read_only: false,
            explain_only: false,
        }
    }
}

/// 数据提交的落定结果（§8.4/R11）。
///
/// 目前只承载 INSERT 的真实身份：自增列/序列/默认值生成的主键无法从编辑输入得知，
/// 只能由服务端 `RETURNING` 返回，否则补偿 SQL 会定位到错误的行。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppliedChangeOutcome {
    /// 按 `DataChangeSet.inserts` 顺序给出每行插入的真实身份；无法取回时为空。
    pub inserted_identities: Vec<RowIdentity>,
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
    /// 生成该补偿 SQL 时使用的方言（§8.4）。旧记录缺省为 None，按 MySQL 兼容渲染，
    /// 保证既有 MySQL 历史可读可用；新记录写入真实方言，PG 走双引号标识符与类型化字面量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_kind: Option<DatabaseKind>,
    pub table: String,
    pub identities: Vec<RowIdentity>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryUpdateRollbackSnapshot {
    /// 生成该补偿 SQL 时使用的方言（§8.4）。旧记录缺省为 None，按 MySQL 兼容渲染，
    /// 保证既有 MySQL 历史可读可用；新记录写入真实方言，PG 走双引号标识符与类型化字面量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_kind: Option<DatabaseKind>,
    pub table: String,
    pub columns: Vec<Column>,
    pub changed_columns: Vec<String>,
    pub rows: Vec<QueryRollbackRowSnapshot>,
    pub fallback_where: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryDeleteRollbackSnapshot {
    /// 生成该补偿 SQL 时使用的方言（§8.4）。旧记录缺省为 None，按 MySQL 兼容渲染，
    /// 保证既有 MySQL 历史可读可用；新记录写入真实方言，PG 走双引号标识符与类型化字面量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_kind: Option<DatabaseKind>,
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

/// 精确的 SQL 参数值（设计 §20.2 的 tagged wire 值；**绝不**做字符串插值）。
///
/// i64/u64/decimal 用十进制文本保存，避免经过 f64 丢精度；bytes 已由应用层做 base64 解码与体积校验。
#[derive(Clone, Debug, PartialEq)]
pub enum SqlParameter {
    Null,
    Bool(bool),
    /// i64：精确十进制文本。
    I64(String),
    /// u64：精确十进制文本。
    U64(String),
    /// decimal：精确文本（尺度与范围由应用层校验）。
    Decimal(String),
    /// 浮点：仅当调用方明确用 f64 时才走这里；必须 finite。
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
}

/// 参数化执行请求：在既有 `QueryRequest` 之上附加精确参数。
///
/// 独立类型是为了不改动既有 `QueryRequest` 的所有构造点；连接器对它必须**真 bind**，
/// 不支持就明确报错（默认实现即如此），不允许退化成把参数拼进 SQL。
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterizedQueryRequest {
    pub request: QueryRequest,
    pub parameters: Vec<SqlParameter>,
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
/// 4：表指纹改为十六进制字符串落盘（u64 超出 TOML 的 i64 整数上限，旧格式会整份写不出）。
/// 5：达梦过滤 `SREF_CON_*` 等内部表；旧快照可能仍含这些不可引用对象。
pub const COMPLETION_INDEX_VERSION: u32 = 5;

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
    /// 候选对象所属 schema（PG=schema；MySQL/SQLite 通常为 None）。
    ///
    /// 取值是**补全时使用的 schema 作用域**（即 `completion_namespace_scope` 的结果），
    /// 与被写入 CompletionIndex 的桶键一致，因此「候选身份」可直接当作索引查询键：
    /// 详情面板据此按 (库, schema, 表) 命中列的完整身份，避免同名跨 schema 取错对象。
    pub schema: Option<String>,
    /// 插入时是否按方言补标识符引号。CTE / 派生表推导出的名字须按原样插入，
    /// 否则 Oracle 中 `"item_total"` 与未加引号的 `ITEM_TOTAL` 不是同一标识符。
    pub quote_identifier: bool,
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
            schema: None,
            quote_identifier: true,
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
    /// 表/视图注释（PG `obj_description`），用于补全项的文档提示；无注释为 None。
    pub comment: Option<String>,
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
    /// ER 重绑使用的列身份：PostgreSQL 为 attnum，其他方言不伪造。
    pub stable: Option<u64>,
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
    /// 表结构指纹（64 位哈希，用于快照是否变化的比较）。
    ///
    /// 以十六进制字符串落盘：TOML 整数上限是 i64，直接写 u64 时只要哈希最高位为 1
    /// 就会让整份快照序列化失败（且调用方吞掉错误 → 索引静默不落盘）。
    #[serde(with = "u64_hex")]
    pub fingerprint: u64,
}

/// `u64` 的十六进制字符串序列化（TOML 整数上限 i64，64 位哈希必须走字符串）。
mod u64_hex {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{value:016x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let text = String::deserialize(deserializer)?;
        u64::from_str_radix(&text, 16).map_err(serde::de::Error::custom)
    }
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
