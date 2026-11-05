//! 达梦 DM8 真实连接集成测试。
//!
//! 仅在显式提供 `FLUXDB_DM_SMOKE=host:port:user:password` 时运行；默认忽略，
//! 避免常规单元测试依赖 Podman 或外部数据库。测试使用登录用户同名 schema，
//! 自建 `FLUXDB_TEST_*` 表并在结束时清理。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    CellUpdate, CellValue, ConnectionConfig, ConnectionId, DamengAdvancedOptions,
    DamengBasicOptions, DamengConnectionProfile, DataChangeSet, DatabaseKind, Endpoint, ObjectKind,
    ObjectPath, QueryExecutionOptions, QueryMode, QueryRequest, Row, RowIdentity, RowUpdate,
    SecretRef, SshAuth, SshTunnelProfile,
};
use std::collections::BTreeMap;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_DM_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    Some(ConnectionConfig {
        id: ConnectionId(9_900),
        name: "DM8 smoke".into(),
        kind: DatabaseKind::Dameng,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port: parts[1].parse().ok()?,
            database: Some("DAMENG".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(DamengConnectionProfile {
            basic: DamengBasicOptions {
                host: parts[0].into(),
                port: parts[1].parse().ok()?,
                database: "DAMENG".into(),
                schema: parts[2].to_ascii_uppercase(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            advanced: DamengAdvancedOptions {
                connect_timeout_secs: 5,
                query_timeout_secs: 0,
                encoding: String::new(),
                application_name: "FluxDB test".into(),
            },
            ssh: ssh_profile(),
        }),
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    })
}

/// `FLUXDB_DM_SSH=host:port:user:password`；未设置时返回禁用档案。
fn ssh_profile() -> SshTunnelProfile {
    let Ok(raw) = std::env::var("FLUXDB_DM_SSH") else {
        return SshTunnelProfile::default();
    };
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_DM_SSH=host:port:user:password");
    SshTunnelProfile {
        enabled: true,
        host: parts[0].into(),
        port: parts[1].parse().expect("SSH port"),
        username: parts[2].into(),
        auth: SshAuth::Password,
        password: SecretRef::inline(parts[3]),
        ..SshTunnelProfile::default()
    }
}

fn qualified(schema: &str, table: &str) -> String {
    format!("\"{}\".\"{}\"", schema, table)
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_live_connection_objects_query_and_transaction() {
    let config = smoke_config().expect("FLUXDB_DM_SMOKE must be host:port:user:password");
    let schema = config
        .dameng_profile
        .as_ref()
        .expect("DM profile")
        .basic
        .schema
        .clone();
    let parent = qualified(&schema, "FLUXDB_TEST_PARENT");
    let child = qualified(&schema, "FLUXDB_TEST_CHILD");

    dm_execute_raw(&config, &format!("DROP TABLE IF EXISTS {child}"));
    dm_execute_raw(&config, &format!("DROP TABLE IF EXISTS {parent}"));
    dm_execute_raw(
        &config,
        &format!(
            "CREATE TABLE {parent} (ID INT PRIMARY KEY, NAME VARCHAR(80) NOT NULL, \
             AMOUNT DECIMAL(18,4), CREATED_AT TIMESTAMP)"
        ),
    );
    dm_execute_raw(
        &config,
        &format!(
            "CREATE TABLE {child} (ID INT PRIMARY KEY, PARENT_ID INT NOT NULL, \
             NOTE VARCHAR(200), CONSTRAINT FK_FLUXDB_TEST_CHILD_PARENT \
             FOREIGN KEY (PARENT_ID) REFERENCES {parent}(ID))"
        ),
    );
    dm_execute_raw(
        &config,
        &format!("COMMENT ON TABLE {parent} IS 'FluxDB DM8 integration test'"),
    );
    dm_execute_raw(
        &config,
        &format!("COMMENT ON COLUMN {parent}.NAME IS 'display name'"),
    );
    dm_execute_raw(
        &config,
        &format!("INSERT INTO {parent} (ID, NAME, AMOUNT) VALUES (1, 'alpha', 12.34)"),
    );
    dm_execute_raw(
        &config,
        &format!("INSERT INTO {parent} (ID, NAME, AMOUNT) VALUES (2, '中文', 56.78)"),
    );
    dm_execute_raw(
        &config,
        &format!("INSERT INTO {child} (ID, PARENT_ID, NOTE) VALUES (10, 1, 'child')"),
    );

    let result = std::panic::catch_unwind(|| {
        let connector = connector_for(&config).expect("create DM connector");
        connector
            .test_connection(&config)
            .expect("test DM connection");

        let databases = connector.list_objects(None).expect("list DM databases");
        let database = databases
            .iter()
            .find(|object| object.path.kind == ObjectKind::Database)
            .expect("DM database");
        let schemas = connector
            .list_objects(Some(&database.path))
            .expect("list DM schemas");
        let schema_path = schemas
            .iter()
            .find(|object| object.path.name == schema)
            .expect("login schema");
        let relations = connector
            .list_objects(Some(&schema_path.path))
            .expect("list DM relations");
        assert!(relations.iter().any(|object| {
            object.path.kind == ObjectKind::Table && object.path.name == "FLUXDB_TEST_PARENT"
        }));
        assert!(relations.iter().any(|object| {
            object.path.name == "FLUXDB_TEST_PARENT"
                && object.comment.as_deref() == Some("FluxDB DM8 integration test")
        }));
        // 外键会让 DM 生成 SREF_CON_* 内部表；对象树与补全都不能把它们当作用户表。
        assert!(
            relations
                .iter()
                .all(|object| !object.path.name.starts_with("SREF_CON_"))
        );
        let completion_tables = connector
            .list_completion_tables(None, Some(&schema), "", 10_000)
            .expect("list DM completion tables");
        assert!(completion_tables.iter().any(|table| {
            table.kind == ObjectKind::Table && table.name == "FLUXDB_TEST_PARENT"
        }));
        assert!(
            completion_tables
                .iter()
                .all(|table| !table.name.starts_with("SREF_CON_"))
        );

        let table = ObjectPath {
            connection_id: config.id,
            database: Some("DAMENG".into()),
            schema: Some(schema.clone()),
            name: "FLUXDB_TEST_PARENT".into(),
            kind: ObjectKind::Table,
        };
        let page = connector
            .load_data(&table, 0, 10, &[], &[])
            .expect("load DM data page");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].values[0], CellValue::I64(1));
        assert_eq!(page.rows[0].values[1], CellValue::Text("alpha".into()));
        assert_eq!(page.rows[1].values[1], CellValue::Text("中文".into()));

        let first = connector
            .load_data(&table, 0, 1, &[], &[])
            .expect("first DM page");
        assert_eq!(first.rows.len(), 1);
        assert!(first.has_more);
        let second = connector
            .load_data(&table, 1, 10, &[], &[])
            .expect("second DM page");
        assert_eq!(second.rows.len(), 1);
        assert_eq!(second.rows[0].values[0], CellValue::I64(2));
        assert!(!second.has_more);

        assert!(
            connector
                .list_indexes(&table)
                .expect("DM indexes")
                .iter()
                .any(|index| index.is_primary)
        );
        let child_path = ObjectPath {
            connection_id: config.id,
            database: Some("DAMENG".into()),
            schema: Some(schema.clone()),
            name: "FLUXDB_TEST_CHILD".into(),
            kind: ObjectKind::Table,
        };
        assert!(
            connector
                .list_foreign_keys(&child_path)
                .expect("DM foreign keys")
                .iter()
                .any(|fk| fk.ref_table == "FLUXDB_TEST_PARENT")
        );
        let ddl = connector.table_ddl(&table).expect("DM DDL");
        assert!(ddl.contains("CREATE TABLE"));
        assert!(ddl.contains("COMMENT ON TABLE"));
        assert!(ddl.contains("COMMENT ON COLUMN"));

        let mut identity = BTreeMap::new();
        identity.insert("ID".into(), CellValue::I64(1));
        connector
            .apply_changes(&DataChangeSet {
                object: table.clone(),
                inserts: vec![Row {
                    values: vec![
                        CellValue::I64(7),
                        CellValue::Text("changed".into()),
                        CellValue::Text("8.5".into()),
                        CellValue::Null,
                    ],
                }],
                updates: vec![RowUpdate {
                    identity: RowIdentity { values: identity },
                    cells: vec![CellUpdate::new("NAME", CellValue::Text("updated".into()))],
                }],
                deletes: vec![RowIdentity {
                    values: [("ID".to_string(), CellValue::I64(2))].into(),
                }],
                insert_intents: None,
            })
            .expect("DM apply changes");
        let after = connector
            .load_data(&table, 0, 10, &[], &[])
            .expect("reload after edits");
        assert!(after.rows.iter().any(|row| {
            row.values[0] == CellValue::I64(1) && row.values[1] == CellValue::Text("updated".into())
        }));
        assert!(
            after
                .rows
                .iter()
                .any(|row| row.values[0] == CellValue::I64(7))
        );
        assert!(
            !after
                .rows
                .iter()
                .any(|row| row.values[0] == CellValue::I64(2))
        );

        let profile = config.dameng_profile.as_ref().expect("DM profile");
        // 裸驱动只认原始 host，无法走 SSH 隧道；隧道模式下事务语义由直连 smoke 覆盖。
        if profile.ssh.enabled {
            eprintln!("skip raw-driver transaction check: dm_pure bypasses SSH tunnel");
        } else {
            let mut client = dm_pure::DmClient::new(
                &profile.basic.host,
                profile.basic.port,
                &profile.basic.username,
                profile.password().expect("DM password"),
            );
            client.connect().expect("raw DM connection");
            client.begin().expect("begin transaction");
            client
                .execute(
                    &format!("INSERT INTO {parent} (ID, NAME) VALUES (?, ?)"),
                    &[dm_pure::Param::Int(9), dm_pure::Param::Str("tx".into())],
                )
                .expect("insert in transaction");
            client.rollback().expect("rollback transaction");
            let count = client
                .execute(
                    &format!("SELECT COUNT(*) FROM {parent} WHERE ID = ?"),
                    &[dm_pure::Param::Int(9)],
                )
                .expect("verify rollback");
            let dm_pure::ExecuteResult::Query { rows, .. } = count else {
                panic!("COUNT query must return rows");
            };
            assert_eq!(rows[0][0].as_i64(), Some(0));
            client.close();
        }
    });

    dm_execute_raw(&config, &format!("DROP TABLE IF EXISTS {child}"));
    dm_execute_raw(&config, &format!("DROP TABLE IF EXISTS {parent}"));
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_live_reports_utf8_encoding_for_unicode_text() {
    let config = smoke_config().expect("FLUXDB_DM_SMOKE must be host:port:user:password");
    let profile = config.dameng_profile.as_ref().expect("DM profile");
    // 裸驱动只认原始 host，无法走 SSH 隧道；字符集行为由直连 smoke 覆盖。
    if profile.ssh.enabled {
        eprintln!("skip raw-driver encoding check: dm_pure bypasses SSH tunnel");
        return;
    }
    let text = "中文往返ABC";
    for encoding in [None, Some("utf-8".to_string()), Some("gb18030".to_string())] {
        let mut client = dm_pure::DmClient::with_config(dm_pure::ClientConfig {
            host: profile.basic.host.clone(),
            port: profile.basic.port,
            user: profile.basic.username.clone(),
            password: profile.password().unwrap_or("").to_string(),
            encoding,
            ..dm_pure::ClientConfig::default()
        });
        client.connect().expect("DM encoding connection");
        // 会话字符集必须与驱动实际编解码一致，否则中文会被静默改写。
        let result = client
            .execute("SELECT ? FROM DUAL", &[dm_pure::Param::Str(text.into())])
            .expect("parameter roundtrip");
        match result.rows().and_then(|rows| rows.first()) {
            Some(row) => assert_eq!(
                row.first(),
                Some(&dm_pure::Value::Str(text.into())),
                "server encoding {}",
                client.server_encoding_name()
            ),
            None => panic!(
                "no row for server encoding {}",
                client.server_encoding_name()
            ),
        }
        client.close();
    }
}

/// 通过生产连接器执行准备/清理 SQL。
///
/// 不用裸 `dm_pure` 客户端：裸驱动只认原始 host，会绕过 SSH 隧道。
fn dm_execute_raw(config: &ConnectionConfig, sql: &str) {
    let connector = connector_for(config).expect("DM connector");
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            session_id: None,
            text: sql.to_string(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .expect("DM test SQL");
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "DM SQL failed: {sql} / {:?}",
        result.summaries
    );
}
