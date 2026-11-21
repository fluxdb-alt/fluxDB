//! 数据同步 / 结构同步的共享模型。
//!
//! 只放跨步骤使用的数据契约：端点、选项、预览与报告。差异算法在
//! [`super::data_diff`]，读写编排在 [`super::data_sync`]。
//!
//! 约定：预览只保存计数与少量样本，绝不把整表行留在内存或 UI 状态里；
//! 执行前会重新比对，避免用过期快照写目标库。

use std::collections::BTreeMap;

use fluxdb_core::{CellValue, ConnectionId, DatabaseKind, Error, ErrorKind};

/// 同步的一端：连接 + 库 + 可选 schema。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncEndpoint {
    pub connection_id: ConnectionId,
    pub database: String,
    pub schema: Option<String>,
}

impl SyncEndpoint {
    pub fn new(
        connection_id: ConnectionId,
        database: impl Into<String>,
        schema: Option<String>,
    ) -> Self {
        Self {
            connection_id,
            database: database.into(),
            schema,
        }
    }

    /// 库名是端点身份的一部分，空值必须显式报错，不能退化成「当前库」。
    pub(crate) fn database(&self) -> fluxdb_core::Result<&str> {
        let database = self.database.trim();
        if database.is_empty() {
            return Err(Error::new(ErrorKind::Query, "请选择数据库"));
        }
        Ok(database)
    }

    pub(crate) fn schema(&self) -> Option<&str> {
        self.schema
            .as_deref()
            .map(str::trim)
            .filter(|schema| !schema.is_empty())
    }

    /// 由端点推导目标对象路径。目标端显式库/schema 优先；缺省时沿用源端路径。
    ///
    /// 目标方言不使用 schema 时（MySQL 家族 / SQLite）必须丢弃源端 schema，
    /// 否则会拼出 `public.\`table\`` 这类目标库不认的限定名。
    pub(crate) fn object(
        &self,
        target_kind: DatabaseKind,
        source: &fluxdb_core::ObjectPath,
    ) -> fluxdb_core::ObjectPath {
        let schema = if target_kind.uses_schemas() {
            self.schema()
                .map(str::to_string)
                .or_else(|| source.schema.clone())
        } else {
            None
        };
        fluxdb_core::ObjectPath {
            connection_id: self.connection_id,
            database: Some(self.database.trim().to_string()),
            schema,
            name: source.name.clone(),
            kind: fluxdb_core::ObjectKind::Table,
        }
    }

    /// 由用户指定目标表配对键推导目标对象路径（`schema.表` 或 `表`）。
    ///
    /// 目标方言不使用 schema 时忽略键里的 schema 部分；键里没有 schema 时
    /// 回落到端点自身 schema，与 [`Self::object`] 保持一致。
    pub(crate) fn object_for_key(
        &self,
        target_kind: DatabaseKind,
        key: &str,
    ) -> fluxdb_core::ObjectPath {
        let key = key.trim();
        let (key_schema, name) = match key.rsplit_once('.') {
            Some((schema, name)) if !schema.trim().is_empty() => (Some(schema.trim()), name.trim()),
            _ => (None, key),
        };
        let schema = if target_kind.uses_schemas() {
            key_schema
                .map(str::to_string)
                .or_else(|| self.schema().map(str::to_string))
        } else {
            None
        };
        fluxdb_core::ObjectPath {
            connection_id: self.connection_id,
            database: Some(self.database.trim().to_string()),
            schema,
            name: name.to_string(),
            kind: fluxdb_core::ObjectKind::Table,
        }
    }
}

/// 每批写入行数。太小 = 往返过多，太大 = 单事务过大、失败回滚成本高。
pub const DEFAULT_BATCH_ROWS: usize = 500;
pub const MAX_BATCH_ROWS: usize = 5_000;

/// 单行写入失败（约束冲突、类型不合法等）时的处理策略。
///
/// 默认跳过：批量写入里一行坏数据不应让整张表停下来，失败行记入报告与日志。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RowErrorPolicy {
    /// 跳过失败行并记录，继续同步。
    #[default]
    Skip,
    /// 立即停止当前表，错误按表级失败返回。
    Stop,
}

/// 数据同步选项。
///
/// `delete` 与 `overwrite` 默认关闭：两者都会删除目标端数据，必须由用户显式勾选。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataSyncOptions {
    /// 目标缺失的行执行 INSERT。
    pub insert: bool,
    /// 主键相同但内容不同时执行 UPDATE。
    pub update: bool,
    /// 目标多出的行执行 DELETE。
    pub delete: bool,
    /// 先清空目标表再全量写入（覆盖模式）。与 `delete` 互斥使用。
    pub overwrite: bool,
    /// 目标表不存在时按源端 DDL 建表。
    pub create_missing: bool,
    /// 每批写入行数，写入前统一夹到 `1..=MAX_BATCH_ROWS`。
    pub batch_rows: usize,
    /// 自定义比对键；为空时使用目标表主键。
    pub key_columns: Vec<String>,
    /// 按表覆盖的比对键：表配对键 → 键列；命中时优先于 `key_columns`。
    pub table_key_columns: BTreeMap<String, Vec<String>>,
    /// 跳过的字段名（两端同名匹配，忽略大小写）。跳过的字段不参与比对与写入。
    pub skip_columns: Vec<String>,
    /// 按表覆盖的跳过字段：表配对键 → 字段名；命中时优先于 `skip_columns`。
    pub table_skip_columns: BTreeMap<String, Vec<String>>,
    /// 源字段 → 目标字段的显式映射，对所有表生效；键不区分大小写。
    pub column_map: BTreeMap<String, String>,
    /// 按表覆盖的字段映射：表配对键 → (源字段 → 目标字段)，优先于 `column_map`。
    pub table_column_map: BTreeMap<String, BTreeMap<String, String>>,
    /// 用户指定的表配对：源表配对键 → 目标表配对键。
    ///
    /// 命中时不再按名称自动配对；目标表不存在时视为缺表（按 `create_missing`
    /// 决定是否在指定名称下建表）。
    pub table_map: BTreeMap<String, String>,
    /// 单行写入失败时的处理策略；默认跳过并记录。
    pub row_error_policy: RowErrorPolicy,
    /// 目标端唯一键冲突时的处理方式；只改 INSERT 语句，默认按普通写入失败处理。
    pub insert_conflict: fluxdb_core::InsertConflictMode,
    /// 覆盖模式下把清空与全部写入放进同一事务：任一批失败则整表回滚。
    ///
    /// 仅覆盖模式支持；增量模式的更新/删除由比对批次各自提交，无法并进同一事务。
    pub atomic_table: bool,
    /// 「导出迁移 SQL」目标文件；`None` 表示不导出。
    ///
    /// 同步照常写目标库，同时把**已执行成功**的修改语句逐条追加到该文件。
    /// 与表级事务互斥：事务回滚后脚本里的语句并不对应最终目标数据。
    pub export_sql_path: Option<std::path::PathBuf>,
    /// 从指定执行记录续传：跳过上次已完成的表，未完成的表从最后一个完整块继续。
    ///
    /// 仅支持增量模式；端点、表清单与其余选项必须与上次一致（签名校验）。
    pub resume_from_run: Option<i64>,
}

impl Default for DataSyncOptions {
    fn default() -> Self {
        Self {
            insert: true,
            update: true,
            delete: false,
            overwrite: false,
            create_missing: false,
            batch_rows: DEFAULT_BATCH_ROWS,
            key_columns: Vec::new(),
            table_key_columns: BTreeMap::new(),
            skip_columns: Vec::new(),
            table_skip_columns: BTreeMap::new(),
            column_map: BTreeMap::new(),
            table_column_map: BTreeMap::new(),
            table_map: BTreeMap::new(),
            row_error_policy: RowErrorPolicy::default(),
            insert_conflict: fluxdb_core::InsertConflictMode::default(),
            atomic_table: false,
            export_sql_path: None,
            resume_from_run: None,
        }
    }
}

impl DataSyncOptions {
    pub fn batch_rows(&self) -> usize {
        self.batch_rows.clamp(1, MAX_BATCH_ROWS)
    }

    /// 指定表的比对键：表级优先，未单独设置时沿用全局 `key_columns`。
    ///
    /// 两者都为空时由比对阶段回退到目标表主键。
    pub fn key_columns_for(&self, table: &str) -> &[String] {
        self.table_key_columns
            .get(table)
            .map(Vec::as_slice)
            .unwrap_or(&self.key_columns)
    }

    /// 指定表的跳过字段：表级优先，未单独设置时沿用全局 `skip_columns`。
    pub fn skip_columns_for(&self, table: &str) -> &[String] {
        self.table_skip_columns
            .get(table)
            .map(Vec::as_slice)
            .unwrap_or(&self.skip_columns)
    }

    /// 合并全局字段映射与指定表的表级映射，表级优先。
    ///
    /// 键不区分大小写：表级键若与全局键仅大小写不同，先移除全局项再插入，
    /// 避免 `mapped_name` 在两张表里命中不同的那条。
    pub fn column_map_for(&self, table: &str) -> BTreeMap<String, String> {
        let mut merged = self.column_map.clone();
        if let Some(map) = self.table_column_map.get(table) {
            for (source, target) in map {
                if let Some(existing) = merged
                    .keys()
                    .find(|key| key.eq_ignore_ascii_case(source))
                    .cloned()
                {
                    merged.remove(&existing);
                }
                merged.insert(source.clone(), target.clone());
            }
        }
        merged
    }

    /// 是否至少启用了一种写动作。全关时同步没有意义，调用方应直接拒绝。
    pub fn writes_anything(&self) -> bool {
        self.overwrite || self.insert || self.update || self.delete
    }
}

/// 一次数据同步请求。`tables` 为空表示「源端全部表」。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataSyncRequest {
    pub source: SyncEndpoint,
    pub target: SyncEndpoint,
    pub tables: Vec<String>,
    pub options: DataSyncOptions,
}

/// 表清单项：跨连接配对键 + 展示名。第一步选择同步对象用。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncTableInfo {
    /// `schema.name` 或 `name`。
    pub key: String,
    pub name: String,
    pub schema: Option<String>,
}

/// 单表比对结果状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableDiffState {
    /// 两端都有数据差异，可同步。
    Different,
    /// 两端完全一致。
    Equal,
    /// 无法比对，原因见 `skip`。
    Skipped,
}

/// 跳过比对的原因。必须原样展示给用户，不允许静默跳过。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableSkipReason {
    TargetMissing,
    SourceMissing,
    /// 目标表没有主键，且用户没有指定比对键。
    NoKey,
    /// 指定或推导出的键列在某一端不存在。
    KeyColumnMissing,
    /// 两端没有任何同名字段，无法安全比对。
    NoSharedColumns,
    /// 行里含未完整读取的值（大字段/二进制摘要），比对会误判。
    UnsupportedValue,
}

impl TableSkipReason {
    pub fn message(self) -> &'static str {
        match self {
            Self::TargetMissing => "目标端不存在该表",
            Self::SourceMissing => "源端不存在该表",
            Self::NoKey => "没有主键，请指定比对键",
            Self::KeyColumnMissing => "比对键在某一端不存在",
            Self::NoSharedColumns => "两端没有同名字段",
            Self::UnsupportedValue => "含未完整读取的大字段，无法安全比对",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowDiffAction {
    Insert,
    Update,
    Delete,
}

/// 预览样本：保留键值展示文本，更新行额外保留变更字段的新旧展示值。
///
/// 只有变更字段的展示文本会外泄到界面，原始行数据不随预览保存。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowDiffSample {
    pub action: RowDiffAction,
    /// `(键列名, 展示值)`，顺序与 `TableDataDiff::key_columns` 一致。
    pub key_values: Vec<(String, String)>,
    /// 变更字段名；新增/删除按整行处理，此时为 `None`。
    pub column: Option<String>,
    /// 该字段在源端的展示值；整行样本或无法读取目标行时为 `None`。
    pub source_value: Option<String>,
    /// 该字段在目标端的展示值；整行样本或无法读取目标行时为 `None`。
    pub target_value: Option<String>,
}

/// 单表数据差异汇总。分页预览时这里只表示「一页」的结果。
#[derive(Clone, Debug, PartialEq)]
pub struct TableDataDiff {
    /// `schema.name` 或 `name`，用于跨连接配对。
    pub key: String,
    pub name: String,
    pub schema: Option<String>,
    pub state: TableDiffState,
    pub skip: Option<TableSkipReason>,
    /// 比对该表时的具体错误（读取失败、键不唯一等）。与 `skip` 互斥使用。
    pub error: Option<String>,
    pub inserted: u64,
    pub updated: u64,
    pub deleted: u64,
    pub unchanged: u64,
    pub key_columns: Vec<String>,
    /// 仅源端存在的字段：同步时不写入，但结构不一致需要提示。
    pub columns_only_in_source: Vec<String>,
    /// 仅目标端存在的字段：同步时不覆盖，保留目标端现有值/默认值。
    pub columns_only_in_target: Vec<String>,
    /// 参与同步的字段总数（键列 + 比对列）。样本里「全部 N 列」按它显示。
    pub column_count: usize,
    /// 该表是否已比对完全部数据；`false` 表示后面还有未加载的差异。
    pub page_done: bool,
    /// 下一页的键上界（比对键第一列的原始值）；`page_done` 为真时无意义。
    pub cursor: Option<CellValue>,
    /// 分块行数（nibble 自适应后的结果），下一页沿用它。
    pub chunk_rows: usize,
    pub samples: Vec<RowDiffSample>,
}

impl TableDataDiff {
    pub(crate) fn skipped(
        key: String,
        name: String,
        schema: Option<String>,
        reason: TableSkipReason,
    ) -> Self {
        Self {
            key,
            name,
            schema,
            state: TableDiffState::Skipped,
            skip: Some(reason),
            error: None,
            inserted: 0,
            updated: 0,
            deleted: 0,
            unchanged: 0,
            key_columns: Vec::new(),
            columns_only_in_source: Vec::new(),
            columns_only_in_target: Vec::new(),
            column_count: 0,
            page_done: true,
            cursor: None,
            chunk_rows: 0,
            samples: Vec::new(),
        }
    }

    /// 可比对结果项：计数由比对器填入，`skip` 为空。
    pub(crate) fn ready(key: String, name: String, schema: Option<String>) -> Self {
        let mut table = Self::skipped(key, name, schema, TableSkipReason::NoKey);
        table.skip = None;
        table
    }

    pub(crate) fn failed(
        key: String,
        name: String,
        schema: Option<String>,
        message: String,
    ) -> Self {
        let mut table = Self::ready(key, name, schema);
        table.state = TableDiffState::Skipped;
        table.error = Some(message);
        table
    }

    /// 需要写入目标库的行数（不含覆盖模式的整表重写）。
    pub fn changed_rows(&self) -> u64 {
        self.inserted + self.updated + self.deleted
    }

    /// 已参与比对的行数（目标端独有的删除行不计）。
    pub fn compared_rows(&self) -> u64 {
        self.inserted + self.updated + self.unchanged
    }

    /// 合并下一页结果：计数累加，样本 / 游标 / 块大小取新页，状态保持「有差异优先」。
    pub fn absorb_page(&mut self, page: TableDataDiff) {
        self.inserted += page.inserted;
        self.updated += page.updated;
        self.deleted += page.deleted;
        self.unchanged += page.unchanged;
        if page.state == TableDiffState::Different {
            self.state = TableDiffState::Different;
        } else if self.state != TableDiffState::Different {
            self.state = page.state;
        }
        self.samples = page.samples;
        self.page_done = page.page_done;
        self.cursor = page.cursor;
        self.chunk_rows = page.chunk_rows;
        self.key_columns = page.key_columns;
        self.columns_only_in_source = page.columns_only_in_source;
        self.columns_only_in_target = page.columns_only_in_target;
        self.column_count = page.column_count;
        if self.error.is_none() {
            self.error = page.error;
        }
    }
}

/// 数据同步预览结果。
#[derive(Clone, Debug, PartialEq)]
pub struct DataSyncPreview {
    pub source_kind: DatabaseKind,
    pub target_kind: DatabaseKind,
    pub tables: Vec<TableDataDiff>,
}

impl DataSyncPreview {
    pub fn total_inserted(&self) -> u64 {
        self.tables.iter().map(|table| table.inserted).sum()
    }

    pub fn total_updated(&self) -> u64 {
        self.tables.iter().map(|table| table.updated).sum()
    }

    pub fn total_deleted(&self) -> u64 {
        self.tables.iter().map(|table| table.deleted).sum()
    }

    pub fn total_changed(&self) -> u64 {
        self.tables.iter().map(TableDataDiff::changed_rows).sum()
    }
}

// ---------------------------------------------------------------------------
// 结构同步
// ---------------------------------------------------------------------------

/// 结构同步选项。所有「删除目标端对象」的动作默认关闭。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureSyncOptions {
    /// 源端有、目标端缺的表：按源端 DDL 建表。
    pub create_table: bool,
    /// 目标端有、源端缺的表：删表。
    pub drop_table: bool,
    pub add_column: bool,
    pub drop_column: bool,
    /// 类型、可空、默认值、注释、生成列差异，以及主键变化。
    pub alter_column: bool,
    pub add_index: bool,
    pub drop_index: bool,
    pub add_foreign_key: bool,
    pub drop_foreign_key: bool,
    /// 源端有、目标端缺的 CHECK 约束。
    pub add_check: bool,
    /// 目标端有、源端缺的 CHECK 约束。
    pub drop_check: bool,
    /// 自定义目标列类型：表配对键 → (列名 → 目标类型文本)。
    ///
    /// 只在跨类型生成路径生效；命中时用用户给的文本替换自动映射结果，并记录提示。
    pub type_overrides: BTreeMap<String, BTreeMap<String, String>>,
    /// 用户指定的表配对：源表配对键 → 目标表配对键；命中时不做名称自动配对。
    pub table_map: BTreeMap<String, String>,
}

impl Default for StructureSyncOptions {
    fn default() -> Self {
        Self {
            create_table: true,
            drop_table: false,
            add_column: true,
            drop_column: false,
            alter_column: true,
            add_index: true,
            drop_index: false,
            add_foreign_key: true,
            drop_foreign_key: false,
            add_check: true,
            drop_check: false,
            type_overrides: BTreeMap::new(),
            table_map: BTreeMap::new(),
        }
    }
}

/// 结构对象种类。用于界面分组与选项门控。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum StructureObject {
    Table,
    Column,
    PrimaryKey,
    Index,
    ForeignKey,
    Check,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructureChangeAction {
    Create,
    Alter,
    Drop,
}

/// 列定义快照（不含连接信息），同时供差异展示与 ALTER 语句生成。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    /// 完整类型文本，如 `varchar(100)` / `numeric(10,2)`。
    pub type_text: String,
    pub nullable: bool,
    pub primary_key: bool,
    /// 自增 / identity 列。跨方言建表时生成目标端身份语法；同方言同步沿用源端 DDL。
    pub auto_increment: bool,
    pub default_expr: Option<String>,
    /// 生成列定义；`None` 表示普通列。生成列的值由数据库计算，不接受显式写入。
    pub generated: Option<fluxdb_core::GeneratedColumn>,
    pub comment: Option<String>,
}

/// 索引定义快照。主键索引也用它表示（`primary = true`）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexDef {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    /// 方法/类型：MySQL `BTREE`/`FULLTEXT`/`SPATIAL`、PG `btree`/`gin`。
    /// `None` 表示方言未读取或源端是跨类型映射，比对方跳过该项。
    pub index_type: Option<String>,
    /// 部分索引谓词（PG）；有值时只覆盖满足条件的行。
    pub predicate: Option<String>,
    /// 键项含表达式。此类索引无法按列名安全重建，结构同步跳过并提示。
    pub has_expression: bool,
}

/// 外键定义快照。复合外键按列序配对。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignKeyDef {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    /// 引用动作（`CASCADE` / `SET NULL` / `RESTRICT` / `NO ACTION` / `SET DEFAULT`）。
    /// `None` 表示方言未读取；两侧都读到才比较。
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

/// CHECK 约束快照。表达式保留目录原文（方言函数可能不可跨库翻译）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckDef {
    pub name: String,
    pub expression: String,
}

/// 单条结构差异的载荷。`before`/`after` 为空表示该侧不存在该对象。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StructureChangeDetail {
    Table,
    Column {
        before: Option<ColumnDef>,
        after: Option<ColumnDef>,
    },
    PrimaryKey {
        before: Option<IndexDef>,
        after: Option<IndexDef>,
    },
    Index {
        before: Option<IndexDef>,
        after: Option<IndexDef>,
    },
    ForeignKey {
        before: Option<ForeignKeyDef>,
        after: Option<ForeignKeyDef>,
    },
    Check {
        before: Option<CheckDef>,
        after: Option<CheckDef>,
    },
}

/// 一条结构差异。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureChange {
    pub object: StructureObject,
    pub action: StructureChangeAction,
    pub name: String,
    pub detail: StructureChangeDetail,
}

impl StructureChange {
    /// 变更前展示文本；新建对象返回 `None`。
    pub fn before_text(&self) -> Option<String> {
        match &self.detail {
            StructureChangeDetail::Table => None,
            StructureChangeDetail::Column { before, .. } => before.as_ref().map(column_text),
            StructureChangeDetail::PrimaryKey { before, .. } => before
                .as_ref()
                .map(|index| index_text(index, "PRIMARY KEY")),
            StructureChangeDetail::Index { before, .. } => before.as_ref().map(|index| {
                index_text(
                    index,
                    if index.unique {
                        "UNIQUE INDEX"
                    } else {
                        "INDEX"
                    },
                )
            }),
            StructureChangeDetail::ForeignKey { before, .. } => {
                before.as_ref().map(foreign_key_text)
            }
            StructureChangeDetail::Check { before, .. } => before.as_ref().map(check_text),
        }
    }

    /// 变更后展示文本；删除对象返回 `None`。
    pub fn after_text(&self) -> Option<String> {
        match &self.detail {
            StructureChangeDetail::Table => None,
            StructureChangeDetail::Column { after, .. } => after.as_ref().map(column_text),
            StructureChangeDetail::PrimaryKey { after, .. } => {
                after.as_ref().map(|index| index_text(index, "PRIMARY KEY"))
            }
            StructureChangeDetail::Index { after, .. } => after.as_ref().map(|index| {
                index_text(
                    index,
                    if index.unique {
                        "UNIQUE INDEX"
                    } else {
                        "INDEX"
                    },
                )
            }),
            StructureChangeDetail::ForeignKey { after, .. } => after.as_ref().map(foreign_key_text),
            StructureChangeDetail::Check { after, .. } => after.as_ref().map(check_text),
        }
    }
}

fn column_text(column: &ColumnDef) -> String {
    let mut text = format!("{} {}", column.name, column.type_text);
    if !column.nullable {
        text.push_str(" NOT NULL");
    }
    if let Some(generated) = &column.generated {
        text.push_str(&format!(" GENERATED ALWAYS AS ({})", generated.expression));
        if generated.stored == Some(true) {
            text.push_str(" STORED");
        }
    }
    if let Some(default) = &column.default_expr {
        text.push_str(&format!(" DEFAULT {default}"));
    }
    if let Some(comment) = &column.comment {
        text.push_str(&format!(" COMMENT {comment}"));
    }
    text
}

fn index_text(index: &IndexDef, kind: &str) -> String {
    let mut text = format!("{kind} {} ({})", index.name, index.columns.join(", "));
    if let Some(index_type) = &index.index_type {
        text.push_str(&format!(" USING {index_type}"));
    }
    if let Some(predicate) = &index.predicate {
        text.push_str(&format!(" WHERE {predicate}"));
    }
    text
}

fn foreign_key_text(foreign_key: &ForeignKeyDef) -> String {
    let referenced = match &foreign_key.ref_schema {
        Some(schema) => format!("{schema}.{}", foreign_key.ref_table),
        None => foreign_key.ref_table.clone(),
    };
    let mut text = format!(
        "FOREIGN KEY {} ({}) REFERENCES {referenced} ({})",
        foreign_key.name,
        foreign_key.columns.join(", "),
        foreign_key.ref_columns.join(", ")
    );
    if let Some(action) = &foreign_key.on_delete {
        text.push_str(&format!(" ON DELETE {action}"));
    }
    if let Some(action) = &foreign_key.on_update {
        text.push_str(&format!(" ON UPDATE {action}"));
    }
    text
}

fn check_text(check: &CheckDef) -> String {
    format!("CONSTRAINT {} CHECK ({})", check.name, check.expression)
}

/// 单表结构比对状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableStructureState {
    Different,
    Equal,
    Skipped,
}

/// 结构比对跳过原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructureSkipReason {
    /// 源端有、目标端缺，且未启用「自动建表」。
    TargetMissing,
    /// 目标端有、源端缺，且未启用「删除多余表」。
    SourceMissing,
    /// 方言不支持结构比对。
    Unsupported,
    /// 元数据缺类型等必要信息，无法生成安全 ALTER。
    IncompleteMetadata,
}

impl StructureSkipReason {
    pub fn message(self) -> &'static str {
        match self {
            Self::TargetMissing => "目标端不存在该表，未启用自动建表",
            Self::SourceMissing => "源端不存在该表，未启用删除多余表",
            Self::Unsupported => "该数据库类型暂不支持结构同步",
            Self::IncompleteMetadata => "结构元数据不完整，无法生成安全的结构变更",
        }
    }
}

/// 单表结构差异。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableStructureDiff {
    pub key: String,
    pub name: String,
    pub schema: Option<String>,
    pub state: TableStructureState,
    pub skip: Option<StructureSkipReason>,
    /// 读取或比对该表时的具体错误。
    pub error: Option<String>,
    /// `changes` 中实际会被执行的数量（按选项过滤后）。
    pub selected: usize,
    pub changes: Vec<StructureChange>,
    /// 跨类型同步的类型/默认值转换提示。与 `error` 不同，这些表仍可继续同步。
    pub warnings: Vec<String>,
    /// 选中变更将要执行的 SQL，仅用于预览展示，执行阶段会重新生成。
    pub ddl: Vec<String>,
    /// 目标端现状的整表定义（目标方言），差异视图左半部分；无法读取时为空。
    pub current_ddl: Option<String>,
    /// 源端目标态的整表定义（目标方言），差异视图右半部分。
    ///
    /// 与 `current_ddl` 用同一方言渲染，避免仅方言写法不同被显示成变更。
    pub desired_ddl: Option<String>,
}

impl TableStructureDiff {
    pub(crate) fn ready(key: String, name: String, schema: Option<String>) -> Self {
        Self {
            key,
            name,
            schema,
            state: TableStructureState::Different,
            skip: None,
            error: None,
            selected: 0,
            changes: Vec::new(),
            warnings: Vec::new(),
            ddl: Vec::new(),
            current_ddl: None,
            desired_ddl: None,
        }
    }

    pub(crate) fn skipped(
        key: String,
        name: String,
        schema: Option<String>,
        reason: StructureSkipReason,
    ) -> Self {
        let mut diff = Self::ready(key, name, schema);
        diff.state = TableStructureState::Skipped;
        diff.skip = Some(reason);
        diff
    }

    pub(crate) fn failed(
        key: String,
        name: String,
        schema: Option<String>,
        message: String,
    ) -> Self {
        let mut diff = Self::ready(key, name, schema);
        diff.state = TableStructureState::Skipped;
        diff.error = Some(message);
        diff
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureSyncRequest {
    pub source: SyncEndpoint,
    pub target: SyncEndpoint,
    /// 为空表示源端全部表（以及目标端多余的表的删除判断）。
    pub tables: Vec<String>,
    pub options: StructureSyncOptions,
}

impl StructureSyncOptions {
    /// 某条差异是否被选项允许执行。界面用同一规则过滤，执行阶段再校验一次。
    pub fn allows(&self, change: &StructureChange) -> bool {
        match (change.object, change.action) {
            (StructureObject::Table, StructureChangeAction::Create) => self.create_table,
            (StructureObject::Table, StructureChangeAction::Drop) => self.drop_table,
            (StructureObject::Column, StructureChangeAction::Create) => self.add_column,
            (StructureObject::Column, StructureChangeAction::Drop) => self.drop_column,
            (StructureObject::Column, StructureChangeAction::Alter) => self.alter_column,
            (StructureObject::PrimaryKey, _) => self.alter_column,
            (StructureObject::Index, StructureChangeAction::Create) => self.add_index,
            (StructureObject::Index, StructureChangeAction::Drop) => self.drop_index,
            // 索引/外键的「修改」是 DROP + ADD，需要两端动作都允许才执行。
            (StructureObject::Index, StructureChangeAction::Alter) => {
                self.add_index && self.drop_index
            }
            (StructureObject::ForeignKey, StructureChangeAction::Create) => self.add_foreign_key,
            (StructureObject::ForeignKey, StructureChangeAction::Drop) => self.drop_foreign_key,
            (StructureObject::ForeignKey, StructureChangeAction::Alter) => {
                self.add_foreign_key && self.drop_foreign_key
            }
            (StructureObject::Check, StructureChangeAction::Create) => self.add_check,
            (StructureObject::Check, StructureChangeAction::Drop) => self.drop_check,
            // CHECK 的「修改」同样是 DROP + ADD。
            (StructureObject::Check, StructureChangeAction::Alter) => {
                self.add_check && self.drop_check
            }
            // 表级 Alter 不会产生；保守地不做任何事。
            (StructureObject::Table, StructureChangeAction::Alter) => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureSyncPreview {
    pub source_kind: DatabaseKind,
    pub target_kind: DatabaseKind,
    pub tables: Vec<TableStructureDiff>,
}

impl StructureSyncPreview {
    pub fn total_changes(&self) -> usize {
        self.tables
            .iter()
            .filter(|table| table.state == TableStructureState::Different)
            .map(|table| table.selected)
            .sum()
    }

    pub fn different_tables(&self) -> usize {
        self.tables
            .iter()
            .filter(|table| table.state == TableStructureState::Different)
            .count()
    }
}

/// 单表结构同步执行结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableStructureReport {
    pub key: String,
    pub name: String,
    pub applied: usize,
    pub skipped: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureSyncReport {
    pub tables: Vec<TableStructureReport>,
}

impl StructureSyncReport {
    pub fn total_applied(&self) -> usize {
        self.tables.iter().map(|table| table.applied).sum()
    }

    pub fn failed_tables(&self) -> Vec<&TableStructureReport> {
        self.tables
            .iter()
            .filter(|table| table.error.is_some() && !table.skipped)
            .collect()
    }
}

/// 被跳过的写入行。
///
/// 只保留键的展示文本与数据库错误消息，不保留整行数据。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowWriteFailure {
    /// 键列展示文本，如 `id=42`；用于定位失败行。
    pub key: String,
    pub message: String,
}

/// 单表执行结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableSyncReport {
    pub key: String,
    pub name: String,
    pub inserted: u64,
    pub updated: u64,
    pub deleted: u64,
    /// 未执行（目标缺失、无键、用户未启用对应动作等）。
    pub skipped: bool,
    pub error: Option<String>,
    /// 按 `RowErrorPolicy::Skip` 跳过的失败行数。
    pub failed_rows: u64,
    /// 失败行明细，条数有上限（见 `FAILURE_DETAIL_LIMIT`）。
    pub failures: Vec<RowWriteFailure>,
}

/// 每张表保留的失败行明细上限；总数仍以 `failed_rows` 为准。
pub const FAILURE_DETAIL_LIMIT: usize = 200;

impl TableSyncReport {
    pub(crate) fn skipped(key: String, name: String, message: impl Into<String>) -> Self {
        Self {
            key,
            name,
            inserted: 0,
            updated: 0,
            deleted: 0,
            skipped: true,
            error: Some(message.into()),
            failed_rows: 0,
            failures: Vec::new(),
        }
    }

    pub(crate) fn failed(key: String, name: String, message: impl Into<String>) -> Self {
        let mut report = Self::skipped(key, name, message);
        report.skipped = false;
        report
    }
}

/// 一次数据同步的执行结果。表级失败不会中断其它表，错误随表返回。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataSyncReport {
    pub tables: Vec<TableSyncReport>,
    /// 本次执行在本地账本里的记录 ID（未接存储或写头失败时为 `None`）。
    pub run_id: Option<i64>,
}

impl DataSyncReport {
    pub fn total_inserted(&self) -> u64 {
        self.tables.iter().map(|table| table.inserted).sum()
    }

    pub fn total_updated(&self) -> u64 {
        self.tables.iter().map(|table| table.updated).sum()
    }

    pub fn total_deleted(&self) -> u64 {
        self.tables.iter().map(|table| table.deleted).sum()
    }

    pub fn failed_tables(&self) -> Vec<&TableSyncReport> {
        self.tables
            .iter()
            .filter(|table| table.error.is_some() && !table.skipped)
            .collect()
    }
}
