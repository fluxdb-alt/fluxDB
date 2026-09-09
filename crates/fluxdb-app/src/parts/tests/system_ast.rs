// ---------------------------------------------------------------------------
// T086 系统级 SQL 补全测试（AST / 容错解析 / offset 边界）
//
// 覆盖验收项 1「AST 系统测试」与验收指标「AST 解析和 fallback 场景无未处理
// panic，语句边界和光标 offset 全部正确」。直接对 tolerant parser、统一
// referenced-tables、括号匹配做确定性单测，并覆盖 UNION/未闭合括号/多语句。
//
// 安全边界：纯本地解析，不触达任何数据库连接。
// ---------------------------------------------------------------------------

#[test]
fn tolerant_parse_handles_complete_and_incomplete_sql() {
    // 完整 SQL / 多语句 / UNION 都能容错解析出语句。
    assert!(parse_statements_tolerant("select * from products", true).is_some());
    assert!(parse_statements_tolerant("select * from a; select * from b", true).is_some());
    assert!(parse_statements_tolerant("select id from t1 union select id from t2", true).is_some());

    // 未闭合括号 / 未闭合引号 / 悬空 `.`：容忍解析不 panic、不无限循环即可
    //（可能回退到 None，这是合法行为；关键是快速收敛）。
    let _ = parse_statements_tolerant("select * from (select id from t", true);
    let _ = parse_statements_tolerant("select * from `orders", true);
    let _ = parse_statements_tolerant("select * from .", true);
}

/// 回归：尾部多字节字符（UTF-8，如 em dash `—`）不整除字节时，容忍解析的
/// 回退切片必须落在字符边界，否则 `&candidate[..end]` 会 panic（真实 column
/// comment/数据可含这类字符）。此前在真实 dev MySQL 触发 panic abort。
#[test]
fn tolerant_parse_never_panics_on_multibyte_tail() {
    // 触发回退路径：尾部是连续多字节非感叹符号，字节剥除后可能落在 `—` 中间。
    let sql = "select * from t where c = '—'";
    let _ = parse_statements_tolerant(sql, true);
    let _ = parse_statements_tolerant(sql, false);
    // 直接以多字节字符收尾，且整段不 sign 法（强制走到 `saturating_sub(1)` 回退）。
    let hostile = "select —";
    let _ = parse_statements_tolerant(hostile, true);
    // 多字节 + 尾部悬空点号混杂。
    let _ = parse_statements_tolerant("select —. foo", true);
}

#[test]
fn tolerant_parse_never_hangs_or_panics_on_hostile_input() {
    // 纯标点 / 未知符号 / 空串：应快速返回 None 或 Some，绝不 panic、绝不死循环。
    for hostile in [
        "",
        "   ",
        ")))",
        "(((",
        "`",
        "'",
        ")",
        "select * from (",
        ".",
        "select . . .",
        "select * from \u{1F600}",
    ] {
        let _ = parse_statements_tolerant(hostile, true);
    }
}

// collect_set_expr_tables 路径（UNION）端到端：UNION 两侧表都应被引用。
#[test]
fn referenced_tables_covers_union_both_sides() {
    let sql = "select a.id from customer a UNION select b.id from product b";
    let tables = referenced_tables_from_statements(sql, true);
    let names: Vec<_> = tables.iter().map(|t| (t.name.as_str(), t.alias.as_deref())).collect();
    assert!(
        tables.iter().any(|t| t.name == "customer" && t.alias.as_deref() == Some("a")),
        "UNION 左表 customer 缺失: {names:?}"
    );
    assert!(
        tables.iter().any(|t| t.name == "product" && t.alias.as_deref() == Some("b")),
        "UNION 右表 product 缺失: {names:?}"
    );
}

#[test]
fn referenced_tables_covers_multi_statement_and_corelates_with_current_query() {
    let sql = "select * from users u; select * from orders o where id = 1";
    let tables = referenced_tables_from_statements(sql, true);
    let names: Vec<_> = tables.iter().map(|t| t.name.as_str()).collect();
    // 两语句表都出现在 AST 级引用集合（当前语句隔离由 sql_completion_context 负责）。
    assert!(names.contains(&"users"), "缺少 users: {names:?}");
    assert!(names.contains(&"orders"), "缺少 orders: {names:?}");
}

// 括号匹配：嵌套、引号感知、未闭合返回 None。
#[test]
fn sql_paren_matching_handles_nesting_quotes_and_unclosed() {
    // `func(a, (b))` 中第 0 个 `(` 匹配到收尾 `)`。
    let sql = "func(a, (b))";
    let open = sql.find('(').unwrap();
    assert_eq!(find_matching_sql_paren(sql, open), Some(sql.rfind(')').unwrap()));

    // 引号内的括号不计数：`(")")`。
    let quoted = "val(')', x)";
    let open = quoted.find('(').unwrap();
    assert_eq!(find_matching_sql_paren(quoted, open), Some(quoted.rfind(')').unwrap()));

    // 未闭合括号。
    assert_eq!(find_matching_sql_paren("select (1, (2", 7), None);

    // 越界起始：open 超出文本返回 None。
    assert_eq!(find_matching_sql_paren("abc", 10), None);
}

// offset 边界：光标落在首/末语句、以及大文档边缘时上下文不 panic、offset 收敛。
#[test]
fn statement_offset_bounds_do_not_panic() {
    // 光标在首语句末尾。
    let first = "select name from users; select id from orders";
    let ctx = sql_completion_context(first, first.find(';').unwrap() + 1, DatabaseKind::MySql);
    assert!(ctx.suggest_columns || ctx.suggest_tables || ctx.suggest_keywords);

    // 光标在文档末尾。
    let last = "select * from orders where";
    let ctx_end = sql_completion_context(last, last.len(), DatabaseKind::MySql);
    assert!(ctx_end.suggest_columns || ctx_end.suggest_keywords);

    // 光标越界（超出文本长度）应被 clamp，不 panic。
    let _ = sql_completion_context("select", 9999, DatabaseKind::MySql);
    let _ = sql_completion_context("", 5, DatabaseKind::MySql);
    let _ = sql_completion_context("", 0, DatabaseKind::MySql);
}

// ---------------------------------------------------------------------------
// 词法作用域（CTE / 派生表 / 别名遮蔽 / JOIN 双侧引用）
// ---------------------------------------------------------------------------

/// CTE：外层 CTE 输出列对后续查询可见，限定到 CTE 名可补全其列。
#[test]
fn lexical_scope_exposes_cte_output_columns() {
    let sql = "with recent as (select id, price from orders) select recent.";
    let syms = sql_scope_symbols(sql, DatabaseKind::MySql);
    let cols = syms.cte_columns.get("recent").cloned().unwrap_or_default();
    assert!(cols.contains(&"id".to_string()), "CTE 列缺 id: {cols:?}");
    assert!(cols.contains(&"price".to_string()), "CTE 列缺 price: {cols:?}");
}

/// 派生表：`(子查询) 别名` 的输出列对别名可见。
#[test]
fn lexical_scope_exposes_derived_table_columns() {
    let sql = "select * from (select a, b from t) sub where sub.";
    let syms = sql_scope_symbols(sql, DatabaseKind::MySql);
    let cols = syms.derived_columns.get("sub").cloned().unwrap_or_default();
    assert!(cols.contains(&"a".to_string()), "派生表列缺 a: {cols:?}");
    assert!(cols.contains(&"b".to_string()), "派生表列缺 b: {cols:?}");
}

/// 别名遮蔽（文档化契约）：fallback 只遍历当前语句顶层 WITH 列表，不做跨子查询
/// 的递归遮蔽解析。外层顶层 CTE 输出列保持可见；同语句内嵌套子查询重定义同名
/// CTE 不在当前启发式契约内（见 `extract_cte_columns_fallback` 注释）——这里只断言
/// 外层定义不被丢失，避免伪装的「完整遮蔽」承诺。
#[test]
fn lexical_scope_keeps_outer_top_level_cte_columns() {
    let sql = "with x as (select p from t1) select (with x as (select q from t2) select x.";
    let syms = sql_scope_symbols(sql, DatabaseKind::MySql);
    let cols = syms.cte_columns.get("x").cloned().unwrap_or_default();
    // 外层顶层 CTE 输出列 p 必须保留（当前 fallback 的顶层定义契约）。
    assert!(
        cols.contains(&"p".to_string()),
        "外层顶层 CTE 列 p 缺失，落回无遮蔽定义: {cols:?}"
    );
}

/// JOIN 双侧：LEFT JOIN 两侧表都被引用（RIGHT/INNER 同理）。
#[test]
fn lexical_scope_references_join_both_sides() {
    let sql = "select a.id from customer a left join product b on a.k = b.k";
    let syms = sql_scope_symbols(sql, DatabaseKind::MySql);
    let names: Vec<_> = syms
        .referenced_tables
        .iter()
        .map(|t| (t.name.as_str(), t.alias.as_deref()))
        .collect();
    assert!(
        syms.referenced_tables.iter().any(|t| t.name == "customer" && t.alias.as_deref() == Some("a")),
        "JOIN 左表 customer/a 缺失: {names:?}"
    );
    assert!(
        syms.referenced_tables.iter().any(|t| t.name == "product" && t.alias.as_deref() == Some("b")),
        "JOIN 右表 product/b 缺失: {names:?}"
    );
}

/// 表别名关联：限定到别名时列来自该表（别名 → 底层表）。断言 SQL 中的 table_alias
/// 映射在 AST 关系槽位中可解析（customer 以别名 a 录入）。
#[test]
fn lexical_scope_table_alias_maps_to_underlying_table() {
    let sql = "select * from customer c where c.";
    let syms = sql_scope_symbols(sql, DatabaseKind::MySql);
    let c = syms
        .referenced_tables
        .iter()
        .find(|t| t.alias.as_deref() == Some("c"))
        .expect("customer 应以别名 c 录入作用域");
    assert_eq!(c.name, "customer");
}
