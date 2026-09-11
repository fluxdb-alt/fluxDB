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
    /// 类型完整文本（PG `format_type`，含长度/精度，如 `varchar(100)`）。
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
