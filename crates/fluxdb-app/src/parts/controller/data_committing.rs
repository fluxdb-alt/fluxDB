impl AppController {
    fn apply_data_changes_command(
        &mut self,
        tab_id: TabId,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
    ) -> AppEvent {
        let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor) => editor.changes.clone().and_then(|changes| {
                editor.original_page.clone().map(|page| {
                    (
                        false,
                        editor.object.clone(),
                        editor.pagination,
                        changes,
                        page,
                    )
                })
            }),
            TabKind::QueryEditor(editor) => active_query_result_editor(editor).and_then(|result| {
                    result
                        .changes
                        .clone()
                        .and_then(|changes| {
                            result.original_page.clone().map(|page| {
                                (
                                    true,
                                    result.object.clone(),
                                    result.pagination,
                                    changes,
                                    page,
                                )
                            })
                        })
                }),
            _ => None,
        });

        let Some((is_query_result, object, pagination, changes, before_page)) = request else {
            return self.fail(Error::new(ErrorKind::Internal, "没有需要提交的更改"));
        };

        if is_query_result {
            return self.apply_query_result_changes_command(tab_id, &object, &changes, &before_page);
        }

        let guarded = if self.connection_config(object.connection_id).is_some_and(|c| c.kind == DatabaseKind::Postgres) {
            match postgres_changes_with_original_values(&changes, &before_page) { Ok(c) => c, Err(e) => return self.fail(e) }
        } else { changes.clone() };
        match self.apply_data_changes(&object, &guarded) {
            Ok(outcome) => match self.load_data_page(&object, pagination, &sort, &filters) {
                Ok(page) => {
                    self.record_data_change_history(&object, &before_page, &changes, &outcome);
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::DataEditor(editor) = &mut tab.kind
                    {
                        editor.page = Some(page.clone());
                        editor.original_page = Some(page.clone());
                        editor.changes = None;
                        editor.editing_cell = None;
                        editor.error = None;
                        tab.dirty = false;
                    }
                    AppEvent::DataLoaded(tab_id, page)
                }
                Err(error) => {
                    let mut user_error = UserFacingError::from(error);
                    user_error.title = "刷新失败".to_string();
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::DataEditor(editor) = &mut tab.kind
                    {
                        editor.original_page = editor.page.clone();
                        editor.changes = None;
                        editor.editing_cell = None;
                        editor.error = Some(user_error.clone());
                        tab.dirty = false;
                    }
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "保存失败".to_string();
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::DataEditor(editor) = &mut tab.kind
                {
                    editor.error = Some(user_error.clone());
                    tab.dirty = true;
                }
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn apply_query_result_changes_command(
        &mut self,
        tab_id: TabId,
        object: &ObjectPath,
        changes: &DataChangeSet,
        before_page: &DataPage,
    ) -> AppEvent {
        let guarded = if self.connection_config(object.connection_id).is_some_and(|c| c.kind == DatabaseKind::Postgres) {
            match postgres_changes_with_original_values(changes, before_page) { Ok(c) => c, Err(e) => return self.fail(e) }
        } else { changes.clone() };
        match self.apply_data_changes(object, &guarded) {
            Ok(outcome) => {
                self.record_data_change_history(object, before_page, changes, &outcome);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                    && let Some(page_index) = editor.active_result_editor
                    && let Some(result) = editor.result_editors.get_mut(&page_index)
                {
                    result.original_page = result.page.clone();
                    result.changes = None;
                    result.editing_cell = None;
                    result.error = None;
                    if let Some(page) = result.page.clone()
                        && let Some(result_page) = editor.results.get_mut(page_index)
                    {
                        *result_page = page;
                    }
                    tab.dirty = false;
                }
                AppEvent::TabActivated(tab_id)
            }
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "保存失败".to_string();
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    if let Some(result) = active_query_result_editor_mut(editor) {
                        result.error = Some(user_error.clone());
                    }
                    editor.error = Some(user_error.clone());
                    tab.dirty = true;
                }
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

}
