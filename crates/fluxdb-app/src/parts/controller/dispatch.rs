impl AppController {
    pub fn dispatch(&mut self, command: AppCommand) -> AppEvent {
        if should_clear_completion_cache(&command) {
            self.clear_completion_cache();
        }

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
                        connection.objects = objects;
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
                                    connection.objects = objects.clone();
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
                let pagination = Pagination::default();

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
                self.open_query_editor(connection_id, None)
            }
            AppCommand::OpenUserAdmin(connection_id) => self.open_user_admin(connection_id),
            AppCommand::OpenSettings => self.open_settings(),
            AppCommand::OpenQueryEditorInDatabase {
                connection_id,
                database,
            } => self.open_query_editor(connection_id, database),
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
            AppCommand::OpenCreateTable {
                connection_id,
                database,
            } => {
                let Some(config) = self.connection_config(connection_id) else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let database_kind = config.kind;
                let tab_id = self.next_tab_id();
                let create = CreateTableState::new(connection_id, database, database_kind);
                self.push_tab(TabState {
                    id: tab_id,
                    title: create.tab_title(),
                    kind: TabKind::CreateTable(create),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::OpenDesignTable(object) => {
                let Some(config) = self.connection_config(object.connection_id) else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let columns = match list_completion_columns_for_connection(
                    &config,
                    object.database.as_deref(),
                    object.schema.as_deref(),
                    &object.name,
                ) {
                    Ok(columns) => columns,
                    Err(error) => return self.fail(error),
                };
                let indexes = match load_table_info_for_connection(&config, &object, TableInfoTab::Indexes) {
                    Ok(TableInfoResult::Indexes(indexes)) => indexes,
                    Ok(_) => Vec::new(),
                    Err(error) => return self.fail(error),
                };
                let foreign_keys = match load_table_info_for_connection(&config, &object, TableInfoTab::ForeignKeys) {
                    Ok(TableInfoResult::ForeignKeys(foreign_keys)) => foreign_keys,
                    Ok(_) => Vec::new(),
                    Err(error) => return self.fail(error),
                };
                let triggers = match load_table_info_for_connection(&config, &object, TableInfoTab::Triggers) {
                    Ok(TableInfoResult::Triggers(triggers)) => triggers,
                    Ok(_) => Vec::new(),
                    Err(error) => return self.fail(error),
                };
                let ddl = match load_table_info_for_connection(&config, &object, TableInfoTab::Ddl) {
                    Ok(TableInfoResult::Ddl(ddl)) => Some(ddl),
                    Ok(_) => None,
                    Err(error) => return self.fail(error),
                };
                let database_kind = config.kind;
                let tab_id = self.next_tab_id();
                let create = CreateTableState::design(
                    object,
                    database_kind,
                    columns,
                    indexes,
                    foreign_keys,
                    triggers,
                    ddl,
                );
                self.push_tab(TabState {
                    id: tab_id,
                    title: create.tab_title(),
                    kind: TabKind::CreateTable(create),
                    dirty: false,
                });
                AppEvent::TabOpened(tab_id)
            }
            AppCommand::RenameTable { object, new_name } => {
                let Some(config) = self.connection_config(object.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let new_name = new_name.trim().to_string();
                let sql = match rename_table_sql_preview(config.kind, &object.name, &new_name) {
                    Ok(sql) => sql,
                    Err(message) => return self.fail(Error::new(ErrorKind::Query, message)),
                };
                let request = QueryRequest {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    text: sql,
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions {
                        continue_on_error: false,
                        split_statements: true,
                        ..QueryExecutionOptions::default()
                    },
                };
                let execution = match self.execute_query(&request) {
                    Ok(execution) => execution,
                    Err(error) => return self.fail(error),
                };
                if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                    return self.fail(Error::new(ErrorKind::Query, summary.message.clone()));
                }

                let mut renamed = object.clone();
                renamed.name = new_name.clone();
                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == object.connection_id)
                {
                    for summary in &mut connection.objects {
                        if summary.path == object {
                            summary.path = renamed.clone();
                        }
                    }
                }
                for tab in &mut self.state.tabs {
                    if let TabKind::DataEditor(editor) = &mut tab.kind
                        && editor.object == object
                    {
                        editor.object = renamed.clone();
                        editor.page = None;
                        editor.original_page = None;
                        editor.changes = None;
                        editor.loading = true;
                        editor.table_info = TableInfoState::default();
                        tab.title = new_name.clone();
                        tab.dirty = false;
                    }
                }
                self.state.last_error = None;
                AppEvent::TableRenamed { object, new_name }
            }
            AppCommand::CopyTable {
                object,
                new_name,
                copy_data,
            } => {
                let Some(config) = self.connection_config(object.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let new_name = new_name.trim().to_string();
                let source_ddl = if config.kind == DatabaseKind::Sqlite {
                    match self.load_table_ddl(&object) {
                        Ok(ddl) => Some(ddl),
                        Err(error) => return self.fail(error),
                    }
                } else {
                    None
                };
                let sql = match copy_table_sql_preview_with_source_ddl(
                    config.kind,
                    &object.name,
                    &new_name,
                    copy_data,
                    source_ddl.as_deref(),
                ) {
                    Ok(sql) => sql,
                    Err(message) => return self.fail(Error::new(ErrorKind::Query, message)),
                };
                let request = QueryRequest {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    text: sql,
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions {
                        continue_on_error: false,
                        split_statements: true,
                        ..QueryExecutionOptions::default()
                    },
                };
                let execution = match self.execute_query(&request) {
                    Ok(execution) => execution,
                    Err(error) => return self.fail(error),
                };
                if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                    return self.fail(Error::new(ErrorKind::Query, summary.message.clone()));
                }

                let mut copied = object.clone();
                copied.name = new_name.clone();
                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == object.connection_id)
                    && !connection
                        .objects
                        .iter()
                        .any(|summary| summary.path == copied)
                {
                    connection.objects.push(ObjectSummary {
                        path: copied,
                        rows: None,
                        modified_at: None,
                        comment: None,
                    });
                }
                self.state.last_error = None;
                AppEvent::TableCopied { object, new_name }
            }
            AppCommand::DropTable {
                object,
                foreign_key_check,
            } => {
                let Some(config) = self.connection_config(object.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let sql = match drop_table_sql_preview(config.kind, &object.name, foreign_key_check)
                {
                    Ok(sql) => sql,
                    Err(message) => return self.fail(Error::new(ErrorKind::Query, message)),
                };
                let request = QueryRequest {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    text: sql,
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions {
                        continue_on_error: false,
                        split_statements: true,
                        ..QueryExecutionOptions::default()
                    },
                };
                let execution = match self.execute_query(&request) {
                    Ok(execution) => execution,
                    Err(error) => return self.fail(error),
                };
                if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                    return self.fail(Error::new(ErrorKind::Query, summary.message.clone()));
                }

                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == object.connection_id)
                {
                    connection.objects.retain(|summary| summary.path != object);
                }
                self.state.tabs.retain(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => editor.object != object,
                    TabKind::CreateTable(create) => match &create.mode {
                        CreateTableMode::Design { object: design_object, .. } => {
                            design_object != &object
                        }
                        CreateTableMode::Create => true,
                    },
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
                AppEvent::TableDropped(object)
            }
            AppCommand::TruncateTable {
                object,
                foreign_key_check,
            } => {
                let Some(config) = self.connection_config(object.connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let sql =
                    match truncate_table_sql_preview(config.kind, &object.name, foreign_key_check)
                    {
                    Ok(sql) => sql,
                    Err(message) => return self.fail(Error::new(ErrorKind::Query, message)),
                    };
                let request = QueryRequest {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    text: sql,
                    mode: fluxdb_core::QueryMode::All,
                    options: QueryExecutionOptions {
                        continue_on_error: false,
                        split_statements: true,
                        ..QueryExecutionOptions::default()
                    },
                };
                let execution = match self.execute_query(&request) {
                    Ok(execution) => execution,
                    Err(error) => return self.fail(error),
                };
                if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                    return self.fail(Error::new(ErrorKind::Query, summary.message.clone()));
                }

                self.state.last_error = None;
                AppEvent::TableTruncated(object)
            }
            AppCommand::StartCreateTableApply(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    if let Some(message) = create.validation_error() {
                        return self.fail(Error::new(ErrorKind::Query, message));
                    }
                    create.applying = true;
                    create.apply_error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::ApplyCreateTable(tab_id) => match self.apply_create_table(tab_id) {
                Ok(()) => AppEvent::CreateTableApplied(tab_id),
                Err(error) => AppEvent::Failed(UserFacingError::from(error)),
            },
            AppCommand::FinishCreateTableApply { tab_id, result } => match result {
                Ok(()) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::CreateTable(create) = &mut tab.kind
                    {
                        create.applying = false;
                        create.apply_error = None;
                        tab.dirty = false;
                        AppEvent::CreateTableApplied(tab_id)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                    }
                }
                Err(error) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::CreateTable(create) = &mut tab.kind
                    {
                        create.applying = false;
                        create.apply_error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::SetCreateTableField {
                tab_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_field(field, value);
                    tab.title = create.tab_title();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableOptionField {
                tab_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_option_field(field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::ToggleCreateTablePartitionEnabled(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.toggle_partition_enabled();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTablePartitionField {
                tab_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_partition_field(field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableTab { tab_id, create_tab } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                if create_tab == CreateTableTab::Ddl && !create.is_design() {
                    return AppEvent::TabActivated(tab_id);
                }
                create.active_tab = create_tab;
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SelectCreateTableColumn { tab_id, column_id } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_column(column_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableColumn(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_column();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableColumnUp { tab_id, column_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_column_up(column_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableColumnDown { tab_id, column_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_column_down(column_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableColumn { tab_id, column_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_column(column_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableColumnField {
                tab_id,
                column_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_column_field(column_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::ToggleCreateTableColumnFlag {
                tab_id,
                column_id,
                flag,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.toggle_column_flag(column_id, flag);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableIndex { tab_id, index_id } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_index(index_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableIndex(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_index();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableIndexUp { tab_id, index_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_index_up(index_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableIndexDown { tab_id, index_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_index_down(index_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableIndex { tab_id, index_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_index(index_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableIndexField {
                tab_id,
                index_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_index_field(index_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::AddCreateTableIndexColumn { tab_id, index_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_index_column(index_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableIndexColumnUp {
                tab_id,
                index_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_index_column_up(index_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableIndexColumnDown {
                tab_id,
                index_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_index_column_down(index_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableIndexColumn {
                tab_id,
                index_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_index_column(index_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableIndexColumnField {
                tab_id,
                index_id,
                column_index,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_index_column_field(index_id, column_index, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableCheck { tab_id, check_id } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_check(check_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableCheck(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_check();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableCheckUp { tab_id, check_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_check_up(check_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableCheckDown { tab_id, check_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_check_down(check_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableCheck { tab_id, check_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_check(check_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableCheckField {
                tab_id,
                check_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_check_field(check_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::ToggleCreateTableCheckNotEnforced { tab_id, check_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.toggle_check_not_enforced(check_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableForeignKey {
                tab_id,
                foreign_key_id,
            } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_foreign_key(foreign_key_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableForeignKey(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_foreign_key();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyUp {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_up(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyDown {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_down(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableForeignKey {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_foreign_key(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableForeignKeyField {
                tab_id,
                foreign_key_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_foreign_key_field(foreign_key_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::StartCreateTableReferenceColumnsLoad {
                tab_id,
                foreign_key_id,
            } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.start_foreign_key_reference_columns_load(foreign_key_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::LoadCreateTableReferenceColumns {
                tab_id,
                foreign_key_id,
            } => match self.load_create_table_reference_columns(tab_id, foreign_key_id) {
                Ok(columns) => AppEvent::CreateTableReferenceColumnsLoaded {
                    tab_id,
                    foreign_key_id,
                    columns,
                },
                Err(error) => AppEvent::Failed(UserFacingError::from(error)),
            },
            AppCommand::FinishCreateTableReferenceColumnsLoad {
                tab_id,
                foreign_key_id,
                result,
            } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.finish_foreign_key_reference_columns_load(foreign_key_id, result.clone());
                match result {
                    Ok(columns) => AppEvent::CreateTableReferenceColumnsLoaded {
                        tab_id,
                        foreign_key_id,
                        columns,
                    },
                    Err(error) => {
                        self.state.last_error = Some(error.clone());
                        AppEvent::Failed(error)
                    }
                }
            }
            AppCommand::AddCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_foreign_key_column(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::AddCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_foreign_key_referenced_column(foreign_key_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyReferencedColumnUp {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_referenced_column_up(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyReferencedColumnDown {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_referenced_column_down(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_foreign_key_referenced_column(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableForeignKeyReferencedColumn {
                tab_id,
                foreign_key_id,
                column_index,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_foreign_key_referenced_column(foreign_key_id, column_index, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyColumnUp {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_column_up(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableForeignKeyColumnDown {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_foreign_key_column_down(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id,
                column_index,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_foreign_key_column(foreign_key_id, column_index);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableForeignKeyColumn {
                tab_id,
                foreign_key_id,
                column_index,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_foreign_key_column(foreign_key_id, column_index, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SelectCreateTableTrigger { tab_id, trigger_id } => {
                let Some(create) = self.find_create_table_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"));
                };
                create.select_trigger(trigger_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::AddCreateTableTrigger(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.add_trigger();
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableTriggerUp { tab_id, trigger_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_trigger_up(trigger_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::MoveCreateTableTriggerDown { tab_id, trigger_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.move_trigger_down(trigger_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::RemoveCreateTableTrigger { tab_id, trigger_id } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.remove_trigger(trigger_id);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableTriggerField {
                tab_id,
                trigger_id,
                field,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_trigger_field(trigger_id, field, value);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }
            AppCommand::SetCreateTableTriggerEvent {
                tab_id,
                trigger_id,
                event,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::CreateTable(create) = &mut tab.kind
                {
                    create.set_trigger_event(trigger_id, event);
                    tab.dirty = true;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "新建表标签页不存在"))
                }
            }

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
            AppCommand::ExecuteRedisWorkbench(tab_id) => {
                // 从 Workbench 标签页读取当前输入文本（独立于运行状态），
                // 再通过共享执行入口发起执行，并清空顶部输入框。
                let text = self
                    .find_tab(tab_id)
                    .and_then(|tab| match &tab.kind {
                        TabKind::RedisWorkbench(workbench) => Some(workbench.text.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                self.execute_redis_workbench(tab_id, text, true)
            }
            AppCommand::FinishRedisWorkbenchExecution { tab_id, result } => match result {
                Ok(mut execution) => {
                    let Some(tab) = self.find_tab_mut(tab_id) else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "Redis Workbench 标签页不存在",
                        ));
                    };
                    if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                        workbench.running = false;
                        workbench.error = None;
                        // 追加一条执行记录：App 层为 connector 产出的 execution 分配自增 id，
                        // 供结果区的单条 Run / Delete 精确定位。
                        execution.id = workbench.next_execution_id;
                        workbench.next_execution_id += 1;
                        workbench.executions.push(execution.clone());
                        // 顶部输入框可能已被 Run 清空；脏标记基于「未执行改动」实时更新。
                        tab.dirty = workbench.has_unsaved_text();
                    } else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "Redis Workbench 标签页不存在",
                        ));
                    }
                    // 命令进入历史体系（execution.target 为 Redis 时记录；持久化由桌面层落盘）。
                    self.record_redis_workbench_history_execution(&execution);
                    AppEvent::RedisWorkbenchFinished(tab_id, execution)
                }
                Err(error) => {
                    let user_error = UserFacingError::from(error);
                    let mut failed_scope = None;
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::RedisWorkbench(workbench) = &mut tab.kind
                    {
                        workbench.running = false;
                        workbench.error = Some(user_error.clone());
                        failed_scope = Some((
                            workbench.connection_id,
                            workbench.database,
                            workbench.text.clone(),
                        ));
                    }
                    if let Some((connection_id, database, text)) = failed_scope {
                        self.record_redis_workbench_history_failure(
                            connection_id,
                            database,
                            &text,
                            &user_error.message,
                        );
                    }
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            AppCommand::RerunRedisWorkbenchRecord { tab_id, execution_id } => {
                // 重跑某条执行记录：按 id 定位记录并复用其命令文本重新执行，
                // 不清空顶部输入框（记录重跑不打扰当前草稿）。
                let text = self
                    .find_tab(tab_id)
                    .and_then(|tab| match &tab.kind {
                        TabKind::RedisWorkbench(workbench) => workbench
                            .executions
                            .iter()
                            .find(|execution| execution.id == execution_id)
                            .map(|execution| execution.text.clone()),
                        _ => None,
                    });
                let Some(text) = text else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 记录不存在",
                    ));
                };
                self.execute_redis_workbench(tab_id, text, false)
            }
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
            AppCommand::ExecuteQuery(tab_id) => {
                let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                    TabKind::QueryEditor(editor) => Some(QueryRequest {
                        connection_id: editor.connection_id,
                        database: editor.database.clone(),
                        text: sql_text_for_execution(&editor.text),
                        mode: fluxdb_core::QueryMode::All,
                        options: self.default_query_execution_options(),
                    }),
                    _ => None,
                });

                let Some(request) = request else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                match self.execute_query(&request) {
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
                        text: sql_text_for_execution(&text),
                        mode: fluxdb_core::QueryMode::Selection,
                        options,
                    }),
                    _ => None,
                });

                let Some(request) = request else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                match self.execute_query(&request) {
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
                    let history_request = self.find_tab(tab_id).and_then(|tab| {
                        let TabKind::QueryEditor(editor) = &tab.kind else {
                            return None;
                        };
                        Some(QueryRequest {
                            connection_id: editor.connection_id,
                            database: editor.database.clone(),
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
                    let history_request = self.find_tab(tab_id).and_then(|tab| {
                        let TabKind::QueryEditor(editor) = &tab.kind else {
                            return None;
                        };
                        Some(QueryRequest {
                            connection_id: editor.connection_id,
                            database: editor.database.clone(),
                            text: sql_text_for_execution(&editor.text),
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
                    let Some((connection_id, database)) =
                        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                            TabKind::QueryEditor(editor) => {
                                Some((editor.connection_id, editor.database.clone()))
                            }
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
            AppCommand::LoadUserAdminUsers(tab_id) => match self.load_user_admin_users(tab_id) {
                Ok(users) => AppEvent::UserAdminUsersLoaded(tab_id, users),
                Err(error) => AppEvent::Failed(UserFacingError::from(error)),
            },
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
        }
    }
}

impl AppController {
    fn default_query_execution_options(&self) -> QueryExecutionOptions {
        QueryExecutionOptions {
            page_size: Pagination::new(0, self.state.settings.page_size).limit,
            ..QueryExecutionOptions::default()
        }
    }

    /// Redis Workbench 共享执行入口：以给定命令文本发起执行。
    ///
    /// `clear_input` 为 true 时（顶部 Run），点击后立刻清空输入框并把
    /// 「未执行改动」指纹置为空文本指纹，避免空输入仍被标脏；为 false 时
    /// （记录重跑）不打扰当前草稿输入。
    fn execute_redis_workbench(&mut self, tab_id: TabId, text: String, clear_input: bool) -> AppEvent {
        let Some(tab) = self.find_tab(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis Workbench 标签页不存在"));
        };
        let TabKind::RedisWorkbench(workbench) = &tab.kind else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis Workbench 标签页不存在"));
        };
        let request = CommandWorkbenchRequest {
            target: CommandExecutionTarget::Redis {
                connection_id: workbench.connection_id,
                database: workbench.database,
            },
            text: text.clone(),
            run_mode: CommandRunMode::Text,
            results_mode: CommandResultsMode::Default,
            batch_size: 0,
            continue_on_error: true,
            source: CommandExecutionSource::Workbench,
        };
        if let Some(tab) = self.find_tab_mut(tab_id)
            && let TabKind::RedisWorkbench(workbench) = &mut tab.kind
        {
            workbench.running = true;
            workbench.error = None;
            if clear_input {
                workbench.text.clear();
                workbench.saved_fingerprint = Some(QueryFingerprint::for_text(""));
                tab.dirty = workbench.has_unsaved_text();
            }
        }
        // 执行单元 = 单条命令：把输入文本交给批量执行入口，得到
        // 「每条命令一个 `CommandWorkbenchExecution`」的列表，逐条追加到
        // 结果区，各自独立占一张卡片（独立 Run / Delete / 时间 / 耗时）。
        match self.execute_command_workbench_commands(&request) {
            Ok(executions) => {
                // 先回写 tab 状态，再统一记录历史（避免与 tab 的可变借用冲突）。
                let mut history_entries = Vec::new();
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::RedisWorkbench(workbench) = &mut tab.kind
                {
                    workbench.running = false;
                    workbench.error = None;
                    for mut execution in executions {
                        // 为 connector 产出的每条 execution 分配自增 id，
                        // 供结果区的单条 Run / Delete 精确定位。
                        execution.id = workbench.next_execution_id;
                        workbench.next_execution_id += 1;
                        history_entries.push(execution.clone());
                        workbench.executions.push(execution);
                    }
                    // 顶部输入框可能已被 Run 清空；脏标记基于「未执行改动」实时更新。
                    tab.dirty = workbench.has_unsaved_text();
                }
                // 每条命令进入历史体系（持久化由桌面层在 state 变化时落盘）。
                for execution in &history_entries {
                    self.record_redis_workbench_history_execution(execution);
                }
                // 结果区由 state 驱动渲染：返回 TabActivated 触发该标签页 UI 重绘，
                // 桌面端 dispatch 包装会统一 cx.notify()。
                AppEvent::TabActivated(tab_id)
            }
            Err(error) => {
                let user_error = UserFacingError::from(error);
                // 失败也要入历史（text 在请求参数中仍可用，失败时输入框已被清空）。
                let mut failed_scope = None;
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::RedisWorkbench(workbench) = &mut tab.kind
                {
                    workbench.running = false;
                    workbench.error = Some(user_error.clone());
                    failed_scope = Some((workbench.connection_id, workbench.database));
                }
                if let Some((connection_id, database)) = failed_scope {
                    self.record_redis_workbench_history_failure(
                        connection_id,
                        database,
                        &text,
                        &user_error.message,
                    );
                }
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
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
        if create.is_design() {
            let sql = create
                .sql_preview()
                .map_err(|message| Error::new(ErrorKind::Query, message))?;
            let request = QueryRequest {
                connection_id: create.connection_id,
                database: create.database.clone(),
                text: sql,
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
            return Ok(());
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
        }
        Ok(())
    }
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
