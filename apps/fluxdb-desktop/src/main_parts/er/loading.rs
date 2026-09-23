// ER 目录/关系/字段分阶段读取、刷新及局部邻域编排。

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

/// 局部 ER：由整范围边集 + 表目录同步重算邻域并重建场景，保留人工坐标/固定/滚动。
///
/// 深度切换（1/2/3 跳）与单节点展开都走这里：只要关系边集已就绪（`er_all_edges`），
/// 就无需撕掉 `er_graphs` 重新读库，避免空画布窗口；`er_scene_positions`/`er_pinned`
/// 不删除 → 同表位置保留，新增邻居表补到布局位（§6.3）。
/// 返回是否已同步重算；未就绪（边集缺失）返回 false，调用方退回触发后台读取。
fn recompute_local_er_graph(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> bool {
    let Some(center) = &er.center_table else {
        return false;
    };
    this.er_center_refs.insert(tab_id, center.clone());
    recompute_local_er_from_center(tab_id, center, this, cx)
}

/// 以指定中心表同步重算局部 ER 邻域并重建场景（单节点展开 / 深度切换共用）。
/// 供 node.rs 的无 ErDiagramState 场景复用；返回是否已同步重算。
fn recompute_local_er_from_center(
    tab_id: TabId,
    center: &fluxdb_core::ErTableRef,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) -> bool {
    let (Some(full_tables), Some(all_edges)) = (
        this.er_full_tables.get(&tab_id).cloned(),
        this.er_all_edges.get(&tab_id).cloned(),
    ) else {
        return false;
    };
    let depth = this.er_depths.get(&tab_id).copied().unwrap_or(1);
    // 展开种子（展示名）→ 结构化身份。
    let extra_refs: BTreeSet<fluxdb_core::ErTableRef> = this
        .er_expanded
        .get(&tab_id)
        .into_iter()
        .flatten()
        .filter_map(|name| {
            full_tables
                .iter()
                .find(|t| &t.name == name)
                .map(|t| t.reference.clone())
        })
        .collect();
    // 邻域纳入「有效本地逻辑关系」：仅逻辑关系相连的邻居也能展开（§二.3）。
    // 只用于身份遍历；画布逻辑边由 sync_er_local_relationship_edges 投影。
    let mut include_edges = all_edges.clone();
    include_edges.extend(er_effective_logical_edges(this, tab_id, &full_tables));
    let included = er_neighborhood_included_tables(center, depth, &extra_refs, &include_edges);
    let tables: Vec<_> = full_tables
        .iter()
        .filter(|t| included.contains(&t.name))
        .cloned()
        .collect();
    // 物理边保留；逻辑边由 sync_er_local_relationship_edges 投影（字段解析后）。
    let edges: Vec<_> = all_edges
        .iter()
        .filter(|e| included.contains(&e.from_table) && included.contains(&e.to_table))
        .cloned()
        .collect();
    this.er_graphs.insert(
        tab_id,
        ErGraphData {
            tables,
            edges,
            relation_status: ErLoadStatus::Loaded,
        },
    );
    this.sync_er_local_relationship_edges(tab_id);
    // 改名但同对象的表（稳定标识命中）继承旧坐标，并剔除已不存在表的死坐标——
    // 否则表名变化后旧坐标键悬空、新表回退网格位导致位置漂移/重叠（§二.4 位置保真）。
    er_rebind_remap_scene_positions(tab_id, this);
    // 重建场景拓扑：坐标保留（er_scene_positions 未动），新表补布局位。
    this.er_scenes.remove(&tab_id);
    maybe_build_scene(tab_id, this);
    // 关系就绪后做一次增量自动落位：保留已持久化的坐标与固定，只给新增/相撞的表找空位。
    // 守卫在 er_auto_place_new_tables 内（er_auto_placed），与用户是否已交互无关，
    // 否则「关系返回前先平移过」会跳过落位，把存量重叠留在画布上（§6.3）。
    this.er_auto_place_new_tables(tab_id);
    cx.notify();
    true
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
    let generation = this.er_generation.get(&tab_id).copied().unwrap_or_default();
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move {
                controller.er_catalog_tables(&config, Some(&database), schema.as_deref())
            })
            .await;
        view.update(cx, |this, cx| {
            if this.er_generation.get(&tab_id).copied() != Some(generation)
                || !this.er_scope_keys.contains_key(&tab_id) {
                return;
            }
            match result {
                Ok(full_tables) => {
                    // 中心表按结构化身份匹配节点（不按展示名/`.split`，跨 schema 同名与含点安全）。
                    let display_tables: Vec<fluxdb_core::ErTableNode> = match &center_table {
                        Some(center) => full_tables
                            .iter()
                            .filter(|t| er_table_ref_eq(&t.reference, center))
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
                    this.er_last_updated.insert(
                        tab_id,
                        now_local_time_display(),
                    );
                    // 同一数据库对象改名时迁移画布状态，避免新名字被当作新节点重新布局。
                    if let Some(old) = this.er_graphs.get(&tab_id) {
                        let renamed: Vec<_> = old.tables.iter().filter_map(|prior| {
                            let stable = prior.stable?;
                            let current = full_tables.iter().find(|t| t.stable == Some(stable)
                                && t.reference.database == prior.reference.database
                                && t.reference.schema == prior.reference.schema)?;
                            (prior.name != current.name).then(|| (prior.name.clone(), current.name.clone()))
                        }).collect();
                        if !renamed.is_empty() {
                            let mut canvas = this.er_canvas.borrow_mut();
                            for (old_name, new_name) in renamed {
                                if let Some(pos) = canvas.er_scene_positions.get_mut(&tab_id) {
                                    if let Some(value) = pos.remove(&old_name) { pos.insert(new_name.clone(), value); }
                                }
                                if let Some(pinned) = canvas.er_pinned.get_mut(&tab_id) {
                                    if pinned.remove(&old_name) { pinned.insert(new_name.clone()); }
                                }
                                if let Some(scroll) = canvas.er_node_scroll_px.remove(&(tab_id, old_name.clone())) {
                                    canvas.er_node_scroll_px.insert((tab_id, new_name.clone()), scroll);
                                }
                            }
                        }
                    }
                    this.er_full_tables.insert(tab_id, full_tables);
                    this.er_graphs.insert(tab_id, graph);
                    // 快照仍指向旧身份时先搬坐标，再扫描并可能推进快照；
                    // 否则重启后改名表的旧位置会因快照已更新而无法识别。
                    er_rebind_remap_scene_positions(tab_id, this);
                    if this.er_relationships.contains_key(&tab_id) {
                        this.er_rebind_scan(tab_id, cx);
                        this.apply_er_rebind_auto(tab_id, cx);
                    }
                    this.sync_er_local_relationship_edges(tab_id);
                    this.er_scenes.remove(&tab_id);
                    // 坐标/固定/视口保留（首次加载时本就为空；手动刷新后保留人工布局，§3.3/§6.3）。
                    this.er_errors.remove(&tab_id);
                    // 表目录就绪但关系可能仍为 NotLoaded：此调用带 settled 守卫，未落定时不动作。
                    maybe_build_scene(tab_id, this);
                    this.er_fit_once_on_first_show(tab_id);
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
    // 首次或分组变化时重建（保留已有人工坐标）；普通 pan/滚动/字段滚动不触发。
    let group_now = this.er_group.get(&tab_id).cloned().unwrap_or(None);
    let built_group = this.er_group_built.get(&tab_id).cloned().unwrap_or(None);
    if this.er_scenes.contains_key(&tab_id) && built_group.as_deref() == group_now.as_deref() {
        return;
    }
    if !this.er_graphs.contains_key(&tab_id) {
        return;
    }
    this.er_rebuild_scene(tab_id);
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
    let generation = this.er_generation.get(&tab_id).copied().unwrap_or_default();
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move {
                controller.er_relations(&config, &database, schema.as_deref())
            })
            .await;
        view.update(cx, |this, cx| {
            if this.er_generation.get(&tab_id).copied() != Some(generation)
                || !this.er_scope_keys.contains_key(&tab_id) {
                return;
            }
            this.er_relation_tasks.remove(&tab_id);
            this.er_refreshing.remove(&tab_id);
            if let Ok(snap) = result {
                let status = snap.status;
                if status == ErLoadStatus::Loaded {
                    // 缓存整范围边集：局部 ER 深度切换/展开可同步重算邻域，不必重新读库。
                    this.er_all_edges.insert(tab_id, snap.edges.clone());
                    if let Some(center) = &center_table {
                        let full = this.er_full_tables.get(&tab_id).cloned().unwrap_or_default();
                        // 额外展开种子（展示名）解析为结构化身份，再按身份计算邻域。
                        let extra_refs: BTreeSet<fluxdb_core::ErTableRef> = extra
                            .iter()
                            .filter_map(|name| {
                                full.iter()
                                    .find(|t| t.name == *name)
                                    .map(|t| t.reference.clone())
                            })
                            .collect();
                        // 邻域计算纳入「有效本地逻辑关系」：仅逻辑关系相连的邻居（无物理外键）
                        // 也能随深度/展开进入图（§二.3）。逻辑边只参与身份遍历，不做字段解析。
                        let mut all_edges = snap.edges.clone();
                        all_edges.extend(er_effective_logical_edges(this, tab_id, &full));
                        let included = er_neighborhood_included_tables(
                            center,
                            depth,
                            &extra_refs,
                            &all_edges,
                        );
                        let tables: Vec<_> = full
                            .into_iter()
                            .filter(|t| included.contains(&t.name))
                            .collect();
                        // 物理边保留；逻辑边由 sync_er_local_relationship_edges 投影到画布。
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
                    // 逻辑关系画面边投影（若关系目录已加载）与拓扑重建。
                    this.sync_er_local_relationship_edges(tab_id);
                    // 关系就绪：自动增量落位一次（守卫见 er_auto_place_new_tables，§6.2/§6.3）。
                    // 新增表与相撞表让位于既有坐标/固定表，不再整体覆盖。
                    this.er_auto_place_new_tables(tab_id);
                    // 重建场景拓扑（坐标保留，新邻域表补布局位）。
                    this.er_scenes.remove(&tab_id);
                    maybe_build_scene(tab_id, this);
                    // 布局刚刚落定：此刻才能按最终范围做首次适配居中（早于此时是临时排列）。
                    this.er_fit_once_on_first_show(tab_id);
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
    let viewport = this.er_canvas.borrow().er_viewports.get(&tab_id).copied().unwrap_or_default();
    let (cw, ch) = this.er_canvas.borrow().er_canvas_sizes.get(&tab_id).copied().unwrap_or((960.0, 640.0));
    let positions = this.er_canvas.borrow().er_scene_positions.get(&tab_id).cloned().unwrap_or_default();
    let selected = this.er_canvas.borrow().er_selected_table.get(&tab_id).cloned().flatten();
    let pinned = this.er_canvas.borrow().er_pinned.get(&tab_id).cloned().unwrap_or_default();
    let env = build_env(&scene, tab_id, &positions, &this.er_canvas.borrow_mut().er_node_scroll_px, selected.as_deref(), &pinned);
    let frame = scene.materialize(&env, viewport, cw, ch);

    // 待请求表集以结构化身份记录（ErTableRef），缓存与结果归并据此进行，不从展示名反推
    // schema/表名（跨 schema 同名、含点标识符不串表）。
    let mut to_request: BTreeSet<fluxdb_core::ErTableRef> = BTreeSet::new();
    for idx in frame.visible_nodes {
        let meta = &scene.nodes[idx];
        let table = graph.tables.iter().find(|t| t.name == meta.name);
        if let Some(table) = table
            && er_column_status_needs_request(table.status)
        {
            to_request.insert(table.reference.clone());
        }
    }
    if this.er_relationship_form_open.contains(&tab_id) {
        for select in [
            this.er_relationship_form_left_tables.get(&tab_id),
            this.er_relationship_form_right_tables.get(&tab_id),
        ]
        .into_iter()
        .flatten()
        {
            let Some(entity_id) = select.read(cx).selected_value().cloned() else {
                continue;
            };
            if let Some(table) = this
                .er_full_tables
                .get(&tab_id)
                .into_iter()
                .flatten()
                .find(|table| er_entity_id(&table.reference) == entity_id)
            {
                if table.status != ErLoadStatus::Loaded {
                    to_request.insert(table.reference.clone());
                }
            }
        }
    }
    // 结构刷新不能只依赖视口内表：本地关系的屏外端点也要有界地补齐字段，
    // 否则其缺列/同名重建状态永远无法完成判断。每帧最多补 32 个，批次结束继续推进。
    if let Some(rels) = this.er_relationships.get(&tab_id)
        && let Some(full) = this.er_full_tables.get(&tab_id)
    {
        for table in full.iter().filter(|table| {
            rels.iter().any(|rel| er_entity_id(&table.reference) == rel.left_entity
                || er_entity_id(&table.reference) == rel.right_entity)
                && er_column_status_needs_request(table.status)
        }).take(32) {
            to_request.insert(table.reference.clone());
        }
    }
    if to_request.is_empty() {
        return;
    }
    let entry = this.er_pending_columns.entry(tab_id).or_default();
    entry.extend(to_request);
    schedule_pending_columns(tab_id, er.connection_id, this, cx);
}

/// `Loading` 可能来自另一个 ER tab 的共享缓存请求；该请求完成后不会直接回填当前 tab，
/// 因此与 `NotLoaded` 一样需要节流复查，不能永久停在骨架屏。
fn er_column_status_needs_request(status: ErLoadStatus) -> bool {
    matches!(status, ErLoadStatus::NotLoaded | ErLoadStatus::Loading)
}

fn should_schedule_er_column_flush(has_pending: bool, task_active: bool) -> bool {
    has_pending && !task_active
}

/// 每个 tab 同时只保留一个「去抖或读取」任务。读取期间出现的新可见表只进入 pending，
/// 当前批次完成后再串行排下一批，避免覆盖 Task 句柄导致旧批次结果无法回填 UI。
fn schedule_pending_columns(
    tab_id: TabId,
    connection_id: ConnectionId,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
) {
    let has_pending = this
        .er_pending_columns
        .get(&tab_id)
        .is_some_and(|pending| !pending.is_empty());
    let task_active = matches!(this.er_column_debounce_tasks.get(&tab_id), Some(Some(_)));
    if !should_schedule_er_column_flush(has_pending, task_active) {
        return;
    }
    let debounce_tab = tab_id;
    let task = cx.spawn(async move |view, cx| {
        cx.background_executor().timer(Duration::from_millis(80)).await;
        let _ = view.update(cx, |this, cx| {
            this.er_column_debounce_tasks.remove(&debounce_tab);
            let Some(pending) = this.er_pending_columns.remove(&debounce_tab) else {
                return;
            };
            flush_pending_columns(debounce_tab, connection_id, this, cx, pending);
        });
    });
    this.er_column_debounce_tasks.insert(tab_id, Some(task));
}

/// 批量读取待请求表字段并合并；字段高度变化只影响卡片自身，不重排坐标。
fn flush_pending_columns(
    tab_id: TabId,
    connection_id: ConnectionId,
    this: &mut NavicatMain,
    cx: &mut Context<NavicatMain>,
    pending: BTreeSet<fluxdb_core::ErTableRef>,
) {
    let Some(config) = this.controller.connection_configs().into_iter().find(|c| c.id == connection_id) else {
        return;
    };
    // 重试语义：对当前在图里状态为 Failed 的表，先作废 app 层 Failed 缓存再真正重新读取。
    // 成功/在飞表不受影响，避免每帧自动重试或清空成功结果。
    let failed_retry: Vec<fluxdb_core::ErTableRef> = {
        let graph = this.er_graphs.get(&tab_id);
        pending
            .iter()
            .filter(|reference| {
                graph
                    .and_then(|g| g.tables.iter().find(|t| &t.reference == *reference))
                    .map(|t| t.status == ErLoadStatus::Failed)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    };
    if !failed_retry.is_empty() {
        this.controller
            .er_columns_invalidate_failed(&config, &failed_retry);
    }
    let controller = this.controller.clone();
    let generation = this.er_generation.get(&tab_id).copied().unwrap_or_default();
    let tables: Vec<fluxdb_core::ErTableRef> = pending.into_iter().collect();
    let requested_tables = tables.clone();
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move { controller.er_columns_for_tables(&config, &tables) })
            .await;
        view.update(cx, |this, cx| {
            if this.er_generation.get(&tab_id).copied() != Some(generation)
                || !this.er_scope_keys.contains_key(&tab_id) {
                return;
            }
            let mut changed = false;
            match result {
                Ok(batch) => {
                // 同步字段到 er_full_tables：局部 ER 的 graph 只含邻居，关系表单选中的表
                // 可能不在 graph 内；字段下拉（er_form_table_by_id）从 full 读，需在此补齐，
                // 否则「选表后无法选择字段」（columns 恒空）。须在独占 batch.tables 之前做。
                if let Some(full) = this.er_full_tables.get_mut(&tab_id) {
                    for (name, cols, status) in batch.tables.iter() {
                        if let Some(node) = full.iter_mut().find(|t| t.name == *name)
                            && (node.status != *status
                                || (status == &ErLoadStatus::Loaded && node.columns != *cols))
                        {
                            node.columns = cols.clone();
                            node.status = *status;
                        }
                    }
                }
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
                }
                Err(error) => {
                    // connector/config 层错误不会产出逐表 Failed；显式落失败态，避免卡片永久 Loading。
                    for reference in &requested_tables {
                        if let Some(full) = this.er_full_tables.get_mut(&tab_id)
                            && let Some(node) = full.iter_mut().find(|t| t.reference == *reference)
                        {
                            node.status = ErLoadStatus::Failed;
                        }
                        if let Some(graph) = this.er_graphs.get_mut(&tab_id)
                            && let Some(node) = graph.tables.iter_mut().find(|t| t.reference == *reference)
                        {
                            node.status = ErLoadStatus::Failed;
                            changed = true;
                        }
                    }
                    tracing::warn!(tab = tab_id.0, error = %error, "ER 字段批次读取失败");
                }
            }

            if changed {
                // 重建拓扑（列内容/高度），坐标不动。若有本地逻辑关系，重投影逻辑边，
                // 用真实列名锚点（刷新后列刚加载，此前占位列名指向汇总端口）；并跑一次
                // 结构重绑扫描（字段已同步进 er_full_tables，产出 unresolved/needs_review）。
                if this.er_relationships.contains_key(&tab_id) {
                    this.sync_er_local_relationship_edges(tab_id);
                    this.er_rebind_scan(tab_id, cx);
                    // 可自动重绑的（改名但同对象）写回模型/存储，避免只显示横幅不改数据。
                    this.apply_er_rebind_auto(tab_id, cx);
                } else {
                    this.er_scenes.remove(&tab_id);
                    maybe_build_scene(tab_id, this);
                }
                tracing::debug!(tab = tab_id.0, "ER 字段批次已合并");
            }

            // 读取期间平移/缩放新增的需求一直留在 pending。剔除已经结束的表，并把命中
            // 共享缓存 Loading 的本批表重新排队；随后在当前任务结束后串行启动下一批。
            let mut queued = this.er_pending_columns.remove(&tab_id).unwrap_or_default();
            queued.extend(requested_tables.iter().filter_map(|reference| {
                er_column_reference_needs_request(this, tab_id, reference).then(|| reference.clone())
            }));
            queued.retain(|reference| er_column_reference_needs_request(this, tab_id, reference));
            if !queued.is_empty() {
                this.er_pending_columns.insert(tab_id, queued);
            }
            this.er_column_debounce_tasks.insert(tab_id, None);
            schedule_pending_columns(tab_id, connection_id, this, cx);
            cx.notify();
        })
        .ok();
    });
    this.er_column_debounce_tasks.insert(tab_id, Some(task));
}

fn er_column_reference_needs_request(
    this: &NavicatMain,
    tab_id: TabId,
    reference: &fluxdb_core::ErTableRef,
) -> bool {
    this.er_graphs
        .get(&tab_id)
        .and_then(|graph| graph.tables.iter().find(|table| table.reference == *reference))
        .or_else(|| {
            this.er_full_tables
                .get(&tab_id)
                .and_then(|tables| tables.iter().find(|table| table.reference == *reference))
        })
        .is_some_and(|table| er_column_status_needs_request(table.status))
}
