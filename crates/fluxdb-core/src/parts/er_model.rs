// ER 关系图「画布原型」用纯数据模型（er-design.md §8 步骤 3）。
//
// 只承载一次后台加载全库表 / 列 / 外键的结果，供 desktop 侧布局与绘制消费。
// 本文件只放跨层纯数据，不包含任何加载 / 驱动 / SQL 逻辑（加载编排在 fluxdb-app 的
// er_service.rs，绘制在 desktop 的 er/canvas.rs）。逻辑关系编辑、required_filters、
// usage、Agent 投影等仍属设计文档未确认部分，本轮不在此建模。

/// 结构化表身份（er-design §5.2 身份与名称分离的展示侧等价物）。
///
/// 跨 schema 同名表、以及 schema/表名含 `.` 的合法标识符，都不能靠展示名或 `rsplit('.')`
/// 字符串拆解可靠区分；本类型用 (database, schema, name) 显式成对保存，供匹配/去重使用。
/// 展示名由 `display()` 统一生成，所有匹配（邻域、布局、场景）都按此身份进行。
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ErTableRef {
    pub database: String,
    /// 仅 PostgreSQL 多 schema 有意义；其余方言为 None。
    pub schema: Option<String>,
    /// 裸表名（不含 schema 前缀；允许含 `.` 等合法字符）。
    pub name: String,
}

impl ErTableRef {
    /// 记录裸表名是否为空；空名视为非法身份。
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
    }

    /// 展示名：PG 带 schema 为 `schema.name`（与旧 display 命名一致），其余裸名。
    pub fn display(&self) -> String {
        match &self.schema {
            Some(s) if !s.is_empty() && !self.name.is_empty() => format!("{s}.{}", self.name),
            _ => self.name.clone(),
        }
    }
}

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
    /// 结构化身份：跨 schema 同名、含点标识符据此唯一匹配，不用字符串拆解。
    pub reference: ErTableRef,
    pub comment: Option<String>,
    /// 数据库稳定对象标识（PG `pg_class.oid`）：改名但同对象可据此在结构刷新重绑中
    /// 自动接回关系；MySQL/SQLite 无公开稳定号 → `None`（不伪造）。
    pub stable: Option<u64>,
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
    /// 列级稳定标识：PG attnum，仅在所属表稳定身份不变时用于自动重绑。
    pub stable: Option<u64>,
}

/// 外键连线：已解析到两端表列引用，desktop 据此直接画线。
/// from_* 为本端（持有约束的表），to_* 为被引用端。
#[derive(Clone, Debug, PartialEq)]
pub struct ErForeignKeyEdge {
    pub name: String,
    /// 本端展示名（display，供既有渲染/tooltip）。
    pub from_table: String,
    pub from_column: String,
    /// 被引用端展示名。
    pub to_table: String,
    pub to_column: String,
    /// 两端结构化身份：匹配邻域/布局按此进行，正确处理跨 schema 同名与含点标识符。
    pub from_reference: ErTableRef,
    pub to_reference: ErTableRef,
}
