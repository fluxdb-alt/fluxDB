//! MongoDB 归档备份/恢复真实链路测试。
//!
//! 需要可写的 MongoDB 与 Database Tools：
//!
//! ```text
//! FLUXDB_MONGO_SMOKE=host:port:user:password
//! FLUXDB_MONGO_TOOLS_DIR=/path/to/mongodb-database-tools/bin
//! cargo test --locked -p fluxdb-connectors --test mongo_backup_live -- --ignored --nocapture
//! ```
//!
//! 用例只操作自己创建的两个库，结束时删除并清理归档文件。

use fluxdb_connectors::{MongoConnector, database_backup};
use fluxdb_core::{
    BackupExecution, BackupFormat, BackupMethod, BackupRequest, BackupScope, ConnectionConfig,
    ConnectionId, DatabaseKind, Endpoint, MongoBasicOptions, MongoConnectionProfile, MongoHost,
    RestoreRequest, SecretRef,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_MONGO_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let port = parts[1].parse().ok()?;
    Some(ConnectionConfig {
        id: ConnectionId(9_941),
        name: "MongoDB backup live".into(),
        kind: DatabaseKind::MongoDb,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: None,
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: Some(MongoConnectionProfile {
            basic: MongoBasicOptions {
                hosts: vec![MongoHost {
                    host: parts[0].into(),
                    port,
                }],
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
                auth_database: "admin".into(),
                ..MongoBasicOptions::default()
            },
            ..MongoConnectionProfile::default()
        }),
        oracle_profile: None,
    })
}

fn tools_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("FLUXDB_MONGO_TOOLS_DIR")
            .expect("FLUXDB_MONGO_TOOLS_DIR 指向 mongodump/mongorestore 所在目录"),
    )
}

fn tool(name: &str) -> PathBuf {
    let binary = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    tools_dir().join(binary)
}

fn count(connector: &MongoConnector, database: &str) -> i64 {
    let text = connector
        .run_command(database, r#"{"count":"c"}"#)
        .expect("count 命令失败");
    let value: serde_json::Value = serde_json::from_str(&text).expect("count 不是 JSON");
    value
        .get("n")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_else(|| panic!("count 响应缺少 n：{text}"))
}

struct DropDatabases {
    config: ConnectionConfig,
    databases: Vec<String>,
}

impl Drop for DropDatabases {
    fn drop(&mut self) {
        let connector = MongoConnector::with_config(self.config.clone());
        for database in &self.databases {
            let _ = connector.run_command(database, r#"{"dropDatabase":1}"#);
        }
    }
}

#[test]
#[ignore = "需要 FLUXDB_MONGO_SMOKE 与 FLUXDB_MONGO_TOOLS_DIR"]
fn mongo_archive_backup_restore_round_trip() {
    let Some(config) = smoke_config() else {
        return;
    };
    let source = format!("fluxdb_mongo_src_{}", std::process::id());
    let target = format!("fluxdb_mongo_dst_{}", std::process::id());
    let connector = MongoConnector::with_config(config.clone());
    let _cleanup = DropDatabases {
        config: config.clone(),
        databases: vec![source.clone(), target.clone()],
    };
    for database in [&source, &target] {
        connector
            .run_command(database, r#"{"dropDatabase":1}"#)
            .expect("清理测试库失败");
    }
    connector
        .run_command(
            &source,
            r#"{"insert":"c","documents":[{"_id":1,"v":"x"},{"_id":2,"v":"y"}]}"#,
        )
        .expect("写入源文档失败");
    assert_eq!(count(&connector, &source), 2);

    let output = std::env::temp_dir().join(format!("fluxdb_mongo_{}.archive", std::process::id()));
    let _ = std::fs::remove_file(&output);
    let provider = database_backup(DatabaseKind::MongoDb).expect("provider");
    assert_eq!(
        provider
            .execution(BackupMethod::Auto, true)
            .expect("execution"),
        BackupExecution::MongoDump
    );
    let manifest = provider
        .backup(
            &BackupRequest {
                config: config.clone(),
                database: source.clone(),
                output: output.clone(),
                execution: BackupExecution::MongoDump,
                tool: tool("mongodump"),
                tool_version: None,
                scope: BackupScope::All {
                    include_views: true,
                },
                include_schema: true,
                include_data: true,
                include_routines: false,
                single_transaction: false,
                lock_tables: false,
                include_owner: false,
                include_acl: false,
            },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .expect("备份失败");
    assert_eq!(manifest.database.as_deref(), Some(source.as_str()));
    assert!(output.is_file(), "归档文件未生成");

    let request = RestoreRequest {
        config: config.clone(),
        source: output.clone(),
        target: target.clone(),
        create_target: true,
        tool: tool("mongorestore"),
        manifest: Some(manifest),
        table_decisions: Vec::new(),
        options: Default::default(),
    };
    let plan = provider
        .inspect_restore(&request, &AtomicBool::new(false))
        .expect("恢复预检查失败");
    assert_eq!(plan.format, BackupFormat::MongoArchive);
    assert!(
        provider
            .probe_restore(&request, &AtomicBool::new(false))
            .expect("探测失败")
            .is_empty(),
        "归档不支持逐集合探测"
    );
    provider
        .restore(&request, &plan, &AtomicBool::new(false), &mut |_| {})
        .expect("恢复失败");
    assert_eq!(count(&connector, &target), 2, "目标库文档数应与源库一致");
    assert_eq!(count(&connector, &source), 2, "源库不应被恢复修改");

    let _ = std::fs::remove_file(&output);
}
