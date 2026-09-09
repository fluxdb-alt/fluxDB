impl NavicatMain {
    fn handle_connection_menu_action(
        &mut self,
        action: ConnectionMenuAction,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connection_context_menu = None;
        match action {
            ConnectionMenuAction::Open => {
                self.open_connection_from_sidebar(connection_id, cx);
            }
            ConnectionMenuAction::Disconnect => {
                self.request_disconnect_connection(connection_id, window, cx);
            }
            ConnectionMenuAction::NewQuery => {
                self.pending_new_query_connection = Some(connection_id);
            }
            ConnectionMenuAction::UserAdmin => {
                self.open_user_admin_for_connection(connection_id, cx);
            }
            ConnectionMenuAction::ExecuteSqlFile => {
                self.show_sql_file_execution_modal(Some(connection_id), None, window, cx);
            }
            ConnectionMenuAction::NewDatabase => {
                self.show_create_database_modal(connection_id, window, cx);
            }
            ConnectionMenuAction::Edit => {
                self.show_edit_connection(connection_id, window, cx);
            }
            ConnectionMenuAction::Copy => {
                self.copy_connection(connection_id, cx);
            }
            ConnectionMenuAction::Refresh => {
                self.open_connection_from_sidebar(connection_id, cx);
            }
            ConnectionMenuAction::SelectDatabases => {
                self.show_display_database_modal(connection_id, window, cx);
            }
            ConnectionMenuAction::Delete => {
                self.request_delete_connection(connection_id, window, cx);
            }
            ConnectionMenuAction::MoveToNewGroup => {
                if let Some(group_id) = self.create_connection_group(cx) {
                    let _ = self.controller.dispatch(AppCommand::MoveConnectionToGroup {
                        connection_id,
                        group_id,
                    });
                    self.persist_sidebar_layout();
                }
            }
            ConnectionMenuAction::MoveToGroup(group_id) => {
                let _ = self.controller.dispatch(AppCommand::MoveConnectionToGroup {
                    connection_id,
                    group_id,
                });
                self.persist_sidebar_layout();
            }
            ConnectionMenuAction::Ungroup => {
                let _ = self
                    .controller
                    .dispatch(AppCommand::MoveConnectionToTopLevel(connection_id));
                self.persist_sidebar_layout();
            }
        }
        cx.notify();
    }

    fn handle_group_menu_action(
        &mut self,
        action: GroupMenuAction,
        group_id: ConnectionGroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.group_context_menu = None;
        match action {
            GroupMenuAction::NewConnection => {
                self.show_new_connection(window, cx);
                self.new_connection_target_group = Some(group_id);
            }
            GroupMenuAction::Rename => {
                self.start_rename_group(group_id, window, cx);
            }
            GroupMenuAction::Delete => {
                let _ = self
                    .controller
                    .dispatch(AppCommand::DeleteConnectionGroup(group_id));
                self.persist_sidebar_layout();
            }
        }
        cx.notify();
    }

    fn handle_database_menu_action(
        &mut self,
        action: DatabaseMenuAction,
        menu: DatabaseContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.database_context_menu = None;
        let database_key = database_tree_key(menu.connection_id, &menu.database);
        match action {
            DatabaseMenuAction::TogglePin => {
                if !self.pinned_databases.remove(&database_key) {
                    self.pinned_databases.insert(database_key);
                }
            }
            DatabaseMenuAction::ToggleOpen => {
                if menu.expanded {
                    self.expanded_databases.remove(&database_key);
                    self.loaded_database_children.remove(&database_key);
                    self.loading_databases.remove(&database_key);
                    self.pinned_databases.remove(&database_key);
                    self._database_tasks.remove(&database_key);
                    self.dispatch(
                        AppCommand::DisconnectDatabase {
                            connection_id: menu.connection_id,
                            database: menu.database,
                        },
                        cx,
                    );
                } else {
                    self.expanded_databases.insert(database_key.clone(), true);
                    if !self.loaded_database_children.contains(&database_key)
                        && !self.loading_databases.contains(&database_key)
                    {
                        self.load_database_children(menu.database_path, database_key, cx);
                        return;
                    }
                }
            }
            DatabaseMenuAction::Refresh => {
                // 备份节点右键「刷新」：按磁盘实际文件核对，移除备份文件已不存在的已完成任务。
                // 备份 tab 渲染时直接扫描磁盘，因此仅需 notify 即可触发重扫。
                if menu.backup_only {
                    self.backup_tasks
                        .retain(|t| t.finished_at.is_none() || t.output_path.is_file());
                    cx.notify();
                    return;
                }
                self.loaded_database_children.remove(&database_key);
                self.load_database_children(menu.database_path, database_key, cx);
                return;
            }
            DatabaseMenuAction::NewQuery => {
                // Redis 数据库使用 Workbench 命令执行器，而非 SQL 查询编辑器。
                let is_redis = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .any(|c| c.config.id == menu.connection_id && c.config.kind == DatabaseKind::Redis);
                if is_redis {
                    // 数据库名即逻辑库编号（"0"~"15"），无法解析时回退到 0。
                    let database = menu.database.parse::<u32>().unwrap_or(0);
                    self.dispatch(
                        AppCommand::OpenRedisWorkbench {
                            connection_id: menu.connection_id,
                            database,
                        },
                        cx,
                    );
                } else {
                    self.dispatch(
                        AppCommand::OpenQueryEditorInDatabase {
                            connection_id: menu.connection_id,
                            database: Some(menu.database),
                        },
                        cx,
                    );
                }
            }
            DatabaseMenuAction::RedisCli => {
                // 数据库名即逻辑库编号（"0"~"15"），无法解析时回退到 0，由 controller 兜底去重。
                let database = menu.database.parse::<u32>().unwrap_or(0);
                self.dispatch(
                    AppCommand::OpenRedisCli {
                        connection_id: menu.connection_id,
                        database,
                    },
                    cx,
                );
            }
            DatabaseMenuAction::PubSub => {
                // 数据库名即逻辑库编号（"0"~"15"），无法解析时回退到 0，由 controller 兜底去重。
                let database = menu.database.parse::<u32>().unwrap_or(0);
                self.dispatch(
                    AppCommand::OpenRedisPubSub {
                        connection_id: menu.connection_id,
                        database,
                    },
                    cx,
                );
            }
            DatabaseMenuAction::RunSqlFile => {
                self.show_sql_file_execution_modal(
                    Some(menu.connection_id),
                    Some(menu.database),
                    window,
                    cx,
                );
            }
            DatabaseMenuAction::Backup => {
                self.show_backup_modal(Some(menu.connection_id), Some(menu.database), window, cx);
            }
            DatabaseMenuAction::SetDefault => {
                self.show_message("设置默认数据库入口已就绪", AppMessageKind::Info, cx);
            }
            DatabaseMenuAction::NewTable => {
                self.dispatch(
                    AppCommand::OpenCreateTable {
                        connection_id: menu.connection_id,
                        database: Some(menu.database),
                    },
                    cx,
                );
            }
            DatabaseMenuAction::FindInDatabase => {
                self.show_message("在数据库中查找入口已就绪", AppMessageKind::Info, cx);
            }
            DatabaseMenuAction::Delete => {
                self.request_delete_database(menu.connection_id, menu.database, window, cx);
            }
        }
        cx.notify();
    }

    fn start_rename_group(
        &mut self,
        group_id: ConnectionGroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self
            .controller
            .state()
            .sidebar_layout
            .groups
            .iter()
            .find(|group| group.id == group_id)
            .map(|group| group.name.clone())
            .unwrap_or_else(|| "新分组".to_string());
        self.pending_rename_group = Some(PendingRenameGroup {
            group_id,
            name: name.clone(),
        });
        self.rename_group_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
    }

    fn cancel_rename_group(&mut self, cx: &mut Context<Self>) {
        self.pending_rename_group = None;
        cx.notify();
    }

    fn confirm_rename_group(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_rename_group.take() else {
            return;
        };
        let name = pending.name.trim();
        if !name.is_empty() {
            let _ = self.controller.dispatch(AppCommand::RenameConnectionGroup {
                group_id: pending.group_id,
                name: name.to_string(),
            });
            self.persist_sidebar_layout();
        }
        cx.notify();
    }

    fn move_connection_into_group(
        &mut self,
        connection_id: ConnectionId,
        group_id: ConnectionGroupId,
        cx: &mut Context<Self>,
    ) {
        if self
            .controller
            .state()
            .sidebar_layout
            .connection_group(connection_id)
            == Some(group_id)
        {
            return;
        }

        let mut layout = self.controller.state().sidebar_layout.clone();
        layout.move_connection_to_group(connection_id, group_id);
        if let Some(group) = layout.groups.iter_mut().find(|group| group.id == group_id) {
            group.collapsed = false;
        }
        let _ = self
            .controller
            .dispatch(AppCommand::ReplaceSidebarLayout(layout));
        self.persist_sidebar_layout();
        cx.notify();
    }

    fn move_connection_after(
        &mut self,
        connection_id: ConnectionId,
        after_connection_id: ConnectionId,
        target_group_id: Option<ConnectionGroupId>,
        cx: &mut Context<Self>,
    ) {
        if connection_id == after_connection_id {
            return;
        }

        let mut layout = self.controller.state().sidebar_layout.clone();
        if let Some(group_id) = target_group_id {
            layout.move_connection_to_group_after(
                connection_id,
                group_id,
                Some(after_connection_id),
            );
            if let Some(group) = layout.groups.iter_mut().find(|group| group.id == group_id) {
                group.collapsed = false;
            }
        } else {
            layout.move_connection_to_top_level_after(connection_id, Some(after_connection_id));
        }
        let _ = self
            .controller
            .dispatch(AppCommand::ReplaceSidebarLayout(layout));
        self.persist_sidebar_layout();
        cx.notify();
    }

    fn move_connection_to_top_level_end(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        let mut layout = self.controller.state().sidebar_layout.clone();
        layout.move_connection_to_top_level_after(connection_id, None);
        let _ = self
            .controller
            .dispatch(AppCommand::ReplaceSidebarLayout(layout));
        self.persist_sidebar_layout();
        cx.notify();
    }

    fn request_delete_connection(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connection_context_menu = None;
        self.pending_delete_connection = Some(connection_id);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn request_disconnect_connection(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connection_context_menu = None;
        let warning = self.disconnect_connection_warning(connection_id);
        if warning.unsaved_queries == 0 && warning.running_queries == 0 {
            self.disconnect_connection(connection_id, cx);
            return;
        }
        self.pending_disconnect_connection = Some(warning);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn disconnect_connection_warning(
        &self,
        connection_id: ConnectionId,
    ) -> PendingDisconnectConnection {
        let mut warning = PendingDisconnectConnection {
            connection_id,
            unsaved_queries: 0,
            running_queries: 0,
        };
        for tab in &self.controller.state().tabs {
            let TabKind::QueryEditor(editor) = &tab.kind else {
                continue;
            };
            if editor.connection_id != connection_id {
                continue;
            }
            if editor.has_unsaved_sql() {
                warning.unsaved_queries += 1;
            }
            if editor.running {
                warning.running_queries += 1;
            }
        }
        warning
    }

    fn cancel_disconnect_connection(&mut self, cx: &mut Context<Self>) {
        self.pending_disconnect_connection = None;
        cx.notify();
    }

    fn confirm_disconnect_connection(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_disconnect_connection.take() else {
            return;
        };
        self.disconnect_connection(pending.connection_id, cx);
    }

    fn disconnect_connection(&mut self, connection_id: ConnectionId, cx: &mut Context<Self>) {
        self.dispatch(AppCommand::DisconnectConnection(connection_id), cx);
        self.connecting_connections.remove(&connection_id);
        self._connection_tasks.remove(&connection_id.0);
        self.loaded_database_children
            .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
        self.loading_databases
            .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
        self.expanded_databases
            .retain(|key, _| !key.starts_with(&format!("{}:", connection_id.0)));
        self.expanded_object_groups
            .retain(|key, _| !key.starts_with(&format!("{}:", connection_id.0)));
    }

    fn cancel_delete_connection(&mut self, cx: &mut Context<Self>) {
        self.pending_delete_connection = None;
        cx.notify();
    }

    fn request_delete_database(
        &mut self,
        connection_id: ConnectionId,
        database: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.database_context_menu = None;
        self.pending_delete_database = Some(PendingDeleteDatabase {
            connection_id,
            database,
        });
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_delete_database(&mut self, cx: &mut Context<Self>) {
        self.pending_delete_database = None;
        cx.notify();
    }

    fn confirm_delete_database(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_delete_database.clone() else {
            return;
        };
        if self._delete_database_tasks.contains_key(&pending.connection_id.0) {
            self.show_message("正在删除数据库", AppMessageKind::Warning, cx);
            return;
        }
        self.show_message("正在删除数据库", AppMessageKind::Warning, cx);
        let mut controller = self.controller.clone();
        let connection_id = pending.connection_id;
        let database = pending.database.clone();
        let previous_tabs = self
            .controller
            .state()
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        let task = cx.spawn(async move |view, cx| {
            let (controller, event, _) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::DeleteDatabase {
                        connection_id,
                        database: database.clone(),
                    });
                    (controller, event, database)
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    match event {
                        AppEvent::DatabaseDeleted {
                            connection_id,
                            database,
                        } => {
                            this.controller.merge_deleted_database_from(
                                &controller,
                                connection_id,
                                &database,
                            );
                            this.apply_closed_tabs(previous_tabs, cx);
                            this.pending_delete_database = None;
                            this.loaded_database_children
                                .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
                            this.loading_databases
                                .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
                            this.show_message(
                                format!("已删除数据库「{}」", database),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {
                            this.show_message("删除数据库没有返回结果", AppMessageKind::Error, cx);
                        }
                    }
                    this._delete_database_tasks.remove(&connection_id.0);
                    cx.notify();
                });
            });
        });
        self._delete_database_tasks.insert(connection_id.0, task);
        cx.notify();
    }

    fn show_display_database_modal(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return;
        };
        let databases = all_connection_database_names(connection);
        self.connection_context_menu = None;
        self.group_context_menu = None;
        self.display_database_connection = Some(connection_id);
        self.display_database_selection = configured_visible_databases(&connection.config.options)
            .unwrap_or_else(|| databases.iter().cloned().collect::<BTreeSet<_>>());
        self.display_database_search.clear();
        self.display_database_search_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.display_database_show_system = false;
        cx.notify();
    }

    fn cancel_display_database_modal(&mut self, cx: &mut Context<Self>) {
        self.display_database_connection = None;
        self.display_database_selection.clear();
        self.display_database_search.clear();
        self.display_database_show_system = false;
        cx.notify();
    }

    fn save_display_database_modal(&mut self, cx: &mut Context<Self>) {
        let Some(connection_id) = self.display_database_connection.take() else {
            return;
        };
        let Some(connection) = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return;
        };
        let mut config = connection.config.clone();
        config.options.insert(
            VISIBLE_DATABASES_OPTION.to_string(),
            self.display_database_selection
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let _ = self
            .controller
            .dispatch(AppCommand::UpdateConnection(config));
        let _ = self
            .storage
            .save_connections(&self.controller.connection_configs());
        self.display_database_selection.clear();
        self.display_database_search.clear();
        self.display_database_show_system = false;
        cx.notify();
    }

    fn toggle_display_database(&mut self, database: String, cx: &mut Context<Self>) {
        if !self.display_database_selection.remove(&database) {
            self.display_database_selection.insert(database);
        }
        cx.notify();
    }

    fn select_all_display_databases(&mut self, cx: &mut Context<Self>) {
        if let Some(connection) = self.display_database_connection.and_then(|connection_id| {
            self.controller
                .state()
                .connections
                .iter()
                .find(|connection| connection.config.id == connection_id)
        }) {
            self.display_database_selection = all_connection_database_names(connection)
                .into_iter()
                .filter(|database| {
                    self.display_database_show_system || !is_system_database(database)
                })
                .collect();
            cx.notify();
        }
    }

    fn clear_display_databases(&mut self, cx: &mut Context<Self>) {
        self.display_database_selection.clear();
        cx.notify();
    }

    fn show_all_display_databases(&mut self, cx: &mut Context<Self>) {
        let Some(connection_id) = self.display_database_connection.take() else {
            return;
        };
        let Some(connection) = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return;
        };
        let mut config = connection.config.clone();
        config.options.remove(VISIBLE_DATABASES_OPTION);
        let _ = self
            .controller
            .dispatch(AppCommand::UpdateConnection(config));
        let _ = self
            .storage
            .save_connections(&self.controller.connection_configs());
        self.display_database_selection.clear();
        self.display_database_search.clear();
        self.display_database_show_system = false;
        cx.notify();
    }

    fn toggle_display_system_databases(&mut self, cx: &mut Context<Self>) {
        self.display_database_show_system = !self.display_database_show_system;
        cx.notify();
    }

    fn show_create_database_modal(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return;
        };
        if !matches!(
            connection.config.kind,
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::Sqlite
        ) {
            self.show_message(
                "当前连接类型暂不支持新建数据库",
                AppMessageKind::Warning,
                cx,
            );
            return;
        }

        self.connection_context_menu = None;
        self.group_context_menu = None;
        self.pending_create_database = Some(CreateDatabaseForm {
            connection_id,
            database_kind: connection.config.kind,
            database_name: String::new(),
            charset: "utf8mb4".to_string(),
            collation: "utf8mb4_unicode_ci".to_string(),
        });
        self.create_database_name_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        self.create_database_charset_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(create_database_charset_options()), window, cx);
            select.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        self.create_database_collation_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(create_database_collation_options("utf8mb4")),
                window,
                cx,
            );
            select.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        cx.notify();
    }

    fn cancel_create_database_modal(&mut self, cx: &mut Context<Self>) {
        self.pending_create_database = None;
        cx.notify();
    }

    fn select_create_database_charset(
        &mut self,
        charset: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let collation = default_collation_for_charset(charset);
        if let Some(form) = &mut self.pending_create_database {
            form.charset = charset.to_string();
            form.collation = collation.to_string();
        }
        let charset_value = charset.to_string();
        self.create_database_charset_select.update(cx, |select, cx| {
            select.set_selected_value(&charset_value, window, cx);
        });
        self.create_database_collation_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(create_database_collation_options(charset)),
                window,
                cx,
            );
            select.set_selected_value(&collation.to_string(), window, cx);
        });
        cx.notify();
    }

    fn select_create_database_collation(
        &mut self,
        collation: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.pending_create_database {
            form.collation = collation.to_string();
        }
        self.create_database_collation_select
            .update(cx, |select, cx| select.set_selected_value(&collation.to_string(), window, cx));
        cx.notify();
    }

    fn confirm_create_database(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.pending_create_database.clone() else {
            return;
        };
        if self.create_database_running.contains(&form.connection_id) {
            self.show_message("正在新建数据库", AppMessageKind::Warning, cx);
            return;
        }

        let database_name = form.database_name.trim().to_string();
        if database_name.is_empty() {
            self.show_message("请输入数据库名称", AppMessageKind::Warning, cx);
            return;
        }

        let Some(connection_config) = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == form.connection_id)
            .map(|connection| connection.config.clone())
        else {
            self.show_message("连接不存在", AppMessageKind::Error, cx);
            return;
        };
        let needs_charset = matches!(
            connection_config.kind,
            DatabaseKind::MySql | DatabaseKind::TiDb
        );
        let charset = if needs_charset {
            let charset = form.charset.trim().to_string();
            if charset.is_empty() {
                self.show_message("请输入字符集", AppMessageKind::Warning, cx);
                return;
            }
            charset
        } else {
            String::new()
        };
        let collation = if needs_charset {
            let collation = form.collation.trim().to_string();
            if collation.is_empty() {
                self.show_message("请输入排序规则", AppMessageKind::Warning, cx);
                return;
            }
            collation
        } else {
            String::new()
        };
        let sqlite_target = if connection_config.kind == DatabaseKind::Sqlite {
            match sqlite_create_database_target(&connection_config, &database_name) {
                Ok(target) => Some(target),
                Err(message) => {
                    self.show_message(message, AppMessageKind::Warning, cx);
                    return;
                }
            }
        } else {
            None
        };

        let request = CreateDatabaseRequest {
            connection_id: form.connection_id,
            name: sqlite_target
                .as_ref()
                .map(|(database, _)| database.clone())
                .unwrap_or_else(|| database_name.clone()),
            charset,
            collation,
            path: sqlite_target.as_ref().map(|(_, path)| path.clone()),
        };
        self.create_database_running.insert(form.connection_id);
        self.show_message("正在新建数据库", AppMessageKind::Success, cx);

        let mut controller = self.controller.clone();
        let connection_id = form.connection_id;
        let task = cx.spawn(async move |view, cx| {
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::CreateDatabase(request));
                    (controller, event)
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if matches!(event, AppEvent::ObjectsLoaded(None, _)) {
                        this.controller
                            .merge_open_connection_from(&controller, connection_id);
                        let _ = this
                            .storage
                            .save_connections(&this.controller.connection_configs());
                        this.pending_create_database = None;
                        this.show_message(
                            format!("已创建数据库「{}」", database_name),
                            AppMessageKind::Success,
                            cx,
                        );
                    } else {
                        this.controller.merge_last_error_from(&controller);
                        if let Some((text, kind)) = app_event_message(&event) {
                            this.show_message(text, kind, cx);
                        }
                    }
                    this.create_database_running.remove(&connection_id);
                    this.loaded_database_children
                        .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
                    this.loading_databases
                        .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
                    this._create_database_tasks.remove(&connection_id.0);
                    cx.notify();
                });
            });
        });
        self._create_database_tasks.insert(connection_id.0, task);
        cx.notify();
    }

}

fn sqlite_create_database_target(
    config: &ConnectionConfig,
    database_name: &str,
) -> Result<(String, PathBuf), String> {
    let Endpoint::SqliteFile { path, read_only } = &config.endpoint else {
        return Err("SQLite 连接缺少文件路径".to_string());
    };
    if *read_only {
        return Err("只读 SQLite 连接不支持新建数据库".to_string());
    }
    if path == Path::new(":memory:") {
        return Err("内存 SQLite 连接不支持新建数据库".to_string());
    }

    let name = database_name.trim();
    let name_path = Path::new(name);
    if name_path.components().count() != 1 || name_path.file_name().is_none() {
        return Err("SQLite 数据库名称不能包含路径".to_string());
    }

    let mut file_name = PathBuf::from(name_path.file_name().unwrap());
    if file_name.extension().is_none() {
        file_name.set_extension("db");
    }
    let database = file_name
        .file_stem()
        .map(|name| name.to_string_lossy().trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "SQLite 数据库名称不能为空".to_string())?;
    if database.eq_ignore_ascii_case("main") || database.eq_ignore_ascii_case("temp") {
        return Err("SQLite 数据库名称不能是 main 或 temp".to_string());
    }
    if sqlite_attached_database_path(config, &database).is_some() {
        return Err("SQLite 数据库名称已存在".to_string());
    }
    Ok((
        database,
        path.parent().unwrap_or_else(|| Path::new("")).join(file_name),
    ))
}
