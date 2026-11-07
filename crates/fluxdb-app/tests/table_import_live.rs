use fluxdb_app::{TableImportFormat, TableImportRequest};
use fluxdb_connectors::{connector_for, table_data_import};
use fluxdb_core::{
    ConnectionConfig, ConnectionId, CreateDatabaseRequest, DatabaseKind, Endpoint,
    MysqlBasicOptions, MysqlConnectionProfile, ObjectKind, ObjectPath, PostgresBasicOptions,
    PostgresConnectionProfile, QueryExecutionOptions, QueryMode, QueryRequest, SecretRef,
};
use std::collections::BTreeMap;
use std::io::Write;

fn execute(config: &ConnectionConfig, text: &str) {
    let connector = connector_for(config).expect("SQLite connector");
    let result = connector
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
        .expect("SQLite execute");
    if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
        panic!("SQLite SQL failed: {}\n{}", summary.message, text);
    }
}

#[test]
fn sqlite_imports_all_export_formats() {
    let dir = std::env::temp_dir().join(format!("fluxdb-sqlite-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let database = dir.join("import.sqlite");
    let _ = std::fs::remove_file(&database);
    let config = ConnectionConfig {
        id: ConnectionId(9_902),
        name: "SQLite import".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: database.clone(),
            read_only: false,
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    };
    execute(
        &config,
        "CREATE TABLE items (ID INTEGER PRIMARY KEY, NAME TEXT, AMOUNT REAL, CREATED_AT TEXT)",
    );
    let object = ObjectPath {
        connection_id: config.id,
        database: None,
        schema: None,
        name: "items".into(),
        kind: ObjectKind::Table,
    };
    for (format, extension, body) in [
        (
            TableImportFormat::Txt,
            "txt",
            "ID\tNAME\tAMOUNT\tCREATED_AT\n1\talpha\t12.5\t2026-01-01\n2\tbeta\tNULL\tNULL\n",
        ),
        (
            TableImportFormat::Csv,
            "csv",
            "ID,NAME,AMOUNT,CREATED_AT\n3,gamma,13.5,2026-01-03\n4,delta,NULL,NULL\n",
        ),
        (
            TableImportFormat::Json,
            "json",
            r#"[{"ID":5,"NAME":"epsilon","AMOUNT":14.5,"CREATED_AT":"2026-01-05"},{"ID":6,"NAME":"zeta","AMOUNT":null,"CREATED_AT":null}]"#,
        ),
        (
            TableImportFormat::Xml,
            "xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><rows><row><field name="ID">7</field><field name="NAME">eta</field><field name="AMOUNT">15.5</field><field name="CREATED_AT">2026-01-07</field></row><row><field name="ID">8</field><field name="NAME">theta</field><field name="AMOUNT">NULL</field><field name="CREATED_AT">NULL</field></row></rows>"#,
        ),
    ] {
        let path = dir.join(format!("sample.{extension}"));
        let mut file = std::fs::File::create(&path).expect("sample file");
        file.write_all(body.as_bytes()).expect("write sample");
        let report = table_data_import(DatabaseKind::Sqlite)
            .expect("SQLite import provider")
            .import(
                &TableImportRequest {
                    config: config.clone(),
                    object: object.clone(),
                    source: path,
                    format,
                    options: fluxdb_core::TableImportOptions::new(),
                    column_mapping: Vec::new(),
                },
                &std::sync::atomic::AtomicBool::new(false),
                &mut |_| {},
            )
            .expect("SQLite import");
        assert_eq!(report.imported_rows, 2, "{format:?}");
    }
    let connector = connector_for(&config).expect("SQLite connector");
    let page = connector
        .load_data(&object, 0, 20, &[], &[])
        .expect("imported rows");
    assert_eq!(page.rows.len(), 8);
    assert!(
        page.rows
            .iter()
            .any(|row| row.values[1].display_label() == "epsilon")
    );
    assert!(
        page.rows
            .iter()
            .any(|row| row.values[2].display_label() == "12.5")
    );
    assert!(
        page.rows
            .iter()
            .any(|row| row.values[3].display_label() == "2026-01-07")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE=host:port:user:password 指向可达的 MySQL 8"]
fn mysql_imports_all_export_formats() {
    let raw = std::env::var("FLUXDB_MYSQL_SMOKE").expect("FLUXDB_MYSQL_SMOKE");
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_MYSQL_SMOKE=host:port:user:password");
    let port = parts[1].parse().expect("MySQL port");
    let database = format!("fluxdb_import_{}", std::process::id());
    let config = ConnectionConfig {
        id: ConnectionId(9_903),
        name: "MySQL import".into(),
        kind: DatabaseKind::MySql,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some(database.clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: parts[0].into(),
                port,
                database: database.clone(),
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
    connector_for(&config)
        .expect("MySQL connector")
        .create_database(&CreateDatabaseRequest {
            connection_id: config.id,
            name: database.clone(),
            charset: "utf8mb4".into(),
            collation: "utf8mb4_general_ci".into(),
            owner: String::new(),
            template: String::new(),
            path: None,
        })
        .expect("create MySQL database");
    let result = std::panic::catch_unwind(|| {
        execute(
            &config,
            "CREATE TABLE items (ID INT PRIMARY KEY, NAME VARCHAR(40), AMOUNT DECIMAL(10,2), CREATED_AT DATETIME)",
        );
        let object = ObjectPath {
            connection_id: config.id,
            database: Some(database.clone()),
            schema: None,
            name: "items".into(),
            kind: ObjectKind::Table,
        };
        let dir = std::env::temp_dir().join(format!("fluxdb-mysql-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (format, extension, body) in [
            (
                TableImportFormat::Txt,
                "txt",
                "ID\tNAME\tAMOUNT\tCREATED_AT\n1\talpha\t12.50\t2026-01-01 00:00:00\n2\tbeta\tNULL\tNULL\n",
            ),
            (
                TableImportFormat::Csv,
                "csv",
                "ID,NAME,AMOUNT,CREATED_AT\n3,gamma,13.50,2026-01-03 00:00:00\n4,delta,NULL,NULL\n",
            ),
            (
                TableImportFormat::Json,
                "json",
                r#"[{"ID":5,"NAME":"epsilon","AMOUNT":14.5,"CREATED_AT":"2026-01-05 00:00:00"},{"ID":6,"NAME":"zeta","AMOUNT":null,"CREATED_AT":null}]"#,
            ),
            (
                TableImportFormat::Xml,
                "xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><rows><row><field name="ID">7</field><field name="NAME">eta</field><field name="AMOUNT">15.50</field><field name="CREATED_AT">2026-01-07 00:00:00</field></row><row><field name="ID">8</field><field name="NAME">theta</field><field name="AMOUNT">NULL</field><field name="CREATED_AT">NULL</field></row></rows>"#,
            ),
        ] {
            let path = dir.join(format!("sample.{extension}"));
            let mut file = std::fs::File::create(&path).expect("sample file");
            file.write_all(body.as_bytes()).expect("write sample");
            let report = table_data_import(DatabaseKind::MySql)
                .expect("MySQL import provider")
                .import(
                    &TableImportRequest {
                        config: config.clone(),
                        object: object.clone(),
                        source: path,
                        format,
                        options: fluxdb_core::TableImportOptions::new(),
                        column_mapping: Vec::new(),
                    },
                    &std::sync::atomic::AtomicBool::new(false),
                    &mut |_| {},
                )
                .expect("MySQL import");
            assert_eq!(report.imported_rows, 2, "{format:?}");
        }
        let page = connector_for(&config)
            .expect("MySQL connector")
            .load_data(&object, 0, 20, &[], &[])
            .expect("MySQL imported rows");
        assert_eq!(page.rows.len(), 8);
        assert!(
            page.rows
                .iter()
                .any(|row| row.values[1].display_label() == "epsilon")
        );
        let _ = std::fs::remove_dir_all(dir);
    });
    let _ = connector_for(&config)
        .expect("MySQL connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            session_id: None,
            text: format!("DROP DATABASE `{database}`"),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        });
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
#[ignore = "需要 FLUXDB_PG_SMOKE=host:port:user:password 指向可达的 PostgreSQL 16"]
fn postgres_imports_all_export_formats() {
    let raw = std::env::var("FLUXDB_PG_SMOKE").expect("FLUXDB_PG_SMOKE");
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_PG_SMOKE=host:port:user:password");
    let port = parts[1].parse().expect("PostgreSQL port");
    let database = format!("fluxdb_import_{}", std::process::id());
    let config = ConnectionConfig {
        id: ConnectionId(9_904),
        name: "PostgreSQL import".into(),
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
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }
    .postgres_resolved();
    connector_for(&config)
        .expect("PostgreSQL connector")
        .create_database(&CreateDatabaseRequest {
            connection_id: config.id,
            name: database.clone(),
            charset: "UTF8".into(),
            collation: String::new(),
            owner: String::new(),
            template: String::new(),
            path: None,
        })
        .expect("create PostgreSQL database");
    let result = std::panic::catch_unwind(|| {
        let mut database_config = ConnectionConfig {
            endpoint: Endpoint::Tcp {
                host: parts[0].into(),
                port,
                database: Some(database.clone()),
            },
            ..config.clone()
        };
        if let Some(profile) = &mut database_config.postgres_profile {
            profile.basic.maintenance_database = database.clone();
        }
        let database_config = database_config.postgres_resolved();
        execute(
            &database_config,
            "CREATE TABLE items (ID INTEGER GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, NAME TEXT, AMOUNT NUMERIC(10,2), CREATED_AT TIMESTAMP, AMOUNTS NUMERIC(12,4)[])",
        );
        let object = ObjectPath {
            connection_id: config.id,
            database: Some(database.clone()),
            schema: Some("public".into()),
            name: "items".into(),
            kind: ObjectKind::Table,
        };
        let dir = std::env::temp_dir().join(format!("fluxdb-pg-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (format, extension, body) in [
            (
                TableImportFormat::Txt,
                "txt",
                "ID\tNAME\tAMOUNT\tCREATED_AT\n1\talpha\t12.50\t2026-01-01 00:00:00\n2\tbeta\tNULL\tNULL\n",
            ),
            (
                TableImportFormat::Csv,
                "csv",
                "ID,NAME,AMOUNT,CREATED_AT\n3,gamma,13.50,2026-01-03 00:00:00\n4,delta,NULL,NULL\n",
            ),
            (
                TableImportFormat::Json,
                "json",
                r#"[{"ID":5,"NAME":"epsilon","AMOUNT":14.5,"CREATED_AT":"2026-01-05 00:00:00","AMOUNTS":"{1.2500,2.5000}"},{"ID":6,"NAME":"zeta","AMOUNT":null,"CREATED_AT":null,"AMOUNTS":null}]"#,
            ),
            (
                TableImportFormat::Xml,
                "xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><rows><row><field name="ID">7</field><field name="NAME">eta</field><field name="AMOUNT">15.50</field><field name="CREATED_AT">2026-01-07 00:00:00</field></row><row><field name="ID">8</field><field name="NAME">theta</field><field name="AMOUNT">NULL</field><field name="CREATED_AT">NULL</field></row></rows>"#,
            ),
        ] {
            let path = dir.join(format!("sample.{extension}"));
            let mut file = std::fs::File::create(&path).expect("sample file");
            file.write_all(body.as_bytes()).expect("write sample");
            let report = table_data_import(DatabaseKind::Postgres)
                .expect("PostgreSQL import provider")
                .import(
                    &TableImportRequest {
                        config: database_config.clone(),
                        object: object.clone(),
                        source: path,
                        format,
                        options: fluxdb_core::TableImportOptions::new(),
                        column_mapping: Vec::new(),
                    },
                    &std::sync::atomic::AtomicBool::new(false),
                    &mut |_| {},
                )
                .expect("PostgreSQL import");
            assert_eq!(report.imported_rows, 2, "{format:?}");
        }
        let page = connector_for(&database_config)
            .expect("PostgreSQL connector")
            .load_data(&object, 0, 20, &[], &[])
            .expect("PostgreSQL imported rows");
        assert_eq!(page.rows.len(), 8);
        assert!(
            page.rows
                .iter()
                .any(|row| row.values[1].display_label() == "epsilon")
        );
        let _ = std::fs::remove_dir_all(dir);
    });
    execute(&config, &format!("DROP DATABASE \"{database}\""));
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn sqlite_import_append_and_truncate_modes() {
    use fluxdb_core::TableImportMode;

    let dir = std::env::temp_dir().join(format!("fluxdb-import-modes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let database = dir.join("modes.sqlite");
    let config = ConnectionConfig {
        id: ConnectionId(9_950),
        name: "SQLite import modes".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: database.clone(),
            read_only: false,
        },
        options: std::collections::BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    };
    let object = ObjectPath {
        connection_id: config.id,
        database: None,
        schema: None,
        name: "items".into(),
        kind: ObjectKind::Table,
    };
    connector_for(&config)
        .expect("SQLite connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            session_id: None,
            text: "CREATE TABLE items (ID INTEGER, NAME TEXT)".into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .expect("create table");

    let csv = dir.join("rows.csv");
    std::fs::write(&csv, "ID,NAME\n1,alpha\n2,beta\n").expect("write csv");
    let provider = table_data_import(DatabaseKind::Sqlite).expect("provider");

    // 追加：两次导入应累计 4 行。
    for _ in 0..2 {
        provider
            .import(
                &TableImportRequest {
                    config: config.clone(),
                    object: object.clone(),
                    source: csv.clone(),
                    format: TableImportFormat::Csv,
                    options: fluxdb_core::TableImportOptions::new(),
                    column_mapping: Vec::new(),
                },
                &std::sync::atomic::AtomicBool::new(false),
                &mut |_| {},
            )
            .expect("append import");
    }
    let count = |config: &ConnectionConfig, object: &ObjectPath| -> usize {
        connector_for(config)
            .expect("connector")
            .load_data(object, 0, 50, &[], &[])
            .expect("load")
            .rows
            .len()
    };
    assert_eq!(count(&config, &object), 4, "append should accumulate rows");

    // 清空后导入：结果应只剩本次 2 行。
    let options = fluxdb_core::TableImportOptions {
        mode: TableImportMode::Truncate,
        ..fluxdb_core::TableImportOptions::new()
    };
    provider
        .import(
            &TableImportRequest {
                config: config.clone(),
                object: object.clone(),
                source: csv.clone(),
                format: TableImportFormat::Csv,
                options,
                column_mapping: Vec::new(),
            },
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_| {},
        )
        .expect("truncate import");
    assert_eq!(count(&config, &object), 2, "truncate should replace rows");

    // 空文件不允许清空：必须保留原有数据。
    let empty = dir.join("empty.csv");
    std::fs::write(&empty, "ID,NAME\n").expect("write empty csv");
    provider
        .import(
            &TableImportRequest {
                config: config.clone(),
                object: object.clone(),
                source: empty,
                format: TableImportFormat::Csv,
                options: fluxdb_core::TableImportOptions {
                    mode: TableImportMode::Truncate,
                    ..fluxdb_core::TableImportOptions::new()
                },
                column_mapping: Vec::new(),
            },
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_| {},
        )
        .expect("empty import is a no-op");
    assert_eq!(count(&config, &object), 2, "empty file must not truncate");

    let _ = std::fs::remove_dir_all(dir);
}
