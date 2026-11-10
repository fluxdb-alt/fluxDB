// 连接级「服务器状态」读模型：会话、锁等待、实例指标与一键维护。
//
// 只承载各数据库能稳定提供的公共字段；方言独有的展示细节（锁类型文案、指标说明）
// 由展示层按 `key` 映射，避免把界面文案固化进连接器。

/// 一次性读取的服务器活动快照。
///
/// 锁与状态分区各自带错误：权限不足（如 MySQL 未授予 `PROCESS`/`performance_schema`
/// 读取权限）时不能把整页判失败，也不能伪造成「没有锁等待」。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerActivity {
    /// 服务端版本，如 `8.0.36` / `16.2`。
    pub version: String,
    /// 当前连接使用的账号（用于「只看我的会话」与终止自身会话的保护）。
    pub current_user: String,
    /// 当前实例全部可见会话。
    pub sessions: Vec<ServerSession>,
    /// 当前锁等待边（等待会话 → 阻塞会话）。
    pub locks: Vec<ServerLockWait>,
    /// 锁等待读取失败原因；`None` 表示读取成功（可能是空列表）。
    pub locks_error: Option<String>,
    /// 实例状态指标。
    pub status: ServerStatus,
    /// 状态指标读取失败原因；`None` 表示读取成功。
    pub status_error: Option<String>,
}

/// 一条服务端会话。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerSession {
    /// 数据库内的会话标识（MySQL 为连接 ID，PostgreSQL 为 pid）。
    pub id: String,
    pub user: String,
    /// `host:port`；无法获取时为空串。
    pub host: String,
    pub database: String,
    /// 会话命令/后端类型（Query、Sleep、idle、active 等）。
    pub command: String,
    /// 已持续时间（秒）。
    pub elapsed_secs: u64,
    /// 等待事件；空闲会话为空。
    pub wait_event: String,
    /// 正在执行或最后执行的 SQL；空闲会话可能为空。
    pub sql: String,
    /// 是否为当前连接自身的会话。
    pub is_self: bool,
}

/// 一条锁等待边：`waiting` 被 `blocking` 阻塞。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerLockWait {
    pub waiting_session: String,
    pub blocking_session: String,
    /// `schema.table` 或数据库对象描述。
    pub object: String,
    /// 锁类型（行锁/表锁/元数据锁/advisor 等由方言映射）。
    pub lock_type: String,
    /// 已等待时长（秒）。
    pub wait_secs: u64,
    /// 等待状态描述。
    pub state: String,
    /// 等待会话的 SQL。
    pub sql: String,
}

/// 实例状态指标。`key` 是服务端原始变量名，展示层负责本地化说明。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerStatus {
    /// 服务端运行时长（秒）；无法获取时为 `None`。
    pub uptime_secs: Option<u64>,
    pub metrics: Vec<ServerMetric>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerMetric {
    /// 服务端变量名或稳定指标键（如 `Threads_connected`、`numbackends`）。
    pub key: String,
    pub value: String,
    /// 需要提示关注的指标（锁等待、慢查询等）。
    pub warning: bool,
}

/// 一键维护动作。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerMaintenanceAction {
    /// PostgreSQL：`VACUUM`。
    Vacuum,
    /// MySQL/PG：`ANALYZE` / `ANALYZE TABLE`。
    Analyze,
    /// MySQL：`OPTIMIZE TABLE`。
    Optimize,
    /// PostgreSQL：`REINDEX DATABASE`。
    Reindex,
}

impl ServerMaintenanceAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vacuum => "vacuum",
            Self::Analyze => "analyze",
            Self::Optimize => "optimize",
            Self::Reindex => "reindex",
        }
    }
}

/// 维护请求。`database` 为目标库；空串表示连接默认库。
#[derive(Clone, Debug, PartialEq)]
pub struct ServerMaintenanceRequest {
    pub connection_id: ConnectionId,
    pub database: String,
    pub action: ServerMaintenanceAction,
}
