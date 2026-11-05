//! 达梦的 AI SQL 执行链真实验证。
//!
//! 仅在提供 `FLUXDB_DM_SMOKE=host:port:user:password` 时运行。覆盖：
//! 只读 SELECT 放行、写语句被策略拒绝、绕过策略的写入被服务端事务回滚。

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DamengAdvancedOptions, DamengBasicOptions,
    DamengConnectionProfile, DatabaseKind, Endpoint, ObjectKind, ObjectPath, QueryExecutionOptions,
    QueryMode, QueryRequest, SecretRef,
};
use std::collections::BTreeMap;

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_DM_SMOKE").ok()?;
    let parts = raw.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let port = parts[1].parse().ok()?;
    Some(ConnectionConfig {
        id: ConnectionId(9_913),
        name: "DM8 AI exec".into(),
        kind: DatabaseKind::Dameng,
        endpoint: Endpoint::Tcp {
            host: parts[0].into(),
            port,
            database: Some("DAMENG".into()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: Some(DamengConnectionProfile {
            basic: DamengBasicOptions {
                host: parts[0].into(),
                port,
                database: "DAMENG".into(),
                schema: parts[2].to_ascii_uppercase(),
                username: parts[2].into(),
                password: SecretRef::inline(parts[3]),
            },
            advanced: DamengAdvancedOptions {
                connect_timeout_secs: 5,
                query_timeout_secs: 30,
                encoding: String::new(),
                application_name: "FluxDB AI test".into(),
            },
            ssh: Default::default(),
        }),
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    })
}

fn schema(config: &ConnectionConfig) -> String {
    config
        .dameng_profile
        .as_ref()
        .map(|profile| profile.basic.schema.clone())
        .filter(|schema| !schema.trim().is_empty())
        .unwrap_or_else(|| "SYSDBA".into())
}

fn qualified(config: &ConnectionConfig, table: &str) -> String {
    format!("\"{}\".\"{}\"", schema(config), table.to_ascii_uppercase())
}

fn execute(config: &ConnectionConfig, sql: &str) {
    let connector = connector_for(config).expect("DM connector");
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: Some("DAMENG".into()),
            schema: Some(schema(config)),
            session_id: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions {
                continue_on_error: false,
                split_statements: true,
                ..QueryExecutionOptions::default()
            },
        })
        .expect("execute DM SQL");
    if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
        panic!("DM SQL failed: {}\n{sql}", summary.message);
    }
}

#[test]
#[ignore = "需要 FLUXDB_DM_SMOKE=host:port:user:password 指向可达的 DM8"]
fn dameng_ai_read_only_executes_select_and_blocks_writes() {
    let Some(config) = smoke_config() else {
        eprintln!("skip: FLUXDB_DM_SMOKE 未设置");
        return;
    };
    let table = format!(
        "FLUXDB_AI_{}",
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .to_ascii_uppercase()
    );
    let object = qualified(&config, &table);
    execute(
        &config,
        &format!(
            "CREATE TABLE {object} (ID INT IDENTITY(1,1) PRIMARY KEY, NAME VARCHAR(50) NOT NULL);
             INSERT INTO {object} (NAME) VALUES ('alpha');
             INSERT INTO {object} (NAME) VALUES ('beta');"
        ),
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let resolver = fluxdb_app::MapSqlResolver::new([config.clone()]);
        let target = fluxdb_app::AiSqlTarget::parse(&format!(
            "conn:{}/db:DAMENG/schema:{}",
            config.id.0,
            schema(&config)
        ))
        .expect("target");
        let should_cancel = || false;

        let outcome = fluxdb_app::execute_sql(
            &resolver as &dyn fluxdb_app::SqlExecutionResolver,
            &target,
            &format!("SELECT NAME FROM {object} ORDER BY ID"),
            10,
            true,
            &should_cancel,
        )
        .expect("read-only select");
        assert_eq!(outcome.rows.len(), 2, "{:?}", outcome.rows);
        assert!(
            outcome.executed_sql.to_ascii_uppercase().contains("LIMIT"),
            "{}",
            outcome.executed_sql
        );

        let error = fluxdb_app::execute_sql(
            &resolver as &dyn fluxdb_app::SqlExecutionResolver,
            &target,
            &format!("UPDATE {object} SET NAME = 'hacked'"),
            10,
            true,
            &should_cancel,
        )
        .expect_err("write must be rejected");
        assert_eq!(error.code, "not_read_only");

        let error = fluxdb_app::execute_sql(
            &resolver as &dyn fluxdb_app::SqlExecutionResolver,
            &target,
            "SELECT UTL_HTTP.REQUEST('http://127.0.0.1') FROM DUAL",
            10,
            true,
            &should_cancel,
        )
        .expect_err("UTL_HTTP must not be read-only");
        assert!(
            matches!(error.code, "not_read_only" | "rejected_sql"),
            "{error:?}"
        );

        // 服务端兜底：即使策略漏判，只读事务也会回滚写入。
        let connector = connector_for(&config).expect("DM connector");
        let blocked = connector
            .execute(&QueryRequest {
                connection_id: config.id,
                database: Some("DAMENG".into()),
                schema: Some(schema(&config)),
                session_id: None,
                text: format!("UPDATE {object} SET NAME = 'rolled-back'"),
                mode: QueryMode::All,
                options: QueryExecutionOptions {
                    read_only: true,
                    continue_on_error: false,
                    split_statements: false,
                    ..QueryExecutionOptions::default()
                },
            })
            .expect("read-only write executes then rolls back");
        assert!(
            blocked.summaries.iter().any(|summary| !summary.success),
            "write in read-only session must not be reported as success"
        );
        let page = connector
            .load_data(
                &ObjectPath {
                    connection_id: config.id,
                    database: Some("DAMENG".into()),
                    schema: Some(schema(&config)),
                    name: table.clone(),
                    kind: ObjectKind::Table,
                },
                0,
                10,
                &[],
                &[],
            )
            .expect("load after rollback");
        let names = page
            .rows
            .iter()
            .map(|row| row.values[1].display_label())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    }));
    execute(&config, &format!("DROP TABLE {object}"));
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
