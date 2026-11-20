// 数据库无关的表结构元数据模型（T08）。
//
// 承载「结构 → DDL 重建」与「单元格可编辑性（T09）」所需的全部结构信息。
// pg_catalog/MySQL/SQLite 目录查询统一投影到此模型；图形编辑器尚不能表达的属性
// （分区、RLS、排除约束、扩展属性等）保留原始 definition，标记只读，不做字符串猜测。

// （ObjectKind 已在 object_query.rs 的 include 作用域内定义，无需重复导入。）

/// 列元数据：类型身份、默认/identity/generated、可空与可编辑性、主键/唯一键标记。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnMeta {
    pub name: String,
    /// 1 起始的字段顺序（对应 PG attnum>0 的序；已 DROPPED 不进入此列）。
    pub ordinal: u32,
    /// 类型完整文本（含长度/精度，如 `varchar(100)`；PG 字符/时间类型归一为应用短别名）。
    pub data_type: String,
    /// 类型 schema 限定身份（PG 为 nspname；用于跨会话类型识别）。
    pub type_schema: Option<String>,
    /// 基础类型名（PG typname，如 `varchar`）；domain/enum 保留其名。
    pub type_name: Option<String>,
    pub nullable: bool,
    /// 列默认表达式原文（PG `pg_get_expr(adbin)`）；DEFAULT 是 SQL 语法节点，不当作参数。
    pub default_expr: Option<String>,
    /// 是否 identity 列（PG attidentity <> ''）。
    pub is_identity: bool,
    /// identity 生成方式：`ALWAYS` / `BY DEFAULT`（仅 is_identity 时有意义）。
    pub identity_generation: Option<String>,
    /// 是否生成列（PG attgenerated <> ''，如 STORED）。
    pub is_generated: bool,
    pub is_editable: bool,
    pub primary_key: bool,
    pub unique_key: bool,
    pub comment: Option<String>,
}

/// 索引键项：可为命名列，也可为表达式（`attnum=0` 时 PG 把 text 形式放入 pg_index.indkey）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexColumnItem {
    /// 命名列；表达式键项为 `None`（expression 有效）。
    pub column: Option<String>,
    /// 表达式文本（PG 从 `pg_get_indexdef` 提取；命名列键项为 None）。
    pub expression: Option<String>,
    pub descending: bool,
    pub nulls_first: bool,
}

/// 索引元数据：键项顺序、INCLUDE、predicate、访问方法、有效状态与原始 definition。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexMeta {
    pub name: String,
    pub columns: Vec<IndexColumnItem>,
    pub include_columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
    /// 访问方法（PG `pg_am.amname`，如 `btree`）。
    pub index_type: Option<String>,
    /// partial index 的 predicate 原文；`None` 表示全表索引。
    pub predicate: Option<String>,
    /// 是否有效（PG `pg_index.indisvalid`；CONCURRENTLY 失败建出的索引无效）。
    pub valid: bool,
    /// `pg_get_indexdef` 完整定义，用于保真展示与重建。
    pub definition: String,
}

/// 外键元数据：约束级组织，保留多列序位配对（conkey/confkey 同序）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignKeyMeta {
    pub name: String,
    /// 本表列序列（由此表列名；序位对应 ref_columns）。
    pub columns: Vec<String>,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    /// 删除/更新动作语义（PG `a`=NO ACTION、`r`=RESTRICT、`c`=CASCADE、`n`=SET NULL、`d`=SET DEFAULT）。
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
    pub match_type: Option<String>,
    pub deferrable: bool,
    pub initially_deferred: bool,
    /// `pg_get_constraintdef` 完整定义，用于保真展示。
    pub definition: String,
}

/// CHECK 约束元数据：独立于列，不从默认值字符串猜测。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckMeta {
    pub name: String,
    pub expression: String,
    pub definition: String,
}

/// 唯一键：PG 唯一约束（pg_constraint contype='u'）与独立唯一索引（contype='x'）在此归一。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UniqueKeyMeta {
    pub name: String,
    pub columns: Vec<String>,
    /// 是否可见索引（唯一约束在 PG 中会隐式创建唯一索引）。
    pub is_constraint: bool,
    pub definition: String,
}

/// 表结构全量元数据：DDL 重建与单元格可编辑性（T09）的输入。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableStructure {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
    pub kind: ObjectKind,
    pub columns: Vec<ColumnMeta>,
    /// 主键列（按序；空表/无键表为空）。
    pub primary_key: Vec<String>,
    pub foreign_keys: Vec<ForeignKeyMeta>,
    pub checks: Vec<CheckMeta>,
    pub unique_keys: Vec<UniqueKeyMeta>,
    pub indexes: Vec<IndexMeta>,
    /// 表级用户触发器（不含内部约束触发器）：供触发器 tab 与 DDL 展示。
    pub triggers: Vec<TriggerMeta>,
    pub comment: Option<String>,
}

/// 索引的附加属性：结构同步判断「索引是否真的变了」以及重建时保留语义。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexAttributes {
    /// 索引方法/类型：MySQL `BTREE`/`FULLTEXT`/`SPATIAL`、PG `btree`/`gin`、
    /// SQL Server `CLUSTERED`/`NONCLUSTERED`、Oracle `NORMAL`/`BITMAP`。
    /// 方言未读取时为 `None`，比对方视为「未知」而不是「没有方法」。
    pub index_type: Option<String>,
    /// 部分索引谓词（PG partial index）；`None` 表示全表索引。
    pub predicate: Option<String>,
    /// 键项含表达式（函数索引）。此类索引无法按列名安全重建，结构同步跳过。
    pub has_expression: bool,
}

/// 外键的引用动作。仅记录方言确实读取到的部分：
/// Oracle/达梦没有 ON UPDATE 动作，`on_update` 恒为 `None`。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ForeignKeyActions {
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

/// 生成列定义：表达式原文 + 存储方式。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedColumn {
    /// 计算表达式原文（目录输出，可能带方言函数）。
    pub expression: String,
    /// 是否物化存储：MySQL/SQLite 有 VIRTUAL/STORED 之分；PG 只有 STORED，
    /// 方言不区分时留 `None`，按目标库默认方式建列。
    pub stored: Option<bool>,
}

/// 打开设计表所需的完整、一致的元数据快照。
///
/// 各连接器可覆盖 `Connector::load_table_design_metadata`，在一条连接内复用同一次
/// 目录读取；默认实现仅为兼容尚未优化的方言。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableDesignMetadata {
    pub columns: Vec<CompletionColumn>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub triggers: Vec<TriggerInfo>,
    /// 列默认表达式原文：键为列名（与 `columns.name` 一致）。
    ///
    /// 结构同步需要它判断 DEFAULT 差异；方言未提供时留空，调用方应跳过默认值比对，
    /// 不能把「未读取」当成「没有默认值」。
    pub column_defaults: BTreeMap<String, String>,
    /// 自增 / identity 列名（保留原始大小写）。跨方言建表据此生成等价语法；
    /// 未读取时为空集合，调用方不得推断。
    pub auto_increment_columns: BTreeSet<String>,
    /// 生成列：列名 → 表达式与存储方式。未出现在此表的列不是生成列。
    pub generated_columns: BTreeMap<String, GeneratedColumn>,
    /// 索引附加属性：索引名 → 方法/谓词/表达式标记。缺项视为未读取。
    pub index_attributes: BTreeMap<String, IndexAttributes>,
    /// 外键引用动作：约束名 → 动作。缺项视为未读取。
    pub foreign_key_actions: BTreeMap<String, ForeignKeyActions>,
    /// CHECK 约束。只有 `checks_loaded` 为真时才可参与比对。
    pub checks: Vec<CheckMeta>,
    /// 方言是否读取了 CHECK 约束；false 时禁止推断「目标端没有检查约束」。
    pub checks_loaded: bool,
    /// 表注释。方言不提供时为 `None`。
    pub table_comment: Option<String>,
    pub ddl: String,
    /// 首屏是否已包含索引/外键/触发器。慢方言可先返回 false，再由后台补齐。
    pub details_loaded: bool,
}

/// 触发器元数据：事件/时相/级别与函数身份，保留完整定义。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggerMeta {
    pub name: String,
    /// 触发事件展示文本，如 `INSERT` / `UPDATE` / `DELETE` / `TRUNCATE`（多事件逗号分隔）。
    pub event: String,
    /// 时相：`BEFORE` / `AFTER` / `INSTEAD OF`。
    pub timing: String,
    /// 行级/语句级（PG `tgtype` 行位）。
    pub level: String,
    /// 触发函数 schema 限定签名（如 `public.my_trigger_fn()`）。
    pub function: String,
    pub enabled: bool,
    /// `pg_get_triggerdef` 完整定义。
    pub definition: String,
}
