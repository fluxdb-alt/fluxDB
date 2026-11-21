impl TableDataImport for SqlServerTableImport {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::SqlServer
    }

    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> fluxdb_core::Result<TableImportReport> {
        import_sqlserver(request, cancel, progress)
    }

    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> fluxdb_core::Result<TableImportPreview> {
        preview_relational(DatabaseKind::SqlServer, request)
    }
}

/// SQL Server 导入与通用导入共用文件解析和列映射，但写入路径必须由
/// tiberius 的 `@P1..@Pn` 绑定；identity 列需要同一会话内开/关 IDENTITY_INSERT。
fn import_sqlserver(
    request: &TableImportRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(TableImportProgress),
) -> fluxdb_core::Result<TableImportReport> {
    if request.config.kind != DatabaseKind::SqlServer {
        return Err(Error::new(ErrorKind::Internal, "导入执行器与连接类型不匹配"));
    }
    if request.object.kind != ObjectKind::Table {
        return Err(Error::new(ErrorKind::Unsupported, "仅表对象支持数据导入"));
    }
    check_cancel(cancel)?;
    progress(TableImportProgress {
        stage: "读取".into(),
        message: format!("正在读取 {} 文件", request.format.label()),
    });
    let parsed = parse_import_file(request, cancel, progress)?;
    if parsed.rows.is_empty() {
        return Ok(TableImportReport {
            total_rows: 0,
            imported_rows: 0,
            warnings: Vec::new(),
        });
    }
    let connector = connector_for(&request.config)?;
    let target_columns = connector.list_completion_columns(
        request.object.database.as_deref(),
        request.object.schema.as_deref(),
        &request.object.name,
    )?;
    let mapping = map_import_columns(&parsed.columns, &target_columns, &request.column_mapping)?;
    let mut selected = Vec::new();
    let mut identity_columns = BTreeSet::new();
    let mut warnings = Vec::new();
    let ddl = connector.table_ddl(&request.object).ok();
    let (excluded, identities) = ddl_import_column_flags(DatabaseKind::SqlServer, ddl.as_deref());
    for (import_index, target_index) in mapping {
        let Some(target_index) = target_index else {
            continue;
        };
        let column = &target_columns[target_index];
        let upper = column.name.to_ascii_uppercase();
        if excluded.contains(&upper) {
            warnings.push(format!("字段 {} 由数据库生成，已跳过", column.name));
            continue;
        }
        if identities.contains(&upper) {
            identity_columns.insert(column.name.clone());
        }
        selected.push((import_index, target_index));
    }
    if selected.is_empty() {
        return Err(Error::new(ErrorKind::Query, "导入文件没有可写入字段"));
    }
    let table = import_qualified_table(DatabaseKind::SqlServer, request)?;
    let column_list = selected
        .iter()
        .map(|(_, target_index)| {
            fluxdb_core::SqlServerDialect.quote_identifier(&target_columns[*target_index].name)
        })
        .collect::<Vec<_>>()
        .join(", ");
    if request.options.mode == TableImportMode::Truncate {
        progress(TableImportProgress {
            stage: "清空".into(),
            message: format!("正在清空 {table}"),
        });
        check_cancel(cancel)?;
        let result = connector.execute(&QueryRequest {
            connection_id: request.config.id,
            database: request.object.database.clone(),
            schema: request.object.schema.clone(),
            session_id: None,
            text: format!("TRUNCATE TABLE {table}"),
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })?;
        if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
            return Err(Error::new(
                ErrorKind::Query,
                format!(
                    "SQL Server 清空失败（TRUNCATE 遇到外键引用时会拒绝）：{}",
                    summary.message
                ),
            ));
        }
    }
    let total_rows = parsed.rows.len();
    let total_batches = total_rows.div_ceil(TABLE_IMPORT_BATCH_ROWS);
    let mut imported_rows = 0u64;
    let identity_insert = !identity_columns.is_empty();
    for (batch_index, chunk) in parsed.rows.chunks(TABLE_IMPORT_BATCH_ROWS).enumerate() {
        check_cancel(cancel)?;
        let written = (batch_index * TABLE_IMPORT_BATCH_ROWS + chunk.len()).min(total_rows);
        progress(TableImportProgress {
            stage: "写入".into(),
            message: format!(
                "第 {}/{} 批，正在写入第 {}-{} 行（共 {} 行）",
                batch_index + 1,
                total_batches,
                written.saturating_sub(chunk.len()) + 1,
                written,
                total_rows,
            ),
        });
        let (sql, parameters) = build_import_batch_sql(
            DatabaseKind::SqlServer,
            &table,
            &column_list,
            "",
            &selected,
            &target_columns,
            chunk,
            InsertConflictMode::Error,
        )?;
        let sql = if identity_insert {
            format!("SET IDENTITY_INSERT {table} ON; {sql}; SET IDENTITY_INSERT {table} OFF")
        } else {
            sql
        };
        let result = connector.execute_parameterized(&ParameterizedQueryRequest {
            request: QueryRequest {
                connection_id: request.config.id,
                database: request.object.database.clone(),
                schema: request.object.schema.clone(),
                session_id: None,
                text: sql,
                mode: QueryMode::All,
                options: QueryExecutionOptions::default(),
            },
            parameters,
        })?;
        if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
            return Err(Error::new(ErrorKind::Query, summary.message.clone()));
        }
        imported_rows += chunk.len() as u64;
    }
    Ok(TableImportReport {
        total_rows: total_rows as u64,
        imported_rows,
        warnings,
    })
}
