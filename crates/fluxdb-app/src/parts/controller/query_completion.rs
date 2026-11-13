
include!("completion_refresh.rs");

/// 达梦连接配置里的默认 schema；用于在不可靠的会话默认 schema 下收窄裸名补全。
fn default_completion_schema(config: &ConnectionConfig) -> Option<String> {
    (config.kind == DatabaseKind::Dameng)
        .then(|| {
            config
                .dameng_profile
                .as_ref()
                .map(|profile| profile.basic.schema.trim())
                .filter(|schema| !schema.is_empty())
                .map(str::to_string)
        })
        .flatten()
}

fn quote_completion_insert_text(
    item: &mut QueryCompletionItem,
    kind: DatabaseKind,
    reserved: &impl Fn(&str) -> bool,
) {
    // 表/视图/列/模式等命名数据库对象：点击即按方言补引号（MySQL/TiDB 反引号，
    // PostgreSQL/SQLite 双引号），与 Navicat 一致，避免“点了表没加上引号”。
    let always_quote = matches!(
        item.kind,
        QueryCompletionKind::Schema
            | QueryCompletionKind::Table
            | QueryCompletionKind::View
            | QueryCompletionKind::Column
    );
    let is_identifier = always_quote
        || matches!(
            item.kind,
            QueryCompletionKind::Function
                | QueryCompletionKind::Procedure
                | QueryCompletionKind::Trigger
        );
    // CTE / 派生表列与投影别名来自语句本身：保持原样，补引号会改变大小写折叠语义。
    if !is_identifier || !item.quote_identifier {
        return;
    }
    // 复合标识符逐段处理；函数调用保留括号。函数/存储过程/触发器仅在“必须引号”
    // （保留字/特殊字符/大写）时才补引号，避免把 `COUNT()` 误写成 `` `COUNT`() ``。
    let quote_name = |name: &str| {
        name.split('.')
            .map(|part| {
                if always_quote {
                    quote_identifier(part, kind, |_| true)
                } else {
                    quote_identifier(part, kind, reserved)
                }
            })
            .collect::<Vec<_>>()
            .join(".")
    };
    if item.kind == QueryCompletionKind::Schema && item.insert_text.ends_with('.') {
        let name = &item.insert_text[..item.insert_text.len() - 1];
        item.insert_text = format!("{}.", quote_name(name));
    } else if item.kind == QueryCompletionKind::Function {
        if let Some(name) = item.insert_text.strip_suffix("()") {
            item.insert_text = format!("{}()", quote_name(name));
        } else {
            item.insert_text = quote_name(&item.insert_text);
        }
    } else {
        item.insert_text = quote_name(&item.insert_text);
    }
}

impl AppController {
    /// AI 找表的本地索引查询：只读取内存/盘上已建好的元数据索引，不访问数据库。
    ///
    /// 返回 `None` 表示该连接没有可用的新鲜索引（功能关闭、从未建索引或全部过期），
    /// 调用方应回退到实时扫描；返回 `Some` 时结果可信，但只覆盖已建索引的库/schema，
    /// 空结果不能推断为整库不存在，由工具回执显式说明覆盖范围。
    fn ai_table_index_search(
        &self,
        config: &ConnectionConfig,
        query: &str,
        limit: usize,
    ) -> Option<(Vec<MetadataSearchHit>, bool)> {
        if !self.state.settings.enable_completion_index {
            return None;
        }
        let now = unix_timestamp_secs();
        // 不能因为某一个库已预热就跳过盘上其它库的快照：找表要覆盖整条连接。
        let scopes_loaded = self
            .completion_index
            .lock()
            .is_ok_and(|index| index.persisted_scopes_loaded(config.id));
        if !scopes_loaded {
            self.load_persisted_completion_indexes(config);
        }
        let index = self.completion_index.lock().ok()?;
        if !index.has_fresh_connection_index(config.id, now) {
            return None;
        }
        Some((
            index.search_fresh_metadata(config.id, query, limit, now),
            index.has_search_scope_gaps(config.id, now),
        ))
    }

    /// 合并一个连接下所有作用域的盘上快照；每个连接只加载一次，
    /// 之后由常规失效/刷新逻辑维护，避免每次找表都重复读文件。
    fn load_persisted_completion_indexes(&self, config: &ConnectionConfig) {
        if self
            .completion_index
            .lock()
            .is_ok_and(|index| index.persisted_scopes_loaded(config.id))
        {
            return;
        }
        let Some(storage) = &self.completion_index_storage else {
            return;
        };
        let snapshots = match storage.load_completion_indexes_for_connection(config) {
            Ok(snapshots) => snapshots,
            Err(error) => {
                tracing::warn!(
                    target: "gdb_sql_completion",
                    op = "completion_index_load_all",
                    error = %error,
                    connection_id = config.id.0,
                    "读取连接的补全索引快照失败，回退实时扫描"
                );
                return;
            }
        };
        if let Ok(mut index) = self.completion_index.lock() {
            for snapshot in snapshots {
                // 内存里已有的作用域可能比盘上快照更新（刚刷新或刚标 dirty），不能被旧快照覆盖。
                if index.has_database_index(
                    snapshot.connection_id,
                    snapshot.database.as_deref(),
                    snapshot.schema.as_deref(),
                ) {
                    continue;
                }
                index.insert_snapshot(snapshot);
            }
            index.mark_persisted_scopes_loaded(config.id);
        }
    }

    fn execute_query(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        self.execute_query_with_cancel(request, &|| false)
    }

    /// 带取消信号执行：`should_cancel` 返回 true 时执行器在检查点向服务端发送取消
    /// （PG 走 CancelToken，不是本地硬超时），并按「已取消」收尾。
    /// 查询编辑器标签把「停止」按钮的取消标志接到这里；其余调用方传 `|| false`，
    /// 与 `execute_query` 旧路径行为一致（`execute` 与无取消的 `execute_with_progress` 等价）。
    fn execute_query_with_cancel(
        &self,
        request: &QueryRequest,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let rollback_snapshots = self.query_history_rollback_snapshots(request)?;
        let config = self
            .connection_config(request.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let mut execution = execute_query_for_connection_with_progress(
            config,
            request,
            &mut |_| {},
            should_cancel,
        )?;
        execution.rollback_snapshots = rollback_snapshots;
        Ok(execution)
    }

    fn execute_query_raw(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        let config = self
            .connection_config(request.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        execute_query_for_connection(config, request)
    }

    /// 按「执行单元 = 单条命令」批量执行：把输入文本切分为单条命令，
    /// 返回「每条命令一个 `CommandWorkbenchExecution`」的列表。
    fn execute_command_workbench_commands(
        &self,
        request: &CommandWorkbenchRequest,
    ) -> fluxdb_core::Result<Vec<CommandWorkbenchExecution>> {
        let connection_id = match request.target {
            CommandExecutionTarget::Redis { connection_id, .. } => connection_id,
            _ => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "该目标暂不支持命令执行器",
                ))
            }
        };
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        execute_command_workbench_commands_for_connection(config, request)
    }

    pub fn execute_query_text_with_progress(
        &self,
        tab_id: TabId,
        text: String,
        options: QueryExecutionOptions,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::QueryEditor(editor) => Some(QueryRequest {
                connection_id: editor.connection_id,
                database: editor.database.clone(),
                // 查询编辑器标签拥有独占会话；带进度执行也必须复用该会话，
                // 否则 BEGIN/COMMIT、临时表和 SET 会退化为每次新连接。
                session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                schema: editor.schema.clone(),
                text: sql_text_for_execution(
                    &text,
                    fluxdb_core::Pagination::DEFAULT_LIMIT,
                    self.connection_kind(editor.connection_id)
                        .unwrap_or(DatabaseKind::MySql),
                ),
                mode: fluxdb_core::QueryMode::Selection,
                options,
            }),
            _ => None,
        });

        let Some(request) = request else {
            return Err(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
        };
        let rollback_snapshots = self.query_history_rollback_snapshots(&request)?;
        let config = self
            .connection_config(request.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let mut execution =
            execute_query_for_connection_with_progress(config, &request, on_summary, should_cancel)?;
        execution.rollback_snapshots = rollback_snapshots;
        Ok(execution)
    }

    pub fn execute_query_text_for_scope_with_progress(
        &self,
        connection_id: ConnectionId,
        database: Option<String>,
        schema: Option<String>,
        text: String,
        options: QueryExecutionOptions,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let request = QueryRequest {
            connection_id,
            database,
            schema,
            session_id: None,
            text: sql_text_for_execution(
                &text,
                fluxdb_core::Pagination::DEFAULT_LIMIT,
                self.connection_kind(connection_id).unwrap_or(DatabaseKind::MySql),
            ),
            mode: fluxdb_core::QueryMode::Selection,
            options,
        };
        let rollback_snapshots = self.query_history_rollback_snapshots(&request)?;
        let config = self
            .connection_config(request.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let mut execution =
            execute_query_for_connection_with_progress(config, &request, on_summary, should_cancel)?;
        execution.rollback_snapshots = rollback_snapshots;
        Ok(execution)
    }

    fn query_completions(
        &self,
        editor: &QueryEditorState,
        cursor: usize,
        _explicit: bool,
    ) -> fluxdb_core::Result<QueryCompletionResult> {
        self.query_completions_with_cancel(editor, cursor, _explicit, &|| false)
    }

    fn query_completions_with_cancel(
        &self,
        editor: &QueryEditorState,
        cursor: usize,
        explicit: bool,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryCompletionResult> {
        // 语义补全统一走 fluxdb-completion 引擎，旧启发式已删除。
        self.query_completions_from_engine(editor, cursor, explicit, should_cancel)
    }

    fn load_persisted_completion_index(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> bool {
        let now = unix_timestamp_secs();
        if let Ok(index) = self.completion_index.lock()
            && index.has_database_index(connection_id, database, schema)
        {
            return !index.is_dirty_or_expired(connection_id, database, schema, now);
        }

        let Some(storage) = &self.completion_index_storage else {
            return false;
        };
        let snapshot = match storage.load_completion_index(config, database, schema) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return false,
            // 快照读失败（文件损坏 / 旧版本格式）：按无快照处理，但必须留下可检索的痕迹，
            // 否则「索引看起来从不落盘」只能靠翻代码猜。
            Err(error) => {
                tracing::warn!(
                    target: "gdb_sql_completion",
                    op = "completion_index_load",
                    error = %error,
                    connection_id = ?connection_id,
                    database,
                    schema,
                    "补全索引快照读取失败，按无快照继续"
                );
                return false;
            }
        };
        if snapshot.connection_id != connection_id {
            return false;
        }
        if let Ok(mut index) = self.completion_index.lock() {
            index.insert_snapshot(snapshot);
            !index.is_dirty_or_expired(connection_id, database, schema, now)
        } else {
            false
        }
    }

    fn save_persisted_completion_index(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) {
        let Some(storage) = &self.completion_index_storage else {
            return;
        };
        let snapshot = self
            .completion_index
            .lock()
            .ok()
            .map(|index| index.snapshot(connection_id, database, schema, config.kind));
        let Some(snapshot) = snapshot else {
            return;
        };
        let current_signature = CompletionSnapshotSignature::from_snapshot(&snapshot);
        if let Ok(Some(persisted)) = storage.load_completion_index(config, database, schema)
            && CompletionSnapshotSignature::from_snapshot(&persisted) == current_signature
        {
            return;
        }
        if let Err(error) = storage.save_completion_index(config, database, schema, &snapshot) {
            // 落盘失败不影响本次补全（内存索引仍可用），但下次冷启动会白跑一遍目录查询，
            // 故必须告警而不是静默丢弃。
            tracing::warn!(
                target: "gdb_sql_completion",
                op = "completion_index_save",
                error = %error,
                connection_id = ?connection_id,
                database,
                schema,
                table_count = snapshot.tables.len(),
                column_count = snapshot.columns.len(),
                "补全索引快照落盘失败"
            );
        }
    }

    fn indexed_completion_tables(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        self.indexed_completion_tables_with_cancel(config, connection_id, database, schema, &|| false)
    }

    fn indexed_completion_tables_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        if config.kind == DatabaseKind::SqlServer && schema.is_none() {
            // 对象树与旧索引可能只覆盖展开过的 schema，不能当作整库目录。
            // 通过现有 TTL 缓存读取完整 catalog，仍按数据库隔离并保留取消检查。
            let tables = self.completion_tables_with_cancel(
                config, connection_id, database, None, should_cancel,
            )?;
            if should_cancel() {
                return Ok(Vec::new());
            }
            if let Ok(mut index) = self.completion_index.lock()
                && index.database_tables(connection_id, database, None) != tables
            {
                index.insert_tables(connection_id, database, None, tables.clone(), config.kind);
            }
            return Ok(tables);
        }
        if let Ok(index) = self.completion_index.lock() {
            let tables = index.database_tables(connection_id, database, schema);
            if !tables.is_empty() {
                return Ok(tables);
            }
        }

        let mut tables = loaded_completion_tables(&self.state, connection_id, database, schema);
        if tables.is_empty() {
            tables = self.completion_tables_with_cancel(
                config,
                connection_id,
                database,
                schema,
                should_cancel,
            )?;
        }
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut index) = self.completion_index.lock() {
            index.insert_tables(connection_id, database, schema, tables.clone(), config.kind);
        }
        Ok(tables)
    }

    fn indexed_table_completion_columns_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(index) = self.completion_index.lock() {
            let columns = index.table_columns(connection_id, database, schema, table);
            if !columns.is_empty() {
                return Ok(columns);
            }
        }

        let mut columns = loaded_completion_columns(&self.state, connection_id, database, table);
        if columns.is_empty() {
            columns = self.completion_columns_with_cancel(
                config,
                connection_id,
                database,
                schema,
                table,
                should_cancel,
            )?;
        }
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut index) = self.completion_index.lock() {
            index.replace_table_columns(
                connection_id,
                database,
                schema,
                table,
                columns.clone(),
                config.kind,
            );
        }
        self.save_persisted_completion_index(config, connection_id, database, schema);
        Ok(columns)
    }

    fn warm_completion_index(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> fluxdb_core::Result<()> {
        if !self.state.settings.enable_completion_index {
            return Ok(());
        }
        let config = self
            .connection_config(connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        if self.load_persisted_completion_index(config, connection_id, database, schema)
            && let Ok(index) = self.completion_index.lock()
            && !index.is_dirty_or_expired(connection_id, database, schema, unix_timestamp_secs())
        {
            return Ok(());
        }

        let tables = self.indexed_completion_tables(config, connection_id, database, schema)?;
        // 大库仅预热表名；列由实际引用表的补全与详情按需加载。
        if tables.len() >= COMPLETION_METADATA_LIMIT as usize {
            tracing::debug!(target: "gdb_sql_completion", op = "index_warmup", table_count = tables.len(), "大库跳过全库列预热");
            self.save_persisted_completion_index(config, connection_id, database, schema);
            return Ok(());
        }
        let mut table_names = tables
            .iter()
            .filter(|table| matches!(table.kind, ObjectKind::Table | ObjectKind::View))
            .map(|table| table.name.clone())
            .collect::<Vec<_>>();
        if let Ok(index) = self.completion_index.lock() {
            let dirty_tables = index.dirty_table_names(connection_id, database, schema);
            if !dirty_tables.is_empty()
                && !index.is_database_dirty(connection_id, database, schema)
            {
                let dirty_tables = dirty_tables.into_iter().collect::<BTreeSet<_>>();
                table_names.retain(|table| dirty_tables.contains(&table.to_ascii_lowercase()));
            }
        }
        if table_names.is_empty() {
            self.save_persisted_completion_index(config, connection_id, database, schema);
            return Ok(());
        }

        self.refresh_completion_index_tables(config, connection_id, database, schema, &table_names)?;
        self.save_persisted_completion_index(config, connection_id, database, schema);
        Ok(())
    }

    /// T051：stale-while-refresh 触发器。索引过期或 dirty 时启动一次性后台刷新，
    /// 当前请求不受阻塞（仍读旧候选）。通过 `CompletionIndex::begin_refresh` 去重，
    /// 避免每次按键重复 spawn 刷新线程。
    fn maybe_refresh_expired_index_in_background(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
    ) {
        if !self.state.settings.enable_completion_index {
            return;
        }
        let db_key = CompletionIndex::db_key(connection_id, database, None);
        // 原子地检查「已有数据且过期/dirty」并登记刷新中，避免并发请求重复触发。
        // 冷启动（新库尚未建索引）走同步 provider 填充，不做后台刷新。
        let should_refresh = {
            let Ok(mut index) = self.completion_index.lock() else {
                return;
            };
            if !index.has_database_index(connection_id, database, None) {
                return;
            }
            if !index.is_dirty_or_expired(connection_id, database, None, unix_timestamp_secs()) {
                return;
            }
            index.begin_refresh(db_key.clone())
        };
        if !should_refresh {
            return; // 已有刷新线程在进行
        }

        let index = Arc::clone(&self.completion_index);
        let config = config.clone();
        let database = database.map(str::to_string);
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "index_refresh",
            phase = "start_background",
            connection_id = ?connection_id,
            database,
            "索引过期，后台刷新已触发，当前请求继续使用旧候选"
        );
        std::thread::spawn(move || {
            let result = refresh_index_columns_in_background(
                &index,
                &config,
                connection_id,
                database.as_deref(),
                None,
            );
            if let Ok(mut guard) = index.lock() {
                guard.end_refresh(&db_key);
            }
            match result {
                Ok((refreshed_tables, refreshed_columns)) => tracing::info!(
                    target: "gdb_sql_completion",
                    op = "index_refresh",
                    phase = "done",
                    connection_id = ?connection_id,
                    database,
                    refreshed_tables,
                    refreshed_columns,
                    "后台索引刷新完成，旧候选已被替换"
                ),
                Err(error) => tracing::warn!(
                    target: "gdb_sql_completion",
                    op = "index_refresh",
                    phase = "failed",
                    error = %error,
                    connection_id = ?connection_id,
                    database,
                    "后台索引刷新失败，保留旧候选"
                ),
            }
        });
    }

    fn refresh_completion_index_tables(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table_names: &[String],
    ) -> fluxdb_core::Result<()> {
        self.refresh_completion_index_tables_with_cancel(
            config,
            connection_id,
            database,
            schema,
            table_names,
            &|| false,
        )
    }

    fn refresh_completion_index_tables_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table_names: &[String],
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<()> {
        for chunk in table_names.chunks(COMPLETION_WARMUP_BATCH_SIZE) {
            if should_cancel() {
                return Ok(());
            }
            let chunk = chunk.to_vec();
            let columns = list_completion_columns_for_tables_for_connection_with_cancel(
                config,
                database,
                schema,
                &chunk,
                should_cancel,
            )?;
            if should_cancel() {
                return Ok(());
            }
            let mut by_table: BTreeMap<String, Vec<CompletionColumn>> = BTreeMap::new();
            for column in columns {
                by_table
                    .entry(column.table.clone())
                    .or_default()
                    .push(column);
            }
            for table in &chunk {
                by_table.entry(table.clone()).or_default();
            }
            if let Ok(mut index) = self.completion_index.lock() {
                index.replace_table_columns_batch(
                    connection_id,
                    database,
                    schema,
                    by_table,
                    config.kind,
                );
            }
        }
        Ok(())
    }

    fn completion_tables_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionTablesKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.tables.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "tables",
                connection_id = ?connection_id,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let tables = list_completion_tables_for_connection_with_cancel(
            config,
            database,
            schema,
            "",
            COMPLETION_METADATA_LIMIT,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.tables.insert(
                key,
                CompletionCacheEntry {
                    value: tables.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "tables",
            connection_id = ?connection_id,
            cache_hit = false,
            item_count = tables.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(tables)
    }

    /// 裸表补全的整库表清单。不同 schema 同名表必须同时保留，供上层按 schema 消歧。
    fn completion_tables_all_schemas_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionAllSchemaTablesKey {
            connection_id,
            database: database.map(str::to_string),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.all_schema_tables.get(&key)
            && entry.is_fresh(now)
        {
            return Ok(entry.value.clone());
        }

        let tables = connector_for(config)?
            .list_completion_tables_all_schemas_with_cancel(
                database,
                "",
                COMPLETION_METADATA_LIMIT,
                should_cancel,
            )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.all_schema_tables.insert(
                key,
                CompletionCacheEntry {
                    value: tables.clone(),
                    fetched_at: now,
                },
            );
        }
        Ok(tables)
    }

    /// 收集 schema 候选（P1.5）：合并索引中的 namespace 与对象树已加载的 schema。
    ///
    /// 索引只覆盖已预热的表（PostgreSQL 通常是 search_path 内的表），
    /// 不能因为索引非空就丢掉对象树中已加载的其他 schema。
    fn completion_schemas(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
    ) -> fluxdb_core::Result<Vec<(Option<String>, Option<String>)>> {
        let database_matches = |candidate: Option<&str>| {
            !matches!(
                config.kind,
                DatabaseKind::Postgres | DatabaseKind::Kingbase | DatabaseKind::SqlServer
            )
                || database.is_none_or(|database| {
                    candidate.is_none_or(|candidate| candidate.eq_ignore_ascii_case(database))
                })
        };

        let mut schemas = Vec::new();
        if let Ok(index) = self.completion_index.lock() {
            schemas.extend(
                index
                    .database_schemas(connection_id)
                    .into_iter()
                    .filter(|(candidate, _)| database_matches(candidate.as_deref())),
            );
        }

        if let Some(connection) = self
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        {
            schemas.extend(connection.objects.iter().filter_map(|object| {
                if !database_matches(object.path.database.as_deref()) {
                    return None;
                }
                match object.path.kind {
                    ObjectKind::Schema => {
                        Some((object.path.database.clone(), Some(object.path.name.clone())))
                    }
                    ObjectKind::Table | ObjectKind::View => {
                        Some((object.path.database.clone(), object.path.schema.clone()))
                    }
                    _ => None,
                }
            }));
        }

        if schemas.is_empty() {
            if let Some(database) = database {
                schemas.push((Some(database.to_string()), None));
            }
        }
        schemas.sort();
        schemas.dedup();
        Ok(schemas)
    }

    fn completion_columns(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        self.completion_columns_with_cancel(config, connection_id, database, schema, table, &|| false)
    }

    fn completion_columns_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionColumnsKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            table: table.to_ascii_lowercase(),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.columns.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "columns",
                connection_id = ?connection_id,
                table = %table,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let columns = list_completion_columns_for_connection_with_cancel(
            config,
            database,
            schema,
            table,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.columns.insert(
                key,
                CompletionCacheEntry {
                    value: columns.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "columns",
            connection_id = ?connection_id,
            table = %table,
            cache_hit = false,
            item_count = columns.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(columns)
    }

    /// 例程候选：索引优先（含签名，随快照持久化），未命中再走连接器并写回索引。
    ///
    /// 与 tables/columns 同一形态：索引里已有该 scope 的例程就直接复用，避免每次补全
    /// 重新拉 catalog；写入后同步持久化，重开应用无需重建。
    /// 表操作（重命名/复制/删除）成功后失效该 scope 的补全缓存。
    ///
    /// 表操作直接执行 SQL、不走查询历史记录路径，故不会经过 `mark_query_history_completion_dirty`；
    /// 这里按 scope 标脏，后台刷薪触发时会重取表清单与列，旧名/已删表不再被建议（§9.3）。
    fn mark_table_action_completion_dirty(&self, object: &ObjectPath) {
        self.invalidate_completion_metadata(object.connection_id, object.database.as_deref(), object.schema.as_deref(), None);
    }

    fn indexed_completion_routines_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(index) = self.completion_index.lock() {
            let routines = index.database_routines(connection_id, database, schema);
            if !routines.is_empty() {
                return Ok(routines);
            }
        }
        let routines =
            self.completion_routines_with_cancel(config, connection_id, database, schema, should_cancel)?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut index) = self.completion_index.lock() {
            index.insert_routines(
                connection_id,
                database,
                schema,
                routines.clone(),
                config.kind,
            );
        }
        self.save_persisted_completion_index(config, connection_id, database, schema);
        Ok(routines)
    }

    fn completion_routines_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionRoutinesKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.routines.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "routines",
                connection_id = ?connection_id,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let routines = list_completion_routines_for_connection_with_cancel(
            config,
            database,
            schema,
            "",
            COMPLETION_METADATA_LIMIT,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.routines.insert(
                key,
                CompletionCacheEntry {
                    value: routines.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "routines",
            connection_id = ?connection_id,
            cache_hit = false,
            item_count = routines.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(routines)
    }

    /// 拉取某张表的真实外键元数据（P2.13），带注内存缓存；失败视为无外键（不阻塞补全）。
    fn completion_foreign_keys_with_cancel(
        &self,
        config: &ConnectionConfig,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let key = CompletionColumnsKey {
            connection_id,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            table: table.to_string(),
        };
        let now = unix_timestamp_secs();
        if let Ok(cache) = self.completion_cache.lock()
            && let Some(entry) = cache.foreign_keys.get(&key)
            && entry.is_fresh(now)
        {
            tracing::debug!(
                target: "gdb_sql_completion",
                op = "metadata_cache_lookup",
                kind = "foreign_keys",
                connection_id = ?connection_id,
                table = %table,
                cache_hit = true,
                item_count = entry.value.len(),
            );
            return Ok(entry.value.clone());
        }

        let started = std::time::Instant::now();
        let foreign_keys = list_foreign_keys_for_connection_with_cancel(
            config,
            database,
            schema,
            table,
            should_cancel,
        )?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.foreign_keys.insert(
                key,
                CompletionCacheEntry {
                    value: foreign_keys.clone(),
                    fetched_at: now,
                },
            );
        }
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "metadata_refresh",
            kind = "foreign_keys",
            connection_id = ?connection_id,
            table = %table,
            cache_hit = false,
            item_count = foreign_keys.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
        );
        Ok(foreign_keys)
    }

    fn clear_completion_cache(&self) {
        if let Ok(mut cache) = self.completion_cache.lock() {
            cache.tables.clear();
            cache.columns.clear();
            cache.routines.clear();
            cache.triggers.clear();
            cache.foreign_keys.clear();
        }
        if let Ok(mut index) = self.completion_index.lock() {
            *index = CompletionIndex::default();
        }
    }

    fn fail(&mut self, error: Error) -> AppEvent {
        let error = UserFacingError::from(error);
        self.state.last_error = Some(error.clone());
        AppEvent::Failed(error)
    }

    /// T054：选中项完整对象说明（懒加载）。候选构建时不调用；仅当选中的候选需要
    /// 更完整说明（如表/视图的列清单）时按需解析。表/视图优先读 `CompletionIndex`；
    /// 索引未覆盖该对象（search_path 之外的表、索引刚被清空等）时按
    /// `(库, schema, 表)` 的完整身份取一次列元数据并写回索引。取不到 / 无可用描述
    /// 返回 `Error`。
    ///
    /// - 表/视图 → 结构元数据：列清单「列名  类型  注释」（注释为空省略）。
    /// - 列 → 内联注释（候选携带的 `documentation`）。
    /// - 函数/过程/触发器 → 名称。
    /// - 其它（Keyword/Schema 等）→ `Error`。
    fn completion_item_documentation(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        item: &QueryCompletionItem,
        should_cancel: &dyn Fn() -> bool,
    ) -> CompletionDocumentationState {
        match item.kind {
            QueryCompletionKind::Table | QueryCompletionKind::View => {
                // 已判废的请求直接返回 Loading：不占用线程/连接做无谓取数（latest-wins）。
                if should_cancel() {
                    return CompletionDocumentationState::Loading;
                }
                let (schema, table) = documentation_object_identity(schema, &item.label);
                let columns = match self.documentation_table_columns(
                    connection_id,
                    database,
                    schema,
                    table,
                    should_cancel,
                ) {
                    Ok(columns) => columns,
                    // 元数据来源不可用与「对象没有列」是两回事，如实区分，不谎报成不存在。
                    Err(reason) => {
                        return CompletionDocumentationState::Error(format!(
                            "列元数据不可用：{reason}"
                        ));
                    }
                };
                if columns.is_empty() {
                    return CompletionDocumentationState::Error("对象不在索引或没有列".to_string());
                }
                // 列清单可能较大；逐行组装时按请求取消回调快速收敛，选中项切换时
                // 旧详情不继续占用线程（latest-wins 由调用方 request id 保证）。
                let mut doc = String::new();
                for column in columns {
                    if should_cancel() {
                        return CompletionDocumentationState::Loading;
                    }
                    if !doc.is_empty() {
                        doc.push('\n');
                    }
                    doc.push_str(&column.name);
                    if let Some(type_name) = column.type_name {
                        doc.push_str("  ");
                        doc.push_str(&type_name);
                    }
                    if let Some(comment) = column.comment.filter(|text| !text.is_empty()) {
                        doc.push_str("  ");
                        doc.push_str(&comment);
                    }
                }
                CompletionDocumentationState::Ready(doc)
            }
            QueryCompletionKind::Column => match item.documentation.as_ref() {
                Some(comment) if !comment.is_empty() => {
                    CompletionDocumentationState::Ready(comment.clone())
                }
                _ => CompletionDocumentationState::Error("无列注释".to_string()),
            },
            QueryCompletionKind::Function
            | QueryCompletionKind::Procedure
            | QueryCompletionKind::Trigger
            | QueryCompletionKind::Snippet => {
                CompletionDocumentationState::Ready(item.label.clone())
            }
            _ => CompletionDocumentationState::Error("无可用文档".to_string()),
        }
    }

    /// 详情面板取某对象列清单的元数据来源，与列补全共用同一条链路：
    /// 内存索引 → 盘上快照 → 已打开数据编辑页的列 → 连接器（取到后写回索引与快照）。
    /// 因此同一张表「按需拉一次」之后，再次悬停即直接命中。
    ///
    /// `Err(原因)` 表示元数据来源不可用（连接不存在 / 拉取失败），与「对象确实没有列」
    /// 区分开：调用方对前者提示「列元数据不可用」，对后者提示「对象不在索引或没有列」，
    /// 不把连接故障谎报成对象不存在。
    ///
    /// 该路径不读 `settings.enable_completion_index`：详情是用户显式选中单个对象的
    /// 动作，与「按前缀批量补全」不同，读写索引不会造成额外批量开销。
    fn documentation_table_columns(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<CompletionColumn>, String> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        if let Ok(index) = self.completion_index.lock() {
            let columns = index.table_columns(connection_id, database, schema, table);
            if !columns.is_empty() {
                tracing::debug!(
                    target: "gdb_sql_completion",
                    op = "documentation_columns",
                    connection_id = ?connection_id,
                    database,
                    schema,
                    table,
                    cache_hit = true,
                    item_count = columns.len(),
                    "详情列清单命中索引"
                );
                return Ok(columns);
            }
        }
        let Some(config) = self.connection_config(connection_id) else {
            tracing::warn!(
                target: "gdb_sql_completion",
                op = "documentation_columns",
                connection_id = ?connection_id,
                database,
                schema,
                table,
                "连接不存在，详情无法取列元数据"
            );
            return Err("连接不存在".to_string());
        };
        // 内存索引会被 DDL / 执行查询等命令整体清空（见 should_clear_completion_cache），
        // 但盘上快照仍在：先尝试恢复，避免用户每执行一次查询后悬停都重新建连取数。
        if self.load_persisted_completion_index(&config, connection_id, database, schema)
            && let Ok(index) = self.completion_index.lock()
        {
            let columns = index.table_columns(connection_id, database, schema, table);
            if !columns.is_empty() {
                tracing::debug!(
                    target: "gdb_sql_completion",
                    op = "documentation_columns",
                    connection_id = ?connection_id,
                    database,
                    schema,
                    table,
                    cache_hit = true,
                    source = "persisted",
                    item_count = columns.len(),
                    "详情列清单命中盘上快照"
                );
                return Ok(columns);
            }
        }
        let started = std::time::Instant::now();
        let columns = self
            .indexed_table_completion_columns_with_cancel(
                &config,
                connection_id,
                database,
                schema,
                table,
                should_cancel,
            )
            .map_err(|error| {
                tracing::warn!(
                    target: "gdb_sql_completion",
                    op = "documentation_columns",
                    error = %error,
                    connection_id = ?connection_id,
                    database,
                    schema,
                    table,
                    "详情列元数据不可用"
                );
                error.to_string()
            })?;
        tracing::debug!(
            target: "gdb_sql_completion",
            op = "documentation_columns",
            connection_id = ?connection_id,
            database,
            schema,
            table,
            cache_hit = false,
            item_count = columns.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
            "详情列清单按需拉取（并写回索引）"
        );
        Ok(columns)
    }
}

/// 详情查询用的对象身份：优先用候选携带的 schema 作用域；未携带时从限定的 label
/// （`schema.table`，跨 schema 同名消歧时由补全项加前缀）里拆出 schema。
///
/// 拆 label 只是兜底：正常路径下（`FROM tenant_a.`）schema 由候选直接携带，
/// 只有 search_path 多 schema 同名表这种消歧场景才会走到 label 拆分。
fn documentation_object_identity<'a>(
    schema: Option<&'a str>,
    label: &'a str,
) -> (Option<&'a str>, &'a str) {
    if let Some(schema) = schema {
        // 跨 schema 同名消歧时 label 已是 `schema.table`；候选另带真实 schema，
        // 不能把整段 label 再当作表名去查列。
        let qualified_prefix = format!("{schema}.");
        return match label.strip_prefix(&qualified_prefix) {
            Some(table) if !table.is_empty() => (Some(schema), table),
            _ => (Some(schema), label),
        };
    }
    match label.rsplit_once('.') {
        Some((schema, table)) if !schema.is_empty() && !table.is_empty() => (Some(schema), table),
        _ => (None, label),
    }
}

#[cfg(test)]
mod completion_documentation_identity_tests {
    use super::documentation_object_identity;

    #[test]
    fn qualified_label_uses_bare_table_when_schema_is_known() {
        assert_eq!(
            documentation_object_identity(Some("FLUXDEMO"), "FLUXDEMO.CUSTOMERS"),
            (Some("FLUXDEMO"), "CUSTOMERS")
        );
        assert_eq!(
            documentation_object_identity(Some("public"), "customers"),
            (Some("public"), "customers")
        );
    }
}

#[cfg(test)]
mod dameng_completion_index_tests {
    use super::*;
    use fluxdb_core::{
        DamengAdvancedOptions, DamengBasicOptions, DamengConnectionProfile, Endpoint, SecretRef,
    };

    fn config_with_schema(schema: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(7),
            name: "dm".into(),
            kind: DatabaseKind::Dameng,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 5236,
                database: Some("DAMENG".into()),
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: Some(DamengConnectionProfile {
                basic: DamengBasicOptions {
                    host: "127.0.0.1".into(),
                    port: 5236,
                    database: "DAMENG".into(),
                    schema: schema.into(),
                    username: "FLUXDEMO".into(),
                    password: SecretRef::inline("secret"),
                },
                advanced: DamengAdvancedOptions {
                    connect_timeout_secs: 1,
                    query_timeout_secs: 0,
                    encoding: String::new(),
                    application_name: "test".into(),
                },
                ssh: Default::default(),
            }),
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    fn config() -> ConnectionConfig {
        config_with_schema("FLUXDEMO")
    }

    fn sqlite_config() -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(11),
            name: "sqlite".into(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: PathBuf::from(":memory:"),
                read_only: false,
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    /// 签名提示只认连接方言的内置函数表：方言不提供的函数（SQLite 无 EXTRACT）不给签名，
    /// 未收录的名字（含用户自定义例程）与未知连接同样不给。
    #[test]
    fn function_signature_follows_connection_dialect() {
        let mut controller = AppController::new();
        let dameng = config();
        let dameng_id = dameng.id;
        let sqlite = sqlite_config();
        let sqlite_id = sqlite.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![dameng, sqlite]));

        assert_eq!(
            controller
                .sql_function_signature(dameng_id, "count")
                .as_deref(),
            Some("COUNT(expr)")
        );
        assert_eq!(
            controller
                .sql_function_signature(dameng_id, "EXTRACT")
                .as_deref(),
            Some("EXTRACT(field FROM source)")
        );
        assert_eq!(controller.sql_function_signature(sqlite_id, "EXTRACT"), None);
        assert_eq!(controller.sql_function_signature(dameng_id, "MY_FUNC"), None);
        assert_eq!(
            controller.sql_function_signature(ConnectionId(99), "COUNT"),
            None
        );
    }

    fn column(table: &str, name: &str) -> CompletionColumn {
        CompletionColumn {
            database: None,
            schema: None,
            table: table.into(),
            name: name.into(),
            type_name: Some("text".into()),
            nullable: true,
            primary_key: false,
            comment: None,
            stable: None,
        }
    }

    fn table(schema: &str, name: &str) -> CompletionTable {
        CompletionTable {
            database: Some("DAMENG".into()),
            schema: Some(schema.into()),
            name: name.into(),
            kind: ObjectKind::Table,
            comment: None,
        }
    }

    #[test]
    fn sqlserver_schema_candidates_do_not_leak_between_databases() {
        let config = ConnectionConfig {
            id: ConnectionId(19),
            name: "SQL Server schema isolation".into(),
            kind: DatabaseKind::SqlServer,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1433,
                database: Some("current_db".into()),
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };
        let mut controller = AppController::new();
        controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));
        for (database, indexed_schema, tree_schema) in [
            ("current_db", "app", "hr"),
            ("other_db", "foreign_index", "foreign_tree"),
        ] {
            controller.completion_index.lock().expect("completion index").insert_tables(
                config.id, Some(database), None,
                vec![CompletionTable {
                    database: Some(database.into()),
                    schema: Some(indexed_schema.into()),
                    name: "customers".into(),
                    kind: ObjectKind::Table,
                    comment: None,
                }],
                config.kind,
            );
            controller.state.connections[0].objects.push(ObjectSummary {
                path: ObjectPath {
                    connection_id: config.id,
                    database: Some(database.into()),
                    schema: Some(tree_schema.into()),
                    name: tree_schema.into(),
                    kind: ObjectKind::Schema,
                },
                rows: None,
                modified_at: None,
                comment: None,
                stable: None,
            });
        }
        assert_eq!(
            controller.completion_schemas(&config, config.id, Some("CURRENT_DB"))
                .expect("schema candidates"),
            vec![
                (Some("current_db".into()), Some("app".into())),
                (Some("current_db".into()), Some("hr".into())),
            ]
        );
    }

    /// 真实落盘快照里表清单在 schema=None 桶、列在默认 schema 桶；裸名补全必须
    /// 读前者，不能因为切到 schema 桶而读到空表清单。
    #[test]
    fn bare_completion_prefers_default_schema_and_falls_back_to_database() {
        let mut controller = AppController::new();
        let config = config();
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        if let Ok(mut index) = controller.completion_index.lock() {
            index.insert_tables(
                connection_id,
                Some("DAMENG"),
                None,
                vec![
                    table("FLUXDEMO", "CUSTOMERS"),
                    table("FLUXSALES", "SALES_ORDER"),
                ],
                DatabaseKind::Dameng,
            );
        }

        let items = controller
            .query_completions_for_text(
                connection_id,
                Some("DAMENG".into()),
                None,
                "SELECT * FROM CUST".into(),
                "SELECT * FROM CUST".len(),
                true,
            )
            .expect("completions")
            .items;
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();

        // 默认 schema 有匹配：只保留默认 schema，避免其他 schema 混入。
        assert!(labels.contains(&"CUSTOMERS"), "{labels:?}");
        assert!(!labels.contains(&"SALES_ORDER"), "{labels:?}");

        // 默认 schema 无匹配：回退全库，并插入 schema 限定名，保证仍可执行。
        let fallback = controller
            .query_completions_for_text(
                connection_id,
                Some("DAMENG".into()),
                None,
                "SELECT * FROM SALES".into(),
                "SELECT * FROM SALES".len(),
                true,
            )
            .expect("fallback completions")
            .items;
        let fallback = fallback
            .iter()
            .filter(|item| item.kind == QueryCompletionKind::Table)
            .map(|item| (&item.label, &item.insert_text))
            .collect::<Vec<_>>();
        assert!(
            fallback
                .iter()
                .any(|(label, insert)| *label == "SALES_ORDER"
                    && *insert == "\"FLUXSALES\".\"SALES_ORDER\""),
            "{fallback:?}"
        );
    }

    /// 索引只覆盖默认 schema 时，对象树中新展开的 schema 表仍需合并进回退候选。
    #[test]
    fn loaded_other_schema_tables_join_fallback_completion() {
        let mut controller = AppController::new();
        let config = config_with_schema("SYSDBA");
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        let database = ObjectPath {
            connection_id,
            database: Some("DAMENG".into()),
            schema: None,
            name: "DAMENG".into(),
            kind: ObjectKind::Database,
        };
        let schema = ObjectPath {
            connection_id,
            database: Some("DAMENG".into()),
            schema: None,
            name: "FLUXSALES".into(),
            kind: ObjectKind::Schema,
        };
        let object = |name: &str| ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("DAMENG".into()),
                schema: Some("FLUXSALES".into()),
                name: name.into(),
                kind: ObjectKind::Table,
            },
            rows: None,
            modified_at: None,
            comment: None,
            stable: None,
        };
        controller.merge_loaded_children(&database, vec![]);
        controller.merge_loaded_children(&schema, vec![object("SALES_REGION")]);
        if let Ok(mut index) = controller.completion_index.lock() {
            index.insert_tables(
                connection_id,
                Some("DAMENG"),
                None,
                vec![table("FLUXDEMO", "CUSTOMERS")],
                DatabaseKind::Dameng,
            );
        }

        let items = controller
            .query_completions_for_text(
                connection_id,
                Some("DAMENG".into()),
                None,
                "SELECT * FROM REGION".into(),
                "SELECT * FROM REGION".len(),
                true,
            )
            .expect("completions")
            .items;

        assert!(
            items.iter().any(|item| {
                item.label == "SALES_REGION"
                    && item.insert_text == "\"FLUXSALES\".\"SALES_REGION\""
            }),
            "{:?}",
            items
                .iter()
                .map(|item| (&item.label, &item.insert_text))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unloaded_other_schema_tables_join_bare_completion() {
        let mut controller = AppController::new();
        let config = config_with_schema("SYSDBA");
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        controller.merge_loaded_children(
            &ObjectPath {
                connection_id,
                database: Some("DAMENG".into()),
                schema: None,
                name: "DAMENG".into(),
                kind: ObjectKind::Database,
            },
            vec![ObjectSummary {
                path: ObjectPath {
                    connection_id,
                    database: Some("DAMENG".into()),
                    schema: None,
                    name: "FLUXSALES".into(),
                    kind: ObjectKind::Schema,
                },
                rows: None,
                modified_at: None,
                comment: None,
                stable: None,
            }],
        );
        if let Ok(mut index) = controller.completion_index.lock() {
            index.insert_tables(
                connection_id,
                Some("DAMENG"),
                Some("SYSDBA"),
                vec![table("SYSDBA", "CUSTOMERS")],
                DatabaseKind::Dameng,
            );
        }
        if let Ok(mut cache) = controller.completion_cache.lock() {
            cache.all_schema_tables.insert(
                CompletionAllSchemaTablesKey {
                    connection_id,
                    database: Some("DAMENG".into()),
                },
                CompletionCacheEntry {
                    value: vec![
                        table("SYSDBA", "CUSTOMERS"),
                        table("FLUXSALES", "SALES_REGION"),
                    ],
                    fetched_at: unix_timestamp_secs(),
                },
            );
        }

        let items = controller
            .query_completions_for_text(
                connection_id,
                Some("DAMENG".into()),
                None,
                "SELECT * FROM REGION".into(),
                "SELECT * FROM REGION".len(),
                true,
            )
            .expect("completions")
            .items;

        assert!(
            items.iter().any(|item| {
                item.label == "SALES_REGION"
                    && item.insert_text == "\"FLUXSALES\".\"SALES_REGION\""
            }),
            "{:?}",
            items
                .iter()
                .map(|item| (&item.label, &item.insert_text))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn qualified_completion_reads_requested_schema_bucket() {
        let mut controller = AppController::new();
        let config = config();
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        if let Ok(mut index) = controller.completion_index.lock() {
            index.insert_tables(
                connection_id,
                Some("DAMENG"),
                Some("FLUXSALES"),
                vec![table("FLUXSALES", "SALES_ORDER")],
                DatabaseKind::Dameng,
            );
        }

        let items = controller
            .query_completions_for_text(
                connection_id,
                Some("DAMENG".into()),
                None,
                "SELECT * FROM FLUXSALES.".into(),
                "SELECT * FROM FLUXSALES.".len(),
                true,
            )
            .expect("completions")
            .items;

        assert!(
            items
                .iter()
                .any(|item| item.label == "SALES_ORDER" && item.insert_text == "\"SALES_ORDER\""),
            "{:?}",
            items
                .iter()
                .map(|item| (&item.label, &item.insert_text))
                .collect::<Vec<_>>()
        );
    }

    /// 管理员连接（默认 schema=SYSDBA）下，表在 FLUXDEMO 时仍需回退给出候选，
    /// 并插入 schema 限定名。
    #[test]
    fn administrator_default_schema_falls_back_to_visible_tables() {
        let mut controller = AppController::new();
        let config = config_with_schema("SYSDBA");
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        if let Ok(mut index) = controller.completion_index.lock() {
            index.insert_tables(
                connection_id,
                Some("DAMENG"),
                None,
                vec![table("FLUXDEMO", "SALES_ORDERS")],
                DatabaseKind::Dameng,
            );
        }

        let items = controller
            .query_completions_for_text(
                connection_id,
                Some("DAMENG".into()),
                None,
                "SELECT * FROM SALES".into(),
                "SELECT * FROM SALES".len(),
                true,
            )
            .expect("completions")
            .items;

        assert!(
            items.iter().any(|item| {
                item.label == "SALES_ORDERS"
                    && item.insert_text == "\"FLUXDEMO\".\"SALES_ORDERS\""
            }),
            "{:?}",
            items
                .iter()
                .map(|item| (&item.label, &item.insert_text))
                .collect::<Vec<_>>()
        );
    }

    /// 接线验证：限定符列补全走新引擎后只替换残词，`u.` 保留。
    #[test]
    fn qualified_column_completion_keeps_qualifier() {
        let mut controller = AppController::new();
        let config = sqlite_config();
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        if let Ok(mut index) = controller.completion_index.lock() {
            index.insert_tables(
                connection_id,
                None,
                None,
                vec![CompletionTable {
                    database: None,
                    schema: None,
                    name: "users".into(),
                    kind: ObjectKind::Table,
                    comment: None,
                }],
                DatabaseKind::Sqlite,
            );
            index.replace_table_columns(
                connection_id,
                None,
                None,
                "users",
                vec![column("users", "id"), column("users", "name")],
                DatabaseKind::Sqlite,
            );
        }

        let sql = "SELECT u.na FROM users u";
        let result = controller
            .query_completions_for_text(
                connection_id,
                None,
                None,
                sql.into(),
                "SELECT u.na".len(),
                true,
            )
            .expect("completions");

        let name = result
            .items
            .iter()
            .find(|item| item.label == "name")
            .expect("应补出 name 列");
        // SQLite 列名按既有规则总是补双引号。
        assert_eq!(name.insert_text, "\"name\"");
        assert_eq!(&sql[result.replace_start..result.replace_end], "na");
    }

    /// 真实 SQLite 连接冒烟：索引为空时走连接器实时元数据，补全仍能出表与列。
    #[test]
    fn sqlite_live_metadata_feeds_completion() {
        let path = std::env::temp_dir().join(format!(
            "fluxdb-completion-{}-{:?}.db",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let connection = rusqlite::Connection::open(&path).expect("创建测试库");
            connection
                .execute_batch("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);")
                .expect("建表");
        }

        let mut controller = AppController::new();
        let mut config = sqlite_config();
        config.endpoint = Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        };
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));

        let text = "SELECT * FROM us";
        let tables = controller
            .query_completions_for_text(
                connection_id,
                None,
                None,
                text.into(),
                text.len(),
                true,
            )
            .expect("表补全")
            .items;
        assert!(
            tables.iter().any(|item| item.label == "users"),
            "{:?}",
            tables.iter().map(|item| &item.label).collect::<Vec<_>>()
        );

        let text = "SELECT u.na FROM users u";
        let columns = controller
            .query_completions_for_text(
                connection_id,
                None,
                None,
                text.into(),
                "SELECT u.na".len(),
                true,
            )
            .expect("列补全")
            .items;
        assert!(
            columns
                .iter()
                .any(|item| item.kind == QueryCompletionKind::Column && item.label == "name"),
            "{:?}",
            columns
                .iter()
                .map(|item| (&item.label, item.kind))
                .collect::<Vec<_>>()
        );

        let _ = std::fs::remove_file(&path);
    }

    /// MySQL 真库冒烟。密码只从环境变量读取，避免写入仓库。
    #[test]
    #[ignore = "需要本地 MySQL 测试库；设置 FLUXDB_SMOKE_MYSQL_PASSWORD 后手动运行"]
    fn mysql_live_metadata_feeds_completion() {
        let Ok(password) = std::env::var("FLUXDB_SMOKE_MYSQL_PASSWORD") else {
            return;
        };
        let options = BTreeMap::from([
            ("username".to_string(), "root".to_string()),
            ("password".to_string(), password),
        ]);
        let config = ConnectionConfig {
            id: ConnectionId(12),
            name: "mysql-smoke".into(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 45595,
                database: Some("fluxdb_demo".into()),
            },
            options,
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };
        let mut controller = AppController::new();
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));

        let text = "SELECT * FROM cus";
        let tables = controller
            .query_completions_for_text(
                connection_id,
                Some("fluxdb_demo".into()),
                None,
                text.into(),
                text.len(),
                true,
            )
            .expect("表补全")
            .items;
        assert!(
            tables.iter().any(|item| item.label == "customers"),
            "{:?}",
            tables.iter().map(|item| &item.label).collect::<Vec<_>>()
        );

        // ON 左侧由外键硬限定：orders 只有 customer_id 参与该外键。
        let text = "SELECT * FROM orders o JOIN customers c ON o.";
        let on_items = controller
            .query_completions_for_text(
                connection_id,
                Some("fluxdb_demo".into()),
                None,
                text.into(),
                text.len(),
                true,
            )
            .expect("ON 补全")
            .items;
        let mut labels = on_items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        assert_eq!(labels, vec!["customer_id"], "{:?}", on_items.len());
    }

    /// 达梦真库冒烟。密码只从环境变量读取，避免写入仓库。
    #[test]
    #[ignore = "需要本地达梦测试库；设置 FLUXDB_SMOKE_DAMENG_PASSWORD 后手动运行"]
    fn dameng_live_metadata_feeds_completion() {
        let Ok(password) = std::env::var("FLUXDB_SMOKE_DAMENG_PASSWORD") else {
            return;
        };
        let mut config = config();
        config.id = ConnectionId(13);
        if let Some(profile) = config.dameng_profile.as_mut() {
            profile.basic.username = "SYSDBA".into();
            profile.basic.password = SecretRef::inline(password);
        }
        let direct = list_completion_tables_for_connection_with_cancel(
            &config,
            Some("DAMENG"),
            Some("FLUXDEMO"),
            "",
            100,
            &|| false,
        );
        assert!(
            direct.as_ref().is_ok_and(|tables| !tables.is_empty()),
            "{:?}",
            direct.err()
        );

        let mut controller = AppController::new();
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));

        let text = "SELECT * FROM CUS";
        let tables = controller
            .query_completions_for_text(
                connection_id,
                Some("DAMENG".into()),
                None,
                text.into(),
                text.len(),
                true,
            )
            .expect("表补全")
            .items;
        assert!(
            tables.iter().any(|item| {
                item.label == "CUSTOMERS"
                    && item.insert_text == "\"FLUXDEMO\".\"CUSTOMERS\""
            }),
            "{:?}",
            tables
                .iter()
                .map(|item| (&item.label, &item.insert_text))
                .collect::<Vec<_>>()
        );

        let text = "SELECT c.CUSTOMER_ FROM CUSTOMERS c";
        let columns = controller
            .query_completions_for_text(
                connection_id,
                Some("DAMENG".into()),
                None,
                text.into(),
                "SELECT c.CUSTOMER_".len(),
                true,
            )
            .expect("列补全")
            .items;
        assert!(
            columns.iter().any(|item| {
                item.kind == QueryCompletionKind::Column && item.label == "CUSTOMER_NO"
            }),
            "{:?}",
            columns
                .iter()
                .map(|item| (&item.label, item.kind))
                .collect::<Vec<_>>()
        );
    }

    /// PostgreSQL 真库冒烟。密码只从环境变量读取，避免写入仓库。
    #[test]
    #[ignore = "需要本地 PostgreSQL 测试库；设置 FLUXDB_SMOKE_POSTGRES_PASSWORD 后手动运行"]
    fn postgres_live_metadata_feeds_completion() {
        let Ok(password) = std::env::var("FLUXDB_SMOKE_POSTGRES_PASSWORD") else {
            return;
        };
        let options = BTreeMap::from([
            ("host".to_string(), "127.0.0.1".to_string()),
            ("port".to_string(), "55432".to_string()),
            ("maintenance_database".to_string(), "fluxdb_smoke".to_string()),
            ("username".to_string(), "postgres".to_string()),
            ("password".to_string(), password),
            ("default_schema".to_string(), "demo".to_string()),
        ]);
        let config = ConnectionConfig {
            id: ConnectionId(14),
            name: "pg-smoke".into(),
            kind: DatabaseKind::Postgres,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 55432,
                database: Some("fluxdb_smoke".into()),
            },
            options: options.clone(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: Some(
                fluxdb_core::PostgresConnectionProfile::from_options(&options),
            ),
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };
        let mut controller = AppController::new();
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));

        let text = "SELECT * FROM ord";
        let tables = controller
            .query_completions_for_text(
                connection_id,
                Some("fluxdb_smoke".into()),
                Some("demo".into()),
                text.into(),
                text.len(),
                true,
            )
            .expect("表补全")
            .items;
        assert!(
            tables.iter().any(|item| item.label == "orders"),
            "{:?}",
            tables
                .iter()
                .map(|item| (&item.label, &item.insert_text))
                .collect::<Vec<_>>()
        );

        // 外键硬限定：orders 只有 customer_id 参与 JOIN。
        let text = "SELECT * FROM orders o JOIN customers c ON o.";
        let on_items = controller
            .query_completions_for_text(
                connection_id,
                Some("fluxdb_smoke".into()),
                Some("demo".into()),
                text.into(),
                text.len(),
                true,
            )
            .expect("ON 补全")
            .items;
        let mut labels = on_items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        assert_eq!(labels, vec!["customer_id"], "{:?}", on_items.len());
    }
}

#[cfg(test)]
mod postgres_completion_index_tests {
    use super::*;
    use fluxdb_core::{Endpoint, PostgresConnectionProfile};

    fn config() -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(15),
            name: "pg-multi-schema-completion".into(),
            kind: DatabaseKind::Postgres,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 5432,
                database: Some("fluxdb_pr46".into()),
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: Some(PostgresConnectionProfile::default()),
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    fn table(schema: &str, name: &str) -> CompletionTable {
        CompletionTable {
            database: Some("fluxdb_pr46".into()),
            schema: Some(schema.into()),
            name: name.into(),
            kind: ObjectKind::Table,
            comment: None,
        }
    }

    #[test]
    fn pg_bare_completion_merges_loaded_other_schema_tables() {
        let mut controller = AppController::new();
        let config = config();
        let connection_id = config.id;
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        if let Ok(mut index) = controller.completion_index.lock() {
            index.insert_tables(
                connection_id,
                Some("fluxdb_pr46"),
                Some("fluxdb_pr46_acceptance"),
                vec![table("fluxdb_pr46_acceptance", "pg_acceptance")],
                DatabaseKind::Postgres,
            );
        }
        controller.merge_loaded_children(
            &ObjectPath {
                connection_id,
                database: Some("fluxdb_pr46".into()),
                schema: None,
                name: "fluxdb_pr46".into(),
                kind: ObjectKind::Database,
            },
            vec![
                ObjectSummary {
                    path: ObjectPath {
                        connection_id,
                        database: Some("fluxdb_pr46".into()),
                        schema: None,
                        name: "fluxdb_pr46_acceptance".into(),
                        kind: ObjectKind::Schema,
                    },
                    rows: None,
                    modified_at: None,
                    comment: None,
                    stable: None,
                },
                ObjectSummary {
                    path: ObjectPath {
                        connection_id,
                        database: Some("fluxdb_pr46".into()),
                        schema: None,
                        name: "fluxdb_pr46_repro".into(),
                        kind: ObjectKind::Schema,
                    },
                    rows: None,
                    modified_at: None,
                    comment: None,
                    stable: None,
                },
            ],
        );
        if let Ok(mut cache) = controller.completion_cache.lock() {
            cache.all_schema_tables.insert(
                CompletionAllSchemaTablesKey {
                    connection_id,
                    database: Some("fluxdb_pr46".into()),
                },
                CompletionCacheEntry {
                    value: vec![
                        table("fluxdb_pr46_acceptance", "pg_acceptance"),
                        table("fluxdb_pr46_repro", "pg_display_probe"),
                    ],
                    fetched_at: unix_timestamp_secs(),
                },
            );
        }

        let text = "SELECT * FROM dis";
        let items = controller
            .query_completions_for_text(
                connection_id,
                Some("fluxdb_pr46".into()),
                Some("fluxdb_pr46_acceptance".into()),
                text.into(),
                text.len(),
                true,
            )
            .expect("bare completions")
            .items;
        assert!(
            items.iter().any(|item| {
                item.label == "pg_display_probe"
                    && item.insert_text == r#""fluxdb_pr46_repro"."pg_display_probe""#
            }),
            "{:?}",
            items
                .iter()
                .map(|item| (&item.label, &item.insert_text))
                .collect::<Vec<_>>()
        );

        let text = r#"SELECT * FROM "fluxdb_pr46_acceptance"."#;
        let items = controller
            .query_completions_for_text(
                connection_id,
                Some("fluxdb_pr46".into()),
                Some("fluxdb_pr46_acceptance".into()),
                text.into(),
                text.len(),
                true,
            )
            .expect("qualified completions")
            .items;
        assert!(
            items
                .iter()
                .all(|item| item.label != "pg_display_probe"),
            "explicit schema completion must not merge other schemas: {:?}",
            items
                .iter()
                .map(|item| (&item.label, &item.insert_text))
                .collect::<Vec<_>>()
        );
    }
}

/// T054：选中项完整对象说明的懒加载状态。
///
/// `Loading` 由调用方在发起懒解析前赋值（等待索引读取 / 异步期间展示）；
/// `Ready` 携带结构元数据文本；`Error` 携带原因。
#[derive(Clone, Debug)]
pub enum CompletionDocumentationState {
    Loading,
    Ready(String),
    Error(String),
}

/// 历史记录的表名提取：表触发词后第一个标识符的末段，加上 DDL 影响表。
/// 旧的作用域符号收集已删，这里只做历史分组所需的轻量词法扫描。
fn query_history_tables(sql: &str) -> Vec<String> {
    let tokens = sql_identifier_tokens(sql);
    let mut tables: BTreeSet<String> = tokens
        .windows(2)
        .filter(|window| {
            // 只跳过纯数值 token（如 LIMIT 1000）；按首字符判断会误杀
            // `3d_active_history` 这类数字开头的表名。
            matches!(
                window[0].to_ascii_lowercase().as_str(),
                "from" | "join" | "update" | "into" | "table"
            ) && window[1].parse::<f64>().is_err()
        })
        .map(|window| window[1].to_ascii_lowercase())
        .collect();
    if let Some(impact) = sql_ddl_impact(sql) {
        tables.extend(impact.tables);
    }
    tables.into_iter().collect()
}

fn query_history_kind(sql: &str) -> QueryHistoryKind {
    let keyword = sql_identifier_tokens(sql)
        .first()
        .map(|keyword| keyword.to_ascii_lowercase());
    match keyword.as_deref() {
        Some("select" | "with" | "show" | "describe" | "desc" | "explain") => {
            QueryHistoryKind::Query
        }
        Some("insert" | "update" | "delete" | "replace" | "merge") => QueryHistoryKind::DataChange,
        Some(
            "create" | "alter" | "drop" | "truncate" | "rename" | "grant" | "revoke" | "use",
        ) => QueryHistoryKind::SchemaChange,
        _ => QueryHistoryKind::Query,
    }
}

#[cfg(test)]
mod identifier_quote_tests {
    use super::*;

    #[test]
    fn virtual_column_keeps_unquoted_insert_text() {
        let mut item = QueryCompletionItem {
            label: "it.item_total".into(),
            insert_text: "it.item_total".into(),
            kind: QueryCompletionKind::Column,
            quote_identifier: false,
            ..Default::default()
        };
        quote_completion_insert_text(&mut item, DatabaseKind::Oracle, &|_| true);
        assert_eq!(item.insert_text, "it.item_total");

        let mut physical = QueryCompletionItem {
            label: "c.CUSTOMER_NAME".into(),
            insert_text: "c.CUSTOMER_NAME".into(),
            kind: QueryCompletionKind::Column,
            ..Default::default()
        };
        quote_completion_insert_text(&mut physical, DatabaseKind::Oracle, &|_| true);
        assert_eq!(physical.insert_text, "\"c\".\"CUSTOMER_NAME\"");
    }
}

#[cfg(test)]
mod query_history_table_tests {
    use super::*;

    #[test]
    fn numeric_prefix_table_survives_limit_literal() {
        let tables = query_history_tables("select * from `3d_active_history` LIMIT 1000");
        assert_eq!(tables, vec!["3d_active_history"]);
    }

    #[test]
    fn numeric_literal_after_from_is_not_a_table() {
        assert!(query_history_tables("select * from 1000").is_empty());
    }
}
