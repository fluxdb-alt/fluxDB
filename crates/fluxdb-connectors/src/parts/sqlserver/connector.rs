#[derive(Clone, Default)]
pub struct SqlServerConnector {
    config: Option<ConnectionConfig>,
}

impl SqlServerConnector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: ConnectionConfig) -> Self {
        Self { config: Some(config) }
    }

    fn config(&self, message: &str) -> fluxdb_core::Result<&ConnectionConfig> {
        self.config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, message))
    }
}

impl Connector for SqlServerConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::SqlServer
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        let mut client = sqlserver_connect(config)?;
        let result = sqlserver_block_on(async {
            let result = sqlserver_run_batch(&mut client, "SELECT 1").await;
            sqlserver_close(client).await;
            result
        });
        result?;
        Ok(())
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        sqlserver_list_objects(
            self.config("SQL Server 对象浏览需要连接配置上下文")?,
            path,
        )
    }

    fn create_database(&self, request: &CreateDatabaseRequest) -> fluxdb_core::Result<()> {
        sqlserver_create_database(
            self.config("SQL Server 新建数据库需要连接配置上下文")?,
            request,
        )
    }

    fn delete_database(
        &self,
        connection_id: ConnectionId,
        database: &str,
    ) -> fluxdb_core::Result<()> {
        sqlserver_delete_database(
            self.config("SQL Server 删除数据库需要连接配置上下文")?,
            connection_id,
            database,
        )
    }

    fn create_schema(
        &self,
        connection_id: ConnectionId,
        database: &str,
        schema: &str,
    ) -> fluxdb_core::Result<()> {
        sqlserver_create_schema(
            self.config("SQL Server 新建 schema 需要连接配置上下文")?,
            connection_id,
            database,
            schema,
        )
    }

    fn load_data(
        &self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        sqlserver_load_data(
            self.config("SQL Server 数据读取需要连接配置上下文")?,
            path,
            offset,
            limit,
            sort,
            filters,
        )
    }

    fn load_data_expression(
        &self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
        sort: &[SortSpec],
        expression: &fluxdb_core::FilterExpression,
    ) -> fluxdb_core::Result<DataPage> {
        sqlserver_load_data_expression(
            self.config("SQL Server 数据读取需要连接配置上下文")?,
            path,
            offset,
            limit,
            sort,
            expression,
        )
    }

    fn preview_data_export(
        &self,
        path: &ObjectPath,
        fields: &[String],
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataExportPreview> {
        sqlserver_preview_data_export(
            self.config("SQL Server 导出预览需要连接配置上下文")?,
            path,
            fields,
            sort,
            filters,
        )
    }

    fn apply_changes(&self, changes: &DataChangeSet) -> fluxdb_core::Result<AppliedChangeOutcome> {
        sqlserver_apply_changes(
            self.config("SQL Server 数据提交需要连接配置上下文")?,
            changes,
        )
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        sqlserver_execute_query(
            self.config("SQL Server SQL 执行需要连接配置上下文")?,
            request,
        )
    }

    fn execute_with_progress(
        &self,
        request: &QueryRequest,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        sqlserver_execute_query_with_progress(
            self.config("SQL Server SQL 执行需要连接配置上下文")?,
            request,
            on_summary,
            should_cancel,
        )
    }

    fn execute_parameterized(
        &self,
        request: &fluxdb_core::ParameterizedQueryRequest,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        sqlserver_execute_parameterized(
            self.config("SQL Server SQL 执行需要连接配置上下文")?,
            &request.request,
            &request.parameters,
        )
    }

    fn list_indexes(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
        sqlserver_indexes_for(
            self.config("SQL Server 索引元数据需要连接配置上下文")?,
            path,
        )
    }

    fn list_foreign_keys(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        sqlserver_foreign_keys_for(
            self.config("SQL Server 外键元数据需要连接配置上下文")?,
            path,
        )
    }

    fn list_triggers(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<TriggerInfo>> {
        sqlserver_triggers_for(
            self.config("SQL Server 触发器元数据需要连接配置上下文")?,
            path,
        )
    }

    fn load_table_design_metadata(
        &self,
        path: &ObjectPath,
    ) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
        sqlserver_load_table_design_metadata(
            self.config("SQL Server 设计表元数据需要连接配置上下文")?,
            path,
            true,
        )
    }

    fn load_table_design_base_metadata(
        &self,
        path: &ObjectPath,
    ) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
        sqlserver_load_table_design_metadata(
            self.config("SQL Server 设计表基础元数据需要连接配置上下文")?,
            path,
            false,
        )
    }

    fn list_completion_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        sqlserver_completion_tables(
            self.config("SQL Server 补全元数据需要连接配置上下文")?,
            database,
            schema,
            filter,
            limit,
        )
    }

    fn list_completion_columns(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        sqlserver_completion_columns_for_table(
            self.config("SQL Server 补全元数据需要连接配置上下文")?,
            database,
            schema,
            table,
        )
    }

    fn list_completion_routines(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        sqlserver_completion_routines(
            self.config("SQL Server 补全元数据需要连接配置上下文")?,
            database,
            schema,
            filter,
            limit,
        )
    }

    fn table_ddl(&self, path: &ObjectPath) -> fluxdb_core::Result<String> {
        sqlserver_table_ddl_for(
            self.config("SQL Server DDL 元数据需要连接配置上下文")?,
            path,
        )
    }

    fn sqlserver_security_snapshot(
        &self,
        connection_id: ConnectionId,
        database: &str,
    ) -> fluxdb_core::Result<fluxdb_core::SqlServerSecuritySnapshot> {
        sqlserver_security_snapshot(
            self.config("SQL Server 用户与权限需要连接配置上下文")?,
            connection_id,
            database,
        )
    }

    fn sqlserver_principal_permissions(
        &self,
        connection_id: ConnectionId,
        database: &str,
        principal: &fluxdb_core::SqlServerPrincipalRef,
    ) -> fluxdb_core::Result<Vec<fluxdb_core::SqlServerPermission>> {
        sqlserver_principal_permissions(
            self.config("SQL Server 权限需要连接配置上下文")?,
            connection_id,
            database,
            principal,
        )
    }

    fn render_sqlserver_security_plan(
        &self,
        connection_id: ConnectionId,
        plan: &fluxdb_core::SqlServerSecurityPlan,
    ) -> fluxdb_core::Result<Vec<String>> {
        render_sqlserver_security_plan(
            self.config("SQL Server 用户与权限预览需要连接配置上下文")?,
            connection_id,
            plan,
        )
    }

    fn apply_sqlserver_security_plan(
        &self,
        connection_id: ConnectionId,
        plan: &fluxdb_core::SqlServerSecurityPlan,
    ) -> fluxdb_core::Result<Vec<String>> {
        apply_sqlserver_security_plan(
            self.config("SQL Server 用户与权限提交需要连接配置上下文")?,
            connection_id,
            plan,
        )
    }
    fn server_activity(&self, _: ConnectionId) -> fluxdb_core::Result<ServerActivity> {
        sqlserver_server_activity(
            self.config("SQL Server 服务器状态需要连接配置上下文")?,
        )
    }

    fn cancel_server_query(
        &self,
        _: ConnectionId,
        session_id: &str,
    ) -> fluxdb_core::Result<()> {
        sqlserver_kill(
            self.config("SQL Server 终止查询需要连接配置上下文")?,
            session_id,
            true,
        )
    }

    fn kill_server_session(
        &self,
        _: ConnectionId,
        session_id: &str,
    ) -> fluxdb_core::Result<()> {
        sqlserver_kill(
            self.config("SQL Server 终止会话需要连接配置上下文")?,
            session_id,
            false,
        )
    }

    fn run_server_maintenance(
        &self,
        request: &ServerMaintenanceRequest,
    ) -> fluxdb_core::Result<String> {
        sqlserver_server_maintenance(
            self.config("SQL Server 维护需要连接配置上下文")?,
            request,
        )
    }

}
