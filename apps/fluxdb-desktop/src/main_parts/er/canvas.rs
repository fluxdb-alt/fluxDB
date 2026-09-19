// ER 关系图「画布原型」标签页的绘制与加载触发（er-design.md §8 步骤 3）。
//
// 分层：本文件只做 desktop 侧渲染与按 tab 缓存。数据库读取编排在 fluxdb-app 的
// er_service.rs（load_er_graph_in_background），这里绝不拼 SQL、不直接摸驱动。
// 首次打开自动生成：标签一进入 content 渲染即触发加载，无额外「生成」按钮。

// —— ER 画布聚合耗时诊断（默认关闭，编译期开关）——
// 聚合记录可见查询与元素构建耗时，达到阈值帧数后以 tracing::debug! 输出一次，
// 然后复位。开启时只在阈值处输出，不逐帧刷日志；关闭时零开销。
// 用于定位平移卡顿热点（见 er-design.md §13），不参与任何逻辑分支。
const ER_PERF_DEBUG: bool = false;
const ER_PERF_THRESHOLD: u32 = 600;

/// 低开销聚合计时器：累计 sum/count/min/max，达阈值输出并复位。
struct ErPerfTimer {
    name: &'static str,
    sum_us: u128,
    count: u32,
    max_us: u128,
}

impl ErPerfTimer {
    fn snapshot(&mut self, dur: std::time::Duration) {
        self.sum_us += dur.as_micros();
        let u = dur.as_micros();
        self.max_us = self.max_us.max(u);
        self.count += 1;
        if self.count >= ER_PERF_THRESHOLD {
            let avg = self.sum_us / self.count as u128;
            if ER_PERF_DEBUG {
                tracing::debug!(
                    name = self.name,
                    frames = self.count,
                    avg_us = avg,
                    max_us = self.max_us,
                    "ER 画布聚合耗时"
                );
            }
            self.sum_us = 0;
            self.count = 0;
            self.max_us = 0;
        }
    }
}

// 画布聚合计时器（线程局部，仅 ER_PERF_DEBUG 开启时取样）。
thread_local! {
    static ER_T_QUERY: std::cell::RefCell<ErPerfTimer> = std::cell::RefCell::new(ErPerfTimer {
        name: "cull_query", sum_us: 0, count: 0, max_us: 0,
    });
}

// —— 固定栅格布局参数（虚拟化画布）——
// 屏幕坐标 = 世界坐标 + pan（无缩放）。固定列网格，节点高度随列数变化。
const NODE_WIDTH: f32 = 220.0; // 表节点宽 px
const NODE_HEADER: f32 = 30.0; // 表名标题高 px
const NODE_ROW: f32 = 22.0; // 每列行高 px
const NODE_GAP_X: f32 = 60.0; // 节点横向间距
const NODE_GAP_Y: f32 = 30.0; // 节点纵向间距
const COLUMNS: usize = 5; // 每行节点数
/// 可见区域四周的 overscan（px，世界坐标），平移边缘减少空白、避免频繁创建/回收。
const OVERSCAN: f32 = 200.0;

/// ER 画布视口：平移（pan）。屏幕坐标 = 世界坐标 + pan。
/// 节点 div 定位、连线 canvas 与可见性裁剪共用同一 pan，保证三端不漂移。
/// （真正缩放需全自绘 Element，见 §13 TODO；届时在此加 zoom 字段。）
#[derive(Clone, Copy, Debug)]
struct ErViewport {
    pan_x: f32,
    pan_y: f32,
}

impl Default for ErViewport {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }
}

impl ErViewport {
    /// 给定画布可见尺寸（px），返回含 overscan 的可见世界坐标范围 (x0,y0,x1,y1)。
    /// 屏幕 = 世界 + pan → 世界 = 屏幕 - pan；可见屏幕区 [0..w]×[0..h]。
    fn visible_world_bounds(&self, canvas_w: f32, canvas_h: f32) -> (f32, f32, f32, f32) {
        (
            -self.pan_x - OVERSCAN,
            -self.pan_y - OVERSCAN,
            -self.pan_x + canvas_w + OVERSCAN,
            -self.pan_y + canvas_h + OVERSCAN,
        )
    }
}

/// 表节点布局：世界坐标下的矩形。
/// 虚拟化后节点控件只对可见节点创建；columns 是随图构建一次算好的字段展示文本
/// （`名  类型`）+ 主键标记，避免平移每帧对可见字段重复 format!。
struct ErNodeLayout {
    name: String,
    x: f32,
    y: f32,
    height: f32,
    columns: Vec<NodeColumnDisplay>,
}

/// 节点字段列的单行展示数据：文本预拼好、主键标记随字段固定。
#[derive(Clone)]
struct NodeColumnDisplay {
    text: String,
    primary: bool,
}

/// 行范围索引：固定列网格中一行节点的世界 y 范围与该行节点在 layouts 的下标闭区间。
/// 节点高度随列数变化，行高取该行最大节点高；y 范围用于可见性相交判断。
struct ErRowRange {
    y_start: f32,
    y_end: f32,
    idx_start: usize,
    idx_end: usize,
}

/// 连线（世界坐标端点）。端点生成过一次，虚拟化只做可见性判定，不重建路径。
#[derive(Clone, Copy, Debug)]
struct ErEdge {
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
}

/// 画布场景：全图轻量几何，一次算好供虚拟化查询与渲染复用。
/// 布局、端点、行索引针对「所有表」；可见性裁剪在渲染/绘制时进行。
struct ErScene {
    layouts: Vec<ErNodeLayout>,
    /// 行范围索引（按 y 排序），用于快速求与视口相交的节点行。
    rows: Vec<ErRowRange>,
    edges: Vec<ErEdge>,
    /// 连线 y 带索引：`edge_bands[k]` 存 bbox y 范围落在第 k 条 y 带的边下标。
    /// 平移时只扫与视口 y 重叠的带（候选边），再逐个做精确线段相交，避免每次重绘线性扫全部边。
    edge_bands: Vec<Vec<u32>>,
    /// bbox 跨越带数超过阈值的「长边」下标。它们不入带（避免按跨越距离无限复制索引项），
    /// 而是单独存一份；查询时额外扫一遍（长边数量少，开销可忽略），保证穿过视口的长线不丢。
    long_edges: Vec<u32>,
}

/// bbox 跨越带数超过该值即视为「长边」，改存 long_edges 而不再逐带复制索引项，
/// 以限制单条边对索引的内存放大（默认 8 带 × 400px ≈ 3200px 跨行距才算长）。
const MAX_LONG_EDGE_BANDS: usize = 8;

/// 连线 y 带高（px，世界坐标）。带越宽候选越粗、索引越小；取能覆盖数行节点的高度，
/// 典型本地外键（同/近邻行）只落入 1~3 条带；跨带数超过 MAX_LONG_EDGE_BANDS 的长边改走 long_edges，
/// 不入带以避免按跨越距离无限复制索引项。
const EDGE_BAND_H: f32 = 400.0;

impl ErScene {
    /// 视口世界矩形 → 与视口完整相交（含 x 与 y）的节点 layouts 下标集。
    /// 虚拟化只对这些节点创建控件。行按 y_start 有序：先用二分跳过视口之前的行，
    /// 再逐节点做完整 x/y 矩形相交判定——避免「只按行最大高度筛 y、之后只判 x」
    /// 导致同排已经离开视口的矮表仍被挂载（一行存在超高表时尤其明显）。
    fn visible_nodes(&self, wx0: f32, wy0: f32, wx1: f32, wy1: f32) -> Vec<usize> {
        let mut out = Vec::new();
        // 行 y_end 与 y_start 均随行号单调不减（行间不重叠），可对 y_end 二分
        // 直达第一个可能与视口相交的行，避免拖到图尾仍从第 0 行线性扫描。
        let first = self.rows.partition_point(|r| r.y_end < wy0);
        for r in &self.rows[first..] {
            if r.y_start > wy1 {
                break; // 后续行起点更大，可直接终止。
            }
            for i in r.idx_start..r.idx_end {
                let l = &self.layouts[i];
                if l.x + NODE_WIDTH >= wx0
                    && l.x <= wx1
                    && l.y + l.height >= wy0
                    && l.y <= wy1
                {
                    out.push(i);
                }
            }
        }
        out
    }

    /// 视口世界矩形 → 与视口**线段精确相交**的连线（克隆可见的少数，不深拷贝全部边）。
    /// y 带索引筛出 bbox 与视口 y 重叠的候选边 → x AABB 快速拒绝 → 逐条 Liang-Barsky
    /// 精确线段相交。只对最终命中视口的边返回（两端都在屏外但线段穿过视口的保留）。
    fn visible_edges(&self, wx0: f32, wy0: f32, wx1: f32, wy1: f32) -> Vec<ErEdge> {
        let mut out = Vec::new();
        let a_band = (wy0 / EDGE_BAND_H).floor().max(0.0) as usize;
        let b_band = (wy1 / EDGE_BAND_H).floor().max(0.0) as usize;
        if a_band >= self.edge_bands.len() {
            return out;
        }
        let b_band = b_band.min(self.edge_bands.len() - 1);
        // 一条边若 bbox 跨多条带会出现在多条带里，用候选集合去重（量小，HashSet 足够）。
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for k in a_band..=b_band {
            for &ei in &self.edge_bands[k] {
                if !seen.insert(ei) {
                    continue;
                }
                let e = &self.edges[ei as usize];
                if edge_bbox_overlaps(e, wx0, wy0, wx1, wy1)
                    && er_edge_intersects_rect(e, wx0, wy0, wx1, wy1)
                {
                    out.push(*e);
                }
            }
        }
        // 补扫长边（不入带、数量少）：保证两端屏外但线段穿过视口的远距边仍被命中。
        for &ei in &self.long_edges {
            let e = &self.edges[ei as usize];
            if edge_bbox_overlaps(e, wx0, wy0, wx1, wy1)
                && er_edge_intersects_rect(e, wx0, wy0, wx1, wy1)
            {
                out.push(*e);
            }
        }
        out
    }
}

/// 连线 bbox 与矩形相交（AABB，快速拒绝；用于索引候选后的一级预筛）。
fn edge_bbox_overlaps(e: &ErEdge, wx0: f32, wy0: f32, wx1: f32, wy1: f32) -> bool {
    let (ex0, ex1) = (e.ax.min(e.bx), e.ax.max(e.bx));
    let (ey0, ey1) = (e.ay.min(e.by), e.ay.max(e.by));
    ex1 >= wx0 && ex0 <= wx1 && ey1 >= wy0 && ey0 <= wy1
}

/// 线段与矩形的**精确相交**判定（Liang-Barsky 参数裁剪）。矩形边界接触视为相交
/// （含恰好压在边界上）；零长度线段（端点重合）点落在矩形内才算相交。
/// 用于剔除「包围盒与视口相交，但斜线本身未穿过」的假阳性候选边。
fn er_edge_intersects_rect(e: &ErEdge, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    let (sx, sy, ex, ey) = (e.ax, e.ay, e.bx, e.by);
    let (dx, dy) = (ex - sx, ey - sy);
    // Liang-Barsky：p/q 四组裁剪边界系数，t∈[0,1] 段与矩形有交则返回 true。
    let p = [-dx, dx, -dy, dy];
    let q = [sx - x0, x1 - sx, sy - y0, y1 - sy];
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    for i in 0..4 {
        if p[i] == 0.0 {
            // 平行于该裁剪边界：整体在其外则线段不可能相交。
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

/// 由图表构建一次画布场景：布局 + 连线端点 + 行索引。
/// 端点到节点矩形左右边缘中点；连线与节点同一世界坐标。
/// 在后台线程调用（见 ensure_er_graph_loaded），返回不含非 Send 字段的布局数据。
fn build_er_scene(graph: &ErGraphData) -> ErScene {
    let (layouts, rows) = er_layout(graph);
    // 表名 → layouts 下标，避免每条边线性扫全表。
    let mut index_of: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, l) in layouts.iter().enumerate() {
        index_of.insert(l.name.as_str(), i);
    }
    let mut edges = Vec::new();
    for edge in &graph.edges {
        let Some(&from_idx) = index_of.get(edge.from_table.as_str()) else {
            continue; // 被引用到不存在的表：跳过该线。
        };
        let Some(&to_idx) = index_of.get(edge.to_table.as_str()) else {
            continue;
        };
        let from = &layouts[from_idx];
        let to = &layouts[to_idx];
        // 起点取左侧边缘中点的表，终点取另一侧边缘中点：连线在节点间隙走。
        let (ax, ay) = if from.x <= to.x {
            (from.x + NODE_WIDTH, from.y + from.height / 2.0)
        } else {
            (from.x, from.y + from.height / 2.0)
        };
        let (bx, by) = if to.x >= from.x {
            (to.x, to.y + to.height / 2.0)
        } else {
            (to.x + NODE_WIDTH, to.y + to.height / 2.0)
        };
        edges.push(ErEdge { ax, ay, bx, by });
    }
    // 建连线 y 带索引：把每条边的 bbox y 范围 [ey0,ey1] 压入其跨越的每条带上。
    // 世界 y 从 0 起（节点坐标非负），带数由最大 y 端定；长对角线 FK 会落入多条带，
    // 但图中本地外键占绝大多数、长外键数量少，入桶总量仍可控。
    // ponytail: 每带存 u32 下标索引，未做带内 x 二次索引；若出现海量长外键导致候选带过宽，
    //     改为按 x 再加一层链接列表或改用线段树。现量级（几千边 × 短跨带）足够。
    let max_y = layouts
        .iter()
        .map(|l| l.y + l.height)
        .fold(0.0f32, f32::max);
    let band_count = (max_y / EDGE_BAND_H).ceil() as usize + 1;
    let mut edge_bands = vec![Vec::new(); band_count];
    let mut long_edges: Vec<u32> = Vec::new();
    for (i, e) in edges.iter().enumerate() {
        let (ey0, ey1) = (e.ay.min(e.by), e.ay.max(e.by));
        let a = (ey0 / EDGE_BAND_H).floor().max(0.0) as usize;
        let b = ((ey1 / EDGE_BAND_H).floor().max(0.0) as usize).min(band_count - 1);
        // 长边：跨越带数过多 → 不入带，避免按跨越距离无限复制索引项造成内存放大；
        // 单独存一份供查询补扫，仍是精确相交判定，不丢穿过视口的长线。
        if b - a + 1 > MAX_LONG_EDGE_BANDS {
            long_edges.push(i as u32);
            continue;
        }
        for k in a..=b {
            edge_bands[k].push(i as u32);
        }
    }
    ErScene {
        layouts,
        rows,
        edges,
        edge_bands,
        long_edges,
    }
}

/// 由纯数据图算出每个节点的世界坐标 + 行范围索引。
/// 返回 (各节点布局, 行范围索引)。行内下标连续、按 y 有序，
/// 供 visible_nodes 做视口相交查询（高度随列数变化，行高取该行最大节点高）。
fn er_layout(graph: &ErGraphData) -> (Vec<ErNodeLayout>, Vec<ErRowRange>) {
    let mut layouts = Vec::with_capacity(graph.tables.len());
    let mut rows: Vec<ErRowRange> = Vec::new();
    let mut max_row_h: f32 = 0.0; // 当前行最大高度（决定下一行起点）
    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;
    let mut col = 0usize;
    let mut row_start_idx = 0usize;
    let mut row_y_start = 0.0f32;

    for (idx, table) in graph.tables.iter().enumerate() {
        let h = NODE_HEADER + table.columns.len() as f32 * NODE_ROW + 8.0;
        if col == 0 {
            // 新一行：记录行起点（idx 与 y）。
            row_start_idx = idx;
            row_y_start = y;
            max_row_h = 0.0;
        }
        // 预拼每列展示文本（`名  类型`），一次算好供每帧渲染复用，避免平移时重复 format!。
        let columns: Vec<NodeColumnDisplay> = table
            .columns
            .iter()
            .map(|col| NodeColumnDisplay {
                text: match &col.type_name {
                    Some(ty) => format!("{}  {}", col.name, ty),
                    None => col.name.clone(),
                },
                primary: col.primary_key,
            })
            .collect();
        layouts.push(ErNodeLayout {
            name: table.name.clone(),
            x,
            y,
            height: h,
            columns,
        });
        if h > max_row_h {
            max_row_h = h;
        }
        if col + 1 >= COLUMNS {
            // 行满：闭行记录 y_end + index 区间。
            rows.push(ErRowRange {
                y_start: row_y_start,
                y_end: row_y_start + max_row_h,
                idx_start: row_start_idx,
                idx_end: idx + 1,
            });
            col = 0;
            x = 0.0;
            y += max_row_h + NODE_GAP_Y;
        } else {
            col += 1;
            x += NODE_WIDTH + NODE_GAP_X;
        }
    }
    // 末行不满时补记。
    if col != 0 {
        rows.push(ErRowRange {
            y_start: row_y_start,
            y_end: row_y_start + max_row_h,
            idx_start: row_start_idx,
            idx_end: layouts.len(),
        });
    }
    (layouts, rows)
}

/// ER 标签页主内容：管理加载状态并渲染画布。
fn er_diagram_content(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 首次渲染时触发自动加载（不重复：进行中或已有结果则跳过）。
    ensure_er_graph_loaded(tab_id, er, this, cx);

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(colors.content_bg)
        .text_color(colors.text)
        .child(er_toolbar(tab_id, er, this, colors, cx))
        .child(
            if let Some(error) = this.er_errors.get(&tab_id) {
                er_error_state(tab_id, er, error.clone(), colors, cx).into_any_element()
            } else if this.er_load_tasks.contains_key(&tab_id)
                || !this.er_graphs.contains_key(&tab_id)
            {
                er_loading_state(colors).into_any_element()
            } else if let Some(scene) = this.er_scenes.get(&tab_id).cloned()
                && this.er_graphs.contains_key(&tab_id)
            {
                // 全库/局部都直连同一画布；图与场景通过 Rc 共享，渲染路径不深拷贝全图。
                let viewport = this.er_viewports.get(&tab_id).copied().unwrap_or_default();
                let canvas_size = this.er_canvas_sizes.get(&tab_id).copied();
                let expandable = er.center_table.is_some();
                er_canvas_view(
                    tab_id,
                    scene,
                    viewport,
                    canvas_size,
                    expandable,
                    colors,
                    cx,
                )
                .into_any_element()
            } else {
                // scene 未就绪（加载/重载过渡）→ 短暂 loading。
                er_loading_state(colors).into_any_element()
            },
        )
}

/// 触发整库 ER 数据后台加载：首次打开自动生成。
/// 用 `er_load_tasks` 与 `er_graphs` 双重守卫，避免渲染期重复发起。
fn ensure_er_graph_loaded(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) {
    if this.er_load_tasks.contains_key(&tab_id) || this.er_graphs.contains_key(&tab_id) {
        return;
    }
    let Some(config) = this
        .controller
        .connection_configs()
        .into_iter()
        .find(|c| c.id == er.connection_id)
    else {
        // 连接配置缺失：记录错误，UI 显示重试而非空图。
        this.er_errors
            .insert(tab_id, "连接配置不存在，请重新连接后重试".to_string());
        return;
    };
    let database = er.database.clone();
    let schema = er.schema.clone();
    let center_table = er.center_table.clone();
    // 展开深度（跳数）按 tab 隔离，默认 1 跳；切深度后清缓存重载。
    let depth = this.er_depths.get(&tab_id).copied().unwrap_or(1);
    // 单节点式展开的额外种子（点某节点后加入），随重载一并扩 1 跳。
    let extra = this.er_expanded.get(&tab_id).cloned().unwrap_or_default();
    let task = cx.spawn(async move |view, cx| {
        let result: fluxdb_core::Result<(ErGraphData, ErScene)> = cx
            .background_spawn(async move {
                // 数据读取与场景几何构建都在后台线程，避免大库布局/建索引阻塞 UI。
                let graph = match &center_table {
                    // 当前表关联 ER：以该表为中心 depth 跳邻域 + 显式展开节点。
                    Some(center) => fluxdb_app::load_er_neighborhood_in_background(
                        &config,
                        Some(&database),
                        schema.as_deref(),
                        center,
                        depth,
                        &extra,
                    ),
                    None => fluxdb_app::load_er_graph_in_background(
                        &config,
                        Some(&database),
                        schema.as_deref(),
                    ),
                }?;
                let scene = build_er_scene(&graph);
                Ok((graph, scene))
            })
            .await;
        view.update(cx, |this, cx| {
            match result {
                Ok((graph, scene)) => {
                    // 全库直连同一画布；预计算全图轻量场景（布局/端点/行索引）一次算好。
                    // 不重置视口：重载保留平移；新 tab 无条目则 pan=0。
                    tracing::debug!(
                        tab = tab_id.0,
                        tables = graph.tables.len(),
                        edges = graph.edges.len(),
                        scene_nodes = scene.layouts.len(),
                        scene_rows = scene.rows.len(),
                        scene_edges = scene.edges.len(),
                        "ER 场景构建完成"
                    );
                    this.er_scenes.insert(tab_id, Rc::new(scene));
                    this.er_graphs.insert(tab_id, graph);
                    this.er_errors.remove(&tab_id);
                }
                Err(err) => {
                    this.er_errors.insert(tab_id, err.message);
                }
            }
            this.er_load_tasks.remove(&tab_id);
            cx.notify();
        })
        .ok();
    });
    this.er_load_tasks.insert(tab_id, task);
}

/// 顶部工具栏：标题 + 当前表 ER 的展开深度切换。
fn er_toolbar(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let current_depth = this.er_depths.get(&tab_id).copied().unwrap_or(1);
    let mut base = div()
        .h(px(36.))
        .flex()
        .items_center()
        .px(px(12.))
        .gap(px(12.))
        .border_b_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(match &er.center_table {
                    Some(table) => format!("{table} 的关联 ER 关系图（{current_depth} 跳）"),
                    None => format!("{} 的 ER 关系图", er.database),
                }),
        );

    // 当前表关联 ER 才支持逐层展开：1/2/3 跳切换，切换即清缓存重载。
    if er.center_table.is_some() {
        base = base.child(er_depth_selector(tab_id, current_depth, colors, cx));
    }
    base
}

/// 展开深度选择按钮组：点选后清该 tab 图缓存并重载 neighborhood。
fn er_depth_selector(
    tab_id: TabId,
    current: u8,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(div().text_size(px(12.)).text_color(colors.muted).child("展开深度"))
        .children((1..=3).map(|depth| {
            let active = depth == current;
            div()
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(colors.radius)
                .border_1()
                .border_color(if active { colors.text } else { colors.border })
                .bg(if active { colors.panel_alt } else { colors.panel_bg })
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if active { colors.text } else { colors.muted })
                .cursor_pointer()
                .hover(|style| style.bg(colors.hover))
                .child(format!("{depth} 跳"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if this.er_depths.get(&tab_id).copied().unwrap_or(1) != depth {
                            // 切深度：清图缓存 + 错误，触发 ensure_er_graph_loaded 以新深度重载。
                            this.er_depths.insert(tab_id, depth);
                            this.er_graphs.remove(&tab_id);
                            this.er_errors.remove(&tab_id);
                            this.er_load_tasks.remove(&tab_id);
                            cx.notify();
                        }
                    }),
                )
        }))
}

/// 加载中状态。
fn er_loading_state(colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.muted)
        .text_size(px(13.))
        .child("正在读取表结构与外键关系…")
}

/// 加载失败状态：错误文案 + 重试。
fn er_error_state(
    tab_id: TabId,
    er: &ErDiagramState,
    message: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let retry_tab_id = tab_id;
    let retry_er = er.clone();
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .text_color(colors.text)
        .child(
            div()
                .max_w(px(480.))
                .text_center()
                .text_color(colors.muted)
                .text_size(px(13.))
                .child(format!("读取 ER 关系图失败：{message}")),
        )
        .child(
            Button::new("er_retry")
                .label("重试")
                .on_click(cx.listener(move |this, _, _, cx| {
                    // 清错误与结果缓存，触发 ensure_er_graph_loaded 重新加载。
                    this.er_errors.remove(&retry_tab_id);
                    this.er_graphs.remove(&retry_tab_id);
                    let er = retry_er.clone();
                    ensure_er_graph_loaded(retry_tab_id, &er, this, cx);
                    cx.notify();
                })),
        )
}

/// 画布视图：二维虚拟化。只对「视口 + overscan」内的节点创建控件、
/// 只把相交的连线交给绘制层；平移只改 pan 与可见集合，不重建全图布局。
/// 画布实际尺寸由 ErCanvasProbe 回报（首帧未知时用估算值，测量到位后重渲染）。
fn er_canvas_view(
    tab_id: TabId,
    scene: Rc<ErScene>,
    viewport: ErViewport,
    canvas_size: Option<(f32, f32)>,
    expandable: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let current_tab = tab_id;
    // 首帧画布尺寸未测量：用估算值先渲染少量节点，probe 上报真实尺寸后重渲染。
    let (canvas_w, canvas_h) = canvas_size.unwrap_or((960.0, 640.0));
    let (wx0, wy0, wx1, wy1) = viewport.visible_world_bounds(canvas_w, canvas_h);
    // 可选：聚合计时（默认关闭）。可见查询先计时；元素构建耗时在函数末尾取样。
    let q_start = ER_PERF_DEBUG.then(std::time::Instant::now);
    // 节点可见性：行索引二分 + 逐节点完整 x/y 相交，只对结果创建控件。
    let visible_nodes: Vec<usize> = scene.visible_nodes(wx0, wy0, wx1, wy1);
    // 连线可见性：y 带索引筛候选 + 精确线段相交；两端在屏外但穿过视口的线段保留。
    let visible_edges: Vec<ErEdge> = scene.visible_edges(wx0, wy0, wx1, wy1);
    if let Some(t0) = q_start {
        ER_T_QUERY.with(|t| t.borrow_mut().snapshot(t0.elapsed()));
    }

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .overflow_hidden()
                // 尺寸探针：回报画布真实可见区域（含窗口/侧边栏/工具栏变化），驱动可见集合重算。
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(ErCanvasProbe {
                            tab_id,
                            view: cx.entity().clone(),
                        }),
                )
                // 空白处按住拖动平移画布（节点上点击是展开，见 node_view 的 stop_propagation）。
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, _cx| {
                        this.er_viewport_drag = Some((
                            current_tab,
                            f32::from(event.position.x),
                            f32::from(event.position.y),
                            this.er_viewports
                                .get(&current_tab)
                                .map(|v| v.pan_x)
                                .unwrap_or(0.0),
                            this.er_viewports
                                .get(&current_tab)
                                .map(|v| v.pan_y)
                                .unwrap_or(0.0),
                        ));
                    }),
                )
                .on_mouse_move(
                    cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        if let Some((tab, down_x, down_y, pan_x, pan_y)) = this.er_viewport_drag {
                            // 拖到画布外松开鼠标时 move 事件仍在窗口内派发：
                            // 左键已不在按下态则结束拖动，避免拖动状态卡住。
                            if event.pressed_button != Some(MouseButton::Left) {
                                this.er_viewport_drag = None;
                                cx.notify();
                            } else if tab == current_tab {
                                let vp = this.er_viewports.entry(tab).or_default();
                                vp.pan_x = pan_x + (f32::from(event.position.x) - down_x);
                                vp.pan_y = pan_y + (f32::from(event.position.y) - down_y);
                                cx.notify();
                            }
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, _| {
                        if this
                            .er_viewport_drag
                            .as_ref()
                            .is_some_and(|(tab, _, _, _, _)| *tab == current_tab)
                        {
                            this.er_viewport_drag = None;
                        }
                    }),
                )
                // 连线层：铺满可见区域，paint 只画传入的可见边（坐标经同一 pan 变换）。
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(ErCanvas {
                            edges: visible_edges,
                            color: colors.border,
                            pan_x: viewport.pan_x,
                            pan_y: viewport.pan_y,
                        }),
                )
                // 节点层：只挂载可见节点；字段展示文本已在场景构建期预拼（layout.columns），
                // 渲染只复用不 format!。
                .children(visible_nodes.into_iter().map(|i| {
                    let layout = &scene.layouts[i];
                    node_view(tab_id, layout, expandable, viewport, colors, cx)
                })),
        )
}

/// 画布尺寸探针：铺满画布可见区域，paint 阶段回报真实 bounds 尺寸。
/// 尺寸变化才写入并 notify，避免「测量 ↔ notify」无休止重绘。
/// 首帧在测量到位前用估算值渲染；窗口缩放、侧边栏宽度变化都会触发一次重算。
struct ErCanvasProbe {
    tab_id: TabId,
    view: gpui::Entity<NavicatMain>,
}

impl IntoElement for ErCanvasProbe {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ErCanvasProbe {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        Some(("er-canvas-probe", self.tab_id.0).into())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // 铺满画布可见区域（flex_1 + overflow_hidden 的父容器）。
        let mut style = gpui::Style::default();
        style.size = gpui::Size::full();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let tab_id = self.tab_id;
        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        // 窗口未完成布局时可能测到 0：不记录，等下一帧有效尺寸。
        if size.0 <= 0.0 || size.1 <= 0.0 {
            return;
        }
        self.view.update(cx, |this, cx| {
            if this.er_canvas_sizes.get(&tab_id) != Some(&size) {
                // 尺寸变化才记录：画布真实可用区域 + 该尺寸下的可见节点/边数。
                // 仅首次测量与窗口/侧边栏变化时触发，不逐帧刷日志。
                if let Some(scene) = this.er_scenes.get(&tab_id).cloned() {
                    let vp = this.er_viewports.get(&tab_id).copied().unwrap_or_default();
                    let (wx0, wy0, wx1, wy1) =
                        vp.visible_world_bounds(size.0, size.1);
                    let visible_nodes = scene.visible_nodes(wx0, wy0, wx1, wy1).len();
                    let visible_edges = scene.visible_edges(wx0, wy0, wx1, wy1).len();
                    tracing::debug!(
                        tab = tab_id.0,
                        canvas_w = size.0,
                        canvas_h = size.1,
                        pan_x = vp.pan_x,
                        pan_y = vp.pan_y,
                        visible_nodes,
                        visible_edges,
                        "ER 画布尺寸测量"
                    );
                }
                this.er_canvas_sizes.insert(tab_id, size);
                cx.notify();
            }
        });
        // paint 内已直接更新实体状态，无需再走 window 绘制。
        let _ = window;
    }
}

/// 单个表节点：表名标题 + 逐列。absolute 定位到世界坐标 + 视口平移。
/// 字段展示文本已在场景构建期预拼（layout.columns），这里只复用，不做每帧 format!。
/// 仅「当前表关联 ER」（expandable）支持点击单节点展开；全库 ER 点击不触发重载。
fn node_view(
    tab_id: TabId,
    layout: &ErNodeLayout,
    expandable: bool,
    viewport: ErViewport,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let expand_table = layout.name.clone();
    let mut node = div()
        .absolute()
        .left(px(layout.x + viewport.pan_x))
        .top(px(layout.y + viewport.pan_y))
        .w(px(NODE_WIDTH))
        .h(px(layout.height))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .flex()
        .flex_col();
    if expandable {
        node = node.cursor_pointer().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                // 节点点击是「单节点式展开」而非画布平移，阻断事件冒泡。
                cx.stop_propagation();
                // 单节点展开：把该表加入显式种子，清缓存重载（neighborhood 会以它扩 1 跳）。
                // 仅局部邻域模式；全库 ER 不走这里，避免无意义的全库重载。
                this.er_expanded
                    .entry(tab_id)
                    .or_default()
                    .insert(expand_table.clone());
                this.er_graphs.remove(&tab_id);
                this.er_errors.remove(&tab_id);
                this.er_load_tasks.remove(&tab_id);
                cx.notify();
            }),
        );
    }
    node
        .child(
            div()
                .h(px(NODE_HEADER))
                .px(px(8.))
                .flex()
                .items_center()
                .bg(colors.panel_alt)
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .overflow_hidden()
                .text_ellipsis()
                .child(layout.name.clone()),
        )
        .children(
            layout.columns.iter().map(|col| {
                let primary = col.primary;
                div()
                    .h(px(NODE_ROW))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .w(px(6.))
                            .h(px(6.))
                            .rounded_full()
                            .bg(if primary {
                                rgb(0xf59e0b)
                            } else {
                                colors.muted
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .font_weight(if primary {
                                gpui::FontWeight::SEMIBOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(if primary { colors.text } else { colors.muted })
                            .child(col.text.clone()),
                    )
            }),
        )
}

/// 连线画布 Element：仅画传入的可见外键线段（构建前已按视口 bbox 裁剪），
/// 节点由上方 div 渲染。线段存世界坐标，paint 时统一加平移，与节点 div 对齐。
struct ErCanvas {
    /// 可见连线（世界坐标端点）。
    edges: Vec<ErEdge>,
    /// 线/描边颜色。
    color: gpui::Rgba,
    /// 视口平移（屏幕偏移），与 node_view 的绝对定位共用同一 pan。
    pan_x: f32,
    pan_y: f32,
}

impl IntoElement for ErCanvas {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ErCanvas {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // 由父 div 压入的尺寸决定，占满即可。
        let mut style = gpui::Style::default();
        style.size = gpui::Size::full();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        ()
    }
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        // 节点坐标是相对画布 div 的局部坐标；paint 阶段加平移 + 本元素 bounds.origin
        // 才能落到实际屏幕位置（与节点 div 的视口平移对齐），否则线会整体偏移/被裁。
        for e in &self.edges {
            // PathBuilder 的 move_to/line_to 返回 ()，不能链式；逐条构建路径。
            let mut builder = gpui::PathBuilder::stroke(px(1.5));
            builder.move_to(point(
                px(e.ax + self.pan_x) + bounds.origin.x,
                px(e.ay + self.pan_y) + bounds.origin.y,
            ));
            builder.line_to(point(
                px(e.bx + self.pan_x) + bounds.origin.x,
                px(e.by + self.pan_y) + bounds.origin.y,
            ));
            if let Ok(path) = builder.build() {
                window.paint_path(path, self.color);
            }
        }
    }
}
