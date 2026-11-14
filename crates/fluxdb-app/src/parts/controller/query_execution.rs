impl AppController {
    // 不能通过递归 dispatch 补默认参数：大型命令分派帧会重复占栈，
    // 在 macOS GCD 的小栈后台线程中挤掉 SQLx 握手/收包所需空间。
    fn execute_query_text_with_options(
        &mut self,
        tab_id: TabId,
        pane_id: QueryPaneId,
        text: String,
        options: QueryExecutionOptions,
    ) -> AppEvent {
        let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::QueryEditor(editor) => {
                editor.pane(pane_id)?;
                Some(QueryRequest {
                    connection_id: editor.connection_id,
                    database: editor.database.clone(),
                    // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                    // 同标签的多屏共享这条会话，屏只隔离文本与结果。
                    session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                    schema: editor.schema.clone(),
                    text: sql_text_for_execution(
                        &text,
                        options.page_size,
                        self.connection_kind(editor.connection_id)
                            .unwrap_or(DatabaseKind::MySql),
                    ),
                    mode: fluxdb_core::QueryMode::Selection,
                    options,
                })
            }
            _ => None,
        });

        let Some(request) = request else {
            return self.fail(Error::new(ErrorKind::Internal, "查询编辑器分屏不存在"));
        };
        self.release_previous_query_result_content(tab_id, pane_id);

        let key = QueryPaneKey::new(tab_id, pane_id);
        let cancel_flag = self.query_cancel_flag(key);
        let result =
            self.execute_query_with_cancel(&request, &|| query_cancel_requested(&cancel_flag));
        self.clear_query_cancel_flag(key);
        match result {
            Ok(execution) => {
                let result_editors = query_result_editors(self, &request, &execution);
                let active_result_editor = result_editors.keys().next().copied();
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                    && let Some(pane) = editor.pane_mut(pane_id)
                {
                    pane.running = false;
                    pane.results = execution.results.clone();
                    pane.result_editors = result_editors;
                    pane.active_result_editor = active_result_editor;
                    pane.summaries = execution.summaries.clone();
                    pane.error = None;
                    tab.dirty = false;
                }
                self.record_query_execution_history(&request, &execution);
                AppEvent::QueryFinished(tab_id, pane_id, execution)
            }
            Err(error) => {
                self.record_failed_query_history(&request);
                let user_error = UserFacingError::from(error);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                    && let Some(pane) = editor.pane_mut(pane_id)
                {
                    pane.running = false;
                    pane.summaries = Vec::new();
                    pane.result_editors.clear();
                    pane.active_result_editor = None;
                    pane.error = Some(user_error.clone());
                }
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 重新执行前释放该屏上一批结果外置的大字段内容。
    ///
    /// 内容归属按「标签会话」计算（连接器侧公式），同标签各屏共用一条会话；
    /// 因此只要还有兄弟屏持有结果，就不能整批释放，否则会把兄弟屏的单元格引用一起变成失效。
    /// 代价是这些旧内容要等标签关闭才回收（`ponytail:` 有界泄漏）。要精确到屏，
    /// 需要给 `QueryRequest` 增加独立的归属维度，暂不做。
    fn release_previous_query_result_content(&self, tab_id: TabId, pane_id: QueryPaneId) {
        let has_sibling_results = self.find_tab(tab_id).is_some_and(|tab| match &tab.kind {
            TabKind::QueryEditor(editor) => editor
                .panes
                .iter()
                .any(|pane| pane.id != pane_id && !pane.results.is_empty()),
            _ => false,
        });
        if has_sibling_results {
            return;
        }
        let Some(connection_id) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::QueryEditor(editor) => Some(editor.connection_id),
            _ => None,
        }) else {
            return;
        };
        if let Some(store) = fluxdb_core::cell_content_store() {
            store.release_owner(&fluxdb_core::result_content_owner(
                Some(fluxdb_core::QuerySessionId(tab_id.0)),
                connection_id,
            ));
        }
    }
}

#[cfg(test)]
mod query_execution_stack_tests {
    use super::*;

    #[test]
    fn text_commands_preserve_options_and_publish_results() {
        for explicit in [false, true] {
            let mut controller = AppController::new();
            let config = ConnectionConfig {
                id: ConnectionId(1),
                name: "query-options-test".into(),
                kind: DatabaseKind::Sqlite,
                endpoint: fluxdb_core::Endpoint::SqliteFile {
                    path: ":memory:".into(),
                    read_only: false,
                },
                options: BTreeMap::new(),
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: None,
                mongo_profile: None,
                sqlserver_profile: None,
                oracle_profile: None,
                redis_profile: None,
            };
            controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
            controller.dispatch(AppCommand::OpenQueryEditor(ConnectionId(1)));
            let tab_id = controller.state().active_tab.unwrap();
            controller.dispatch(AppCommand::StartQueryExecution { tab_id, pane_id: QueryPaneId(1) });
            let text = "SELECT 1 AS value UNION ALL SELECT 2 UNION ALL SELECT 3".to_string();
            let command = if explicit {
                AppCommand::ExecuteQueryTextWithOptions {
                    tab_id,
                    pane_id: QueryPaneId(1),
                    text,
                    options: QueryExecutionOptions {
                        page_size: 2,
                        read_only: true,
                        ..QueryExecutionOptions::default()
                    },
                }
            } else {
                AppCommand::ExecuteQueryText { tab_id, pane_id: QueryPaneId(1), text }
            };

            let AppEvent::QueryFinished(result_tab, result_pane, execution) = controller.dispatch(command)
            else {
                panic!("in-memory SELECT must return a query result");
            };

            assert_eq!(result_tab, tab_id);
            assert_eq!(result_pane, QueryPaneId(1));
            assert_eq!(execution.results.len(), 1);
            let page = &execution.results[0];
            assert_eq!(page.columns[0].name, "value");
            let values: Vec<_> = page.rows.iter().map(|row| row.values[0].clone()).collect();
            let expected = if explicit { vec![1, 2] } else { vec![1, 2, 3] };
            assert_eq!(
                values,
                expected.into_iter().map(CellValue::I64).collect::<Vec<_>>()
            );
            if explicit {
                assert_eq!(page.limit, 2);
            }
            let TabKind::QueryEditor(editor) = &controller.state().active_tab().unwrap().kind
            else {
                panic!("query tab must remain open");
            };
            assert!(!editor.running);
            assert!(editor.error.is_none());
            assert_eq!(editor.results, execution.results);
            assert_eq!(editor.summaries, execution.summaries);
            assert_eq!(controller.state().query_history.len(), 1);
            assert!(controller.query_cancel_flag(QueryPaneKey::new(tab_id, QueryPaneId(1))).is_none());
        }
    }
}
