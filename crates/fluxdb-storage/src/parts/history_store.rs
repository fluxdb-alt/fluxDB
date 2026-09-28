//! `history` 表：所有数据库类型的执行历史统一落一张表。
//!
//! 建表口径（对数据库类型中立，新增类型不改表结构）：
//! - `kind` 是**数据库类型**（mysql / postgres / redis / mongo…，写入时按连接解析），
//!   `category` 是**历史类别**（哪个入口来的：SQL 查询 / Redis 命令 / Key 搜索），
//!   `operation_kind` 是**规范化操作类别**（这条记录干了什么，见
//!   [`fluxdb_core::OperationKind`]）；
//! - 只有跨类型通用的字段才建列（作用域、载荷、成败、时间、结果量级、摘要、来源）；
//! - 类别专属字段统一进 `extra_json`，按 `(kind, category)` 解释；
//! - 每个作用域（连接 + 类别 + 库 + 命名空间）保留 [`PER_SCOPE_LIMIT`] 条，超出即裁剪。

use std::collections::HashMap;

use rusqlite::{Connection, params, params_from_iter};
use serde::{Deserialize, Serialize};

use fluxdb_core::{ConnectionId, Error, ErrorKind, OperationKind, QueryRollbackSnapshot, Result};

use super::connection_store;
use crate::sqlite::sqlite_error;

/// 建表语句（幂等；由 [`crate::sqlite::create_schema`] 执行）。
pub const CREATE_SQL: &str = "
CREATE TABLE IF NOT EXISTS history (
    id                    INTEGER PRIMARY KEY,       -- rowid，兼作记录 ID
    kind                  TEXT    NOT NULL,          -- 数据库类型：mysql|postgres|sqlite|redis|mongo…
    category              TEXT    NOT NULL,          -- 历史类别：query|redis_command|redis_key_search
    operation_kind        TEXT,                      -- 规范化操作类别；无操作语义的类别为 NULL
    connection_id         INTEGER NOT NULL,
    database              TEXT,                      -- 主作用域：库名 / Redis 逻辑库序号 / Mongo database
    namespace             TEXT,                      -- 次作用域：PG schema / Mongo collection / 其它二级命名
    text                  TEXT    NOT NULL,          -- 载荷：SQL / 命令 / 搜索词
    success               INTEGER,                   -- 0/1；无成败语义的类别为 NULL
    executed_at_unix_secs INTEGER NOT NULL DEFAULT 0,
    elapsed_ms            INTEGER NOT NULL DEFAULT 0,
    returned_rows         INTEGER NOT NULL DEFAULT 0,
    affected_rows         INTEGER NOT NULL DEFAULT 0,
    summary               TEXT,                      -- 各类型渲染的可读摘要（列表展示用）
    source                TEXT,                      -- 来源：Workbench|HistoryRerun|KeyShortcut…
    extra_json            TEXT    NOT NULL DEFAULT '{}'  -- 类别专属字段，按 (kind, category) 解释
);
CREATE INDEX IF NOT EXISTS idx_history_scope
    ON history(kind, connection_id, category, database, namespace, id DESC);
";

/// 历史类别：SQL 工作台查询历史。
pub(crate) const CATEGORY_QUERY: &str = "query";
/// 历史类别：Redis Workbench 命令历史。
pub(crate) const CATEGORY_REDIS_COMMAND: &str = "redis_command";
/// 历史类别：Redis Key 搜索历史。
pub(crate) const CATEGORY_REDIS_KEY_SEARCH: &str = "redis_key_search";

/// 每个作用域保留的历史条数上限。
pub(crate) const PER_SCOPE_LIMIT: i64 = 1000;

/// 一条待落盘的历史行（`id` 由数据库分配，写入时忽略；加载时带回 rowid）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HistoryRow {
    /// 加载时带回的记录 ID；写入时为 0（由 rowid 分配）。
    pub id: i64,
    pub operation_kind: Option<OperationKind>,
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    pub namespace: Option<String>,
    pub text: String,
    pub success: Option<bool>,
    /// `None` 表示该类别不携带时间：沿用同键旧行的时间，没有旧行则记当前时间。
    pub executed_at_unix_secs: Option<u64>,
    pub elapsed_ms: u64,
    pub returned_rows: u64,
    pub affected_rows: u64,
    pub summary: Option<String>,
    pub source: Option<String>,
    /// 类别专属字段（JSON object，按 `kind` + `category` 解释）。
    pub extra_json: String,
}

/// 作用域去重键：连接 + 主作用域 + 次作用域 + 文本。
type ScopeKey = (u64, String, String, String);

/// 全量替换某类别的历史（调用方负责事务）。
///
/// 与旧的 blob 覆盖语义一致：调用方传入的是该类别的完整列表。整表替换必须**无损**，
/// 因此对不携带时间的类别（Key 搜索）沿用同键旧行的时间，避免每次落盘都把时间刷新一遍。
pub(crate) fn replace_category(
    conn: &Connection,
    category: &str,
    rows: &[HistoryRow],
) -> Result<()> {
    let carried = existing_timestamps(conn, category)?;
    let kinds = connection_store::kind_map(conn)?;
    conn.execute("DELETE FROM history WHERE category = ?1", params![category])
        .map_err(sqlite_error)?;
    let mut stmt = conn
        .prepare(
            "INSERT INTO history
                 (kind, category, operation_kind, connection_id, database, namespace, text,
                  success, executed_at_unix_secs, elapsed_ms, returned_rows, affected_rows,
                  summary, source, extra_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )
        .map_err(sqlite_error)?;
    for row in rows {
        let kind = kinds.get(&row.connection_id.0).map(String::as_str);
        let kind = kind.unwrap_or(connection_store::UNKNOWN_KIND);
        let executed_at = row.executed_at_unix_secs.unwrap_or_else(|| {
            carried
                .get(&scope_key(row))
                .copied()
                .unwrap_or_else(now_unix_secs)
        });
        stmt.execute(params![
            kind,
            category,
            row.operation_kind.map(OperationKind::as_str),
            row.connection_id.0 as i64,
            row.database,
            row.namespace,
            row.text,
            row.success.map(i64::from),
            executed_at as i64,
            row.elapsed_ms as i64,
            row.returned_rows as i64,
            row.affected_rows as i64,
            row.summary,
            row.source,
            row.extra_json,
        ])
        .map_err(sqlite_error)?;
    }
    prune_category(conn, category)
}

/// 读取某类别的全部历史（按 id 升序，即写入顺序）。
pub(crate) fn load_category(conn: &Connection, category: &str) -> Result<Vec<HistoryRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, operation_kind, connection_id, database, namespace, text, success,
                    executed_at_unix_secs, elapsed_ms, returned_rows, affected_rows,
                    summary, source, extra_json
               FROM history
              WHERE category = ?1
              ORDER BY id",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map(params![category], |row| {
            Ok(HistoryRow {
                id: row.get(0)?,
                operation_kind: row
                    .get::<_, Option<String>>(1)?
                    .map(|value| OperationKind::from_storage(&value)),
                connection_id: ConnectionId(row.get::<_, i64>(2)? as u64),
                database: row.get(3)?,
                namespace: row.get(4)?,
                text: row.get(5)?,
                success: row.get::<_, Option<i64>>(6)?.map(|value| value != 0),
                executed_at_unix_secs: Some(row.get::<_, i64>(7)? as u64),
                elapsed_ms: row.get::<_, i64>(8)? as u64,
                returned_rows: row.get::<_, i64>(9)? as u64,
                affected_rows: row.get::<_, i64>(10)? as u64,
                summary: row.get(11)?,
                source: row.get(12)?,
                extra_json: row.get(13)?,
            })
        })
        .map_err(sqlite_error)?;
    let mut history = Vec::new();
    for row in rows {
        history.push(row.map_err(sqlite_error)?);
    }
    Ok(history)
}

/// 删除已不存在的连接的历史（连接删除时级联清理，调用方负责事务）。
///
/// 历史属于连接，连接没了的历史既打不开也删不掉，因此随连接一起清掉。
pub(crate) fn delete_history_of_missing_connections(
    conn: &Connection,
    keep: &[ConnectionId],
) -> Result<()> {
    if keep.is_empty() {
        conn.execute("DELETE FROM history", [])
            .map_err(sqlite_error)?;
        return Ok(());
    }
    let placeholders = vec!["?"; keep.len()].join(", ");
    let ids = keep.iter().map(|id| id.0 as i64).collect::<Vec<_>>();
    conn.execute(
        &format!("DELETE FROM history WHERE connection_id NOT IN ({placeholders})"),
        params_from_iter(ids),
    )
    .map_err(sqlite_error)?;
    Ok(())
}

/// 类别内按作用域裁剪到 [`PER_SCOPE_LIMIT`]（窗口函数按 id 倒序取每个作用域的前 N 条）。
fn prune_category(conn: &Connection, category: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM history
          WHERE category = ?1
            AND id IN (
                SELECT id FROM (
                    SELECT id,
                           ROW_NUMBER() OVER (
                               PARTITION BY kind, connection_id, category,
                                            IFNULL(database, ''), IFNULL(namespace, '')
                               ORDER BY id DESC
                           ) AS row_no
                      FROM history
                     WHERE category = ?1
                )
                WHERE row_no > ?2
            )",
        params![category, PER_SCOPE_LIMIT],
    )
    .map_err(sqlite_error)?;
    Ok(())
}

/// 读取该类别中带时间的行，供整表替换时沿用（键为作用域 + 文本）。
fn existing_timestamps(conn: &Connection, category: &str) -> Result<HashMap<ScopeKey, u64>> {
    let mut stmt = conn
        .prepare(
            "SELECT connection_id, IFNULL(database, ''), IFNULL(namespace, ''), text,
                    executed_at_unix_secs
               FROM history
              WHERE category = ?1 AND executed_at_unix_secs > 0",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map(params![category], |row| {
            Ok((
                (
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ),
                row.get::<_, i64>(4)? as u64,
            ))
        })
        .map_err(sqlite_error)?;
    let mut timestamps = HashMap::new();
    for row in rows {
        let (key, executed_at) = row.map_err(sqlite_error)?;
        timestamps.insert(key, executed_at);
    }
    Ok(timestamps)
}

fn scope_key(row: &HistoryRow) -> ScopeKey {
    (
        row.connection_id.0,
        row.database.clone().unwrap_or_default(),
        row.namespace.clone().unwrap_or_default(),
        row.text.clone(),
    )
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

// ---------- SQL 查询历史（category='query'） ----------

/// SQL 查询历史单条记录（落 `history` 表，`category='query'`）。
///
/// 通用字段进列，SQL 专属字段（涉及表、回滚快照、事务状态…）进 `extra_json`，
/// 便于将来接入更多 SQL 方言或非 SQL 类型而不改表结构。
#[derive(Clone, Debug, PartialEq)]
pub struct QueryHistoryRecord {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// schema 作用域（PG），落 `namespace` 列；MySQL/TiDB 恒为 None。
    pub schema: Option<String>,
    pub text: String,
    pub tables: Vec<String>,
    /// 规范化操作类别（只读 / 改数据 / 改结构…），落 `operation_kind` 列。
    pub operation_kind: OperationKind,
    pub success: bool,
    pub executed_at_unix_secs: u64,
    pub object: Option<String>,
    pub rollback_sql: Option<String>,
    pub rollback_snapshot: Option<QueryRollbackSnapshot>,
    /// 写入事务状态（committed/uncommitted/rolled_back，§8.4）。
    pub transaction_state: Option<String>,
    pub message: Option<String>,
    pub returned_rows: u64,
    pub affected_rows: u64,
    pub elapsed_ms: u64,
}

/// `extra_json` 的内容：SQL 专属字段，全部可缺省。
#[derive(Debug, Default, Serialize, Deserialize)]
struct QueryHistoryExtra {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    object: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rollback_sql: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rollback_snapshot: Option<QueryRollbackSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transaction_state: Option<String>,
}

/// 空 extra（无类别专属字段的类别用）。
pub(crate) const EMPTY_EXTRA: &str = "{}";

pub(crate) fn query_to_row(record: &QueryHistoryRecord) -> Result<HistoryRow> {
    let extra = QueryHistoryExtra {
        tables: record.tables.clone(),
        object: record.object.clone(),
        rollback_sql: record.rollback_sql.clone(),
        rollback_snapshot: record.rollback_snapshot.clone(),
        transaction_state: record.transaction_state.clone(),
    };
    Ok(HistoryRow {
        id: 0,
        operation_kind: Some(record.operation_kind),
        connection_id: record.connection_id,
        database: record.database.clone(),
        namespace: record.schema.clone(),
        text: record.text.clone(),
        success: Some(record.success),
        executed_at_unix_secs: Some(record.executed_at_unix_secs),
        elapsed_ms: record.elapsed_ms,
        returned_rows: record.returned_rows,
        affected_rows: record.affected_rows,
        summary: record.message.clone(),
        source: None,
        extra_json: serde_json::to_string(&extra)
            .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?,
    })
}

pub(crate) fn query_from_row(row: HistoryRow) -> Result<QueryHistoryRecord> {
    let extra: QueryHistoryExtra = serde_json::from_str(&row.extra_json).map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("查询历史 {}(extra) 无法解析: {error}", row.id),
        )
    })?;
    Ok(QueryHistoryRecord {
        connection_id: row.connection_id,
        database: row.database,
        schema: row.namespace,
        text: row.text,
        tables: extra.tables,
        operation_kind: row.operation_kind.unwrap_or_default(),
        success: row.success.unwrap_or(true),
        executed_at_unix_secs: row.executed_at_unix_secs.unwrap_or(0),
        object: extra.object,
        rollback_sql: extra.rollback_sql,
        rollback_snapshot: extra.rollback_snapshot,
        transaction_state: extra.transaction_state,
        message: row.summary,
        returned_rows: row.returned_rows,
        affected_rows: row.affected_rows,
        elapsed_ms: row.elapsed_ms,
    })
}

// ---------- Redis 命令历史 / Key 搜索历史 ----------

/// Redis Workbench 命令历史单条记录（`category='redis_command'`），按连接 + 逻辑库隔离。
///
/// - `id`：加载时带回的 rowid（用于删除定位）；写入时忽略，由数据库分配。
/// - `text`：命令文本，可回填到 Workbench 输入框。
/// - `summary` / `source`：结果摘要与来源（Workbench / HistoryRerun / KeyShortcut）。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisWorkbenchHistoryRecord {
    pub id: u64,
    pub connection_id: ConnectionId,
    pub database: u32,
    pub text: String,
    pub success: bool,
    pub executed_at_unix_secs: u64,
    pub summary: String,
    pub source: String,
}

/// Redis Key 搜索历史单条记录（`category='redis_key_search'`），按连接 + 数据库隔离。
///
/// - `database`：Redis DB 索引字符串（如 `"0"`、`"1"`），`None` 视为 `"0"`。
/// - `text`：搜索词。该类别不携带执行时间，落盘时按「沿用同键旧行时间，否则记当前时间」。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisKeySearchHistoryRecord {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    pub text: String,
}

pub(crate) fn redis_command_to_row(record: &RedisWorkbenchHistoryRecord) -> HistoryRow {
    HistoryRow {
        id: 0,
        // 一条历史可能含多条命令（混合读写），当前不做命令级分类，统一按「无法归类」。
        operation_kind: Some(OperationKind::Unknown),
        connection_id: record.connection_id,
        database: Some(record.database.to_string()),
        namespace: None,
        text: record.text.clone(),
        success: Some(record.success),
        executed_at_unix_secs: Some(record.executed_at_unix_secs),
        elapsed_ms: 0,
        returned_rows: 0,
        affected_rows: 0,
        summary: Some(record.summary.clone()),
        source: Some(record.source.clone()),
        extra_json: EMPTY_EXTRA.to_string(),
    }
}

pub(crate) fn redis_command_from_row(row: HistoryRow) -> Result<RedisWorkbenchHistoryRecord> {
    let database = parse_redis_database(&row)?;
    Ok(RedisWorkbenchHistoryRecord {
        id: row.id as u64,
        connection_id: row.connection_id,
        database,
        text: row.text,
        success: row.success.unwrap_or(true),
        executed_at_unix_secs: row.executed_at_unix_secs.unwrap_or(0),
        summary: row.summary.unwrap_or_default(),
        source: row.source.unwrap_or_default(),
    })
}

pub(crate) fn redis_key_search_to_row(record: &RedisKeySearchHistoryRecord) -> HistoryRow {
    HistoryRow {
        id: 0,
        // Key 搜索是查找类操作，归类到只读。
        operation_kind: Some(OperationKind::Query),
        connection_id: record.connection_id,
        database: record.database.clone(),
        namespace: None,
        text: record.text.clone(),
        success: None,
        executed_at_unix_secs: None,
        elapsed_ms: 0,
        returned_rows: 0,
        affected_rows: 0,
        summary: None,
        source: None,
        extra_json: EMPTY_EXTRA.to_string(),
    }
}

pub(crate) fn redis_key_search_from_row(row: HistoryRow) -> RedisKeySearchHistoryRecord {
    RedisKeySearchHistoryRecord {
        connection_id: row.connection_id,
        database: row.database,
        text: row.text,
    }
}

fn parse_redis_database(row: &HistoryRow) -> Result<u32> {
    row.database
        .as_deref()
        .unwrap_or("0")
        .parse::<u32>()
        .map_err(|_| {
            Error::new(
                ErrorKind::Internal,
                format!(
                    "Redis 历史 {} 的逻辑库序号无法解析: {:?}",
                    row.id, row.database
                ),
            )
        })
}
