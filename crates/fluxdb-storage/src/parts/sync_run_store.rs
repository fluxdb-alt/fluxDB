//! 数据同步执行记录（本地审计账本）。
//!
//! 执行阶段逐块写入：`sync_runs`（一次任务）→ `sync_run_tables`（表汇总）→
//! `sync_run_chunks`（块明细）→ `sync_run_samples`（行级样本）。
//!
//! 只给回看/排障用，不参与同步决策；写账本失败不能中断同步，由调用方降级为日志。
//! 行级样本按表截断（[`SAMPLE_LIMIT`]），计数始终精确——大表不该把整表差异再复制一份。

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use fluxdb_core::Result;

use crate::sqlite::sqlite_error;

/// 建表语句（幂等；由存储层建表入口执行）。
pub const CREATE_SQL: &str = "
CREATE TABLE IF NOT EXISTS sync_runs (
    id               INTEGER PRIMARY KEY,   -- rowid，兼作 run ID
    started_unix_ms  INTEGER NOT NULL,
    finished_unix_ms INTEGER NOT NULL DEFAULT 0,
    status           TEXT    NOT NULL,      -- running / success / failed / canceled
    source           TEXT    NOT NULL,      -- 源端点展示文本（连接/库/schema）
    target           TEXT    NOT NULL,
    inserted         INTEGER NOT NULL DEFAULT 0,
    updated          INTEGER NOT NULL DEFAULT 0,
    deleted          INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sync_run_tables (
    run_id    INTEGER NOT NULL,
    table_key TEXT    NOT NULL,
    name      TEXT    NOT NULL,
    state     TEXT    NOT NULL,             -- synced / skipped / failed
    inserted  INTEGER NOT NULL DEFAULT 0,
    updated   INTEGER NOT NULL DEFAULT 0,
    deleted   INTEGER NOT NULL DEFAULT 0,
    error     TEXT,
    PRIMARY KEY (run_id, table_key)
);
CREATE TABLE IF NOT EXISTS sync_run_chunks (
    run_id    INTEGER NOT NULL,
    table_key TEXT    NOT NULL,
    ordinal   INTEGER NOT NULL,             -- 表内块序号，从 1 开始
    key_from  TEXT,                         -- 本块键下界（开区间），首页为空
    key_to    TEXT,                         -- 本块键上界（闭区间），收尾块为空
    inserted  INTEGER NOT NULL DEFAULT 0,
    updated   INTEGER NOT NULL DEFAULT 0,
    deleted   INTEGER NOT NULL DEFAULT 0,
    elapsed_ms INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, table_key, ordinal)
);
CREATE TABLE IF NOT EXISTS sync_run_samples (
    run_id       INTEGER NOT NULL,
    table_key    TEXT    NOT NULL,
    action       TEXT    NOT NULL,          -- insert / update / delete
    row_key      TEXT    NOT NULL,
    column_name  TEXT,
    source_value TEXT,
    target_value TEXT
);
CREATE INDEX IF NOT EXISTS idx_sync_run_samples
    ON sync_run_samples(run_id, table_key);
";

/// 每个表最多落盘的行级样本数，与预览的样本上限保持一致。
pub const SAMPLE_LIMIT: usize = 100;

/// 保留最近多少次执行记录，更早的整批删除（样本行随之删除）。
pub const KEEP_RUNS: i64 = 20;

/// 一次同步任务的头部记录。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncRunRecord {
    pub id: i64,
    pub started_unix_ms: u64,
    pub finished_unix_ms: u64,
    pub status: String,
    pub source: String,
    pub target: String,
    pub inserted: u64,
    pub updated: u64,
    pub deleted: u64,
}

/// 单表汇总。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncRunTableRecord {
    pub table_key: String,
    pub name: String,
    pub state: String,
    pub inserted: u64,
    pub updated: u64,
    pub deleted: u64,
    pub error: Option<String>,
}

/// 单块明细；`key_from` / `key_to` 是键首列的展示文本，便于回看「这一块扫了哪段」。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncRunChunkRecord {
    pub table_key: String,
    pub ordinal: u64,
    pub key_from: Option<String>,
    pub key_to: Option<String>,
    pub inserted: u64,
    pub updated: u64,
    pub deleted: u64,
    pub elapsed_ms: u64,
}

/// 行级样本：`column` 为空表示整行级（新增/删除，或字段级细节未保留）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncRunSampleRecord {
    pub table_key: String,
    pub action: String,
    pub row_key: String,
    pub column: Option<String>,
    pub source_value: Option<String>,
    pub target_value: Option<String>,
}

/// 开始一次记录，返回 run ID。
pub(crate) fn begin(
    conn: &Connection,
    source: &str,
    target: &str,
    started_unix_ms: u64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO sync_runs (started_unix_ms, status, source, target)
         VALUES (?1, 'running', ?2, ?3)",
        params![started_unix_ms as i64, source, target],
    )
    .map_err(sqlite_error)?;
    Ok(conn.last_insert_rowid())
}

/// 收尾：写状态与总计数，并裁剪过老的记录。
pub(crate) fn finish(
    conn: &Connection,
    run_id: i64,
    status: &str,
    inserted: u64,
    updated: u64,
    deleted: u64,
    finished_unix_ms: u64,
) -> Result<()> {
    conn.execute(
        "UPDATE sync_runs
            SET status = ?1, finished_unix_ms = ?2, inserted = ?3, updated = ?4, deleted = ?5
          WHERE id = ?6",
        params![
            status,
            finished_unix_ms as i64,
            inserted as i64,
            updated as i64,
            deleted as i64,
            run_id
        ],
    )
    .map_err(sqlite_error)?;
    prune(conn, KEEP_RUNS)
}

/// 只保留最近 `keep` 次记录；样本与块明细随 run 一起删除。
pub(crate) fn prune(conn: &Connection, keep: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM sync_run_samples WHERE run_id IN (
             SELECT id FROM sync_runs ORDER BY id DESC LIMIT -1 OFFSET ?1)",
        params![keep.max(1)],
    )
    .map_err(sqlite_error)?;
    conn.execute(
        "DELETE FROM sync_run_chunks WHERE run_id IN (
             SELECT id FROM sync_runs ORDER BY id DESC LIMIT -1 OFFSET ?1)",
        params![keep.max(1)],
    )
    .map_err(sqlite_error)?;
    conn.execute(
        "DELETE FROM sync_run_tables WHERE run_id IN (
             SELECT id FROM sync_runs ORDER BY id DESC LIMIT -1 OFFSET ?1)",
        params![keep.max(1)],
    )
    .map_err(sqlite_error)?;
    conn.execute(
        "DELETE FROM sync_runs WHERE id IN (
             SELECT id FROM sync_runs ORDER BY id DESC LIMIT -1 OFFSET ?1)",
        params![keep.max(1)],
    )
    .map_err(sqlite_error)?;
    Ok(())
}

/// 写入（或覆盖）单表汇总。
pub(crate) fn upsert_table(
    conn: &Connection,
    run_id: i64,
    table: &SyncRunTableRecord,
) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_run_tables
             (run_id, table_key, name, state, inserted, updated, deleted, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(run_id, table_key) DO UPDATE SET
             name = excluded.name, state = excluded.state,
             inserted = excluded.inserted, updated = excluded.updated,
             deleted = excluded.deleted, error = excluded.error",
        params![
            run_id,
            table.table_key,
            table.name,
            table.state,
            table.inserted as i64,
            table.updated as i64,
            table.deleted as i64,
            table.error,
        ],
    )
    .map_err(sqlite_error)?;
    Ok(())
}

/// 追加一块明细。
pub(crate) fn insert_chunk(
    conn: &Connection,
    run_id: i64,
    chunk: &SyncRunChunkRecord,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sync_run_chunks
             (run_id, table_key, ordinal, key_from, key_to, inserted, updated, deleted, elapsed_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            run_id,
            chunk.table_key,
            chunk.ordinal as i64,
            chunk.key_from,
            chunk.key_to,
            chunk.inserted as i64,
            chunk.updated as i64,
            chunk.deleted as i64,
            chunk.elapsed_ms as i64,
        ],
    )
    .map_err(sqlite_error)?;
    Ok(())
}

/// 追加行级样本（调用方已按表截断）。
pub(crate) fn insert_samples(
    conn: &Connection,
    run_id: i64,
    samples: &[SyncRunSampleRecord],
) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "INSERT INTO sync_run_samples
                 (run_id, table_key, action, row_key, column_name, source_value, target_value)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(sqlite_error)?;
    for sample in samples {
        stmt.execute(params![
            run_id,
            sample.table_key,
            sample.action,
            sample.row_key,
            sample.column,
            sample.source_value,
            sample.target_value,
        ])
        .map_err(sqlite_error)?;
    }
    Ok(())
}

/// 最近 `limit` 次执行记录，新的在前。
pub(crate) fn load_runs(conn: &Connection, limit: i64) -> Result<Vec<SyncRunRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, started_unix_ms, finished_unix_ms, status, source, target,
                    inserted, updated, deleted
               FROM sync_runs ORDER BY id DESC LIMIT ?1",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map(params![limit.max(1)], |row| {
            Ok(SyncRunRecord {
                id: row.get(0)?,
                started_unix_ms: row.get::<_, i64>(1)?.max(0) as u64,
                finished_unix_ms: row.get::<_, i64>(2)?.max(0) as u64,
                status: row.get(3)?,
                source: row.get(4)?,
                target: row.get(5)?,
                inserted: row.get::<_, i64>(6)?.max(0) as u64,
                updated: row.get::<_, i64>(7)?.max(0) as u64,
                deleted: row.get::<_, i64>(8)?.max(0) as u64,
            })
        })
        .map_err(sqlite_error)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sqlite_error)
}

/// 按 ID 读取单次任务；不存在返回 `None`（记录可能已被裁剪）。
pub(crate) fn load_run(conn: &Connection, run_id: i64) -> Result<Option<SyncRunRecord>> {
    conn.query_row(
        "SELECT id, started_unix_ms, finished_unix_ms, status, source, target,
                inserted, updated, deleted
           FROM sync_runs WHERE id = ?1",
        params![run_id],
        |row| {
            Ok(SyncRunRecord {
                id: row.get(0)?,
                started_unix_ms: row.get::<_, i64>(1)?.max(0) as u64,
                finished_unix_ms: row.get::<_, i64>(2)?.max(0) as u64,
                status: row.get(3)?,
                source: row.get(4)?,
                target: row.get(5)?,
                inserted: row.get::<_, i64>(6)?.max(0) as u64,
                updated: row.get::<_, i64>(7)?.max(0) as u64,
                deleted: row.get::<_, i64>(8)?.max(0) as u64,
            })
        },
    )
    .optional()
    .map_err(sqlite_error)
}

/// 某次任务的表汇总，按表键排序。
pub(crate) fn load_tables(conn: &Connection, run_id: i64) -> Result<Vec<SyncRunTableRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT table_key, name, state, inserted, updated, deleted, error
               FROM sync_run_tables WHERE run_id = ?1 ORDER BY table_key",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map(params![run_id], |row| {
            Ok(SyncRunTableRecord {
                table_key: row.get(0)?,
                name: row.get(1)?,
                state: row.get(2)?,
                inserted: row.get::<_, i64>(3)?.max(0) as u64,
                updated: row.get::<_, i64>(4)?.max(0) as u64,
                deleted: row.get::<_, i64>(5)?.max(0) as u64,
                error: row.get(6)?,
            })
        })
        .map_err(sqlite_error)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sqlite_error)
}

/// 某次任务的块明细，按表键 + 块序号排序。
pub(crate) fn load_chunks(conn: &Connection, run_id: i64) -> Result<Vec<SyncRunChunkRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT table_key, ordinal, key_from, key_to, inserted, updated, deleted, elapsed_ms
               FROM sync_run_chunks WHERE run_id = ?1 ORDER BY table_key, ordinal",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map(params![run_id], |row| {
            Ok(SyncRunChunkRecord {
                table_key: row.get(0)?,
                ordinal: row.get::<_, i64>(1)?.max(0) as u64,
                key_from: row.get(2)?,
                key_to: row.get(3)?,
                inserted: row.get::<_, i64>(4)?.max(0) as u64,
                updated: row.get::<_, i64>(5)?.max(0) as u64,
                deleted: row.get::<_, i64>(6)?.max(0) as u64,
                elapsed_ms: row.get::<_, i64>(7)?.max(0) as u64,
            })
        })
        .map_err(sqlite_error)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sqlite_error)
}

/// 某次任务的行级样本。
pub(crate) fn load_samples(conn: &Connection, run_id: i64) -> Result<Vec<SyncRunSampleRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT table_key, action, row_key, column_name, source_value, target_value
               FROM sync_run_samples WHERE run_id = ?1 ORDER BY rowid",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map(params![run_id], |row| {
            Ok(SyncRunSampleRecord {
                table_key: row.get(0)?,
                action: row.get(1)?,
                row_key: row.get(2)?,
                column: row.get(3)?,
                source_value: row.get(4)?,
                target_value: row.get(5)?,
            })
        })
        .map_err(sqlite_error)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sqlite_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库");
        conn.execute_batch(CREATE_SQL).expect("建表");
        conn
    }

    fn chunk(ordinal: u64) -> SyncRunChunkRecord {
        SyncRunChunkRecord {
            table_key: "items".into(),
            ordinal,
            key_from: None,
            key_to: Some(format!("{ordinal}000")),
            inserted: 1,
            updated: 0,
            deleted: 0,
            elapsed_ms: 5,
        }
    }

    #[test]
    fn run_roundtrip_records_tables_chunks_and_samples() {
        let conn = memory();
        let run = begin(&conn, "src/main", "dst/main", 100).expect("begin");
        upsert_table(
            &conn,
            run,
            &SyncRunTableRecord {
                table_key: "items".into(),
                name: "items".into(),
                state: "synced".into(),
                inserted: 2,
                updated: 1,
                deleted: 1,
                error: None,
            },
        )
        .expect("table");
        insert_chunk(&conn, run, &chunk(1)).expect("chunk");
        insert_samples(
            &conn,
            run,
            &[SyncRunSampleRecord {
                table_key: "items".into(),
                action: "update".into(),
                row_key: "id=7".into(),
                column: Some("name".into()),
                source_value: Some("new".into()),
                target_value: Some("old".into()),
            }],
        )
        .expect("samples");
        finish(&conn, run, "success", 2, 1, 1, 200).expect("finish");

        let runs = load_runs(&conn, 10).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "success");
        assert_eq!(runs[0].finished_unix_ms, 200);
        assert_eq!(
            (runs[0].inserted, runs[0].updated, runs[0].deleted),
            (2, 1, 1)
        );
        assert_eq!(load_tables(&conn, run).expect("tables").len(), 1);
        assert_eq!(
            load_chunks(&conn, run).expect("chunks")[0].key_to,
            Some("1000".into())
        );
        let samples = load_samples(&conn, run).expect("samples");
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].column.as_deref(), Some("name"));
    }

    #[test]
    fn prune_keeps_only_recent_runs() {
        let conn = memory();
        for index in 0..4 {
            let run = begin(&conn, "s", "t", index).expect("begin");
            finish(&conn, run, "success", 0, 0, 0, index).expect("finish");
        }
        prune(&conn, 2).expect("prune");
        let runs = load_runs(&conn, 10).expect("runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].started_unix_ms, 3, "保留最新的两次");
    }
}
