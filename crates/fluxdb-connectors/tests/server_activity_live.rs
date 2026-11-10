//! 服务器面板真实数据库集成测试。
//!
//! 每个用例只在提供对应环境变量时运行；默认忽略，不依赖本机数据库：
//! - `FLUXDB_MYSQL_ACTIVITY=host:port:user:password:database`
//! - `FLUXDB_PG_ACTIVITY=host:port:user:password:database`
//! - `FLUXDB_MSSQL_ACTIVITY=host:port:user:password:database`
//! - `FLUXDB_ORACLE_ACTIVITY=host:port:service:user:password`
//! - `FLUXDB_DM_ACTIVITY=host:port:user:password:database`
//!
//! 终止用例会自己起一条慢查询会话，再通过服务器面板接口终止；不会碰其它会话。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DamengAdvancedOptions, DamengBasicOptions,
    DamengConnectionProfile, DatabaseKind, Endpoint, MysqlBasicOptions, MysqlConnectionProfile,
    OracleAdvancedOptions, OracleBasicOptions, OracleConnectKind, OracleConnectionProfile,
    OraclePrivilege, PostgresBasicOptions, PostgresConnectionProfile, QueryExecutionOptions,
    QueryMode, QueryRequest, SecretRef, ServerActivity, SqlServerBasicOptions,
    SqlServerConnectionProfile,
};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn parts(env: &str) -> Option<Vec<String>> {
    std::env::var(env)
        .ok()
        .map(|raw| raw.splitn(5, ':').map(str::to_string).collect())
}

fn mysql_config() -> ConnectionConfig {
    let p = parts("FLUXDB_MYSQL_ACTIVITY")
        .expect("FLUXDB_MYSQL_ACTIVITY=host:port:user:password:database");
    let port = p[1].parse().expect("MySQL port");
    ConnectionConfig {
        id: ConnectionId(9_961),
        name: "MySQL activity".into(),
        kind: DatabaseKind::MySql,
        endpoint: Endpoint::Tcp {
            host: p[0].clone(),
            port,
            database: Some(p[4].clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: p[0].clone(),
                port,
                database: p[4].clone(),
                username: p[2].clone(),
                password: SecretRef::inline(p[3].clone()),
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

fn pg_config() -> ConnectionConfig {
    let p =
        parts("FLUXDB_PG_ACTIVITY").expect("FLUXDB_PG_ACTIVITY=host:port:user:password:database");
    let port = p[1].parse().expect("PostgreSQL port");
    ConnectionConfig {
        id: ConnectionId(9_962),
        name: "PG activity".into(),
        kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp {
            host: p[0].clone(),
            port,
            database: Some(p[4].clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: p[0].clone(),
                port,
                maintenance_database: p[4].clone(),
                username: p[2].clone(),
                password: SecretRef::inline(p[3].clone()),
            },
            ..PostgresConnectionProfile::default()
        }),
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
    .postgres_resolved()
}

fn mssql_config() -> ConnectionConfig {
    let p = parts("FLUXDB_MSSQL_ACTIVITY")
        .expect("FLUXDB_MSSQL_ACTIVITY=host:port:user:password:database");
    let port = p[1].parse().expect("SQL Server port");
    ConnectionConfig {
        id: ConnectionId(9_963),
        name: "SQL Server activity".into(),
        kind: DatabaseKind::SqlServer,
        endpoint: Endpoint::Tcp {
            host: p[0].clone(),
            port,
            database: Some(p[4].clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: Some(SqlServerConnectionProfile {
            basic: SqlServerBasicOptions {
                host: p[0].clone(),
                port,
                database: p[4].clone(),
                schema: String::new(),
                instance: String::new(),
                username: p[2].clone(),
                password: SecretRef::inline(p[3].clone()),
            },
            ..SqlServerConnectionProfile::default()
        }),
        mongo_profile: None,
        oracle_profile: None,
    }
}

fn oracle_config() -> ConnectionConfig {
    let p = parts("FLUXDB_ORACLE_ACTIVITY")
        .expect("FLUXDB_ORACLE_ACTIVITY=host:port:service:user:password");
    let port = p[1].parse().expect("Oracle port");
    ConnectionConfig {
        id: ConnectionId(9_964),
        name: "Oracle activity".into(),
        kind: DatabaseKind::Oracle,
        endpoint: Endpoint::Tcp {
            host: p[0].clone(),
            port,
            database: Some(p[2].clone()),
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
                host: p[0].clone(),
                port,
                connect_kind: OracleConnectKind::ServiceName,
                service_name: p[2].clone(),
                sid: String::new(),
                username: p[3].to_ascii_uppercase(),
                schema: p[3].to_ascii_uppercase(),
                privilege: OraclePrivilege::Normal,
                password: SecretRef::inline(p[4].clone()),
            },
            advanced: OracleAdvancedOptions {
                connect_timeout_secs: 8,
                query_timeout_secs: 0,
                application_name: "FluxDBActivity".into(),
                wallet_location: String::new(),
                wallet_password: SecretRef::default(),
            },
            ssh: Default::default(),
        }),
    }
}

fn dm_config() -> ConnectionConfig {
    let p =
        parts("FLUXDB_DM_ACTIVITY").expect("FLUXDB_DM_ACTIVITY=host:port:user:password:database");
    let port = p[1].parse().expect("DM port");
    ConnectionConfig {
        id: ConnectionId(9_965),
        name: "DM activity".into(),
        kind: DatabaseKind::Dameng,
        endpoint: Endpoint::Tcp {
            host: p[0].clone(),
            port,
            database: Some(p[4].clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(DamengConnectionProfile {
            basic: DamengBasicOptions {
                host: p[0].clone(),
                port,
                database: p[4].clone(),
                schema: p[2].to_ascii_uppercase(),
                username: p[2].clone(),
                password: SecretRef::inline(p[3].clone()),
            },
            advanced: DamengAdvancedOptions {
                connect_timeout_secs: 5,
                query_timeout_secs: 0,
                encoding: String::new(),
                application_name: "FluxDBActivity".into(),
            },
            ssh: Default::default(),
        }),
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
}

fn request(config: &ConnectionConfig, sql: &str) -> QueryRequest {
    QueryRequest {
        connection_id: config.id,
        database: None,
        schema: None,
        text: sql.into(),
        mode: QueryMode::All,
        options: QueryExecutionOptions::default(),
        session_id: None,
    }
}

fn assert_activity(activity: &ServerActivity) {
    assert!(!activity.current_user.is_empty(), "当前用户为空");
    assert!(!activity.sessions.is_empty(), "没有读到任何会话");
    assert!(
        activity
            .sessions
            .iter()
            .all(|session| !session.id.is_empty())
    );
    assert!(!activity.status.metrics.is_empty(), "没有读到状态指标");
    assert!(
        activity.locks_error.is_none(),
        "锁读取失败: {:?}",
        activity.locks_error
    );
    assert!(
        activity.status_error.is_none(),
        "状态读取失败: {:?}",
        activity.status_error
    );
}

/// 起一条慢查询，等它出现在会话列表里，返回会话 id。
fn find_slow_session(
    config: &ConnectionConfig,
    marker: &str,
) -> (std::thread::JoinHandle<fluxdb_core::Result<()>>, String) {
    let worker = config.clone();
    let marker = marker.to_string();
    let worker_marker = marker.clone();
    let handle = std::thread::spawn(move || {
        let connector = connector_for(&worker)?;
        connector
            .execute(&request(&worker, &worker_marker))
            .map(|_| ())
    });
    let connector = connector_for(config).expect("create connector");
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let activity = connector.server_activity(config.id).expect("read activity");
        if let Some(session) = activity.sessions.iter().find(|session| {
            session
                .sql
                .to_ascii_lowercase()
                .contains(&marker.to_ascii_lowercase())
        }) {
            return (handle, session.id.clone());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    panic!("慢查询没有出现在会话列表: {marker}");
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_ACTIVITY=host:port:user:password:database"]
fn mysql_server_activity_reports_sessions_and_kills_session() {
    let config = mysql_config();
    let connector = connector_for(&config).expect("create MySQL connector");
    assert_activity(
        &connector
            .server_activity(config.id)
            .expect("server activity"),
    );

    let (handle, session_id) = find_slow_session(&config, "SELECT SLEEP(20)");
    connector
        .kill_server_session(config.id, &session_id)
        .expect("kill session");
    let _ = handle.join().expect("slow query thread");
}

#[test]
#[ignore = "需要 FLUXDB_PG_ACTIVITY=host:port:user:password:database"]
fn postgres_server_activity_reports_sessions_and_cancels_query() {
    let config = pg_config();
    let connector = connector_for(&config).expect("create PostgreSQL connector");
    assert_activity(
        &connector
            .server_activity(config.id)
            .expect("server activity"),
    );

    let (handle, session_id) = find_slow_session(&config, "SELECT pg_sleep(20)");
    connector
        .cancel_server_query(config.id, &session_id)
        .expect("cancel query");
    let _ = handle.join().expect("slow query thread");
}

#[test]
#[ignore = "需要 FLUXDB_MSSQL_ACTIVITY=host:port:user:password:database"]
fn sqlserver_server_activity_reports_sessions_and_kills_session() {
    let config = mssql_config();
    let connector = connector_for(&config).expect("create SQL Server connector");
    assert_activity(
        &connector
            .server_activity(config.id)
            .expect("server activity"),
    );

    let (handle, session_id) = find_slow_session(&config, "WAITFOR DELAY '00:00:20'");
    connector
        .kill_server_session(config.id, &session_id)
        .expect("kill session");
    let _ = handle.join().expect("slow query thread");
}

#[test]
#[ignore = "需要 FLUXDB_ORACLE_ACTIVITY=host:port:service:user:password"]
fn oracle_server_activity_reports_sessions_and_kills_session() {
    let config = oracle_config();
    let connector = connector_for(&config).expect("create Oracle connector");
    assert_activity(
        &connector
            .server_activity(config.id)
            .expect("server activity"),
    );

    let (handle, session_id) = find_slow_session(
        &config,
        "SELECT COUNT(*) FROM all_objects a, all_objects b, all_objects c",
    );
    connector
        .kill_server_session(config.id, &session_id)
        .expect("kill session");
    let _ = handle.join().expect("slow query thread");
}

#[test]
#[ignore = "需要 FLUXDB_DM_ACTIVITY=host:port:user:password:database"]
fn dameng_server_activity_reports_sessions_and_kills_session() {
    let config = dm_config();
    let connector = connector_for(&config).expect("create DM connector");
    assert_activity(
        &connector
            .server_activity(config.id)
            .expect("server activity"),
    );

    let (handle, session_id) = find_slow_session(&config, "DBMS_LOCK.SLEEP(20)");
    connector
        .kill_server_session(config.id, &session_id)
        .expect("kill session");
    let _ = handle.join().expect("slow query thread");
}
