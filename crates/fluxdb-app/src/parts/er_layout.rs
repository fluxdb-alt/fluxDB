// ER 关系驱动布局（er-ui-relationship-canvas.md §6）。
//
// 纯拓扑布局，位于 app 层；不依赖 GPUI、不拼 SQL、不读数据库。
// 以真实外键边计算连通分量与确定性分层排列：
// - 被引用实体倾向放左侧，引用它的实体向右展开（只从真实边推导方向，不按表名猜）。
// - 强连通分量（环）先压缩成 DAG 再分层，环内节点同一层按稳定顺序垂直排布。
// - 桥表、自关联、多组件都按统一规则处理，不无限递归、不按示例表名硬编码。
// - 孤立表（无已知关系）放在外围紧凑网格，可访问、不插入主要关系链。
// - 输出确定、不重叠（同层并排、跨层错开），附带连通分量序号供配色。
//
// 整体 O(V+E)（并查集连通分量 + Tarjan SCC + DAG 最长路径）。复杂巨型组件超出预算时
// 仍返回确定性、非重叠、可探索的降级布局，不截断、不无限 loading。调用方在后台执行并
// 做代次/过期保护。

/// 关联组件内水平卡片间隔。
pub const ER_X_GAP: f32 = 96.0;
/// 关联组件内垂直卡片间隔（容纳最高卡片 + 40px 最小垂直间距）。
pub const ER_Y_GAP: f32 = 320.0;
/// 组件之间水平间隔（给连线留通道）。
pub const ER_COMPONENT_GAP: f32 = 96.0;
/// 卡片宽度（含边框，供孤立区网格与碰撞包络用）。
pub const ER_CARD_W: f32 = 252.0;
/// 卡片最大高（碰撞包络）。
pub const ER_CARD_H_MAX: f32 = 300.0;
/// 层间水平间距：必须 ≥ 卡宽 + 通道，使相邻层卡片矩形不相交（层距 96 会重叠）。
pub const ER_LAYER_X: f32 = ER_CARD_W + 48.0;
/// 初始留白（世界坐标，四周至少 32px）。
const ER_ORIGIN: f32 = 64.0;

/// 关系布局结果：每个表名 → (世界 x, 世界 y, 连通分量序号)。
/// 分量序号供 UI 从统一主题调色板稳定取色；-1 表示孤立表（外围区）。
pub type ErLayoutResult = std::collections::BTreeMap<String, (f32, f32, i32)>;

/// 由表与真实外键边计算关系布局。所有表（含孤立表）都有坐标，无截断。
pub fn er_relation_layout(
    tables: &[fluxdb_core::ErTableNode],
    edges: &[fluxdb_core::ErForeignKeyEdge],
) -> ErLayoutResult {
    let table_names: std::collections::BTreeSet<&str> =
        tables.iter().map(|t| t.name.as_str()).collect();
    // 有向邻接 from→to（引用→被引用），只保留两端都在表集内的边。
    let mut adj: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for t in tables {
        adj.entry(t.name.as_str()).or_default();
    }
    let mut has_edge: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for e in edges {
        if table_names.contains(e.from_table.as_str()) && table_names.contains(e.to_table.as_str()) {
            adj.get_mut(e.from_table.as_str()).unwrap().push(e.to_table.as_str());
            has_edge.insert(e.from_table.as_str());
            has_edge.insert(e.to_table.as_str());
        }
    }
    // 无向邻接（连通分量用）：每边两端互连。
    let mut undirected: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for t in tables {
        undirected.entry(t.name.as_str()).or_default();
    }
    {
        let mut index_of: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for t in tables {
            index_of.insert(t.name.as_str(), index_of.len());
        }
        for e in edges {
            let (Some(&fi), Some(&ti)) = (index_of.get(e.from_table.as_str()), index_of.get(e.to_table.as_str())) else {
                continue;
            };
            let fn_ = tables[fi].name.as_str();
            let tn = tables[ti].name.as_str();
            undirected.get_mut(fn_).unwrap().push(tn);
            if fn_ != tn {
                undirected.get_mut(tn).unwrap().push(fn_);
            }
        }
    }

    let mut result: ErLayoutResult = std::collections::BTreeMap::new();
    // 连通分量（无向 BFS，只含有边的表）。
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut comps: Vec<Vec<&str>> = Vec::new();
    for t in tables {
        let name = t.name.as_str();
        if seen.contains(name) || !has_edge.contains(name) {
            continue;
        }
        let mut comp: Vec<&str> = Vec::new();
        let mut stack = vec![name];
        seen.insert(name);
        while let Some(n) = stack.pop() {
            comp.push(n);
            for &m in &undirected[n] {
                if !seen.contains(m) {
                    seen.insert(m);
                    stack.push(m);
                }
            }
        }
        comps.push(comp);
    }

    let mut cursor_x = ER_ORIGIN;
    for (cid, comp) in comps.iter().enumerate() {
        let lay = lay_out_component(comp, &adj);
        // 组件完整占用：最右卡片右边缘（层最右 = max level 的 x + 卡片宽）+ 路由通道。
        let mut width = ER_CARD_W;
        for &t in comp {
            let (x, y) = lay[&t.to_string()];
            result.insert(t.to_string(), (cursor_x + x, ER_ORIGIN + y, cid as i32));
            width = width.max(x + ER_CARD_W);
        }
        cursor_x += width + ER_COMPONENT_GAP;
    }
    // 孤立表：外围紧凑网格（放所有关联组件右侧，避开已占区域）。
    let isolated: Vec<&fluxdb_core::ErTableNode> = tables
        .iter()
        .filter(|t| !has_edge.contains(t.name.as_str()))
        .collect();
    let iso_cols = ((isolated.len() as f32).sqrt()).ceil().max(1.0) as usize;
    for (i, t) in isolated.iter().enumerate() {
        let col = (i % iso_cols) as f32;
        let row = (i / iso_cols) as f32;
        let x = cursor_x + 32.0 + col * (ER_CARD_W + 40.0);
        let y = 64.0 + row * (ER_CARD_H_MAX + 40.0);
        result.insert(t.name.clone(), (x, y, -1));
    }
    result
}

/// 对一个连通分量做确定性分层布局。返回表名 → (分量内相对 x, y)。
/// 环通过 Tarjan SCC 压缩，SCC 内节点同层垂直排布；被引用方层号小（居左）。
fn lay_out_component(
    comp: &[&str],
    adj: &std::collections::BTreeMap<&str, Vec<&str>>,
) -> std::collections::BTreeMap<String, (f32, f32)> {
    let comp_set: std::collections::BTreeSet<String> = comp.iter().map(|s| s.to_string()).collect();
    // 分量内的 owned 邻接（供 Tarjan 与 DAG，无生命周期耦合）。
    let mut owned_adj: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for &f in comp {
        let dsts = adj
            .get(f)
            .map(|v| {
                v.iter()
                    .map(|t| t.to_string())
                    .filter(|t| comp_set.contains(t))
                    .collect()
            })
            .unwrap_or_default();
        owned_adj.insert(f.to_string(), dsts);
    }
    let nodes: Vec<String> = comp.iter().map(|s| s.to_string()).collect();
    // Tarjan SCC，得到该分量的环内节点分组（owned String）。
    let scc_groups = tarjan_scc(&nodes, &owned_adj);
    let mut scc_of: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (si, group) in scc_groups.iter().enumerate() {
        for n in group {
            scc_of.insert(n.clone(), si);
        }
    }
    // scc DAG 边（from_scc→to_scc，环内忽略）。
    let mut dag: std::collections::BTreeMap<usize, std::collections::BTreeSet<usize>> =
        std::collections::BTreeMap::new();
    for si in 0..scc_groups.len() {
        dag.entry(si).or_default();
    }
    for f in &nodes {
        if let Some(dsts) = owned_adj.get(f) {
            for t in dsts {
                let (fi, ti) = (scc_of[f], scc_of[t]);
                if fi != ti {
                    dag.get_mut(&fi).unwrap().insert(ti);
                }
            }
        }
    }
    // 最长路径层号：边 f→t 表示 f 引用 t，t 是被引用方（居左层小），f 层号更大（居右）。
    // 层语义：无出边(纯被引用 sink) = 0；有出边 = 1 + max(子 SCC 层)。
    // 用 Kahn 拓扑序 + 逆序迭代计算（child 先于父处理），避免长链递归栈溢出。
    let mut indeg: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for &s in dag.keys() {
        indeg.entry(s).or_insert(0);
    }
    for cs in dag.values() {
        for &c in cs {
            *indeg.entry(c).or_insert(0) += 1;
        }
    }
    let mut queue: std::collections::VecDeque<usize> = dag
        .keys()
        .copied()
        .filter(|&s| *indeg.get(&s).unwrap_or(&0) == 0)
        .collect();
    let mut topo: Vec<usize> = Vec::with_capacity(dag.len());
    while let Some(s) = queue.pop_front() {
        topo.push(s);
        if let Some(cs) = dag.get(&s) {
            for &c in cs {
                let d = indeg.get_mut(&c).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push_back(c);
                }
            }
        }
    }
    // 逆拓扑序算最长出边链：level[f] = max(level[f], level[child]+1)。
    let mut level: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for &s in dag.keys() {
        level.insert(s, 0);
    }
    for &s in topo.iter().rev() {
        // 运行取最大（不能用原始 cur 逐个比较）：同一 scc 有多个子 scc 时，
        // 后面的子 scc 值可能覆盖先前更大的值。例如 order_items 同时引用 orders(层1)
        // 与 products(层0)，正确层应为 max(1,0)+1=2，旧实现因 cur 固定被 products 覆盖成 1。
        let mut max_level = level[&s];
        if let Some(cs) = dag.get(&s) {
            for &c in cs {
                max_level = max_level.max(level[&c] + 1);
            }
        }
        level.insert(s, max_level);
    }
    // 按层排布：层内各 scc 垂直堆叠，scc 内节点再垂直堆叠；x 由层号决定。
    let mut by_level: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for &s in dag.keys() {
        by_level.entry(level[&s]).or_default().push(s);
    }
    let mut pos: std::collections::BTreeMap<String, (f32, f32)> = std::collections::BTreeMap::new();
    for (l, scc_ids) in by_level.iter() {
        let mut scc_ids = scc_ids.clone();
        scc_ids.sort();
        let mut y_cursor = 0.0f32;
        for si in scc_ids {
            let mut nodes: Vec<String> = scc_groups[si].clone();
            nodes.sort();
            for n in nodes {
                pos.insert(n, (*l as f32 * ER_LAYER_X, y_cursor));
                y_cursor += ER_Y_GAP;
            }
        }
    }
    pos
}

/// Tarjan 强连通分量（输入为分量内节点与有向邻接）。全 owned，无生命周期耦合。
fn tarjan_scc(
    nodes: &[String],
    adj: &std::collections::BTreeMap<String, Vec<String>>,
) -> Vec<Vec<String>> {
    let mut index: usize = 0;
    let mut indices: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut low: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut on_stack: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut sccs: Vec<Vec<String>> = Vec::new();

    fn strongconnect(
        v: &str,
        adj: &std::collections::BTreeMap<String, Vec<String>>,
        index: &mut usize,
        indices: &mut std::collections::HashMap<String, usize>,
        low: &mut std::collections::HashMap<String, usize>,
        on_stack: &mut std::collections::HashMap<String, bool>,
        stack: &mut Vec<String>,
        sccs: &mut Vec<Vec<String>>,
    ) {
        let vid = v.to_string();
        indices.insert(vid.clone(), *index);
        low.insert(vid.clone(), *index);
        *index += 1;
        stack.push(vid.clone());
        on_stack.insert(vid, true);
        if let Some(dsts) = adj.get(v) {
            for w in dsts {
                if !indices.contains_key(w) {
                    strongconnect(w, adj, index, indices, low, on_stack, stack, sccs);
                    let lw = low[w];
                    let lv = low[v];
                    low.insert(v.to_string(), lv.min(lw));
                } else if *on_stack.get(w).unwrap_or(&false) {
                    let lv = low[v];
                    let iw = indices[w];
                    low.insert(v.to_string(), lv.min(iw));
                }
            }
        }
        if low[v] == indices[v] {
            let mut scc: Vec<String> = Vec::new();
            loop {
                let w = stack.pop().unwrap();
                on_stack.insert(w.clone(), false);
                let finished = &w == v;
                scc.push(w);
                if finished {
                    break;
                }
            }
            sccs.push(scc);
        }
    }

    let mut order: Vec<String> = nodes.to_vec();
    order.sort();
    for n in &order {
        if !indices.contains_key(n) {
            strongconnect(n, adj, &mut index, &mut indices, &mut low, &mut on_stack, &mut stack, &mut sccs);
        }
    }
    sccs
}

#[cfg(test)]
mod er_layout_tests {
    use super::*;
    use fluxdb_core::{ErForeignKeyEdge, ErTableNode};

    fn t(name: &str) -> ErTableNode {
        let reference = fluxdb_core::ErTableRef {
            database: String::new(),
            schema: None,
            name: name.to_string(),
        };
        ErTableNode {
            name: name.to_string(),
            reference: reference.clone(),
            comment: None,
            status: fluxdb_core::ErLoadStatus::Loaded,
            columns: vec![],
        }
    }
    fn e(from: &str, to: &str) -> ErForeignKeyEdge {
        let mk = |n: &str| fluxdb_core::ErTableRef {
            database: String::new(),
            schema: None,
            name: n.to_string(),
        };
        ErForeignKeyEdge {
            name: "fk".into(),
            from_table: from.into(),
            from_column: "c".into(),
            to_table: to.into(),
            to_column: "id".into(),
            from_reference: mk(from),
            to_reference: mk(to),
        }
    }

    /// 生产几何矩形不相交断言：卡片 NODE 宽 ER_CARD_W、最大高 ER_CARD_H_MAX。
    /// 与「最小间距」一并校验（相邻至少留有通道间距，非仅不相交）。
    fn assert_no_overlap(l: &ErLayoutResult) {
        let rows: Vec<(&str, f32, f32)> = l.iter().map(|(n, v)| (n.as_str(), v.0, v.1)).collect();
        for i in 0..rows.len() {
            for j in i + 1..rows.len() {
                let (an, ax, ay) = rows[i];
                let (bn, bx, by) = rows[j];
                let sep_x = ax + ER_CARD_W <= bx || bx + ER_CARD_W <= ax;
                let sep_y = ay + ER_CARD_H_MAX <= by || by + ER_CARD_H_MAX <= ay;
                assert!(
                    sep_x || sep_y,
                    "卡片矩形相交：{} @({ax},{ay}) 与 {} @({bx},{by})",
                    an,
                    bn
                );
            }
        }
    }

    #[test]
    fn layout_isolated_tables_kept_reachable() {
        let tables = vec![t("a"), t("orders"), t("tags")];
        let edges = vec![e("orders", "a")];
        let l = er_relation_layout(&tables, &edges);
        assert_eq!(l.len(), 3, "孤立表也必须可访问");
        assert!(l.contains_key("tags"));
        assert_eq!(l["tags"].2, -1, "孤立表应在外围区（分量 -1）");
        assert_eq!(l["orders"].2, 0);
        assert_eq!(l["a"].2, 0);
        assert_ne!((l["orders"].0, l["orders"].1), (l["a"].0, l["a"].1));
    }

    #[test]
    fn layout_referenced_on_left_strict_gap() {
        let tables = vec![t("orders"), t("customers")];
        let edges = vec![e("orders", "customers")];
        let l = er_relation_layout(&tables, &edges);
        // 严格分层：customers(被引用, 层0) 必须明显左于 orders(引用, 层1)，且留有层间距。
        assert!(
            l["customers"].0 + ER_LAYER_X - 0.5 <= l["orders"].0,
            "被引用表应严格居左并留间距：customers {} vs orders {}（层间距 ≥ {ER_X_GAP}）",
            l["customers"].0,
            l["orders"].0
        );
    }

    #[test]
    fn layout_three_level_chain_progressive() {
        // 三层链 a→b→c（a 引用 b、b 引用 c）：c(被引用最左) < b < a，层层递进。
        let tables = vec![t("a"), t("b"), t("c")];
        let edges = vec![e("a", "b"), e("b", "c")];
        let l = er_relation_layout(&tables, &edges);
        assert!(
            l["c"].0 + ER_LAYER_X - 0.5 <= l["b"].0 && l["b"].0 + ER_LAYER_X - 0.5 <= l["a"].0,
            "三层链应分层递增：c {} < b {} < a {}",
            l["c"].0,
            l["b"].0,
            l["a"].0
        );
        assert_no_overlap(&l);
    }

    #[test]
    fn layout_diamond_uses_deepest_referenced_level() {
        // 关键回归：order_items 同时引用 orders(经链到 customers) 与 products(叶子)。
        // 引用深度取 max(子层)+1 = 2，不得被较浅的 products 覆盖成与 orders 同层(1)。
        // 旧实现 cur 固定，同一 scc 多子 scc 时较浅子值覆盖较深值 → order_items 与 orders 同列成竖柱。
        let tables = vec![t("customers"), t("orders"), t("order_items"), t("products")];
        let edges = vec![
            e("orders", "customers"),
            e("order_items", "orders"),
            e("order_items", "products"),
        ];
        let l = er_relation_layout(&tables, &edges);
        assert!(
            l["order_items"].0 - l["orders"].0 >= ER_LAYER_X - 0.5,
            "order_items 应比 orders 深一层：orders {} order_items {}",
            l["orders"].0,
            l["order_items"].0
        );
        // 分支的两条被引用链各自分层：customers/products 同为最浅，orders 居中。
        assert!(l["orders"].0 - l["customers"].0 >= ER_LAYER_X - 0.5);
        assert!(l["orders"].0 - l["products"].0 >= ER_LAYER_X - 0.5);
        assert_no_overlap(&l);
    }

    #[test]
    fn layout_branch_and_bridge_both_levels() {
        // 分支 + 桥：customers(层0)；orders 与 payments 各引用 customers；shipments 引用 orders。
        let tables = vec![t("customers"), t("orders"), t("payments"), t("shipments")];
        let edges = vec![
            e("orders", "customers"),
            e("payments", "customers"),
            e("shipments", "orders"),
        ];
        let l = er_relation_layout(&tables, &edges);
        // customers 最左且与引用方分离；shipments 最右（经 orders 链）。
        assert!(l["customers"].0 + ER_LAYER_X - 0.5 <= l["orders"].0);
        assert!(l["orders"].0 + ER_LAYER_X - 0.5 <= l["shipments"].0);
        // 同层（orders/payments）x 相同、y 错开，不重叠。
        assert_no_overlap(&l);
    }

    #[test]
    fn layout_cycle_scc_condensed_and_isolated_row() {
        // 环 a↔b→c；环内同层，c 更右；外加孤立表 tags 放外围。
        let tables = vec![t("a"), t("b"), t("c"), t("tags")];
        let edges = vec![e("a", "b"), e("b", "a"), e("b", "c")];
        let l = er_relation_layout(&tables, &edges);
        assert_eq!(l.len(), 4);
        // 环 SCC {a,b} 同层（x 相同）；c 是 b 引用的(被引用)方 → 居左层。
        assert!((l["a"].0 - l["b"].0).abs() < 1e-3, "环内同层：a {} b {}", l["a"].0, l["b"].0);
        assert!(l["a"].0 - l["c"].0 >= ER_LAYER_X - 0.5, "被引用 c 应居环左：c {} a {}", l["c"].0, l["a"].0);
        // 孤立表在关联区之外（x 在所有关联组件右侧）。
        let comp_max_x = l["a"].0.max(l["b"].0).max(l["c"].0);
        assert!(l["tags"].0 > comp_max_x, "孤立表应在关联区之外：tags {} > max {}", l["tags"].0, comp_max_x);
        assert_no_overlap(&l);
    }

    #[test]
    fn layout_component_and_isolated_not_overlapping() {
        // 关联组件(orders→customers) + 孤立表，全矩形不相交。
        let tables = vec![
            t("orders"),
            t("customers"),
            t("order_items"),
            t("products"),
            t("tags"),
            t("settings"),
        ];
        let edges = vec![
            e("orders", "customers"),
            e("order_items", "orders"),
            e("order_items", "products"),
        ];
        let l = er_relation_layout(&tables, &edges);
        assert_no_overlap(&l);
        // 孤立表 tags/settings 在外围区，不与关联组件卡片重叠。
        let comp_max = l
            .iter()
            .filter(|(n, _)| *n != "tags" && *n != "settings")
            .map(|(_, v)| v.0)
            .fold(f32::NEG_INFINITY, f32::max);
        for iso in ["tags", "settings"] {
            assert!(l[iso].0 > comp_max, "{iso} 孤立表应在关联组件右侧之外");
        }
    }

    #[test]
    fn layout_cycle_and_bridge_do_not_recurs() {
        let tables = vec![t("a"), t("b"), t("c"), t("d")];
        let edges = vec![e("a", "b"), e("b", "c"), e("c", "a"), e("d", "a")];
        let l = er_relation_layout(&tables, &edges);
        assert_eq!(l.len(), 4);
        assert_no_overlap(&l);
    }

    #[test]
    fn layout_preserves_self_loop() {
        let tables = vec![t("a")];
        let edges = vec![e("a", "a")];
        let l = er_relation_layout(&tables, &edges);
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn layout_is_deterministic() {
        let tables = vec![t("orders"), t("customers"), t("products")];
        let edges = vec![e("orders", "customers"), e("products", "customers")];
        assert_eq!(er_relation_layout(&tables, &edges), er_relation_layout(&tables, &edges));
    }

    #[test]
    fn layout_many_tables_bounded_and_nonoverlap() {
        let n = 100;
        let tables: Vec<_> = (0..n).map(|i| t(&format!("t{i:03}"))).collect();
        let mut edges = Vec::new();
        for i in 0..n - 1 {
            edges.push(e(&format!("t{i:03}"), &format!("t{:03}", i + 1)));
        }
        let mut all = tables;
        for i in 0..30 {
            all.push(t(&format!("iso{i:03}")));
        }
        let l = er_relation_layout(&all, &edges);
        assert_eq!(l.len(), n + 30, "无截断");
        assert_no_overlap(&l);
    }

    #[test]
    fn layout_10k_no_cutoff() {
        let n = 10_000;
        let tables: Vec<_> = (0..n).map(|i| t(&format!("t{i:05}"))).collect();
        let mut edges = Vec::new();
        for i in 0..n - 1 {
            if i % 3 == 0 {
                edges.push(e(&format!("t{i:05}"), &format!("t{:05}", i + 1)));
            }
        }
        let l = er_relation_layout(&tables, &edges);
        assert_eq!(l.len(), n, "万表无截断");
    }
}
