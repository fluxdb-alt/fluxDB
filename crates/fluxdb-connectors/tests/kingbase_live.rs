//! KingbaseES 真实连接集成测试。
//!
//! 仅在显式提供 `FLUXDB_KB_SMOKE=host:port:user:password:database` 时运行；默认忽略。
//! 要求服务端使用 PG 兼容模式（非 Oracle 兼容模式），测试自建临时表并在结束时清理。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, ObjectKind, ObjectPath,
    PostgresBasicOptions, PostgresConnectionProfile, QueryExecutionOptions, QueryMode,
    QueryRequest, SecretRef,
};
use std::collections::BTreeMap;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_KB_SMOKE").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    assert_eq!(
        parts.len(),
        5,
        "FLUXDB_KB_SMOKE=host:port:user:password:database"
    );
    let (host, port, user, password, database) = (
        parts[0].to_string(),
        parts[1].parse().expect("Kingbase port"),
        parts[2].to_string(),
        parts[3].to_string(),
        parts[4].to_string(),
    );
    Some(ConnectionConfig {
        id: ConnectionId(9_978),
        name: "Kingbase smoke".into(),
        kind: DatabaseKind::Kingbase,
        endpoint: Endpoint::Tcp {
            host: host.clone(),
            port,
            database: Some(database.clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host,
                port,
                maintenance_database: database,
                username: user,
                password: SecretRef::inline(password),
            },
            ..PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    })
}

fn execute(connector: &dyn fluxdb_core::Connector, config: &ConnectionConfig, sql: &str) {
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            text: sql.into(),
            mode: QueryMode::CurrentStatement,
            options: QueryExecutionOptions {
                page_size: 100,
                ..QueryExecutionOptions::default()
            },
            session_id: None,
        })
        .unwrap_or_else(|err| panic!("执行失败: {sql}: {err}"));
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "语句失败: {sql}: {result:?}"
    );
}

#[test]
#[ignore = "需要 FLUXDB_KB_SMOKE=host:port:user:password:database"]
fn kingbase_reuses_postgres_connector() {
    let config = smoke_config().expect("FLUXDB_KB_SMOKE is required");
    let connector = connector_for(&config).expect("create Kingbase connector");
    assert_eq!(connector.kind(), DatabaseKind::Kingbase);

    connector.test_connection(&config).expect("connect");

    let databases = connector.list_objects(None).expect("list databases");
    assert!(
        databases
            .iter()
            .any(|object| object.path.kind == ObjectKind::Database),
        "至少应列出一个数据库: {databases:?}"
    );

    let database_path = ObjectPath {
        connection_id: config.id,
        database: config
            .postgres_profile
            .as_ref()
            .map(|profile| profile.basic.maintenance_database.clone()),
        schema: None,
        name: "smoke".into(),
        kind: ObjectKind::Database,
    };
    let schemas = connector
        .list_objects(Some(&database_path))
        .expect("list schemas");
    let public = schemas
        .iter()
        .find(|object| object.path.name == "public")
        .expect("public schema")
        .path
        .clone();

    execute(
        connector.as_ref(),
        &config,
        "DROP TABLE IF EXISTS fluxdb_kb_smoke",
    );
    execute(
        connector.as_ref(),
        &config,
        // serial 会生成序列；Kingbase 的 pg_sequence 数值列声明为 int2，
        // DDL 生成曾在这里因按 i64 解码而 panic，必须走一次带序列的表。
        "CREATE TABLE fluxdb_kb_smoke(id serial primary key, name varchar(20))",
    );
    execute(
        connector.as_ref(),
        &config,
        "INSERT INTO fluxdb_kb_smoke VALUES (1, 'alice'), (2, 'bob')",
    );
    execute(
        connector.as_ref(),
        &config,
        "UPDATE fluxdb_kb_smoke SET name = 'carol' WHERE id = 2",
    );

    let tables = connector.list_objects(Some(&public)).expect("list tables");
    let table = tables
        .iter()
        .find(|object| object.path.name == "fluxdb_kb_smoke")
        .expect("临时表应出现在 public")
        .path
        .clone();
    let page = connector
        .load_data(&table, 0, 100, &[], &[])
        .expect("load data");
    assert_eq!(page.rows.len(), 2);
    assert!(
        page.rows
            .iter()
            .any(|row| row.values.iter().any(
                |value| matches!(value, fluxdb_core::CellValue::Text(text) if text == "carol")
            )),
        "写回的数据应可读: {page:?}"
    );

    // 表属性 → DDL：此前 Kingbase 在 pg_sequence 数值列解码处 panic。
    let ddl = connector.table_ddl(&table).expect("table ddl");
    assert!(ddl.contains("CREATE TABLE"), "DDL 缺少建表语句: {ddl}");
    assert!(ddl.contains("fluxdb_kb_smoke"), "DDL 缺少表名: {ddl}");

    execute(connector.as_ref(), &config, "DROP TABLE fluxdb_kb_smoke");
}
