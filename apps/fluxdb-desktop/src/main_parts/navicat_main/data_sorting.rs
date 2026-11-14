impl NavicatMain {
    fn start_data_filter_sort_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        // 先解析表达式再领取新 generation：录入错误应直接提示并终止本次应用，
        // 不能清空上一页结果或让加载态悬空。
        let expression = match self.data_filter_expression_for_tab(tab_id) {
            Ok(expression) => Some(expression),
            Err(error) => {
                self.data_filter_applying_tabs.remove(&tab_id);
                self.show_message(error.to_string(), AppMessageKind::Warning, cx);
                return;
            }
        };
        // 用户再次应用筛选时直接接管：领取新令牌，旧请求的结果会在回写时被丢弃。
        let event = self
            .controller
            .dispatch(AppCommand::BeginDataPageLoad(tab_id));
        let generation = match event {
            AppEvent::DataPageLoadStarted { generation, .. } => generation,
            _ => {
                self.data_filter_applying_tabs.remove(&tab_id);
                return;
            }
        };
        self._data_filter_apply_generations
            .insert(tab_id.0, generation);
        let mut controller = self.controller.clone();
        let sort = self.data_sort_specs_for_tab(tab_id);
        let filters = self.data_filter_specs_for_tab(tab_id);
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadDataPageWithSort {
                        tab_id,
                        generation,
                        sort,
                        filters,
                        expression,
                    }) {
                        AppEvent::DataLoaded(_, page) => Ok(Some(page)),
                        // 已被新的加载或用户操作取代：不写回、不提示。
                        AppEvent::DataPageLoadDiscarded { .. } => Ok(None),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: tr("app.data.error.load").to_string(),
                            message: tr("app.data.error.no_load_result").to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    let still_current = this
                        ._data_filter_apply_generations
                        .get(&tab_id.0)
                        .is_some_and(|current| *current == generation);
                    if !still_current {
                        // 已被更新的用户操作或 AI 接管：不清理新任务，也不写旧结果。
                        cx.notify();
                        return;
                    }
                    this.data_filter_applying_tabs.remove(&tab_id);
                    this._data_filter_apply_tasks.remove(&tab_id.0);
                    this._data_filter_apply_generations.remove(&tab_id.0);
                    let result = match result {
                        Ok(Some(page)) => Ok(page),
                        Ok(None) => {
                            cx.notify();
                            return;
                        }
                        Err(error) => Err(error),
                    };
                    let tab_still_exists = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .any(|tab| tab.id == tab_id);
                    if tab_still_exists {
                        let event = this
                            .controller
                            .dispatch(AppCommand::FinishDataPageLoad {
                                tab_id,
                                generation,
                                result,
                            });
                        this.apply_app_event(&event, cx);
                    }
                    cx.notify();
                });
            });
        });
        self._data_filter_apply_tasks.insert(tab_id.0, task);
    }

    fn data_sort_specs_for_tab(&self, tab_id: TabId) -> Vec<SortSpec> {
        self.data_sort_rules
            .get(&tab_id)
            .map(|rules| {
                rules
                    .iter()
                    .filter(|rule| rule.enabled)
                    .map(|rule| SortSpec {
                        field: rule.field.clone(),
                        direction: if rule.ascending {
                            SortDirection::Asc
                        } else {
                            SortDirection::Desc
                        },
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn data_filter_specs_for_tab(&self, tab_id: TabId) -> Vec<FilterSpec> {
        let mut specs = self
            .data_filter_rules
            .get(&tab_id)
            .map(|rules| data_filter_specs_from_rules(rules))
            .unwrap_or_default();
        specs.extend(
            self.local_table_filters
                .get(&tab_id)
                .into_iter()
                .flat_map(|filters| filters.iter())
                .filter(|(_, values)| !values.is_empty())
                .map(|(field, values)| FilterSpec {
                    field: field.clone(),
                    op: FilterOp::InList,
                    values: values
                        .iter()
                        .map(|value| CellValue::Text(value.clone()))
                        .collect(),
                    enabled: true,
                }),
        );
        specs
    }

    fn data_filter_expression_for_tab(
        &self,
        tab_id: TabId,
    ) -> fluxdb_core::Result<fluxdb_core::FilterExpression> {
        if self
            .data_filter_modes
            .get(&tab_id)
            .copied()
            .unwrap_or(DataFilterMode::Builder)
            == DataFilterMode::Text
        {
            let text = self
                .data_filter_texts
                .get(&tab_id)
                .map(String::as_str)
                .unwrap_or_default();
            let kind = self.data_editor_database_kind(tab_id);
            if text.trim().is_empty() {
                return Ok(fluxdb_core::FilterExpression::And(Vec::new()));
            }
            return fluxdb_core::filter_dialect_for(kind)
                .ok_or_else(|| {
                    fluxdb_core::Error::new(
                        fluxdb_core::ErrorKind::Unsupported,
                        "当前数据源不支持 SQL 条件筛选",
                    )
                })?
                .parse(text)
                .map(|parsed| parsed.expression)
                .map_err(|error| {
                    fluxdb_core::Error::new(fluxdb_core::ErrorKind::Query, error.to_string())
                });
        }
        let rules = self
            .data_filter_rules
            .get(&tab_id)
            .cloned()
            .unwrap_or_default();
        let columns = self
            .controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if tab.id == tab_id => editor.page.as_ref(),
                _ => None,
            })
            .map(|page| page.columns.clone())
            .unwrap_or_default();
        data_filter_expression_from_rules(&rules, &columns)
    }

    fn data_changes_for_tab(&self, tab_id: TabId) -> Option<DataChangeSet> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.changes.clone(),
                _ => None,
            })
    }

    fn data_table_sort_handler(&self, cx: &mut Context<Self>) -> DataTableSortHandler {
        let view = cx.entity().downgrade();
        Arc::new(move |tab_id, field, direction, cx| {
            let _ = view.update(cx, |this, cx| {
                this.apply_data_table_header_sort(tab_id, field, direction, cx);
            });
        })
    }

    fn apply_data_table_header_sort(
        &mut self,
        tab_id: TabId,
        field: String,
        direction: Option<DataTableSortDirection>,
        cx: &mut Context<Self>,
    ) {
        let action = PendingDirtyDataAction::HeaderSort {
            tab_id,
            field: field.clone(),
            direction,
        };
        if self.request_dirty_data_action(action, cx) {
            return;
        }
        self.perform_data_table_header_sort(tab_id, field, direction, cx);
    }

    fn perform_data_table_header_sort(
        &mut self,
        tab_id: TabId,
        field: String,
        direction: Option<DataTableSortDirection>,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .data_sort_draft_rules
            .get(&tab_id)
            .or_else(|| self.data_sort_rules.get(&tab_id))
            .cloned()
            .unwrap_or_default();
        let rules = data_sort_rules_after_header_sort(&current, field, direction);

        if rules.is_empty() {
            self.data_sort_draft_rules.remove(&tab_id);
            self.data_sort_rules.remove(&tab_id);
            self.data_sort_texts.remove(&tab_id);
        } else {
            self.data_sort_draft_rules.insert(tab_id, rules.clone());
            self.data_sort_rules.insert(tab_id, rules.clone());
            let db_kind = self.data_editor_database_kind(tab_id);
            self.data_sort_texts
                .insert(tab_id, data_sort_rules_text(&rules, db_kind));
        }
        self.perform_data_filter_and_sort(tab_id, cx);
    }

    fn refresh_data_table_state(&mut self, tab_id: TabId, page: &DataPage, cx: &mut Context<Self>) {
        let Some(table_state) = self.data_table_states.get(&tab_id) else {
            return;
        };
        let redis_table_width = table_state.read(cx).delegate().redis_table_width();
        let display_page = self.data_page_for_display(tab_id, page);
        let page = match display_page {
            Cow::Borrowed(page) => Arc::new(page.clone()),
            Cow::Owned(page) => Arc::new(page),
        };
        let changes = self.data_changes_for_tab(tab_id);
        let column_choices = self.column_choices_for_tab(tab_id);
        let mut delegate = DataPageTableDelegate::from_page_with_rule(
            cx.entity().downgrade(),
            tab_id,
            page,
            self.data_sort_rules.get(&tab_id).map(Vec::as_slice),
            self.visible_table_fields.get(&tab_id),
            &column_choices,
            changes.as_ref(),
            self.data_search_queries.get(&tab_id).map(String::as_str),
            self.data_search_active_matches.get(&tab_id).copied(),
            self.data_search_highlight_all_tabs.contains(&tab_id),
            self.table_info_highlighted_column(tab_id),
            None,
            self.data_cell_edit_input.clone(),
            self.data_cell_editing.clone(),
            self.temporal_part_input.clone(),
            self.temporal_part_editing,
            true,
            redis_table_width,
            self.data_table_sort_handler(cx),
        );
        self.apply_foreign_key_targets_to_delegate(tab_id, &mut delegate);
        if !delegate.redis_page {
            self.apply_data_table_column_widths(&mut delegate);
        }
        table_state.update(cx, |table, cx| {
            let old = table.delegate();
            let old_selected_row = old.selected_row;
            let old_selected_cell = old.selected_cell;
            // 携带选中状态重建委托时，若沿用「原始行下标」会在数据刷新后落到错误的键上
            // （例如 WRONGTYPE 恢复触发的 key 列表刷新把某行替换/重排成别的键）。
            // 对 Redis 键列表按「键名」重映射选中行，保证详情面板始终跟随用户选中的那个键；
            // 找不到同名键（已被删除）则清空选中，避免详情卡在错误的键。
            let (selected_row, selected_cell, selected_cells, selected_rows, selection_anchor) =
                if delegate.redis_page {
                    let selected_key = selected_redis_key(old, old_selected_row, old_selected_cell);
                    match selected_key
                        .as_deref()
                        .and_then(|key| redis_delegate_row_for_key(&delegate, key))
                    {
                        Some(new_visible_row) => (
                            Some(new_visible_row),
                            old_selected_cell.map(|(_, col)| (new_visible_row, col)),
                            old_selected_cell
                                .map(|(_, col)| BTreeSet::from([(new_visible_row, col)]))
                                .unwrap_or_default(),
                            BTreeSet::from([new_visible_row]),
                            Some(DataTableSelectionAnchor::Row {
                                row: new_visible_row,
                            }),
                        ),
                        // 同名的键不在了（被删除或列表已重排），清空选中，避免详情卡在错误的键上。
                        None => (
                            None,
                            None,
                            BTreeSet::new(),
                            BTreeSet::new(),
                            None,
                        ),
                    }
                } else {
                    (
                        old_selected_row,
                        old_selected_cell,
                        old.selected_cells.clone(),
                        old.selected_rows.clone(),
                        old.selection_anchor,
                    )
                };
            delegate.selected_row = selected_row;
            delegate.selected_cell = selected_cell;
            delegate.selected_cells = selected_cells;
            delegate.selected_rows = selected_rows;
            delegate.selection_anchor = selection_anchor;
            delegate.hovered_cell = old.hovered_cell;
            *table.delegate_mut() = delegate;
            table.refresh(cx);
        });
    }

}

/// 取旧委托当前选中行的键名，用于重映射。Redis 键列表第一列即「键」。
fn selected_redis_key(
    old: &DataPageTableDelegate,
    selected_row: Option<usize>,
    selected_cell: Option<(usize, usize)>,
) -> Option<String> {
    selected_row
        .or_else(|| selected_cell.map(|(row, _)| row))
        .and_then(|visible_row| old.page.rows.get(visible_row))
        .and_then(|row| row.values.first())
        .map(|value| value.display_label())
}

/// 在新委托的可见行里按键名查找目标行下标；找不到（键已被删除/列表刷新）返回 None。
fn redis_delegate_row_for_key(
    delegate: &DataPageTableDelegate,
    key: &str,
) -> Option<usize> {
    delegate
        .page
        .rows
        .iter()
        .position(|row| {
            row.values
                .first()
                .is_some_and(|value| value.display_label() == key)
        })
}
