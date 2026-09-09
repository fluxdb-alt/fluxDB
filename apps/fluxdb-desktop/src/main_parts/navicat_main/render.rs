impl Render for NavicatMain {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width;
        let compact = width < px(760.);
        let app_state = self.controller.state();
        let state = render_state_snapshot(app_state);
        let colors = ui_colors_from_theme(self.theme_mode, cx);
        let show_status_bar = if self
            .controller
            .state()
            .tabs
            .iter()
            .any(|tab| matches!(&tab.kind, TabKind::Settings))
        {
            self.settings_editor_draft.show_status_bar
        } else {
            state.settings.show_status_bar
        };
        // Redis 连接串导入 / 云自动发现：异步回调里只更新了表单数据（拿不到 `Window`），
        // 这里消费「待同步」标记，把表单值写回各输入框实体。
        if self.redis_discovery_pending_sync {
            self.redis_discovery_pending_sync = false;
            self.new_connection_inputs
                .sync_from_form(&self.new_connection_form, window, cx);
        }
        let connection_browser_width =
            clamp_connection_browser_width(self.connection_browser_width);
        let workspace_groups = workspace_tab_groups(&state);
        let two_level_tabs = !workspace_groups.is_empty();
        let query_history_detail = self.query_history_detail.clone();
        let pending_apply_preview = self.pending_apply_data_changes.and_then(|tab_id| {
            self.data_change_preview_for_tab(tab_id)
                .map(|(page, changes)| {
                    (
                        tab_id,
                        data_change_item_count(&changes),
                        data_change_statement_count(&changes),
                        data_change_sql_preview(&page, &changes),
                    )
                })
        });
        let (sidebar_rows, sidebar_item_sizes) = if self.show_connection_browser {
            self.sidebar_tree_rows(
                &state,
                &self.loading_databases,
                &self.pinned_databases,
                &self.pinned_tables,
                &self.table_folders,
                &self.table_folder_assignments,
                &self.expanded_databases,
                &self.expanded_object_groups,
                &self.saved_queries,
                &self.sidebar_search,
            )
        } else {
            (Rc::new(Vec::new()), Rc::new(Vec::new()))
        };

        div()
            .size_full()
            .relative()
            .bg(colors.app_bg)
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .key_context("NavicatMain")
            .on_action(cx.listener(Self::new_query))
            .on_action(cx.listener(Self::open_new_connection_action))
            .on_action(cx.listener(Self::open_settings_menu_action))
            .on_action(cx.listener(Self::open_query_history_action))
            .on_action(cx.listener(Self::toggle_theme_action))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::save_or_apply))
            .on_action(cx.listener(Self::close_current_tab))
            .on_action(cx.listener(Self::toggle_connection_browser))
            .on_action(cx.listener(Self::execute_or_apply))
            .on_action(cx.listener(Self::open_data_search))
            .on_action(cx.listener(Self::open_query_history_quick_search))
            .on_action(cx.listener(Self::query_history_quick_search_previous))
            .on_action(cx.listener(Self::query_history_quick_search_next))
            .on_action(cx.listener(Self::query_history_quick_search_confirm))
            .on_action(cx.listener(Self::copy_data_selection))
            .on_action(cx.listener(Self::copy_footer_sql_selection))
            .on_action(cx.listener(Self::delete_connection_shortcut))
            .on_action(cx.listener(Self::cancel_dialog))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.close_context_menus(cx);
                    let mut changed = false;
                    if let Some(tab_id) = this.controller.state().active_tab
                        && let Some(editor) = this.query_editors.get(&tab_id).cloned()
                        && editor.read(cx).completion_visible()
                    {
                        editor.update(cx, |editor, cx| editor.hide_completion(cx));
                        changed = true;
                    }
                    if this.data_filter_popover.take().is_some() {
                        changed = true;
                    }
                    if this.field_filter_popover.take().is_some() {
                        changed = true;
                    }
                    if this.local_filter_popover.take().is_some() {
                        changed = true;
                    }
                    if this.local_filter_manager_popover.take().is_some() {
                        changed = true;
                    }
                    if this.query_history_open {
                        this.query_history_open = false;
                        changed = true;
                    }
                    if this.redis_history_open {
                        this.redis_history_open = false;
                        this.redis_history_scope = None;
                        this.close_redis_history_search(window, cx);
                        changed = true;
                    }
                    if this.query_history_quick_open {
                        this.query_history_quick_open = false;
                        changed = true;
                    }
                    if this.tab_switcher.take().is_some() {
                        changed = true;
                    }
                    if changed {
                        this.sync_table_hover_overlay_block(cx);
                        cx.notify();
                    }
                }),
            )
            .child(topbar(&state, self.show_connection_browser, colors, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .overflow_hidden()
                    .when(self.show_connection_browser, |this| {
                        this.child(sidebar(
                            compact,
                            connection_browser_width,
                            &self.sidebar_tree_scroll,
                            sidebar_rows.clone(),
                            sidebar_item_sizes.clone(),
                            self.table_folder_rename_input.clone(),
                            self.rename_group_input.clone(),
                            self.sidebar_search_input.clone(),
                            &self.sidebar_search,
                            colors,
                            cx,
                        ))
                    })
                    .when(!self.show_connection_browser, |this| {
                        this.child(connection_browser_restore_button(&state, colors, cx))
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w(px(0.))
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(tabs(compact, &state, self, window, colors, cx))
                            .child(content(&state, self, window, colors, cx))
                            .when_some(self.tab_switcher, |this, kind| {
                                this.child(tab_switcher_popup(
                                    kind,
                                    &state,
                                    active_workspace_scope(&state).as_ref(),
                                    self.tab_switcher_search_input.clone(),
                                    &self.tab_switcher_search,
                                    two_level_tabs,
                                    &self.workspace_tab_order,
                                    &self.tab_order,
                                    &self.pinned_tabs,
                                    colors,
                                    cx,
                                ))
                            }),
                    ),
            )
            .when(show_status_bar, |this| {
                this.child(statusbar(
                    &state,
                    // 「执行 SQL 文件」任务列表与 Dialog 共享（Rc<RefCell>），状态栏只读快照
                    &self.sql_file_modal.borrow().tasks,
                    &self.data_export_tasks,
                    &self.backup_tasks,
                    colors,
                    cx,
                ))
            })
            .when_some(self.app_message.as_ref(), |this, message| {
                this.child(app_message_overlay(message, window, colors))
            })
            .when(
                self.connection_context_menu.is_some()
                    || self.database_context_menu.is_some()
                    || self.table_context_menu.is_some()
                    || self.table_group_context_menu.is_some()
                    || self.table_folder_context_menu.is_some()
                    || self.tab_context_menu.is_some()
                    || self.data_cell_context_menu.is_some()
                    || self.data_row_context_menu.is_some()
                    || self.group_context_menu.is_some(),
                |this| this.child(context_menu_backdrop(cx)),
            )
            .when_some(self.connection_context_menu, |this, menu| {
                this.child(connection_context_menu(menu, &state, colors, cx))
            })
            .when_some(self.database_context_menu.clone(), |this, menu| {
                // Redis CLI 只对 Redis 连接下的数据库显示。
                let is_redis = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .any(|c| c.config.id == menu.connection_id && c.config.kind == DatabaseKind::Redis);
                this.child(database_context_menu(
                    menu,
                    &self.pinned_databases,
                    is_redis,
                    colors,
                    cx,
                ))
            })
            .when_some(self.table_context_menu.clone(), |this, menu| {
                this.child(table_context_menu(
                    menu,
                    &self.pinned_tables,
                    &self.table_folders,
                    &self.table_folder_assignments,
                    colors,
                    cx,
                ))
            })
            .when_some(self.table_group_context_menu.clone(), |this, menu| {
                this.child(table_group_context_menu(menu, colors, cx))
            })
            .when_some(self.table_folder_context_menu.clone(), |this, menu| {
                this.child(table_folder_context_menu(menu, colors, cx))
            })
            .when_some(self.tab_context_menu, |this, menu| {
                this.child(tab_context_menu(
                    menu,
                    &state,
                    &self.pinned_tabs,
                    colors,
                    cx,
                ))
            })
            .when_some(self.data_cell_context_menu.clone(), |this, menu| {
                this.child(data_cell_context_menu(menu, colors, cx))
            })
            .when_some(self.data_row_context_menu.clone(), |this, menu| {
                this.child(data_row_context_menu(menu, &state, window, colors, cx))
            })
            .when_some(self.group_context_menu, |this, menu| {
                this.child(group_context_menu(menu, &state, colors, cx))
            })
            .when_some(self.pending_delete_connection, |this, connection_id| {
                this.child(delete_connection_modal(
                    connection_id,
                    &state,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_delete_database.clone(), |this, pending| {
                this.child(delete_database_modal(
                    pending,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_disconnect_connection, |this, pending| {
                this.child(disconnect_connection_modal(
                    pending,
                    &state,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_close_workspace.clone(), |this, pending| {
                this.child(close_workspace_modal(pending, colors, cx))
            })
            .when_some(self.pending_delete_data_row.clone(), |this, menu| {
                this.child(delete_data_row_modal(menu, colors, cx))
            })
            .when_some(self.data_row_viewer.clone(), |this, viewer| {
                this.child(data_row_viewer_modal(
                    viewer,
                    &state,
                    self.row_detail_search_input.clone(),
                    &self.row_detail_search,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_dirty_data_action.clone(), |this, action| {
                this.child(dirty_data_action_modal(action, colors, cx))
            })
            .when_some(state.pending_dirty_tab_close, |this, tab_id| {
                this.child(dirty_tab_close_modal(tab_id, &state, colors, cx))
            })
            .when_some(pending_apply_preview, |this, preview| {
                this.child(apply_data_changes_modal(preview, colors, cx))
            })
            .when_some(self.pending_query_parameters.clone(), |this, pending| {
                this.child(query_parameter_prompt_modal(
                    pending,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_dangerous_query.clone(), |this, pending| {
                this.child(dangerous_query_modal(
                    pending,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(
                self.pending_dangerous_redis_command.clone(),
                |this, pending| {
                    this.child(dangerous_redis_command_modal(
                        pending,
                        self.focus_handle.clone(),
                        colors,
                        cx,
                    ))
                },
            )
            // 「执行 SQL 文件」弹框已迁移到 gpui-component Dialog（window.open_dialog），
            // 由下方 Root::render_dialog_layer 统一渲染，不再在此条件挂载自绘弹层
            .when_some(self.pending_backup_modal.clone(), |this, form| {
                this.child(database_backup_modal(
                    form,
                    self.backup_file_name_input.clone(),
                    self.backup_note_input.clone(),
                    self.backup_object_search_input.clone(),
                    &self.backup_objects_scroll,
                    &self.backup_tasks,
                    colors,
                    cx,
                ))
            })
            // 备份 tab：「备份表」查看弹框。
            .when_some(self.backup_tables_modal.clone(), |this, modal| {
                this.child(backup_tables_modal(modal, colors, cx))
            })
            // 备份 tab：备注编辑弹框（仅当弹框打开时挂载）。
            .when_some(self.backup_note_modal_path.clone(), |this, _| {
                this.child(backup_note_modal(
                    self.backup_note_edit_input.clone(),
                    self.backup_note_modal_path
                        .as_ref()
                        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
                        .unwrap_or_default(),
                    colors,
                    cx,
                ))
            })
            // 备份 tab：删除确认弹框。
            .when_some(self.pending_delete_backup.clone(), |this, path| {
                this.child(backup_delete_confirm_modal(path, colors, cx))
            })
            .when_some(self.backup_log_task, |this, task_id| {
                if let Some(task) = self
                    .backup_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .cloned()
                {
                    this.child(database_backup_log_modal(task, &self.backup_tasks, colors, cx))
                } else {
                    this
                }
            })
            .when_some(self.pending_data_export.clone(), |this, form| {
                let export_filter_tab_id = table_data_export_filter_tab_id(form.tab_id);
                let database_label = data_export_object_database_label(&form.object);
                let database_select = self.data_export_object_select(
                    DataExportObjectSelectKey::Database(form.tab_id),
                    vec![database_label.clone()],
                    &database_label,
                    window,
                    cx,
                );
                let table_select = self.data_export_object_select(
                    DataExportObjectSelectKey::Table(form.tab_id),
                    self.table_data_export_table_select_options(&form),
                    &form.object.name,
                    window,
                    cx,
                );
                let custom_filter_rules = self
                    .data_filter_draft_rules
                    .get(&export_filter_tab_id)
                    .cloned()
                    .unwrap_or_else(|| form.custom_filter_rules.clone());
                let custom_sort_rules = self
                    .data_sort_draft_rules
                    .get(&export_filter_tab_id)
                    .cloned()
                    .unwrap_or_else(|| form.custom_sort_rules.clone());
                this.child(table_data_export_modal(
                    form,
                    database_select,
                    table_select,
                    self.data_export_preview.clone(),
                    self.data_export_custom_conditions_open,
                    custom_filter_rules,
                    custom_sort_rules,
                    self.data_filter_popover,
                    self.data_filter_value_input.clone(),
                    self.data_filter_search_input.clone(),
                    self.data_filter_value_search.clone(),
                    self.data_filter_value_search_loading_until.is_some(),
                    window,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_rename_table.clone(), |this, form| {
                this.child(rename_table_modal(
                    form.clone(),
                    self.rename_table_sql_for_form(&form),
                    self.rename_table_input.clone(),
                    self._rename_table_task.is_some(),
                    self.focus_handle.clone(),
                    window,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_copy_table.clone(), |this, form| {
                this.child(copy_table_modal(
                    form.clone(),
                    self.copy_table_sql_for_form(&form),
                    self.copy_table_input.clone(),
                    self._copy_table_task.is_some(),
                    self.focus_handle.clone(),
                    window,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_column_choices.clone(), |this, form| {
                this.child(column_choices_modal(
                    form,
                    self.column_choice_value_input.clone(),
                    self.column_choice_label_input.clone(),
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_danger_table_action.clone(), |this, form| {
                this.child(danger_table_modal(
                    form.clone(),
                    self.danger_table_sql_for_form(&form),
                    self.danger_table_foreign_key_check_select.clone(),
                    self._danger_table_task.is_some(),
                    self.focus_handle.clone(),
                    window,
                    colors,
                    cx,
                ))
            })
            .when_some(self.data_export_log_task, |this, task_id| {
                if let Some(task) = self
                    .data_export_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .cloned()
                {
                    this.child(table_data_export_log_modal(task, colors, cx))
                } else {
                    this
                }
            })
            .when_some(self.display_database_connection, |this, connection_id| {
                this.child(display_database_modal(
                    connection_id,
                    &state,
                    &self.display_database_selection,
                    &self.display_database_search,
                    self.display_database_search_input.clone(),
                    self.display_database_show_system,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_create_database.clone(), |this, form| {
                let running = self
                    .create_database_running
                    .contains(&form.connection_id);
                this.child(create_database_modal(
                    form,
                    self.create_database_name_input.clone(),
                    self.create_database_charset_select.clone(),
                    self.create_database_collation_select.clone(),
                    running,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_query_save, |this, tab_id| {
                this.child(query_save_choice_modal(
                    tab_id,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_connection_query_save, |this, tab_id| {
                this.child(query_save_connection_modal(
                    tab_id,
                    self.query_save_name_input.clone(),
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_new_query_connection, |this, connection_id| {
                this.child(new_query_scope_modal(
                    connection_id,
                    &state,
                    &self.connecting_connections,
                    colors,
                    cx,
                ))
            })
            .when_some(query_history_detail, |this, entry| {
                this.child(query_history_detail_modal(
                    entry,
                    self.controller.state(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.new_connection_kind, |this, kind| {
                this.child(new_connection_modal(
                    kind,
                    self.new_connection_tab,
                    &self.new_connection_form,
                    &self.new_connection_inputs,
                    self.editing_connection_id.is_some(),
                    colors,
                    window,
                    cx,
                ))
            })
            .when(self.query_history_open, |this| {
                this.child(query_history_drawer(&state, self, colors, cx))
            })
            .when(self.redis_history_open, |this| {
                this.child(redis_workbench_history_drawer(self, colors, cx))
            })
            .when(self.query_history_quick_open, |this| {
                this.child(query_history_quick_search_modal(&state, self, colors, cx))
            })
            // gpui-component 的 Dialog 层：应用自身渲染 dialog layer，
            // 否则 `window.open_dialog` 推入的弹框（除 Redis hash 完整值外）永远不会显示。
            // 注意：dialog builder 会在 NavicatMain::render 期间被同步执行，builder 内禁止
            // re-enter `view.read(cx)`，必须读取与视图共享的状态。
            // Redis hash 大字段完整值已改为 Hash Data 表格区域内嵌面板，不再走弹框。
            .when_some(Root::render_dialog_layer(window, cx), |this, layer| {
                this.child(layer)
            })
            // 性能诊断 HUD：设置开启时在窗口右上角悬浮显示 FPS / 帧耗时 / CPU / GPU / 内存。
            // 挂在根容器（`relative`）最上层，fps_monitor 首次调用会懒建每窗口单例并复用。
            .when(state.settings.performance_diagnostics, |this| {
                this.child(fps_monitor(window, cx))
            })
    }
}

fn render_state_snapshot(state: &AppState) -> AppState {
    AppState {
        connections: state.connections.clone(),
        tabs: state
            .tabs
            .iter()
            .map(|tab| render_tab_snapshot(tab, state.active_tab == Some(tab.id)))
            .collect(),
        active_tab: state.active_tab,
        pending_dirty_tab_close: state.pending_dirty_tab_close,
        settings: state.settings.clone(),
        sidebar_layout: state.sidebar_layout.clone(),
        tasks: state.tasks.clone(),
        query_history: state.query_history.clone(),
        redis_workbench_history: state.redis_workbench_history.clone(),
        next_redis_workbench_history_id: state.next_redis_workbench_history_id,
        last_error: state.last_error.clone(),
    }
}

fn render_tab_snapshot(tab: &TabState, keep_content: bool) -> TabState {
    if keep_content {
        return tab.clone();
    }

    TabState {
        id: tab.id,
        title: tab.title.clone(),
        kind: render_tab_kind_snapshot(&tab.kind),
        dirty: tab.dirty,
    }
}

fn render_tab_kind_snapshot(kind: &TabKind) -> TabKind {
    match kind {
        TabKind::ObjectList(editor) => TabKind::ObjectList(ObjectListState {
            parent: editor.parent.clone(),
            objects: Vec::new(),
            loading: editor.loading,
            error: editor.error.clone(),
        }),
        TabKind::DataEditor(editor) => TabKind::DataEditor(DataEditorState {
            object: editor.object.clone(),
            page: None,
            original_page: None,
            pagination: editor.pagination,
            changes: None,
            editing_cell: editor.editing_cell,
            cell_detail_panel: CellDetailPanelState::default(),
            table_info: TableInfoState::default(),
            loading: editor.loading,
            error: editor.error.clone(),
        }),
        TabKind::QueryEditor(editor) => TabKind::QueryEditor(QueryEditorState {
            connection_id: editor.connection_id,
            database: editor.database.clone(),
            text: String::new(),
            origin: None,
            saved_fingerprint: None,
            running: editor.running,
            results: Vec::new(),
            result_editors: BTreeMap::new(),
            active_result_editor: None,
            summaries: Vec::new(),
            error: editor.error.clone(),
        }),
        TabKind::RedisWorkbench(workbench) => TabKind::RedisWorkbench(RedisWorkbenchState {
            connection_id: workbench.connection_id,
            database: workbench.database,
            text: String::new(),
            running: workbench.running,
            // 快照避免携带完整结果，保持与 QueryEditor 一致的口径。
            executions: Vec::new(),
            error: workbench.error.clone(),
            saved_fingerprint: None,
            next_execution_id: 1,
            collapsed: std::collections::BTreeSet::new(),
            json_views: std::collections::BTreeSet::new(),
        }),
        TabKind::CreateTable(create) => TabKind::CreateTable(create.clone()),
        // 备份列表 tab 状态只有连接+库标识，行数据渲染时现扫磁盘，快照直接克隆。
        TabKind::BackupList(list) => TabKind::BackupList(list.clone()),
        // Redis CLI 状态本身很轻量，快照直接克隆即可（会话复用依赖 connection_id + database）。
        TabKind::RedisCli(cli) => TabKind::RedisCli(cli.clone()),
        // Redis Pub/Sub 状态很轻量（仅 connection_id + database），快照直接克隆。
        TabKind::RedisPubSub(pubsub) => TabKind::RedisPubSub(pubsub.clone()),

        TabKind::UserAdmin(admin) => TabKind::UserAdmin(UserAdminState {
            connection_id: admin.connection_id,
            active_detail_tab: admin.active_detail_tab,
            users: Vec::new(),
            selected_user: admin.selected_user.clone(),
            creating_user: admin.creating_user,
            grants: Vec::new(),
            grants_loaded_user: admin.grants_loaded_user.clone(),
            member_grants: admin.member_grants.clone(),
            member_grants_loaded_role: admin.member_grants_loaded_role.clone(),
            search: admin.search.clone(),
            loading_users: admin.loading_users,
            loading_grants: admin.loading_grants,
            loading_member_grants: admin.loading_member_grants,
            applying: admin.applying,
            users_error: admin.users_error.clone(),
            grants_error: admin.grants_error.clone(),
            member_grants_error: admin.member_grants_error.clone(),
            apply_error: admin.apply_error.clone(),
            privilege_scope: admin.privilege_scope,
            privilege_database: admin.privilege_database.clone(),
            privilege_table: admin.privilege_table.clone(),
            privilege_role: admin.privilege_role.clone(),
            grant_option: admin.grant_option,
            selected_privileges: admin.selected_privileges.clone(),
            privilege_rows: admin.privilege_rows.clone(),
            base_privilege_rows: admin.base_privilege_rows.clone(),
            next_privilege_row_id: admin.next_privilege_row_id,
            create_user: admin.create_user.clone(),
            create_host: admin.create_host.clone(),
            auth_plugin: admin.auth_plugin.clone(),
            password_expiry_policy: admin.password_expiry_policy.clone(),
            create_password: String::new(),
            new_password: String::new(),
            max_queries_per_hour: admin.max_queries_per_hour.clone(),
            max_updates_per_hour: admin.max_updates_per_hour.clone(),
            max_connections_per_hour: admin.max_connections_per_hour.clone(),
            max_user_connections: admin.max_user_connections.clone(),
            ssl_type: admin.ssl_type.clone(),
            ssl_cipher: admin.ssl_cipher.clone(),
            ssl_issuer: admin.ssl_issuer.clone(),
            ssl_subject: admin.ssl_subject.clone(),
            role_membership_edits: admin.role_membership_edits.clone(),
            member_grant_edits: admin.member_grant_edits.clone(),
            pending_sql: admin.pending_sql.clone(),
        }),
        TabKind::Settings => TabKind::Settings,
    }
}

impl Focusable for NavicatMain {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
