//! MySQL TLS / 会话参数真实链路测试。
//!
//! 由 `scripts/ssh-tunnel-e2e.sh mysql` 启动带自签证书的 MySQL，并生成测试 CA。
//! 覆盖：
//! 1. `verify_ca` + 受信 CA → 连接成功；
//! 2. `verify_ca` 不提供 CA → 自签证书不可信，连接必须失败；
//! 3. `verify_identity` + 受信 CA → 证书 SAN 匹配主机名/IP，连接成功；
//! 4. `preferred` 不校验 → 无 CA 也能连接；
//! 5. 关闭 TLS → 明确 `ssl-mode=DISABLED`；
//! 6. 旧档案 `verify=true` + `preferred` → 按 `verify_ca` 处理；
//! 7. 字符集 / 排序规则 / 时区 / 初始化 SQL 真实生效。
//!
//! 未设置 `FLUXDB_MYSQL_TLS` 时跳过。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlAdvancedOptions,
    MysqlBasicOptions, MysqlConnectionProfile, MysqlSslMode, MysqlTlsOptions,
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
    split_endpoint(&std::env::var("FLUXDB_MYSQL_TLS").ok()?)
}

fn config(
    mode: MysqlSslMode,
    ca_path: Option<&str>,
    advanced: MysqlAdvancedOptions,
    charset: &str,
    collation: &str,
) -> Option<ConnectionConfig> {
    let (host, port, username, password) = target()?;
    let ca = ca_path
        .map(|path| SecretRef::ref_key(path.to_string()))
        .unwrap_or_default();
    Some(
        ConnectionConfig {
            id: ConnectionId(9_922),
            name: "MySQL TLS live".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: host.clone(),
                port,
                database: Some("mysql".into()),
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
                    ..MysqlBasicOptions::default()
                },
                tls: MysqlTlsOptions {
                    enabled: mode != MysqlSslMode::Disabled,
                    ssl_mode: mode,
                    ca,
                    charset: charset.into(),
                    collation: collation.into(),
                    ..MysqlTlsOptions::default()
                },
                advanced: MysqlAdvancedOptions {
                    connect_timeout_secs: 5,
                    ..advanced
                },
                ..MysqlConnectionProfile::default()
            }),
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
        .mysql_resolved(),
    )
}

fn simple(mode: MysqlSslMode, ca_path: Option<&str>) -> Option<ConnectionConfig> {
    config(
        mode,
        ca_path,
        MysqlAdvancedOptions::default(),
        "utf8mb4",
        "",
    )
}

fn execute(
    config: &ConnectionConfig,
    sql: &str,
) -> fluxdb_core::Result<fluxdb_core::QueryExecutionResult> {
    connector_for(config)
        .expect("创建 MySQL 连接器")
        .execute(&QueryRequest {
            connection_id: config.id,
            database: None,
            schema: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
            session_id: None,
        })
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TLS=host:port:user:password 与 FLUXDB_MYSQL_TLS_CA=ca.pem"]
fn mysql_tls_verify_ca_requires_trusted_ca() {
    let ca_path = std::env::var("FLUXDB_MYSQL_TLS_CA").expect("需要 FLUXDB_MYSQL_TLS_CA");
    let trusted = simple(MysqlSslMode::VerifyCa, Some(&ca_path)).expect("需要 FLUXDB_MYSQL_TLS");
    execute(&trusted, "SELECT 1").expect("提供受信 CA 时应能建立 TLS 连接");

    let untrusted = simple(MysqlSslMode::VerifyCa, None).expect("需要 FLUXDB_MYSQL_TLS");
    let error = execute(&untrusted, "SELECT 1").expect_err("未提供 CA 时自签证书应校验失败");
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("certificate"),
        "错误应指向证书校验失败，实际: {error}"
    );
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TLS 与 DNS/IP SAN 覆盖连接地址的证书"]
fn mysql_tls_verify_identity_checks_hostname() {
    let ca_path = std::env::var("FLUXDB_MYSQL_TLS_CA").expect("需要 FLUXDB_MYSQL_TLS_CA");
    let trusted =
        simple(MysqlSslMode::VerifyIdentity, Some(&ca_path)).expect("需要 FLUXDB_MYSQL_TLS");
    execute(&trusted, "SELECT 1").expect("证书 SAN 覆盖目标地址时应通过主机名校验");

    // 同一份 CA 在 verify_ca 下通过、在无 CA 的 identity 下失败，证明校验链确实生效。
    let untrusted = simple(MysqlSslMode::VerifyIdentity, None).expect("需要 FLUXDB_MYSQL_TLS");
    execute(&untrusted, "SELECT 1").expect_err("缺少 CA 时 identity 模式必须失败");
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TLS=host:port:user:password"]
fn mysql_tls_preferred_and_disabled_connect_without_ca() {
    let preferred = simple(MysqlSslMode::Preferred, None).expect("需要 FLUXDB_MYSQL_TLS");
    execute(&preferred, "SELECT 1").expect("preferred 不校验证书，应连接成功");

    let disabled = simple(MysqlSslMode::Disabled, None).expect("需要 FLUXDB_MYSQL_TLS");
    execute(&disabled, "SELECT 1").expect("关闭 TLS 后应退化为明文连接成功");
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TLS 与 FLUXDB_MYSQL_TLS_CA"]
fn mysql_legacy_verify_flag_behaves_as_verify_ca() {
    let ca_path = std::env::var("FLUXDB_MYSQL_TLS_CA").expect("需要 FLUXDB_MYSQL_TLS_CA");
    let mut trusted =
        simple(MysqlSslMode::Preferred, Some(&ca_path)).expect("需要 FLUXDB_MYSQL_TLS");
    let profile = trusted.mysql_profile.as_mut().expect("MySQL 档案");
    profile.tls.verify = true;
    assert_eq!(
        profile.tls.effective_ssl_mode(),
        MysqlSslMode::VerifyCa,
        "旧 verify=true 必须迁移为 verify_ca"
    );
    execute(&trusted, "SELECT 1").expect("迁移后提供 CA 应连接成功");
}

#[test]
#[ignore = "需要 FLUXDB_MYSQL_TLS=host:port:user:password"]
fn mysql_session_collation_timezone_and_init_sql_apply() {
    let advanced = MysqlAdvancedOptions {
        timezone: "+08:00".into(),
        init_sql: "SET @fluxdb_init = 'ok';SET SESSION group_concat_max_len = 2048".into(),
        ..MysqlAdvancedOptions::default()
    };
    let config = config(
        MysqlSslMode::Preferred,
        None,
        advanced,
        "utf8mb4",
        "utf8mb4_unicode_ci",
    )
    .expect("需要 FLUXDB_MYSQL_TLS");
    let result = execute(
        &config,
        "SELECT @@collation_connection, @@time_zone, @fluxdb_init, @@group_concat_max_len",
    )
    .expect("会话参数应生效");
    let row = format!("{:?}", result.results.first().map(|page| &page.rows));
    assert!(row.contains("utf8mb4_unicode_ci"), "排序规则未生效: {row}");
    assert!(row.contains("+08:00"), "会话时区未生效: {row}");
    assert!(row.contains("ok"), "初始化 SQL 未执行: {row}");
    assert!(row.contains("2048"), "多条初始化 SQL 未全部执行: {row}");
}
