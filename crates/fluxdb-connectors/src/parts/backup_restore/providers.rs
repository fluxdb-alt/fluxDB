pub fn database_backup(kind: DatabaseKind) -> fluxdb_core::Result<Box<dyn DatabaseBackup>> {
    match kind {
        DatabaseKind::MySql => Ok(Box::new(MySqlBackup)),
        DatabaseKind::TiDb => Ok(Box::new(TiDbBackup { mysql: MySqlBackup })),
        DatabaseKind::Postgres => Ok(Box::new(PostgresBackup)),
        DatabaseKind::Sqlite => Ok(Box::new(SqliteBackup)),
        _ => Err(task_error("该数据库尚不支持备份与恢复")),
    }
}
struct MySqlBackup;
struct TiDbBackup {
    mysql: MySqlBackup,
}
struct PostgresBackup;
struct SqliteBackup;
/// 查询服务端主版本；探测失败时返回 Unknown，由调用方决定兼容策略。
fn mysql_server_major(config: &ConnectionConfig) -> fluxdb_core::Result<u32> {
    let connector = connector_for(config)?;
    let request = fluxdb_core::QueryRequest {
        connection_id: config.id,
        database: None,
        schema: None,
        text: "SELECT VERSION()".into(),
        mode: fluxdb_core::QueryMode::All,
        options: fluxdb_core::QueryExecutionOptions::default(),
        session_id: None,
    };
    let result = connector.execute(&request)?;
    let value = result
        .results
        .first()
        .and_then(|page| page.rows.first())
        .and_then(|row| row.values.first());
    Ok(match value {
        Some(fluxdb_core::CellValue::Text(version)) => version
            .split(['-', ' '])
            .next()
            .and_then(|v| v.split('.').next())
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| task_error("无法识别 MySQL 服务端版本"))?,
        _ => return Err(task_error("无法识别 MySQL 服务端版本")),
    })
}
fn manifest(request: &BackupRequest, objects: Vec<String>) -> BackupManifest {
    BackupManifest {
        kind: Some(request.config.kind),
        execution: Some(request.execution),
        complete: true,
        include_schema: request.include_schema,
        include_data: request.include_data,
        objects,
        tool_version: request.tool_version.clone(),
    }
}
macro_rules! restore_methods {
    () => {
        fn inspect_restore(
            &self,
            request: &RestoreRequest,
            cancel: &AtomicBool,
        ) -> fluxdb_core::Result<RestorePlan> {
            inspect_restore(self.kind(), request, cancel)
        }
        fn probe_restore(
            &self,
            request: &RestoreRequest,
            cancel: &AtomicBool,
        ) -> fluxdb_core::Result<Vec<RestoreObjectProbe>> {
            probe_restore(self.kind(), request, cancel)
        }
        fn restore(
            &self,
            request: &RestoreRequest,
            plan: &RestorePlan,
            cancel: &AtomicBool,
            progress: &mut dyn FnMut(DatabaseTaskProgress),
        ) -> fluxdb_core::Result<RestoreOutcome> {
            restore_database(self.kind(), request, plan, cancel, progress)
        }
    };
}
impl DatabaseBackup for MySqlBackup {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MySql
    }
    fn execution(
        &self,
        method: BackupMethod,
        native: bool,
    ) -> fluxdb_core::Result<BackupExecution> {
        Ok(
            if method == BackupMethod::Logical || (method == BackupMethod::Auto && !native) {
                BackupExecution::SqlDump
            } else {
                BackupExecution::MySqlDump
            },
        )
    }
    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        if request.execution == BackupExecution::SqlDump {
            return logical_backup(request, cancel, progress);
        }
        let version = request
            .tool_version
            .as_deref()
            .and_then(mysql_client_version)
            .ok_or_else(|| task_error("无法识别 mysqldump 版本"))?;
        let mut request = request.clone();
        // mysqldump 9.x 会在例程阶段查询 MySQL 9 的 INFORMATION_SCHEMA.LIBRARIES；
        // 老服务端没有该表。这里只禁用例程并明示，不让备份被标成失败后残留半成品。
        if request.include_routines && !version.mariadb && mysql_server_major(&request.config)? < 9 {
            request.include_routines = false;
            report(
                progress,
                "范围",
                "mysqldump 客户端版本高于 MySQL 服务端，为避开客户端例程查询不兼容，本次不导出存储过程/函数",
            );
        }
        // 执行前把动态范围解析成固定快照；工具参数与 manifest 使用同一份结果。
        let resolved = resolve_backup_scope(&request)?;
        let (host, port, user, password, tunnel) = native_connection(&request.config)?;
        let invocation = mysql_dump_invocation(
            &request.tool.to_string_lossy(),
            &version,
            if tunnel.is_some() { "127.0.0.1" } else { &host },
            port,
            &user,
            &password,
            &request.database,
            &resolved.keys,
            MySqlDumpOptions {
                include_schema: request.include_schema,
                include_data: request.include_data,
                include_routines: request.include_routines,
                single_transaction: request.single_transaction,
                lock_tables: request.lock_tables,
            },
        );
        let mut command = Command::new(&request.tool);
        command.arg("--no-defaults");
        mysql_tls(
            &mut command,
            &request.config,
            version.mariadb,
            tunnel.is_some(),
        )?;
        command
            .args(invocation.args)
            .envs(invocation.env)
            .stdin(Stdio::null())
            .stdout(new_output(&request.output)?);
        report(progress, "备份", "正在执行 MySQL SQL 转储");
        run_client(command, cancel, progress)?;
        Ok(manifest(&request, resolved.keys))
    }
    restore_methods!();
}
impl DatabaseBackup for TiDbBackup {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::TiDb
    }
    fn execution(
        &self,
        method: BackupMethod,
        native: bool,
    ) -> fluxdb_core::Result<BackupExecution> {
        self.mysql.execution(method, native)
    }
    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        // TiDB 组合复用 MySQL 实现；其对象能力独立校验，不复制协议逻辑。
        if request.include_routines {
            report(
                progress,
                "范围",
                "TiDB 不支持 MySQL 存储过程，本次不导出存储过程",
            );
        }
        let mut request = request.clone();
        request.include_routines = false;
        self.mysql.backup(&request, cancel, progress)
    }
    restore_methods!();
}
impl DatabaseBackup for PostgresBackup {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Postgres
    }
    fn execution(&self, _: BackupMethod, _: bool) -> fluxdb_core::Result<BackupExecution> {
        Ok(BackupExecution::PgDump)
    }
    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        // 执行前把动态范围解析成固定快照；-t 参数使用 schema 限定键，避免跨 schema 误配。
        let snapshot = resolve_backup_scope(request)?;
        let (host, port, user, password, tunnel) = native_connection(&request.config)?;
        let config = resolved(&request.config);
        let profile = config.postgres_profile.clone().unwrap_or_else(|| {
            fluxdb_core::PostgresConnectionProfile::from_options(&config.options)
        });
        let scope = match (request.include_schema, request.include_data) {
            (true, false) => PgDumpScope::SchemaOnly,
            (false, true) => PgDumpScope::DataOnly,
            _ => PgDumpScope::Full,
        };
        let tls_paths = pg_native_tls_paths(&profile.tls);
        let invocation = pg_dump_invocation(
            &request.tool.to_string_lossy(),
            &host,
            port,
            &user,
            &request.database,
            Some(&password),
            profile.tls.ssl_mode,
            &tls_paths,
            scope,
            request.include_owner,
            request.include_acl,
            &snapshot.keys,
        );
        let mut command = Command::new(&request.tool);
        command.args(invocation.args).envs(invocation.env);
        postgres_env(&mut command, &config, tunnel.is_some())?;
        command
            .stdin(Stdio::null())
            .stdout(new_output(&request.output)?);
        report(progress, "备份", "正在执行 PostgreSQL plain SQL 转储");
        run_client(command, cancel, progress)?;
        Ok(manifest(request, snapshot.keys))
    }
    restore_methods!();
}
impl DatabaseBackup for SqliteBackup {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Sqlite
    }
    fn execution(&self, _: BackupMethod, _: bool) -> fluxdb_core::Result<BackupExecution> {
        Ok(BackupExecution::SqliteBinary)
    }
    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        if !matches!(request.scope, BackupScope::SqliteSnapshot) {
            return Err(task_error("SQLite 只支持完整快照备份，不能按对象选择"));
        }
        if !request.include_data || !request.include_schema {
            return Err(task_error("SQLite 二进制快照必须同时包含结构和数据"));
        }
        let Endpoint::SqliteFile { path, .. } = &request.config.endpoint else {
            return Err(task_error("SQLite 文件路径缺失"));
        };
        // VACUUM INTO 通过 SQLite 引擎生成一致快照；不依赖外部 sqlite3，也不会丢失 WAL 中已提交数据。
        canceled(cancel)?;
        if request.output.exists() {
            return Err(task_error("目标备份文件已存在"));
        }
        let destination = request
            .output
            .to_str()
            .ok_or_else(|| task_error("SQLite 目标路径不是有效 UTF-8"))?
            .to_owned();
        report(progress, "备份", "SQLite 正在创建完整一致快照");
        sqlite_runtime(path, |connection| {
            Box::pin(async move {
                sqlx::query("VACUUM INTO ?")
                    .bind(destination)
                    .execute(connection)
                    .await
                    .map_err(io_error)?;
                Ok(())
            })
        })?;
        canceled(cancel)?;
        validate_sqlite(&request.output)?;
        let meta = manifest(request, Vec::new());
        Ok(meta)
    }
    restore_methods!();
}
fn new_output(path: &Path) -> fluxdb_core::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error)
}
fn sqlite_runtime<T>(
    path: &Path,
    action: impl for<'a> FnOnce(
        &'a mut sqlx::SqliteConnection,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = fluxdb_core::Result<T>> + 'a>,
    >,
) -> fluxdb_core::Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io_error)?;
    runtime.block_on(async {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false);
        let mut connection = sqlx::SqliteConnection::connect_with(&options)
            .await
            .map_err(io_error)?;
        action(&mut connection).await
    })
}
fn validate_sqlite(path: &Path) -> fluxdb_core::Result<()> {
    sqlite_runtime(path, |connection| {
        Box::pin(async move {
            let integrity: Vec<(String,)> = sqlx::query_as("PRAGMA integrity_check")
                .fetch_all(&mut *connection)
                .await
                .map_err(io_error)?;
            if integrity.len() != 1 || integrity[0].0 != "ok" {
                return Err(task_error("SQLite 完整性检查失败"));
            }
            let violations = sqlx::query("PRAGMA foreign_key_check")
                .fetch_optional(connection)
                .await
                .map_err(io_error)?;
            if violations.is_some() {
                return Err(task_error("SQLite 外键检查失败"));
            }
            Ok(())
        })
    })
}
