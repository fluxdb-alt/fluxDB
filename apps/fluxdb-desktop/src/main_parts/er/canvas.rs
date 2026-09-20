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

impl NavicatMain {
    /// 用户已操作画布（平移/拖动/选择/字段滚动）：关系就绪后不再自动重排（§6.2）。
    fn mark_er_interacted(&mut self, tab: TabId) {
        self.er_user_interacted.insert(tab);
    }

    /// 结束节点拖动（画布外释放/窗口失焦/切标签同样调用，§7）。
    fn finish_node_drag(&mut self, tab: TabId) {
        if self.er_node_drag.as_ref().is_some_and(|(t, ..)| *t == tab) {
            self.er_node_drag = None;
        }
    }

    /// 应用关系布局：非固定表采用布局坐标；固定（手动拖动过）表保留坐标。
    /// 不重读数据库、不清字段/关系缓存（§6.3）。
    fn apply_er_layout(&mut self, tab: TabId) {
        let Some(graph) = self.er_graphs.get(&tab) else {
            return;
        };
        let layout = er_relation_layout(&graph.tables, &graph.edges);
        let pinned = self.er_pinned.get(&tab).cloned().unwrap_or_default();
        let entry = self.er_scene_positions.entry(tab).or_default();
        for t in &graph.tables {
            if pinned.contains(&t.name) {
                continue;
            }
            if let Some(&(x, y, _)) = layout.get(&t.name) {
                entry.insert(t.name.clone(), (x, y));
            }
        }
        self.er_layout_applied.insert(tab);
    }
}

/// ER 标签页主内容。
fn er_diagram_content(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
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
        .child(
            if let Some(error) = this.er_errors.get(&tab_id) {
                er_error_state(tab_id, er, error.clone(), colors, cx).into_any_element()
            } else if !this.er_full_tables.contains_key(&tab_id) {
                er_loading_state(colors).into_any_element()
            } else {
                let viewport = this.er_viewports.get(&tab_id).copied().unwrap_or_default();
                let canvas_size = this.er_canvas_sizes.get(&tab_id).copied();
                let expandable = er.center_table.is_some();
                let relation_status = this
                    .er_graphs
                    .get(&tab_id)
                    .map(|g| g.relation_status)
                    .unwrap_or(ErLoadStatus::NotLoaded);
                let scene = this.er_scenes.get(&tab_id).cloned();
                // 物化本帧：网格查询可见节点 + 字段端口路由（拖动/滚动只改状态，下一帧生效）。
                let frame = scene.as_ref().map(|s| {
                    let positions = this.er_scene_positions.get(&tab_id).cloned().unwrap_or_default();
                    let selected = this.er_selected_table.get(&tab_id).cloned().flatten();
                    let pinned = this.er_pinned.get(&tab_id).cloned().unwrap_or_default();
                    let env = build_env(s, tab_id, &positions, &this.er_node_scroll_px, selected.as_deref(), &pinned);
                    s.materialize(&env, viewport, canvas_size.unwrap_or((960.0, 640.0)).0, canvas_size.unwrap_or((960.0, 640.0)).1)
                });
                // 缓存可见折线供空白点击的关系线命中（有界：仅本帧可见）。
                if let Some(f) = &frame {
                    this.er_frame_edges.insert(tab_id, f.edge_views.clone());
                } else {
                    this.er_frame_edges.remove(&tab_id);
                }
                let canvas = er_canvas_view(
                    tab_id,
                    scene,
                    frame,
                    viewport,
                    canvas_size,
                    expandable,
                    colors,
                    cx,
                );
                if relation_status == ErLoadStatus::Loaded {
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
                }
            },
        )
}

/// 关系未就绪时的轻量提示条（不清空可用图）。
fn er_relation_status_banner(status: ErLoadStatus, colors: UiColors) -> Div {
    let (text, tone) = match status {
        ErLoadStatus::NotLoaded | ErLoadStatus::Loading => ("关系加载中，完成后可按关系排列…", colors.muted),
        ErLoadStatus::Failed => ("关系加载失败，部分连线可能缺失", rgb(0xef4444)),
        ErLoadStatus::Loaded => unreachable!("Loaded 状态不显示 banner"),
    };
    div()
        .h(px(24.))
        .flex()
        .items_center()
        .px(px(12.))
        .text_size(px(12.))
        .text_color(tone)
        .child(text)
}

/// 触发表目录后台加载（阶段 1）。
fn ensure_er_graph_loaded(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) {
    if this.er_load_tasks.contains_key(&tab_id) || this.er_full_tables.contains_key(&tab_id) {
        return;
    }
    let Some(config) = this
        .controller
        .connection_configs()
        .into_iter()
        .find(|c| c.id == er.connection_id)
    else {
        this.er_errors
            .insert(tab_id, "连接配置不存在，请重新连接后重试".to_string());
        return;
    };
    let database = er.database.clone();
    let schema = er.schema.clone();
    let center_table = er.center_table.clone();
    let controller = this.controller.clone();
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move {
                controller.er_catalog_tables(&config, Some(&database), schema.as_deref())
            })
            .await;
        view.update(cx, |this, cx| {
            match result {
                Ok(full_tables) => {
                    let display_tables: Vec<fluxdb_core::ErTableNode> = match &center_table {
                        Some(center) => full_tables
                            .iter()
                            .filter(|t| t.name == *center)
                            .cloned()
                            .collect(),
                        None => full_tables.clone(),
                    };
                    let graph = ErGraphData {
                        tables: display_tables,
                        edges: Vec::new(),
                        relation_status: ErLoadStatus::NotLoaded,
                    };
                    tracing::debug!(tab = tab_id.0, tables = full_tables.len(), shown = graph.tables.len(), "ER 表目录就绪");
                    this.er_full_tables.insert(tab_id, full_tables);
                    this.er_graphs.insert(tab_id, graph);
                    this.er_scenes.remove(&tab_id);
                    this.er_scene_positions.remove(&tab_id);
                    this.er_layout_applied.remove(&tab_id);
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

/// 场景构建：图就绪且场景未建 → 由关系布局（关系未到时为临时概览排列）构图。
/// 首次建场景时采纳布局坐标；此后字段/关系变更只重建拓扑，坐标保留。
fn maybe_build_scene(tab_id: TabId, this: &mut NavicatMain) {
    if this.er_scenes.contains_key(&tab_id) {
        return;
    }
    let Some(graph) = this.er_graphs.get(&tab_id) else {
        return;
    };
    let layout = er_relation_layout(&graph.tables, &graph.edges);
    // 首次：坐标为空 → 采纳布局；非首次（重建）→ 保留已有坐标，新表取布局位。
    let first_build = !this.er_scene_positions.contains_key(&tab_id);
    if first_build {
        let mut positions = BTreeMap::new();
        for t in &graph.tables {
            if let Some(&(x, y, _)) = layout.get(&t.name) {
                positions.insert(t.name.clone(), (x, y));
            }
        }
        this.er_scene_positions.insert(tab_id, positions);
    } else if let Some(pos) = this.er_scene_positions.get_mut(&tab_id) {
        for t in &graph.tables {
            if !pos.contains_key(&t.name)
                && let Some(&(x, y, _)) = layout.get(&t.name)
            {
                pos.insert(t.name.clone(), (x, y));
            }
        }
    }
    let scene = build_er_scene(graph, &layout);
    this.er_scenes.insert(tab_id, Rc::new(scene));
}

/// 驱动关系索引后台加载（阶段 3）并回填连线。首次就绪且用户未操作时自动应用一次布局。
fn sync_er_relations(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) {
    if !this.er_full_tables.contains_key(&tab_id) {
        return;
    }
    let current = this
        .er_graphs
        .get(&tab_id)
        .map(|g| g.relation_status)
        .unwrap_or(ErLoadStatus::NotLoaded);
    if current == ErLoadStatus::Loaded || current == ErLoadStatus::Failed {
        return;
    }
    if this.er_relation_tasks.contains_key(&tab_id) {
        return;
    }
    let Some(config) = this
        .controller
        .connection_configs()
        .into_iter()
        .find(|c| c.id == er.connection_id)
    else {
        return;
    };
    let database = er.database.clone();
    let schema = er.schema.clone();
    let center_table = er.center_table.clone();
    let depth = this.er_depths.get(&tab_id).copied().unwrap_or(1);
    let extra = this.er_expanded.get(&tab_id).cloned().unwrap_or_default();
    let controller = this.controller.clone();
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move {
                controller.er_relations(&config, &database, schema.as_deref())
            })
            .await;
        view.update(cx, |this, cx| {
            this.er_relation_tasks.remove(&tab_id);
            if let Ok(snap) = result {
                let status = snap.status;
                if status == ErLoadStatus::Loaded {
                    if let Some(center) = &center_table {
                        let full = this.er_full_tables.get(&tab_id).cloned().unwrap_or_default();
                        let included =
                            er_neighborhood_included_tables(center, depth, &extra, &snap.edges);
                        let tables: Vec<_> = full
                            .into_iter()
                            .filter(|t| included.contains(&t.name))
                            .collect();
                        let edges: Vec<_> = snap
                            .edges
                            .iter()
                            .filter(|e| included.contains(&e.from_table) && included.contains(&e.to_table))
                            .cloned()
                            .collect();
                        if let Some(graph) = this.er_graphs.get_mut(&tab_id) {
                            graph.tables = tables;
                            graph.edges = edges;
                            graph.relation_status = ErLoadStatus::Loaded;
                        }
                    } else if let Some(graph) = this.er_graphs.get_mut(&tab_id) {
                        graph.edges = snap.edges.clone();
                        graph.relation_status = ErLoadStatus::Loaded;
                    }
                    // 首次关系就绪且用户尚未操作：自动应用一次关系布局（只此一次，§6.2）。
                    if !this.er_user_interacted.contains(&tab_id) && !this.er_layout_applied.contains(&tab_id) {
                        this.apply_er_layout(tab_id);
                    }
                    // 重建场景拓扑（坐标保留，新邻域表补布局位）。
                    this.er_scenes.remove(&tab_id);
                    maybe_build_scene(tab_id, this);
                    tracing::debug!(tab = tab_id.0, "ER 关系索引就绪");
                } else if status == ErLoadStatus::Failed {
                    if let Some(graph) = this.er_graphs.get_mut(&tab_id) {
                        graph.relation_status = ErLoadStatus::Failed;
                    }
                }
            }
            cx.notify();
        })
        .ok();
    });
    this.er_relation_tasks.insert(tab_id, task);
}

/// 字段按需加载：当前视口可见表提交为读取需求（去抖合并）。
fn request_visible_columns(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) {
    let Some(scene) = this.er_scenes.get(&tab_id).cloned() else {
        return;
    };
    let Some(graph) = this.er_graphs.get(&tab_id).cloned() else {
        return;
    };
    let viewport = this.er_viewports.get(&tab_id).copied().unwrap_or_default();
    let (cw, ch) = this.er_canvas_sizes.get(&tab_id).copied().unwrap_or((960.0, 640.0));
    let positions = this.er_scene_positions.get(&tab_id).cloned().unwrap_or_default();
    let selected = this.er_selected_table.get(&tab_id).cloned().flatten();
    let pinned = this.er_pinned.get(&tab_id).cloned().unwrap_or_default();
    let env = build_env(&scene, tab_id, &positions, &this.er_node_scroll_px, selected.as_deref(), &pinned);
    let frame = scene.materialize(&env, viewport, cw, ch);

    let mut to_request = BTreeSet::new();
    for idx in frame.visible_nodes {
        let meta = &scene.nodes[idx];
        let status = graph.tables.iter().find(|t| t.name == meta.name).map(|t| t.status);
        if matches!(status, Some(ErLoadStatus::NotLoaded) | None) {
            to_request.insert(meta.name.clone());
        }
    }
    if to_request.is_empty() {
        return;
    }
    let entry = this.er_pending_columns.entry(tab_id).or_default();
    let before = entry.len();
    entry.extend(to_request);
    let connection_id = er.connection_id;
    let database = er.database.clone();
    let schema = er.schema.clone();
    if before == 0 || this.er_column_debounce_tasks.get(&tab_id).is_none() {
        let debounce_tab = tab_id;
        let task = cx.spawn(async move |view, cx| {
            cx.background_executor().timer(Duration::from_millis(80)).await;
            let _ = view.update(cx, |this, cx| {
                this.er_column_debounce_tasks.remove(&debounce_tab);
                let Some(pending) = this.er_pending_columns.remove(&debounce_tab) else {
                    return;
                };
                flush_pending_columns(debounce_tab, connection_id, &database, &schema, this, cx, pending);
            });
        });
        this.er_column_debounce_tasks.insert(tab_id, Some(task));
    }
}

/// 批量读取待请求表字段并合并；字段高度变化只影响卡片自身，不重排坐标。
fn flush_pending_columns(
    tab_id: TabId,
    connection_id: ConnectionId,
    database: &str,
    schema: &Option<String>,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
    pending: BTreeSet<String>,
) {
    let Some(config) = this.controller.connection_configs().into_iter().find(|c| c.id == connection_id) else {
        return;
    };
    let database = database.to_string();
    let schema = schema.clone();
    let controller = this.controller.clone();
    let tables: Vec<String> = pending.into_iter().collect();
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move {
                controller.er_columns_for_tables(&config, &database, schema.as_deref(), &tables)
            })
            .await;
        view.update(cx, |this, cx| {
            if let Ok(batch) = result {
                let mut changed = false;
                if let Some(graph) = this.er_graphs.get_mut(&tab_id) {
                    for (name, cols, status) in batch.tables {
                        if let Some(node) = graph.tables.iter_mut().find(|t| t.name == name) {
                            let status_changed = node.status != status
                                || (status == ErLoadStatus::Loaded && node.columns != cols);
                            if status_changed {
                                node.columns = cols;
                                node.status = status;
                                changed = true;
                            }
                        }
                    }
                }
                if changed {
                    // 只重建拓扑（列内容/高度），坐标不动。
                    this.er_scenes.remove(&tab_id);
                    maybe_build_scene(tab_id, this);
                    tracing::debug!(tab = tab_id.0, "ER 字段批次已合并");
                }
            }
            this.er_column_debounce_tasks.insert(tab_id, None);
            cx.notify();
        })
        .ok();
    });
    this.er_column_debounce_tasks.insert(tab_id, Some(task));
}

/// 顶部工具栏：标题 + 计数 + 关系状态 + 深度 + 回到起点 / 按关系排列 / 重置手动位置。
fn er_toolbar(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let current_depth = this.er_depths.get(&tab_id).copied().unwrap_or(1);
    let table_count = this
        .er_graphs
        .get(&tab_id)
        .map(|g| g.tables.len())
        .unwrap_or(0);
    let relation_status = this
        .er_graphs
        .get(&tab_id)
        .map(|g| g.relation_status)
        .unwrap_or(ErLoadStatus::NotLoaded);
    let interacted = this.er_user_interacted.contains(&tab_id);
    let applied = this.er_layout_applied.contains(&tab_id);

    let mut base = div()
        .h(px(40.))
        .flex()
        .items_center()
        .px(px(12.))
        .gap(px(12.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg);

    let title = match &er.center_table {
        Some(table) => format!("{} · ER（{} · {} 跳）", er.database, table, current_depth),
        None => format!("{} · ER", er.database),
    };
    base = base.child(
        div()
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(colors.text)
            .child(title),
    );
    base = base.child(
        div()
            .text_size(px(12.))
            .text_color(colors.muted)
            .child(format!("{} 张表", table_count)),
    );
    if relation_status != ErLoadStatus::Loaded {
        let (label, tone) = match relation_status {
            ErLoadStatus::NotLoaded | ErLoadStatus::Loading => ("关系加载中…", colors.muted),
            ErLoadStatus::Failed => ("关系不完整", rgb(0xef4444)),
            _ => unreachable!(),
        };
        base = base.child(div().text_size(px(12.)).text_color(tone).child(label));
    } else if interacted && !applied {
        // 用户已操作且关系布局未应用：轻量提示 + 按钮可用（§6.2）。
        base = base.child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("关系已就绪，可按关系排列"),
        );
    }
    base = base.child(div().flex_1());

    if er.center_table.is_some() {
        base = base.child(er_depth_selector(tab_id, current_depth, colors, cx));
    }

    // 回到起点：回到布局初始视口，不清人工坐标/数据缓存（§6.3）。
    let back_tab = tab_id;
    base = base.child(
        Button::new(("er-back", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("回到起点")
            .child(app_icon(AppIcon::Home, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(vp) = this.er_viewports.get_mut(&back_tab) {
                    vp.pan_x = 0.0;
                    vp.pan_y = 0.0;
                }
                cx.notify();
            })),
    );
    // 按关系排列：保留手动固定节点；不重读数据库、不清缓存（§6.3）。
    let arrange_tab = tab_id;
    base = base.child(
        Button::new(("er-arrange", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("按关系排列（保留手动位置）")
            .child(app_icon(AppIcon::Wand, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.apply_er_layout(arrange_tab);
                if let Some(vp) = this.er_viewports.get_mut(&arrange_tab) {
                    vp.pan_x = 0.0;
                    vp.pan_y = 0.0;
                }
                cx.notify();
            })),
    );
    // 重置手动位置并排列：显式清除固定（§6.3）。
    let reset_tab = tab_id;
    base = base.child(
        Button::new(("er-reset-pins", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("重置手动位置并排列")
            .child(app_icon(AppIcon::Undo, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.er_pinned.remove(&reset_tab);
                this.apply_er_layout(reset_tab);
                if let Some(vp) = this.er_viewports.get_mut(&reset_tab) {
                    vp.pan_x = 0.0;
                    vp.pan_y = 0.0;
                }
                cx.notify();
            })),
    );

    base
}

/// 展开深度选择按钮组。
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
                            this.er_depths.insert(tab_id, depth);
                            this.er_graphs.remove(&tab_id);
                            this.er_scenes.remove(&tab_id);
                            this.er_relation_tasks.remove(&tab_id);
                            this.er_errors.remove(&tab_id);
                            this.er_load_tasks.remove(&tab_id);
                            this.er_scene_positions.remove(&tab_id);
                            this.er_layout_applied.remove(&tab_id);
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
        .child("正在读取表结构…")
}

/// 加载失败状态。
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
                    this.er_errors.remove(&retry_tab_id);
                    this.er_graphs.remove(&retry_tab_id);
                    this.er_full_tables.remove(&retry_tab_id);
                    this.er_scenes.remove(&retry_tab_id);
                    this.er_scene_positions.remove(&retry_tab_id);
                    this.er_layout_applied.remove(&retry_tab_id);
                    let er = retry_er.clone();
                    ensure_er_graph_loaded(retry_tab_id, &er, this, cx);
                    cx.notify();
                })),
        )
}
