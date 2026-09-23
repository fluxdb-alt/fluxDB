// ER 关系画布标签页编排（er-ui-relationship-canvas.md）。
//
// 本文件只做 desktop 侧加载编排与「画布/工具栏/状态」组装：
// - 阶段流：表目录先到（临时概览排列，可操作）→ 关系索引后台补线 → 首次关系就绪且
//   用户尚未操作时自动应用一次关系布局（§6.2）；已操作则保留坐标并提供「按关系排列」。
// - 字段按可见表按需到达；字段/关系变更只重建场景拓扑，不重排已有坐标、不清缓存。
// - 坐标/滚动/选中/固定为可变状态（按表名），场景为不可变拓扑；渲染期物化本帧。
// 元数据读取编排在 fluxdb-app（er_service/er_catalog/er_layout），这里绝不拼 SQL。

/// 取当前主题强调色（选中轮廓/高亮连线共用）。
fn er_accent_color(cx: &Context<NavicatMain>) -> gpui::Rgba {
    let mut hsla = ComponentTheme::global(cx).primary;
    hsla.s *= 0.55; // 低饱和
    hsla.into()
}

/// 表身份相等：按 (schema, name) 比较。忽略 database（同一视图内同库）；
/// 结构化身份保证跨 schema 同名与含点标识符不误配，不按展示名/`.split` 拆。
fn er_table_ref_eq(a: &fluxdb_core::ErTableRef, b: &fluxdb_core::ErTableRef) -> bool {
    a.schema == b.schema && a.name == b.name
}

/// 本地时间显示串（`HH:MM:SS`），用于 ER「更新时间」（§3.5 缓存时效反馈）。
fn now_local_time_display() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // UTC→本地不做偏置方法依赖；这里给 UTC 时间带明确标记，三端一致、可读又诚实。
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02} UTC")
}

/// 关系面板可拖宽范围（px，GPUI 逻辑像素）。
const ER_REL_PANEL_MIN_W: f32 = 320.0;
const ER_REL_PANEL_MAX_W: f32 = 800.0;
/// 默认宽度（打开时）；若可用空间更小则退而用之。
const ER_REL_PANEL_DEFAULT_W: f32 = 560.0;

/// 把关系面板宽度约束到 [min, min(max, 可用宽)]；窗口缩小时随之收缩，不超出界面。
fn clamp_er_rel_panel_width(width: f32, available_w: f32) -> f32 {
    let max = ER_REL_PANEL_MAX_W.min(available_w.max(ER_REL_PANEL_MIN_W));
    let min = ER_REL_PANEL_MIN_W.min(available_w.max(1.0));
    width.clamp(min, max)
}

/// ER 标签页主内容。
fn er_diagram_content(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 表搜索输入框（§五.6）：首次渲染本 tab 时惰性创建输入实体 + 订阅（查询变化更新字符串并重绘）。
    if !this.er_search_input.contains_key(&tab_id) {
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("搜索表…"));
        let sub_tab = tab_id;
        let sub = cx.subscribe(&search_input, move |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let txt = input.read(cx).value().to_string();
                this.er_search_query.insert(sub_tab, txt);
                this.er_search_sel.insert(sub_tab, 0);
                cx.notify();
            }
        });
        this.er_search_input.insert(tab_id, search_input);
        this.er_search_subs.insert(tab_id, sub);
    }

    // 作用域持久化 key（打开时确定）——重复打开复用同一 tab，key 不变。
    if !this.er_scope_keys.contains_key(&tab_id) {
        let key = format!(
            "{}:{}:{}",
            er.connection_id.0,
            er.database,
            er.center_table.as_ref().map(|r| r.display()).unwrap_or_default()
        );
        this.er_scope_keys.insert(tab_id, key);
        this.er_generation.entry(tab_id).or_insert(0);
        let relationship_key = format!(
            "{}:{}:{}",
            er.connection_id.0,
            er.database,
            er.schema.as_deref().unwrap_or_default()
        );
        this.er_relationship_scope_keys.insert(tab_id, relationship_key);
    }
    this.ensure_er_relationships_loaded(tab_id, cx);
    // 首次打开本 tab：应用上次保存的分组/坐标/固定（§十 分组折叠态持久化）。
    this.er_restore_view_state(tab_id);

    // 相位编排：表目录 → 场景（临时排列）→ 字段按需 → 关系索引 + 一次性关系布局。
    ensure_er_graph_loaded(tab_id, er, this, cx);
    maybe_build_scene(tab_id, this);
    sync_er_relations(tab_id, er, this, cx);
    request_visible_columns(tab_id, er, this, cx);

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(colors.canvas_bg)
        .text_color(colors.text)
        .child(er_toolbar(tab_id, er, this, colors, cx))
        .child(er_search_bar(tab_id, this, colors, cx))
        .child(er_group_bar(tab_id, this, window, colors, cx))
        .child(
            if let Some(error) = this.er_errors.get(&tab_id) {
                er_error_state(tab_id, er, error.clone(), colors, cx).into_any_element()
            } else if !this.er_full_tables.contains_key(&tab_id)
                && !(this.er_refreshing.contains(&tab_id) && this.er_graphs.contains_key(&tab_id))
            {
                // 刷新中但保留可用旧图 → 继续渲染画布，仅工具栏显示「刷新中…」（§3.5 刷新失败保留旧图）。
                er_loading_state(colors).into_any_element()
            } else if this.er_full_tables.get(&tab_id).is_some_and(|t| t.is_empty()) {
                // 库内无表：明确反馈，不能被当成空白画布（§3.5 空库）。
                er_empty_state("该数据库暂无表", er, tab_id, colors, cx).into_any_element()
            } else {
                // Render 属 GPUI「cannot unwind」路径：所有「读」用一次 borrow()（不可变借用）
                // 且借用到物化结束即释放，绝不与后续 borrow_mut 嵌套 → 杜绝 RefCell 双重借用 abort。
                let canvas = this.er_canvas.borrow();
                let viewport = canvas.er_viewports.get(&tab_id).copied().unwrap_or_default();
                let canvas_size = canvas.er_canvas_sizes.get(&tab_id).copied();
                let expandable = er.center_table.is_some();
                let relation_status = this
                    .er_graphs
                    .get(&tab_id)
                    .map(|g| g.relation_status)
                    .unwrap_or(ErLoadStatus::NotLoaded);
                let scene = this.er_scenes.get(&tab_id).cloned();
                // 物化本帧：网格查询可见节点 + 字段端口路由（拖动/滚动只改状态，下一帧生效）。
                let frame = scene.as_ref().map(|s| {
                    let positions = canvas.er_scene_positions.get(&tab_id).cloned().unwrap_or_default();
                    let selected = canvas.er_selected_table.get(&tab_id).cloned().flatten();
                    let pinned = canvas.er_pinned.get(&tab_id).cloned().unwrap_or_default();
                    let env = build_env(s, tab_id, &positions, &canvas.er_node_scroll_px, selected.as_deref(), &pinned);
                    s.materialize(&env, viewport, canvas_size.unwrap_or((960.0, 640.0)).0, canvas_size.unwrap_or((960.0, 640.0)).1)
                });
                drop(canvas);
                // 缓存可见折线供空白点击的关系线命中（有界：仅本帧可见）。
                if let Some(f) = &frame {
                    let mut c = this.er_canvas.borrow_mut();
                    c.er_frame_edges.insert(tab_id, f.edge_views.clone());
                    // 端口命中测试需要本帧节点视图（真实几何），与折线缓存同源同生命周期。
                    c.er_frame_nodes.insert(tab_id, f.node_views.clone());
                } else {
                    let mut c = this.er_canvas.borrow_mut();
                    c.er_frame_edges.remove(&tab_id);
                    c.er_frame_nodes.remove(&tab_id);
                }
                // 画布浮层数据（端口圆点/临时连线/原点）：由状态读出后传入视图层。
                let overlay = {
                    let c = this.er_canvas.borrow();
                    ErCanvasOverlay {
                        row_hover: c.er_row_hover.get(&tab_id).cloned().flatten(),
                        port_hover: c.er_port_hover.get(&tab_id).cloned().flatten(),
                        link_drag: c.er_link_drag.clone(),
                    }
                };
                let canvas = er_canvas_view(
                    tab_id,
                    scene,
                    frame,
                    viewport,
                    canvas_size,
                    expandable,
                    this.er_canvas.borrow().er_field_highlights.get(&tab_id).cloned(),
                    overlay,
                    colors,
                    cx,
                );
                let inner = if relation_status == ErLoadStatus::Loaded {
                    canvas.into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(er_relation_status_banner(relation_status, colors))
                        .child(canvas)
                        .into_any_element()
                };
                // 搜索命中浮层与右下角小地图作为画布后置兄弟叠放其上（GPUI 无 z-index，
                // 后置者在上）。空时二者返回空/透明，不遮挡画布。
                // 画布包裹层必须自身是 flex 容器（flex_col），否则其 flex_1 子（er_canvas_view）
                // 在 block 布局下高度塌陷 → 探针测 0、不建场景 → 画布空白（12b 同款 flex 塌陷回归）。
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(inner)
                    .child(er_search_dropdown(tab_id, er, this, colors, cx))
                    .child(er_minimap_view(tab_id, colors, cx))
                    .child(er_export_menu(tab_id, er, this, colors, cx))
                    .child(
                        if this.er_relationship_panel_open.contains(&tab_id) {
                            er_relationship_panel(tab_id, this, window, colors, cx).into_any_element()
                        } else {
                            div().into_any_element()
                        },
                    )
                    .into_any_element()
            },
        )
}
