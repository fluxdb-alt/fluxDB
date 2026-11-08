// 任务中心：汇总各功能已有的后台任务状态，不新建独立任务执行器。
//
// 备份 / 表数据导出 / 执行 SQL 文件本身会保留完整任务列表，这里只在渲染时投影；
// 恢复、Mongo 导出/导入、表数据导入的会话状态原本随弹窗关闭而丢弃，因此启动时
// 在 `TaskCenterState.records` 留一条轻量记录，弹窗关闭后仍可在任务中心查看。

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// 用“当前时刻 - 年龄”还原历史任务的 Instant；时钟异常时退化为当前时刻。
fn instant_before(now: Instant, now_unix_ms: u64, unix_ms: u64) -> Instant {
    let age = now_unix_ms.saturating_sub(unix_ms);
    now.checked_sub(Duration::from_millis(age)).unwrap_or(now)
}

/// 任务类型。用于分组文案、图标与取消路由；稳定值不参与持久化。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TaskCenterKind {
    Backup,
    Restore,
    SqlFile,
    TableExport,
    TableImport,
    MongoExport,
    MongoImport,
    StructureSync,
    DataTransfer,
}

impl TaskCenterKind {
    /// 持久化稳定键；改名不影响已保存历史。
    fn key(self) -> &'static str {
        match self {
            Self::Backup => "backup",
            Self::Restore => "restore",
            Self::SqlFile => "sql_file",
            Self::TableExport => "table_export",
            Self::TableImport => "table_import",
            Self::MongoExport => "mongo_export",
            Self::MongoImport => "mongo_import",
            Self::StructureSync => "structure_sync",
            Self::DataTransfer => "data_transfer",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "backup" => Self::Backup,
            "restore" => Self::Restore,
            "sql_file" => Self::SqlFile,
            "table_export" => Self::TableExport,
            "table_import" => Self::TableImport,
            "mongo_export" => Self::MongoExport,
            "mongo_import" => Self::MongoImport,
            "structure_sync" => Self::StructureSync,
            "data_transfer" => Self::DataTransfer,
            _ => return None,
        })
    }

    fn label(self) -> SharedString {
        match self {
            Self::Backup => tr("app.task_center.kind.backup"),
            Self::Restore => tr("app.task_center.kind.restore"),
            Self::SqlFile => tr("app.task_center.kind.sql_file"),
            Self::TableExport => tr("app.task_center.kind.table_export"),
            Self::TableImport => tr("app.task_center.kind.table_import"),
            Self::MongoExport => tr("app.task_center.kind.mongo_export"),
            Self::MongoImport => tr("app.task_center.kind.mongo_import"),
            Self::StructureSync => tr("app.task_center.kind.structure_sync"),
            Self::DataTransfer => tr("app.task_center.kind.data_transfer"),
        }
    }

    fn icon(self) -> AppIcon {
        match self {
            Self::Backup => AppIcon::Save,
            Self::Restore => AppIcon::Download,
            Self::SqlFile => AppIcon::FileSql,
            Self::TableExport => AppIcon::Upload,
            Self::TableImport => AppIcon::FolderInput,
            Self::MongoExport => AppIcon::Upload,
            Self::MongoImport => AppIcon::FolderInput,
            Self::StructureSync => AppIcon::Schema,
            Self::DataTransfer => AppIcon::ArrowUpDown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskCenterStatus {
    Running,
    Success,
    Failed,
    Canceled,
}

impl TaskCenterStatus {
    fn label(self, cancel_requested: bool) -> SharedString {
        match self {
            Self::Running if cancel_requested => tr("app.task_center.status.stopping"),
            Self::Running => tr("app.task_center.status.running"),
            Self::Success => tr("app.task_center.status.success"),
            Self::Failed => tr("app.task_center.status.failed"),
            Self::Canceled => tr("app.task_center.status.canceled"),
        }
    }

    fn is_running(self) -> bool {
        self == Self::Running
    }

    fn key(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "success" => Self::Success,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            // 进程中断时运行中的任务不会落盘，这里按失败处理并保留记录。
            _ => Self::Failed,
        }
    }
}

/// 不随弹窗关闭而丢弃的任务记录；日志按行保存，统一截断到 300 行。
#[derive(Clone, Debug)]
struct TaskCenterRecord {
    id: u64,
    kind: TaskCenterKind,
    title: String,
    target: String,
    status: TaskCenterStatus,
    progress: Option<f32>,
    cancel_requested: bool,
    stage: String,
    logs: Vec<String>,
    started_at: Instant,
    finished_at: Option<Instant>,
    error: Option<String>,
    cancel: Option<Arc<AtomicBool>>,
}

impl TaskCenterRecord {
    fn push_log(&mut self, line: String) {
        self.logs.push(line);
        if self.logs.len() > 300 {
            self.logs.remove(0);
        }
    }
}

/// 行来源：运行中的功能任务来自各功能自己的列表，历史行来自持久化记录。
/// 两类 id 空间相互独立，必须分开才能避免选择/删除串行。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskCenterRowSource {
    Live,
    History,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TaskCenterRowKey {
    kind: TaskCenterKind,
    id: u64,
    source: TaskCenterRowSource,
}

impl TaskCenterRowKey {
    fn live(kind: TaskCenterKind, id: u64) -> Self {
        Self {
            kind,
            id,
            source: TaskCenterRowSource::Live,
        }
    }

    fn history(kind: TaskCenterKind, id: u64) -> Self {
        Self {
            kind,
            id,
            source: TaskCenterRowSource::History,
        }
    }
}

#[derive(Default)]
struct TaskCenterState {
    open: bool,
    selected: Option<TaskCenterRowKey>,
    filter: TaskCenterFilter,
    records: Vec<TaskCenterRecord>,
    seq: u64,
    /// 本次会话已快照进历史的运行任务 `(kind, live_id)`，用于隐藏重复的实时行。
    archived: BTreeSet<(TaskCenterKind, u64)>,
}

impl TaskCenterState {
    /// 从持久化历史恢复；`seq` 接着最大 id 递增，避免重启后与新记录撞号。
    fn from_history(records: Vec<TaskHistoryRecord>) -> Self {
        let now = Instant::now();
        let now_ms = unix_ms_now();
        let records = records
            .into_iter()
            .filter_map(|saved| {
                let kind = TaskCenterKind::from_key(&saved.kind)?;
                let started_at = instant_before(now, now_ms, saved.started_unix_ms);
                let finished_at = Some(instant_before(
                    now,
                    now_ms,
                    saved.finished_unix_ms.max(saved.started_unix_ms),
                ));
                Some(TaskCenterRecord {
                    id: saved.id,
                    kind,
                    title: saved.title,
                    target: saved.target,
                    status: TaskCenterStatus::from_key(&saved.status),
                    progress: Some(if saved.status == TaskCenterStatus::Success.key() {
                        1.
                    } else {
                        0.
                    }),
                    cancel_requested: false,
                    stage: saved.stage,
                    logs: saved.logs,
                    started_at,
                    finished_at,
                    error: saved.error,
                    cancel: None,
                })
            })
            .collect::<Vec<_>>();
        let mut state = Self {
            seq: records.iter().map(|record| record.id).max().unwrap_or(0),
            records,
            ..Self::default()
        };
        state.trim_records();
        state
    }

    /// 登记一个需要跨弹窗保留的任务；返回记录 id，供后续进度/收尾更新。
    fn begin(
        &mut self,
        kind: TaskCenterKind,
        title: String,
        target: String,
        cancel: Option<Arc<AtomicBool>>,
    ) -> u64 {
        self.seq += 1;
        let id = self.seq;
        self.records.push(TaskCenterRecord {
            id,
            kind,
            title,
            target,
            status: TaskCenterStatus::Running,
            progress: None,
            cancel_requested: false,
            stage: tr("app.task_center.status.running").to_string(),
            logs: Vec::new(),
            started_at: Instant::now(),
            finished_at: None,
            error: None,
            cancel,
        });
        self.trim_records();
        id
    }

    fn update_progress(
        &mut self,
        id: u64,
        progress: Option<f32>,
        stage: Option<String>,
        log: Option<String>,
    ) {
        let Some(record) = self.records.iter_mut().find(|record| record.id == id) else {
            return;
        };
        if let Some(progress) = progress {
            record.progress = Some(progress.clamp(0., 1.));
        }
        if let Some(stage) = stage {
            record.stage = stage;
        }
        if let Some(log) = log {
            record.push_log(log);
        }
    }

    fn finish(
        &mut self,
        id: u64,
        status: TaskCenterStatus,
        stage: String,
        error: Option<String>,
    ) {
        let Some(record) = self.records.iter_mut().find(|record| record.id == id) else {
            return;
        };
        record.status = status;
        record.stage = stage.clone();
        record.finished_at = Some(Instant::now());
        record.cancel_requested = false;
        if status == TaskCenterStatus::Success {
            record.progress = Some(1.);
        }
        if let Some(error) = &error {
            record.push_log(error.clone());
        }
        record.error = error;
        record.cancel = None;
    }

    fn mark_cancel_requested(&mut self, id: u64) {
        if let Some(record) = self.records.iter_mut().find(|record| record.id == id) {
            record.cancel_requested = true;
            record.stage = tr("app.task_center.status.stopping").to_string();
        }
    }

    /// 把已结束的实时任务写成历史记录；返回新记录 id。
    fn push_history(&mut self, row: &TaskCenterRow) -> u64 {
        self.seq += 1;
        let id = self.seq;
        self.records.push(TaskCenterRecord {
            id,
            kind: row.kind,
            title: row.title.clone(),
            target: row.target.clone(),
            status: row.status,
            progress: row.progress,
            cancel_requested: false,
            stage: row.stage.clone(),
            logs: row.logs.clone(),
            started_at: row.started_at,
            finished_at: row.finished_at.or(Some(Instant::now())),
            error: row.error.clone(),
            cancel: None,
        });
        self.trim_records();
        id
    }

    /// 已结束记录转持久化结构；运行中的任务不落盘，避免重启后出现永假的“运行中”。
    fn history_records(&self) -> Vec<TaskHistoryRecord> {
        let now = Instant::now();
        let now_ms = unix_ms_now();
        self.records
            .iter()
            .filter(|record| !record.status.is_running())
            .map(|record| {
                let finished = record.finished_at.unwrap_or(now);
                let finished_ms = now_ms.saturating_sub(
                    now.saturating_duration_since(finished).as_millis() as u64,
                );
                let started_ms = finished_ms.saturating_sub(
                    finished
                        .saturating_duration_since(record.started_at)
                        .as_millis() as u64,
                );
                TaskHistoryRecord {
                    id: record.id,
                    kind: record.kind.key().to_string(),
                    title: record.title.clone(),
                    target: record.target.clone(),
                    status: record.status.key().to_string(),
                    stage: record.stage.clone(),
                    logs: record.logs.clone(),
                    started_unix_ms: started_ms,
                    finished_unix_ms: finished_ms,
                    error: record.error.clone(),
                }
            })
            .collect()
    }

    /// 最多保留 200 条；优先淘汰最早的已结束记录，运行中的任务不淘汰。
    fn trim_records(&mut self) {
        while self.records.len() > 200
            && let Some(index) = self
                .records
                .iter()
                .position(|record| !record.status.is_running())
        {
            self.records.remove(index);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum TaskCenterFilter {
    #[default]
    All,
    Running,
    Failed,
}

/// 渲染用的统一行；仅覆盖任务中心展示字段，不复制原始任务的完整状态。
struct TaskCenterRow {
    key: TaskCenterRowKey,
    kind: TaskCenterKind,
    title: String,
    target: String,
    status: TaskCenterStatus,
    cancel_requested: bool,
    progress: Option<f32>,
    stage: String,
    logs: Vec<String>,
    started_at: Instant,
    finished_at: Option<Instant>,
    error: Option<String>,
}

impl NavicatMain {
    fn open_task_center(&mut self, cx: &mut Context<Self>) {
        self.task_center.open = true;
        self.task_center.filter = TaskCenterFilter::All;
        // 优先选中最近一条运行中任务，打开即可看到实时进度。
        let rows = self.task_center_rows();
        self.task_center.selected = rows
            .iter()
            .find(|row| row.status.is_running())
            .or_else(|| rows.first())
            .map(|row| row.key);
        cx.notify();
    }

    fn close_task_center(&mut self, cx: &mut Context<Self>) {
        self.task_center.open = false;
        cx.notify();
    }

    fn task_center_rows(&self) -> Vec<TaskCenterRow> {
        let mut rows = Vec::new();
        for task in &self.backup_tasks {
            if self
                .task_center
                .archived
                .contains(&(TaskCenterKind::Backup, task.id))
            {
                continue;
            }
            let database = task.database.clone().unwrap_or_default();
            let status = if task.running() {
                TaskCenterStatus::Running
            } else if task.canceled {
                TaskCenterStatus::Canceled
            } else if task.error.is_some() {
                TaskCenterStatus::Failed
            } else {
                TaskCenterStatus::Success
            };
            rows.push(TaskCenterRow {
                key: TaskCenterRowKey::live(TaskCenterKind::Backup, task.id),
                kind: TaskCenterKind::Backup,
                title: tr("app.task_center.title.backup").replace("%{database}", &database),
                target: task.output_path.display().to_string(),
                status,
                cancel_requested: task.cancel_requested,
                progress: None,
                stage: task.stage.clone(),
                logs: task
                    .logs
                    .iter()
                    .map(|entry| format!("{}：{}", entry.stage, entry.message))
                    .collect(),
                started_at: task.started_at,
                finished_at: task.finished_at,
                error: task.error.clone(),
            });
        }
        for task in &self.sql_file_modal.borrow().tasks {
            if self
                .task_center
                .archived
                .contains(&(TaskCenterKind::SqlFile, task.id))
            {
                continue;
            }
            let status = if task.running() {
                TaskCenterStatus::Running
            } else if task.canceled {
                TaskCenterStatus::Canceled
            } else if task.error.is_some() || task.errors > 0 {
                TaskCenterStatus::Failed
            } else {
                TaskCenterStatus::Success
            };
            let progress = if task.total > 0 {
                Some(task.processed as f32 / task.total as f32)
            } else {
                None
            };
            rows.push(TaskCenterRow {
                key: TaskCenterRowKey::live(TaskCenterKind::SqlFile, task.id),
                kind: TaskCenterKind::SqlFile,
                title: tr("app.task_center.title.sql_file").replace("%{file}", &task.file_name),
                target: task.path.display().to_string(),
                status,
                cancel_requested: task.cancel_requested,
                progress,
                stage: format!("{}/{}", task.processed, task.total),
                logs: task
                    .logs
                    .iter()
                    .map(|entry| format!("#{} {}", entry.index, entry.message))
                    .collect(),
                started_at: task.started_at,
                finished_at: task.finished_at,
                error: task.error.clone(),
            });
        }
        for task in &self.data_export_tasks {
            if self
                .task_center
                .archived
                .contains(&(TaskCenterKind::TableExport, task.id))
            {
                continue;
            }
            let status = if task.running() {
                TaskCenterStatus::Running
            } else if task.canceled {
                TaskCenterStatus::Canceled
            } else if task.error.is_some() {
                TaskCenterStatus::Failed
            } else {
                TaskCenterStatus::Success
            };
            rows.push(TaskCenterRow {
                key: TaskCenterRowKey::live(TaskCenterKind::TableExport, task.id),
                kind: TaskCenterKind::TableExport,
                title: tr("app.task_center.title.table_export")
                    .replace("%{table}", &task.table_name),
                target: task.path.display().to_string(),
                status,
                cancel_requested: task.cancel_requested,
                progress: None,
                stage: tr("app.task_center.stage.rows")
                    .replace("%{rows}", &task.exported_rows.to_string()),
                logs: task.logs.iter().map(|entry| entry.message.clone()).collect(),
                started_at: task.started_at,
                finished_at: task.finished_at,
                error: task.error.clone(),
            });
        }
        for record in &self.task_center.records {
            rows.push(TaskCenterRow {
                key: TaskCenterRowKey::history(record.kind, record.id),
                kind: record.kind,
                title: record.title.clone(),
                target: record.target.clone(),
                status: record.status,
                cancel_requested: record.cancel_requested,
                progress: record.progress,
                stage: record.stage.clone(),
                logs: record.logs.clone(),
                started_at: record.started_at,
                finished_at: record.finished_at,
                error: record.error.clone(),
            });
        }
        // 运行中任务固定在上，其余按开始时间倒序，避免状态变化时行序跳动。
        rows.sort_by(|left, right| {
            right
                .status
                .is_running()
                .cmp(&left.status.is_running())
                .then_with(|| right.started_at.cmp(&left.started_at))
        });
        rows
    }

    fn selected_task_center_row(&self) -> Option<TaskCenterRow> {
        let selected = self.task_center.selected?;
        self.task_center_rows()
            .into_iter()
            .find(|row| row.key == selected)
    }

    fn cancel_task_center_task(&mut self, key: TaskCenterRowKey, cx: &mut Context<Self>) {
        match (key.source, key.kind) {
            (TaskCenterRowSource::Live, TaskCenterKind::Backup) => {
                self.cancel_backup_task(key.id, cx)
            }
            (TaskCenterRowSource::Live, TaskCenterKind::SqlFile) => {
                self.cancel_sql_file_task(key.id, cx)
            }
            (TaskCenterRowSource::Live, TaskCenterKind::TableExport) => {
                self.cancel_table_data_export_task(key.id, cx)
            }
            _ => {
                if let Some(record) = self
                    .task_center
                    .records
                    .iter_mut()
                    .find(|record| record.id == key.id && record.kind == key.kind)
                    && let Some(flag) = &record.cancel
                {
                    flag.store(true, Ordering::Relaxed);
                }
                self.task_center.mark_cancel_requested(key.id);
                self.show_message(
                    tr("app.task_center.cancel_requested"),
                    AppMessageKind::Warning,
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// 删除已结束任务记录。运行中的任务必须先停止，避免记录消失但后台仍在写入。
    fn delete_task_center_task(
        &mut self,
        key: TaskCenterRowKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let running = match (key.source, key.kind) {
            (TaskCenterRowSource::Live, TaskCenterKind::Backup) => self
                .backup_tasks
                .iter()
                .any(|task| task.id == key.id && task.running()),
            (TaskCenterRowSource::Live, TaskCenterKind::SqlFile) => self
                .sql_file_modal
                .borrow()
                .tasks
                .iter()
                .any(|task| task.id == key.id && task.running()),
            (TaskCenterRowSource::Live, TaskCenterKind::TableExport) => self
                .data_export_tasks
                .iter()
                .any(|task| task.id == key.id && task.running()),
            _ => self.task_center.records.iter().any(|record| {
                record.id == key.id
                    && record.kind == key.kind
                    && record.status == TaskCenterStatus::Running
            }),
        };
        if running {
            self.show_message(
                tr("app.task_center.cannot_delete_running"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        match (key.source, key.kind) {
            (TaskCenterRowSource::Live, TaskCenterKind::Backup) => {
                self.clear_backup_task(key.id, cx)
            }
            (TaskCenterRowSource::Live, TaskCenterKind::SqlFile) => {
                self.clear_sql_file_task(key.id, window, cx)
            }
            (TaskCenterRowSource::Live, TaskCenterKind::TableExport) => {
                self.clear_table_data_export_task(key.id, cx)
            }
            _ => {
                self.task_center
                    .records
                    .retain(|record| !(record.id == key.id && record.kind == key.kind));
                self.persist_task_center_history();
            }
        }
        if self.task_center.selected == Some(key) {
            self.task_center.selected = None;
        }
        cx.notify();
    }

    /// 结束记录并立即落盘，保证重启后仍能看到历史。
    fn finish_task_center_record(
        &mut self,
        id: u64,
        status: TaskCenterStatus,
        stage: String,
        error: Option<String>,
    ) {
        self.task_center.finish(id, status, stage, error);
        self.persist_task_center_history();
    }

    /// 已结束任务写入 sqlite；失败只记日志，不影响界面。
    fn persist_task_center_history(&self) {
        let records = self.task_center.history_records();
        if let Err(error) = self.storage.save_task_history(&records) {
            tracing::warn!(error = %error, "任务中心历史写入失败");
        }
    }

    /// 把刚结束的实时任务快照存入历史，并隐藏对应实时行，避免任务中心出现重复记录。
    fn archive_task_center_live_task(&mut self, kind: TaskCenterKind, id: u64) {
        if self.task_center.archived.contains(&(kind, id)) {
            return;
        }
        let row = self
            .task_center_rows()
            .into_iter()
            .find(|row| row.key == TaskCenterRowKey::live(kind, id));
        if let Some(row) = row
            && !row.status.is_running()
        {
            self.task_center.push_history(&row);
            self.task_center.archived.insert((kind, id));
            self.persist_task_center_history();
        }
    }
}

include!("task_center/ui.rs");
