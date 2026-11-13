impl NavicatMain {
    fn close_context_menus(&mut self, cx: &mut Context<Self>) {
        if self.pending_rename_group.is_some() {
            self.confirm_rename_group(cx);
        }
        if self.pending_rename_table_folder.is_some() {
            self.confirm_rename_table_folder(cx);
        }
        if self.connection_context_menu.take().is_some()
            || self.database_context_menu.take().is_some()
            || self.schema_context_menu.take().is_some()
            || self.table_context_menu.take().is_some()
            || self.collection_context_menu.take().is_some()
            || self.table_group_context_menu.take().is_some()
            || self.table_folder_context_menu.take().is_some()
            || self.tab_context_menu.take().is_some()
            || self.data_cell_context_menu.take().is_some()
            || self.data_row_context_menu.take().is_some()
            || self.tab_switcher.take().is_some()
            || self.group_context_menu.take().is_some()
            || std::mem::take(&mut self.query_history_open)
            || std::mem::take(&mut self.query_history_quick_open)
            || std::mem::take(&mut self.quick_open_table_open)
        {
            cx.notify();
        }
    }

    fn toggle_tab_switcher(
        &mut self,
        kind: TabSwitcherKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quick_open_table_open = false;
        self.command_palette.open = false;
        if self.tab_switcher == Some(kind) {
            self.tab_switcher = None;
        } else {
            self.tab_switcher = Some(kind);
            self.tab_switcher_search.clear();
            self.tab_switcher_search_input.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn toggle_tab_pin(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let tab_ids = self
            .controller
            .state()
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        if self.pinned_tabs.contains(&tab_id) {
            self.tab_order =
                tab_order_after_unpin(&tab_ids, &self.tab_order, &self.pinned_tabs, tab_id);
            self.pinned_tabs.remove(&tab_id);
        } else {
            self.tab_order =
                tab_order_after_pin(&tab_ids, &self.tab_order, &self.pinned_tabs, tab_id);
            self.pinned_tabs.insert(tab_id);
        }
        cx.notify();
    }

    fn drop_tab_after(
        &mut self,
        dragged_tab_id: TabId,
        target_tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let tab_ids = self
            .controller
            .state()
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        let result = tab_order_after_tab_drop(
            &tab_ids,
            &self.tab_order,
            &self.pinned_tabs,
            dragged_tab_id,
            target_tab_id,
        );
        self.tab_order = result.order;
        if let Some(pinned) = result.pinned {
            if pinned {
                self.pinned_tabs.insert(dragged_tab_id);
            } else {
                self.pinned_tabs.remove(&dragged_tab_id);
            }
        }
        cx.notify();
    }

    fn drop_workspace_tab_after(
        &mut self,
        dragged_scope: FirstLevelScope,
        target_scope: FirstLevelScope,
        cx: &mut Context<Self>,
    ) {
        let scopes = first_level_tab_groups(self.controller.state())
            .into_iter()
            .map(|group| group.scope())
            .collect::<Vec<_>>();
        self.workspace_tab_order = workspace_tab_order_after_drop(
            &scopes,
            &self.workspace_tab_order,
            &dragged_scope,
            &target_scope,
        );
        cx.notify();
    }

    fn request_close_workspace_scope(
        &mut self,
        scope: FirstLevelScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = self.close_workspace_warning(scope);
        if pending.unsaved_queries == 0 && pending.running_queries == 0 {
            self.close_workspace_scope(pending.scope, cx);
            return;
        }
        self.pending_close_workspace = Some(pending);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// 退出应用前检查会丢失的工作。
    ///
    /// 存在未保存修改（含表数据编辑、SQL/Redis/Mongo 草稿）或运行中的查询时弹确认框，
    /// 避免 Cmd+Q、托盘退出、关窗等路径静默丢弃内容；否则直接退出。
    fn request_quit(&mut self, cx: &mut Context<Self>) {
        if self.has_unsaved_work() {
            self.pending_exit_confirm = true;
            cx.notify();
            return;
        }
        cx.quit();
    }

    /// 是否存在未保存修改或运行中的查询。
    fn has_unsaved_work(&self) -> bool {
        self.controller.state().tabs.iter().any(|tab| {
            tab.dirty
                || match &tab.kind {
                    TabKind::QueryEditor(editor) => editor.running,
                    TabKind::RedisWorkbench(workbench) => workbench.running,
                    _ => false,
                }
        })
    }

    fn close_workspace_warning(&self, scope: FirstLevelScope) -> PendingCloseWorkspace {
        let state = self.controller.state();
        let display_name = match &scope {
            FirstLevelScope::Database(workspace) => Some(workspace.database.clone()),
            FirstLevelScope::RedisConnection(connection_id) => {
                Some(connection_name(state, *connection_id))
            }
            FirstLevelScope::Settings | FirstLevelScope::AiChat => None,
        };
        let mut pending = PendingCloseWorkspace {
            scope,
            display_name,
            unsaved_queries: 0,
            running_queries: 0,
        };
        for tab in &state.tabs {
            if tab_first_level_scope(tab).as_ref() != Some(&pending.scope) {
                continue;
            }
            match &tab.kind {
                TabKind::QueryEditor(editor) => {
                    if editor.has_unsaved_sql() {
                        pending.unsaved_queries += 1;
                    }
                    if editor.running {
                        pending.running_queries += 1;
                    }
                }
                TabKind::RedisWorkbench(workbench) => {
                    if workbench.has_unsaved_text() {
                        pending.unsaved_queries += 1;
                    }
                    if workbench.running {
                        pending.running_queries += 1;
                    }
                }
                _ => {}
            }
        }
        pending
    }

    fn cancel_close_workspace_scope(&mut self, cx: &mut Context<Self>) {
        self.pending_close_workspace = None;
        cx.notify();
    }

    fn confirm_close_workspace_scope(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_close_workspace.take() else {
            return;
        };
        self.close_workspace_scope(pending.scope, cx);
    }

    fn close_workspace_scope(&mut self, scope: FirstLevelScope, cx: &mut Context<Self>) {
        let tab_ids = self
            .controller
            .state()
            .tabs
            .iter()
            .filter(|tab| tab_first_level_scope(tab).as_ref() == Some(&scope))
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        if tab_ids.is_empty() {
            return;
        }
        self.workspace_tab_order.retain(|item| item != &scope);
        self.hovered_first_level_tab = None;
        self.dispatch(AppCommand::CloseTabs(tab_ids), cx);
    }

    fn handle_tab_menu_action(
        &mut self,
        action: TabMenuAction,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        self.tab_context_menu = None;
        match action {
            TabMenuAction::CopyTableName => {
                if let Some(table_name) = data_editor_table_name(self.controller.state(), tab_id) {
                    cx.write_to_clipboard(ClipboardItem::new_string(table_name));
                    self.show_message(tr("app.table.toast.name_copied").as_ref(), AppMessageKind::Success, cx);
                }
            }
            TabMenuAction::Pin => {
                self.toggle_tab_pin(tab_id, cx);
            }
            TabMenuAction::Close => {
                self.dispatch(AppCommand::CloseTab(tab_id), cx);
            }
            TabMenuAction::CloseOthers => {
                let target_tab_ids = tab_context_menu_close_targets(
                    self.controller
                        .state()
                        .tabs
                        .iter()
                        .map(|tab| (tab.id, tab_first_level_scope(tab))),
                    tab_id,
                    false,
                );
                for target_tab_id in target_tab_ids {
                    self.dispatch(AppCommand::CloseTab(target_tab_id), cx);
                }
            }
            TabMenuAction::CloseAll => {
                let target_tab_ids = tab_context_menu_close_targets(
                    self.controller
                        .state()
                        .tabs
                        .iter()
                        .map(|tab| (tab.id, tab_first_level_scope(tab))),
                    tab_id,
                    true,
                );
                for target_tab_id in target_tab_ids {
                    self.dispatch(AppCommand::CloseTab(target_tab_id), cx);
                }
            }
        }
        cx.notify();
    }

}
