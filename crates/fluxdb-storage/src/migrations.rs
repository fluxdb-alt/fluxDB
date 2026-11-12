//! SQLite 结构版本迁移（`PRAGMA user_version` 门控）。
//!
//! 约定（与主流桌面/嵌入式做法一致）：
//! - 新库建表后直接打上 [`LATEST_VERSION`]；只有「已存在的旧库」才逐版本升级；
//! - 每一步迁移与版本号在同一事务内提交，失败整体回滚，不留下半迁移状态；
//! - 迁移前用 `VACUUM INTO` 备份一份（保留最近 [`BACKUP_KEEP`] 份），备份失败即中止；
//! - 磁盘版本高于本程序时失败关闭（提示升级应用），不猜测、不兼容；
//! - 业务代码只认最新结构：旧结构的处理只出现在本文件，且发布后不可修改已发布的步骤。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use fluxdb_core::{Error, ErrorKind, Result};

use crate::sqlite::sqlite_error;

/// 当前代码认识的最新结构版本。新增结构变化时 +1，并在 [`STEPS`] 末尾追加一步。
pub const LATEST_VERSION: i64 = 2;

/// 迁移前备份保留份数。
const BACKUP_KEEP: usize = 3;

/// 一次结构迁移：编号从 1 连续递增，`apply` 必须幂等且只做结构/数据改写。
struct Migration {
    version: i64,
    name: &'static str,
    apply: fn(&Transaction<'_>) -> Result<()>,
}

/// 全部迁移步骤。已发布的步骤不得修改或重排；需要替代时追加新版本。
const STEPS: &[Migration] = &[
    Migration {
        version: 1,
        name: "放宽 ai_mcp_server_profiles.transport 约束",
        apply: relax_mcp_transport_check,
    },
    Migration {
        version: 2,
        name: "MCP Server Profile 传输方式统一为应用内 HTTP",
        apply: unify_mcp_transport_http,
    },
];

/// 当前连接的 `user_version`。
pub fn user_version(conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sqlite_error)
}

/// 库是否全新（没有任何用户表）：由调用方在建表**之前**判定。
///
/// 全新库由 `CREATE_SQL` 直接建成最新结构并打上最新版本，不跑迁移；
/// 已有库（哪怕只有 `kv` 表）才按版本升级。
pub fn is_fresh_database(conn: &Connection) -> Result<bool> {
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
              WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    Ok(tables == 0)
}

/// 把全新库标记为最新版本。
pub fn stamp_latest(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "user_version", LATEST_VERSION)
        .map_err(sqlite_error)
}

/// 把已有库升级到 [`LATEST_VERSION`]，返回升级后的版本。
///
/// 版本高于本程序时直接报错（失败关闭）：旧程序读新库只会静默错读。
pub fn upgrade(conn: &Connection) -> Result<i64> {
    let current = user_version(conn)?;
    if current > LATEST_VERSION {
        return Err(Error::new(
            ErrorKind::Internal,
            format!(
                "数据库结构版本为 {current}，高于当前应用支持的 {LATEST_VERSION}，请升级应用后再打开"
            ),
        ));
    }
    if current == LATEST_VERSION {
        return Ok(current);
    }

    let pending: Vec<&Migration> = STEPS.iter().filter(|step| step.version > current).collect();
    validate_steps(current, &pending)?;
    backup_before_upgrade(conn, current)?;

    // 重建表（DROP + RENAME）要求迁移期间关闭外键约束；PRAGMA 必须在事务外切换。
    // 关闭后由迁移事务内的 `foreign_key_check` 兜底，任何悬空引用都会回滚。
    conn.pragma_update(None, "foreign_keys", false)
        .map_err(sqlite_error)?;
    let applied = apply_pending(conn, &pending);
    let restored = conn.pragma_update(None, "foreign_keys", true);
    applied?;
    restored.map_err(sqlite_error)?;

    verify_integrity(conn)?;
    let version = user_version(conn)?;
    if version != LATEST_VERSION {
        return Err(Error::new(
            ErrorKind::Internal,
            format!("迁移完成后版本为 {version}，期望 {LATEST_VERSION}"),
        ));
    }
    Ok(version)
}

/// 步骤编号必须与起始版本连续，避免漏写版本号导致版本与结构错配。
fn validate_steps(current: i64, pending: &[&Migration]) -> Result<()> {
    for (index, step) in pending.iter().enumerate() {
        let expected = current + index as i64 + 1;
        if step.version != expected {
            // 暴露代码缺陷，而不是让版本号与结构静默漂移。
            return Err(Error::new(
                ErrorKind::Internal,
                format!(
                    "迁移步骤编号不连续：期望 {expected}，实际 {}（{}）",
                    step.version, step.name
                ),
            ));
        }
    }
    Ok(())
}

/// 在一个事务内执行全部待迁移步骤，版本号与结构改写一起提交。
fn apply_pending(conn: &Connection, pending: &[&Migration]) -> Result<()> {
    // `new_unchecked`：调用方只持有 `&Connection`（存储层每次操作新开连接），
    // 这里用 Immediate 先拿写锁，保证并发启动时只有一个进程执行迁移。
    let tx =
        Transaction::new_unchecked(conn, TransactionBehavior::Immediate).map_err(sqlite_error)?;
    for step in pending {
        (step.apply)(&tx)?;
        tx.pragma_update(None, "user_version", step.version)
            .map_err(sqlite_error)?;
    }
    // 迁移事务内自检：外键此时关闭，提交前必须确认没有留下悬空引用。
    let violations = {
        let mut stmt = tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(sqlite_error)?;
        let mut rows = stmt.query([]).map_err(sqlite_error)?;
        rows.next().map_err(sqlite_error)?.is_some()
    };
    if violations {
        return Err(Error::new(
            ErrorKind::Internal,
            "数据库迁移后外键校验失败：存在悬空引用，已回滚",
        ));
    }
    tx.commit().map_err(sqlite_error)
}

/// 迁移前备份：`VACUUM INTO` 生成一致快照，再收紧权限，最后只保留最近几份。
///
/// 备份失败直接中止迁移 —— 结构改写不可逆，没有快照就不动手。
fn backup_before_upgrade(conn: &Connection, from_version: i64) -> Result<()> {
    let Some(db_path) = conn.path() else {
        // 内存库（单测/临时场景）没有可备份的文件。
        return Ok(());
    };
    let db_path = Path::new(db_path);
    let Some(dir) = db_path.parent() else {
        return Ok(());
    };
    let file_name = db_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("fluxdb.sqlite");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let target = dir.join(format!("{file_name}.bak-v{from_version}-{stamp}"));

    // VACUUM INTO 是唯一安全的活动库快照方式：直接拷贝 WAL 模式下的文件可能拿到半写状态。
    conn.execute("VACUUM INTO ?1", params![target.to_string_lossy()])
        .map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("迁移前备份失败，已中止升级：{error}"),
            )
        })?;
    crate::harden_file_perms(&target)?;
    prune_backups(dir, file_name);
    Ok(())
}

/// 清理过期备份（只删本次应用自己生成的 `*.bak-v*`）。
fn prune_backups(dir: &Path, file_name: &str) {
    let prefix = format!("{file_name}.bak-v");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .collect();
    if backups.len() <= BACKUP_KEEP {
        return;
    }
    // 文件名带秒级时间戳，字典序即时间序。
    backups.sort();
    let remove_count = backups.len() - BACKUP_KEEP;
    for path in backups.into_iter().take(remove_count) {
        if let Err(error) = fs::remove_file(&path) {
            tracing::warn!(path = %path.display(), %error, "清理旧备份失败");
        }
    }
}

/// 迁移后完整性校验：损坏或全表扫描失败都必须阻止应用继续运行。
fn verify_integrity(conn: &Connection) -> Result<()> {
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(sqlite_error)?;
    if result != "ok" {
        return Err(Error::new(
            ErrorKind::Internal,
            format!("数据库完整性校验失败：{result}"),
        ));
    }
    Ok(())
}

/// 步骤 1：放宽 `ai_mcp_server_profiles.transport` 的 CHECK，为应用内 HTTP 端点做准备。
///
/// SQLite 不能修改 CHECK 约束，只能重建表；外键由 `apply_pending` 统一关闭并校验。
fn relax_mcp_transport_check(tx: &Transaction<'_>) -> Result<()> {
    let Some(sql) = table_sql(tx, "ai_mcp_server_profiles")? else {
        // 表不存在：新结构已由建表语句创建，无需重建。
        return Ok(());
    };
    if !sql.contains("transport='stdio'") {
        // 已是新约束（例如新库或已迁移过的库），重复执行安全跳过。
        return Ok(());
    }
    tx.execute_batch(
        r##"
        CREATE TABLE ai_mcp_server_profiles_migrating (
            id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0,1)),
            transport TEXT NOT NULL DEFAULT 'stdio' CHECK (transport IN ('stdio','http')),
            options_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(options_json)),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL
        );
        INSERT INTO ai_mcp_server_profiles_migrating
            (id, display_name, enabled, transport, options_json, revision,
             created_at_ms, updated_at_ms)
        SELECT id, display_name, enabled, transport, options_json, revision,
               created_at_ms, updated_at_ms
          FROM ai_mcp_server_profiles;
        DROP TABLE ai_mcp_server_profiles;
        ALTER TABLE ai_mcp_server_profiles_migrating
            RENAME TO ai_mcp_server_profiles;
        "##,
    )
    .map_err(sqlite_error)
}

/// 步骤 2：不再提供独立 stdio 进程，历史 Profile 的传输标记统一为 `http`。
///
/// 只改标记，不动授权与数据源；应用行为由 Profile 的启用开关决定。
fn unify_mcp_transport_http(tx: &Transaction<'_>) -> Result<()> {
    if table_sql(tx, "ai_mcp_server_profiles")?.is_none() {
        return Ok(());
    }
    tx.execute(
        "UPDATE ai_mcp_server_profiles SET transport='http' WHERE transport='stdio'",
        [],
    )
    .map_err(sqlite_error)?;
    Ok(())
}

fn table_sql(tx: &Transaction<'_>, name: &str) -> Result<Option<String>> {
    tx.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |row| row.get(0),
    )
    .optional()
    .map_err(sqlite_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "fluxdb-migrations-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp root");
        root
    }

    const LEGACY_PROFILES_DDL: &str = r##"CREATE TABLE ai_mcp_server_profiles (
        id TEXT PRIMARY KEY,
        display_name TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0,1)),
        transport TEXT NOT NULL DEFAULT 'stdio' CHECK (transport='stdio'),
        options_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(options_json)),
        revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
        created_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );"##;

    fn seed_legacy_database(root: &Path) {
        let conn = crate::sqlite::open(root).expect("open legacy db");
        conn.execute_batch(LEGACY_PROFILES_DDL)
            .expect("legacy profiles ddl");
        conn.execute_batch(
            r##"CREATE TABLE ai_mcp_server_datasources (
                profile_id TEXT NOT NULL REFERENCES ai_mcp_server_profiles(id) ON DELETE RESTRICT,
                connection_id INTEGER NOT NULL,
                write_policy TEXT NOT NULL DEFAULT 'default'
                    CHECK (write_policy IN ('default','read_only','read_write')),
                revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY(profile_id, connection_id)
            );"##,
        )
        .expect("legacy datasources ddl");
        conn.execute(
            "INSERT INTO ai_mcp_server_profiles
                (id, display_name, enabled, transport, options_json, revision,
                 created_at_ms, updated_at_ms)
             VALUES ('fluxdb.local','本地 MCP',1,'stdio','{}',1,1,1)",
            [],
        )
        .expect("seed profile");
        conn.execute(
            "INSERT INTO ai_mcp_server_datasources
                (profile_id, connection_id, write_policy, revision, created_at_ms, updated_at_ms)
             VALUES ('fluxdb.local',2,'default',1,1,1)",
            [],
        )
        .expect("seed datasource");
        conn.pragma_update(None, "user_version", 0)
            .expect("legacy version");
    }

    #[test]
    fn fresh_database_is_stamped_latest_and_accepts_http_transport() {
        let root = temp_root("fresh");
        let conn = crate::sqlite::open(&root).expect("open");
        let version = crate::sqlite::create_schema(&conn).expect("create schema");
        assert_eq!(version, LATEST_VERSION);

        conn.execute(
            "INSERT INTO ai_mcp_server_profiles
                (id, display_name, enabled, transport, options_json, revision,
                 created_at_ms, updated_at_ms)
             VALUES ('p','P',1,'http','{}',1,1,1)",
            [],
        )
        .expect("fresh database must accept http transport");
    }

    #[test]
    fn legacy_database_is_upgraded_in_place_keeping_rows() {
        let root = temp_root("legacy");
        seed_legacy_database(&root);

        let conn = crate::sqlite::open(&root).expect("open");
        let version = crate::sqlite::create_schema(&conn).expect("upgrade");
        assert_eq!(version, LATEST_VERSION);

        let transport: String = conn
            .query_row(
                "SELECT transport FROM ai_mcp_server_profiles WHERE id='fluxdb.local'",
                [],
                |row| row.get(0),
            )
            .expect("profile row must survive migration");
        assert_eq!(transport, "http");

        let datasource: i64 = conn
            .query_row(
                "SELECT connection_id FROM ai_mcp_server_datasources WHERE profile_id='fluxdb.local'",
                [],
                |row| row.get(0),
            )
            .expect("datasource row must survive migration");
        assert_eq!(datasource, 2);

        conn.execute(
            "INSERT INTO ai_mcp_server_profiles
                (id, display_name, enabled, transport, options_json, revision,
                 created_at_ms, updated_at_ms)
             VALUES ('http.local','HTTP',1,'http','{}',1,1,1)",
            [],
        )
        .expect("upgraded database must accept http transport");

        // 迁移前的快照必须留存，用于回退。
        let backups: Vec<_> = fs::read_dir(&root)
            .expect("read root")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.contains(".bak-v0-"))
            })
            .collect();
        assert_eq!(backups.len(), 1, "migration must leave exactly one backup");
    }

    #[test]
    fn future_version_fails_closed() {
        let root = temp_root("future");
        let conn = crate::sqlite::open(&root).expect("open");
        crate::sqlite::create_schema(&conn).expect("create schema");
        conn.pragma_update(None, "user_version", LATEST_VERSION + 1)
            .expect("bump version");

        let error = crate::sqlite::create_schema(&conn).expect_err("must refuse newer schema");
        assert!(error.message.contains("请升级应用"), "{}", error.message);
    }
}
