// Redis Workbench 命令历史的 App 层实现：把 fluxdb-core 的通用历史 trait 接到
// `AppState.redis_workbench_history`（内存态，桌面层负责持久化到 workbench-history.toml）。
//
// 分层约定：
// - fluxdb-core 定义 `WorkbenchHistoryScope` / `WorkbenchHistoryItem` / `WorkbenchHistoryStore` trait。
// - fluxdb-app 在此为 `AppController` 实现该 trait，按 scope（Sql / Redis）分叉路由，
//   让上层「历史」入口只调 trait、不关心当前是 SQL 还是 Redis。
// - SQL 历史沿用现有 `record_query_execution_history` / `query_history` 的完整能力
//   （含 rollback 快照），此处仅提供 trait 的 SQL 适配（load/delete/clear 复用同一份 state）。

/// Redis Workbench 历史记录 ID 的分配：从 AppState 计数器中取出并递增。
impl AppController {
    fn next_redis_workbench_history_id(&mut self) -> u64 {
        let id = self.state.next_redis_workbench_history_id;
        self.state.next_redis_workbench_history_id += 1;
        id
    }

    /// 命令执行成功（`CommandWorkbenchExecution` 产出）后写入一条历史记录。
    ///
    /// 成功 / 失败都要入历史；当前所有执行来源统一为 Workbench（重跑的 source 区分
    /// 属后续增强，不影响历史主链路）。
    pub(crate) fn record_redis_workbench_history_execution(
        &mut self,
        execution: &CommandWorkbenchExecution,
    ) {
        let CommandExecutionTarget::Redis {
            connection_id,
            database,
        } = &execution.target
        else {
            return;
        };
        let id = self.next_redis_workbench_history_id();
        self.state.redis_workbench_history.push(RedisWorkbenchHistoryEntry {
            id,
            connection_id: *connection_id,
            database: *database,
            text: execution.text.trim().to_string(),
            success: execution.summary.failed == 0,
            executed_at_unix_secs: execution.started_at_unix_secs,
            summary: redis_workbench_history_summary(&execution.summary),
            source: CommandExecutionSource::Workbench,
        });
    }

    /// 命令执行失败后写入一条历史记录（不丢失排障信息）。
    pub(crate) fn record_redis_workbench_history_failure(
        &mut self,
        connection_id: ConnectionId,
        database: u32,
        text: &str,
        error_message: &str,
    ) {
        if text.trim().is_empty() {
            return;
        }
        let id = self.next_redis_workbench_history_id();
        self.state.redis_workbench_history.push(RedisWorkbenchHistoryEntry {
            id,
            connection_id,
            database,
            text: text.trim().to_string(),
            success: false,
            executed_at_unix_secs: current_unix_secs(),
            summary: format!("执行失败: {error_message}"),
            source: CommandExecutionSource::Workbench,
        });
    }
}

/// 把执行汇总转成一行人类可读摘要（与 RedisInsight 的 `{total, success, fail}` 对齐）。
fn redis_workbench_history_summary(summary: &CommandExecutionSummary) -> String {
    format!(
        "{} 个命令 · 成功 {} · 失败 {} · 跳过 {}",
        summary.total, summary.success, summary.failed, summary.skipped
    )
}

/// 把 App 层历史记录转成 trait 的通用 item（供顶层「历史」入口统一消费）。
fn redis_workbench_entry_to_item(entry: &RedisWorkbenchHistoryEntry) -> WorkbenchHistoryItem {
    WorkbenchHistoryItem {
        id: entry.id,
        text: entry.text.clone(),
        success: entry.success,
        executed_at_unix_secs: entry.executed_at_unix_secs,
        summary: entry.summary.clone(),
        source: entry.source,
    }
}

/// SQL 历史条目的适配：现有 SQL 条目无稳定主键，用其在「scope 过滤后列表」中的下标作为
/// id，`delete_history` 按该下标还原原记录再删除。
fn sql_history_item(entry: &QueryHistoryEntry, index: usize) -> WorkbenchHistoryItem {
    WorkbenchHistoryItem {
        id: index as u64,
        text: entry.text.clone(),
        success: entry.success,
        executed_at_unix_secs: entry.executed_at_unix_secs,
        summary: if entry.summary.message.is_empty() {
            sql_history_kind_label(&entry.kind)
        } else {
            entry.summary.message.clone()
        },
        source: CommandExecutionSource::Workbench,
    }
}

fn sql_history_kind_label(kind: &QueryHistoryKind) -> String {
    match kind {
        QueryHistoryKind::Query => "查询".to_string(),
        QueryHistoryKind::DataChange => "数据变更".to_string(),
        QueryHistoryKind::SchemaChange => "结构变更".to_string(),
    }
}

impl WorkbenchHistoryStore for AppController {
    fn load_history(
        &self,
        scope: &WorkbenchHistoryScope,
        limit: usize,
    ) -> Vec<WorkbenchHistoryItem> {
        let mut entries = match scope {
            WorkbenchHistoryScope::Redis {
                connection_id,
                database,
            } => self
                .state
                .redis_workbench_history
                .iter()
                .filter(|entry| {
                    entry.connection_id == *connection_id && entry.database == *database
                })
                .map(redis_workbench_entry_to_item)
                .collect::<Vec<_>>(),
            WorkbenchHistoryScope::Sql {
                connection_id,
                database,
            } => self
                .state
                .query_history
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry.connection_id == *connection_id && entry.database == *database
                })
                .map(|(index, entry)| sql_history_item(entry, index))
                .collect::<Vec<_>>(),
        };
        // 最新在前（与 RedisInsight 加载后 reverse 一致）。
        entries.reverse();
        entries.truncate(limit);
        entries
    }

    fn append_history(&mut self, scope: &WorkbenchHistoryScope, item: WorkbenchHistoryItem) {
        // SQL 历史走现有 `record_query_execution_history` / `record_failed_query_history`
        // 的完整能力（含 rollback 快照），不在此处通过最小 item 追加。
        let WorkbenchHistoryScope::Redis {
            connection_id,
            database,
        } = scope
        else {
            return;
        };
        let id = self.next_redis_workbench_history_id();
        self.state.redis_workbench_history.push(RedisWorkbenchHistoryEntry {
            id,
            connection_id: *connection_id,
            database: *database,
            text: item.text,
            success: item.success,
            executed_at_unix_secs: item.executed_at_unix_secs,
            summary: item.summary,
            source: item.source,
        });
    }

    fn delete_history(&mut self, scope: &WorkbenchHistoryScope, id: u64) {
        match scope {
            WorkbenchHistoryScope::Redis {
                connection_id,
                database,
            } => {
                self.state.redis_workbench_history.retain(|entry| {
                    !(entry.connection_id == *connection_id
                        && entry.database == *database
                        && entry.id == id)
                });
            }
            WorkbenchHistoryScope::Sql {
                connection_id,
                database,
            } => {
                // id 是 scope 过滤后列表的下标，还原出原记录的 (text, time) 再删除，
                // 避免误删其他 scope 或相同文本的记录。
                let Some((text, time)) = self
                    .state
                    .query_history
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| {
                        entry.connection_id == *connection_id && entry.database == *database
                    })
                    .nth(id as usize)
                    .map(|(_, entry)| (entry.text.clone(), entry.executed_at_unix_secs))
                else {
                    return;
                };
                self.state.query_history.retain(|entry| {
                    entry.connection_id != *connection_id
                        || entry.database != *database
                        || entry.text != text
                        || entry.executed_at_unix_secs != time
                });
            }
        }
    }

    fn clear_history(&mut self, scope: &WorkbenchHistoryScope) {
        match scope {
            WorkbenchHistoryScope::Redis {
                connection_id,
                database,
            } => {
                self.state.redis_workbench_history.retain(|entry| {
                    entry.connection_id != *connection_id || entry.database != *database
                });
            }
            WorkbenchHistoryScope::Sql {
                connection_id,
                database,
            } => {
                self.state.query_history.retain(|entry| {
                    entry.connection_id != *connection_id || entry.database != *database
                });
            }
        }
    }
}
