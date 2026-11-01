// MongoDB Connector 实现：对象树、文档读取、集合/索引元数据、库与集合的新建/删除、命令执行。

/// 侧边栏展示的库上限：`listDatabaseNames` 在超大实例上可能返回数千个库，
/// 只用于浏览，超出部分由库搜索覆盖（与 Redis 侧边栏 SCAN 上限同一取舍）。
const MONGO_SIDEBAR_DATABASE_LIMIT: usize = 200;

/// 单次 `find` 的文档上限：防止一次拉取把内存打爆（UI 分页远小于此值）。
///
/// 真值在 core（`MONGO_MAX_DOCUMENTS_PER_QUERY`）：输入框校验要用同一个上限，
/// 这里只做别名，避免两处各写一个数后出现「填得进去、执行时被改小」。
const MONGO_MAX_DOCUMENTS_PER_QUERY: u64 = fluxdb_core::MONGO_MAX_DOCUMENTS_PER_QUERY;

/// 系统库：可以连接但不在对象树里展示（与 Compass 一致）。
const MONGO_SYSTEM_DATABASES: [&str; 3] = ["admin", "local", "config"];

#[derive(Clone, Debug, Default)]
pub struct MongoConnector {
    config: Option<ConnectionConfig>,
}

impl MongoConnector {
    pub fn new() -> Self {
        Self { config: None }
    }

    pub fn with_config(config: ConnectionConfig) -> Self {
        Self {
            config: Some(config),
        }
    }

    fn config(&self) -> fluxdb_core::Result<&ConnectionConfig> {
        self.config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MongoDB 操作需要连接配置上下文"))
    }

    /// 执行任意数据库命令（查询编辑器）。
    pub fn run_command(&self, database: &str, command: &str) -> fluxdb_core::Result<String> {
        let config = self.config()?;
        mongo_run_command(config, database, command)
    }
}

impl Connector for MongoConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MongoDb
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        // 用 `ping` 验证认证与选主，而不是只建客户端（建客户端不做握手）。
        let client = mongo_connect(config)?;
        let database = {
            let profile_database = config
                .mongo_profile
                .as_ref()
                .map(|profile| profile.basic.auth_database.clone())
                .unwrap_or_default();
            if profile_database.is_empty() {
                "admin".to_string()
            } else {
                profile_database
            }
        };
        mongo_block_on(20, "连接 MongoDB", async move {
            client
                .database(&database)
                .run_command(doc! {"ping": 1})
                .await
        })
        .map(|_| ())
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        let config = self.config()?;
        match path {
            None => mongo_list_databases(config),
            Some(path) if path.kind == ObjectKind::Database => {
                let database = path.database.clone().unwrap_or_else(|| path.name.clone());
                mongo_list_collections(config, path.connection_id, &database)
            }
            // 集合之下没有子对象：返回空而不是报错，避免树展开时弹错误。
            Some(path) if path.kind == ObjectKind::Collection => Ok(Vec::new()),
            Some(_) => Err(Error::new(
                ErrorKind::Unsupported,
                "MongoDB 仅支持数据库与集合浏览",
            )),
        }
    }

    fn load_data(
        &self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
        sort: &[SortSpec],
        _filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        // 集合在统一 `DataEditor` 里也用 `load_data` 拉首屏；转成 `find` 保持一条读取路径，
        // 避免 UI 侧为 Mongo 单独写一套分页。
        if path.kind != ObjectKind::Collection {
            return Err(Error::new(ErrorKind::Unsupported, "MongoDB 仅支持集合读取"));
        }
        let config = self.config()?;
        let sort_json = mongo_sort_to_extjson(sort);
        let request = MongoFindRequest {
            collection: path.name.clone(),
            sort: sort_json,
            skip: offset,
            limit,
            ..MongoFindRequest::default()
        };
        let database = path
            .database
            .clone()
            .ok_or_else(|| Error::new(ErrorKind::Query, "集合缺少所属数据库"))?;
        let page = mongo_find(config, &database, &request, &|| false)?;
        Ok(mongo_page_to_data_page(page, offset, limit))
    }

    fn mongo_find(
        &self,
        path: &ObjectPath,
        request: &MongoFindRequest,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<MongoDocumentsPage> {
        let config = self.config()?;
        let database = path
            .database
            .clone()
            .ok_or_else(|| Error::new(ErrorKind::Query, "集合缺少所属数据库"))?;
        mongo_find(config, &database, request, should_cancel)
    }

    fn mongo_aggregate(
        &self,
        path: &ObjectPath,
        pipeline: &str,
    ) -> fluxdb_core::Result<MongoDocumentsPage> {
        mongo_aggregate(self.config()?, path, pipeline)
    }

    fn mongo_sample_documents(
        &self,
        path: &ObjectPath,
        size: u32,
    ) -> fluxdb_core::Result<Vec<String>> {
        mongo_sample_documents(self.config()?, path, size)
    }

    fn mongo_collection_info(&self, path: &ObjectPath) -> fluxdb_core::Result<MongoCollectionInfo> {
        let config = self.config()?;
        let database = path
            .database
            .clone()
            .ok_or_else(|| Error::new(ErrorKind::Query, "集合缺少所属数据库"))?;
        mongo_collection_info(config, &database, &path.name)
    }

    fn mongo_indexes(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<MongoIndexInfo>> {
        let config = self.config()?;
        let database = path
            .database
            .clone()
            .ok_or_else(|| Error::new(ErrorKind::Query, "集合缺少所属数据库"))?;
        mongo_indexes(config, &database, &path.name)
    }

    fn mongo_replace_document(
        &self,
        _: ConnectionId,
        database: &str,
        collection: &str,
        before_json: &str,
        after_json: &str,
    ) -> fluxdb_core::Result<()> {
        mongo_replace_document(
            self.config()?,
            database,
            collection,
            before_json,
            after_json,
        )
    }

    fn mongo_delete_document(
        &self,
        _: ConnectionId,
        database: &str,
        collection: &str,
        document_json: &str,
    ) -> fluxdb_core::Result<()> {
        mongo_delete_document(self.config()?, database, collection, document_json)
    }

    fn mongo_insert_document(
        &self,
        _: ConnectionId,
        database: &str,
        collection: &str,
        document_json: &str,
    ) -> fluxdb_core::Result<String> {
        mongo_insert_document(self.config()?, database, collection, document_json)
    }

    fn mongo_create_collection(
        &self,
        _: ConnectionId,
        database: &str,
        collection: &str,
    ) -> fluxdb_core::Result<()> {
        mongo_create_collection(self.config()?, database, collection)
    }

    fn mongo_drop_collection(
        &self,
        _: ConnectionId,
        database: &str,
        collection: &str,
    ) -> fluxdb_core::Result<()> {
        mongo_drop_collection(self.config()?, database, collection)
    }

    fn mongo_drop_index(
        &self,
        _: ConnectionId,
        database: &str,
        collection: &str,
        name: &str,
    ) -> fluxdb_core::Result<()> {
        mongo_drop_index(self.config()?, database, collection, name)
    }

    fn mongo_rename_collection(
        &self,
        _: ConnectionId,
        database: &str,
        collection: &str,
        new_name: &str,
    ) -> fluxdb_core::Result<()> {
        mongo_rename_collection(self.config()?, database, collection, new_name)
    }

    fn mongo_drop_database(&self, _: ConnectionId, database: &str) -> fluxdb_core::Result<()> {
        mongo_drop_database(self.config()?, database)
    }

    fn mongo_run_command(
        &self,
        _: ConnectionId,
        database: &str,
        command: &str,
    ) -> fluxdb_core::Result<String> {
        mongo_run_command(self.config()?, database, command)
    }

    fn mongo_shell(
        &self,
        _: ConnectionId,
        database: &str,
        input: &str,
    ) -> fluxdb_core::Result<fluxdb_core::MongoShellResult> {
        mongo_shell(self.config()?, database, input)
    }

    fn mongo_server_info(&self, _: ConnectionId) -> fluxdb_core::Result<MongoServerInfo> {
        mongo_server_info(self.config()?)
    }

    fn mongo_preview_import(
        &self,
        request: &MongoImportPreviewRequest,
    ) -> fluxdb_core::Result<MongoImportPreview> {
        mongo_preview_import(request)
    }

    fn mongo_import_documents(
        &self,
        request: &MongoImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(MongoImportProgress),
    ) -> fluxdb_core::Result<MongoImportReport> {
        mongo_import_documents(self.config()?, request, cancel, progress)
    }

    fn create_database(&self, request: &CreateDatabaseRequest) -> fluxdb_core::Result<()> {
        // MongoDB 的库由首个集合隐式创建：没有集合就删掉，库也就不存在。
        // 因此 CreateDatabaseRequest 必须带集合名（`charset` 位置在该语义下复用为集合名，
        // 由 UI 明确填入），否则拒绝而不是建出一个空库。
        let collection = request.charset.trim();
        if collection.is_empty() {
            return Err(Error::new(
                ErrorKind::Query,
                "MongoDB 创建数据库需要同时指定首个集合名",
            ));
        }
        mongo_create_collection(self.config()?, &request.name, collection)
    }

    fn delete_database(&self, _: ConnectionId, database: &str) -> fluxdb_core::Result<()> {
        mongo_drop_database(self.config()?, database)
    }

    fn apply_changes(&self, _: &DataChangeSet) -> fluxdb_core::Result<AppliedChangeOutcome> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "MongoDB 暂不支持表格行编辑，请在 JSON 视图中修改文档",
        ))
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        // 命令行走 `mongo_run_command`：查询编辑器与命令行终端共用同一实现，
        // 单条命令的结果作为一行 `result`（Extended JSON）返回。
        let started = std::time::Instant::now();
        let database = request
            .database
            .clone()
            .unwrap_or_else(|| "admin".to_string());
        let text = mongo_run_command(self.config()?, &database, &request.text)?;
        let page = mongo_result_set(text);
        Ok(QueryExecutionResult {
            summaries: vec![QueryExecutionSummary {
                sql: request.text.clone(),
                kind: QueryStatementKind::ResultSet,
                success: true,
                message: String::new(),
                returned_rows: page.rows.len() as u64,
                affected_rows: 0,
                elapsed_ms: started.elapsed().as_millis() as u64,
            }],
            results: vec![page],
            rollback_snapshots: vec![None],
        })
    }
}

/// 文档页转统一 `DataPage`（表格/编辑器首屏用）。每行固定两列：`_id` 与文档 JSON。
fn mongo_page_to_data_page(page: MongoDocumentsPage, offset: u64, limit: u64) -> DataPage {
    let columns = vec![
        Column {
            name: "_id".to_string(),
            type_name: Some("ObjectId".to_string()),
            nullable: false,
            primary_key: true,
            comment: None,
        },
        Column {
            name: "document".to_string(),
            type_name: Some("Document".to_string()),
            nullable: false,
            primary_key: false,
            comment: None,
        },
    ];
    let rows = page
        .documents
        .iter()
        .map(|document| Row {
            values: vec![
                CellValue::Text(document.id.clone()),
                CellValue::Json(document.json.clone()),
            ],
        })
        .collect();
    DataPage {
        columns,
        rows,
        offset,
        limit,
        has_more: page.has_more,
        incomplete: None,
    }
}

/// 排序规格转 Extended JSON 文本，供 `find` 使用。
fn mongo_sort_to_extjson(sort: &[SortSpec]) -> String {
    if sort.is_empty() {
        return String::new();
    }
    let mut document = Document::new();
    for spec in sort {
        let direction = match spec.direction {
            SortDirection::Asc => 1i32,
            SortDirection::Desc => -1i32,
        };
        document.insert(spec.field.clone(), direction);
    }
    mongo_json_text(Bson::Document(document))
}

/// 命令结果转成单结果集，供查询编辑器展示。
fn mongo_result_set(text: String) -> DataPage {
    DataPage {
        columns: vec![Column {
            name: "result".to_string(),
            type_name: Some("Document".to_string()),
            nullable: false,
            primary_key: false,
            comment: None,
        }],
        rows: vec![Row {
            values: vec![CellValue::Json(text)],
        }],
        offset: 0,
        limit: 1,
        has_more: false,
        incomplete: None,
    }
}
