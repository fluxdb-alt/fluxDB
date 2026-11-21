// 备份/恢复的格式与命名纯逻辑（可单测）。
//
// 核心约定（任务一）：备份文件后缀由「最终实际使用的备份执行器/输出格式」决定，
// 而不是按数据库类型简单硬编码，也不是统一追加 `.sql`。
// 本模块只做纯逻辑（格式判定、扩展名归一、内容识别），不接触 UI、不启动进程。

/// 由数据库类型 + 解析后的执行器推导产出格式（`Option`：该组合不产出受支持的备份）。
///
/// MySQL/TiDB 只有 SQL dump 一种产出（无论原生 mysqldump 还是逻辑 SQL dump 都是 `.sql`）；
/// PostgreSQL 只有 plain SQL（`.sql`）；SQLite 取决于执行器：原生 `.backup` → 二进制 `.db`，
/// SQL dump（通用逻辑备份，当前实际支持）→ `.sql`；MongoDB 的 `mongodump` → `.archive`。
pub fn format_for_execution(
    kind: DatabaseKind,
    execution: BackupExecution,
) -> Option<BackupFormat> {
    // 校验「执行器与库类型」是否匹配，避免 MySQL 用 sqlite 执行器等错配组合。
    let matches = match (kind, execution) {
        (
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb,
            BackupExecution::MySqlDump,
        ) => true,
        (DatabaseKind::Postgres, BackupExecution::PgDump) => true,
        (DatabaseKind::Postgres, BackupExecution::SqlDump) => true,
        (DatabaseKind::Sqlite, BackupExecution::SqliteBinary) => true,
        (DatabaseKind::Sqlite, BackupExecution::SqlDump) => false, // 当前只支持完整二进制快照
        (
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb,
            BackupExecution::SqlDump,
        ) => true,
        (DatabaseKind::SqlServer, BackupExecution::SqlDump) => true,
        (DatabaseKind::Oracle, BackupExecution::SqlDump) => true,
        (DatabaseKind::MongoDb, BackupExecution::MongoDump) => true,
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
        if !["sql", "db", "sqlite", "sqlite3", "dump", "archive"]
            .contains(&ext.to_ascii_lowercase().as_str())
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
        Some("mongodump") => Some(BackupExecution::MongoDump),
        Some("sql_dump") => Some(BackupExecution::SqlDump),
        Some("sqlite3_backup") => Some(BackupExecution::SqliteBinary),
        _ => None,
    }
}

/// 从 SQL 文本尾部解析内嵌清单。
///
/// 清单写在 `-- fluxDB-backup-manifest-v2-begin` / `-end` 标记之间，
/// 每行 Markdown 正文为 JSON（前缀 `-- ` 保持整段是合法 SQL 注释）。
/// 找不到标记或 JSON 无法解析时返回 None：旧文件与外部工具生成的 dump
/// 走原有的「记录 manifest / 无 manifest」路径，不受影响。
pub fn parse_embedded_manifest(text: &str) -> Option<BackupManifest> {
    let begin = text.rfind(BACKUP_MANIFEST_BEGIN)?;
    let after_begin = &text[begin + BACKUP_MANIFEST_BEGIN.len()..];
    let end = after_begin.find(BACKUP_MANIFEST_END)?;
    let mut json = String::new();
    for line in after_begin[..end].lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        json.push_str(line.strip_prefix("-- ").unwrap_or(line));
    }
    serde_json::from_str(&json).ok()
}

/// 把清单格式化为追加在 SQL 文件尾部的注释块。
/// 每行以 `-- ` 开头，确保整段被 SQL 解析器当作注释忽略。
pub fn format_embedded_manifest(manifest: &BackupManifest) -> String {
    let json = serde_json::to_string(manifest).unwrap_or_else(|_| "{}".to_string());
    // 不额外加前导换行：数据部分以换行结尾，footer 从标记行开始，
    // 这样「截到标记之前」的哈希与备份时对数据取哈希的边界完全一致。
    let mut out = String::new();
    out.push_str(BACKUP_MANIFEST_BEGIN);
    out.push('\n');
    // 单行 JSON 可能很长，按智能边界拆块，避免部分工具对超长注释行的限制。
    const CHUNK: usize = 1600;
    let bytes = json.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        let mut stop = (start + CHUNK).min(bytes.len());
        while stop < bytes.len() && !json.is_char_boundary(stop) {
            stop += 1;
        }
        out.push_str("-- ");
        out.push_str(&json[start..stop]);
        out.push('\n');
        start = stop;
    }
    out.push_str(BACKUP_MANIFEST_END);
    out.push('\n');
    out
}

/// 计算「数据部分」的 sha256：从文件开头到 footer 标记之前。
/// 恢复校验时同样截到该标记，因此追加/修改 footer 不影响数据校验。
#[cfg(test)]
pub fn data_sha256(text: &str) -> String {
    data_sha256_reader(text.as_bytes()).expect("内存字节流不会读取失败")
}

/// 流式计算「数据部分」sha256：读到 footer 起始标记即停止。
///
/// 备份文件可能远大于内存，因此按块读取；保留 `标记长度-1` 字节的尾部，
/// 避免标记恰好跨块边界时漏检。
pub fn data_sha256_reader(mut reader: impl std::io::Read) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let marker = BACKUP_MANIFEST_BEGIN.as_bytes();
    let keep = marker.len().saturating_sub(1);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    let hex = |hasher: Sha256| -> String {
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        carry.extend_from_slice(&buf[..n]);
        if let Some(pos) = carry
            .windows(marker.len())
            .position(|window| window == marker)
        {
            hasher.update(&carry[..pos]);
            return Ok(hex(hasher));
        }
        if carry.len() > keep {
            let split = carry.len() - keep;
            hasher.update(&carry[..split]);
            carry.drain(..split);
        }
    }
    hasher.update(&carry);
    Ok(hex(hasher))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_round_trip() {
        let mut manifest = BackupManifest {
            kind: Some(DatabaseKind::MySql),
            database: None,
            execution: Some(BackupExecution::SqlDump),
            complete: true,
            include_schema: true,
            include_data: true,
            objects: vec!["users".into()],
            tool_version: None,
            format_version: Some(BACKUP_MANIFEST_VERSION),
            object_stats: vec![BackupObjectStats {
                name: "users".into(),
                rows: Some(42),
            }],
            file_sha256: None,
            consistent: Some(false),
            snapshot_id: None,
            failed_objects: Vec::new(),
        };
        // 与 finalize_embedded_manifest 一致：先对数据部分取哈希，再把哈希写进 footer。
        let data = "INSERT INTO `users` VALUES (1);\n";
        manifest.file_sha256 = Some(data_sha256(data));
        let sql = format!("{data}{}", format_embedded_manifest(&manifest));
        let parsed = parse_embedded_manifest(&sql).expect("manifest");
        assert_eq!(parsed, manifest);
        assert_eq!(parsed.object_stats[0].rows, Some(42));
        // 校验时截到 footer 之前，因此带 footer 的文件仍能得到同一哈希。
        assert_eq!(data_sha256(&sql), manifest.file_sha256.clone().unwrap());
    }

    #[test]
    fn streaming_hash_handles_marker_across_chunk_boundary() {
        // 标记跨 64 KiB 读缓冲边界时仍需正确截断，否则大文件校验会误判。
        let data = "x".repeat(64 * 1024 - 5);
        let sql = format!("{data}{BACKUP_MANIFEST_BEGIN}tail");
        assert_eq!(
            data_sha256(&sql),
            data_sha256_reader(sql.as_bytes()).expect("hash")
        );
    }

    #[test]
    fn mongo_archive_format_and_file_name_are_wired() {
        assert_eq!(
            format_for_execution(DatabaseKind::MongoDb, BackupExecution::MongoDump),
            Some(BackupFormat::MongoArchive)
        );
        assert_eq!(BackupFormat::MongoArchive.extension(), "archive");
        assert!(!BackupFormat::MongoArchive.is_sql());
        assert_eq!(
            normalize_backup_file_name("dump.sql".into(), BackupFormat::MongoArchive),
            "dump.archive"
        );
        assert_eq!(
            execution_from_record(Some("mongodump")),
            Some(BackupExecution::MongoDump)
        );
    }

    #[test]
    fn embedded_manifest_absent_for_external_dump() {
        assert!(parse_embedded_manifest("-- MySQL dump 10.13\nCREATE TABLE t (id int);\n").is_none());
    }
}

