// Redis 连接级「服务器」读模型：运行概览、客户端列表与慢日志。
//
// 与 SQL 的 `ServerActivity` 分开：Redis 没有会话/锁模型，连接与命令的观测口径
// 是 `INFO` / `CLIENT LIST` / `SLOWLOG`，强行复用 SQL 结构会把语义拧在一起。

/// 一次读取的 Redis 实例快照。
///
/// 三个分区各自带错误：ACL 不允许 `CLIENT LIST` 或 `SLOWLOG GET` 时，
/// 概览仍可展示，不能把整页判失败。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedisServerActivity {
    /// `redis_version`。
    pub version: String,
    /// `redis_mode`（standalone / cluster / sentinel）。
    pub mode: String,
    /// `role`（master / slave）。
    pub role: String,
    /// `uptime_in_seconds`。
    pub uptime_secs: Option<u64>,
    /// `connected_clients`。
    pub connected_clients: u64,
    /// `used_memory`，单位字节。
    pub used_memory_bytes: u64,
    /// 概览指标，`key` 为 INFO 原始字段名，展示层负责本地化。
    pub metrics: Vec<ServerMetric>,
    /// 当前连接自身在服务端的 client id（用于禁止终止自己）。
    pub current_client_id: String,
    pub clients: Vec<RedisClient>,
    /// 客户端列表读取失败原因；`None` 表示读取成功（可能是空列表）。
    pub clients_error: Option<String>,
    pub slowlog: Vec<RedisSlowLogEntry>,
    /// 慢日志读取失败原因；`None` 表示读取成功。
    pub slowlog_error: Option<String>,
    /// `SLOWLOG LEN`。
    pub slowlog_len: u64,
    /// `slowlog-log-slower-than`（微秒）；读取失败为 `None`。
    pub slowlog_log_slower_than_micros: Option<i64>,
    /// `slowlog-max-len`；读取失败为 `None`。
    pub slowlog_max_len: Option<i64>,
}

/// 一条客户端连接（`CLIENT LIST` 的一行）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedisClient {
    /// 客户端 ID；终止客户端时使用。
    pub id: String,
    /// 对端地址 `host:port`。
    pub addr: String,
    /// 本地监听地址。
    pub local_addr: String,
    /// `CLIENT SETNAME` 设置的名称。
    pub name: String,
    /// ACL 用户名。
    pub user: String,
    /// 当前选择的逻辑库序号。
    pub database: String,
    /// 最近一次命令（`cmd=client|list` 这类）。
    pub command: String,
    /// 连接已建立时长（秒）。
    pub age_secs: u64,
    /// 空闲时长（秒）。
    pub idle_secs: u64,
    /// 客户端标志位（N / S / M / O 等）。
    pub flags: String,
    /// 是否为当前面板自身连接。
    pub is_self: bool,
}

/// 一条慢日志（`SLOWLOG GET` 的一行）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedisSlowLogEntry {
    /// 慢日志 ID。
    pub id: u64,
    /// 发生时间（Unix 秒）。
    pub timestamp_unix_secs: i64,
    /// 执行耗时（微秒）。
    pub duration_micros: u64,
    /// 命令文本；参数已按 Redis 规则做了省略/截断。
    pub command: String,
    /// 来源客户端地址。
    pub client_addr: String,
    /// 来源客户端名称。
    pub client_name: String,
}
