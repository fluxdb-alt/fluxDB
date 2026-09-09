impl AppController {
    fn load_data_page(
        &self,
        object: &ObjectPath,
        pagination: Pagination,
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        let config = self
            .connection_config(object.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        load_data_for_connection(config, object, pagination, sort, filters)
    }

    fn load_table_info(
        &self,
        object: &ObjectPath,
        tab: TableInfoTab,
    ) -> fluxdb_core::Result<TableInfoResult> {
        let config = self
            .connection_config(object.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        load_table_info_for_connection(config, object, tab)
    }

    pub fn load_table_ddl(&self, object: &ObjectPath) -> fluxdb_core::Result<String> {
        match self.load_table_info(object, TableInfoTab::Ddl)? {
            TableInfoResult::Ddl(ddl) => Ok(ddl),
            _ => Err(Error::new(ErrorKind::Internal, "表结构读取没有返回 DDL")),
        }
    }

    /// 列出某数据库下的对象（表/视图等），用于全量逻辑备份枚举。
    pub fn list_objects(
        &self,
        connection_id: ConnectionId,
        path: Option<&ObjectPath>,
    ) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        list_objects_for_connection(config, path)
    }

    fn load_data_page_command(
        &mut self,
        tab_id: TabId,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
    ) -> AppEvent {
        let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor) => Some((editor.object.clone(), editor.pagination)),
            _ => None,
        });

        let Some((object, pagination)) = request else {
            return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
        };

        if let Some(tab) = self.find_tab_mut(tab_id)
            && let TabKind::DataEditor(editor) = &mut tab.kind
        {
            editor.loading = true;
            editor.error = None;
        }

        match self.load_data_page(&object, pagination, &sort, &filters) {
            Ok(page) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::DataEditor(editor) = &mut tab.kind
                {
                    editor.page = Some(page.clone());
                    editor.original_page = Some(page.clone());
                    editor.changes = None;
                    editor.editing_cell = None;
                    editor.loading = false;
                    editor.error = None;
                    tab.dirty = false;
                }
                AppEvent::DataLoaded(tab_id, page)
            }
            Err(error) => {
                let user_error = UserFacingError::from(error);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::DataEditor(editor) = &mut tab.kind
                {
                    editor.loading = false;
                    editor.error = Some(user_error.clone());
                }
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn load_redis_key_command(&mut self, tab_id: TabId, key: String) -> AppEvent {
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key,
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };

        match self.load_data_page(&object, Pagination::new(0, 1), &[], &[]) {
            Ok(page) => AppEvent::DataLoaded(tab_id, page),
            Err(error) => {
                let user_error = UserFacingError::from(error);
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 惰性补齐 Redis Key 列表元信息（类型/值/大小/TTL）：
    /// 虚拟键名 → 批量取回完整行 → 按「键」列原位合并进当前页（不重建分页/滚动）。
    /// 首屏只拿键名（见 `redis_load_data` RedisDb 分支），此处只补当前可见行缺失的元信息。
    fn load_redis_key_metadata_command(&mut self, tab_id: TabId, keys: Vec<String>) -> AppEvent {
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(editor.object.clone())
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match load_redis_key_metadata_for_connection(&config, &object, &keys) {
            Ok(page) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::DataEditor(editor) = &mut tab.kind
                    && let Some(current) = editor.page.as_mut()
                {
                    // 只合并当前页里仍存在的键；页面刷新后已不在的键直接忽略。
                    for row in page.rows {
                        if let Some(key) = redis_row_key(&page.columns, &row) {
                            replace_redis_key_row(current, &key, row);
                        }
                    }
                }
                AppEvent::RedisKeyMetadataLoaded { tab_id }
            }
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "元信息加载失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 拉取 Redis 连接级运行概览（版本 / 内存 / CPU 采样）。
    /// 先取配置，再执行 `INFO`，成功后将新采样合并进当前连接的概览状态，
    /// 以便把状态交给 live 控制器合并（底栏连接状态摘要展示）。
    fn load_redis_overview_command(
        &mut self,
        connection_id: ConnectionId,
    ) -> AppEvent {
        let Some(config) = self.connection_config(connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).overview() {
            Ok(overview) => {
                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                {
                    connection.redis_overview.apply(overview.clone());
                }
                AppEvent::RedisOverviewLoaded(connection_id, overview)
            }
            Err(error) => {
                let user_error = UserFacingError::from(error);
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 拉取 Redis 服务端版本，作为字段级 TTL 编辑等能力开关。
    /// 非 Redis 连接 / 解析失败 / 命令失败统一回 `version: None`（能力未知，静默降级），
    /// 不弹 toast——能力探测失败不应打断用户操作。
    fn load_redis_server_version_command(
        &mut self,
        connection_id: ConnectionId,
    ) -> AppEvent {
        let Some(config) = self.connection_config(connection_id).cloned() else {
            return AppEvent::RedisServerVersionLoaded(connection_id, None);
        };
        let version = RedisConnector::with_config(config).server_version().ok().flatten();
        AppEvent::RedisServerVersionLoaded(connection_id, version)
    }

    fn finish_redis_key_refresh_command(
        &mut self,
        tab_id: TabId,
        key: String,
        result: std::result::Result<Row, UserFacingError>,
    ) -> AppEvent {
        let row = match result {
            Ok(row) => row,
            Err(error) => {
                self.state.last_error = Some(error.clone());
                return AppEvent::Failed(error);
            }
        };
        let Some(tab) = self.find_tab_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
        };
        let TabKind::DataEditor(editor) = &mut tab.kind else {
            return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
        };
        let Some(page) = editor.page.as_mut() else {
            return self.fail(Error::new(ErrorKind::Internal, "数据页尚未加载"));
        };
        let refreshed_key = redis_row_key(&page.columns, &row).unwrap_or_else(|| key.clone());
        if !replace_redis_key_row(page, &key, row.clone()) {
            return self.fail(Error::new(ErrorKind::Internal, "当前页找不到 Redis Key"));
        }
        if let Some(original_page) = editor.original_page.as_mut() {
            replace_redis_key_row(original_page, &key, row);
        }
        if editor.object.kind == ObjectKind::RedisKey {
            editor.object.name = refreshed_key;
        }
        editor.error = None;
        let page = page.clone();
        AppEvent::DataLoaded(tab_id, page)
    }

    fn apply_redis_key_value_command(
        &mut self,
        tab_id: TabId,
        key: String,
        new_key: String,
        ttl: Option<String>,
        value: Option<String>,
    ) -> AppEvent {
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(editor.object.clone())
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let mut cells = Vec::new();
        if new_key != key {
            cells.push(CellUpdate {
                column: "键".to_string(),
                value: CellValue::Text(new_key.clone()),
            });
        }
        if let Some(ttl) = ttl {
            cells.push(CellUpdate {
                column: "TTL".to_string(),
                value: CellValue::Text(ttl),
            });
        }
        if let Some(value) = value {
            cells.push(CellUpdate {
                column: "值".to_string(),
                value: CellValue::Text(value),
            });
        }
        if cells.is_empty() {
            return self.fail(Error::new(ErrorKind::Internal, "Redis Key 没有需要保存的修改"));
        }

        let changes = DataChangeSet {
            object: object.clone(),
            inserts: Vec::new(),
            updates: vec![RowUpdate {
                identity: RowIdentity {
                    values: [("键".to_string(), CellValue::Text(key.clone()))].into(),
                },
                cells,
            }],
            deletes: Vec::new(),
        };

        match self.apply_data_changes(&object, &changes) {
            Ok(()) => {
                let key_object = ObjectPath {
                    connection_id: object.connection_id,
                    database: object.database.clone(),
                    schema: object.schema.clone(),
                    name: new_key,
                    kind: ObjectKind::RedisKey,
                };
                match self.load_data_page(&key_object, Pagination::new(0, 1), &[], &[]) {
                    Ok(page) => AppEvent::DataLoaded(tab_id, page),
                    Err(error) => {
                        let mut user_error = UserFacingError::from(error);
                        user_error.title = "刷新失败".to_string();
                        self.state.last_error = Some(user_error.clone());
                        AppEvent::Failed(user_error)
                    }
                }
            }
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "保存失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn apply_redis_stream_entry_add_command(
        &mut self,
        tab_id: TabId,
        key: String,
        id: String,
        fields: Vec<(String, String)>,
        maxlen: Option<u64>,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, |connector, object| {
            connector.add_stream_entry(object, &id, &fields, maxlen)
        })
    }

    fn apply_redis_stream_entry_delete_command(
        &mut self,
        tab_id: TabId,
        key: String,
        entry_id: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, |connector, object| {
            connector.delete_stream_entry(object, &entry_id)
        })
    }

    fn apply_redis_set_member_delete_command(
        &mut self,
        tab_id: TabId,
        key: String,
        member: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, |connector, object| {
            connector.delete_set_member(object, &member)
        })
    }

    fn apply_redis_set_member_add_command(
        &mut self,
        tab_id: TabId,
        key: String,
        member: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, |connector, object| {
            connector.add_set_member(object, &member)
        })
    }

    fn load_redis_set_members_command(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    ) -> AppEvent {
        // 与 connector SSCAN COUNT 保持一致的每页大小
        const REDIS_SET_PAGE_SIZE: usize = 200;
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key.clone(),
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).load_set_members(&object, &query, &cursor, REDIS_SET_PAGE_SIZE) {
            Ok(page) => AppEvent::RedisSetMembersLoaded {
                tab_id,
                key,
                query,
                cursor,
                members: page.members,
                next_cursor: page.next_cursor,
                total: page.total,
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "查询失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn load_redis_hash_fields_command(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    ) -> AppEvent {
        const REDIS_HASH_PAGE_SIZE: usize = 200;
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key.clone(),
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).load_hash_fields(&object, &query, &cursor, REDIS_HASH_PAGE_SIZE) {
            Ok(page) => AppEvent::RedisHashFieldsLoaded {
                tab_id,
                key,
                query,
                cursor,
                fields: page.fields,
                next_cursor: page.next_cursor,
                total: page.total,
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "查询失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 完整值弹框懒加载：读取 hash 字段非截断完整原始值。
    /// 走 `load_hash_field_full`（HGET，不截断），结果回传 `RedisHashFieldFullValueLoaded`。
    fn load_redis_hash_field_full_command(
        &mut self,
        tab_id: TabId,
        key: String,
        field: String,
    ) -> AppEvent {
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key.clone(),
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).load_hash_field_full(&object, &field) {
            Ok(Some(value)) => AppEvent::RedisHashFieldFullValueLoaded {
                tab_id,
                field,
                value,
            },
            Ok(None) => AppEvent::Failed(UserFacingError {
                title: "加载失败".to_string(),
                message: "字段不存在或已删除".to_string(),
                detail: None,
                retryable: false,
            }),
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "加载失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// String 详情值加载：`full=false` 取预览（STRLEN+GETRANGE），`full=true` 取完整值（GET）。
    /// 结果回传 [`AppEvent::RedisStringValueLoaded`]，让详情面板脱离列表 200 字符 preview。
    fn load_redis_string_value_command(
        &mut self,
        tab_id: TabId,
        key: String,
        full: bool,
    ) -> AppEvent {
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key.clone(),
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).load_string_value(&object, full) {
            Ok(value) => AppEvent::RedisStringValueLoaded {
                tab_id,
                key,
                value: value.value,
                len: value.len,
                loaded_all: value.loaded_all,
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "加载失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// String / JSON 值下载：取完整原始字节供导出文件（GET / JSON.GET），不经 UTF-8 容错。
    fn download_redis_string_value_command(&mut self, tab_id: TabId, key: String) -> AppEvent {
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key.clone(),
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).download_string_value(&object) {
            Ok(bytes) => AppEvent::RedisStringValueDownloaded {
                tab_id,
                key,
                bytes,
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "下载失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn load_redis_zset_members_command(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    ) -> AppEvent {
        const REDIS_ZSET_PAGE_SIZE: usize = 200;
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key.clone(),
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).load_zset_members(&object, &query, &cursor, REDIS_ZSET_PAGE_SIZE) {
            Ok(page) => AppEvent::RedisZSetMembersLoaded {
                tab_id,
                key,
                query,
                cursor,
                members: page.members,
                next_cursor: page.next_cursor,
                total: page.total,
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "查询失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    // Stream 条目分页查询：与 Hash/Set/ZSet/List 一致走服务端分页，
    // 不再复用 Key 详情里那份只有最新 5 条的预览文本。
    /// 读取 Stream 消费者组概览（只读）。
    fn load_redis_stream_groups_command(&mut self, tab_id: TabId, key: String) -> AppEvent {
        let Some(object) = self.redis_key_object(tab_id, key.clone()) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).load_stream_groups(&object) {
            Ok(groups) => AppEvent::RedisStreamGroupsLoaded {
                tab_id,
                key,
                groups: groups
                    .into_iter()
                    .map(|group| {
                        (
                            group.name,
                            group.consumers,
                            group.pending,
                            group.last_delivered_id,
                            group.consumer_detail,
                        )
                    })
                    .collect(),
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "查询失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 从 tab 拿到 Redis Key 的 ObjectPath（Redis 各类查询命令共用）。
    fn redis_key_object(&self, tab_id: TabId, key: String) -> Option<ObjectPath> {
        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key,
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        })
    }

    fn load_redis_stream_entries_command(
        &mut self,
        tab_id: TabId,
        key: String,
        since_ms: Option<u64>,
        until_ms: Option<u64>,
        cursor: String,
    ) -> AppEvent {
        // 与其他集合类保持一致的每页大小
        const REDIS_STREAM_PAGE_SIZE: usize = 200;
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key.clone(),
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).load_stream_entries(
            &object,
            RedisStreamRange {
                since_ms,
                until_ms,
            },
            &cursor,
            REDIS_STREAM_PAGE_SIZE,
        ) {
            Ok(page) => AppEvent::RedisStreamEntriesLoaded {
                tab_id,
                key,
                cursor,
                entries: page
                    .entries
                    .into_iter()
                    .map(|entry| (entry.id, entry.time, entry.fields))
                    .collect(),
                next_cursor: page.next_cursor,
                total: page.total,
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "查询失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn load_redis_list_items_command(
        &mut self,
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    ) -> AppEvent {
        const REDIS_LIST_PAGE_SIZE: usize = 200;
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key.clone(),
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match RedisConnector::with_config(config).load_list_items(
            &object,
            &query,
            &cursor,
            REDIS_LIST_PAGE_SIZE,
        ) {
            Ok(page) => AppEvent::RedisListItemsLoaded {
                tab_id,
                key,
                query,
                cursor,
                items: page.items,
                next_cursor: page.next_cursor,
                total: page.total,
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "查询失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn apply_redis_hash_field_set_command(
        &mut self,
        tab_id: TabId,
        key: String,
        field: String,
        value: String,
        ttl: RedisHashFieldTtl,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.set_hash_field(object, &field, &value, ttl)
        })
    }

    /// 完整值弹框保存：走 raw 写回（放行 >1MB 被截断标记的完整值），行内编辑仍走普通 set。
    fn apply_redis_hash_field_set_raw_command(
        &mut self,
        tab_id: TabId,
        key: String,
        field: String,
        value: String,
        ttl: RedisHashFieldTtl,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.set_hash_field_raw(object, &field, &value, ttl)
        })
    }

    /// 仅更新 Hash 字段 TTL（纯 TTL 命令，不重写 value）。
    /// 大字段（>1MB 被截断）的 TTL 编辑必须走此路径，避免把截断片段回写覆盖完整数据。
    fn apply_redis_hash_field_ttl_command(
        &mut self,
        tab_id: TabId,
        key: String,
        field: String,
        ttl: RedisHashFieldTtl,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.set_hash_field_ttl(object, &field, ttl)
        })
    }

    fn apply_redis_hash_field_rename_command(
        &mut self,
        tab_id: TabId,
        key: String,
        old_field: String,
        new_field: String,
        value: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.rename_hash_field(object, &old_field, &new_field, &value)
        })
    }

    fn apply_redis_hash_field_delete_command(
        &mut self,
        tab_id: TabId,
        key: String,
        field: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.delete_hash_field(object, &field)
        })
    }

    fn apply_redis_zset_member_add_command(
        &mut self,
        tab_id: TabId,
        key: String,
        member: String,
        score: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.add_zset_member(object, &member, &score)
        })
    }

    fn apply_redis_zset_score_update_command(
        &mut self,
        tab_id: TabId,
        key: String,
        member: String,
        score: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.update_zset_score(object, &member, &score)
        })
    }

    fn apply_redis_zset_member_delete_command(
        &mut self,
        tab_id: TabId,
        key: String,
        member: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.delete_zset_member(object, &member)
        })
    }

    fn apply_redis_list_items_push_command(
        &mut self,
        tab_id: TabId,
        key: String,
        items: Vec<String>,
        head: bool,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.push_list_items(object, &items, head)
        })
    }

    fn apply_redis_list_items_pop_command(
        &mut self,
        tab_id: TabId,
        key: String,
        head: bool,
        count: usize,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.pop_list_items(object, head, count)
        })
    }

    fn apply_redis_list_item_set_command(
        &mut self,
        tab_id: TabId,
        key: String,
        index: usize,
        expected_old: Option<String>,
        value: String,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.set_list_item(object, index, expected_old.as_deref(), &value)
        })
    }

    fn apply_redis_list_item_delete_command(
        &mut self,
        tab_id: TabId,
        key: String,
        index: usize,
        expected_old: Option<String>,
    ) -> AppEvent {
        self.apply_redis_key_child_change(tab_id, key, move |connector, object| {
            connector.delete_list_item(object, index, expected_old.as_deref())
        })
    }

    fn apply_redis_key_child_change(
        &mut self,
        tab_id: TabId,
        key: String,
        apply: impl FnOnce(&RedisConnector, &ObjectPath) -> fluxdb_core::Result<()>,
    ) -> AppEvent {
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: key,
                    kind: ObjectKind::RedisKey,
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let connector = RedisConnector::with_config(config);
        match apply(&connector, &object) {
            Ok(()) => match self.load_data_page(&object, Pagination::new(0, 1), &[], &[]) {
                Ok(page) => AppEvent::DataLoaded(tab_id, page),
                Err(error) => {
                    let mut user_error = UserFacingError::from(error);
                    user_error.title = "刷新失败".to_string();
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "保存失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 「新增 Key」（对齐 RedisInsight AddKey）。目标是目标库（`RedisDb`）路径，
    /// 建 Key 成功后回传 [`AppEvent::RedisKeyCreated`]，UI 据此关抽屉、刷新键列表并打开新键详情。
    fn apply_redis_create_key_command(
        &mut self,
        tab_id: TabId,
        request: RedisAddKeyRequest,
    ) -> AppEvent {
        // 从数据编辑器标签页解析出目标库对象路径（键列表页挂的是 RedisDb 对象）。
        let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor)
                if matches!(editor.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey) =>
            {
                Some(ObjectPath {
                    connection_id: editor.object.connection_id,
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    name: editor.object.name.clone(),
                    kind: if editor.object.kind == ObjectKind::RedisKey {
                        ObjectKind::RedisDb
                    } else {
                        editor.object.kind
                    },
                })
            }
            _ => None,
        }) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 数据编辑器标签页不存在"));
        };
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let key = request.key.clone();
        match RedisConnector::with_config(config).add_key(&object, &request) {
            Ok(()) => AppEvent::RedisKeyCreated {
                tab_id,
                object,
                key,
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "新建失败".to_string();
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn apply_data_changes_command(
        &mut self,
        tab_id: TabId,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
    ) -> AppEvent {
        let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor) => editor.changes.clone().and_then(|changes| {
                editor.original_page.clone().map(|page| {
                    (
                        false,
                        editor.object.clone(),
                        editor.pagination,
                        changes,
                        page,
                    )
                })
            }),
            TabKind::QueryEditor(editor) => active_query_result_editor(editor).and_then(|result| {
                    result
                        .changes
                        .clone()
                        .and_then(|changes| {
                            result.original_page.clone().map(|page| {
                                (
                                    true,
                                    result.object.clone(),
                                    result.pagination,
                                    changes,
                                    page,
                                )
                            })
                        })
                }),
            _ => None,
        });

        let Some((is_query_result, object, pagination, changes, before_page)) = request else {
            return self.fail(Error::new(ErrorKind::Internal, "没有需要提交的更改"));
        };

        if is_query_result {
            return self.apply_query_result_changes_command(tab_id, &object, &changes, &before_page);
        }

        match self.apply_data_changes(&object, &changes) {
            Ok(()) => match self.load_data_page(&object, pagination, &sort, &filters) {
                Ok(page) => {
                    self.record_data_change_history(&object, &before_page, &changes);
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::DataEditor(editor) = &mut tab.kind
                    {
                        editor.page = Some(page.clone());
                        editor.original_page = Some(page.clone());
                        editor.changes = None;
                        editor.editing_cell = None;
                        editor.error = None;
                        tab.dirty = false;
                    }
                    AppEvent::DataLoaded(tab_id, page)
                }
                Err(error) => {
                    let mut user_error = UserFacingError::from(error);
                    user_error.title = "刷新失败".to_string();
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::DataEditor(editor) = &mut tab.kind
                    {
                        editor.original_page = editor.page.clone();
                        editor.changes = None;
                        editor.editing_cell = None;
                        editor.error = Some(user_error.clone());
                        tab.dirty = false;
                    }
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "保存失败".to_string();
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::DataEditor(editor) = &mut tab.kind
                {
                    editor.error = Some(user_error.clone());
                    tab.dirty = true;
                }
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn apply_query_result_changes_command(
        &mut self,
        tab_id: TabId,
        object: &ObjectPath,
        changes: &DataChangeSet,
        before_page: &DataPage,
    ) -> AppEvent {
        match self.apply_data_changes(object, changes) {
            Ok(()) => {
                self.record_data_change_history(object, before_page, changes);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                    && let Some(page_index) = editor.active_result_editor
                    && let Some(result) = editor.result_editors.get_mut(&page_index)
                {
                    result.original_page = result.page.clone();
                    result.changes = None;
                    result.editing_cell = None;
                    result.error = None;
                    if let Some(page) = result.page.clone()
                        && let Some(result_page) = editor.results.get_mut(page_index)
                    {
                        *result_page = page;
                    }
                    tab.dirty = false;
                }
                AppEvent::TabActivated(tab_id)
            }
            Err(error) => {
                let mut user_error = UserFacingError::from(error);
                user_error.title = "保存失败".to_string();
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    if let Some(result) = active_query_result_editor_mut(editor) {
                        result.error = Some(user_error.clone());
                    }
                    editor.error = Some(user_error.clone());
                    tab.dirty = true;
                }
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn download_binary_cell(
        &self,
        tab_id: TabId,
        row: usize,
        column: usize,
    ) -> fluxdb_core::Result<Vec<u8>> {
        let editor = self
            .find_tab(tab_id)
            .and_then(|tab| editable_data_editor(&tab.kind))
            .ok_or_else(|| Error::new(ErrorKind::Internal, "数据结果不存在"))?;
        ensure_binary_cell(editor, row, column)?;

        match current_cell_value(editor, row, column)? {
            CellValue::Bytes(bytes) => Ok(bytes),
            CellValue::BinarySummary(summary) if summary.is_null => {
                Err(Error::new(ErrorKind::Query, "NULL 二进制没有可下载内容"))
            }
            CellValue::BinarySummary(_) => self.load_cell_binary(tab_id, row, column),
            _ => Err(Error::new(
                ErrorKind::Unsupported,
                "当前单元格不是二进制字段",
            )),
        }
    }

    fn update_binary_cell_command(
        &mut self,
        tab_id: TabId,
        row: usize,
        column: usize,
        payload: BinaryUpdatePayload,
    ) -> AppEvent {
        if let Some(tab) = self.find_tab_mut(tab_id)
            && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
        {
            match update_binary_cell(editor, row, column, payload) {
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

    fn apply_data_changes(
        &self,
        object: &ObjectPath,
        changes: &DataChangeSet,
    ) -> fluxdb_core::Result<()> {
        let config = self
            .connection_config(object.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        apply_data_changes_for_connection(config, changes)
    }

}

fn replace_redis_key_row(page: &mut DataPage, key: &str, row: Row) -> bool {
    let Some(key_column) = page.columns.iter().position(|column| column.name == "键") else {
        return false;
    };
    let Some(existing_row) = page.rows.iter_mut().find(|candidate| {
        candidate
            .values
            .get(key_column)
            .is_some_and(|value| value.display_label() == key)
    }) else {
        return false;
    };
    *existing_row = row;
    true
}

fn redis_row_key(columns: &[Column], row: &Row) -> Option<String> {
    let key_column = columns.iter().position(|column| column.name == "键")?;
    row.values.get(key_column).map(CellValue::display_label)
}
