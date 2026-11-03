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
            AppCommand::ImportTableData {
                object,
                source,
                options,
                column_mapping,
            } => self
                .import_table_data(&object, &source, options, column_mapping, cancel, &mut |event| {
                    progress(DatabaseTaskProgress {
                        stage: event.stage,
                        message: event.message,
                    });
                })
                .map(|report| AppEvent::TableDataImported { object, report }),
            AppCommand::ImportMongoDocuments {
                object,
                source,
                options,
            } => self
                .import_mongo_documents(
                    &object,
                    &source,
                    options,
                    cancel,
                    &mut |event| {
                        progress(DatabaseTaskProgress {
                            stage: event.stage,
                            message: event.message,
                        });
                    },
                )
                .map(|report| AppEvent::MongoDocumentsImported { object, report }),
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
            DatabaseKind::Dameng => PathBuf::new(),
            DatabaseKind::Oracle => PathBuf::new(),
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
        ensure_read_only_allows(&request.config, fluxdb_core::WriteIntent::Mutation)?;
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

impl AppController {
    pub fn import_table_data(
        &self,
        object: &ObjectPath,
        source: &std::path::Path,
        options: fluxdb_core::TableImportOptions,
        column_mapping: Vec<fluxdb_core::TableImportColumnMapping>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(fluxdb_core::TableImportProgress),
    ) -> fluxdb_core::Result<fluxdb_core::TableImportReport> {
        let config = self
            .connection_config(object.connection_id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        ensure_read_only_allows(&config, fluxdb_core::WriteIntent::Mutation)?;
        let format = fluxdb_core::TableImportFormat::from_path(source).ok_or_else(|| {
            Error::new(
                ErrorKind::Unsupported,
                "导入文件格式必须是 TXT、CSV、JSON 或 XML",
            )
        })?;
        let provider = fluxdb_connectors::table_data_import(config.kind)?;
        provider.import(
            &fluxdb_core::TableImportRequest {
                config,
                object: object.clone(),
                source: source.to_path_buf(),
                format,
                options,
                column_mapping,
            },
            cancel,
            progress,
        )
    }

    /// 解析导入样本并返回目标表字段，供导入对话框预览；不写数据库。
    pub fn preview_table_import(
        &self,
        object: &ObjectPath,
        source: &std::path::Path,
        options: fluxdb_core::TableImportOptions,
    ) -> fluxdb_core::Result<fluxdb_core::TableImportPreview> {
        let config = self
            .connection_config(object.connection_id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let format = fluxdb_core::TableImportFormat::from_path(source).ok_or_else(|| {
            Error::new(
                ErrorKind::Unsupported,
                "导入文件格式必须是 TXT、CSV、JSON 或 XML",
            )
        })?;
        fluxdb_connectors::table_data_import(config.kind)?.preview(
            &fluxdb_core::TableImportPreviewRequest {
                config,
                object: object.clone(),
                source: source.to_path_buf(),
                format,
                options,
            },
        )
    }

    /// 解析 MongoDB 导入文件样本（格式、字段与类型推断），不写数据库。
    ///
    /// `format` 由连接器按内容嗅探，因此调用方只需给文件与解析选项。
    pub fn preview_mongo_import(
        &self,
        object: &ObjectPath,
        source: &std::path::Path,
        options: MongoImportOptions,
    ) -> fluxdb_core::Result<MongoImportPreview> {
        let config = self.mongo_connection_config(object)?;
        connector_for(&config)?.mongo_preview_import(&fluxdb_core::MongoImportPreviewRequest {
            object: object.clone(),
            source: source.to_path_buf(),
            options,
        })
    }

    /// 把文件里的文档批量写入集合。坏文档跳过并计入报告，不中断整批。
    pub fn import_mongo_documents(
        &self,
        object: &ObjectPath,
        source: &std::path::Path,
        options: MongoImportOptions,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(MongoImportProgress),
    ) -> fluxdb_core::Result<MongoImportReport> {
        let config = self.mongo_connection_config(object)?;
        ensure_read_only_allows(&config, fluxdb_core::WriteIntent::Mutation)?;
        connector_for(&config)?.mongo_import_documents(
            &fluxdb_core::MongoImportRequest {
                object: object.clone(),
                source: source.to_path_buf(),
                options,
            },
            cancel,
            progress,
        )
    }

    /// 导入只对 MongoDB 连接有意义：别的连接类型没有「文档」这个概念。
    fn mongo_connection_config(
        &self,
        object: &ObjectPath,
    ) -> fluxdb_core::Result<ConnectionConfig> {
        let config = self
            .connection_config(object.connection_id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        if config.kind != DatabaseKind::MongoDb {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "目标连接不是 MongoDB，无法导入文档",
            ));
        }
        Ok(config)
    }
}

