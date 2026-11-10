#[derive(Clone, Default)]
pub struct DamengConnector {
    config: Option<ConnectionConfig>,
}

impl DamengConnector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: ConnectionConfig) -> Self {
        Self {
            config: Some(config),
        }
    }

    fn config(&self, message: &str) -> fluxdb_core::Result<&ConnectionConfig> {
        self.config
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Connection, message))
    }
}

impl Connector for DamengConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Dameng
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        let mut client = dm_connect(config)?;
        dm_execute(&mut client, "SELECT 1 FROM DUAL", &[])?;
        client.close();
        Ok(())
    }

    fn create_schema(
        &self,
        connection_id: ConnectionId,
        _: &str,
        schema: &str,
    ) -> fluxdb_core::Result<()> {
        if connection_id != self.config("达梦新建 schema 需要连接配置上下文")?.id {
            return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
        }
        let schema = schema.trim();
        if schema.is_empty() {
            return Err(Error::new(ErrorKind::Query, "schema 名称不能为空"));
        }
        let mut client = dm_connect(self.config("达梦新建 schema 需要连接配置上下文")?)?;
        dm_execute(
            &mut client,
            &format!(
                "CREATE SCHEMA {}",
                fluxdb_core::DamengDialect.quote_identifier(schema)
            ),
            &[],
        )?;
        client.close();
        Ok(())
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        dm_list_objects(
            self.config("达梦对象浏览需要连接配置上下文")?,
            path,
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
        dm_load_data(
            self.config("达梦数据读取需要连接配置上下文")?,
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
        dm_load_data_expression(
            self.config("达梦数据读取需要连接配置上下文")?,
            path,
            offset,
            limit,
            sort,
            expression,
        )
    }

    fn apply_changes(&self, changes: &DataChangeSet) -> fluxdb_core::Result<AppliedChangeOutcome> {
        dm_apply_changes(
            self.config("达梦数据提交需要连接配置上下文")?,
            changes,
        )
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        dm_execute_query(
            self.config("达梦 SQL 执行需要连接配置上下文")?,
            request,
        )
    }

    fn execute_with_progress(
        &self,
        request: &QueryRequest,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        dm_execute_query_with_progress(
            self.config("达梦 SQL 执行需要连接配置上下文")?,
            request,
            on_summary,
            should_cancel,
        )
    }

    fn execute_parameterized(
        &self,
        request: &fluxdb_core::ParameterizedQueryRequest,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        dm_execute_parameterized(
            self.config("达梦 SQL 执行需要连接配置上下文")?,
            &request.request,
            &request.parameters,
        )
    }

    fn list_indexes(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
        dm_indexes_for(self.config("达梦索引元数据需要连接配置上下文")?, path)
    }

    fn list_foreign_keys(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        dm_foreign_keys_for(
            self.config("达梦外键元数据需要连接配置上下文")?,
            path,
        )
    }

    fn list_triggers(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<TriggerInfo>> {
        dm_triggers_for(self.config("达梦触发器元数据需要连接配置上下文")?, path)
    }

    fn load_table_design_metadata(
        &self,
        path: &ObjectPath,
    ) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
        dm_load_table_design_metadata(
            self.config("达梦设计表元数据需要连接配置上下文")?,
            path,
        )
    }

    fn load_table_design_base_metadata(
        &self,
        path: &ObjectPath,
    ) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
        dm_load_table_design_base_metadata(
            self.config("达梦设计表基础元数据需要连接配置上下文")?,
            path,
        )
    }

    fn list_completion_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        let config = self.config("达梦补全元数据需要连接配置上下文")?;
        let schema = schema
            .filter(|schema| !schema.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                config
                    .dameng_profile
                    .as_ref()
                    .map(|profile| profile.basic.schema.clone())
                    .filter(|schema| !schema.trim().is_empty())
            })
            .ok_or_else(|| Error::new(ErrorKind::Query, "缺少达梦 schema 上下文"))?;
        let mut client = dm_connect(config)?;
        let result = dm_execute(
            &mut client,
            "SELECT OBJECT_NAME, OBJECT_TYPE FROM ALL_OBJECTS \
             WHERE OWNER=? AND OBJECT_TYPE IN ('TABLE','VIEW') AND OBJECT_NAME LIKE ? \
             ORDER BY OBJECT_NAME",
            &[
                Param::Str(schema.clone()),
                Param::Str(format!("%{}%", filter.trim())),
            ],
        )?;
        client.close();
        let ExecuteResult::Query { rows, .. } = result else {
            return Err(Error::new(ErrorKind::Query, "达梦补全表列表返回类型异常"));
        };
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let name = row.first().and_then(dm_value_text)?;
                if dm_internal_object_name(&name) {
                    return None;
                }
                let object_type = row.get(1).and_then(dm_value_text).unwrap_or_default();
                Some(CompletionTable {
                    database: database.map(str::to_string),
                    schema: Some(schema.clone()),
                    kind: if object_type.eq_ignore_ascii_case("VIEW") {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                    name,
                    comment: None,
                })
            })
            .take(limit as usize)
            .collect())
    }

    fn list_completion_tables_all_schemas(
        &self,
        database: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        let config = self.config("达梦补全元数据需要连接配置上下文")?;
        let mut client = dm_connect(config)?;
        let result = dm_execute(
            &mut client,
            "SELECT OBJECT_NAME, OBJECT_TYPE, OWNER FROM ALL_OBJECTS \
             WHERE OBJECT_TYPE IN ('TABLE','VIEW') AND OBJECT_NAME LIKE ? \
             ORDER BY OWNER, OBJECT_NAME",
            &[Param::Str(format!("%{}%", filter.trim()))],
        )?;
        client.close();
        let ExecuteResult::Query { rows, .. } = result else {
            return Err(Error::new(ErrorKind::Query, "达梦补全表列表返回类型异常"));
        };
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let name = row.first().and_then(dm_value_text)?;
                if dm_internal_object_name(&name) {
                    return None;
                }
                let object_type = row.get(1).and_then(dm_value_text).unwrap_or_default();
                let schema = row.get(2).and_then(dm_value_text)?;
                Some(CompletionTable {
                    database: database.map(str::to_string),
                    schema: Some(schema),
                    kind: if object_type.eq_ignore_ascii_case("VIEW") {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                    name,
                    comment: None,
                })
            })
            .take(limit as usize)
            .collect())
    }

    fn list_completion_columns(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let config = self.config("达梦补全元数据需要连接配置上下文")?;
        let schema = schema
            .filter(|schema| !schema.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                config
                    .dameng_profile
                    .as_ref()
                    .map(|profile| profile.basic.schema.clone())
                    .filter(|schema| !schema.trim().is_empty())
            })
            .ok_or_else(|| Error::new(ErrorKind::Query, "缺少达梦 schema 上下文"))?;
        let mut client = dm_connect(config)?;
        let columns = dm_load_columns(&mut client, &schema, table)?;
        client.close();
        Ok(columns
            .into_iter()
            .map(|column| CompletionColumn {
                database: database.map(str::to_string),
                schema: Some(schema.clone()),
                table: table.to_string(),
                name: column.name,
                type_name: column.type_name,
                nullable: column.nullable,
                primary_key: column.primary_key,
                comment: column.comment,
                stable: None,
            })
            .collect())
    }

    fn table_ddl(&self, path: &ObjectPath) -> fluxdb_core::Result<String> {
        dm_table_ddl_for(self.config("达梦 DDL 元数据需要连接配置上下文")?, path)
    }

    fn oracle_user_admin_snapshot(
        &self,
        connection_id: ConnectionId,
    ) -> fluxdb_core::Result<OracleUserAdminSnapshot> {
        dm_user_admin_snapshot(
            self.config("达梦用户与权限需要连接配置上下文")?,
            connection_id,
        )
    }

    fn apply_oracle_user_admin_plan(
        &self,
        connection_id: ConnectionId,
        plan: &OracleUserAdminPlan,
    ) -> fluxdb_core::Result<Vec<String>> {
        dm_apply_user_admin_plan(
            self.config("达梦用户与权限变更需要连接配置上下文")?,
            connection_id,
            plan,
        )
    }

    fn server_activity(&self, _: ConnectionId) -> fluxdb_core::Result<ServerActivity> {
        dm_server_activity(
            self.config("达梦服务器状态需要连接配置上下文")?,
        )
    }

    fn cancel_server_query(
        &self,
        _: ConnectionId,
        session_id: &str,
    ) -> fluxdb_core::Result<()> {
        dm_kill(
            self.config("达梦终止查询需要连接配置上下文")?,
            session_id,
            true,
        )
    }

    fn kill_server_session(
        &self,
        _: ConnectionId,
        session_id: &str,
    ) -> fluxdb_core::Result<()> {
        dm_kill(
            self.config("达梦终止会话需要连接配置上下文")?,
            session_id,
            false,
        )
    }

}
