// PostgresConnector：实现同步 `Connector` trait（T04）。
//
// 整体保持 trait 的「无状态、每次调用按需拨号/复用会话」形态：`test_connection`
// 拨一条隔离连接做 `SELECT version()` 探活后即弃；`execute` 走共享 runtime 的会话。

#[derive(Clone, Default)]
pub struct PostgresConnector {
    config: Option<ConnectionConfig>,
}

impl PostgresConnector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: ConnectionConfig) -> Self {
        Self {
            config: Some(config),
        }
    }

    /// 取连接配置；缺失时按指定说明报「需连接配置上下文」错误。
    fn as_config(&self, message: &str) -> fluxdb_core::Result<&ConnectionConfig> {
        self.config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, message))
    }
}

impl Connector for PostgresConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Postgres
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL 对象浏览需要连接配置上下文",
            ));
        };
        pg_list_objects(config, path)
    }

    fn create_database(&self, request: &CreateDatabaseRequest) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL 新建数据库需要连接配置上下文",
            ));
        };
        pg_create_database(config, request)
    }

    fn delete_database(&self, connection_id: ConnectionId, database: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL 删除数据库需要连接配置上下文",
            ));
        };
        pg_delete_database(config, connection_id, database)
    }

    fn create_schema(&self, connection_id: ConnectionId, schema: &str) -> fluxdb_core::Result<()> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL 新建 schema 需要连接配置上下文",
            ));
        };
        pg_create_schema(config, connection_id, schema)
    }

    fn list_indexes(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL 索引元数据需要连接配置上下文",
            ));
        };
        pg_list_indexes(config, path)
    }

    fn list_foreign_keys(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL 外键元数据需要连接配置上下文",
            ));
        };
        pg_list_foreign_keys(config, path)
    }

    fn list_triggers(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<TriggerInfo>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL 触发器元数据需要连接配置上下文",
            ));
        };
        pg_list_triggers(config, path)
    }

    fn list_completion_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_tables(config, database, schema, filter, limit, &|| false)
    }

    fn list_completion_columns(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_columns(config, database, schema, table, &|| false)
    }

    fn list_completion_columns_for_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_columns_for_tables(config, database, schema, tables, &|| false)
    }

    fn list_completion_routines(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_routines(config, database, schema, filter, limit, &|| false)
    }

    fn list_completion_triggers(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_triggers(config, database, schema, filter, limit, &|| false)
    }

    // 取消支持：PG 补全的每次调用含「建连 + search_path + 主 catalog 查询」多段往返，
    // 仅靠 trait 默认的「调用前后各查一次」无法在往返之间中断，故这里显式下传 should_cancel，
    // 在每段往返之间提前返回空结果（§8.4 列表支持取消；不中断已发出的单条查询）。
    fn list_completion_tables_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_tables(config, database, schema, filter, limit, should_cancel)
    }

    fn list_completion_columns_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_columns(config, database, schema, table, should_cancel)
    }

    fn list_completion_columns_for_tables_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_columns_for_tables(config, database, schema, tables, should_cancel)
    }

    fn list_completion_routines_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_routines(config, database, schema, filter, limit, should_cancel)
    }

    fn list_completion_triggers_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let config = self.as_config("PostgreSQL 补全需要连接配置上下文")?;
        pg_list_completion_triggers(config, database, schema, filter, limit, should_cancel)
    }

    fn table_ddl(&self, path: &ObjectPath) -> fluxdb_core::Result<String> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL DDL 元数据需要连接配置上下文",
            ));
        };
        pg_table_ddl(config, path)
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
                "PostgreSQL 数据读取需要连接配置上下文",
            ));
        };
        pg_load_data(config, path, offset, limit, sort, filters)
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
                "PostgreSQL 导出预览需要连接配置上下文",
            ));
        };
        pg_preview_data_export(config, path, fields, sort, filters)
    }

    fn apply_changes(
        &self,
        changes: &DataChangeSet,
    ) -> fluxdb_core::Result<AppliedChangeOutcome> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "PostgreSQL 数据编辑提交需要连接配置上下文",
            ));
        };
        pg_apply_changes(config, changes)
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
                "PostgreSQL 二进制读取需要连接配置上下文",
            ));
        };
        pg_load_cell_binary(config, path, identity, column)
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        if config.kind != DatabaseKind::Postgres {
            return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
        }
        let database = pg_request_database(config, None);
        // 拨一条隔离短连接做真实建连 + 认证 + 版本读取，用后即弃。
        // 单一 block_on：拨号与查询都 `.await`，避免嵌套 block_on。
        pg_runtime().block_on(async {
            let session = pg_connect(config, &database).await?;
            let row = tokio::time::timeout(Duration::from_secs(10), session.client.query_one("SELECT version()", &[]))
                .await
                .map_err(|_| Error::new(ErrorKind::Connection, "连接超时"))?
                .map_err(pg_error)?;
            let version: String = row
                .try_get(0)
                .map_err(|error| Error::new(ErrorKind::Query, error.to_string()))?;
            tracing::info!(
                target: "fluxdb_connectors",
                connection_id = ?config.id,
                connection = ?config.name,
                version = %version,
                "PostgreSQL 连接测试成功"
            );
            Ok::<_, fluxdb_core::Error>(())
        })
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL SQL 执行需要连接配置上下文"))?;
        tracing::info!(
            target: "fluxdb_connectors",
            connection_id = ?request.connection_id,
            database = ?request.database,
            sql = %truncate_sql_for_log(&request.text),
            "PostgreSQL SQL 执行开始"
        );
        pg_execute_query(config, request)
    }

    fn execute_with_progress(
        &self,
        request: &QueryRequest,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "PostgreSQL SQL 执行需要连接配置上下文"))?;
        pg_execute_query_with_progress(config, request, on_summary, should_cancel)
    }
}
