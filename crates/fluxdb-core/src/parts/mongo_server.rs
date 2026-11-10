// MongoDB 连接级服务器面板读模型：实例状态与当前操作。
//
// 与 SQL 的 `ServerActivity`、Redis 的 `RedisServerActivity` 分开：
// MongoDB 的观测口径是 `serverStatus` / `$currentOp`，会话与锁不适用。

/// 一条 `$currentOp` 记录（已裁剪为排障所需字段）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MongoOperation {
    /// `opid` 的十进制文本；终止操作时原样回传。
    pub opid: String,
    /// 客户端地址（`client` / `clientMetadata` 推导）。
    pub client: String,
    /// 操作类型：query / insert / update / remove / getmore / command / none。
    pub op: String,
    /// 命名空间 `db.collection`；命令类操作可能为空。
    pub namespace: String,
    /// 操作描述（`desc` 或命令名）。
    pub description: String,
    /// 已运行秒数（`secs_running`）。
    pub secs_running: u64,
    /// 是否在等待锁（`waitingForLock`）。
    pub waiting_for_lock: bool,
    /// 查询计划摘要（`planSummary`）。
    pub plan_summary: String,
}

impl MongoOperation {
    /// 慢操作判定：运行超过 1 秒。界面「只看慢操作」与告警都复用这个口径。
    pub fn is_slow(&self) -> bool {
        self.secs_running >= 1
    }
}

/// 连接级 MongoDB 服务器状态快照。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MongoServerActivity {
    pub version: String,
    pub edition: String,
    pub topology: MongoTopology,
    /// 副本集名称（`hello.setName`）；单机为空。
    pub replica_set_name: String,
    /// 当前节点角色（`hello.myState` → PRIMARY/SECONDARY/ARBITER），单机为空。
    pub member_state: String,
    pub uptime_secs: Option<u64>,
    pub connections_current: u64,
    pub connections_available: Option<u64>,
    /// 概览指标，key 使用 `serverStatus` 的层级路径（如 `opcounters.query`）。
    pub metrics: Vec<ServerMetric>,
    pub operations: Vec<MongoOperation>,
    /// `$currentOp` 失败不影响概览，只影响「当前操作」子页。
    pub operations_error: Option<String>,
}
