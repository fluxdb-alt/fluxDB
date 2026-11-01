impl AppController {
    /// MongoDB 集合工作台 / 连接信息 / 集合增删 / 命令执行。
    fn dispatch_mongo_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::OpenMongoCollection { object, read_only } => {
                self.open_mongo_collection(object, read_only)
            }
            AppCommand::BeginMongoLoad(tab_id) => self.begin_mongo_load(tab_id),
            AppCommand::LoadMongoDocuments {
                tab_id,
                generation,
                query,
            } => self.load_mongo_documents(tab_id, generation, query),
            AppCommand::FinishMongoDocuments {
                tab_id,
                generation,
                result,
            } => self.finish_mongo_documents(tab_id, generation, result),
            AppCommand::LoadMongoCollectionInfo(tab_id) => self.load_mongo_collection_info(tab_id),
            AppCommand::FinishMongoCollectionInfo { tab_id, result } => {
                self.finish_mongo_collection_info(tab_id, result)
            }
            AppCommand::LoadMongoFieldSample { tab_id, object } => {
                self.load_mongo_field_sample(tab_id, object)
            }
            AppCommand::LoadMongoIndexes(tab_id) => self.load_mongo_indexes(tab_id),
            AppCommand::FinishMongoIndexes { tab_id, result } => {
                self.finish_mongo_indexes(tab_id, result)
            }
            AppCommand::LoadMongoServerInfo(connection_id) => {
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                match connector_for(&config)
                    .and_then(|connector| connector.mongo_server_info(connection_id))
                {
                    Ok(info) => AppEvent::MongoServerInfoLoaded(connection_id, Ok(info)),
                    Err(error) => {
                        let user_error = UserFacingError::from(error);
                        self.state.last_error = Some(user_error.clone());
                        AppEvent::MongoServerInfoLoaded(connection_id, Err(user_error))
                    }
                }
            }
            AppCommand::CreateMongoCollection {
                connection_id,
                database,
                collection,
            } => self.create_mongo_collection(connection_id, database, collection),
            AppCommand::DropMongoCollection {
                connection_id,
                database,
                collection,
            } => self.drop_mongo_collection(connection_id, database, collection),
            AppCommand::RenameMongoCollection {
                connection_id,
                database,
                collection,
                new_name,
            } => self.rename_mongo_collection(connection_id, database, collection, new_name),
            AppCommand::MongoRunCommand {
                connection_id,
                database,
                command,
            } => self.run_mongo_command(connection_id, database, command),
            AppCommand::OpenMongoCli {
                connection_id,
                database,
            } => self.open_mongo_cli(connection_id, database),
            AppCommand::SelectMongoSection { tab_id, section } => {
                let Some(state) = self.find_mongo_collection_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
                };
                state.section = section;
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SelectMongoDisplayMode { tab_id, mode } => {
                let Some(state) = self.find_mongo_collection_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
                };
                state.display = mode;
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetMongoOffset { tab_id, offset } => {
                let Some(state) = self.find_mongo_collection_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
                };
                state.pagination = Pagination::new(offset, state.pagination.limit);
                // 起点只有一个来源：面板的 skip。翻页后同步写回，刷新/重开标签页才不会
                // 拿着旧起点又从头查一遍。
                state.applied_query.skip = offset.to_string();
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SelectMongoDocument { tab_id, index } => {
                let Some(state) = self.find_mongo_collection_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
                };
                // 下标越界（换页后旧选中）视为关闭抽屉，不留悬空选中态。
                state.selected_document = index.filter(|index| {
                    state
                        .page
                        .as_ref()
                        .is_some_and(|page| *index < page.documents.len())
                });
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SaveMongoDocument {
                tab_id,
                before_json,
                after_json,
            } => self.save_mongo_document(tab_id, before_json, after_json),
            AppCommand::DeleteMongoDocument {
                tab_id,
                document_json,
            } => self.delete_mongo_document(tab_id, document_json),
            AppCommand::InsertMongoDocument {
                tab_id,
                document_json,
            } => self.insert_mongo_document(tab_id, document_json),
            AppCommand::UpdateMongoAggregation { tab_id, pipeline } => {
                let Some(state) = self.find_mongo_collection_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
                };
                state.aggregation_pipeline = pipeline;
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::BeginMongoAggregation(tab_id) => {
                let Some(state) = self.find_mongo_collection_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
                };
                let generation = state.begin_aggregation();
                AppEvent::MongoAggregationStarted { tab_id, generation }
            }
            AppCommand::RunMongoAggregation {
                tab_id,
                generation,
                pipeline,
            } => self.run_mongo_aggregation(tab_id, generation, pipeline),
            AppCommand::FinishMongoAggregation {
                tab_id,
                generation,
                result,
            } => self.finish_mongo_aggregation(tab_id, generation, result),
            AppCommand::RunMongoAggregationPreview { tab_id, pipeline } => {
                self.run_mongo_aggregation_preview(tab_id, pipeline)
            }
            _ => self.fail(Error::new(ErrorKind::Internal, "非 MongoDB 命令")),
        }
    }

    /// 打开集合工作台：同一集合复用已有标签页，视图以只读打开。
    fn open_mongo_collection(&mut self, object: ObjectPath, read_only: bool) -> AppEvent {
        if let Some(tab_id) = self.state.tabs.iter().find_map(|tab| match &tab.kind {
            TabKind::MongoCollection(state) if state.object == object => Some(tab.id),
            _ => None,
        }) {
            self.state.active_tab = Some(tab_id);
            return AppEvent::TabActivated(tab_id);
        }
        let tab_id = self.next_tab_id();
        // 条数跟随「默认查询限制」而不是数据表页大小：两者在设置里是独立项，
        // 集合工作台查的是查询结果，语义上属于前者。
        let page_size = mongo_query_limit(self.state.settings.page_size);
        let title = object.name.clone();
        self.push_tab(TabState {
            id: tab_id,
            title,
            kind: TabKind::MongoCollection(MongoCollectionState::new(object, read_only, page_size)),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    /// 打开（或复用）MongoDB 命令行标签页：同一连接 + 同一库只保留一个。
    fn open_mongo_cli(&mut self, connection_id: ConnectionId, database: String) -> AppEvent {
        let cli = MongoCliState {
            connection_id,
            database,
        };
        if let Some(tab_id) = self.find_tab_by_session_key(&cli.session_key()) {
            self.state.active_tab = Some(tab_id);
            return AppEvent::TabActivated(tab_id);
        }
        let tab_id = self.next_tab_id();
        // 与语言包 `app.tabs.mongo_cli` / adapter 标题统一叫 Shell；展示层会按语言包覆盖标题，
        // 但 AI 上下文与拖拽预览直接用这里的字符串。
        let title = format!("MongoDB Shell - {}", cli.database);
        self.push_tab(TabState {
            id: tab_id,
            title,
            kind: TabKind::MongoCli(cli),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    fn find_mongo_collection_mut(&mut self, tab_id: TabId) -> Option<&mut MongoCollectionState> {
        self.find_tab_mut(tab_id)
            .and_then(|tab| match &mut tab.kind {
                TabKind::MongoCollection(state) => Some(state),
                _ => None,
            })
    }

    fn begin_mongo_load(&mut self, tab_id: TabId) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        let generation = state.begin_load();
        AppEvent::MongoLoadStarted { tab_id, generation }
    }

    /// 执行查询。查询草稿在这里落到标签页（`applied_query`），后台只用快照，避免并发改表单。
    fn load_mongo_documents(
        &mut self,
        tab_id: TabId,
        generation: u64,
        query: MongoQueryDraft,
    ) -> AppEvent {
        let Some(tab) = self.find_tab(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        let TabKind::MongoCollection(state) = &tab.kind else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        if !state.accepts_load(generation) {
            return AppEvent::MongoLoadDiscarded { tab_id, generation };
        }
        let object = state.object.clone();
        // 输入框已按上限校验，这里再夹一次：命令也可能由 AI/恢复路径直接派发，
        // 不能让超过连接器上限的值走到分页器（分页器按 limit 算页码，值变了页码就错）。
        let limit = mongo_query_limit(
            query
                .limit
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|limit| *limit > 0)
                .unwrap_or(state.pagination.limit),
        );
        // 起始位置以查询面板的 `skip` 为准：用户可以手填任意起点，翻页按钮也把它写回这一格，
        // 因此分页器必须跟着走，不能各存一份（否则页码与真实起点会对不上）。
        let offset = query.skip_value();

        if let Some(state) = self.find_mongo_collection_mut(tab_id) {
            state.pagination = Pagination::new(offset, limit);
            state.applied_query = query.clone();
            state.loading = true;
            state.error = None;
            state.selected_document = None;
        }

        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let request = query.to_request(&object.name);
        match connector_for(&config)
            .and_then(|connector| connector.mongo_find(&object, &request, &|| false))
        {
            Ok(page) => AppEvent::MongoDocumentsLoaded(tab_id, page),
            Err(error) => AppEvent::Failed(UserFacingError::from(error)),
        }
    }

    fn finish_mongo_documents(
        &mut self,
        tab_id: TabId,
        generation: u64,
        result: std::result::Result<MongoDocumentsPage, UserFacingError>,
    ) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        if !state.accepts_load(generation) {
            return AppEvent::MongoLoadDiscarded { tab_id, generation };
        }
        state.loading = false;
        match result {
            Ok(page) => {
                state.error = None;
                // 当前页本身就是最好的字段来源：抽样还没回来（或抽样失败）时，
                // 用户至少能补全眼前这页见过的字段。
                let documents = page
                    .documents
                    .iter()
                    .map(|document| document.json.clone())
                    .collect::<Vec<_>>();
                state.field_catalog.extend_from_documents(&documents);
                state.page = Some(page.clone());
                AppEvent::MongoDocumentsLoaded(tab_id, page)
            }
            Err(error) => {
                state.error = Some(error.clone());
                self.state.last_error = Some(error.clone());
                AppEvent::Failed(error)
            }
        }
    }

    fn run_mongo_aggregation(
        &mut self,
        tab_id: TabId,
        generation: u64,
        pipeline: String,
    ) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        if !state.accepts_aggregation(generation) {
            return AppEvent::MongoLoadDiscarded { tab_id, generation };
        }
        state.aggregation_pipeline = pipeline.clone();
        let object = state.object.clone();
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match connector_for(&config)
            .and_then(|connector| connector.mongo_aggregate(&object, &pipeline))
        {
            Ok(page) => AppEvent::MongoAggregationLoaded(tab_id, page),
            Err(error) => AppEvent::Failed(UserFacingError::from(error)),
        }
    }

    /// 单阶段预览：与正式聚合走同一个连接器，但不分配令牌、不写 `aggregation`。
    ///
    /// 预览是「看一眼这一步输出什么」的辅助手段，改了参数就重跑；把它写进标签页
    /// 状态会让结果区显示一段用户没点「执行」的管道。
    fn run_mongo_aggregation_preview(&mut self, tab_id: TabId, pipeline: String) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        let object = state.object.clone();
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let result = connector_for(&config)
            .and_then(|connector| connector.mongo_aggregate(&object, &pipeline))
            .map_err(UserFacingError::from);
        AppEvent::MongoAggregationPreviewLoaded {
            tab_id,
            pipeline,
            result,
        }
    }

    fn finish_mongo_aggregation(
        &mut self,
        tab_id: TabId,
        generation: u64,
        result: std::result::Result<MongoDocumentsPage, UserFacingError>,
    ) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        if !state.accepts_aggregation(generation) {
            return AppEvent::MongoLoadDiscarded { tab_id, generation };
        }
        match result {
            Ok(page) => {
                state.aggregation = LoadState::Loaded(page.clone());
                AppEvent::MongoAggregationLoaded(tab_id, page)
            }
            Err(error) => {
                state.aggregation = LoadState::Failed(error.clone());
                self.state.last_error = Some(error.clone());
                AppEvent::Failed(error)
            }
        }
    }

    /// 整篇替换文档。只读（视图）标签页直接拒绝，避免把写入送到不可写的集合。
    fn save_mongo_document(
        &mut self,
        tab_id: TabId,
        before_json: String,
        after_json: String,
    ) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        if state.read_only {
            return self.fail(Error::new(ErrorKind::Query, "视图不可修改"));
        }
        let object = state.object.clone();
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let database = object.database.clone().unwrap_or_default();
        match connector_for(&config).and_then(|connector| {
            connector.mongo_replace_document(
                object.connection_id,
                &database,
                &object.name,
                &before_json,
                &after_json,
            )
        }) {
            Ok(()) => {
                if let Some(state) = self.find_mongo_collection_mut(tab_id) {
                    // 保存后立即关闭抽屉：重新查询会重建本页文档，旧下标已无意义。
                    state.selected_document = None;
                }
                AppEvent::MongoDocumentSaved(tab_id)
            }
            Err(error) => {
                let user_error = UserFacingError::from(error);
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 删除一篇文档。只读（视图）标签页直接拒绝，与整篇替换同一条前置检查。
    fn delete_mongo_document(&mut self, tab_id: TabId, document_json: String) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        if state.read_only {
            return self.fail(Error::new(ErrorKind::Query, "视图不可修改"));
        }
        let object = state.object.clone();
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let database = object.database.clone().unwrap_or_default();
        match connector_for(&config).and_then(|connector| {
            connector.mongo_delete_document(
                object.connection_id,
                &database,
                &object.name,
                &document_json,
            )
        }) {
            Ok(()) => {
                if let Some(state) = self.find_mongo_collection_mut(tab_id) {
                    // 本页文档已重排，旧下标（抽屉/选中态）不再指向被删的那篇。
                    state.selected_document = None;
                }
                AppEvent::MongoDocumentDeleted(tab_id)
            }
            Err(error) => {
                let user_error = UserFacingError::from(error);
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    /// 插入一篇新文档。只读（视图）标签页同样拒绝，与删除/替换一致。
    fn insert_mongo_document(&mut self, tab_id: TabId, document_json: String) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        if state.read_only {
            return self.fail(Error::new(ErrorKind::Query, "视图不可修改"));
        }
        let object = state.object.clone();
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let database = object.database.clone().unwrap_or_default();
        match connector_for(&config).and_then(|connector| {
            connector.mongo_insert_document(
                object.connection_id,
                &database,
                &object.name,
                &document_json,
            )
        }) {
            Ok(id) => {
                if let Some(state) = self.find_mongo_collection_mut(tab_id) {
                    // 新文档的 `_id` 由服务端生成，旧下标/抽屉不再对应当前页。
                    state.selected_document = None;
                }
                AppEvent::MongoDocumentInserted(tab_id, id)
            }
            Err(error) => {
                let user_error = UserFacingError::from(error);
                self.state.last_error = Some(user_error.clone());
                AppEvent::Failed(user_error)
            }
        }
    }

    fn load_mongo_collection_info(&mut self, tab_id: TabId) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        state.info = LoadState::Loading;
        let object = state.object.clone();
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match connector_for(&config).and_then(|connector| connector.mongo_collection_info(&object))
        {
            Ok(info) => AppEvent::MongoCollectionInfoLoaded(tab_id, info),
            Err(error) => AppEvent::Failed(UserFacingError::from(error)),
        }
    }

    fn finish_mongo_collection_info(
        &mut self,
        tab_id: TabId,
        result: std::result::Result<MongoCollectionInfo, UserFacingError>,
    ) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        match result {
            Ok(info) => {
                state.info = LoadState::Loaded(info.clone());
                AppEvent::MongoCollectionInfoLoaded(tab_id, info)
            }
            Err(error) => {
                state.info = LoadState::Failed(error.clone());
                AppEvent::Failed(error)
            }
        }
    }

    /// 采样一批文档，摊平成补全用的字段目录，并并入标签页已有的目录。
    ///
    /// 失败不报错：字段目录只是补全的加分项，抽样失败时补全仍有运算符与构造器，
    /// 为此在集合页弹一个错误块是本末倒置。
    fn load_mongo_field_sample(&mut self, tab_id: TabId, object: ObjectPath) -> AppEvent {
        let documents = self
            .connection_config(object.connection_id)
            .cloned()
            .and_then(|config| connector_for(&config).ok())
            .and_then(|connector| {
                connector
                    .mongo_sample_documents(&object, MONGO_FIELD_SAMPLE_SIZE)
                    .ok()
            })
            .unwrap_or_default();
        // 拿不到标签页（用户已关标签）时直接返回：抽样结果没有落点了。
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return AppEvent::MongoFieldSampleLoaded(tab_id, MongoFieldCatalog::default());
        };
        state.field_catalog.extend_from_documents(&documents);
        AppEvent::MongoFieldSampleLoaded(tab_id, state.field_catalog.clone())
    }

    fn load_mongo_indexes(&mut self, tab_id: TabId) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        state.indexes = LoadState::Loading;
        let object = state.object.clone();
        let Some(config) = self.connection_config(object.connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match connector_for(&config).and_then(|connector| connector.mongo_indexes(&object)) {
            Ok(indexes) => AppEvent::MongoIndexesLoaded(tab_id, indexes),
            Err(error) => AppEvent::Failed(UserFacingError::from(error)),
        }
    }

    fn finish_mongo_indexes(
        &mut self,
        tab_id: TabId,
        result: std::result::Result<Vec<MongoIndexInfo>, UserFacingError>,
    ) -> AppEvent {
        let Some(state) = self.find_mongo_collection_mut(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "集合标签页不存在"));
        };
        match result {
            Ok(indexes) => {
                state.indexes = LoadState::Loaded(indexes.clone());
                AppEvent::MongoIndexesLoaded(tab_id, indexes)
            }
            Err(error) => {
                state.indexes = LoadState::Failed(error.clone());
                AppEvent::Failed(error)
            }
        }
    }

    /// 创建集合，并按新的对象树回填连接（新集合立刻出现在侧边栏）。
    fn create_mongo_collection(
        &mut self,
        connection_id: ConnectionId,
        database: String,
        collection: String,
    ) -> AppEvent {
        let Some(connection) = self
            .state
            .connections
            .iter_mut()
            .find(|connection| connection.config.id == connection_id)
        else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        let config = connection.config.clone();
        if let Err(error) = connector_for(&config).and_then(|connector| {
            connector.mongo_create_collection(connection_id, &database, &collection)
        }) {
            return self.fail(error);
        }
        match list_objects_for_connection(&config, None) {
            Ok(objects) => {
                self.state.last_error = None;
                connection.connected = true;
                connection.expanded = true;
                replace_connection_level0(connection, objects);
                AppEvent::MongoCollectionCreated {
                    connection_id,
                    database,
                    collection,
                }
            }
            Err(error) => self.fail(error),
        }
    }

    /// 删除集合：成功则关闭对应标签页并刷新对象树。
    fn drop_mongo_collection(
        &mut self,
        connection_id: ConnectionId,
        database: String,
        collection: String,
    ) -> AppEvent {
        let Some(config) = self.connection_config(connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        if let Err(error) = connector_for(&config).and_then(|connector| {
            connector.mongo_drop_collection(connection_id, &database, &collection)
        }) {
            return self.fail(error);
        }
        self.state.tabs.retain(|tab| match &tab.kind {
            TabKind::MongoCollection(state) => {
                state.object.connection_id != connection_id
                    || state.object.database.as_deref() != Some(database.as_str())
                    || state.object.name != collection
            }
            _ => true,
        });
        match list_objects_for_connection(&config, None) {
            Ok(objects) => {
                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                {
                    replace_connection_level0(connection, objects);
                }
                AppEvent::MongoCollectionDropped {
                    connection_id,
                    database,
                    collection,
                }
            }
            Err(error) => self.fail(error),
        }
    }

    /// 重命名集合：成功后重取对象树，并就地改写已打开的集合标签页（标题 + 对象）。
    fn rename_mongo_collection(
        &mut self,
        connection_id: ConnectionId,
        database: String,
        collection: String,
        new_name: String,
    ) -> AppEvent {
        let Some(config) = self.connection_config(connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        if let Err(error) = connector_for(&config).and_then(|connector| {
            connector.mongo_rename_collection(connection_id, &database, &collection, &new_name)
        }) {
            return self.fail(error);
        }
        for tab in &mut self.state.tabs {
            if let TabKind::MongoCollection(state) = &mut tab.kind
                && state.object.connection_id == connection_id
                && state.object.database.as_deref() == Some(database.as_str())
                && state.object.name == collection
            {
                state.object.name = new_name.clone();
                tab.title = new_name.clone();
            }
        }
        match list_objects_for_connection(&config, None) {
            Ok(objects) => {
                self.state.last_error = None;
                if let Some(connection) = self
                    .state
                    .connections
                    .iter_mut()
                    .find(|connection| connection.config.id == connection_id)
                {
                    connection.connected = true;
                    replace_connection_level0(connection, objects);
                }
                AppEvent::MongoCollectionRenamed {
                    connection_id,
                    database,
                    collection,
                    new_name,
                }
            }
            Err(error) => self.fail(error),
        }
    }

    fn run_mongo_command(
        &mut self,
        connection_id: ConnectionId,
        database: String,
        command: String,
    ) -> AppEvent {
        let Some(config) = self.connection_config(connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        match connector_for(&config)
            .and_then(|connector| connector.mongo_run_command(connection_id, &database, &command))
        {
            Ok(result) => AppEvent::MongoCommandExecuted {
                connection_id,
                database,
                command,
                result,
            },
            Err(error) => self.fail(error),
        }
    }
}
