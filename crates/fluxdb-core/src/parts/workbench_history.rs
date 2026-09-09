// 通用 Workbench 历史抽象。
//
// 目标：把「某种数据库的工作台历史」抽成统一 trait 与 scope，供 SQL / Redis /
// 后续 MySQL 等不同后端按数据库类型差异化实现；UI 的「历史」入口只依赖 trait，
// 不再关心当前是 SQL 还是 Redis，从而让历史能力从「SQL 专属」升级为通用能力。
//
// 参考：
// - RedisInsight 的 Workbench 历史按 databaseId 作用域持久化（workbenchStorage.ts，
//   一个 objectStore，主键 [id, databaseId]，每个数据库上限 WORKBENCH_HISTORY_MAX_LENGTH）。
// - 本仓库现有 SQL 历史 (QueryHistoryEntry / query_history.toml) 的「按连接 + DB 隔离」约定。

/// 历史记录所属后端与作用域。
///
/// 按「数据库类型」分叉：SQL 与 Redis 各自表达自己的 scope。scope 至少包含
/// `connection_id`，Redis 再携带逻辑数据库编号 `database`，保证不同连接、不同库
/// 的历史互不串台。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkbenchHistoryScope {
    /// SQL 工作台：连接 + 可选 database / schema 字符串。
    Sql {
        connection_id: ConnectionId,
        database: Option<String>,
    },
    /// Redis 工作台：连接 + 逻辑数据库编号（0-15）。
    Redis {
        connection_id: ConnectionId,
        database: u32,
    },
}

impl WorkbenchHistoryScope {
    /// 归属的连接 ID。
    pub fn connection_id(&self) -> ConnectionId {
        match self {
            WorkbenchHistoryScope::Sql { connection_id, .. }
            | WorkbenchHistoryScope::Redis { connection_id, .. } => *connection_id,
        }
    }
}

/// 一条可回填 / 可删除的历史记录（通用最小字段，各后端持久化时可扩展）。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkbenchHistoryItem {
    /// 记录唯一 ID（按 scope 内单调递增 / 去重，用于删除定位）。
    pub id: u64,
    /// 命令 / SQL 文本，点击历史可回填到输入框。
    pub text: String,
    /// 执行是否成功（成功 / 失败都要入历史，保留排障信息）。
    pub success: bool,
    /// 执行 Unix 秒级时间戳。
    pub executed_at_unix_secs: u64,
    /// 人类可读的结果摘要（供列表展示，与 RedisInsight summary 对齐）。
    pub summary: String,
    /// 来源类型（Workbench / HistoryRerun / KeyShortcut）。
    pub source: CommandExecutionSource,
}

/// 通用 Workbench 历史存储能力。
///
/// 各后端（SQL / Redis / 后续 MySQL…）按自己的持久化与记录结构分别实现，
/// 上层「历史」入口只依赖该 trait，不关心当前是 SQL 还是 Redis。
pub trait WorkbenchHistoryStore {
    /// 按 scope 拉取历史。`limit` 控制返回条数；返回顺序由后端决定（建议最新在前）。
    fn load_history(
        &self,
        scope: &WorkbenchHistoryScope,
        limit: usize,
    ) -> Vec<WorkbenchHistoryItem>;

    /// 向 scope 追加一条历史记录。
    fn append_history(&mut self, scope: &WorkbenchHistoryScope, item: WorkbenchHistoryItem);

    /// 删除 scope 内指定 id 的一条记录（不影响其他 scope）。
    fn delete_history(&mut self, scope: &WorkbenchHistoryScope, id: u64);

    /// 清空 scope 内全部历史（不影响其他 scope）。
    fn clear_history(&mut self, scope: &WorkbenchHistoryScope);
}
