//! SQLite 本地持久化原语。
//!
//! 数据分两类落盘：
//! - **真表**：`connections`（一条连接一行，见 [`crate::connection_store`]）与
//!   `history`（所有数据库类型的执行历史，见 [`crate::history_store`]）；
//! - **kv 表**：仍是 load-all / save-all 全量读写的设置类数据（settings、
//!   sidebar layout、saved queries、ER 视图与关系、备份记录等），每条记录序列化为
//!   JSON blob，避免按字段建列导致的 serde 漂移风险。
//!
//! 每次调用新开 Connection 并关闭，与按次打开文件对齐，同时保证
//! FileStorage 只含 PathBuf、天然 Send（可被移入 async 任务）。

use std::fs;
use std::path::Path;

use rusqlite::{Connection, params};

use fluxdb_core::{Error, ErrorKind, Result};
use serde::{Serialize, de::DeserializeOwned};

/// 数据库文件名，位于 root 下，与 config.toml / completion-index 同级。
const DB_FILE: &str = "fluxdb.sqlite";

/// 顶层 kv 表的逻辑 key（连接与历史已改为真表，不再走 kv）。
pub const KEY_SIDEBAR_LAYOUT: &str = "sidebar_layout";
pub const KEY_SAVED_QUERIES: &str = "saved_queries";
pub const KEY_BACKUP_RECORDS: &str = "backup_records";
pub const KEY_ER_VIEWS: &str = "er_views";

/// 将 rusqlite/serde_json 错误映射为 fluxdb 内部错误，对齐 lib.rs `storage_error`。
pub(crate) fn sqlite_error(message: impl ToString) -> Error {
    Error::new(ErrorKind::Internal, message.to_string())
}

/// FileStorage 的 sqlite db 路径。
pub fn db_path(root: &Path) -> std::path::PathBuf {
    root.join(DB_FILE)
}

/// 打开 root 下的 sqlite 数据库，开启 WAL。
pub fn open(root: &Path) -> Result<Connection> {
    let path = db_path(root);
    // root 目录可能尚未创建（如全新安装 / 测试临时目录），先确保父目录存在。
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(sqlite_error)?;
    }
    // bundled 特性保证 libsqlite3 静态编译，无系统依赖。
    let conn = Connection::open(&path).map_err(sqlite_error)?;
    // 数据库含工作台历史等业务数据，创建后立即收紧为 0o600（方案 §12.2）；
    // WAL/SHM 文件继承 db 文件权限。
    super::harden_file_perms(&path)?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(sqlite_error)?;
    Ok(conn)
}

/// 建表（idempotent），并读回当前 schema 版本（PRAGMA user_version）。
///
/// 表定义的唯一来源是各职责模块的 `CREATE_SQL` 常量（连接、历史），
/// 不在此处复制一份，避免两处漂移。
pub fn create_schema(conn: &Connection) -> Result<i64> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS kv (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .map_err(sqlite_error)?;
    conn.execute_batch(crate::connection_store::CREATE_SQL)
        .map_err(sqlite_error)?;
    crate::connection_store::drop_legacy_credential_ref(conn)?;
    conn.execute_batch(crate::history_store::CREATE_SQL)
        .map_err(sqlite_error)?;
    crate::connection_secrets::ensure_schema(conn)?;
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sqlite_error)
}

/// 读单个 key 的 JSON 值，缺失返回 None。
pub fn get_json<T: DeserializeOwned>(conn: &Connection, key: &str) -> Result<Option<T>> {
    let mut stmt = conn
        .prepare("SELECT value FROM kv WHERE key = ?1")
        .map_err(sqlite_error)?;
    let mut rows = stmt.query(params![key]).map_err(sqlite_error)?;
    let Some(row) = rows.next().map_err(sqlite_error)? else {
        return Ok(None);
    };
    let value: String = row.get(0).map_err(sqlite_error)?;
    serde_json::from_str(&value).map(Some).map_err(sqlite_error)
}

/// 写单个 key 的 JSON 值（upsert）。
pub fn put_json<T: Serialize>(conn: &Connection, key: &str, value: &T) -> Result<()> {
    let json = serde_json::to_string(value).map_err(sqlite_error)?;
    conn.execute(
        "INSERT INTO kv (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, json],
    )
    .map_err(sqlite_error)?;
    Ok(())
}
