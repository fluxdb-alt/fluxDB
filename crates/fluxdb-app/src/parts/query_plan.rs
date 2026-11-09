// 查询计划解析：把各数据库的 EXPLAIN 输出归一为可渲染的节点树。
//
// 输入是查询结果 `DataPage`，不额外访问数据库：
// - MySQL / MariaDB / TiDB 的 `EXPLAIN FORMAT=JSON`；
// - PostgreSQL / KingbaseES 的 `EXPLAIN (FORMAT JSON)`；
// - SQLite 的 `EXPLAIN QUERY PLAN` 表格；
// - 以及用户手写普通 `EXPLAIN` 后得到的 MySQL 风格表格。
//
// 解析失败一律返回 `None`，调用方保留原始结果表格，不猜测计划结构。
// 这里只描述事实（操作类型、代价、行数、索引、告警），不生成面向用户的文案。
//
// 本文件由 crate 根 `include!` 展开，直接复用根作用域里的 CellValue / DataPage / DatabaseKind。
use serde_json::Value as JsonValue;

/// 单次解析最多保留的节点数，避免异常计划把 UI 拖垮。
pub const MAX_PLAN_NODES: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanOpKind {
    Query,
    Join,
    Scan,
    IndexLookup,
    Sort,
    Aggregate,
    Limit,
    Union,
    Subquery,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanIssue {
    FullTableScan,
    Filesort,
    NoIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryPlanSource {
    MySqlJson,
    PostgresJson,
    SqliteQueryPlan,
    TabularExplain,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryPlan {
    /// 根节点在 `nodes` 中的下标。
    pub root: usize,
    pub nodes: Vec<PlanNode>,
    pub total_cost: Option<f64>,
    pub estimated_rows: Option<u64>,
    pub source: QueryPlanSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanNode {
    pub id: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub op: PlanOpKind,
    /// 方言原始操作名（如 `Hash Join`、`Seq Scan`）；UI 对已知操作可覆盖为本地化名称。
    pub operation: String,
    pub object: Option<String>,
    pub detail: Option<String>,
    pub cost: Option<f64>,
    pub rows: Option<u64>,
    pub width: Option<u64>,
    pub filtered: Option<f64>,
    /// 访问/连接条件（Index Cond、Filter、attached_condition 等）。
    pub condition: Option<String>,
    pub index: Option<String>,
    pub possible_keys: Option<String>,
    pub extra: Option<String>,
    pub loops: Option<u64>,
    pub issues: Vec<PlanIssue>,
}

impl QueryPlan {
    pub fn node(&self, id: usize) -> Option<&PlanNode> {
        self.nodes.get(id)
    }

    pub fn warning_count(&self) -> usize {
        self.nodes.iter().map(|node| node.issues.len()).sum()
    }
}

/// 从一次查询的结果页解析计划；非计划结果返回 `None`。
///
/// 先按内容识别结构化 JSON（不依赖方言，手写 JSON 计划也能识别），
/// 再按方言解析 SQLite / MySQL 风格表格。
pub fn query_plan_from_page(kind: DatabaseKind, page: &DataPage) -> Option<QueryPlan> {
    if let Some(text) = single_cell_text(page) {
        let trimmed = text.trim_start();
        if trimmed.starts_with('{')
            && trimmed.contains("\"query_block\"")
            && let Some(plan) = parse_mysql_json(trimmed)
        {
            return Some(plan);
        }
        if (trimmed.starts_with('[') || trimmed.starts_with('{'))
            && trimmed.contains("\"Plan\"")
            && let Some(plan) = parse_postgres_json(trimmed)
        {
            return Some(plan);
        }
    }

    if kind == DatabaseKind::Sqlite {
        return parse_sqlite_query_plan(page);
    }
    parse_tabular_explain(page)
}

#[derive(Clone, Debug)]
struct RawNode {
    op: PlanOpKind,
    operation: String,
    object: Option<String>,
    detail: Option<String>,
    cost: Option<f64>,
    rows: Option<u64>,
    width: Option<u64>,
    filtered: Option<f64>,
    condition: Option<String>,
    index: Option<String>,
    possible_keys: Option<String>,
    extra: Option<String>,
    loops: Option<u64>,
    issues: Vec<PlanIssue>,
    children: Vec<RawNode>,
}

impl RawNode {
    fn new(op: PlanOpKind, operation: impl Into<String>) -> Self {
        Self {
            op,
            operation: operation.into(),
            object: None,
            detail: None,
            cost: None,
            rows: None,
            width: None,
            filtered: None,
            condition: None,
            index: None,
            possible_keys: None,
            extra: None,
            loops: None,
            issues: Vec::new(),
            children: Vec::new(),
        }
    }
}

fn flatten_plan(raw: RawNode, source: QueryPlanSource) -> QueryPlan {
    let total_cost = raw.cost.or_else(|| raw_max_cost(&raw));
    let estimated_rows = raw.rows.or_else(|| raw_max_rows(&raw));
    let mut nodes = Vec::new();
    flatten_node(raw, None, &mut nodes);
    QueryPlan {
        root: 0,
        nodes,
        total_cost,
        estimated_rows,
        source,
    }
}

fn flatten_node(raw: RawNode, parent: Option<usize>, out: &mut Vec<PlanNode>) {
    if out.len() >= MAX_PLAN_NODES {
        return;
    }
    let id = out.len();
    out.push(PlanNode {
        id,
        parent,
        children: Vec::new(),
        op: raw.op,
        operation: raw.operation,
        object: raw.object,
        detail: raw.detail,
        cost: raw.cost,
        rows: raw.rows,
        width: raw.width,
        filtered: raw.filtered,
        condition: raw.condition,
        index: raw.index,
        possible_keys: raw.possible_keys,
        extra: raw.extra,
        loops: raw.loops,
        issues: raw.issues,
    });
    for child in raw.children {
        if out.len() >= MAX_PLAN_NODES {
            break;
        }
        let child_id = out.len();
        flatten_node(child, Some(id), out);
        if out.len() > child_id
            && let Some(node) = out.get_mut(id)
        {
            node.children.push(child_id);
        }
    }
}

fn raw_max_cost(raw: &RawNode) -> Option<f64> {
    let own = raw.cost;
    raw.children
        .iter()
        .filter_map(raw_max_cost)
        .fold(own, |best, cost| Some(best.map_or(cost, |best| best.max(cost))))
}

fn raw_max_rows(raw: &RawNode) -> Option<u64> {
    let own = raw.rows;
    raw.children
        .iter()
        .filter_map(raw_max_rows)
        .fold(own, |best, rows| Some(best.map_or(rows, |best| best.max(rows))))
}

// ---------------------------------------------------------------------------
// 通用取值
// ---------------------------------------------------------------------------

fn single_cell_text(page: &DataPage) -> Option<String> {
    if page.rows.len() != 1 || page.rows[0].values.len() != 1 {
        return None;
    }
    plan_cell_text(&page.rows[0].values[0])
}

fn plan_cell_text(cell: &CellValue) -> Option<String> {
    match cell {
        CellValue::Null => None,
        CellValue::Bool(value) => Some(value.to_string()),
        CellValue::I64(value) => Some(value.to_string()),
        CellValue::F64(value) => Some(value.to_string()),
        CellValue::Text(value) | CellValue::Json(value) => Some(value.clone()),
        CellValue::TruncatedText { preview, .. } => Some(preview.clone()),
        CellValue::Bytes(_) | CellValue::BinarySummary(_) | CellValue::LargeValue(_) => None,
    }
}

fn column_index(page: &DataPage, name: &str) -> Option<usize> {
    page.columns
        .iter()
        .position(|column| column.name.eq_ignore_ascii_case(name))
}

fn row_text(row: &fluxdb_core::Row, index: Option<usize>) -> Option<String> {
    let index = index?;
    row.values.get(index).and_then(plan_cell_text)
}

fn json_str(value: Option<&JsonValue>, key: &str) -> Option<String> {
    let value = value?.get(key)?;
    match value {
        JsonValue::String(text) if !text.is_empty() => Some(text.clone()),
        JsonValue::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn json_u64(value: Option<&JsonValue>, key: &str) -> Option<u64> {
    let value = value?.get(key)?;
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number.max(0.) as u64))
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()).map(|number| number.max(0.) as u64))
}

fn json_f64(value: Option<&JsonValue>, key: &str) -> Option<f64> {
    let value = value?.get(key)?;
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
}

fn json_bool(value: Option<&JsonValue>, key: &str) -> bool {
    value
        .and_then(|value| value.get(key))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn json_string_array(value: Option<&JsonValue>, key: &str) -> Option<String> {
    let items = value?.get(key)?.as_array()?;
    let parts = items
        .iter()
        .filter_map(JsonValue::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn cost_from_info(info: Option<&JsonValue>, key: &str) -> Option<f64> {
    json_f64(info, key)
}

fn mysql_table_cost(table: &JsonValue) -> Option<f64> {
    let cost_info = table.get("cost_info");
    cost_from_info(cost_info, "prefix_cost").or_else(|| {
        let read = cost_from_info(cost_info, "read_cost")?;
        let eval = cost_from_info(cost_info, "eval_cost").unwrap_or(0.);
        Some(read + eval)
    })
}

// ---------------------------------------------------------------------------
// MySQL / MariaDB / TiDB：EXPLAIN FORMAT=JSON
// ---------------------------------------------------------------------------

fn parse_mysql_json(text: &str) -> Option<QueryPlan> {
    let value: JsonValue = serde_json::from_str(text).ok()?;
    let block = value.get("query_block")?;
    let raw = mysql_query_block(block);
    Some(flatten_plan(raw, QueryPlanSource::MySqlJson))
}

fn mysql_query_block(block: &JsonValue) -> RawNode {
    let mut node = RawNode::new(PlanOpKind::Query, "Query");
    node.cost = cost_from_info(block.get("cost_info"), "query_cost");
    // 常量表无匹配行等情况 MySQL 只返回 message，没有 table/cost 节点；
    // 保留说明，避免界面把它误判成“没有发现任何问题”。
    node.detail = json_str(Some(block), "message");
    node.children = mysql_children(block);
    node
}

fn mysql_children(value: &JsonValue) -> Vec<RawNode> {
    let mut children = Vec::new();

    if let Some(ordering) = value.get("ordering_operation") {
        children.push(mysql_wrapped_operation(ordering, PlanOpKind::Sort, "Sort"));
    }
    if let Some(grouping) = value.get("grouping_operation") {
        children.push(mysql_wrapped_operation(
            grouping,
            PlanOpKind::Aggregate,
            "Group",
        ));
    }
    if let Some(distinct) = value.get("duplicates_removal") {
        children.push(mysql_wrapped_operation(
            distinct,
            PlanOpKind::Other,
            "Distinct",
        ));
    }
    if let Some(nested_loop) = value.get("nested_loop").and_then(JsonValue::as_array) {
        let mut join = RawNode::new(PlanOpKind::Join, "Nested Loop");
        for item in nested_loop {
            if let Some(child) = mysql_item(item) {
                join.children.push(child);
            }
        }
        if !join.children.is_empty() {
            children.push(join);
        }
    } else if let Some(table) = value.get("table") {
        children.push(mysql_table(table));
    }

    if let Some(union) = value.get("union_result") {
        children.push(mysql_union(union));
    }

    for key in ["subqueries", "attached_subqueries"] {
        if let Some(subqueries) = value.get(key).and_then(JsonValue::as_object) {
            let mut container = RawNode::new(PlanOpKind::Subquery, "Subquery");
            for subquery in subqueries.values() {
                let block = subquery.get("query_block").unwrap_or(subquery);
                container.children.push(mysql_query_block(block));
            }
            if !container.children.is_empty() {
                children.push(container);
            }
        }
    }

    children
}

fn mysql_wrapped_operation(
    value: &JsonValue,
    op: PlanOpKind,
    operation: &str,
) -> RawNode {
    let mut node = RawNode::new(op, operation);
    if json_bool(Some(value), "using_filesort") {
        node.issues.push(PlanIssue::Filesort);
        node.extra = Some("Using filesort".to_string());
    }
    if json_bool(Some(value), "using_temporary_table") {
        merge_extra(&mut node.extra, "Using temporary");
    }
    node.cost = mysql_operation_cost(value);
    node.children = mysql_children(value);
    node
}

fn mysql_operation_cost(value: &JsonValue) -> Option<f64> {
    let info = value.get("cost_info");
    cost_from_info(info, "prefix_cost").or_else(|| cost_from_info(info, "query_cost"))
}

fn mysql_item(item: &JsonValue) -> Option<RawNode> {
    if let Some(table) = item.get("table") {
        return Some(mysql_table(table));
    }
    if let Some(nested_loop) = item.get("nested_loop").and_then(JsonValue::as_array) {
        let mut join = RawNode::new(PlanOpKind::Join, "Nested Loop");
        for child in nested_loop {
            if let Some(child) = mysql_item(child) {
                join.children.push(child);
            }
        }
        return Some(join);
    }
    if let Some(ordering) = item.get("ordering_operation") {
        return Some(mysql_wrapped_operation(ordering, PlanOpKind::Sort, "Sort"));
    }
    if let Some(grouping) = item.get("grouping_operation") {
        return Some(mysql_wrapped_operation(
            grouping,
            PlanOpKind::Aggregate,
            "Group",
        ));
    }
    None
}

fn mysql_table(table: &JsonValue) -> RawNode {
    let access_type = json_str(Some(table), "access_type").unwrap_or_default();
    let lower = access_type.to_ascii_lowercase();
    let op = match lower.as_str() {
        "all" => PlanOpKind::Scan,
        "index" | "range" | "ref" | "eq_ref" | "const" | "system" | "index_merge" => {
            PlanOpKind::IndexLookup
        }
        _ => PlanOpKind::Other,
    };

    let mut node = RawNode::new(op, if access_type.is_empty() { "Table" } else { &access_type });
    node.object = json_str(Some(table), "table_name");
    node.index = json_str(Some(table), "key");
    node.possible_keys = json_string_array(Some(table), "possible_keys");
    node.rows = json_u64(Some(table), "rows_produced_per_join")
        .or_else(|| json_u64(Some(table), "rows_examined_per_scan"));
    node.filtered = json_f64(Some(table), "filtered");
    node.cost = mysql_table_cost(table);
    node.condition = json_str(Some(table), "attached_condition")
        .or_else(|| json_str(Some(table), "index_condition"));
    node.detail = json_str(Some(table), "using_join_buffer");
    if json_bool(Some(table), "using_temporary_table") {
        merge_extra(&mut node.extra, "Using temporary");
    }
    if lower == "all" {
        node.issues.push(PlanIssue::FullTableScan);
    } else if node.index.is_none() && matches!(lower.as_str(), "index" | "range" | "ref" | "eq_ref") {
        node.issues.push(PlanIssue::NoIndex);
    }
    node
}

fn mysql_union(union: &JsonValue) -> RawNode {
    let mut node = RawNode::new(PlanOpKind::Union, "Union");
    if let Some(specs) = union.get("query_specifications").and_then(JsonValue::as_array) {
        for spec in specs {
            let block = spec.get("query_block").unwrap_or(spec);
            node.children.push(mysql_query_block(block));
        }
    }
    node
}

// ---------------------------------------------------------------------------
// PostgreSQL / KingbaseES：EXPLAIN (FORMAT JSON)
// ---------------------------------------------------------------------------

fn parse_postgres_json(text: &str) -> Option<QueryPlan> {
    let value: JsonValue = serde_json::from_str(text).ok()?;
    let plan = if let Some(items) = value.as_array() {
        items.first()?.get("Plan").or_else(|| items.first())?
    } else {
        value.get("Plan").unwrap_or(&value)
    };
    let raw = pg_node(plan)?;
    Some(flatten_plan(raw, QueryPlanSource::PostgresJson))
}

fn pg_node(plan: &JsonValue) -> Option<RawNode> {
    let node_type = json_str(Some(plan), "Node Type")?;
    let lower = node_type.to_ascii_lowercase();
    let op = if lower.contains("join") || lower == "nested loop" {
        PlanOpKind::Join
    } else if lower.contains("seq scan") {
        PlanOpKind::Scan
    } else if lower.contains("scan") {
        PlanOpKind::IndexLookup
    } else if lower.contains("sort") {
        PlanOpKind::Sort
    } else if lower.contains("aggregate") || lower.contains("group") {
        PlanOpKind::Aggregate
    } else if lower.contains("limit") {
        PlanOpKind::Limit
    } else if lower.contains("union") || lower.contains("append") {
        PlanOpKind::Union
    } else if lower.contains("subquery") || lower.contains("cte") {
        PlanOpKind::Subquery
    } else {
        PlanOpKind::Other
    };

    let mut node = RawNode::new(op, node_type);
    let relation = json_str(Some(plan), "Relation Name");
    let alias = json_str(Some(plan), "Alias");
    node.object = match (relation, alias) {
        (Some(relation), Some(alias)) if relation != alias => Some(format!("{relation} ({alias})")),
        (Some(relation), _) => Some(relation),
        (None, alias) => alias,
    };
    node.index = json_str(Some(plan), "Index Name");
    node.cost = json_f64(Some(plan), "Total Cost");
    node.rows = json_u64(Some(plan), "Plan Rows");
    node.width = json_u64(Some(plan), "Plan Width");
    node.loops = json_u64(Some(plan), "Actual Loops");
    node.condition = json_str(Some(plan), "Index Cond")
        .or_else(|| json_str(Some(plan), "Filter"))
        .or_else(|| json_str(Some(plan), "Hash Cond"))
        .or_else(|| json_str(Some(plan), "Merge Cond"))
        .or_else(|| json_str(Some(plan), "Join Filter"));
    node.detail = json_str(Some(plan), "Join Type")
        .or_else(|| json_str(Some(plan), "Sort Key"))
        .or_else(|| json_str(Some(plan), "Hash Cond"))
        .or_else(|| json_str(Some(plan), "Index Cond"));
    node.extra = json_str(Some(plan), "Sort Key");

    if lower.contains("seq scan") {
        node.issues.push(PlanIssue::FullTableScan);
    }
    if lower == "sort" {
        node.issues.push(PlanIssue::Filesort);
    }

    if let Some(children) = plan.get("Plans").and_then(JsonValue::as_array) {
        for child in children {
            if let Some(child) = pg_node(child) {
                node.children.push(child);
            }
        }
    }
    Some(node)
}

// ---------------------------------------------------------------------------
// SQLite：EXPLAIN QUERY PLAN
// ---------------------------------------------------------------------------

fn parse_sqlite_query_plan(page: &DataPage) -> Option<QueryPlan> {
    let id_index = column_index(page, "id")?;
    let parent_index = column_index(page, "parent")?;
    let detail_index = column_index(page, "detail")?;

    let has_notused = column_index(page, "notused").is_some();
    let mut rows = Vec::new();
    let mut all_plan_like = true;
    for row in &page.rows {
        let detail = row_text(row, Some(detail_index))?;
        let id = row_text(row, Some(id_index))?.parse::<i64>().ok()?;
        let parent = row_text(row, Some(parent_index))
            .and_then(|value| value.parse::<i64>().ok());
        all_plan_like &= sqlite_detail_is_plan(&detail);
        rows.push((id, parent, sqlite_detail_node(&detail)));
    }
    // 普通结果也可能有 id/parent/detail 三列；旧版 SQLite 没有 notused 列，
    // 此时要求 detail 都是计划动作词，避免把用户数据误判成计划。
    if rows.is_empty() || (!has_notused && !all_plan_like) {
        return None;
    }

    let mut children: std::collections::BTreeMap<i64, Vec<usize>> =
        std::collections::BTreeMap::new();
    let mut roots = Vec::new();
    for (index, (_id, parent, _)) in rows.iter().enumerate() {
        match parent {
            Some(parent) if rows.iter().any(|(other, _, _)| other == parent) => {
                children.entry(*parent).or_default().push(index);
            }
            _ => roots.push(index),
        }
    }

    fn build(
        index: usize,
        rows: &[(i64, Option<i64>, RawNode)],
        children: &std::collections::BTreeMap<i64, Vec<usize>>,
        depth: usize,
    ) -> RawNode {
        let (id, _, mut node) = rows[index].clone();
        if depth < MAX_PLAN_NODES
            && let Some(child_indexes) = children.get(&id)
        {
            for child_index in child_indexes {
                node.children
                    .push(build(*child_index, rows, children, depth + 1));
            }
        }
        node
    }

    let mut root_nodes = roots
        .iter()
        .map(|index| build(*index, &rows, &children, 0))
        .collect::<Vec<_>>();
    let raw = if root_nodes.len() == 1 {
        root_nodes.remove(0)
    } else {
        let mut root = RawNode::new(PlanOpKind::Query, "Query");
        root.children = root_nodes;
        root
    };
    Some(flatten_plan(raw, QueryPlanSource::SqliteQueryPlan))
}

fn sqlite_detail_is_plan(detail: &str) -> bool {
    let upper = detail.trim().to_ascii_uppercase();
    ["SCAN ", "SEARCH ", "USE TEMP B-TREE", "CO-ROUTINE", "SUBQUERY", "MATERIALIZE"]
        .iter()
        .any(|prefix| upper.starts_with(prefix))
}

fn sqlite_detail_node(detail: &str) -> RawNode {
    let trimmed = detail.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("SCAN ") {
        let mut node = sqlite_scan_node(trimmed, PlanOpKind::Scan);
        if upper.contains("USING INDEX") || upper.contains("USING COVERING INDEX") {
            node.op = PlanOpKind::IndexLookup;
            node.index = sqlite_index_name(trimmed);
            node.issues.clear();
        } else {
            node.issues.push(PlanIssue::FullTableScan);
        }
        return node;
    }
    if upper.starts_with("SEARCH ") {
        let mut node = sqlite_scan_node(trimmed, PlanOpKind::IndexLookup);
        node.index = sqlite_index_name(trimmed)
            .or_else(|| upper.contains("INTEGER PRIMARY KEY").then(|| "INTEGER PRIMARY KEY".to_string()));
        return node;
    }
    if upper.contains("TEMP B-TREE") {
        let mut node = RawNode::new(PlanOpKind::Sort, trimmed);
        if upper.contains("ORDER BY") {
            node.issues.push(PlanIssue::Filesort);
        }
        return node;
    }
    if upper.starts_with("CO-ROUTINE") || upper.starts_with("SUBQUERY") {
        return RawNode::new(PlanOpKind::Subquery, trimmed);
    }
    if upper.starts_with("MATERIALIZE") {
        return RawNode::new(PlanOpKind::Aggregate, trimmed);
    }
    RawNode::new(PlanOpKind::Other, trimmed)
}

fn sqlite_scan_node(detail: &str, op: PlanOpKind) -> RawNode {
    let mut node = RawNode::new(op, detail);
    node.object = detail
        .split_whitespace()
        .nth(1)
        .map(|name| name.trim_matches(|ch| matches!(ch, '`' | '"' | '\'' | '(')).to_string())
        .filter(|name| !name.is_empty());
    node.condition = detail
        .find('(')
        .and_then(|start| detail.rfind(')').map(|end| detail[start + 1..end].to_string()))
        .filter(|condition| !condition.is_empty());
    node
}

fn sqlite_index_name(detail: &str) -> Option<String> {
    let upper = detail.to_ascii_uppercase();
    for marker in ["USING COVERING INDEX ", "USING INDEX "] {
        if let Some(start) = upper.find(marker) {
            let rest = &detail[start + marker.len()..];
            let end = rest.find(" (").unwrap_or(rest.len());
            let name = rest[..end].trim().trim_matches('`').trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// MySQL 风格表格：普通 EXPLAIN / 手写 EXPLAIN
// ---------------------------------------------------------------------------

fn parse_tabular_explain(page: &DataPage) -> Option<QueryPlan> {
    // 普通查询也可能有名为 table 的列；必须再看到 MySQL EXPLAIN 的特征列，
    // 否则会把用户数据误判成计划，凭空多出「执行计划」页签。
    let looks_like_mysql_explain = column_index(page, "select_type").is_some()
        || (column_index(page, "id").is_some() && column_index(page, "type").is_some());
    if !looks_like_mysql_explain {
        return None;
    }
    let table_index = column_index(page, "table")?;
    let rows = page
        .rows
        .iter()
        .filter(|row| row_text(row, Some(table_index)).is_some())
        .take(MAX_PLAN_NODES)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return None;
    }

    let type_index = column_index(page, "type");
    let key_index = column_index(page, "key");
    let possible_index = column_index(page, "possible_keys");
    let rows_index = column_index(page, "rows");
    let filtered_index = column_index(page, "filtered");
    let extra_index = column_index(page, "Extra");

    let mut nodes = rows
        .iter()
        .map(|row| {
            let access = row_text(row, type_index).unwrap_or_default();
            let lower = access.to_ascii_lowercase();
            let op = match lower.as_str() {
                "all" => PlanOpKind::Scan,
                "index" | "range" | "ref" | "eq_ref" | "const" | "system" | "index_merge" => {
                    PlanOpKind::IndexLookup
                }
                _ => PlanOpKind::Other,
            };
            let mut node = RawNode::new(op, if access.is_empty() { "Table" } else { &access });
            node.object = row_text(row, Some(table_index));
            node.index = row_text(row, key_index);
            node.possible_keys = row_text(row, possible_index);
            node.rows = row_text(row, rows_index).and_then(|value| value.parse::<f64>().ok()).map(|value| value.max(0.) as u64);
            node.filtered = row_text(row, filtered_index)
                .and_then(|value| value.parse::<f64>().ok());
            node.extra = row_text(row, extra_index);
            if lower == "all" {
                node.issues.push(PlanIssue::FullTableScan);
            }
            if node
                .extra
                .as_deref()
                .is_some_and(|extra| extra.to_ascii_lowercase().contains("using filesort"))
            {
                node.issues.push(PlanIssue::Filesort);
            }
            node
        })
        .collect::<Vec<_>>();

    let raw = if nodes.len() == 1 {
        nodes.remove(0)
    } else {
        let mut root = RawNode::new(PlanOpKind::Query, "Query");
        root.children = nodes;
        root
    };
    Some(flatten_plan(raw, QueryPlanSource::TabularExplain))
}

fn merge_extra(extra: &mut Option<String>, value: &str) {
    match extra {
        Some(existing) if existing.contains(value) => {}
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(value);
        }
        None => *extra = Some(value.to_string()),
    }
}

#[cfg(test)]
mod query_plan_tests {
    use super::*;
    use fluxdb_core::{Column, Row};

    fn page(columns: &[&str], rows: Vec<Vec<CellValue>>) -> DataPage {
        DataPage {
            columns: columns
                .iter()
                .map(|name| Column {
                    name: (*name).to_string(),
                    type_name: None,
                    nullable: true,
                    primary_key: false,
                    comment: None,
                })
                .collect(),
            rows: rows
                .into_iter()
                .map(|values| Row { values })
                .collect(),
            offset: 0,
            limit: 200,
            has_more: false,
            incomplete: None,
        }
    }

    fn text(value: &str) -> CellValue {
        CellValue::Text(value.to_string())
    }

    #[test]
    fn mysql_json_builds_tree_with_warnings() {
        let json = r#"{
          "query_block": {
            "select_id": 1,
            "cost_info": { "query_cost": "42.50" },
            "ordering_operation": {
              "using_filesort": true,
              "nested_loop": [
                { "table": {
                    "table_name": "orders",
                    "access_type": "ref",
                    "possible_keys": ["idx_status"],
                    "key": "idx_status",
                    "rows_produced_per_join": 12480,
                    "filtered": "100.00",
                    "cost_info": { "prefix_cost": "18200.00" }
                }},
                { "table": {
                    "table_name": "customers",
                    "access_type": "ALL",
                    "rows_produced_per_join": 50000,
                    "filtered": "100.00",
                    "cost_info": { "prefix_cost": "41500.00", "read_cost": "24300.00" }
                }}
              ]
            }
          }
        }"#;
        let plan = query_plan_from_page(
            DatabaseKind::MySql,
            &page(&["EXPLAIN"], vec![vec![text(json)]]),
        )
        .expect("mysql json plan");

        assert_eq!(plan.source, QueryPlanSource::MySqlJson);
        assert_eq!(plan.total_cost, Some(42.5));
        assert_eq!(plan.nodes[plan.root].op, PlanOpKind::Query);
        assert_eq!(plan.warning_count(), 2);

        let sort = &plan.nodes[plan.nodes[plan.root].children[0]];
        assert_eq!(sort.op, PlanOpKind::Sort);
        assert!(sort.issues.contains(&PlanIssue::Filesort));
        let join = &plan.nodes[sort.children[0]];
        assert_eq!(join.op, PlanOpKind::Join);
        assert_eq!(join.children.len(), 2);
        let scan = &plan.nodes[join.children[1]];
        assert_eq!(scan.object.as_deref(), Some("customers"));
        assert!(scan.issues.contains(&PlanIssue::FullTableScan));
        assert_eq!(scan.parent, Some(join.id));
    }

    #[test]
    fn mysql_const_table_message_is_kept_as_detail() {
        let json = r#"{
          "query_block": {
            "select_id": 1,
            "message": "no matching row in const table"
          }
        }"#;
        let plan = query_plan_from_page(
            DatabaseKind::MySql,
            &page(&["EXPLAIN"], vec![vec![text(json)]]),
        )
        .expect("mysql message plan");

        assert_eq!(plan.nodes[plan.root].op, PlanOpKind::Query);
        assert_eq!(
            plan.nodes[plan.root].detail.as_deref(),
            Some("no matching row in const table")
        );
        assert!(plan.nodes[plan.root].children.is_empty());
        assert_eq!(plan.warning_count(), 0);
    }

    #[test]
    fn mysql_const_table_builds_single_lookup_child() {
        let json = r#"{
          "query_block": {
            "select_id": 1,
            "cost_info": { "query_cost": "1.00" },
            "table": {
              "table_name": "3d_countries",
              "access_type": "const",
              "possible_keys": ["PRIMARY"],
              "key": "PRIMARY",
              "rows_examined_per_scan": 1,
              "rows_produced_per_join": 1,
              "filtered": "100.00",
              "cost_info": { "prefix_cost": "0.00" }
            }
          }
        }"#;
        let plan = query_plan_from_page(
            DatabaseKind::MySql,
            &page(&["EXPLAIN"], vec![vec![text(json)]]),
        )
        .expect("mysql const plan");

        assert_eq!(plan.total_cost, Some(1.0));
        let root = &plan.nodes[plan.root];
        assert_eq!(root.children.len(), 1);
        let lookup = &plan.nodes[root.children[0]];
        assert_eq!(lookup.op, PlanOpKind::IndexLookup);
        assert_eq!(lookup.object.as_deref(), Some("3d_countries"));
        assert_eq!(lookup.index.as_deref(), Some("PRIMARY"));
        assert_eq!(lookup.rows, Some(1));
    }

    #[test]
    fn postgres_json_builds_nested_plan() {
        let json = r#"[{
          "Plan": {
            "Node Type": "Hash Join",
            "Join Type": "Inner",
            "Total Cost": 42.5,
            "Plan Rows": 100,
            "Plan Width": 40,
            "Hash Cond": "(c.id = o.customer_id)",
            "Plans": [
              { "Node Type": "Seq Scan", "Relation Name": "orders", "Alias": "o",
                "Total Cost": 18.2, "Plan Rows": 12480, "Filter": "(status = 'paid')" },
              { "Node Type": "Index Scan", "Relation Name": "customers", "Alias": "c",
                "Index Name": "customers_pkey", "Total Cost": 24.3, "Plan Rows": 50000,
                "Index Cond": "(id = o.customer_id)" }
            ]
          }
        }]"#;
        let plan = query_plan_from_page(
            DatabaseKind::Postgres,
            &page(&["QUERY PLAN"], vec![vec![text(json)]]),
        )
        .expect("postgres json plan");

        assert_eq!(plan.source, QueryPlanSource::PostgresJson);
        assert_eq!(plan.nodes[plan.root].op, PlanOpKind::Join);
        assert_eq!(plan.total_cost, Some(42.5));
        let first = &plan.nodes[plan.nodes[plan.root].children[0]];
        assert_eq!(first.object.as_deref(), Some("orders (o)"));
        assert_eq!(first.condition.as_deref(), Some("(status = 'paid')"));
        assert!(first.issues.contains(&PlanIssue::FullTableScan));
        let second = &plan.nodes[plan.nodes[plan.root].children[1]];
        assert_eq!(second.index.as_deref(), Some("customers_pkey"));
    }

    #[test]
    fn sqlite_query_plan_uses_parent_links() {
        let page = page(
            &["id", "parent", "notused", "detail"],
            vec![
                vec![CellValue::I64(2), CellValue::I64(0), CellValue::I64(0), text("SCAN customers")],
                vec![CellValue::I64(6), CellValue::I64(2), CellValue::I64(0), text("SEARCH orders USING INDEX idx_status (status=?)")],
                vec![CellValue::I64(8), CellValue::I64(2), CellValue::I64(0), text("USE TEMP B-TREE FOR ORDER BY")],
            ],
        );
        let plan = query_plan_from_page(DatabaseKind::Sqlite, &page).expect("sqlite plan");

        assert_eq!(plan.nodes[plan.root].object.as_deref(), Some("customers"));
        assert_eq!(plan.nodes[plan.root].op, PlanOpKind::Scan);
        assert!(plan.nodes[plan.root].issues.contains(&PlanIssue::FullTableScan));
        assert_eq!(plan.nodes[plan.root].children.len(), 2);
        let index = &plan.nodes[plan.nodes[plan.root].children[0]];
        assert_eq!(index.op, PlanOpKind::IndexLookup);
        assert_eq!(index.index.as_deref(), Some("idx_status"));
        assert_eq!(index.condition.as_deref(), Some("status=?"));
        let sort = &plan.nodes[plan.nodes[plan.root].children[1]];
        assert_eq!(sort.op, PlanOpKind::Sort);
    }

    #[test]
    fn tabular_explain_falls_back_to_flat_nodes() {
        let page = page(
            &["id", "select_type", "table", "type", "possible_keys", "key", "rows", "filtered", "Extra"],
            vec![
                vec![CellValue::I64(1), text("SIMPLE"), text("customers"), text("ALL"), CellValue::Null, CellValue::Null, CellValue::I64(50000), text("10.00"), text("Using where")],
                vec![CellValue::I64(1), text("SIMPLE"), text("orders"), text("ref"), text("idx_status"), text("idx_status"), CellValue::I64(12480), text("100.00"), text("Using filesort")],
            ],
        );
        let plan = query_plan_from_page(DatabaseKind::MySql, &page).expect("tabular plan");

        assert_eq!(plan.source, QueryPlanSource::TabularExplain);
        assert_eq!(plan.nodes[plan.root].op, PlanOpKind::Query);
        assert_eq!(plan.nodes[plan.root].children.len(), 2);
        assert_eq!(plan.warning_count(), 2);
        assert_eq!(plan.estimated_rows, Some(50000));
    }

    #[test]
    fn ordinary_result_is_not_mistaken_for_plan() {
        let plain = page(
            &["id", "name"],
            vec![vec![CellValue::I64(1), text("alice")]],
        );
        assert_eq!(query_plan_from_page(DatabaseKind::MySql, &plain), None);

        // 普通结果里恰好有 table 列，但不是 EXPLAIN，也不能多出计划页签。
        let table_like = page(
            &["table", "rows"],
            vec![vec![text("menu"), CellValue::I64(12)]],
        );
        assert_eq!(query_plan_from_page(DatabaseKind::MySql, &table_like), None);
    }
}
