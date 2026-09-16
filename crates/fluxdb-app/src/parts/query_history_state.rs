#[derive(Clone, Debug, PartialEq)]
pub struct QueryHistoryEntry {
    /// 仅进程内关联事务；历史文件不恢复活会话。
    pub session_id: Option<fluxdb_core::QuerySessionId>,
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// 历史记录所属 schema 作用域（PG）；MySQL/TiDB/Redis 恒为 None。
    pub schema: Option<String>,
    pub text: String,
    pub tables: Vec<String>,
    pub kind: QueryHistoryKind,
    pub success: bool,
    pub summary: QueryExecutionSummary,
    pub executed_at_unix_secs: u64,
    pub object: Option<String>,
    pub rollback_snapshot: Option<QueryRollbackSnapshot>,
    /// 写入是否已提交（§8.4/R11）。显式事务里未 COMMIT 的写入不显示为已提交。
    pub transaction_state: QueryHistoryTransactionState,
}

/// 写入的事务状态。默认 `Committed`（无显式事务、旧记录缺省时按已提交展示）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QueryHistoryTransactionState {
    /// 已提交（自动提交或显式 COMMIT）。
    #[default]
    Committed,
    /// 事务仍未提交（显式事务进行中）。
    Uncommitted,
    /// 已回滚（显式 ROLLBACK，或连接释放时未提交被服务端回滚）。
    RolledBack,
    /// 断连或取消未确认，不能推断提交/回滚。
    Unknown,
}

impl QueryHistoryTransactionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Uncommitted => "uncommitted",
            Self::RolledBack => "rolled_back",
            Self::Unknown => "unknown",
        }
    }

    /// 中文展示标签（历史详情 UI 用）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Committed => "已提交",
            Self::Uncommitted => "未提交",
            Self::RolledBack => "已回滚",
            Self::Unknown => "结果待核实",
        }
    }

    /// 从持久化字符串还原；未知值按已提交（旧记录兼容）。
    pub fn from_storage(value: Option<&str>) -> Self {
        match value {
            Some("uncommitted") => Self::Uncommitted,
            Some("rolled_back") => Self::RolledBack,
            Some("unknown") => Self::Unknown,
            _ => Self::Committed,
        }
    }
}
