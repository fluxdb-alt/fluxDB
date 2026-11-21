/// SQL Server 逻辑备份：DDL + 逐页 INSERT。
///
/// 不使用 `BACKUP DATABASE`：那是数据库级物理备份，文件落在服务器端，
/// 与「表右键 → 备份」的对象级语义不一致，也无法只备份选中的表。
/// 恢复只接受可静态分析的 SQL 子集，并要求目标对象不存在。
impl DatabaseBackup for SqlServerBackup {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::SqlServer
    }

    fn execution(&self, method: BackupMethod, _: bool) -> fluxdb_core::Result<BackupExecution> {
        match method {
            BackupMethod::Logical | BackupMethod::Auto => Ok(BackupExecution::SqlDump),
            BackupMethod::Native => Err(task_error(
                "SQL Server 表级备份不支持 BACKUP DATABASE 原生模式；请使用逻辑 SQL 备份",
            )),
        }
    }

    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        sqlserver_logical_backup(request, cancel, progress)
    }

    fn inspect_restore(
        &self,
        request: &RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<RestorePlan> {
        sqlserver_inspect_restore(request, cancel)
    }

    fn probe_restore(
        &self,
        request: &RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<Vec<RestoreObjectProbe>> {
        let existing = sqlserver_existing_relations(request)?;
        canceled(cancel)?;
        Ok(request
            .manifest
            .as_ref()
            .map(|meta| {
                meta.objects
                    .iter()
                    .map(|name| RestoreObjectProbe {
                        exists_in_target: existing
                            .iter()
                            .any(|relation| relation.eq_ignore_ascii_case(name)),
                        name: name.clone(),
                        key: name.clone(),
                        has_ddl: true,
                        has_data: true,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn restore(
        &self,
        request: &RestoreRequest,
        plan: &RestorePlan,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<RestoreOutcome> {
        sqlserver_restore(request, plan, cancel, progress)
    }
}

fn sqlserver_backup_objects(request: &BackupRequest) -> fluxdb_core::Result<Vec<ObjectPath>> {
    let connector = connector_for(&request.config)?;
    let database = if request.database.trim().is_empty() {
        sqlserver_profile_database(&request.config)?
    } else {
        request.database.clone()
    };
    let database_path = ObjectPath {
        connection_id: request.config.id,
        database: Some(database.clone()),
        schema: None,
        name: database,
        kind: ObjectKind::Database,
    };
    let mut relations = Vec::new();
    for schema in connector.list_objects(Some(&database_path))? {
        relations.extend(connector.list_objects(Some(&schema.path))?);
    }
    match &request.scope {
        BackupScope::All { include_views } => {
            let objects = relations
                .into_iter()
                .map(|object| object.path)
                .filter(|path| {
                    path.kind == ObjectKind::Table
                        || (*include_views && path.kind == ObjectKind::View)
                })
                .collect::<Vec<_>>();
            if objects.is_empty() {
                return Err(task_error(if *include_views {
                    "数据库中没有可备份的表或视图"
                } else {
                    "数据库中没有可备份的用户表"
                }));
            }
            Ok(objects)
        }
        BackupScope::Objects(objects) => {
            if objects.is_empty() {
                return Err(task_error("请至少选择一个备份对象"));
            }
            let default_schema = request
                .config
                .sqlserver_profile
                .as_ref()
                .map(|profile| profile.basic.schema.as_str())
                .unwrap_or("dbo");
            objects
                .iter()
                .map(|object| sqlserver_select_backup_object(&relations, object, default_schema))
                .collect()
        }
        BackupScope::SqliteSnapshot => Err(task_error("SQL Server 不支持 SQLite 快照备份")),
    }
}

fn sqlserver_select_backup_object(
    relations: &[ObjectSummary],
    requested: &BackupObjectRef,
    default_schema: &str,
) -> fluxdb_core::Result<ObjectPath> {
    let kind = match requested.kind {
        BackupObjectKind::Table => ObjectKind::Table,
        BackupObjectKind::View => ObjectKind::View,
    };
    let mut candidates = relations
        .iter()
        .filter(|object| {
            object.path.kind == kind
                && object.path.name.eq_ignore_ascii_case(requested.name.trim())
                && requested.schema.as_deref().is_none_or(|schema| {
                    object
                        .path
                        .schema
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(schema.trim()))
                })
        })
        .collect::<Vec<_>>();
    let default_schema = if default_schema.trim().is_empty() {
        "dbo"
    } else {
        default_schema.trim()
    };
    if candidates.len() > 1 {
        candidates.retain(|object| {
            object
                .path
                .schema
                .as_deref()
                .is_some_and(|schema| schema.eq_ignore_ascii_case(default_schema))
        });
    }
    match candidates.as_slice() {
        [] => Err(task_error(format!(
            "备份对象在数据库中不存在：{}",
            requested.key()
        ))),
        [object] => Ok(object.path.clone()),
        _ => Err(task_error(format!(
            "SQL Server 备份对象 {} 在多个 schema 中存在，请从具体 schema 中选择",
            requested.key()
        ))),
    }
}

fn sqlserver_logical_backup(
    request: &BackupRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<BackupManifest> {
    let connector = connector_for(&request.config)?;
    let objects = sqlserver_backup_objects(request)?;
    let mut writer = std::io::BufWriter::new(new_output(&request.output)?);
    writeln!(writer, "-- fluxDB SQL Server logical backup").map_err(io_error)?;
    let mut meta = manifest(request, Vec::new());
    for object in objects {
        canceled(cancel)?;
        let key = sqlserver_backup_key(&object);
        report(progress, "对象", format!("正在导出 {key}"));
        if request.include_schema {
            writeln!(writer, "{}", connector.table_ddl(&object)?).map_err(io_error)?;
        }
        let mut exported_rows: Option<u64> = None;
        if request.include_data && object.kind == ObjectKind::Table {
            let mut offset = 0;
            let mut rows_exported = 0u64;
            loop {
                canceled(cancel)?;
                let page = connector.load_data(&object, offset, 1000, &[], &[])?;
                if !page.rows.is_empty() {
                    // IDENTITY 列必须显式插入才能保留原主键；同一会话同时只允许一张表开启。
                    let has_identity = page.columns.iter().any(|column| {
                        column.primary_key
                            && column.type_name.as_deref().is_some_and(|name| {
                                matches!(
                                    name.to_ascii_lowercase().as_str(),
                                    "int" | "bigint" | "smallint" | "tinyint"
                                )
                            })
                    });
                    if has_identity {
                        writeln!(
                            writer,
                            "SET IDENTITY_INSERT {} ON;",
                            sqlserver_qualified_object(&object)
                        )
                        .map_err(io_error)?;
                    }
                    let columns = page
                        .columns
                        .iter()
                        .map(|column| {
                            fluxdb_core::SqlServerDialect.quote_identifier(&column.name)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    for row in &page.rows {
                        let values = row
                            .values
                            .iter()
                            .map(sqlserver_backup_literal)
                            .collect::<fluxdb_core::Result<Vec<_>>>()?;
                        writeln!(
                            writer,
                            "INSERT INTO {} ({columns}) VALUES ({});",
                            sqlserver_qualified_object(&object),
                            values.join(", ")
                        )
                        .map_err(io_error)?;
                    }
                    if has_identity {
                        writeln!(
                            writer,
                            "SET IDENTITY_INSERT {} OFF;",
                            sqlserver_qualified_object(&object)
                        )
                        .map_err(io_error)?;
                    }
                }
                offset += page.rows.len() as u64;
                rows_exported += page.rows.len() as u64;
                if !page.has_more || page.rows.is_empty() {
                    break;
                }
            }
            exported_rows = Some(rows_exported);
        }
        meta.objects.push(key.clone());
        meta.object_stats.push(BackupObjectStats {
            name: key,
            rows: exported_rows,
        });
    }
    // 本次为逐页独立读取，不是同一事务快照；清单如实标注。
    meta.consistent = Some(false);
    writer.flush().map_err(io_error)?;
    drop(writer);
    finalize_embedded_manifest(&request.output, &mut meta)?;
    Ok(meta)
}

fn sqlserver_backup_key(path: &ObjectPath) -> String {
    path.schema
        .as_deref()
        .filter(|schema| !schema.trim().is_empty())
        .map(|schema| format!("{schema}.{}", path.name))
        .unwrap_or_else(|| path.name.clone())
}

fn sqlserver_qualified_object(path: &ObjectPath) -> String {
    match path.schema.as_deref().filter(|schema| !schema.trim().is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            fluxdb_core::SqlServerDialect.quote_identifier(schema),
            fluxdb_core::SqlServerDialect.quote_identifier(&path.name)
        ),
        None => fluxdb_core::SqlServerDialect.quote_identifier(&path.name),
    }
}

fn sqlserver_backup_literal(value: &CellValue) -> fluxdb_core::Result<String> {
    Ok(match value {
        CellValue::Null => "NULL".into(),
        CellValue::Bool(value) => if *value { "1" } else { "0" }.into(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) if value.is_finite() => value.to_string(),
        // 字符串统一 N'' 前缀：nvarchar/nchar 列写入非 ASCII 时不会因排序规则丢字符。
        CellValue::Text(value) | CellValue::Json(value) => {
            format!("N'{}'", value.replace('\'', "''"))
        }
        CellValue::Bytes(value) => format!(
            "0x{}",
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
        CellValue::BinarySummary(summary) if summary.is_null => "NULL".into(),
        _ => {
            return Err(task_error(
                "数据包含未加载的二进制值、截断的 LOB 或非有限浮点数，请改用原生备份",
            ));
        }
    })
}

fn sqlserver_profile_database(config: &ConnectionConfig) -> fluxdb_core::Result<String> {
    config
        .sqlserver_profile
        .as_ref()
        .map(|profile| profile.basic.database.clone())
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| task_error("SQL Server 数据库名缺失"))
}

fn sqlserver_existing_relations(request: &RestoreRequest) -> fluxdb_core::Result<Vec<String>> {
    let connector = connector_for(&request.config)?;
    let database = if request.target.trim().is_empty() {
        sqlserver_profile_database(&request.config)?
    } else {
        request.target.clone()
    };
    let database_path = ObjectPath {
        connection_id: request.config.id,
        database: Some(database.clone()),
        schema: None,
        name: database,
        kind: ObjectKind::Database,
    };
    let mut names = Vec::new();
    for schema in connector.list_objects(Some(&database_path))? {
        for object in connector.list_objects(Some(&schema.path))? {
            if matches!(object.path.kind, ObjectKind::Table | ObjectKind::View) {
                names.push(sqlserver_backup_key(&object.path));
            }
        }
    }
    Ok(names)
}

fn sqlserver_inspect_restore(
    request: &RestoreRequest,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<RestorePlan> {
    canceled(cancel)?;
    if request.config.kind != DatabaseKind::SqlServer {
        return Err(task_error("恢复执行器与目标数据库类型不匹配"));
    }
    if let Some(meta) = &request.manifest {
        if !meta.complete {
            return Err(task_error("备份未完整成功，不能自动恢复"));
        }
        if meta.kind.is_some_and(|kind| kind != DatabaseKind::SqlServer) {
            return Err(task_error("源数据库与目标类型不兼容"));
        }
    }
    let metadata = fs::metadata(&request.source).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(task_error("备份必须是非空普通文件"));
    }
    sqlserver_validate_script(&request.source, cancel)?;
    let existing = sqlserver_existing_relations(request)?;
    // 只检查脚本真正要创建的对象；目标库里的其他表与本次恢复无关，
    // 不能因为它们存在就阻断恢复到另一个 schema。
    let targets = request
        .manifest
        .as_ref()
        .map(|meta| meta.objects.clone())
        .unwrap_or_else(|| sqlserver_script_objects(&request.source, cancel).unwrap_or_default());
    let conflicts = targets
        .iter()
        .filter(|name| existing.iter().any(|relation| relation.eq_ignore_ascii_case(name)))
        .cloned()
        .collect::<Vec<_>>();
    if !conflicts.is_empty() && request.table_decisions.is_empty() {
        return Err(task_error(format!(
            "SQL Server 恢复要求目标对象不存在；已存在：{}",
            conflicts.join("、")
        )));
    }
    let tables = request
        .manifest
        .as_ref()
        .map(|meta| {
            meta.objects
                .iter()
                .map(|name| RestoreTableInfo {
                    name: name.clone(),
                    exists_in_target: existing
                        .iter()
                        .any(|relation| relation.eq_ignore_ascii_case(name)),
                    has_ddl: true,
                    has_data: true,
                    default_action: RestoreTableAction::Create,
                    risks: Vec::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(RestorePlan {
        format: BackupFormat::Sql,
        summary: format!("{} → SQL Server（逻辑 SQL 导入）", request.source.display()),
        warnings: vec![
            "SQL Server 逻辑恢复要求目标对象不存在；失败可能留下部分对象。".into(),
            "IDENTITY 列用 SET IDENTITY_INSERT 显式写入，不会重新分配主键。".into(),
        ],
        source_size: metadata.len(),
        source_modified: metadata.modified().ok(),
        tables,
        per_table: false,
    })
}

/// 从脚本的 CREATE TABLE / CREATE VIEW 语句提取 `schema.name`。
/// 用于没有 manifest 的 SQL 文件：只检查本次会创建的对象，不误判目标库其他表。
fn sqlserver_script_objects(path: &Path, cancel: &AtomicBool) -> fluxdb_core::Result<Vec<String>> {
    let mut objects = Vec::new();
    for_each_statement(path, DatabaseKind::SqlServer, cancel, &mut |statement| {
        let trimmed = statement.trim_start();
        let upper = trimmed.to_ascii_uppercase();
        let prefix_len = if upper.starts_with("CREATE TABLE ") {
            "CREATE TABLE ".len()
        } else if upper.starts_with("CREATE VIEW ") {
            "CREATE VIEW ".len()
        } else {
            return Ok(());
        };
        let rest = trimmed[prefix_len..].trim_start();
        if let Some(name) = sqlserver_parse_qualified_name(rest) {
            objects.push(name);
        }
        Ok(())
    })?;
    Ok(objects)
}

fn sqlserver_parse_qualified_name(value: &str) -> Option<String> {
    let (first, rest) = sqlserver_parse_identifier(value)?;
    let rest = rest.trim_start();
    if let Some(after_dot) = rest.strip_prefix('.') {
        let (second, _) = sqlserver_parse_identifier(after_dot.trim_start())?;
        return Some(format!("{first}.{second}"));
    }
    Some(format!("dbo.{first}"))
}

fn sqlserver_parse_identifier(value: &str) -> Option<(String, &str)> {
    let value = value.trim_start();
    if let Some(rest) = value.strip_prefix('[') {
        let mut name = String::new();
        let mut rest = rest;
        loop {
            let end = rest.find(']')?;
            name.push_str(&rest[..end]);
            rest = &rest[end + 1..];
            if let Some(after_escape) = rest.strip_prefix(']') {
                name.push(']');
                rest = after_escape;
                continue;
            }
            return Some((name, rest));
        }
    }
    let end = value
        .find(|ch: char| ch.is_whitespace() || ch == '(' || ch == '.')
        .unwrap_or(value.len());
    (end > 0).then(|| (value[..end].to_string(), &value[end..]))
}

fn sqlserver_validate_script(path: &Path, cancel: &AtomicBool) -> fluxdb_core::Result<()> {
    let mut count = 0usize;
    for_each_statement(path, DatabaseKind::SqlServer, cancel, &mut |statement| {
        let upper = statement.trim_start().to_ascii_uppercase();
        let allowed = [
            "CREATE TABLE ",
            "CREATE INDEX ",
            "CREATE UNIQUE INDEX ",
            "CREATE VIEW ",
            "ALTER TABLE ",
            "INSERT INTO ",
            "SET IDENTITY_INSERT ",
        ];
        if !allowed.iter().any(|prefix| upper.starts_with(prefix)) {
            return Err(task_error(
                "SQL Server 恢复脚本包含当前不支持的语句；请人工检查",
            ));
        }
        if upper.contains("DROP DATABASE ")
            || upper.contains("BACKUP DATABASE ")
            || upper.contains("GRANT ")
            || upper.contains("REVOKE ")
            || upper.contains("CREATE USER ")
            || upper.contains("CREATE ROLE ")
        {
            return Err(task_error(
                "SQL Server 恢复脚本包含身份、实例或数据库级语句，禁止自动恢复",
            ));
        }
        count += 1;
        Ok(())
    })?;
    if count == 0 {
        return Err(task_error("SQL 文件没有可恢复的语句"));
    }
    Ok(())
}

fn sqlserver_restore(
    request: &RestoreRequest,
    plan: &RestorePlan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<RestoreOutcome> {
    let current = sqlserver_inspect_restore(request, cancel)?;
    if current.source_size != plan.source_size || current.source_modified != plan.source_modified {
        return Err(task_error("备份在确认后发生变化，请重新预检查"));
    }
    report(progress, "恢复", "正在执行 SQL Server 逻辑备份");
    let sql = fs::read_to_string(&request.source).map_err(io_error)?;
    canceled(cancel)?;
    let database = if request.target.trim().is_empty() {
        sqlserver_profile_database(&request.config)?
    } else {
        request.target.clone()
    };
    let connector = connector_for(&request.config)?;
    let execution = connector.execute(&QueryRequest {
        connection_id: request.config.id,
        database: Some(database.clone()),
        schema: request
            .config
            .sqlserver_profile
            .as_ref()
            .map(|profile| profile.basic.schema.clone())
            .filter(|schema| !schema.trim().is_empty())
            .or_else(|| Some("dbo".into())),
        session_id: None,
        text: sql,
        mode: fluxdb_core::QueryMode::All,
        options: QueryExecutionOptions {
            continue_on_error: false,
            split_statements: true,
            ..QueryExecutionOptions::default()
        },
    })?;
    if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
        return Err(task_error(format!(
            "SQL Server 恢复执行失败，可能已留下部分对象：{}",
            summary.message
        )));
    }
    report(progress, "验证", "核对 SQL Server 目标对象");
    let existing = sqlserver_existing_relations(request)?;
    let mut objects = Vec::new();
    if let Some(meta) = &request.manifest {
        for name in &meta.objects {
            let ok = existing
                .iter()
                .any(|relation| relation.eq_ignore_ascii_case(name));
            objects.push(RestoreObjectResult {
                name: name.clone(),
                action: Some(RestoreTableAction::Create),
                status: if ok {
                    RestoreObjectStatus::Succeeded
                } else {
                    RestoreObjectStatus::Failed
                },
                detail: if ok {
                    String::new()
                } else {
                    "恢复后未找到目标表".into()
                },
            });
        }
    }
    if objects
        .iter()
        .any(|object| object.status == RestoreObjectStatus::Failed)
    {
        return Err(task_error("SQL Server 恢复完成，但目标对象核对失败"));
    }
    Ok(RestoreOutcome {
        verification: format!("已在数据库 {database} 核对到备份清单中的对象；未逐表核对行数"),
        total_objects: objects.len(),
        warnings: plan.warnings.clone(),
        objects,
    })
}
