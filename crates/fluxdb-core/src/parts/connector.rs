#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateDatabaseRequest {
    pub connection_id: ConnectionId,
    pub name: String,
    /// MySQL 字符集；PostgreSQL 作 ENCODING。
    pub charset: String,
    /// MySQL 排序规则；PostgreSQL 作 LC_COLLATE/LC_CTYPE。
    pub collation: String,
    /// PostgreSQL OWNER（可选，空则不指定）。
    pub owner: String,
    /// PostgreSQL TEMPLATE（可选，空则不指定）。
    pub template: String,
    pub path: Option<PathBuf>,
}

pub trait Connector {
    fn kind(&self) -> DatabaseKind;
    fn test_connection(&self, config: &ConnectionConfig) -> Result<()>;
    fn list_objects(&self, path: Option<&ObjectPath>) -> Result<Vec<ObjectSummary>>;
    fn create_database(&self, _: &CreateDatabaseRequest) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持新建数据库"))
    }
    fn create_schema(&self, _: ConnectionId, _: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持新建 schema"))
    }
    fn list_roles(&self, _: ConnectionId) -> Result<Vec<PgRole>> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持角色管理"))
    }
    fn create_role(
        &self,
        _: ConnectionId,
        _: &str,
        _: bool,
        _: Option<&str>,
    ) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持创建角色"))
    }
    fn alter_role_password(&self, _: ConnectionId, _: &str, _: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持修改角色密码"))
    }
    fn rename_role(&self, _: ConnectionId, _: &str, _: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持重命名角色"))
    }
    fn drop_role(&self, _: ConnectionId, _: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持删除角色"))
    }
    fn alter_role_options(
        &self,
        _: ConnectionId,
        _: &str,
        _: Option<bool>,
        _: Option<bool>,
        _: Option<bool>,
        _: Option<bool>,
        _: Option<bool>,
        _: Option<bool>,
        _: Option<bool>,
        _: Option<i32>,
        _: Option<&str>,
    ) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持修改角色属性"))
    }
    fn grant_role_membership(&self, _: ConnectionId, _: &str, _: &str, _: bool) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持成员授权"))
    }
    fn revoke_role_membership(&self, _: ConnectionId, _: &str, _: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持撤销成员关系"))
    }
    fn grant_object_privilege(&self, _: ConnectionId, _: &str, _: &str, _: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持对象授权"))
    }
    fn revoke_object_privilege(&self, _: ConnectionId, _: &str, _: &str, _: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持对象撤销"))
    }
    /// 列成员关系：(grantee, member, admin_option)。
    fn list_role_membership(&self, _: ConnectionId) -> Result<Vec<(String, String, bool)>> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持读取成员关系"))
    }
    /// 列对象权限：(grantee, privilege, grant_option)；grantee 空表示 PUBLIC。
    fn list_relation_grants(
        &self,
        _: ConnectionId,
        _: &str,
        _: &str,
    ) -> Result<Vec<(String, String, bool)>> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持读取对象权限"))
    }
    fn delete_database(&self, _: ConnectionId, _: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持删除数据库"))
    }

    fn load_data(
        &self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> Result<DataPage>;
    fn preview_data_export(
        &self,
        _: &ObjectPath,
        _: &[String],
        _: &[SortSpec],
        _: &[FilterSpec],
    ) -> Result<DataExportPreview> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持导出预览"))
    }
    fn apply_changes(&self, changes: &DataChangeSet) -> Result<AppliedChangeOutcome>;
    fn execute(&self, request: &QueryRequest) -> Result<QueryExecutionResult>;
    fn execute_command_workbench(
        &self,
        request: &CommandWorkbenchRequest,
    ) -> Result<CommandWorkbenchExecution> {
        let _ = request;
        Err(Error::new(
            ErrorKind::Unsupported,
            "该连接暂不支持命令执行器",
        ))
    }
    fn execute_with_progress(
        &self,
        request: &QueryRequest,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        _should_cancel: &dyn Fn() -> bool,
    ) -> Result<QueryExecutionResult> {
        let execution = self.execute(request)?;
        for summary in execution.summaries.iter().cloned() {
            on_summary(summary);
        }
        Ok(execution)
    }
    fn list_completion_tables(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
        _: u64,
    ) -> Result<Vec<CompletionTable>> {
        Ok(Vec::new())
    }

    fn list_completion_tables_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<CompletionTable>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let result = self.list_completion_tables(database, schema, filter, limit)?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        Ok(result)
    }

    fn list_completion_columns(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
    ) -> Result<Vec<CompletionColumn>> {
        Ok(Vec::new())
    }

    fn list_completion_columns_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<CompletionColumn>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let result = self.list_completion_columns(database, schema, table)?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        Ok(result)
    }

    fn list_completion_columns_for_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
    ) -> Result<Vec<CompletionColumn>> {
        let mut columns = Vec::new();
        for table in tables {
            columns.extend(self.list_completion_columns(database, schema, table)?);
        }
        Ok(columns)
    }

    fn list_completion_columns_for_tables_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<CompletionColumn>> {
        let mut columns = Vec::new();
        for table in tables {
            if should_cancel() {
                return Ok(Vec::new());
            }
            columns.extend(self.list_completion_columns_with_cancel(
                database,
                schema,
                table,
                should_cancel,
            )?);
        }
        if should_cancel() {
            return Ok(Vec::new());
        }
        Ok(columns)
    }

    fn list_completion_routines(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
        _: u64,
    ) -> Result<Vec<CompletionRoutine>> {
        Ok(Vec::new())
    }

    fn list_completion_routines_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<CompletionRoutine>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let result = self.list_completion_routines(database, schema, filter, limit)?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        Ok(result)
    }

    fn list_completion_triggers(
        &self,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
        _: u64,
    ) -> Result<Vec<CompletionTrigger>> {
        Ok(Vec::new())
    }

    fn list_completion_triggers_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<CompletionTrigger>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let result = self.list_completion_triggers(database, schema, filter, limit)?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        Ok(result)
    }

    fn load_cell_binary(&self, _: &ObjectPath, _: &RowIdentity, _: &str) -> Result<Vec<u8>> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "暂不支持二进制单元格读取",
        ))
    }

    fn list_indexes(&self, _: &ObjectPath) -> Result<Vec<IndexInfo>> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持索引元数据"))
    }

    fn list_foreign_keys(&self, _: &ObjectPath) -> Result<Vec<ForeignKeyInfo>> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持外键元数据"))
    }

    fn list_foreign_keys_with_cancel(
        &self,
        path: &ObjectPath,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<ForeignKeyInfo>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let result = self.list_foreign_keys(path)?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        Ok(result)
    }

    fn list_triggers(&self, _: &ObjectPath) -> Result<Vec<TriggerInfo>> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持触发器元数据"))
    }

    fn table_ddl(&self, _: &ObjectPath) -> Result<String> {
        Err(Error::new(ErrorKind::Unsupported, "暂不支持 DDL 元数据"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataExportPreview {
    pub sql: String,
    pub row_count: u64,
}
