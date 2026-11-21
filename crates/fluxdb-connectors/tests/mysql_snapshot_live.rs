//! MySQL 一致快照真实连接测试。
//!
//! 仅在显式提供 `FLUXDB_MYSQL_SMOKE=host:port:user:password` 时运行；默认忽略。
//! 测试自建 `fluxdb_snapshot_live_<pid>` 库，结束时只删除本次创建的库。
//!
//! 断言放在驱动回调之外：回调里 panic 无法可靠展开，会把进程直接 abort。

use fluxdb_connectors::{connector_for, database_backup};
use fluxdb_core::{
    ConnectionConfig, ConnectionId, Connector, DatabaseKind, Endpoint, MysqlBasicOptions,
    MysqlConnectionProfile, ObjectKind, ObjectPath, QueryExecutionOptions, QueryMode, QueryRequest,
    SecretRef,
};
use std::collections::BTreeMap;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_MYSQL_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let port = parts[1].parse().ok()?;
    Some(
        ConnectionConfig {
            id: ConnectionId(9_921),
            name: "MySQL snapshot live".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: parts[0].into(),
                port,
                database: None,
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: Some(MysqlConnectionProfile {
                basic: MysqlBasicOptions {
                    host: parts[0].into(),
                    port,
                    database: String::new(),
                    username: parts[2].into(),
                    password: SecretRef::inline(parts[3]),
                    ..MysqlBasicOptions::default()
                },
                ..MysqlConnectionProfile::default()
            }),
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
        .mysql_resolved(),
    )
}

fn execute(config: &ConnectionConfig, sql: &str) {
    let connector = connector_for(config).expect("connector");
    connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        })
        .unwrap_or_else(|error| panic!("execute failed: {sql}: {error}"));
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password"]
fn mysql_snapshot_ignores_concurrent_inserts() {
    let Some(config) = smoke_config() else {
        return;
    };
    let connector = connector_for(&config).expect("connector");
    let database = format!("fluxdb_snapshot_live_{}", std::process::id());
    execute(&config, &format!("DROP DATABASE IF EXISTS `{database}`"));
    execute(&config, &format!("CREATE DATABASE `{database}`"));
    // 无论断言是否失败都只清理本次创建的库。
    let _cleanup = DropDatabase {
        config: config.clone(),
        database: database.clone(),
    };
    {
        execute(
            &config,
            &format!(
                "CREATE TABLE `{database}`.`a` (id INT PRIMARY KEY, note VARCHAR(64)) ENGINE=InnoDB"
            ),
        );
        execute(
            &config,
            &format!("INSERT INTO `{database}`.`a` VALUES (1,'x'),(2,'y'),(3,'z')"),
        );
        let path = ObjectPath {
            connection_id: config.id,
            database: Some(database.clone()),
            schema: None,
            name: "a".into(),
            kind: ObjectKind::Table,
        };
        let mut snapshot = connector.backup_snapshot(&database).expect("snapshot");
        // 第一页同时建立事务快照。
        let first = snapshot.load_data(&path, 0, 2).expect("page 1");
        assert_eq!(first.rows.len(), 2, "第一页应有两行");
        assert!(first.has_more, "第一页应提示还有数据");
        assert!(
            snapshot
                .table_ddl(&path)
                .expect("ddl")
                .contains("CREATE TABLE"),
            "快照内应能读取建表语句"
        );

        // 并发写入发生在快照建立之后。
        execute(
            &config,
            &format!("INSERT INTO `{database}`.`a` VALUES (4,'w')"),
        );

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
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password"]
fn mysql_snapshot_keyset_pagination() {
    use fluxdb_core::CellValue;

    let Some(config) = smoke_config() else {
        return;
    };
    let database = format!("fluxdb_keyset_live_{}", std::process::id());
    execute(&config, &format!("DROP DATABASE IF EXISTS `{database}`"));
    execute(&config, &format!("CREATE DATABASE `{database}`"));
    let _cleanup = DropDatabase {
        config: config.clone(),
        database: database.clone(),
    };
    execute(
        &config,
        &format!("CREATE TABLE `{database}`.`t` (id INT PRIMARY KEY, v INT) ENGINE=InnoDB"),
    );
    execute(
        &config,
        &format!("INSERT INTO `{database}`.`t` VALUES (1,1),(2,2),(3,3),(4,4),(5,5)"),
    );
    // 复合主键验证行构造器比较按 (a,b) 字典序推进。
    execute(
        &config,
        &format!("CREATE TABLE `{database}`.`c` (a INT, b INT, PRIMARY KEY (a,b)) ENGINE=InnoDB"),
    );
    execute(
        &config,
        &format!("INSERT INTO `{database}`.`c` VALUES (1,1),(1,2),(2,1)"),
    );
    // 快照建立后提交的 DDL 在 REPEATABLE READ 里不可见，测试表必须提前建好。
    execute(
        &config,
        &format!("CREATE TABLE `{database}`.`bin` (id VARBINARY(4) PRIMARY KEY) ENGINE=InnoDB"),
    );
    execute(
        &config,
        &format!("INSERT INTO `{database}`.`bin` VALUES (x'01'),(x'02')"),
    );

    let connector = connector_for(&config).expect("connector");
    let mut snapshot = connector.backup_snapshot(&database).expect("snapshot");
    let path = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
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

/// 仅清理测试自建的库；Drop 在断言失败展开时同样执行。
struct DropDatabase {
    config: ConnectionConfig,
    database: String,
}

impl Drop for DropDatabase {
    fn drop(&mut self) {
        execute(
            &self.config,
            &format!("DROP DATABASE IF EXISTS `{}`", self.database),
        );
    }
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password"]
fn mysql_logical_backup_marks_consistent_snapshot() {
    use fluxdb_core::{
        BackupExecution, BackupObjectKind, BackupObjectRef, BackupRequest, BackupScope,
    };
    use std::sync::atomic::AtomicBool;

    let Some(config) = smoke_config() else {
        return;
    };
    let database = format!("fluxdb_snapshot_dump_{}", std::process::id());
    execute(&config, &format!("DROP DATABASE IF EXISTS `{database}`"));
    execute(&config, &format!("CREATE DATABASE `{database}`"));
    let _cleanup = DropDatabase {
        config: config.clone(),
        database: database.clone(),
    };
    execute(
        &config,
        &format!("CREATE TABLE `{database}`.`a` (id INT PRIMARY KEY) ENGINE=InnoDB"),
    );
    execute(
        &config,
        &format!("CREATE TABLE `{database}`.`b` (id INT PRIMARY KEY) ENGINE=InnoDB"),
    );
    execute(
        &config,
        &format!("INSERT INTO `{database}`.`a` VALUES (1),(2),(3)"),
    );
    execute(&config, &format!("INSERT INTO `{database}`.`b` VALUES (1)"));

    let output = std::env::temp_dir().join(format!("fluxdb_snapshot_{}.sql", std::process::id()));
    let request = BackupRequest {
        config: config.clone(),
        database: database.clone(),
        output: output.clone(),
        execution: BackupExecution::SqlDump,
        tool: std::path::PathBuf::new(),
        tool_version: None,
        scope: BackupScope::Objects(vec![
            BackupObjectRef {
                schema: None,
                kind: BackupObjectKind::Table,
                name: "a".into(),
            },
            BackupObjectRef {
                schema: None,
                kind: BackupObjectKind::Table,
                name: "b".into(),
            },
        ]),
        include_schema: true,
        include_data: true,
        include_routines: false,
        single_transaction: false,
        lock_tables: false,
        include_owner: false,
        include_acl: false,
    };
    let manifest = database_backup(DatabaseKind::MySql)
        .expect("provider")
        .backup(&request, &AtomicBool::new(false), &mut |_| {})
        .expect("backup");
    let text = std::fs::read_to_string(&output).expect("read output");
    let _ = std::fs::remove_file(&output);

    assert_eq!(manifest.consistent, Some(true), "应建立一致快照");
    let rows = manifest
        .object_stats
        .iter()
        .map(|stat| (stat.name.as_str(), stat.rows))
        .collect::<Vec<_>>();
    assert_eq!(rows, vec![("a", Some(3)), ("b", Some(1))]);
    assert!(
        text.contains("-- fluxDB-backup-manifest-v2-begin"),
        "缺少内嵌清单 footer"
    );
    assert!(text.contains("\"consistent\":true"), "清单未标注一致快照");
}

/// 侧边栏统计必须绕过 MySQL 8 的 information_schema 缓存：
/// 先列一次对象填充缓存，再批量写入，第二次列对象应看到新的行数。
#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password"]
fn mysql_object_stats_refresh_after_bulk_insert() {
    let Some(config) = smoke_config() else {
        return;
    };
    let connector = connector_for(&config).expect("connector");
    let database = format!("fluxdb_objects_live_{}", std::process::id());
    execute(&config, &format!("DROP DATABASE IF EXISTS `{database}`"));
    execute(&config, &format!("CREATE DATABASE `{database}`"));
    execute(
        &config,
        &format!("CREATE TABLE `{database}`.`t` (payload VARCHAR(900)) ENGINE=InnoDB"),
    );
    let path = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: database.clone(),
        kind: ObjectKind::Database,
    };
    let rows = |connector: &dyn Connector| {
        connector
            .list_objects(Some(&path))
            .expect("list objects")
            .into_iter()
            .find(|object| object.path.name == "t")
            .and_then(|object| object.rows)
            .unwrap_or(0)
    };
    let before = rows(connector.as_ref());

    execute(
        &config,
        &format!(
            "INSERT INTO `{database}`.`t` (payload) \
             SELECT REPEAT('x', 900) FROM information_schema.columns a \
             JOIN information_schema.columns b ON 1 = 1 LIMIT 5000"
        ),
    );
    let after = rows(connector.as_ref());
    execute(&config, &format!("DROP DATABASE `{database}`"));
    assert_eq!(before, 0, "建表后应为空");
    assert_eq!(after, 5000, "统计缓存未刷新：after={after}");
}
