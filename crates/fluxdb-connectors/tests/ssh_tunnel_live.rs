//! SSH 隧道真实链路集成测试（MongoDB）。
//!
//! 由 `scripts/ssh-tunnel-e2e.sh` 启动 Podman：MongoDB 与 sshd 在同一 pod 网络命名空间，
//! 只发布 sshd 端口；MongoDB 在宿主机不可达，只能经跳板机转发访问。因此本用例同时覆盖
//! 「SSH 档案 → 隧道 → 驱动拨号 → 命令回执」整条链路，以及失败时不退回直连。
//!
//! 未设置 `FLUXDB_SSH_E2E_SSH` / `FLUXDB_SSH_E2E_MONGO` 时跳过，常规单测不依赖外部实例。
//! 凭据只从环境变量读入内存（`SecretRef::inline`），不写文件、不打印到日志。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MongoBasicOptions,
    MongoConnectionProfile, MongoHost, SecretRef, SshAuth, SshTunnelProfile,
};

/// 跳板机与目标库参数；缺任一项即视为未启用（用例跳过）。
struct E2eSettings {
    ssh_host: String,
    ssh_port: u16,
    ssh_user: String,
    ssh_password: Option<String>,
    ssh_key: Option<String>,
    mongo_host: String,
    mongo_port: u16,
    mongo_user: String,
    mongo_password: String,
}

fn parse_host_port(value: &str) -> Option<(String, u16)> {
    let (host, port) = value.trim().rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
}

fn settings() -> Option<E2eSettings> {
    let (ssh_host, ssh_port) = parse_host_port(&std::env::var("FLUXDB_SSH_E2E_SSH").ok()?)?;
    let (mongo_host, mongo_port) = parse_host_port(&std::env::var("FLUXDB_SSH_E2E_MONGO").ok()?)?;
    let ssh_user = std::env::var("FLUXDB_SSH_E2E_SSH_USER").ok()?;
    let ssh_password = std::env::var("FLUXDB_SSH_E2E_SSH_PASSWORD")
        .ok()
        .filter(|value| !value.is_empty());
    let ssh_key = std::env::var("FLUXDB_SSH_E2E_SSH_KEY")
        .ok()
        .filter(|value| !value.is_empty());
    if ssh_password.is_none() && ssh_key.is_none() {
        eprintln!("跳过：FLUXDB_SSH_E2E_SSH_PASSWORD 与 FLUXDB_SSH_E2E_SSH_KEY 至少要设置一个");
        return None;
    }
    Some(E2eSettings {
        ssh_host,
        ssh_port,
        ssh_user,
        ssh_password,
        ssh_key,
        mongo_host,
        mongo_port,
        mongo_user: std::env::var("FLUXDB_SSH_E2E_MONGO_USER").unwrap_or_default(),
        mongo_password: std::env::var("FLUXDB_SSH_E2E_MONGO_PASSWORD").unwrap_or_default(),
    })
}

fn ssh_profile(settings: &E2eSettings, use_private_key: bool) -> SshTunnelProfile {
    let mut profile = SshTunnelProfile {
        enabled: true,
        host: settings.ssh_host.clone(),
        port: settings.ssh_port,
        username: settings.ssh_user.clone(),
        auth: SshAuth::Password,
        connect_timeout_secs: 10,
        ..SshTunnelProfile::default()
    };
    if use_private_key {
        profile.auth = SshAuth::PrivateKey;
        profile.private_key = SecretRef::ref_key(settings.ssh_key.clone().unwrap_or_default());
    } else {
        profile.password = SecretRef::inline(settings.ssh_password.clone().unwrap_or_default());
    }
    profile
}

fn config(settings: &E2eSettings, ssh: Option<SshTunnelProfile>) -> ConnectionConfig {
    let profile = MongoConnectionProfile {
        basic: MongoBasicOptions {
            hosts: vec![MongoHost {
                host: settings.mongo_host.clone(),
                port: settings.mongo_port,
            }],
            username: settings.mongo_user.clone(),
            password: if settings.mongo_password.is_empty() {
                SecretRef::default()
            } else {
                SecretRef::inline(settings.mongo_password.clone())
            },
            auth_database: if settings.mongo_user.is_empty() {
                String::new()
            } else {
                "admin".into()
            },
            ..MongoBasicOptions::default()
        },
        ssh: ssh.unwrap_or_default(),
        ..MongoConnectionProfile::default()
    };
    ConnectionConfig {
        id: ConnectionId(9_902),
        name: "MongoDB SSH tunnel live".into(),
        kind: DatabaseKind::MongoDb,
        endpoint: Endpoint::Tcp {
            host: settings.mongo_host.clone(),
            port: settings.mongo_port,
            database: None,
        },
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: Some(profile),
        oracle_profile: None,
    }
}

/// 经隧道连接并执行真实命令；直连同一目标必须失败，证明流量确实走了跳板机。
#[test]
fn mongo_connects_through_ssh_tunnel_and_direct_dial_fails() {
    let Some(settings) = settings() else {
        eprintln!("跳过：未设置 FLUXDB_SSH_E2E_SSH / FLUXDB_SSH_E2E_MONGO");
        return;
    };
    // 有私钥就优先覆盖私钥分支；密码分支由下面的错误密码用例覆盖。
    let use_private_key = settings.ssh_key.is_some();
    let tunneled = config(&settings, Some(ssh_profile(&settings, use_private_key)));
    let connector = connector_for(&tunneled).expect("MongoDB connector");

    connector
        .test_connection(&tunneled)
        .expect("经 SSH 隧道必须能完成认证与选主");
    let reply = connector
        .mongo_run_command(tunneled.id, "admin", r#"{ "ping": 1 }"#)
        .expect("经隧道执行 ping");
    assert!(
        reply.contains("\"ok\""),
        "ping 回执必须来自真实 MongoDB: {reply}"
    );

    // 目标库不在宿主机网络里：去掉隧道后同一配置必须失败，排除「悄悄直连」。
    let direct = config(&settings, None);
    let direct_result = connector.test_connection(&direct);
    assert!(
        direct_result.is_err(),
        "目标库不应能从宿主机直连，否则本用例不能证明隧道生效"
    );
}

/// 错误密码必须在 SSH 层失败并报认证错误，而不是回退直连。
#[test]
fn wrong_ssh_password_fails_at_ssh_layer() {
    let Some(settings) = settings() else {
        return;
    };
    let mut ssh = ssh_profile(&settings, false);
    ssh.password = SecretRef::inline("fluxdb-wrong-password");
    let config = config(&settings, Some(ssh));
    let connector = connector_for(&config).expect("MongoDB connector");

    let error = connector
        .test_connection(&config)
        .expect_err("错误 SSH 密码必须失败");
    let message = error.to_string();
    assert!(
        message.contains("SSH"),
        "失败应来自 SSH 认证而不是直连数据库: {message}"
    );
}
