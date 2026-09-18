pub use fluxdb_connectors::normalize_backup_file_name;
pub use fluxdb_core::{
    BackupExecution, BackupFormat, BackupManifest, BackupMethod, BackupObjectKind, BackupObjectRef,
    BackupRequest, BackupScope, DatabaseTaskProgress, RestoreObjectProbe, RestoreObjectResult,
    RestoreObjectStatus, RestoreOptions, RestoreOutcome, RestorePlan, RestoreRequest,
    RestoreTableAction, RestoreTransactionMode, RestoreValidation,
};

impl AppController {
    /// 供后台任务调用的命令入口，保留取消和进度通道；不能从 UI 渲染线程同步调用。
    pub fn dispatch_database_task(
        &self,
        command: AppCommand,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> AppEvent {
        let result = match command {
            AppCommand::PrepareBackup { request, method } => self
                .prepare_backup(request, method)
                .map(AppEvent::BackupPrepared),
            AppCommand::RunBackup(request) => self
                .execute_backup(&request, cancel, progress)
                .map(AppEvent::BackupCompleted),
            AppCommand::PrepareRestore(request) => self
                .prepare_restore(request, cancel)
                .map(|(request, plan)| AppEvent::RestorePrepared { request, plan }),
            AppCommand::ProbeRestore(request) => self
                .probe_restore(request, cancel)
                .map(AppEvent::RestoreObjectsProbed),
            AppCommand::RunRestore { request, plan } => self
                .execute_restore(&request, &plan, cancel, progress)
                .map(AppEvent::RestoreCompleted),
            _ => Err(Error::new(ErrorKind::Internal, "不是备份恢复命令")),
        };
        result.unwrap_or_else(|error| AppEvent::Failed(error.into()))
    }

    /// 解析实际执行器及工具；调用方必须在后台运行（工具版本探测可能启动进程）。
    pub fn prepare_backup(
        &self,
        mut request: BackupRequest,
        method: BackupMethod,
    ) -> fluxdb_core::Result<BackupRequest> {
        // SQLite 只有完整快照一种能力；范围在准备阶段收敛，避免 UI 传入无效选择。
        if request.config.kind == DatabaseKind::Sqlite {
            request.scope = BackupScope::SqliteSnapshot;
        } else if let BackupScope::Objects(objects) = &request.scope {
            if objects.is_empty() {
                return Err(Error::new(ErrorKind::Unsupported, "请至少选择一个备份对象"));
            }
        }
        let provider = fluxdb_connectors::database_backup(request.config.kind)?;
        let settings = &self.state.settings;
        let mysql = if matches!(
            request.config.kind,
            DatabaseKind::MySql | DatabaseKind::TiDb
        ) && method != BackupMethod::Logical
        {
            resolve_mysql_client_tool(settings, MySqlClientTool::Dump)
        } else {
            None
        };
        request.execution = provider.execution(method, mysql.is_some())?;
        match request.execution {
            BackupExecution::MySqlDump => {
                let client = mysql.ok_or_else(|| {
                    Error::new(ErrorKind::Unsupported, mysql_client_install_hint())
                })?;
                request.tool = client.program;
                request.tool_version = Some(format!(
                    "mysqldump Ver {}.{}{}",
                    client.version.major,
                    client.version.minor,
                    if client.version.mariadb {
                        " MariaDB"
                    } else {
                        ""
                    }
                ));
            }
            BackupExecution::PgDump => {
                let major = pg_server_major_version(&request.config)?;
                let client = resolve_pg_client_tool(settings, PgClientTool::Dump, major)
                    .ok_or_else(|| Error::new(ErrorKind::Unsupported, pg_client_install_hint()))?;
                if !pg_dump_version_compatible(client.major_version, major) {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "pg_dump 版本低于服务端版本",
                    ));
                }
                request.tool = PathBuf::from(client.program);
                request.tool_version = client.major_version.map(|v| format!("PostgreSQL {v}"));
            }
            _ => {}
        }
        let name = request
            .output
            .file_name()
            .ok_or_else(|| Error::new(ErrorKind::Internal, "备份文件名无效"))?
            .to_string_lossy();
        request.output.set_file_name(normalize_backup_file_name(
            name.to_string(),
            request.execution.format(),
        ));
        Ok(request)
    }
    pub fn execute_backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        if let Some(parent) = request.output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        }
        if request.output.exists() {
            return Err(Error::new(ErrorKind::Internal, "备份文件已存在，拒绝覆盖"));
        }
        fluxdb_connectors::database_backup(request.config.kind)?.backup(request, cancel, progress)
    }
    pub fn prepare_restore(
        &self,
        mut request: RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<(RestoreRequest, RestorePlan)> {
        request.tool = match request.config.kind {
            DatabaseKind::MySql | DatabaseKind::TiDb => {
                resolve_mysql_client_tool(&self.state.settings, MySqlClientTool::Console)
                    .map(|t| t.program)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::Unsupported, mysql_client_install_hint())
                    })?
            }
            DatabaseKind::Postgres => PathBuf::from(
                resolve_pg_client_tool(&self.state.settings, PgClientTool::Psql, None)
                    .ok_or_else(|| Error::new(ErrorKind::Unsupported, pg_client_install_hint()))?
                    .program,
            ),
            DatabaseKind::Sqlite => PathBuf::new(),
            _ => return Err(Error::new(ErrorKind::Unsupported, "此数据库尚不支持恢复")),
        };
        let plan = fluxdb_connectors::database_backup(request.config.kind)?
            .inspect_restore(&request, cancel)?;
        Ok((request, plan))
    }
    /// 对象页进入时的只读探测：解析备份内容 + 查询目标存在性，供 UI 预渲染默认动作与合法候选。
    /// 不解析客户端工具、不创建库、不执行 SQL；探测失败不影响用户手动改为整库/新建恢复。
    pub fn probe_restore(
        &self,
        request: RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<Vec<RestoreObjectProbe>> {
        fluxdb_connectors::database_backup(request.config.kind)?
            .probe_restore(&request, cancel)
    }
    pub fn execute_restore(
        &self,
        request: &RestoreRequest,
        plan: &RestorePlan,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<RestoreOutcome> {
        fluxdb_connectors::database_backup(request.config.kind)?
            .restore(request, plan, cancel, progress)
    }
    pub fn record_backup(
        &self,
        storage: &fluxdb_storage::FileStorage,
        record: fluxdb_storage::BackupRecord,
    ) -> fluxdb_core::Result<()> {
        let mut records = storage.load_backup_records()?;
        records.push(record);
        storage.save_backup_records(&records)
    }
    pub fn record_restore(
        &self,
        storage: &fluxdb_storage::FileStorage,
        record: fluxdb_storage::RestoreRecord,
    ) -> fluxdb_core::Result<()> {
        let mut records = storage.load_restore_records()?;
        records.push(record);
        storage.save_restore_records(&records)
    }
}

#[cfg(test)]
mod backup_restore_app_tests {
    use super::*;
    #[test]
    fn sqlite_application_roundtrip_uses_actual_format() {
        use sqlx::Connection;
        let dir = std::env::temp_dir().join(format!(
            "fluxdb-app-restore-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let source = dir.join("source.db");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut conn = sqlx::SqliteConnection::connect_with(
                &sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&source)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
            sqlx::raw_sql(
                "CREATE TABLE t(id INTEGER, value TEXT); INSERT INTO t VALUES(1,'中文'),(2,NULL);",
            )
            .execute(&mut conn)
            .await
            .unwrap();
        });
        let config = ConnectionConfig {
            id: ConnectionId(42),
            name: "test".into(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: source,
                read_only: false,
            },
            credential_ref: None,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        };
        let controller = AppController::new();
        let request = controller
            .prepare_backup(
                BackupRequest {
                    config: config.clone(),
                    database: "main".into(),
                    output: dir.join("snapshot.sql"),
                    execution: BackupExecution::SqlDump,
                    tool: PathBuf::new(),
                    tool_version: None,
                    scope: BackupScope::All { include_views: true },
                    include_schema: true,
                    include_data: true,
                    include_routines: false,
                    single_transaction: true,
                    lock_tables: false,
                    include_owner: false,
                    include_acl: false,
                },
                BackupMethod::Auto,
            )
            .unwrap();
        assert_eq!(request.output.extension().unwrap(), "db");
        let cancel = AtomicBool::new(false);
        let AppEvent::BackupCompleted(meta) = controller.dispatch_database_task(
            AppCommand::RunBackup(request.clone()),
            &cancel,
            &mut |_| {},
        ) else {
            panic!("备份命令失败");
        };
        let target = dir.join("restored.db");
        let event = controller.dispatch_database_task(
            AppCommand::PrepareRestore(RestoreRequest {
                config,
                source: request.output,
                target: target.to_string_lossy().into_owned(),
                create_target: true,
                tool: PathBuf::new(),
                manifest: Some(meta),
                table_decisions: Vec::new(),
                options: RestoreOptions::default(),
            }),
            &cancel,
            &mut |_| {},
        );
        let AppEvent::RestorePrepared {
            request: restore,
            plan,
        } = event
        else {
            panic!("{event:?}");
        };
        let command = AppCommand::RunRestore {
            request: restore,
            plan,
        };
        let event = controller.dispatch_database_task(command.clone(), &cancel, &mut |_| {});
        assert!(matches!(event, AppEvent::RestoreCompleted(_)), "{event:?}");
        assert!(
            matches!(
                controller.dispatch_database_task(command, &cancel, &mut |_| {}),
                AppEvent::Failed(_)
            ),
            "已有文件必须保留"
        );
        rt.block_on(async {
            let mut conn = sqlx::SqliteConnection::connect_with(
                &sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(target)
                    .read_only(true),
            )
            .await
            .unwrap();
            let values: Vec<(i64, Option<String>)> = sqlx::query_as("SELECT * FROM t ORDER BY id")
                .fetch_all(&mut conn)
                .await
                .unwrap();
            assert_eq!(values, vec![(1, Some("中文".into())), (2, None)]);
        });
        std::fs::remove_dir_all(dir).unwrap();
    }
}
