// ER 关系图「画布原型」用纯数据模型（er-design.md §8 步骤 3）。
//
// 只承载一次后台加载全库表 / 列 / 外键的结果，供 desktop 侧布局与绘制消费。
// 本文件只放跨层纯数据，不包含任何加载 / 驱动 / SQL 逻辑（加载编排在 fluxdb-app 的
// er_service.rs，绘制在 desktop 的 er/canvas.rs）。逻辑关系编辑、required_filters、
// usage、Agent 投影等仍属设计文档未确认部分，本轮不在此建模。

/// ER 范围/实体的加载状态。空字段列表不能同时代表「未读取」与「没有字段」，
/// 关系未加载也不能显示成「没有关系」；本枚举用于明确区分。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ErLoadStatus {
    /// 尚未读取（例如该表未进入读取范围）。
    #[default]
    NotLoaded,
    /// 读取进行中。
    Loading,
    /// 已读取完成（字段列表可能为空 —— 空是「确实没有字段」，不是「未读」）。
    Loaded,
    /// 读取失败；保留现有可用内容，不整体清空。
    Failed,
}

/// 整库 ER 画布数据：表节点 + 外键连线。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ErGraphData {
    pub tables: Vec<ErTableNode>,
    pub edges: Vec<ErForeignKeyEdge>,
    /// 关系（外键索引）的加载状态：与会话「无关系」区分开，
    /// 关系未加载/部分失败时不能展示成「没有关系」。
    pub relation_status: ErLoadStatus,
}

/// 表节点：展示表名 + 逐行可见列。
#[derive(Clone, Debug, PartialEq)]
pub struct ErTableNode {
    /// 展示与去重用名。PostgreSQL 多 schema 时拼 `schema.table`，
    /// MySQL/SQLite 单 schema 时即为裸表名。
    pub name: String,
    pub comment: Option<String>,
    /// 该表字段（列）加载状态；字段是否为空据此理解，不能当作「未读」。
    pub status: ErLoadStatus,
    pub columns: Vec<ErColumn>,
}

/// 列：ER 画布首版仅需要名称 / 类型 / 主键 / 可空，用于表节点内逐行展示。
#[derive(Clone, Debug, PartialEq)]
pub struct ErColumn {
    pub name: String,
    pub type_name: Option<String>,
    pub primary_key: bool,
    pub nullable: bool,
}

/// 外键连线：已解析到两端表列引用，desktop 据此直接画线。
/// from_* 为本端（持有约束的表），to_* 为被引用端。
#[derive(Clone, Debug, PartialEq)]
pub struct ErForeignKeyEdge {
    pub name: String,
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
}
