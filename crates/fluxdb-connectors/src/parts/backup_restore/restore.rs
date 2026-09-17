include!("logical.rs");
include!("script.rs");
include!("partition.rs");
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
            return Err(task_error("第一版不支持仅数据备份；需要已有兼容结构"));
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
        let version = Command::new(&request.tool)
            .arg("--version")
            .output()
            .map_err(io_error)?;
        if !version.status.success() {
            return Err(task_error("恢复客户端版本检查失败"));
        }
        warnings.push("只执行受支持的静态 SQL 转储；不支持任意脚本、存储过程或动态 SQL。源版本兼容性需要用户确认。".into());
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

/// 解析逐表决策：为每个源表定默认动作，并校验用户决策的合法性。返回 UI 回显用的表清单。
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
    let mut out = Vec::new();
    let mut any_keep = false;
    // 用户决策按表名建立覆盖映射。
    let choice: std::collections::BTreeMap<&str, RestoreTableAction> = request
        .table_decisions
        .iter()
        .map(|d| (d.table.as_str(), d.action))
        .collect();

    for part in parts {
        let has_ddl = part.has_ddl_drop || part.has_ddl_create;
        let has_data = part.has_data;
        let exists = existing.contains(&part.key);
        let action = choice.get(&part.key.as_str()).copied().unwrap_or_else(|| {
            if exists {
                if has_ddl {
                    RestoreTableAction::Overwrite
                } else {
                    RestoreTableAction::Append
                }
            } else if has_ddl {
                RestoreTableAction::Overwrite
            } else {
                RestoreTableAction::Skip
            }
        });
        // 校验
        match action {
            RestoreTableAction::Overwrite if !has_ddl => {
                warnings.push(format!(
                    "表 {} 无结构语句，覆盖退化为仅追加",
                    part.name
                ));
                any_keep = any_keep || has_data;
            }
            RestoreTableAction::Append => {
                if !exists {
                    // 目标无同名表且无 DDL 可建：无法追加。
                    if !has_ddl {
                        return Err(task_error(format!(
                            "表 {} 在目标库不存在且备份不含结构，无法追加",
                            part.name
                        )));
                    }
                    // 有 DDL 但用户仍选追加：警告，目标缺失时追加会失败。
                    warnings.push(format!(
                        "表 {} 在目标库不存在，追加将无法导入；建议改为覆盖",
                        part.name
                    ));
                }
                any_keep = true;
            }
            RestoreTableAction::Overwrite => any_keep = true,
            RestoreTableAction::Skip => {}
        }
        out.push(RestoreTableInfo {
            name: part.name.clone(),
            exists_in_target: exists,
            has_data,
            has_ddl,
            default_action: action,
        });
    }
    if !any_keep {
        return Err(task_error("未选择任何需要恢复的表"));
    }
    // 决策里出现了不在文件中的表名 → 报错而非静默。
    for d in &request.table_decisions {
        if !by_key.contains_key(d.table.as_str()) {
            return Err(task_error(format!("备份文件中不存在表 {}", d.table)));
        }
    }
    let _ = stmts;
    Ok(out)
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
        return Ok(RestoreOutcome {
            verification: "SQLite integrity_check 与 foreign_key_check 均通过；未逐表核对行数"
                .into(),
        });
    }
    let (host, port, user, password, tunnel) = native_connection(&request.config)?;
    let mut command = Command::new(&request.tool);
    if kind == DatabaseKind::Postgres {
        let config = resolved(&request.config);
        let profile = config.postgres_profile.clone().unwrap_or_else(|| {
            fluxdb_core::PostgresConnectionProfile::from_options(&config.options)
        });
        let invocation = pg_psql_invocation(
            &host,
            port,
            &user,
            &request.target,
            "-",
            Some(&password),
            true,
            profile.tls.ssl_mode,
        );
        command
            .args(invocation.args)
            .envs(invocation.env)
            .arg("--no-password");
        postgres_env(&mut command, &config, tunnel.is_some())?;
    } else {
        let version = Command::new(&request.tool)
            .arg("--version")
            .output()
            .map_err(io_error)?;
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
        let (_parts, stmts) = partition_statements(&request.source, kind, cancel)?;
        let (sql, fk_warnings) = reconstruct_sql(&stmts, &request.table_decisions);
        for w in fk_warnings {
            report(progress, "恢复", &format!("提示：{w}"));
        }
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
    Ok(RestoreOutcome {
        verification: format!(
            "SQL 执行成功，目标可访问，枚举到 {} 个对象。未核对行数、序列值或全部约束。",
            objects.len()
        ),
    })
}
