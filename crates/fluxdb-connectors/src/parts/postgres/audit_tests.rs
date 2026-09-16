use super::*;
use fluxdb_core::{PostgresConnectionProfile, PostgresSslMode, QueryExecutionOptions, QueryMode};

fn config() -> ConnectionConfig {
    let profile = PostgresConnectionProfile::from_uri(
        &std::env::var("FLUXDB_PG_AUDIT_URL").unwrap_or_else(|_| "postgres://postgres:secret@127.0.0.1:5432/postgres".into())
    ).expect("测试 URI");
    ConnectionConfig {
        id: ConnectionId(918_001), name: "PG audit regression".into(), kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp { host: profile.basic.host.clone(), port: profile.basic.port, database: Some(profile.maintenance_database().into()) },
        credential_ref: None, options: Default::default(), redis_profile: None, mysql_profile: None, postgres_profile: Some(profile),
    }
}
fn request(config: &ConnectionConfig, sql: &str) -> QueryRequest {
    QueryRequest { connection_id: config.id, database: None, schema: None, text: sql.into(), mode: QueryMode::All,
        options: QueryExecutionOptions::default(), session_id: Some(QuerySessionId(918_001)) }
}

#[test]
fn tls_policy_requires_encryption_for_all_strict_modes() {
    for mode in [PostgresSslMode::Require, PostgresSslMode::VerifyCa, PostgresSslMode::VerifyFull] {
        let mut config = config();
        let tls = &mut config.postgres_profile.as_mut().unwrap().tls;
        tls.enabled = true; tls.ssl_mode = mode;
        assert_eq!(pg_config(&config, "postgres").unwrap().get_ssl_mode(), tokio_postgres::config::SslMode::Require);
    }
}
#[test]
fn text_protocol_preserves_non_null_complex_values() {
    for (ty, value) in [("numeric", "12345678901234567890.123456789"), ("uuid", "00000000-0000-0000-0000-000000000001"), ("_int4", "{1,NULL,2}"), ("date", "infinity"), ("inet", "127.0.0.1"), ("int4range", "[1,3)")] {
        let col = query_column("v", ty);
        assert_eq!(pg_text_value(Some(value), &col).unwrap(), CellValue::Text(value.into()));
        assert_eq!(pg_text_value(None, &col).unwrap(), CellValue::Null);
    }
    assert_eq!(pg_decode_bytea(r"\x00ff5c"), Some(vec![0,255,92]));
    assert_eq!(pg_decode_bytea(r"\000\377\\"), Some(vec![0,255,92]));
    assert!(pg_decode_bytea(r"\999").is_none());
    assert!(pg_text_value(Some("bad"), &query_column("v", "int4")).is_err());
}
#[test]
fn recovery_bypasses_nested_leading_comments() {
    assert!(pg_is_transaction_recovery("/* a /* b */ c */ -- next\nROLLBACK TO SAVEPOINT x"));
    assert!(!pg_is_transaction_recovery("/* ROLLBACK */ SELECT 1"));
}

/// 必须显式运行；未连接真库时显示 ignored，不能伪装通过。
#[test]
#[ignore = "requires isolated PostgreSQL (FLUXDB_PG_AUDIT_URL)"]
fn live_audit_query_protocol_and_session() {
    let config = config();
    let mut req = request(&config, "CREATE TEMP TABLE audit_rows (id int PRIMARY KEY, n numeric, j jsonb)");
    let execute = |req: &QueryRequest| {
        let result = pg_execute_query(&config, req).unwrap();
        assert!(result.summaries.iter().all(|s| s.success), "{:?}", result.summaries);
        result
    };
    execute(&req);
    req.text = "/* write */ INSERT INTO audit_rows VALUES (1,12345678901234567890.12345,'{}') RETURNING *".into();
    let result = execute(&req);
    assert_eq!(result.summaries[0].affected_rows, 1);
    assert_eq!(result.results[0].rows[0].values[1], CellValue::Text("12345678901234567890.12345".into()));
    assert_eq!(result.results[0].rows[0].values[2], CellValue::Json("{}".into()));
    req.text = "-- result\nSELECT n FROM audit_rows WHERE false".into();
    let result = execute(&req);
    assert_eq!(result.results[0].columns.len(), 1);
    req.text = "BEGIN; SELECT 1/0; SELECT 1; ROLLBACK; SELECT 42".into();
    let result = pg_execute_query(&config, &req).unwrap();
    assert!(!result.summaries[1].success);
    assert!(result.summaries[3].success && result.summaries[4].success, "{:?}", result.summaries);
    req.text = "SELECT generate_series(1, 10000)".into(); req.options.page_size = 3;
    let result = execute(&req);
    assert_eq!(result.results[0].rows.len(), 3);
    assert!(result.results[0].has_more);
    assert_eq!(result.summaries[0].returned_rows, 10000);
    req.text = "SELECT pg_sleep(30)".into();
    let started = Instant::now();
    let result = pg_execute_query_with_progress(&config, &req, &mut |_| {}, &|| started.elapsed() > Duration::from_millis(200)).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!result.summaries[0].success);
    req.text = "SELECT count(*) FROM audit_rows".into(); execute(&req);
    assert_eq!(pg_close_query_session(config.id, req.session_id.unwrap()), 1);
}
