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
            recency: Arc::new(Mutex::new(RecencyFrequency::new())),
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
        text: String,
        cursor: usize,
        explicit: bool,
    ) -> fluxdb_core::Result<QueryCompletionResult> {
        self.query_completions_for_text_with_cancel(
            connection_id,
            database,
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
        text: String,
        cursor: usize,
        explicit: bool,
        latest_request: Arc<std::sync::atomic::AtomicU64>,
        request_id: u64,
    ) -> fluxdb_core::Result<QueryCompletionResult> {
        let editor = QueryEditorState {
            connection_id,
            database,
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
    /// 全部来自内存 CompletionIndex，无远程查询；对象不在索引 / 无可用文档返回 Error。
    ///
    /// `comment` 为候选自带的内联注释，仅对 Column 有意义（懒加载详情面板不另存
    /// 全文快照，沿用候选在补全时可得的注释文本）。
    pub fn completion_documentation_for(
        &self,
        connection_id: ConnectionId,
        database: Option<String>,
        kind: fluxdb_core::QueryCompletionKind,
        label: String,
        comment: Option<String>,
    ) -> CompletionDocumentationState {
        let no_cancel = || false;
        self.completion_documentation_for_with_cancel(
            connection_id,
            database,
            kind,
            label,
            comment,
            &no_cancel,
        )
    }

    /// F005 变体：带 latest-wins 取消回调的详情解析。
    ///
    /// 让 UI 在选中项切换时把 `should_cancel` 绑定到新请求 id，列清单逐行组装期间
    /// 一旦最新请求 id 变化即提前返回 `Loading`（停止旧请求线程），保证旧详情结果
    /// 不覆盖新选中项。数据仍全部来自内存 `CompletionIndex`，无远程查询。
    pub fn completion_documentation_for_with_cancel(
        &self,
        connection_id: ConnectionId,
        database: Option<String>,
        kind: fluxdb_core::QueryCompletionKind,
        label: String,
        comment: Option<String>,
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
                    ..Default::default()
};
        self.completion_item_documentation(connection_id, database.as_deref(), &item, should_cancel)
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
