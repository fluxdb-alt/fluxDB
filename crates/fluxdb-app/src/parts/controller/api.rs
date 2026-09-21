impl AppController {
    pub fn new() -> Self {
        Self {
            state: AppState::default(),
            next_connection_id: 1,
            next_group_id: 1,
            next_tab_id: 1,
            completion_cache: Arc::new(Mutex::new(CompletionCache::default())),
            completion_index: Arc::new(Mutex::new(CompletionIndex::default())),
            completion_index_storage: None,
            er_model_storage: None,
            er_model_services: Arc::new(Mutex::new(BTreeMap::new())),
            recency: Arc::new(Mutex::new(RecencyFrequency::new())),
            query_cancel_flags: Arc::new(Mutex::new(BTreeMap::new())),
            er_catalog: Arc::new(Mutex::new(ErCatalogCache::default())),
        }
    }

    /// 为标签登记一个新的查询取消标志（每次执行前调用），同时清掉已关闭标签的旧标志，
    /// 避免长会话里标志表随历史标签单调增长。
    fn register_query_cancel_flag(&mut self, tab_id: TabId) {
        if let Ok(mut flags) = self.query_cancel_flags.lock() {
            let open_tabs = self
                .state
                .tabs
                .iter()
                .map(|tab| tab.id)
                .collect::<BTreeSet<_>>();
            flags.retain(|tab_id, _| open_tabs.contains(tab_id));
            flags.insert(
                tab_id,
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            );
        }
    }

    /// 取标签当前的取消标志；没有登记过（例如后台任务、测试直接派发执行）时返回 `None`，
    /// 调用方按「不可取消」处理，与旧行为一致。
    fn query_cancel_flag(&self, tab_id: TabId) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        self.query_cancel_flags
            .lock()
            .ok()
            .and_then(|flags| flags.get(&tab_id).cloned())
    }

    /// 置位标签的取消标志：执行线程会在下一次检查点（每 100ms）发送服务端取消。
    /// 返回是否确有在执行的查询（无标志 = 没有进行中的执行）。
    fn request_query_cancel(&self, tab_id: TabId) -> bool {
        let Some(flag) = self.query_cancel_flag(tab_id) else {
            return false;
        };
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        true
    }

    /// 查询完成后移除本次执行的取消标志，避免已经置位的旧标志污染后续不经过
    /// `StartQueryExecution` 的执行入口。后台执行线程已经持有自己的 `Arc`，移除映射
    /// 不会影响它正在进行的收尾。
    fn clear_query_cancel_flag(&self, tab_id: TabId) {
        if let Ok(mut flags) = self.query_cancel_flags.lock() {
            flags.remove(&tab_id);
        }
    }

    pub fn with_mock_data() -> Self {
        let mut controller = Self::new();
        let _ = controller.dispatch(AppCommand::LoadConnections);
        controller
    }

    pub fn set_completion_index_storage(&mut self, storage: fluxdb_storage::FileStorage) {
        self.completion_index_storage = Some(storage);
    }

    /// 注册 ER 关系目录的持久化存储。服务实例由应用层按 scope 惰性创建并复用。
    pub fn set_er_model_storage(&mut self, storage: fluxdb_storage::FileStorage) {
        self.er_model_storage = Some(storage);
        self.er_model_services.lock().unwrap().clear();
    }

    pub fn er_model_service(&mut self, scope_key: impl Into<String>) -> Option<Arc<ErModelService>> {
        let scope_key = scope_key.into();
        let mut services = self.er_model_services.lock().unwrap();
        if let Some(service) = services.get(&scope_key) {
            return Some(service.clone());
        }
        let storage = self.er_model_storage.clone()?;
        let service = Arc::new(ErModelService::new(
            scope_key.clone(),
            Box::new(FileErRelationshipStore::new(storage, scope_key.clone())),
        ));
        services.insert(scope_key, service.clone());
        Some(service)
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// 切换 Redis Key 列表视图前清空当前 data 页，让 UI 走 RedisInsight 那种
    /// “先重置、再重新加载首批数据” 的节奏。
    pub fn reset_data_page_for_reload(&mut self, tab_id: TabId) {
        let Some(tab) = self.find_tab_mut(tab_id) else {
            return;
        };
        let TabKind::DataEditor(editor) = &mut tab.kind else {
            return;
        };
        editor.page = None;
        editor.original_page = None;
        editor.changes = None;
        editor.editing_cell = None;
        editor.loading = true;
        editor.error = None;
        tab.dirty = false;
    }

    pub fn connection_configs(&self) -> Vec<ConnectionConfig> {
        self.state
            .connections
            .iter()
            .map(|connection| connection.config.clone())
            .collect()
    }

    pub fn load_data_for_export(
        &self,
        object: &ObjectPath,
        offset: u64,
        limit: u64,
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        self.load_data_page(object, Pagination::new(offset, limit), sort, filters)
    }

    /// 一致快照分页导出（PG 经单 REPEATABLE READ 事务，其余默认逐页）。
    /// 供桌面导出驱动对 PG 获得全量一致快照，避免并发写行间漂移。
    pub fn export_pages_for_connection(
        &self,
        object: &ObjectPath,
        sort: &[SortSpec],
        filters: &[FilterSpec],
        on_cancel: &dyn Fn() -> bool,
        on_page: &mut dyn FnMut(DataPage) -> bool,
    ) -> fluxdb_core::Result<()> {
        let config = self
            .connection_config(object.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        export_pages_for_connection(&config, object, sort, filters, on_cancel, on_page)
    }

    pub fn preview_data_export(
        &self,
        object: &ObjectPath,
        fields: &[String],
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataExportPreview> {
        let config = self
            .connection_config(object.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        preview_data_export_for_connection(config, object, fields, sort, filters)
    }

    pub fn merge_open_connection_from(&mut self, loaded: &Self, connection_id: ConnectionId) {
        let Some(source) = loaded
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return;
        };
        let Some(target) = self
            .state
            .connections
            .iter_mut()
            .find(|connection| connection.config.id == connection_id)
        else {
            return;
        };
        if target.config != source.config {
            return;
        }
        target.connected = source.connected;
        target.expanded = source.expanded;
        target.objects = source.objects.clone();
        self.state.last_error = loaded.state.last_error.clone();
    }

    /// 把后台「刷新连接树」的结果合并回 live 控制器：只搬运各连接的 `objects` 与全局
    /// `last_error`，**不**回写 `connected` / `expanded`。
    ///
    /// 后台任务跑在克隆控制器上，期间用户可能已折叠某个连接；若照搬展开态会把用户
    /// 的操作覆盖掉，因此这两个字段以 live 控制器为准。
    pub fn merge_refreshed_tree_from(&mut self, loaded: &Self) {
        for source in &loaded.state.connections {
            let Some(target) = self
                .state
                .connections
                .iter_mut()
                .find(|connection| connection.config.id == source.config.id)
            else {
                continue;
            };
            if target.config != source.config {
                continue;
            }
            target.objects = source.objects.clone();
        }
        self.state.last_error = loaded.state.last_error.clone();
    }

    /// 把后台拉取到的 Redis 连接级运行概览合并回 live 控制器。
    /// 概览是「一次性快照」，由 `load_redis_overview_command` 在克隆控制器上计算好
    /// CPU 增量后，通过克隆控制器的 `redis_overview` 字段直接带回来。
    pub fn merge_redis_overview_from(&mut self, loaded: &Self, connection_id: ConnectionId) {
        let Some(source) = loaded
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return;
        };
        let Some(target) = self
            .state
            .connections
            .iter_mut()
            .find(|connection| connection.config.id == connection_id)
        else {
            return;
        };
        if target.config != source.config {
            return;
        }
        target.redis_overview = source.redis_overview.clone();
    }

    /// 把后台控制器取回的 Redis Key 元信息按键名合并回当前页，避免覆盖刷新期间发生的分页变化。
    pub fn merge_redis_key_metadata_from(
        &mut self,
        loaded: &Self,
        tab_id: TabId,
        keys: &[String],
    ) {
        let Some(source) = loaded.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::DataEditor(editor) => editor.page.as_ref(),
            _ => None,
        }) else {
            return;
        };
        let Some(target) = self.find_tab_mut(tab_id).and_then(|tab| match &mut tab.kind {
            TabKind::DataEditor(editor) => editor.page.as_mut(),
            _ => None,
        }) else {
            return;
        };
        for key in keys {
            if let Some(row) = source
                .rows
                .iter()
                .find(|row| redis_row_key(&source.columns, row).as_deref() == Some(key))
            {
                replace_redis_key_row(target, key, row.clone());
            }
        }
    }

    /// 生命周期旧响应防覆盖判定：该连接当前仍存在、仍连接、且 config 与发起加载时一致。
    ///
    /// 单飞只保证同 key 无并行；断开、重连、改配置后迟到的旧响应不得写回新状态。
    /// 这里是纯函数，便于单元测试覆盖各生命周期场景。
    pub fn connection_load_is_current(
        state: &AppState,
        connection_id: ConnectionId,
        expected_config: Option<&ConnectionConfig>,
    ) -> bool {
        state
            .connections
            .iter()
            .find(|c| c.config.id == connection_id)
            .is_some_and(|c| {
                c.connected && expected_config.map(|expected| expected == &c.config).unwrap_or(false)
            })
    }

    /// 实例包装：树加载完成时用当前 state 判断连接加载是否仍有效。
    pub fn is_connection_load_current(
        &self,
        connection_id: ConnectionId,
        expected_config: Option<&ConnectionConfig>,
    ) -> bool {
        Self::connection_load_is_current(&self.state, connection_id, expected_config)
    }

    pub fn merge_loaded_children(&mut self, parent: &ObjectPath, children: Vec<ObjectSummary>) {
        let Some(connection) = self
            .state
            .connections
            .iter_mut()
            .find(|connection| connection.config.id == parent.connection_id)
        else {
            return;
        };
        replace_loaded_children(&mut connection.objects, parent, children);
        self.state.last_error = None;
    }

    pub fn merge_renamed_table_from(
        &mut self,
        loaded: &Self,
        object: &ObjectPath,
        new_name: &str,
    ) {
        if let Some(source) = loaded
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == object.connection_id)
            && let Some(target) = self
                .state
                .connections
                .iter_mut()
                .find(|connection| connection.config.id == object.connection_id)
            && target.config == source.config
        {
            target.objects = source.objects.clone();
        }

        let mut renamed = object.clone();
        renamed.name = new_name.to_string();
        for tab in &mut self.state.tabs {
            if let TabKind::DataEditor(editor) = &mut tab.kind
                && editor.object == *object
            {
                editor.object = renamed.clone();
                editor.page = None;
                editor.original_page = None;
                editor.changes = None;
                editor.loading = true;
                editor.table_info = TableInfoState::default();
                tab.title = new_name.to_string();
                tab.dirty = false;
            }
        }
        self.state.last_error = loaded.state.last_error.clone();
    }

    pub fn merge_dropped_table_from(&mut self, loaded: &Self, object: &ObjectPath) {
        if let Some(source) = loaded
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == object.connection_id)
            && let Some(target) = self
                .state
                .connections
                .iter_mut()
                .find(|connection| connection.config.id == object.connection_id)
            && target.config == source.config
        {
            target.objects = source.objects.clone();
        }

        self.state.tabs.retain(|tab| match &tab.kind {
            TabKind::DataEditor(editor) => editor.object != *object,
            TabKind::CreateTable(create) => match &create.mode {
                CreateTableMode::Design {
                    object: design_object,
                    ..
                } => design_object != object,
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
        self.state.last_error = loaded.state.last_error.clone();
    }

    pub fn merge_deleted_database_from(
        &mut self,
        loaded: &Self,
        connection_id: ConnectionId,
        database: &str,
    ) {
        if let Some(source) = loaded
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            && let Some(target) = self
                .state
                .connections
                .iter_mut()
                .find(|connection| connection.config.id == connection_id)
            && target.config == source.config
        {
            target.objects = source.objects.clone();
        }

        self.state.tabs.retain(|tab| match &tab.kind {
            TabKind::DataEditor(editor) => {
                editor.object.connection_id != connection_id
                    || editor.object.database.as_deref() != Some(database)
            }
            TabKind::CreateTable(create) => {
                create.connection_id != connection_id || create.database.as_deref() != Some(database)
            }
            TabKind::QueryEditor(editor) => {
                editor.connection_id != connection_id || editor.database.as_deref() != Some(database)
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
        self.state.last_error = loaded.state.last_error.clone();
    }

    pub fn merge_last_error_from(&mut self, loaded: &Self) {
        self.state.last_error = loaded.state.last_error.clone();
    }

    pub fn query_completions_for_text(
        &self,
        connection_id: ConnectionId,
        database: Option<String>,
        schema: Option<String>,
        text: String,
        cursor: usize,
        explicit: bool,
    ) -> fluxdb_core::Result<QueryCompletionResult> {
        self.query_completions_for_text_with_cancel(
            connection_id,
            database,
            schema,
            text,
            cursor,
            explicit,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            0,
        )
    }

    pub fn query_completions_for_text_with_cancel(
        &self,
        connection_id: ConnectionId,
        database: Option<String>,
        schema: Option<String>,
        text: String,
        cursor: usize,
        explicit: bool,
        latest_request: Arc<std::sync::atomic::AtomicU64>,
        request_id: u64,
    ) -> fluxdb_core::Result<QueryCompletionResult> {
        let editor = QueryEditorState {
            connection_id,
            database,
            schema,
            text,
            origin: None,
            saved_fingerprint: None,
            running: false,
            results: Vec::new(),
            result_editors: BTreeMap::new(),
            active_result_editor: None,
            summaries: Vec::new(),
            error: None,
        };
        let should_cancel = || latest_request.load(std::sync::atomic::Ordering::Acquire) != request_id;
        self.query_completions_with_cancel(&editor, cursor, explicit, &should_cancel)
    }

    fn load_create_table_reference_columns(
        &mut self,
        tab_id: TabId,
        foreign_key_id: u64,
    ) -> fluxdb_core::Result<Vec<String>> {
        let create = self
            .find_tab(tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::CreateTable(create) => Some(create.clone()),
                _ => None,
            })
            .ok_or_else(|| Error::new(ErrorKind::Internal, "新建表标签页不存在"))?;
        let foreign_key = create
            .foreign_keys
            .iter()
            .find(|foreign_key| foreign_key.id == foreign_key_id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Internal, "外键不存在"))?;
        let table = foreign_key.referenced_table.trim();
        if table.is_empty() {
            return Err(Error::new(ErrorKind::Query, "请选择目标表"));
        }
        let referenced_database = foreign_key.referenced_database.trim();
        let database = if referenced_database.is_empty() {
            create.database.as_deref().unwrap_or("").to_string()
        } else {
            referenced_database.to_string()
        };
        let config = self
            .connection_config(create.connection_id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let columns = self.completion_columns(
            &config,
            create.connection_id,
            (!database.trim().is_empty()).then_some(database.as_str()),
            None,
            table,
        )?;
        Ok(columns.into_iter().map(|column| column.name).collect())
    }

    fn load_cell_binary(
        &self,
        tab_id: TabId,
        row: usize,
        column: usize,
    ) -> fluxdb_core::Result<Vec<u8>> {
        let editor = self
            .find_tab(tab_id)
            .and_then(|tab| editable_data_editor(&tab.kind))
            .ok_or_else(|| Error::new(ErrorKind::Internal, "数据结果不存在"))?;
        let page = editor
            .page
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
        let column_name = page
            .columns
            .get(column)
            .map(|column| column.name.clone())
            .ok_or_else(|| Error::new(ErrorKind::Internal, "列不存在"))?;
        let identity = row_identity(page, row)?;
        let config = self
            .connection_config(editor.object.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;

        load_cell_binary_for_connection(config, &editor.object, &identity, &column_name)
    }

    /// F005：选中补全项的说明文档（懒加载）。表/视图→列清单、列→注释、函数等→名称。
    /// 表/视图优先读内存 CompletionIndex，索引未覆盖该对象时按 (库, schema, 表) 取一次
    /// 列元数据并写回索引；对象取不到列 / 无可用文档返回 Error。
    ///
    /// `comment` 为候选自带的内联注释，仅对 Column 有意义（懒加载详情面板不另存
    /// 全文快照，沿用候选在补全时可得的注释文本）。`schema` 为候选携带的对象 schema
    /// 作用域（见 `QueryCompletionItem::schema`），是命中列身份的必要信息。
    pub fn completion_documentation_for(
        &self,
        connection_id: ConnectionId,
        database: Option<String>,
        kind: fluxdb_core::QueryCompletionKind,
        label: String,
        comment: Option<String>,
        schema: Option<String>,
    ) -> CompletionDocumentationState {
        let no_cancel = || false;
        self.completion_documentation_for_with_cancel(
            connection_id,
            database,
            kind,
            label,
            comment,
            schema,
            &no_cancel,
        )
    }

    /// F005 变体：带 latest-wins 取消回调的详情解析。
    ///
    /// 让 UI 在选中项切换时把 `should_cancel` 绑定到新请求 id：索引未命中而需要按需
    /// 取元数据时，旧请求在建连/查询阶段即可提前收敛；列清单逐行组装期间一旦最新请求
    /// id 变化也立即返回 `Loading`，保证旧详情不覆盖新选中项。
    pub fn completion_documentation_for_with_cancel(
        &self,
        connection_id: ConnectionId,
        database: Option<String>,
        kind: fluxdb_core::QueryCompletionKind,
        label: String,
        comment: Option<String>,
        schema: Option<String>,
        should_cancel: &dyn Fn() -> bool,
    ) -> CompletionDocumentationState {
        let item = QueryCompletionItem {
            label,
            insert_text: String::new(),
            kind,
            detail: None,
            documentation: comment,
            filter_text: None,
            sort_text: None,
            schema,
            ..Default::default()
        };
        self.completion_item_documentation(
            connection_id,
            database.as_deref(),
            item.schema.as_deref(),
            &item,
            should_cancel,
        )
    }

    /// 侧栏表/视图节点悬停预览：加载字段清单，供浮层预览卡渲染。
    ///
    /// 仅 `Table`/`View` 生效，其余节点直接返回空列表。复用补全链路的
    /// `completion_columns_with_cancel`（自带 `CompletionCache` 缓存与最新优先取消），
    /// 不新建第二套缓存。`should_cancel` 由 UI 绑定到最新请求 id，目标切换时提前返回空。
    ///
    /// 可由外部调用（与 `query_completions_for_text_with_cancel` 同模式），挂载
    /// `HoverPreviewState` 到 UI 层。
    pub fn load_hover_columns(
        &self,
        object: &ObjectPath,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if !matches!(object.kind, ObjectKind::Table | ObjectKind::View) {
            return Ok(Vec::new());
        }
        let config = self
            .connection_config(object.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        self.completion_columns_with_cancel(
            config,
            object.connection_id,
            object.database.as_deref(),
            object.schema.as_deref(),
            &object.name,
            should_cancel,
        )
    }

    /// F004：开启/关闭补全个性化（recency/frequency）。默认关闭——关闭时
    /// `record_completion_accept` 不记录、个性化分数恒 0，排序与确定性基线完全一致。
    pub fn set_completion_personalization_enabled(&self, enabled: bool) {
        if let Ok(mut recency) = self.recency.lock() {
            recency.set_enabled(enabled);
        }
    }

    /// F004：记录一次补全候选采纳（仅匿名 label，不记完整 SQL / 密码 / 敏感值）。
    /// 关闭状态下由 `RecencyFrequency::record_accept` 内部直接忽略（不记录）。
    pub fn record_completion_accept(&self, label: &str) {
        if let Ok(mut recency) = self.recency.lock() {
            recency.record_accept(label, now_ts());
        }
    }

    /// F004：读取当前个性化是否启用（供排序路径据此决定是否应用加分）。
    pub fn completion_personalization_enabled(&self) -> bool {
        self.recency.lock().map(|r| r.enabled()).unwrap_or(false)
    }

}
