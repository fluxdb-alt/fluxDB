impl AppController {
    pub fn dispatch(&mut self, command: AppCommand) -> AppEvent {
        if should_clear_completion_cache(&command) {
            self.clear_completion_cache();
        }
        // 对象刷新保留旧候选，仅标记相关索引失效，避免主线程释放并重建整个索引。
        match &command {
            AppCommand::RefreshObject(Some(object)) => self.invalidate_completion_metadata(
                object.connection_id, object.database.as_deref(), object.schema.as_deref(), None,
            ),
            AppCommand::RefreshObject(None) | AppCommand::RefreshConnectionTree => {
                let scopes = self.completion_index.lock().ok()
                    .map(|index| index.metas.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                for scope in scopes {
                    self.invalidate_completion_metadata(scope.connection_id, scope.database.as_deref(), scope.schema.as_deref(), None);
                }
            }
            _ => {}
        }

        match command {
            command @ (AppCommand::PrepareBackup { .. } | AppCommand::RunBackup(_) | AppCommand::PrepareRestore(_) | AppCommand::ProbeRestore(_) | AppCommand::RunRestore { .. }) => {
                self.dispatch_database_task(command, &AtomicBool::new(false), &mut |_| {})
            }
            AppCommand::LoadConnections => {
                let connections = mock_connections();
                self.next_connection_id = connections
                    .iter()
                    .map(|connection| connection.id.0)
                    .max()
                    .unwrap_or(0)
                    + 1;
                self.state.connections = connections
                    .iter()
                    .cloned()
                    .map(|config| ConnectionState {
                        config,
                        connected: false,
                        expanded: false,
                        objects: Vec::new(),
                        redis_overview: RedisConnectionOverview::default(),
                    })
                    .collect();
                self.state.sidebar_layout = SidebarLayout::for_connections(&connections);
                AppEvent::ConnectionsLoaded(connections)
            }
            AppCommand::ReplaceConnections(connections) => {
                self.next_connection_id = connections
                    .iter()
                    .map(|connection| connection.id.0)
                    .max()
                    .unwrap_or(0)
                    + 1;
                self.state.connections = connections
                    .iter()
                    .cloned()
                    .map(|config| ConnectionState {
                        config,
                        connected: false,
                        expanded: false,
                        objects: Vec::new(),
                        redis_overview: RedisConnectionOverview::default(),
                    })
                    .collect();
                self.state.sidebar_layout.repair(&connections);
                AppEvent::ConnectionsLoaded(connections)
            }
            AppCommand::ReplaceSidebarLayout(mut layout) => {
                let connections = self.connection_configs();
                layout.repair(&connections);
                self.next_group_id = layout
                    .groups
                    .iter()
                    .map(|group| group.id.0)
                    .max()
                    .unwrap_or(0)
                    + 1;
                self.state.sidebar_layout = layout.clone();
                AppEvent::SidebarLayoutChanged(layout)
            }
            AppCommand::CreateConnection(draft) => {
                let connection_id = ConnectionId(self.next_connection_id);
                let mut config = draft.into_config(connection_id);
                self.next_connection_id += 1;
                if config.credential_ref.is_some()
                    || config.options.contains_key("password")
                    || config.redis_profile.as_ref().is_some_and(|profile| {
                        profile.basic.password.inline.is_some()
                            || profile.ssh.password.inline.is_some()
                            || profile.ssh.passphrase.inline.is_some()
                    })
                    || config.mysql_profile.as_ref().is_some_and(|profile| {
                        profile.basic.password.inline.is_some()
                            || profile.ssh().is_some_and(|ssh| {
                                ssh.password.inline.is_some() || ssh.passphrase.inline.is_some()
                            })
                            || profile.proxy().is_some_and(|proxy| proxy.password.inline.is_some())
                    })
                {
                    config.credential_ref = Some(format!("gdb.connection.{}", connection_id.0));
                }
                self.state.connections.push(ConnectionState {
                    config: config.clone(),
                    connected: false,
                    expanded: false,
                    objects: Vec::new(),
                    redis_overview: RedisConnectionOverview::default(),
                });
                self.state
                    .sidebar_layout
                    .move_connection_to_top_level(config.id);
                AppEvent::ConnectionCreated(config)
            }
            AppCommand::UpdateConnection(config) => {
                let endpoint_changed = {
                    let Some(connection) = self
                        .state
                        .connections
                        .iter_mut()
                        .find(|connection| connection.config.id == config.id)
                    else {
                        return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                    };

                    let endpoint_changed = connection.config.kind != config.kind
                        || connection.config.endpoint != config.endpoint;
                    connection.config = config.clone();
                    // 配置已变更：旧会话按旧档案建立，一律释放（含仅改密码/TLS 的情况）。
                    fluxdb_connectors::pg_close_connection_sessions(config.id);
                    settle_closed_query_history(&mut self.state.query_history, config.id, None);
                    if endpoint_changed {
                        connection.connected = false;
                        connection.expanded = false;
                        connection.objects.clear();
                    }
                    endpoint_changed
                };
                if endpoint_changed {
                    self.state
                        .tabs
                        .retain(|tab| !tab_belongs_to_connection(tab, config.id));
                    if self
                        .state
                        .active_tab
                        .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                    {
                        self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                    }
                }
                // 连接配置变更：ER 元数据缓存按连接修订整体失效，避免旧连接结果复用。
                self.er_bump_connection_revision(config.id);
                self.state.last_error = None;
                AppEvent::ConnectionUpdated(config)
            }
            AppCommand::CreateConnectionGroup(name) => {
                let group = ConnectionGroup {
                    id: ConnectionGroupId(self.next_group_id),
                    name,
                    collapsed: false,
                };
                self.next_group_id += 1;
                self.state.sidebar_layout.add_group(group.clone());
                AppEvent::ConnectionGroupCreated(group)
            }
            AppCommand::RenameConnectionGroup { group_id, name } => {
                let name = name.trim();
                if name.is_empty() {
                    return AppEvent::SidebarLayoutChanged(self.state.sidebar_layout.clone());
                }
                if let Some(group) = self
                    .state
                    .sidebar_layout
                    .groups
                    .iter_mut()
                    .find(|group| group.id == group_id)
                {
                    group.name = name.to_string();
                }
                AppEvent::SidebarLayoutChanged(self.state.sidebar_layout.clone())
            }
            AppCommand::ToggleConnectionGroup(group_id) => {
                if let Some(group) = self
                    .state
                    .sidebar_layout
                    .groups
                    .iter_mut()
                    .find(|group| group.id == group_id)
                {
                    group.collapsed = !group.collapsed;
                }
                AppEvent::SidebarLayoutChanged(self.state.sidebar_layout.clone())
            }
            AppCommand::DeleteConnectionGroup(group_id) => {
                self.state.sidebar_layout.delete_group(group_id);
                AppEvent::ConnectionGroupDeleted(group_id)
            }
            AppCommand::MoveConnectionToGroup {
                connection_id,
                group_id,
            } => {
                self.state
                    .sidebar_layout
                    .move_connection_to_group(connection_id, group_id);
                AppEvent::SidebarLayoutChanged(self.state.sidebar_layout.clone())
            }
            AppCommand::MoveConnectionToTopLevel(connection_id) => {
                self.state
                    .sidebar_layout
                    .move_connection_to_top_level(connection_id);
                AppEvent::SidebarLayoutChanged(self.state.sidebar_layout.clone())
            }
            AppCommand::TestConnection(config) => {
                let result = test_connection(&config);
                AppEvent::ConnectionTested(config.id, result)
            }
            AppCommand::DiscoverRedisConnection {
                provider,
                connection_string,
            } => {
                // 统一走连接串/URI 解析，作为「连接串导入 + 云自动发现」的单一入口。
                // `from_uri` 支持 redis:// / rediss:// / redis+sentinel:// / redis-cluster://。
                match RedisConnectionProfile::from_uri(&connection_string) {
                    Ok(mut profile) => {
                        // 云自动发现：把云元数据记录进档案，供弹框与后续对接展示。
                        if !provider.is_empty() {
                            profile.cloud.provider = provider.clone();
                            // 记录来源需抹掉口令，避免明文口令随导入元数据落盘。
                            profile.cloud.imported_name = redact_uri_password(&connection_string);
                            // 从托管主机名推断资源/订阅标识，让云元数据有真实语义。
                            let (host, _) = profile.dial_endpoint();
                            let (resource, subscription) =
                                infer_cloud_from_host(&provider, &host);
                            if profile.cloud.resource.is_empty() {
                                profile.cloud.resource = resource;
                            }
                            if profile.cloud.subscription.is_empty() {
                                profile.cloud.subscription = subscription;
                            }
                        }
                        let (host, port) = profile.dial_endpoint();
                        let name = if host.is_empty() {
                            "Redis-新连接".to_string()
                        } else {
                            format!("{host}:{port}")
                        };
                        let draft = ConnectionDraft {
                            name,
                            kind: DatabaseKind::Redis,
                            endpoint: Endpoint::Tcp {
                                host,
                                port,
                                database: profile.basic.database.clone(),
                            },
                            credential_ref: None,
                            options: BTreeMap::new(),
                            redis_profile: Some(profile),
                            mysql_profile: None,
                            postgres_profile: None,
                        };
                        AppEvent::RedisConnectionDiscovered(draft)
                    }
                    Err(message) => self.fail(Error::new(ErrorKind::Connection, message)),
                }
            }
            AppCommand::OpenConnection(connection_id) => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };

                let config = connection.config.clone();
                match list_objects_for_connection(&config, None) {
                    Ok(objects) => {
                        self.state.last_error = None;
                        connection.connected = true;
                        connection.expanded = true;
                        connection.objects = objects.clone();
                        AppEvent::ObjectsLoaded(None, objects)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::ToggleConnectionExpanded(connection_id) => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };

                if connection.connected {
                    connection.expanded = !connection.expanded;
                }
                AppEvent::ObjectsLoaded(None, connection.objects.clone())
            }
            AppCommand::DisconnectConnection(connection_id) => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };

                connection.connected = false;
                connection.expanded = false;
                connection.objects.clear();
                // 断开即释放该连接的全部 PG 会话：连接驱动、SSH 桥线程随会话 drop 收敛，
                // 不再挂到空闲 TTL（设计 §3.3）。
                fluxdb_connectors::pg_close_connection_sessions(connection_id);
                settle_closed_query_history(&mut self.state.query_history, connection_id, None);
                self.state
                    .tabs
                    .retain(|tab| !tab_belongs_to_connection(tab, connection_id));
                if self
                    .state
                    .active_tab
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                if self
                    .state
                    .pending_dirty_tab_close
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.pending_dirty_tab_close = None;
                }
                self.state.last_error = None;
                AppEvent::ObjectsLoaded(None, Vec::new())
            }
            AppCommand::DisconnectDatabase {
                connection_id,
                database,
            } => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };

                connection.objects.retain(|object| {
                    matches!(object.path.kind, ObjectKind::Database | ObjectKind::Schema | ObjectKind::RedisDb)
                        || object.path.database.as_deref().unwrap_or("main") != database
                });
                self.state
                    .tabs
                    .retain(|tab| !tab_belongs_to_database(tab, connection_id, &database));
                if self
                    .state
                    .active_tab
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                if self
                    .state
                    .pending_dirty_tab_close
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.pending_dirty_tab_close = None;
                }
                self.state.last_error = None;
                AppEvent::ObjectsLoaded(None, connection.objects.clone())
            }
            AppCommand::DeleteConnection(connection_id) => {
                fluxdb_connectors::pg_close_connection_sessions(connection_id);
                settle_closed_query_history(&mut self.state.query_history, connection_id, None);
                let original_len = self.state.connections.len();
                self.state
                    .connections
                    .retain(|connection| connection.config.id != connection_id);
                if self.state.connections.len() == original_len {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                }

                self.state
                    .tabs
                    .retain(|tab| !tab_belongs_to_connection(tab, connection_id));
                if self
                    .state
                    .active_tab
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                if self
                    .state
                    .pending_dirty_tab_close
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.pending_dirty_tab_close = None;
                }
                self.state.last_error = None;
                self.state.sidebar_layout.repair(&self.connection_configs());
                // 连接删除：清空该连接 ER 缓存与修订，避免同 id 复用旧图。
                self.er_invalidate_connection(connection_id);

                AppEvent::ConnectionDeleted(connection_id)
            }
            AppCommand::CreateDatabase(request) => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == request.connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };

                let mut config = connection.config.clone();
                if config.kind == DatabaseKind::Sqlite {
                    let database = request.name.trim();
                    if database.eq_ignore_ascii_case("main")
                        || database.eq_ignore_ascii_case("temp")
                        || sqlite_attached_database_path(&config, database).is_some()
                    {
                        return self.fail(Error::new(ErrorKind::Query, "SQLite 数据库名称已存在"));
                    }
                }
                if let Err(error) = create_database_for_connection(&config, &request) {
                    return self.fail(error);
                }
                if config.kind == DatabaseKind::Sqlite {
                    let path = request.path.clone().ok_or_else(|| {
                        Error::new(ErrorKind::Connection, "SQLite 新建数据库需要文件路径")
                    });
                    match path {
                        Ok(path) => {
                            set_sqlite_attached_database(&mut config, request.name.trim(), path);
                            connection.config = config.clone();
                        }
                        Err(error) => return self.fail(error),
                    }
                }

                match list_objects_for_connection(&config, None) {
                    Ok(objects) => {
                        self.state.last_error = None;
                        connection.connected = true;
                        connection.expanded = true;
                        connection.objects = objects.clone();
                        AppEvent::ObjectsLoaded(None, objects)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::CreateSchema {
                connection_id,
                database,
                schema,
            } => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let config = connection.config.clone();
                if let Err(error) = create_schema_for_connection(
                    &config,
                    connection_id,
                    database.as_str(),
                    schema.as_str(),
                ) {
                    return self.fail(error);
                }
                // 建 schema 成功后通知 UI 失效该库 schema 缓存并重取（见 SchemaCreated 消费点）。
                AppEvent::SchemaCreated { connection_id, schema }
            }
            AppCommand::LoadPgRoles(connection_id) => {
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                match role_operation_for_connection(&config, |connector| {
                    connector.list_roles(connection_id)
                }) {
                    Ok(roles) => AppEvent::PgRolesLoaded(connection_id, roles),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::CreatePgRole {
                connection_id,
                name,
                can_login,
                password,
            } => {
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                match role_operation_for_connection(&config, |connector| {
                    connector.create_role(connection_id, &name, can_login, password.as_deref())
                }) {
                    Ok(()) => AppEvent::PgRoleChanged(connection_id),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::AlterPgRolePassword {
                connection_id,
                name,
                password,
            } => {
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                match role_operation_for_connection(&config, |connector| {
                    connector.alter_role_password(connection_id, &name, &password)
                }) {
                    Ok(()) => AppEvent::PgRoleChanged(connection_id),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::RenamePgRole {
                connection_id,
                old_name,
                new_name,
            } => {
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                match role_operation_for_connection(&config, |connector| {
                    connector.rename_role(connection_id, &old_name, &new_name)
                }) {
                    Ok(()) => AppEvent::PgRoleChanged(connection_id),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::DropPgRole { connection_id, name } => {
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                match role_operation_for_connection(&config, |connector| {
                    connector.drop_role(connection_id, &name)
                }) {
                    Ok(()) => AppEvent::PgRoleChanged(connection_id),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::DeleteDatabase {
                connection_id,
                database,
            } => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };

                let config = connection.config.clone();
                if let Err(error) =
                    delete_database_for_connection(&config, connection_id, database.as_str())
                {
                    return self.fail(error);
                }

                match list_objects_for_connection(&config, None) {
                    Ok(objects) => {
                        self.state.tabs.retain(|tab| match &tab.kind {
                            TabKind::DataEditor(editor) => {
                                editor.object.connection_id != connection_id
                                    || editor.object.database.as_deref() != Some(database.as_str())
                            }
                            TabKind::CreateTable(create) => {
                                create.connection_id != connection_id
                                    || create.database.as_deref() != Some(database.as_str())
                            }
                            TabKind::QueryEditor(editor) => {
                                editor.connection_id != connection_id
                                    || editor.database.as_deref() != Some(database.as_str())
                            }
                            _ => true,
                        });
                        if self
                            .state
                            .active_tab
                            .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                        {
                            self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                        }
                        self.state.last_error = None;
                        connection.connected = true;
                        connection.expanded = true;
                        // 删除库后重拉连接第一层：只替换库/schema 层并保留仍存活库下已加载的表/视图，
                        // 避免整体替换把其他已展开库的深层对象（表/视图）一并清掉。
                        replace_connection_level0(connection, objects);
                        AppEvent::DatabaseDeleted {
                            connection_id,
                            database,
                        }
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::LoadObjectChildren(parent) => {
                let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == parent.connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };

                let config = connection.config.clone();
                match list_objects_for_connection(&config, Some(&parent)) {
                    Ok(objects) => {
                        self.state.last_error = None;
                        replace_loaded_children(&mut connection.objects, &parent, objects.clone());
                        AppEvent::ObjectsLoaded(Some(parent), objects)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::RefreshObject(path) => {
                if path.is_none() {
                    let mut all_objects = Vec::new();
                    for connection in &mut self.state.connections {
                        if connection.connected || connection.expanded {
                            let config = connection.config.clone();
                            match list_objects_for_connection(&config, None) {
                                Ok(objects) => {
                                    // 刷新连接第一层：保留仍存活库下已加载的表/视图，避免整体替换
                                    // 把其他已展开库的深层对象（表/视图）一并清掉。
                                    replace_connection_level0(connection, objects.clone());
                                    all_objects.extend(objects);
                                }
                                Err(error) => return self.fail(error),
                            }
                        }
                    }
                    for tab in &mut self.state.tabs {
                        if let TabKind::ObjectList(list) = &mut tab.kind
                            && list.parent.is_none()
                        {
                            list.objects = all_objects.clone();
                            list.loading = false;
                            list.error = None;
                        }
                    }
                    return AppEvent::ObjectsLoaded(None, all_objects);
                }

                let objects = if let Some(path) = path.as_ref() {
                    let Some(connection) = self
                        .state
                        .connections
                        .iter()
                        .find(|connection| connection.config.id == path.connection_id)
                    else {
                        return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                    };
                    match list_objects_for_connection(&connection.config, Some(path)) {
                        Ok(objects) => objects,
                        Err(error) => return self.fail(error),
                    }
                } else {
                    mock_child_objects(path.as_ref())
                };
                for tab in &mut self.state.tabs {
                    if let TabKind::ObjectList(list) = &mut tab.kind
                        && list.parent == path
                    {
                        list.objects = objects.clone();
                        list.loading = false;
                        list.error = None;
                    }
                }
                AppEvent::ObjectsLoaded(path.clone(), objects)
            }
            // 侧边栏「刷新连接树」：只重拉树上可见（已展开）连接的第一层对象。
            // 与 `RefreshObject(None)` 的两点区别：
            //  1. 折叠连接不发请求（`RefreshObject` 用的是 `connected || expanded`）；
            //  2. 只换第一层，保留已加载的表 / 视图行，不打断已展开的数据库子树。
            // 刷新不负责建立连接，也不改变用户的折叠态，因此不写 `connected` / `expanded`。
            AppCommand::RefreshConnectionTree => {
                let mut first_error: Option<Error> = None;
                for connection in &mut self.state.connections {
                    if !connection.expanded {
                        continue;
                    }
                    let config = connection.config.clone();
                    match list_objects_for_connection(&config, None) {
                        Ok(level0) => replace_connection_level0(connection, level0),
                        // 单个连接失败不中断整轮刷新：保留该连接原有对象，记下首个错误继续。
                        Err(error) => {
                            first_error.get_or_insert(error);
                        }
                    }
                }

                match first_error {
                    None => AppEvent::ObjectsLoaded(None, Vec::new()),
                    Some(error) => self.fail(error),
                }
            }
            AppCommand::OpenObjectList(parent) => {
                let tab_id = self.next_tab_id();
                let objects = mock_child_objects(parent.as_ref());
                self.push_tab(TabState {
                    id: tab_id,
                    title: parent
                        .as_ref()
                        .map(|path| path.name.clone())
                        .unwrap_or_else(|| "对象".to_string()),
                    kind: TabKind::ObjectList(ObjectListState {
                        parent,
                        objects,
                        loading: false,
                        error: None,
                    }),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::OpenDataEditor(object) => {
                if let Some(existing_tab_id) = self.state.tabs.iter().find_map(|tab| {
                    if let TabKind::DataEditor(editor) = &tab.kind
                        && editor.object == object
                    {
                        return Some(tab.id);
                    }
                    None
                }) {
                    self.state.active_tab = Some(existing_tab_id);
                    return AppEvent::TabActivated(existing_tab_id);
                }

                let tab_id = self.next_tab_id();
                let title = object.name.clone();
                // 数据表默认每页行数来自「设置-数据」，与 SQL 查询结果的 page_size 无关。
                // 经 Pagination::new 收敛到 1..=MAX_LIMIT，避免脏配置把页大小置成 0。
                let pagination = Pagination::new(0, self.state.settings.data_table_page_size);

                self.push_tab(TabState {
                    id: tab_id,
                    title,
                    kind: TabKind::DataEditor(DataEditorState {
                        object,
                        page: None,
                        original_page: None,
                        pagination,
                        changes: None,
                        editing_cell: None,
                        cell_detail_panel: CellDetailPanelState::default(),
                        table_info: TableInfoState::default(),
                        loading: true,
                        error: None,
                    }),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::OpenBackupList(database_path) => {
                // 按库去重：同一连接+库已有备份 tab 则激活既有 tab。
                let database = database_path
                    .database
                    .clone()
                    .unwrap_or_else(|| database_path.name.clone());
                let connection_id = database_path.connection_id;
                if let Some(existing_tab_id) = self.state.tabs.iter().find_map(|tab| {
                    if let TabKind::BackupList(list) = &tab.kind
                        && list.connection_id == connection_id
                        && list.database == database
                    {
                        return Some(tab.id);
                    }
                    None
                }) {
                    self.state.active_tab = Some(existing_tab_id);
                    return AppEvent::TabActivated(existing_tab_id);
                }

                let tab_id = self.next_tab_id();
                self.push_tab(TabState {
                    id: tab_id,
                    title: format!("备份-{database}"),
                    kind: TabKind::BackupList(BackupListState {
                        connection_id,
                        database,
                    }),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::LoadErRelationships { scope_key } => {
                let service = self.er_model_service(scope_key.clone()).ok_or_else(|| UserFacingError {
                    title: "ER 关系加载失败".into(), message: "ER 存储尚未初始化".into(), detail: None, retryable: true,
                });
                match service.and_then(|service| service.list().map_err(|error| UserFacingError {
                    title: "ER 关系加载失败".into(), message: error.to_string(), detail: None, retryable: true,
                })) {
                    Ok(relationships) => AppEvent::ErRelationshipsLoaded { scope_key, relationships },
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::CreateErRelationship { scope_key, relationship } => {
                let result = self.er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError { title: "ER 关系创建失败".into(), message: "ER 存储尚未初始化".into(), detail: None, retryable: true })
                    .and_then(|service| service.create(relationship).map_err(|error| UserFacingError { title: "ER 关系创建失败".into(), message: error.to_string(), detail: None, retryable: false }));
                match result { Ok(relationship) => AppEvent::ErRelationshipChanged { scope_key, relationship }, Err(error) => AppEvent::Failed(error) }
            }
            AppCommand::UpdateErRelationship { scope_key, relationship, expected_revision } => {
                let id = relationship.id.clone();
                let result = self.er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError { title: "ER 关系更新失败".into(), message: "ER 存储尚未初始化".into(), detail: None, retryable: true })
                    .and_then(|service| service.update(&id, expected_revision, move |current| { *current = relationship; Ok(()) }).map_err(|error| UserFacingError { title: "ER 关系更新失败".into(), message: error.to_string(), detail: None, retryable: false }));
                match result { Ok(relationship) => AppEvent::ErRelationshipChanged { scope_key, relationship }, Err(error) => AppEvent::Failed(error) }
            }
            AppCommand::ConfirmErRelationship { scope_key, id, expected_revision, by } => {
                let result = self.er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError { title: "ER 关系确认失败".into(), message: "ER 存储尚未初始化".into(), detail: None, retryable: true })
                    .and_then(|service| service.confirm(&id, expected_revision, &by).map_err(|error| UserFacingError { title: "ER 关系确认失败".into(), message: error.to_string(), detail: None, retryable: false }));
                match result { Ok(relationship) => AppEvent::ErRelationshipChanged { scope_key, relationship }, Err(error) => AppEvent::Failed(error) }
            }
            AppCommand::RejectErRelationship { scope_key, id, expected_revision } => {
                let result = self.er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError { title: "ER 关系拒绝失败".into(), message: "ER 存储尚未初始化".into(), detail: None, retryable: true })
                    .and_then(|service| service.reject(&id, expected_revision).map_err(|error| UserFacingError { title: "ER 关系拒绝失败".into(), message: error.to_string(), detail: None, retryable: false }));
                match result { Ok(relationship) => AppEvent::ErRelationshipChanged { scope_key, relationship }, Err(error) => AppEvent::Failed(error) }
            }
            AppCommand::DeleteErRelationship { scope_key, id, expected_revision } => {
                let result = self.er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError { title: "ER 关系删除失败".into(), message: "ER 存储尚未初始化".into(), detail: None, retryable: true })
                    .and_then(|service| service.delete(&id, expected_revision).map_err(|error| UserFacingError { title: "ER 关系删除失败".into(), message: error.to_string(), detail: None, retryable: false }));
                match result { Ok(()) => AppEvent::ErRelationshipDeleted { scope_key, id }, Err(error) => AppEvent::Failed(error) }
            }
            AppCommand::OpenErDiagram(path) => {
                // 表级 path → 当前表关联 ER（以该表为中心 1 跳）；库/其它 → 整库 ER。
                let database = path
                    .database
                    .clone()
                    .unwrap_or_else(|| path.name.clone());
                let schema = path.schema.clone();
                // 中心表用结构化身份（schema + 裸名）：PG 跨 schema 与含点标识符都安全；
                // 展示名由身份统一生成，匹配关系边不靠字符串拆解。
                let center_table = (path.kind == ObjectKind::Table).then(|| fluxdb_core::ErTableRef {
                    database: database.clone(),
                    schema: schema.clone(),
                    name: path.name.clone(),
                });
                let connection_id = path.connection_id;
                // 按「连接+库+schema+中心表」去重：整库与当前表关联视图各自唯一。
                if let Some(existing_tab_id) = self.state.tabs.iter().find_map(|tab| {
                    if let TabKind::ErDiagram(er) = &tab.kind
                        && er.connection_id == connection_id
                        && er.database == database
                        && er.schema == schema
                        && er.center_table == center_table
                    {
                        return Some(tab.id);
                    }
                    None
                }) {
                    self.state.active_tab = Some(existing_tab_id);
                    return AppEvent::TabActivated(existing_tab_id);
                }

                let tab_id = self.next_tab_id();
                let title = match &center_table {
                    Some(table) => format!("{} · 关联 ER", table.display()),
                    None => format!("{database} · ER"),
                };
                self.push_tab(TabState {
                    id: tab_id,
                    title,
                    kind: TabKind::ErDiagram(ErDiagramState {
                        connection_id,
                        database,
                        schema,
                        center_table,
                    }),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::LoadDataPage(tab_id) => {
                self.load_data_page_command(tab_id, Vec::new(), Vec::new())
            }
            AppCommand::SetDataPagePagination {
                tab_id,
                offset,
                limit,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::DataEditor(editor) = &mut tab.kind
                {
                    editor.pagination = Pagination::new(offset, limit);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"))
                }
            }
            AppCommand::LoadDataPageWithSort {
                tab_id,
                sort,
                filters,
            } => self.load_data_page_command(tab_id, sort, filters),
            AppCommand::LoadRedisKey { tab_id, key } => self.load_redis_key_command(tab_id, key),
            AppCommand::LoadRedisKeyMetadata { tab_id, keys } => {
                self.load_redis_key_metadata_command(tab_id, keys)
            }
            AppCommand::LoadRedisOverview(connection_id) => {
                self.load_redis_overview_command(connection_id)
            }
            AppCommand::LoadRedisServerVersion(connection_id) => {
                self.load_redis_server_version_command(connection_id)
            }
            AppCommand::FinishRedisKeyRefresh {
                tab_id,
                key,
                result,
            } => self.finish_redis_key_refresh_command(tab_id, key, result),
            AppCommand::ApplyRedisKeyValue {
                tab_id,
                key,
                new_key,
                ttl,
                value,
            } => self.apply_redis_key_value_command(tab_id, key, new_key, ttl, value),
            AppCommand::AddRedisStreamEntry {
                tab_id,
                key,
                id,
                fields,
                maxlen,
            } => self.apply_redis_stream_entry_add_command(tab_id, key, id, fields, maxlen),
            AppCommand::DeleteRedisStreamEntry {
                tab_id,
                key,
                entry_id,
            } => self.apply_redis_stream_entry_delete_command(tab_id, key, entry_id),
            AppCommand::DeleteRedisSetMember {
                tab_id,
                key,
                member,
            } => self.apply_redis_set_member_delete_command(tab_id, key, member),
            AppCommand::AddRedisSetMember {
                tab_id,
                key,
                member,
            } => self.apply_redis_set_member_add_command(tab_id, key, member),
            AppCommand::LoadRedisSetMembers {
                tab_id,
                key,
                query,
                cursor,
            } => self.load_redis_set_members_command(tab_id, key, query, cursor),
            AppCommand::LoadRedisHashFields {
                tab_id,
                key,
                query,
                cursor,
            } => self.load_redis_hash_fields_command(tab_id, key, query, cursor),
            AppCommand::SetRedisHashField {
                tab_id,
                key,
                field,
                value,
                ttl,
            } => self.apply_redis_hash_field_set_command(tab_id, key, field, value, ttl),
            AppCommand::SetRedisHashFieldRaw {
                tab_id,
                key,
                field,
                value,
                ttl,
            } => self.apply_redis_hash_field_set_raw_command(tab_id, key, field, value, ttl),
            AppCommand::SetRedisHashFieldTtl {
                tab_id,
                key,
                field,
                ttl,
            } => self.apply_redis_hash_field_ttl_command(tab_id, key, field, ttl),
            AppCommand::LoadRedisHashFieldFull {
                tab_id,
                key,
                field,
            } => self.load_redis_hash_field_full_command(tab_id, key, field),
            AppCommand::LoadRedisStringValue { tab_id, key, full } => {
                self.load_redis_string_value_command(tab_id, key, full)
            }
            AppCommand::DownloadRedisStringValue { tab_id, key } => {
                self.download_redis_string_value_command(tab_id, key)
            }
            AppCommand::RenameRedisHashField {
                tab_id,
                key,
                old_field,
                new_field,
                value,
            } => self.apply_redis_hash_field_rename_command(tab_id, key, old_field, new_field, value),
            AppCommand::DeleteRedisHashField { tab_id, key, field } => {
                self.apply_redis_hash_field_delete_command(tab_id, key, field)
            }
            AppCommand::LoadRedisZSetMembers {
                tab_id,
                key,
                query,
                cursor,
            } => self.load_redis_zset_members_command(tab_id, key, query, cursor),
            AppCommand::AddRedisZSetMember {
                tab_id,
                key,
                member,
                score,
            } => self.apply_redis_zset_member_add_command(tab_id, key, member, score),
            AppCommand::UpdateRedisZSetScore {
                tab_id,
                key,
                member,
                score,
            } => self.apply_redis_zset_score_update_command(tab_id, key, member, score),
            AppCommand::DeleteRedisZSetMember {
                tab_id,
                key,
                member,
            } => self.apply_redis_zset_member_delete_command(tab_id, key, member),
            AppCommand::LoadRedisListItems {
                tab_id,
                key,
                query,
                cursor,
            } => self.load_redis_list_items_command(tab_id, key, query, cursor),
            AppCommand::LoadRedisStreamEntries {
                tab_id,
                key,
                since_ms,
                until_ms,
                cursor,
            } => self.load_redis_stream_entries_command(tab_id, key, since_ms, until_ms, cursor),
            AppCommand::LoadRedisStreamGroups { tab_id, key } => {
                self.load_redis_stream_groups_command(tab_id, key)
            }
            AppCommand::PushRedisListItems {
                tab_id,
                key,
                items,
                head,
            } => self.apply_redis_list_items_push_command(tab_id, key, items, head),
            AppCommand::CreateRedisKey { tab_id, request } => {
                self.apply_redis_create_key_command(tab_id, request)
            }
            AppCommand::SetRedisListItem {
                tab_id,
                key,
                index,
                expected_old,
                value,
            } => self.apply_redis_list_item_set_command(tab_id, key, index, expected_old, value),
            AppCommand::DeleteRedisListItem {
                tab_id,
                key,
                index,
                expected_old,
            } => self.apply_redis_list_item_delete_command(tab_id, key, index, expected_old),
            AppCommand::PopRedisListItems {
                tab_id,
                key,
                head,
                count,
            } => self.apply_redis_list_items_pop_command(tab_id, key, head, count),
            AppCommand::FinishDataPageLoad { tab_id, result } => match result {
                Ok(page) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::DataEditor(editor) = &mut tab.kind
                    {
                        editor.page = Some(page.clone());
                        editor.original_page = Some(page.clone());
                        editor.changes = None;
                        editor.editing_cell = None;
                        editor.cell_detail_panel = CellDetailPanelState::default();
                        editor.loading = false;
                        editor.error = None;
                        tab.dirty = false;
                        AppEvent::DataLoaded(tab_id, page)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"))
                    }
                }
                Err(user_error) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::DataEditor(editor) = &mut tab.kind
                    {
                        editor.loading = false;
                        editor.error = Some(user_error.clone());
                    }
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            AppCommand::ToggleTableInfo { tab_id, tab } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                if editor.table_info.open && editor.table_info.active_tab == tab {
                    editor.table_info.open = false;
                } else {
                    editor.table_info.open = true;
                    editor.table_info.active_tab = tab;
                    mark_table_info_loading(&mut editor.table_info, tab);
                }
                AppEvent::TableInfoChanged(tab_id, tab)
            }
            AppCommand::CloseTableInfo(tab_id) => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                let active_tab = editor.table_info.active_tab;
                editor.table_info.open = false;
                AppEvent::TableInfoChanged(tab_id, active_tab)
            }
            AppCommand::SelectTableInfoTab { tab_id, tab } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.open = true;
                editor.table_info.active_tab = tab;
                mark_table_info_loading(&mut editor.table_info, tab);
                AppEvent::TableInfoChanged(tab_id, tab)
            }
            AppCommand::LoadTableInfo { tab_id, tab } => {
                let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => Some(editor.object.clone()),
                    _ => None,
                }) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };

                match self.load_table_info(&object, tab) {
                    Ok(result) => AppEvent::TableInfoLoaded(tab_id, result.tab(), result),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::FinishTableInfoLoad {
                tab_id,
                tab,
                result,
            } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                match result {
                    Ok(result) => {
                        apply_table_info_result(&mut editor.table_info, result);
                        AppEvent::TableInfoChanged(tab_id, tab)
                    }
                    Err(error) => {
                        set_table_info_failed(&mut editor.table_info, tab, error.clone());
                        self.state.last_error = Some(error);
                        AppEvent::TableInfoChanged(tab_id, tab)
                    }
                }
            }
            AppCommand::SetTableInfoSearch { tab_id, search } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.search = search;
                AppEvent::TableInfoChanged(tab_id, editor.table_info.active_tab)
            }
            AppCommand::SetTableInfoWidth { tab_id, width } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.width = width.clamp(260., 800.);
                AppEvent::TableInfoChanged(tab_id, editor.table_info.active_tab)
            }
            AppCommand::ToggleDdlWrap(tab_id) => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.ddl_wrap = !editor.table_info.ddl_wrap;
                AppEvent::TableInfoChanged(tab_id, TableInfoTab::Ddl)
            }
            AppCommand::HighlightDataColumn { tab_id, column } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.highlighted_column = Some(column);
                AppEvent::TableInfoChanged(tab_id, TableInfoTab::Columns)
            }
            AppCommand::EditDataCell {
                tab_id,
                row,
                column,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
                {
                    match edit_data_cell(editor, row, column, value) {
                        Ok(()) => {
                            tab.dirty = editor
                                .changes
                                .as_ref()
                                .is_some_and(|changes| !changes.is_empty());
                            AppEvent::TabActivated(tab_id)
                        }
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::LoadBinaryPreview {
                tab_id,
                row,
                column,
            } => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match binary_preview(editor, row, column) {
                    Ok(preview) => AppEvent::BinaryPreviewLoaded {
                        tab_id,
                        row,
                        column,
                        preview,
                    },
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::DownloadBinaryCell {
                tab_id,
                row,
                column,
            } => match self.download_binary_cell(tab_id, row, column) {
                Ok(bytes) => AppEvent::BinaryCellDownloaded {
                    tab_id,
                    row,
                    column,
                    bytes,
                },
                Err(error) => self.fail(error),
            },
            AppCommand::UpdateBinaryCell {
                tab_id,
                row,
                column,
                payload,
            } => self.update_binary_cell_command(tab_id, row, column, payload),
            AppCommand::SetBinaryCellNull {
                tab_id,
                row,
                column,
            } => self.update_binary_cell_command(tab_id, row, column, BinaryUpdatePayload::SetNull),
            AppCommand::ReplaceBinaryCellFromFile {
                tab_id,
                row,
                column,
                path,
            } => self.update_binary_cell_command(
                tab_id,
                row,
                column,
                BinaryUpdatePayload::FilePath(path),
            ),
            AppCommand::OpenCellDetail {
                tab_id,
                row,
                column,
            } => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match current_cell_value(editor, row, column) {
                    Ok(value) => {
                        editor.cell_detail_panel = CellDetailPanelState {
                            open: true,
                            active_cell: Some(CellPosition { row, column }),
                            mode: CellDetailMode::View,
                            edit_value: cell_value_edit_text(&value),
                        };
                        AppEvent::TabActivated(tab_id)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::CloseCellDetail(tab_id) => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                editor.cell_detail_panel = CellDetailPanelState::default();
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::StartCellDetailEdit(tab_id) => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match active_detail_cell_value(editor) {
                    Ok(CellValue::BinarySummary(summary))
                        if summary.byte_length > HEX_EDIT_LIMIT =>
                    {
                        self.fail(Error::new(
                            ErrorKind::Unsupported,
                            "大二进制值不能直接 Hex 编辑，请使用下载、上传替换或设为 NULL",
                        ))
                    }
                    Ok(value) => {
                        editor.cell_detail_panel.mode = CellDetailMode::Edit;
                        editor.cell_detail_panel.edit_value = cell_value_edit_text(&value);
                        AppEvent::TabActivated(tab_id)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::UpdateCellDetailEditValue { tab_id, value } => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                if editor.cell_detail_panel.open {
                    editor.cell_detail_panel.edit_value = value;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::CancelCellDetailEdit(tab_id) => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match active_detail_cell_value(editor) {
                    Ok(value) => {
                        editor.cell_detail_panel.mode = CellDetailMode::View;
                        editor.cell_detail_panel.edit_value = cell_value_edit_text(&value);
                        AppEvent::TabActivated(tab_id)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::SaveCellDetailEdit(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
                {
                    match save_cell_detail_edit(editor) {
                        Ok(()) => {
                            tab.dirty = editor
                                .changes
                                .as_ref()
                                .is_some_and(|changes| !changes.is_empty());
                            AppEvent::TabActivated(tab_id)
                        }
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::SetCellDetailNull(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
                {
                    match set_cell_detail_value(editor, CellValue::Null) {
                        Ok(()) => {
                            tab.dirty = editor
                                .changes
                                .as_ref()
                                .is_some_and(|changes| !changes.is_empty());
                            AppEvent::TabActivated(tab_id)
                        }
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::RestoreCellDetailOriginalValue(tab_id) => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match active_original_cell_value(editor) {
                    Ok(value) => {
                        editor.cell_detail_panel.edit_value = cell_value_edit_text(&value);
                        AppEvent::TabActivated(tab_id)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::InsertDataRow {
                tab_id,
                result_index,
                after_row,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id) {
                    match apply_data_editor_edit(tab, result_index, |editor| {
                        insert_data_row(editor, after_row)
                    }) {
                        Ok(()) => AppEvent::TabActivated(tab_id),
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::CloneDataRow {
                tab_id,
                result_index,
                row,
                after_row,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id) {
                    match apply_data_editor_edit(tab, result_index, |editor| {
                        clone_data_row(editor, row, after_row)
                    }) {
                        Ok(()) => AppEvent::TabActivated(tab_id),
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::DeleteDataRow {
                tab_id,
                result_index,
                row,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id) {
                    match apply_data_editor_edit(tab, result_index, |editor| {
                        delete_data_row(editor, row)
                    }) {
                        Ok(()) => AppEvent::TabActivated(tab_id),
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::DiscardDataChanges(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
                {
                    editor.page = editor.original_page.clone();
                    editor.changes = None;
                    editor.editing_cell = None;
                    if editor.cell_detail_panel.open {
                        if let Ok(value) = active_detail_cell_value(editor) {
                            editor.cell_detail_panel.mode = CellDetailMode::View;
                            editor.cell_detail_panel.edit_value = cell_value_edit_text(&value);
                        }
                    }
                    editor.error = None;
                    if let TabKind::QueryEditor(query_editor) = &mut tab.kind
                        && let Some(page_index) = query_editor.active_result_editor
                        && let Some(page) = query_editor
                            .result_editors
                            .get(&page_index)
                            .and_then(|result| result.page.clone())
                        && let Some(result) = query_editor.results.get_mut(page_index)
                    {
                        *result = page;
                    }
                    tab.dirty = false;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::ApplyDataChanges(tab_id) => {
                self.apply_data_changes_command(tab_id, Vec::new(), Vec::new())
            }
            AppCommand::ApplyDataChangesWithView {
                tab_id,
                sort,
                filters,
            } => self.apply_data_changes_command(tab_id, sort, filters),
            AppCommand::OpenQueryEditor(connection_id) => {
                self.open_query_editor(connection_id, None, None)
            }
            AppCommand::OpenUserAdmin(connection_id) => self.open_user_admin(connection_id),
            AppCommand::OpenSettings => self.open_settings(),
            AppCommand::OpenQueryEditorInDatabase {
                connection_id,
                database,
                schema,
            } => self.open_query_editor(connection_id, database, schema),
            AppCommand::OpenRedisWorkbench {
                connection_id,
                database,
            } => self.open_redis_workbench(connection_id, database),
            AppCommand::OpenRedisCli {
                connection_id,
                database,
            } => self.open_redis_cli(connection_id, database),
            AppCommand::OpenRedisPubSub {
                connection_id,
                database,
            } => self.open_redis_pubsub(connection_id, database),
            AppCommand::UpdateQueryText { tab_id, text } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    editor.text = text;
                    editor.error = None;
                    tab.dirty = editor.has_unsaved_sql();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::UpdateRedisWorkbenchText { tab_id, text } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::RedisWorkbench(workbench) = &mut tab.kind
                {
                    workbench.text = text;
                    workbench.error = None;
                    tab.dirty = workbench.has_unsaved_text();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "Redis Workbench 标签页不存在"))
                }
            }
            AppCommand::BeginRedisWorkbenchExecution {
                tab_id,
                execution_id,
            } => {
                // 准备步只做状态变更：真实 RESP 往返由 UI 派发到后台线程跑
                // `RunRedisWorkbench`，结果再经 `FinishRedisWorkbenchExecution` 落回。
                // 重跑要按 id 定位记录，记录已被删除时明确失败，不进入运行态。
                if execution_id.is_some() && self.redis_workbench_text(tab_id, execution_id).is_none()
                {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 记录不存在",
                    ));
                }
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                let TabKind::RedisWorkbench(workbench) = &mut tab.kind else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                workbench.running = true;
                workbench.error = None;
                // 执行顶部草稿才清空输入框；重跑结果区记录不打扰当前草稿。
                if execution_id.is_none() {
                    workbench.text.clear();
                    workbench.saved_fingerprint = Some(QueryFingerprint::for_text(""));
                    tab.dirty = workbench.has_unsaved_text();
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::RunRedisWorkbench {
                tab_id,
                execution_id,
            } => {
                // 只执行、不写标签页状态：本命令跑在后台线程的控制器副本上，
                // 状态回写统一交给主线程的 FinishRedisWorkbenchExecution。
                let Some((target, text)) = self.redis_workbench_run_scope(tab_id, execution_id)
                else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                // 执行单元 = 单条命令：把输入文本交给批量执行入口，得到
                // 「每条命令一个 `CommandWorkbenchExecution`」的列表，逐条追加到
                // 结果区，各自独立占一张卡片（独立 Run / Delete / 时间 / 耗时）。
                let request = CommandWorkbenchRequest {
                    target,
                    text: text.clone(),
                    run_mode: CommandRunMode::Text,
                    results_mode: CommandResultsMode::Default,
                    batch_size: 0,
                    continue_on_error: true,
                    source: CommandExecutionSource::Workbench,
                };
                // 失败也用同一事件回传：入历史要带实际执行的文本，而不是执行期间的草稿。
                let result = self
                    .execute_command_workbench_commands(&request)
                    .map_err(UserFacingError::from);
                AppEvent::RedisWorkbenchCommandsRan {
                    tab_id,
                    text,
                    result,
                }
            }
            AppCommand::FinishRedisWorkbenchExecution {
                tab_id,
                text,
                result,
            } => match result {
                Ok(executions) => {
                    let mut history_entries = Vec::new();
                    let Some(tab) = self.find_tab_mut(tab_id) else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "Redis Workbench 标签页不存在",
                        ));
                    };
                    let TabKind::RedisWorkbench(workbench) = &mut tab.kind else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "Redis Workbench 标签页不存在",
                        ));
                    };
                    workbench.running = false;
                    workbench.error = None;
                    for mut execution in executions {
                        // App 层为 connector 产出的每条 execution 分配自增 id，
                        // 供结果区的单条 Run / Delete 精确定位。
                        execution.id = workbench.next_execution_id;
                        workbench.next_execution_id += 1;
                        history_entries.push(execution.clone());
                        workbench.executions.push(execution);
                    }
                    // 顶部输入框可能已被 Run 清空；脏标记基于「未执行改动」实时更新。
                    tab.dirty = workbench.has_unsaved_text();
                    // 每条命令进入历史体系（持久化由桌面层在 state 变化时落盘）。
                    for execution in &history_entries {
                        self.record_redis_workbench_history_execution(execution);
                    }
                    // 结果区由 state 驱动渲染：返回 TabActivated 触发该标签页 UI 重绘，
                    // 桌面端 dispatch 包装会统一 cx.notify()。
                    AppEvent::TabActivated(tab_id)
                }
                Err(user_error) => {
                    let Some(tab) = self.find_tab_mut(tab_id) else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "Redis Workbench 标签页不存在",
                        ));
                    };
                    let (connection_id, database) = match &mut tab.kind {
                        TabKind::RedisWorkbench(workbench) => {
                            workbench.running = false;
                            workbench.error = Some(user_error.clone());
                            (workbench.connection_id, workbench.database)
                        }
                        _ => {
                            return self.fail(Error::new(
                                ErrorKind::Internal,
                                "Redis Workbench 标签页不存在",
                            ));
                        }
                    };
                    self.record_redis_workbench_history_failure(
                        connection_id,
                        database,
                        &text,
                        &user_error.message,
                    );
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            AppCommand::DeleteRedisWorkbenchRecord { tab_id, execution_id } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                    // 只删除指定 id 的一条记录，不影响其他记录；同时清理其折叠态与 JSON 视图标记。
                    workbench.executions.retain(|execution| execution.id != execution_id);
                    workbench.collapsed.remove(&execution_id);
                    workbench
                        .json_views
                        .retain(|(old_id, _)| *old_id != execution_id);
                } else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::ClearRedisWorkbenchResults(tab_id) => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "Redis Workbench 标签页不存在"));
                };
                if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                    workbench.executions.clear();
                    workbench.error = None;
                    // 记录清空后折叠态与 JSON 视图标记一并清空，避免残留已删除记录的标记。
                    workbench.collapsed.clear();
                    workbench.json_views.clear();
                } else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::ToggleRedisWorkbenchRecordCollapse {
                tab_id,
                execution_id,
            } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                    // 折叠/展开切换：下落（指记录仍在）、否则补回。
                    if !workbench.collapsed.insert(execution_id) {
                        workbench.collapsed.remove(&execution_id);
                    }
                } else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::ToggleRedisWorkbenchJsonView {
                tab_id,
                execution_id,
                command_index,
            } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                    // Text / JSON 切换：下落（指当前在 JSON 视图）、否则补回，按执行项维度记忆。
                    if !workbench.json_views.insert((execution_id, command_index)) {
                        workbench.json_views.remove(&(execution_id, command_index));
                    }
                } else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::DeleteRedisWorkbenchHistory { scope, id } => {
                // 按 scope + id 删除单条历史；删除是状态变化，无需特定标签页事件。
                self.delete_history(&scope, id);
                AppEvent::RedisWorkbenchHistoryChanged
            }
            AppCommand::ClearRedisWorkbenchHistory { scope } => {
                // 清空某 scope 下的全部历史。
                self.clear_history(&scope);
                AppEvent::RedisWorkbenchHistoryChanged
            }
            AppCommand::RequestQueryCompletions {
                tab_id,
                request_seq,
                cursor,
                explicit,
            } => {
                let Some(editor) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                    TabKind::QueryEditor(editor) => Some(editor),
                    _ => None,
                }) else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };
                match self.query_completions(editor, cursor, explicit) {
                    Ok(result) => AppEvent::QueryCompletionsLoaded(tab_id, request_seq, result),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::WarmCompletionIndex {
                connection_id,
                database,
            } => match self.warm_completion_index(connection_id, database.as_deref(), None) {
                Ok(()) => AppEvent::CompletionIndexWarmed(connection_id, database),
                Err(error) => self.fail(error),
            },
            AppCommand::FormatQuerySql(tab_id) => {
                let Some((connection_id, text)) = self.find_tab(tab_id).and_then(|tab| {
                    let TabKind::QueryEditor(editor) = &tab.kind else {
                        return None;
                    };
                    Some((editor.connection_id, editor.text.clone()))
                }) else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                let kind = self
                    .connection_kind(connection_id)
                    .unwrap_or(DatabaseKind::MySql);
                let formatted = format_sql_text_for_dialect(&text, kind);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    editor.text = formatted;
                    editor.error = None;
                    tab.dirty = editor.has_unsaved_sql();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::CompressQuerySql(tab_id) => {
                let Some(text) = self.find_tab(tab_id).and_then(|tab| {
                    let TabKind::QueryEditor(editor) = &tab.kind else {
                        return None;
                    };
                    Some(editor.text.clone())
                }) else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                let compressed = compress_sql_text(&text);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    editor.text = compressed;
                    editor.error = None;
                    tab.dirty = editor.has_unsaved_sql();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::MarkQuerySaved {
                tab_id,
                title,
                origin,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    tab.title = title;
                    editor.origin = Some(origin);
                    editor.saved_fingerprint = Some(QueryFingerprint::for_text(&editor.text));
                    tab.dirty = false;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::StartQueryExecution(tab_id) => {
                // 每次执行前登记一个全新的取消标志：既重置上一次「停止」的残留置位，
                // 也让后台执行线程能拿到与「停止」按钮同一个标志。
                self.register_query_cancel_flag(tab_id);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    editor.running = true;
                    editor.error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::CancelQueryExecution(tab_id) => {
                // 没有登记标志（该标签当前没有在执行的查询）时不报错，只如实记录，避免
                // 「停止」按钮在竞态窗口内点两次就弹错误。
                let had_running_query = self.request_query_cancel(tab_id);
                tracing::info!(
                    target: "fluxdb_app",
                    tab_id = tab_id.0,
                    had_running_query,
                    "已请求取消查询执行"
                );
                AppEvent::QueryCancelRequested(tab_id)
            }
            AppCommand::ExecuteQuery(tab_id) => {
                let options = self.default_query_execution_options();
                let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                    TabKind::QueryEditor(editor) => Some(QueryRequest {
                        connection_id: editor.connection_id,
                        database: editor.database.clone(),
                        // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                        session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                        schema: editor.schema.clone(),
                        text: sql_text_for_execution(
                            &editor.text,
                            options.page_size,
                            self.connection_kind(editor.connection_id)
                                .unwrap_or(DatabaseKind::MySql),
                        ),
                        mode: fluxdb_core::QueryMode::All,
                        options,
                    }),
                    _ => None,
                });

                let Some(request) = request else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                let cancel_flag = self.query_cancel_flag(tab_id);
                let result = self.execute_query_with_cancel(&request, &|| {
                    query_cancel_requested(&cancel_flag)
                });
                // 取消标志只属于本次执行。执行器已经返回后立即回收，不能等 UI 的异步
                // Finish 命令，否则直接调用 App 层的入口会遗留已置位标志。
                self.clear_query_cancel_flag(tab_id);
                match result {
                    Ok(execution) => {
                        let result_editors = query_result_editors(self, &request, &execution);
                        let active_result_editor = result_editors.keys().next().copied();
                        if let Some(tab) = self.find_tab_mut(tab_id)
                            && let TabKind::QueryEditor(editor) = &mut tab.kind
                        {
                            editor.running = false;
                            editor.results = execution.results.clone();
                            editor.result_editors = result_editors;
                            editor.active_result_editor = active_result_editor;
                            editor.summaries = execution.summaries.clone();
                            editor.error = None;
                            tab.dirty = false;
                        }
                        self.record_query_execution_history(&request, &execution);
                        AppEvent::QueryFinished(tab_id, execution)
                    }
                    Err(error) => {
                        self.record_failed_query_history(&request);
                        let user_error = UserFacingError::from(error);
                        if let Some(tab) = self.find_tab_mut(tab_id)
                            && let TabKind::QueryEditor(editor) = &mut tab.kind
                        {
                            editor.running = false;
                            editor.summaries = Vec::new();
                            editor.result_editors.clear();
                            editor.active_result_editor = None;
                            editor.error = Some(user_error.clone());
                        }
                        self.state.last_error = Some(user_error.clone());
                        AppEvent::Failed(user_error)
                    }
                }
            }
            AppCommand::ExecuteQueryText { tab_id, text } => {
                let command = AppCommand::ExecuteQueryTextWithOptions {
                    tab_id,
                    text,
                    options: self.default_query_execution_options(),
                };
                return self.dispatch(command);
            }
            AppCommand::ExecuteQueryTextWithOptions {
                tab_id,
                text,
                options,
            } => {
                let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                    TabKind::QueryEditor(editor) => Some(QueryRequest {
                        connection_id: editor.connection_id,
                        database: editor.database.clone(),
                        // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
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
                    }),
                    _ => None,
                });

                let Some(request) = request else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                let cancel_flag = self.query_cancel_flag(tab_id);
                let result = self.execute_query_with_cancel(&request, &|| {
                    query_cancel_requested(&cancel_flag)
                });
                self.clear_query_cancel_flag(tab_id);
                match result {
                    Ok(execution) => {
                        let result_editors = query_result_editors(self, &request, &execution);
                        let active_result_editor = result_editors.keys().next().copied();
                        if let Some(tab) = self.find_tab_mut(tab_id)
                            && let TabKind::QueryEditor(editor) = &mut tab.kind
                        {
                            editor.running = false;
                            editor.results = execution.results.clone();
                            editor.result_editors = result_editors;
                            editor.active_result_editor = active_result_editor;
                            editor.summaries = execution.summaries.clone();
                            editor.error = None;
                            tab.dirty = false;
                        }
                        self.record_query_execution_history(&request, &execution);
                        AppEvent::QueryFinished(tab_id, execution)
                    }
                    Err(error) => {
                        self.record_failed_query_history(&request);
                        let user_error = UserFacingError::from(error);
                        if let Some(tab) = self.find_tab_mut(tab_id)
                            && let TabKind::QueryEditor(editor) = &mut tab.kind
                        {
                            editor.running = false;
                            editor.summaries = Vec::new();
                            editor.result_editors.clear();
                            editor.active_result_editor = None;
                            editor.error = Some(user_error.clone());
                        }
                        self.state.last_error = Some(user_error.clone());
                        AppEvent::Failed(user_error)
                    }
                }
            }
            AppCommand::FinishQueryExecution { tab_id, result } => match result {
                Ok(execution) => {
                    self.clear_query_cancel_flag(tab_id);
                    let history_request = self.find_tab(tab_id).and_then(|tab| {
                        let TabKind::QueryEditor(editor) = &tab.kind else {
                            return None;
                        };
                        Some(QueryRequest {
                            connection_id: editor.connection_id,
                            database: editor.database.clone(),
                            // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                            session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                            schema: editor.schema.clone(),
                            text: editor.text.clone(),
                            mode: fluxdb_core::QueryMode::All,
                            options: QueryExecutionOptions::default(),
                        })
                    });
                    let result_editor = history_request
                        .as_ref()
                        .map(|request| query_result_editors(self, request, &execution))
                        .unwrap_or_default();
                    let active_result_editor = result_editor.keys().next().copied();
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        editor.running = false;
                        editor.results = execution.results.clone();
                        editor.result_editors = result_editor;
                        editor.active_result_editor = active_result_editor;
                        editor.summaries = execution.summaries.clone();
                        editor.error = None;
                        tab.dirty = false;
                    } else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "查询编辑器标签页不存在",
                        ));
                    }
                    if let Some(request) = history_request {
                        self.record_query_execution_history(&request, &execution);
                    }
                    AppEvent::QueryFinished(tab_id, execution)
                }
                Err(error) => {
                    self.clear_query_cancel_flag(tab_id);
                    let history_request = self.find_tab(tab_id).and_then(|tab| {
                        let TabKind::QueryEditor(editor) = &tab.kind else {
                            return None;
                        };
                        Some(QueryRequest {
                            connection_id: editor.connection_id,
                            database: editor.database.clone(),
                            // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                            session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                            schema: editor.schema.clone(),
                            text: sql_text_for_execution(
                                &editor.text,
                                Pagination::DEFAULT_LIMIT,
                                self.connection_kind(editor.connection_id)
                                    .unwrap_or(DatabaseKind::MySql),
                            ),
                            mode: fluxdb_core::QueryMode::All,
                            options: QueryExecutionOptions::default(),
                        })
                    });
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        editor.running = false;
                        editor.summaries = Vec::new();
                        editor.result_editors.clear();
                        editor.active_result_editor = None;
                        editor.error = Some(error.clone());
                    }
                    if let Some(request) = history_request {
                        self.record_failed_query_history(&request);
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::FinishQueryResultPageRefresh {
                tab_id,
                result_index,
                page_index,
                result,
            } => match result {
                Ok(execution) => {
                    let Some(page) = execution.results.first().cloned() else {
                        return self.fail(Error::new(ErrorKind::Internal, "刷新结果页没有返回结果表"));
                    };
                    let Some(summary) = first_successful_result_summary(&execution).cloned() else {
                        return self.fail(Error::new(ErrorKind::Internal, "刷新结果页没有返回结果摘要"));
                    };
                    let Some((connection_id, database, schema)) =
                        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                            TabKind::QueryEditor(editor) => Some((
                                editor.connection_id,
                                editor.database.clone(),
                                editor.schema.clone(),
                            )),
                            _ => None,
                        })
                    else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "查询编辑器标签页不存在",
                        ));
                    };
                    let request = QueryRequest {
                        connection_id,
                        database,
                        schema,
                        // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                        session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                        text: summary.sql.clone(),
                        mode: fluxdb_core::QueryMode::Selection,
                        options: QueryExecutionOptions::default(),
                    };
                    let mut refreshed_result_editor =
                        query_result_editors(self, &request, &execution).remove(&0);
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        if page_index >= editor.results.len() {
                            return self.fail(Error::new(ErrorKind::Internal, "结果页不存在"));
                        }
                        editor.running = false;
                        editor.results[page_index] = page.clone();
                        if let Some(result_editor) = refreshed_result_editor.as_mut() {
                            result_editor.cell_detail_panel = editor
                                .result_editors
                                .get(&page_index)
                                .map(|editor| editor.cell_detail_panel.clone())
                                .unwrap_or_default();
                        }
                        if let Some(result_editor) = refreshed_result_editor {
                            editor.result_editors.insert(page_index, result_editor);
                            editor.active_result_editor = Some(page_index);
                        } else {
                            editor.result_editors.remove(&page_index);
                            if editor.active_result_editor == Some(page_index) {
                                editor.active_result_editor =
                                    editor.result_editors.keys().next().copied();
                            }
                        }
                        if let Some(summary_index) =
                            query_result_summary_index(&editor.summaries, result_index)
                        {
                            editor.summaries[summary_index] = summary;
                        }
                        editor.error = None;
                        tab.dirty = false;
                    } else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "查询编辑器标签页不存在",
                        ));
                    }
                    AppEvent::QueryResultPageRefreshed {
                        tab_id,
                        result_index,
                        page_index,
                        page,
                    }
                }
                Err(error) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        editor.running = false;
                        editor.error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::ActivateQueryResultEditor { tab_id, page_index } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    set_active_query_result_editor(editor, page_index);
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::LoadUserAdminUsers(tab_id) => {
                match self.load_user_admin_users(tab_id) {
                    Ok(users) => AppEvent::UserAdminUsersLoaded(tab_id, users),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::StartUserAdminUsersLoad(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.loading_users = true;
                    admin.users_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::FinishUserAdminUsersLoad { tab_id, result } => match result {
                Ok(users) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        admin.loading_users = false;
                        admin.users = users.clone();
                        Self::reset_user_admin_selection_after_users_load(admin, &users);
                        admin.users_error = None;
                        AppEvent::UserAdminUsersLoaded(tab_id, users)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        admin.loading_users = false;
                        admin.users.clear();
                        admin.users_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::LoadUserAdminGrants { tab_id, user } => {
                match self.load_user_admin_grants(tab_id, &user) {
                    Ok(grants) => AppEvent::UserAdminGrantsLoaded(tab_id, grants),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::LoadUserAdminMemberGrants { tab_id, role } => {
                match self.load_user_admin_member_grants(tab_id, &role) {
                    Ok(members) => AppEvent::UserAdminMemberGrantsLoaded(tab_id, members),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::StartUserAdminGrantsLoad { tab_id, user } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.selected_user = Some(user);
                    admin.loading_grants = true;
                    admin.grants_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::FinishUserAdminGrantsLoad {
                tab_id,
                user,
                result,
            } => match result {
                Ok(grants) => {
                    let privilege_grants = self
                        .user_admin_provider_for_tab(tab_id)
                        .map(|provider| provider.privilege_grants_from_grants(&grants))
                        .unwrap_or_default();
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        if admin.selected_user.as_ref() != Some(&user) {
                            return AppEvent::TabActivated(tab_id);
                        }
                        admin.loading_grants = false;
                        admin.grants = grants.clone();
                        admin.grants_loaded_user = Some(user);
                        admin.set_privilege_grants(privilege_grants);
                        admin.role_membership_edits.clear();
                        admin.grants_error = None;
                        AppEvent::UserAdminGrantsLoaded(tab_id, grants)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        if admin.selected_user.as_ref() != Some(&user) {
                            return AppEvent::TabActivated(tab_id);
                        }
                        admin.loading_grants = false;
                        admin.grants.clear();
                        admin.grants_loaded_user = None;
                        admin.privilege_rows.clear();
                        admin.base_privilege_rows.clear();
                        admin.grants_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::StartUserAdminMemberGrantsLoad { tab_id, role } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.selected_user = Some(role);
                    admin.loading_member_grants = true;
                    admin.member_grants_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::FinishUserAdminMemberGrantsLoad {
                tab_id,
                role,
                result,
            } => match result {
                Ok(members) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        if admin.selected_user.as_ref() != Some(&role) {
                            return AppEvent::TabActivated(tab_id);
                        }
                        admin.loading_member_grants = false;
                        admin.member_grants = members.clone();
                        admin.member_grants_loaded_role = Some(role);
                        admin.member_grant_edits.clear();
                        admin.member_grants_error = None;
                        AppEvent::UserAdminMemberGrantsLoaded(tab_id, members)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        if admin.selected_user.as_ref() != Some(&role) {
                            return AppEvent::TabActivated(tab_id);
                        }
                        admin.loading_member_grants = false;
                        admin.member_grants.clear();
                        admin.member_grants_loaded_role = None;
                        admin.member_grants_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::SelectUserAdminUser { tab_id, user } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.selected_user = Some(user);
                    admin.creating_user = false;
                    admin.grants.clear();
                    admin.grants_loaded_user = None;
                    admin.member_grants.clear();
                    admin.member_grants_loaded_role = None;
                    admin.grants_error = None;
                    admin.member_grants_error = None;
                    admin.role_membership_edits.clear();
                    admin.member_grant_edits.clear();
                    admin.privilege_rows.clear();
                    admin.base_privilege_rows.clear();
                    if let Some(selected) = &admin.selected_user {
                        admin.create_user = selected.user.clone();
                        admin.create_host = selected.host.clone();
                        admin.auth_plugin = selected
                            .plugin
                            .clone()
                            .unwrap_or_else(|| "caching_sha2_password".to_string());
                    }
                    admin.password_expiry_policy = "DEFAULT".to_string();
                    admin.create_password.clear();
                    admin.new_password.clear();
                    admin.reset_advanced_defaults();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::BeginUserAdminCreateUser(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.creating_user = true;
                    admin.search.clear();
                    admin.grants.clear();
                    admin.grants_loaded_user = None;
                    admin.member_grants.clear();
                    admin.member_grants_loaded_role = None;
                    admin.grants_error = None;
                    admin.member_grants_error = None;
                    admin.role_membership_edits.clear();
                    admin.member_grant_edits.clear();
                    admin.privilege_rows.clear();
                    admin.base_privilege_rows.clear();
                    admin.create_user = "new_user".to_string();
                    admin.create_host = "%".to_string();
                    admin.auth_plugin = "caching_sha2_password".to_string();
                    admin.password_expiry_policy = "DEFAULT".to_string();
                    admin.create_password.clear();
                    admin.new_password.clear();
                    admin.reset_advanced_defaults();
                    admin.selected_user = Some(admin.draft_user_identity());
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::EndUserAdminCreateUser(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.creating_user = false;
                    admin.grants.clear();
                    admin.grants_loaded_user = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::BeginUserAdminDeleteUser(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    // 仅在选中了已有用户（非新建草稿态）时允许弹出删除确认。
                    if let Some(selected) = admin.selected_user.clone().filter(|_| !admin.creating_user) {
                        admin.pending_delete_user = Some(selected);
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::CancelUserAdminDeleteUser(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pending_delete_user = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SelectPgRole { tab_id, name } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_selected_role = Some(name.clone());
                    // 切换角色是新的编辑会话：目标选择、成员/授权草稿和预览都不能带到新角色。
                    admin.pg_reset_role_editor_session();
                    // 从基线派生干净草稿：右侧面板始终以草稿渲染；干净草稿不计脏。
                    admin.pg_reset_draft_from_baseline();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgRoleSwitchPending { tab_id, target } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_pending_switch = Some(target);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgCancelSwitchRole(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_pending_switch = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::DiscardPgDraftAndSelect { tab_id, name } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_selected_role = Some(name.clone());
                    admin.pg_reset_role_editor_session();
                    admin.pg_reset_draft_from_baseline();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgBeginCreateRole(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    // 新建也是独立编辑会话；旧角色的权限目标/成员状态不能带入新建表单。
                    admin.pg_selected_role = None;
                    admin.pg_reset_role_editor_session();
                    admin.pg_draft = Some(PgRoleDraft::new_create());
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgCancelDraft(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_draft = None;
                    admin.pg_membership_edits.clear();
                    admin.pg_grant_edits.clear();
                    admin.pg_plan_preview = None;
                    admin.pg_plan_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgDraftName { tab_id, name } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    if let Some(draft) = admin.pg_draft.as_mut() {
                        draft.name = name;
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgDraftCanLogin { tab_id, can_login } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    if let Some(draft) = admin.pg_draft.as_mut() {
                        draft.can_login = can_login;
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgDraftAttr { tab_id, field, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id)
                    && let Some(draft) = admin.pg_draft.as_mut()
                {
                    match field {
                        PgDraftAttrField::IsSuperuser => draft.is_superuser = value,
                        PgDraftAttrField::CanCreateDb => draft.can_create_db = value,
                        PgDraftAttrField::CanCreateRole => draft.can_create_role = value,
                        PgDraftAttrField::Inherit => draft.inherit = value,
                        PgDraftAttrField::IsReplication => draft.is_replication = value,
                        PgDraftAttrField::BypassRls => draft.bypass_rls = value,
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgDraftConnectionLimit { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id)
                    && let Some(draft) = admin.pg_draft.as_mut()
                {
                    draft.connection_limit_text = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgDraftValidUntil { tab_id, op } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id)
                    && let Some(draft) = admin.pg_draft.as_mut()
                {
                    draft.valid_until = op;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgDraftPasswordOp { tab_id, op } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id)
                    && let Some(draft) = admin.pg_draft.as_mut()
                {
                    draft.password = op;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgDraftPassword { tab_id, password } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id)
                    && let Some(draft) = admin.pg_draft.as_mut()
                {
                    draft.password = PgPasswordOp::Set(password);
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::StartUserAdminPgRolesLoad(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_roles_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::LoadUserAdminPgRoles(tab_id) => {
                let Some(connection_id) = self
                    .user_admin_state(tab_id)
                    .map(|admin| admin.connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"));
                };
                match self.list_pg_roles_for_connection(connection_id) {
                    Ok(roles) => AppEvent::UserAdminPgRolesLoaded(tab_id, roles),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::FinishUserAdminPgRolesLoad { tab_id, result } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    match result {
                        Ok(roles) => {
                            admin.pg_roles = roles;
                            admin.pg_roles_error = None;
                            // 选中角色被删除后回退到第一个角色；无角色则清空选择。
                            let names: Vec<String> =
                                admin.pg_roles.iter().map(|role| role.name.clone()).collect();
                            if !names.iter().any(|name| Some(name) == admin.pg_selected_role.as_ref()) {
                                admin.pg_selected_role = names.first().cloned();
                            }
                            // 刷新是重新读取服务端基线：右侧会话态与权限目标都回到初始态。
                            admin.pg_reset_role_editor_session();
                            admin.pg_reset_draft_from_baseline();
                        }
                        Err(error) => {
                            // 失败保留旧数据（UI 标记未刷新），显示错误并可重试。
                            admin.pg_roles_error = Some(error.clone());
                        }
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::StartPgMembershipsLoad(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_memberships_loaded = false;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::LoadPgMemberships(tab_id) => {
                let Some(connection_id) = self
                    .user_admin_state(tab_id)
                    .map(|admin| admin.connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"));
                };
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let member_options_supported = role_operation_for_connection(&config, |connector| {
                    connector.supports_member_options(connection_id)
                })
                .unwrap_or(false);
                match self.list_pg_memberships_for_connection(connection_id) {
                    Ok(memberships) => AppEvent::UserAdminPgMembershipsLoaded(
                        tab_id,
                        memberships,
                        member_options_supported,
                    ),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::FinishPgMembershipsLoad { tab_id, result, member_options_supported } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_member_options_supported = Some(member_options_supported);
                    match result {
                        Ok(memberships) => {
                            admin.pg_memberships = memberships;
                            admin.pg_memberships_loaded = true;
                        }
                        Err(error) => {
                            // 失败保留旧数据并标记未加载，不显示为「无成员」。
                            admin.pg_memberships_loaded = false;
                            admin.pg_plan_error = Some(error.clone());
                        }
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgMembershipGrant { tab_id, role, member, admin: admin_option, inherit, set } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    let edit = PgRoleChange::GrantMembership {
                        role: role.clone(),
                        member: member.clone(),
                        admin: admin_option,
                        inherit,
                        set,
                    };
                    // 同 (role, member) 只保留一条最新变更，避免重复提交。
                    if let Some(index) = admin.pg_membership_edit_index(&role, &member) {
                        admin.pg_membership_edits[index] = edit;
                    } else {
                        admin.pg_membership_edits.push(edit);
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgMembershipRevoke { tab_id, role, member } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    let edit = PgRoleChange::RevokeMembership {
                        role: role.clone(),
                        member: member.clone(),
                    };
                    if let Some(index) = admin.pg_membership_edit_index(&role, &member) {
                        admin.pg_membership_edits[index] = edit;
                    } else {
                        admin.pg_membership_edits.push(edit);
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgMembershipRemoveEdit { tab_id, index } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id)
                    && index < admin.pg_membership_edits.len()
                {
                    admin.pg_membership_edits.remove(index);
                    AppEvent::TabActivated(tab_id)
                } else {
                    AppEvent::TabActivated(tab_id)
                }
            }
            AppCommand::SetPgGrantDatabase { tab_id, database } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    // 一期同批只允许一个数据库：切换数据库前 UI 已确认保存或放弃授权草稿；
                    // 目标选择也必须清空，避免旧库的 schema/object 被带到新库。对象种类保留。
                    let kind = admin.pg_grant_kind;
                    admin.pg_reset_grant_target_session();
                    admin.pg_grant_kind = kind;
                    admin.pg_grant_database = database;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::StartPgGrantTargetsLoad(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_loading_targets = true;
                    admin.pg_targets_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::LoadPgGrantTargets { tab_id, database } => {
                let Some(connection_id) = self
                    .user_admin_state(tab_id)
                    .map(|admin| admin.connection_id)
                else {
                    return self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"));
                };
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                match role_operation_for_connection(&config, |connector| {
                    connector.list_grant_targets(connection_id, &database)
                }) {
                    Ok(lists) => AppEvent::UserAdminPgGrantTargetsLoaded(tab_id, lists),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::FinishPgGrantTargetsLoad { tab_id, result } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_loading_targets = false;
                    match result {
                        Ok(lists) => admin.pg_grant_targets = Some(lists),
                        Err(error) => admin.pg_targets_error = Some(error),
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgToggleGrant { tab_id, privilege, scope, op, grant_option } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    let edit = match op {
                        PgGrantEditOp::Grant => PgRoleChange::GrantObject {
                            privilege: privilege.clone(),
                            scope: scope.clone(),
                            grantee: admin.pg_effective_grantee_name(),
                            grant_option,
                        },
                        PgGrantEditOp::Revoke => PgRoleChange::RevokeObject {
                            privilege: privilege.clone(),
                            scope: scope.clone(),
                            grantee: admin.pg_effective_grantee_name(),
                        },
                        PgGrantEditOp::RevokeGrantOption => PgRoleChange::RevokeGrantOption {
                            privilege: privilege.clone(),
                            scope: scope.clone(),
                            grantee: admin.pg_effective_grantee_name(),
                        },
                    };
                    if let Some(index) = admin.pg_grant_edit_index(&privilege, &scope) {
                        admin.pg_grant_edits[index] = edit;
                    } else {
                        admin.pg_grant_edits.push(edit);
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgRemoveGrantEdit { tab_id, index } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id)
                    && index < admin.pg_grant_edits.len()
                {
                    admin.pg_grant_edits.remove(index);
                    AppEvent::TabActivated(tab_id)
                } else {
                    AppEvent::TabActivated(tab_id)
                }
            }
            AppCommand::StartPgPlanPreview(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_preview_loading = true;
                    admin.pg_plan_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::LoadPgPlanPreview(tab_id) => match self.build_pg_role_plan(tab_id) {
                Ok(plan) => {
                    if plan.is_empty() {
                        AppEvent::UserAdminPgPlanPreview(tab_id, Vec::new())
                    } else {
                        let connection_id = self
                            .user_admin_state(tab_id)
                            .map(|admin| admin.connection_id);
                        let Some(connection_id) = connection_id else {
                            return self
                                .fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"));
                        };
                        let Some(config) = self.connection_config(connection_id).cloned() else {
                            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                        };
                        match role_operation_for_connection(&config, |connector| {
                            connector.render_role_plan(connection_id, &plan, true)
                        }) {
                            Ok(stmts) => AppEvent::UserAdminPgPlanPreview(tab_id, stmts),
                            Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                        }
                    }
                }
                Err(error) => AppEvent::Failed(UserFacingError::from(error)),
            },
            AppCommand::FinishPgPlanPreview { tab_id, result } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_preview_loading = false;
                    match result {
                        Ok(stmts) => {
                            admin.pg_plan_preview = Some(stmts.clone());
                            admin.pg_plan_preview_masked = admin
                                .pg_draft
                                .as_ref()
                                .is_some_and(|draft| {
                                    matches!(&draft.password, PgPasswordOp::Set(_))
                                });
                            admin.pg_plan_error = None;
                        }
                        Err(error) => {
                            admin.pg_plan_preview = None;
                            admin.pg_plan_error = Some(error);
                        }
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::StartPgPlanApply(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    if admin.pg_save_status == PgRoleSaveStatus::Saving {
                        return AppEvent::TabActivated(tab_id);
                    }
                    admin.pg_save_status = PgRoleSaveStatus::Saving;
                    admin.pg_plan_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ApplyPgRolePlan(tab_id) => {
                // 由 App 构建（校验）计划并单事务应用；结果无论成败都带计划回传 Finish。
                match self.build_pg_role_plan(tab_id) {
                    Ok(plan) => {
                        let Some(connection_id) = self
                            .user_admin_state(tab_id)
                            .map(|admin| admin.connection_id)
                        else {
                            return self
                                .fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"));
                        };
                        let Some(config) = self.connection_config(connection_id).cloned() else {
                            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                        };
                        let result =
                            role_operation_for_connection(&config, |connector| {
                                connector.apply_role_plan(connection_id, &plan)
                            });
                        match result {
                            Ok(_) => {
                                AppEvent::UserAdminPgRolePlanFinished(tab_id, plan, Ok(Vec::new()))
                            }
                            Err(error) => AppEvent::UserAdminPgRolePlanFinished(
                                tab_id,
                                plan,
                                Err(UserFacingError::from(error)),
                            ),
                        }
                    }
                    Err(error) => AppEvent::UserAdminPgRolePlanFinished(
                        tab_id,
                        PgRoleSavePlan {
                            database: None,
                            role_name: String::new(),
                            changes: Vec::new(),
                        },
                        Err(UserFacingError::from(error)),
                    ),
                }
            }
            AppCommand::FinishPgRolePlanApply { tab_id, plan, result } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    match result {
                        Ok(_) => {
                            admin.pg_save_status = PgRoleSaveStatus::Idle;
                            admin.pg_draft = None;
                            admin.pg_membership_edits.clear();
                            admin.pg_grant_edits.clear();
                            admin.pg_plan_preview = None;
                            // 应用成功后对象权限基线已变化，标记过期等待重新读取。
                            admin.pg_loaded_target.clear();
                            // 改名后选中角色跟随新身份。
                            admin.pg_selected_role = Some(plan.role_name.clone());
                        }
                        Err(error) => {
                            // 失败保留草稿；结果不确定时（连接中断）标记待核实，不直接重试。
                            admin.pg_save_status = if error.retryable {
                                PgRoleSaveStatus::Idle
                            } else {
                                PgRoleSaveStatus::NeedsVerify
                            };
                            admin.pg_plan_error = Some(error.clone());
                        }
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgBeginDeleteRole(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    if let Some(name) = admin
                        .pg_selected_role
                        .clone()
                        .filter(|name| !UserAdminState::pg_is_predefined_role(name))
                    {
                        admin.pg_pending_delete = Some(name);
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PgCancelDeleteRole(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_pending_delete = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetPgRoleFilter { tab_id, filter } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_role_filter = filter;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPgGrantTarget {
                tab_id,
                kind,
                schema,
                object,
                signature,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pg_grant_kind = kind;
                    admin.pg_grant_schema = schema;
                    admin.pg_grant_object = object;
                    admin.pg_grant_signature = signature;
                    // 目标已变化：旧请求结果不可用；桌面层同时取消旧 Task，避免悬挂 loading。
                    admin.loading_pg_grants = false;
                    admin.pg_grants_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::LoadUserAdminPgObjectGrants(tab_id) => {
                match self.load_pg_object_grants(tab_id) {
                    Ok(result) => AppEvent::UserAdminPgObjectGrantsLoaded(tab_id, Ok(result)),
                    Err(error) => AppEvent::UserAdminPgObjectGrantsLoaded(
                        tab_id,
                        Err(error.into()),
                    ),
                }
            }
            AppCommand::StartUserAdminPgObjectGrantsLoad(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.loading_pg_grants = true;
                    admin.pg_grants_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::FinishUserAdminPgObjectGrantsLoad {
                tab_id,
                target_fingerprint,
                result,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    // 读取期间目标可能已被用户切换；过期结果不能结束新目标的
                    // loading，也不能覆盖新目标状态。
                    if pg_grant_target_fingerprint(admin) == target_fingerprint {
                        admin.loading_pg_grants = false;
                        match result {
                            Ok((grants, effective)) => {
                                admin.pg_object_grants = Some(grants);
                                admin.pg_effective_grants = effective;
                                admin.pg_grants_error = None;
                                // 记录已加载目标指纹，供 UI 判断选择器目标是否已过期需重取。
                                admin.pg_loaded_target = target_fingerprint;
                            }
                            Err(error) => {
                                admin.pg_object_grants = None;
                                admin.pg_effective_grants.clear();
                                admin.pg_grants_error = Some(error);
                            }
                        }
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SelectUserAdminDetailTab { tab_id, detail_tab } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.active_detail_tab = detail_tab;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSearch { tab_id, search } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.search = search;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPrivilegeDatabase { tab_id, database } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.privilege_database = database;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPrivilegeTable { tab_id, table } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.privilege_table = table;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ToggleUserAdminPrivilege { tab_id, privilege } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    if let Some(index) = admin
                        .selected_privileges
                        .iter()
                        .position(|item| item == &privilege)
                    {
                        admin.selected_privileges.remove(index);
                    } else {
                        admin.selected_privileges.push(privilege);
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminGrantOption { tab_id, enabled } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.grant_option = enabled;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::AddUserAdminPrivilegeRow { tab_id, database } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.add_privilege_row(database);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPrivilegeRowDatabase {
                tab_id,
                row_id,
                database,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_privilege_row_database(row_id, database);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ToggleUserAdminPrivilegeRowPrivilege {
                tab_id,
                row_id,
                privilege,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.toggle_privilege_row_privilege(row_id, privilege);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPrivilegeRowGrantOption {
                tab_id,
                row_id,
                enabled,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_privilege_row_grant_option(row_id, enabled);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminCreateUser { tab_id, user } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.create_user = user;
                    if admin.creating_user {
                        admin.selected_user = Some(admin.draft_user_identity());
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminCreateHost { tab_id, host } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.create_host = host;
                    if admin.creating_user {
                        admin.selected_user = Some(admin.draft_user_identity());
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminAuthPlugin { tab_id, plugin } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.auth_plugin = plugin;
                    if admin.creating_user {
                        admin.selected_user = Some(admin.draft_user_identity());
                    }
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminPasswordExpiryPolicy { tab_id, policy } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.password_expiry_policy = policy;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminCreatePassword { tab_id, password } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.create_password = password;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminNewPassword { tab_id, password } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.new_password = password;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminMaxQueriesPerHour { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.max_queries_per_hour = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminMaxUpdatesPerHour { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.max_updates_per_hour = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminMaxConnectionsPerHour { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.max_connections_per_hour = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminMaxUserConnections { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.max_user_connections = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSslType { tab_id, ssl_type } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.ssl_type = ssl_type;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSslCipher { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.ssl_cipher = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSslIssuer { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.ssl_issuer = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminSslSubject { tab_id, value } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.ssl_subject = value;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminRoleMembershipGranted {
                tab_id,
                role,
                granted,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_role_membership_granted(role, granted);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminRoleMembershipDefault {
                tab_id,
                role,
                default_role,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_role_membership_default(role, default_role);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::SetUserAdminRoleMemberGranted {
                tab_id,
                member,
                granted,
            } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.set_role_member_granted(member, granted);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::PreviewUserAdminSql { tab_id, sql, danger } => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pending_sql = Some(UserAdminPendingSql { sql, danger });
                    admin.apply_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ClearUserAdminPendingSql(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.pending_sql = None;
                    admin.apply_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::StartUserAdminSqlApply(tab_id) => {
                if let Some(admin) = self.user_admin_state_mut(tab_id) {
                    admin.applying = true;
                    admin.apply_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                }
            }
            AppCommand::ApplyUserAdminSql { tab_id, sql } => {
                match self.apply_user_admin_sql(tab_id, &sql) {
                    Ok(()) => AppEvent::UserAdminSqlApplied(tab_id),
                    Err(error) => AppEvent::Failed(UserFacingError::from(error)),
                }
            }
            AppCommand::FinishUserAdminSqlApply { tab_id, result } => match result {
                Ok(()) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        admin.applying = false;
                        admin.pending_sql = None;
                        admin.apply_error = None;
                        admin.creating_user = false;
                        admin.create_password.clear();
                        admin.new_password.clear();
                        admin.grants_loaded_user = None;
                        admin.member_grants.clear();
                        admin.member_grants_loaded_role = None;
                        admin.member_grants_error = None;
                        admin.role_membership_edits.clear();
                        admin.member_grant_edits.clear();
                        admin.base_privilege_rows = admin.privilege_rows.clone();
                        AppEvent::UserAdminSqlApplied(tab_id)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "用户与权限标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(admin) = self.user_admin_state_mut(tab_id) {
                        admin.applying = false;
                        admin.apply_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::CloseTab(tab_id) => {
                if self
                    .state
                    .tabs
                    .iter()
                    .any(|tab| tab.id == tab_id && tab.dirty)
                {
                    self.state.pending_dirty_tab_close = Some(tab_id);
                    return AppEvent::TabCloseRequested(tab_id);
                }

                self.release_tab_query_sessions(&[tab_id]);
                self.state.tabs.retain(|tab| tab.id != tab_id);
                if self.state.pending_dirty_tab_close == Some(tab_id) {
                    self.state.pending_dirty_tab_close = None;
                }
                if self.state.active_tab == Some(tab_id) {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                AppEvent::TabClosed(tab_id)
            }
            AppCommand::CloseTabs(tab_ids) => {
                let tab_ids = tab_ids.into_iter().collect::<BTreeSet<_>>();
                let closed = self
                    .state
                    .tabs
                    .iter()
                    .find(|tab| tab_ids.contains(&tab.id))
                    .map(|tab| tab.id);
                let closed_ids = tab_ids.iter().copied().collect::<Vec<_>>();
                self.release_tab_query_sessions(&closed_ids);
                self.state.tabs.retain(|tab| !tab_ids.contains(&tab.id));
                if self
                    .state
                    .active_tab
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                if self
                    .state
                    .pending_dirty_tab_close
                    .is_some_and(|tab_id| !self.state.tabs.iter().any(|tab| tab.id == tab_id))
                {
                    self.state.pending_dirty_tab_close = None;
                }
                closed
                    .map(AppEvent::TabClosed)
                    .unwrap_or(AppEvent::TabActivated(TabId(0)))
            }
            AppCommand::ConfirmCloseDirtyTab(tab_id) => {
                if self.state.pending_dirty_tab_close != Some(tab_id) {
                    return self.fail(Error::new(ErrorKind::Internal, "没有待关闭的标签页"));
                }

                self.release_tab_query_sessions(&[tab_id]);
                self.state.tabs.retain(|tab| tab.id != tab_id);
                self.state.pending_dirty_tab_close = None;
                if self.state.active_tab == Some(tab_id) {
                    self.state.active_tab = self.state.tabs.last().map(|tab| tab.id);
                }
                AppEvent::TabClosed(tab_id)
            }
            AppCommand::CancelCloseDirtyTab(tab_id) => {
                if self.state.pending_dirty_tab_close == Some(tab_id) {
                    self.state.pending_dirty_tab_close = None;
                }
                AppEvent::TabCloseCancelled(tab_id)
            }
            AppCommand::ActivateTab(tab_id) => {
                if self.state.tabs.iter().any(|tab| tab.id == tab_id) {
                    self.state.active_tab = Some(tab_id);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "标签页不存在"))
                }
            }
            // 回首页：仅取消激活当前标签（active_tab 置空），已打开的标签保留在标签栏不关闭。
            // 事件用 TabActivated(TabId(0)) 空态约定（与 CloseTabs 一致），供桌面清理局部状态。
            AppCommand::DeactivateTab => {
                self.state.active_tab = None;
                AppEvent::TabActivated(TabId(0))
            }
            AppCommand::PinTab(tab_id) => {
                if let Some(index) = self.state.tabs.iter().position(|tab| tab.id == tab_id) {
                    let tab = self.state.tabs.remove(index);
                    self.state.tabs.insert(0, tab);
                    self.state.active_tab = Some(tab_id);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "标签页不存在"))
                }
            }
            AppCommand::ReplaceQueryHistory(history) => {
                self.state.query_history = history;
                AppEvent::SettingsSaved
            }
            AppCommand::SaveSettings(settings) => {
                self.state.settings = settings;
                AppEvent::SettingsSaved
            }
            AppCommand::SetTabDirty { tab_id, dirty } => {
                if let Some(tab) = self.state.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                    tab.dirty = dirty;
                }
                AppEvent::SettingsSaved
            }
            AppCommand::ClearCompletionIndexCache => {
                self.clear_completion_cache();
                if let Some(storage) = &self.completion_index_storage {
                    let _ = storage.clear_completion_indexes();
                }
                AppEvent::SettingsSaved
            }
            // 建表/设计表/表操作命令经域路由转发至 dispatch_table_command，
            // 避免在此巨型 match 中持续膨胀（未在此处显式匹配的其余命令兜底转发）。
            _ => self.dispatch_table_command(command),
        }
    }
}

impl AppController {
    /// 释放与这些标签页绑定的独占查询会话（关闭标签时调用）。
    ///
    /// 标签的查询会话 id 就是 `tab_id`（见各 `QueryRequest` 构造点）；标签关闭后连接由
    /// 服务端回收，未提交事务随之回滚——不这样做，连接会一直挂到空闲 TTL 才消失。
    fn release_tab_query_sessions(&mut self, tab_ids: &[TabId]) {
        // 关标签先置位取消标志（标签关了就不该再让服务端跑下去），再从表里移除；
        // 仍在收尾的执行线程持的是自己的 `Arc`，置位照常生效。
        if let Ok(mut flags) = self.query_cancel_flags.lock() {
            for tab_id in tab_ids {
                if let Some(flag) = flags.remove(tab_id) {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        for tab in &self.state.tabs {
            if !tab_ids.contains(&tab.id) {
                continue;
            }
            if let TabKind::QueryEditor(editor) = &tab.kind {
                fluxdb_connectors::pg_close_query_session(
                    editor.connection_id,
                    fluxdb_core::QuerySessionId(tab.id.0),
                );
                settle_closed_query_history(&mut self.state.query_history, editor.connection_id, Some(fluxdb_core::QuerySessionId(tab.id.0)));
            }
        }
    }

    fn default_query_execution_options(&self) -> QueryExecutionOptions {
        QueryExecutionOptions {
            // 直接透传 page_size：0 表示「不限制」，不能经 Pagination::new 的
            // clamp(1, MAX) 被改成 1。
            page_size: self.state.settings.page_size,
            ..QueryExecutionOptions::default()
        }
    }

    /// 解析本次 Redis Workbench 要执行的命令文本：给了 `execution_id` 取结果区那条记录，
    /// 否则取顶部草稿；标签页不存在或不是 Workbench 时返回 None。
    fn redis_workbench_text(&self, tab_id: TabId, execution_id: Option<u64>) -> Option<String> {
        let tab = self.find_tab(tab_id)?;
        let TabKind::RedisWorkbench(workbench) = &tab.kind else {
            return None;
        };
        match execution_id {
            Some(execution_id) => workbench
                .executions
                .iter()
                .find(|execution| execution.id == execution_id)
                .map(|execution| execution.text.clone()),
            None => Some(workbench.text.clone()),
        }
    }

    /// 解析一次执行的目标作用域与命令文本。
    ///
    /// 文本由本函数按 `execution_id` 解析、不接 UI 传值：`RunRedisWorkbench` 跑在后台线程的
    /// 控制器副本上，而 UI 在派发前已把草稿清空，只有副本自己知道本次该执行什么。
    fn redis_workbench_run_scope(
        &self,
        tab_id: TabId,
        execution_id: Option<u64>,
    ) -> Option<(CommandExecutionTarget, String)> {
        let tab = self.find_tab(tab_id)?;
        let target = match &tab.kind {
            TabKind::RedisWorkbench(workbench) => CommandExecutionTarget::Redis {
                connection_id: workbench.connection_id,
                database: workbench.database,
            },
            _ => return None,
        };
        let text = self.redis_workbench_text(tab_id, execution_id)?;
        Some((target, text))
    }

    fn apply_create_table(&self, tab_id: TabId) -> fluxdb_core::Result<()> {
        let create = self
            .find_tab(tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::CreateTable(create) => Some(create.clone()),
                _ => None,
            })
            .ok_or_else(|| Error::new(ErrorKind::Internal, "新建表标签页不存在"))?;
        if let Some(message) = create.validation_error() {
            return Err(Error::new(ErrorKind::Query, message));
        }
        // PG 的 CREATE TABLE / ALTER 计划是同批事务性语句：走单批路径（与设计模式一致），
        // 不套用 MySQL/SQLite 的「建表 SQL + 单独触发器」拆分（那会丢掉 PG 触发器）。
        if create.is_design() || create.database_kind == DatabaseKind::Postgres {
            if create.database_kind == DatabaseKind::Postgres {
                self.ensure_postgres_design_not_stale(&create)?;
            }
            let sql = create
                .sql_preview()
                .map_err(|message| Error::new(ErrorKind::Query, message))?;
            return self.apply_postgres_or_design_sql(&create, &sql);
        }
        let mut base_create = create.clone();
        base_create.triggers.clear();
        let base_sql = base_create
            .sql_preview()
            .map_err(|message| Error::new(ErrorKind::Query, message))?;
        let mut statements = vec![(base_sql, true)];
        let trigger_statements = match create.database_kind {
            DatabaseKind::MySql | DatabaseKind::TiDb => create_table_mysql_trigger_statements(&create),
            DatabaseKind::Sqlite => create_table_sqlite_trigger_statements(&create),
            _ => Ok(Vec::new()),
        }
        .map_err(|message| Error::new(ErrorKind::Query, message))?;
        statements.extend(trigger_statements.into_iter().map(|sql| (sql, false)));

        for (sql, split_statements) in statements {
            let request = QueryRequest {
                connection_id: create.connection_id,
                database: create.database.clone(),
                session_id: None,
                // 建表向导的 schema 作用域随状态下传（PG 显式 schema 时与会话 search_path 对齐）。
                schema: create
                    .schema
                    .trim()
                    .is_empty()
                    .then_some(None)
                    .unwrap_or_else(|| Some(create.schema.trim().to_string())),
                text: sql,
                mode: fluxdb_core::QueryMode::All,
                options: QueryExecutionOptions {
                    continue_on_error: false,
                    split_statements,
                    ..QueryExecutionOptions::default()
                },
            };
            let execution = self.execute_query(&request)?;
            if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                return Err(Error::new(ErrorKind::Query, summary.message.clone()));
            }
            self.mark_query_history_completion_dirty(&request, &request.text);
        }
        Ok(())
    }

    /// 执行 PG 建表/设计 SQL：同一事务内提交，失败整体回滚（§9.2）。
    ///
    /// 生成的语句都是事务性 DDL（不含 CREATE INDEX CONCURRENTLY 等），因此统一包裹
    /// `BEGIN … COMMIT`；任一条失败时不执行 COMMIT，连接释放即回滚，不会留下半套结构。
    fn apply_postgres_or_design_sql(
        &self,
        create: &CreateTableState,
        sql: &str,
    ) -> fluxdb_core::Result<()> {
        let text = format!("BEGIN;
{sql}
COMMIT;");
        let request = QueryRequest {
            connection_id: create.connection_id,
            database: create.database.clone(),
            session_id: None,
            schema: create
                .schema
                .trim()
                .is_empty()
                .then_some(None)
                .unwrap_or_else(|| Some(create.schema.trim().to_string())),
            text,
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions {
                continue_on_error: false,
                split_statements: true,
                ..QueryExecutionOptions::default()
            },
        };
        let execution = self.execute_query(&request)?;
        if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
            return Err(Error::new(ErrorKind::Query, summary.message.clone()));
        }
        self.mark_query_history_completion_dirty(&request, &request.text);
        Ok(())
    }

    /// 外部 DDL 保护：保存前重查表结构，与打开设计器时的 DDL 不一致就拒绝应用（§9.2）。
    ///
    /// 不拿过期快照覆盖别人的改动；用户刷新后重新预览即可继续。
    fn ensure_postgres_design_not_stale(&self, create: &CreateTableState) -> fluxdb_core::Result<()> {
        let CreateTableMode::Design {
            object,
            original_ddl: Some(original_ddl),
            ..
        } = &create.mode
        else {
            return Ok(());
        };
        let config = self
            .connection_config(create.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let current = table_ddl_for_connection(config, object)?;
        // 打开设计器时基线经 format_sql_text_for_dialect 规整（load_table_info_for_connection 的 Ddl 路径）。
        // 校验侧必须用同一步规整：否则原始 DDL 与规整 DDL 对同一表逐字不等，会把「自己保存的改动」误判为外部变化。
        let current = format_sql_text_for_dialect(&current, config.kind);
        if current.trim() != original_ddl.trim() {
            tracing::warn!(
                target: "gdb_create_table",
                connection_id = ?create.connection_id,
                table = %object.name,
                "表结构已在外部变化，拒绝应用过期设计"
            );
            return Err(Error::new(
                ErrorKind::Query,
                "表结构已在外部变化，请重新打开设计器并确认预览后再保存",
            ));
        }
        Ok(())
    }
}

/// 查询取消判定：标志未登记（非编辑器执行/后台任务）视为不可取消，与旧行为一致。
fn query_cancel_requested(flag: &Option<Arc<std::sync::atomic::AtomicBool>>) -> bool {
    flag.as_ref()
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
}

fn query_result_summary_has_result_tab(summary: &QueryExecutionSummary) -> bool {
    summary.kind == fluxdb_core::QueryStatementKind::ResultSet || !summary.success
}

fn query_result_summary_index(
    summaries: &[QueryExecutionSummary],
    result_index: usize,
) -> Option<usize> {
    let mut entry_index = 0usize;
    for (summary_index, summary) in summaries.iter().enumerate() {
        if !query_result_summary_has_result_tab(summary) {
            continue;
        }
        if entry_index == result_index {
            return Some(summary_index);
        }
        entry_index += 1;
    }
    None
}

fn first_successful_result_summary(
    execution: &QueryExecutionResult,
) -> Option<&QueryExecutionSummary> {
    execution
        .summaries
        .iter()
        .find(|summary| summary.kind == fluxdb_core::QueryStatementKind::ResultSet && summary.success)
}
