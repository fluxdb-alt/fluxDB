impl AppController {
    /// 提交数据改动的准备步：只置运行态，让 UI 立即显示提交中并拦截重复提交。
    /// 不做数据库 I/O——写入与提交后重载在后台线程执行。
    pub fn start_apply_data_changes_command(&mut self, tab_id: TabId) -> AppEvent {
        match self.find_tab_mut(tab_id).map(|tab| &mut tab.kind) {
            Some(TabKind::DataEditor(editor)) => {
                editor.submitting = true;
                editor.error = None;
            }
            Some(TabKind::QueryEditor(editor)) => {
                editor.running = true;
                editor.error = None;
            }
            _ => return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在")),
        }
        AppEvent::TabActivated(tab_id)
    }

    /// 后台线程执行提交：只做数据库 I/O（写入 + 提交后重载），不改 `AppState`。
    ///
    /// `AppController` 的 `state` 按克隆隔离，后台副本上的状态改动不会回流到 UI；
    /// 因此这里只返回产物，状态写回由 `FinishApplyDataChanges` 在 UI 线程完成。
    pub fn run_apply_data_changes_command(
        &mut self,
        tab_id: TabId,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
    ) -> std::result::Result<DataApplyResult, UserFacingError> {
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
            TabKind::QueryEditor(editor) => {
                active_query_result_editor(editor).and_then(|result| {
                    result.changes.clone().and_then(|changes| {
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
                })
            }
            _ => None,
        });
        let Some((query_result, object, pagination, changes, before_page)) = request else {
            return Err(UserFacingError::from(Error::new(
                ErrorKind::Internal,
                "没有需要提交的更改",
            )));
        };
        let guarded = if self
            .connection_config(object.connection_id)
            .is_some_and(|c| c.kind.is_postgres_family())
        {
            postgres_changes_with_original_values(&changes, &before_page).map_err(UserFacingError::from)?
        } else {
            changes.clone()
        };
        let outcome = self.apply_data_changes(&object, &guarded).map_err(|error| {
            let mut user_error = UserFacingError::from(error);
            user_error.title = "保存失败".to_string();
            user_error
        })?;
        // 查询结果页签由调用方另行重跑 SQL 刷新，这里不重复按对象重载。
        let page = if query_result {
            before_page.clone()
        } else {
            self.load_data_page(&object, pagination, &sort, &filters, None)
                .map_err(|error| {
                    let mut user_error = UserFacingError::from(error);
                    user_error.title = "刷新失败".to_string();
                    user_error
                })?
        };
        // 历史记录依赖提交前的页面快照；后台副本也要记录，才能在回桥时合并。
        self.record_data_change_history(&object, &before_page, &changes, &outcome);
        Ok(DataApplyResult {
            query_result,
            page,
            outcome,
        })
    }

    /// 回桥：把后台提交产物写回编辑器状态（改动集清空、页面与原值同步、dirty 复位）。
    pub fn finish_apply_data_changes_command(
        &mut self,
        tab_id: TabId,
        result: std::result::Result<DataApplyResult, UserFacingError>,
    ) -> AppEvent {
        match result {
            Ok(applied) => {
                if applied.query_result {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                        && let Some(page_index) = editor.active_result_editor
                        && let Some(result) = editor.result_editors.get_mut(&page_index)
                    {
                        result.original_page = result.page.clone();
                        result.changes = None;
                        result.editing_cell = None;
                        result.error = None;
                        result.submitting = false;
                        if let Some(page) = result.page.clone()
                            && let Some(result_page) = editor.results.get_mut(page_index)
                        {
                            *result_page = page;
                        }
                        tab.dirty = false;
                    }
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        editor.running = false;
                    }
                    AppEvent::TabActivated(tab_id)
                } else if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::DataEditor(editor) = &mut tab.kind
                {
                    editor.page = Some(applied.page.clone());
                    editor.original_page = Some(applied.page.clone());
                    editor.changes = None;
                    editor.editing_cell = None;
                    editor.submitting = false;
                    editor.error = None;
                    tab.dirty = false;
                    AppEvent::DataLoaded(tab_id, applied.page)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"))
                }
            }
            Err(user_error) => {
                match self.find_tab_mut(tab_id).map(|tab| &mut tab.kind) {
                    Some(TabKind::DataEditor(editor)) => {
                        editor.submitting = false;
                        editor.error = Some(user_error.clone());
                    }
                    Some(TabKind::QueryEditor(editor)) => {
                        editor.running = false;
                        editor.error = Some(user_error.clone());
                        if let Some(result) = active_query_result_editor_mut(editor) {
                            result.submitting = false;
                            result.error = Some(user_error.clone());
                        }
                    }
                    _ => {}
                }
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }
}
