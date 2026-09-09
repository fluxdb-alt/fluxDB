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
            || self.table_context_menu.take().is_some()
            || self.table_group_context_menu.take().is_some()
            || self.table_folder_context_menu.take().is_some()
            || self.tab_context_menu.take().is_some()
            || self.data_cell_context_menu.take().is_some()
            || self.data_row_context_menu.take().is_some()
            || self.tab_switcher.take().is_some()
            || self.group_context_menu.take().is_some()
            || std::mem::take(&mut self.query_history_open)
            || std::mem::take(&mut self.query_history_quick_open)
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
        dragged_scope: WorkspaceScope,
        target_scope: WorkspaceScope,
        cx: &mut Context<Self>,
    ) {
        let scopes = workspace_tab_groups(self.controller.state())
            .into_iter()
            .map(|group| group.scope)
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
        scope: WorkspaceScope,
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

    fn close_workspace_warning(&self, scope: WorkspaceScope) -> PendingCloseWorkspace {
        let mut pending = PendingCloseWorkspace {
            scope,
            unsaved_queries: 0,
            running_queries: 0,
        };
        for tab in &self.controller.state().tabs {
            if tab_workspace_scope(tab).as_ref() != Some(&pending.scope) {
                continue;
            }
            let TabKind::QueryEditor(editor) = &tab.kind else {
                continue;
            };
            if editor.has_unsaved_sql() {
                pending.unsaved_queries += 1;
            }
            if editor.running {
                pending.running_queries += 1;
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

    fn close_workspace_scope(&mut self, scope: WorkspaceScope, cx: &mut Context<Self>) {
        let tab_ids = self
            .controller
            .state()
            .tabs
            .iter()
            .filter(|tab| tab_workspace_scope(tab).as_ref() == Some(&scope))
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        if tab_ids.is_empty() {
            return;
        }
        self.workspace_tab_order.retain(|item| item != &scope);
        self.hovered_database_tab = None;
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
                    self.show_message("已复制表名", AppMessageKind::Success, cx);
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
                        .map(|tab| (tab.id, tab_workspace_scope(tab))),
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
                        .map(|tab| (tab.id, tab_workspace_scope(tab))),
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
