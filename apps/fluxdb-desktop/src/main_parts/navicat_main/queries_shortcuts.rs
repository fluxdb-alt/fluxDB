impl NavicatMain {
    fn query_editor_database_kind(&self, tab_id: TabId) -> DatabaseKind {
        let connection_id = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => Some(editor.connection_id),
                _ => None,
            });
        connection_id
            .and_then(|connection_id| {
                self.controller
                    .state()
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == connection_id)
                    .map(|connection| connection.config.kind)
            })
            .unwrap_or(DatabaseKind::MySql)
    }

    fn format_query_editor_sql(
        &mut self,
        tab_id: TabId,
        sql_editor: Entity<editor_component::Editor>,
        cx: &mut Context<Self>,
    ) -> bool {
        // 新编辑器：优先格式化选区文本，否则格式化整篇，仅替换对应字节区间。
        let selected = sql_editor.read(cx).selected_text();
        let (text, range) = match selected {
            Some(sel) if !sel.trim().is_empty() => {
                let r = sql_editor.read(cx).selection_range();
                (sel, fluxdb_editor_core::Range::new(r.start, r.end))
            }
            _ => {
                let whole = sql_editor.read(cx).text();
                let len = whole.len();
                (whole, fluxdb_editor_core::Range::new(0, len))
            }
        };
        if text.trim().is_empty() {
            return false;
        }
        let formatted = format_sql_text_for_dialect(&text, self.query_editor_database_kind(tab_id));
        sql_editor.update(cx, |editor, editor_cx| {
            editor.replace_text_range(range, &formatted, editor_cx);
        });
        true
    }

    fn compress_query_editor_sql(
        &mut self,
        _tab_id: TabId,
        sql_editor: Entity<editor_component::Editor>,
        cx: &mut Context<Self>,
    ) -> bool {
        // 新编辑器：优先压缩选区文本，否则压缩整篇，仅替换对应字节区间。
        let selected = sql_editor.read(cx).selected_text();
        let (text, range) = match selected {
            Some(sel) if !sel.trim().is_empty() => {
                let r = sql_editor.read(cx).selection_range();
                (sel, fluxdb_editor_core::Range::new(r.start, r.end))
            }
            _ => {
                let whole = sql_editor.read(cx).text();
                let len = whole.len();
                (whole, fluxdb_editor_core::Range::new(0, len))
            }
        };
        if text.trim().is_empty() {
            return false;
        }
        let compressed = compress_sql_text(&text);
        sql_editor.update(cx, |editor, editor_cx| {
            editor.replace_text_range(range, &compressed, editor_cx);
        });
        true
    }

    fn first_connection_id(&self) -> Option<fluxdb_core::ConnectionId> {
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.connected)
            .or_else(|| self.controller.state().connections.first())
            .map(|connection| connection.config.id)
    }

    fn current_query_scope(&self) -> Option<(ConnectionId, Option<String>)> {
        if let Some(scope) = self
            .controller
            .state()
            .active_tab()
            .and_then(tab_workspace_scope)
        {
            return Some((scope.connection_id, Some(scope.database)));
        }

        let mut connections = self.controller.state().connections.iter();
        let connection = connections.next()?;
        if connections.next().is_some() {
            return None;
        }
        let connection_id = connection.config.id;
        Some({
            let database = self
                .controller
                .state()
                .connections
                .iter()
                .find(|connection| connection.config.id == connection_id)
                .and_then(|connection| connection_default_database(&connection.config));
            (connection_id, database)
        })
    }

    fn open_new_query(&mut self, scope: Option<WorkspaceScope>, cx: &mut Context<Self>) {
        let target = scope
            .map(|scope| (scope.connection_id, Some(scope.database)))
            .or_else(|| self.current_query_scope());
        if let Some((connection_id, database)) = target {
            self.open_query_for_context(connection_id, database, cx);
            return;
        }

        self.pending_new_query_connection = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.connected)
            .or_else(|| self.controller.state().connections.first())
            .map(|connection| connection.config.id);
        if self.pending_new_query_connection.is_none() {
            self.show_message("请先创建连接", AppMessageKind::Warning, cx);
        }
        cx.notify();
    }

    /// 根据目标连接的数据库类型分发「新建查询」：
    /// Redis 连接打开独立 Workbench 面板，其余数据库走 SQL 查询编辑器。
    fn open_query_for_context(
        &mut self,
        connection_id: ConnectionId,
        database: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let is_redis = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| connection.config.kind == DatabaseKind::Redis)
            .unwrap_or(false);

        if is_redis {
            // Redis 的数据库即 DB 编号；解析失败或缺失时默认 0 号库。
            let database = database
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0);
            self.dispatch(
                AppCommand::OpenRedisWorkbench {
                    connection_id,
                    database,
                },
                cx,
            );
            return;
        }

        self.dispatch(
            AppCommand::OpenQueryEditorInDatabase {
                connection_id,
                database,
            },
            cx,
        );
    }

    fn toggle_query_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query_history_open = !self.query_history_open;
        if self.query_history_open {
            self.query_history_quick_open = false;
            let (connection, database, table) = self.active_query_history_filters(cx);
            self.query_history_connection_filter = connection;
            self.query_history_database_filter = database;
            self.query_history_table_filter = table;
            self.query_history_kind_filter = QueryHistoryKindFilter::All;
            self.query_history_detail = None;
            self.refresh_query_history_filter_selects(window, cx);
        }
        cx.notify();
    }

    /// 顶部「历史」入口：按当前激活标签所属数据库类型路由。
    ///
    /// - Redis Workbench：打开 Redis 历史抽屉（按连接 + 逻辑库作用域），无记录时提示。
    /// - 其余（SQL 查询编辑器等）：走原有 SQL 历史逻辑，保持既有行为不回归。
    fn toggle_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active_history = self
            .controller
            .state()
            .active_tab()
            .and_then(|tab| match &tab.kind {
                TabKind::RedisWorkbench(workbench) => {
                    Some(WorkbenchHistoryScope::Redis {
                        connection_id: workbench.connection_id,
                        database: workbench.database,
                    })
                }
                _ => None,
            });
        let Some(scope) = active_history else {
            // SQL 历史入口：保留原有守卫（无记录提示，有记录再切换抽屉）。
            if self.controller.state().query_history.is_empty() {
                self.show_message("暂无 SQL 历史", AppMessageKind::Info, cx);
            } else {
                self.toggle_query_history(window, cx);
            }
            return;
        };
        if self.controller.load_history(&scope, usize::MAX).is_empty() {
            self.show_message("暂无历史", AppMessageKind::Info, cx);
            return;
        }
        self.redis_history_open = true;
        self.redis_history_scope = Some(scope);
        // 打开抽屉时清空历史搜索框并聚焦，避免残留上次的搜索状态。
        self.redis_history_search.clear();
        self.redis_history_search_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        // 与 Redis 抽屉互斥：关闭 SQL 历史相关抽屉。
        self.query_history_open = false;
        self.query_history_quick_open = false;
        cx.notify();
    }

    /// 关闭 Redis 历史抽屉时清空搜索状态（文本 + 输入框），避免下次打开残留。
    fn close_redis_history_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.redis_history_search.clear();
        self.redis_history_search_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
    }

    /// 同步 Redis 历史抽屉的作用域：让抽屉始终跟随当前激活的 Redis Workbench
    /// （connection_id + database），避免切换 / 关闭标签后残留旧连接、旧库的数据。
    ///
    /// - 抽屉未打开：不动作（打开时 `toggle_history` 会重新锚定 scope）。
    /// - 激活标签是 Redis Workbench 且 scope 变化：重置 scope 并清空搜索，抽屉随新 scope 重展。
    /// - 激活标签不是 Redis Workbench：关闭抽屉、清空 scope，避免展示旧库历史。
    ///   此处没有 window 句柄，仅清空搜索过滤串；输入框残留文本在下次 `toggle_history`
    ///   打开时统一重置，不影响抽屉实际展示的数据。
    fn sync_redis_history_scope(&mut self, cx: &mut Context<Self>) {
        if !self.redis_history_open {
            return;
        }
        let scope = self
            .controller
            .state()
            .active_tab()
            .and_then(|tab| match &tab.kind {
                TabKind::RedisWorkbench(workbench) => Some(WorkbenchHistoryScope::Redis {
                    connection_id: workbench.connection_id,
                    database: workbench.database,
                }),
                _ => None,
            });
        let Some(scope) = scope else {
            // 激活标签已不是 Redis Workbench：关闭抽屉，清空 scope，避免展示旧连接/旧库历史。
            self.redis_history_open = false;
            self.redis_history_scope = None;
            self.redis_history_search.clear();
            cx.notify();
            return;
        };
        if self.redis_history_scope.as_ref() == Some(&scope) {
            return; // scope 未变化，无需刷新。
        }
        // scope 变化：重置到激活标签并清空搜索，抽屉重新按新 scope 展示历史。
        self.redis_history_scope = Some(scope);
        self.redis_history_search.clear();
        cx.notify();
    }

    fn open_query_history_quick_search(
        &mut self,
        _: &OpenQueryHistoryQuickSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_query_history_quick_search(window, cx);
    }

    fn show_query_history_quick_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.controller.state().query_history.is_empty() {
            self.show_message("暂无 SQL 历史", AppMessageKind::Info, cx);
            return;
        }
        self.query_history_open = false;
        self.query_history_quick_open = true;
        self.query_history_quick_search.clear();
        self.query_history_quick_selected = 0;
        self.query_history_quick_kind_filter = QueryHistoryKindFilter::All;
        self.query_history_detail = None;
        self.query_history_quick_search_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn query_history_quick_search_previous(
        &mut self,
        _: &QueryHistoryQuickSearchPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.query_history_quick_open {
            return;
        }
        let len = query_history_quick_entries(self.controller.state(), self).len();
        if len == 0 {
            self.query_history_quick_selected = 0;
        } else if self.query_history_quick_selected == 0 {
            self.query_history_quick_selected = len - 1;
        } else {
            self.query_history_quick_selected -= 1;
        }
        cx.notify();
    }

    fn query_history_quick_search_next(
        &mut self,
        _: &QueryHistoryQuickSearchNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.query_history_quick_open {
            return;
        }
        let len = query_history_quick_entries(self.controller.state(), self).len();
        if len == 0 {
            self.query_history_quick_selected = 0;
        } else {
            self.query_history_quick_selected = (self.query_history_quick_selected + 1) % len;
        }
        cx.notify();
    }

    fn query_history_quick_search_confirm(
        &mut self,
        _: &QueryHistoryQuickSearchConfirm,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_query_history_quick_search(cx);
    }

    fn confirm_query_history_quick_search(&mut self, cx: &mut Context<Self>) {
        if !self.query_history_quick_open {
            return;
        }
        let entries = query_history_quick_entries(self.controller.state(), self);
        let Some(entry) = entries
            .get(self.query_history_quick_selected.min(entries.len().saturating_sub(1)))
            .map(|item| item.entry.clone())
        else {
            return;
        };
        self.query_history_quick_open = false;
        self.open_query_history_entry(entry, cx);
    }

    fn set_query_history_quick_kind_filter(
        &mut self,
        kind: QueryHistoryKindFilter,
        cx: &mut Context<Self>,
    ) {
        self.query_history_quick_kind_filter = kind;
        self.query_history_quick_selected = 0;
        cx.notify();
    }

    fn open_query_history_entry(
        &mut self,
        entry: fluxdb_app::QueryHistoryEntry,
        cx: &mut Context<Self>,
    ) {
        self.query_history_open = false;
        let active_query = self.controller.state().active_tab().and_then(|tab| {
            matches!(tab.kind, TabKind::QueryEditor(_)).then_some((tab.id, tab.kind.clone()))
        });
        let tab_id = if let Some((tab_id, TabKind::QueryEditor(editor))) = active_query {
            let text = append_sql_history_text(&editor.text, &entry.text);
            self.dispatch(AppCommand::UpdateQueryText { tab_id, text }, cx);
            tab_id
        } else {
            let target = self
                .controller
                .state()
                .connections
                .iter()
                .any(|connection| connection.config.id == entry.connection_id)
                .then_some((entry.connection_id, entry.database.clone()))
                .or_else(|| self.current_query_scope())
                .or_else(|| self.first_connection_id().map(|connection_id| (connection_id, None)));
            let Some((connection_id, database)) = target else {
                self.show_message("请先创建连接", AppMessageKind::Warning, cx);
                return;
            };
            self.dispatch(
                AppCommand::OpenQueryEditorInDatabase {
                    connection_id,
                    database,
                },
                cx,
            );
            let Some(tab_id) = self.controller.state().active_tab else {
                return;
            };
            self.dispatch(
                AppCommand::UpdateQueryText {
                    tab_id,
                    text: entry.text,
                },
                cx,
            );
            tab_id
        };
        let _ = tab_id;
    }

    fn show_query_history_detail(&mut self, entry: QueryHistoryEntry, cx: &mut Context<Self>) {
        self.query_history_detail = Some(entry);
        cx.notify();
    }

    fn set_query_history_kind_filter(&mut self, kind: QueryHistoryKindFilter, cx: &mut Context<Self>) {
        self.query_history_kind_filter = kind;
        cx.notify();
    }

    fn active_query_history_filters(
        &self,
        cx: &mut Context<Self>,
    ) -> (Option<ConnectionId>, Option<String>, Option<String>) {
        let Some(tab) = self.controller.state().active_tab() else {
            return (None, None, None);
        };
        match &tab.kind {
            TabKind::QueryEditor(editor) => {
                let table = self
                    .query_editors
                    .get(&tab.id)
                    .and_then(|sql_editor| {
                        // 新编辑器：依据光标偏移取光标所在语句文本。
                        let text = sql_editor.read(cx).text();
                        let offset = sql_editor.read(cx).cursor_offset().min(text.len());
                        sql_editor_adapter::statement_around(&text, offset)
                            .map(|(start, end)| text[start..end].to_string())
                    })
                    .and_then(|sql| query_history_sql_tables(&sql).into_iter().next());
                (Some(editor.connection_id), editor.database.clone(), table)
            }
            TabKind::DataEditor(editor) => (
                Some(editor.object.connection_id),
                editor.object.database.clone(),
                Some(editor.object.name.clone()),
            ),
            _ => (None, None, None),
        }
    }

    fn refresh_query_history_filter_selects(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.controller.state();
        let connection_items = query_history_connection_filter_items(state);
        let connection_index =
            query_history_connection_filter_index(&connection_items, self.query_history_connection_filter);
        let database_items =
            query_history_database_filter_items(state, self.query_history_connection_filter);
        let database_index =
            query_history_text_filter_index(&database_items, self.query_history_database_filter.as_deref());
        let table_items = query_history_table_filter_items(
            state,
            self.query_history_connection_filter,
            self.query_history_database_filter.as_deref(),
        );
        let table_index =
            query_history_text_filter_index(&table_items, self.query_history_table_filter.as_deref());

        self.query_history_connection_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(connection_items), window, cx);
            select.set_selected_index(connection_index, window, cx);
        });
        self.query_history_database_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(database_items), window, cx);
            select.set_selected_index(database_index, window, cx);
        });
        self.query_history_table_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(table_items), window, cx);
            select.set_selected_index(table_index, window, cx);
        });
    }

    fn new_query(&mut self, _: &NewQuery, _: &mut Window, cx: &mut Context<Self>) {
        self.open_new_query(None, cx);
    }

    fn confirm_new_query_scope(
        &mut self,
        connection_id: ConnectionId,
        database: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.pending_new_query_connection = None;
        self.dispatch(
            AppCommand::OpenQueryEditorInDatabase {
                connection_id,
                database,
            },
            cx,
        );
    }

    fn refresh(&mut self, _: &Refresh, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_active(window, cx);
    }

    fn save_or_apply(&mut self, _: &SaveOrApply, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .controller
            .state()
            .active_tab()
            .is_some_and(|tab| matches!(tab.kind, TabKind::Settings(_)))
        {
            let settings = self.settings_editor_draft.clone();
            save_settings_from_ui(self, settings, "设置已保存", cx);
            return;
        }

        if self
            .controller
            .state()
            .active_tab()
            .is_some_and(|tab| matches!(tab.kind, TabKind::QueryEditor(_)))
        {
            self.save_active_query(window, cx);
            return;
        }

        if let Some((tab_id, true)) = self.active_data_editor_dirty() {
            self.request_apply_data_changes(tab_id, cx);
        }
    }

    fn close_current_tab(&mut self, _: &CloseCurrentTab, _: &mut Window, cx: &mut Context<Self>) {
        if self.new_connection_kind.is_some() {
            self.cancel_new_connection(cx);
            return;
        }

        if let Some(tab_id) = self.controller.state().active_tab {
            self.dispatch(AppCommand::CloseTab(tab_id), cx);
        }
    }

    fn toggle_connection_browser(
        &mut self,
        _: &ToggleConnectionBrowser,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_connection_browser = !self.show_connection_browser;
        cx.notify();
    }

    fn execute_or_apply(
        &mut self,
        _: &ExecuteOrApply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.controller.state().active_tab().cloned() else {
            return;
        };

        match tab.kind {
            TabKind::QueryEditor(editor) if !editor.text.trim().is_empty() => {
                let Some(sql_editor) = self.query_editors.get(&tab.id).cloned() else {
                    self.show_message("查询编辑器未就绪", AppMessageKind::Warning, cx);
                    return;
                };
                if !self.start_query_selected_execution(tab.id, sql_editor, window, cx) {
                    self.show_message(
                        "请先选中 SQL 或将光标放在一条 SQL 内",
                        AppMessageKind::Warning,
                        cx,
                    );
                }
            }
            TabKind::DataEditor(editor)
                if editor
                    .changes
                    .as_ref()
                    .is_some_and(|changes| !changes.is_empty()) =>
            {
                self.request_apply_data_changes(tab.id, cx);
            }
            TabKind::CreateTable(create) => {
                if let Some(command) = create_table_add_row_command(tab.id, create.active_tab) {
                    self.dispatch(command, cx);
                }
            }
            _ => {}
        }
    }

    fn copy_footer_sql_selection(
        &mut self,
        _: &CopyFooterSqlSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .data_sql_panel_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            return;
        }
        let Some(text) = self.data_sql_footer_selection.selected_text() else {
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.show_message("已复制 SQL", AppMessageKind::Success, cx);
    }

    fn copy_data_selection(
        &mut self,
        _: &CopyDataSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.copy_data_selection_should_defer_to_focused_editor(window, cx) {
            return;
        }
        let Some(tab_id) = self.controller.state().active_tab else {
            return;
        };
        self.copy_data_table_selection(tab_id, cx);
    }

    fn copy_data_selection_should_defer_to_focused_editor(
        &self,
        window: &Window,
        cx: &App,
    ) -> bool {
        let input_focused = [
            &self.rename_group_input,
            &self.sidebar_search_input,
            &self.field_filter_search_input,
            &self.data_filter_value_input,
            &self.local_filter_value_input,
            &self.local_filter_search_input,
            &self.data_filter_search_input,
            &self.data_filter_text_input,
            &self.data_sort_text_input,
            &self.data_sql_panel_input,
            &self.data_page_input,
            &self.data_cell_edit_input,
            &self.temporal_part_input,
            &self.cell_detail_input,
            &self.data_search_input,
            &self.row_detail_search_input,
            &self.tab_switcher_search_input,
            &self.query_history_quick_search_input,
            &self.display_database_search_input,
        ]
        .iter()
        .any(|input| input.read(cx).focus_handle(cx).is_focused(window));

        input_focused
            || self
                .query_editors
                .values()
                .any(|editor| editor.read(cx).focus_handle.is_focused(window))
    }

    fn delete_connection_shortcut(
        &mut self,
        _: &DeleteConnectionShortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_delete_connection.is_some() {
            self.confirm_delete_connection(cx);
            return;
        }

        if self.pending_delete_database.is_some() {
            self.confirm_delete_database(cx);
            return;
        }

        if let Some(menu) = self.connection_context_menu {
            self.request_delete_connection(menu.connection_id, window, cx);
        }
    }

    fn cancel_dialog(&mut self, _: &CancelDialog, window: &mut Window, cx: &mut Context<Self>) {
        // 备份 tab 的三个弹框（表清单/备注/删除确认）优先响应 Esc。
        if self.backup_tables_modal.take().is_some() {
            cx.notify();
            return;
        }
        if self.backup_note_modal_path.take().is_some() {
            cx.notify();
            return;
        }
        if self.pending_delete_backup.take().is_some() {
            cx.notify();
            return;
        }
        if self.query_history_quick_open {
            self.query_history_quick_open = false;
            cx.notify();
            return;
        }

        if self.query_history_open {
            self.query_history_open = false;
            cx.notify();
            return;
        }

        if self.redis_history_open {
            self.redis_history_open = false;
            self.redis_history_scope = None;
            self.close_redis_history_search(window, cx);
            cx.notify();
            return;
        }

        if self.cancel_data_cell_edit(cx) {
            return;
        }

        // 完整值内嵌面板：编辑中先取消编辑，否则关闭面板（Esc 兜底）
        if self.redis_hash_full_value_viewer.borrow().is_some() {
            if self
                .redis_hash_full_value_viewer
                .borrow()
                .as_ref()
                .is_some_and(|viewer| viewer.editing)
            {
                self.cancel_redis_hash_full_value_edit(cx);
            } else {
                self.close_redis_hash_full_value_viewer(cx);
            }
            return;
        }

        if self.query_history_detail.take().is_some() {
            cx.notify();
            return;
        }

        if let Some(tab_id) = self.active_user_admin_tab_id()
            && self
                .controller
                .state()
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(|tab| match &tab.kind {
                    TabKind::UserAdmin(admin) => admin.pending_sql.as_ref(),
                    _ => None,
                })
                .is_some()
        {
            self.dispatch(AppCommand::ClearUserAdminPendingSql(tab_id), cx);
            return;
        }

        if self.pending_delete_data_row.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_redis_key_delete.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_redis_set_member_delete.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_dirty_data_action.take().is_some() {
            cx.notify();
            return;
        }

        if let Some(tab_id) = self.controller.state().pending_dirty_tab_close {
            self.dispatch(AppCommand::CancelCloseDirtyTab(tab_id), cx);
            return;
        }

        if self.pending_apply_data_changes.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_query_parameters.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_dangerous_query.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_dangerous_redis_command.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_query_save.is_some() || self.pending_connection_query_save.is_some() {
            self.cancel_query_save_modal(cx);
            return;
        }

        if self.pending_delete_connection.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_delete_database.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_disconnect_connection.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_close_workspace.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_new_query_connection.take().is_some() {
            cx.notify();
            return;
        }

        if self.pending_create_database.is_some() {
            self.cancel_create_database_modal(cx);
            return;
        }

        if self.pending_rename_table.is_some() {
            self.cancel_rename_table_modal(cx);
            return;
        }

        if self.pending_copy_table.is_some() {
            self.cancel_copy_table_modal(cx);
            return;
        }

        if self.pending_danger_table_action.is_some() {
            self.cancel_danger_table_modal(cx);
            return;
        }

        if self.display_database_connection.is_some() {
            self.cancel_display_database_modal(cx);
            return;
        }

        if self.pending_rename_group.is_some() {
            self.cancel_rename_group(cx);
            return;
        }

        if self.pending_rename_table_folder.is_some() {
            self.cancel_rename_table_folder(cx);
            return;
        }

        if self.new_connection_kind.is_some() {
            self.cancel_new_connection(cx);
            return;
        }

        if self.local_filter_popover.take().is_some() {
            self.sync_table_hover_overlay_block(cx);
            cx.notify();
            return;
        }

        if self.local_filter_manager_popover.take().is_some() {
            self.sync_table_hover_overlay_block(cx);
            cx.notify();
            return;
        }

        if let Some(tab_id) = self.controller.state().active_tab
            && let Some(editor) = self.query_editors.get(&tab_id).cloned()
            && editor.read(cx).completion_visible()
        {
            editor.update(cx, |editor, cx| editor.hide_completion(cx));
            cx.notify();
            return;
        }

        if let Some(tab_id) = self.controller.state().active_tab
            && let Some(editor) = self.query_editors.get(&tab_id).cloned()
            && editor.read(cx).find_open()
        {
            editor.update(cx, |editor, cx| editor.close_find(cx));
            return;
        }

        if let Some(tab_id) = self.active_searchable_data_tab_id()
            && self.data_search_panels.remove(&tab_id)
        {
            self.clear_data_search_state(tab_id, window, cx);
            self.refresh_active_data_table(tab_id, cx);
            cx.notify();
            return;
        }

        if self.connection_context_menu.take().is_some()
            || self.database_context_menu.take().is_some()
            || self.table_context_menu.take().is_some()
            || self.table_group_context_menu.take().is_some()
            || self.table_folder_context_menu.take().is_some()
            || self.tab_context_menu.take().is_some()
            || self.data_cell_context_menu.take().is_some()
            || self.tab_switcher.take().is_some()
            || self.group_context_menu.take().is_some()
        {
            cx.notify();
        }
    }

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let mode = if self.theme_mode == ThemeMode::Dark {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        self.set_theme_mode(mode, cx);
        self.save_appearance_settings(self.settings_editor_draft.clone(), cx);
    }

    // 菜单入口处理（顶部栏移除后防功能丢失）：新建连接 / 设置 / 主题切换。
    fn open_new_connection_action(
        &mut self,
        _: &OpenNewConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_new_connection(window, cx);
    }

    fn open_settings_menu_action(
        &mut self,
        _: &OpenSettingsMenu,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(AppCommand::OpenSettings, cx);
    }

    fn toggle_theme_action(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_theme(cx);
    }

    // 顶部「历史」入口在顶部栏移除后保留可达：走菜单项 OpenQueryHistory。
    fn open_query_history_action(
        &mut self,
        _: &OpenQueryHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_history(window, cx);
    }

    fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.theme_mode = mode;
        self.table_hover_color = None;
        self.restore_table_hover_if_ready(cx);
        // 明暗切换时同步重注入所有 SQL 编辑器的主题（按当前所选主题名派生）。
        let editor_theme = self.editor_theme_for(cx);
        for editor in self.query_editors.values() {
            editor.update(cx, |editor, ecx| editor.set_theme(editor_theme, ecx));
        }

        self.settings_editor_draft.theme = app_theme_from_mode(self.theme_mode);
        self.preview_settings(cx);
    }

    fn set_theme_name(&mut self, mode: ThemeMode, name: String, cx: &mut Context<Self>) {
        let mut settings = self.settings_editor_draft.clone();
        if mode == ThemeMode::Dark {
            settings.dark_theme = name;
        } else {
            settings.light_theme = name;
        }
        self.settings_editor_draft = settings;
        self.preview_settings(cx);
    }

    fn reset_appearance_settings(&mut self, cx: &mut Context<Self>) {
        self.table_hover_color = None;
        self.restore_table_hover_if_ready(cx);

        let mut settings = self.settings_editor_draft.clone();
        let defaults = Settings::default();
        settings.theme = app_theme_from_mode(self.theme_mode);
        settings.light_theme = defaults.light_theme;
        settings.dark_theme = defaults.dark_theme;
        settings.button_radius = defaults.button_radius;
        settings.large_radius = defaults.large_radius;
        settings.show_shadows = defaults.show_shadows;
        settings.focus_ring = defaults.focus_ring;
        settings.scrollbar_mode = defaults.scrollbar_mode;
        settings.ui_density = defaults.ui_density;
        settings.show_status_bar = defaults.show_status_bar;
        settings.reduce_motion = defaults.reduce_motion;
        settings.global_font_family = defaults.global_font_family;
        self.settings_editor_draft = settings;
        self.preview_settings(cx);
        self.show_message("已恢复默认外观，点击保存后生效", AppMessageKind::Info, cx);
    }

    fn confirm_appearance_settings_saved(&mut self, cx: &mut Context<Self>) {
        save_settings_section_from_ui(self, SettingsPanelSection::Appearance, cx);
    }

    fn save_appearance_settings(&mut self, settings: fluxdb_core::Settings, cx: &mut Context<Self>) {
        let _ = self.controller.dispatch(AppCommand::SaveSettings(settings));
        self.settings_editor_draft = self.controller.state().settings.clone();
        self.preview_settings(cx);
        let _ = self
            .storage
            .save_settings(&self.controller.state().settings);
        self.sync_settings_tab_dirty();
        cx.notify();
    }

    fn preview_settings(&mut self, cx: &mut Context<Self>) {
        apply_registered_theme(&self.settings_editor_draft, self.theme_mode, cx);
        apply_component_theme_colors(self.theme_mode, cx);
        let editor_theme = self.editor_theme_for(cx);
        for editor in self.query_editors.values() {
            editor.update(cx, |editor, ecx| editor.set_theme(editor_theme, ecx));
        }
        cx.notify();
    }

    fn refresh_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.controller.state().active_tab().cloned() else {
            if let Some(connection_id) = self.first_connection_id() {
                self.dispatch(AppCommand::OpenConnection(connection_id), cx);
            }
            return;
        };

        if self.redis_key_list_hovered_tab == Some(tab.id) && self.is_redis_data_tab(tab.id) {
            self.request_data_editor_refresh(tab.id, cx);
            return;
        }

        match tab.kind {
            TabKind::ObjectList(list) => {
                self.dispatch(AppCommand::RefreshObject(list.parent), cx);
            }
            TabKind::DataEditor(_) => {
                self.request_data_editor_refresh(tab.id, cx);
            }
            TabKind::QueryEditor(editor) if !editor.text.trim().is_empty() => {
                self.start_query_execution(tab.id, window, cx);
            }
            _ => {}
        }
    }

    /// 侧边栏「刷新连接树」：重拉已展开连接的第一层对象，并重拉已展开数据库的表清单。
    ///
    /// 与 [`Self::refresh_active`] 的关键区别是**不跟随活动标签页**：不会执行 SQL、
    /// 不会重取数据页，也不改变任何连接的展开态。命令本身只在后台线程读一次
    /// `expanded`，因此连点刷新不会把用户刚收起的连接又展开。
    fn refresh_connection_tree(&mut self, cx: &mut Context<Self>) {
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::RefreshConnectionTree);
                    (controller, event)
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if matches!(event, AppEvent::ObjectsLoaded(_, _)) {
                        this.controller.merge_refreshed_tree_from(&controller);
                    } else {
                        this.controller.merge_last_error_from(&controller);
                    }
                    if let Some((text, kind)) = app_event_message(&event) {
                        this.show_message(text, kind, cx);
                    }
                    this.reload_expanded_database_children(cx);
                    cx.notify();
                });
            });
        });
        // 只保留最近一次：连点刷新时旧任务被丢弃即取消，避免多轮结果交错回写。
        self._tree_refresh_task = Some(task);
        cx.notify();
    }

    /// 连接树第一层刷新后，重拉当前处于展开态数据库的表清单，使命中"外部新增/删除表"的场景。
    /// 未被展开的库不请求（展开时由 `load_database_children` 自然拉取），
    /// `load_database_children` 自带 `loading_databases` 去重，重复触发无副作用。
    fn reload_expanded_database_children(&mut self, cx: &mut Context<Self>) {
        let expanded_databases = &self.expanded_databases;
        let targets = self
            .controller
            .state()
            .connections
            .iter()
            .filter(|connection| connection.expanded)
            .flat_map(|connection| {
                let connection_id = connection.config.id;
                connection_databases(connection)
                    .into_iter()
                    .map(move |database| {
                        let name = database
                            .path
                            .database
                            .clone()
                            .unwrap_or_else(|| database.path.name.clone());
                        (database_tree_key(connection_id, &name), database.path)
                    })
            })
            .filter(|(key, _)| expanded_databases.get(key).copied().unwrap_or(false))
            .collect::<Vec<_>>();

        for (key, path) in targets {
            self.load_database_children(path, key, cx);
        }
    }

    fn active_data_editor_dirty(&self) -> Option<(TabId, bool)> {
        self.controller.state().active_tab().and_then(|tab| {
            if let TabKind::DataEditor(editor) = &tab.kind {
                Some((
                    tab.id,
                    editor
                        .changes
                        .as_ref()
                        .is_some_and(|changes| !changes.is_empty()),
                ))
            } else {
                None
            }
        })
    }
}

fn append_sql_history_text(current: &str, sql: &str) -> String {
    let current = current.trim_end();
    if current.is_empty() {
        sql.to_string()
    } else {
        format!("{current}\n\n{sql}")
    }
}

fn query_history_sql_tables(sql: &str) -> Vec<String> {
    let tokens = sql
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut tables = BTreeSet::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].to_ascii_lowercase();
        let is_table_trigger = matches!(token.as_str(), "from" | "join" | "update" | "into")
            || (token == "delete"
                && tokens
                    .get(index + 1)
                    .is_some_and(|next| next.eq_ignore_ascii_case("from")));
        if is_table_trigger {
            if token == "delete" {
                index += 1;
            }
            if let Some(name) = tokens.get(index + 1) {
                tables.insert(
                    name.rsplit_once('.')
                        .map(|(_, table)| table)
                        .unwrap_or(name)
                        .to_string(),
                );
            }
        }
        index += 1;
    }
    tables.into_iter().collect()
}
