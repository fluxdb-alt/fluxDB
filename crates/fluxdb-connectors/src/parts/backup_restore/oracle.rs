// Oracle 逻辑导出 / 导入。
//
// 三条硬约束：
// 1. 不提供原生备份。Data Pump 的 `DIRECTORY` 指向数据库服务器端目录，
//    与桌面端选择的本地路径不是一回事；把它包装成「备份」会误导用户。
// 2. 逐页读取不是同一个快照。Oracle 默认读已提交，多次独立分页查询之间
//    数据可能变化（长导出还可能遇到 `ORA-01555`），所以产出物只能叫
//    「逻辑导出」，不能宣称一致快照或可完整回滚的备份。
// 3. 导入只允许可静态分析、且只建对象不删对象的语句：Oracle 的 DDL 隐式提交，
//    中途失败已执行的语句不会回滚，计划里必须直说。
impl DatabaseBackup for OracleBackup {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Oracle
    }

    fn execution(&self, method: BackupMethod, _: bool) -> fluxdb_core::Result<BackupExecution> {
        match method {
            BackupMethod::Logical | BackupMethod::Auto => Ok(BackupExecution::SqlDump),
            BackupMethod::Native => Err(task_error(
                "Oracle 原生备份需要 Data Pump 与数据库服务器端 DIRECTORY；当前只提供本机逻辑导出",
            )),
        }
    }

    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        oracle_logical_backup(request, cancel, progress)
    }

    fn inspect_restore(
        &self,
        request: &RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<RestorePlan> {
        oracle_inspect_restore(request, cancel)
    }

    fn probe_restore(
        &self,
        request: &RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<Vec<RestoreObjectProbe>> {
        canceled(cancel)?;
        let existing = oracle_relation_names(request).unwrap_or_default();
        Ok(request
            .manifest
            .as_ref()
            .map(|meta| {
                meta.objects
                    .iter()
                    .map(|name| {
                        let bare = oracle_bare_name(name);
                        RestoreObjectProbe {
                            name: bare.to_string(),
                            key: name.clone(),
                            exists_in_target: existing
                                .iter()
                                .any(|relation| relation.eq_ignore_ascii_case(bare)),
                            has_ddl: true,
                            has_data: true,
                        }
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
        oracle_restore(request, plan, cancel, progress)
    }
}

fn oracle_inspect_restore(
    request: &RestoreRequest,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<RestorePlan> {
    canceled(cancel)?;
    let metadata = fs::metadata(&request.source).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(task_error("导出文件必须是非空普通文件"));
    }
    if let Some(meta) = &request.manifest {
        if !meta.complete {
            return Err(task_error("导出未完整成功，不能自动导入"));
        }
        if meta.kind.is_some_and(|kind| kind != DatabaseKind::Oracle) {
            return Err(task_error("源数据库与目标类型不兼容"));
        }
        if !meta.include_schema {
            return Err(task_error("Oracle 导入需要包含表结构的导出文件"));
        }
    }
    if !request.table_decisions.is_empty() {
        return Err(task_error("Oracle 当前只支持整 schema 导入，不支持逐表选择"));
    }
    let schema = oracle_restore_schema(request)?;
    validate_oracle_script(&request.source, cancel)?;
    oracle_check_target(request, &schema)?;
    let tables = request
        .manifest
        .as_ref()
        .map(|meta| {
            meta.objects
                .iter()
                .map(|name| RestoreTableInfo {
                    name: oracle_bare_name(name).to_string(),
                    exists_in_target: false,
                    has_ddl: true,
                    has_data: true,
                    default_action: RestoreTableAction::Create,
                    risks: Vec::new(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(RestorePlan {
        format: BackupFormat::Sql,
        summary: format!("{} → schema {}", request.source.display(), schema),
        warnings: vec![
            "Oracle 导入按整 schema 重建对象：目标 schema 必须为空或由本次新建。".into(),
            "Oracle DDL 会隐式提交：中途失败无法整体回滚，已执行的语句不会自动撤销。".into(),
            "导出按分页独立读取，不是同一快照；跨页期间的数据变化不会体现在文件里。".into(),
        ],
        source_size: metadata.len(),
        source_modified: metadata.modified().ok(),
        tables,
        per_table: false,
    })
}

fn oracle_restore(
    request: &RestoreRequest,
    plan: &RestorePlan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<RestoreOutcome> {
    let current = oracle_inspect_restore(request, cancel)?;
    if current.source_size != plan.source_size || current.source_modified != plan.source_modified {
        return Err(task_error("导出文件在确认后发生变化，请重新预检查"));
    }
    let schema = oracle_restore_schema(request)?;
    let connector = connector_for(&request.config)?;
    if request.create_target {
        report(progress, "目标", format!("正在创建 Oracle 账号 {schema}"));
        // Oracle 的 schema 就是账号，建 schema 等价于建用户，需要 CREATE USER 权限。
        // 数据库参数对 Oracle 无意义：schema 就是账号，连接器内部只用 schema。
        connector.create_schema(request.config.id, "", &schema)?;
    }
    report(progress, "导入", format!("正在执行逻辑导出文件到 {schema}"));
    let mut sql = fs::read_to_string(&request.source).map_err(io_error)?;
    // 源 Owner 与目标不同时改写限定名；其余文本保持原样。
    if let Some(source_schema) = oracle_backup_source_schema(request, &sql)
        && !source_schema.eq_ignore_ascii_case(&schema)
    {
        sql = sql
            .replace(
                &format!("\"{source_schema}\"."),
                &format!("\"{schema}\"."),
            )
            .replace(&format!("{source_schema}."), &format!("{schema}."));
    }
    canceled(cancel)?;
    let result = connector.execute(&QueryRequest {
        connection_id: request.config.id,
        database: None,
        schema: Some(schema.clone()),
        session_id: None,
        text: sql,
        mode: QueryMode::All,
        options: QueryExecutionOptions {
            continue_on_error: false,
            split_statements: true,
            ..QueryExecutionOptions::default()
        },
    })?;
    if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
        // 已执行的 DDL 已隐式提交，只能如实报告，不能声称已回滚。
        return Err(task_error(format!(
            "Oracle 导入中断（DDL 已隐式提交，无法整体回滚）：{}",
            summary.message
        )));
    }
    report(progress, "验证", "核对 Oracle 目标对象");
    let existing = oracle_relation_names(request)?;
    let mut objects = Vec::new();
    if let Some(meta) = &request.manifest {
        for name in &meta.objects {
            let bare = oracle_bare_name(name);
            let ok = existing.iter().any(|relation| relation == bare);
            objects.push(RestoreObjectResult {
                name: bare.to_string(),
                action: Some(RestoreTableAction::Create),
                status: if ok {
                    RestoreObjectStatus::Succeeded
                } else {
                    RestoreObjectStatus::Failed
                },
                detail: if ok {
                    String::new()
                } else {
                    "导入后未找到目标对象".into()
                },
            });
        }
    }
    if objects
        .iter()
        .any(|object| object.status == RestoreObjectStatus::Failed)
    {
        return Err(task_error("Oracle 导入完成，但目标对象核对失败"));
    }
    Ok(RestoreOutcome {
        verification: format!("已在 schema {schema} 核对到导出清单中的对象；未逐表核对行数"),
        total_objects: objects.len(),
        warnings: plan.warnings.clone(),
        objects,
    })
}

/// 目标 schema：优先用对话框里填的名字，其次回落到连接档案的默认 schema。
fn oracle_restore_schema(request: &RestoreRequest) -> fluxdb_core::Result<String> {
    let target = request.target.trim();
    if !target.is_empty() {
        if target.contains(['\0', '/', '\\', '"', '\'']) {
            return Err(task_error("目标 schema 名称无效"));
        }
        return Ok(target.to_string());
    }
    request
        .config
        .oracle_profile
        .as_ref()
        .map(|profile| profile.basic.schema.trim().to_string())
        .filter(|schema| !schema.is_empty())
        .ok_or_else(|| task_error("缺少 Oracle 目标 schema"))
}

/// 目标必须「新建」或「已存在且为空」；两者都靠查询确认，不假设。
fn oracle_check_target(request: &RestoreRequest, schema: &str) -> fluxdb_core::Result<()> {
    let connector = connector_for(&request.config)?;
    let exists = oracle_schema_names(&*connector)?
        .iter()
        .any(|name| name.eq_ignore_ascii_case(schema));
    if request.create_target {
        if exists {
            return Err(task_error(format!("Oracle 账号 {schema} 已存在，不能按新建导入")));
        }
        return Ok(());
    }
    if !exists {
        return Err(task_error(format!("Oracle 目标 schema 不存在：{schema}")));
    }
    let existing = oracle_relation_names(request)?;
    if !existing.is_empty() {
        return Err(task_error(format!(
            "Oracle 目标 schema {schema} 非空；当前导入只允许写入空 schema"
        )));
    }
    Ok(())
}

fn oracle_relation_names(request: &RestoreRequest) -> fluxdb_core::Result<Vec<String>> {
    let schema = oracle_restore_schema(request)?;
    let connector = connector_for(&request.config)?;
    Ok(connector
        .list_objects(Some(&ObjectPath {
            connection_id: request.config.id,
            database: None,
            schema: None,
            name: schema,
            kind: ObjectKind::Schema,
        }))?
        .into_iter()
        .filter(|object| matches!(object.path.kind, ObjectKind::Table | ObjectKind::View))
        .map(|object| object.path.name)
        .collect())
}

/// 导入前置校验：只接受「建对象 + 写数据」，拒绝删对象与身份语句。
fn validate_oracle_script(path: &Path, cancel: &AtomicBool) -> fluxdb_core::Result<()> {
    let mut count = 0usize;
    for_each_statement(path, DatabaseKind::Oracle, cancel, &mut |statement| {
        let upper = statement.trim_start().to_ascii_uppercase();
        let allowed = [
            "CREATE TABLE ",
            "CREATE INDEX ",
            "CREATE UNIQUE INDEX ",
            "CREATE VIEW ",
            "CREATE OR REPLACE VIEW ",
            "COMMENT ON ",
            "INSERT INTO ",
            "ALTER TABLE ",
        ];
        if !allowed.iter().any(|prefix| upper.starts_with(prefix)) {
            return Err(task_error(
                "Oracle 导入脚本包含当前不支持的语句；请人工检查",
            ));
        }
        if upper.contains("DROP ")
            || upper.contains("TRUNCATE ")
            || upper.contains("GRANT ")
            || upper.contains("REVOKE ")
            || upper.contains("CREATE USER ")
            || upper.contains("ALTER USER ")
            || upper.contains("CREATE ROLE ")
        {
            return Err(task_error("Oracle 导入脚本包含删除或身份语句，禁止自动导入"));
        }
        count += 1;
        Ok(())
    })?;
    if count == 0 {
        return Err(task_error("SQL 文件没有可导入的语句"));
    }
    Ok(())
}

fn oracle_backup_source_schema(request: &RestoreRequest, sql: &str) -> Option<String> {
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
            let upper = sql.to_ascii_uppercase();
            let create = upper.find("CREATE TABLE")? + "CREATE TABLE".len();
            let start = sql[create..].find('"')? + create + 1;
            let end = sql[start..].find('"')? + start;
            (end > start).then(|| sql[start..end].to_string())
        })
}

fn oracle_logical_backup(
    request: &BackupRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<BackupManifest> {
    let connector = connector_for(&request.config)?;
    let objects = oracle_backup_objects(request)?;
    let mut writer = std::io::BufWriter::new(new_output(&request.output)?);
    writeln!(
        writer,
        "-- fluxDB Oracle logical export (per-page reads, not a consistent snapshot)"
    )
    .map_err(io_error)?;
    let mut meta = manifest(request, Vec::new());
    for object in objects {
        canceled(cancel)?;
        let key = oracle_backup_key(&object);
        report(progress, "对象", format!("正在导出 {key}"));
        if request.include_schema {
            writeln!(writer, "{}", connector.table_ddl(&object)?).map_err(io_error)?;
        }
        let mut exported_rows: Option<u64> = None;
        if request.include_data && object.kind == ObjectKind::Table {
            let mut offset = 0u64;
            let mut rows_exported = 0u64;
            loop {
                canceled(cancel)?;
                // 分页独立读取：不是同一 SCN，跨页期间的变化可能混进文件。
                let page = connector.load_data(&object, offset, 1000, &[], &[])?;
                let columns = page
                    .columns
                    .iter()
                    .map(|column| fluxdb_core::OracleDialect.quote_identifier(&column.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                for row in &page.rows {
                    let values = row
                        .values
                        .iter()
                        .map(oracle_backup_literal)
                        .collect::<fluxdb_core::Result<Vec<_>>>()?;
                    writeln!(
                        writer,
                        "INSERT INTO {} ({columns}) VALUES ({});",
                        oracle_qualified_object(&object),
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

/// Oracle 的对象树是「容器 → Owner → 表」，没有库这一层。
/// 根节点用空名请求，得到当前容器下的可见 Owner 列表。
fn oracle_schema_names(connector: &dyn Connector) -> fluxdb_core::Result<Vec<String>> {
    let root = connector.list_objects(None)?;
    let mut names = Vec::new();
    for container in root {
        for schema in connector.list_objects(Some(&container.path))? {
            names.push(schema.path.name);
        }
    }
    Ok(names)
}

fn oracle_backup_objects(request: &BackupRequest) -> fluxdb_core::Result<Vec<ObjectPath>> {
    let connector = connector_for(&request.config)?;
    let mut relations = Vec::new();
    for container in connector.list_objects(None)? {
        for schema in connector.list_objects(Some(&container.path))? {
            relations.extend(connector.list_objects(Some(&schema.path))?);
        }
    }
    match &request.scope {
        BackupScope::All { include_views } => {
            let selected = relations
                .into_iter()
                .map(|object| object.path)
                .filter(|path| {
                    path.kind == ObjectKind::Table
                        || (*include_views && path.kind == ObjectKind::View)
                })
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Err(task_error("没有可导出的表或视图"));
            }
            Ok(selected)
        }
        BackupScope::Objects(requested) => {
            if requested.is_empty() {
                return Err(task_error("请至少选择一个导出对象"));
            }
            let default_schema = request
                .config
                .oracle_profile
                .as_ref()
                .map(|profile| profile.basic.schema.clone())
                .unwrap_or_default();
            requested
                .iter()
                .map(|object| oracle_select_backup_object(&relations, object, &default_schema))
                .collect()
        }
        BackupScope::SqliteSnapshot => Err(task_error("Oracle 不支持 SQLite 快照导出")),
    }
}

fn oracle_select_backup_object(
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
    // 表右键没带 schema；多个 Owner 有同名表时按当前会话的默认 Owner 收敛。
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
            "导出对象在数据库中不存在：{}",
            requested.key()
        ))),
        [object] => Ok(object.path.clone()),
        _ => Err(task_error(format!(
            "导出对象 {} 在多个 Owner 中存在，请从具体 schema 中选择",
            requested.key()
        ))),
    }
}

fn oracle_qualified_object(path: &ObjectPath) -> String {
    match path
        .schema
        .as_deref()
        .filter(|schema| !schema.trim().is_empty())
    {
        Some(schema) => format!(
            "{}.{}",
            fluxdb_core::OracleDialect.quote_identifier(schema),
            fluxdb_core::OracleDialect.quote_identifier(&path.name)
        ),
        None => fluxdb_core::OracleDialect.quote_identifier(&path.name),
    }
}

fn oracle_backup_key(path: &ObjectPath) -> String {
    path.schema
        .as_deref()
        .filter(|schema| !schema.trim().is_empty())
        .map(|schema| format!("{schema}.{}", path.name))
        .unwrap_or_else(|| path.name.clone())
}

fn oracle_bare_name(key: &str) -> &str {
    key.split_once('.').map(|(_, name)| name).unwrap_or(key)
}

/// Oracle 字面量：二进制走 `HEXTORAW`，日期时间按 `TIMESTAMP '…'` 避免依赖 `NLS_DATE_FORMAT`。
/// NUMBER 在连接器里已经转成精确文本，这里不转数值类型，避免大数丢精度。
fn oracle_backup_literal(value: &CellValue) -> fluxdb_core::Result<String> {
    Ok(match value {
        CellValue::Null => "NULL".into(),
        CellValue::Bool(value) => if *value { "1" } else { "0" }.into(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) if value.is_finite() => value.to_string(),
        CellValue::Text(value) => match oracle_timestamp_literal(value) {
            Some(literal) => literal,
            None => format!("'{}'", value.replace('\'', "''")),
        },
        CellValue::Json(value) => format!("'{}'", value.replace('\'', "''")),
        CellValue::Bytes(value) => format!(
            "HEXTORAW('{}')",
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
        CellValue::BinarySummary(summary) if summary.is_null => "NULL".into(),
        _ => {
            return Err(task_error(
                "数据包含未加载的二进制值、截断的 LOB 或非有限浮点数，无法无损导出",
            ));
        }
    })
}

/// 只把 `YYYY-MM-DD HH:MM:SS[.fffffffff]` 形态的文本识别为时间戳字面量，
/// 其余文本保持字符串，避免把用户数据静默改成日期类型。
fn oracle_timestamp_literal(value: &str) -> Option<String> {
    let (date, time) = value.split_once(' ')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?;
    let month = date_parts.next()?;
    let day = date_parts.next()?;
    if date_parts.next().is_some()
        || year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || !year.chars().all(|c| c.is_ascii_digit())
        || !month.chars().all(|c| c.is_ascii_digit())
        || !day.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let (clock, fraction) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (time, None),
    };
    let mut clock_parts = clock.split(':');
    let hour = clock_parts.next()?;
    let minute = clock_parts.next()?;
    let second = clock_parts.next()?;
    if clock_parts.next().is_some()
        || clock.len() != 8
        || hour.len() != 2
        || minute.len() != 2
        || second.len() != 2
        || !clock.chars().all(|c| c.is_ascii_digit() || c == ':')
    {
        return None;
    }
    if let Some(fraction) = fraction
        && (fraction.is_empty()
            || fraction.len() > 9
            || !fraction.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(format!("TIMESTAMP '{value}'"))
}
