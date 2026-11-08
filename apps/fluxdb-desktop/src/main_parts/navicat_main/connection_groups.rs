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
                self.open_user_admin_for_connection(connection_id, window, cx);
            }
            ConnectionMenuAction::ExecuteSqlFile => {
                self.show_sql_file_execution_modal(Some(connection_id), None, window, cx);
            }
            ConnectionMenuAction::CompareStructure => {
                self.show_compare_transfer_modal(
                    connection_id,
                    CompareTransferTab::Structure,
                    window,
                    cx,
                );
            }
            ConnectionMenuAction::TransferData => {
                self.show_compare_transfer_modal(
                    connection_id,
                    CompareTransferTab::Data,
                    window,
                    cx,
                );
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
                // 已展开连接：刷新连接树，只换第一层并保留已加载的表/视图（避免 open_connection_from_sidebar
                // 整体重开把已加载的深层表清空）；未展开时才视为首次打开。
                let expanded = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .find(|c| c.config.id == connection_id)
                    .is_some_and(|c| c.expanded);
                if expanded {
                    self.refresh_connection_tree(cx);
                } else {
                    self.open_connection_from_sidebar(connection_id, cx);
                }
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
            ConnectionMenuAction::MongoInfo => {
                self.show_mongo_info_dialog(connection_id, cx);
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
                            schema: None
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
                self.show_message(
                    tr("app.connection_action.default_database_ready"),
                    AppMessageKind::Info,
                    cx,
                );
            }
            DatabaseMenuAction::NewTable => {
                self.dispatch(
                    AppCommand::OpenCreateTable {
                        connection_id: menu.connection_id,
                        database: Some(menu.database),
                        schema: None,
                    },
                    cx,
                );
            }
            DatabaseMenuAction::NewSchema => {
                // Oracle 的 schema 就是账号，且必须带口令、按需授登录权限才能用。
                // 复用「用户与权限」里的新建账号表单，不另造一个只填名字的入口：
                // 那样只能生成一个随机口令、无法登录的空 schema。
                let is_oracle = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == menu.connection_id)
                    .is_some_and(|connection| connection.config.kind == DatabaseKind::Oracle);
                if is_oracle {
                    // 打开面板会把该页签置为活动页，随后直接进入新建账号表单。
                    self.open_user_admin_for_connection(menu.connection_id, window, cx);
                    if let Some(tab_id) = self.active_user_admin_tab_id() {
                        self.dispatch(AppCommand::BeginOracleCreateUser(tab_id), cx);
                    }
                } else {
                    self.show_create_schema_modal(menu.database_path.clone(), window, cx);
                }
            }
            DatabaseMenuAction::NewCollection => {
                self.show_create_collection_modal(menu.connection_id, menu.database.clone(), window, cx);
            }
            DatabaseMenuAction::MongoCli => {
                self.dispatch(
                    AppCommand::OpenMongoCli {
                        connection_id: menu.connection_id,
                        database: menu.database.clone(),
                    },
                    cx,
                );
            }
            DatabaseMenuAction::MongoWorkbench => {
                self.dispatch(
                    AppCommand::OpenMongoWorkbench {
                        connection_id: menu.connection_id,
                        database: menu.database.clone(),
                    },
                    cx,
                );
            }
            DatabaseMenuAction::FindInDatabase => {
                self.show_message(
                    tr("app.connection_action.find_in_database_ready"),
                    AppMessageKind::Info,
                    cx,
                );
            }
            DatabaseMenuAction::Delete => {
                self.request_delete_database(menu.connection_id, menu.database, window, cx);
            }
        }
        cx.notify();
    }

    fn handle_schema_menu_action(
        &mut self,
        action: SchemaMenuAction,
        menu: SchemaContextMenu,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.schema_context_menu = None;
        // schema 路径：供懒加载/刷新复用。
        let schema_path = ObjectPath {
            connection_id: menu.connection_id,
            database: Some(menu.database.clone()),
            schema: Some(menu.schema.clone()),
            name: menu.schema.clone(),
            kind: ObjectKind::Schema,
        };
        match action {
            SchemaMenuAction::NewQuery => {
                self.dispatch(
                    AppCommand::OpenQueryEditorInDatabase {
                        connection_id: menu.connection_id,
                        database: Some(menu.database.clone()),
                        schema: Some(menu.schema.clone()),
                    },
                    cx,
                );
            }
            SchemaMenuAction::NewTable => {
                self.dispatch(
                    AppCommand::OpenCreateTable {
                        connection_id: menu.connection_id,
                        database: Some(menu.database.clone()),
                        schema: Some(menu.schema.clone()),
                    },
                    cx,
                );
            }
            SchemaMenuAction::SetDefault => {
                self.show_message(
                    tr("app.connection_action.default_schema_ready"),
                    AppMessageKind::Info,
                    cx,
                );
            }
            SchemaMenuAction::Refresh => {
                let schema_key = schema_tree_key(
                    menu.connection_id,
                    &menu.database,
                    &menu.schema,
                );
                self.loaded_database_children.remove(&schema_key);
                // 按 schema 路径重载关系（load_database_children 内部按 key 去重并置 loading）。
                self.load_database_children(schema_path, schema_key, cx);
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
            .unwrap_or_else(|| tr("app.connection_action.new_group").to_string());
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
        // 删库保护：该连接下有未保存或运行中的查询时禁止直接删除，先处理/断开，避免静默丢数据。
        let warning = self.disconnect_connection_warning(connection_id);
        if warning.unsaved_queries > 0 || warning.running_queries > 0 {
            self.show_message(
                tr("app.connection_action.connection_busy_delete"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
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
        // 删库保护：该库下存在未保存或运行中的查询时禁止直接删除，避免静默丢数据。
        let has_active_query = self.controller.state().tabs.iter().any(|tab| {
            let TabKind::QueryEditor(editor) = &tab.kind else {
                return false;
            };
            editor.connection_id == connection_id
                && editor.database.as_deref() == Some(database.as_str())
                && (editor.has_unsaved_sql() || editor.running)
        });
        if has_active_query {
            self.show_message(
                tr("app.connection_action.database_busy_delete"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
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
            self.show_message(
                tr("app.connection_action.deleting_database"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.show_message(
            tr("app.connection_action.deleting_database"),
            AppMessageKind::Warning,
            cx,
        );
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
                                tr("app.connection_action.database_deleted")
                                    .replace("%{name}", &database),
                                AppMessageKind::Success,
                                cx,
                            );
                        }
                        AppEvent::Failed(error) => {
                            this.controller.merge_last_error_from(&controller);
                            this.show_message(error.message, AppMessageKind::Error, cx);
                        }
                        _ => {
                            this.show_message(
                                tr("app.connection_action.delete_database_no_result"),
                                AppMessageKind::Error,
                                cx,
                            );
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
        // 默认勾选全部非系统库（系统库默认不显示）；勾选“显示系统库”后可再加选系统库。
        self.display_database_selection = configured_visible_databases(&connection.config.options)
            .unwrap_or_else(|| {
                databases
                    .iter()
                    .filter(|database| !is_system_database(database))
                    .cloned()
                    .collect::<BTreeSet<_>>()
            });
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
            DatabaseKind::MySql
                | DatabaseKind::TiDb
                | DatabaseKind::MariaDb
                | DatabaseKind::Sqlite
                | DatabaseKind::Postgres
                | DatabaseKind::SqlServer
                // MongoDB 的库由首个集合隐式创建，弹框多一个「首个集合」字段。
                | DatabaseKind::MongoDb
        ) {
            self.show_message(
                tr("app.connection_action.new_database_unsupported"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }

        self.connection_context_menu = None;
        self.group_context_menu = None;
        let is_postgres = connection.config.kind == DatabaseKind::Postgres;
        let is_sqlserver = connection.config.kind == DatabaseKind::SqlServer;
        let is_mongo = connection.config.kind == DatabaseKind::MongoDb;
        self.pending_create_database = Some(CreateDatabaseForm {
            connection_id,
            database_kind: connection.config.kind,
            database_name: String::new(),
            // PG：charset 即 ENCODING、collation 即 LC_COLLATE/LC_CTYPE(locale)。
            // Mongo 没有字符集概念（charset 位置改承载首个集合名，见 confirm_create_database）。
            charset: if is_postgres {
                "UTF8".to_string()
            } else if is_sqlserver || is_mongo {
                String::new()
            } else {
                "utf8mb4".to_string()
            },
            collation: if is_postgres {
                "C".to_string()
            } else if is_sqlserver {
                "SQL_Latin1_General_CP1_CI_AS".to_string()
            } else if is_mongo {
                String::new()
            } else {
                "utf8mb4_unicode_ci".to_string()
            },
            owner: String::new(),
            template: String::new(),
            first_collection: String::new(),
        });
        self.create_database_name_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        self.create_database_first_collection_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        let charset_options = if is_postgres {
            pg_create_database_encoding_options()
        } else {
            create_database_charset_options()
        };
        let collation_options = if is_postgres {
            pg_locale_options().iter().map(|s| s.to_string()).collect()
        } else if is_sqlserver {
            sqlserver_create_database_collation_options()
        } else {
            create_database_collation_options("utf8mb4")
        };
        self.create_database_charset_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(charset_options), window, cx);
            select.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        self.create_database_collation_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(collation_options), window, cx);
            select.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        cx.notify();
    }

    fn cancel_create_database_modal(&mut self, cx: &mut Context<Self>) {
        self.pending_create_database = None;
        cx.notify();
    }

    fn show_create_schema_modal(
        &mut self,
        database_path: ObjectPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_id = database_path.connection_id;
        self.database_context_menu = None;
        self.pending_create_schema = Some((connection_id, database_path, String::new()));
        self.create_schema_name_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_create_schema_modal(&mut self, cx: &mut Context<Self>) {
        self.pending_create_schema = None;
        cx.notify();
    }

    fn confirm_create_schema(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((connection_id, database_path, schema)) = self.pending_create_schema.clone() else {
            return;
        };
        let schema = schema.trim().to_string();
        if schema.is_empty() {
            self.show_message(
                tr("app.connection_action.enter_schema_name"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        if self.create_schema_running {
            self.show_message(
                tr("app.connection_action.creating_schema"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.create_schema_running = true;
        let schema_display = schema.clone();
        let database_path_display = database_path.clone();
        // 目标库取右键所在的数据库节点（database 字段优先，回退节点名）：schema 必须建在
        // 用户选中的库里，不能落到连接的维护库。
        let target_database = database_path
            .database
            .clone()
            .unwrap_or_else(|| database_path.name.clone());
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let (_, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::CreateSchema {
                        connection_id,
                        database: target_database,
                        schema: schema.clone(),
                    });
                    (controller, event)
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.create_schema_running = false;
                    match &event {
                        AppEvent::SchemaCreated { connection_id: _, schema: _ } => {
                            // 成功后失效该库 schema 缓存并重取：清 loaded 标记，触发重载。
                            this.invalidate_database_schema_cache(&database_path_display, cx);
                            this.show_message(
                                tr("app.connection_action.schema_created")
                                    .replace("%{schema}", &schema_display),
                                AppMessageKind::Success,
                                cx,
                            );
                            this.pending_create_schema = None;
                        }
                        AppEvent::Failed(error) => {
                            this.show_message(
                                tr("app.connection_action.create_schema_failed")
                                    .replace("%{error}", &error.message),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                        _ => {
                            this.show_message(
                                tr("app.connection_action.create_schema_failed_short"),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._create_schema_task = Some(task);
        cx.notify();
    }

    fn select_create_database_charset(
        &mut self,
        charset: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // PG：charset 即 ENCODING，collation 保持 PG locale 选项，不套用 MySQL 排序规则。
        let is_postgres = self
            .pending_create_database
            .as_ref()
            .is_some_and(|form| form.database_kind == DatabaseKind::Postgres);
        let collation = if is_postgres {
            "C".to_string()
        } else {
            default_collation_for_charset(charset).to_string()
        };
        if let Some(form) = &mut self.pending_create_database {
            form.charset = charset.to_string();
            form.collation = collation.clone();
        }
        let charset_value = charset.to_string();
        self.create_database_charset_select.update(cx, |select, cx| {
            select.set_selected_value(&charset_value, window, cx);
        });
        let collation_options = if is_postgres {
            pg_locale_options().iter().map(|s| s.to_string()).collect()
        } else {
            create_database_collation_options(charset)
        };
        self.create_database_collation_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(collation_options), window, cx);
            select.set_selected_value(&collation, window, cx);
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
            self.show_message(
                tr("app.connection_action.creating_database"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }

        let database_name = form.database_name.trim().to_string();
        if database_name.is_empty() {
            self.show_message(
                tr("app.connection_action.enter_database_name"),
                AppMessageKind::Warning,
                cx,
            );
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
            self.show_message(
                tr("app.connection_action.connection_missing"),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        // MongoDB 没有字符集与排序规则：charset 位置改承载「首个集合名」，由连接器
        // 建集合来隐式建库（没有集合的空库在 Mongo 里不存在）。
        let is_mongo = connection_config.kind == DatabaseKind::MongoDb;
        let first_collection = if is_mongo {
            let collection = form.first_collection.trim().to_string();
            if collection.is_empty() {
                self.show_message(
                    tr("app.mongo.create_database.enter_collection"),
                    AppMessageKind::Warning,
                    cx,
                );
                return;
            }
            collection
        } else {
            String::new()
        };
        // MySQL/TiDB 需要 charset+collation；PG 需要 ENCODING+locale（复用 charset/collation 字段）。
        let requires_charset = matches!(
            connection_config.kind,
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb | DatabaseKind::Postgres
        );
        let charset = if is_mongo {
            first_collection
        } else if requires_charset {
            let charset = form.charset.trim().to_string();
            if charset.is_empty() {
                self.show_message(
                    tr("app.connection_action.enter_charset"),
                    AppMessageKind::Warning,
                    cx,
                );
                return;
            }
            charset
        } else {
            String::new()
        };
        let collation = if connection_config.kind == DatabaseKind::SqlServer {
            // SQL Server 的 COLLATE 可选；空值表示使用服务器默认排序规则。
            form.collation.trim().to_string()
        } else if requires_charset {
            let collation = form.collation.trim().to_string();
            if collation.is_empty() {
                self.show_message(
                    tr("app.connection_action.enter_collation"),
                    AppMessageKind::Warning,
                    cx,
                );
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
            owner: form.owner.trim().to_string(),
            template: form.template.trim().to_string(),
            path: sqlite_target.as_ref().map(|(_, path)| path.clone()),
        };
        self.create_database_running.insert(form.connection_id);
        self.show_message(
            tr("app.connection_action.creating_database"),
            AppMessageKind::Success,
            cx,
        );

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
                            tr("app.connection_action.database_created")
                                .replace("%{name}", &database_name),
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
        return Err(tr("app.connection_action.sqlite_missing_path").to_string());
    };
    if *read_only {
        return Err(tr("app.connection_action.sqlite_read_only").to_string());
    }
    if path == Path::new(":memory:") {
        return Err(tr("app.connection_action.sqlite_memory").to_string());
    }

    let name = database_name.trim();
    let name_path = Path::new(name);
    if name_path.components().count() != 1 || name_path.file_name().is_none() {
        return Err(tr("app.connection_action.sqlite_name_path").to_string());
    }

    let mut file_name = PathBuf::from(name_path.file_name().unwrap());
    if file_name.extension().is_none() {
        file_name.set_extension("db");
    }
    let database = file_name
        .file_stem()
        .map(|name| name.to_string_lossy().trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| tr("app.connection_action.sqlite_name_empty").to_string())?;
    if database.eq_ignore_ascii_case("main") || database.eq_ignore_ascii_case("temp") {
        return Err(tr("app.connection_action.sqlite_name_reserved").to_string());
    }
    if sqlite_attached_database_path(config, &database).is_some() {
        return Err(tr("app.connection_action.sqlite_name_exists").to_string());
    }
    Ok((
        database,
        path.parent().unwrap_or_else(|| Path::new("")).join(file_name),
    ))
}
