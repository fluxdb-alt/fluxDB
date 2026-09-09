impl NavicatMain {
    // ------------------------------------------------------------ 查询编辑器纯逻辑辅助
    //
    // 通用编辑器不感知 SQL 语义，以下方法基于编辑器文本 / 选区 + 适配层语句切分，
    // 提供 statement / selection 语义，供执行路径与工具栏使用。

    /// 取查询页当前文本。
    fn query_editor_text(&self, tab_id: TabId, cx: &Context<Self>) -> String {
        self.query_editors
            .get(&tab_id)
            .map(|editor| editor.read(cx).text())
            .unwrap_or_default()
    }

    /// 取当前选区字节区间（空选区为 `cursor..cursor`）。
    fn query_selection_range(&self, tab_id: TabId, cx: &Context<Self>) -> std::ops::Range<usize> {
        self.query_editors
            .get(&tab_id)
            .map(|editor| {
                let range = editor.read(cx).selection_range();
                range.start..range.end
            })
            .unwrap_or(0..0)
    }

    /// 取当前全部语句列表（带稳定 id）。
    fn query_statement_runs(&self, tab_id: TabId, cx: &Context<Self>) -> Vec<SqlStatementRun> {
        let text = self.query_editor_text(tab_id, cx);
        sql_editor_adapter::build_statement_runs(&text)
    }

    /// 语句执行状态映射：写入某条语句状态（语句被编辑后 id 变化自然衰减）。
    ///
    /// 写入与 SqlAdapter 共享的状态仓库，并通知该 tab 的编辑器重绘，使行背景装饰
    /// 即时反映运行结果。仓库为空（无该 tab 的仓库）时仅记日志，不 panic。
    fn set_statement_status(
        &mut self,
        tab_id: TabId,
        id: SqlStatementId,
        status: SqlStatementStatus,
        cx: &mut Context<Self>,
    ) {
        if let Some(store) = self.query_statement_statuses.get(&tab_id) {
            if let Ok(mut map) = store.lock() {
                map.set_id_status(id, status);
            }
        }
        if let Some(editor) = self.query_editors.get(&tab_id) {
            let editor = editor.clone();
            editor.update(cx, |_, cx| cx.notify());
        }
    }

    /// 选中的 SQL 文本（非空选区且 trim 后非空）。
    fn selected_sql_text(&self, tab_id: TabId, cx: &Context<Self>) -> Option<String> {
        let text = self.query_editor_text(tab_id, cx);
        let range = self.query_selection_range(tab_id, cx);
        if range.start >= range.end {
            return None;
        }
        let selected = text.get(range.clone())?.trim().to_string();
        (!selected.is_empty()).then_some(selected)
    }

    /// 选区恰好完整命中某条语句（trim 语义对齐旧实现）时返回该语句。
    fn selected_complete_statement(&self, tab_id: TabId, cx: &Context<Self>) -> Option<SqlStatementRun> {
        let text = self.query_editor_text(tab_id, cx);
        let range = self.query_selection_range(tab_id, cx);
        let trimmed = trim_range(&text, range);
        sql_editor_adapter::build_statement_runs(&text)
            .into_iter()
            .find(|statement| trim_range(&text, statement.range.clone()).as_ref() == trimmed.as_ref())
    }

    /// 光标所在语句。
    fn current_statement(&self, tab_id: TabId, cx: &Context<Self>) -> Option<SqlStatementRun> {
        let text = self.query_editor_text(tab_id, cx);
        let cursor = self
            .query_editors
            .get(&tab_id)
            .map(|editor| editor.read(cx).cursor_offset())
            .unwrap_or(0)
            .min(text.len());
        sql_editor_adapter::build_statement_runs(&text)
            .into_iter()
            .find(|statement| statement.range.start <= cursor && cursor <= statement.range.end)
    }

    /// 当前选中语句的 EXPLAIN 版本（EXPLAIN / SELECT / WITH 可解释）。
    fn selected_explain_sql_text(&self, tab_id: TabId, cx: &Context<Self>) -> Option<String> {
        let statement = self.selected_complete_statement(tab_id, cx)?;
        sql_editor_adapter::explain_sql_text(&statement.text)
    }

    /// 编辑器触发 Execute 事件时的统一入口：按区间还原语句执行，否则按文本执行。
    /// `mode` 表达用户本次操作的真实意图（Run/Select/Explain），不得丢弃：
    /// Explain 路由到 EXPLAIN 包装路径，Run/Select 走语句/文本执行（整改 6.1/6.2 模式保真）。
    fn start_query_execute_event(
        &mut self,
        tab_id: TabId,
        editor: Entity<editor_component::Editor>,
        range: fluxdb_editor_core::Range,
        mode: fluxdb_editor_core::ExecuteMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self._query_execute_tasks.contains_key(&tab_id.0) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }
        // 按 range 还原成语句（用于状态追踪）；匹配不到则按该区间文本执行。
        // 事件不再携带全文，文本在此按需从编辑器读取。
        let full = editor.read(cx).text();
        let statement = sql_editor_adapter::build_statement_runs(&full)
            .into_iter()
            .find(|statement| {
                statement.range.start == range.start && statement.range.end == range.end
            });

        // Explain 模式（cmd-shift-e）：把本次执行单元包装为 EXPLAIN 语句后按文本执行，
        // 与 Run/Select 走不同路径；不可解释的语句给出提示而非静默忽略。
        if mode == fluxdb_editor_core::ExecuteMode::Explain {
            let source = match statement {
                Some(statement) => statement.text,
                None => editor
                    .read(cx)
                    .text_in_range(fluxdb_editor_core::Range::new(range.start, range.end)),
            };
            match sql_editor_adapter::explain_sql_text(&source) {
                Some(explain) => {
                    self.start_query_text_execution(tab_id, explain, window, cx);
                }
                None => {
                    self.show_message("当前语句不支持 EXPLAIN", AppMessageKind::Warning, cx);
                }
            }
            return;
        }

        // Run / Select：按语句或区间文本执行（Select 为只读语义，与 Run 共享执行流程）。
        if let Some(statement) = statement {
            self.start_query_statement_execution(tab_id, editor, statement, window, cx);
        } else {
            let text = editor
                .read(cx)
                .text_in_range(fluxdb_editor_core::Range::new(range.start, range.end));
            self.start_query_text_execution(tab_id, text, window, cx);
        }
    }
}

/// 把选区/语句字节区间 trim 到首尾非空白（与旧实现语义一致）。
fn trim_range(text: &str, range: std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    while start < end {
        let ch = text.get(start..)?.chars().next()?;
        if !ch.is_whitespace() {
            break;
        }
        start += ch.len_utf8();
    }
    while end > start {
        let ch = text.get(..end)?.chars().next_back()?;
        if !ch.is_whitespace() {
            break;
        }
        end -= ch.len_utf8();
    }
    (start < end).then_some(start..end)
}

// 注：`explain_sql_text` / `first_sql_keyword` 已随旧 sql_editor 模块移除，迁移至
// `sql_editor_adapter`（statements.rs），本文件通过 `sql_editor_adapter::explain_sql_text` 引用。

impl NavicatMain {
    fn start_query_statement_execution(
        &mut self,
        tab_id: TabId,
        sql_editor: Entity<editor_component::Editor>,
        statement: SqlStatementRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self._query_execute_tasks.contains_key(&tab_id.0) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }

        if self.prepare_query_parameter_prompt(
            tab_id,
            &statement.text,
            PendingQueryExecution::Statement {
                statement: statement.clone(),
            },
            window,
            cx,
        ) {
            return;
        }
        if self.request_dangerous_query_confirmation(
            tab_id,
            &statement.text,
            PendingQueryExecution::Statement {
                statement: statement.clone(),
            },
            cx,
        ) {
            return;
        }

        self.start_query_statement_execution_resolved(
            tab_id,
            sql_editor,
            statement.clone(),
            statement.text.clone(),
            cx,
        );
    }

    fn start_query_statement_execution_resolved(
        &mut self,
        tab_id: TabId,
        _sql_editor: Entity<editor_component::Editor>,
        statement: SqlStatementRun,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if self._query_execute_tasks.contains_key(&tab_id.0) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }

        self.set_statement_status(tab_id, statement.id, SqlStatementStatus::Running, cx);
        self.dispatch(AppCommand::StartQueryExecution(tab_id), cx);
        let mut controller = self.controller.clone();
        let statement_id = statement.id;
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::ExecuteQueryText { tab_id, text }) {
                        AppEvent::QueryFinished(_, execution) => Ok(execution),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "执行失败".to_string(),
                            message: "SQL 执行没有返回结果".to_string(),
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
                    this._query_execute_tasks.remove(&tab_id.0);
                    let status = if result
                        .as_ref()
                        .is_ok_and(|execution| execution.summaries.iter().all(|summary| summary.success))
                    {
                        SqlStatementStatus::Success
                    } else {
                        SqlStatementStatus::Failure
                    };
                    this.set_statement_status(tab_id, statement_id, status, cx);
                    let tab_still_exists = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .any(|tab| tab.id == tab_id);
                    if tab_still_exists {
                        this.dispatch(
                            AppCommand::FinishQueryExecution {
                                tab_id,
                                result,
                            },
                            cx,
                        );
                    }
                });
            });
        });
        self._query_execute_tasks.insert(tab_id.0, task);
    }

    fn start_query_text_execution(
        &mut self,
        tab_id: TabId,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self._query_execute_tasks.contains_key(&tab_id.0) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }

        if self.prepare_query_parameter_prompt(
            tab_id,
            &text,
            PendingQueryExecution::Text,
            window,
            cx,
        ) {
            return;
        }
        if self.request_dangerous_query_confirmation(tab_id, &text, PendingQueryExecution::Text, cx)
        {
            return;
        }

        self.start_query_text_execution_resolved(tab_id, text, cx);
    }

    fn start_query_text_execution_resolved(
        &mut self,
        tab_id: TabId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if self._query_execute_tasks.contains_key(&tab_id.0) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }

        self.dispatch(AppCommand::StartQueryExecution(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::ExecuteQueryText { tab_id, text }) {
                        AppEvent::QueryFinished(_, execution) => Ok(execution),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "执行失败".to_string(),
                            message: "SQL 执行没有返回结果".to_string(),
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
                    this._query_execute_tasks.remove(&tab_id.0);
                    let tab_still_exists = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .any(|tab| tab.id == tab_id);
                    if tab_still_exists {
                        this.dispatch(AppCommand::FinishQueryExecution { tab_id, result }, cx);
                    }
                });
            });
        });
        self._query_execute_tasks.insert(tab_id.0, task);
    }

    fn start_query_result_page_refresh(
        &mut self,
        request: QueryResultRefreshRequest,
        cx: &mut Context<Self>,
    ) {
        if self._query_execute_tasks.contains_key(&request.tab_id.0) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }

        self.dispatch(AppCommand::StartQueryExecution(request.tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn({
                    let sql = request.sql.clone();
                    let options = QueryExecutionOptions {
                        page_offset: request.offset,
                        page_size: request.limit,
                        ..QueryExecutionOptions::default()
                    };
                    async move {
                        match controller.dispatch(AppCommand::ExecuteQueryTextWithOptions {
                            tab_id: request.tab_id,
                            text: sql,
                            options,
                        }) {
                            AppEvent::QueryFinished(_, execution) => Ok(execution),
                            AppEvent::Failed(error) => Err(error),
                            _ => Err(fluxdb_core::UserFacingError {
                                title: "刷新失败".to_string(),
                                message: "SQL 执行没有返回结果".to_string(),
                                detail: None,
                                retryable: true,
                            }),
                        }
                    }
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._query_execute_tasks.remove(&request.tab_id.0);
                    let tab_still_exists = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .any(|tab| tab.id == request.tab_id);
                    if tab_still_exists {
                        this.dispatch(
                            AppCommand::FinishQueryResultPageRefresh {
                                tab_id: request.tab_id,
                                result_index: request.result_index,
                                page_index: request.page_index,
                                result,
                            },
                            cx,
                        );
                    }
                });
            });
        });
        self._query_execute_tasks.insert(request.tab_id.0, task);
    }

    fn start_query_selected_execution(
        &mut self,
        tab_id: TabId,
        sql_editor: Entity<editor_component::Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(statement) = self.selected_complete_statement(tab_id, cx) {
            self.start_query_statement_execution(tab_id, sql_editor, statement, window, cx);
            return true;
        }

        if let Some(text) = self.selected_sql_text(tab_id, cx) {
            self.start_query_text_execution(tab_id, text, window, cx);
            return true;
        }

        if let Some(statement) = self.current_statement(tab_id, cx) {
            self.start_query_statement_execution(tab_id, sql_editor, statement, window, cx);
            return true;
        }

        false
    }

    fn start_query_explain_execution(
        &mut self,
        tab_id: TabId,
        sql_editor: Entity<editor_component::Editor>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let _ = &sql_editor;
        let Some(text) = self.selected_explain_sql_text(tab_id, cx) else {
            return false;
        };

        self.start_query_text_execution(tab_id, text, _window, cx);
        true
    }

    fn start_query_execution(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if self._query_execute_tasks.contains_key(&tab_id.0) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }

        if let Some(sql_editor) = self.query_editors.get(&tab_id).cloned()
            && self.start_query_selected_execution(tab_id, sql_editor, window, cx)
        {
            return;
        }

        let statements = self.query_statement_runs(tab_id, cx);

        let text = self.query_editor_text(tab_id, cx);
        if self.prepare_query_parameter_prompt(
            tab_id,
            &text,
            PendingQueryExecution::All {
                statements: statements.clone(),
            },
            window,
            cx,
        ) {
            return;
        }
        if self.request_dangerous_query_confirmation(
            tab_id,
            &text,
            PendingQueryExecution::All {
                statements: statements.clone(),
            },
            cx,
        ) {
            return;
        }

        self.start_query_all_execution_resolved(tab_id, statements, None, cx);
    }

    fn start_query_all_execution_resolved(
        &mut self,
        tab_id: TabId,
        statements: Vec<SqlStatementRun>,
        text: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self._query_execute_tasks.contains_key(&tab_id.0) {
            self.show_message("SQL 正在执行", AppMessageKind::Warning, cx);
            return;
        }

        for statement in &statements {
            self.set_statement_status(tab_id, statement.id, SqlStatementStatus::Running, cx);
        }

        self.dispatch(AppCommand::StartQueryExecution(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    let event = if let Some(text) = text {
                        controller.dispatch(AppCommand::ExecuteQueryText { tab_id, text })
                    } else {
                        controller.dispatch(AppCommand::ExecuteQuery(tab_id))
                    };
                    match event {
                        AppEvent::QueryFinished(_, execution) => Ok(execution),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "执行失败".to_string(),
                            message: "SQL 执行没有返回结果".to_string(),
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
                    this._query_execute_tasks.remove(&tab_id.0);
                    {
                        let statuses = match &result {
                            Ok(execution) => statements
                                .iter()
                                .enumerate()
                                .map(|(index, statement)| {
                                    let status = if execution
                                        .summaries
                                        .get(index)
                                        .is_some_and(|summary| summary.success)
                                    {
                                        SqlStatementStatus::Success
                                    } else {
                                        SqlStatementStatus::Failure
                                    };
                                    (statement.id, status)
                                })
                                .collect::<Vec<_>>(),
                            Err(_) => statements
                                .iter()
                                .map(|statement| (statement.id, SqlStatementStatus::Failure))
                                .collect::<Vec<_>>(),
                        };
                        for (statement_id, status) in statuses {
                            this.set_statement_status(tab_id, statement_id, status, cx);
                        }
                    }
                    let tab_still_exists = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .any(|tab| tab.id == tab_id);
                    if tab_still_exists {
                        this.dispatch(
                            AppCommand::FinishQueryExecution { tab_id, result },
                            cx,
                        );
                    }
                });
            });
        });
        self._query_execute_tasks.insert(tab_id.0, task);
    }

    fn start_data_page_load_if_needed(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self._data_load_tasks.contains_key(&tab_id.0) {
            return;
        }

        let Some(tab) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .cloned()
        else {
            return;
        };

        let TabKind::DataEditor(editor) = tab.kind else {
            return;
        };
        if !editor.loading || editor.page.is_some() {
            return;
        }

        let mut controller = self.controller.clone();
        let sort = self.data_sort_specs_for_tab(tab_id);
        let filters = self.data_filter_specs_for_tab(tab_id);
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadDataPageWithSort {
                        tab_id,
                        sort,
                        filters,
                    }) {
                        AppEvent::DataLoaded(_, page) => Ok(page),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载失败".to_string(),
                            message: "数据加载没有返回结果".to_string(),
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
                    this._data_load_tasks.remove(&tab_id.0);
                    let tab_still_exists = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .any(|tab| tab.id == tab_id);
                    if tab_still_exists {
                        let event = this
                            .controller
                            .dispatch(AppCommand::FinishDataPageLoad { tab_id, result });
                        this.apply_app_event(&event, cx);
                    }
                    cx.notify();
                });
            });
        });
        self._data_load_tasks.insert(tab_id.0, task);
    }

    fn start_table_info_load_if_needed(
        &mut self,
        tab_id: TabId,
        info_tab: TableInfoTab,
        cx: &mut Context<Self>,
    ) {
        if info_tab == TableInfoTab::Columns {
            return;
        }
        let key = (tab_id, info_tab);
        if self._table_info_tasks.contains_key(&key) {
            return;
        }
        if !self.table_info_tab_is_loading(tab_id, info_tab) {
            return;
        }

        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadTableInfo {
                        tab_id,
                        tab: info_tab,
                    }) {
                        AppEvent::TableInfoLoaded(_, _, result) => Ok(result),
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: "加载失败".to_string(),
                            message: "表属性加载没有返回结果".to_string(),
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
                    this._table_info_tasks.remove(&(tab_id, info_tab));
                    let tab_still_exists = this
                        .controller
                        .state()
                        .tabs
                        .iter()
                        .any(|tab| tab.id == tab_id);
                    if tab_still_exists {
                        let event = this.controller.dispatch(AppCommand::FinishTableInfoLoad {
                            tab_id,
                            tab: info_tab,
                            result,
                        });
                        this.apply_app_event(&event, cx);
                    }
                    cx.notify();
                });
            });
        });
        self._table_info_tasks.insert(key, task);
    }

    fn table_info_tab_is_loading(&self, tab_id: TabId, info_tab: TableInfoTab) -> bool {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => Some(match info_tab {
                    TableInfoTab::Columns => false,
                    TableInfoTab::Indexes => {
                        matches!(editor.table_info.indexes, LoadState::Loading)
                    }
                    TableInfoTab::ForeignKeys => {
                        matches!(editor.table_info.foreign_keys, LoadState::Loading)
                    }
                    TableInfoTab::Triggers => {
                        matches!(editor.table_info.triggers, LoadState::Loading)
                    }
                    TableInfoTab::Ddl => matches!(editor.table_info.ddl, LoadState::Loading),
                }),
                _ => None,
            })
            .unwrap_or(false)
    }

    fn table_info_highlighted_column(&self, tab_id: TabId) -> Option<String> {
        self.controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.table_info.highlighted_column.clone(),
                _ => None,
            })
    }

    fn scroll_to_data_column(&mut self, tab_id: TabId, column: &str, cx: &mut Context<Self>) {
        let Some(page) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => editor.page.as_ref(),
                _ => None,
            })
        else {
            return;
        };
        let visible_fields = self.visible_table_fields.get(&tab_id);
        let visible_columns = page
            .columns
            .iter()
            .filter(|data_column| {
                visible_fields
                    .map(|fields| fields.contains(&data_column.name))
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();
        let Some(column_ix) = visible_columns
            .iter()
            .position(|data_column| data_column.name == column)
            .map(|ix| ix + 1)
        else {
            return;
        };
        let Some(table_state) = self.data_table_states.get(&tab_id) else {
            return;
        };
        let redis_table_width = table_state.read(cx).delegate().redis_table_width();
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
            Some(column.to_string()),
            None,
            self.data_cell_edit_input.clone(),
            self.data_cell_editing.clone(),
            self.temporal_part_input.clone(),
            self.temporal_part_editing,
            true,
            redis_table_width,
            self.data_table_sort_handler(cx),
        );
        if !delegate.redis_page {
            self.apply_data_table_column_widths(&mut delegate);
        }
        table_state.update(cx, |table, cx| {
            *table.delegate_mut() = delegate;
            table.scroll_to_col(column_ix, cx);
            table.refresh(cx);
        });
    }

    fn request_dangerous_query_confirmation(
        &mut self,
        tab_id: TabId,
        text: &str,
        execution: PendingQueryExecution,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.controller.state().settings.confirm_dangerous_sql || !is_dangerous_sql(text) {
            return false;
        }
        self.pending_dangerous_query = Some(PendingDangerousQuery {
            tab_id,
            text: text.to_string(),
            execution,
        });
        cx.notify();
        true
    }

    fn cancel_dangerous_query(&mut self, cx: &mut Context<Self>) {
        self.pending_dangerous_query = None;
        cx.notify();
    }

    fn confirm_dangerous_query(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_dangerous_query.take() else {
            return;
        };
        match pending.execution {
            PendingQueryExecution::All { statements } => {
                self.start_query_all_execution_resolved(
                    pending.tab_id,
                    statements,
                    Some(pending.text),
                    cx,
                );
            }
            PendingQueryExecution::Text => {
                self.start_query_text_execution_resolved(pending.tab_id, pending.text, cx);
            }
            PendingQueryExecution::Statement { statement } => {
                let Some(sql_editor) = self.query_editors.get(&pending.tab_id).cloned() else {
                    self.show_message("查询编辑器未就绪", AppMessageKind::Warning, cx);
                    return;
                };
                self.start_query_statement_execution_resolved(
                    pending.tab_id,
                    sql_editor,
                    statement,
                    pending.text,
                    cx,
                );
            }
        }
    }

    /// Redis Workbench 执行前拦截：文本含破坏性命令且启用二次确认时，
    /// 挂起待确认目标并返回 true（由渲染层弹出确认弹框）。
    fn request_redis_dangerous_confirmation(
        &mut self,
        tab_id: TabId,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.controller.state().settings.confirm_dangerous_redis
            || !is_dangerous_redis_command(text)
        {
            return false;
        }
        self.pending_dangerous_redis_command = Some(PendingDangerousRedisCommand {
            tab_id,
            text: text.to_string(),
            execution_id: None,
        });
        cx.notify();
        true
    }

    /// 结果区某条执行记录重跑前的危险命令二次确认：确认后按记录 id 重跑该条记录。
    fn request_redis_record_rerun_confirmation(
        &mut self,
        tab_id: TabId,
        text: &str,
        execution_id: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.controller.state().settings.confirm_dangerous_redis
            || !is_dangerous_redis_command(text)
        {
            return false;
        }
        self.pending_dangerous_redis_command = Some(PendingDangerousRedisCommand {
            tab_id,
            text: text.to_string(),
            execution_id: Some(execution_id),
        });
        cx.notify();
        true
    }

    fn cancel_dangerous_redis_command(&mut self, cx: &mut Context<Self>) {
        self.pending_dangerous_redis_command = None;
        cx.notify();
    }

    /// 确认危险命令后才真正派发执行：顶部草稿走 ExecuteRedisWorkbench，
    /// 结果区记录重跑走 RerunRedisWorkbenchRecord。
    fn confirm_dangerous_redis_command(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_dangerous_redis_command.take() else {
            return;
        };
        if let Some(execution_id) = pending.execution_id {
            self.dispatch(
                AppCommand::RerunRedisWorkbenchRecord {
                    tab_id: pending.tab_id,
                    execution_id,
                },
                cx,
            );
        } else {
            self.dispatch(AppCommand::ExecuteRedisWorkbench(pending.tab_id), cx);
        }
    }
}


/// 判断一段 Redis 命令文本是否含破坏性/高危命令，需要二次确认。
///
/// 按行粗分命令，取每行首个 argv 词判段；与 Workbench 切分逻辑保持一致，
/// 均把风险词做大写归一处理。
fn is_dangerous_redis_command(text: &str) -> bool {
    text.lines().any(|line| {
        let first = line
            .split_whitespace()
            .next()
            .map(str::to_ascii_uppercase);
        matches!(
            first.as_deref(),
            Some("FLUSHDB") | Some("FLUSHALL") | Some("SHUTDOWN") | Some("DEBUG")
                | Some("SLAVEOF") | Some("REPLICAOF") | Some("CLUSTER")
                | Some("MIGRATE") | Some("CLIENT")
        )
    })
}

fn is_dangerous_sql(sql: &str) -> bool {
    sql.split(';').any(is_dangerous_sql_statement)
}

fn is_dangerous_sql_statement(statement: &str) -> bool {
    let normalized = statement
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let Some(first) = normalized.first().map(String::as_str) else {
        return false;
    };
    matches!(first, "drop" | "truncate")
        || matches!(first, "update" | "delete")
            && !normalized.iter().any(|token| token == "where")
}
