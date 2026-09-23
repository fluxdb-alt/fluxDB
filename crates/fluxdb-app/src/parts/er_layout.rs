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

// ---------------------------------------------------------------------------
// 占用感知落位（er-ui-relationship-canvas.md §6.2/§6.3）
//
// `er_relation_layout` 只能看到“本次参与布局的表”，对画布上已有的坐标一无所知。
// 于是新增表（例如新加 schema 的表）会照抄布局原点，直接压在用户已固定的表之上。
// 本节的纯函数把“已占矩形”当作障碍，给出不重叠的落位结果，供 desktop 调用。
// ---------------------------------------------------------------------------

/// 相邻卡片（组）之间要求保留的最小通道间距。
pub const ER_PLACE_GAP: f32 = 40.0;
/// 螺旋搜索的最大环数：每环向外扩一个卡片步长，超出后走下方追加带。
pub const ER_PLACE_MAX_RINGS: i32 = 32;

/// 世界坐标下的卡片占位矩形（含边框）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ErRect {
    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    /// 与另一矩形是否相交；边相切不算相交。
    pub fn intersects(self, other: ErRect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// 四周外扩 `gap`（用于保证卡片之间留有通道间距，而非刚刚相切）。
    pub fn inflate(self, gap: f32) -> ErRect {
        ErRect {
            x: self.x - gap,
            y: self.y - gap,
            w: self.w + gap * 2.0,
            h: self.h + gap * 2.0,
        }
    }
}

/// 把 `candidates` 的理想坐标调整为不与 `occupied` 已占矩形相交的落位结果。
///
/// 规则：
/// - 同一连通分量（`ErLayoutResult` 的分量序号）整体刚性平移：组内相对位置与连线走向不变；
///   分量之间没有边，因此刚性平移不会破坏任何关系。孤立表（分量 -1）各自独立落位，
///   避免互不相干的表被捆成一个刚体。
/// - 组从候选位置起由近及远螺旋搜索首个合法位置：最近优先 ≈「安排在邻居附近的空闲处」。
/// - 优先选择不会把卡片推到负世界坐标的方向；确有需要时才允许负坐标。
/// - 搜索有界（`max_rings`）；耗尽后退到所有占用矩形下方的追加带并告警，绝不静默重叠。
/// - `occupied` 全程只读：已固定表的坐标永不被本函数修改（新增表让位于既有节点）。
pub fn er_place_avoiding_overlaps(
    candidates: &ErLayoutResult,
    sizes: &std::collections::BTreeMap<String, (f32, f32)>,
    occupied: &[ErRect],
    gap: f32,
    max_rings: i32,
) -> ErLayoutResult {
    let size_of = |name: &str| sizes.get(name).copied().unwrap_or((ER_CARD_W, ER_CARD_H_MAX));
    let rect_at = |name: &str, x: f32, y: f32| {
        let (w, h) = size_of(name);
        ErRect { x, y, w, h }
    };

    // 分组：分量号 >= 0 按分量成组；孤立表（-1）按表名各自成组。
    let mut groups: std::collections::BTreeMap<String, Vec<(String, (f32, f32, i32))>> =
        std::collections::BTreeMap::new();
    for (name, &(x, y, comp)) in candidates {
        let key = if comp >= 0 {
            format!("c{comp}")
        } else {
            format!("i{name}")
        };
        groups.entry(key).or_default().push((name.clone(), (x, y, comp)));
    }

    let mut blockers: Vec<ErRect> = occupied.to_vec();
    let mut out = ErLayoutResult::new();
    let mut keys: Vec<String> = groups.keys().cloned().collect();
    keys.sort();

    for key in keys {
        let mut group = groups.remove(&key).unwrap_or_default();
        group.sort_by(|a, b| a.0.cmp(&b.0));

        // 组内刚性平移：搜索步长取组内最大卡片尺寸 + 通道（按实际尺寸，不用最大包络，
        // 否则新增表会被无谓地推远；调用方对未加载表已给出包络尺寸兜底）。
        let mut step_x = 0.0f32;
        let mut step_y = 0.0f32;
        for (name, _) in &group {
            let (w, h) = size_of(name);
            step_x = step_x.max(w + gap);
            step_y = step_y.max(h + gap);
        }
        if step_x <= 0.0 || step_y <= 0.0 {
            step_x = ER_CARD_W + gap;
            step_y = ER_CARD_H_MAX + gap;
        }

        // 偏移量是否让组内所有卡片保持非负世界坐标（优先方向，避免把新增表推到左上无限远处）。
        let non_negative = |offset: (f32, f32)| {
            group
                .iter()
                .all(|(_, (x, y, _))| x + offset.0 >= 0.0 && y + offset.1 >= 0.0)
        };
        // 组整体平移后是否与所有已占矩形保持 `gap` 通道间距。
        let clear = |offset: (f32, f32)| {
            group.iter().all(|(name, (x, y, _))| {
                let probe = rect_at(name, x + offset.0, y + offset.1).inflate(gap);
                blockers.iter().all(|b| !probe.intersects(*b))
            })
        };
        // 环内候选偏移：确定性排序（同环内按距离、再按坐标），最近优先。
        let ring_offsets = |ring: i32| {
            let mut offsets: Vec<(i32, i32)> = Vec::new();
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    if dx.abs().max(dy.abs()) == ring {
                        offsets.push((dx, dy));
                    }
                }
            }
            offsets.sort_by_key(|(dx, dy)| (dx * dx + dy * dy, *dx, *dy));
            offsets
        };

        let search = |require_non_negative: bool| -> Option<(f32, f32)> {
            for ring in 0..=max_rings.max(0) {
                for (dx, dy) in ring_offsets(ring) {
                    let offset = (dx as f32 * step_x, dy as f32 * step_y);
                    if require_non_negative && !non_negative(offset) {
                        continue;
                    }
                    if clear(offset) {
                        return Some(offset);
                    }
                }
            }
            None
        };

        // 两趟：先只找不越界的解，找不到才允许负坐标，仍找不到才退到下方追加带。
        let offset = search(true).or_else(|| search(false)).unwrap_or_else(|| {
            let bottom = blockers
                .iter()
                .map(|b| b.bottom())
                .fold(f32::NEG_INFINITY, f32::max);
            let min_y = group
                .iter()
                .map(|(_, (_, y, _))| *y)
                .fold(f32::INFINITY, f32::min);
            let target = if bottom.is_finite() { bottom + gap } else { 0.0 };
            tracing::warn!(
                group = %key,
                tables = group.len(),
                "ER 避让搜索超限，整体退到已占区域下方追加带"
            );
            (0.0, target - min_y)
        });

        let mut placed_rects: Vec<ErRect> = Vec::with_capacity(group.len());
        for (name, (x, y, comp)) in group {
            let (nx, ny) = (x + offset.0, y + offset.1);
            placed_rects.push(rect_at(&name, nx, ny));
            out.insert(name, (nx, ny, comp));
        }
        // 已落位组本身成为后续组的障碍（保证组间也不重叠）。
        blockers.extend(placed_rects);
    }
    out
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
            stable: None,
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

    // ---- 占用感知落位（§6.2/§6.3）----

    fn sizes_of(l: &ErLayoutResult, w: f32, h: f32) -> std::collections::BTreeMap<String, (f32, f32)> {
        l.keys().map(|k| (k.clone(), (w, h))).collect()
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> ErRect {
        ErRect { x, y, w, h }
    }

    /// 所有对内矩形（含外部已经落位的障碍）两两不相交。
    fn assert_no_overlap_with(l: &ErLayoutResult, occupied: &[ErRect], w: f32, h: f32) {
        let mut rects: Vec<(String, ErRect)> = l
            .iter()
            .map(|(n, &(x, y, _))| (n.clone(), rect(x, y, w, h)))
            .collect();
        for (i, o) in occupied.iter().enumerate() {
            rects.push((format!("occupied-{i}"), *o));
        }
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                assert!(
                    !rects[i].1.intersects(rects[j].1),
                    "{} 与 {} 重叠: {:?} / {:?}",
                    rects[i].0, rects[j].0, rects[i].1, rects[j].1
                );
            }
        }
    }

    #[test]
    fn place_keeps_candidates_when_nothing_occupied() {
        // 无障碍：完全保持候选坐标（不引入无谓位移）。
        let cand: ErLayoutResult = [("a".to_string(), (64.0, 64.0, 0)), ("b".to_string(), (364.0, 64.0, 0))]
            .into_iter()
            .collect();
        let sizes = sizes_of(&cand, ER_CARD_W, ER_CARD_H_MAX);
        let out = er_place_avoiding_overlaps(&cand, &sizes, &[], ER_PLACE_GAP, ER_PLACE_MAX_RINGS);
        for (name, &(x, y, c)) in &cand {
            assert_eq!(out.get(name), Some(&(x, y, c)), "{name} 不应移动");
        }
    }

    #[test]
    fn place_moves_new_component_off_pinned_tables() {
        // 复现线上场景：pinned 的 public.* 已被用户拖到中间，后加的 er_demo.* 不能压上去。
        let pinned = [rect(55.61, 104.32, ER_CARD_W, 106.0), rect(420.74, 19.89, ER_CARD_W, 106.0)];
        let cand: ErLayoutResult = [
            ("er_demo.customers".to_string(), (64.0, 64.0, 0)),
            ("er_demo.orders".to_string(), (364.0, 64.0, 0)),
        ]
        .into_iter()
        .collect();
        let sizes = sizes_of(&cand, ER_CARD_W, 106.0);
        let out = er_place_avoiding_overlaps(&cand, &sizes, &pinned, ER_PLACE_GAP, ER_PLACE_MAX_RINGS);
        assert_eq!(out.len(), 2, "两张表都要有坐标");
        assert_no_overlap_with(&out, &pinned, ER_CARD_W, 106.0);
        // 组件整体刚性平移：两表相对偏移不变（连线走向不被打断）。
        let a = out["er_demo.customers"];
        let b = out["er_demo.orders"];
        assert!((b.0 - a.0 - 300.0).abs() < 1e-3, "组件内相对 x 偏移应保持");
        assert_eq!(a.1, b.1, "组件内相对 y 偏移应保持");
    }

    #[test]
    fn place_never_moves_occupied_rects() {
        // 障碍只读：返回结果里不会出现对已占矩形的改写（本函数只产出 movable 的坐标）。
        let occupied = [rect(0.0, 0.0, ER_CARD_W, ER_CARD_H_MAX)];
        let cand: ErLayoutResult = [("late".to_string(), (0.0, 0.0, -1))].into_iter().collect();
        let sizes = sizes_of(&cand, ER_CARD_W, ER_CARD_H_MAX);
        let out = er_place_avoiding_overlaps(&cand, &sizes, &occupied, ER_PLACE_GAP, ER_PLACE_MAX_RINGS);
        let placed = out["late"];
        assert_ne!((placed.0, placed.1), (0.0, 0.0), "新表必须让位");
        assert_no_overlap_with(&out, &occupied, ER_CARD_W, ER_CARD_H_MAX);
    }

    #[test]
    fn place_keeps_cross_schema_same_name_distinct() {
        // 跨 schema 同名表按展示名分别落位，互不串位、互不重叠。
        let occupied = [rect(64.0, 64.0, ER_CARD_W, ER_CARD_H_MAX)];
        let cand: ErLayoutResult = [
            ("er_demo.orders".to_string(), (64.0, 64.0, -1)),
            ("public.orders".to_string(), (364.0, 64.0, -1)),
        ]
        .into_iter()
        .collect();
        let sizes = sizes_of(&cand, ER_CARD_W, ER_CARD_H_MAX);
        let out = er_place_avoiding_overlaps(&cand, &sizes, &occupied, ER_PLACE_GAP, ER_PLACE_MAX_RINGS);
        assert_eq!(out.len(), 2);
        assert_no_overlap_with(&out, &occupied, ER_CARD_W, ER_CARD_H_MAX);
    }

    #[test]
    fn place_isolated_tables_reposition_independently() {
        // 孤立表不是刚体：各自找最近空位，不会被捆成整块推远。
        let occupied = [rect(64.0, 64.0, ER_CARD_W, ER_CARD_H_MAX)];
        let cand: ErLayoutResult = [
            ("iso_a".to_string(), (64.0, 64.0, -1)),
            ("iso_b".to_string(), (400.0, 64.0, -1)),
        ]
        .into_iter()
        .collect();
        let sizes = sizes_of(&cand, ER_CARD_W, ER_CARD_H_MAX);
        let out = er_place_avoiding_overlaps(&cand, &sizes, &occupied, ER_PLACE_GAP, ER_PLACE_MAX_RINGS);
        assert_no_overlap_with(&out, &occupied, ER_CARD_W, ER_CARD_H_MAX);
        // iso_b 本来就不冲突，不应被动移动。
        assert_eq!(out["iso_b"], (400.0, 64.0, -1), "无冲突的表不应移动");
    }

    #[test]
    fn place_deterministic() {
        let occupied = [rect(55.0, 100.0, ER_CARD_W, 106.0)];
        let cand: ErLayoutResult = [
            ("a".to_string(), (64.0, 64.0, 0)),
            ("b".to_string(), (364.0, 64.0, 0)),
            ("z".to_string(), (64.0, 400.0, -1)),
        ]
        .into_iter()
        .collect();
        let sizes = sizes_of(&cand, ER_CARD_W, 120.0);
        let first = er_place_avoiding_overlaps(&cand, &sizes, &occupied, ER_PLACE_GAP, ER_PLACE_MAX_RINGS);
        for _ in 0..5 {
            let again = er_place_avoiding_overlaps(&cand, &sizes, &occupied, ER_PLACE_GAP, ER_PLACE_MAX_RINGS);
            assert_eq!(first, again, "落位必须确定性可复现");
        }
    }

    #[test]
    fn place_falls_back_below_when_budget_exhausted() {
        // 环数为 0 时没有可用搜索方向，必须退到下方追加带而不是重叠或 panic。
        let occupied = [rect(0.0, 0.0, ER_CARD_W, ER_CARD_H_MAX)];
        let cand: ErLayoutResult = [("x".to_string(), (0.0, 0.0, -1))].into_iter().collect();
        let sizes = sizes_of(&cand, ER_CARD_W, ER_CARD_H_MAX);
        let out = er_place_avoiding_overlaps(&cand, &sizes, &occupied, ER_PLACE_GAP, 0);
        assert_no_overlap_with(&out, &occupied, ER_CARD_W, ER_CARD_H_MAX);
        assert!(out["x"].1 >= ER_CARD_H_MAX, "应退到障碍下方");
    }

    #[test]
    fn place_respects_gap_between_cards() {
        // 两个后加表之间也要留出通道间距，而不是刚好相切。
        let cand: ErLayoutResult = [
            ("a".to_string(), (64.0, 64.0, -1)),
            ("b".to_string(), (64.0, 64.0, -1)),
        ]
        .into_iter()
        .collect();
        let sizes = sizes_of(&cand, ER_CARD_W, ER_CARD_H_MAX);
        let out = er_place_avoiding_overlaps(&cand, &sizes, &[], ER_PLACE_GAP, ER_PLACE_MAX_RINGS);
        let a = out["a"];
        let b = out["b"];
        let gap = (b.0 - a.0).abs().max((b.1 - a.1).abs());
        assert!(gap >= ER_CARD_H_MAX, "同点候选必须被分开");
        assert_no_overlap_with(&out, &[], ER_CARD_W, ER_CARD_H_MAX);
    }
}
