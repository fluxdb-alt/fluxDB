include!("logical.rs");
include!("script.rs");
include!("partition.rs");
include!("checks.rs");
fn inspect_restore(
    kind: DatabaseKind,
    request: &RestoreRequest,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<RestorePlan> {
    canceled(cancel)?;
    if request.config.kind != kind {
        return Err(task_error("恢复执行器与目标数据库类型不匹配"));
    }
    let source = fs::metadata(&request.source).map_err(io_error)?;
    if !source.is_file() || source.len() == 0 {
        return Err(task_error("备份必须是非空普通文件"));
    }
    let mut header = [0; 4096];
    let n = fs::File::open(&request.source)
        .map_err(io_error)?
        .read(&mut header)
        .map_err(io_error)?;
    let format = detect_backup_format(&header[..n], std::str::from_utf8(&header[..n]).ok())
        .ok_or_else(|| task_error("无法识别备份真实格式"))?;
    if (kind == DatabaseKind::Sqlite) != (format == BackupFormat::SqliteBinary) {
        return Err(task_error(
            "备份格式与目标数据库不匹配；SQLite 当前仅支持二进制快照",
        ));
    }
    let mut warnings = vec!["恢复失败或取消可能留下部分目标数据，任务不会自动删除目标。".into()];
    if let Some(meta) = &request.manifest {
        if !meta.complete {
            return Err(task_error("备份未完整成功，不能自动恢复"));
        }
        if !meta.include_schema {
            // 仅数据备份：只有在“现有库 + 逐表 + 全部动作为追加/清空/跳过”时才允许，
            // 因为这些动作保留目标既有结构；否则拒绝（无处建表）。
            let data_only_ok = !request.create_target
                && !request.table_decisions.is_empty()
                && request.table_decisions.iter().all(|d| {
                    matches!(
                        d.action,
                        RestoreTableAction::Append
                            | RestoreTableAction::TruncateAndLoad
                            | RestoreTableAction::Skip
                    )
                });
            if !data_only_ok {
                return Err(task_error(
                    "仅数据备份只能追加/清空导入到已有兼容结构的现有库，且需逐表选择动作",
                ));
            }
            warnings.push("此备份仅含数据，将导入到目标已有结构；列兼容性以预检查为准。".into());
        }
        if meta.kind.is_some_and(|source| {
            source != kind && !(source == DatabaseKind::MySql && kind == DatabaseKind::TiDb)
        }) {
            return Err(task_error("源数据库与目标类型不兼容"));
        }
        if !meta.include_data {
            warnings.push("此备份仅包含结构，不会恢复表数据。".into());
        }
    } else {
        warnings.push("旧备份或外部文件没有完整元数据；范围与源版本未知。".into());
    }
    let per_table = !request.table_decisions.is_empty();
    let mut plan_tables: Vec<RestoreTableInfo> = Vec::new();
    if kind == DatabaseKind::Sqlite {
        let target = Path::new(&request.target);
        if !request.create_target || target.exists() {
            return Err(task_error("SQLite 只能恢复到不存在的新文件"));
        }
        if target.file_name().is_none() || !target.parent().unwrap_or(Path::new(".")).is_dir() {
            return Err(task_error("SQLite 目标目录不存在"));
        }
        validate_sqlite(&request.source)?;
    } else {
        if request.target.trim().is_empty() || request.target.contains(['\0', '/', '\\', '=']) {
            return Err(task_error("目标数据库名无效"));
        }
        validate_script(&request.source, kind, cancel)?;
        // 逐表模式：分桶 + 解析默认动作/校验决策；整库模式 plan_tables 为空。
        if per_table {
            let (parts, stmts) = partition_statements(&request.source, kind, cancel)?;
            plan_tables =
                resolve_table_decisions(kind, request, &parts, &stmts, &mut warnings, cancel)?;
        }
        // 实际执行前再次检查，不依赖 UI 的旧对象树快照。
        inspect_target(request)?;
        if request.tool.as_os_str().is_empty() {
            return Err(task_error("未找到数据库恢复客户端"));
        }
        let mut version_cmd = Command::new(&request.tool);
        version_cmd.arg("--version");
        let version = fluxdb_core::no_console(version_cmd).output().map_err(io_error)?;
        if !version.status.success() {
            return Err(task_error("恢复客户端版本检查失败"));
        }
        warnings.push("只执行受支持的静态 SQL 转储；不支持任意脚本、存储过程或动态 SQL。源版本兼容性需要用户确认。".into());
    }
    // 事务范围按引擎能力兑现，并把「实际范围」写进计划（设计文档 §6.7：不用布尔值概括）。
    if request.options.transaction == RestoreTransactionMode::SingleTransaction {
        match kind {
            DatabaseKind::Postgres => warnings.push(
                "PostgreSQL 以单事务执行（psql --single-transaction），失败整体回滚。".into(),
            ),
            DatabaseKind::MySql | DatabaseKind::TiDb => {
                if mysql_single_transaction_ok(request, &plan_tables) {
                    warnings.push(
                        "MySQL/TiDB 在单事务中导入数据（无建表/重建 DDL），失败整体回滚。".into(),
                    );
                } else {
                    warnings.push(
                        "MySQL/TiDB 含建表/重建（DDL 隐式提交）或整库导入，无法整任务原子回滚；按引擎默认逐语句提交。"
                            .into(),
                    );
                }
            }
            _ => {}
        }
    }
    Ok(RestorePlan {
        format,
        summary: format!(
            "{} → {}（{}）",
            request.source.display(),
            request.target,
            if request.create_target {
                "创建新目标"
            } else if per_table {
                "现有库(逐表)"
            } else {
                "现有空库"
            }
        ),
        warnings,
        source_size: source.len(),
        source_modified: source.modified().ok(),
        tables: plan_tables,
        per_table,
    })
}
/// 对象页进入时的轻量探测：解析备份内容 + 查询目标存在性，返回逐对象事实。
/// 只读、不阻断、不定动作——与最终 `inspect_restore` 分离；UI 据此按存在性/内容
/// 给出默认动作与合法候选（设计文档 §6.3/§14.7）。SQLite 为整库快照无逐对象语义，返回空。
fn probe_restore(
    kind: DatabaseKind,
    request: &RestoreRequest,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<Vec<RestoreObjectProbe>> {
    canceled(cancel)?;
    if request.config.kind != kind {
        return Err(task_error("恢复执行器与目标数据库类型不匹配"));
    }
    if kind == DatabaseKind::Sqlite {
        return Ok(Vec::new());
    }
    if !request.source.is_file() {
        return Err(task_error("备份必须是非空普通文件"));
    }
    if request.target.trim().is_empty() || request.target.contains(['\0', '/', '\\', '=']) {
        return Err(task_error("目标数据库名无效"));
    }
    // 解析备份内容：文件损坏/方言未知在此暴露；不执行 SQL、不建库。
    let (parts, _stmts) = partition_statements(&request.source, kind, cancel)?;
    canceled(cancel)?;
    // 探测目标存在性：只读枚举目标库对象，需目标库可访问。
    let connector = connector_for(&request.config)?;
    connector.test_connection(&request.config)?;
    let existing = target_table_keys(kind, request, &*connector)?;
    Ok(parts
        .into_iter()
        // 展示名用 key：PG 非 public 自带 schema 前缀（schema.name），其余为裸名。
        .map(|part| RestoreObjectProbe {
            exists_in_target: existing.contains(&part.key),
            has_ddl: part.has_ddl_drop || part.has_ddl_create,
            has_data: part.has_data,
            name: part.key.clone(),
            key: part.key,
        })
        .collect())
}
fn inspect_target(request: &RestoreRequest) -> fluxdb_core::Result<()> {
    let connector = connector_for(&request.config)?;
    connector.test_connection(&request.config)?;
    let databases = connector.list_objects(None)?;
    let exists = databases.iter().any(|o| {
        o.path.name == request.target || o.path.database.as_deref() == Some(&request.target)
    });
    // 逐表模式（带 table_decisions）放行非空目标；整库模式仍要求空（向后兼容）。
    let per_table = !request.table_decisions.is_empty();
    if request.create_target {
        if exists {
            return Err(task_error("目标数据库已存在，请选择新名称或现有库模式"));
        }
    } else {
        if !exists {
            return Err(task_error("目标数据库不存在"));
        }
        if per_table {
            return Ok(());
        }
        // 计数检查覆盖表、视图、序列、例程；不以 UI 当前 schema 的列表判断整库为空。
        let sql = if request.config.kind == DatabaseKind::Postgres {
            "SELECT (SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT LIKE 'pg_%' AND n.nspname <> 'information_schema') + (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname NOT LIKE 'pg_%' AND n.nspname <> 'information_schema')"
        } else {
            "SELECT (SELECT count(*) FROM information_schema.tables WHERE table_schema=DATABASE()) + (SELECT count(*) FROM information_schema.routines WHERE routine_schema=DATABASE()) + (SELECT count(*) FROM information_schema.events WHERE event_schema=DATABASE())"
        };
        let result = connector.execute(&fluxdb_core::QueryRequest {
            connection_id: request.config.id,
            database: Some(request.target.clone()),
            schema: None,
            text: sql.into(),
            mode: fluxdb_core::QueryMode::All,
            options: fluxdb_core::QueryExecutionOptions {
                continue_on_error: false,
                ..Default::default()
            },
            session_id: None,
        })?;
        let count = result
            .results
            .first()
            .and_then(|p| p.rows.first())
            .and_then(|r| r.values.first())
            .ok_or_else(|| task_error("无法验证目标是否为空"))?;
        if !matches!(count, CellValue::I64(0)) && !matches!(count, CellValue::Text(v) if v == "0") {
            return Err(task_error("目标数据库非空，第一版禁止覆盖或合并"));
        }
    }
    Ok(())
}

/// 枚举目标库的所有表名（决策匹配键）。PG 遍历 schema；其余枚举库下表。
fn target_table_keys(
    kind: DatabaseKind,
    request: &RestoreRequest,
    connector: &dyn Connector,
) -> fluxdb_core::Result<std::collections::BTreeSet<String>> {
    use fluxdb_core::ObjectKind;
    let root = ObjectPath {
        connection_id: request.config.id,
        database: Some(request.target.clone()),
        schema: None,
        name: String::new(),
        kind: ObjectKind::Schema,
    };
    let mut keys = std::collections::BTreeSet::new();
    let objects = if kind == DatabaseKind::Postgres {
        let database_path = ObjectPath {
            kind: ObjectKind::Database,
            ..root.clone()
        };
        let mut v = Vec::new();
        for schema in connector.list_objects(Some(&database_path))? {
            v.extend(connector.list_objects(Some(&schema.path))?);
        }
        v
    } else {
        connector.list_objects(Some(&root))?
    };
    for o in objects {
        if o.path.kind != ObjectKind::Table && o.path.kind != ObjectKind::View {
            continue;
        }
        let bare = o.path.name.clone();
        if kind == DatabaseKind::Postgres {
            let schema = o.path.schema.clone().unwrap_or_else(|| "public".to_string());
            if schema != "public" {
                keys.insert(format!("{schema}.{bare}"));
            } else {
                keys.insert(bare);
            }
        } else {
            keys.insert(bare);
        }
    }
    Ok(keys)
}

/// 依目标存在性与备份内容推导默认动作。
fn default_action(exists: bool, has_ddl: bool, has_data: bool) -> RestoreTableAction {
    match (exists, has_ddl) {
        (false, true) => RestoreTableAction::Create,
        (true, true) => RestoreTableAction::Recreate,
        (true, false) if has_data => RestoreTableAction::Append,
        _ => RestoreTableAction::Skip,
    }
}

/// MySQL/TiDB 单事务能否真正兑现：逐表模式、不新建目标库、且无建表/重建（DDL 会隐式提交）。
/// 清空后导入用 DELETE、追加用 INSERT，均为事务性语句，可纳入单事务。
fn mysql_single_transaction_ok(
    request: &RestoreRequest,
    plan_tables: &[RestoreTableInfo],
) -> bool {
    !request.create_target
        && !request.table_decisions.is_empty()
        && plan_tables.iter().all(|t| {
            !matches!(
                t.default_action,
                RestoreTableAction::Create | RestoreTableAction::Recreate
            )
        })
}

/// 逐表行数验证（设计文档 §6.7 完成验证=逐表行数）：对目标库执行 count(*)。
/// 表名可能是 `schema.name`（PG）或裸名；失败返回 None，不阻断整体恢复。
fn count_target_rows(
    kind: DatabaseKind,
    request: &RestoreRequest,
    connector: &dyn Connector,
    key: &str,
) -> Option<i64> {
    let qualified = if kind == DatabaseKind::Postgres {
        match key.split_once('.') {
            Some((schema, name)) => format!("\"{schema}\".\"{name}\""),
            None => format!("\"{key}\""),
        }
    } else {
        format!("`{key}`")
    };
    let result = connector
        .execute(&fluxdb_core::QueryRequest {
            connection_id: request.config.id,
            database: Some(request.target.clone()),
            schema: None,
            text: format!("SELECT count(*) FROM {qualified}"),
            mode: fluxdb_core::QueryMode::All,
            options: fluxdb_core::QueryExecutionOptions {
                continue_on_error: false,
                ..Default::default()
            },
            session_id: None,
        })
        .ok()?;
    let cell = result
        .results
        .first()?
        .rows
        .first()?
        .values
        .first()?
        .clone();
    match cell {
        CellValue::I64(n) => Some(n),
        CellValue::Text(v) => v.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// 解析逐表决策：为每个源表定默认动作，阻断式校验用户决策，并附元数据风险。
/// 返回 UI 回显用的表清单。
fn resolve_table_decisions(
    kind: DatabaseKind,
    request: &RestoreRequest,
    parts: &[TablePart],
    stmts: &[PStmt],
    warnings: &mut Vec<String>,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<Vec<RestoreTableInfo>> {
    canceled(cancel)?;
    let connector = connector_for(&request.config)?;
    let existing = target_table_keys(kind, request, &*connector)?;
    let by_key: std::collections::BTreeMap<&str, &TablePart> =
        parts.iter().map(|p| (p.key.as_str(), p)).collect();
    // 决策里出现了不在文件中的表名 → 报错而非静默。
    for d in &request.table_decisions {
        if !by_key.contains_key(d.table.as_str()) {
            return Err(task_error(format!("备份文件中不存在表 {}", d.table)));
        }
    }
    let choice: std::collections::BTreeMap<&str, RestoreTableAction> = request
        .table_decisions
        .iter()
        .map(|d| (d.table.as_str(), d.action))
        .collect();

    // 第一遍：确定每表动作并做离线（存在性/DDL/数据）阻断校验。
    struct Row<'a> {
        part: &'a TablePart,
        action: RestoreTableAction,
        exists: bool,
        has_ddl: bool,
        has_data: bool,
    }
    let mut rows: Vec<Row> = Vec::new();
    for part in parts {
        let has_ddl = part.has_ddl_drop || part.has_ddl_create;
        let has_data = part.has_data;
        let exists = existing.contains(&part.key);
        let action = choice
            .get(part.key.as_str())
            .copied()
            .unwrap_or_else(|| default_action(exists, has_ddl, has_data));
        // 阻断式校验（设计文档 §6.4：不再把“覆盖”静默退化为“追加”）。
        match action {
            RestoreTableAction::Create => {
                if exists {
                    return Err(task_error(format!(
                        "表 {} 在目标库已存在，不能新建；请改用重建/清空后导入/追加",
                        part.name
                    )));
                }
                if !has_ddl {
                    return Err(task_error(format!(
                        "表 {} 备份不含结构，无法新建",
                        part.name
                    )));
                }
            }
            RestoreTableAction::Recreate => {
                if !has_ddl {
                    return Err(task_error(format!(
                        "表 {} 备份不含结构，无法重建",
                        part.name
                    )));
                }
            }
            RestoreTableAction::TruncateAndLoad => {
                if !exists {
                    return Err(task_error(format!(
                        "表 {} 在目标库不存在，无法清空后导入；请改用新建",
                        part.name
                    )));
                }
                if !has_data {
                    return Err(task_error(format!(
                        "表 {} 备份不含数据，清空后导入无意义",
                        part.name
                    )));
                }
            }
            RestoreTableAction::Append => {
                if !exists {
                    return Err(task_error(format!(
                        "表 {} 在目标库不存在，无法追加；请改用新建",
                        part.name
                    )));
                }
                if !has_data {
                    warnings.push(format!("表 {} 备份不含数据，追加为空操作", part.name));
                }
            }
            RestoreTableAction::Skip => {}
        }
        rows.push(Row { part, action, exists, has_ddl, has_data });
    }

    // 第二遍：仅对保留目标结构的动作查询目标元数据（外键/唯一键/列）。
    let need_meta: std::collections::BTreeSet<String> = rows
        .iter()
        .filter(|r| {
            matches!(
                r.action,
                RestoreTableAction::Append | RestoreTableAction::TruncateAndLoad
            )
        })
        .map(|r| r.part.key.clone())
        .collect();
    let checks = collect_table_checks(kind, request, &*connector, parts, &need_meta);

    let mut out = Vec::new();
    let mut any_keep = false;
    for row in &rows {
        let table_checks = checks.get(&row.part.key);
        // 阻断：清空后导入且被其它表外键引用 → 拒绝（设计文档 §15.2）。
        if row.action == RestoreTableAction::TruncateAndLoad {
            if let Some(reason) = truncate_block_reason(table_checks) {
                return Err(task_error(format!("表 {}：{reason}", row.part.name)));
            }
        }
        let source_cols = source_columns(kind, stmts, &row.part.key);
        let risks = risks_for(row.action, table_checks, source_cols.as_ref());
        if matches!(
            row.action,
            RestoreTableAction::Create
                | RestoreTableAction::Recreate
                | RestoreTableAction::TruncateAndLoad
                | RestoreTableAction::Append
        ) {
            any_keep = true;
        }
        out.push(RestoreTableInfo {
            name: row.part.name.clone(),
            exists_in_target: row.exists,
            has_data: row.has_data,
            has_ddl: row.has_ddl,
            default_action: row.action,
            risks,
        });
    }
    if !any_keep {
        return Err(task_error("未选择任何需要恢复的表"));
    }
    Ok(out)
}

/// 计算恢复后仍具有可用结构的表键集合，供 reconstruct_sql 的 FK 修剪判断。
/// Create/Recreate 会重放 DDL；Append/TruncateAndLoad 保留目标既有结构（仅当目标已存在）。
fn structure_keys(
    parts: &[TablePart],
    decisions: &[PerTableDecision],
    existing: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<String> {
    let choice: std::collections::BTreeMap<&str, RestoreTableAction> = decisions
        .iter()
        .map(|d| (d.table.as_str(), d.action))
        .collect();
    let mut keys = std::collections::BTreeSet::new();
    for part in parts {
        let exists = existing.contains(&part.key);
        let has_ddl = part.has_ddl_drop || part.has_ddl_create;
        let action = choice.get(part.key.as_str()).copied().unwrap_or_else(|| {
            default_action(exists, has_ddl, part.has_data)
        });
        match action {
            RestoreTableAction::Create | RestoreTableAction::Recreate => {
                keys.insert(part.key.clone());
            }
            RestoreTableAction::Append | RestoreTableAction::TruncateAndLoad if exists => {
                keys.insert(part.key.clone());
            }
            _ => {}
        }
    }
    keys
}

fn restore_database(
    kind: DatabaseKind,
    request: &RestoreRequest,
    plan: &RestorePlan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<RestoreOutcome> {
    report(progress, "预检查", "重新验证文件和目标状态");
    let current = inspect_restore(kind, request, cancel)?;
    if current.source_size != plan.source_size
        || current.source_modified != plan.source_modified
        || current.format != plan.format
    {
        return Err(task_error("备份在确认后发生变化，请重新预检查"));
    }
    canceled(cancel)?;
    if kind == DatabaseKind::Sqlite {
        report(progress, "恢复", "正在写入新 SQLite 数据库文件");
        let mut source = fs::File::open(&request.source).map_err(io_error)?;
        let mut target = new_output(Path::new(&request.target))?;
        let mut buffer = [0; 64 * 1024];
        loop {
            canceled(cancel)?;
            let n = source.read(&mut buffer).map_err(io_error)?;
            if n == 0 {
                break;
            }
            target.write_all(&buffer[..n]).map_err(io_error)?;
        }
        target.sync_all().map_err(io_error)?;
        drop(target);
        report(progress, "验证", "检查 SQLite 完整性与外键");
        validate_sqlite(Path::new(&request.target))?;
        let objects: Vec<RestoreObjectResult> = request
            .manifest
            .as_ref()
            .map(|m| {
                m.objects
                    .iter()
                    .map(|name| RestoreObjectResult {
                        name: name.clone(),
                        action: None,
                        status: RestoreObjectStatus::Succeeded,
                        detail: String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let total_objects = objects.len();
        return Ok(RestoreOutcome {
            verification: "SQLite integrity_check 与 foreign_key_check 均通过；未逐表核对行数"
                .into(),
            total_objects,
            warnings: Vec::new(),
            objects,
        });
    }
    let (host, port, user, password, tunnel) = native_connection(&request.config)?;
    let mut command = Command::new(&request.tool);
    if kind == DatabaseKind::Postgres {
        let config = resolved(&request.config);
        let profile = config.postgres_profile.clone().unwrap_or_else(|| {
            fluxdb_core::PostgresConnectionProfile::from_options(&config.options)
        });
        let tls_paths = pg_native_tls_paths(&profile.tls);
        let invocation = pg_psql_invocation(
            &host,
            port,
            &user,
            &request.target,
            "-",
            Some(&password),
            true,
            profile.tls.ssl_mode,
            &tls_paths,
        );
        command
            .args(invocation.args)
            .envs(invocation.env)
            .arg("--no-password");
        // 用户选择单事务时，psql 原生支持整任务原子回滚。
        if request.options.transaction == RestoreTransactionMode::SingleTransaction {
            command.arg("--single-transaction");
        }
        postgres_env(&mut command, &config, tunnel.is_some())?;
    } else {
        let mut version_cmd = Command::new(&request.tool);
        version_cmd.arg("--version");
        let version = fluxdb_core::no_console(version_cmd).output().map_err(io_error)?;
        let version = mysql_client_version(&String::from_utf8_lossy(&version.stdout))
            .ok_or_else(|| task_error("无法识别 MySQL 恢复客户端版本"))?;
        command.args([
            "--no-defaults",
            "--batch",
            "--binary-mode",
            "--skip-force",
            "--local-infile=0",
            "--protocol=tcp",
            "--default-character-set=utf8mb4",
        ]);
        mysql_tls(
            &mut command,
            &request.config,
            version.mariadb,
            tunnel.is_some(),
        )?;
        command
            .arg("-h")
            .arg(if tunnel.is_some() { "127.0.0.1" } else { &host })
            .arg("-P")
            .arg(port.to_string())
            .arg("-u")
            .arg(user)
            .arg("--")
            .arg(&request.target)
            .env("MYSQL_PWD", password);
    }
    // 工具/传输准备完毕后才创建目标库；创建失败不运行导入。
    if request.create_target {
        report(progress, "目标", "正在创建新数据库");
        connector_for(&request.config)?.create_database(&CreateDatabaseRequest {
            connection_id: request.config.id,
            name: request.target.clone(),
            charset: if kind == DatabaseKind::Postgres {
                "UTF8"
            } else {
                "utf8mb4"
            }
            .into(),
            // MySQL 建库必须指定 collation；utf8mb4 全版本通用的默认排序规则，
            // 各表自身的 COLLATE 由备份 SQL 逐表指定，不受此默认影响。
            collation: if kind == DatabaseKind::Postgres {
                String::new()
            } else {
                "utf8mb4_general_ci".into()
            },
            owner: String::new(),
            template: String::new(),
            path: None,
        })?;
    }
    canceled(cancel)?;
    // 逐表模式：按决策重建过滤后的 SQL 写入临时文件再喂客户端；整库模式直接用原文件。
    let _temp_sql_path = if !request.table_decisions.is_empty() {
        let (parts, stmts) = partition_statements(&request.source, kind, cancel)?;
        let conn = connector_for(&request.config)?;
        let existing = target_table_keys(kind, request, &*conn)?;
        let skeys = structure_keys(&parts, &request.table_decisions, &existing);
        let (sql, fk_warnings) = reconstruct_sql(kind, &stmts, &request.table_decisions, &skeys);
        for w in fk_warnings {
            report(progress, "恢复", &format!("提示：{w}"));
        }
        // 单事务仅在安全时启用：无建库/无 DDL（DDL 隐式提交），否则保持引擎默认逐语句提交。
        let sql = if request.options.transaction == RestoreTransactionMode::SingleTransaction
            && kind != DatabaseKind::Postgres
            && mysql_single_transaction_ok(request, &current.tables)
        {
            report(progress, "恢复", "以单事务导入数据（失败整体回滚）");
            format!("START TRANSACTION;\n{sql}\nCOMMIT;\n")
        } else {
            sql
        };
        let temp = std::env::temp_dir().join(format!(
            "fluxdb-restore-{}-{}.sql",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::write(&temp, sql).map_err(io_error)?;
        Some(temp)
    } else {
        None
    };
    command
        .stdin(if let Some(ref p) = _temp_sql_path {
            Stdio::from(fs::File::open(p).map_err(io_error)?)
        } else {
            Stdio::from(fs::File::open(&request.source).map_err(io_error)?)
        })
        .stdout(Stdio::null());
    report(progress, "恢复", "正在导入 SQL，遇错停止");
    run_client(command, cancel, progress)?;
    if let Some(ref p) = _temp_sql_path {
        let _ = fs::remove_file(p);
    }
    report(progress, "验证", "验证恢复后的目标对象");
    let connector = connector_for(&request.config)?;
    let root = ObjectPath {
        connection_id: request.config.id,
        database: Some(request.target.clone()),
        schema: None,
        name: String::new(),
        kind: ObjectKind::Schema,
    };
    let objects = if kind == DatabaseKind::Postgres {
        let database = ObjectPath {
            kind: ObjectKind::Database,
            ..root.clone()
        };
        let mut objects = Vec::new();
        for schema in connector.list_objects(Some(&database))? {
            objects.extend(connector.list_objects(Some(&schema.path))?);
        }
        objects
    } else {
        connector.list_objects(Some(&root))?
    };
    // 逐表模式下：跳过（Skip）的表不在恢复范围，验证时剔除。
    let skipped: std::collections::BTreeSet<&str> = request
        .table_decisions
        .iter()
        .filter(|d| d.action == RestoreTableAction::Skip)
        .map(|d| d.table.as_str())
        .collect();
    let kept = |name: &str| !skipped.contains(name);
    if let Some(meta) = &request.manifest {
        for name in &meta.objects {
            if !kept(name) {
                continue;
            }
            if !objects.iter().any(|o| {
                &o.path.name == name
                    || format!(
                        "{}.{}",
                        o.path.schema.as_deref().unwrap_or("public"),
                        o.path.name
                    ) == *name
            }) {
                return Err(task_error(format!(
                    "SQL 执行完成，但未找到预期对象 {name}；验证失败"
                )));
            }
        }
    }
    // 逐对象结果（设计文档 §15.3）：逐表模式取解析后的动作与风险；整库模式取清单对象。
    let mut object_results: Vec<RestoreObjectResult> = if !current.tables.is_empty() {
        current
            .tables
            .iter()
            .map(|info| {
                let status = if info.default_action == RestoreTableAction::Skip {
                    RestoreObjectStatus::Skipped
                } else if info.risks.is_empty() {
                    RestoreObjectStatus::Succeeded
                } else {
                    RestoreObjectStatus::Warning
                };
                RestoreObjectResult {
                    name: info.name.clone(),
                    action: Some(info.default_action),
                    status,
                    detail: info.risks.join("；"),
                }
            })
            .collect()
    } else {
        request
            .manifest
            .as_ref()
            .map(|m| {
                m.objects
                    .iter()
                    .filter(|name| kept(name))
                    .map(|name| RestoreObjectResult {
                        name: name.clone(),
                        action: None,
                        status: RestoreObjectStatus::Succeeded,
                        detail: String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let total_objects = object_results.len();
    // 完成验证=逐表行数：对已恢复对象补充 count(*)，跳过（Skip）不核对；查询失败不阻断。
    let row_count_checked = if request.options.validation == RestoreValidation::RowCount
        && kind != DatabaseKind::Sqlite
    {
        report(progress, "验证", "逐表核对行数（可能较慢）");
        let mut checked = false;
        for item in object_results.iter_mut() {
            if item.status == RestoreObjectStatus::Skipped {
                continue;
            }
            if let Some(n) = count_target_rows(kind, request, &*connector, &item.name) {
                checked = true;
                let suffix = format!("{n} 行");
                item.detail = if item.detail.is_empty() {
                    suffix
                } else {
                    format!("{}；{suffix}", item.detail)
                };
            }
        }
        checked
    } else {
        false
    };
    let verification = if row_count_checked {
        format!(
            "SQL 执行成功，目标可访问，枚举到 {} 个对象，并已逐表核对行数（未核对序列值或全部约束）。",
            objects.len()
        )
    } else {
        format!(
            "SQL 执行成功，目标可访问，枚举到 {} 个对象。未核对行数、序列值或全部约束。",
            objects.len()
        )
    };
    Ok(RestoreOutcome {
        verification,
        total_objects,
        warnings: current.warnings.clone(),
        objects: object_results,
    })
}
