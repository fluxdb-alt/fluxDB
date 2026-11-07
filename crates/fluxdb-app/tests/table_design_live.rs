use fluxdb_app::AppController;
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlBasicOptions,
    MysqlConnectionProfile, ObjectKind, ObjectPath, QueryExecutionOptions, QueryMode, QueryRequest,
    SecretRef,
};
use std::collections::BTreeMap;

fn execute(config: &ConnectionConfig, text: &str) {
    let result = connector_for(config)
        .expect("connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            session_id: None,
            text: text.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions {
                continue_on_error: false,
                split_statements: true,
                ..QueryExecutionOptions::default()
            },
        })
        .expect("execute setup SQL");
    if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
        panic!("setup SQL failed: {}\n{}", summary.message, text);
    }
}

fn assert_design_snapshot(config: &ConnectionConfig, object: ObjectPath, expected_columns: usize) {
    let mut controller = AppController::new();
    controller.dispatch(fluxdb_app::AppCommand::ReplaceConnections(vec![
        config.clone(),
    ]));
    let create = match controller.dispatch(fluxdb_app::AppCommand::LoadDesignTableBase(object)) {
        fluxdb_app::AppEvent::DesignTableLoaded(Ok(create)) => create,
        fluxdb_app::AppEvent::DesignTableLoaded(Err(error)) => {
            panic!("load design metadata: {}", error.message)
        }
        event => panic!("unexpected design load event: {event:?}"),
    };
    let details_loading = create.details_loading;
    if details_loading {
        assert_eq!(
            create.validation_error(),
            Some("正在读取索引、外键和触发器，请稍候再保存")
        );
    }
    let tab_id = match controller
        .dispatch(fluxdb_app::AppCommand::FinishOpenDesignTable { result: Ok(create) })
    {
        fluxdb_app::AppEvent::TabOpened(tab_id) => tab_id,
        event => panic!("unexpected install design event: {event:?}"),
    };
    if details_loading {
        let object = {
            let tab = controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .expect("design tab");
            let fluxdb_app::TabKind::CreateTable(create) = &tab.kind else {
                panic!("design tab kind");
            };
            match &create.mode {
                fluxdb_app::CreateTableMode::Design { object, .. } => object.clone(),
                fluxdb_app::CreateTableMode::Create => unreachable!(),
            }
        };
        let metadata =
            match controller.dispatch(fluxdb_app::AppCommand::LoadDesignTableDetails(object)) {
                fluxdb_app::AppEvent::DesignTableDetailsLoaded(metadata) => metadata,
                fluxdb_app::AppEvent::Failed(error) => {
                    panic!("load design details: {}", error.message)
                }
                event => panic!("unexpected design details event: {event:?}"),
            };
        let _ = controller
            .dispatch(fluxdb_app::AppCommand::FinishDesignTableDetails { tab_id, metadata });
    }
    let tab = controller
        .state()
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .expect("design tab");
    let fluxdb_app::TabKind::CreateTable(create) = &tab.kind else {
        panic!("design tab kind");
    };
    assert_eq!(create.columns.len(), expected_columns);
    assert!(!create.indexes.is_empty());
    assert!(create.is_design());
    assert!(match &create.mode {
        fluxdb_app::CreateTableMode::Design { original_ddl, .. } => original_ddl
            .as_deref()
            .is_some_and(|ddl| !ddl.trim().is_empty()),
        fluxdb_app::CreateTableMode::Create => false,
    });
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_design_snapshot_uses_one_metadata_read() {
    let raw = std::env::var("FLUXDB_DM_SMOKE").expect("FLUXDB_DM_SMOKE");
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_DM_SMOKE=host:port:user:password");
    let port = parts[1].parse().expect("DM8 port");
    let config = ConnectionConfig {
        id: ConnectionId(9_905),
        name: "DM8 design".into(),
        kind: DatabaseKind::Dameng,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: None,
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(fluxdb_core::DamengConnectionProfile {
            basic: fluxdb_core::DamengBasicOptions {
                host: parts[0].into(),
                port,
                database: "DAMENG".into(),
                schema: "SYSDBA".into(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            ..fluxdb_core::DamengConnectionProfile::default()
        }),
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    };
    let _ = connector_for(&config)
        .expect("DM8 connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: Some("SYSDBA".into()),
            session_id: None,
            text: "DROP TABLE FLUXDB_DESIGN_SNAPSHOT".into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions {
                continue_on_error: true,
                split_statements: true,
                ..QueryExecutionOptions::default()
            },
        });
    execute(
        &config,
        "CREATE TABLE FLUXDB_DESIGN_SNAPSHOT (ID INT PRIMARY KEY, NAME VARCHAR(40) NOT NULL)",
    );
    execute(
        &config,
        "CREATE INDEX IDX_DESIGN_SNAPSHOT_NAME ON FLUXDB_DESIGN_SNAPSHOT(NAME)",
    );
    let object = ObjectPath {
        connection_id: config.id,
        database: None,
        schema: Some("SYSDBA".into()),
        name: "FLUXDB_DESIGN_SNAPSHOT".into(),
        kind: ObjectKind::Table,
    };
    assert_design_snapshot(&config, object, 2);
    execute(&config, "DROP TABLE FLUXDB_DESIGN_SNAPSHOT");
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password 指向可达的 MySQL 8"]
fn mysql_design_snapshot_uses_one_metadata_read() {
    let raw = std::env::var("FLUXDB_MYSQL_SMOKE").expect("FLUXDB_MYSQL_SMOKE");
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_MYSQL_SMOKE=host:port:user:password");
    let port = parts[1].parse().expect("MySQL port");
    let database = "mysql";
    let config = ConnectionConfig {
        id: ConnectionId(9_906),
        name: "MySQL design".into(),
        kind: DatabaseKind::MySql,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some(database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: parts[0].into(),
                port,
                database: database.into(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
                ..fluxdb_core::MysqlBasicOptions::default()
            },
            ..MysqlConnectionProfile::default()
        }),
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }
    .mysql_resolved();
    let table = format!("fluxdb_design_snapshot_{}", std::process::id());
    execute(
        &config,
        &format!("CREATE TABLE {table} (id INT PRIMARY KEY, name VARCHAR(40) NOT NULL)"),
    );
    execute(
        &config,
        &format!("CREATE INDEX idx_{table}_name ON {table}(name)"),
    );
    let object = ObjectPath {
        connection_id: config.id,
        database: Some(database.into()),
        schema: None,
        name: table.clone(),
        kind: ObjectKind::Table,
    };
    assert_design_snapshot(&config, object, 2);
    execute(&config, &format!("DROP TABLE {table}"));
}
