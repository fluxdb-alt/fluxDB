use super::{ConformanceTarget, UNICODE_LABEL, run_suite};
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DamengAdvancedOptions, DamengBasicOptions,
    DamengConnectionProfile, DatabaseKind, Endpoint, MysqlBasicOptions, MysqlConnectionProfile,
    ObjectKind, ObjectPath, OracleAdvancedOptions, OracleBasicOptions, OracleConnectKind,
    OracleConnectionProfile, OraclePrivilege, PostgresBasicOptions, PostgresConnectionProfile,
    SecretRef, SqlDialect as _, SqlServerBasicOptions, SqlServerConnectionProfile,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDb(PathBuf);

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    format!("{}_{}", std::process::id(), nanos)
}

fn sqlite_target() -> (ConformanceTarget, TempDb) {
    let suffix = unique_suffix();
    let path = std::env::temp_dir().join(format!("fluxdb-conformance-{suffix}.sqlite"));
    let db = TempDb(path.clone());
    let table_name = "fluxdb_conformance";
    let quoted_table = format!("\"{table_name}\"");
    let config = ConnectionConfig {
        id: ConnectionId(9_970),
        name: "SQLite conformance".into(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path,
            read_only: false,
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    };
    let table = ObjectPath {
        connection_id: config.id,
        database: Some("main".into()),
        schema: None,
        name: table_name.into(),
        kind: ObjectKind::Table,
    };
    let catalog_parent = ObjectPath {
        connection_id: config.id,
        database: Some("main".into()),
        schema: None,
        name: "main".into(),
        kind: ObjectKind::Database,
    };
    let target = ConformanceTarget {
        label: "SQLite",
        config,
        table,
        catalog_parent,
        catalog_database: None,
        bootstrap_sql: Vec::new(),
        bootstrap_teardown_sql: Vec::new(),
        setup_sql: vec![
            format!(
                "CREATE TABLE {quoted_table} (\
                 id INTEGER PRIMARY KEY, \
                 label TEXT NOT NULL, \
                 amount DECIMAL(18,4), \
                 note TEXT)"
            ),
            format!(
                "INSERT INTO {quoted_table} (id, label, amount, note) VALUES \
                 (1, 'alpha', 12.5000, NULL), \
                 (2, '{UNICODE_LABEL_ESCAPED}', -1.2500, 'kept'), \
                 (3, 'nulls', NULL, NULL)",
                UNICODE_LABEL_ESCAPED = UNICODE_LABEL.replace('\'', "''")
            ),
            format!(
                "CREATE UNIQUE INDEX \"fluxdb_conformance_label_idx\" \
                 ON {quoted_table} (label)"
            ),
        ],
        teardown_sql: vec![format!("DROP TABLE {quoted_table}")],
        select_sql: format!("SELECT id, label, amount, note FROM {quoted_table} ORDER BY id"),
        read_only_write_sql: format!(
            "INSERT INTO {quoted_table} (id, label, amount, note) \
             VALUES (99, 'should-not-write', 0, NULL)"
        ),
    };
    (target, db)
}

fn postgres_target() -> Option<ConformanceTarget> {
    let raw = std::env::var("FLUXDB_PG_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_PG_SMOKE=host:port:user:password");
    let port = parts[1].parse().expect("PostgreSQL port");
    Some(postgres_family_target(
        "PostgreSQL",
        DatabaseKind::Postgres,
        ConnectionId(9_971),
        PostgresFamilyConnection {
            host: parts[0],
            port,
            username: parts[2],
            password: parts[3],
            database: "postgres",
        },
    ))
}

fn kingbase_target() -> Option<ConformanceTarget> {
    let raw = std::env::var("FLUXDB_KB_SMOKE").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    assert_eq!(
        parts.len(),
        5,
        "FLUXDB_KB_SMOKE=host:port:user:password:database"
    );
    let port = parts[1].parse().expect("Kingbase port");
    Some(postgres_family_target(
        "Kingbase",
        DatabaseKind::Kingbase,
        ConnectionId(9_975),
        PostgresFamilyConnection {
            host: parts[0],
            port,
            username: parts[2],
            password: parts[3],
            database: parts[4],
        },
    ))
}

struct PostgresFamilyConnection<'a> {
    host: &'a str,
    port: u16,
    username: &'a str,
    password: &'a str,
    database: &'a str,
}

fn postgres_family_target(
    label: &'static str,
    kind: DatabaseKind,
    connection_id: ConnectionId,
    connection: PostgresFamilyConnection<'_>,
) -> ConformanceTarget {
    let PostgresFamilyConnection {
        host,
        port,
        username,
        password,
        database,
    } = connection;
    let schema = "public";
    let table_name = format!("fluxdb_conformance_{}", unique_suffix());
    let qualified_table = format!("\"{schema}\".\"{table_name}\"");
    let config = ConnectionConfig {
        id: connection_id,
        name: format!("{label} conformance"),
        kind,
        endpoint: Endpoint::Tcp {
            host: host.into(),
            port,
            database: Some(database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: host.into(),
                port,
                maintenance_database: database.into(),
                username: username.into(),
                password: SecretRef::inline(password),
            },
            ..PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
    .postgres_resolved();
    let table = ObjectPath {
        connection_id,
        database: Some(database.into()),
        schema: Some(schema.into()),
        name: table_name.clone(),
        kind: ObjectKind::Table,
    };
    let catalog_parent = ObjectPath {
        connection_id,
        database: Some(database.into()),
        schema: None,
        name: schema.into(),
        kind: ObjectKind::Schema,
    };
    let catalog_database = ObjectPath {
        connection_id,
        database: Some(database.into()),
        schema: None,
        name: database.into(),
        kind: ObjectKind::Database,
    };
    ConformanceTarget {
        label,
        config,
        table,
        catalog_parent,
        catalog_database: Some(catalog_database),
        bootstrap_sql: Vec::new(),
        bootstrap_teardown_sql: Vec::new(),
        setup_sql: vec![
            format!(
                "CREATE TABLE {qualified_table} (\
                 id INTEGER PRIMARY KEY, \
                 label VARCHAR(80) NOT NULL, \
                 amount DECIMAL(18,4), \
                 note VARCHAR(200))"
            ),
            format!(
                "INSERT INTO {qualified_table} (id, label, amount, note) VALUES \
                 (1, 'alpha', 12.5000, NULL), \
                 (2, '{UNICODE_LABEL_ESCAPED}', -1.2500, 'kept'), \
                 (3, 'nulls', NULL, NULL)",
                UNICODE_LABEL_ESCAPED = UNICODE_LABEL.replace('\'', "''")
            ),
            format!(
                "CREATE UNIQUE INDEX \"{table_name}_label_idx\" \
                 ON {qualified_table} (label)"
            ),
        ],
        teardown_sql: vec![format!("DROP TABLE {qualified_table}")],
        select_sql: format!("SELECT id, label, amount, note FROM {qualified_table} ORDER BY id"),
        read_only_write_sql: format!(
            "INSERT INTO {qualified_table} (id, label, amount, note) \
             VALUES (99, 'should-not-write', 0, NULL)"
        ),
    }
}

fn mysql_target(
    env_name: &str,
    kind: DatabaseKind,
    label: &'static str,
    connection_id: ConnectionId,
) -> Option<ConformanceTarget> {
    let raw = std::env::var(env_name).ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "{env_name}=host:port:user:password");
    let port = parts[1].parse().expect("MySQL port");
    let database = format!(
        "fluxdb_conformance_{}_{}",
        label.to_ascii_lowercase(),
        unique_suffix()
    );
    let table_name = "fluxdb_conformance";
    let qualified_table = format!(
        "{}.{}",
        fluxdb_core::MySqlDialect.quote_identifier(&database),
        fluxdb_core::MySqlDialect.quote_identifier(table_name)
    );
    let config = ConnectionConfig {
        id: connection_id,
        name: format!("{label} conformance"),
        kind,
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
    .mysql_resolved();
    let table = ObjectPath {
        connection_id,
        database: Some(database.clone()),
        schema: None,
        name: table_name.into(),
        kind: ObjectKind::Table,
    };
    let catalog_parent = ObjectPath {
        connection_id,
        database: Some(database.clone()),
        schema: None,
        name: database.clone(),
        kind: ObjectKind::Database,
    };
    Some(ConformanceTarget {
        label,
        config,
        table,
        catalog_parent,
        catalog_database: None,
        bootstrap_sql: vec![format!(
            "CREATE DATABASE {} CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci",
            fluxdb_core::MySqlDialect.quote_identifier(&database)
        )],
        bootstrap_teardown_sql: vec![format!(
            "DROP DATABASE IF EXISTS {}",
            fluxdb_core::MySqlDialect.quote_identifier(&database)
        )],
        setup_sql: vec![
            format!(
                "CREATE TABLE {qualified_table} (\
                 id INT PRIMARY KEY, \
                 label VARCHAR(80) NOT NULL, \
                 amount DECIMAL(18,4), \
                 note VARCHAR(200), \
                 UNIQUE KEY fluxdb_conformance_label_idx (label)\
                 ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"
            ),
            format!(
                "INSERT INTO {qualified_table} (id, label, amount, note) VALUES \
                 (1, 'alpha', 12.5000, NULL), \
                 (2, '{UNICODE_LABEL_ESCAPED}', -1.2500, 'kept'), \
                 (3, 'nulls', NULL, NULL)",
                UNICODE_LABEL_ESCAPED = UNICODE_LABEL.replace('\'', "''")
            ),
        ],
        teardown_sql: vec![format!("DROP TABLE {qualified_table}")],
        select_sql: format!("SELECT id, label, amount, note FROM {qualified_table} ORDER BY id"),
        read_only_write_sql: format!(
            "INSERT INTO {qualified_table} (id, label, amount, note) \
             VALUES (99, 'should-not-write', 0, NULL)"
        ),
    })
}

fn sqlserver_target() -> Option<ConformanceTarget> {
    let raw = std::env::var("FLUXDB_MSSQL_SMOKE").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    assert_eq!(
        parts.len(),
        5,
        "FLUXDB_MSSQL_SMOKE=host:port:user:password:database"
    );
    let port = parts[1].parse().expect("SQL Server port");
    let initial_database = parts[4];
    let database = format!("fluxdb_conformance_mssql_{}", unique_suffix());
    let schema = "dbo";
    let table_name = "fluxdb_conformance";
    let quoted_database = fluxdb_core::SqlServerDialect.quote_identifier(&database);
    let qualified_table = format!(
        "{}.{}",
        fluxdb_core::SqlServerDialect.quote_identifier(schema),
        fluxdb_core::SqlServerDialect.quote_identifier(table_name)
    );
    let config = ConnectionConfig {
        id: ConnectionId(9_974),
        name: "SQL Server conformance".into(),
        kind: DatabaseKind::SqlServer,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some(initial_database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: Some(SqlServerConnectionProfile {
            basic: SqlServerBasicOptions {
                host: parts[0].into(),
                port,
                database: initial_database.into(),
                schema: schema.into(),
                instance: String::new(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            ..SqlServerConnectionProfile::default()
        }),
        mongo_profile: None,
        oracle_profile: None,
    }
    .sqlserver_resolved();
    let table = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: Some(schema.into()),
        name: table_name.into(),
        kind: ObjectKind::Table,
    };
    let catalog_parent = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: schema.into(),
        kind: ObjectKind::Schema,
    };
    let catalog_database = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: database.clone(),
        kind: ObjectKind::Database,
    };
    Some(ConformanceTarget {
        label: "SQL Server",
        config,
        table,
        catalog_parent,
        catalog_database: Some(catalog_database),
        bootstrap_sql: vec![format!("CREATE DATABASE {quoted_database}")],
        bootstrap_teardown_sql: vec![
            format!("ALTER DATABASE {quoted_database} SET SINGLE_USER WITH ROLLBACK IMMEDIATE"),
            format!("DROP DATABASE {quoted_database}"),
        ],
        setup_sql: vec![
            format!(
                "CREATE TABLE {qualified_table} (\
                 id INT PRIMARY KEY, \
                 label NVARCHAR(80) NOT NULL, \
                 amount DECIMAL(18,4), \
                 note NVARCHAR(200), \
                 CONSTRAINT fluxdb_conformance_label_idx UNIQUE (label)\
                 )"
            ),
            format!(
                "INSERT INTO {qualified_table} (id, label, amount, note) VALUES \
                 (1, N'alpha', 12.5000, NULL), \
                 (2, N'{UNICODE_LABEL_ESCAPED}', -1.2500, N'kept'), \
                 (3, N'nulls', NULL, NULL)",
                UNICODE_LABEL_ESCAPED = UNICODE_LABEL.replace('\'', "''")
            ),
        ],
        teardown_sql: vec![format!("DROP TABLE {qualified_table}")],
        select_sql: format!("SELECT id, label, amount, note FROM {qualified_table} ORDER BY id"),
        read_only_write_sql: format!(
            "INSERT INTO {qualified_table} (id, label, amount, note) \
             VALUES (99, N'should-not-write', 0, NULL)"
        ),
    })
}

fn dameng_target() -> Option<ConformanceTarget> {
    let raw = std::env::var("FLUXDB_DM_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_DM_SMOKE=host:port:user:password");
    let port = parts[1].parse().expect("DM8 port");
    let database = "DAMENG";
    let schema = parts[2].to_ascii_uppercase();
    let table_name = format!("fluxdb_conformance_{}", unique_suffix());
    let quoted_schema = fluxdb_core::DamengDialect.quote_identifier(&schema);
    let quoted_table = fluxdb_core::DamengDialect.quote_identifier(&table_name);
    let qualified_table = format!("{quoted_schema}.{quoted_table}");
    let index_name = format!("{table_name}_label_idx");
    let quoted_index = fluxdb_core::DamengDialect.quote_identifier(&index_name);
    let config = ConnectionConfig {
        id: ConnectionId(9_976),
        name: "DM8 conformance".into(),
        kind: DatabaseKind::Dameng,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some(database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(DamengConnectionProfile {
            basic: DamengBasicOptions {
                host: parts[0].into(),
                port,
                database: database.into(),
                schema: schema.clone(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            advanced: DamengAdvancedOptions {
                connect_timeout_secs: 8,
                query_timeout_secs: 0,
                encoding: String::new(),
                application_name: "FluxDBConformance".into(),
            },
            ssh: Default::default(),
        }),
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    };
    let table = ObjectPath {
        connection_id: config.id,
        database: Some(database.into()),
        schema: Some(schema.clone()),
        name: table_name.clone(),
        kind: ObjectKind::Table,
    };
    let catalog_parent = ObjectPath {
        connection_id: config.id,
        database: Some(database.into()),
        schema: None,
        name: schema.clone(),
        kind: ObjectKind::Schema,
    };
    let catalog_database = ObjectPath {
        connection_id: config.id,
        database: Some(database.into()),
        schema: None,
        name: database.into(),
        kind: ObjectKind::Database,
    };
    Some(ConformanceTarget {
        label: "DM8",
        config,
        table,
        catalog_parent,
        catalog_database: Some(catalog_database),
        bootstrap_sql: Vec::new(),
        bootstrap_teardown_sql: Vec::new(),
        setup_sql: vec![
            format!(
                "CREATE TABLE {qualified_table} (\
                 \"id\" INT PRIMARY KEY, \
                 \"label\" VARCHAR(80) NOT NULL, \
                 \"amount\" DECIMAL(18,4), \
                 \"note\" VARCHAR(200), \
                 CONSTRAINT {quoted_index} UNIQUE (\"label\")\
                 )"
            ),
            format!(
                "INSERT INTO {qualified_table} (\"id\", \"label\", \"amount\", \"note\") VALUES \
                 (1, 'alpha', 12.5000, NULL), \
                 (2, '{UNICODE_LABEL_ESCAPED}', -1.2500, 'kept'), \
                 (3, 'nulls', NULL, NULL)",
                UNICODE_LABEL_ESCAPED = UNICODE_LABEL.replace('\'', "''")
            ),
        ],
        teardown_sql: vec![format!("DROP TABLE {qualified_table}")],
        select_sql: format!(
            "SELECT \"id\", \"label\", \"amount\", \"note\" FROM {qualified_table} ORDER BY \"id\""
        ),
        read_only_write_sql: format!(
            "INSERT INTO {qualified_table} (\"id\", \"label\", \"amount\", \"note\") \
             VALUES (99, 'should-not-write', 0, NULL)"
        ),
    })
}

fn oracle_target() -> Option<ConformanceTarget> {
    let raw = std::env::var("FLUXDB_ORACLE_SMOKE").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    assert_eq!(
        parts.len(),
        5,
        "FLUXDB_ORACLE_SMOKE=host:port:service:user:password"
    );
    let port = parts[1].parse().expect("Oracle port");
    let service = parts[2];
    let owner = parts[3].to_ascii_uppercase();
    let table_name = format!("fluxdb_conformance_{}", unique_suffix());
    let quoted_owner = fluxdb_core::OracleDialect.quote_identifier(&owner);
    let quoted_table = fluxdb_core::OracleDialect.quote_identifier(&table_name);
    let qualified_table = format!("{quoted_owner}.{quoted_table}");
    let index_name = format!("{table_name}_label_idx");
    let quoted_index = fluxdb_core::OracleDialect.quote_identifier(&index_name);
    let config = ConnectionConfig {
        id: ConnectionId(9_977),
        name: "Oracle conformance".into(),
        kind: DatabaseKind::Oracle,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some(service.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: Some(OracleConnectionProfile {
            basic: OracleBasicOptions {
                host: parts[0].into(),
                port,
                connect_kind: OracleConnectKind::ServiceName,
                service_name: service.into(),
                sid: String::new(),
                username: parts[3].into(),
                schema: owner.clone(),
                privilege: OraclePrivilege::Normal,
                password: SecretRef::inline(parts[4]),
            },
            advanced: OracleAdvancedOptions {
                connect_timeout_secs: 8,
                query_timeout_secs: 0,
                application_name: "FluxDBConformance".into(),
                wallet_location: String::new(),
                wallet_password: SecretRef::default(),
            },
            ssh: Default::default(),
        }),
    };
    let table = ObjectPath {
        connection_id: config.id,
        database: Some(service.into()),
        schema: Some(owner.clone()),
        name: table_name.clone(),
        kind: ObjectKind::Table,
    };
    let catalog_parent = ObjectPath {
        connection_id: config.id,
        database: Some(service.into()),
        schema: None,
        name: owner.clone(),
        kind: ObjectKind::Schema,
    };
    let catalog_database = ObjectPath {
        connection_id: config.id,
        database: Some(service.into()),
        schema: None,
        name: service.into(),
        kind: ObjectKind::Database,
    };
    Some(ConformanceTarget {
        label: "Oracle",
        config,
        table,
        catalog_parent,
        catalog_database: Some(catalog_database),
        bootstrap_sql: Vec::new(),
        bootstrap_teardown_sql: Vec::new(),
        setup_sql: vec![
            format!(
                "CREATE TABLE {qualified_table} (\
                 \"id\" NUMBER(10) PRIMARY KEY, \
                 \"label\" VARCHAR2(80) NOT NULL, \
                 \"amount\" NUMBER(18,4), \
                 \"note\" VARCHAR2(200), \
                 CONSTRAINT {quoted_index} UNIQUE (\"label\")\
                 )"
            ),
            format!(
                "INSERT INTO {qualified_table} (\"id\", \"label\", \"amount\", \"note\") VALUES \
                 (1, 'alpha', 12.5000, NULL), \
                 (2, '{UNICODE_LABEL_ESCAPED}', -1.2500, 'kept'), \
                 (3, 'nulls', NULL, NULL)",
                UNICODE_LABEL_ESCAPED = UNICODE_LABEL.replace('\'', "''")
            ),
        ],
        teardown_sql: vec![format!("DROP TABLE {qualified_table} PURGE")],
        select_sql: format!(
            "SELECT \"id\", \"label\", \"amount\", \"note\" FROM {qualified_table} ORDER BY \"id\""
        ),
        read_only_write_sql: format!(
            "INSERT INTO {qualified_table} (\"id\", \"label\", \"amount\", \"note\") \
             VALUES (99, 'should-not-write', 0, NULL)"
        ),
    })
}

#[test]
fn sqlite_sql_conformance() {
    let (target, _db) = sqlite_target();
    run_suite(&target);
}

#[test]
#[ignore = "需要 FLUXDB_PG_SMOKE=host:port:user:password 指向可达的 PostgreSQL"]
fn postgres_sql_conformance() {
    let Some(target) = postgres_target() else {
        eprintln!("跳过：未设置 FLUXDB_PG_SMOKE");
        return;
    };
    run_suite(&target);
}

#[test]
#[ignore = "需要 FLUXDB_KB_SMOKE=host:port:user:password:database 指向可达的 KingbaseES"]
fn kingbase_sql_conformance() {
    let Some(target) = kingbase_target() else {
        eprintln!("跳过：未设置 FLUXDB_KB_SMOKE");
        return;
    };
    run_suite(&target);
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TARGET=host:port:user:password 指向可达的 MySQL"]
fn mysql_sql_conformance() {
    let Some(target) = mysql_target(
        "FLUXDB_MYSQL_TARGET",
        DatabaseKind::MySql,
        "MySQL",
        ConnectionId(9_972),
    ) else {
        eprintln!("跳过：未设置 FLUXDB_MYSQL_TARGET");
        return;
    };
    run_suite(&target);
}

#[test]
#[ignore = "需要 FLUXDB_MARIADB_SMOKE=host:port:user:password 指向可达的 MariaDB"]
fn mariadb_sql_conformance() {
    let Some(target) = mysql_target(
        "FLUXDB_MARIADB_SMOKE",
        DatabaseKind::MariaDb,
        "MariaDB",
        ConnectionId(9_973),
    ) else {
        eprintln!("跳过：未设置 FLUXDB_MARIADB_SMOKE");
        return;
    };
    run_suite(&target);
}

#[test]
#[ignore = "需要 FLUXDB_TIDB_TARGET=host:port:user:password 指向可达的 TiDB"]
fn tidb_sql_conformance() {
    let Some(target) = mysql_target(
        "FLUXDB_TIDB_TARGET",
        DatabaseKind::TiDb,
        "TiDB",
        ConnectionId(9_978),
    ) else {
        eprintln!("跳过：未设置 FLUXDB_TIDB_TARGET");
        return;
    };
    run_suite(&target);
}

#[test]
#[ignore = "需要 FLUXDB_MSSQL_SMOKE=host:port:user:password:database 指向可达的 SQL Server"]
fn sqlserver_sql_conformance() {
    let Some(target) = sqlserver_target() else {
        eprintln!("跳过：未设置 FLUXDB_MSSQL_SMOKE");
        return;
    };
    run_suite(&target);
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_sql_conformance() {
    let Some(target) = dameng_target() else {
        eprintln!("跳过：未设置 FLUXDB_DM_SMOKE");
        return;
    };
    run_suite(&target);
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_SMOKE=host:port:service:user:password 指向可达的 Oracle"]
fn oracle_sql_conformance() {
    let Some(target) = oracle_target() else {
        eprintln!("跳过：未设置 FLUXDB_ORACLE_SMOKE");
        return;
    };
    run_suite(&target);
}
