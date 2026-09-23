// ER 本地逻辑关系、结构重绑与确认编排。与画布渲染隔离，仍使用既有 include! 同作用域。

impl NavicatMain {
    fn er_advance_generation(&mut self, tab: TabId) {
        let generation = self.er_generation.entry(tab).or_default();
        *generation = generation.wrapping_add(1);
        self.er_pending_columns.remove(&tab);
        self.er_column_debounce_tasks.remove(&tab);
    }

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
            if !er_relationship_effective(&relationship)
                || er_relationship_needs_rebuild_review(self, tab_id, &relationship.id)
            {
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
                // 列名解析：命中已加载列用真实名；字段尚未加载（刷新后待加载）时用 column_id
                // 的末段占位，仍投影逻辑边，避免刷新后连线丢失（§3.6 刷新保留本地编辑）。
                let left_column = left
                    .columns
                    .iter()
                    .find(|column| er_column_id(&left.reference, &column.name) == pair.left_column)
                    .map(|column| column.name.clone())
                    .unwrap_or_else(|| er_column_display_name(&pair.left_column));
                let right_column = right
                    .columns
                    .iter()
                    .find(|column| er_column_id(&right.reference, &column.name) == pair.right_column)
                    .map(|column| column.name.clone())
                    .unwrap_or_else(|| er_column_display_name(&pair.right_column));
                local_edges.push(fluxdb_core::ErForeignKeyEdge {
                    name: format!("logic:{}:{index}", relationship.id),
                    from_table: left.reference.display(),
                    from_column: left_column,
                    to_table: right.reference.display(),
                    to_column: right_column,
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
                        this.er_rebind_scan(tab_id, cx);
                        this.apply_er_rebind_auto(tab_id, cx);
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

    /// 点击画布上的本地逻辑关系边：打开右侧抽屉并高亮该关系（供编辑/确认/删除）。
    fn er_select_relationship(
        &mut self,
        tab_id: TabId,
        relationship_id: String,
        cx: &mut Context<Self>,
    ) {
        // 打开抽屉（若未开）。
        self.er_relationship_panel_open.insert(tab_id);
        self.er_relationship_panel_selected
            .insert(tab_id, Some(relationship_id));
        // 关系目录可能尚未加载：先确保加载，加载后保留选中定位。
        if !self.er_relationships.contains_key(&tab_id)
            && !self.er_relationship_loading.contains(&tab_id)
        {
            self.ensure_er_relationships_loaded(tab_id, cx);
        }
        cx.notify();
    }

    /// 结构刷新重绑扫描（§5.2/D9）：把当前已加载结构比对上一次持久化快照，判断每条关系
    /// 两端是否 unresolved / 同名重建需确认 / 缺列，产出待处理项（不自动改关系）。
    /// 同时按稳定对象标识识别「改名但同对象」的两端，计算可自动重绑的关系（§5.2 第 1 步），
    /// 存入 `er_rebind_auto` 供 `apply_er_rebind_auto` 写回；两端任一 unresolved/needs_review
    /// 则整条留待处理、不自动改（不猜）。
    /// 随后把当前结构存为快照，供下次刷新比对。无连接器稳定标识 → 只走限定名/列名重绑，
    /// 不伪造稳定身份（也不自动重绑）。
    fn er_rebind_scan(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(scope_key) = self.er_relationship_scope_keys.get(&tab_id).cloned() else {
            return;
        };
        let Some(rels) = self.er_relationships.get(&tab_id).cloned() else {
            return;
        };
        let Some(full) = self.er_full_tables.get(&tab_id).cloned() else {
            return;
        };
        let old_snap = self
            .storage
            .load_er_structure_snapshot(&scope_key)
            .unwrap_or_default();
        let old_map: std::collections::HashMap<String, fluxdb_core::ErRebindEntity> = old_snap
            .iter()
            .cloned()
            .map(|e| (e.entity_id.clone(), e))
            .collect();
        // 字段按视口分批加载：只扫描端点已就绪的关系，其他关系沿用旧快照。
        // 不能让一个屏外端点阻塞所有关系，也不能用 Loading 的空字段覆盖旧快照。
        let new_snap = er_rebind_merge_snapshot(&old_snap, &full);
        let mut pending = Vec::new();
        let mut auto = Vec::new();
        let mut invalidations = Vec::new();
        for mut rel in rels {
            if !er_rebind_relation_ready(&rel, &full) {
                pending.extend(self.er_rebind_pending.get(&tab_id).into_iter().flatten()
                    .filter(|item| item.rel_id == rel.id).cloned());
                continue;
            }
            let mut rebuilt = false;
            let mut unresolved = false;
            // 左右两端各自的 "可自动重绑的新实体 id + 列名重写映射（新列 id）"。
            // None = 该端不能自动重绑（unresolved/needs_review/无稳定标识）。
            let mut side_new_entity: (Option<String>, Option<String>) = (None, None);
            let mut side_new_cols: (Option<Vec<(String, String)>>, Option<Vec<(String, String)>>) =
                (None, None);
            let mut can_auto = true;
            for (side, endpoint_entity, label) in [
                (fluxdb_core::ErRelationSide::Left, rel.left_entity.clone(), "左表"),
                (fluxdb_core::ErRelationSide::Right, rel.right_entity.clone(), "右表"),
            ] {
                let Some(old) = old_map.get(&endpoint_entity) else {
                    continue; // 该端尚无旧记录（首次打开），不算待处理，也不自动重绑。
                };
                let used = er_rel_columns_side(&rel, side.clone());
                let out = fluxdb_core::rebind_entity(old, &used, &new_snap);
                if out.entity_unresolved {
                    can_auto = false;
                    unresolved = true;
                    pending.push(ErRebindPendingItem {
                        rel_id: rel.id.clone(),
                        endpoint: format!("{label}：实体未找到"),
                        kind: format!("{}", old.qualified_name),
                    });
                } else if out.entity_needs_review {
                    can_auto = false;
                    rebuilt = true;
                    pending.push(ErRebindPendingItem {
                        rel_id: rel.id.clone(),
                        endpoint: format!("{label}：同名重建需确认"),
                        kind: old.qualified_name.clone(),
                    });
                }
                let mut columns_ok = true;
                let mut col_map = Vec::new();
                for col in out.columns {
                    if col.unresolved {
                        can_auto = false;
                        unresolved = true;
                        columns_ok = false;
                        pending.push(ErRebindPendingItem {
                            rel_id: rel.id.clone(),
                            endpoint: format!("{label}：缺列"),
                            kind: col.old_column_id,
                        });
                    } else if let Some(nc) = col.new_column_id {
                        col_map.push((col.old_column_id, nc));
                    }
                }
                // 仅当「旧实体确带稳定标识且命中（改名但同对象）」才自动重绑实体层级；
                // 无稳定标识时（MySQL/SQLite）实体按名匹配恒等于自身，不重写，避免虚假变更。
                let entity_auto = old.stable_id.is_some()
                    && !out.entity_unresolved
                    && !out.entity_needs_review;
                if !columns_ok || !entity_auto {
                    can_auto = false;
                }
                match side {
                    fluxdb_core::ErRelationSide::Left => {
                        side_new_entity.0 = entity_auto.then(|| out.matched_entity.clone()).flatten();
                        side_new_cols.0 = columns_ok.then_some(col_map);
                    }
                    fluxdb_core::ErRelationSide::Right => {
                        side_new_entity.1 = entity_auto.then(|| out.matched_entity.clone()).flatten();
                        side_new_cols.1 = columns_ok.then_some(col_map);
                    }
                }
            }
            // 整条可自动重绑（两端都无 unresolved/needs_review 且实体带稳定标识）→ 写回。
            if unresolved && rel.validity.state != fluxdb_core::ErValidityState::Unresolved {
                rel.validity.state = fluxdb_core::ErValidityState::Unresolved;
                rel.validity.reason = Some("关系端点或字段已不存在，请编辑后重新确认".into());
                invalidations.push(rel.clone());
            } else if rebuilt && rel.review.state == fluxdb_core::ErReviewState::Confirmed {
                invalidations.push(rel.clone());
            }
            if can_auto
                && (side_new_entity.0.is_some() || side_new_entity.1.is_some())
            {
                if let Some(nl) = side_new_entity.0 {
                    rel.left_entity = nl.clone();
                }
                if let Some(nr) = side_new_entity.1 {
                    rel.right_entity = nr.clone();
                }
                // 重写列对（按旧列 id → 新列 id）。
                for pair in rel.column_pairs.iter_mut() {
                    if let Some(map) = &side_new_cols.0 {
                        if let Some(nc) = map.iter().find(|(oc, _)| oc == &pair.left_column) {
                            pair.left_column = nc.1.clone();
                        }
                    }
                    if let Some(map) = &side_new_cols.1 {
                        if let Some(nc) = map.iter().find(|(oc, _)| oc == &pair.right_column) {
                            pair.right_column = nc.1.clone();
                        }
                    }
                }
                // required_filters 里的列同样按该侧映射重写（保持附加条件有效）。
                for f in rel.required_filters.iter_mut() {
                    let target = match f.side {
                        fluxdb_core::ErRelationSide::Left => &side_new_cols.0,
                        fluxdb_core::ErRelationSide::Right => &side_new_cols.1,
                    };
                    if let Some(map) = target {
                        if let Some(nc) = map.iter().find(|(oc, _)| oc == &f.column_id) {
                            f.column_id = nc.1.clone();
                        }
                    }
                }
                auto.push(rel);
            }
        }
        // 结构失效必须先持久化，才能推进快照；失败时下次刷新仍可重试。
        if invalidations.is_empty() {
            if let Err(error) = self.storage.save_er_structure_snapshot(&scope_key, &new_snap) {
                tracing::warn!(tab = tab_id.0, error = %error, "ER 结构快照保存失败");
            }
        }
        self.er_rebind_pending.insert(tab_id, pending);
        self.sync_er_local_relationship_edges(tab_id);
        self.er_rebind_auto.insert(tab_id, auto);
        self.apply_er_rebuild_reviews(tab_id, scope_key, invalidations, new_snap, cx);
    }

    /// 同名重建或缺失端点/字段不沿用旧确认：经应用服务乐观锁持久化。
    /// 快照仅在全部写入成功后推进，失败时下一次刷新仍能重新检测旧 oid。
    fn apply_er_rebuild_reviews(
        &mut self,
        tab_id: TabId,
        scope_key: String,
        reviews: Vec<fluxdb_core::ErRelationship>,
        new_snap: Vec<fluxdb_core::ErRebindEntity>,
        cx: &mut Context<Self>,
    ) {
        if reviews.is_empty() || self.er_rebuild_review_tasks.contains_key(&tab_id) {
            return;
        }
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let mut all_saved = true;
            for rel in reviews {
                let id = rel.id.clone();
                let revision = rel.revision;
                let scope = scope_key.clone();
                let ctrl = controller.clone();
                let event = cx.background_spawn(async move {
                    let mut controller = ctrl;
                    controller.dispatch(AppCommand::UpdateErRelationship {
                        scope_key: scope,
                        relationship: rel,
                        expected_revision: revision,
                    })
                }).await;
                let ok = matches!(event, AppEvent::ErRelationshipChanged { .. });
                all_saved &= ok;
                let _ = view.update(cx, |this, cx| {
                    if this.er_relationship_scope_keys.get(&tab_id) != Some(&scope_key) {
                        return;
                    }
                    if ok {
                        this.apply_app_event(&event, cx);
                    } else {
                        tracing::warn!(tab = tab_id.0, rel = %id, "ER 结构失效状态写入失败");
                        if let AppEvent::Failed(error) = &event {
                            this.er_relationship_errors.insert(tab_id, error.message.clone());
                        }
                    }
                    cx.notify();
                });
            }
            let _ = view.update(cx, |this, cx| {
                this.er_rebuild_review_tasks.remove(&tab_id);
                if all_saved && this.er_relationship_scope_keys.get(&tab_id) == Some(&scope_key) {
                    if let Err(error) = this.storage.save_er_structure_snapshot(&scope_key, &new_snap) {
                        tracing::warn!(tab = tab_id.0, error = %error, "ER 结构快照保存失败");
                    }
                }
                cx.notify();
            });
        });
        self.er_rebuild_review_tasks.insert(tab_id, task);
    }

    /// 把 `er_rebind_scan` 计算的、确认可自动重绑的关系逐个写回（§5.2 第 1 步）。
    /// 遵守应用/存储边界：经 AppCommand::UpdateErRelationship 走应用服务 + expected_revision
    /// 并发守卫；每写成功同步更新内存关系列表与逻辑边，失败记录但不回滚其它。
    /// 调用方须在字段合并完成、扫描产出后调用（有 cx 的上下文）。
    fn apply_er_rebind_auto(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(rels) = self.er_rebind_auto.remove(&tab_id) else {
            return;
        };
        if rels.is_empty() {
            return;
        }
        let Some(scope_key) = self.er_relationship_scope_keys.get(&tab_id).cloned() else {
            return;
        };
        // 逐个后台写回：expected_revision 用内存在飞列表里该关系当前修订，避免批间竞争。
        let pending = rels;
        let task_tab = tab_id;
        let task_scope = scope_key;
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            for mut relationship in pending {
                // 只取当前修订；服务端用一个原子写入保留机械重绑前的确认状态。
                let expected = view.update(cx, |this, _| {
                    this.er_relationships.get(&task_tab)
                        .and_then(|list| list.iter().find(|r| r.id == relationship.id))
                        .map(|r| r.revision)
                }).ok().flatten().unwrap_or(relationship.revision);
                relationship.revision = expected;
                // 闭包按 move 捕获，逐条克隆独立状态（并行写回天然有序不共享可变）。
                let rel_for_task = relationship.clone();
                let ctrl = controller.clone();
                let scope_for_task = task_scope.clone();
                let event = cx
                    .background_spawn(async move {
                        let mut controller = ctrl;
                        controller.dispatch(AppCommand::RebindErRelationship {
                            scope_key: scope_for_task,
                            relationship: rel_for_task,
                            expected_revision: expected,
                        })
                    })
                    .await;
                let rel_clone = relationship.clone();
                let _ = view.update(cx, |this, cx| {
                    match &event {
                        AppEvent::ErRelationshipChanged { relationship, .. } => {
                            // 以服务返回为准更新内存与画布。
                            if this.er_relationship_scope_keys.get(&task_tab) == Some(&task_scope) {
                                this.apply_app_event(&event, cx);
                                tracing::info!(tab = task_tab.0, rel = %relationship.id, "ER 结构刷新自动重绑已原子写回");
                            }
                            // 服务已原子更新 confirmed_revision；不再异步二次确认，避免崩溃窗口。

                        }
                        AppEvent::Failed(error) => {
                            // 并发冲突/失败：保留给用户在待处理项里人工处理。
                            tracing::warn!(
                                tab = task_tab.0,
                                rel = %rel_clone.id,
                                error = %error.message,
                                "ER 结构刷新自动重绑写回失败，保留人工处理"
                            );
                        }
                        _ => {}
                    }
                    cx.notify();
                });
            }
            let _ = view.update(cx, |this, _| {
                this.er_rebind_tasks.remove(&task_tab);
            });
        });
        self.er_rebind_tasks.insert(tab_id, task);
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
        let deleted_relation = match &command {
            AppCommand::DeleteErRelationship { id, .. } => self.er_relationships.get(&tab_id)
                .and_then(|relations| relations.iter().find(|rel| &rel.id == id)).cloned(),
            _ => None,
        };
        let undoing_delete = match &command {
            AppCommand::CreateErRelationship { relationship, .. } => self.er_relationship_delete_undo.get(&tab_id)
                .is_some_and(|deleted| deleted.id == relationship.id),
            _ => false,
        };
        let resolved_id = match &command {
            AppCommand::ConfirmErRelationship { id, .. } => Some(id.clone()),
            AppCommand::UpdateErRelationship { relationship, .. } => Some(relationship.id.clone()),
            _ => None,
        };
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
                            if let Some(original) = deleted_relation.clone() {
                                this.er_relationship_delete_undo.insert(tab_id, original);
                                this.show_message("已删除本地关系，可在面板内恢复（需重新确认）", AppMessageKind::Info, cx);
                            }
                        } else if undoing_delete {
                            this.er_relationship_delete_undo.remove(&tab_id);
                        }
                        if closes_form {
                            this.er_relationship_form_open.remove(&tab_id);
                        }
                        if let Some(id) = &resolved_id {
                            if let Some(items) = this.er_rebind_pending.get_mut(&tab_id) {
                                items.retain(|item| &item.rel_id != id);
                            }
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

/// 由 column_id（`db:schema:table::col`）取展示列名（`::` 后末段）。
/// 字段未加载时作占位列名，供逻辑边投影不与真实 FK 冲突（§3.6 刷新保留）。
fn er_column_display_name(column_id: &str) -> String {
    column_id
        .rsplit("::")
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(column_id)
        .to_string()
}

/// 只有已完成字段读取的端点才能做字段失效判断；不存在的表则可立即判定失联。
fn er_rebind_relation_ready(
    rel: &fluxdb_core::ErRelationship,
    tables: &[fluxdb_core::ErTableNode],
) -> bool {
    [&rel.left_entity, &rel.right_entity].iter().all(|id| {
        tables.iter().find(|table| er_entity_id(&table.reference) == **id)
            .is_none_or(|table| table.status == ErLoadStatus::Loaded)
    })
}

/// 部分字段加载时沿用旧端点的列快照；真正已删除的表不沿用旧身份。
/// 表改名但同对象时通过 stable 保留旧身份，等新表字段就绪后再重绑。
fn er_rebind_merge_snapshot(
    old: &[fluxdb_core::ErRebindEntity],
    tables: &[fluxdb_core::ErTableNode],
) -> Vec<fluxdb_core::ErRebindEntity> {
    let mut merged = er_snapshot_entities_from(tables);
    for prior in old {
        if merged.iter().any(|current| current.entity_id == prior.entity_id) {
            continue;
        }
        let incomplete = tables.iter().any(|table| {
            table.status != ErLoadStatus::Loaded
                && (er_entity_id(&table.reference) == prior.entity_id
                    || (prior.stable_id.is_some() && prior.stable_id == table.stable))
        });
        if incomplete {
            merged.push(prior.clone());
        }
    }
    merged
}

/// 由已加载表节点（含字段）建结构快照实体列表（§5.2/D1）：结构化身份、跳过未加载、
/// 无稳定标识如实留空（不伪造）。供刷新重绑扫描 `er_rebind_scan` 使用。
fn er_snapshot_entities_from(
    full: &[fluxdb_core::ErTableNode],
) -> Vec<fluxdb_core::ErRebindEntity> {
    full.iter()
        .filter(|t| t.status == fluxdb_core::ErLoadStatus::Loaded)
        .map(|t| fluxdb_core::ErRebindEntity {
            entity_id: er_entity_id(&t.reference),
            qualified_name: t.reference.display(),
            // PG 表稳定对象标识（pg_class.oid）：改名但同对象可据此自动重绑；无则 None。
            stable_id: t.stable,
            columns: t
                .columns
                .iter()
                .map(|c| fluxdb_core::ErRebindColumn {
                    column_id: er_column_id(&t.reference, &c.name),
                    name: c.name.clone(),
                    stable_id: c.stable,
                })
                .collect(),
        })
        .collect()
}

/// 取关系某侧用到的列 ID（column_pairs 该侧 + required_filters 该侧），去重保序。
fn er_rel_columns_side(
    rel: &fluxdb_core::ErRelationship,
    side: fluxdb_core::ErRelationSide,
) -> Vec<String> {
    let mut out = Vec::new();
    for p in &rel.column_pairs {
        let col = match side {
            fluxdb_core::ErRelationSide::Left => &p.left_column,
            fluxdb_core::ErRelationSide::Right => &p.right_column,
        };
        if !out.contains(col) {
            out.push(col.clone());
        }
    }
    for f in &rel.required_filters {
        if f.side == side && !out.contains(&f.column_id) {
            out.push(f.column_id.clone());
        }
    }
    out
}

/// 刷新后按稳定对象标识继承「改名但同对象」表的场景坐标/固定，并剔除已不存在表的死键。
///
/// 原因：`er_scene_positions`/`er_pinned` 按展示名键（PG `schema.table`）。表改名后旧键
/// 悬空、新名无坐标 → 新表回退网格位，导致画布位置漂移甚至成排重叠。此处用旧持久化快照
/// 与当前图表的表稳定标识（pg_class.oid）配对：同对象旧名 → 新名，坐标/固定整体平移；
/// 未出现在新图中的旧键（真删表）剔除。无稳定标识（MySQL/SQLite）则靠展示名精确匹配搬家
/// （同名不移），不一致即当死键剔除。
fn er_rebind_remap_scene_positions(tab_id: TabId, this: &mut NavicatMain) {
    let Some(scope_key) = this.er_relationship_scope_keys.get(&tab_id).cloned() else {
        return;
    };
    let Some(graph) = this.er_graphs.get(&tab_id).cloned() else {
        return;
    };
    let old_snap = this.storage.load_er_structure_snapshot(&scope_key).unwrap_or_default();
    let positions = this.er_canvas.borrow().er_scene_positions.get(&tab_id).cloned().unwrap_or_default();
    let pinned = this.er_canvas.borrow().er_pinned.get(&tab_id).cloned().unwrap_or_default();
    let (new_positions, new_pinned) =
        er_rebind_remap_positions_pure(&old_snap, &graph.tables, &positions, &pinned);
    let mut canvas = this.er_canvas.borrow_mut();
    canvas.er_scene_positions.insert(tab_id, new_positions);
    canvas.er_pinned.insert(tab_id, new_pinned);
}

/// 纯函数（可单测）：刷新后按稳定对象标识继承「改名但同对象」表的场景坐标/固定，并剔除
/// 已不存在表的死键。原因：`er_scene_positions`/`er_pinned` 按展示名键（PG `schema.table`），
/// 表改名后旧键悬空、新名无坐标 → 新表回退网格位导致画布漂移/重叠。用旧快照与当前图表的
/// 表稳定标识（pg_class.oid）配对平移坐标/固定；无稳定标识（MySQL/SQLite）只靠展示名精确
/// 搬家，命名不符且不在新图中的旧键剔除（位置不猜）。
fn er_rebind_remap_positions_pure(
    old_snap: &[fluxdb_core::ErRebindEntity],
    new_tables: &[fluxdb_core::ErTableNode],
    old_positions: &std::collections::BTreeMap<String, (f32, f32)>,
    old_pinned: &std::collections::BTreeSet<String>,
) -> (
    std::collections::BTreeMap<String, (f32, f32)>,
    std::collections::BTreeSet<String>,
) {
    let new_names: std::collections::HashSet<String> =
        new_tables.iter().map(|t| t.name.clone()).collect();
    let mut old_stable_to_name: std::collections::HashMap<u64, String> =
        std::collections::HashMap::new();
    for e in old_snap {
        if let Some(sid) = e.stable_id {
            old_stable_to_name.insert(sid, e.qualified_name.clone());
        }
    }
    let mut new_stable_to_name: std::collections::HashMap<u64, String> =
        std::collections::HashMap::new();
    for t in new_tables {
        if let Some(sid) = t.stable {
            new_stable_to_name.insert(sid, t.name.clone());
        }
    }
    let mut new_positions: std::collections::BTreeMap<String, (f32, f32)> = std::collections::BTreeMap::new();
    let mut new_pinned: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (name, pos) in old_positions.iter() {
        if new_names.contains(name) {
            // 名字未变：直接保留。
            new_positions.insert(name.clone(), *pos);
            if old_pinned.contains(name) {
                new_pinned.insert(name.clone());
            }
            continue;
        }
        // 名字变了：用稳定标识找同对象新名。
        if let Some(new_name) = old_stable_to_name
            .iter()
            .find(|(_, old_name)| *old_name == name)
            .and_then(|(sid, _)| new_stable_to_name.get(sid))
        {
            new_positions.insert(new_name.clone(), *pos);
            if old_pinned.contains(name) {
                new_pinned.insert(new_name.clone());
            }
        }
        // 无稳定标识命名不符且不是新图表 → 死键，剔除（位置不猜）。
    }
    (new_positions, new_pinned)
}

/// 判断一条本地逻辑关系是否为「有效」关系（§二.3）：只有已确认且结构有效（current）的
/// 关系才进入邻域展开与画布连线。Proposed/Rejected/Stale/Unresolved/Invalid 一律不伪装成
/// 物理外键或已确认关系（默认不混入未确认候选、失效关系不冒充实约束）。
fn er_relationship_needs_rebuild_review(this: &NavicatMain, tab_id: TabId, id: &str) -> bool {
    this.er_rebind_pending.get(&tab_id).is_some_and(|items| {
        items.iter().any(|item| item.rel_id == id)
    })
}

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
        if !er_relationship_effective(&rel)
            || er_relationship_needs_rebuild_review(this, tab_id, &rel.id)
        {
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

