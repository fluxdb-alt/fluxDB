/// 任务中心历史记录：保存已结束任务的展示快照，重启后仍可查看。
///
/// - `kind` / `status`：稳定枚举键（backup / restore / ...，success / failed / canceled）。
/// - `title` / `target` / `stage` / `logs` / `error`：结束时已本地化的展示文本。
/// - `started_unix_ms` / `finished_unix_ms`：墙钟时间，用于重启后还原耗时。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskHistoryRecord {
    pub id: u64,
    pub kind: String,
    pub title: String,
    pub target: String,
    pub status: String,
    pub stage: String,
    #[serde(default)]
    pub logs: Vec<String>,
    pub started_unix_ms: u64,
    pub finished_unix_ms: u64,
    #[serde(default)]
    pub error: Option<String>,
}
