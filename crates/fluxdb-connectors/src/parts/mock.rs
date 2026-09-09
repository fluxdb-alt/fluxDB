#[derive(Clone, Debug)]
pub struct MockConnector {
    kind: DatabaseKind,
}

impl MockConnector {
    pub fn new(kind: DatabaseKind) -> Self {
        Self { kind }
    }

    pub fn sqlite() -> Self {
        Self::new(DatabaseKind::Sqlite)
    }
}

impl Connector for MockConnector {
    fn kind(&self) -> DatabaseKind {
        self.kind
    }

    fn test_connection(&self, config: &ConnectionConfig) -> fluxdb_core::Result<()> {
        if config.kind == self.kind {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::Connection, "连接类型不匹配"))
        }
    }

    fn list_objects(&self, path: Option<&ObjectPath>) -> fluxdb_core::Result<Vec<ObjectSummary>> {
        let connection_id = path
            .map(|path| path.connection_id)
            .unwrap_or(ConnectionId(1));
        if path.is_none() {
            return Ok(vec![database_object(connection_id, "main")]);
        }

        Ok(mock_objects(connection_id))
    }

    fn load_data(
        &self,
        path: &ObjectPath,
        offset: u64,
        limit: u64,
        _sort: &[SortSpec],
        _filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataPage> {
        if path.kind != ObjectKind::Table {
            return Err(Error::new(ErrorKind::Unsupported, "仅表对象支持数据读取"));
        }

        Ok(mock_data_page(offset, limit))
    }

    fn preview_data_export(
        &self,
        path: &ObjectPath,
        fields: &[String],
        sort: &[SortSpec],
        filters: &[FilterSpec],
    ) -> fluxdb_core::Result<DataExportPreview> {
        let fields = if fields.is_empty() {
            "*".to_string()
        } else {
            fields.join(", ")
        };
        let mut sql = format!("SELECT {fields} FROM {}", path.name);
        if filters.iter().any(|filter| filter.enabled) {
            sql.push_str(" WHERE ...");
        }
        if !sort.is_empty() {
            sql.push_str(" ORDER BY ...");
        }
        Ok(DataExportPreview {
            sql,
            row_count: 100,
        })
    }

    fn apply_changes(&self, changes: &DataChangeSet) -> fluxdb_core::Result<()> {
        if changes.object.name == "FailSubmit" {
            return Err(Error::new(ErrorKind::Query, "模拟提交失败"));
        }

        if changes.is_empty() {
            return Err(Error::new(ErrorKind::Query, "没有需要提交的更改"));
        }

        Ok(())
    }

    fn execute(&self, request: &QueryRequest) -> fluxdb_core::Result<QueryExecutionResult> {
        self.execute_with_progress(request, &mut |_| {}, &|| false)
    }

    fn execute_with_progress(
        &self,
        request: &QueryRequest,
        on_summary: &mut dyn FnMut(QueryExecutionSummary),
        should_cancel: &dyn Fn() -> bool,
    ) -> fluxdb_core::Result<QueryExecutionResult> {
        let statements = query_statements_for_execution(request);
        if statements.is_empty() {
            return Err(Error::new(ErrorKind::Query, "查询不能为空"));
        }
        let mut summaries = Vec::new();
        let mut results = Vec::new();
        for statement in statements {
            if should_cancel() {
                break;
            }
            if statement.to_ascii_lowercase().contains("error") {
                let summary = QueryExecutionSummary {
                    sql: statement,
                    kind: QueryStatementKind::ResultSet,
                    success: false,
                    message: "模拟查询错误".to_string(),
                    returned_rows: 0,
                    affected_rows: 0,
                    elapsed_ms: 0,
                };
                on_summary(summary.clone());
                summaries.push(summary);
                if !request.options.continue_on_error {
                    break;
                }
                continue;
            }
            let page = mock_data_page(request.options.page_offset, request.options.page_size);
            let summary = QueryExecutionSummary {
                sql: statement,
                kind: QueryStatementKind::ResultSet,
                success: true,
                message: format!("返回 {} 行结果表", page.rows.len()),
                returned_rows: page.rows.len() as u64,
                affected_rows: 0,
                elapsed_ms: 0,
            };
            on_summary(summary.clone());
            summaries.push(summary);
            results.push(page);
        }
        Ok(QueryExecutionResult {
            summaries,
            results,
            rollback_snapshots: Vec::new(),
        })
    }

    fn list_completion_tables(
        &self,
        database: Option<&str>,
        _schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTable>> {
        Ok(mock_objects(ConnectionId(1))
            .into_iter()
            .filter(|object| matches!(object.path.kind, ObjectKind::Table | ObjectKind::View))
            .filter(|object| matches_completion_fuzzy_filter(&object.path.name, filter))
            .take(limit as usize)
            .map(|object| CompletionTable {
                database: database.map(str::to_string).or(object.path.database),
                schema: object.path.schema,
                name: object.path.name,
                kind: object.path.kind,
            })
            .collect())
    }

    fn list_completion_columns(
        &self,
        _database: Option<&str>,
        _schema: Option<&str>,
        table: &str,
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        Ok(mock_completion_columns(table)
            .into_iter()
            .map(|column| CompletionColumn {
                table: table.to_string(),
                name: column.name,
                type_name: column.type_name,
                nullable: column.nullable,
                primary_key: column.primary_key,
                comment: column.comment,
            })
            .collect())
    }

    fn list_completion_columns_for_tables(
        &self,
        database: Option<&str>,
        schema: Option<&str>,
        tables: &[String],
    ) -> fluxdb_core::Result<Vec<CompletionColumn>> {
        let mut columns = Vec::new();
        for table in tables {
            columns.extend(self.list_completion_columns(database, schema, table)?);
        }
        Ok(columns)
    }

    fn list_completion_routines(
        &self,
        _database: Option<&str>,
        _schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionRoutine>> {
        Ok([
            CompletionRoutine {
                schema: None,
                name: "refresh_product".to_string(),
                kind: CompletionRoutineKind::Procedure,
            },
            CompletionRoutine {
                schema: None,
                name: "normalize_price".to_string(),
                kind: CompletionRoutineKind::Function,
            },
        ]
        .into_iter()
        .filter(|routine| matches_completion_filter(&routine.name, filter))
        .take(limit as usize)
        .collect())
    }

    fn list_completion_triggers(
        &self,
        _database: Option<&str>,
        _schema: Option<&str>,
        filter: &str,
        limit: u64,
    ) -> fluxdb_core::Result<Vec<CompletionTrigger>> {
        Ok([CompletionTrigger {
            schema: None,
            name: "product_ai".to_string(),
            table: Some("Product".to_string()),
        }]
        .into_iter()
        .filter(|trigger| matches_completion_filter(&trigger.name, filter))
        .take(limit as usize)
        .collect())
    }

    fn list_indexes(&self, _: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
        Ok(vec![IndexInfo {
            name: "PRIMARY".to_string(),
            columns: vec!["id".to_string()],
            is_unique: true,
            is_primary: true,
            index_type: Some("BTREE".to_string()),
            comment: None,
        }])
    }

    fn list_foreign_keys(&self, path: &ObjectPath) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
        Ok(mock_completion_foreign_keys(&path.name))
    }

    fn list_triggers(&self, _: &ObjectPath) -> fluxdb_core::Result<Vec<TriggerInfo>> {
        Ok(vec![TriggerInfo {
            name: "product_bu".to_string(),
            event: "UPDATE".to_string(),
            timing: "BEFORE".to_string(),
            body: Some("BEGIN\n  SELECT NEW.id;\nEND".to_string()),
        }])
    }

    fn table_ddl(&self, path: &ObjectPath) -> fluxdb_core::Result<String> {
        Ok(format!(
            "CREATE TABLE {} (\n  id INTEGER PRIMARY KEY,\n  name TEXT NOT NULL\n);",
            sqlite_quote_identifier(&path.name)
        ))
    }
}
