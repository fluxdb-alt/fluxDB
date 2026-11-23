//! MySQL 5.7/8.0 索引元数据兼容性真实连接测试。
//!
//! 仅在显式提供 `FLUXDB_MYSQL_SMOKE=host:port:user:password` 时运行；默认忽略。
//! 测试自建 `fluxdb_index_live_<pid>` 库，结束时只删除本次创建的库。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlBasicOptions,
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
            id: ConnectionId(9_923),
            name: "MySQL index compatibility live".into(),
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
    connector_for(config)
        .expect("connector")
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
fn mysql_list_indexes_across_server_versions() {
    let Some(config) = smoke_config() else {
        return;
    };
    let database = format!("fluxdb_index_live_{}", std::process::id());
    execute(&config, &format!("DROP DATABASE IF EXISTS `{database}`"));
    execute(&config, &format!("CREATE DATABASE `{database}`"));
    let cleanup_config = config.clone();
    let cleanup_database = database.clone();
    struct Cleanup(ConnectionConfig, String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            execute(&self.0, &format!("DROP DATABASE IF EXISTS `{}`", self.1));
        }
    }
    let _cleanup = Cleanup(cleanup_config, cleanup_database);

    execute(
        &config,
        &format!(
            "CREATE TABLE `{database}`.`items` (\
             id INT PRIMARY KEY, name VARCHAR(64), UNIQUE KEY uq_name (name))"
        ),
    );
    let path = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: "items".into(),
        kind: ObjectKind::Table,
    };
    // 同一入口还要提供连接级版本探测，供 UI 按版本裁剪用户与权限入口。
    let version = fluxdb_connectors::mysql_server_version(&config).expect("server version");
    assert!(!version.is_empty());
    let connector = connector_for(&config).expect("connector");
    let indexes = connector.list_indexes(&path).expect("list_indexes");
    assert!(indexes.iter().any(|index| index.is_primary));
    assert!(
        indexes
            .iter()
            .any(|index| index.name == "uq_name" && index.is_unique)
    );
    let design = connector
        .load_table_design_metadata(&path)
        .expect("load_table_design_metadata");
    assert!(design.details_loaded);
    assert!(
        design
            .indexes
            .iter()
            .any(|index| index.name == "uq_name" && index.is_unique)
    );
}
