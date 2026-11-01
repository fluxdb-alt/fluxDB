impl AppController {
    // 连接、数据库与连接树操作独立成帧，避免无关命令长期占用公共分派器的小栈空间。
    fn dispatch_connection_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
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
                let config = draft.into_config(connection_id);
                self.next_connection_id += 1;
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
                    fluxdb_connectors::sqlserver_close_connection_sessions(config.id);
                    fluxdb_connectors::mongo_close_connection_client(config.id);
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
                            let (resource, subscription) = infer_cloud_from_host(&provider, &host);
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
                            options: BTreeMap::new(),
                            redis_profile: Some(profile),
                            mysql_profile: None,
                            postgres_profile: None,
                            dameng_profile: None,
                            sqlserver_profile: None,
                            mongo_profile: None,
                            oracle_profile: None,
                        };
                        AppEvent::RedisConnectionDiscovered(draft)
                    }
                    Err(message) => self.fail(Error::new(ErrorKind::Connection, message)),
                }
            }
            AppCommand::DiscoverMongoConnection { connection_string } => {
                // Mongo 连接串导入：解析出的档案保留 SRV 语义，密码只进 `SecretRef::inline`，
                // 不回写 `options`，也不出现在 `Endpoint` 里。
                match fluxdb_core::MongoConnectionProfile::from_uri(&connection_string) {
                    Ok(profile) => {
                        let (host, port) = profile.dial_endpoint();
                        let name = if host.is_empty() {
                            "MongoDB-新连接".to_string()
                        } else if profile.srv {
                            host.clone()
                        } else {
                            format!("{host}:{port}")
                        };
                        let draft = ConnectionDraft {
                            name,
                            kind: DatabaseKind::MongoDb,
                            endpoint: Endpoint::Tcp {
                                host,
                                port,
                                database: Some(profile.basic.default_database.clone())
                                    .filter(|database| !database.is_empty()),
                            },
                            options: BTreeMap::new(),
                            redis_profile: None,
                            mysql_profile: None,
                            postgres_profile: None,
                            dameng_profile: None,
                            sqlserver_profile: None,
                            mongo_profile: Some(profile),
                            oracle_profile: None,
                        };
                        AppEvent::MongoConnectionDiscovered(draft)
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
                fluxdb_connectors::sqlserver_close_connection_sessions(connection_id);
                fluxdb_connectors::mongo_close_connection_client(connection_id);
                settle_closed_query_history(&mut self.state.query_history, connection_id, None);
                // 断开后这些标签页的结果页里外置的大字段内容没有消费者，随之释放配额。
                if let Some(store) = fluxdb_core::cell_content_store() {
                    for tab in self
                        .state
                        .tabs
                        .iter()
                        .filter(|tab| tab_belongs_to_connection(tab, connection_id))
                    {
                        match &tab.kind {
                            TabKind::QueryEditor(editor) => store.release_owner(
                                &fluxdb_core::result_content_owner(
                                    Some(fluxdb_core::QuerySessionId(tab.id.0)),
                                    editor.connection_id,
                                ),
                            ),
                            TabKind::DataEditor(editor) => store
                                .release_owner(&fluxdb_core::table_content_owner(&editor.object)),
                            _ => {}
                        }
                    }
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
                    matches!(
                        object.path.kind,
                        ObjectKind::Database | ObjectKind::Schema | ObjectKind::RedisDb
                    ) || object.path.database.as_deref().unwrap_or("main") != database
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
                fluxdb_connectors::sqlserver_close_connection_sessions(connection_id);
                fluxdb_connectors::mongo_close_connection_client(connection_id);
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
                AppEvent::SchemaCreated {
                    connection_id,
                    schema,
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
                        if self.state.active_tab.is_some_and(|tab_id| {
                            !self.state.tabs.iter().any(|tab| tab.id == tab_id)
                        }) {
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
            // 私有入口仅由 dispatch 的对应变体分组调用。
            _ => unreachable!("仅连接、数据库与连接树操作命令进入此分发器"),
        }
    }
}
