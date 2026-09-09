impl NavicatMain {
    fn persist_sidebar_layout(&self) {
        let mut layout = self.controller.state().sidebar_layout.clone();
        layout.table_folders = self.table_folders.clone();
        layout.table_folder_assignments = self.table_folder_assignments.clone();
        let _ = self.storage.save_sidebar_layout(
            &self.controller.connection_configs(),
            &layout,
        );
    }

    fn create_connection_group(&mut self, cx: &mut Context<Self>) -> Option<ConnectionGroupId> {
        let base_name = "新分组";
        let existing_count = self
            .controller
            .state()
            .sidebar_layout
            .groups
            .iter()
            .filter(|group| group.name.starts_with(base_name))
            .count();
        let name = if existing_count == 0 {
            base_name.to_string()
        } else {
            format!("{base_name} {}", existing_count + 1)
        };
        let event = self
            .controller
            .dispatch(AppCommand::CreateConnectionGroup(name));
        self.persist_sidebar_layout();
        cx.notify();
        if let AppEvent::ConnectionGroupCreated(group) = event {
            Some(group.id)
        } else {
            None
        }
    }

    fn create_connection_group_and_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(group_id) = self.create_connection_group(cx) {
            self.start_rename_group(group_id, window, cx);
        }
    }

    fn copy_connection(&mut self, connection_id: ConnectionId, cx: &mut Context<Self>) {
        let Some(config) = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| connection.config.clone())
        else {
            return;
        };
        let group_id = self
            .controller
            .state()
            .sidebar_layout
            .connection_group(connection_id);
        let draft = ConnectionDraft {
            name: self.copied_connection_name(&config.name),
            kind: config.kind,
            endpoint: config.endpoint,
            credential_ref: config.credential_ref,
            options: config.options,
            redis_profile: config.redis_profile.clone(),
            mysql_profile: config.mysql_profile.clone(),
        };
        let event = self
            .controller
            .dispatch(AppCommand::CreateConnection(draft));
        if let AppEvent::ConnectionCreated(new_config) = event {
            if let Some(group_id) = group_id {
                let _ = self.controller.dispatch(AppCommand::MoveConnectionToGroup {
                    connection_id: new_config.id,
                    group_id,
                });
            }
            self.persist_sidebar_layout();
            let _ = self
                .storage
                .save_connections(&self.controller.connection_configs());
        }
        cx.notify();
    }

    fn copied_connection_name(&self, name: &str) -> String {
        let names = self
            .controller
            .state()
            .connections
            .iter()
            .map(|connection| connection.config.name.as_str())
            .collect::<BTreeSet<_>>();
        let base = format!("{name} 副本");
        if !names.contains(base.as_str()) {
            return base;
        }
        for index in 2.. {
            let candidate = format!("{base} {index}");
            if !names.contains(candidate.as_str()) {
                return candidate;
            }
        }
        unreachable!()
    }

    fn open_connection_from_sidebar(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        self.connection_context_menu = None;
        self.database_context_menu = None;
        if self.connecting_connections.contains(&connection_id) {
            return;
        }

        self.connecting_connections.insert(connection_id);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::OpenConnection(connection_id));
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
                        // Redis 连接打开后：立即拉取一次概览（版本/内存/CPU）并启动定期刷新
                        this.ensure_redis_overview_refresh(cx);
                        // 版本能力缓存随重连清除，下次打开 Hash 面板自动重新探测（字段级 TTL 开关）
                        this.redis_server_versions.remove(&connection_id);
                        this.redis_server_version_tasks.remove(&connection_id);
                    } else {
                        this.controller.merge_last_error_from(&controller);
                    }
                    if let Some((text, kind)) = app_event_message(&event) {
                        this.show_message(text, kind, cx);
                    }
                    this.connecting_connections.remove(&connection_id);
                    this.loaded_database_children
                        .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
                    this.loading_databases
                        .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
                    this._connection_tasks.remove(&connection_id.0);
                    cx.notify();
                });
            });
        });
        self._connection_tasks.insert(connection_id.0, task);
        cx.notify();
    }

    /// Redis 连接概览（版本/内存/CPU）的定期刷新间隔（秒）。
    /// CPU 使用率需两次 `INFO` 采样做增量，间隔太短会失真，太长老化；5s 为宜。
    const REDIS_OVERVIEW_REFRESH_SECS: u64 = 5;

    /// 确保 Redis 连接概览的定期刷新任务在运行（连接打开时调用一次）。
    /// 任务每次 tick 对已连接的 Redis 连接执行 `INFO` 并合并回 live 控制器供底栏展示；
    /// 没有已连接的 Redis 连接时自动停止。
    fn ensure_redis_overview_refresh(&mut self, cx: &mut Context<Self>) {
        if self.redis_overview_refresh_task.is_some() {
            return;
        }
        // 首次立即刷新一次，使底栏尽快出现指标
        self.refresh_connected_redis_overviews(cx);

        self.redis_overview_refresh_task = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(Self::REDIS_OVERVIEW_REFRESH_SECS))
                    .await;
                let should_continue = match view.update(cx, |this, cx| {
                    this.refresh_connected_redis_overviews(cx);
                    this.has_connected_redis_connections()
                }) {
                    Ok(should_continue) => should_continue,
                    Err(_) => false,
                };
                if !should_continue {
                    if let Some(view) = view.upgrade() {
                        let _ = view.update(cx, |this, _cx| {
                            this.redis_overview_refresh_task = None;
                        });
                    }
                    break;
                }
            }
        }));
    }

    /// 对当前所有已连接的 Redis 连接各发起一次概览拉取（后台 `INFO` + 合并回 live 控制器）。
    fn refresh_connected_redis_overviews(&mut self, cx: &mut Context<Self>) {
        let redis_ids: Vec<ConnectionId> = self
            .controller
            .state()
            .connections
            .iter()
            .filter(|connection| {
                connection.connected && connection.config.kind == DatabaseKind::Redis
            })
            .map(|connection| connection.config.id)
            .collect();
        for id in redis_ids {
            let task = self.spawn_redis_overview_pull(id, cx);
            self.redis_overview_refresh_tasks.insert(id.0, task);
        }
    }

    /// 后台拉取一次指定 Redis 连接的概览，成功时把结果合并回 live 控制器。
    /// 概览（含 CPU 增量）由 `load_redis_overview_command` 在克隆控制器上算好，回并即可。
    fn spawn_redis_overview_pull(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let mut controller = self.controller.clone();
        cx.spawn(async move |view, cx| {
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::LoadRedisOverview(connection_id));
                    (controller, event)
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, _cx| {
                    if matches!(event, AppEvent::RedisOverviewLoaded(_, _)) {
                        this.controller
                            .merge_redis_overview_from(&controller, connection_id);
                    }
                    // 失败事件已由 controller 记录到 last_error，此处静默忽略
                });
            });
        })
    }

    /// 是否仍有已连接的 Redis 连接（决定概览定时任务是否继续）。
    fn has_connected_redis_connections(&self) -> bool {
        self.controller.state().connections.iter().any(|connection| {
            connection.connected && connection.config.kind == DatabaseKind::Redis
        })
    }

    fn toggle_database_tree(
        &mut self,
        connection_id: ConnectionId,
        database_path: ObjectPath,
        has_loaded_children: bool,
        cx: &mut Context<Self>,
    ) {
        let database = database_path
            .database
            .clone()
            .unwrap_or_else(|| database_path.name.clone());
        let key = database_tree_key(connection_id, &database);
        let expanded = self.expanded_databases.get(&key).copied().unwrap_or(false);
        self.expanded_databases.insert(key.clone(), !expanded);

        if !expanded && !has_loaded_children && !self.loading_databases.contains(&key) {
            self.load_database_children(database_path, key, cx);
            return;
        }

        cx.notify();
    }

    fn load_database_children(
        &mut self,
        database_path: ObjectPath,
        database_key: String,
        cx: &mut Context<Self>,
    ) {
        if self.loading_databases.contains(&database_key) {
            return;
        }

        self.loading_databases.insert(database_key.clone());
        let mut controller = self.controller.clone();
        let task_key = database_key.clone();
        let task = cx.spawn(async move |view, cx| {
            let (controller, event) = cx
                .background_spawn(async move {
                    let event = controller.dispatch(AppCommand::LoadObjectChildren(database_path));
                    (controller, event)
                })
                .await;

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    let loaded = if let AppEvent::ObjectsLoaded(Some(parent), objects) = &event {
                        this.controller
                            .merge_loaded_children(parent, objects.clone());
                        true
                    } else {
                        this.controller.merge_last_error_from(&controller);
                        false
                    };
                    this.loading_databases.remove(&task_key);
                    if loaded {
                        this.loaded_database_children.insert(task_key.clone());
                    }
                    this._database_tasks.remove(&task_key);
                    cx.notify();
                });
            });
        });
        self._database_tasks.insert(database_key, task);
        cx.notify();
    }

    fn toggle_object_group_tree(
        &mut self,
        connection_id: ConnectionId,
        database: String,
        group: ObjectGroup,
        cx: &mut Context<Self>,
    ) {
        let key = object_group_tree_key(connection_id, &database, group);
        let expanded = self
            .expanded_object_groups
            .get(&key)
            .copied()
            .unwrap_or(false);
        self.expanded_object_groups.insert(key, !expanded);
        cx.notify();
    }

    fn toggle_table_folder_tree(
        &mut self,
        parent_key: String,
        folder: String,
        cx: &mut Context<Self>,
    ) {
        self.selected_table_folder = Some((parent_key.clone(), folder.clone()));
        let key = table_folder_tree_key(&parent_key, &folder);
        let expanded = self
            .expanded_object_groups
            .get(&key)
            .copied()
            .unwrap_or(true);
        self.expanded_object_groups.insert(key, !expanded);
        cx.notify();
    }

}
