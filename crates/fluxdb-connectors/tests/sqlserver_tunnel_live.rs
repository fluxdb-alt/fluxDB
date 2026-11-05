//! SQL Server 经 SSH 隧道的真实连接测试。
//!
//! 仅在显式提供 `FLUXDB_MSSQL_SMOKE=host:port:user:password:database` 时运行；
//! 另用 `FLUXDB_MSSQL_SSH=host:port:user:password` 启用跳板机。目标主机只在
//! 容器网络内可达，因此去掉隧道后必须连不上，可证明请求确实走了 SSH 转发。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, QueryExecutionOptions, QueryMode,
    QueryRequest, SecretRef, SqlServerBasicOptions, SqlServerConnectionProfile, SshAuth,
    SshTunnelProfile,
};
use std::collections::BTreeMap;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_MSSQL_SMOKE").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    assert_eq!(
        parts.len(),
        5,
        "FLUXDB_MSSQL_SMOKE=host:port:user:password:database"
    );
    let host = parts[0].to_string();
    let port: u16 = parts[1].parse().expect("SQL Server port");
    let database = parts[4].to_string();
    Some(ConnectionConfig {
        id: ConnectionId(9_914),
        name: "SQL Server tunnel".into(),
        kind: DatabaseKind::SqlServer,
        endpoint: Endpoint::Tcp {
            host: host.clone(),
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
                host,
                port,
                database,
                schema: String::new(),
                instance: String::new(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            ssh: ssh_profile(),
            ..SqlServerConnectionProfile::default()
        }),
        mongo_profile: None,
        oracle_profile: None,
    })
}

/// `FLUXDB_MSSQL_SSH=host:port:user:password`；未设置时返回禁用档案。
fn ssh_profile() -> SshTunnelProfile {
    let Ok(raw) = std::env::var("FLUXDB_MSSQL_SSH") else {
        return SshTunnelProfile::default();
    };
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 4, "FLUXDB_MSSQL_SSH=host:port:user:password");
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

#[test]
#[ignore = "需要 FLUXDB_MSSQL_SMOKE=host:port:user:password:database"]
fn sqlserver_connects_through_ssh_tunnel_and_direct_dial_fails() {
    let config = smoke_config().expect("FLUXDB_MSSQL_SMOKE is required");
    let connector = connector_for(&config).expect("create SQL Server connector");
    connector
        .test_connection(&config)
        .expect("test SQL Server connection over tunnel");

    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            session_id: None,
            text: "SELECT 1 AS OK".into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .expect("query over tunnel");
    assert!(
        result.summaries.iter().all(|summary| summary.success),
        "query failed: {:?}",
        result.summaries
    );

    if ssh_profile().enabled {
        // 目标主机只有跳板机可见；关闭隧道后直连必须失败，排除静默回退。
        let mut direct_config = config.clone();
        direct_config
            .sqlserver_profile
            .as_mut()
            .expect("profile")
            .ssh = SshTunnelProfile::default();
        let direct_connector = connector_for(&direct_config).expect("create direct connector");
        assert!(
            direct_connector.test_connection(&direct_config).is_err(),
            "关闭 SSH 隧道后不应仍能连上容器内 SQL Server"
        );
    }
}
