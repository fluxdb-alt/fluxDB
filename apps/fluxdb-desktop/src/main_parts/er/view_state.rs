// ER 自由坐标、视口、分组与视图持久化。沿用 main.rs 的 include! 同作用域。

impl NavicatMain {
    /// 用户已操作画布（平移/拖动/选择/字段滚动）：关系就绪后不再自动重排（§6.2）。
    fn mark_er_interacted(&mut self, tab: TabId) {
        self.er_user_interacted.insert(tab);
    }

    /// 结束节点拖动（画布外释放/窗口失焦/切标签同样调用，§7）。
    fn finish_node_drag(&mut self, tab: TabId) {
        if self.er_canvas.borrow().er_node_drag.as_ref().is_some_and(|(t, ..)| *t == tab) {
            self.er_canvas.borrow_mut().er_node_drag = None;
            // 拖动结束持久化坐标/固定（§十）。
            self.er_save_view_state(tab);
        }
    }

    fn er_remember_layout(&mut self, tab: TabId) {
        let canvas = self.er_canvas.borrow();
        self.er_previous_layout.insert(tab, (
            canvas.er_scene_positions.get(&tab).cloned().unwrap_or_default(),
            canvas.er_pinned.get(&tab).cloned().unwrap_or_default(),
            canvas.er_viewports.get(&tab).copied().unwrap_or_default(),
        ));
    }

    fn er_restore_previous_layout(&mut self, tab: TabId) -> bool {
        let Some((positions, pinned, viewport)) = self.er_previous_layout.remove(&tab) else {
            return false;
        };
        let mut canvas = self.er_canvas.borrow_mut();
        canvas.er_scene_positions.insert(tab, positions);
        canvas.er_pinned.insert(tab, pinned);
        canvas.er_viewports.insert(tab, viewport);
        drop(canvas);
        self.er_save_view_state(tab)
    }

    /// 表卡片世界尺寸：宽固定；高度已加载按真实内容，未加载/加载中按最大包络预留，
    /// 使字段随后加载变高时不会与邻居重叠（§6.2 碰撞包络）。
    fn er_card_size(table: &fluxdb_core::ErTableNode) -> (f32, f32) {
        let h = match table.status {
            ErLoadStatus::Loaded => card_height(table.status, table.columns.len()),
            _ => fluxdb_app::ER_CARD_H_MAX,
        };
        (NODE_WIDTH, h)
    }

    /// 当前图中各表的占位矩形尺寸表（供避让落位使用）。
    fn er_card_sizes(&self, tab: TabId) -> BTreeMap<String, (f32, f32)> {
        self.er_graphs
            .get(&tab)
            .map(|g| {
                g.tables
                    .iter()
                    .map(|t| (t.name.clone(), Self::er_card_size(t)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 与 `er_card_sizes` 配套的矩形构造（尺寸缺失时按最大包络兜底）。
    fn er_rect_with(
        sizes: &BTreeMap<String, (f32, f32)>,
        name: &str,
        x: f32,
        y: f32,
    ) -> fluxdb_app::ErRect {
        let (w, h) = sizes
            .get(name)
            .copied()
            .unwrap_or((fluxdb_app::ER_CARD_W, fluxdb_app::ER_CARD_H_MAX));
        fluxdb_app::ErRect { x, y, w, h }
    }

    /// 已占障碍矩形：`blocked` 中且有坐标的表。`blocked` 传 pinned 即“只躲固定表”，
    /// 传全部已落位表即“躲开所有既有节点”。障碍只读，绝不被本流程移动。
    fn er_obstacle_rects(
        &self,
        tab: TabId,
        sizes: &BTreeMap<String, (f32, f32)>,
        blocked: &BTreeSet<String>,
    ) -> Vec<fluxdb_app::ErRect> {
        let positions = self
            .er_canvas
            .borrow()
            .er_scene_positions
            .get(&tab)
            .cloned()
            .unwrap_or_default();
        let Some(graph) = self.er_graphs.get(&tab) else {
            return Vec::new();
        };
        graph
            .tables
            .iter()
            .filter(|t| blocked.contains(&t.name))
            .filter_map(|t| {
                positions
                    .get(&t.name)
                    .map(|&(x, y)| Self::er_rect_with(sizes, &t.name, x, y))
            })
            .collect()
    }

    /// 「按关系排列」用的落位：所有未固定表采用关系布局坐标，但把已固定表当障碍避让。
    /// 固定表坐标不动（§6.3）；`er_place_avoiding_overlaps` 按连通分量整体平移，
    /// 不会掰断组件内的相对关系。
    fn er_layout_avoiding_pinned(&self, tab: TabId) -> ErLayoutResult {
        let Some(graph) = self.er_graphs.get(&tab) else {
            return ErLayoutResult::new();
        };
        let layout = er_relation_layout(&graph.tables, &graph.edges);
        let pinned = self
            .er_canvas
            .borrow()
            .er_pinned
            .get(&tab)
            .cloned()
            .unwrap_or_default();
        let candidates: ErLayoutResult = layout
            .iter()
            .filter(|(name, _)| !pinned.contains(*name))
            .map(|(name, &(x, y, c))| (name.clone(), (x, y, c)))
            .collect();
        if candidates.is_empty() || pinned.is_empty() {
            return candidates;
        }
        let sizes = self.er_card_sizes(tab);
        let occupied = self.er_obstacle_rects(tab, &sizes, &pinned);
        er_place_avoiding_overlaps(
            &candidates,
            &sizes,
            &occupied,
            fluxdb_app::ER_PLACE_GAP,
            fluxdb_app::ER_PLACE_MAX_RINGS,
        )
    }

    /// 增量落位：保留仍不冲突的既有坐标，只给「新增表」和「确实撞上已固定/已占节点」的表找空位。
    ///
    /// 顺序确定、幂等：
    /// 1. pinned 表视为固定障碍，永不移动（§6.3 用户坐标优先）；
    /// 2. 其余表按名称升序沿用不冲突的既有坐标，避免无谓跳位；
    /// 3. 冲突或尚无坐标的表，用关系布局候选整体避让落位（§6.3 新增表安排在空闲位置）。
    ///
    /// 返回是否有坐标发生变化。
    fn er_reconcile_positions(&mut self, tab: TabId) -> bool {
        let Some(graph) = self.er_graphs.get(&tab) else {
            return false;
        };
        let layout = er_relation_layout(&graph.tables, &graph.edges);
        let pinned = self
            .er_canvas
            .borrow()
            .er_pinned
            .get(&tab)
            .cloned()
            .unwrap_or_default();
        let existing = self
            .er_canvas
            .borrow()
            .er_scene_positions
            .get(&tab)
            .cloned()
            .unwrap_or_default();
        let sizes = self.er_card_sizes(tab);

        // 1) 固定障碍 + 2) 沿用不冲突的既有坐标（按名称升序，结果确定）。
        let mut blockers = self.er_obstacle_rects(tab, &sizes, &pinned);
        let mut accepted: BTreeMap<String, (f32, f32)> = BTreeMap::new();
        let mut movable: BTreeSet<String> = BTreeSet::new();
        let mut names: Vec<&str> = graph.tables.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        for name in names {
            if pinned.contains(name) {
                continue;
            }
            let Some(&(x, y)) = existing.get(name) else {
                movable.insert(name.to_string());
                continue;
            };
            let probe = Self::er_rect_with(&sizes, name, x, y).inflate(fluxdb_app::ER_PLACE_GAP);
            if blockers.iter().any(|b| probe.intersects(*b)) {
                movable.insert(name.to_string());
                continue;
            }
            blockers.push(Self::er_rect_with(&sizes, name, x, y));
            accepted.insert(name.to_string(), (x, y));
        }

        // 3) 冲突/新增表按关系布局候选整体避让落位。
        let mut next = accepted;
        if !movable.is_empty() {
            // 只把需要落位的表交给避让算法：固定表/已接受表已作为障碍传入，
            // 若把它们的候选坐标也一并传入，会额外产生并不存在的障碍。
            let candidates: ErLayoutResult = layout
                .iter()
                .filter(|(name, _)| movable.contains(*name))
                .map(|(name, &(x, y, c))| (name.clone(), (x, y, c)))
                .collect();
            let placed = er_place_avoiding_overlaps(
                &candidates,
                &sizes,
                &blockers,
                fluxdb_app::ER_PLACE_GAP,
                fluxdb_app::ER_PLACE_MAX_RINGS,
            );
            for (name, &(x, y, _)) in placed.iter() {
                if movable.contains(name) {
                    next.insert(name.clone(), (x, y));
                }
            }
            // 兜底：布局未覆盖的表保持原坐标，避免消失。
            for name in &movable {
                if !next.contains_key(name)
                    && let Some(&(x, y)) = existing.get(name)
                {
                    next.insert(name.clone(), (x, y));
                }
            }
        }
        let changed = next != existing;
        self.er_canvas.borrow_mut().er_scene_positions.insert(tab, next);
        changed
    }

    /// 自动落位（关系就绪后调用一次）：只给新增表与撞上既有节点的表找空位，
    /// 保留用户既有坐标与固定，避免每次会话把非固定表整体照抄布局造成跳位（§6.3）。
    ///
    /// 用独立的 `er_auto_placed` 守卫而非 `er_user_interacted`：用户可能在关系返回前先
    /// 平移/缩放，那也会置 interacted；若据此跳过，存量重叠就会一直留在画布上。
    fn er_auto_place_new_tables(&mut self, tab: TabId) {
        if !self.er_auto_placed.insert(tab) {
            return;
        }
        self.er_reconcile_positions(tab);
        self.er_layout_applied.insert(tab);
    }

    /// 应用关系布局：未固定表采用关系布局坐标（并避让已固定表），固定表坐标不动。
    /// 不重读数据库、不清字段/关系缓存（§6.3）。
    fn apply_er_layout(&mut self, tab: TabId) {
        if self.er_graphs.get(&tab).is_none() {
            return;
        }
        let layout = self.er_layout_avoiding_pinned(tab);
        let mut canvas = self.er_canvas.borrow_mut();
        let entry = canvas.er_scene_positions.entry(tab).or_default();
        for (name, &(x, y, _)) in layout.iter() {
            entry.insert(name.clone(), (x, y));
        }
        drop(canvas);
        self.er_layout_applied.insert(tab);
    }

    /// 适配当前范围：把图中节点世界范围整体放进画布可见区（留 32px 边距，§六.23）。
    /// 只改 viewport（pan/scale），不动节点坐标/固定/滚动。空图或无节点则回到起点。
    fn er_fit_scope(&mut self, tab: TabId) {
        let bbox = self.er_world_bbox(tab);
        let (cw, ch) = self.er_canvas.borrow().er_canvas_sizes.get(&tab).copied().unwrap_or((960.0, 640.0));
        let mut canvas = self.er_canvas.borrow_mut();
        let vp = canvas.er_viewports.entry(tab).or_default();
        *vp = match bbox {
            Some((min_x, min_y, max_x, max_y)) => fit_viewport_to_bbox(min_x, min_y, max_x, max_y, cw, ch),
            None => ErViewport::default(),
        };
        drop(canvas);
        self.mark_er_interacted(tab);
    }

    /// 首次显示某 ER tab 时，把整图适配居中一次（`er_fit_scope` 的自动版）。
    /// 两个必须等待的条件：
    /// - **布局已落定**：关系索引到达前是「临时概览排列」，之后 `apply_er_layout` 会重排全部
    ///   坐标；若提前适配，缩放到的是临时范围，重排后画面仍偏。故只在该状态非 NotLoaded
    ///   （Loaded/Failed）时适配，与自动布局同一时刻对齐。
    /// - **画布真实尺寸已回报**：否则按估算尺寸算出的缩放不准，等探针回报后重试。
    /// 用户一旦交互过（平移/缩放/拖动）则不再自动改视口，尊重其手动视口与坐标；只做一次。
    fn er_fit_once_on_first_show(&mut self, tab: TabId) {
        if self.er_fit_done.contains(&tab) || self.er_user_interacted.contains(&tab) {
            return;
        }
        let settled = self
            .er_graphs
            .get(&tab)
            .map(|g| g.relation_status)
            .is_some_and(|st| st != ErLoadStatus::NotLoaded);
        if !settled {
            return; // 关系未落定：本次不适配，等就绪帧再试
        }
        let Some((cw, ch)) = self.er_canvas.borrow().er_canvas_sizes.get(&tab).copied() else {
            return;
        };
        if cw <= 0.0 || ch <= 0.0 {
            return;
        }
        if self.er_world_bbox(tab).is_none() {
            return;
        }
        // 首次展示与「回到起点」共用同一可读范围；不能把 pan 生硬置零。
        self.er_restore_initial_view(tab);
    }

    /// 回到首次展示的可读视口；不修改节点位置、固定状态或字段滚动。
    fn er_restore_initial_view(&mut self, tab: TabId) {
        let size = self.er_canvas.borrow().er_canvas_sizes.get(&tab).copied();
        let bbox = self.er_world_bbox(tab);
        let (Some((min_x, min_y, max_x, max_y)), Some((cw, ch))) = (bbox, size) else {
            return;
        };
        if cw <= 0.0 || ch <= 0.0 {
            return;
        }
        let viewport = fit_viewport_to_bbox_with_floor(
            min_x, min_y, max_x, max_y, cw, ch, ER_COLLAPSE_SCALE,
        );
        self.er_canvas.borrow_mut().er_viewports.insert(tab, viewport);
        self.er_fit_done.insert(tab);
    }

    /// 以画布中心为锚缩放（工具栏 +/- 按钮，§六.23）。
    fn zoom_er_around_center(&mut self, tab: TabId, factor: f32) {
        let (cw, ch) = self.er_canvas.borrow().er_canvas_sizes.get(&tab).copied().unwrap_or((960.0, 640.0));
        let mut canvas = self.er_canvas.borrow_mut();
        let vp = canvas.er_viewports.entry(tab).or_default();
        vp.zoom_around((cw / 2.0, ch / 2.0), factor);
        drop(canvas);
        self.mark_er_interacted(tab);
    }

    /// 将某表居中到画布中心并选中（§五.6 搜索定位）：保持当前缩放，仅平移视口。
    /// 返回是否定位到该表（表不在场景/无坐标时返回 false，无法定位不假装成功）。
    fn er_center_on_table(&mut self, tab: TabId, table_name: &str) -> bool {
        let Some(scene) = self.er_scenes.get(&tab).cloned() else {
            return false;
        };
        let Some(&(x, y)) = self
            .er_canvas
            .borrow()
            .er_scene_positions
            .get(&tab)
            .and_then(|m| m.get(table_name))
        else {
            return false;
        };
        let (cw, ch) = self.er_canvas.borrow().er_canvas_sizes.get(&tab).copied().unwrap_or((960.0, 640.0));
        let mut canvas = self.er_canvas.borrow_mut();
        let vp = canvas.er_viewports.entry(tab).or_default();
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
        drop(canvas);
        self.er_canvas.borrow_mut().er_selected_table.insert(tab, Some(table_name.to_string()));
        self.er_canvas.borrow_mut().er_field_highlights.remove(&tab);
        self.mark_er_interacted(tab);
        true
    }

    /// 当前分组下的展示图：None=全部；Some(schema)=仅该 schema 的表及其内部边。
    /// 不复制关系目录（§五.7）；只按结构化 schema 过滤视图内容。
    fn er_display_group_graph(&self, tab: TabId, graph: &ErGraphData, group: &Option<String>) -> ErGraphData {
        let Some(name) = group.as_deref().and_then(|g| g.strip_prefix("\0custom:")) else {
            return er_group_subset_graph(graph, group.as_deref());
        };
        let members = self.er_custom_groups.get(&tab).and_then(|groups| groups.get(name));
        er_custom_group_subset_graph(graph, members)
    }

    /// 重建场景：按当前分组过滤展示图，重算布局，保留已有人工坐标（同表坐标不丢，
    /// 新增表取布局位），替换 er_scenes。普通 pan/滚动不触发，仅首次或分组变化时。
    fn er_rebuild_scene(&mut self, tab: TabId) {
        let Some(graph) = self.er_graphs.get(&tab) else {
            return;
        };
        let group = self.er_group.get(&tab).cloned().unwrap_or(None);
        let display = self.er_display_group_graph(tab, graph, &group);
        let layout = er_relation_layout(&display.tables, &display.edges);
        // 保留已有坐标；只给尚无坐标的表补位，且补位要避让已占矩形（§6.2/§6.3），
        // 否则新增表会照抄布局原点压到既有/固定表上。
        let sizes = self.er_card_sizes(tab);
        let mut missing: BTreeSet<String> = BTreeSet::new();
        {
            let canvas = self.er_canvas.borrow();
            let positions = canvas.er_scene_positions.get(&tab);
            for t in &display.tables {
                if !positions.is_some_and(|p| p.contains_key(&t.name)) {
                    missing.insert(t.name.clone());
                }
            }
        }
        if !missing.is_empty() {
            let occupied = self
                .er_canvas
                .borrow()
                .er_scene_positions
                .get(&tab)
                .map(|p| {
                    p.iter()
                        .map(|(name, &(x, y))| Self::er_rect_with(&sizes, name, x, y))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let candidates: ErLayoutResult = layout
                .iter()
                .filter(|(name, _)| missing.contains(*name))
                .map(|(name, &(x, y, c))| (name.clone(), (x, y, c)))
                .collect();
            let placed = er_place_avoiding_overlaps(
                &candidates,
                &sizes,
                &occupied,
                fluxdb_app::ER_PLACE_GAP,
                fluxdb_app::ER_PLACE_MAX_RINGS,
            );
            let mut canvas = self.er_canvas.borrow_mut();
            let pos = canvas.er_scene_positions.entry(tab).or_default();
            for name in &missing {
                if let Some(&(x, y, _)) = placed.get(name) {
                    pos.insert(name.clone(), (x, y));
                } else if let Some(&(x, y, _)) = layout.get(name) {
                    // 布局未覆盖（理论上不会）：保持可访问的兜底坐标。
                    pos.entry(name.clone()).or_insert((x, y));
                }
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

    fn er_add_selected_to_custom_group(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let name = self.er_group_inputs.get(&tab)
            .map(|input| input.read(cx).value().trim().to_string()).unwrap_or_default();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            self.show_message("请输入 1–64 字符的业务组名", AppMessageKind::Warning, cx);
            return;
        }
        let Some(selected) = self.er_canvas.borrow().er_selected_table.get(&tab).cloned().flatten() else {
            self.show_message("请先在画布中选中一张表", AppMessageKind::Warning, cx);
            return;
        };
        if !self.er_graphs.get(&tab).is_some_and(|g| g.tables.iter().any(|t| t.name == selected)) {
            return;
        }
        self.er_custom_groups.entry(tab).or_default().entry(name.clone()).or_default().insert(selected);
        self.er_set_group(tab, Some(format!("\0custom:{name}")));
        cx.notify();
    }

    fn er_delete_empty_custom_group(&mut self, tab: TabId, name: &str, cx: &mut Context<Self>) {
        let Some(groups) = self.er_custom_groups.get_mut(&tab) else { return; };
        if groups.get(name).is_some_and(|members| !members.is_empty()) { return; }
        if groups.remove(name).is_some() {
            self.er_set_group(tab, None);
            cx.notify();
        }
    }

    fn er_remove_selected_from_custom_group(&mut self, tab: TabId, name: &str, cx: &mut Context<Self>) {
        let Some(selected) = self.er_canvas.borrow().er_selected_table.get(&tab).cloned().flatten() else {
            self.show_message("请先选中组内表", AppMessageKind::Warning, cx);
            return;
        };
        if let Some(members) = self.er_custom_groups.get_mut(&tab).and_then(|groups| groups.get_mut(name)) {
            members.remove(&selected);
            self.er_rebuild_scene(tab);
            self.er_save_view_state(tab);
            cx.notify();
        }
    }

    /// 把当前 tab 的分组/固定/坐标写入持久化（按 scope key）。
    fn er_save_view_state(&mut self, tab: TabId) -> bool {
        let Some(key) = self.er_scope_keys.get(&tab).cloned() else {
            return false;
        };
        let group = self.er_group.get(&tab).cloned().unwrap_or(None);
        let pinned: Vec<String> = self
            .er_canvas
            .borrow()
            .er_pinned
            .get(&tab)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let positions: Vec<(String, f32, f32)> = self
            .er_canvas
            .borrow()
            .er_scene_positions
            .get(&tab)
            .map(|m| m.iter().map(|(n, (x, y))| (n.clone(), *x, *y)).collect())
            .unwrap_or_default();
        // 视口（平移/缩放）也是必要视图状态：与坐标一同持久化，重启后平移/缩放一致（§三）。
        let view_port = self
            .er_canvas
            .borrow()
            .er_viewports
            .get(&tab)
            .map(|vp| fluxdb_storage::ErViewportState {
                pan_x: vp.pan_x,
                pan_y: vp.pan_y,
                scale: vp.safe_scale(),
            });
        let mut states = match self.storage.load_er_view_states() {
            Ok(states) => states,
            Err(error) => {
                tracing::warn!(tab = tab.0, %error, "ER 视图状态读取失败，未覆盖现有存储");
                self.er_view_save_failed.insert(tab);
                return false;
            }
        };
        states.insert(
            key,
            fluxdb_storage::ErViewScopeState {
                group,
                custom_groups: self.er_custom_groups.get(&tab).into_iter().flat_map(|groups| groups.iter())
                    .map(|(name, members)| (name.clone(), members.iter().cloned().collect()))
                    .collect(),
                pinned,
                positions,
                view_port,
            },
        );
        if let Err(e) = self.storage.save_er_view_states(&states) {
            tracing::warn!(tab = tab.0, "er: 保存ER视图状态失败（将重试或下次保存覆盖）: {e}");
            self.er_view_save_failed.insert(tab);
            return false;
        }
        self.er_view_save_failed.remove(&tab);
        true
    }

    /// 导入只应用当前数据库中同名表的布局；数据库结构和外键仍以实时库为准。
    /// 保存失败时回滚内存视图，不给用户虚假的成功反馈。
    fn er_apply_import_layout(&mut self, tab: TabId, connection_id: u64, database: &str, cx: &mut Context<Self>) {
        let Some(report) = self.er_last_import.get(&tab) else { return; };
        if !er_import_scope_matches(report, connection_id, database) {
            self.show_message("导入文件连接或数据库不匹配（旧版无连接身份仅支持预览）", AppMessageKind::Warning, cx);
            return;
        }
        let Some(graph) = self.er_graphs.get(&tab) else { return; };
        let mut canvas = self.er_canvas.borrow_mut();
        let before_positions = canvas.er_scene_positions.get(&tab).cloned();
        let before_pinned = canvas.er_pinned.get(&tab).cloned();
        let (positions, pinned, matched) = er_import_layout_merge(
            report, graph,
            &before_positions.clone().unwrap_or_default(),
            &before_pinned.clone().unwrap_or_default(),
        );
        if matched == 0 {
            drop(canvas);
            self.show_message("导入文件没有可应用的同名表布局", AppMessageKind::Warning, cx);
            return;
        }
        canvas.er_scene_positions.insert(tab, positions);
        canvas.er_pinned.insert(tab, pinned);
        drop(canvas);
        if !self.er_save_view_state(tab) {
            let mut canvas = self.er_canvas.borrow_mut();
            match before_positions {
                Some(positions) => { canvas.er_scene_positions.insert(tab, positions); }
                None => { canvas.er_scene_positions.remove(&tab); }
            }
            match before_pinned {
                Some(pinned) => { canvas.er_pinned.insert(tab, pinned); }
                None => { canvas.er_pinned.remove(&tab); }
            }
            drop(canvas);
            self.show_message("导入布局保存失败，画布已恢复原状", AppMessageKind::Error, cx);
            return;
        }
        self.er_last_import.remove(&tab);
        self.mark_er_interacted(tab);
        self.show_message(format!("已应用 {} 张同名表的布局；未修改数据库结构或关系", matched), AppMessageKind::Info, cx);
        cx.notify();
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
        self.er_custom_groups.insert(tab, s.custom_groups.iter()
            .map(|(name, members)| (name.clone(), members.iter().cloned().collect()))
            .collect());
        if let Some(group) = &s.group {
            self.er_group.insert(tab, Some(group.clone()));
        }
        if !s.positions.is_empty() && !self.er_canvas.borrow().er_scene_positions.contains_key(&tab) {
            let pos: BTreeMap<String, (f32, f32)> = s
                .positions
                .iter()
                .map(|(n, x, y)| (n.clone(), (*x, *y)))
                .collect();
            self.er_canvas.borrow_mut().er_scene_positions.insert(tab, pos);
        }
        if !s.pinned.is_empty() {
            self.er_canvas.borrow_mut().er_pinned.insert(tab, s.pinned.iter().cloned().collect());
        }
        // 恢复保存的视口（平移/缩放）：有保存值则打开即用，不再走首次自动适配，
        // 使重启后视图位置与上次一致（§三）。同时在 fit_done 登记，抑制首次适配覆盖。
        if let Some(vp) = s.view_port.clone() {
            let scale = vp.scale.clamp(ER_MIN_SCALE, ER_MAX_SCALE);
            let mut canvas = self.er_canvas.borrow_mut();
            canvas.er_viewports.insert(tab, ErViewport { pan_x: vp.pan_x, pan_y: vp.pan_y, scale });
        }
        // 有保存视口 = 已有一致视图，跳过首次自动适配（尊重保存状态优先于初始策略）。
        self.er_fit_done.insert(tab);
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
        // 克隆本表坐标，避免跨语句持有 RefCell 借用。
        let positions = self.er_canvas.borrow().er_scene_positions.get(&tab)?.clone();
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
        let (cw, ch) = self.er_canvas.borrow().er_canvas_sizes.get(&tab).copied().unwrap_or((960.0, 640.0));
        let wx = min_x + nx.clamp(0.0, 1.0) * (max_x - min_x).max(1.0);
        let wy = min_y + ny.clamp(0.0, 1.0) * (max_y - min_y).max(1.0);
        let mut canvas = self.er_canvas.borrow_mut();
        let vp = canvas.er_viewports.entry(tab).or_default();
        let sc = vp.safe_scale();
        vp.pan_x = cw / 2.0 - wx * sc;
        vp.pan_y = ch / 2.0 - wy * sc;
        drop(canvas);
        self.mark_er_interacted(tab);
        true
    }
}

