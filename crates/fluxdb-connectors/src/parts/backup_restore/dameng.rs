impl DatabaseBackup for DamengBackup {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Dameng
    }

    fn execution(&self, _: BackupMethod, _: bool) -> fluxdb_core::Result<BackupExecution> {
        Ok(BackupExecution::SqlDump)
    }

    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        dameng_logical_backup(request, cancel, progress)
    }

    fn inspect_restore(
        &self,
        request: &RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<RestorePlan> {
        dameng_inspect_restore(request, cancel)
    }

    fn probe_restore(
        &self,
        request: &RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<Vec<RestoreObjectProbe>> {
        dameng_probe_restore(request, cancel)
    }

    fn restore(
        &self,
        request: &RestoreRequest,
        plan: &RestorePlan,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<RestoreOutcome> {
        dameng_restore(request, plan, cancel, progress)
    }
}

fn dameng_inspect_restore(
    request: &RestoreRequest,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<RestorePlan> {
    canceled(cancel)?;
    if let Some(meta) = &request.manifest {
        if !meta.complete {
            return Err(task_error("备份未完整成功，不能自动恢复"));
        }
        if meta.kind.is_some_and(|kind| kind != DatabaseKind::Dameng) {
            return Err(task_error("源数据库与目标类型不兼容"));
        }
    }
    if !request.table_decisions.is_empty() {
        return Err(task_error("达梦当前只支持整 schema 恢复，不支持逐表选择"));
    }
    if let Some(meta) = &request.manifest {
        let schemas = meta
            .objects
            .iter()
            .filter_map(|name| name.split_once('.').map(|(schema, _)| schema.to_ascii_uppercase()))
            .collect::<std::collections::BTreeSet<_>>();
        if schemas.len() > 1 {
            return Err(task_error("达梦备份包含多个 schema，当前只支持单 schema 恢复"));
        }
    }
    let metadata = fs::metadata(&request.source).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(task_error("备份必须是非空普通文件"));
    }
    validate_dameng_script(&request.source, cancel)?;
    let schema = dameng_restore_schema(&request.config, &request.target)?;
    dameng_require_empty_schema(request, &schema)?;
    let table_names = request
        .manifest
        .as_ref()
        .map(|meta| {
            meta.objects
                .iter()
                .map(|name| {
                    RestoreTableInfo {
                        name: name.clone(),
                        exists_in_target: false,
                        has_ddl: true,
                        has_data: true,
                        default_action: RestoreTableAction::Create,
                        risks: Vec::new(),
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(RestorePlan {
        format: BackupFormat::Sql,
        summary: format!(
            "{} → schema {}（新建表）",
            request.source.display(),
            schema
        ),
        warnings: vec![
            "达梦恢复只支持整 schema 导入；目标 schema 必须为空，失败可能留下部分对象。".into(),
            "恢复不会切换数据库或自动删除目标 schema。".into(),
        ],
        source_size: metadata.len(),
        source_modified: metadata.modified().ok(),
        tables: table_names,
        per_table: false,
    })
}

fn dameng_probe_restore(
    request: &RestoreRequest,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<Vec<RestoreObjectProbe>> {
    canceled(cancel)?;
    validate_dameng_script(&request.source, cancel)?;
    let schema = dameng_restore_schema(&request.config, &request.target)?;
    let connector = connector_for(&request.config)?;
    let existing = match dameng_relation_names(&*connector, &request.config, &schema) {
        Ok(existing) => existing,
        Err(_) if request.create_target => Vec::new(),
        Err(error) => return Err(error),
    };
    Ok(request
        .manifest
        .as_ref()
        .map(|meta| {
            meta.objects
                .iter()
                .map(|name| {
                    let bare = name
                        .split_once('.')
                        .map(|(_, table)| table)
                        .unwrap_or(name);
                    RestoreObjectProbe {
                        name: name.clone(),
                        key: name.clone(),
                        exists_in_target: existing.iter().any(|table| table == bare),
                        has_ddl: true,
                        has_data: true,
                    }
                })
                .collect()
        })
        .unwrap_or_default())
}

fn dameng_restore(
    request: &RestoreRequest,
    plan: &RestorePlan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<RestoreOutcome> {
    let current = dameng_inspect_restore(request, cancel)?;
    if current.source_size != plan.source_size
        || current.source_modified != plan.source_modified
    {
        return Err(task_error("备份在确认后发生变化，请重新预检查"));
    }
    let schema = dameng_restore_schema(&request.config, &request.target)?;
    if request.create_target {
        report(progress, "目标", "正在创建达梦 schema");
        connector_for(&request.config)?.create_schema(
            request.config.id,
            &request
                .config
                .dameng_profile
                .as_ref()
                .map(|profile| profile.basic.database.clone())
                .unwrap_or_default(),
            &schema,
        )?;
    }
    report(progress, "恢复", "正在执行达梦 SQL 备份");
    let mut sql = fs::read_to_string(&request.source).map_err(io_error)?;
    if let Some(source_schema) = dameng_backup_source_schema(request)
        && !source_schema.eq_ignore_ascii_case(&schema)
    {
        sql = sql
            .replace(
                &format!("\"{source_schema}\"."),
                &format!("\"{schema}\"."),
            )
            .replace(
                &format!("{source_schema}."),
                &format!("{schema}."),
            );
    }
    canceled(cancel)?;
    let connector = connector_for(&request.config)?;
    let database = request
        .config
        .dameng_profile
        .as_ref()
        .map(|profile| profile.basic.database.clone())
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| task_error("达梦数据库名缺失"))?;
    let execution = connector.execute(&QueryRequest {
        connection_id: request.config.id,
        database: Some(database),
        schema: Some(schema.clone()),
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
            "达梦恢复执行失败，可能已留下部分对象：{}",
            summary.message
        )));
    }
    report(progress, "验证", "核对达梦目标对象");
    let existing = dameng_relation_names(&*connector, &request.config, &schema)?;
    let mut objects = Vec::new();
    if let Some(meta) = &request.manifest {
        for name in &meta.objects {
            let bare = name
                .split_once('.')
                .map(|(_, table)| table)
                .unwrap_or(name);
            let ok = existing.iter().any(|table| table == bare);
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
        return Err(task_error("达梦恢复完成，但目标对象核对失败"));
    }
    Ok(RestoreOutcome {
        verification: format!("已在 schema {schema} 核对到备份清单中的对象；未逐表核对行数"),
        total_objects: objects.len(),
        warnings: plan.warnings.clone(),
        objects,
    })
}

fn validate_dameng_script(path: &Path, cancel: &AtomicBool) -> fluxdb_core::Result<()> {
    let mut count = 0usize;
    for_each_statement(path, DatabaseKind::Dameng, cancel, &mut |statement| {
        let upper = statement.trim_start().to_ascii_uppercase();
        let allowed = [
            "CREATE TABLE ",
            "CREATE INDEX ",
            "CREATE UNIQUE INDEX ",
            "CREATE VIEW ",
            "CREATE OR REPLACE VIEW ",
            "COMMENT ON ",
            "INSERT INTO ",
            "DROP TABLE IF EXISTS ",
            "DROP VIEW IF EXISTS ",
            "ALTER TABLE ",
            "SET ",
        ];
        if !allowed.iter().any(|prefix| upper.starts_with(prefix)) {
            return Err(task_error(
                "达梦恢复脚本包含当前不支持的语句；请人工检查",
            ));
        }
        if upper.contains("GRANT ")
            || upper.contains("REVOKE ")
            || upper.contains("CREATE USER ")
            || upper.contains("CREATE ROLE ")
            || upper.contains("DROP USER ")
            || upper.contains("DROP DATABASE ")
        {
            return Err(task_error("达梦恢复脚本包含身份或实例级语句，禁止自动恢复"));
        }
        count += 1;
        Ok(())
    })?;
    if count == 0 {
        return Err(task_error("SQL 文件没有可恢复的语句"));
    }
    Ok(())
}

fn dameng_restore_schema(
    config: &ConnectionConfig,
    target: &str,
) -> fluxdb_core::Result<String> {
    let profile = config
        .dameng_profile
        .as_ref()
        .ok_or_else(|| task_error("达梦连接档案缺失"))?;
    let target = target.trim();
    if target.is_empty() || target.eq_ignore_ascii_case(&profile.basic.database) {
        return Ok(profile.basic.schema.clone());
    }
    if target.contains(['"', '/', '\\', '\0']) {
        return Err(task_error("目标 schema 名称无效"));
    }
    Ok(target.to_string())
}

fn dameng_require_empty_schema(
    request: &RestoreRequest,
    schema: &str,
) -> fluxdb_core::Result<()> {
    let connector = connector_for(&request.config)?;
    let database = request
        .config
        .dameng_profile
        .as_ref()
        .map(|profile| profile.basic.database.clone())
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| task_error("达梦数据库名缺失"))?;
    let database_path = ObjectPath {
        connection_id: request.config.id,
        database: Some(database.clone()),
        schema: None,
        name: database,
        kind: ObjectKind::Database,
    };
    let schemas = connector.list_objects(Some(&database_path))?;
    let target = schemas
        .into_iter()
        .find(|object| object.path.name.eq_ignore_ascii_case(schema));
    let Some(target) = target else {
        if request.create_target {
            return Ok(());
        }
        return Err(task_error(format!("达梦目标 schema 不存在：{schema}")));
    };
    if request.create_target {
        return Err(task_error(format!("达梦目标 schema 已存在：{schema}")));
    }
    let existing = connector
        .list_objects(Some(&target.path))?
        .into_iter()
        .filter(|object| {
            matches!(object.path.kind, ObjectKind::Table | ObjectKind::View)
        })
        .map(|object| object.path.name)
        .collect::<Vec<_>>();
    if !existing.is_empty() {
        return Err(task_error(format!(
            "达梦目标 schema {schema} 非空；当前恢复只允许导入空 schema"
        )));
    }
    Ok(())
}

fn dameng_relation_names(
    connector: &dyn Connector,
    config: &ConnectionConfig,
    schema: &str,
) -> fluxdb_core::Result<Vec<String>> {
    let database = config
        .dameng_profile
        .as_ref()
        .map(|profile| profile.basic.database.clone())
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| task_error("达梦数据库名缺失"))?;
    let database_path = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: database,
        kind: ObjectKind::Database,
    };
    let schemas = connector.list_objects(Some(&database_path))?;
    let target = schemas
        .into_iter()
        .find(|object| object.path.name.eq_ignore_ascii_case(schema))
        .ok_or_else(|| task_error(format!("达梦目标 schema 不存在：{schema}")))?;
    Ok(connector
        .list_objects(Some(&target.path))?
        .into_iter()
        .filter(|object| {
            matches!(object.path.kind, ObjectKind::Table | ObjectKind::View)
        })
        .map(|object| object.path.name)
        .collect())
}

fn dameng_backup_source_schema(request: &RestoreRequest) -> Option<String> {
    request
        .manifest
        .as_ref()
        .and_then(|manifest| {
            manifest
                .objects
                .iter()
                .find_map(|object| object.split_once('.').map(|(schema, _)| schema.to_string()))
        })
        .or_else(|| {
            let sql = fs::read_to_string(&request.source).ok()?;
            let upper = sql.to_ascii_uppercase();
            let create = upper.find("CREATE TABLE")? + "CREATE TABLE".len();
            let start = sql[create..].find('"')? + create + 1;
            let end = sql[start..].find('"')? + start;
            (end > start).then(|| sql[start..end].to_string())
        })
}

fn dameng_logical_backup(
    request: &BackupRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<BackupManifest> {
    let connector = connector_for(&request.config)?;
    let objects = dameng_backup_objects(request)?;
    let mut writer = std::io::BufWriter::new(new_output(&request.output)?);
    writeln!(writer, "-- fluxDB Dameng logical backup").map_err(io_error)?;
    let mut meta = manifest(request, Vec::new());
    for object in objects {
        canceled(cancel)?;
        let key = dameng_backup_key(&object);
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
                for row in &page.rows {
                    let columns = page
                        .columns
                        .iter()
                        .map(|column| fluxdb_core::DamengDialect.quote_identifier(&column.name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let values = row
                        .values
                        .iter()
                        .map(dameng_backup_literal)
                        .collect::<fluxdb_core::Result<Vec<_>>>()?;
                    writeln!(
                        writer,
                        "INSERT INTO {} ({columns}) VALUES ({});",
                        dameng_qualified_object(&object),
                        values.join(", ")
                    )
                    .map_err(io_error)?;
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

fn dameng_backup_objects(request: &BackupRequest) -> fluxdb_core::Result<Vec<ObjectPath>> {
    let connector = connector_for(&request.config)?;
    let database = if request.database.trim().is_empty() {
        request
            .config
            .dameng_profile
            .as_ref()
            .map(|profile| profile.basic.database.clone())
            .filter(|database| !database.trim().is_empty())
            .ok_or_else(|| task_error("达梦数据库名缺失"))?
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
    let schemas = connector.list_objects(Some(&database_path))?;
    let mut relations = Vec::new();
    for schema in schemas {
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
                .dameng_profile
                .as_ref()
                .map(|profile| profile.basic.schema.as_str())
                .unwrap_or("");
            objects
                .iter()
                .map(|object| dameng_select_backup_object(&relations, object, default_schema))
                .collect()
        }
        BackupScope::SqliteSnapshot => Err(task_error("达梦不支持 SQLite 快照备份")),
    }
}

fn dameng_select_backup_object(
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
                && requested
                    .schema
                    .as_deref()
                    .is_none_or(|schema| {
                        object
                            .path
                            .schema
                            .as_deref()
                            .is_some_and(|value| value.eq_ignore_ascii_case(schema.trim()))
                    })
        })
        .collect::<Vec<_>>();
    if candidates.len() > 1 && !default_schema.trim().is_empty() {
        candidates.retain(|object| {
            object
                .path
                .schema
                .as_deref()
                .is_some_and(|schema| schema.eq_ignore_ascii_case(default_schema.trim()))
        });
    }
    match candidates.as_slice() {
        [] => Err(task_error(format!(
            "备份对象在数据库中不存在：{}",
            requested.key()
        ))),
        [object] => Ok(object.path.clone()),
        _ => Err(task_error(format!(
            "达梦备份对象 {} 在多个 schema 中存在，请从具体 schema 中选择",
            requested.key()
        ))),
    }
}

fn dameng_qualified_object(path: &ObjectPath) -> String {
    match path.schema.as_deref().filter(|schema| !schema.trim().is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            fluxdb_core::DamengDialect.quote_identifier(schema),
            fluxdb_core::DamengDialect.quote_identifier(&path.name)
        ),
        None => fluxdb_core::DamengDialect.quote_identifier(&path.name),
    }
}

fn dameng_backup_key(path: &ObjectPath) -> String {
    path.schema
        .as_deref()
        .filter(|schema| !schema.trim().is_empty())
        .map(|schema| format!("{schema}.{}", path.name))
        .unwrap_or_else(|| path.name.clone())
}

fn dameng_backup_literal(value: &CellValue) -> fluxdb_core::Result<String> {
    Ok(match value {
        CellValue::Null => "NULL".into(),
        CellValue::Bool(value) => if *value { "1" } else { "0" }.into(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) if value.is_finite() => value.to_string(),
        CellValue::Text(value) | CellValue::Json(value) => {
            format!("'{}'", value.replace('\'', "''"))
        }
        CellValue::Bytes(value) => format!(
            "HEXTORAW('{}')",
            value.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        ),
        CellValue::BinarySummary(summary) if summary.is_null => "NULL".into(),
        _ => {
            return Err(task_error(
                "数据包含未加载的二进制值、截断的 LOB 或非有限浮点数，请改用原生备份",
            ));
        }
    })
}
