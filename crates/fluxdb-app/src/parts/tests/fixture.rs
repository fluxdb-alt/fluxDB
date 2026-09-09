// T081 四项目统一 completion fixture + 采集器。
//
// 场景集覆盖：语句头关键字、FROM/JOIN 表源、限定/别名列、WHERE/ORDER/操作符、
// 星号展开、FK JOIN 建议、函数/触发器、INSERT/UPDATE、schema、替换区间与插入文本、
// 引用标识符、别名遮蔽与相关子查询（内层不泄漏）、语义 must-not-contain。
//
// 采集器 `completion_fixture_collect` 对每个场景跑完整 App 补全管线，累计
// Top-1 / Top-3 / Top-5 命中、替换区间与插入文本正确性、错误率，输出可复现报告，
/// 一个补全 fixture 场景的期望（部分场景可组合多条断言）。
struct FixtureWant {
    /// 期望 Top-1 命中的候选。命中即置 top1 命中。
    top1: Option<(QueryCompletionKind, &'static str)>,
    /// 期望在 top 前 K 位内命中（含 Top-1），用于前缀可能命中多个的宽松锚点。
    in_top: Option<(QueryCompletionKind, &'static str, usize)>,
    /// 语义上必须不出现的候选（负例，防泄漏/防噪音）。
    absent: Vec<(QueryCompletionKind, &'static str)>,
    /// 期望替换区间 [start, end)。
    replace: Option<(usize, usize)>,
    /// 期望 Top-1 候选的插入文本（P1.5 替换正确性）。
    insert: Option<&'static str>,
}

struct CompletionScenario {
    name: &'static str,
    sql: &'static str,
    cursor: Option<usize>,
    want: FixtureWant,
}

impl CompletionScenario {
    fn new(name: &'static str, sql: &'static str, want: FixtureWant) -> Self {
        Self {
            name,
            sql,
            cursor: None,
            want,
        }
    }
    fn cursor(mut self, cursor: usize) -> Self {
        self.cursor = Some(cursor);
        self
    }
}

/// 完整的补全管线结果（含替换区间），供 fixture 采集器复用。
fn matrix_request_result(sql: &str, cursor: Option<usize>) -> QueryCompletionResult {
    let mut controller = AppController::with_mock_data();
    controller.dispatch(AppCommand::WarmCompletionIndex {
        connection_id: ConnectionId(1),
        database: Some("main".to_string()),
    });
    controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
        connection_id: ConnectionId(1),
        database: Some("main".to_string()),
    });
    controller.dispatch(AppCommand::UpdateQueryText {
        tab_id: TabId(1),
        text: sql.to_string(),
    });
    let event = controller.dispatch(AppCommand::RequestQueryCompletions {
        tab_id: TabId(1),
        request_seq: 1,
        cursor: cursor.unwrap_or_else(|| sql.len()),
        explicit: false,
    });
    let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
        panic!("expected query completions for: {sql:?}");
    };
    result
}

// ---------- 场景集 ----------

fn completion_scenarios() -> Vec<CompletionScenario> {
    use QueryCompletionKind::*;
    let top = |kind, label| FixtureWant {
        top1: Some((kind, label)),
        in_top: None,
        absent: Vec::new(),
        replace: None,
        insert: None,
    };
    let absent = |top1: Option<(QueryCompletionKind, &'static str)>, absent: Vec<(QueryCompletionKind, &'static str)>| {
        FixtureWant {
            top1,
            in_top: None,
            absent,
            replace: None,
            insert: None,
        }
    };

    vec![
        // ---- 语句头关键字与 snippet ----
        CompletionScenario::new("语句头 sel", "sel", top(Keyword, "SELECT")),
        // 语句头：`ins/dele` 下实际候选为带模板的 `INSERT INTO` / `DELETE FROM` 关键字。
        CompletionScenario::new(
            "语句头 ins",
            "ins",
            FixtureWant {
                top1: None,
                in_top: Some((Keyword, "INSERT INTO", 4)),
                absent: vec![(Column, "id"), (Table, "Order")],
                replace: None,
                insert: None,
            },
        ),
        CompletionScenario::new("语句头 upd", "upd", top(Keyword, "UPDATE")),
        CompletionScenario::new(
            "语句头 del",
            "dele",
            FixtureWant {
                top1: None,
                in_top: Some((Keyword, "DELETE FROM", 4)),
                absent: vec![(Column, "id"), (Table, "Order")],
                replace: None,
                insert: None,
            },
        ),
        CompletionScenario::new("语句头 DROP", "drop", top(Keyword, "DROP")),
        // ---- FROM 表源 + schema ----
        CompletionScenario::new(
            "FROM 表源含 schema",
            "select * from ",
            absent(
                None,
                vec![
                    (Column, "id"),
                    (Column, "name"),
                    (Function, "normalize_price"),
                ],
            ),
        ),
        CompletionScenario::new("FROM 前缀 Pr", "select * from Pr", top(Table, "Product")),
        CompletionScenario::new("FROM 前缀 Cu", "select * from Cu", top(Table, "Customer")),
        CompletionScenario::new(
            "FROM 精确 Order",
            "select * from Order ",
            absent(None, vec![(Column, "product_id")]),
        ),
        // ---- 限定 / 别名列 ----
        // 光标在点号之后（T082：cursor 取点号后一字节），限定表只给该表列候选；
        // T082 修复前 cursor 落在点号字节上导致 qualifier 未识别，关键字/函数
        // （PRAGMA/OFFSET/UPPER 等）漏入候选集，故这里用 absent 负例锁定「无噪音」。
        CompletionScenario::new(
            "限定 p. 列",
            "select p. from Product p",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "category_id", 20)),
                absent: vec![
                    (Column, "sort_order"),
                    (Column, "email"),
                    (Keyword, "PRAGMA"),
                    (Keyword, "OFFSET"),
                    (Function, "UPPER"),
                ],
                replace: None,
                insert: None,
            },
        )
        .cursor(9),
        CompletionScenario::new(
            "限定 Order 列",
            "select o. from Order o",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "customer_id", 40)),
                absent: vec![
                    (Column, "sort_order"),
                    (Column, "city"),
                    (Column, "price"),
                    (Keyword, "DROP"),
                ],
                replace: None,
                insert: None,
            },
        )
        .cursor(9),
        // ---- 派生表别名列（T082）----
        // `(subquery) t` 的 `t.` 应解析为子查询投影列（id/name），而非底层表 Product
        // 的全部列；T082 前派生别名列未解析，`t.` 落入底层表 metadata。
        CompletionScenario::new(
            "派生表 t.i 列",
            "select * from (select id, name from Product) t where t.i",
            top(Column, "id"),
        ),
        CompletionScenario::new(
            "派生表 t. 不泄漏底层表列",
            "select * from (select id, name from Product) t where t.",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "id", 2)),
                absent: vec![
                    (Column, "active"),
                    (Column, "price"),
                    (Column, "category_id"),
                    (Column, "created_at"),
                ],
                replace: None,
                insert: None,
            },
        ),
        // ---- WHERE/值/操作符 ----
        // 比较操作符候选集中，== 与 != 均为正确结果，Top-1 按确定性排序取 !=。
        CompletionScenario::new(
            "WHERE id 操作符",
            "select * from Product where id ",
            top(Keyword, "!="),
        ),
        // 值语境布尔/空值关键字候选，Top-1 按字母序取 FALSE；NULL 应在 Top-5 内。
        CompletionScenario::new(
            "WHERE name 值关键字",
            "select * from Product where name = ",
            FixtureWant {
                top1: None,
                in_top: Some((Keyword, "NULL", 5)),
                absent: vec![(Table, "Product")],
                replace: None,
                insert: None,
            },
        ),
        CompletionScenario::new(
            "ORDER BY na",
            "select * from Product order by na",
            top(Column, "name"),
        ),
        // ---- 星号展开 ----
        // 星号展开为 snippet 候选；光标须位于 `*` 之后，此处锁定 snippet 插入文本。
        CompletionScenario::new(
            "限定星号 c.* 展开 ProductCategory",
            "select c.* from Product p join ProductCategory c on p.id = c.id",
            FixtureWant {
                top1: None,
                in_top: Some((Snippet, "展开所有列 (*)", 90)),
                absent: Vec::new(),
                replace: None,
                insert: Some("c.id, c.name, c.sort_order"),
            },
        )
        .cursor(10),
        // ---- FK JOIN 建议（P2.13）----
        // `Product join ` 后应给出基于 Product.category_id 的外键 JOIN 建议。
        CompletionScenario::new(
            "FK JOIN 建议 Product→ProductCategory",
            "select * from Product join ",
            FixtureWant {
                top1: None,
                in_top: Some((Snippet, "JOIN ProductCategory ON", 20)),
                absent: vec![(Column, "category_id")],
                replace: None,
                insert: Some("JOIN ProductCategory ON Product.category_id = ProductCategory.id"),
            },
        ),
        // `Order join ` 触发两个独立外键的 JOIN 建议（Product 与 Customer）。
        CompletionScenario::new(
            "FK JOIN 双外键 Order",
            "select * from Order join ",
            FixtureWant {
                top1: None,
                in_top: Some((Snippet, "JOIN Product ON", 20)),
                absent: vec![(Column, "product_id")],
                replace: None,
                insert: None,
            },
        ),
        // ---- 相关子查询 / 作用域 ----
        // 内层 `o.` 只应给出 Order 列，不应泄漏外层 Customer 的列（相关子查询不变量）。
        CompletionScenario::new(
            "相关子查询内层仅本层列",
            "select * from Customer c where exists (select 1 from Order o where o.",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "product_id", 20)),
                absent: vec![(Column, "email"), (Column, "city")],
                replace: None,
                insert: None,
            },
        )
        .cursor("select * from Customer c where exists (select 1 from Order o where o.".len()),
        // 别名遮蔽：内层 Order 别名 o，限定列应只指向 Order，不指向外层 Customer。
        CompletionScenario::new(
            "别名遮蔽内层 o.",
            "select c.email from Customer c where exists (select 1 from Order o where o.",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "customer_id", 20)),
                absent: vec![(Column, "email"), (Column, "city")],
                replace: None,
                insert: None,
            },
        )
        .cursor("select c.email from Customer c where exists (select 1 from Order o where o.".len()),
        // ---- schema 限定 ----
        CompletionScenario::new(
            "schema 限定 main.Pr",
            "select * from main.Pr",
            top(Table, "Product"),
        ),
        // ---- 函数 / 触发器 ----
        CompletionScenario::new(
            "函数 normalize_price",
            "select norm",
            top(Function, "normalize_price"),
        ),
        CompletionScenario::new(
            "DROP TRIGGER 触发器",
            "drop trigger product_a",
            top(Trigger, "product_ai"),
        ),
        // ---- INSERT / UPDATE ----
        CompletionScenario::new(
            "INSERT 列 id",
            "insert into Product (id",
            top(Column, "id"),
        ),
        CompletionScenario::new(
            "UPDATE set na",
            "update Product set na",
            top(Column, "name"),
        ),
        // ---- 更多语句头关键字 ----
        CompletionScenario::new("语句头 CREATE", "cre", top(Keyword, "CREATE")),
        CompletionScenario::new("语句头 ALTER", "alt", top(Keyword, "ALTER")),
        CompletionScenario::new("语句头 EXPLAIN", "expl", top(Keyword, "EXPLAIN")),
        CompletionScenario::new("语句头 WITH", "wit", top(Keyword, "WITH")),
        // ---- 聚合/内置函数 ----
        CompletionScenario::new(
            "SELECT 聚合函数 count",
            "select cou",
            top(Function, "COUNT"),
        ),
        CompletionScenario::new(
            "SELECT 聚合函数 sum",
            "select s",
            FixtureWant {
                top1: None,
                in_top: Some((Function, "SUM", 25)),
                absent: vec![(Table, "Product")],
                replace: None,
                insert: None,
            },
        ),
        // ---- GROUP BY / HAVING / LIMIT ----
        CompletionScenario::new(
            "GROUP BY na",
            "select * from Product group by na",
            top(Column, "name"),
        ),
        CompletionScenario::new(
            "HAVING count",
            "select * from Product group by name having cou",
            top(Function, "COUNT"),
        ),
        CompletionScenario::new(
            "LIMIT 后占位",
            "select * from Product limit ",
            absent(None, vec![(Table, "Product"), (Column, "id")]),
        ),
        // ---- 多表 JOIN 限定列 ----
        // 多表 JOIN 下 `p.`/`c.` 应分别给出对应表的列。注意：限定列候选的空前缀
        // 下关键/函数仍会进入候选集（T082/T084 收敛），此处仅锁定目标列存在性。
        CompletionScenario::new(
            "JOIN 后 p. 限定 Product 列",
            "select p. from Product p join ProductCategory c on p.category_id = c.id",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "price", 40)),
                absent: Vec::new(),
                replace: None,
                insert: None,
            },
        )
        .cursor(8),
        CompletionScenario::new(
            "JOIN 后 c. 限定 ProductCategory 列",
            "select c. from Product p join ProductCategory c on p.category_id = c.id",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "sort_order", 40)),
                absent: Vec::new(),
                replace: None,
                insert: None,
            },
        )
        .cursor(8),
        // ---- 保留字表名 / 子查询派生表 ----
        // 已知缺口（T082 处理）：`) t where t.` 派生表别名的列未解析（当前仅返表源），
        // 故此处不设正例；INSERT columns 上下文会把整张表全列列出（不过滤前缀）。
        // ---- 别名遮蔽顶层 ----
        CompletionScenario::new(
            "外层别名遮蔽前缀列",
            "select c. from Customer c",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "city", 40)),
                absent: vec![(Column, "sort_order"), (Column, "price")],
                replace: None,
                insert: None,
            },
        )
        .cursor(8),
        // ---- 替换区间（续）----
        CompletionScenario::new(
            "替换区间仅覆盖表前缀",
            "select * from Pro",
            FixtureWant {
                top1: Some((Table, "Product")),
                in_top: None,
                absent: Vec::new(),
                replace: Some(("select * from ".len(), "select * from Pro".len())),
                insert: None,
            },
        ),
        CompletionScenario::new(
            "替换区间覆盖限定列前缀",
            "select p.pri from Product p",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "price", 40)),
                absent: Vec::new(),
                replace: Some(("select p.".len(), "select p.pri".len())),
                insert: None,
            },
        )
        .cursor(12),
        // ---- 保留字表名引用 ----
        // INSERT 列上下文为整表全列（不过滤前缀）；语义判定：只含 Order 列、不含
        // Customer.email 等其它表列。
        CompletionScenario::new(
            "保留字 Order INSERT 列",
            "insert into Order (cust",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "customer_id", 40)),
                absent: vec![(Column, "email"), (Column, "city")],
                replace: None,
                insert: None,
            },
        ),
        // ---- 各表列前缀压力覆盖 ----
        CompletionScenario::new(
            "WHERE pr→price",
            "select * from Product where pr",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "price", 5)),
                absent: vec![(Table, "Product"), (Table, "Customer")],
                replace: None,
                insert: None,
            },
        ),
        CompletionScenario::new(
            "WHERE ac→active",
            "select * from Product where ac",
            top(Column, "active"),
        ),
        CompletionScenario::new(
            "WHERE cr→created_at",
            "select * from Product where cr",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "created_at", 5)),
                absent: vec![(Table, "Product"), (Table, "Customer")],
                replace: None,
                insert: None,
            },
        ),
        CompletionScenario::new(
            "WHERE ci→city",
            "select * from Customer where ci",
            top(Column, "city"),
        ),
        CompletionScenario::new(
            "WHERE em→email",
            "select * from Customer where em",
            top(Column, "email"),
        ),
        CompletionScenario::new(
            "WHERE qu→quantity",
            "select * from Order where qu",
            top(Column, "quantity"),
        ),
        // ---- FROM 前缀压力覆盖 ----
        CompletionScenario::new("FROM Prod", "select * from Prod", top(Table, "Product")),
        CompletionScenario::new("FROM Custo", "select * from Custo", top(Table, "Customer")),
        CompletionScenario::new("FROM Orde", "select * from Orde", top(Table, "Order")),
        CompletionScenario::new(
            "FROM ProductC",
            "select * from ProductC",
            top(Table, "ProductCategory"),
        ),
        // ---- 其它 ----
        CompletionScenario::new(
            "SELECT ca 前缀",
            "select ca",
            FixtureWant {
                top1: None,
                in_top: Some((Column, "category_id", 20)),
                absent: vec![(Table, "Product")],
                replace: None,
                insert: None,
            },
        ),
        CompletionScenario::new(
            "DELETE FROM 表源",
            "delete from ",
            FixtureWant {
                top1: None,
                in_top: Some((Table, "Customer", 10)),
                absent: vec![(Column, "id")],
                replace: None,
                insert: None,
            },
        ),
        CompletionScenario::new(
            "DROP TRIGGER 空前缀",
            "drop trigger ",
            FixtureWant {
                top1: None,
                in_top: Some((Trigger, "product_ai", 10)),
                absent: vec![(Table, "Product"), (Keyword, "SELECT")],
                replace: None,
                insert: None,
            },
        ),
        // ---- 替换区间 ----
        CompletionScenario::new(
            "替换区间仅覆盖前缀",
            "select * from Product where na",
            FixtureWant {
                top1: Some((Column, "name")),
                in_top: None,
                absent: Vec::new(),
                replace: Some(("select * from Product where na".len() - 2, "select * from Product where na".len())),
                insert: None,
            },
        ),
    ]
}

// ---------- 采集器 ----------

/// 采集器：对全部 fixture 场景跑补全，输出可复现报告并就地锁定硬性断言。
/// 返回结构化指标便于 T085 验收复用。
fn completion_fixture_collect(print_report: bool) -> FixtureReport {
    let scenarios = completion_scenarios();
    let mut report = FixtureReport::default();
    for scenario in &scenarios {
        let result = matrix_request_result(scenario.sql, scenario.cursor);
        let items = &result.items;
        report.scenarios += 1;

        // Top-1 / InTop 命中。Top-1 命中率仅在声明 top1 锚点的场景上统计。
        if let Some((kind, label)) = &scenario.want.top1 {
            report.anchored += 1;
            if matches_first(items, *kind, label) {
                report.hit1 += 1;
            }
            // Top-5 命中：Top-1 目标出现在前 5 个候选内（T085 验收：≥98%）。
            if matches_in_top(items, *kind, label, 5) {
                report.hit_top5 += 1;
            }
        }
        if let Some((kind, label, k)) = &scenario.want.in_top {
            if matches_in_top(items, *kind, label, *k) {
                report.hit_top += 1;
            }
        }
        let top1_ok = scenario
            .want
            .top1
            .map_or(true, |(kind, label)| matches_first(items, kind, label));
        let in_top_ok = scenario.want.in_top.map_or(true, |(kind, label, k)| {
            matches_in_top(items, kind, label, k)
        });

        // Absent 负例。
        let mut absent_ok = true;
        for (kind, label) in &scenario.want.absent {
            if items.iter().any(|i| i.kind == *kind && i.label.eq_ignore_ascii_case(label)) {
                absent_ok = false;
                report.absent_violations += 1;
            }
        }

        // Replace 区间与 Insert 文本。
        let mut replace_ok = true;
        if let Some((start, end)) = scenario.want.replace {
            if result.replace_start != start || result.replace_end != end {
                replace_ok = false;
                report.replace_errors += 1;
            }
        }
        let mut insert_ok = true;
        // insert 期望绑定到 Top-1 目标；若要校验非 Top-1 候选（如星号展开 snippet），
        // 用 in_top + insert 组合，这里按 in_top 目标扫描其插入文本。
        if let Some(expected_insert) = scenario.want.insert {
            let target = scenario
                .want
                .in_top
                .and_then(|(kind, label, _)| {
                    items
                        .iter()
                        .find(|i| label_matches_prefix(i, kind, label))
                })
                .or_else(|| {
                    scenario
                        .want
                        .top1
                        .and_then(|(kind, label)| {
                            items
                                .iter()
                                .find(|i| label_matches_prefix(i, kind, label))
                        })
                });
            if target.map(|i| i.insert_text.as_str()) != Some(expected_insert) {
                insert_ok = false;
                report.insert_errors += 1;
            }
        }

        let ok = top1_ok && in_top_ok && absent_ok && replace_ok && insert_ok;
        if !ok {
            report.failed += 1;
        }
        if print_report {
            println!(
                "FIXTURE {}  {:>50}  {}  top1={:?}  top1_pos={:?}  replace=[{},{})  items={}",
                if ok { "PASS" } else { "FAIL" },
                scenario.name,
                scenario.sql,
                scenario.want.top1,
                items.first().map(|i| (i.kind, i.label.as_str())),
                result.replace_start,
                result.replace_end,
                items.len(),
            );
        }
        if !ok && print_report {
            for item in items.iter().take(8) {
                println!(
                    "      - {:?} label={} insert={}",
                    item.kind, item.label, item.insert_text
                );
            }
        }
    }
    report
}

#[derive(Default)]
struct FixtureReport {
    scenarios: usize,
    /// 声明了 top1 锚点的场景数。
    anchored: usize,
    hit1: usize,
    /// Top-1 目标出现于前 5 的锚点场景数（T085 Top-5 指标）。
    hit_top5: usize,
    hit_top: usize,
    failed: usize,
    absent_violations: usize,
    replace_errors: usize,
    insert_errors: usize,
}

fn matches_first(items: &[QueryCompletionItem], kind: QueryCompletionKind, label: &str) -> bool {
    items
        .first()
        .is_some_and(|i| i.kind == kind && i.label.eq_ignore_ascii_case(label))
}

// in_top 使用前缀匹配：snippet 候选 label 较长（如 `JOIN ProductCategory ON ...`），
// 期望可写成前缀。普通候选的 label 前缀即为全名，两者兼容。
fn label_matches_prefix(item: &QueryCompletionItem, kind: QueryCompletionKind, prefix: &str) -> bool {
    item.kind == kind
        && item
            .label
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn matches_in_top(items: &[QueryCompletionItem], kind: QueryCompletionKind, label: &str, k: usize) -> bool {
    items.iter().take(k).any(|i| label_matches_prefix(i, kind, label))
}

#[test]
fn completion_fixture_full_suite() {
    // T081 统一 corpus：锁定每场景硬性断言 + 汇总指标（Top-1 / 错误率 / 替换错误）。
    let report = completion_fixture_collect(true);
    let scenarios = report.scenarios;
    let hit1 = report.hit1;
    let anchored = report.anchored;
    let top1_rate = if anchored == 0 { 0.0 } else { hit1 as f64 / anchored as f64 };
    let error_rate = if scenarios == 0 {
        0.0
    } else {
        report.failed as f64 / scenarios as f64
    };
    let top5_rate = if anchored == 0 {
        0.0
    } else {
        report.hit_top5 as f64 / anchored as f64
    };
    println!(
        "\nFIXTURE REPORT (T085): scenarios={}  top1_anchored={}  Top-1={} ({:.1}%)  Top-5={} ({:.1}%)  错误候选率={:.2}%  hard_errors={}  absent_violations={}  replace_errors={}  insert_errors={}",
        scenarios,
        anchored,
        hit1,
        top1_rate * 100.0,
        report.hit_top5,
        top5_rate * 100.0,
        error_rate * 100.0,
        report.failed,
        report.absent_violations,
        report.replace_errors,
        report.insert_errors,
    );
    // 硬性收敛：所有 Top-1 锚点必须命中（100%）；替换区间/插入文本/负例错误必须为 0。
    assert!(
        report.anchored > 0 && hit1 == report.anchored,
        "所有 Top-1 锚点场景必须命中（hit1={}/{}）",
        hit1,
        report.anchored
    );
    // T085 验收门槛：Top-5 ≥98%、错误候选率 ≤2%、替换区间错误为 0。
    assert!(
        anchored == 0 || top5_rate >= 0.98,
        "Top-5 命中率不足（{:.1}% < 98%）",
        top5_rate * 100.0
    );
    assert!(error_rate <= 0.02, "错误候选率超限（{:.2}% > 2%）", error_rate * 100.0);
    assert_eq!(report.replace_errors, 0, "替换区间错误必须为 0");
    assert_eq!(report.insert_errors, 0, "插入文本错误必须为 0");
    assert_eq!(report.absent_violations, 0, "负例泄漏必须为 0");
    assert_eq!(report.failed, 0, "硬性断言不得有任何失败");
}


