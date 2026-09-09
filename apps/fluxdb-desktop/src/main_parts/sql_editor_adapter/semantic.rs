// sql_editor_adapter/semantic.rs —— SQL 语义作用域层（Phase 8，DM-800~804）。
//
// 用 tree-sitter-sequel 解析文档全文，收集**查询内**符号——CTE 名、子查询派生表/
// JOIN 关系别名及其投影列——作为补全的**动态**候选注入（不进静态 completion_items_cache，
// 查询内符号随语句变化）。
//
// - DM-800 跨 statement 符号作用域：先出现的 WITH CTE 与别名，后文语句均可提示。
// - DM-801 CTE/view/temp 依赖：CTE 列集从其子查询投影推导，可提升提示；view/temp
//   是持久元数据，由 SqlCompletionSource 提供（此处不重复）。
// - DM-802 表达式结果类型：已知函数结果类型表 `function_result_type`（轻量，不建
//   通用类型代数，函数重载按参数个数选择的签名浮层由既有 SqlSignatureProvider 覆盖）。
// - DM-803 相关子查询/派生表/lateral：`ponytail:` 平铺文档级作用域，子查询内层
//   可见外层别名由「全局收集」天然满足；真实逐层遮蔽影子作用域若出现再升级。
// - DM-804 方言语义留在 adapter：本模块只被 sql_editor_adapter 消费，core 零依赖。
//
// 纯函数、零 GPUI、不依赖数据库；独立于 syntax.rs 的高亮 cache（语义只取符号**名**，
// 不取字节区间，避免其 MySQL 归一化偏移 bookkeeping 纠缠）。

use tree_sitter::{Language, Parser};

/// 查询内 CTE 符号（DM-800/801）：`WITH name AS (SELECT ...)`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlCte {
    /// CTE 名，可提示。
    pub name: String,
    /// 从 CTE 子查询 select_expression 投影推导的列名。
    pub columns: Vec<String>,
}

/// 查询内关系别名符号（DM-800/803）：`users u`、`(SELECT ...) x`、`JOIN t top`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlAlias {
    /// 别名本身，可提示。
    pub name: String,
    /// 派生表/子查询自推导的投影列（无则空）。
    pub columns: Vec<String>,
    /// 是否为子查询派生表别名（`(SELECT ...) x`），是则列来自子查询。
    pub derived: bool,
}

/// 文档级 SQL 符号作用域（DM-800）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SqlScope {
    /// 全部 CTE（含跨 statement 依赖，后文可引用前文 WITH 定义）。
    pub ctes: Vec<SqlCte>,
    /// 全部关系别名（含 FROM/JOIN/子查询派生表）。
    pub aliases: Vec<SqlAlias>,
}

impl SqlScope {
    /// 收集全文符号。tree-sitter 解析失败返回空作用域。
    pub fn from_text(text: &str) -> SqlScope {
        let mut scope = SqlScope::default();
        let mut parser = Parser::new();
        if parser
            .set_language(&Language::new(tree_sitter_sequel::LANGUAGE))
            .is_err()
        {
            return scope;
        }
        let Some(tree) = parser.parse(text.as_bytes(), None) else {
            return scope;
        };
        // 从根 program 出发，逐 statement 收集（DM-800 跨 statement）。
        let root = tree.root_node();
        for i in 0..root.named_child_count() {
            if let Some(stmt) = root.named_child(i as u32) {
                collect_statement(stmt, &mut scope, text.as_bytes());
            }
        }
        scope
    }
}

/// 把作用域符号转成补全候选（DM-800/801/803 的消费端）。
///
/// CTE 名与别名 → Table；CTE 列与派生表列 → Column。提升优先级使其优先于
/// 静态元数据同名项，且带中文 detail。
pub fn scope_to_items(scope: &SqlScope) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for cte in &scope.ctes {
        let mut item = CompletionItem::new(&cte.name, CompletionKind::Table);
        item.priority = 62; // 高于元数据表(60)，CTE 为当前查询定义、最相关
        item.detail = "CTE".to_string();
        items.push(item);
        for col in &cte.columns {
            let mut item = CompletionItem::new(col, CompletionKind::Column);
            item.priority = 61;
            item.detail = format!("CTE 列：{}", cte.name);
            items.push(item);
        }
    }
    for alias in &scope.aliases {
        let mut item = CompletionItem::new(&alias.name, CompletionKind::Table);
        item.priority = 60;
        item.detail = if alias.derived {
            "派生表别名".to_string()
        } else {
            "表别名".to_string()
        };
        items.push(item);
        for col in &alias.columns {
            let mut item = CompletionItem::new(col, CompletionKind::Column);
            item.priority = 61;
            item.detail = format!("派生表列：{}", alias.name);
            items.push(item);
        }
    }
    items
}

/// 已知 SQL 函数 → 结果类型（DM-802，轻量白名单）。
///
/// 不建通用类型代数，只对常用聚合/标量函数给出结果类型，用于函数补全项的
/// detail 增强。未知函数返回 None（保守）。键为小写函数名。
pub fn function_result_type(fn_name: &str) -> Option<&'static str> {
    Some(match fn_name.to_ascii_lowercase().as_str() {
        "count" | "sum" | "count_big" | "if" | "ifnull" | "coalesce" => "数值",
        "avg" | "max" | "min" => "数值/日期/字符串",
        "concat" | "substring" | "substr" | "lower" | "lcase" | "upper" | "ucase"
        | "trim" | "ltrim" | "rtrim" | "replace" | "cast" | "convert" | "now"
        | "current_date" | "current_timestamp" => "字符串/日期",
        "len" | "length" | "char_length" | "round" | "floor" | "ceil" | "abs"
        | "mod" | "rand" => "数值",
        "date" | "year" | "month" | "day" | "hour" | "minute" | "second" => "日期/数值",
        _ => return None,
    })
}

/// 全树遍历：递归进所有 named 子节点，按 kind 收集符号（DM-800）。
/// `relation` 可嵌套在 from/join 下，故不停在 statement 层。
fn collect_statement(stmt: tree_sitter::Node, scope: &mut SqlScope, src: &[u8]) {
    for i in 0..stmt.named_child_count() {
        let Some(node) = stmt.named_child(i as u32) else { continue };
        match node.kind() {
            "cte" => {
                let name = first_identifier(node, src).unwrap_or_default();
                let mut columns = Vec::new();
                // cte 的列来自其子 statement 的 select 投影。
                for j in 0..node.named_child_count() {
                    let Some(st) = node.named_child(j as u32) else { continue };
                    if st.kind() == "statement"
                        && let Some(cols) = projected_columns_of_statement(st, src)
                    {
                        columns = cols;
                    }
                }
                if !name.is_empty() {
                    scope.ctes.push(SqlCte { name, columns });
                }
            }
            "relation" => {
                collect_relation(node, scope, src);
            }
            _ => {}
        }
        // 始终递归，收集更深层（from 下的、子查询内的、join 下的 relation）。
        collect_statement(node, scope, src);
    }
}

/// 收集单个 relation 的别名与子查询派生表，并递归其内部子查询（DM-800/803）。
fn collect_relation(node: tree_sitter::Node, scope: &mut SqlScope, src: &[u8]) {
    let mut has_sub = false;
    let mut alias = None;
    let mut saw_source = false;
    for i in 0..node.named_child_count() {
        let Some(child) = node.named_child(i as u32) else { continue };
        match child.kind() {
            "subquery" => {
                has_sub = true;
                saw_source = true;
            }
            "object_reference" => saw_source = true,
            "identifier" if saw_source => {
                // 源（表/子查询）之后的 identifier 即别名。
                alias = Some(node_text(child, src).to_string());
                break;
            }
            _ => {}
        }
    }
    if let Some(name) = alias {
        let derived_cols = if has_sub {
            projected_columns_of_node(node, src).unwrap_or_default()
        } else {
            Vec::new()
        };
        scope.aliases.push(SqlAlias {
            name,
            columns: derived_cols,
            derived: has_sub,
        });
    }
}

/// 取一个语句节点（select/statement 包着的 select）的 select_expression 投影列
/// （每个 field 的最右 identifier，剥掉表限定）。
fn projected_columns_of_statement(
    stmt: tree_sitter::Node,
    src: &[u8],
) -> Option<Vec<String>> {
    if stmt.kind() == "statement" {
        for i in 0..stmt.named_child_count() {
            let child = stmt.named_child(i as u32)?;
            if child.kind() == "select" {
                return projected_columns_of_node(child, src);
            }
        }
        return None;
    }
    projected_columns_of_node(stmt, src)
}

fn projected_columns_of_node(node: tree_sitter::Node, src: &[u8]) -> Option<Vec<String>> {
    // 从 node 内找 select_expression → term* → field 的投影列。
    let mut cols = Vec::new();
    let mut found = false;
    for term in descendants_of_kind(node, "term") {
        for i in 0..term.named_child_count() {
            let Some(field) = term.named_child(i as u32) else { continue };
            if field.kind() != "field" {
                continue;
            }
            found = true;
            // field 的最右 identifier 即投影列名（剥掉 object_reference 限定）。
            if let Some(id) = last_identifier(field, src) {
                cols.push(id.to_string());
            }
        }
    }
    if found {
        Some(cols)
    } else {
        None
    }
}

/// 深度遍历 node，收集所有 kind 为 `kind` 的祖先级子孙节点。
fn descendants_of_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
) -> Vec<tree_sitter::Node<'a>> {
    let mut out = Vec::new();
    collect_kind(node, kind, &mut out);
    out
}

fn collect_kind<'a>(
    node: tree_sitter::Node<'a>,
    kind: &str,
    acc: &mut Vec<tree_sitter::Node<'a>>,
) {
    if node.kind() == kind {
        acc.push(node);
    }
    for i in 0..node.named_child_count() {
        if let Some(c) = node.named_child(i as u32) {
            collect_kind(c, kind, acc);
        }
    }
}

fn first_identifier(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    for i in 0..node.named_child_count() {
        let child = node.named_child(i as u32)?;
        if child.kind() == "identifier" {
            return Some(node_text(child, src).to_string());
        }
    }
    None
}

fn last_identifier<'a>(node: tree_sitter::Node<'_>, src: &'a [u8]) -> Option<&'a str> {
    node_text(node, src)
        .rsplit('.')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn node_text<'a>(node: tree_sitter::Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("")
}

#[cfg(test)]
mod semantic_tests {
    use super::*;

    fn scope(sql: &str) -> SqlScope {
        SqlScope::from_text(sql)
    }

    #[test]
    fn cte_name_and_columns_collected() {
        let s = scope("WITH recent AS (SELECT id, name FROM users WHERE id > 0) SELECT * FROM recent");
        assert_eq!(s.ctes.len(), 1);
        assert_eq!(s.ctes[0].name, "recent");
        assert!(s.ctes[0].columns.contains(&"id".to_string()));
        assert!(s.ctes[0].columns.contains(&"name".to_string()));
    }

    #[test]
    fn multiple_ctes_cross_statement_dependency() {
        // DM-801：后文语句引用前文 WITH 定义的 CTE；两个 CTE 都应可提示。
        let s = scope(
            "WITH a AS (SELECT id FROM t), b AS (SELECT id FROM a) SELECT * FROM b",
        );
        let names: Vec<_> = s.ctes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn derived_table_alias_and_columns() {
        // DM-800/803：子查询派生表 `(SELECT a.id FROM t a) x` 得别名 x 与投影列 id。
        let s = scope("SELECT x.id FROM (SELECT a.id FROM t a) x WHERE x.id = 1");
        assert!(s.aliases.iter().any(|a| a.name == "x" && a.derived));
        assert!(s.aliases.iter().any(|a| a.name == "a"));
        let x = s.aliases.iter().find(|a| a.name == "x").unwrap();
        assert!(x.columns.contains(&"id".to_string()));
    }

    #[test]
    fn join_alias_collected() {
        let s = scope("SELECT * FROM u JOIN top t ON u.id = t.id");
        assert!(s.aliases.iter().any(|a| a.name == "t"));
        // 无别名的 `u` 不登记
        assert!(!s.aliases.iter().any(|a| a.name == "u"));
    }

    #[test]
    fn correlated_subquery_sees_outer_alias() {
        // DM-803：相关子查询内层引用外层别名 r；外层别名全局可收集。
        let s = scope(
            "SELECT name FROM users r WHERE r.id IN (SELECT t.id FROM t WHERE t.x = r.id)",
        );
        assert!(s.aliases.iter().any(|a| a.name == "r"));
    }

    #[test]
    fn malformed_sql_returns_empty_scope() {
        assert!(scope("").ctes.is_empty());
        assert!(scope("\u{fffd}").aliases.is_empty());
    }

    #[test]
    fn function_result_type_known_and_unknown() {
        assert_eq!(function_result_type("COUNT"), Some("数值"));
        assert_eq!(function_result_type("count"), Some("数值"));
        assert_eq!(function_result_type("upper"), Some("字符串/日期"));
        assert_eq!(function_result_type("totally_unknown_fn"), None);
    }
}
