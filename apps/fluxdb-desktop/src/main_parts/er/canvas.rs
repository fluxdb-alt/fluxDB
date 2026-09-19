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

/// 表节点布局：世界坐标下的矩形与列数，供节点 div 与连线 canvas 共用，
/// 保证连线锚点与节点边缘严格对齐。
struct ErNodeLayout {
    name: String,
    x: f32,
    y: f32,
    column_count: usize,
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
        .child(er_toolbar(er, colors))
        .child(
            if let Some(error) = this.er_errors.get(&tab_id) {
                er_error_state(tab_id, er, error.clone(), colors, cx).into_any_element()
            } else if this.er_load_tasks.contains_key(&tab_id)
                || !this.er_graphs.contains_key(&tab_id)
            {
                er_loading_state(colors).into_any_element()
            } else {
                let graph = this.er_graphs.get(&tab_id).cloned().unwrap_or_default();
                er_canvas_view(tab_id, &graph, colors).into_any_element()
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
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move {
                fluxdb_app::load_er_graph_in_background(&config, Some(&database), schema.as_deref())
            })
            .await;
        view.update(cx, |this, cx| {
            match result {
                Ok(graph) => {
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

/// 顶部工具栏：标题 + 刷新（清除缓存并重新加载）。
fn er_toolbar(er: &ErDiagramState, colors: UiColors) -> Div {
    div()
        .h(px(36.))
        .flex()
        .items_center()
        .px(px(12.))
        .gap(px(8.))
        .border_b_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(format!("{} 的 ER 关系图", er.database)),
        )
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

/// 画布视图：节点 div（含文字）在上，连线 canvas 在背景，共用同一套世界坐标。
fn er_canvas_view(tab_id: TabId, graph0: &ErGraphData, colors: UiColors) -> impl IntoElement {
    let original_len = graph0.tables.len();
    // 超大库降级：超过单次画布承载上限时只渲染前 MAX 张表（按名排序），
    // 其余仅提示数量。防止 1000+ 表一次性挂载全部节点导致卡顿/无响应。
    // ponytail: 截断非分组视图，后续做 >200 表的 schema/分组概览与局部 ER（design §4.1）。
    let graph = if original_len > MAX_CANVAS_TABLES {
        let mut visible = graph0.clone();
        visible.tables.sort_by(|a, b| a.name.cmp(&b.name));
        visible.tables.truncate(MAX_CANVAS_TABLES);
        let visible_names: std::collections::BTreeSet<String> =
            visible.tables.iter().map(|t| t.name.clone()).collect();
        visible.edges.retain(|e| {
            visible_names.contains(&e.from_table) && visible_names.contains(&e.to_table)
        });
        visible
    } else {
        graph0.clone()
    };
    let truncated = original_len > MAX_CANVAS_TABLES;
    let (content_w, content_h, layouts) = er_layout(&graph);
    // 连线锚点：预先把端点到节点矩形左/右边缘中点算好，传入 canvas。
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
                .overflow_scroll()
                .child(
                    // 连接线层：铺满内容尺寸，paint 阶段用 PathBuilder 画线。
                    div()
                        .relative()
                        .w(px(content_w))
                        .h(px(content_h))
                        .child(ErCanvas {
                            edges,
                            color: colors.border,
                        })
                        .children(layouts.iter().map(|layout| node_view(layout, &graph, colors))),
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

/// 单个表节点：表名标题 + 逐列。absolute 定位到世界坐标。
fn node_view(layout: &ErNodeLayout, graph: &ErGraphData, colors: UiColors) -> Div {
    // layout 由同一份 graph 生成，故名必命中；找不到则回退空列节点（不应发生）。
    let columns = graph
        .tables
        .iter()
        .find(|t| t.name == layout.name)
        .map(|t| t.columns.clone())
        .unwrap_or_default();
    let node_h = header_h(layout);
    div()
        .absolute()
        .left(px(layout.x))
        .top(px(layout.y))
        .w(px(NODE_WIDTH))
        .h(px(node_h))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .overflow_hidden()
        .flex()
        .flex_col()
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
struct ErCanvas {
    /// 每对 ((起点 x,y),(终点 x,y)) 的世界坐标线段。
    edges: Vec<((f32, f32), (f32, f32))>,
    /// 线/描边颜色。
    color: gpui::Rgba,
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
        // 节点坐标是相对内容 div 的局部坐标；paint 阶段需加本元素 bounds.origin
        // 才能落到实际屏幕位置（与节点 div 的 GPUI 布局对齐），否则线会整体偏移/被裁。
        for ((ax, ay), (bx, by)) in &self.edges {
            // PathBuilder 的 move_to/line_to 返回 ()，不能链式；逐条构建路径。
            let mut builder = gpui::PathBuilder::stroke(px(1.5));
            builder.move_to(point(px(*ax) + bounds.origin.x, px(*ay) + bounds.origin.y));
            builder.line_to(point(px(*bx) + bounds.origin.x, px(*by) + bounds.origin.y));
            if let Ok(path) = builder.build() {
                window.paint_path(path, self.color);
            }
        }
    }
}
