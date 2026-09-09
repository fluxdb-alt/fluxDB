impl NavicatMain {
    fn data_editor_has_dirty_changes(&self, tab_id: TabId) -> bool {
        self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .is_some_and(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor
                    .changes
                    .as_ref()
                    .is_some_and(|changes| !changes.is_empty()),
                TabKind::QueryEditor(editor) => editor
                        .result_editors
                        .values()
                        .any(|editor| {
                            editor
                                .changes
                                .as_ref()
                                .is_some_and(|changes| !changes.is_empty())
                        }),
                _ => false,
            })
    }

    fn data_change_preview_for_tab(&self, tab_id: TabId) -> Option<(DataPage, DataChangeSet)> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => {
                    let page = editor.page.clone()?;
                    let changes = editor.changes.clone()?;
                    (!changes.is_empty()).then_some((page, changes))
                }
                TabKind::QueryEditor(editor) => {
                    let editor = active_query_result_editor_state(editor)?;
                    let page = editor.page.clone()?;
                    let changes = editor.changes.clone()?;
                    (!changes.is_empty()).then_some((page, changes))
                }
                _ => None,
            })
    }

    fn request_apply_data_changes(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.data_change_preview_for_tab(tab_id).is_some() {
            self.pending_apply_data_changes = Some(tab_id);
            cx.notify();
        }
    }

    fn confirm_apply_data_changes(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.pending_apply_data_changes.take() else {
            return;
        };
        let query_refresh = self.active_query_result_refresh_request(tab_id);
        let is_query_editor = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .is_some_and(|tab| matches!(tab.kind, TabKind::QueryEditor(_)));
        let event = self.dispatch(
            if is_query_editor {
                AppCommand::ApplyDataChanges(tab_id)
            } else {
                AppCommand::ApplyDataChangesWithView {
                    tab_id,
                    sort: self.data_sort_specs_for_tab(tab_id),
                    filters: self.data_filter_specs_for_tab(tab_id),
                }
            },
            cx,
        );
        if !matches!(event, AppEvent::Failed(_))
            && let Some(request) = query_refresh
        {
            self.start_query_result_page_refresh(request, cx);
        }
        cx.notify();
    }

    fn active_query_result_refresh_request(
        &self,
        tab_id: TabId,
    ) -> Option<QueryResultRefreshRequest> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => {
                    let result_index = self
                        .query_output_tabs
                        .get(&tab_id)
                        .and_then(|tab| tab.result_index())
                        .unwrap_or(0);
                    let page_index = query_result_page_index(editor, result_index)?;
                    let sql = query_result_sql(editor, result_index)?;
                    let page = editor.results.get(page_index)?;
                    Some(QueryResultRefreshRequest {
                        tab_id,
                        result_index,
                        page_index,
                        sql,
                        offset: page.offset,
                        limit: page.limit,
                    })
                }
                _ => None,
            })
    }

    fn cancel_apply_data_changes(&mut self, cx: &mut Context<Self>) {
        self.pending_apply_data_changes = None;
        cx.notify();
    }

    fn toggle_data_change_sql_preview(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.data_change_sql_preview_tabs.contains(&tab_id) {
            self.data_change_sql_preview_tabs.remove(&tab_id);
        } else if self.data_change_preview_for_tab(tab_id).is_some() {
            self.data_change_sql_preview_tabs.insert(tab_id);
        }
        cx.notify();
    }

    fn request_dirty_data_action(
        &mut self,
        action: PendingDirtyDataAction,
        cx: &mut Context<Self>,
    ) -> bool {
        let tab_id = action.tab_id();
        if self.data_editor_has_dirty_changes(tab_id) {
            self.pending_dirty_data_action = Some(action);
            cx.notify();
            true
        } else {
            false
        }
    }

    fn request_data_editor_refresh(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.apply_data_filter_and_sort(tab_id, cx);
    }

    fn request_data_editor_pagination(
        &mut self,
        tab_id: TabId,
        offset: u64,
        limit: u64,
        cx: &mut Context<Self>,
    ) {
        let action = PendingDirtyDataAction::SetPagination {
            tab_id,
            offset,
            limit,
        };
        if self.request_dirty_data_action(action, cx) {
            return;
        }
        self.perform_data_editor_pagination(tab_id, offset, limit, cx);
    }

    fn sync_data_page_input_value(
        &mut self,
        offset: u64,
        limit: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page_no = data_page_number(offset, limit);
        self.data_page_input.update(cx, |input, cx| {
            input.set_value(page_no.to_string(), window, cx);
        });
    }

    fn perform_data_editor_pagination(
        &mut self,
        tab_id: TabId,
        offset: u64,
        limit: u64,
        cx: &mut Context<Self>,
    ) {
        let _ = self.controller.dispatch(AppCommand::SetDataPagePagination {
            tab_id,
            offset,
            limit,
        });
        self.request_data_editor_refresh(tab_id, cx);
    }

    fn active_data_editor_page_context(&self) -> Option<(TabId, u64, u64)> {
        self.controller.state().active_tab().and_then(|tab| {
            let TabKind::DataEditor(editor) = &tab.kind else {
                return None;
            };
            let (offset, limit) = editor
                .page
                .as_ref()
                .map(|page| (page.offset, page.limit))
                .unwrap_or((editor.pagination.offset, editor.pagination.limit));
            let limit = data_page_limit(limit);
            Some((tab.id, data_page_number(offset, limit), limit))
        })
    }

    fn apply_data_page_input(
        &mut self,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        enum PageInputTarget {
            DataEditor {
                tab_id: TabId,
                current_page: u64,
                limit: u64,
            },
            QueryResult(QueryResultRefreshRequest),
        }

        let target = self
            .active_data_editor_page_context()
            .map(|(tab_id, current_page, limit)| PageInputTarget::DataEditor {
                tab_id,
                current_page,
                limit,
            })
            .or_else(|| {
                let tab_id = self.controller.state().active_tab()?.id;
                self.active_query_result_refresh_request(tab_id)
                    .map(PageInputTarget::QueryResult)
            });
        let Some(target) = target else {
            return;
        };
        let (current_page, limit) = match &target {
            PageInputTarget::DataEditor {
                current_page,
                limit,
                ..
            } => (*current_page, *limit),
            PageInputTarget::QueryResult(request) => {
                let limit = data_page_limit(request.limit);
                (data_page_number(request.offset, limit), limit)
            }
        };
        let page_no = match value.trim().parse::<u64>() {
            Ok(page_no) if page_no > 0 => page_no,
            _ => {
                self.show_message("页码必须是正整数", AppMessageKind::Warning, cx);
                self.data_page_input.update(cx, |input, cx| {
                    input.set_value(current_page.to_string(), window, cx);
                });
                return;
            }
        };
        let page_no = data_page_supported_number(page_no);
        self.data_page_input.update(cx, |input, cx| {
            input.set_value(page_no.to_string(), window, cx);
        });
        let offset = data_page_offset_for_supported_page(page_no, limit);
        match target {
            PageInputTarget::DataEditor { tab_id, .. } => {
                self.request_data_editor_pagination(tab_id, offset, limit, cx);
            }
            PageInputTarget::QueryResult(mut request) => {
                request.offset = offset;
                request.limit = limit;
                self.start_query_result_page_refresh(request, cx);
            }
        }
    }

    fn confirm_dirty_data_action(&mut self, cx: &mut Context<Self>) {
        let Some(action) = self.pending_dirty_data_action.take() else {
            return;
        };
        let tab_id = action.tab_id();
        self.dispatch(AppCommand::DiscardDataChanges(tab_id), cx);
        self.data_change_sql_preview_tabs.remove(&tab_id);
        if self.pending_apply_data_changes == Some(tab_id) {
            self.pending_apply_data_changes = None;
        }
        self.perform_dirty_data_action(action, cx);
        cx.notify();
    }

    fn cancel_dirty_data_action(&mut self, cx: &mut Context<Self>) {
        self.pending_dirty_data_action = None;
        cx.notify();
    }

    fn perform_dirty_data_action(
        &mut self,
        action: PendingDirtyDataAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            PendingDirtyDataAction::ApplyFilterSort(tab_id) => {
                self.perform_data_filter_and_sort(tab_id, cx);
            }
            PendingDirtyDataAction::HeaderSort {
                tab_id,
                field,
                direction,
            } => self.perform_data_table_header_sort(tab_id, field, direction, cx),
            PendingDirtyDataAction::ContextFilter { menu, operator } => {
                self.perform_context_filter(&menu, operator, cx);
            }
            PendingDirtyDataAction::ContextSort { menu, ascending } => {
                self.perform_context_sort(&menu, ascending, cx);
            }
            PendingDirtyDataAction::RemoveContextFilter(menu) => {
                self.perform_remove_context_filter(&menu, cx);
            }
            PendingDirtyDataAction::RemoveContextSort(menu) => {
                self.perform_remove_context_sort(&menu, cx);
            }
            PendingDirtyDataAction::ClearFilterSort(tab_id) => {
                self.clear_data_filter_and_sort_rules(tab_id, cx);
                self.perform_data_filter_and_sort(tab_id, cx);
            }
            PendingDirtyDataAction::SetPagination {
                tab_id,
                offset,
                limit,
            } => self.perform_data_editor_pagination(tab_id, offset, limit, cx),
        }
    }

    fn apply_context_filter(
        &mut self,
        menu: &DataCellContextMenu,
        operator: DataFilterOperator,
        cx: &mut Context<Self>,
    ) {
        let action = PendingDirtyDataAction::ContextFilter {
            menu: menu.clone(),
            operator,
        };
        if self.request_dirty_data_action(action, cx) {
            self.data_cell_context_menu = None;
            return;
        }
        self.perform_context_filter(menu, operator, cx);
    }

    fn perform_context_filter(
        &mut self,
        menu: &DataCellContextMenu,
        operator: DataFilterOperator,
        cx: &mut Context<Self>,
    ) {
        self.data_filter_modes
            .insert(menu.tab_id, DataFilterMode::Builder);
        self.data_filter_draft_rules.insert(
            menu.tab_id,
            vec![DataFilterRule {
                enabled: true,
                field: Some(menu.column_name.clone()),
                operator,
                values: BTreeSet::from([menu.value.clone()]),
                grouped: false,
            }],
        );
        self.data_cell_context_menu = None;
        self.perform_data_filter_and_sort(menu.tab_id, cx);
    }

    fn apply_context_sort(
        &mut self,
        menu: &DataCellContextMenu,
        ascending: bool,
        cx: &mut Context<Self>,
    ) {
        let action = PendingDirtyDataAction::ContextSort {
            menu: menu.clone(),
            ascending,
        };
        if self.request_dirty_data_action(action, cx) {
            self.data_cell_context_menu = None;
            return;
        }
        self.perform_context_sort(menu, ascending, cx);
    }

    fn perform_context_sort(
        &mut self,
        menu: &DataCellContextMenu,
        ascending: bool,
        cx: &mut Context<Self>,
    ) {
        self.data_filter_modes
            .insert(menu.tab_id, DataFilterMode::Builder);
        self.data_sort_draft_rules.insert(
            menu.tab_id,
            vec![DataSortRule {
                enabled: true,
                field: menu.column_name.clone(),
                ascending,
            }],
        );
        self.data_cell_context_menu = None;
        self.perform_data_filter_and_sort(menu.tab_id, cx);
    }

    fn remove_context_filter(&mut self, menu: &DataCellContextMenu, cx: &mut Context<Self>) {
        let action = PendingDirtyDataAction::RemoveContextFilter(menu.clone());
        if self.request_dirty_data_action(action, cx) {
            self.data_cell_context_menu = None;
            return;
        }
        self.perform_remove_context_filter(menu, cx);
    }

    fn perform_remove_context_filter(
        &mut self,
        menu: &DataCellContextMenu,
        cx: &mut Context<Self>,
    ) {
        for rules in [
            self.data_filter_draft_rules.get_mut(&menu.tab_id),
            self.data_filter_rules.get_mut(&menu.tab_id),
        ]
        .into_iter()
        .flatten()
        {
            rules.retain(|rule| rule.field.as_deref() != Some(menu.column_name.as_str()));
        }
        self.data_cell_context_menu = None;
        self.perform_data_filter_and_sort(menu.tab_id, cx);
    }

    fn remove_context_sort(&mut self, menu: &DataCellContextMenu, cx: &mut Context<Self>) {
        let action = PendingDirtyDataAction::RemoveContextSort(menu.clone());
        if self.request_dirty_data_action(action, cx) {
            self.data_cell_context_menu = None;
            return;
        }
        self.perform_remove_context_sort(menu, cx);
    }

    fn perform_remove_context_sort(&mut self, menu: &DataCellContextMenu, cx: &mut Context<Self>) {
        for rules in [
            self.data_sort_draft_rules.get_mut(&menu.tab_id),
            self.data_sort_rules.get_mut(&menu.tab_id),
        ]
        .into_iter()
        .flatten()
        {
            rules.retain(|rule| rule.field.as_str() != menu.column_name.as_str());
        }
        self.data_cell_context_menu = None;
        self.perform_data_filter_and_sort(menu.tab_id, cx);
    }

}
