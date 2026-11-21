//! 数据同步执行账本：把「哪张表、哪一块、写了多少、样本行」落到本地 SQLite。
//!
//! 只做回看与排障，不参与同步决策。所有写操作都是最佳努力：失败只记 warn，
//! 绝不让账本问题中断同步。覆盖模式（truncate + 全量写）只记表级汇总，不记块明细。

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use fluxdb_core::{Error, ErrorKind};
use fluxdb_storage::sync_run_store::SAMPLE_LIMIT;
use fluxdb_storage::{
    FileStorage, SyncRunChunkRecord, SyncRunFailureRecord, SyncRunRecord, SyncRunSampleRecord,
    SyncRunTableRecord,
};

use crate::AppController;

use super::model::{
    DataSyncOptions, DataSyncReport, RowDiffAction, RowDiffSample, TableSyncReport,
};

/// 一次执行任务的账本句柄；`run_id` 为 `None` 表示未接存储或写头失败。
pub(crate) struct SyncLedger {
    storage: Option<FileStorage>,
    run_id: Option<i64>,
}

impl SyncLedger {
    /// 开始记录。存储缺失或写头失败时退化为空账本，调用方无需分支。
    pub(crate) fn begin(
        storage: Option<FileStorage>,
        source: &str,
        target: &str,
        signature: &str,
    ) -> Self {
        let run_id = match storage.as_ref() {
            Some(store) => match store.begin_sync_run(source, target, signature, now_ms()) {
                Ok(id) => Some(id),
                Err(error) => {
                    tracing::warn!(target: "fluxdb_app", %error, "数据同步记录：写任务头失败，本次不落账本");
                    None
                }
            },
            None => None,
        };
        Self { storage, run_id }
    }

    /// 本次记录的 run ID；未接存储或写头失败时为 `None`。
    pub(crate) fn run_id(&self) -> Option<i64> {
        self.run_id
    }

    /// 记录一条被跳过的失败行。执行期逐条落盘，宁可慢一点也不丢明细；
    /// 写失败只记 warn，不中断同步。
    pub(crate) fn record_failure(&self, table_key: &str, row_key: &str, message: &str) {
        let (Some(store), Some(run_id)) = (self.storage.as_ref(), self.run_id) else {
            return;
        };
        let failure = SyncRunFailureRecord {
            table_key: table_key.to_string(),
            row_key: row_key.to_string(),
            message: message.to_string(),
        };
        if let Err(error) = store.save_sync_run_failure(run_id, &failure) {
            tracing::warn!(target: "fluxdb_app", %error, table = %table_key, "数据同步记录：写失败行失败");
        }
    }

    /// 单表块记录器；`options` 决定哪些差异样本真正落盘。
    pub(crate) fn table(&self, key: &str, options: &DataSyncOptions) -> TableLedger<'_> {
        TableLedger {
            ledger: self,
            table_key: key.to_string(),
            ordinal: 0,
            samples_left: SAMPLE_LIMIT,
            record_insert: options.insert,
            record_update: options.update,
            record_delete: options.delete,
        }
    }

    /// 写表级汇总。
    pub(crate) fn record_table(&self, report: &TableSyncReport) {
        let (Some(store), Some(run_id)) = (self.storage.as_ref(), self.run_id) else {
            return;
        };
        let state = if report.skipped {
            "skipped"
        } else if report.error.is_some() {
            "failed"
        } else {
            "synced"
        };
        let record = SyncRunTableRecord {
            table_key: report.key.clone(),
            name: report.name.clone(),
            state: state.to_string(),
            inserted: report.inserted,
            updated: report.updated,
            deleted: report.deleted,
            error: report.error.clone(),
        };
        if let Err(error) = store.save_sync_run_table(run_id, &record) {
            tracing::warn!(target: "fluxdb_app", %error, table = %report.key, "数据同步记录：写表汇总失败");
        }
    }

    /// 收尾：写状态与总计数（同时裁剪过老记录）。
    pub(crate) fn finish(&self, status: &str, tables: &[TableSyncReport]) {
        let (Some(store), Some(run_id)) = (self.storage.as_ref(), self.run_id) else {
            return;
        };
        let total = |pick: fn(&TableSyncReport) -> u64| tables.iter().map(pick).sum::<u64>();
        let result = store.finish_sync_run(
            run_id,
            status,
            total(|table| table.inserted),
            total(|table| table.updated),
            total(|table| table.deleted),
            now_ms(),
        );
        if let Err(error) = result {
            tracing::warn!(target: "fluxdb_app", %error, run_id, "数据同步记录：收尾写状态失败");
        }
    }
}

/// 单表块记录器：累计块序号，并按表截断样本。
pub(crate) struct TableLedger<'a> {
    ledger: &'a SyncLedger,
    table_key: String,
    ordinal: u64,
    /// 本表还能写多少条样本（跨块累计上限）。
    samples_left: usize,
    record_insert: bool,
    record_update: bool,
    record_delete: bool,
}

impl TableLedger<'_> {
    /// 记录一块。`key_from` 是本块开区间下界，`key_to` 是闭区间上界（收尾块为空）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &mut self,
        key_from: Option<String>,
        key_to: Option<String>,
        key_to_value: Option<String>,
        inserted: u64,
        updated: u64,
        deleted: u64,
        elapsed_ms: u64,
        samples: &[RowDiffSample],
    ) {
        let (Some(store), Some(run_id)) = (self.ledger.storage.as_ref(), self.ledger.run_id) else {
            return;
        };
        self.ordinal += 1;
        let chunk = SyncRunChunkRecord {
            table_key: self.table_key.clone(),
            ordinal: self.ordinal,
            key_from,
            key_to,
            key_to_value,
            inserted,
            updated,
            deleted,
            elapsed_ms,
        };
        let records = self.sample_records(samples);
        if let Err(error) = store.save_sync_run_chunk(run_id, &chunk, &records) {
            tracing::warn!(target: "fluxdb_app", %error, table = %self.table_key, "数据同步记录：写块明细失败");
        }
    }

    /// 只保留本次真正执行的差异动作，并受每表样本上限约束。
    fn sample_records(&mut self, samples: &[RowDiffSample]) -> Vec<SyncRunSampleRecord> {
        let mut records = Vec::new();
        for sample in samples {
            if self.samples_left == 0 {
                break;
            }
            let allowed = match sample.action {
                RowDiffAction::Insert => self.record_insert,
                RowDiffAction::Update => self.record_update,
                RowDiffAction::Delete => self.record_delete,
            };
            if !allowed {
                continue;
            }
            self.samples_left -= 1;
            records.push(SyncRunSampleRecord {
                table_key: self.table_key.clone(),
                action: action_label(sample.action).to_string(),
                row_key: row_key_label(sample),
                column: sample.column.clone(),
                source_value: sample.source_value.clone(),
                target_value: sample.target_value.clone(),
            });
        }
        records
    }
}

/// 一次执行记录的回看快照：任务头 + 表汇总 + 块明细 + 行级样本。
#[derive(Clone, Debug, PartialEq)]
pub struct SyncRunDetail {
    pub run: SyncRunRecord,
    pub tables: Vec<SyncRunTableRecord>,
    pub chunks: Vec<SyncRunChunkRecord>,
    pub samples: Vec<SyncRunSampleRecord>,
    /// 被跳过的失败行（全量明细，不再受报告 200 条上限约束）。
    pub failures: Vec<SyncRunFailureRecord>,
}

/// 单表续传位置：已完成则整表跳过；否则从 `cursor`（块上界 JSON）之后继续。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SyncResumeTable {
    pub completed: bool,
    pub cursor: Option<String>,
}

/// 一次执行记录的续传计划。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SyncResume {
    pub signature: String,
    pub tables: BTreeMap<String, SyncResumeTable>,
}

impl AppController {
    /// 读取续传计划：表汇总判断完成，块明细给出最后完成的块上界。
    pub(crate) fn load_sync_resume(&self, run_id: i64) -> fluxdb_core::Result<SyncResume> {
        let storage = self
            .sync_run_storage
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Internal, "未启用本地执行记录，无法续传"))?;
        let run = storage
            .load_sync_run(run_id)?
            .ok_or_else(|| Error::new(ErrorKind::Internal, "执行记录不存在或已被清理"))?;
        let tables = storage.load_sync_run_tables(run_id)?;
        let chunks = storage.load_sync_run_chunks(run_id)?;
        let mut plan = BTreeMap::new();
        for chunk in chunks {
            let entry = plan
                .entry(chunk.table_key)
                .or_insert_with(|| SyncResumeTable {
                    completed: false,
                    cursor: None,
                });
            match chunk.key_to_value {
                // 收尾块没有上界：整表已经扫完。
                None => entry.completed = true,
                Some(value) => entry.cursor = Some(value),
            }
        }
        for table in tables {
            let entry = plan
                .entry(table.table_key)
                .or_insert_with(|| SyncResumeTable {
                    completed: false,
                    cursor: None,
                });
            if table.state == "synced" || table.state == "skipped" {
                entry.completed = true;
            }
        }
        Ok(SyncResume {
            signature: run.signature,
            tables: plan,
        })
    }

    /// 最近一次执行记录 ID；取消后用它把「继续」按钮接到刚中断的任务上。
    pub fn latest_sync_run_id(&self) -> Option<i64> {
        self.load_sync_runs(1).first().map(|run| run.id)
    }

    /// 最近的执行记录（新的在前）；未接存储或读取失败时返回空列表。
    pub fn load_sync_runs(&self, limit: usize) -> Vec<SyncRunRecord> {
        let Some(storage) = self.sync_run_storage.as_ref() else {
            return Vec::new();
        };
        match storage.load_sync_runs(limit as i64) {
            Ok(runs) => runs,
            Err(error) => {
                tracing::warn!(target: "fluxdb_app", %error, "数据同步记录：读取任务列表失败");
                Vec::new()
            }
        }
    }

    /// 读取一次执行记录；未接存储或记录已被裁剪时给出可见错误。
    pub fn load_sync_run_detail(&self, run_id: i64) -> fluxdb_core::Result<SyncRunDetail> {
        let storage = self
            .sync_run_storage
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Internal, "未启用本地执行记录，无法回看"))?;
        let run = storage
            .load_sync_run(run_id)?
            .ok_or_else(|| Error::new(ErrorKind::Internal, "执行记录不存在或已被清理"))?;
        Ok(SyncRunDetail {
            run,
            tables: storage.load_sync_run_tables(run_id)?,
            chunks: storage.load_sync_run_chunks(run_id)?,
            samples: storage.load_sync_run_samples(run_id)?,
            failures: storage.load_sync_run_failures(run_id)?,
        })
    }
}

fn action_label(action: RowDiffAction) -> &'static str {
    match action {
        RowDiffAction::Insert => "insert",
        RowDiffAction::Update => "update",
        RowDiffAction::Delete => "delete",
    }
}

/// `id=7, tenant=a`：键列名与展示值成对拼接，便于回看定位。
fn row_key_label(sample: &RowDiffSample) -> String {
    sample
        .key_values
        .iter()
        .map(|(column, value)| format!("{column}={value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 执行报告的整体状态：有表失败记 failed，否则 success。
pub(crate) fn report_status(report: &DataSyncReport) -> &'static str {
    if report.failed_tables().is_empty() {
        "success"
    } else {
        "failed"
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
