// ER 画布场景几何与查询（er-ui-relationship-canvas.md）。
//
// 本文件从「固定网格」改为「自由世界坐标 + 关系驱动布局」：
// - 节点位置(自由世界坐标)与字段滚动、选中、拖动等可变状态分离：
//   场景只保存不可变拓扑与展示列（Send，可在后台线程构建）；可变的坐标/滚动存在
//   桌面状态中，渲染期由此物化出本帧的「场景视图」。
// - 字段级端口：连线端点接具体字段行中心；字段随滚动移动；字段离屏时用上下汇总端口。
// - 空间查询改用自由矩形网格索引（支持负坐标、单节点更新），不再依赖固定行索引。
// - 布局由 app 层 `er_relation_layout` 的连通分量分层结果提供（er_layout.rs）。

/// 卡片宽度（px，逻辑像素）。集中定义，不按表名变化。
const NODE_WIDTH: f32 = 252.0;
/// 顶部色带高。
const ACCENT_BAR: f32 = 4.0;
/// 表头高。
const NODE_HEADER: f32 = 30.0;
/// 字段区上下内边距合计。
const FIELD_PAD_Y: f32 = 6.0;
/// 单行字段高。
const NODE_FIELD_ROW: f32 = 22.0;
/// 卡片内字段可视最大行数。
const MAX_FIELD_ROWS: usize = 8;
/// 长表页脚高。
const LONG_FOOTER: f32 = 20.0;
/// 连线从端口水平出走距离。
const PORT_OFFSET: f32 = 14.0;
/// 可见区域四周的 overscan。
const OVERSCAN: f32 = 200.0;
/// 孤立表分量序号（外围区）。
const ISOLATE_COMPONENT: i32 = -1;

/// 一列字段行的展示数据（名称/图标状态/类型来源分开）。
#[derive(Clone, Debug, PartialEq)]
struct NodeColumnDisplay {
    name: String,
    type_name: Option<String>,
    primary: bool,
    /// 该列是否出现在已加载关系（外键）的某一端。只依据真实边数据回填。
    foreign_key: bool,
}

/// 表节点元数据（不可变拓扑 + 展示列）。坐标为可变状态，不在场景里。
struct ErNodeMeta {
    name: String,
    idx: usize,
    /// 连通分量序号：UI 据此从统一主题调色板稳定取色；-1 为孤立表（外围）。
    color_idx: i32,
    status: ErLoadStatus,
    /// 表注释（搜索命中注释、tooltip 展示完整说明用）。
    comment: Option<String>,
    columns: Vec<NodeColumnDisplay>,
    /// 该表是否参与至少一条已加载关系（决定是否显示「分配色」）。
    has_edge: bool,
    /// 该表作为关系端点出现的字段列下标集合（用于隐藏字段汇总端口计数）。
    edge_columns: std::collections::BTreeSet<usize>,
    /// 字段下标 → 关联目标展示串（`表.字段`），供隐藏字段汇总端口的定位菜单逐项列出
    /// （§4.2：多字段列出字段及关联目标，单字段可直接定位）。
    field_targets: std::collections::BTreeMap<usize, Vec<String>>,
}

/// 由边名提取本地逻辑关系 id：`logic:{id}:{idx}` → 取 `logic:` 后、下一个 `:` 前的段。
/// 物理外键边名不以 `logic:` 开头 → None。
fn er_logical_rel_id(edge_name: &str) -> Option<String> {
    edge_name
        .strip_prefix("logic:")
        .and_then(|rest| rest.split(':').next())
        .map(str::to_string)
}

/// 连线拓扑：端点表 + 字段列（全局列序；字段未加载/不存在时为 None → 汇总/待加载端口）。
/// 端点所在表可自关联（from_idx==to_idx）。
#[derive(Clone, Debug)]
struct ErEdgeTopo {
    from_idx: usize,
    to_idx: usize,
    from_column: Option<usize>,
    to_column: Option<usize>,
    /// 完整身份，供 tooltip/详情（复合关系不合并）。
    from_column_name: String,
    to_column_name: String,
    /// 本地逻辑关系 id（边名 `logic:{id}:{idx}` 提取）；物理外键为 None。
    /// 供「点击逻辑边展开抽屉选中该关系」定位（§五.7）。
    logical_rel_id: Option<String>,
}

/// 画布场景：不可变拓扑与展示列（Send，可后台构建）。
struct ErScene {
    nodes: Vec<ErNodeMeta>,
    edges: Vec<ErEdgeTopo>,
}

/// 单帧物化的节点视图（节点矩形 + 状态）。
#[derive(Clone)]
struct ErNodeView {
    idx: usize,
    name: String,
    x: f32,
    y: f32,
    height: f32,
    scroll_px: f32,
    selected: bool,
}

/// 单帧物化的折线视图（world 坐标折线段 + 两端锚点 + 关系说明）。
#[derive(Clone, Debug)]
struct ErEdgeView {
    points: Vec<(f32, f32)>,
    from_idx: usize,
    to_idx: usize,
    from_anchor: ErAnchor,
    to_anchor: ErAnchor,
    /// 本地逻辑关系 id（供点击展开抽屉定位；物理外键为 None）。
    logical_rel_id: Option<String>,
    /// 关系说明：`表.字段 → 表.字段`（约束名）。选中/hover 展示（§5.2）。
    desc: String,
}

/// 环境：每帧物化时需要的可变状态（坐标/滚动/选中/固定），由桌面状态读入。
struct ErEnv {
    /// 每个节点下标 → (x, y)（自由世界坐标）。
    positions: Vec<(f32, f32)>,
    /// 每个节点下标 → 字段滚动 px。
    scrolls: Vec<f32>,
    /// 选中表下标。
    selected: Option<usize>,
}

/// 卡片外框高度（视觉可见高度）。几何与绘制共用同一来源。
fn card_height(status: ErLoadStatus, ncols: usize) -> f32 {
    let content_rows = match status {
        ErLoadStatus::Loaded if ncols > 0 => ncols.min(MAX_FIELD_ROWS),
        ErLoadStatus::Loaded => 1,
        ErLoadStatus::NotLoaded | ErLoadStatus::Loading => 3,
        ErLoadStatus::Failed => 1,
    };
    let footer = if matches!(status, ErLoadStatus::Loaded) && ncols > MAX_FIELD_ROWS {
        LONG_FOOTER
    } else {
        0.0
    };
    ACCENT_BAR + NODE_HEADER + FIELD_PAD_Y + content_rows as f32 * NODE_FIELD_ROW + footer
}

/// 由纯数据图构图（拓扑 + 展示列 + 分量色）。
/// `layout` 为 app 层 `er_relation_layout` 的结果（表名 → (x,y,分量)）。
fn build_er_scene(graph: &ErGraphData, layout: &ErLayoutResult) -> ErScene {
    let mut name_to_layout: std::collections::HashMap<&str, (f32, f32, i32)> =
        std::collections::HashMap::new();
    for (name, &(x, y, c)) in layout {
        name_to_layout.insert(name.as_str(), (x, y, c));
    }
    // 表名 → 节点下标。
    let mut nodes = Vec::with_capacity(graph.tables.len());
    let mut name_to_idx: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (idx, table) in graph.tables.iter().enumerate() {
        name_to_idx.insert(table.name.as_str(), idx);
        let color = name_to_layout
            .get(table.name.as_str())
            .map(|t| t.2)
            .unwrap_or(ISOLATE_COMPONENT);
        let cols = table
            .columns
            .iter()
            .map(|c| NodeColumnDisplay {
                name: c.name.clone(),
                type_name: c.type_name.clone(),
                primary: c.primary_key,
                foreign_key: false,
            })
            .collect();
        nodes.push(ErNodeMeta {
            name: table.name.clone(),
            idx,
            color_idx: color,
            status: table.status,
            comment: table.comment.clone(),
            columns: cols,
            has_edge: false,
            edge_columns: std::collections::BTreeSet::new(),
            field_targets: std::collections::BTreeMap::new(),
        });
    }
    // 回填外键列标记 + has_edge。
    // 只把真正持有外键的 from 端列标为 FK（引用端字段自身持有外键约束）。
    // 被引用的 to 端字段不是自身持有外键，仅当它也作为某条边的 from 端时才是 FK；
    // 否则它通常是主键，应保留 key 图标而非 link 图标（§3.2、er-ui-redesign §4.2）。
    for e in &graph.edges {
        if let Some(&fi) = name_to_idx.get(e.from_table.as_str()) {
            if let Some(c) = nodes[fi]
                .columns
                .iter_mut()
                .find(|c| c.name == e.from_column)
            {
                c.foreign_key = true;
            }
            nodes[fi].has_edge = true;
        }
        if let Some(&ti) = name_to_idx.get(e.to_table.as_str()) {
            nodes[ti].has_edge = true;
        }
    }
    // 边拓扑（字段列序 -> 全局下标）+ 每表端点字段集合（供汇总端口计数 + 定位菜单）。
    let mut edges = Vec::new();
    let mut edge_cols: Vec<std::collections::BTreeSet<usize>> = vec![std::collections::BTreeSet::new(); nodes.len()];
    let mut field_targets: Vec<std::collections::BTreeMap<usize, Vec<String>>> =
        vec![std::collections::BTreeMap::new(); nodes.len()];
    for e in &graph.edges {
        let Some(&fi) = name_to_idx.get(e.from_table.as_str()) else {
            continue;
        };
        let Some(&ti) = name_to_idx.get(e.to_table.as_str()) else {
            continue;
        };
        let from_col = nodes[fi]
            .columns
            .iter()
            .position(|c| c.name == e.from_column);
        let to_col = nodes[ti]
            .columns
            .iter()
            .position(|c| c.name == e.to_column);
        if let Some(c) = from_col {
            edge_cols[fi].insert(c);
            field_targets[fi]
                .entry(c)
                .or_default()
                .push(format!("{}.{}", nodes[ti].name, e.to_column));
        }
        if let Some(c) = to_col {
            edge_cols[ti].insert(c);
            field_targets[ti]
                .entry(c)
                .or_default()
                .push(format!("{}.{}", nodes[fi].name, e.from_column));
        }
        edges.push(ErEdgeTopo {
            from_idx: fi,
            to_idx: ti,
            from_column: from_col,
            to_column: to_col,
            from_column_name: e.from_column.clone(),
            to_column_name: e.to_column.clone(),
            logical_rel_id: er_logical_rel_id(&e.name),
        });
    }
    for i in 0..nodes.len() {
        nodes[i].edge_columns = std::mem::take(&mut edge_cols[i]);
        nodes[i].field_targets = std::mem::take(&mut field_targets[i]);
    }
    ErScene { nodes, edges }
}

/// 由环境物化每帧场景视图（节点矩形 + 网格 + 可见查询 + 折线）。
impl ErScene {
    /// 生成节点视图（rect + 状态），供渲染与网格重建。
    fn node_views(&self, env: &ErEnv) -> Vec<ErNodeView> {
        self.nodes
            .iter()
            .map(|m| {
                let (x, y) = env.positions[m.idx];
                ErNodeView {
                    idx: m.idx,
                    name: m.name.clone(),
                    x,
                    y,
                    height: card_height(m.status, m.columns.len()),
                    scroll_px: env.scrolls.get(m.idx).copied().unwrap_or(0.0),
                    selected: env.selected == Some(m.idx),
                }
            })
            .collect()
    }
}

/// 缩放安全范围（视图变换缩放，view-transform，§六.23-24）。
/// 卡片仍以固定屏幕尺寸渲染（不 transform 文字），仅世界坐标乘 scale 做平移/命中/连线缩放。
const ER_MIN_SCALE: f32 = 0.15;
const ER_MAX_SCALE: f32 = 4.0;

/// 视口。
#[derive(Clone, Copy, Debug)]
struct ErViewport {
    pan_x: f32,
    pan_y: f32,
    /// 视图缩放：screen = pan + world * scale。
    /// 卡片固定尺寸（宽 NODE_WIDTH、高按内容），仅在非 1 缩放时卡片会重叠/分散；
    /// 路径与锚点按世界坐标统一乘 scale，命中与绘制共用同一变换。
    scale: f32,
}

impl Default for ErViewport {
    fn default() -> Self {
        Self { pan_x: 0.0, pan_y: 0.0, scale: 1.0 }
    }
}

impl ErViewport {
    /// 安全缩放（避免除零/越界）。
    fn safe_scale(&self) -> f32 {
        self.scale.clamp(ER_MIN_SCALE, ER_MAX_SCALE)
    }

    /// 屏幕坐标 → 世界坐标（需调用方先减画布原点）。
    ///
    /// 屏幕 → 世界逆变换即 `(screen - pan) / scale`；世界 → 屏幕（`pan + world*scale`）
    /// 由各绘制层内联（paint 需另加画布原点，与命中逆变换保持一致）。
    fn to_world_x(&self, sx: f32) -> f32 {
        (sx - self.pan_x) / self.safe_scale()
    }

    fn to_world_y(&self, sy: f32) -> f32 {
        (sy - self.pan_y) / self.safe_scale()
    }

    /// 世界坐标下当前可视范围（含 overscan）。
    fn visible_world_bounds(&self, canvas_w: f32, canvas_h: f32) -> (f32, f32, f32, f32) {
        let sc = self.safe_scale();
        (
            -self.pan_x / sc - OVERSCAN,
            -self.pan_y / sc - OVERSCAN,
            (-self.pan_x + canvas_w) / sc + OVERSCAN,
            (-self.pan_y + canvas_h) / sc + OVERSCAN,
        )
    }

    /// 以屏幕 `anchor` 为锚点缩放 `factor`：锚点下的世界点保持静止（§六.24）。
    /// 仅调整 scale 与 pan；pan 是屏幕偏移，independent of scale。
    fn zoom_around(&mut self, anchor_screen: (f32, f32), factor: f32) {
        let old = self.safe_scale();
        let new = (old * factor).clamp(ER_MIN_SCALE, ER_MAX_SCALE);
        if (new - old).abs() < f32::EPSILON {
            return;
        }
        let wx = self.to_world_x(anchor_screen.0);
        let wy = self.to_world_y(anchor_screen.1);
        self.scale = new;
        self.pan_x = anchor_screen.0 - wx * new;
        self.pan_y = anchor_screen.1 - wy * new;
    }
}

/// 统一网格空间索引（自由世界坐标，支持负坐标）。
/// 每格存与该格相交的节点下标；查询时取视口覆盖的格，再做精确矩形相交。
struct ErNodeGrid {
    cell_w: f32,
    cell_h: f32,
    /// 每格 -> 候选节点下标（去重）。
    cells: std::collections::HashMap<(i64, i64), Vec<usize>>,
}

impl ErNodeGrid {
    fn build(node_views: &[ErNodeView], cell_w: f32, cell_h: f32) -> Self {
        let mut cells: std::collections::HashMap<(i64, i64), Vec<usize>> =
            std::collections::HashMap::new();
        for v in node_views {
            let x0 = (v.x / cell_w).floor() as i64;
            let x1 = ((v.x + NODE_WIDTH) / cell_w).floor() as i64;
            let y0 = (v.y / cell_h).floor() as i64;
            let y1 = ((v.y + v.height) / cell_h).floor() as i64;
            for cx in x0..=x1 {
                for cy in y0..=y1 {
                    cells.entry((cx, cy)).or_default().push(v.idx);
                }
            }
        }
        ErNodeGrid { cell_w, cell_h, cells }
    }

    fn query(&self, wx0: f32, wy0: f32, wx1: f32, wy1: f32) -> Vec<usize> {
        let cx0 = (wx0 / self.cell_w).floor() as i64;
        let cx1 = (wx1 / self.cell_w).floor() as i64;
        let cy0 = (wy0 / self.cell_h).floor() as i64;
        let cy1 = (wy1 / self.cell_h).floor() as i64;
        let mut out = std::collections::BTreeSet::new();
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                if let Some(bucket) = self.cells.get(&(cx, cy)) {
                    out.extend(bucket.iter().copied());
                }
            }
        }
        out.into_iter().collect()
    }
}

/// 字段区视口顶部在卡片内的偏移（色带 + 表头）。
fn field_viewport_offset() -> f32 {
    ACCENT_BAR + NODE_HEADER
}

/// 字段端口锚点类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErAnchorKind {
    /// 字段行可见：接到真实字段行中心。
    Field,
    /// 字段被滚出可视区上沿：上汇总端口（count 为隐藏且有关联的不同字段数）。
    SummaryTop,
    /// 滚出下沿：下汇总端口。
    SummaryBottom,
    /// 字段尚未加载：表头上「字段待加载」汇总端口（临时状态，不能伪装成精确匹配）。
    Pending,
    /// 关系引用的字段不存在（加载后找不到）：表头上端口 + tooltip 说明未解析。
    Missing,
}

/// 字段端口锚点（世界坐标 + 类型 + 汇总计数）。
#[derive(Clone, Copy, Debug)]
struct ErAnchor {
    x: f32,
    y: f32,
    kind: ErAnchorKind,
}

/// 当前滚动下，字段区可见行号区间 [first, last]（含端点，闭区间）。
fn visible_row_range(ncols: usize, scroll_px: f32) -> (usize, usize) {
    let visible_rows = ncols.min(MAX_FIELD_ROWS);
    let row0 = (scroll_px / NODE_FIELD_ROW).floor().max(0.0) as usize;
    let last = (row0 + visible_rows.saturating_sub(1)).min(ncols.saturating_sub(1));
    (row0.min(ncols.saturating_sub(1)), last)
}

/// 隐藏（滚出可视区）且有关联的不同字段数：top 为可视区之上、bottom 之下。
fn summary_counts(meta: &ErNodeMeta, scroll_px: f32) -> (usize, usize) {
    if meta.status != ErLoadStatus::Loaded || meta.columns.is_empty() {
        return (0, 0);
    }
    let (row0, row1) = visible_row_range(meta.columns.len(), scroll_px);
    let top = meta.edge_columns.iter().filter(|&&c| c < row0).count();
    let bottom = meta.edge_columns.iter().filter(|&&c| c > row1).count();
    (top, bottom)
}

/// 解析一条边端点的字段锚点（世界坐标）。
/// `side_right` 决定端口接卡片左/右边界；坐标由节点世界坐标、表头高度、
/// 字段 padding、真实字段顺序与滚动偏移共同决定（§4.1）。
fn resolve_anchor(
    meta: &ErNodeMeta,
    v: &ErNodeView,
    column: Option<usize>,
    side_right: bool,
) -> ErAnchor {
    let px = if side_right { v.x + NODE_WIDTH } else { v.x };
    // 字段未加载/失败/空：表头上「字段待加载」汇总端口（临时状态）。
    if meta.status != ErLoadStatus::Loaded {
        return ErAnchor {
            x: px,
            y: v.y + field_viewport_offset() / 2.0,
            kind: ErAnchorKind::Pending,
        };
    }
    let Some(ci) = column else {
        // 关系引用的字段不存在：不静默接到同名近似字段，用表头端口 + tooltip 说明。
        return ErAnchor {
            x: px,
            y: v.y + field_viewport_offset() / 2.0,
            kind: ErAnchorKind::Missing,
        };
    };
    let n = meta.columns.len();
    if n == 0 {
        return ErAnchor {
            x: px,
            y: v.y + field_viewport_offset() / 2.0,
            kind: ErAnchorKind::Missing,
        };
    }
    let scroll = v.scroll_px.clamp(
        0.0,
        ((n.saturating_sub(MAX_FIELD_ROWS)) as f32 * NODE_FIELD_ROW).max(0.0),
    );
    let (row0, row1) = visible_row_range(n, scroll);
    let field_top = v.y + field_viewport_offset();
    if ci < row0 {
        // 滚出上沿：上汇总端口（在字段视口上边缘，不遮字段文字/表头）。
        ErAnchor { x: px, y: field_top, kind: ErAnchorKind::SummaryTop }
    } else if ci > row1 {
        // 滚出下沿：下汇总端口（在字段视口下边缘）。
        let visible_rows = n.min(MAX_FIELD_ROWS);
        ErAnchor {
            x: px,
            y: field_top + visible_rows as f32 * NODE_FIELD_ROW,
            kind: ErAnchorKind::SummaryBottom,
        }
    } else {
        // 可见字段：行中心（含半行滚动的真实位置，随滚动平滑移动）。
        let y = field_top + FIELD_PAD_Y / 2.0 + (ci as f32 * NODE_FIELD_ROW - scroll) + NODE_FIELD_ROW / 2.0;
        ErAnchor { x: px, y, kind: ErAnchorKind::Field }
    }
}

/// 线段与矩形相交（含边界接触），用于障碍规避与保守命中。
fn segment_intersects_rect(
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    rx0: f32,
    ry0: f32,
    rx1: f32,
    ry1: f32,
) -> bool {
    let p = [-bx + ax, bx - ax, -by + ay, by - ay];
    let q = [ax - rx0, rx1 - ax, ay - ry0, ry1 - ay];
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for i in 0..4 {
        if p[i] == 0.0 {
            if q[i] < 0.0 {
                return false;
            }
        } else {
            let t = q[i] / p[i];
            if p[i] < 0.0 {
                if t > t1 {
                    return false;
                }
                if t > t0 {
                    t0 = t;
                }
            } else {
                if t < t0 {
                    return false;
                }
                if t < t1 {
                    t1 = t;
                }
            }
        }
    }
    t0 <= t1
}

/// 正交折线路由：字段端口 → 水平出走 PORT_OFFSET → 通道 → 目标端口。
/// 保证不穿任何卡片正文（含源/目标表）：仅允许合法端口出口接触，正文为障碍。
/// 用空间查询限定障碍范围，不逐边遍历全库。候选失败不接收穿卡路径，改确定性外侧绕行。
/// `channel_hint` 按稳定关系身份分配（不在可见边集合变化时改变）。
#[allow(clippy::too_many_arguments)]
fn route_field_edge(
    a: ErAnchor,
    b: ErAnchor,
    from_view: &ErNodeView,
    to_view: &ErNodeView,
    grid: &ErNodeGrid,
    node_views: &[ErNodeView],
    channel_hint: usize,
) -> Vec<(f32, f32)> {
    let is_self = from_view.idx == to_view.idx;
    let side_gap = channel_hint as f32 * 7.0;
    // 右侧通道 x（卡片外）：from/target 都取右侧端口（自关联统一右侧）。
    let fr = from_view.x + NODE_WIDTH;
    let tr = to_view.x + NODE_WIDTH;

    if is_self {
        // 自关联：两端都在右侧边界。路径：右端口 → 右走 out → 右侧竖直通道 → 到目标右端口。
        // 通道 y 复用两端字段 y；同字段(ay==by)时用一小段上下形成可见回路。
        let out = fr + PORT_OFFSET + side_gap;
        let (ya, yb) = (a.y, b.y);
        let path = if (ya - yb).abs() < 0.5 {
            // 同字段自关联（同一行）：右出 → 下到通道 → 再下小段 → 回。
            let dy = 8.0;
            vec![
                (a.x, ya),
                (out, ya),
                (out, ya + dy),
                (fr + PORT_OFFSET * 2.0 + side_gap, ya + dy),
                (fr + PORT_OFFSET * 2.0 + side_gap, ya),
                (out, ya),
                (b.x, yb),
            ]
        } else {
            // 不同字段：右出 → 竖直通道（在外侧）→ 到目标端口。
            vec![(a.x, ya), (out, ya), (out, yb), (b.x, yb)]
        };
        // 自关联路径整体在卡片外部（x ∈ [fr, out]），不进入正文；直接返回（无 obstacle 检查）。
        return path;
    }

    let a_out_right = a.x > from_view.x;
    let b_out_right = b.x > to_view.x;
    let a_exit = if a_out_right { a.x + PORT_OFFSET } else { a.x - PORT_OFFSET };
    let b_enter = if b_out_right { b.x + PORT_OFFSET } else { b.x - PORT_OFFSET };

    // 校验整条折线不穿任一卡正文（除两端卡允许端口出口接触）。
    fn path_clear(
        cand: &[(f32, f32)],
        from_view: &ErNodeView,
        to_view: &ErNodeView,
        grid: &ErNodeGrid,
        node_views: &[ErNodeView],
    ) -> bool {
        for w in cand.windows(2) {
            let (ax, ay, bx, by) = (w[0].0, w[0].1, w[1].0, w[1].1);
            let (x0, x1) = (ax.min(bx), ax.max(bx));
            let (y0, y1) = (ay.min(by), ay.max(by));
            for &ni in &grid.query(x0 - 0.5, y0 - 0.5, x1 + 0.5, y1 + 0.5) {
                if ni == from_view.idx || ni == to_view.idx {
                    continue; // 端点表：允许端口出口接触
                }
                let v = &node_views[ni];
                if v.x <= x1 && v.x + NODE_WIDTH >= x0 && v.y <= y1 && v.y + v.height >= y0
                    && segment_intersects_rect(ax, ay, bx, by, v.x, v.y, v.x + NODE_WIDTH, v.y + v.height)
                {
                    return false;
                }
            }
        }
        true
    }

    // 正交折线底稿：水平出源卡 → 竖直降到目标行（x 停在源卡外的出走列）→ 水平进目标端口。
    // 不能直接把 (a_exit,a.y)→(b_enter,b.y) 两点相连（y 不同时会形成斜线，违反 §5.1 正交要求）。
    let base_cand = vec![(a.x, a.y), (a_exit, a.y), (a_exit, b.y), (b.x, b.y)];
    if path_clear(&base_cand, from_view, to_view, grid, node_views) {
        return base_cand;
    }

    // 横向错开：两卡间隙竖直通道（分多次外移，避开中间卡）。
    let horizontally_clear = from_view.x + NODE_WIDTH <= to_view.x || to_view.x + NODE_WIDTH <= from_view.x;
    let mut candidates: Vec<Vec<(f32, f32)>> = Vec::new();
    if horizontally_clear {
        let (gap_l, gap_r) = if fr <= to_view.x {
            (fr, to_view.x)
        } else if tr <= from_view.x {
            (tr, from_view.x)
        } else {
            (0.0, 0.0)
        };
        let base = (gap_l + gap_r) / 2.0;
        for o in [0.0f32, 6.0, -6.0, 16.0, -16.0] {
            let cx = base + o + (channel_hint as f32);
            candidates.push(vec![
                (a.x, a.y),
                (a_exit, a.y),
                (cx, a.y),
                (cx, b.y),
                (b_enter, b.y),
                (b.x, b.y),
            ]);
        }
    } else {
        // 横向重叠：走两卡右外侧通道（绕开正文；出口默认右侧）。
        let out_r = fr.max(tr) + PORT_OFFSET + side_gap;
        candidates.push(vec![
            (a.x, a.y),
            (a_exit, a.y),
            (out_r, a.y),
            (out_r, b.y),
            (b_enter, b.y),
            (b.x, b.y),
        ]);
    }

    for cand in candidates {
        if path_clear(&cand, from_view, to_view, grid, node_views) {
            return cand;
        }
    }
    // 预算用尽（间隙通道全被障碍占用）：在有限预算内做确定性外侧绕行。
    // 生成一张卡左/右外侧的竖直绕行车道，逐条 `path_clear`；全被占则选碰撞最少的一条，
    // 绝不静默接受明显穿卡的单一那条兜底路径（§5.1 / 任务：有限候选被占时不接受穿卡最后路径）。
    // 车道集有界（卡左右外侧各若干档 + 间隙扩展档），不做无限搜索。
    fn count_crossings(
        cand: &[(f32, f32)],
        from_view: &ErNodeView,
        to_view: &ErNodeView,
        grid: &ErNodeGrid,
        node_views: &[ErNodeView],
    ) -> usize {
        let mut n = 0;
        for w in cand.windows(2) {
            let (ax, ay, bx, by) = (w[0].0, w[0].1, w[1].0, w[1].1);
            let (x0, x1) = (ax.min(bx), ax.max(bx));
            let (y0, y1) = (ay.min(by), ay.max(by));
            for &ni in &grid.query(x0 - 0.5, y0 - 0.5, x1 + 0.5, y1 + 0.5) {
                if ni == from_view.idx || ni == to_view.idx {
                    continue;
                }
                let v = &node_views[ni];
                if v.x <= x1 && v.x + NODE_WIDTH >= x0 && v.y <= y1 && v.y + v.height >= y0
                    && segment_intersects_rect(ax, ay, bx, by, v.x, v.y, v.x + NODE_WIDTH, v.y + v.height)
                {
                    n += 1;
                }
            }
        }
        n
    }
    // 竖直绕行段（从 a_exit 竖到 mid_y）、水平段（到 lane_x）、再竖到 b.y。正交折线。
    fn detour_path(
        a: ErAnchor, b: ErAnchor, a_exit: f32, b_enter: f32, lane_x: f32, mid_y: f32,
    ) -> Vec<(f32, f32)> {
        vec![
            (a.x, a.y), (a_exit, a.y), (a_exit, mid_y), (lane_x, mid_y), (lane_x, b.y), (b_enter, b.y), (b.x, b.y),
        ]
    }
    // 车道集（x）与绕行带（y）都取有界候选，不能无限搜索：
    // 车道 = 两卡左右外侧若干档 + 间隙扩展；带 = 两端口 y + 障碍区上方/下方。
    let mut lanes: Vec<f32> = Vec::new();
    let mut add_lane = |x: f32| {
        let x = x + (channel_hint as f32);
        if !lanes.contains(&x) {
            lanes.push(x);
        }
    };
    let (left_min, right_max) = (from_view.x.min(to_view.x), (from_view.x.max(to_view.x) + NODE_WIDTH));
    for k in 0..3 {
        add_lane(left_min - PORT_OFFSET - k as f32 * 14.0);
        add_lane(right_max + PORT_OFFSET + k as f32 * 14.0);
    }
    if let Some((gl, gr)) = if fr <= to_view.x { Some((fr, to_view.x)) } else if tr <= from_view.x { Some((tr, from_view.x)) } else { None } {
        let base = (gl + gr) / 2.0;
        for o in [-32.0f32, 32.0, -64.0, 64.0] {
            add_lane(base + o);
        }
    }
    // 挡在两卡之间的障碍：带候选（在其上方/下方穿行绕过，避开同 y 封死走廊的卡）。
    let (bx0, bx1) = (a.x.min(b.x) - PORT_OFFSET, a.x.max(b.x).max(fr).max(tr) + PORT_OFFSET);
    let (by0, by1) = (a.y.min(b.y), a.y.max(b.y));
    let mut obs_top = f32::NEG_INFINITY;
    let mut obs_bot = f32::INFINITY;
    for &ni in &grid.query(bx0, by0, bx1, by1) {
        if ni == from_view.idx || ni == to_view.idx {
            continue;
        }
        let v = &node_views[ni];
        if v.x + NODE_WIDTH >= bx0 && v.x <= bx1 {
            obs_top = obs_top.max(v.y + v.height);
            obs_bot = obs_bot.min(v.y);
        }
    }
    let mut bands: Vec<f32> = vec![a.y, b.y];
    if obs_top.is_finite() {
        bands.push(obs_top + 24.0);
    }
    if obs_bot.is_finite() {
        bands.push(obs_bot - 24.0);
    }
    bands.dedup();
    let mut best: Option<(Vec<(f32, f32)>, usize)> = None;
    'outer: for cx in &lanes {
        for &my in &bands {
            let cand = detour_path(a, b, a_exit, b_enter, *cx, my);
            if path_clear(&cand, from_view, to_view, grid, node_views) {
                best = Some((cand, 0));
                break 'outer; // 命中首个合法绕行即返回（确定性、有界）。
            }
            let c = count_crossings(&cand, from_view, to_view, grid, node_views);
            if best.as_ref().map(|b| c < b.1).unwrap_or(true) {
                best = Some((cand, c));
            }
        }
    }
    // 无零碰撞路径（极密集/叠卡）：返回碰撞最少的绕行候选，绝不接受未评分的穿卡兜底。
    match best {
        Some((p, _)) => p,
        None => {
            let out_r = right_max + PORT_OFFSET + 6.0 + side_gap;
            vec![(a.x, a.y), (a_exit, a.y), (out_r, a.y), (out_r, b.y), (b_enter, b.y), (b.x, b.y)]
        }
    }
}

/// 单帧物化结果：可见节点 + 可见折线。
struct ErFrame {
    node_views: Vec<ErNodeView>,
    visible_nodes: Vec<usize>,
    edge_views: Vec<ErEdgeView>,
}

impl ErScene {
    /// 由环境 + 视口物化本帧：网格查询可见节点；边按「两端卡片联合包围盒」保守筛选后
    /// 解析字段锚点并路由（两端屏外但路径穿视口保留；折线实际位置由当前滚动决定）。
    fn materialize(&self, env: &ErEnv, vp: ErViewport, cw: f32, ch: f32) -> ErFrame {
        let node_views = self.node_views(env);
        let grid = ErNodeGrid::build(&node_views, 420.0, 420.0);
        let (wx0, wy0, wx1, wy1) = vp.visible_world_bounds(cw, ch);
        let mut visible_nodes: Vec<usize> = grid
            .query(wx0, wy0, wx1, wy1)
            .into_iter()
            .filter(|&i| {
                let v = &node_views[i];
                v.x + NODE_WIDTH >= wx0 && v.x <= wx1 && v.y + v.height >= wy0 && v.y <= wy1
            })
            .collect();
        visible_nodes.sort_unstable();

        // 并行边稳定通道序：同 (from,to) 无序对内按边序号给 hint，避免重合成一条粗线。
        let mut pair_seq: std::collections::HashMap<(usize, usize), usize> =
            std::collections::HashMap::new();
        let mut edge_views = Vec::with_capacity(self.edges.len());
        for e in &self.edges {
            let (fv, tv) = (&node_views[e.from_idx], &node_views[e.to_idx]);
            // 保守包围盒：两端卡片联合范围外扩端口出走距离（路径不会远离两卡范围太多）。
            let bx0 = fv.x.min(tv.x) - PORT_OFFSET * 3.0;
            let bx1 = (fv.x.max(tv.x) + NODE_WIDTH) + PORT_OFFSET * 3.0;
            let by0 = fv.y.min(tv.y);
            let by1 = fv.y.max(tv.y) + fv.height.max(tv.height);
            if bx1 < wx0 || bx0 > wx1 || by1 < wy0 || by0 > wy1 {
                continue;
            }
            // 端口侧别：
            // - 自关联(from==to)：两端接同一侧（右侧），路径在卡片外侧竖直通道，不横穿卡片正文。
            // - 普通边：目标在右 → 出口右/入口左；否则反向。横向重叠时不强行用会穿表的左右端口，
            //   由路由层统一改走两卡同侧外侧通道。
            let from_right = if e.from_idx == e.to_idx {
                true // 自关联统一右侧
            } else if tv.x + NODE_WIDTH <= fv.x {
                false
            } else if fv.x + NODE_WIDTH <= tv.x {
                true
            } else {
                // 横向重叠：仍指定默认侧，由 route_field_edge 据两卡外侧重选通道。
                true
            };
            let to_right = if e.from_idx == e.to_idx { true } else { !from_right };
            let a = resolve_anchor(&self.nodes[e.from_idx], fv, e.from_column, from_right);
            let b = resolve_anchor(&self.nodes[e.to_idx], tv, e.to_column, to_right);
            let key = (e.from_idx.min(e.to_idx), e.from_idx.max(e.to_idx));
            let seq = {
                let n = pair_seq.entry(key).or_insert(0);
                let s = *n;
                *n += 1;
                s
            };
            let points = route_field_edge(a, b, fv, tv, &grid, &node_views, seq);
            // 真实路径包围盒裁剪：两端屏外但绕行路径可见时仍绘制；路径未穿视口则跳过。
            let mut px0 = f32::INFINITY;
            let mut py0 = f32::INFINITY;
            let mut px1 = f32::NEG_INFINITY;
            let mut py1 = f32::NEG_INFINITY;
            for &(x, y) in &points {
                px0 = px0.min(x);
                py0 = py0.min(y);
                px1 = px1.max(x);
                py1 = py1.max(y);
            }
            if px1 < wx0 || px0 > wx1 || py1 < wy0 || py0 > wy1 {
                continue;
            }
            let desc = format!(
                "{}.{} → {}.{}",
                self.nodes[e.from_idx].name, e.from_column_name,
                self.nodes[e.to_idx].name, e.to_column_name,
            );
            edge_views.push(ErEdgeView {
                points,
                from_idx: e.from_idx,
                to_idx: e.to_idx,
                from_anchor: a,
                to_anchor: b,
                logical_rel_id: e.logical_rel_id.clone(),
                desc,
            });
        }
        ErFrame { node_views, visible_nodes, edge_views }
    }
}

/// 由桌面可变状态构建本帧环境（坐标/滚动/选中/固定，按稳定表身份读取）。
/// 缺坐标的表（布局未及）回退到临时网格位，保证可访问。
fn build_env(
    scene: &ErScene,
    tab: TabId,
    positions: &BTreeMap<String, (f32, f32)>,
    scrolls: &BTreeMap<(TabId, String), f32>,
    selected: Option<&str>,
    pinned: &BTreeSet<String>,
) -> ErEnv {
    let pos: Vec<(f32, f32)> = scene
        .nodes
        .iter()
        .map(|m| {
            positions
                .get(&m.name)
                .copied()
                .unwrap_or_else(|| fallback_grid_pos(m.idx))
        })
        .collect();
    let sc: Vec<f32> = scene
        .nodes
        .iter()
        .map(|m| scrolls.get(&(tab, m.name.clone())).copied().unwrap_or(0.0))
        .collect();
    let sel = selected.and_then(|name| scene.nodes.iter().find(|m| m.name == name).map(|m| m.idx));
    let _ = pinned; // 固定语义在 app 层（布局应用保留坐标）；环境内不参与几何。
    ErEnv { positions: pos, scrolls: sc, selected: sel }
}

/// 布局未及时的临时网格位（可操作的概览排列，§6.2）。
fn fallback_grid_pos(idx: usize) -> (f32, f32) {
    const COLS: usize = 5;
    (
        64.0 + (idx % COLS) as f32 * (NODE_WIDTH + 72.0),
        64.0 + (idx / COLS) as f32 * (card_height(ErLoadStatus::Loaded, MAX_FIELD_ROWS) + 44.0),
    )
}
