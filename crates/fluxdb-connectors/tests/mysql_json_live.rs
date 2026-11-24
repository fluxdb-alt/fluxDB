//! MySQL JSON 列查询回归测试。
//!
//! 仅在显式提供 `FLUXDB_MYSQL_SMOKE=host:port:user:password` 时运行；默认忽略。
//! 测试自建 `fluxdb_json_live_<pid>` 库，结束时只删除本次创建的库。
//!
//! 回归点：sqlx 的 MySQL 解码器不接受 `ColumnType::Json`，`SELECT *` 曾把带值的
//! JSON 列静默投影成 NULL（表浏览因 `CAST(... AS CHAR)` 掩盖了该问题）。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    CellValue, ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlBasicOptions,
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
            id: ConnectionId(9_931),
            name: "MySQL JSON live".into(),
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

fn execute(config: &ConnectionConfig, sql: &str) -> fluxdb_core::QueryExecutionResult {
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
        .unwrap_or_else(|error| panic!("execute failed: {sql}: {error}"))
}

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
fn mysql_query_keeps_json_column_values() {
    let Some(config) = smoke_config() else {
        return;
    };
    let database = format!("fluxdb_json_live_{}", std::process::id());
    execute(&config, &format!("DROP DATABASE IF EXISTS `{database}`"));
    execute(&config, &format!("CREATE DATABASE `{database}`"));
    let _cleanup = DropDatabase {
        config: config.clone(),
        database: database.clone(),
    };

    execute(
        &config,
        &format!(
            "CREATE TABLE `{database}`.`agent` (id INT PRIMARY KEY, kb_mounts JSON, tool_mounts JSON)"
        ),
    );
    execute(
        &config,
        &format!(
            "INSERT INTO `{database}`.`agent` VALUES (1, '[{{\"kbId\":\"a\"}}]', '[{{\"sites\":[\"docs\"]}}]')"
        ),
    );

    let result = execute(&config, &format!("SELECT * FROM `{database}`.`agent`"));
    let row = &result.results[0].rows[0].values;
    // 服务端返回规范化后的 JSON 文本（空格形态由 MySQL 决定），关键是值不再为 NULL。
    assert_eq!(row[1], CellValue::Json(r#"[{"kbId": "a"}]"#.into()));
    assert_eq!(row[2], CellValue::Json(r#"[{"sites": ["docs"]}]"#.into()));

    // 表浏览的投影口径也必须仍然有值（防止修复只覆盖查询编辑器）。
    let connector = connector_for(&config).expect("connector");
    let page = connector
        .load_data(
            &ObjectPath {
                connection_id: config.id,
                database: Some(database.clone()),
                schema: None,
                name: "agent".into(),
                kind: ObjectKind::Table,
            },
            0,
            10,
            &[],
            &[],
        )
        .expect("load data");
    assert_ne!(page.rows[0].values[1], CellValue::Null);
    assert_ne!(page.rows[0].values[2], CellValue::Null);
}
