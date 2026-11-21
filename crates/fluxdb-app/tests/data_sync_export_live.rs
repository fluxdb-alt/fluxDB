//! 导出迁移 SQL 真实库验证（MySQL / PostgreSQL）。
//!
//! 仅在显式提供 smoke 环境变量时运行，默认忽略：
//! - `FLUXDB_MYSQL_SMOKE=host:port:user:password`
//! - `FLUXDB_PG_SMOKE=host:port:user:password:maintenance_database`
//! - 或 `FLUXDB_SMOKE_FILE=/path/to/env`：从文件读取上面两行的 `export` 形式，
//!   避免把密码写进命令行历史。
//!
//! 覆盖：覆盖模式与增量模式执行后，把导出的脚本在第三个库上重放，
//! 断言重放结果与同步后的目标库逐行一致（含引号/反斜杠转义、NULL）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use fluxdb_app::{AppCommand, AppController, DataSyncOptions, DataSyncRequest, SyncEndpoint};
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    CellValue, ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlBasicOptions,
    MysqlConnectionProfile, PostgresBasicOptions, PostgresConnectionProfile, QueryExecutionOptions,
    QueryMode, QueryRequest, SecretRef,
};

const ITEMS_SQL: &str =
    "CREATE TABLE items (id {$int} NOT NULL PRIMARY KEY, name {$text} NULL, amount {$int} NULL)";

fn smoke_parts(key: &str, expected: usize) -> Option<Vec<String>> {
    let raw = std::env::var(key).ok().or_else(|| smoke_file_value(key))?;
    let parts = raw
        .splitn(expected, ':')
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(parts.len(), expected, "{key} 参数个数不对");
    Some(parts)
}

/// 从 `FLUXDB_SMOKE_FILE` 指向的 `export KEY='value'` 文件读取配置。
fn smoke_file_value(key: &str) -> Option<String> {
    let path = std::env::var("FLUXDB_SMOKE_FILE").ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        line.trim()
            .strip_prefix(&format!("export {key}="))
            .map(|value| value.trim().trim_matches('\'').to_string())
    })
}

fn mysql_config(id: u64, database: &str) -> ConnectionConfig {
    let parts = smoke_parts("FLUXDB_MYSQL_SMOKE", 4).expect("FLUXDB_MYSQL_SMOKE");
    let port = parts[1].parse().expect("MySQL 端口");
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("MySQL {database}"),
        kind: DatabaseKind::MySql,
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
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }
    .mysql_resolved()
}

fn pg_config(id: u64, database: &str) -> ConnectionConfig {
    let parts = smoke_parts("FLUXDB_PG_SMOKE", 5).expect("FLUXDB_PG_SMOKE");
    let port = parts[1].parse().expect("PostgreSQL 端口");
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("PostgreSQL {database}"),
        kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp {
            host: parts[0].clone(),
            port,
            database: Some(database.into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: parts[0].clone(),
                port,
                maintenance_database: parts[4].clone(),
                username: parts[2].clone(),
                password: SecretRef::inline(parts[3].clone()),
            },
            ..PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        mongo_profile: None,
        sqlserver_profile: None,
        oracle_profile: None,
    }
}

/// PostgreSQL 以 `request.database` 选择物理库（缺省回落到维护库），
/// 因此执行辅助函数必须显式带上配置里的目标库。
fn request_database(config: &ConnectionConfig) -> Option<String> {
    match &config.endpoint {
        Endpoint::Tcp { database, .. } => database.clone().filter(|database| !database.is_empty()),
        _ => None,
    }
}

fn execute(config: &ConnectionConfig, sql: &str) {
    let result = connector_for(config)
        .expect("connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: request_database(config),
            schema: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions {
                page_size: 1000,
                split_statements: true,
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

/// 读出整表，按主键排序；重放结果与目标库逐行比对用。
fn rows(config: &ConnectionConfig) -> Vec<Vec<CellValue>> {
    let result = connector_for(config)
        .expect("connector")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: request_database(config),
            schema: None,
            text: "SELECT id, name, amount FROM items ORDER BY id".into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        })
        .expect("select");
    result
        .results
        .into_iter()
        .flat_map(|page| page.rows)
        .map(|row| row.values)
        .collect()
}

fn script_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("fluxdb-export-{tag}-{}.sql", std::process::id()))
}

/// 在维护库上重建三个库；脚本重放依赖库名隔离。
fn reset_databases(
    admin: &ConnectionConfig,
    databases: &[String],
    create_sql: impl Fn(&str) -> String,
) {
    for database in databases {
        execute(admin, &format!("DROP DATABASE IF EXISTS {database}"));
        execute(admin, &create_sql(database));
    }
}

fn prepare_items(config: &ConnectionConfig, int_type: &str, text_type: &str, seed: &str) {
    let ddl = ITEMS_SQL
        .replace("{$int}", int_type)
        .replace("{$text}", text_type);
    execute(config, "DROP TABLE IF EXISTS items");
    execute(config, &ddl);
    if !seed.is_empty() {
        execute(config, seed);
    }
}

fn run_data_sync(
    controller: &mut AppController,
    source: &ConnectionConfig,
    source_db: &str,
    target: &ConnectionConfig,
    target_db: &str,
    table_key: &str,
    options: DataSyncOptions,
) {
    let request = DataSyncRequest {
        source: SyncEndpoint::new(source.id, source_db, None),
        target: SyncEndpoint::new(target.id, target_db, None),
        tables: vec![table_key.into()],
        options,
    };
    let cancel = AtomicBool::new(false);
    controller
        .execute_data_sync(&request, &cancel, &mut |_| {})
        .expect("数据同步");
}

fn replay(target: &ConnectionConfig, path: &PathBuf) {
    let script = std::fs::read_to_string(path).expect("脚本");
    assert!(script.contains("INSERT INTO"), "{script}");
    execute(target, &script);
}

/// 覆盖模式：清空 + 全量 INSERT 脚本重放后与目标库一致。
#[test]
#[ignore = "需要 FLUXDB_MYSQL_SMOKE 指向可达的 MySQL 8"]
fn mysql_export_overwrite_script_replays() {
    if smoke_parts("FLUXDB_MYSQL_SMOKE", 4).is_none() {
        return;
    }
    let suffix = std::process::id();
    let names = [
        format!("fluxdb_exp_src_{suffix}"),
        format!("fluxdb_exp_dst_{suffix}"),
        format!("fluxdb_exp_replay_{suffix}"),
    ];
    let admin = mysql_config(9_970, "");
    reset_databases(&admin, &names, |name| format!("CREATE DATABASE {name}"));
    let source = mysql_config(9_971, &names[0]);
    let target = mysql_config(9_972, &names[1]);
    let replay_db = mysql_config(9_973, &names[2]);

    prepare_items(
        &source,
        "BIGINT",
        "TEXT",
        "INSERT INTO items (id, name, amount) VALUES (1, 'plain', 10), (2, 'it''s', NULL), (3, 'a\\\\b', 30)",
    );
    prepare_items(
        &target,
        "BIGINT",
        "TEXT",
        "INSERT INTO items (id, name, amount) VALUES (9, 'stale', 90)",
    );
    prepare_items(&replay_db, "BIGINT", "TEXT", "");

    let path = script_path("mysql-overwrite");
    let mut controller = AppController::new();
    let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![
        source.clone(),
        target.clone(),
        replay_db.clone(),
    ]));
    run_data_sync(
        &mut controller,
        &source,
        &names[0],
        &target,
        &names[1],
        "items",
        DataSyncOptions {
            overwrite: true,
            export_sql_path: Some(path.clone()),
            ..DataSyncOptions::default()
        },
    );

    replay(&replay_db, &path);
    assert_eq!(rows(&target), rows(&replay_db), "重放结果必须与目标库一致");
    assert_eq!(rows(&target).len(), 3);
    assert!(
        !std::fs::read_to_string(&path)
            .expect("脚本")
            .contains("stale")
    );

    cleanup(&admin, &names, &path);
}

/// 增量模式：INSERT / UPDATE / DELETE 脚本重放后与目标库一致。
#[test]
#[ignore = "需要 FLUXDB_PG_SMOKE 指向可达的 PostgreSQL 16"]
fn postgres_export_incremental_script_replays() {
    if smoke_parts("FLUXDB_PG_SMOKE", 5).is_none() {
        return;
    }
    let suffix = std::process::id();
    let names = [
        format!("fluxdb_exp_src_{suffix}"),
        format!("fluxdb_exp_dst_{suffix}"),
        format!("fluxdb_exp_replay_{suffix}"),
    ];
    // 维护连接：不能连在待删除的库上，否则 PG 拒绝 DROP DATABASE。
    let maintenance = smoke_parts("FLUXDB_PG_SMOKE", 5).expect("FLUXDB_PG_SMOKE")[4].clone();
    let admin = pg_config(9_974, &maintenance);
    reset_databases(&admin, &names, |name| format!("CREATE DATABASE {name}"));
    let source = pg_config(9_975, &names[0]);
    let target = pg_config(9_976, &names[1]);
    let replay_db = pg_config(9_977, &names[2]);

    prepare_items(
        &source,
        "bigint",
        "text",
        "INSERT INTO items (id, name, amount) VALUES (1, 'new', 10), (2, 'same', 20), (3, 'changed', 30)",
    );
    let target_seed = "INSERT INTO items (id, name, amount) VALUES (2, 'same', 20), (3, 'old', 30), (4, 'gone', 40)";
    prepare_items(&target, "bigint", "text", target_seed);
    prepare_items(&replay_db, "bigint", "text", target_seed);

    let path = script_path("pg-incremental");
    let mut controller = AppController::new();
    let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![
        source.clone(),
        target.clone(),
        replay_db.clone(),
    ]));
    run_data_sync(
        &mut controller,
        &source,
        &names[0],
        &target,
        &names[1],
        "public.items",
        DataSyncOptions {
            insert: true,
            update: true,
            delete: true,
            export_sql_path: Some(path.clone()),
            ..DataSyncOptions::default()
        },
    );

    replay(&replay_db, &path);
    assert_eq!(rows(&target), rows(&replay_db), "重放结果必须与目标库一致");
    let script = std::fs::read_to_string(&path).expect("脚本");
    assert!(script.contains("UPDATE "), "{script}");
    assert!(script.contains("DELETE FROM "), "{script}");

    cleanup(&admin, &names, &path);
}

/// 只清理本测试创建的库与脚本，不碰其它库。
///
/// 必须用不指定默认库的 admin 连接执行：用连在待删库上的连接去删库，
/// 删掉当前默认库后后续连接会报 `1049 Unknown database`。
fn cleanup(admin: &ConnectionConfig, names: &[String], path: &PathBuf) {
    let _ = std::fs::remove_file(path);
    for database in names {
        execute(admin, &format!("DROP DATABASE IF EXISTS {database}"));
    }
}
