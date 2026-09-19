// ER 关系图「画布原型」标签页的绘制与加载触发（er-design.md §8 步骤 3）。
//
// 分层：本文件只做 desktop 侧渲染与按 tab 缓存。数据库读取编排在 fluxdb-app 的
// er_service.rs（load_er_graph_in_background），这里绝不拼 SQL、不直接摸驱动。
// 首次打开自动生成：标签一进入 content 渲染即触发加载，无额外「生成」按钮。

// —— 固定栅格布局参数（首版不缩放不平移，直接按计算坐标排布）——
// ponytail: 固定列数网格，无平移/缩放/hit-test。需要大库探索时再引入
// ErViewportController 变换与视口裁剪。单屏列数随节点数自适应（少于 4 表不空列）。
const NODE_WIDTH: f32 = 220.0; // 表节点宽 px
const NODE_HEADER: f32 = 30.0; // 表名标题高 px
const NODE_ROW: f32 = 22.0; // 每列行高 px
const NODE_GAP_X: f32 = 60.0; // 节点横向间距
const NODE_GAP_Y: f32 = 30.0; // 节点纵向间距
const COLUMNS: usize = 5; // 每行节点数
/// 画布单次渲染的表节点数上限。超过则降级只渲染前 MAX 张（见 er_truncation_banner）。
/// 150 张内流畅，1500 张一次性挂载会卡；上限取安全余量，先保可用再分组。
const MAX_CANVAS_TABLES: usize = 300;

/// ER 画布视口：平移（pan）。冲屏坐标 = 世界坐标 + pan。
/// 节点 div 定位、连线 canvas 与可见裁剪共用同一 pan，保证三端不漂移。
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

/// 表节点布局：世界坐标下的矩形与列数，供节点 div 与连线 canvas 共用，
/// 保证连线锚点与节点边缘严格对齐。
struct ErNodeLayout {
    name: String,
    x: f32,
    y: f32,
    column_count: usize,
}

/// 画布场景缓存：布局 + 连线端点一次算好，渲染复用避免每帧重算（design §4）。
struct ErScene {
    content_w: f32,
    content_h: f32,
    layouts: Vec<ErNodeLayout>,
    /// 连线端点（世界坐标），元素为 (起点, 终点)。
    edges: Vec<((f32, f32), (f32, f32))>,
    /// 截断前的原始表数；0 表示未截断（无降级提示）。
    original_tables: usize,
}

/// 由图表构建一次画布场景：布局 + 连线端点到节点边缘中点。
/// 连线端点复用 er_layout 的节点坐标，保证与节点 div 严格对齐。
fn build_er_scene(graph: &ErGraphData, original_tables: usize) -> ErScene {
    let (content_w, content_h, layouts) = er_layout(graph);
    let mut edges = Vec::new();
    for edge in &graph.edges {
        let Some(from) = layouts.iter().find(|l| l.name == edge.from_table) else {
            continue; // 被引用到未加载/不存在的表：跳过该线。
        };
        let Some(to) = layouts.iter().find(|l| l.name == edge.to_table) else {
            continue;
        };
        // 起点取左侧边缘中点的表，终点取另一侧边缘中点：连线在节点间隙走。
        let (from_x, from_y) = if from.x <= to.x {
            (from.x + NODE_WIDTH, from.y + header_h(from) / 2.0)
        } else {
            (from.x, from.y + header_h(from) / 2.0)
        };
        let (to_x, to_y) = if to.x >= from.x {
            (to.x, to.y + header_h(to) / 2.0)
        } else {
            (to.x + NODE_WIDTH, to.y + header_h(to) / 2.0)
        };
        edges.push(((from_x, from_y), (to_x, to_y)));
    }
    ErScene {
        content_w,
        content_h,
        layouts,
        edges,
        original_tables,
    }
}

/// 超大库降级截断：超过上限只保留前 MAX 张表（按名排序）与两端都在内的边。
/// 返回 (截断后图, 原始表数)。未超限时返回克隆图，original=len（scene 判 0 为未截断，故用 0）。
fn truncate_er_graph(graph: &ErGraphData) -> (ErGraphData, usize) {
    if graph.tables.len() <= MAX_CANVAS_TABLES {
        return (graph.clone(), 0);
    }
    let original = graph.tables.len();
    let mut visible = graph.clone();
    visible.tables.sort_by(|a, b| a.name.cmp(&b.name));
    visible.tables.truncate(MAX_CANVAS_TABLES);
    let visible_names: std::collections::BTreeSet<String> =
        visible.tables.iter().map(|t| t.name.clone()).collect();
    visible.edges.retain(|e| {
        visible_names.contains(&e.from_table) && visible_names.contains(&e.to_table)
    });
    (visible, original)
}

/// 由纯数据图算出每个节点的世界坐标 + 内容整体尺寸。
/// 返回 (整体宽, 整体高, 各节点布局)。连线锚点取节点矩形左右边缘垂直中点。
fn er_layout(graph: &ErGraphData) -> (f32, f32, Vec<ErNodeLayout>) {
    let mut layouts = Vec::with_capacity(graph.tables.len());
    let mut max_row_h: f32 = 0.0; // 当前行最大高度（决定下一行起点）
    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;
    let mut col = 0usize;
    for table in &graph.tables {
        let h = NODE_HEADER + table.columns.len() as f32 * NODE_ROW + 8.0;
        layouts.push(ErNodeLayout {
            name: table.name.clone(),
            x,
            y,
            column_count: table.columns.len(),
        });
        if h > max_row_h {
            max_row_h = h;
        }
        col += 1;
        if col >= COLUMNS {
            col = 0;
            x = 0.0;
            y += max_row_h + NODE_GAP_Y;
            max_row_h = 0.0;
        } else {
            x += NODE_WIDTH + NODE_GAP_X;
        }
    }
    if col != 0 {
        y += max_row_h + NODE_GAP_Y;
    }
    let width = (COLUMNS.min(graph.tables.len().max(1)) as f32) * (NODE_WIDTH + NODE_GAP_X) - NODE_GAP_X;
    (width.max(NODE_WIDTH), y, layouts)
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
            } else {
                // 预计算场景存在才渲染画布（布局/连线端点缓存，配合图一起更新）。
                match this.er_scenes.get(&tab_id).cloned() {
                    Some(scene) => {
                        let graph = this.er_graphs.get(&tab_id).cloned().unwrap_or_default();
                        let viewport =
                            this.er_viewports.get(&tab_id).copied().unwrap_or_default();
                        er_canvas_view(tab_id, scene, &graph, viewport, colors, cx)
                            .into_any_element()
                    }
                    // 图已加载但场景未就绪（不应发生，防御）→ 短暂 loading。
                    None => er_loading_state(colors).into_any_element(),
                }
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
        let result = cx
            .background_spawn(async move {
                match &center_table {
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
                }
            })
            .await;
        view.update(cx, |this, cx| {
            match result {
                Ok(graph) => {
                    // 超大库降级截断 + 预计算画布场景（布局/连线端点一次算好），
                    // 渲染复用避免每帧重算；新图重置视口（平移回到原点）。
                    let (graph, original) = truncate_er_graph(&graph);
                    this.er_scenes.insert(tab_id, Rc::new(build_er_scene(&graph, original)));
                    // 不重置视口：单节点展开/深度切换重载保留当前平移，避免点节点后视野跳回原点。
                    // 首次打开新 tab 时 er_viewports 无条目，渲染 fallback 到 pan=0。
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

/// 画布视图：复用预计算场景（布局+连线端点缓存），节点 div 在上、连线 canvas 在下。
fn er_canvas_view(
    tab_id: TabId,
    scene: Rc<ErScene>,
    graph: &ErGraphData,
    viewport: ErViewport,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let content_w = scene.content_w;
    let content_h = scene.content_h;
    let truncated = scene.original_tables > 0;
    let original_len = scene.original_tables;
    let current_tab = tab_id;

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(colors.content_bg)
        .when(truncated, |this| {
            this.child(er_truncation_banner(original_len, colors))
        })
        .child(
            div()
                .id(("er-canvas-scroll", tab_id.0))
                .flex_1()
                .min_h_0()
                .overflow_hidden()
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
                            if tab == current_tab {
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
                .child(
                    // 连接线层：铺满内容尺寸，paint 阶段用 PathBuilder 画线，坐标过视口平移。
                    div()
                        .relative()
                        .w(px(content_w))
                        .h(px(content_h))
                        .child(ErCanvas {
                            edges: scene.edges.clone(),
                            color: colors.border,
                            pan_x: viewport.pan_x,
                            pan_y: viewport.pan_y,
                        })
                        .children(scene.layouts.iter().map(|layout| {
                            node_view(tab_id, layout, graph, viewport, colors, cx)
                        })),
                ),
        )
}

/// 超大库降级提示条：告知表数超出画布单次承载，展示数量与后续方向。
fn er_truncation_banner(total: usize, colors: UiColors) -> Div {
    div()
        .h(px(32.))
        .px(px(12.))
        .flex()
        .items_center()
        .bg(rgb(0xfef3c7).opacity(0.35))
        .border_b_1()
        .border_color(colors.border_soft)
        .text_size(px(12.))
        .text_color(rgb(0x92400e))
        .child(format!(
            "该库共 {total} 张表，超出画布单次承载（{MAX_CANVAS_TABLES}），仅显示部分；后续将支持分组/局部视图"
        ))
}

fn header_h(layout: &ErNodeLayout) -> f32 {
    NODE_HEADER + layout.column_count as f32 * NODE_ROW + 8.0
}

/// 单个表节点：表名标题 + 逐列。absolute 定位到世界坐标 + 视口平移。
/// 点击节点触发「单节点式展开」：把该表加入显式展开种子，重载后其更深层邻居并入。
fn node_view(
    tab_id: TabId,
    layout: &ErNodeLayout,
    graph: &ErGraphData,
    viewport: ErViewport,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // layout 由同一份 graph 生成，故名必命中；找不到则回退空列节点（不应发生）。
    let columns = graph
        .tables
        .iter()
        .find(|t| t.name == layout.name)
        .map(|t| t.columns.clone())
        .unwrap_or_default();
    let node_h = header_h(layout);
    let expand_table = layout.name.clone();
    div()
        .absolute()
        .left(px(layout.x + viewport.pan_x))
        .top(px(layout.y + viewport.pan_y))
        .w(px(NODE_WIDTH))
        .h(px(node_h))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .flex()
        .flex_col()
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                // 节点点击是「单节点式展开」而非画布平移，阻断事件冒泡。
                cx.stop_propagation();
                // 单节点展开：把该表加入显式种子，清缓存重载（neighborhood 会以它扩 1 跳）。
                this.er_expanded.entry(tab_id).or_default().insert(expand_table.clone());
                this.er_graphs.remove(&tab_id);
                this.er_errors.remove(&tab_id);
                this.er_load_tasks.remove(&tab_id);
                cx.notify();
            }),
        )
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
            columns
                .iter()
                .map(|col| {
                    let primary = col.primary_key;
                    let text = if let Some(ty) = &col.type_name {
                        format!("{}  {}", col.name, ty)
                    } else {
                        col.name.clone()
                    };
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
                                .child(text),
                        )
                }),
        )
}

/// 连线画布 Element：仅画外键线段，节点由上方 div 渲染。
/// 线段存世界坐标，paint 时统一加平移，与节点 div 的视口平移保持一致。
struct ErCanvas {
    /// 每对 ((起点 x,y),(终点 x,y)) 的世界坐标线段。
    edges: Vec<((f32, f32), (f32, f32))>,
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
        // 节点坐标是相对内容 div 的局部坐标；paint 阶段加平移 + 本元素 bounds.origin
        // 才能落到实际屏幕位置（与节点 div 的视口平移对齐），否则线会整体偏移/被裁。
        for ((ax, ay), (bx, by)) in &self.edges {
            // PathBuilder 的 move_to/line_to 返回 ()，不能链式；逐条构建路径。
            let mut builder = gpui::PathBuilder::stroke(px(1.5));
            builder.move_to(point(
                px(*ax + self.pan_x) + bounds.origin.x,
                px(*ay + self.pan_y) + bounds.origin.y,
            ));
            builder.line_to(point(
                px(*bx + self.pan_x) + bounds.origin.x,
                px(*by + self.pan_y) + bounds.origin.y,
            ));
            if let Ok(path) = builder.build() {
                window.paint_path(path, self.color);
            }
        }
    }
}
