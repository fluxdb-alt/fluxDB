#[derive(Clone, Debug, Default)]
pub struct MySqlConnector {
    config: Option<ConnectionConfig>,
}

impl MySqlConnector {
    pub fn new() -> Self {
        Self { config: None }
    }

    pub fn with_config(config: ConnectionConfig) -> Self {
        Self {
            config: Some(config),
        }
    }
}

impl Connector for MySqlConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MySql
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        tracing::info!(
            target: "fluxdb_connectors",
            connection_id = ?config.id,
            connection = ?config.name,
            endpoint = ?config.endpoint,
            "MySQL 连接测试开始"
        );
        if !is_mysql_protocol_kind(config.kind) {
            return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
        }

        let (options, _tunnel) = mysql_dial(config)?;
        let profile = config.mysql_profile.as_ref();
        // 建连超时：档案优先，缺省 5 秒。
        let connect_timeout = profile
            .map(|p| Duration::from_secs(u64::from(p.connect_timeout_secs())))
            .unwrap_or(Duration::from_secs(5));
        // 查询超时：仅当 >0 时对探活 ping 生效（全量 per-query 延后到执行器）。
        let query_timeout = profile
            .and_then(|p| (p.advanced.query_timeout_secs > 0).then_some(p.advanced.query_timeout_secs))
            .map(|secs| Duration::from_secs(u64::from(secs)));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

        runtime.block_on(async {
            let connect = tokio::time::timeout(connect_timeout, options.connect()).await;
            let mut connection = match connect {
                Ok(Ok(connection)) => connection,
                Ok(Err(error)) => return Err(mysql_error(error)),
                Err(_) => return Err(Error::new(ErrorKind::Connection, "连接超时")),
            };

            let ping = match query_timeout {
                Some(limit) => tokio::time::timeout(limit, connection.ping()).await,
                None => Ok(connection.ping().await),
            };
            match ping {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(mysql_error(error)),
                Err(_) => return Err(Error::new(ErrorKind::Connection, "探活超时")),
            }
            connection.close().await.map_err(mysql_error)
        })
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "MySQL 对象浏览需要连接配置上下文",
            ));
        };

        mysql_list_objects(config, path)
    }

    fn create_database(&self, request: &CreateDatabaseRequest) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "MySQL 新建数据库需要连接配置上下文",
            ));
        };

        mysql_create_database(config, request)
    }

    fn delete_database(&self, connection_id: ConnectionId, database: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "MySQL 删除数据库需要连接配置上下文",
            ));
        };

        mysql_delete_database(config, connection_id, database)
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
                "MySQL 数据读取需要连接配置上下文",
            ));
        };

        mysql_load_data(config, path, offset, limit, sort, filters)
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
                "MySQL 导出预览需要连接配置上下文",
            ));
        };

        mysql_preview_data_export(config, path, fields, sort, filters)
    }

    fn apply_changes(&self, changes: &DataChangeSet) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "MySQL 数据提交需要连接配置上下文",
            ));
        };

        mysql_apply_changes(config, changes)
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
                "MySQL 二进制读取需要连接配置上下文",
            ));
        };

        mysql_load_cell_binary(config, path, identity, column)
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "MySQL SQL 执行需要连接配置上下文",
            ));
        };

        tracing::info!(
            target: "fluxdb_connectors",
            connection_id = ?request.connection_id,
            database = ?request.database,
            sql = %truncate_sql_for_log(&request.text),
            "MySQL SQL 执行开始"
        );
        mysql_execute_query(config, request)
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
                "MySQL SQL 执行需要连接配置上下文",
            ));
        };

        mysql_execute_query_with_progress(config, request, on_summary, should_cancel)
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
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_tables(config, database, schema, filter, limit)
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
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_tables_with_cancel(config, database, schema, filter, limit, should_cancel)
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
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_columns(config, database, schema, table)
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
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_columns_with_cancel(config, database, schema, table, should_cancel)
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
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_columns_for_tables(config, database, schema, tables)
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
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_columns_for_tables_with_cancel(config, database, schema, tables, should_cancel)
    }

    fn list_completion_routines(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_routines(config, database, schema, filter, limit)
    }

    fn list_completion_routines_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_routines_with_cancel(config, database, schema, filter, limit, should_cancel)
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
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_triggers(config, database, schema, filter, limit)
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
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 补全需要连接配置上下文"))?;
        mysql_completion_triggers_with_cancel(config, database, schema, filter, limit, should_cancel)
    }

    fn list_indexes(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 索引读取需要连接配置上下文"))?;
        mysql_indexes(config, path)
    }

    fn list_foreign_keys(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 外键读取需要连接配置上下文"))?;
        mysql_foreign_keys(config, path)
    }

    fn list_foreign_keys_with_cancel(
        &self,
        path: &ObjectPath,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL 外键读取需要连接配置上下文"))?;
        mysql_foreign_keys_with_cancel(config, path, should_cancel)
    }

    fn list_triggers(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<TriggerInfo>> {
        let config = self.config.as_ref().ok_or_else(|| {
            Error::new(ErrorKind::Connection, "MySQL 触发器读取需要连接配置上下文")
        })?;
        mysql_triggers(config, path)
    }

    fn table_ddl(&self, path: &ObjectPath) -> fluxdb_core::Result<String> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "MySQL DDL 读取需要连接配置上下文"))?;
        mysql_table_ddl(config, path)
    }
}
