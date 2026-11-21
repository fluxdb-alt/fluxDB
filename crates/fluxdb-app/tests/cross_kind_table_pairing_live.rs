//! 显式表配对（`table_map`）在 MySQL 家族之外的方言上真实落库验证。
//!
//! 每种目标库单独用 smoke 环境变量开启，默认忽略：
//! - `FLUXDB_PAIR_MYSQL=host:port:user:password:database`（锚点 MySQL）
//! - `FLUXDB_MARIADB_SMOKE=host:port:user:password:database`
//! - `FLUXDB_TIDB_SMOKE=host:port:user:password:database`
//! - `FLUXDB_MSSQL_SMOKE=host:port:user:password:database`
//! - `FLUXDB_ORACLE_SMOKE=host:port:service:user:password`
//! - `FLUXDB_DM_SMOKE=host:port:user:password`
//! - `FLUXDB_KB_SMOKE=host:port:user:password:database`
//!
//! 每种库验证三段：MySQL 源表 → 改名目标表的结构建表；MySQL 源行 → 改名目标表
//! 的数据写入；目标库源行 → MySQL 改名目标表的数据写入。断言只放在业务调用之外，
//! 驱动回调里 panic 无法展开（会直接 abort 进程）。

use fluxdb_app::{
    AppCommand, AppController, DataSyncOptions, DataSyncRequest, StructureSyncOptions,
    StructureSyncRequest, SyncEndpoint,
};
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DamengBasicOptions, DamengConnectionProfile, DatabaseKind,
    Endpoint, MysqlBasicOptions, MysqlConnectionProfile, ObjectKind, ObjectPath,
    OracleBasicOptions, OracleConnectKind, OracleConnectionProfile, PostgresBasicOptions,
    PostgresConnectionProfile, QueryExecutionOptions, QueryMode, QueryRequest, SecretRef,
    SqlServerBasicOptions, SqlServerConnectionProfile,
};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

fn smoke_parts(key: &str, expected: usize) -> Option<Vec<String>> {
    let raw = std::env::var(key).ok()?;
    let parts = raw
        .splitn(expected, ':')
        .map(str::to_string)
        .collect::<Vec<_>>();
    if parts.len() != expected {
        return None;
    }
    Some(parts)
}

fn mysql_family_config(
    id: u64,
    kind: DatabaseKind,
    parts: &[String],
    database: &str,
) -> ConnectionConfig {
    let port = parts[1].parse().expect("MySQL 端口");
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("{kind:?} {database}"),
        kind,
        endpoint: Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some(database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: parts[0].clone(),
                port,
                database: database.into(),
                username: parts[2].clone(),
                password: SecretRef::inline(parts[3].clone()),
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
    .mysql_resolved()
}

fn mssql_config(id: u64, parts: &[String]) -> ConnectionConfig {
    let port = parts[1].parse().expect("SQL Server 端口");
    let database = parts[4].clone();
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("SQL Server {database}"),
        kind: DatabaseKind::SqlServer,
        endpoint: Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some(database.clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: Some(SqlServerConnectionProfile {
            basic: SqlServerBasicOptions {
                host: parts[0].clone(),
                port,
                database,
                schema: "dbo".into(),
                instance: String::new(),
                username: parts[2].clone(),
                password: SecretRef::inline(parts[3].clone()),
            },
            ..SqlServerConnectionProfile::default()
        }),
        mongo_profile: None,
        oracle_profile: None,
    }
}

fn oracle_config(id: u64, parts: &[String]) -> ConnectionConfig {
    let port = parts[1].parse().expect("Oracle 端口");
    let service = parts[2].clone();
    let user = parts[3].to_ascii_uppercase();
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("Oracle {service}"),
        kind: DatabaseKind::Oracle,
        endpoint: Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some(service.clone()),
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
                host: parts[0].clone(),
                port,
                connect_kind: OracleConnectKind::ServiceName,
                service_name: service,
                sid: String::new(),
                username: user.clone(),
                schema: user,
                privilege: fluxdb_core::OraclePrivilege::Normal,
                password: SecretRef::inline(parts[4].clone()),
            },
            ..OracleConnectionProfile::default()
        }),
    }
}

fn dameng_config(id: u64, parts: &[String]) -> ConnectionConfig {
    let port = parts[1].parse().expect("达梦端口");
    let user = parts[2].to_ascii_uppercase();
    ConnectionConfig {
        id: ConnectionId(id),
        name: "达梦 DAMENG".into(),
        kind: DatabaseKind::Dameng,
        endpoint: Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some("DAMENG".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(DamengConnectionProfile {
            basic: DamengBasicOptions {
                host: parts[0].clone(),
                port,
                database: "DAMENG".into(),
                schema: user.clone(),
                username: user,
                password: SecretRef::inline(parts[3].clone()),
            },
            ..DamengConnectionProfile::default()
        }),
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
}

fn kingbase_config(id: u64, parts: &[String]) -> ConnectionConfig {
    let port = parts[1].parse().expect("Kingbase 端口");
    let database = parts[4].clone();
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("Kingbase {database}"),
        kind: DatabaseKind::Kingbase,
        endpoint: Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some(database.clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: parts[0].clone(),
                port,
                maintenance_database: database,
                username: parts[2].clone(),
                password: SecretRef::inline(parts[3].clone()),
            },
            ..PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
}

fn execute(config: &ConnectionConfig, sql: &str) {
    let result = connector_for(config)
        .expect("connector")
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
        .unwrap_or_else(|error| panic!("执行失败: {sql}: {error}"));
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "语句失败: {sql}: {result:?}"
    );
}

fn path(config: &ConnectionConfig, database: &str, schema: Option<&str>, name: &str) -> ObjectPath {
    ObjectPath {
        connection_id: config.id,
        database: Some(database.into()),
        schema: schema.map(str::to_string),
        name: name.into(),
        kind: ObjectKind::Table,
    }
}

fn row_count(config: &ConnectionConfig, path: &ObjectPath) -> usize {
    connector_for(config)
        .expect("connector")
        .load_data(path, 0, 100, &[], &[])
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.name))
        .rows
        .len()
}

fn columns(config: &ConnectionConfig, path: &ObjectPath) -> Vec<String> {
    connector_for(config)
        .expect("connector")
        .load_data(path, 0, 1, &[], &[])
        .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.name))
        .columns
        .into_iter()
        .map(|column| column.name.to_ascii_lowercase())
        .collect()
}

fn table_missing(config: &ConnectionConfig, path: &ObjectPath) -> bool {
    connector_for(config)
        .expect("connector")
        .load_data(path, 0, 1, &[], &[])
        .is_err()
}

fn qualified(path: &ObjectPath) -> String {
    match path.schema.as_deref().filter(|schema| !schema.is_empty()) {
        Some(schema) => format!("{schema}.{}", path.name),
        None => path.name.clone(),
    }
}

/// 只删本用例创建的表；不依赖各方言是否支持 `DROP TABLE IF EXISTS`。
fn drop_if_exists(config: &ConnectionConfig, path: &ObjectPath) {
    if !table_missing(config, path) {
        execute(config, &format!("DROP TABLE {}", qualified(path)));
    }
}

/// 每种库的命名/方言信息。
struct KindCase {
    label: &'static str,
    /// 目标库连接与端点（连接 id 固定为 2）。
    target: ConnectionConfig,
    target_ep: SyncEndpoint,
    /// 目标端建表 SQL；`{}` 处填物理表名。
    ddl: fn(&str) -> String,
    /// 结构同步目标表（完整配对键）与物理 SQL 名。
    struct_target_key: String,
    struct_target_name: &'static str,
    /// 数据同步目标表（完整配对键）与物理 SQL 名。
    data_target_key: String,
    data_target_name: &'static str,
    /// 反向数据同步的源表（完整配对键 / 物理 SQL 名）。
    reverse_source_key: String,
    reverse_source_name: &'static str,
}

fn run_kind_case(case: KindCase) {
    let Some(mysql_parts) = smoke_parts("FLUXDB_PAIR_MYSQL", 5) else {
        return;
    };
    let mysql_db = mysql_parts[4].clone();
    let mysql = mysql_family_config(1, DatabaseKind::MySql, &mysql_parts, &mysql_db);
    let source_ep = SyncEndpoint::new(ConnectionId(1), mysql_db.clone(), None);
    let controller = {
        let mut controller = AppController::new();
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![
            mysql.clone(),
            case.target.clone(),
        ]));
        controller
    };
    let cancel = AtomicBool::new(false);
    let target_name = case.struct_target_name.to_string();

    // 1. 结构同步：MySQL 源表建到改名目标表。
    execute(&mysql, "DROP TABLE IF EXISTS pair_src");
    execute(
        &mysql,
        "CREATE TABLE pair_src (id INT NOT NULL PRIMARY KEY, name VARCHAR(32) NOT NULL)",
    );
    let target_path = path(
        &case.target,
        case.target_ep.database.as_str(),
        case.target_ep.schema.as_deref(),
        &target_name,
    );
    drop_if_exists(&case.target, &target_path);
    let structure_request = StructureSyncRequest {
        source: source_ep.clone(),
        target: case.target_ep.clone(),
        tables: vec!["pair_src".into()],
        options: StructureSyncOptions {
            table_map: BTreeMap::from([("pair_src".to_string(), case.struct_target_key.clone())]),
            ..StructureSyncOptions::default()
        },
    };
    let preview = controller
        .preview_structure_sync(&structure_request, &cancel, &mut |_| {})
        .expect("结构预览失败");
    assert_eq!(preview.tables.len(), 1, "{}: {preview:?}", case.label);
    assert!(
        preview.tables[0].error.is_none(),
        "{}: {:?}",
        case.label,
        preview.tables[0]
    );
    let report = controller
        .execute_structure_sync(&structure_request, &cancel, &mut |_| {})
        .expect("结构执行失败");
    assert!(
        report.tables.iter().all(|table| table.error.is_none()),
        "{}: {report:?}",
        case.label
    );
    let created = columns(&case.target, &target_path);
    assert!(
        created.contains(&"id".to_string()) && created.contains(&"name".to_string()),
        "{}: 改名表缺少列 {created:?}",
        case.label
    );

    // 2. 数据同步：MySQL 行写入改名目标表。
    execute(&mysql, "DROP TABLE IF EXISTS pair_rows");
    execute(
        &mysql,
        "CREATE TABLE pair_rows (id INT NOT NULL PRIMARY KEY, name VARCHAR(32) NOT NULL)",
    );
    execute(&mysql, "INSERT INTO pair_rows (id, name) VALUES (1, 'a')");
    execute(&mysql, "INSERT INTO pair_rows (id, name) VALUES (2, 'b')");
    let data_path = path(
        &case.target,
        case.target_ep.database.as_str(),
        case.target_ep.schema.as_deref(),
        case.data_target_name,
    );
    drop_if_exists(&case.target, &data_path);
    execute(&case.target, &(case.ddl)(case.data_target_name));
    let data_request = DataSyncRequest {
        source: source_ep.clone(),
        target: case.target_ep.clone(),
        tables: vec!["pair_rows".into()],
        options: DataSyncOptions {
            insert: true,
            table_map: BTreeMap::from([("pair_rows".to_string(), case.data_target_key.clone())]),
            ..DataSyncOptions::default()
        },
    };
    let report = controller
        .execute_data_sync(&data_request, &cancel, &mut |_| {})
        .expect("数据执行失败");
    assert!(
        report.failed_tables().is_empty(),
        "{}: {report:?}",
        case.label
    );
    assert_eq!(
        row_count(&case.target, &data_path),
        2,
        "{}: 行必须落到显式配对表",
        case.label
    );

    // 3. 反向数据同步：目标库源行写入 MySQL 改名目标表。
    let reverse_source_path = path(
        &case.target,
        case.target_ep.database.as_str(),
        case.target_ep.schema.as_deref(),
        case.reverse_source_name,
    );
    drop_if_exists(&case.target, &reverse_source_path);
    execute(&case.target, &(case.ddl)(case.reverse_source_name));
    let insert = |id: u32, name: &str| {
        format!(
            "INSERT INTO {}(id, name) VALUES ({id}, '{name}')",
            case.reverse_source_name
        )
    };
    for (id, name) in [(1, "a"), (2, "b"), (3, "c")] {
        execute(&case.target, &insert(id, name));
    }
    let reverse_mysql_table = format!("pair_reverse_{}", case.label);
    execute(
        &mysql,
        &format!("DROP TABLE IF EXISTS {reverse_mysql_table}"),
    );
    execute(
        &mysql,
        &format!(
            "CREATE TABLE {reverse_mysql_table} (id INT NOT NULL PRIMARY KEY, name VARCHAR(32) NOT NULL)"
        ),
    );
    let reverse_request = DataSyncRequest {
        source: case.target_ep.clone(),
        target: source_ep.clone(),
        tables: vec![case.reverse_source_key.clone()],
        options: DataSyncOptions {
            insert: true,
            table_map: BTreeMap::from([(
                case.reverse_source_key.clone(),
                reverse_mysql_table.clone(),
            )]),
            ..DataSyncOptions::default()
        },
    };
    let report = controller
        .execute_data_sync(&reverse_request, &cancel, &mut |_| {})
        .expect("反向数据执行失败");
    assert!(
        report.failed_tables().is_empty(),
        "{}: {report:?}",
        case.label
    );
    let reverse_path = path(&mysql, &mysql_db, None, &reverse_mysql_table);
    assert_eq!(
        row_count(&mysql, &reverse_path),
        3,
        "{}: 反向行必须落到显式配对表",
        case.label
    );

    // 收尾：只清理本用例创建的改名表。
    drop_if_exists(&case.target, &target_path);
    drop_if_exists(&case.target, &data_path);
    drop_if_exists(&case.target, &reverse_source_path);
}

fn mysql_family_ddl(name: &str) -> String {
    format!("CREATE TABLE {name} (id INT NOT NULL PRIMARY KEY, name VARCHAR(32) NOT NULL)")
}

#[test]
#[ignore = "需要 FLUXDB_PAIR_MYSQL 与 FLUXDB_MARIADB_SMOKE"]
fn mariadb_renamed_pairing_lands() {
    let Some(parts) = smoke_parts("FLUXDB_MARIADB_SMOKE", 5) else {
        return;
    };
    let db = parts[4].clone();
    let target = mysql_family_config(2, DatabaseKind::MariaDb, &parts, &db);
    // 库不存在时先连系统库建库，再连目标库。
    let setup = mysql_family_config(2, DatabaseKind::MariaDb, &parts, "mysql");
    execute(&setup, &format!("CREATE DATABASE IF NOT EXISTS {db}"));
    run_kind_case(KindCase {
        label: "mariadb",
        target_ep: SyncEndpoint::new(ConnectionId(2), db, None),
        target,
        ddl: mysql_family_ddl,
        struct_target_key: "pair_struct_mariadb".into(),
        struct_target_name: "pair_struct_mariadb",
        data_target_key: "pair_rows_mariadb".into(),
        data_target_name: "pair_rows_mariadb",
        reverse_source_key: "pair_src_rows_mariadb".into(),
        reverse_source_name: "pair_src_rows_mariadb",
    });
}

#[test]
#[ignore = "需要 FLUXDB_PAIR_MYSQL 与 FLUXDB_TIDB_SMOKE"]
fn tidb_renamed_pairing_lands() {
    let Some(parts) = smoke_parts("FLUXDB_TIDB_SMOKE", 5) else {
        return;
    };
    let db = parts[4].clone();
    let target = mysql_family_config(2, DatabaseKind::TiDb, &parts, &db);
    // TiDB 不自动建库，先连 `mysql` 建目标库。
    let setup = mysql_family_config(2, DatabaseKind::TiDb, &parts, "mysql");
    execute(&setup, &format!("CREATE DATABASE IF NOT EXISTS {db}"));
    run_kind_case(KindCase {
        label: "tidb",
        target_ep: SyncEndpoint::new(ConnectionId(2), db, None),
        target,
        ddl: mysql_family_ddl,
        struct_target_key: "pair_struct_tidb".into(),
        struct_target_name: "pair_struct_tidb",
        data_target_key: "pair_rows_tidb".into(),
        data_target_name: "pair_rows_tidb",
        reverse_source_key: "pair_src_rows_tidb".into(),
        reverse_source_name: "pair_src_rows_tidb",
    });
}

#[test]
#[ignore = "需要 FLUXDB_PAIR_MYSQL 与 FLUXDB_MSSQL_SMOKE"]
fn sqlserver_renamed_pairing_lands() {
    let Some(parts) = smoke_parts("FLUXDB_MSSQL_SMOKE", 5) else {
        return;
    };
    let db = parts[4].clone();
    let target = mssql_config(2, &parts);
    run_kind_case(KindCase {
        label: "mssql",
        target_ep: SyncEndpoint::new(ConnectionId(2), db, Some("dbo".into())),
        target,
        ddl: |name| {
            format!(
                "CREATE TABLE dbo.{name} (id INT NOT NULL PRIMARY KEY, name NVARCHAR(32) NOT NULL)"
            )
        },
        struct_target_key: "dbo.pair_struct_mssql".into(),
        struct_target_name: "pair_struct_mssql",
        data_target_key: "dbo.pair_rows_mssql".into(),
        data_target_name: "pair_rows_mssql",
        reverse_source_key: "dbo.pair_src_rows_mssql".into(),
        reverse_source_name: "pair_src_rows_mssql",
    });
}

#[test]
#[ignore = "需要 FLUXDB_PAIR_MYSQL 与 FLUXDB_ORACLE_SMOKE"]
fn oracle_renamed_pairing_lands() {
    let Some(parts) = smoke_parts("FLUXDB_ORACLE_SMOKE", 5) else {
        return;
    };
    let service = parts[2].clone();
    let schema = parts[3].to_ascii_uppercase();
    let target = oracle_config(2, &parts);
    let key = |name: &str| format!("{schema}.{name}");
    run_kind_case(KindCase {
        label: "oracle",
        target_ep: SyncEndpoint::new(ConnectionId(2), service, Some(schema.clone())),
        target,
        ddl: |name| {
            format!(
                "CREATE TABLE {name} (id NUMBER(10) NOT NULL PRIMARY KEY, name VARCHAR2(32) NOT NULL)"
            )
        },
        struct_target_key: key("PAIR_STRUCT_ORACLE"),
        struct_target_name: "PAIR_STRUCT_ORACLE",
        data_target_key: key("PAIR_ROWS_ORACLE"),
        data_target_name: "PAIR_ROWS_ORACLE",
        reverse_source_key: key("PAIR_SRC_ROWS_ORACLE"),
        reverse_source_name: "PAIR_SRC_ROWS_ORACLE",
    });
}

#[test]
#[ignore = "需要 FLUXDB_PAIR_MYSQL 与 FLUXDB_DM_SMOKE"]
fn dameng_renamed_pairing_lands() {
    let Some(parts) = smoke_parts("FLUXDB_DM_SMOKE", 4) else {
        return;
    };
    let schema = parts[2].to_ascii_uppercase();
    let target = dameng_config(2, &parts);
    let key = |name: &str| format!("{schema}.{name}");
    run_kind_case(KindCase {
        label: "dm",
        target_ep: SyncEndpoint::new(ConnectionId(2), "DAMENG", Some(schema.clone())),
        target,
        ddl: |name| {
            format!("CREATE TABLE {name} (id INT NOT NULL PRIMARY KEY, name VARCHAR(32) NOT NULL)")
        },
        struct_target_key: key("PAIR_STRUCT_DM"),
        struct_target_name: "PAIR_STRUCT_DM",
        data_target_key: key("PAIR_ROWS_DM"),
        data_target_name: "PAIR_ROWS_DM",
        reverse_source_key: key("PAIR_SRC_ROWS_DM"),
        reverse_source_name: "PAIR_SRC_ROWS_DM",
    });
}

#[test]
#[ignore = "需要 FLUXDB_PAIR_MYSQL 与 FLUXDB_KB_SMOKE"]
fn kingbase_renamed_pairing_lands() {
    let Some(parts) = smoke_parts("FLUXDB_KB_SMOKE", 5) else {
        return;
    };
    let db = parts[4].clone();
    let target = kingbase_config(2, &parts);
    run_kind_case(KindCase {
        label: "kb",
        target_ep: SyncEndpoint::new(ConnectionId(2), db, Some("public".into())),
        target,
        ddl: |name| {
            format!(
                "CREATE TABLE public.{name} (id integer NOT NULL PRIMARY KEY, name varchar(32) NOT NULL)"
            )
        },
        struct_target_key: "public.pair_struct_kb".into(),
        struct_target_name: "pair_struct_kb",
        data_target_key: "public.pair_rows_kb".into(),
        data_target_name: "pair_rows_kb",
        reverse_source_key: "public.pair_src_rows_kb".into(),
        reverse_source_name: "pair_src_rows_kb",
    });
}
