//! 真实 Oracle 的 SQL 补全端到端验证。
//!
//! 仅在显式提供 `FLUXDB_ORACLE_SMOKE=host:port:user:password[:service]` 时运行；
//! 默认忽略。测试数据需在目标库中预先准备好，本测试只读，不建表、不改数据。

use fluxdb_app::{AppCommand, AppController};
use fluxdb_core::{
    ConnectionConfig, ConnectionId, Connector, DatabaseKind, Endpoint, OracleBasicOptions,
    OracleConnectKind, OracleConnectionProfile, OraclePrivilege, QueryCompletionItem,
    QueryCompletionResult, QueryExecutionOptions, QueryMode, QueryRequest, SecretRef,
};
use std::collections::BTreeMap;

const SCHEMA: &str = "COMPLETION_LAB";
const MARKER: &str = "__CURSOR__";
/// 已知缺口：Oracle 包成员补全还没有跨层元数据接口，补全 17 仍应失败。
const KNOWN_MISSING: &[&str] = &["17"];

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_ORACLE_SMOKE").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    assert!(
        parts.len() >= 4,
        "FLUXDB_ORACLE_SMOKE=host:port:user:password[:service]"
    );
    let port = parts[1].parse().expect("Oracle port");
    let service = parts.get(4).copied().unwrap_or("FREEPDB1").to_string();
    Some(
        ConnectionConfig {
            id: ConnectionId(9_911),
            name: "Oracle completion".into(),
            kind: DatabaseKind::Oracle,
            endpoint: Endpoint::Tcp {
                host: parts[0].into(),
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
                    host: parts[0].into(),
                    port,
                    connect_kind: OracleConnectKind::ServiceName,
                    service_name: service,
                    sid: String::new(),
                    username: parts[2].into(),
                    schema: SCHEMA.into(),
                    privilege: OraclePrivilege::Normal,
                    password: SecretRef::inline(parts[3]),
                },
                ..OracleConnectionProfile::default()
            }),
        }
        .oracle_resolved(),
    )
}

fn service_name(config: &ConnectionConfig) -> Option<String> {
    match &config.endpoint {
        Endpoint::Tcp { database, .. } => database.clone(),
        _ => None,
    }
}

fn complete(
    controller: &AppController,
    config: &ConnectionConfig,
    input: &str,
) -> QueryCompletionResult {
    let cursor = input.find(MARKER).expect("marker");
    let text = input.replace(MARKER, "");
    controller
        .query_completions_for_text(
            config.id,
            service_name(config),
            Some(SCHEMA.into()),
            text,
            cursor,
            true,
        )
        .expect("completions")
}

fn labels(result: &QueryCompletionResult) -> Vec<String> {
    result
        .items
        .iter()
        .map(|item| format!("{} => {}", item.label, item.insert_text))
        .collect()
}

fn find<'a>(result: &'a QueryCompletionResult, label: &str) -> Option<&'a QueryCompletionItem> {
    result
        .items
        .iter()
        .find(|item| item.label.eq_ignore_ascii_case(label))
}

/// 把选中的候选套回原文，验证替换范围与插入文本组合后的最终 SQL。
fn apply(text_with_marker: &str, result: &QueryCompletionResult, label: &str) -> String {
    let text = text_with_marker.replace(MARKER, "");
    let item = find(result, label).unwrap_or_else(|| {
        panic!("missing candidate {label}; got {:?}", labels(result));
    });
    let mut out = String::new();
    out.push_str(&text[..result.replace_start]);
    out.push_str(&item.insert_text);
    out.push_str(&text[result.replace_end..]);
    out
}

fn execute(connector: &dyn Connector, config: &ConnectionConfig, sql: &str) -> Vec<Vec<String>> {
    let result = connector
        .execute(&QueryRequest {
            connection_id: config.id,
            database: service_name(config),
            schema: Some(SCHEMA.into()),
            session_id: None,
            text: sql.into(),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })
        .unwrap_or_else(|error| panic!("execute failed: {error:?}\n{sql}"));
    for summary in &result.summaries {
        assert!(summary.success, "{sql}\n{}", summary.message);
    }
    let page = result
        .results
        .first()
        .unwrap_or_else(|| panic!("no result page: {sql}"));
    page.rows
        .iter()
        .map(|row| {
            row.values
                .iter()
                .map(|value| value.display_label())
                .collect()
        })
        .collect()
}

/// 教程允许 `140` 与 `140.00` 等价；其余按展示文本精确比较。
fn same_value(actual: &str, expected: &str) -> bool {
    if actual == expected {
        return true;
    }
    match (actual.parse::<f64>(), expected.parse::<f64>()) {
        (Ok(actual), Ok(expected)) => (actual - expected).abs() < 1e-9,
        _ => false,
    }
}

fn expected_rows(id: &str) -> Option<Vec<Vec<&'static str>>> {
    let rows: Vec<Vec<&'static str>> = match id {
        "01" => vec![vec!["张三"]],
        "02" => vec![vec!["Alice Chen"], vec!["Bob Li"], vec!["张三"]],
        "03" => vec![vec!["前缀中文", "张三"]],
        "04" => vec![
            vec!["1", "1001", "Alice Chen", "140", "140", "0"],
            vec!["1", "1002", "Alice Chen", "300", "0", "300"],
            vec!["1", "1004", "NULL", "0", "0", "0"],
            vec!["2", "1001", "Alice South", "120", "120", "0"],
            vec!["2", "1005", "O'Neil", "300", "300", "0"],
            vec!["2", "1006", "O'Neil", "0", "0", "0"],
        ],
        "05" => vec![
            vec!["1", "101", "Alice Chen", "440"],
            vec!["2", "104", "O'Neil", "300"],
        ],
        "06" => vec![vec!["2", "104", "O'Neil"]],
        "07" => vec![
            vec!["101", "Alice Chen"],
            vec!["102", "Bob Li"],
            vec!["103", "张三"],
        ],
        "08" => vec![
            vec!["1", "Director", "1", "0"],
            vec!["2", "Sales A", "1/2", "1"],
            vec!["3", "Sales B", "1/3", "1"],
            vec!["4", "Lead", "1/4", "1"],
            vec!["5", "Engineer", "1/4/5", "2"],
            vec!["6", "Unassigned", "6", "0"],
        ],
        "09" => vec![
            vec!["1", "10", "1", "1", "30000"],
            vec!["2", "10", "2", "2", "40000"],
            vec!["3", "10", "2", "3", "50000"],
            vec!["4", "20", "1", "1", "20000"],
            vec!["5", "20", "2", "2", "35000"],
        ],
        "10" => vec![
            vec!["1", "103", "张三"],
            vec!["1", "1004", "guest, no lines"],
        ],
        "11" => vec![
            vec!["1", "101", "Alice Chen", "Old Alice", "merged"],
            vec!["1", "102", "Bob Li", "NULL", "NULL"],
            vec!["1", "103", "张三", "NULL", "NULL"],
            vec!["2", "101", "Alice South", "NULL", "NULL"],
            vec!["2", "104", "O'Neil", "NULL", "NULL"],
            vec!["9", "999", "NULL", "Legacy", "retired"],
        ],
        "12" => vec![
            vec!["1", "张三", "keyword column", "Upper", "25"],
            vec!["2", "李四", "second", "NULL", "0"],
        ],
        "13" => vec![
            vec!["1", "101", "Alice Chen", "300"],
            vec!["1", "102", "Bob Li", "50"],
            vec!["1", "103", "张三", "NULL"],
            vec!["2", "101", "Alice South", "120"],
            vec!["2", "104", "O'Neil", "0"],
        ],
        "14" => vec![vec!["张三"]],
        "15" => vec![vec!["48", "47"]],
        "16" => vec![vec!["1", "new"], vec!["1", "vip"]],
        "18" => vec![vec!["1", "101", "Alice Chen", "merged"]],
        "21" => vec![vec!["customers"]],
        _ => return None,
    };
    Some(rows)
}

fn assert_rows(id: &str, actual: Vec<Vec<String>>, expected: Vec<Vec<&'static str>>) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "case {id}: row count, actual={actual:?}"
    );
    for (row_index, (actual_row, expected_row)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(
            actual_row.len(),
            expected_row.len(),
            "case {id}: row {row_index} width, actual={actual:?}"
        );
        for (column, (actual, expected)) in actual_row.iter().zip(expected_row).enumerate() {
            assert!(
                same_value(actual, expected),
                "case {id}: row {row_index} column {column}: actual={actual:?} expected={expected:?}\nrows={actual:?}"
            );
        }
    }
}

#[test]
fn oracle_sql_completion_tutorial() {
    std::panic::set_hook(Box::new(|info| {
        let message = info.to_string();
        let _ = std::fs::write("/tmp/oracle-completion-panic.txt", &message);
        eprintln!("{message}");
    }));
    let Some(config) = smoke_config() else {
        eprintln!("skip: FLUXDB_ORACLE_SMOKE not set");
        return;
    };
    let connector = fluxdb_connectors::connector_for(&config).expect("connector");
    connector
        .test_connection(&config)
        .expect("oracle connection");
    let mut controller = AppController::new();
    let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));

    let cases: Vec<(&str, &str, &str, &[&str])> = vec![
        (
            "01",
            r#"SELECT c.customer_name FROM COMPLETION_LAB.cust__CURSOR__ c
WHERE c.tenant_id=1 AND c.customer_id=103;"#,
            "customers",
            &["orders", "order_items", "archive_reason"],
        ),
        (
            "02",
            r#"SELECT c.__CURSOR__ FROM COMPLETION_LAB.customers c
WHERE c.tenant_id=1 ORDER BY c.customer_id;"#,
            "customer_name",
            &["order_id", "total_amount", "archive_reason"],
        ),
        (
            "03",
            r#"-- 中文注释：后方语句不应发生坐标漂移
SELECT '前缀中文' AS marker,
  c.cust__CURSOR__omer_name AS chosen
FROM COMPLETION_LAB.customers c WHERE c.tenant_id=1 AND c.customer_id=103;"#,
            "customer_name",
            &["customer_nameomer_name", "custcustomer_name"],
        ),
        (
            "04",
            r#"WITH item_totals AS (
  SELECT tenant_id, order_id, SUM(quantity*unit_price-discount) AS item_total
  FROM COMPLETION_LAB.order_items GROUP BY tenant_id, order_id
), payment_totals AS (
  SELECT tenant_id, order_id, SUM(amount) AS paid_total
  FROM COMPLETION_LAB.payments GROUP BY tenant_id, order_id
)
SELECT o.tenant_id, o.order_id, c.customer_name,
       COALESCE(it.it__CURSOR__,0) AS item_total,
       COALESCE(pt.paid_total,0) AS paid_total,
       o.total_amount-COALESCE(pt.paid_total,0) AS outstanding
FROM COMPLETION_LAB.orders o
LEFT JOIN COMPLETION_LAB.customers c
  ON o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id
LEFT JOIN item_totals it ON it.tenant_id=o.tenant_id AND it.order_id=o.order_id
LEFT JOIN payment_totals pt ON pt.tenant_id=o.tenant_id AND pt.order_id=o.order_id
WHERE o.status<>'cancelled'
ORDER BY o.tenant_id,o.order_id;"#,
            "item_total",
            &["quantity", "unit_price", "amount", "paid_total"],
        ),
        (
            "05",
            r#"WITH base(tid,cid,amount) AS (
  SELECT tenant_id,customer_id,total_amount FROM COMPLETION_LAB.orders WHERE customer_id IS NOT NULL
), totals AS (
  SELECT tid,cid,SUM(amount) AS total_spent FROM base GROUP BY tid,cid
), ranked AS (
  SELECT tid,cid,total_spent,
         ROW_NUMBER() OVER(PARTITION BY tid ORDER BY total_spent DESC,cid) AS tenant_rank
  FROM totals
)
SELECT r.tid,r.cid,c.customer_name,r.total_s__CURSOR__
FROM ranked r JOIN COMPLETION_LAB.customers c ON c.tenant_id=r.tid AND c.customer_id=r.cid
WHERE r.tenant_rank=1 ORDER BY r.tid;"#,
            "total_spent",
            &["total_amount", "amount", "order_id", "email"],
        ),
        (
            "06",
            r#"SELECT c.tenant_id,c.customer_id,c.customer_name FROM COMPLETION_LAB.customers c
WHERE EXISTS (
  SELECT 1 FROM COMPLETION_LAB.orders o
  WHERE o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id AND o.status='paid'
    AND EXISTS (
      SELECT 1 FROM COMPLETION_LAB.order_items i
      WHERE i.tenant_id=o.tenant_id AND i.order_id=o.order_id
        AND EXISTS (SELECT 1 FROM COMPLETION_LAB.products p
                    WHERE p.product_id=i.product_id AND p.unit_price>=300
                      AND o.tenant_id=c.tenant_id AND o.customer_id=c.cust__CURSOR__)
    )
) ORDER BY c.tenant_id,c.customer_id;"#,
            "customer_id",
            &["order_id", "line_no"],
        ),
        (
            "07",
            r#"SELECT x.customer_id,x.customer_name FROM COMPLETION_LAB.customers x
WHERE x.tenant_id=1 AND EXISTS (
  SELECT 1 FROM COMPLETION_LAB.orders x
  WHERE x.tenant_id=2 AND x.order___CURSOR__=1005
) ORDER BY x.customer_id;"#,
            "order_id",
            &["customer_name", "email", "credit_limit"],
        ),
        (
            "08",
            r#"WITH chain(employee_id,employee_name,manager_id,path,depth) AS (
  SELECT employee_id,employee_name,manager_id,CAST(employee_id AS VARCHAR2(500)),0
  FROM COMPLETION_LAB.employees WHERE manager_id IS NULL
  UNION ALL
  SELECT e.employee_id,e.employee_name,e.manager_id,CAST(chain.path||'/'||CAST(e.employee_id AS VARCHAR2(20)) AS VARCHAR2(500)),chain.depth+1
  FROM COMPLETION_LAB.employees e JOIN chain ON e.manager_id=chain.employee_id
) SELECT chain.employee_id,chain.employee_name,chain.pa__CURSOR__,chain.depth
FROM chain ORDER BY chain.employee_id;"#,
            "path",
            &["salary", "department_id"],
        ),
        (
            "09",
            r#"WITH ranked AS (
  SELECT e.employee_id,e.department_id,e.salary,
         DENSE_RANK() OVER(PARTITION BY e.department_id ORDER BY e.salary DESC) AS salary_rank,
         ROW_NUMBER() OVER(PARTITION BY e.department_id ORDER BY e.salary DESC,e.employee_id) AS row_num,
         SUM(e.salary) OVER(PARTITION BY e.department_id ORDER BY e.salary DESC,e.employee_id
                            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running_pay
  FROM COMPLETION_LAB.employees e WHERE e.department_id IS NOT NULL
) SELECT r.employee_id,r.department_id,r.salary_r__CURSOR__,r.row_num,r.running_pay
FROM ranked r ORDER BY r.department_id,r.row_num;"#,
            "salary_rank",
            &["manager_id", "employee_name"],
        ),
        (
            "10",
            r#"SELECT u.tenant_id,u.entity___CURSOR__,u.label
FROM (
  SELECT c.tenant_id,c.customer_id AS entity_id,c.customer_name AS label
  FROM COMPLETION_LAB.customers c WHERE c.tenant_id=1 AND c.customer_id=103
  UNION ALL
  SELECT o.tenant_id,o.order_id,o.note
  FROM COMPLETION_LAB.orders o WHERE o.tenant_id=1 AND o.customer_id IS NULL
) u ORDER BY u.entity_id;"#,
            "entity_id",
            &["customer_id", "order_id", "note"],
        ),
        (
            "11",
            r#"SELECT c.tenant_id,c.customer_id,c.customer_name AS current_name,
       a.archived_name AS historical_name,a.archive_r__CURSOR__
FROM COMPLETION_LAB.customers c LEFT JOIN COMPLETION_ARCHIVE.customers a
  ON a.tenant_id=c.tenant_id AND a.customer_id=c.customer_id
UNION ALL
SELECT a.tenant_id,a.customer_id,NULL,a.archived_name,a.archive_reason
FROM COMPLETION_ARCHIVE.customers a WHERE NOT EXISTS (
  SELECT 1 FROM COMPLETION_LAB.customers c WHERE c.tenant_id=a.tenant_id AND c.customer_id=a.customer_id
) ORDER BY tenant_id,customer_id;"#,
            "archive_reason",
            &["customer_name", "email", "credit_limit"],
        ),
        (
            "12",
            r#"SELECT d."Row ID" AS row_id,d."客户__CURSOR__" AS customer_name,
       d."select" AS reserved_value,d."MixedCase" AS mixed_value,
       ROUND(d."unit-price"*2,2) AS doubled
FROM COMPLETION_LAB."Order Details" d ORDER BY d."Row ID";"#,
            "客户名称",
            &["Row ID", "select", "MixedCase", "unit-price"],
        ),
        (
            "13",
            r#"SELECT c.tenant_id,c.customer_id,c.customer_name,recent.latest___CURSOR__
FROM COMPLETION_LAB.customers c OUTER APPLY (
  SELECT o.total_amount AS latest_amount FROM COMPLETION_LAB.orders o
  WHERE o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id
  ORDER BY o.ordered_at DESC,o.order_id DESC FETCH FIRST 1 ROWS ONLY
) recent
ORDER BY c.tenant_id,c.customer_id;"#,
            "latest_amount",
            &["total_amount", "ordered_at", "customer_id"],
        ),
        (
            "14",
            r#"WITH original AS (SELECT c.* FROM COMPLETION_LAB.customers c),
trimmed AS (SELECT tenant_id,customer_id,customer_name FROM original)
SELECT t.customer_n__CURSOR__ FROM trimmed t
WHERE t.tenant_id=1 AND t.customer_id=103;"#,
            "customer_name",
            &["email", "region", "credit_limit", "created_at"],
        ),
        (
            "15",
            r#"WITH measures AS (
  SELECT metric_id,w.metric_48 AS last_metric,w.metric_01 AS first_metric
  FROM COMPLETION_LAB.wide_metrics w
) SELECT m.last_m__CURSOR__,m.last_metric-m.first_metric AS gap
FROM measures m WHERE m.metric_id=1;"#,
            "last_metric",
            &["metric_02", "metric_47", "metric_48"],
        ),
        (
            "16",
            r#"SELECT e.event_id,jt.ta__CURSOR__ FROM COMPLETION_LAB.audit_events e,
  JSON_TABLE(e.payload, '$.tags[*]' COLUMNS (tag VARCHAR2(30) PATH '$')) jt
WHERE e.event_id=1 ORDER BY jt.tag;"#,
            "tag",
            &["payload", "entity_id"],
        ),
        (
            "17",
            r#"SELECT o.order_id,COMPLETION_LAB.lab_pkg.dis__CURSOR__(o.total_amount) AS default_discount,
  COMPLETION_LAB.lab_pkg.discount(o.total_amount,0.2) AS custom_discount
FROM COMPLETION_LAB.orders o WHERE o.tenant_id=1 AND o.order_id=1002;"#,
            "discount",
            &["customer_id", "total_amount"],
        ),
        (
            "18",
            r#"SELECT live.tenant_id,live.customer_id,live.customer_n__CURSOR__,old.archive_reason
FROM COMPLETION_LAB.customer_alias live JOIN COMPLETION_LAB.archive_customer_alias old
  ON old.tenant_id=live.tenant_id AND old.customer_id=live.customer_id
ORDER BY live.tenant_id,live.customer_id;"#,
            "customer_name",
            &["archive_reason", "archived_name"],
        ),
        (
            "19",
            r#"INSERT INTO COMPLETION_LAB.customers (tenant_id,customer_id,customer_name,em__CURSOR__,created_at)
SELECT 2,901,'Completion Insert',NULL,CURRENT_TIMESTAMP FROM dual WHERE 1=0;"#,
            "email",
            &["tenant_id", "customer_id", "customer_name", "created_at"],
        ),
        (
            "20",
            r#"UPDATE COMPLETION_LAB.customers SET customer_name=customer_name, credit___CURSOR__=credit_limit
WHERE tenant_id=1 AND customer_id=101 AND 1=0;"#,
            "credit_limit",
            &["customer_name"],
        ),
        (
            "21",
            r#"SELECT 'cust__CURSOR__omers' AS sample FROM dual;"#,
            "",
            &["customers"],
        ),
    ];

    let only = std::env::var("FLUXDB_ORACLE_CASE").ok();
    for (id, input, pick, forbidden) in cases {
        if only.as_deref().is_some_and(|only| only != id) {
            continue;
        }
        eprintln!("case {id}: completing");
        let result = complete(&controller, &config, input);
        let missing = if pick.is_empty() {
            false
        } else {
            find(&result, pick).is_none()
        };

        if KNOWN_MISSING.contains(&id) {
            assert!(
                missing,
                "case {id} 已能补全 {}，请从 KNOWN_MISSING 移除并补断言；候选 {:?}",
                pick,
                labels(&result)
            );
            continue;
        }

        if pick.is_empty() {
            assert!(
                result.items.is_empty(),
                "case {id}: 字符串内不应出现 SQL 候选，实际 {:?}",
                labels(&result)
            );
            let adopted = input.replace(MARKER, "");
            assert_eq!(adopted, "SELECT 'customers' AS sample FROM dual;");
            continue;
        }

        assert!(
            !missing,
            "case {id}: 缺少候选 {pick}，实际 {:?}",
            labels(&result)
        );
        for forbidden in forbidden {
            assert!(
                find(&result, forbidden).is_none(),
                "case {id}: 不应出现候选 {forbidden}，实际 {:?}",
                labels(&result)
            );
        }

        let adopted = apply(input, &result, pick);
        if let Some(rows) = expected_rows(id) {
            eprintln!("case {id}: executing adopted SQL");
            let actual = execute(connector.as_ref(), &config, &adopted);
            eprintln!("case {id}: executed, {} rows", actual.len());
            assert_rows(id, actual, rows);
        } else {
            // 19/20 只核对补完文字，不执行写入。
            assert!(
                adopted.to_uppercase().contains(&pick.to_uppercase()),
                "case {id}: 采纳后文本缺少 {pick}：{adopted}"
            );
        }
    }
}
