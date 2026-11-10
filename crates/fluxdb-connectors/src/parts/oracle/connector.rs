#[derive(Clone, Default)]
pub struct OracleConnector {
    config: Option<ConnectionConfig>,
}

impl OracleConnector {
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

impl Connector for OracleConnector {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Oracle

    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        let connection = oracle_connect(config)?;
        let rows = oracle_query(&connection, "SELECT 1 FROM DUAL", &[])?;
        if rows.is_empty() {
            return Err(Error::new(ErrorKind::Query, "Oracle 连接探测没有返回结果"));
        }
        Ok(())
    }

    fn create_schema(
        &self,
        connection_id: ConnectionId,
        _: &str,
        schema: &str,
    ) -> fluxdb_core::Result<()> {
        if connection_id != self.config("Oracle 新建 schema 需要连接配置上下文")?.id {
            return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
        }
        let schema = schema.trim();
        if schema.is_empty() {
            return Err(Error::new(ErrorKind::Query, "schema 名称不能为空"));
        }
        // Oracle 的 schema 就是账号：单独 CREATE SCHEMA 只能建空壳，无法承载对象。
        // 这里建账号（需 CREATE USER 权限），并显式告知默认表空间由服务端决定。
        let connection = oracle_connect(self.config("Oracle 新建 schema 需要连接配置上下文")?)?;
        let password = oracle_random_password();
        let sql = format!(
            "CREATE USER {user} IDENTIFIED BY \"{password}\"",
            user = fluxdb_core::OracleDialect.quote_identifier(schema),
        );
        oracle_execute(&connection, &sql, &[]).map_err(|error| {
            Error::new(
                error.kind,
                format!("新建 Oracle 账号失败（需要 CREATE USER 权限）：{}", error.message),
            )
        })?;
        Ok(())
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        oracle_list_objects(self.config("Oracle 对象浏览需要连接配置上下文")?, path)
    }

    fn load_data(
        &self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        oracle_load_data(
            self.config("Oracle 数据读取需要连接配置上下文")?,
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
        oracle_load_data_expression(
            self.config("Oracle 数据读取需要连接配置上下文")?,
            path,
            offset,
            limit,
            sort,
            expression,
        )
    }

    fn load_cell_text(
        &self,
        path: &ObjectPath,
        identity: &fluxdb_core::RowIdentity,
        column: &str,
    ) -> fluxdb_core::Result<String> {
        oracle_load_cell_text(
            self.config("Oracle 文本读取需要连接配置上下文")?,
            path,
            identity,
            column,
        )
    }

    fn preview_data_export(
        &self,
        path: &ObjectPath,
        fields: &[String],
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataExportPreview> {
        oracle_preview_data_export(
            self.config("Oracle 导出预览需要连接配置上下文")?,
            path,
            fields,
            sort,
            filters,
        )
    }

    fn apply_changes(&self, changes: &DataChangeSet) -> fluxdb_core::Result<AppliedChangeOutcome> {
        oracle_apply_changes(
            self.config("Oracle 数据提交需要连接配置上下文")?,
            changes,
        )
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        oracle_execute_query(
            self.config("Oracle SQL 执行需要连接配置上下文")?,
            request,
        )
    }

    fn execute_with_progress(
        &self,
        request: &QueryRequest,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        oracle_execute_query_with_progress(
            self.config("Oracle SQL 执行需要连接配置上下文")?,
            request,
            on_summary,
            should_cancel,
        )
    }

    fn execute_parameterized(
        &self,
        request: &fluxdb_core::ParameterizedQueryRequest,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        oracle_execute_parameterized(
            self.config("Oracle SQL 执行需要连接配置上下文")?,
            &request.request,
            &request.parameters,
        )
    }

    fn list_indexes(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
        oracle_indexes_for(self.config("Oracle 索引元数据需要连接配置上下文")?, path)
    }

    fn list_foreign_keys(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        oracle_foreign_keys_for(
            self.config("Oracle 外键元数据需要连接配置上下文")?,
            path,
        )
    }

    /// 一次登录读完整个 Owner 的外键，替代逐表 N+1（ER 整库关系拓扑会读遍全部表）。
    /// `tables` 为空表示该 Owner 下的全部表；schema 缺省时回退连接档案的默认 owner。
    fn list_foreign_keys_for_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
    ) -> fluxdb_core::Result<Vec<(String, ForeignKeyInfo)>> {
        let config = self.config("Oracle 外键元数据需要连接配置上下文")?;
        let owner = oracle_request_schema(
            config,
            &ObjectPath {
                connection_id: config.id,
                database: database.map(str::to_string),
                schema: schema.map(str::to_string),
                name: String::new(),
                kind: ObjectKind::Schema,
            },
        )?;
        let connection = oracle_connect_with_schema(config, Some(&owner))?;
        if tables.is_empty() {
            // 空列表在 trait 里表示「全部表」，先取该 Owner 的表名再批量查外键。
            let tables = oracle_load_table_names(&connection, &owner)?;
            return oracle_load_foreign_keys_for_tables(&connection, &owner, &tables, &|| false);
        }
        oracle_load_foreign_keys_for_tables(&connection, &owner, tables, &|| false)
    }

    fn list_foreign_keys_with_cancel(
        &self,
        path: &ObjectPath,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let config = self.config("Oracle 外键元数据需要连接配置上下文")?;
        let owner = oracle_request_schema(config, path)?;
        // 单表外键走批量实现，多一条 `IN` 谓词但不增加往返，取消行为与批量路径一致。
        let fks = oracle_connect_with_schema(config, Some(&owner)).and_then(|connection| {
            oracle_load_foreign_keys_for_tables(
                &connection,
                &owner,
                std::slice::from_ref(&path.name),
                should_cancel,
            )
        })?;
        Ok(fks.into_iter().map(|(_, fk)| fk).collect())
    }

    fn list_triggers(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<TriggerInfo>> {
        oracle_triggers_for(self.config("Oracle 触发器元数据需要连接配置上下文")?, path)
    }

    fn load_table_design_metadata(
        &self,
        path: &ObjectPath,
    ) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
        oracle_load_table_design_metadata(
            self.config("Oracle 设计表元数据需要连接配置上下文")?,
            path,
        )
    }

    fn load_table_design_base_metadata(
        &self,
        path: &ObjectPath,
    ) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
        oracle_load_table_design_base_metadata(
            self.config("Oracle 设计表基础元数据需要连接配置上下文")?,
            path,
        )
    }

    fn table_ddl(&self, path: &ObjectPath) -> fluxdb_core::Result<String> {
        oracle_table_ddl_for(self.config("Oracle DDL 元数据需要连接配置上下文")?, path)
    }

    fn oracle_user_admin_snapshot(
        &self,
        connection_id: ConnectionId,
    ) -> fluxdb_core::Result<OracleUserAdminSnapshot> {
        oracle_load_user_admin_snapshot(
            self.config("Oracle 用户与权限需要连接配置上下文")?,
            connection_id,
        )
    }

    fn apply_oracle_user_admin_plan(
        &self,
        connection_id: ConnectionId,
        plan: &OracleUserAdminPlan,
    ) -> fluxdb_core::Result<Vec<String>> {
        oracle_apply_user_admin_plan(
            self.config("Oracle 用户与权限变更需要连接配置上下文")?,
            connection_id,
            plan,
        )
    }

    fn list_completion_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        let config = self.config("Oracle 补全元数据需要连接配置上下文")?;
        let schema = oracle_request_schema(
            config,
            &ObjectPath {
                connection_id: config.id,
                database: database.map(str::to_string),
                schema: schema.map(str::to_string),
                name: String::new(),
                kind: ObjectKind::Schema,
            },
        )?;
        let connection = oracle_connect(config)?;
        Ok(oracle_completion_tables(
            &connection,
            Some(&schema),
            filter,
            limit,
            database,
        )?)
    }

    /// 补全可取消：建连与目录查询之间各检查一次，取消后返回空结果。
    /// 驱动是同步阻塞实现，没有中断 API，已发出的单条查询仍然跑完
    /// （与执行器"只在安全点取消"的语义一致）。
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
        let config = self.config("Oracle 补全元数据需要连接配置上下文")?;
        let schema = oracle_request_schema(
            config,
            &ObjectPath {
                connection_id: config.id,
                database: database.map(str::to_string),
                schema: schema.map(str::to_string),
                name: String::new(),
                kind: ObjectKind::Schema,
            },
        )?;
        let connection = oracle_connect(config)?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        let tables = oracle_completion_tables(&connection, Some(&schema), filter, limit, database)?;
        Ok(if should_cancel() { Vec::new() } else { tables })
    }

    fn list_completion_tables_all_schemas(
        &self,
        database: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        let connection = oracle_connect(self.config("Oracle 补全元数据需要连接配置上下文")?)?;
        oracle_completion_tables(&connection, None, filter, limit, database)
    }

    fn list_completion_tables_all_schemas_with_cancel(
        &self,
        database: Option<&str>,
        filter: &str,
        limit: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        if should_cancel() {
            return Ok(Vec::new());
        }
        let connection = oracle_connect(self.config("Oracle 补全元数据需要连接配置上下文")?)?;
        if should_cancel() {
            return Ok(Vec::new());
        }
        let tables = oracle_completion_tables(&connection, None, filter, limit, database)?;
        Ok(if should_cancel() { Vec::new() } else { tables })
    }

    fn list_completion_columns(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let config = self.config("Oracle 补全元数据需要连接配置上下文")?;
        let schema = oracle_request_schema(
            config,
            &ObjectPath {
                connection_id: config.id,
                database: database.map(str::to_string),
                schema: schema.map(str::to_string),
                name: table.to_string(),
                kind: ObjectKind::Table,
            },
        )?;
        let connection = oracle_connect(config)?;
        let columns = oracle_load_column_infos(&connection, &schema, table)?;
        Ok(columns
            .into_iter()
            .map(|column| CompletionColumn {
                database: database.map(str::to_string),
                schema: Some(schema.clone()),
                table: table.to_string(),
                name: column.name,
                type_name: Some(column.type_name),
                nullable: column.nullable,
                primary_key: column.primary_key,
                comment: column.comment,
                stable: None,
            })
            .collect())
    }

    /// 一次登录读完一批表的列（ER 字段面板按 schema 整批请求）。
    fn list_completion_columns_for_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        self.list_completion_columns_for_tables_with_cancel(
            database,
            schema,
            tables,
            &|| false,
        )
    }

    fn list_completion_columns_for_tables_with_cancel(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        if should_cancel() || tables.is_empty() {
            return Ok(Vec::new());
        }
        let config = self.config("Oracle 补全元数据需要连接配置上下文")?;
        let owner = oracle_request_schema(
            config,
            &ObjectPath {
                connection_id: config.id,
                database: database.map(str::to_string),
                schema: schema.map(str::to_string),
                name: String::new(),
                kind: ObjectKind::Schema,
            },
        )?;
        let connection = oracle_connect(config)?;
        oracle_load_completion_columns_for_tables(
            &connection,
            &owner,
            tables,
            database,
            should_cancel,
        )
    }

    fn list_completion_routines(
        &self,
        _database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        let config = self.config("Oracle 补全元数据需要连接配置上下文")?;
        let owner = schema
            .map(str::trim)
            .filter(|schema| !schema.is_empty())
            .map(str::to_string)
            .or_else(|| {
                config
                    .oracle_profile
                    .as_ref()
                    .map(|profile| profile.basic.schema.clone())
                    .filter(|schema| !schema.trim().is_empty())
            });
        let connection = oracle_connect(config)?;
        // 只取可见对象；包内过程需要再展开一级，这里先给出一级候选。
        let rows = oracle_query(
            &connection,
            "SELECT OWNER, OBJECT_NAME, OBJECT_TYPE FROM ALL_PROCEDURES \
             WHERE OBJECT_NAME IS NOT NULL \
               AND (:1 IS NULL OR OWNER = :2) \
               AND UPPER(OBJECT_NAME) LIKE UPPER(:3) \
             ORDER BY OWNER, OBJECT_NAME",
            &[
                owner.clone().map_or(OracleBindValue::Null, OracleBindValue::Text),
                owner.map_or(OracleBindValue::Null, OracleBindValue::Text),
                OracleBindValue::Text(format!("%{}%", filter.trim())),
            ],
        )?;
        let mut seen = std::collections::BTreeSet::new();
        let mut routines = Vec::new();
        for row in &rows {
            let owner = oracle_meta_text(row, 0).unwrap_or_default();
            let Some(name) = oracle_meta_text(row, 1) else {
                continue;
            };
            if !seen.insert((owner.clone(), name.clone())) {
                continue;
            }
            routines.push(CompletionRoutine {
                schema: Some(owner),
                name,
                kind: CompletionRoutineKind::Procedure,
                signature: None,
            });
            if routines.len() >= limit as usize {
                break;
            }
        }
        Ok(routines)
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
        let routines = self.list_completion_routines(database, schema, filter, limit)?;
        Ok(if should_cancel() { Vec::new() } else { routines })
    }

    fn list_completion_triggers(
        &self,
        _database: Option<&str>,
        schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
        let config = self.config("Oracle 补全元数据需要连接配置上下文")?;
        let owner = schema
            .map(str::trim)
            .filter(|schema| !schema.is_empty())
            .map(str::to_string)
            .or_else(|| {
                config
                    .oracle_profile
                    .as_ref()
                    .map(|profile| profile.basic.schema.clone())
                    .filter(|schema| !schema.trim().is_empty())
            });
        let connection = oracle_connect(config)?;
        let rows = oracle_query(
            &connection,
            "SELECT OWNER, TRIGGER_NAME, TABLE_NAME FROM ALL_TRIGGERS \
             WHERE (:1 IS NULL OR OWNER = :2) AND UPPER(TRIGGER_NAME) LIKE UPPER(:3) \
             ORDER BY OWNER, TRIGGER_NAME",
            &[
                owner.clone().map_or(OracleBindValue::Null, OracleBindValue::Text),
                owner.map_or(OracleBindValue::Null, OracleBindValue::Text),
                OracleBindValue::Text(format!("%{}%", filter.trim())),
            ],
        )?;
        Ok(rows
            .iter()
            .take(limit as usize)
            .filter_map(|row| {
                Some(CompletionTrigger {
                    schema: oracle_meta_text(row, 0),
                    name: oracle_meta_text(row, 1)?,
                    table: oracle_meta_text(row, 2),
                })
            })
            .collect())
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
        let triggers = self.list_completion_triggers(database, schema, filter, limit)?;
        Ok(if should_cancel() { Vec::new() } else { triggers })
    }
    fn server_activity(&self, _: ConnectionId) -> fluxdb_core::Result<ServerActivity> {
        oracle_server_activity(
            self.config("Oracle 服务器状态需要连接配置上下文")?,
        )
    }

    fn cancel_server_query(
        &self,
        _: ConnectionId,
        session_id: &str,
    ) -> fluxdb_core::Result<()> {
        oracle_kill(
            self.config("Oracle 终止查询需要连接配置上下文")?,
            session_id,
            true,
        )
    }

    fn kill_server_session(
        &self,
        _: ConnectionId,
        session_id: &str,
    ) -> fluxdb_core::Result<()> {
        oracle_kill(
            self.config("Oracle 终止会话需要连接配置上下文")?,
            session_id,
            false,
        )
    }

    fn run_server_maintenance(
        &self,
        request: &ServerMaintenanceRequest,
    ) -> fluxdb_core::Result<String> {
        oracle_server_maintenance(
            self.config("Oracle 维护需要连接配置上下文")?,
            request,
        )
    }

}

fn oracle_completion_tables(
    connection: &OracleConnection,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
    database: Option<&str>,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    let rows = oracle_query(
        connection,
        "SELECT OWNER, OBJECT_NAME, OBJECT_TYPE FROM ALL_OBJECTS \
         WHERE OBJECT_TYPE IN ('TABLE','VIEW') \
           AND (:1 IS NULL OR OWNER = :2) \
           AND UPPER(OBJECT_NAME) LIKE UPPER(:3) \
         ORDER BY OWNER, OBJECT_NAME",
        &[
            schema.map_or(OracleBindValue::Null, |value| {
                OracleBindValue::Text(value.to_string())
            }),
            schema.map_or(OracleBindValue::Null, |value| {
                OracleBindValue::Text(value.to_string())
            }),
            OracleBindValue::Text(format!("%{}%", filter.trim())),
        ],
    )?;
    Ok(rows
        .iter()
        .take(limit as usize)
        .filter_map(|row| {
            let owner = oracle_meta_text(row, 0)?;
            let name = oracle_meta_text(row, 1)?;
            let object_type = oracle_meta_text(row, 2).unwrap_or_default();
            Some(CompletionTable {
                database: database.map(str::to_string),
                schema: Some(owner),
                kind: if object_type.eq_ignore_ascii_case("VIEW") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
                name,
                comment: None,
            })
        })
        .collect())
}

/// 新建 Oracle 账号时的初始口令。只用于本次 DDL，不返回、不落盘、不记录。
fn oracle_random_password() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    // 口令必须满足默认校验（大小写+数字），随机部分来自时间与进程号。
    format!("Fx{:x}{:x}", nanos, std::process::id())}
