// 数据页外键关系：单元格跳转、关系面板数据与「按外键筛选打开目标表」。
//
// 约束：
// - 这里只做 UI 编排，不拼 SQL；真实查询仍由后台命令 + 连接器参数化执行。
// - 跳转条件统一写入既有筛选规则（`data_filter_rules`），因此目标页可直接
//   继续编辑筛选、翻页和复制，不会出现一套独立的“外键查询模式”。

/// 关系面板默认宽度与拖拽范围（像素）。
const DATA_RELATION_PANEL_DEFAULT_WIDTH: f32 = 364.;
const DATA_RELATION_PANEL_MIN_WIDTH: f32 = 280.;
const DATA_RELATION_PANEL_MAX_WIDTH: f32 = 640.;
/// 面板展开预览的单次读取上限；超过显示「50+」，不为此新增 count 查询。
const DATA_RELATION_PREVIEW_LIMIT: u64 = 50;

impl NavicatMain {
    /// 表页打开时预取外键元数据。
    ///
    /// 单元格跳转不能等到用户打开「表属性 → 外键」才有数据；这里复用现有
    /// `LoadTableInfo` 命令，加载完成后经 `TableInfoChanged` 重建表格委托。
    fn start_foreign_keys_prefetch(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.foreign_keys_for_tab(tab_id).is_some() {
            return;
        }
        if self
            ._table_info_tasks
            .contains_key(&(tab_id, TableInfoTab::ForeignKeys))
        {
            return;
        }
        let Some(editor) = self.data_editor_state(tab_id) else {
            return;
        };
        if !matches!(editor.table_info.foreign_keys, LoadState::NotLoaded) {
            return;
        }
        let task = self.spawn_controller_command(
            AppCommand::LoadTableInfo {
                tab_id,
                tab: TableInfoTab::ForeignKeys,
            },
            move |this, event, cx| {
                let result = match event {
                    AppEvent::TableInfoLoaded(_, _, result) => Ok(result),
                    AppEvent::Failed(error) => Err(error),
                    _ => Err(fluxdb_core::UserFacingError {
                        title: tr("app.data.error.load").to_string(),
                        message: tr("app.data.error.no_table_info_result").to_string(),
                        detail: None,
                        retryable: true,
                    }),
                };
                this._table_info_tasks
                    .remove(&(tab_id, TableInfoTab::ForeignKeys));
                if this.data_editor_state(tab_id).is_some() {
                    let event = this.controller.dispatch(AppCommand::FinishTableInfoLoad {
                        tab_id,
                        tab: TableInfoTab::ForeignKeys,
                        result,
                    });
                    this.apply_app_event(&event, cx);
                }
            },
            cx,
        );
        self._table_info_tasks
            .insert((tab_id, TableInfoTab::ForeignKeys), task);
    }

    /// 表页打开时预取当前库的 ER 逻辑关系。
    ///
    /// 很多库没有物理外键，但用户在 ER 设计里维护了关系；这些关系同样应能
    /// 从数据页顺着走。读取走既有 `LoadErRelationships` 命令（本地模型），
    /// 与 ER 页共享同一作用域目录。
    fn start_logical_relationships_prefetch(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.data_logical_relationships.contains_key(&tab_id)
            || self._data_logical_relationship_tasks.contains_key(&tab_id)
        {
            return;
        }
        let Some(scope_key) = self.data_tab_scope_key(tab_id) else {
            return;
        };
        let task = self.spawn_controller_command(
            AppCommand::LoadErRelationships {
                scope_key: scope_key.clone(),
            },
            move |this, event, cx| {
                this._data_logical_relationship_tasks.remove(&tab_id);
                if this.data_tab_scope_key(tab_id).as_deref() != Some(scope_key.as_str()) {
                    return;
                }
                if let AppEvent::ErRelationshipsLoaded {
                    scope_key: loaded_scope,
                    relationships,
                } = event
                    && loaded_scope == scope_key
                {
                    this.data_logical_relationships
                        .insert(tab_id, relationships);
                    this.refresh_data_table_after_relationship_change(tab_id, cx);
                }
            },
            cx,
        );
        self._data_logical_relationship_tasks.insert(tab_id, task);
    }

    /// 数据页所属的 ER 关系作用域键；非表页或缺少库时返回 `None`。
    fn data_tab_scope_key(&self, tab_id: TabId) -> Option<String> {
        let editor = self.data_editor_state(tab_id)?;
        let database = editor.object.database.as_deref()?;
        Some(fluxdb_core::er_relationship_scope_key(
            editor.object.connection_id,
            database,
            editor.object.schema.as_deref(),
        ))
    }

    /// 当前作用域下所有打开的表页（关系变更时同步刷新它们的跳转目标）。
    fn data_tab_ids_for_scope(&self, scope_key: &str) -> Vec<TabId> {
        self.controller
            .state()
            .tabs
            .iter()
            .filter_map(|tab| match &tab.kind {
                TabKind::DataEditor(_) if self.data_tab_scope_key(tab.id).as_deref() == Some(scope_key) => {
                    Some(tab.id)
                }
                _ => None,
            })
            .collect()
    }

    /// ER 关系加载/变更后重建当前表页的跳转目标，并对已打开面板触发重绘。
    fn refresh_data_table_after_relationship_change(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let page = self
            .data_editor_state(tab_id)
            .and_then(|editor| editor.page.clone());
        if let Some(page) = page {
            self.refresh_data_table_state(tab_id, &page, cx);
        }
        cx.notify();
    }

    /// 处理关系目录变更事件：作用域内已打开的表页都同步一份目录，
    /// 包括尚未完成首次预取的表页，避免它们随后用旧结果覆盖。
    fn refresh_logical_relationships_for_scope(
        &mut self,
        scope_key: &str,
        mut update: impl FnMut(&mut Vec<fluxdb_core::ErRelationship>),
        cx: &mut Context<Self>,
    ) {
        for tab_id in self.data_tab_ids_for_scope(scope_key) {
            let relationships = self.data_logical_relationships.entry(tab_id).or_default();
            update(relationships);
            self.refresh_data_table_after_relationship_change(tab_id, cx);
        }
    }

    /// 关系面板宽度；未拖拽过时用默认值。
    fn data_relation_panel_width(&self, tab_id: TabId) -> f32 {
        self.data_relation_panel_widths
            .get(&tab_id)
            .copied()
            .unwrap_or(DATA_RELATION_PANEL_DEFAULT_WIDTH)
            .clamp(DATA_RELATION_PANEL_MIN_WIDTH, DATA_RELATION_PANEL_MAX_WIDTH)
    }

    /// 展开/收起关系面板条目的内联预览。
    ///
    /// 首次展开才读取目标表的前 50 行；收起不影响缓存，同一行再次展开直接复用。
    /// `key` 已由面板带上行身份，换行不会命中旧结果。
    fn toggle_data_relation_preview(
        &mut self,
        tab_id: TabId,
        key: String,
        target: ObjectPath,
        filters: Vec<(String, CellValue)>,
        cx: &mut Context<Self>,
    ) {
        let entry = (tab_id, key.clone());
        if self.data_relation_expanded.remove(&entry) {
            // 失败结果不缓存到下次展开，收起后再次点开可重试。
            if matches!(
                self.data_relation_previews.get(&entry),
                Some(RelationPreviewState::Failed)
            ) {
                self.data_relation_previews.remove(&entry);
            }
            cx.notify();
            return;
        }
        self.data_relation_expanded.insert(entry.clone());
        if !self.data_relation_previews.contains_key(&entry)
            && !self._data_relation_preview_tasks.contains_key(&entry)
        {
            self.start_data_relation_preview(tab_id, key, target, filters, cx);
        }
        cx.notify();
    }

    /// 后台读取一条关系预览；UI 只传字段与值，SQL 由连接器构造。
    fn start_data_relation_preview(
        &mut self,
        tab_id: TabId,
        key: String,
        target: ObjectPath,
        filters: Vec<(String, CellValue)>,
        cx: &mut Context<Self>,
    ) {
        let specs = filters
            .into_iter()
            .map(|(field, value)| FilterSpec {
                field,
                op: FilterOp::Eq,
                values: vec![value],
                enabled: true,
            })
            .collect::<Vec<_>>();
        let entry = (tab_id, key.clone());
        self.data_relation_previews
            .insert(entry.clone(), RelationPreviewState::Loading);
        let task = self.spawn_controller_command(
            AppCommand::LoadRelationRows {
                tab_id,
                key: key.clone(),
                object: target,
                filters: specs,
                limit: DATA_RELATION_PREVIEW_LIMIT,
            },
            move |this, event, cx| {
                let entry = (tab_id, key.clone());
                this._data_relation_preview_tasks.remove(&entry);
                // 标签页已关闭时缓存一并清理，迟到的结果不再写回。
                if !this.data_relation_previews.contains_key(&entry) {
                    return;
                }
                if let AppEvent::RelationRowsLoaded {
                    tab_id: loaded_tab,
                    key: loaded_key,
                    result,
                } = event
                    && loaded_tab == tab_id
                    && loaded_key == key
                {
                    let state = match result {
                        Ok(page) => RelationPreviewState::Loaded(page),
                        Err(_) => RelationPreviewState::Failed,
                    };
                    this.data_relation_previews.insert(entry, state);
                    cx.notify();
                }
            },
            cx,
        );
        self._data_relation_preview_tasks.insert(entry, task);
    }

    /// 当前表已加载的外键元数据；未加载/加载中返回 `None`。
    fn foreign_keys_for_tab(&self, tab_id: TabId) -> Option<(&ObjectPath, &[ForeignKeyInfo])> {
        let editor = self.data_editor_state(tab_id)?;
        match &editor.table_info.foreign_keys {
            LoadState::Loaded(foreign_keys) => {
                Some((&editor.object, foreign_keys.as_slice()))
            }
            _ => None,
        }
    }

    fn data_editor_state(&self, tab_id: TabId) -> Option<&DataEditorState> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => Some(editor),
                _ => None,
            })
    }

    /// 给数据页表格委托补上单元格外键跳转目标；其它页（查询结果等）无外键元数据。
    fn apply_foreign_key_targets_to_delegate(
        &self,
        tab_id: TabId,
        delegate: &mut DataPageTableDelegate,
    ) {
        if let Some((object, foreign_keys)) = self.foreign_keys_for_tab(tab_id) {
            delegate.apply_foreign_key_targets(object, foreign_keys);
        }
        if let Some(editor) = self.data_editor_state(tab_id)
            && let Some(relationships) = self.data_logical_relationships.get(&tab_id)
        {
            delegate.apply_logical_relationship_targets(&editor.object, relationships);
        }
    }

    /// 打开关系面板；首次打开时后台读取整库外键索引（与 ER 图共享缓存）。
    fn toggle_data_relation_panel(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.data_relation_panels.remove(&tab_id) {
            cx.notify();
            return;
        }
        self.data_relation_panels.insert(tab_id);
        self.start_data_relation_edges_load(tab_id, cx);
        cx.notify();
    }

    /// 按需读取当前库的外键关系边。
    ///
    /// `er_relations` 首次调用会在后台线程读库，期间返回 `Loading`；这里短暂
    /// 轮询到终态后将结果落到 UI 侧缓存。缓存与 ER 图共用，后续打开面板不再读库。
    fn start_data_relation_edges_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.data_relation_edges.contains_key(&tab_id)
            || self.data_relation_loading.contains(&tab_id)
            || self._data_relation_tasks.contains_key(&tab_id)
        {
            return;
        }
        let Some(editor) = self.data_editor_state(tab_id) else {
            return;
        };
        let Some(database) = editor.object.database.clone() else {
            self.data_relation_failed.insert(tab_id);
            cx.notify();
            return;
        };
        let schema = editor.object.schema.clone();
        let Some(config) = self
            .controller
            .connection_configs()
            .into_iter()
            .find(|config| config.id == editor.object.connection_id)
        else {
            self.data_relation_failed.insert(tab_id);
            cx.notify();
            return;
        };
        self.data_relation_loading.insert(tab_id);
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let mut snapshot = None;
            // 首次读取由后台线程完成，`Loading` 只表示别的调用方已发起读取。
            // 上限 10 秒，避免异常情况下面板一直停在加载态。
            for _ in 0..50 {
                let config = config.clone();
                let database = database.clone();
                let schema = schema.clone();
                let controller = controller.clone();
                let result = cx
                    .background_spawn(async move {
                        controller.er_relations(&config, &database, schema.as_deref())
                    })
                    .await;
                match result {
                    Ok(snapshot_value)
                        if snapshot_value.status == ErLoadStatus::Loading =>
                    {
                        cx.background_executor()
                            .timer(Duration::from_millis(200))
                            .await;
                    }
                    other => {
                        snapshot = Some(other);
                        break;
                    }
                }
            }
            let _ = view.update(cx, |this, cx| {
                this._data_relation_tasks.remove(&tab_id);
                this.data_relation_loading.remove(&tab_id);
                if this.data_editor_state(tab_id).is_none() {
                    return;
                }
                match snapshot {
                    Some(Ok(snapshot)) if snapshot.status == ErLoadStatus::Loaded => {
                        this.data_relation_failed.remove(&tab_id);
                        this.data_relation_edges.insert(tab_id, snapshot.edges);
                    }
                    _ => {
                        this.data_relation_failed.insert(tab_id);
                    }
                }
                cx.notify();
            });
        });
        self._data_relation_tasks.insert(tab_id, task);
    }

    /// 打开目标表并按给定列的等值条件筛选。
    ///
    /// 条件写入与工具栏完全相同的筛选规则；目标页已有同表标签时复用该标签，
    /// 这样外键跳转不会无限新开标签，也能继续沿用现有筛选交互。
    fn open_related_table(
        &mut self,
        object: ObjectPath,
        filters: Vec<(String, CellValue)>,
        cx: &mut Context<Self>,
    ) {
        if filters.is_empty() {
            return;
        }
        let event = self.dispatch(AppCommand::OpenDataEditor(object), cx);
        let tab_id = match event {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => tab_id,
            AppEvent::Failed(error) => {
                self.show_message(error.message, AppMessageKind::Warning, cx);
                return;
            }
            _ => return,
        };
        let rules = data_filter_rules_for_foreign_key(filters);
        self.data_filter_draft_rules.insert(tab_id, rules.clone());
        self.data_filter_rules.insert(tab_id, rules);
        self.data_filter_modes.insert(tab_id, DataFilterMode::Builder);
        self.perform_data_filter_and_sort(tab_id, cx);
    }
}

/// 把当前表相关的 ER 逻辑关系投影成单列外键目标。
///
/// 逻辑关系是对称的「等值关联」，没有物理外键方向；当前表在左端时源列取
/// `left_column`、目标取右表，在右端时反之。复合关系不生成单元格跳转，
/// 与物理复合外键一致，统一走右侧面板。
fn logical_relationship_foreign_keys(
    object: &ObjectPath,
    relationships: &[fluxdb_core::ErRelationship],
) -> Vec<ForeignKeyInfo> {
    let mut foreign_keys = Vec::new();
    for relationship in relationships {
        if matches!(
            relationship.validity.state,
            fluxdb_core::ErValidityState::Invalid
        ) || relationship.column_pairs.len() != 1
        {
            continue;
        }
        let Some(left) = er_reference_from_entity(&relationship.left_entity) else {
            continue;
        };
        let Some(right) = er_reference_from_entity(&relationship.right_entity) else {
            continue;
        };
        let pair = &relationship.column_pairs[0];
        if er_reference_matches_object(&left, object) {
            foreign_keys.push(logical_foreign_key(
                relationship,
                &pair.left_column,
                &right,
                &pair.right_column,
            ));
        }
        if er_reference_matches_object(&right, object) {
            foreign_keys.push(logical_foreign_key(
                relationship,
                &pair.right_column,
                &left,
                &pair.left_column,
            ));
        }
    }
    foreign_keys
}

fn logical_foreign_key(
    relationship: &fluxdb_core::ErRelationship,
    source_column_id: &str,
    target: &fluxdb_core::ErTableRef,
    target_column_id: &str,
) -> ForeignKeyInfo {
    ForeignKeyInfo {
        name: format!("logic:{}", relationship.id),
        column: fluxdb_core::er_column_id_name(source_column_id).to_string(),
        ref_schema: target.schema.clone(),
        ref_table: target.name.clone(),
        ref_column: fluxdb_core::er_column_id_name(target_column_id).to_string(),
        columns: Vec::new(),
        ref_columns: Vec::new(),
    }
}

/// 把外键值转成等值筛选规则。
///
/// 有类型值时保留原始类型（整数列不会按字符串比较）；无类型值退回展示文本，
/// 与用户手工输入的行为一致。
fn data_filter_rules_for_foreign_key(filters: Vec<(String, CellValue)>) -> Vec<DataFilterRule> {
    filters
        .into_iter()
        .map(|(field, value)| {
            let typed = cell_value_filter_value(&value);
            let text = value.display_label();
            DataFilterRule {
                enabled: true,
                field: Some(field),
                operator: DataFilterOperator::Eq,
                values: vec![text],
                advanced_expression: None,
                typed_values: typed.map(|value| vec![value]),
                connector: fluxdb_core::FilterCombinator::And,
                group_depth: 0,
                group_end: 0,
            }
        })
        .collect()
}

fn cell_value_filter_value(value: &CellValue) -> Option<fluxdb_core::FilterValue> {
    match value {
        CellValue::Null => None,
        CellValue::Bool(value) => Some(fluxdb_core::FilterValue::Boolean(*value)),
        CellValue::I64(value) => Some(fluxdb_core::FilterValue::Integer(*value)),
        CellValue::F64(value) => Some(fluxdb_core::FilterValue::Float(*value)),
        CellValue::Text(value) | CellValue::Json(value) => {
            Some(fluxdb_core::FilterValue::Text(value.clone()))
        }
        CellValue::TruncatedText { preview, .. } => {
            Some(fluxdb_core::FilterValue::Text(preview.clone()))
        }
        CellValue::LargeValue(large) => {
            Some(fluxdb_core::FilterValue::Text(large.preview.clone()))
        }
        CellValue::Bytes(_) | CellValue::BinarySummary(_) => None,
    }
}

#[cfg(test)]
mod foreign_key_filter_tests {
    use super::*;

    #[test]
    fn keeps_typed_value_for_equality_filter() {
        let rules = data_filter_rules_for_foreign_key(vec![
            ("user_id".to_string(), CellValue::I64(7)),
        ]);

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].field.as_deref(), Some("user_id"));
        assert_eq!(rules[0].operator, DataFilterOperator::Eq);
        assert_eq!(
            rules[0].typed_values,
            Some(vec![fluxdb_core::FilterValue::Integer(7)])
        );
    }

    #[test]
    fn null_value_has_no_typed_filter() {
        let rules = data_filter_rules_for_foreign_key(vec![
            ("coupon_id".to_string(), CellValue::Null),
        ]);

        assert_eq!(rules[0].typed_values, None);
    }

    #[test]
    fn logical_relationship_exposes_single_column_target() {
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("community_test".to_string()),
            schema: None,
            name: "3d_ip_region".to_string(),
            kind: ObjectKind::Table,
        };
        let left_reference = fluxdb_core::ErTableRef {
            database: "community_test".to_string(),
            schema: None,
            name: "3d_ip_region".to_string(),
        };
        let right_reference = fluxdb_core::ErTableRef {
            database: "community_test".to_string(),
            schema: None,
            name: "3d_countries".to_string(),
        };
        let relationship = fluxdb_core::ErRelationship {
            id: "rel_country".to_string(),
            revision: 2,
            left_entity: fluxdb_core::er_entity_id(&left_reference),
            right_entity: fluxdb_core::er_entity_id(&right_reference),
            role: "ip关联国家".to_string(),
            column_pairs: vec![fluxdb_core::ErColumnPair {
                left_column: fluxdb_core::er_column_id(&left_reference, "country_id"),
                right_column: fluxdb_core::er_column_id(&right_reference, "id"),
            }],
            required_filters: Vec::new(),
            match_cardinality: fluxdb_core::ErMatchCardinality {
                left_to_right: fluxdb_core::ErCardinality {
                    min: fluxdb_core::ErCardinalityBound::Unknown,
                    max: fluxdb_core::ErCardinalityBound::Unknown,
                },
                right_to_left: fluxdb_core::ErCardinality {
                    min: fluxdb_core::ErCardinalityBound::Unknown,
                    max: fluxdb_core::ErCardinalityBound::Unknown,
                },
                basis: fluxdb_core::ErCardinalityBasis::Unknown,
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
            description: None,
            evidence_refs: Vec::new(),
        };

        let foreign_keys = logical_relationship_foreign_keys(&object, &[relationship]);

        assert_eq!(foreign_keys.len(), 1);
        assert_eq!(foreign_keys[0].column, "country_id");
        assert_eq!(foreign_keys[0].ref_table, "3d_countries");
        assert_eq!(foreign_keys[0].ref_column, "id");
    }
}
