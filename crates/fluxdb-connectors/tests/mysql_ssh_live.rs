//! MySQL SSH 隧道真实链路集成测试。
//!
//! 由 `scripts/ssh-tunnel-e2e.sh mysql` 启动 Podman：MySQL 与 sshd 在同一 pod，
//! 只发布 sshd 端口，MySQL 在宿主机不可达。用例成功即证明对象浏览、SQL 执行
//! 与查询超时都真的经 SSH 转发，而不是只有 `test_connection` 走了隧道。
//!
//! 未设置 `FLUXDB_MYSQL_SSH` / `FLUXDB_MYSQL_TARGET` 时跳过；
//! 凭据只从环境变量读入内存，不写文件、不打印日志。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlAdvancedOptions,
    MysqlBasicOptions, MysqlConnectionProfile, MysqlSshAuth, MysqlSshOptions, MysqlTransportLayer,
    QueryExecutionOptions, QueryMode, QueryRequest, SecretRef,
};
use std::collections::BTreeMap;
use std::time::Instant;

/// `host:port:user:password`
fn split_endpoint(raw: &str) -> Option<(String, u16, String, String)> {
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    Some((
        parts[0].into(),
        parts[1].parse().ok()?,
        parts[2].into(),
        parts[3].into(),
    ))
}

struct Settings {
    jump: (String, u16, String, String),
    target: (String, u16, String, String),
}

fn settings() -> Option<Settings> {
    Some(Settings {
        jump: split_endpoint(&std::env::var("FLUXDB_MYSQL_SSH").ok()?)?,
        target: split_endpoint(&std::env::var("FLUXDB_MYSQL_TARGET").ok()?)?,
    })
}

/// 构造连接配置；`ssh` 为空表示直连（用于对照「不走隧道就失败」）。
fn config(settings: &Settings, ssh: bool) -> ConnectionConfig {
    let (host, port, username, password) = settings.target.clone();
    let transport = if ssh {
        vec![MysqlTransportLayer::Ssh(MysqlSshOptions {
            enabled: true,
            host: settings.jump.0.clone(),
            port: settings.jump.1,
            username: settings.jump.2.clone(),
            auth: MysqlSshAuth::Password,
            password: SecretRef::inline(settings.jump.3.clone()),
            private_key: SecretRef::default(),
            passphrase: SecretRef::default(),
            connect_timeout_secs: 5,
            keepalive_interval_secs: 0,
            host_key_policy: fluxdb_core::SshHostKeyPolicy::Strict,
        })]
    } else {
        Vec::new()
    };
    ConnectionConfig {
        id: ConnectionId(9_921),
        name: "MySQL SSH live".into(),
        kind: DatabaseKind::MySql,
        endpoint: Endpoint::Tcp {
            host: host.clone(),
            port,
            database: None,
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host,
                port,
                database: "mysql".into(),
                username,
                password: SecretRef::inline(password),
                ..fluxdb_core::MysqlBasicOptions::default()
            },
            transport,
            advanced: MysqlAdvancedOptions {
                connect_timeout_secs: 5,
                ..MysqlAdvancedOptions::default()
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

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SSH=host:port:user:password 与 FLUXDB_MYSQL_TARGET=host:port:user:password"]
fn mysql_ssh_tunnel_covers_objects_and_queries() {
    let settings = settings().expect("需要 FLUXDB_MYSQL_SSH 与 FLUXDB_MYSQL_TARGET");
    let connector_config = config(&settings, true);
    let connector = connector_for(&connector_config).expect("创建 MySQL 连接器");

    // 1. 直连应失败：目标库只在 pod 内可达，证明后续成功确实来自 SSH 转发。
    let direct = config(&settings, false);
    assert!(
        connector_for(&direct)
            .expect("创建直连连接器")
            .test_connection(&direct)
            .is_err(),
        "目标 MySQL 不应在宿主机直连可达，否则本用例无法证明走了 SSH"
    );

    // 2. 测试连接。
    connector
        .test_connection(&connector_config)
        .expect("SSH 隧道下测试连接应成功");

    // 3. 对象浏览走隧道（修复前此路径直连目标主机，必然失败）。
    let databases = connector.list_objects(None).expect("SSH 隧道下列库应成功");
    assert!(
        databases.iter().any(|object| object.path.name == "mysql"),
        "列库结果应包含系统库 mysql"
    );

    // 4. SQL 执行走隧道（修复前此路径直连目标主机，必然失败）。
    let result = connector
        .execute(&request(&connector_config, "SELECT DATABASE() AS db"))
        .expect("SSH 隧道下查询应成功");
    let value = result
        .results
        .first()
        .and_then(|page| page.rows.first())
        .and_then(|row| row.values.first())
        .map(fluxdb_core::CellValue::display_label)
        .unwrap_or_default();
    assert_eq!(value, "mysql");
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_SSH=host:port:user:password 与 FLUXDB_MYSQL_TARGET=host:port:user:password"]
fn mysql_ssh_tunnel_applies_query_timeout() {
    let settings = settings().expect("需要 FLUXDB_MYSQL_SSH 与 FLUXDB_MYSQL_TARGET");
    let mut connector_config = config(&settings, true);
    if let Some(profile) = connector_config.mysql_profile.as_mut() {
        profile.advanced.query_timeout_secs = 1;
    }
    let connector_config = connector_config.mysql_resolved();
    let connector = connector_for(&connector_config).expect("创建 MySQL 连接器");

    let started = Instant::now();
    // 语句级失败收敛在执行摘要里，连接本身仍成功：断言摘要而不是 `Err`。
    let result = connector
        .execute(&request(&connector_config, "SELECT SLEEP(10)"))
        .expect("连接与执行入口应正常返回");
    let elapsed = started.elapsed();
    let summary = result.summaries.first().expect("应有一条执行摘要");
    assert!(!summary.success, "被超时打断的语句不应标记成功");
    assert!(
        summary.message.contains("查询超时"),
        "摘要应说明查询超时，实际: {}",
        summary.message
    );
    assert!(result.results.is_empty(), "超时不应返回被打断的结果集");
    assert!(
        elapsed.as_secs() < 6,
        "超时应尽快返回，实际耗时 {elapsed:?}"
    );
}

#[test]
#[ignore = "使用 TEST-NET-1 黑洞地址验证建连超时，需要当前网络不直连该网段"]
fn mysql_connect_timeout_is_honored() {
    let config = ConnectionConfig {
        id: ConnectionId(9_923),
        name: "MySQL connect timeout".into(),
        kind: DatabaseKind::MySql,
        endpoint: Endpoint::Tcp {
            host: "192.0.2.1".into(),
            port: 3306,
            database: None,
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: Some(MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: "192.0.2.1".into(),
                port: 3306,
                database: String::new(),
                username: "root".into(),
                password: SecretRef::inline("nope"),
                ..fluxdb_core::MysqlBasicOptions::default()
            },
            advanced: MysqlAdvancedOptions {
                connect_timeout_secs: 1,
                ..MysqlAdvancedOptions::default()
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

    let started = Instant::now();
    let error = connector_for(&config)
        .expect("创建 MySQL 连接器")
        .test_connection(&config)
        .expect_err("黑洞地址应在建连超时后失败");
    let elapsed = started.elapsed();
    assert!(
        error.to_string().contains("连接超时") || error.to_string().contains("超时"),
        "错误应说明超时，实际: {error}"
    );
    assert!(
        elapsed.as_secs() < 6,
        "档案 1 秒建连超时应生效，实际耗时 {elapsed:?}"
    );
}
