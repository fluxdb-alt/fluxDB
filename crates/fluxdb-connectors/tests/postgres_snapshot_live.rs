//! PostgreSQL 一致快照与逻辑备份真实连接测试。
//!
//! 仅在显式提供 `FLUXDB_PG_SMOKE=host:port:user:password` 时运行；默认忽略。
//! 测试自建 `fluxdb_pg_snapshot_<pid>` 等库，结束时只删除本次创建的库。

use fluxdb_connectors::{connector_for, database_backup};
use fluxdb_core::{
    BackupExecution, BackupObjectKind, BackupObjectRef, BackupRequest, BackupScope, CellValue,
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
    PostgresBasicOptions, PostgresConnectionProfile, QueryExecutionOptions, QueryMode,
    QueryRequest, SecretRef,
};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_PG_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let port = parts[1].parse().ok()?;
    Some(ConnectionConfig {
        id: ConnectionId(9_931),
        name: "PostgreSQL snapshot live".into(),
        kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some("postgres".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: parts[0].into(),
                port,
                maintenance_database: "postgres".into(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            ..PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    })
}

fn execute(config: &ConnectionConfig, database: Option<&str>, sql: &str) {
    let connector = connector_for(config).expect("connector");
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: database.map(str::to_string),
            schema: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        })
        .unwrap_or_else(|error| panic!("execute failed: {sql}: {error}"));
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "语句失败: {sql}: {result:?}"
    );
}

fn count(config: &ConnectionConfig, database: &str, table: &str) -> i64 {
    let connector = connector_for(config).expect("connector");
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: Some(database.into()),
            schema: None,
            text: format!("SELECT count(*) FROM {table}"),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        })
        .expect("count");
    match result
        .results
        .first()
        .and_then(|page| page.rows.first())
        .and_then(|row| row.values.first())
    {
        Some(CellValue::I64(value)) => *value,
        Some(CellValue::Text(value)) => value.parse().expect("count text"),
        other => panic!("unexpected count value: {other:?}"),
    }
}

struct DropDatabase {
    config: ConnectionConfig,
    databases: Vec<String>,
}

impl Drop for DropDatabase {
    fn drop(&mut self) {
        for database in &self.databases {
            execute(
                &self.config,
                None,
                &format!("DROP DATABASE IF EXISTS \"{database}\" WITH (FORCE)"),
            );
        }
    }
}

/// 按主键列顺序取一页最后一行的游标，模拟逻辑备份的翻页方式。
fn page_cursor(page: &fluxdb_core::DataPage) -> Vec<fluxdb_core::CellValue> {
    let last = page.rows.last().expect("page has rows");
    page.columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.primary_key)
        .map(|(index, _)| last.values[index].clone())
        .collect()
}

#[test]
#[ignore = "需要 FLUXDB_PG_SMOKE=host:port:user:password"]
fn postgres_snapshot_keyset_pagination() {
    use fluxdb_core::CellValue;

    let Some(config) = smoke_config() else {
        return;
    };
    let database = format!("fluxdb_pg_keyset_{}", std::process::id());
    let _cleanup = DropDatabase {
        config: config.clone(),
        databases: vec![database.clone()],
    };
    execute(
        &config,
        None,
        &format!("DROP DATABASE IF EXISTS \"{database}\" WITH (FORCE)"),
    );
    execute(&config, None, &format!("CREATE DATABASE \"{database}\""));
    execute(
        &config,
        Some(&database),
        "CREATE TABLE t (id INT PRIMARY KEY, v INT)",
    );
    execute(
        &config,
        Some(&database),
        "INSERT INTO t VALUES (1,1),(2,2),(3,3),(4,4),(5,5)",
    );
    execute(
        &config,
        Some(&database),
        "CREATE TABLE c (a INT, b INT, PRIMARY KEY (a,b))",
    );
    execute(
        &config,
        Some(&database),
        "INSERT INTO c VALUES (1,1),(1,2),(2,1)",
    );
    // 快照建立后提交的 DDL 在 REPEATABLE READ 里不可见，测试表必须提前建好。
    execute(
        &config,
        Some(&database),
        "CREATE TABLE bin (id BYTEA PRIMARY KEY)",
    );
    execute(
        &config,
        Some(&database),
        "INSERT INTO bin VALUES ('\\x01'::bytea),('\\x02'::bytea)",
    );

    let connector = connector_for(&config).expect("connector");
    let mut snapshot = connector.backup_snapshot(&database).expect("snapshot");
    let path = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: Some("public".into()),
        name: "t".into(),
        kind: ObjectKind::Table,
    };
    let first = snapshot.load_data_keyset(&path, &[], 2).expect("page 1");
    assert_eq!(first.rows.len(), 2);
    assert!(first.has_more);
    let second = snapshot
        .load_data_keyset(&path, &page_cursor(&first), 2)
        .expect("page 2");
    assert_eq!(second.rows.len(), 2);
    let third = snapshot
        .load_data_keyset(&path, &page_cursor(&second), 2)
        .expect("page 3");
    assert_eq!(third.rows.len(), 1);
    assert!(!third.has_more, "最后一页不应还有数据");
    let ids = [&first, &second, &third]
        .into_iter()
        .flat_map(|page| page.rows.iter())
        .map(|row| match &row.values[0] {
            CellValue::I64(value) => *value,
            CellValue::Text(value) => value.parse().expect("numeric id text"),
            other => panic!("unexpected id value: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![1, 2, 3, 4, 5], "键集翻页应无重复无遗漏");

    let composite = ObjectPath {
        name: "c".into(),
        ..path.clone()
    };
    let first = snapshot
        .load_data_keyset(&composite, &[], 2)
        .expect("composite page 1");
    assert_eq!(first.rows.len(), 2);
    let second = snapshot
        .load_data_keyset(&composite, &page_cursor(&first), 2)
        .expect("composite page 2");
    assert_eq!(second.rows.len(), 1, "复合主键应继续返回剩余行");

    // 二进制主键只有摘要，不能当游标绑定；必须明确返回 Unsupported 让备份退回 OFFSET。
    let binary = ObjectPath {
        name: "bin".into(),
        ..path.clone()
    };
    let first = snapshot
        .load_data_keyset(&binary, &[], 1)
        .expect("binary page 1");
    assert_eq!(first.rows.len(), 1);
    let error = snapshot
        .load_data_keyset(&binary, &page_cursor(&first), 1)
        .expect_err("二进制主键游标必须被拒绝");
    assert_eq!(error.kind, fluxdb_core::ErrorKind::Unsupported);
    snapshot.finish().expect("finish");
}

#[test]
#[ignore = "需要 FLUXDB_PG_SMOKE=host:port:user:password"]
fn postgres_snapshot_ignores_concurrent_inserts() {
    let Some(config) = smoke_config() else {
        return;
    };
    let database = format!("fluxdb_pg_snapshot_{}", std::process::id());
    let _cleanup = DropDatabase {
        config: config.clone(),
        databases: vec![database.clone()],
    };
    execute(
        &config,
        None,
        &format!("DROP DATABASE IF EXISTS \"{database}\" WITH (FORCE)"),
    );
    execute(&config, None, &format!("CREATE DATABASE \"{database}\""));
    execute(
        &config,
        Some(&database),
        "CREATE TABLE a (id INT PRIMARY KEY, note TEXT)",
    );
    execute(
        &config,
        Some(&database),
        "INSERT INTO a VALUES (1,'x'),(2,'y'),(3,'z')",
    );

    let connector = connector_for(&config).expect("connector");
    let path = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: Some("public".into()),
        name: "a".into(),
        kind: ObjectKind::Table,
    };
    let mut snapshot = connector.backup_snapshot(&database).expect("snapshot");
    let first = snapshot.load_data(&path, 0, 2).expect("page 1");
    assert_eq!(first.rows.len(), 2);
    assert!(first.has_more);
    assert!(
        snapshot
            .table_ddl(&path)
            .expect("ddl")
            .contains("CREATE TABLE"),
        "快照内应能读取建表语句"
    );

    execute(&config, Some(&database), "INSERT INTO a VALUES (4,'w')");
    let second = snapshot.load_data(&path, 2, 2).expect("page 2");
    assert_eq!(
        second.rows.len(),
        1,
        "快照读不应看到快照建立后的新行，实际 {} 行",
        second.rows.len()
    );
    let live = connector
        .load_data(&path, 0, 10, &[], &[])
        .expect("live read");
    assert_eq!(live.rows.len(), 4, "普通读取应看到 4 行");
    snapshot.finish().expect("finish");
}

#[test]
#[ignore = "需要 FLUXDB_PG_SMOKE=host:port:user:password"]
fn postgres_logical_backup_round_trip() {
    let Some(config) = smoke_config() else {
        return;
    };
    let source = format!("fluxdb_pg_dump_src_{}", std::process::id());
    let target = format!("fluxdb_pg_dump_dst_{}", std::process::id());
    let _cleanup = DropDatabase {
        config: config.clone(),
        databases: vec![source.clone(), target.clone()],
    };
    for database in [&source, &target] {
        execute(
            &config,
            None,
            &format!("DROP DATABASE IF EXISTS \"{database}\" WITH (FORCE)"),
        );
        execute(&config, None, &format!("CREATE DATABASE \"{database}\""));
    }
    execute(
        &config,
        Some(&source),
        "CREATE TABLE a (id INT PRIMARY KEY, note TEXT)",
    );
    execute(
        &config,
        Some(&source),
        "INSERT INTO a VALUES (1,'x'),(2,'y\\o''z')",
    );

    let output = std::env::temp_dir().join(format!("fluxdb_pg_dump_{}.sql", std::process::id()));
    let request = BackupRequest {
        config: config.clone(),
        database: source.clone(),
        output: output.clone(),
        execution: BackupExecution::SqlDump,
        tool: std::path::PathBuf::new(),
        tool_version: None,
        scope: BackupScope::Objects(vec![BackupObjectRef {
            schema: Some("public".into()),
            kind: BackupObjectKind::Table,
            name: "a".into(),
        }]),
        include_schema: true,
        include_data: true,
        include_routines: false,
        single_transaction: false,
        lock_tables: false,
        include_owner: false,
        include_acl: false,
    };
    let manifest = database_backup(DatabaseKind::Postgres)
        .expect("provider")
        .backup(&request, &AtomicBool::new(false), &mut |_| {})
        .expect("backup");
    let text = std::fs::read_to_string(&output).expect("read output");
    let _ = std::fs::remove_file(&output);

    assert_eq!(manifest.consistent, Some(true), "应建立一致快照");
    assert_eq!(manifest.object_stats[0].rows, Some(2));
    assert!(text.contains("-- fluxDB-backup-manifest-v2-begin"));
    assert!(text.contains("\"consistent\":true"));

    // 把生成的脚本导回目标库，验证语法与数据完整（不依赖本机 psql）。
    execute(&config, Some(&target), &text);
    assert_eq!(count(&config, &target, "a"), 2, "导入后应有 2 行");
}
