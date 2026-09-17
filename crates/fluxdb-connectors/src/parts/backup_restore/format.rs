// 备份/恢复的格式与命名纯逻辑（可单测）。
//
// 核心约定（任务一）：备份文件后缀由「最终实际使用的备份执行器/输出格式」决定，
// 而不是按数据库类型简单硬编码，也不是统一追加 `.sql`。
// 本模块只做纯逻辑（格式判定、扩展名归一、内容识别），不接触 UI、不启动进程。

/// 由数据库类型 + 解析后的执行器推导产出格式（`Option`：该组合不产出受支持的备份）。
///
/// MySQL/TiDB 只有 SQL dump 一种产出（无论原生 mysqldump 还是逻辑 SQL dump 都是 `.sql`）；
/// PostgreSQL 只有 plain SQL（`.sql`）；SQLite 取决于执行器：原生 `.backup` → 二进制 `.db`，
/// SQL dump（通用逻辑备份，当前实际支持）→ `.sql`。
pub fn format_for_execution(
    kind: DatabaseKind,
    execution: BackupExecution,
) -> Option<BackupFormat> {
    // 校验「执行器与库类型」是否匹配，避免 MySQL 用 sqlite 执行器等错配组合。
    let matches = match (kind, execution) {
        (DatabaseKind::MySql | DatabaseKind::TiDb, BackupExecution::MySqlDump) => true,
        (DatabaseKind::Postgres, BackupExecution::PgDump) => true,
        (DatabaseKind::Postgres, BackupExecution::SqlDump) => false, // PG 逻辑归一为原生 pg_dump
        (DatabaseKind::Sqlite, BackupExecution::SqliteBinary) => true,
        (DatabaseKind::Sqlite, BackupExecution::SqlDump) => false, // 当前只支持完整二进制快照
        (DatabaseKind::MySql | DatabaseKind::TiDb, BackupExecution::SqlDump) => true,
        _ => false,
    };
    matches.then(|| execution.format())
}

/// 把用户输入的后缀归一为格式的标准扩展名，避免 `xxx.sql.db` / `xxx.db.sql` 等叠缀。
///
/// - 文件名无扩展名：直接追加标准扩展名。
/// - 文件名已有其它被识别出的备份扩展名（`.sql`/`.db`）且与目标格式不符：替换为格式扩展名。
/// - 文件名已有标准扩展名且与格式一致：原样保留。
pub fn normalize_backup_file_name(file_name: String, format: BackupFormat) -> String {
    let mut stem = file_name.as_str();
    while let Some((base, ext)) = stem.rsplit_once('.') {
        if !["sql", "db", "sqlite", "sqlite3", "dump"].contains(&ext.to_ascii_lowercase().as_str())
        {
            break;
        }
        stem = base;
    }
    format!("{stem}.{}", format.extension())
}

/// 是否可能是 `sqlite3 .backup` 二进制文件：文件头为 `SQLite format 3\0`。
pub fn is_sqlite_binary_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"SQLite format 3\0")
}

/// 由文件头/文本片段识别真实备份格式。
///
/// 关键点（任务一）：识别真实格式不能只依赖后缀——历史上 SQLite 二进制备份被错误命名为 `.sql`，
/// 恢复时必须仍能识别为二进制。因此本函数以内容为准：先看 SQLite 魔数，再看是否像 SQL 文本。
pub fn detect_backup_format(header: &[u8], text_sample: Option<&str>) -> Option<BackupFormat> {
    if is_sqlite_binary_bytes(header) {
        return Some(BackupFormat::SqliteBinary);
    }
    // SQL 文本特征：空文件不算；存在 SQL 常见关键字即可（`--` 注释、CREATE、INSERT、BEGIN 等）。
    let sample = text_sample.unwrap_or_default();
    let looks_sql = !sample.is_empty()
        && (sample.contains("CREATE TABLE")
            || sample.contains("CREATE DATABASE")
            || sample.contains("INSERT INTO")
            || sample.to_ascii_lowercase().contains("-- mysql")
            || sample.starts_with("-- ")
            || sample.starts_with("BEGIN"));
    looks_sql.then_some(BackupFormat::Sql)
}

/// 由备份记录中的执行方式字符串反解执行器（旧记录可能没有该字段，返回 None）。
pub fn execution_from_record(value: Option<&str>) -> Option<BackupExecution> {
    match value {
        Some("mysqldump") => Some(BackupExecution::MySqlDump),
        Some("pg_dump") => Some(BackupExecution::PgDump),
        Some("sql_dump") => Some(BackupExecution::SqlDump),
        Some("sqlite3_backup") => Some(BackupExecution::SqliteBinary),
        _ => None,
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;
    use fluxdb_core::DatabaseKind;

    #[test]
    fn execution_decides_extension_not_db_kind() {
        // MySQL/TiDB/Postgres 均是 SQL dump → .sql
        assert_eq!(
            format_for_execution(DatabaseKind::MySql, BackupExecution::MySqlDump),
            Some(BackupFormat::Sql)
        );
        assert_eq!(
            format_for_execution(DatabaseKind::TiDb, BackupExecution::MySqlDump),
            Some(BackupFormat::Sql)
        );
        assert_eq!(
            format_for_execution(DatabaseKind::Postgres, BackupExecution::PgDump),
            Some(BackupFormat::Sql)
        );
        assert_eq!(
            format_for_execution(DatabaseKind::Sqlite, BackupExecution::SqlDump),
            None
        );
        // SQLite 原生二进制 .backup → .db
        assert_eq!(
            format_for_execution(DatabaseKind::Sqlite, BackupExecution::SqliteBinary),
            Some(BackupFormat::SqliteBinary)
        );
        // 错配组合不产出
        assert_eq!(
            format_for_execution(DatabaseKind::Sqlite, BackupExecution::MySqlDump),
            None
        );
        assert_eq!(
            format_for_execution(DatabaseKind::MySql, BackupExecution::PgDump),
            None
        );
        // PostgreSQL 没有通用逐表 SQL dump 路径（逻辑归一为原生 pg_dump）
        assert_eq!(
            format_for_execution(DatabaseKind::Postgres, BackupExecution::SqlDump),
            None
        );
    }

    #[test]
    fn normalizes_user_entered_suffix() {
        // 已匹配：保留
        assert_eq!(
            normalize_backup_file_name("backup.sql".into(), BackupFormat::Sql),
            "backup.sql"
        );
        // 无扩展名：追加
        assert_eq!(
            normalize_backup_file_name("backup".into(), BackupFormat::Sql),
            "backup.sql"
        );
        assert_eq!(
            normalize_backup_file_name("backup".into(), BackupFormat::SqliteBinary),
            "backup.db"
        );
        // 后缀不匹配目标格式：替换，避免叠缀
        assert_eq!(
            normalize_backup_file_name("backup.sql".into(), BackupFormat::SqliteBinary),
            "backup.db"
        );
        assert_eq!(
            normalize_backup_file_name("backup.db".into(), BackupFormat::Sql),
            "backup.sql"
        );
    }

    #[test]
    fn removes_stacked_backup_suffixes() {
        assert_eq!(
            normalize_backup_file_name("snapshot.sql.db".into(), BackupFormat::SqliteBinary),
            "snapshot.db"
        );
        assert_eq!(
            normalize_backup_file_name("snapshot.db.sql".into(), BackupFormat::Sql),
            "snapshot.sql"
        );
    }

    #[test]
    fn detects_format_from_content_not_suffix() {
        // SQLite 魔数优先（即使命名 .sql，历史上误命名的二进制备份也能识别）
        assert_eq!(
            detect_backup_format(b"SQLite format 3\0abc", Some("not sql")),
            Some(BackupFormat::SqliteBinary)
        );
        // SQL 文本
        assert_eq!(
            detect_backup_format(
                b"-- MySQL dump",
                Some("-- MySQL dump\nCREATE TABLE t (id INT);")
            ),
            Some(BackupFormat::Sql)
        );
        // 非空但既非 SQLite 也非 SQL → None
        assert_eq!(detect_backup_format(b"\x00\x01\x02", Some("")), None);
        // 空文件 → None
        assert_eq!(detect_backup_format(b"", Some("")), None);
    }

    #[test]
    fn execution_from_record_roundtrip() {
        assert_eq!(
            execution_from_record(Some("sqlite3_backup")),
            Some(BackupExecution::SqliteBinary)
        );
        assert_eq!(
            execution_from_record(Some("mysqldump")),
            Some(BackupExecution::MySqlDump)
        );
        assert_eq!(execution_from_record(None), None);
        assert_eq!(execution_from_record(Some("unknown")), None);
    }
}
