//! 真实 PostgreSQL 的 SQL 补全端到端验证。
//!
//! 仅在显式提供 `FLUXDB_PG_COMPLETION=host:port:user:password:database` 时运行；
//! 否则直接跳过。测试数据由 `docs/testing/sql-completion/postgres/数据准备.sql`
//! 预置，本测试只读，不建表、不改数据。用例对应同目录《测试教程.md》。
//!
//! 走的是生产补全路径：`AppController::query_completions_for_text` → 补全引擎 →
//! PostgreSQL connector 的 pg_catalog 元数据查询，因此同时验证候选范围、插入文本
//! 和采纳后的 SQL 结果。

use fluxdb_app::{AppCommand, AppController};
use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, Connector, DatabaseKind, Endpoint, PostgresBasicOptions,
    PostgresConnectionProfile, PostgresScopeOptions, QueryCompletionItem, QueryCompletionResult,
    QueryExecutionOptions, QueryMode, QueryRequest, SecretRef,
};
use std::collections::BTreeMap;

const DATABASE: &str = "completion_lab";
const SCHEMA: &str = "lab";
const MARKER: &str = "__CURSOR__";

fn smoke_config() -> Option<ConnectionConfig> {
    let raw = std::env::var("FLUXDB_PG_COMPLETION").ok()?;
    let parts = raw.splitn(5, ':').collect::<Vec<_>>();
    assert_eq!(
        parts.len(),
        5,
        "FLUXDB_PG_COMPLETION=host:port:user:password:database"
    );
    let host = parts[0].to_string();
    let port: u16 = parts[1].parse().expect("PostgreSQL port");
    let database = parts[4].to_string();
    let mut options = BTreeMap::new();
    options.insert("host".into(), host.clone());
    options.insert("port".into(), port.to_string());
    options.insert("maintenance_database".into(), database.clone());
    options.insert("username".into(), parts[2].into());
    options.insert("password".into(), parts[3].into());
    options.insert("default_schema".into(), SCHEMA.into());
    Some(
        ConnectionConfig {
            id: ConnectionId(9_912),
            name: "PostgreSQL completion".into(),
            kind: DatabaseKind::Postgres,
            endpoint: Endpoint::Tcp {
                host,
                port,
                database: Some(database),
            },
            options: options.clone(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: Some(PostgresConnectionProfile {
                basic: PostgresBasicOptions {
                    host: parts[0].into(),
                    port,
                    maintenance_database: parts[4].into(),
                    username: parts[2].into(),
                    password: SecretRef::inline(parts[3]),
                },
                scope: PostgresScopeOptions {
                    default_schema: SCHEMA.into(),
                    ..PostgresScopeOptions::default()
                },
                ..PostgresConnectionProfile::default()
            }),
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
        .postgres_resolved(),
    )
}

fn complete(controller: &AppController, id: ConnectionId, input: &str) -> QueryCompletionResult {
    let cursor = input.find(MARKER).expect("marker");
    let text = input.replace(MARKER, "");
    controller
        .query_completions_for_text(
            id,
            Some(DATABASE.into()),
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
    result.items.iter().find(|item| {
        // 例程候选带签名（`lab_discount(p_amount numeric)`），教程允许按名字选中。
        item.label.eq_ignore_ascii_case(label)
            || item
                .label
                .split('(')
                .next()
                .is_some_and(|base| base.eq_ignore_ascii_case(label))
    })
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

fn execute(connector: &dyn Connector, id: ConnectionId, sql: &str) -> Vec<Vec<String>> {
    let result = connector
        .execute(&QueryRequest {
            connection_id: id,
            database: Some(DATABASE.into()),
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
        "17" => vec![vec!["1002", "270", "240"]],
        "18" => vec![
            vec!["1", "101", "1002", "300"],
            vec!["1", "102", "1003", "50"],
            vec!["2", "101", "1001", "120"],
            vec!["2", "104", "1006", "0"],
        ],
        "21" => vec![vec!["customers"]],
        _ => return None,
    };
    Some(rows)
}

fn check_rows(actual: Vec<Vec<String>>, expected: Vec<Vec<&'static str>>) -> Result<(), String> {
    if actual.len() != expected.len() {
        return Err(format!(
            "row count, actual={actual:?} expected={expected:?}"
        ));
    }
    for (row_index, (actual_row, expected_row)) in actual.iter().zip(&expected).enumerate() {
        if actual_row.len() != expected_row.len() {
            return Err(format!(
                "row {row_index} width, actual={actual:?} expected={expected:?}"
            ));
        }
        for (column, (actual, expected)) in actual_row.iter().zip(expected_row).enumerate() {
            if !same_value(actual, expected) {
                return Err(format!(
                    "row {row_index} column {column}: actual={actual:?} expected={expected:?}\nrows={actual:?}"
                ));
            }
        }
    }
    Ok(())
}

struct TutorialCase {
    id: &'static str,
    input: &'static str,
    pick: &'static str,
    forbidden: &'static [&'static str],
}

const CASES: &[TutorialCase] = &[
    TutorialCase {
        id: "01",
        input: "SELECT c.customer_name FROM lab.cust__CURSOR__ c\nWHERE c.tenant_id=1 AND c.customer_id=103;",
        pick: "customers",
        forbidden: &["orders", "order_items", "archive_reason"],
    },
    TutorialCase {
        id: "02",
        input: "SELECT c.__CURSOR__ FROM lab.customers c\nWHERE c.tenant_id=1 ORDER BY c.customer_id;",
        pick: "customer_name",
        forbidden: &["order_id", "total_amount", "archive_reason"],
    },
    TutorialCase {
        id: "03",
        input: "-- 中文注释：后方语句不应发生坐标漂移\nSELECT '前缀中文' AS marker,\n  c.cust__CURSOR__omer_name AS chosen\nFROM lab.customers c WHERE c.tenant_id=1 AND c.customer_id=103;",
        pick: "customer_name",
        forbidden: &["customer_nameomer_name", "custcustomer_name"],
    },
    TutorialCase {
        id: "04",
        input: "WITH item_totals AS (\n  SELECT tenant_id, order_id, SUM(quantity*unit_price-discount) AS item_total\n  FROM lab.order_items GROUP BY tenant_id, order_id\n), payment_totals AS (\n  SELECT tenant_id, order_id, SUM(amount) AS paid_total\n  FROM lab.payments GROUP BY tenant_id, order_id\n)\nSELECT o.tenant_id, o.order_id, c.customer_name,\n       COALESCE(it.it__CURSOR__,0) AS item_total,\n       COALESCE(pt.paid_total,0) AS paid_total,\n       o.total_amount-COALESCE(pt.paid_total,0) AS outstanding\nFROM lab.orders o\nLEFT JOIN lab.customers c\n  ON o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id\nLEFT JOIN item_totals it ON it.tenant_id=o.tenant_id AND it.order_id=o.order_id\nLEFT JOIN payment_totals pt ON pt.tenant_id=o.tenant_id AND pt.order_id=o.order_id\nWHERE o.status<>'cancelled'\nORDER BY o.tenant_id,o.order_id;",
        pick: "item_total",
        forbidden: &["quantity", "unit_price", "amount", "paid_total"],
    },
    TutorialCase {
        id: "05",
        input: "WITH base(tid,cid,amount) AS (\n  SELECT tenant_id,customer_id,total_amount FROM lab.orders WHERE customer_id IS NOT NULL\n), totals AS (\n  SELECT tid,cid,SUM(amount) AS total_spent FROM base GROUP BY tid,cid\n), ranked AS (\n  SELECT tid,cid,total_spent,\n         ROW_NUMBER() OVER(PARTITION BY tid ORDER BY total_spent DESC,cid) AS tenant_rank\n  FROM totals\n)\nSELECT r.tid,r.cid,c.customer_name,r.total_s__CURSOR__\nFROM ranked r JOIN lab.customers c ON c.tenant_id=r.tid AND c.customer_id=r.cid\nWHERE r.tenant_rank=1 ORDER BY r.tid;",
        pick: "total_spent",
        forbidden: &["total_amount", "amount", "order_id", "email"],
    },
    TutorialCase {
        id: "06",
        input: "SELECT c.tenant_id,c.customer_id,c.customer_name FROM lab.customers c\nWHERE EXISTS (\n  SELECT 1 FROM lab.orders o\n  WHERE o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id AND o.status='paid'\n    AND EXISTS (\n      SELECT 1 FROM lab.order_items i\n      WHERE i.tenant_id=o.tenant_id AND i.order_id=o.order_id\n        AND EXISTS (SELECT 1 FROM lab.products p\n                    WHERE p.product_id=i.product_id AND p.unit_price>=300\n                      AND o.tenant_id=c.tenant_id AND o.customer_id=c.cust__CURSOR__)\n    )\n) ORDER BY c.tenant_id,c.customer_id;",
        pick: "customer_id",
        forbidden: &["order_id", "line_no"],
    },
    TutorialCase {
        id: "07",
        input: "SELECT x.customer_id,x.customer_name FROM lab.customers x\nWHERE x.tenant_id=1 AND EXISTS (\n  SELECT 1 FROM lab.orders x\n  WHERE x.tenant_id=2 AND x.order___CURSOR__=1005\n) ORDER BY x.customer_id;",
        pick: "order_id",
        forbidden: &["customer_name", "email", "credit_limit"],
    },
    TutorialCase {
        id: "08",
        input: "WITH RECURSIVE chain(employee_id,employee_name,manager_id,path,depth) AS (\n  SELECT employee_id,employee_name,manager_id,CAST(employee_id AS VARCHAR(500)),0\n  FROM lab.employees WHERE manager_id IS NULL\n  UNION ALL\n  SELECT e.employee_id,e.employee_name,e.manager_id,CAST(chain.path||'/'||CAST(e.employee_id AS VARCHAR(20)) AS VARCHAR(500)),chain.depth+1\n  FROM lab.employees e JOIN chain ON e.manager_id=chain.employee_id\n) SELECT chain.employee_id,chain.employee_name,chain.pa__CURSOR__,chain.depth\nFROM chain ORDER BY chain.employee_id;",
        pick: "path",
        forbidden: &["salary", "department_id"],
    },
    TutorialCase {
        id: "09",
        input: "WITH ranked AS (\n  SELECT e.employee_id,e.department_id,e.salary,\n         DENSE_RANK() OVER(PARTITION BY e.department_id ORDER BY e.salary DESC) AS salary_rank,\n         ROW_NUMBER() OVER(PARTITION BY e.department_id ORDER BY e.salary DESC,e.employee_id) AS row_num,\n         SUM(e.salary) OVER(PARTITION BY e.department_id ORDER BY e.salary DESC,e.employee_id\n                            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running_pay\n  FROM lab.employees e WHERE e.department_id IS NOT NULL\n) SELECT r.employee_id,r.department_id,r.salary_r__CURSOR__,r.row_num,r.running_pay\nFROM ranked r ORDER BY r.department_id,r.row_num;",
        pick: "salary_rank",
        forbidden: &["manager_id", "employee_name"],
    },
    TutorialCase {
        id: "10",
        input: "SELECT u.tenant_id,u.entity___CURSOR__,u.label\nFROM (\n  SELECT c.tenant_id,c.customer_id AS entity_id,c.customer_name AS label\n  FROM lab.customers c WHERE c.tenant_id=1 AND c.customer_id=103\n  UNION ALL\n  SELECT o.tenant_id,o.order_id,o.note\n  FROM lab.orders o WHERE o.tenant_id=1 AND o.customer_id IS NULL\n) u ORDER BY u.entity_id;",
        pick: "entity_id",
        forbidden: &["customer_id", "order_id", "note"],
    },
    TutorialCase {
        id: "11",
        input: "SELECT c.tenant_id,c.customer_id,c.customer_name AS current_name,\n       a.archived_name AS historical_name,a.archive_r__CURSOR__\nFROM lab.customers c LEFT JOIN archive.customers a\n  ON a.tenant_id=c.tenant_id AND a.customer_id=c.customer_id\nUNION ALL\nSELECT a.tenant_id,a.customer_id,NULL,a.archived_name,a.archive_reason\nFROM archive.customers a WHERE NOT EXISTS (\n  SELECT 1 FROM lab.customers c WHERE c.tenant_id=a.tenant_id AND c.customer_id=a.customer_id\n) ORDER BY tenant_id,customer_id;",
        pick: "archive_reason",
        forbidden: &["customer_name", "email", "credit_limit"],
    },
    TutorialCase {
        id: "12",
        input: "SELECT d.\"Row ID\" AS row_id,d.\"客户__CURSOR__\" AS customer_name,\n       d.\"select\" AS reserved_value,d.\"MixedCase\" AS mixed_value,\n       ROUND(d.\"unit-price\"*2,2) AS doubled\nFROM lab.\"Order Details\" d ORDER BY d.\"Row ID\";",
        pick: "客户名称",
        forbidden: &["Row ID", "select", "MixedCase", "unit-price"],
    },
    TutorialCase {
        id: "13",
        input: "SELECT c.tenant_id,c.customer_id,c.customer_name,recent.latest___CURSOR__\nFROM lab.customers c LEFT JOIN LATERAL (\n  SELECT o.total_amount AS latest_amount FROM lab.orders o\n  WHERE o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id\n  ORDER BY o.ordered_at DESC,o.order_id DESC LIMIT 1\n) recent ON TRUE\nORDER BY c.tenant_id,c.customer_id;",
        pick: "latest_amount",
        forbidden: &["total_amount", "ordered_at", "customer_id"],
    },
    TutorialCase {
        id: "14",
        input: "WITH original AS (SELECT c.* FROM lab.customers c),\ntrimmed AS (SELECT tenant_id,customer_id,customer_name FROM original)\nSELECT t.customer_n__CURSOR__ FROM trimmed t\nWHERE t.tenant_id=1 AND t.customer_id=103;",
        pick: "customer_name",
        forbidden: &["email", "region", "credit_limit", "created_at"],
    },
    TutorialCase {
        id: "15",
        input: "WITH measures AS (\n  SELECT metric_id,w.metric_48 AS last_metric,w.metric_01 AS first_metric\n  FROM lab.wide_metrics w\n) SELECT m.last_m__CURSOR__,m.last_metric-m.first_metric AS gap\nFROM measures m WHERE m.metric_id=1;",
        pick: "last_metric",
        forbidden: &["metric_02", "metric_47", "metric_48"],
    },
    TutorialCase {
        id: "16",
        input: "SELECT e.event_id,j.val__CURSOR__ AS tag\nFROM lab.audit_events e\nCROSS JOIN LATERAL jsonb_array_elements_text(e.payload->'tags') AS j(value)\nWHERE e.event_id=1 ORDER BY j.value;",
        pick: "value",
        forbidden: &["payload", "entity_id"],
    },
    TutorialCase {
        id: "17",
        input: "SELECT o.order_id,lab.lab_dis__CURSOR__(o.total_amount) AS default_discount,\n  lab.lab_discount(o.total_amount,0.2) AS custom_discount\nFROM lab.orders o WHERE o.tenant_id=1 AND o.order_id=1002;",
        pick: "lab_discount",
        forbidden: &[],
    },
    TutorialCase {
        id: "18",
        input: "SELECT DISTINCT ON (o.tenant_id,o.cust__CURSOR__) o.tenant_id,o.customer_id,o.order_id,o.total_amount\nFROM lab.orders o WHERE o.customer_id IS NOT NULL\nORDER BY o.tenant_id,o.customer_id,o.ordered_at DESC,o.order_id DESC;",
        pick: "customer_id",
        forbidden: &["customer_name"],
    },
    TutorialCase {
        id: "19",
        input: "INSERT INTO lab.customers (tenant_id,customer_id,customer_name,em__CURSOR__,created_at)\nSELECT 2,901,'Completion Insert',NULL,CURRENT_TIMESTAMP WHERE 1=0;",
        pick: "email",
        forbidden: &["tenant_id", "customer_id", "customer_name", "created_at"],
    },
    TutorialCase {
        id: "20",
        input: "UPDATE lab.customers SET customer_name=customer_name, credit___CURSOR__=credit_limit\nWHERE tenant_id=1 AND customer_id=101 AND 1=0;",
        pick: "credit_limit",
        forbidden: &["customer_name"],
    },
    TutorialCase {
        id: "21",
        input: "SELECT 'cust__CURSOR__omers' AS sample;",
        pick: "",
        forbidden: &["customers"],
    },
];

#[test]
fn postgres_sql_completion_tutorial() {
    let Some(config) = smoke_config() else {
        eprintln!("skip: FLUXDB_PG_COMPLETION not set");
        return;
    };
    let connector = connector_for(&config).expect("connector");
    connector
        .test_connection(&config)
        .expect("postgres connection");
    let mut controller = AppController::new();
    let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));

    let only = std::env::var("FLUXDB_PG_COMPLETION_CASE").ok();
    let mut failures = Vec::new();
    for case in CASES {
        if only.as_deref().is_some_and(|only| only != case.id) {
            continue;
        }
        let result = complete(&controller, config.id, case.input);
        let actual_labels = labels(&result);
        eprintln!("case {}: labels={actual_labels:?}", case.id);
        if case.pick.is_empty() {
            if !result.items.is_empty() {
                failures.push(format!(
                    "case {}: 字符串内不应出现 SQL 候选，实际 {actual_labels:?}",
                    case.id
                ));
            }
            let adopted = case.input.replace(MARKER, "");
            if adopted != "SELECT 'customers' AS sample;" {
                failures.push(format!("case {}: 原文被改写：{adopted}", case.id));
            }
            continue;
        }

        if find(&result, case.pick).is_none() {
            failures.push(format!(
                "case {}: 缺少候选 {}，实际 {actual_labels:?}",
                case.id, case.pick
            ));
            continue;
        }
        for forbidden in case.forbidden {
            if find(&result, forbidden).is_some() {
                failures.push(format!(
                    "case {}: 不应出现候选 {}，实际 {actual_labels:?}",
                    case.id, forbidden
                ));
            }
        }

        let adopted = apply(case.input, &result, case.pick);
        eprintln!("case {}: adopted={adopted:?}", case.id);
        if let Some(rows) = expected_rows(case.id) {
            let actual = execute(connector.as_ref(), config.id, &adopted);
            if let Err(error) = check_rows(actual, rows) {
                failures.push(format!("case {}: {error}", case.id));
            }
        } else if !adopted.to_uppercase().contains(&case.pick.to_uppercase()) {
            // 19/20 只核对补完文字，不执行写入。
            failures.push(format!(
                "case {}: 采纳后文本缺少 {}：{adopted}",
                case.id, case.pick
            ));
        }
    }
    // 教程 21 的补充负例：行注释里同样不应弹 SQL 候选。
    if only.as_deref().is_none_or(|only| only == "21") {
        let result = complete(&controller, config.id, "-- cust__CURSOR__");
        if !result.items.is_empty() {
            failures.push(format!(
                "case 21: 注释内不应出现 SQL 候选，实际 {:?}",
                labels(&result)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "PostgreSQL SQL 补全失败用例（{}）:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
