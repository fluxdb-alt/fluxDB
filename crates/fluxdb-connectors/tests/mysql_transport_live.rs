//! MySQL 传输/选项真实链路集成测试：SOCKS5、HTTP CONNECT、字符集、TCP 保活。
//!
//! 由 `scripts/ssh-tunnel-e2e.sh mysql` 启动 Podman MySQL 与本地代理进程后运行。
//! 代理用例除了查询成功，脚本还会核对代理进程日志里的转发记录，证明流量确实经过
//! 代理；代理不可达/凭据错误用例则反向证明不会静默退回直连。
//!
//! 未设置对应环境变量时跳过；凭据只从环境变量读入内存，不写文件、不打印日志。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    CellValue, ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlAdvancedOptions,
    MysqlBasicOptions, MysqlConnectionProfile, MysqlProxy, MysqlProxyType, MysqlTransportLayer,
    QueryExecutionOptions, QueryMode, QueryRequest, SecretRef,
};
use std::collections::BTreeMap;

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

fn target() -> Option<(String, u16, String, String)> {
    split_endpoint(&std::env::var("FLUXDB_MYSQL_TARGET").ok()?)
}

/// 代理地址 `host:port:user:password`；密码可为空（此时按 `host:port:user` 解析）。
fn proxy_endpoint(raw: &str) -> Option<(String, u16, String, String)> {
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    Some((
        parts[0].into(),
        parts[1].parse().ok()?,
        parts.get(2).copied().unwrap_or_default().into(),
        parts.get(3).copied().unwrap_or_default().into(),
    ))
}

fn config(
    target: &(String, u16, String, String),
    proxy: Option<(MysqlProxyType, (String, u16, String, String))>,
    charset: &str,
    tcp_keepalive: bool,
) -> ConnectionConfig {
    let (host, port, username, password) = target.clone();
    let transport = proxy
        .map(
            |(proxy_type, (proxy_host, proxy_port, proxy_username, proxy_password))| {
                vec![MysqlTransportLayer::Proxy(MysqlProxy {
                    enabled: true,
                    proxy_type,
                    host: proxy_host,
                    port: proxy_port,
                    username: proxy_username,
                    password: SecretRef::inline(proxy_password),
                })]
            },
        )
        .unwrap_or_default();
    ConnectionConfig {
        id: ConnectionId(9_931),
        name: "MySQL transport live".into(),
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
            tls: fluxdb_core::MysqlTlsOptions {
                charset: charset.into(),
                ..fluxdb_core::MysqlTlsOptions::default()
            },
            transport,
            advanced: MysqlAdvancedOptions {
                connect_timeout_secs: 5,
                tcp_keepalive,
                ..MysqlAdvancedOptions::default()
            },
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

/// 执行一条只返回单值的查询。
fn scalar(config: &ConnectionConfig, sql: &str) -> String {
    let connector = connector_for(config).expect("创建 MySQL 连接器");
    let result = connector
        .execute(&request(config, sql))
        .expect("查询应成功");
    result
        .results
        .first()
        .and_then(|page| page.rows.first())
        .and_then(|row| row.values.first())
        .map(CellValue::display_label)
        .unwrap_or_default()
}

/// 代理下的完整链路：测试连接、列库、执行查询。
fn assert_proxy_path(config: &ConnectionConfig) {
    let connector = connector_for(config).expect("创建 MySQL 连接器");
    connector
        .test_connection(config)
        .expect("经代理测试连接应成功");
    let databases = connector.list_objects(None).expect("经代理列库应成功");
    assert!(
        databases.iter().any(|object| object.path.name == "mysql"),
        "列库结果应包含系统库 mysql"
    );
    assert_eq!(scalar(config, "SELECT DATABASE() AS db"), "mysql");
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TARGET=host:port:user:password 与 FLUXDB_MYSQL_PROXY_SOCKS=host:port:user:password"]
fn mysql_socks5_proxy_covers_objects_and_queries() {
    let target = target().expect("需要 FLUXDB_MYSQL_TARGET");
    let proxy = proxy_endpoint(
        &std::env::var("FLUXDB_MYSQL_PROXY_SOCKS").expect("需要 FLUXDB_MYSQL_PROXY_SOCKS"),
    )
    .expect("FLUXDB_MYSQL_PROXY_SOCKS 格式应为 host:port[:user:password]");
    assert_proxy_path(&config(
        &target,
        Some((MysqlProxyType::Socks5, proxy)),
        "utf8mb4",
        false,
    ));
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TARGET=host:port:user:password 与 FLUXDB_MYSQL_PROXY_HTTP=host:port:user:password"]
fn mysql_http_connect_proxy_covers_objects_and_queries() {
    let target = target().expect("需要 FLUXDB_MYSQL_TARGET");
    let proxy = proxy_endpoint(
        &std::env::var("FLUXDB_MYSQL_PROXY_HTTP").expect("需要 FLUXDB_MYSQL_PROXY_HTTP"),
    )
    .expect("FLUXDB_MYSQL_PROXY_HTTP 格式应为 host:port[:user:password]");
    assert_proxy_path(&config(
        &target,
        Some((MysqlProxyType::HttpConnect, proxy)),
        "utf8mb4",
        false,
    ));
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TARGET 与 FLUXDB_MYSQL_PROXY_SOCKS"]
fn mysql_proxy_wrong_password_fails() {
    let target = target().expect("需要 FLUXDB_MYSQL_TARGET");
    let (host, port, username, _) = proxy_endpoint(
        &std::env::var("FLUXDB_MYSQL_PROXY_SOCKS").expect("需要 FLUXDB_MYSQL_PROXY_SOCKS"),
    )
    .expect("代理地址格式错误");
    let config = config(
        &target,
        Some((
            MysqlProxyType::Socks5,
            (host, port, username, "definitely-wrong".into()),
        )),
        "utf8mb4",
        false,
    );
    let error = connector_for(&config)
        .expect("创建 MySQL 连接器")
        .test_connection(&config)
        .expect_err("代理凭据错误必须失败");
    assert!(
        error.to_string().contains("代理"),
        "错误应指向代理，实际: {error}"
    );
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TARGET 与 FLUXDB_MYSQL_PROXY_DEAD=host:port"]
fn mysql_proxy_unreachable_does_not_fall_back_to_direct() {
    let target = target().expect("需要 FLUXDB_MYSQL_TARGET");
    let (host, port, _, _) = proxy_endpoint(
        &std::env::var("FLUXDB_MYSQL_PROXY_DEAD").expect("需要 FLUXDB_MYSQL_PROXY_DEAD"),
    )
    .expect("代理地址格式错误");
    // 目标本身直连可达：若实现偷偷直连，本用例会错误地成功。
    let config = config(
        &target,
        Some((
            MysqlProxyType::Socks5,
            (host, port, String::new(), String::new()),
        )),
        "utf8mb4",
        false,
    );
    let error = connector_for(&config)
        .expect("创建 MySQL 连接器")
        .test_connection(&config)
        .expect_err("代理不可达时不应静默直连");
    assert!(
        error.to_string().contains("代理"),
        "错误应指向代理，实际: {error}"
    );
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TARGET=host:port:user:password"]
fn mysql_charset_and_keepalive_are_applied() {
    let target = target().expect("需要 FLUXDB_MYSQL_TARGET");
    // 字符集交给驱动：会话变量应反映 latin1，而不是服务端默认 utf8mb4。
    let latin1 = config(&target, None, "latin1", false);
    assert_eq!(scalar(&latin1, "SELECT @@character_set_client"), "latin1");
    assert_eq!(
        scalar(&latin1, "SELECT @@character_set_connection"),
        "latin1"
    );

    // TCP 保活走本地转发：连接与查询必须仍可用。
    let keepalive = config(&target, None, "utf8mb4", true);
    assert_eq!(scalar(&keepalive, "SELECT 1"), "1");
    let connector = connector_for(&keepalive).expect("创建 MySQL 连接器");
    connector
        .test_connection(&keepalive)
        .expect("开启 TCP 保活后测试连接应成功");
}
