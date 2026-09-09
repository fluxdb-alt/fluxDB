#[derive(Clone, Debug, Default)]
pub struct SqliteConnector {
    config: Option<ConnectionConfig>,
}

impl SqliteConnector {
    pub fn new() -> Self {
        Self { config: None }
    }

    pub fn with_config(config: ConnectionConfig) -> Self {
        Self {
            config: Some(config),
        }
    }
}

impl Connector for SqliteConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Sqlite
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        tracing::info!(
            target: "fluxdb_connectors",
            connection_id = ?config.id,
            connection = ?config.name,
            endpoint = ?config.endpoint,
            "SQLite 连接测试开始"
        );
        if config.kind != DatabaseKind::Sqlite {
            return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
        }

        let options = sqlite_connection_options(config)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

        runtime.block_on(async {
            let connect = tokio::time::timeout(Duration::from_secs(5), options.connect()).await;
            let mut connection = match connect {
                Ok(Ok(connection)) => connection,
                Ok(Err(error)) => return Err(sqlite_error(error)),
                Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
            };

            connection.ping().await.map_err(sqlite_error)?;
            connection.close().await.map_err(sqlite_error)
        })
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SQLite 对象浏览需要连接配置上下文",
            ));
        };

        sqlite_list_objects(config, path)
    }

    fn create_database(&self, request: &CreateDatabaseRequest) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SQLite 新建数据库需要连接配置上下文",
            ));
        };

        sqlite_create_database(config, request)
    }

    fn load_data(
        &self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SQLite 数据读取需要连接配置上下文",
            ));
        };

        sqlite_load_data(config, path, offset, limit, sort, filters)
    }

    fn preview_data_export(
        &self,
        path: &ObjectPath,
        fields: &[String],
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataExportPreview> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SQLite 导出预览需要连接配置上下文",
            ));
        };

        sqlite_preview_data_export(config, path, fields, sort, filters)
    }

    fn apply_changes(&self, changes: &DataChangeSet) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SQLite 数据提交需要连接配置上下文",
            ));
        };

        sqlite_apply_changes(config, changes)
    }

    fn load_cell_binary(
        &self,
        path: &ObjectPath,
        identity: &fluxdb_core::RowIdentity,
        column: &str,
    ) -> fluxdb_core::Result<Vec<u8>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SQLite 二进制读取需要连接配置上下文",
            ));
        };

        sqlite_load_cell_binary(config, path, identity, column)
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SQLite SQL 执行需要连接配置上下文",
            ));
        };

        sqlite_execute_query(config, request)
    }

    fn execute_with_progress(
        &self,
        request: &QueryRequest,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "SQLite SQL 执行需要连接配置上下文",
            ));
        };

        sqlite_execute_query_with_progress(config, request, on_summary, should_cancel)
    }

    fn list_completion_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 补全需要连接配置上下文"))?;
        sqlite_completion_tables(config, database, schema, filter, limit)
    }

    fn list_completion_tables_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 补全需要连接配置上下文"))?;
        sqlite_completion_tables_with_cancel(config, database, schema, filter, limit, should_cancel)
    }

    fn list_completion_columns(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 补全需要连接配置上下文"))?;
        sqlite_completion_columns(config, database, schema, table)
    }

    fn list_completion_columns_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 补全需要连接配置上下文"))?;
        sqlite_completion_columns_with_cancel(config, database, schema, table, should_cancel)
    }

    fn list_completion_columns_for_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 补全需要连接配置上下文"))?;
        sqlite_completion_columns_for_tables(config, database, schema, tables)
    }

    fn list_completion_columns_for_tables_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 补全需要连接配置上下文"))?;
        sqlite_completion_columns_for_tables_with_cancel(config, database, schema, tables, should_cancel)
    }

    fn list_completion_triggers(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 补全需要连接配置上下文"))?;
        sqlite_completion_triggers(config, database, schema, filter, limit)
    }

    fn list_completion_triggers_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 补全需要连接配置上下文"))?;
        sqlite_completion_triggers_with_cancel(config, database, schema, filter, limit, should_cancel)
    }

    fn list_indexes(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
        let config = self.config.as_ref().ok_or_else(|| {
            Error::new(ErrorKind::Connection, "SQLite 索引读取需要连接配置上下文")
        })?;
        sqlite_indexes(config, path)
    }

    fn list_foreign_keys(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        let config = self.config.as_ref().ok_or_else(|| {
            Error::new(ErrorKind::Connection, "SQLite 外键读取需要连接配置上下文")
        })?;
        sqlite_foreign_keys(config, path)
    }

    fn list_foreign_keys_with_cancel(
        &self,
        path: &ObjectPath,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "SQLite 外键读取需要连接配置上下文"))?;
        sqlite_foreign_keys_with_cancel(config, path, should_cancel)
    }

    fn list_triggers(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<TriggerInfo>> {
        let config = self.config.as_ref().ok_or_else(|| {
            Error::new(ErrorKind::Connection, "SQLite 触发器读取需要连接配置上下文")
        })?;
        sqlite_triggers(config, path)
    }

    fn table_ddl(&self, path: &ObjectPath) -> fluxdb_core::Result<String> {
        let config = self.config.as_ref().ok_or_else(|| {
            Error::new(ErrorKind::Connection, "SQLite DDL 读取需要连接配置上下文")
        })?;
        sqlite_table_ddl(config, path)
    }
}
