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

/// 关系面板可拖宽范围（px）。
const ER_REL_PANEL_MIN_W: f32 = 280.0;
const ER_REL_PANEL_MAX_W: f32 = 560.0;

impl NavicatMain {
    /// 将本地逻辑关系投影为画布边；物理外键仍来自 metadata，不执行 DDL。
    /// 关系目录变化后只替换 `logic:` 边，保留数据库外键和现有坐标。
    fn sync_er_local_relationship_edges(&mut self, tab_id: TabId) {
        let Some(graph_snapshot) = self.er_graphs.get(&tab_id).cloned() else {
            return;
        };
        let Some(relationships) = self.er_relationships.get(&tab_id).cloned() else {
            if let Some(graph) = self.er_graphs.get_mut(&tab_id) {
                graph.edges.retain(|edge| !edge.name.starts_with("logic:"));
            }
            self.er_scenes.remove(&tab_id);
            maybe_build_scene(tab_id, self);
            return;
        };
        let Some(tables) = self.er_full_tables.get(&tab_id).cloned() else {
            return;
        };
        let in_graph = |reference: &fluxdb_core::ErTableRef| {
            graph_snapshot
                .tables
                .iter()
                .any(|table| table.reference == *reference)
        };
        let mut local_edges = Vec::new();
        for relationship in relationships {
            // 只投影「有效」关系（已确认且 validity=current）：未确认/已拒绝/失效关系不冒充实约束
            // 或已确认关系（§二.3）。画布上的逻辑边与物理外键须可区分。
            if !er_relationship_effective(&relationship) {
                continue;
            }
            let Some(left) = tables
                .iter()
                .find(|table| er_entity_id(&table.reference) == relationship.left_entity)
            else {
                continue;
            };
            let Some(right) = tables
                .iter()
                .find(|table| er_entity_id(&table.reference) == relationship.right_entity)
            else {
                continue;
            };
            if !in_graph(&left.reference) || !in_graph(&right.reference) {
                continue;
            }
            for (index, pair) in relationship.column_pairs.iter().enumerate() {
                let Some(left_column) = left.columns.iter().find(|column| {
                    er_column_id(&left.reference, &column.name) == pair.left_column
                }) else {
                    continue;
                };
                let Some(right_column) = right.columns.iter().find(|column| {
                    er_column_id(&right.reference, &column.name) == pair.right_column
                }) else {
                    continue;
                };
                local_edges.push(fluxdb_core::ErForeignKeyEdge {
                    name: format!("logic:{}:{index}", relationship.id),
                    from_table: left.reference.display(),
                    from_column: left_column.name.clone(),
                    to_table: right.reference.display(),
                    to_column: right_column.name.clone(),
                    from_reference: left.reference.clone(),
                    to_reference: right.reference.clone(),
                });
            }
        }
        if let Some(graph) = self.er_graphs.get_mut(&tab_id) {
            graph.edges.retain(|edge| !edge.name.starts_with("logic:"));
            graph.edges.extend(local_edges);
        }
        self.er_scenes.remove(&tab_id);
        maybe_build_scene(tab_id, self);
    }

    /// 后台读取当前 ER 作用域的本地逻辑关系目录。结果回填时再次校验 scope，防止关闭/切换
    /// 标签后的旧请求覆盖新状态；UI 只消费 AppCommand/AppEvent，不直接访问文件存储。
    fn ensure_er_relationships_loaded(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(scope_key) = self.er_relationship_scope_keys.get(&tab_id).cloned() else {
            return;
        };
        if self.er_relationships.contains_key(&tab_id)
            || self.er_relationship_loading.contains(&tab_id)
        {
            return;
        }
        self.er_relationship_loading.insert(tab_id);
        let controller = self.controller.clone();
        let request_scope = scope_key.clone();
        let task = cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move {
                    let mut controller = controller;
                    controller.dispatch(AppCommand::LoadErRelationships {
                        scope_key: request_scope,
                    })
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.er_relationship_loading.remove(&tab_id);
                this.er_relationship_tasks.remove(&tab_id);
                if this.er_relationship_scope_keys.get(&tab_id) != Some(&scope_key) {
                    return;
                }
                match event {
                    AppEvent::ErRelationshipsLoaded { relationships, .. } => {
                        this.er_relationship_errors.remove(&tab_id);
                        this.er_relationships.insert(tab_id, relationships.clone());
                        // 关系目录就绪：投影逻辑边到画布；若为局部 ER 还按逻辑关系重算邻域，
                        // 把「仅逻辑关系相连」的邻居纳入图（§二.3）。
                        if let Some(center) = this.er_center_refs.get(&tab_id).cloned() {
                            recompute_local_er_from_center(tab_id, &center, this, cx);
                        } else {
                            this.sync_er_local_relationship_edges(tab_id);
                        }
                    }
                    AppEvent::Failed(error) => {
                        this.er_relationship_errors.insert(tab_id, error.message);
                    }
                    _ => {
                        this.er_relationship_errors
                            .insert(tab_id, "关系目录返回了未知结果".into());
                    }
                }
                cx.notify();
            });
        });
        self.er_relationship_tasks.insert(tab_id, task);
    }

    fn retry_er_relationships(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.er_relationships.remove(&tab_id);
        self.er_relationship_errors.remove(&tab_id);
        self.er_relationship_loading.remove(&tab_id);
        self.ensure_er_relationships_loaded(tab_id, cx);
        cx.notify();
    }

    fn run_er_relationship_command(
        &mut self,
        tab_id: TabId,
        command: AppCommand,
        cx: &mut Context<Self>,
    ) {
        let Some(scope_key) = self.er_relationship_scope_keys.get(&tab_id).cloned() else {
            return;
        };
        if self.er_relationship_tasks.contains_key(&tab_id) {
            return;
        }
        let closes_form = matches!(
            &command,
            AppCommand::CreateErRelationship { .. } | AppCommand::UpdateErRelationship { .. }
        );
        self.er_relationship_loading.insert(tab_id);
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move {
                    let mut controller = controller;
                    controller.dispatch(command)
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.er_relationship_loading.remove(&tab_id);
                this.er_relationship_tasks.remove(&tab_id);
                if this.er_relationship_scope_keys.get(&tab_id) != Some(&scope_key) {
                    return;
                }
                match &event {
                    AppEvent::ErRelationshipChanged { .. }
                    | AppEvent::ErRelationshipDeleted { .. } => {
                        this.er_relationship_errors.remove(&tab_id);
                        if matches!(event, AppEvent::ErRelationshipDeleted { .. }) {
                            this.er_relationship_delete_pending.remove(&tab_id);
                        }
                        if closes_form {
                            this.er_relationship_form_open.remove(&tab_id);
                        }
                        this.apply_app_event(&event, cx);
                    }
                    AppEvent::Failed(error) => {
                        this.er_relationship_errors.insert(tab_id, error.message.clone());
                    }
                    _ => {
                        this.er_relationship_errors
                            .insert(tab_id, "关系操作返回了未知结果".into());
                    }
                }
                cx.notify();
            });
        });
        self.er_relationship_tasks.insert(tab_id, task);
    }

    /// 用户已操作画布（平移/拖动/选择/字段滚动）：关系就绪后不再自动重排（§6.2）。
    fn mark_er_interacted(&mut self, tab: TabId) {
        self.er_user_interacted.insert(tab);
    }

    /// 结束节点拖动（画布外释放/窗口失焦/切标签同样调用，§7）。
    fn finish_node_drag(&mut self, tab: TabId) {
        if self.er_node_drag.as_ref().is_some_and(|(t, ..)| *t == tab) {
            self.er_node_drag = None;
            // 拖动结束持久化坐标/固定（§十）。
            self.er_save_view_state(tab);
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

    /// 适配当前范围：把图中节点世界范围整体放进画布可见区（留 32px 边距，§六.23）。
    /// 只改 viewport（pan/scale），不动节点坐标/固定/滚动。空图或无节点则回到起点。
    fn er_fit_scope(&mut self, tab: TabId) {
        let Some(scene) = self.er_scenes.get(&tab) else {
            return;
        };
        let positions = self.er_scene_positions.get(&tab).cloned().unwrap_or_default();
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut any = false;
        for m in &scene.nodes {
            let Some(&(x, y)) = positions.get(&m.name) else { continue };
            let h = card_height(m.status, m.columns.len());
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + NODE_WIDTH);
            max_y = max_y.max(y + h);
            any = true;
        }
        if !any {
            if let Some(vp) = self.er_viewports.get_mut(&tab) {
                vp.pan_x = 0.0;
                vp.pan_y = 0.0;
                vp.scale = 1.0;
            }
            return;
        }
        let (cw, ch) = self.er_canvas_sizes.get(&tab).copied().unwrap_or((960.0, 640.0));
        // 需拿到场景高度：由卡片最低 y + 高度得出（同 apply 布局坐标来源）。
        let world_w = (max_x - min_x).max(1.0);
        let world_h = (max_y - min_y).max(1.0);
        let margin = 32.0 * 2.0;
        let scale = ((cw - margin) / world_w).min((ch - margin) / world_h).min(1.0).max(ER_MIN_SCALE);
        let vp = self.er_viewports.entry(tab).or_default();
        vp.scale = scale;
        // 居中对齐：把 (min_x,min_y) 平移到左上一角。
        vp.pan_x = (cw - world_w * scale) / 2.0 - min_x * scale;
        vp.pan_y = (ch - world_h * scale) / 2.0 - min_y * scale;
        self.mark_er_interacted(tab);
    }

    /// 以画布中心为锚缩放（工具栏 +/- 按钮，§六.23）。
    fn zoom_er_around_center(&mut self, tab: TabId, factor: f32) {
        let (cw, ch) = self.er_canvas_sizes.get(&tab).copied().unwrap_or((960.0, 640.0));
        let vp = self.er_viewports.entry(tab).or_default();
        vp.zoom_around((cw / 2.0, ch / 2.0), factor);
        self.mark_er_interacted(tab);
    }

    /// 将某表居中到画布中心并选中（§五.6 搜索定位）：保持当前缩放，仅平移视口。
    /// 返回是否定位到该表（表不在场景/无坐标时返回 false，无法定位不假装成功）。
    fn er_center_on_table(&mut self, tab: TabId, table_name: &str) -> bool {
        let Some(scene) = self.er_scenes.get(&tab).cloned() else {
            return false;
        };
        let Some(&(x, y)) = self
            .er_scene_positions
            .get(&tab)
            .and_then(|m| m.get(table_name))
        else {
            return false;
        };
        let (cw, ch) = self.er_canvas_sizes.get(&tab).copied().unwrap_or((960.0, 640.0));
        let vp = self.er_viewports.entry(tab).or_default();
        let sc = vp.safe_scale();
        // 卡片高度按该表实际内容（未加载用占位高度），卡中心世界坐标 → 画布中心：
        // pan = 中心 - 卡中心*scale。
        let h = scene
            .nodes
            .iter()
            .find(|n| n.name == table_name)
            .map(|n| card_height(n.status, n.columns.len()))
            .unwrap_or(NODE_HEADER + ACCENT_BAR + 3.0 * NODE_FIELD_ROW);
        let card_center_wx = x + NODE_WIDTH / 2.0;
        let card_center_wy = y + h / 2.0;
        vp.pan_x = cw / 2.0 - card_center_wx * sc;
        vp.pan_y = ch / 2.0 - card_center_wy * sc;
        self.er_selected_table.insert(tab, Some(table_name.to_string()));
        self.er_field_highlights.remove(&tab);
        self.mark_er_interacted(tab);
        true
    }

    /// 当前分组下的展示图：None=全部；Some(schema)=仅该 schema 的表及其内部边。
    /// 不复制关系目录（§五.7）；只按结构化 schema 过滤视图内容。
    fn er_display_group_graph(&self, graph: &ErGraphData, group: &Option<String>) -> ErGraphData {
        er_group_subset_graph(graph, group.as_deref())
    }

    /// 重建场景：按当前分组过滤展示图，重算布局，保留已有人工坐标（同表坐标不丢，
    /// 新增表取布局位），替换 er_scenes。普通 pan/滚动不触发，仅首次或分组变化时。
    fn er_rebuild_scene(&mut self, tab: TabId) {
        let Some(graph) = self.er_graphs.get(&tab) else {
            return;
        };
        let group = self.er_group.get(&tab).cloned().unwrap_or(None);
        let display = self.er_display_group_graph(graph, &group);
        let layout = er_relation_layout(&display.tables, &display.edges);
        // 保留已有坐标；为组内表补布局位。
        let first_build = !self.er_scene_positions.contains_key(&tab);
        if first_build {
            let mut positions = BTreeMap::new();
            for t in &display.tables {
                if let Some(&(x, y, _)) = layout.get(&t.name) {
                    positions.insert(t.name.clone(), (x, y));
                }
            }
            self.er_scene_positions.insert(tab, positions);
        } else if let Some(pos) = self.er_scene_positions.get_mut(&tab) {
            for t in &display.tables {
                pos.entry(t.name.clone())
                    .or_insert_with(|| layout.get(&t.name).map(|&(x, y, _)| (x, y)).unwrap_or((0.0, 0.0)));
            }
        }
        let scene = build_er_scene(&display, &layout);
        self.er_scenes.insert(tab, Rc::new(scene));
        self.er_group_built.insert(tab, group);
    }

    /// 设置分组（None=全部；Some(schema)=进入该 schema 组）：重建场景，标记已交互。
    fn er_set_group(&mut self, tab: TabId, group: Option<String>) {
        self.er_group.insert(tab, group);
        self.er_rebuild_scene(tab);
        self.mark_er_interacted(tab);
        // 折叠态是视图状态：变化即持久化（重启可恢复，§十）。
        self.er_save_view_state(tab);
    }

    /// 把当前 tab 的分组/固定/坐标写入持久化（按 scope key）。
    fn er_save_view_state(&mut self, tab: TabId) {
        let Some(key) = self.er_scope_keys.get(&tab).cloned() else {
            return;
        };
        let group = self.er_group.get(&tab).cloned().unwrap_or(None);
        let pinned: Vec<String> = self
            .er_pinned
            .get(&tab)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let positions: Vec<(String, f32, f32)> = self
            .er_scene_positions
            .get(&tab)
            .map(|m| m.iter().map(|(n, (x, y))| (n.clone(), *x, *y)).collect())
            .unwrap_or_default();
        let mut states = self.storage.load_er_view_states().unwrap_or_default();
        states.insert(
            key,
            fluxdb_storage::ErViewScopeState {
                group,
                pinned,
                positions,
            },
        );
        let _ = self.storage.save_er_view_states(&states);
    }

    /// 首次打开某 ER tab：应用该作用域上次保存的分组/坐标/固定（一次）。
    /// 坐标/固定仅按展示名应用；表不匹配（改名/换库）时多余项被忽略、缺失表按布局回退。
    fn er_restore_view_state(&mut self, tab: TabId) {
        if self.er_view_restored.contains(&tab) {
            return;
        }
        self.er_view_restored.insert(tab);
        let Some(key) = self.er_scope_keys.get(&tab).cloned() else {
            return;
        };
        let states = self.storage.load_er_view_states().unwrap_or_default();
        let Some(s) = states.get(&key) else {
            return;
        };
        if let Some(group) = &s.group {
            self.er_group.insert(tab, Some(group.clone()));
        }
        if !s.positions.is_empty() && !self.er_scene_positions.contains_key(&tab) {
            let pos: BTreeMap<String, (f32, f32)> = s
                .positions
                .iter()
                .map(|(n, x, y)| (n.clone(), (*x, *y)))
                .collect();
            self.er_scene_positions.insert(tab, pos);
        }
        if !s.pinned.is_empty() {
            self.er_pinned.insert(tab, s.pinned.iter().cloned().collect());
        }
    }

    /// 图中各 schema 分组的表数（供分组选择条展示；按 schema 聚合，含 None）。
    fn er_schema_groups(&self, tab: TabId) -> Vec<(Option<String>, usize)> {
        let Some(graph) = self.er_graphs.get(&tab) else {
            return Vec::new();
        };
        let mut counts: std::collections::BTreeMap<Option<String>, usize> =
            std::collections::BTreeMap::new();
        for t in &graph.tables {
            *counts.entry(t.reference.schema.clone()).or_insert(0) += 1;
        }
        counts.into_iter().collect()
    }

    /// 当前场景所有已定位节点（含高度）的世界包围盒；无定位节点返回 None。
    fn er_world_bbox(&self, tab: TabId) -> Option<(f32, f32, f32, f32)> {
        let scene = self.er_scenes.get(&tab)?;
        let positions = self.er_scene_positions.get(&tab)?;
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut any = false;
        for m in &scene.nodes {
            if !positions.contains_key(&m.name) {
                continue;
            }
            let &(x, y) = positions.get(&m.name).unwrap();
            let h = card_height(m.status, m.columns.len());
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + NODE_WIDTH);
            max_y = max_y.max(y + h);
            any = true;
        }
        if !any {
            return None;
        }
        Some((min_x, min_y, max_x, max_y))
    }

    /// 表搜索命中列表（§五.6）：按当前查询过滤场景节点（名称/注释包含，忽略大小写，稳定排序）。
    fn er_search_matches(&self, tab: TabId) -> Vec<String> {
        let query = self.er_search_query.get(&tab).cloned().unwrap_or_default();
        let normalized = query.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(scene) = self.er_scenes.get(&tab) {
            for n in &scene.nodes {
                if er_search_text_matches(n.name.as_str(), n.comment.as_deref(), &normalized) {
                    out.push(n.name.clone());
                }
            }
        }
        out.sort();
        out
    }

    /// 把小地图中 `(nx,ny)`（归一化 0..1，按世界 bbox）对应的世界点放到画布中心。
    /// 世界不在 bbox 内时（单点场景）返回 false，不假装定位（§六.25 导航开销有界）。
    fn er_minimap_center_world(&mut self, tab: TabId, nx: f32, ny: f32) -> bool {
        let Some((min_x, min_y, max_x, max_y)) = self.er_world_bbox(tab) else {
            return false;
        };
        let (cw, ch) = self.er_canvas_sizes.get(&tab).copied().unwrap_or((960.0, 640.0));
        let wx = min_x + nx.clamp(0.0, 1.0) * (max_x - min_x).max(1.0);
        let wy = min_y + ny.clamp(0.0, 1.0) * (max_y - min_y).max(1.0);
        let vp = self.er_viewports.entry(tab).or_default();
        let sc = vp.safe_scale();
        vp.pan_x = cw / 2.0 - wx * sc;
        vp.pan_y = ch / 2.0 - wy * sc;
        self.mark_er_interacted(tab);
        true
    }
}

fn er_entity_id(reference: &fluxdb_core::ErTableRef) -> String {
    format!(
        "{}:{}:{}",
        reference.database,
        reference.schema.as_deref().unwrap_or_default(),
        reference.name
    )
}

fn er_column_id(reference: &fluxdb_core::ErTableRef, column: &str) -> String {
    format!("{}::{column}", er_entity_id(reference))
}

/// 判断一条本地逻辑关系是否为「有效」关系（§二.3）：只有已确认且结构有效（current）的
/// 关系才进入邻域展开与画布连线。Proposed/Rejected/Stale/Unresolved/Invalid 一律不伪装成
/// 物理外键或已确认关系（默认不混入未确认候选、失效关系不冒充实约束）。
fn er_relationship_effective(rel: &fluxdb_core::ErRelationship) -> bool {
    rel.review.state == fluxdb_core::ErReviewState::Confirmed
        && rel.validity.state == fluxdb_core::ErValidityState::Current
}

/// 把「有效」本地逻辑关系投影为结构化外键边，供局部邻域计算使用（只读 from/to 身份，
/// 不要求字段已加载）。有效关系 = 已确认且 validity=Current（§二.3）。Neighborhood 遍历
/// 只依据身份，故这里每条有效关系产出一条代表边，用于把「仅逻辑关系相连的邻居」纳入展开。
fn er_effective_logical_edges(
    this: &NavicatMain,
    tab_id: TabId,
    full_tables: &[fluxdb_core::ErTableNode],
) -> Vec<fluxdb_core::ErForeignKeyEdge> {
    let Some(rels) = this.er_relationships.get(&tab_id).cloned() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for rel in rels {
        if !er_relationship_effective(&rel) {
            continue;
        }
        let Some(left) = full_tables
            .iter()
            .find(|t| er_entity_id(&t.reference) == rel.left_entity)
        else {
            continue;
        };
        let Some(right) = full_tables
            .iter()
            .find(|t| er_entity_id(&t.reference) == rel.right_entity)
        else {
            continue;
        };
        let name = format!("logic:{}", rel.id);
        out.push(fluxdb_core::ErForeignKeyEdge {
            name,
            from_table: left.reference.display(),
            from_column: String::new(),
            to_table: right.reference.display(),
            to_column: String::new(),
            from_reference: left.reference.clone(),
            to_reference: right.reference.clone(),
        });
    }
    out
}

fn er_table_options(
    tables: &[fluxdb_core::ErTableNode],
) -> Vec<ErRelationshipSelectOption> {
    tables
        .iter()
        .map(|table| ErRelationshipSelectOption {
            id: er_entity_id(&table.reference),
            label: table.reference.display(),
        })
        .collect()
}

fn er_column_options(
    table: Option<&fluxdb_core::ErTableNode>,
) -> Vec<ErRelationshipSelectOption> {
    table
        .into_iter()
        .flat_map(|table| {
            table.columns.iter().map(|column| ErRelationshipSelectOption {
                id: er_column_id(&table.reference, &column.name),
                label: column.name.clone(),
            })
        })
        .collect()
}

fn er_filter_op_id(op: &fluxdb_core::ErFilterOp) -> &'static str {
    match op {
        fluxdb_core::ErFilterOp::Eq => "eq",
        fluxdb_core::ErFilterOp::Ne => "ne",
        fluxdb_core::ErFilterOp::IsNull => "is_null",
        fluxdb_core::ErFilterOp::IsNotNull => "is_not_null",
        fluxdb_core::ErFilterOp::In => "in",
    }
}

fn er_literal_text(literal: &fluxdb_core::ErLiteral) -> String {
    match literal {
        fluxdb_core::ErLiteral::Text(value) => value.clone(),
        fluxdb_core::ErLiteral::Int(value) => value.to_string(),
        fluxdb_core::ErLiteral::Float(value) => value.to_string(),
        fluxdb_core::ErLiteral::Bool(value) => value.to_string(),
        fluxdb_core::ErLiteral::Null => String::new(),
    }
}

/// 基数选择选项（1:1 / 1:N / N:1 / N:N / 未知）。
fn er_cardinality_options() -> Vec<ErRelationshipSelectOption> {
    vec![
        ErRelationshipSelectOption { id: "1_1".into(), label: "1 对 1".into() },
        ErRelationshipSelectOption { id: "1_n".into(), label: "1 对多（1:N）".into() },
        ErRelationshipSelectOption { id: "n_1".into(), label: "多对 1（N:1）".into() },
        ErRelationshipSelectOption { id: "n_n".into(), label: "多对多（N:N）".into() },
        ErRelationshipSelectOption { id: "unknown".into(), label: "未知（不声明）".into() },
    ]
}

/// 由匹配基数推导选择框 id（未知任一向 → unknown；其余按 max 映射）。
fn er_cardinality_to_option_id(
    card: &fluxdb_core::ErMatchCardinality,
) -> &'static str {
    use fluxdb_core::ErCardinalityBound::{Many, One, Unknown, Zero};
    // max 为 One 或 Zero（0..1）都按「单条」归到 One；Unknown 视为未声明。
    let (l2r, r2l) = (card.left_to_right.max, card.right_to_left.max);
    match (l2r, r2l) {
        (One, One) | (Zero, One) | (One, Zero) | (Zero, Zero) => "1_1",
        (Many, One) | (Many, Zero) => "1_n",
        (One, Many) | (Zero, Many) => "n_1",
        (Many, Many) => "n_n",
        (Unknown, _) | (_, Unknown) => "unknown",
    }
}

/// 由选择框 id 生成匹配基数（新增/编辑表单；basis=UserAssertion，未知不声明）。
fn er_option_id_to_cardinality(
    id: &str,
) -> fluxdb_core::ErMatchCardinality {
    use fluxdb_core::ErCardinalityBound::{Many, One, Unknown};
    let (l2r, r2l) = match id {
        "1_1" => (One, One),
        "1_n" => (Many, One),
        "n_1" => (One, Many),
        "n_n" => (Many, Many),
        _ => (Unknown, Unknown),
    };
    fluxdb_core::ErMatchCardinality {
        left_to_right: fluxdb_core::ErCardinality { min: Unknown, max: l2r },
        right_to_left: fluxdb_core::ErCardinality { min: Unknown, max: r2l },
        basis: if id == "unknown" {
            fluxdb_core::ErCardinalityBasis::Unknown
        } else {
            fluxdb_core::ErCardinalityBasis::UserAssertion
        },
    }
}

fn er_parse_literal(text: &str) -> Option<fluxdb_core::ErLiteral> {
    if text.is_empty() {
        return None;
    }
    if text.eq_ignore_ascii_case("true") {
        return Some(fluxdb_core::ErLiteral::Bool(true));
    }
    if text.eq_ignore_ascii_case("false") {
        return Some(fluxdb_core::ErLiteral::Bool(false));
    }
    if let Ok(value) = text.parse::<i64>() {
        return Some(fluxdb_core::ErLiteral::Int(value));
    }
    if let Ok(value) = text.parse::<f64>() {
        return Some(fluxdb_core::ErLiteral::Float(value));
    }
    Some(fluxdb_core::ErLiteral::Text(text.to_string()))
}

impl NavicatMain {
    fn er_form_table_by_id(
        &self,
        tab_id: TabId,
        entity_id: Option<&String>,
    ) -> Option<fluxdb_core::ErTableNode> {
        let entity_id = entity_id?;
        self.er_full_tables
            .get(&tab_id)?
            .iter()
            .find(|table| er_entity_id(&table.reference) == *entity_id)
            .cloned()
    }

    fn er_refresh_select_options(
        select: &Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>,
        options: Vec<ErRelationshipSelectOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = select.read(cx).selected_value().cloned();
        let selected_is_valid = selected
            .as_ref()
            .is_some_and(|id| options.iter().any(|option| &option.id == id));
        select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(options), window, cx);
            if selected_is_valid {
                if let Some(selected) = &selected {
                    state.set_selected_value(selected, window, cx);
                }
            } else {
                state.set_selected_index(None, window, cx);
            }
        });
    }

    fn ensure_er_relationship_form_controls(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tables = self.er_full_tables.get(&tab_id).cloned().unwrap_or_default();
        let table_options = er_table_options(&tables);
        if !self.er_relationship_form_role_inputs.contains_key(&tab_id) {
            self.er_relationship_form_role_inputs.insert(
                tab_id,
                cx.new(|cx| InputState::new(window, cx).placeholder("业务角色")),
            );
        }
        if !self
            .er_relationship_form_description_inputs
            .contains_key(&tab_id)
        {
            self.er_relationship_form_description_inputs.insert(
                tab_id,
                cx.new(|cx| InputState::new(window, cx).placeholder("说明（可选）")),
            );
        }
        if let Some(select) = self.er_relationship_form_left_tables.get(&tab_id) {
            Self::er_refresh_select_options(select, table_options.clone(), window, cx);
        } else {
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(table_options.clone()), None, window, cx)
                    .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                move |this: &mut NavicatMain,
                      _select,
                      event: &SelectEvent<SearchableVec<ErRelationshipSelectOption>>,
                      window,
                      cx| {
                    if matches!(event, SelectEvent::Confirm(_)) {
                        this.er_clear_relationship_pair_selections(tab_id, window, cx);
                        this.er_clear_relationship_filter_columns(tab_id, window, cx);
                        cx.notify();
                    }
                },
            );
            self.er_relationship_form_left_tables.insert(tab_id, select);
            self.er_relationship_form_subscriptions
                .entry(tab_id)
                .or_default()
                .push(subscription);
        }
        if let Some(select) = self.er_relationship_form_right_tables.get(&tab_id) {
            Self::er_refresh_select_options(select, table_options, window, cx);
        } else {
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(table_options), None, window, cx)
                    .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                move |this: &mut NavicatMain,
                      _select,
                      event: &SelectEvent<SearchableVec<ErRelationshipSelectOption>>,
                      window,
                      cx| {
                    if matches!(event, SelectEvent::Confirm(_)) {
                        this.er_clear_relationship_pair_selections(tab_id, window, cx);
                        this.er_clear_relationship_filter_columns(tab_id, window, cx);
                        cx.notify();
                    }
                },
            );
            self.er_relationship_form_right_tables.insert(tab_id, select);
            self.er_relationship_form_subscriptions
                .entry(tab_id)
                .or_default()
                .push(subscription);
        }
        if !self.er_relationship_form_pairs.contains_key(&tab_id) {
            self.er_relationship_form_pairs.insert(tab_id, Vec::new());
        }
        if self
            .er_relationship_form_pairs
            .get(&tab_id)
            .is_some_and(Vec::is_empty)
        {
            self.er_add_relationship_pair(tab_id, window, cx);
        }

        let left_id = self
            .er_relationship_form_left_tables
            .get(&tab_id)
            .and_then(|select| select.read(cx).selected_value().cloned());
        let right_id = self
            .er_relationship_form_right_tables
            .get(&tab_id)
            .and_then(|select| select.read(cx).selected_value().cloned());
        let left_table = self.er_form_table_by_id(tab_id, left_id.as_ref());
        let right_table = self.er_form_table_by_id(tab_id, right_id.as_ref());
        let pair_options = self
            .er_relationship_form_pairs
            .get_mut(&tab_id)
            .expect("ER relationship form pairs initialized");
        for pair in pair_options {
            let left_options = er_column_options(left_table.as_ref());
            let right_options = er_column_options(right_table.as_ref());
            let left_key = left_options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            let right_key = right_options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            if pair.left_options_key != left_key {
                Self::er_refresh_select_options(&pair.left, left_options, window, cx);
                pair.left_options_key = left_key;
            }
            if pair.right_options_key != right_key {
                Self::er_refresh_select_options(&pair.right, right_options, window, cx);
                pair.right_options_key = right_key;
            }
        }
        let filter_controls = self.er_relationship_form_filters.entry(tab_id).or_default();
        for filter in filter_controls {
            let side = filter
                .side
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_else(|| "left".into());
            let table = if side == "right" {
                right_table.as_ref()
            } else {
                left_table.as_ref()
            };
            let options = er_column_options(table);
            let key = options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            if filter.column_options_key != key {
                Self::er_refresh_select_options(&filter.column, options, window, cx);
                filter.column_options_key = key;
            }
        }
        // 基数选择：缺失时创建，默认「未知」。
        if let Some(select) = self.er_relationship_form_cardinality.get(&tab_id) {
            Self::er_refresh_select_options(select, er_cardinality_options(), window, cx);
        } else {
            let select = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(er_cardinality_options()),
                    Some(IndexPath::default().row(4)), // 默认选中「未知」
                    window,
                    cx,
                )
                .searchable(false)
            });
            self.er_relationship_form_cardinality.insert(tab_id, select);
        }
    }

    fn er_add_relationship_pair(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let left = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::<ErRelationshipSelectOption>::new()), None, window, cx)
                .searchable(true)
        });
        let right = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::<ErRelationshipSelectOption>::new()), None, window, cx)
                .searchable(true)
        });
        self.er_relationship_form_pairs
            .entry(tab_id)
            .or_default()
            .push(ErRelationshipPairControls {
                left,
                right,
                left_options_key: String::new(),
                right_options_key: String::new(),
            });
    }

    fn er_add_relationship_filter(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let side = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ErRelationshipSelectOption {
                        id: "left".into(),
                        label: "左表".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "right".into(),
                        label: "右表".into(),
                    },
                ]),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
            .searchable(false)
        });
        let column = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<ErRelationshipSelectOption>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let op = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    ErRelationshipSelectOption {
                        id: "eq".into(),
                        label: "等于".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "ne".into(),
                        label: "不等于".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "is_null".into(),
                        label: "为空".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "is_not_null".into(),
                        label: "不为空".into(),
                    },
                    ErRelationshipSelectOption {
                        id: "in".into(),
                        label: "包含".into(),
                    },
                ]),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
            .searchable(false)
        });
        let literal = cx.new(|cx| InputState::new(window, cx).placeholder("常量值"));
        let subscription = cx.subscribe_in(
            &side,
            window,
            move |_this: &mut NavicatMain,
                  _select,
                  _event: &SelectEvent<SearchableVec<ErRelationshipSelectOption>>,
                  _window,
                  cx| {
                cx.notify();
            },
        );
        self.er_relationship_form_subscriptions
            .entry(tab_id)
            .or_default()
            .push(subscription);
        self.er_relationship_form_filters
            .entry(tab_id)
            .or_default()
            .push(ErRelationshipFilterControls {
                side,
                column,
                op,
                literal,
                column_options_key: String::new(),
            });
    }

    fn er_clear_relationship_filter_columns(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(filters) = self.er_relationship_form_filters.get(&tab_id) {
            for filter in filters {
                filter.column.update(cx, |state, cx| {
                    state.set_selected_index(None, window, cx);
                });
            }
        }
    }

    fn er_clear_relationship_pair_selections(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(pairs) = self.er_relationship_form_pairs.get(&tab_id) {
            for pair in pairs {
                pair.left.update(cx, |state, cx| {
                    state.set_selected_index(None, window, cx);
                });
                pair.right.update(cx, |state, cx| {
                    state.set_selected_index(None, window, cx);
                });
            }
        }
    }

    fn er_prepare_new_relationship_form(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        self.er_relationship_form_editing.insert(tab_id, None);
        if let Some(input) = self.er_relationship_form_role_inputs.get(&tab_id) {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        if let Some(input) = self.er_relationship_form_description_inputs.get(&tab_id) {
            input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        }
        for select in [
            self.er_relationship_form_left_tables.get(&tab_id),
            self.er_relationship_form_right_tables.get(&tab_id),
        ]
        .into_iter()
        .flatten()
        {
            select.update(cx, |state, cx| state.set_selected_index(None, window, cx));
        }
        self.er_relationship_form_pairs.insert(tab_id, Vec::new());
        self.er_add_relationship_pair(tab_id, window, cx);
        self.er_relationship_form_filters.insert(tab_id, Vec::new());
        // 新建时基数复位为「未知」。
        if let Some(select) = self.er_relationship_form_cardinality.get(&tab_id) {
            select.update(cx, |state, cx| {
                state.set_selected_value(&"unknown".to_string(), window, cx)
            });
        }
    }

    fn er_begin_edit_relationship(
        &mut self,
        tab_id: TabId,
        relationship: &fluxdb_core::ErRelationship,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        self.er_relationship_form_open.insert(tab_id);
        self.er_relationship_form_editing
            .insert(tab_id, Some(relationship.id.clone()));
        if let Some(input) = self.er_relationship_form_role_inputs.get(&tab_id) {
            input.update(cx, |input, cx| {
                input.set_value(relationship.role.clone(), window, cx)
            });
        }
        if let Some(input) = self.er_relationship_form_description_inputs.get(&tab_id) {
            input.update(cx, |input, cx| {
                input.set_value(relationship.description.clone().unwrap_or_default(), window, cx)
            });
        }
        if let Some(select) = self.er_relationship_form_left_tables.get(&tab_id) {
            select.update(cx, |state, cx| {
                state.set_selected_value(&relationship.left_entity, window, cx)
            });
        }
        if let Some(select) = self.er_relationship_form_right_tables.get(&tab_id) {
            select.update(cx, |state, cx| {
                state.set_selected_value(&relationship.right_entity, window, cx)
            });
        }
        self.er_relationship_form_pairs.insert(tab_id, Vec::new());
        for _ in &relationship.column_pairs {
            self.er_add_relationship_pair(tab_id, window, cx);
        }
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        if let Some(pairs) = self.er_relationship_form_pairs.get(&tab_id) {
            for (controls, pair) in pairs.iter().zip(&relationship.column_pairs) {
                controls.left.update(cx, |state, cx| {
                    state.set_selected_value(&pair.left_column, window, cx)
                });
                controls.right.update(cx, |state, cx| {
                    state.set_selected_value(&pair.right_column, window, cx)
                });
            }
        }
        self.er_relationship_form_filters.insert(tab_id, Vec::new());
        for _ in &relationship.required_filters {
            self.er_add_relationship_filter(tab_id, window, cx);
        }
        if let Some(filters) = self.er_relationship_form_filters.get(&tab_id) {
            for (controls, filter) in filters.iter().zip(&relationship.required_filters) {
                let side = match filter.side {
                    fluxdb_core::ErRelationSide::Left => "left",
                    fluxdb_core::ErRelationSide::Right => "right",
                };
                controls.side.update(cx, |state, cx| {
                    state.set_selected_value(&side.to_string(), window, cx)
                });
                let op = er_filter_op_id(&filter.op).to_string();
                controls.op.update(cx, |state, cx| {
                    state.set_selected_value(&op, window, cx)
                });
                controls.literal.update(cx, |input, cx| {
                    input.set_value(er_literal_text(&filter.literal), window, cx)
                });
            }
        }
        self.ensure_er_relationship_form_controls(tab_id, window, cx);
        if let Some(filters) = self.er_relationship_form_filters.get(&tab_id) {
            for (controls, filter) in filters.iter().zip(&relationship.required_filters) {
                controls.column.update(cx, |state, cx| {
                    state.set_selected_value(&filter.column_id, window, cx)
                });
            }
        }
        // 预填当前基数（编辑时可改；若不改则沿用）。
        if let Some(select) = self.er_relationship_form_cardinality.get(&tab_id) {
            let card_id = er_cardinality_to_option_id(&relationship.match_cardinality).to_string();
            select.update(cx, |state, cx| {
                state.set_selected_value(&card_id, window, cx)
            });
        }
        cx.notify();
    }

    fn submit_er_relationship_form(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        if self.er_relationship_tasks.contains_key(&tab_id) {
            return;
        }
        let Some(scope_key) = self.er_relationship_scope_keys.get(&tab_id).cloned() else {
            return;
        };
        let Some(role_input) = self.er_relationship_form_role_inputs.get(&tab_id) else {
            return;
        };
        let role = role_input.read(cx).value().trim().to_string();
        let Some(left_entity) = self
            .er_relationship_form_left_tables
            .get(&tab_id)
            .and_then(|select| select.read(cx).selected_value().cloned())
        else {
            self.er_relationship_errors.insert(tab_id, "请选择左表".into());
            return;
        };
        let Some(right_entity) = self
            .er_relationship_form_right_tables
            .get(&tab_id)
            .and_then(|select| select.read(cx).selected_value().cloned())
        else {
            self.er_relationship_errors.insert(tab_id, "请选择右表".into());
            return;
        };
        if role.is_empty() {
            self.er_relationship_errors.insert(tab_id, "业务角色不能为空".into());
            return;
        }
        let mut column_pairs = Vec::new();
        for pair in self.er_relationship_form_pairs.get(&tab_id).into_iter().flatten() {
            let Some(left_column) = pair.left.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请完成全部字段配对".into());
                return;
            };
            let Some(right_column) = pair.right.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请完成全部字段配对".into());
                return;
            };
            column_pairs.push(fluxdb_core::ErColumnPair {
                left_column,
                right_column,
            });
        }
        if column_pairs.is_empty() {
            self.er_relationship_errors.insert(tab_id, "至少需要一组字段配对".into());
            return;
        }
        let mut required_filters = Vec::new();
        for filter in self.er_relationship_form_filters.get(&tab_id).into_iter().flatten() {
            let Some(side) = filter.side.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请选择过滤条件端点".into());
                return;
            };
            let Some(column_id) = filter.column.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请选择过滤条件字段".into());
                return;
            };
            let Some(op_id) = filter.op.read(cx).selected_value().cloned() else {
                self.er_relationship_errors.insert(tab_id, "请选择过滤条件操作".into());
                return;
            };
            let op = match op_id.as_str() {
                "eq" => fluxdb_core::ErFilterOp::Eq,
                "ne" => fluxdb_core::ErFilterOp::Ne,
                "is_null" => fluxdb_core::ErFilterOp::IsNull,
                "is_not_null" => fluxdb_core::ErFilterOp::IsNotNull,
                "in" => fluxdb_core::ErFilterOp::In,
                _ => {
                    self.er_relationship_errors.insert(tab_id, "未知的过滤条件操作".into());
                    return;
                }
            };
            let literal_text = filter.literal.read(cx).value().trim().to_string();
            let literal = if matches!(op, fluxdb_core::ErFilterOp::IsNull | fluxdb_core::ErFilterOp::IsNotNull) {
                fluxdb_core::ErLiteral::Null
            } else if let Some(literal) = er_parse_literal(&literal_text) {
                literal
            } else {
                self.er_relationship_errors.insert(tab_id, "过滤条件字面量不能为空".into());
                return;
            };
            required_filters.push(fluxdb_core::ErRequiredFilter {
                side: if side == "right" {
                    fluxdb_core::ErRelationSide::Right
                } else {
                    fluxdb_core::ErRelationSide::Left
                },
                column_id,
                op,
                literal,
            });
        }
        let description = self
            .er_relationship_form_description_inputs
            .get(&tab_id)
            .map(|input| input.read(cx).value().trim().to_string())
            .filter(|value| !value.is_empty());
        let mut relationship = fluxdb_core::ErRelationship {
            id: format!(
                "user-{}-{}",
                tab_id.0,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default()
            ),
            revision: 1,
            left_entity,
            right_entity,
            role,
            column_pairs,
            required_filters,
            match_cardinality: {
                let card_id = self
                    .er_relationship_form_cardinality
                    .get(&tab_id)
                    .and_then(|select| select.read(cx).selected_value().cloned())
                    .unwrap_or_else(|| "unknown".to_string());
                er_option_id_to_cardinality(&card_id)
            },
            origin: fluxdb_core::ErRelationshipOrigin::User,
            review: fluxdb_core::ErRelationshipReview {
                state: fluxdb_core::ErReviewState::Proposed,
                confirmed_revision: None,
                confirmed_by: None,
            },
            enforcement: fluxdb_core::ErRelationshipEnforcement {
                kind: fluxdb_core::ErEnforcementKind::None,
                constraint_ref: None,
                enforced: None,
            },
            validity: fluxdb_core::ErValidity {
                state: fluxdb_core::ErValidityState::Current,
                reason: None,
            },
            description,
            evidence_refs: Vec::new(),
        };
        let command = if let Some(editing_id) = self
            .er_relationship_form_editing
            .get(&tab_id)
            .cloned()
            .flatten()
        {
            let Some(current) = self
                .er_relationships
                .get(&tab_id)
                .into_iter()
                .flatten()
                .find(|relationship| relationship.id == editing_id)
                .cloned()
            else {
                self.er_relationship_errors.insert(tab_id, "关系已不存在，请刷新列表".into());
                return;
            };
            relationship.id = current.id.clone();
            relationship.revision = current.revision;
            // match_cardinality 取表单选择（编辑时已预填当前值，可改）。
            relationship.origin = current.origin;
            relationship.review = current.review;
            relationship.enforcement = current.enforcement;
            relationship.validity = current.validity;
            relationship.evidence_refs = current.evidence_refs;
            AppCommand::UpdateErRelationship {
                scope_key,
                relationship,
                expected_revision: current.revision,
            }
        } else {
            AppCommand::CreateErRelationship {
                scope_key,
                relationship,
            }
        };
        self.run_er_relationship_command(
            tab_id,
            command,
            cx,
        );
    }
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
        .child(er_group_bar(tab_id, this, colors, cx))
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
                    this.er_field_highlights.get(&tab_id).cloned(),
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
                    .child(er_search_dropdown(tab_id, this, colors, cx))
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

/// 本地逻辑关系列表。它是画布上的非模态右侧面板：不改变画布 scope，也不因普通画布
/// 点击关闭；关闭只由标题栏按钮完成。列表数据来自应用层 service 的后台读取缓存。
fn er_relationship_panel(
    tab_id: TabId,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let loading = this.er_relationship_loading.contains(&tab_id);
    let error = this.er_relationship_errors.get(&tab_id).cloned();
    let relationships = this.er_relationships.get(&tab_id).cloned().unwrap_or_default();
    if this.er_relationship_form_open.contains(&tab_id) {
        this.ensure_er_relationship_form_controls(tab_id, window, cx);
    }
    let close_tab = tab_id;
    // 面板宽度可拖动（左缘），按 tab 记忆。
    let panel_width = this
        .er_relationship_panel_width
        .get(&tab_id)
        .copied()
        .unwrap_or(360.0)
        .clamp(ER_REL_PANEL_MIN_W, ER_REL_PANEL_MAX_W);
    let mut panel = div()
        .absolute()
        .top(px(0.))
        .right(px(0.))
        .bottom(px(0.))
        .w(px(panel_width))
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .shadow_lg();

    // 左缘拖宽手柄。
    panel = panel.child(
        div()
            .id(("er-rel-panel-handle", tab_id.0))
            .absolute()
            .top_0()
            .left_0()
            .bottom_0()
            .w(px(6.))
            .cursor_ew_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    this.er_relationship_panel_resize_start = Some(SidebarResizeStart {
                        x: f32::from(event.position.x),
                        width: panel_width,
                    });
                    cx.stop_propagation();
                }),
            )
            .on_drag(SidebarResizeDrag, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_drag_move(cx.listener(
                move |this, event: &gpui::DragMoveEvent<SidebarResizeDrag>, _, cx| {
                    cx.stop_propagation();
                    if let Some(start) = this.er_relationship_panel_resize_start {
                        // 手柄在面板左缘：向右拖 = 面板变窄（delta 为负向加大宽度）。
                        let delta = start.x - f32::from(event.event.position.x);
                        let width = (start.width + delta)
                            .clamp(ER_REL_PANEL_MIN_W, ER_REL_PANEL_MAX_W);
                        this.er_relationship_panel_width
                            .entry(tab_id)
                            .and_modify(|w| *w = width)
                            .or_insert(width);
                        cx.notify();
                    }
                },
            ))
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
    );

    panel = panel.child(
        div()
            .h(px(42.))
            .flex()
            .items_center()
            .px(px(12.))
            .border_b_1()
            .border_color(colors.border_soft)
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(colors.text)
                    .child("本地逻辑关系"),
            )
            .child(
                Button::new(("er-rel-new", tab_id.0))
                    .secondary()
                    .xsmall()
                    .label(if this.er_relationship_form_open.contains(&tab_id) {
                        "关闭新建"
                    } else {
                        "新建关系"
                    })
                    .disabled(loading || this.er_full_tables.get(&tab_id).is_none_or(Vec::is_empty))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.er_relationship_form_open.contains(&tab_id) {
                            this.er_relationship_form_open.remove(&tab_id);
                        } else {
                            this.er_relationship_form_open.insert(tab_id);
                            this.er_prepare_new_relationship_form(tab_id, window, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("er-rel-close", tab_id.0))
                    .ghost()
                    .xsmall()
                    .tooltip("关闭关系面板")
                    .child(app_icon(AppIcon::Close, 15., colors.muted))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.er_relationship_panel_open.remove(&close_tab);
                        cx.notify();
                    })),
            ),
    );

    if this.er_relationship_form_open.contains(&tab_id) {
        panel = panel.child(er_relationship_form(tab_id, this, colors, cx));
    }

    if loading {
        panel = panel.child(
            div()
                .px(px(14.))
                .py(px(16.))
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("正在加载关系…"),
        );
    } else if let Some(error) = error {
        let retry_tab = tab_id;
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .px(px(14.))
                .py(px(16.))
                .child(div().text_size(px(12.)).text_color(rgb(0xef4444)).child(error))
                .child(
                    Button::new(("er-rel-retry-list", tab_id.0))
                        .secondary()
                        .small()
                        .label("重试")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.retry_er_relationships(retry_tab, cx);
                        })),
                ),
        );
    } else if relationships.is_empty() {
        panel = panel.child(
            div()
                .px(px(14.))
                .py(px(16.))
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("当前作用域没有本地逻辑关系"),
        );
    } else {
        let mut list = div()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(8.));
        for relationship in relationships {
            let edit_relationship = relationship.clone();
            let relationship_id = relationship.id.clone();
            let relationship_revision = relationship.revision;
            let scope_key = this
                .er_relationship_scope_keys
                .get(&tab_id)
                .cloned()
                .unwrap_or_default();
            let busy = loading;
            let delete_pending = this
                .er_relationship_delete_pending
                .get(&tab_id)
                .is_some_and(|(id, revision)| {
                    id == &relationship_id && *revision == relationship_revision
                });
            let delete_action = if delete_pending {
                let cancel_tab = tab_id;
                let cancel_id = relationship_id.clone();
                let confirm_scope = scope_key.clone();
                let confirm_id = relationship_id.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(0xef4444))
                            .child("仅删除本地关系，不修改数据库外键"),
                    )
                    .child(
                        Button::new(format!("er-rel-delete-cancel-{}-{}", tab_id.0, cancel_id))
                            .ghost()
                            .xsmall()
                            .label("取消")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.er_relationship_delete_pending.remove(&cancel_tab);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(format!("er-rel-delete-confirm-{}-{}", tab_id.0, confirm_id))
                            .danger()
                            .xsmall()
                            .label("确认删除")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_er_relationship_command(
                                    tab_id,
                                    AppCommand::DeleteErRelationship {
                                        scope_key: confirm_scope.clone(),
                                        id: confirm_id.clone(),
                                        expected_revision: relationship_revision,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .into_any_element()
            } else {
                let pending_tab = tab_id;
                let pending_id = relationship_id.clone();
                Button::new(format!("er-rel-delete-{}-{}", tab_id.0, pending_id))
                    .ghost()
                    .xsmall()
                    .label("删除")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.er_relationship_delete_pending.insert(
                            pending_tab,
                            (pending_id.clone(), relationship_revision),
                        );
                        cx.notify();
                    }))
                    .into_any_element()
            };
            let state = match relationship.review.state {
                fluxdb_core::ErReviewState::Proposed => ("待确认", colors.muted),
                fluxdb_core::ErReviewState::Confirmed => ("已确认", rgb(0x16a34a)),
                fluxdb_core::ErReviewState::Rejected => ("已拒绝", rgb(0xef4444)),
            };
            let pairs = relationship
                .column_pairs
                .iter()
                .map(|pair| er_relationship_pair_label(this, tab_id, &relationship, pair))
                .collect::<Vec<_>>()
                .join("\n");
            list = list.child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border_soft)
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().text_size(px(12.)).text_color(colors.text).child(relationship.role))
                            .child(div().text_size(px(11.)).text_color(state.1).child(state.0)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(format!("修订 {} · {}", relationship.revision, relationship.id)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(format!(
                                "{} → {}",
                                this.er_form_table_by_id(tab_id, Some(&relationship.left_entity))
                                    .map(|table| table.reference.display())
                                    .unwrap_or_else(|| relationship.left_entity.clone()),
                                this.er_form_table_by_id(tab_id, Some(&relationship.right_entity))
                                    .map(|table| table.reference.display())
                                    .unwrap_or_else(|| relationship.right_entity.clone())
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.text)
                            .child(pairs),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .pt(px(3.))
                            .child(
                                Button::new(format!("er-rel-edit-{}-{}", tab_id.0, relationship_id))
                                    .ghost()
                                    .xsmall()
                                    .label("编辑")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.er_begin_edit_relationship(
                                            tab_id,
                                            &edit_relationship,
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(format!("er-rel-confirm-{}-{}", tab_id.0, relationship_id))
                                    .ghost()
                                    .xsmall()
                                    .label("确认")
                                    .disabled(busy || relationship.review.state == fluxdb_core::ErReviewState::Confirmed)
                                    .on_click(cx.listener({
                                        let scope_key = scope_key.clone();
                                        let relationship_id = relationship_id.clone();
                                        move |this, _, _, cx| {
                                            this.run_er_relationship_command(
                                                tab_id,
                                                AppCommand::ConfirmErRelationship {
                                                    scope_key: scope_key.clone(),
                                                    id: relationship_id.clone(),
                                                    expected_revision: relationship_revision,
                                                    by: "user".into(),
                                                },
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(
                                Button::new(format!("er-rel-reject-{}-{}", tab_id.0, relationship_id))
                                    .ghost()
                                    .xsmall()
                                    .label("拒绝")
                                    .disabled(busy || relationship.review.state == fluxdb_core::ErReviewState::Rejected)
                                    .on_click(cx.listener({
                                        let scope_key = scope_key.clone();
                                        let relationship_id = relationship_id.clone();
                                        move |this, _, _, cx| {
                                            this.run_er_relationship_command(
                                                tab_id,
                                                AppCommand::RejectErRelationship {
                                                    scope_key: scope_key.clone(),
                                                    id: relationship_id.clone(),
                                                    expected_revision: relationship_revision,
                                                },
                                                cx,
                                            );
                                        }
                                    })),
                            )
                            .child(delete_action),
                    ),
            );
        }
        panel = panel.child(list);
    }
    panel
}

fn er_relationship_pair_label(
    this: &NavicatMain,
    tab_id: TabId,
    relationship: &fluxdb_core::ErRelationship,
    pair: &fluxdb_core::ErColumnPair,
) -> String {
    let table = |entity_id: &String| {
        this.er_full_tables
            .get(&tab_id)
            .into_iter()
            .flatten()
            .find(|table| er_entity_id(&table.reference) == *entity_id)
    };
    let left = table(&relationship.left_entity);
    let right = table(&relationship.right_entity);
    let left_column = left
        .into_iter()
        .flat_map(|table| table.columns.iter())
        .find(|column| {
            left.is_some_and(|table| er_column_id(&table.reference, &column.name) == pair.left_column)
        })
        .map(|column| column.name.clone())
        .unwrap_or_else(|| pair.left_column.clone());
    let right_column = right
        .into_iter()
        .flat_map(|table| table.columns.iter())
        .find(|column| {
            right.is_some_and(|table| {
                er_column_id(&table.reference, &column.name) == pair.right_column
            })
        })
        .map(|column| column.name.clone())
        .unwrap_or_else(|| pair.right_column.clone());
    let left_name = left
        .map(|table| table.reference.display())
        .unwrap_or_else(|| relationship.left_entity.clone());
    let right_name = right
        .map(|table| table.reference.display())
        .unwrap_or_else(|| relationship.right_entity.clone());
    format!("{left_name}.{left_column} ↔ {right_name}.{right_column}")
}

fn er_relationship_form(
    tab_id: TabId,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(role_input) = this.er_relationship_form_role_inputs.get(&tab_id).cloned() else {
        return div();
    };
    let Some(description_input) = this
        .er_relationship_form_description_inputs
        .get(&tab_id)
        .cloned()
    else {
        return div();
    };
    let Some(left_table) = this.er_relationship_form_left_tables.get(&tab_id).cloned() else {
        return div();
    };
    let Some(right_table) = this.er_relationship_form_right_tables.get(&tab_id).cloned() else {
        return div();
    };
    let Some(cardinality) = this.er_relationship_form_cardinality.get(&tab_id).cloned() else {
        return div();
    };
    let pairs = this
        .er_relationship_form_pairs
        .get(&tab_id)
        .map(|pairs| {
            pairs
                .iter()
                .map(|pair| (pair.left.clone(), pair.right.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let filters = this
        .er_relationship_form_filters
        .get(&tab_id)
        .map(|filters| {
            filters
                .iter()
                .map(|filter| {
                    (
                        filter.side.clone(),
                        filter.column.clone(),
                        filter.op.clone(),
                        filter.literal.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let submitting = this.er_relationship_tasks.contains_key(&tab_id);
    let mut form = div()
        .mx(px(10.))
        .my(px(8.))
        .p(px(10.))
        .rounded_md()
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.canvas_bg)
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child("新建本地逻辑关系"),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .child(Select::new(&left_table).small().search_placeholder("选择左表")),
                )
                .child(
                    div()
                        .flex_1()
                        .child(Select::new(&right_table).small().search_placeholder("选择右表")),
                ),
        )
        .child(Input::new(&role_input).small())
        .child(Input::new(&description_input).small())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .w(px(64.))
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child("基数"),
                )
                .child(div().flex_1().child(Select::new(&cardinality).small())),
        );

    for (index, (left, right)) in pairs.into_iter().enumerate() {
        form = form.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .w(px(18.))
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("{}", index + 1)),
                )
                .child(
                    div()
                        .flex_1()
                        .child(Select::new(&left).small().search_placeholder("左字段")),
                )
                .child(div().text_color(colors.muted).child("↔"))
                .child(
                    div()
                        .flex_1()
                        .child(Select::new(&right).small().search_placeholder("右字段")),
                ),
        );
    }

    for (index, (side, column, op, literal)) in filters.into_iter().enumerate() {
        form = form.child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .w(px(18.))
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("F{}", index + 1)),
                )
                .child(div().w(px(58.)).child(Select::new(&side).small()))
                .child(div().flex_1().child(Select::new(&column).small().search_placeholder("字段")))
                .child(div().w(px(74.)).child(Select::new(&op).small()))
                .child(div().flex_1().child(Input::new(&literal).small())),
        );
    }

    let add_tab = tab_id;
    form = form.child(
        div()
            .flex()
            .gap(px(6.))
            .child(
                Button::new(("er-rel-add-pair", tab_id.0))
                    .ghost()
                    .xsmall()
                    .label("添加字段配对")
                    .disabled(submitting)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.er_add_relationship_pair(add_tab, window, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("er-rel-add-filter", tab_id.0))
                    .ghost()
                    .xsmall()
                    .label("添加常驻条件")
                    .disabled(submitting)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.er_add_relationship_filter(tab_id, window, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("er-rel-submit", tab_id.0))
                    .primary()
                    .xsmall()
                    .label(if submitting { "提交中…" } else { "创建" })
                    .disabled(submitting)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit_er_relationship_form(tab_id, cx);
                        cx.notify();
                    })),
            ),
    );
    form
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
    // 重建场景拓扑：坐标保留（er_scene_positions 未动），新表补布局位。
    this.er_scenes.remove(&tab_id);
    maybe_build_scene(tab_id, this);
    // 关系就绪且用户未操作过：允许一次性自动按关系排列（此后再不自动）。
    if !this.er_user_interacted.contains(&tab_id) && !this.er_layout_applied.contains(&tab_id) {
        this.apply_er_layout(tab_id);
    }
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
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move {
                controller.er_catalog_tables(&config, Some(&database), schema.as_deref())
            })
            .await;
        view.update(cx, |this, cx| {
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
                    this.er_full_tables.insert(tab_id, full_tables);
                    this.er_graphs.insert(tab_id, graph);
                    this.sync_er_local_relationship_edges(tab_id);
                    this.er_scenes.remove(&tab_id);
                    // 坐标/固定/视口保留（首次加载时本就为空；手动刷新后保留人工布局，§3.3/§6.3）。
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
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move {
                controller.er_relations(&config, &database, schema.as_deref())
            })
            .await;
        view.update(cx, |this, cx| {
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

    // 待请求表集以结构化身份记录（ErTableRef），缓存与结果归并据此进行，不从展示名反推
    // schema/表名（跨 schema 同名、含点标识符不串表）。
    let mut to_request: BTreeSet<fluxdb_core::ErTableRef> = BTreeSet::new();
    for idx in frame.visible_nodes {
        let meta = &scene.nodes[idx];
        let table = graph.tables.iter().find(|t| t.name == meta.name);
        if let Some(table) = table
            && matches!(table.status, ErLoadStatus::NotLoaded)
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
    if to_request.is_empty() {
        return;
    }
    let entry = this.er_pending_columns.entry(tab_id).or_default();
    let before = entry.len();
    entry.extend(to_request);
    let connection_id = er.connection_id;
    if before == 0 || this.er_column_debounce_tasks.get(&tab_id).is_none() {
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
    let tables: Vec<fluxdb_core::ErTableRef> = pending.into_iter().collect();
    let task = cx.spawn(async move |view, cx| {
        let result = cx
            .background_spawn(async move { controller.er_columns_for_tables(&config, &tables) })
            .await;
        view.update(cx, |this, cx| {
            if let Ok(batch) = result {
                let mut changed = false;
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
                if changed {
                    // 只重建拓扑（列内容/高度），坐标不动。full_changed 只影响关系表单字段下拉，
                    // 由下方 notify 触发重渲染刷新选项，无需重建场景。
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
        Some(table) => format!("{} · ER（{} · {} 跳）", er.database, table.display(), current_depth),
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
        if relation_status == ErLoadStatus::Failed {
            // 关系加载失败：提供显式重试入口（作废 Failed 缓存后由 sync_er_relations 重新读取）。
            let retry_conn = er.connection_id;
            let retry_db = er.database.clone();
            let retry_schema = er.schema.clone();
            let retry_tab = tab_id;
            base = base.child(
                Button::new(("er-rel-retry", tab_id.0))
                    .ghost()
                    .xsmall()
                    .h(px(24.))
                    .label("重试关系")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(config) = this
                            .controller
                            .connection_configs()
                            .into_iter()
                            .find(|c| c.id == retry_conn)
                        {
                            this.controller.er_relations_invalidate_failed(
                                &config,
                                &retry_db,
                                retry_schema.as_deref(),
                            );
                            // 清 graph 的 Failed，使 sync_er_relations 重新发起读取。
                            if let Some(graph) = this.er_graphs.get_mut(&retry_tab) {
                                graph.relation_status = ErLoadStatus::NotLoaded;
                            }
                            this.er_relation_tasks.remove(&retry_tab);
                        }
                        cx.notify();
                    })),
            );
        }
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

    let rel_panel_tab = tab_id;
    base = base.child(
        Button::new(("er-relationships", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("本地逻辑关系")
            .child(app_icon(AppIcon::PanelRight, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.er_relationship_panel_open.contains(&rel_panel_tab) {
                    this.er_relationship_panel_open.remove(&rel_panel_tab);
                } else {
                    this.er_relationship_panel_open.insert(rel_panel_tab);
                }
                cx.notify();
            })),
    );

    // 缩放控件（视图变换缩放，§六.23-24）：- / % / + / 适配当前范围。
    let zoom = this.er_viewports.get(&tab_id).map(|v| v.safe_scale()).unwrap_or(1.0);
    let zoom_out_tab = tab_id;
    base = base.child(
        Button::new(("er-zoom-out", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("缩小（-）")
            .child(app_icon(AppIcon::Minus, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.zoom_er_around_center(zoom_out_tab, 1.0 / 1.25);
                cx.notify();
            })),
    );
    base = base.child(
        div()
            .w(px(44.))
            .text_center()
            .text_size(px(11.))
            .text_color(colors.muted)
            .child(format!("{:.0}%", zoom * 100.0)),
    );
    let zoom_in_tab = tab_id;
    base = base.child(
        Button::new(("er-zoom-in", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("放大（+）")
            .child(app_icon(AppIcon::Plus, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.zoom_er_around_center(zoom_in_tab, 1.25);
                cx.notify();
            })),
    );
    let fit_tab = tab_id;
    base = base.child(
        Button::new(("er-fit", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("适配当前范围")
            .child(app_icon(AppIcon::Maximize, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.er_fit_scope(fit_tab);
                cx.notify();
            })),
    );

    // 导出（§十）：打开菜单，选格式生成文本写入剪贴板。
    let export_tab = tab_id;
    base = base.child(
        Button::new(("er-export", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("导出（JSON / DBML / Mermaid / SVG）")
            .child(app_icon(AppIcon::Save, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.er_export_open.contains(&export_tab) {
                    this.er_export_open.remove(&export_tab);
                } else {
                    this.er_export_open.insert(export_tab);
                }
                cx.notify();
            })),
    );

    if er.center_table.is_some() {
        base = base.child(er_depth_selector(tab_id, er, current_depth, colors, cx));
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
                    vp.scale = 1.0;
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
                    vp.scale = 1.0;
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
                    vp.scale = 1.0;
                }
                cx.notify();
            })),
    );

    // 更新时间（缓存时效反馈，§3.5）。
    if let Some(upd) = this.er_last_updated.get(&tab_id) {
        base = base.child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(format!("更新于 {upd}")),
        );
    }
    // 刷新进行中：轻量提示（保留当前可用图，只重读元数据，§3.3/§3.5）。
    if this.er_refreshing.contains(&tab_id) {
        base = base.child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("刷新中…"),
        );
    }
    // 手动刷新：重读元数据（表目录+关系），保留当前可用图、视口与人工布局（§3.3）。
    let refresh_er = er.clone();
    let refresh_tab = tab_id;
    base = base.child(
        Button::new(("er-refresh", tab_id.0))
            .ghost()
            .xsmall()
            .h(px(28.))
            .tooltip("刷新元数据（保留布局）")
            .disabled(this.er_refreshing.contains(&tab_id))
            .child(app_icon(AppIcon::Refresh, 15., colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.er_refreshing.contains(&refresh_tab) {
                    return;
                }
                this.er_refreshing.insert(refresh_tab);
                // 清表目录与关系缓存使重读；保留 er_graphs 当前可用图、er_scene_positions/视口/固定。
                this.er_full_tables.remove(&refresh_tab);
                this.er_all_edges.remove(&refresh_tab);
                this.er_relation_tasks.remove(&refresh_tab);
                // 记录刷新开始为“加载中”，展示「刷新中…」。
                let er = refresh_er.clone();
                ensure_er_graph_loaded(refresh_tab, &er, this, cx);
                cx.notify();
            })),
    );

    base
}

/// ER 表搜索条（§五.6）：仅输入框行（命中列表作为画布后置兄弟绝对定位浮层渲染，
/// 因 GPUI 无 z-index，后置者在上，见 `er_search_dropdown`）。
/// 无场景（元数据未到）时返回空元素，不占工具栏行。
fn er_search_bar(
    tab_id: TabId,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if !this.er_scenes.contains_key(&tab_id) {
        return div();
    }
    let Some(search_input) = this.er_search_input.get(&tab_id).cloned() else {
        return div();
    };
    let matches = this.er_search_matches(tab_id);
    let count_text = if this.er_search_query.get(&tab_id).map(|q| !q.trim().is_empty()).unwrap_or(false) {
        format!("{} 个匹配", matches.len())
    } else {
        String::new()
    };
    let bar_tab = tab_id;
    div()
        .px(px(12.))
        .py(px(6.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .gap_2()
        .child(app_icon(AppIcon::Search, 15., colors.muted))
        .child(
            div()
                .flex_1()
                .h(px(28.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .flex()
                .items_center()
                .px_2()
                // 键盘导航：上/下移动选择，Enter 定位选中表，Esc 清空查询并关闭悬浮层。
                .on_key_down(
                    cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        let key = event.keystroke.key.as_str();
                        let matches = this.er_search_matches(bar_tab);
                        if matches.is_empty() {
                            return;
                        }
                        let cur = this.er_search_sel.get(&bar_tab).copied().unwrap_or(0);
                        let next = match key {
                            "Down" => Some((cur + 1).min(matches.len() - 1)),
                            "Up" => Some(cur.saturating_sub(1)),
                            "Enter" | "Return" => {
                                this.er_center_on_table(bar_tab, &matches[cur]);
                                cx.notify();
                                return;
                            }
                            "escape" => {
                                this.er_search_query.remove(&bar_tab);
                                this.er_search_sel.remove(&bar_tab);
                                cx.notify();
                                return;
                            }
                            _ => None,
                        };
                        if let Some(n) = next {
                            this.er_search_sel.insert(bar_tab, n);
                            cx.notify();
                        }
                    }),
                )
                .child(
                    Input::new(&search_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(13.)),
                ),
        )
        .child(div().text_size(px(11.)).text_color(colors.muted).child(count_text))
}

/// 分组过滤的纯逻辑（§五.10-11）：None=返回全部；Some(schema)=仅保留该 schema 的表，
/// 边只保留两端都属本组者（真实外键，不伪装聚合 JOIN）。孤立表仍保留（始终可达可搜索）。
fn er_group_subset_graph(graph: &ErGraphData, group: Option<&str>) -> ErGraphData {
    let Some(schema) = group else {
        return graph.clone();
    };
    let included: std::collections::BTreeSet<String> = graph
        .tables
        .iter()
        .filter(|t| t.reference.schema.as_deref() == Some(schema))
        .map(|t| t.name.clone())
        .collect();
    let edges = graph
        .edges
        .iter()
        .filter(|e| included.contains(&e.from_table) && included.contains(&e.to_table))
        .cloned()
        .collect();
    let tables = graph
        .tables
        .iter()
        .filter(|t| included.contains(&t.name))
        .cloned()
        .collect();
    ErGraphData {
        tables,
        edges,
        relation_status: graph.relation_status,
    }
}

/// ER 导出菜单（§十）：JSON / DBML / Mermaid / SVG，生成文本写入剪贴板。作为画布后置
/// 兄弟绝对定位叠加。导出内容为「结构快照 + 现有物理外键 + 视图坐标」；SVG 为展示型
/// （表级连线简化）；JSON 带 schema_version，不含密码。导入另行校验（未做 D1-D9 逻辑关系）。
fn er_export_menu(
    tab_id: TabId,
    er: &ErDiagramState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if !this.er_export_open.contains(&tab_id) {
        return div();
    }
    let Some(graph) = this.er_graphs.get(&tab_id) else {
        return div();
    };
    let positions = this.er_scene_positions.get(&tab_id).cloned().unwrap_or_default();
    let pinned = this.er_pinned.get(&tab_id).cloned().unwrap_or_default();
    let label = this
        .controller
        .connection_configs()
        .into_iter()
        .find(|c| c.id == er.connection_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "连接".to_string());
    let title = format!("{} · {}", er.database, er.schema.as_deref().unwrap_or(""));
    let actions: Vec<(&str, &str, String)> = vec![
        (
            "JSON",
            "含 schema_version 的结构 + 坐标 + 固定",
            er_export_json(graph, &positions, &pinned, &label, &er.database),
        ),
        (
            "DBML",
            "Table + Ref（交换格式）",
            er_export_dbml(graph),
        ),
        (
            "Mermaid",
            "erDiagram（文档嵌入）",
            er_export_mermaid(graph),
        ),
        (
            "SVG",
            "展示型（表级连线，有损）",
            er_export_svg(graph, &positions, &title),
        ),
    ];
    let mut menu = div()
        .absolute()
        .top(px(4.))
        .left(px(12.))
        .w(px(280.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(4.),
            px(12.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.45 } else { 0.14 }),
        )])
        .flex()
        .flex_col()
        .py_1();
    menu = menu.child(
        div()
            .h(px(30.))
            .px_3()
            .flex()
            .items_center()
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(colors.text)
            .child("导出 ER"),
    );
    for (name, desc, content) in actions {
        let mtab = tab_id;
        menu = menu.child(
            div()
                .h(px(30.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(|s| s.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(content.clone()));
                        this.er_export_open.remove(&mtab);
                        this.show_message(
                            format!("已复制 {name} 导出到剪贴板"),
                            AppMessageKind::Info,
                            cx,
                        );
                        cx.notify();
                    }),
                )
                .child(div().w(px(64.)).text_size(px(12.)).font_weight(gpui::FontWeight::MEDIUM).text_color(colors.text).child(name))
                .child(div().flex_1().text_size(px(11.)).text_color(colors.muted).child(desc)),
        );
    }
    // 导入 JSON：读剪贴板 → 解析/校验 → 连接绑定 + 差异预览 + 未解析项。先校验后应用，
    // 失败不破坏当前模型；「应用到画布/视图」另行编排（本入口只做校验与预览反馈）。
    let import_tab = tab_id;
    let import_database = er.database.clone();
    let graph_for_import = graph.clone();
    menu = menu.child(
        div()
            .h(px(30.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .hover(|s| s.bg(colors.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.er_export_open.remove(&import_tab);
                    let text = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
                    if text.trim().is_empty() {
                        this.show_message("剪贴板没有可导入的 ER JSON", AppMessageKind::Warning, cx);
                        cx.notify();
                        return;
                    }
                    match er_import_parse(&text, &graph_for_import, &import_database) {
                        Ok(report) => {
                            if !er_import_database_matches(&report, &import_database) {
                                this.show_message(
                                    format!("导入的 database「{}」与当前库「{import_database}」不匹配，未导入", report.database),
                                    AppMessageKind::Warning,
                                    cx,
                                );
                            } else {
                                let mut parts = vec![
                                    format!("导入校验通过：{} 表 / {} 边", report.tables.len(), report.edges.len()),
                                ];
                                if !report.added_tables.is_empty() {
                                    parts.push(format!("新增 {} 表", report.added_tables.len()));
                                }
                                if !report.missing_tables.is_empty() {
                                    parts.push(format!("当前多 {} 表", report.missing_tables.len()));
                                }
                                if !report.unresolved_edges.is_empty() {
                                    parts.push(format!("{} 条边引用未解析", report.unresolved_edges.len()));
                                }
                                this.show_message(parts.join("；"), AppMessageKind::Info, cx);
                            }
                            // 记录最近一次导入结果，供后续「应用导入」使用（本轮未做应用）。
                            this.er_last_import.insert(import_tab, report);
                        }
                        Err(err) => {
                            this.show_message(format!("导入失败（未改动当前模型）：{err}"), AppMessageKind::Error, cx);
                        }
                    }
                    cx.notify();
                }),
            )
            .child(div().w(px(64.)).text_size(px(12.)).font_weight(gpui::FontWeight::MEDIUM).text_color(colors.text).child("导入 JSON"))
            .child(div().flex_1().text_size(px(11.)).text_color(colors.muted).child("解析/校验/绑定/差异预览")),
    );
    menu
}

/// ER 业务分组选择条（§五.9-12）：按 schema 分组的进入/返回全部；折叠进组仅过滤展示
/// 内容，不丢坐标/固定，可随时回全部。无场景或仅一个 schema 时返回空（不占行、不隐藏表）。
fn er_group_bar(
    tab_id: TabId,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let groups = this.er_schema_groups(tab_id);
    let non_none: Vec<_> = groups.iter().filter(|(s, _)| s.is_some()).collect();
    if groups.is_empty() || non_none.len() <= 1 {
        return div();
    }
    let active = this.er_group.get(&tab_id).cloned().unwrap_or(None);
    let mut row = div()
        .px(px(12.))
        .py(px(4.))
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .gap_2()
        .child(div().text_size(px(11.)).text_color(colors.muted).child("分组"));
    // 「全部」chip。
    let all_tab = tab_id;
    row = row.child(
        er_group_chip(&format!("全部（{}）", groups.iter().map(|(_, c)| c).sum::<usize>()), active.is_none(), all_tab, None, colors, cx),
    );
    // 各 schema chip。
    for (schema, count) in groups.iter() {
        if schema.is_none() {
            continue;
        }
        let label = format!("{}（{}）", schema.as_deref().unwrap_or(""), count);
        let chip_schema = schema.clone();
        let chip_tab = tab_id;
        row = row.child(er_group_chip(&label, active.as_deref() == schema.as_deref(), chip_tab, chip_schema, colors, cx));
    }
    row
}

/// 单个分组 chip：active 高亮，点击进入该组（None=全部）。
fn er_group_chip(
    label: &str,
    active: bool,
    tab_id: TabId,
    group: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.to_string();
    let group_tab = tab_id;
    div()
        .h(px(22.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(colors.radius)
        .border_1()
        .border_color(if active { colors.text } else { colors.border })
        .bg(if active { colors.panel_alt } else { colors.panel_bg })
        .text_size(px(11.))
        .text_color(if active { colors.text } else { colors.muted })
        .cursor_pointer()
        .hover(|s| s.bg(colors.hover))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                let current = this.er_group.get(&group_tab).cloned().unwrap_or(None);
                if current.as_deref() != group.as_deref() {
                    this.er_set_group(group_tab, group.clone());
                    cx.notify();
                }
            }),
        )
}

/// 表搜索命中判定（§五.6）：表名或注释包含查询（均已小写化，忽略大小写）。
/// 空查询不命中任何表（由调用方在查询为空时不展示列表）。
fn er_search_text_matches(name: &str, comment: Option<&str>, normalized_query: &str) -> bool {
    if name.to_ascii_lowercase().contains(normalized_query) {
        return true;
    }
    comment
        .map(|c| c.to_ascii_lowercase().contains(normalized_query))
        .unwrap_or(false)
}

/// ER 搜索命中下拉浮层（§五.6）：命中表按名称/注释包含查询（忽略大小写），点击命中表
/// 居中并选中（`er_center_on_table`）。作为画布后置兄弟绝对定位叠放其上（GPUI 无
/// z-index，后置者在上）。空查询/无匹配返回空元素，不遮挡画布。
fn er_search_dropdown(
    tab_id: TabId,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if this.er_search_query.get(&tab_id).map(|q| q.trim()).unwrap_or("").is_empty() {
        return div();
    }
    let matches = this.er_search_matches(tab_id);
    if matches.is_empty() {
        return div();
    }
    let sel_idx = this
        .er_search_sel
        .get(&tab_id)
        .copied()
        .unwrap_or(0)
        .min(matches.len() - 1);
    let mut list = div()
        .mx(px(12.))
        .mt(px(6.))
        .max_h(px(280.))
        .overflow_y_scrollbar()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(4.),
            px(12.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.45 } else { 0.14 }),
        )])
        .flex()
        .flex_col()
        .py_1();
    for (idx, name) in matches.iter().enumerate() {
        let sel_name = name.clone();
        let sel_tab = tab_id;
        let display = name.clone();
        let is_sel = idx == sel_idx;
        list = list.child(
            div()
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(if is_sel { colors.text } else { colors.muted })
                .when(is_sel, |s| s.bg(colors.hover))
                .cursor_pointer()
                .hover(|s| s.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if this.er_center_on_table(sel_tab, &sel_name) {
                            cx.notify();
                        }
                    }),
                )
                .child(display),
        );
    }
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .child(list)
}

/// 展开深度选择按钮组。
fn er_depth_selector(
    tab_id: TabId,
    er: &ErDiagramState,
    current: u8,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let er = er.clone();
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(div().text_size(px(12.)).text_color(colors.muted).child("展开深度"))
        .children((1..=3).map(|depth| {
            let active = depth == current;
            let depth_tab = tab_id;
            let depth_er = er.clone();
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
                        if this.er_depths.get(&depth_tab).copied().unwrap_or(1) != depth {
                            this.er_depths.insert(depth_tab, depth);
                            // 同步重算邻域：保留人工坐标/滚动/固定，不撕图、不产生空画布窗口。
                            // 边集未就绪时才回退触发后台读取。
                            if !recompute_local_er_graph(depth_tab, &depth_er, this, cx) {
                                this.er_graphs.remove(&depth_tab);
                                this.er_scenes.remove(&depth_tab);
                                this.er_relation_tasks.remove(&depth_tab);
                                this.er_errors.remove(&depth_tab);
                            }
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

/// 空库/无关系等明确空状态：给可理解文案 + 刷新入口，不伪装成空白画布（§3.5）。
fn er_empty_state(
    message: &'static str,
    er: &ErDiagramState,
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let msg = message.to_string();
    let refresh_er = er.clone();
    let refresh_tab = tab_id;
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .text_color(colors.muted)
        .text_size(px(13.))
        .child(div().child(msg))
        .child(
            Button::new(("er-empty-refresh", tab_id.0))
                .ghost()
                .xsmall()
                .label("刷新")
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.er_refreshing.contains(&refresh_tab) {
                        return;
                    }
                    this.er_refreshing.insert(refresh_tab);
                    this.er_full_tables.remove(&refresh_tab);
                    this.er_all_edges.remove(&refresh_tab);
                    this.er_relation_tasks.remove(&refresh_tab);
                    let er = refresh_er.clone();
                    ensure_er_graph_loaded(refresh_tab, &er, this, cx);
                    cx.notify();
                })),
        )
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
