// MySQL SQL 补全端到端验证：沿用 docs/testing/sql-completion/mysql/测试教程.md
// 的 21 个用例，只验证「Ctrl+Space 后候选能不能出来」。
//
// 驱动的是真实生产视图（NavicatMain + 真实 Editor + SqlAdapter + 真实 MySQL 连接），
// 但用无头 TestAppContext 注入按键，因此不覆盖系统级快捷键拦截、IME 与像素渲染。
// 依赖本机已启动的 unit-mysql（127.0.0.1:3306，root/o9i8u7y6）与教程种子数据。
// 跑完全部用例后统一断言，失败清单保留用例编号、实际候选与预期。

struct TutorialCase {
    id: &'static str,
    name: &'static str,
    /// 原始输入，`__CURSOR__` 标记补全触发位置。
    input: &'static str,
    /// 期望出现的候选；None 表示此处不应弹出任何候选。
    expected: Option<&'static str>,
}

const MYSQL_COMPLETION_CASES: &[TutorialCase] = &[
    TutorialCase {
        id: "01",
        name: "从表名前缀补出 customers",
        input: "SELECT c.customer_name FROM completion_lab.cust__CURSOR__ c\nWHERE c.tenant_id=1 AND c.customer_id=103;",
        expected: Some("customers"),
    },
    TutorialCase {
        id: "02",
        name: "从别名 c. 补客户字段",
        input: "SELECT c.__CURSOR__ FROM completion_lab.customers c\nWHERE c.tenant_id=1 ORDER BY c.customer_id;",
        expected: Some("customer_name"),
    },
    TutorialCase {
        id: "03",
        name: "光标在词中只替换整个字段名",
        input: "-- 中文注释：后方语句不应发生坐标漂移\nSELECT '前缀中文' AS marker,\n  c.cust__CURSOR__omer_name AS chosen\nFROM completion_lab.customers c WHERE c.tenant_id=1 AND c.customer_id=103;",
        expected: Some("customer_name"),
    },
    TutorialCase {
        id: "04",
        name: "五表关系 + 两个预聚合 CTE",
        input: "WITH item_totals AS (\n  SELECT tenant_id, order_id, SUM(quantity*unit_price-discount) AS item_total\n  FROM completion_lab.order_items GROUP BY tenant_id, order_id\n), payment_totals AS (\n  SELECT tenant_id, order_id, SUM(amount) AS paid_total\n  FROM completion_lab.payments GROUP BY tenant_id, order_id\n)\nSELECT o.tenant_id, o.order_id, c.customer_name,\n       COALESCE(it.it__CURSOR__,0) AS item_total,\n       COALESCE(pt.paid_total,0) AS paid_total,\n       o.total_amount-COALESCE(pt.paid_total,0) AS outstanding\nFROM completion_lab.orders o\nLEFT JOIN completion_lab.customers c\n  ON o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id\nLEFT JOIN item_totals it ON it.tenant_id=o.tenant_id AND it.order_id=o.order_id\nLEFT JOIN payment_totals pt ON pt.tenant_id=o.tenant_id AND pt.order_id=o.order_id\nWHERE o.status<>'cancelled'\nORDER BY o.tenant_id,o.order_id;",
        expected: Some("item_total"),
    },
    TutorialCase {
        id: "05",
        name: "三级 CTE + 显式列重命名 + 每租户 Top 1",
        input: "WITH base(tid,cid,amount) AS (\n  SELECT tenant_id,customer_id,total_amount FROM completion_lab.orders WHERE customer_id IS NOT NULL\n), totals AS (\n  SELECT tid,cid,SUM(amount) AS total_spent FROM base GROUP BY tid,cid\n), ranked AS (\n  SELECT tid,cid,total_spent,\n         ROW_NUMBER() OVER(PARTITION BY tid ORDER BY total_spent DESC,cid) AS tenant_rank\n  FROM totals\n)\nSELECT r.tid,r.cid,c.customer_name,r.total_s__CURSOR__\nFROM ranked r JOIN completion_lab.customers c ON c.tenant_id=r.tid AND c.customer_id=r.cid\nWHERE r.tenant_rank=1 ORDER BY r.tid;",
        expected: Some("total_spent"),
    },
    TutorialCase {
        id: "06",
        name: "四层相关 EXISTS 引用三个外层关系",
        input: "SELECT c.tenant_id,c.customer_id,c.customer_name FROM completion_lab.customers c\nWHERE EXISTS (\n  SELECT 1 FROM completion_lab.orders o\n  WHERE o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id AND o.status='paid'\n    AND EXISTS (\n      SELECT 1 FROM completion_lab.order_items i\n      WHERE i.tenant_id=o.tenant_id AND i.order_id=o.order_id\n        AND EXISTS (SELECT 1 FROM completion_lab.products p\n                    WHERE p.product_id=i.product_id AND p.unit_price>=300\n                      AND o.tenant_id=c.tenant_id AND o.customer_id=c.cust__CURSOR__)\n    )\n) ORDER BY c.tenant_id,c.customer_id;",
        expected: Some("customer_id"),
    },
    TutorialCase {
        id: "07",
        name: "同名 x 别名内层遮蔽",
        input: "SELECT x.customer_id,x.customer_name FROM completion_lab.customers x\nWHERE x.tenant_id=1 AND EXISTS (\n  SELECT 1 FROM completion_lab.orders x\n  WHERE x.tenant_id=2 AND x.order___CURSOR__=1005\n) ORDER BY x.customer_id;",
        expected: Some("order_id"),
    },
    TutorialCase {
        id: "08",
        name: "递归员工路径",
        input: "WITH RECURSIVE chain(employee_id,employee_name,manager_id,path,depth) AS (\n  SELECT employee_id,employee_name,manager_id,CAST(employee_id AS CHAR(500)),0\n  FROM completion_lab.employees WHERE manager_id IS NULL\n  UNION ALL\n  SELECT e.employee_id,e.employee_name,e.manager_id,CAST(CONCAT(chain.path,'/',CAST(e.employee_id AS CHAR)) AS CHAR(500)),chain.depth+1\n  FROM completion_lab.employees e JOIN chain ON e.manager_id=chain.employee_id\n) SELECT chain.employee_id,chain.employee_name,chain.pa__CURSOR__,chain.depth\nFROM chain ORDER BY chain.employee_id;",
        expected: Some("path"),
    },
    TutorialCase {
        id: "09",
        name: "窗口帧 + 并列排名 + 外层派生列",
        input: "WITH ranked AS (\n  SELECT e.employee_id,e.department_id,e.salary,\n         DENSE_RANK() OVER(PARTITION BY e.department_id ORDER BY e.salary DESC) AS salary_rank,\n         ROW_NUMBER() OVER(PARTITION BY e.department_id ORDER BY e.salary DESC,e.employee_id) AS row_num,\n         SUM(e.salary) OVER(PARTITION BY e.department_id ORDER BY e.salary DESC,e.employee_id\n                            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running_pay\n  FROM completion_lab.employees e WHERE e.department_id IS NOT NULL\n) SELECT r.employee_id,r.department_id,r.salary_r__CURSOR__,r.row_num,r.running_pay\nFROM ranked r ORDER BY r.department_id,r.row_num;",
        expected: Some("salary_rank"),
    },
    TutorialCase {
        id: "10",
        name: "UNION ALL 各分支隔离 + 首分支定义输出列名",
        input: "SELECT u.tenant_id,u.entity___CURSOR__,u.label\nFROM (\n  SELECT c.tenant_id,c.customer_id AS entity_id,c.customer_name AS label\n  FROM completion_lab.customers c WHERE c.tenant_id=1 AND c.customer_id=103\n  UNION ALL\n  SELECT o.tenant_id,o.order_id,o.note\n  FROM completion_lab.orders o WHERE o.tenant_id=1 AND o.customer_id IS NULL\n) u ORDER BY u.entity_id;",
        expected: Some("entity_id"),
    },
    TutorialCase {
        id: "11",
        name: "跨空间同名表 + 模拟 FULL JOIN",
        input: "SELECT c.tenant_id,c.customer_id,c.customer_name AS current_name,\n       a.archived_name AS historical_name,a.archive_r__CURSOR__\nFROM completion_lab.customers c LEFT JOIN completion_archive.customers a\n  ON a.tenant_id=c.tenant_id AND a.customer_id=c.customer_id\nUNION ALL\nSELECT a.tenant_id,a.customer_id,NULL,a.archived_name,a.archive_reason\nFROM completion_archive.customers a WHERE NOT EXISTS (\n  SELECT 1 FROM completion_lab.customers c WHERE c.tenant_id=a.tenant_id AND c.customer_id=a.customer_id\n) ORDER BY tenant_id,customer_id;",
        expected: Some("archive_reason"),
    },
    TutorialCase {
        id: "12",
        name: "中文 + 保留字 + 大小写 + 连字符",
        input: "SELECT d.`Row ID` AS row_id,d.`客户__CURSOR__` AS customer_name,\n       d.`select` AS reserved_value,d.`MixedCase` AS mixed_value,\n       ROUND(d.`unit-price`*2,2) AS doubled\nFROM completion_lab.`Order Details` d ORDER BY d.`Row ID`;",
        expected: Some("客户名称"),
    },
    TutorialCase {
        id: "13",
        name: "LATERAL / OUTER APPLY 逐客户取最近订单",
        input: "SELECT c.tenant_id,c.customer_id,c.customer_name,recent.latest___CURSOR__\nFROM completion_lab.customers c LEFT JOIN LATERAL (\n  SELECT o.total_amount AS latest_amount FROM completion_lab.orders o\n  WHERE o.tenant_id=c.tenant_id AND o.customer_id=c.customer_id\n  ORDER BY o.ordered_at DESC,o.order_id DESC LIMIT 1\n) recent ON TRUE\nORDER BY c.tenant_id,c.customer_id;",
        expected: Some("latest_amount"),
    },
    TutorialCase {
        id: "14",
        name: "SELECT * 经 CTE 展开后再次裁剪",
        input: "WITH original AS (SELECT c.* FROM completion_lab.customers c),\ntrimmed AS (SELECT tenant_id,customer_id,customer_name FROM original)\nSELECT t.customer_n__CURSOR__ FROM trimmed t\nWHERE t.tenant_id=1 AND t.customer_id=103;",
        expected: Some("customer_name"),
    },
    TutorialCase {
        id: "15",
        name: "宽表末列经 CTE 改名后参与表达式",
        input: "WITH measures AS (\n  SELECT metric_id,w.metric_48 AS last_metric,w.metric_01 AS first_metric\n  FROM completion_lab.wide_metrics w\n) SELECT m.last_m__CURSOR__,m.last_metric-m.first_metric AS gap\nFROM measures m WHERE m.metric_id=1;",
        expected: Some("last_metric"),
    },
    TutorialCase {
        id: "16",
        name: "MySQL JSON_TABLE 数组展开与派生列",
        input: "SELECT e.event_id,jt.ta__CURSOR__ FROM completion_lab.audit_events e\nJOIN JSON_TABLE(e.payload, '$.tags[*]' COLUMNS (tag VARCHAR(30) PATH '$')) AS jt ON TRUE\nWHERE e.event_id=1 ORDER BY jt.tag;",
        expected: Some("tag"),
    },
    TutorialCase {
        id: "17",
        name: "MySQL 跨库限定函数与字段参数",
        input: "SELECT o.order_id,completion_lab.lab_dis__CURSOR__(o.total_amount) AS discounted,\n  IFNULL(o.note,'EMPTY') AS note_text\nFROM completion_lab.orders o WHERE o.tenant_id=1 AND o.order_id=1002;",
        expected: Some("lab_discount"),
    },
    TutorialCase {
        id: "18",
        name: "MySQL 命名 WINDOW + 帧继承",
        input: "WITH running AS (\n  SELECT o.tenant_id,o.order_id,\n    SUM(o.total_amount) OVER w AS running_amount\n  FROM completion_lab.orders o WHERE o.tenant_id=1\n  WINDOW w AS (PARTITION BY o.tenant_id ORDER BY o.ordered___CURSOR__,o.order_id\n               ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)\n) SELECT r.order_id,r.running_amount FROM running r ORDER BY r.order_id;",
        expected: Some("ordered_at"),
    },
    TutorialCase {
        id: "19",
        name: "INSERT 列清单去重",
        input: "INSERT INTO completion_lab.customers (tenant_id,customer_id,customer_name,em__CURSOR__,created_at)\nSELECT 2,901,'Completion Insert',NULL,CURRENT_TIMESTAMP WHERE 1=0;",
        expected: Some("email"),
    },
    TutorialCase {
        id: "20",
        name: "UPDATE 赋值列去重",
        input: "UPDATE completion_lab.customers SET customer_name=customer_name, credit___CURSOR__=credit_limit\nWHERE tenant_id=1 AND customer_id=101 AND 1=0;",
        expected: Some("credit_limit"),
    },
    TutorialCase {
        id: "21",
        name: "字符串里面不弹 SQL 补全",
        input: "SELECT 'cust__CURSOR__omers' AS sample;",
        expected: None,
    },
];

fn mysql_case_text_and_cursor(case: &TutorialCase) -> (String, usize) {
    let marker = case.input.find("__CURSOR__").expect("case marker");
    let mut text = case.input.to_string();
    text.replace_range(marker..marker + "__CURSOR__".len(), "");
    (text, marker)
}

fn normalize_completion_label(label: &str) -> String {
    label
        .trim_matches(|ch| matches!(ch, '`' | '"' | '[' | ']'))
        .to_string()
}

fn mysql_completion_connection() -> (fluxdb_core::ConnectionConfig, fluxdb_core::ConnectionId) {
    use fluxdb_core::{
        ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MysqlConnectionProfile,
    };

    let mut options = std::collections::BTreeMap::new();
    options.insert("host".to_string(), "127.0.0.1".to_string());
    options.insert("port".to_string(), "3306".to_string());
    options.insert("database".to_string(), "completion_lab".to_string());
    options.insert("username".to_string(), "root".to_string());
    options.insert("password".to_string(), "o9i8u7y6".to_string());
    let id = ConnectionId(91);
    (
        ConnectionConfig {
            id,
            name: "MySQL completion E2E".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 3306,
                database: Some("completion_lab".into()),
            },
            options: options.clone(),
            redis_profile: None,
            mysql_profile: Some(MysqlConnectionProfile::from_options(&options)),
            postgres_profile: None,
            dameng_profile: None,
            mongo_profile: None,
            sqlserver_profile: None,
            oracle_profile: None,
        },
        id,
    )
}

/// 诊断用：绕过编辑器/浮层，直接看 app 补全引擎对同一批输入的候选。
/// 用于区分「补全引擎没结果」与「编辑器/浮层没显示」。
#[test]
fn mysql_sql_completion_engine_diagnostic() {
    use fluxdb_app::{AppCommand, AppController};

    let (config, connection_id) = mysql_completion_connection();
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
    for case in MYSQL_COMPLETION_CASES {
        let (text, cursor) = mysql_case_text_and_cursor(case);
        let result = controller.query_completions_for_text(
            connection_id,
            Some("completion_lab".into()),
            None,
            text,
            cursor,
            true,
        );
        match result {
            Ok(result) => eprintln!(
                "case {} labels={:?}",
                case.id,
                result
                    .items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>()
            ),
            Err(error) => eprintln!("case {} error={error}", case.id),
        }
    }
}

#[gpui::test]
fn mysql_sql_completion_candidates_appear(cx: &mut TestAppContext) {
    use fluxdb_app::{AppCommand, AppEvent};
    use gpui_component::{Root, ThemeMode};

    let _guard = locale_guard();
    cx.background_executor.allow_parking();
    let temp = tempfile::tempdir().expect("isolated profile");
    let storage = FileStorage::new(temp.path().join("profile"));
    let settings = Settings {
        locale: "en".into(),
        show_sidebar: false,
        ..Default::default()
    };
    storage.save_settings(&settings).expect("settings");
    init_i18n(
        &app_assets_base_path().join("locales"),
        &storage.root_dir().join("locales"),
        "en",
    );

    let (config, connection_id) = mysql_completion_connection();
    let mut controller = AppController::new();
    controller.set_ai_storage(storage.clone());
    controller.dispatch(AppCommand::SaveSettings(settings));
    controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
    let AppEvent::TabOpened(tab_id) = controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
        connection_id,
        database: Some("completion_lab".into()),
        schema: None,
    }) else {
        panic!("query tab");
    };

    cx.update(|cx| {
        extend_component_i18n();
        gpui_component::init(cx);
        crate::editor_component::register_editor_shortcuts(cx);
    });
    let profile = storage.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            navicat_main_new(window, cx, controller, profile, ThemeMode::Light)
        });
        Root::new(view, window, cx)
    });
    cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(900.)));
    let view = cx.update(|window, cx| {
        window
            .root::<Root>()
            .expect("root")
            .expect("typed root")
            .read(cx)
            .view()
            .clone()
            .downcast::<NavicatMain>()
            .expect("production view")
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let editor = cx.update(|_, cx| {
        view.read(cx)
            .query_editors
            .get(&tab_id)
            .expect("query editor")
            .clone()
    });

    let mut failures = Vec::new();
    for case in MYSQL_COMPLETION_CASES {
        let (text, cursor) = mysql_case_text_and_cursor(case);
        // 走生产路径改查询模型文本；直接改编辑器缓冲会在下一次渲染时被模型覆盖。
        cx.update(|_, cx| {
            view.update(cx, |app, cx| {
                app.dispatch(
                    AppCommand::UpdateQueryText {
                        tab_id,
                        text: text.clone(),
                    },
                    cx,
                );
            });
        });
        // 画一帧让 query_editor_state 把模型文本同步进编辑器；随后再落光标，
        // 避免 sync_text_silent 把选区重置到 0。
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        editor.update(cx, |editor, cx| {
            editor.select_range(fluxdb_editor_core::Range::new(cursor, cursor), cx);
        });
        let focus = cx.update(|_, cx| editor.read(cx).focus_handle.clone());
        cx.update(|window, cx| window.focus(&focus, cx));
        cx.simulate_keystrokes("ctrl-space");

        let (visible, labels, actual_cursor, actual_text) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.completion_visible,
                editor
                    .completion_items
                    .iter()
                    .map(|item| item.label.clone())
                    .collect::<Vec<_>>(),
                editor.cursor_offset(),
                editor.text(),
            )
        });
        let matched = case.expected.is_none_or(|expected| {
            visible
                && labels
                    .iter()
                    .any(|label| normalize_completion_label(label).contains(expected))
        });
        let prefix = actual_text
            .get(actual_cursor.saturating_sub(20)..actual_cursor)
            .unwrap_or_default();
        eprintln!(
            "UI case {} {}: matched={matched} visible={visible} cursor={actual_cursor} prefix={prefix:?} labels={labels:?}",
            case.id, case.name
        );
        if !matched {
            failures.push(format!(
                "测试 {} {}\n预期: {}\n实际可见: {visible}\n实际光标: {actual_cursor}，光标前文本: {prefix:?}\n实际候选: {labels:?}",
                case.id,
                case.name,
                case.expected.unwrap_or("无候选"),
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "MySQL SQL 补全失败用例（{} / {}）:\n{}",
        failures.len(),
        MYSQL_COMPLETION_CASES.len(),
        failures.join("\n")
    );
}
