// MongoDB 原生备份/恢复：`mongodump --archive --gzip` 与 `mongorestore`。
//
// 只支持单库整库归档；按集合选择不在本阶段范围内。凭据通过临时 0600 配置文件
// （`uri: ...`）传给工具，绝不进命令行参数或日志；路径本身不敏感，可以出现在 argv。
// SSH 隧道先在本进程开通，再把 URI 改写到本地转发端口。

struct MongoBackup;

impl DatabaseBackup for MongoBackup {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MongoDb
    }

    fn execution(
        &self,
        method: BackupMethod,
        native_available: bool,
    ) -> fluxdb_core::Result<BackupExecution> {
        if method == BackupMethod::Logical {
            return Err(task_error("MongoDB 没有逻辑备份实现，请安装 mongodump"));
        }
        if !native_available {
            return Err(task_error(mongo_client_install_hint_for_connectors()));
        }
        Ok(BackupExecution::MongoDump)
    }

    fn backup(
        &self,
        request: &BackupRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<BackupManifest> {
        if !request.include_schema || !request.include_data {
            return Err(task_error(
                "MongoDB 归档必须同时包含结构和数据，不支持只导出其中一部分",
            ));
        }
        // mongodump 按数据库导出，不能按集合选择；逐对象请求在进入工具前就明确拒绝。
        if !matches!(request.scope, BackupScope::All { .. }) {
            return Err(task_error("MongoDB 备份只支持整库归档，不能按对象选择"));
        }
        if request.tool.as_os_str().is_empty() {
            return Err(task_error(mongo_client_install_hint_for_connectors()));
        }
        let database = request.database.trim();
        if database.is_empty() {
            return Err(task_error("请选择要备份的 MongoDB 数据库"));
        }
        validate_mongo_db_name(database)?;
        let connection = mongo_tool_connection(&request.config)?;
        let mut command = Command::new(&request.tool);
        command
            .arg("--config")
            .arg(connection.config_path())
            .arg("--db")
            .arg(database)
            .arg("--gzip")
            // 不带值的 --archive 写 stdout；用独占创建的输出文件，天然拒绝覆盖。
            .arg("--archive")
            .stdin(Stdio::null())
            .stdout(new_output(&request.output)?);
        report(progress, "备份", "正在执行 MongoDB 归档导出");
        let result = run_client(command, cancel, progress);
        if result.is_err() {
            // 失败/取消都会留下不完整归档，不能把半成品当成可恢复备份。
            let _ = fs::remove_file(&request.output);
        }
        result?;
        Ok(BackupManifest {
            kind: Some(DatabaseKind::MongoDb),
            database: Some(database.to_string()),
            execution: Some(BackupExecution::MongoDump),
            complete: true,
            include_schema: true,
            include_data: true,
            objects: Vec::new(),
            tool_version: request.tool_version.clone(),
            // 未使用 --oplog：跨集合不保证同一时点，如实标记而不是宣称一致快照。
            consistent: Some(false),
            ..Default::default()
        })
    }

    fn inspect_restore(
        &self,
        request: &RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<RestorePlan> {
        canceled(cancel)?;
        if request.config.kind != DatabaseKind::MongoDb {
            return Err(task_error("恢复执行器与目标数据库类型不匹配"));
        }
        if request.tool.as_os_str().is_empty() {
            return Err(task_error(mongo_client_install_hint_for_connectors()));
        }
        let source = fs::metadata(&request.source).map_err(io_error)?;
        if !source.is_file() || source.len() == 0 {
            return Err(task_error("备份必须是非空普通文件"));
        }
        validate_mongo_db_name(&request.target)?;
        let manifest = request
            .manifest
            .as_ref()
            .ok_or_else(|| task_error("MongoDB 归档缺少备份记录，无法确定原始库名和目标映射"))?;
        if manifest.kind.is_some_and(|kind| kind != DatabaseKind::MongoDb) {
            return Err(task_error("源数据库与目标类型不兼容"));
        }
        if !manifest.complete {
            return Err(task_error("备份未完整成功，不能自动恢复"));
        }
        let source_database = manifest
            .database
            .as_deref()
            .ok_or_else(|| task_error("备份记录缺少原始库名，无法把归档恢复到目标库"))?;
        validate_mongo_db_name(source_database)?;
        let mut warnings = vec![
            "MongoDB 归档整库恢复，不能按集合选择；目标已有同名集合会追加写入，不会先清空。"
                .into(),
            "恢复失败或取消可能留下部分集合，任务不会自动删除目标数据。".into(),
        ];
        if source_database != request.target {
            warnings.push(format!(
                "归档原始库 `{source_database}` 将重映射到目标库 `{}`。",
                request.target
            ));
        }
        Ok(RestorePlan {
            format: BackupFormat::MongoArchive,
            summary: format!("MongoDB 归档恢复到 `{}`", request.target),
            warnings,
            source_size: source.len(),
            source_modified: source.modified().ok(),
            tables: Vec::new(),
            per_table: false,
        })
    }

    fn probe_restore(
        &self,
        _request: &RestoreRequest,
        cancel: &AtomicBool,
    ) -> fluxdb_core::Result<Vec<RestoreObjectProbe>> {
        canceled(cancel)?;
        // 归档格式不支持按集合枚举；返回空清单，UI 按整库恢复处理。
        Ok(Vec::new())
    }

    fn restore(
        &self,
        request: &RestoreRequest,
        plan: &RestorePlan,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(DatabaseTaskProgress),
    ) -> fluxdb_core::Result<RestoreOutcome> {
        if plan.format != BackupFormat::MongoArchive {
            return Err(task_error("恢复计划与 MongoDB 归档不匹配"));
        }
        let source_database = request
            .manifest
            .as_ref()
            .and_then(|manifest| manifest.database.as_deref())
            .ok_or_else(|| task_error("备份记录缺少原始库名，无法执行恢复"))?;
        let connection = mongo_tool_connection(&request.config)?;
        // mongorestore 的 --archive 是带值选项，值必须用 `=` 连接。
        let mut archive_arg = std::ffi::OsString::from("--archive=");
        archive_arg.push(&request.source);
        let mut command = Command::new(&request.tool);
        command
            .arg("--config")
            .arg(connection.config_path())
            .arg("--gzip")
            .arg(archive_arg)
            .args(mongo_namespace_args(source_database, &request.target))
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        report(progress, "恢复", "正在执行 MongoDB 归档恢复");
        run_client(command, cancel, progress)?;
        Ok(RestoreOutcome {
            verification: "已执行 mongorestore 归档恢复；未逐集合核对行数。".into(),
            total_objects: 1,
            warnings: plan.warnings.clone(),
            objects: vec![RestoreObjectResult {
                name: request.target.clone(),
                action: None,
                status: RestoreObjectStatus::Succeeded,
                detail: "整库归档恢复完成".into(),
            }],
        })
    }
}

/// 临时凭据文件；Drop 时删除，命令行只出现路径。
struct MongoToolConfig {
    path: PathBuf,
}

impl MongoToolConfig {
    fn config_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MongoToolConfig {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// 凭据文件可能含密码：Unix 上以 0600 创建；Windows 依赖用户临时目录 ACL。
fn write_mongo_tool_config(uri: &str) -> fluxdb_core::Result<MongoToolConfig> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fluxdb-mongo-{}-{sequence}.yml",
        std::process::id()
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(io_error)?;
    // YAML 双引号字符串：URI 已百分号编码，仍需转义反斜杠与引号。
    let escaped = uri.replace('\\', "\\\\").replace('"', "\\\"");
    if let Err(error) = file.write_all(format!("uri: \"{escaped}\"\n").as_bytes()) {
        // 结构尚未返回，Drop 不会执行；这里主动清理失败留下的凭据文件。
        let _ = fs::remove_file(&path);
        return Err(io_error(error));
    }
    Ok(MongoToolConfig { path })
}

/// 工具连接：档案 →（必要时开隧道）→ 临时配置。隧道必须活到命令结束，故一并返回。
struct MongoToolConnection {
    config: MongoToolConfig,
    _tunnel: Option<SshTunnel>,
}

impl MongoToolConnection {
    fn config_path(&self) -> &Path {
        self.config.config_path()
    }
}

fn mongo_tool_connection(config: &ConnectionConfig) -> fluxdb_core::Result<MongoToolConnection> {
    let mut profile = match config.mongo_profile.clone() {
        Some(profile) => profile,
        None => match &config.endpoint {
            Endpoint::Uri { uri } => {
                fluxdb_core::MongoConnectionProfile::from_uri(uri).map_err(task_error)?
            }
            Endpoint::Tcp { host, port, .. } => {
                let mut profile = fluxdb_core::MongoConnectionProfile::from_options(&config.options);
                profile.basic.hosts = vec![fluxdb_core::MongoHost {
                    host: host.clone(),
                    port: *port,
                }];
                profile
            }
            Endpoint::SqliteFile { .. } => {
                return Err(task_error("MongoDB 不支持文件型连接"));
            }
        },
    };
    if let Some(message) = profile.validate() {
        return Err(task_error(message));
    }
    // 备份用了显式 --db；清掉 URI 里的库名，避免工具自动选中另一个库。
    profile.basic.default_database.clear();
    let mut tunnel = None;
    let uri = if profile.ssh.enabled {
        let (host, port) = profile.dial_endpoint();
        let opened = open_tunnel_for_profile(
            &profile.ssh,
            (&host, port),
            profile.advanced.connect_timeout_secs,
        )?;
        let uri = profile.to_tunnel_uri("127.0.0.1", opened.local_port);
        tunnel = Some(opened);
        uri
    } else {
        profile.to_uri()
    };
    Ok(MongoToolConnection {
        config: write_mongo_tool_config(&mongo_tool_uri(uri))?,
        _tunnel: tunnel,
    })
}

/// 清掉库名后 URI 可能直接跟查询串；Database Tools 要求 `?` 前必须有 `/`。
fn mongo_tool_uri(uri: String) -> String {
    let Some(query) = uri.find('?') else {
        return uri;
    };
    let authority_start = uri.find("://").map(|index| index + 3).unwrap_or(0);
    if uri[authority_start..query].contains('/') {
        return uri;
    }
    let mut fixed = uri;
    fixed.insert(query, '/');
    fixed
}

/// 目标库名与归档原始库名重映射参数；同名时不需要任何 ns 参数。
fn mongo_namespace_args(source: &str, target: &str) -> Vec<String> {
    if source == target {
        return Vec::new();
    }
    vec![
        format!("--nsInclude={source}.*"),
        format!("--nsFrom={source}.*"),
        format!("--nsTo={target}.*"),
    ]
}

/// MongoDB 库名限制（不含空字符与文件/URI 保留字符，最长 64 字节）。
fn validate_mongo_db_name(name: &str) -> fluxdb_core::Result<()> {
    let invalid = name.is_empty()
        || name.len() > 64
        || name.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, '/' | '\\' | '.' | '"' | '$' | '*' | '<' | '>' | ':' | '|' | '?')
        });
    if invalid {
        return Err(task_error("MongoDB 数据库名无效"));
    }
    Ok(())
}

/// 连接器层没有 UI 依赖，安装提示在此单独维护；内容与 app 层保持一致。
fn mongo_client_install_hint_for_connectors() -> &'static str {
    "未找到 MongoDB 数据库工具（mongodump/mongorestore），请安装 MongoDB Database Tools 并确保其在 PATH 中"
}

#[cfg(test)]
mod mongo_backup_tests {
    use super::*;

    #[test]
    fn namespace_args_only_rename_when_database_differs() {
        assert!(mongo_namespace_args("app", "app").is_empty());
        assert_eq!(
            mongo_namespace_args("app", "app_copy"),
            vec![
                "--nsInclude=app.*".to_string(),
                "--nsFrom=app.*".to_string(),
                "--nsTo=app_copy.*".to_string(),
            ]
        );
    }

    #[test]
    fn tool_uri_puts_slash_before_query_when_database_is_empty() {
        assert_eq!(
            mongo_tool_uri("mongodb://h:27017?authSource=admin".into()),
            "mongodb://h:27017/?authSource=admin"
        );
        assert_eq!(
            mongo_tool_uri("mongodb://h:27017/app?authSource=admin".into()),
            "mongodb://h:27017/app?authSource=admin"
        );
        assert_eq!(
            mongo_tool_uri("mongodb+srv://cluster.example".into()),
            "mongodb+srv://cluster.example"
        );
    }

    #[test]
    fn database_name_validation_rejects_reserved_characters() {
        assert!(validate_mongo_db_name("app_copy").is_ok());
        for invalid in ["", "a.b", "a/b", "a b", "a$b"] {
            assert!(
                validate_mongo_db_name(invalid).is_err(),
                "应拒绝无效库名 {invalid:?}"
            );
        }
    }
}
