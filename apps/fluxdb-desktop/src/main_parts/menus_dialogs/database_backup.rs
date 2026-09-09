use std::process::{Command, Stdio};

const BACKUP_VISIBLE_LOG_LIMIT: usize = 200;

impl NavicatMain {
    fn show_backup_modal(
        &mut self,
        connection_id: Option<ConnectionId>,
        database: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_backup_modal_preselect(connection_id, database, None, window, cx);
    }

    /// 打开备份对话框。`preselect_table` 传 Some(表名) 时（表右键→备份）对象选择
    /// 仅默认勾选该表；传 None 时（数据库右键 / 再次备份）默认全不选，由用户手动勾选。
    fn show_backup_modal_preselect(
        &mut self,
        connection_id: Option<ConnectionId>,
        database: Option<String>,
        preselect_table: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = connection_id
            .or_else(|| self.current_query_scope().map(|(connection_id, _)| connection_id))
            .or_else(|| self.first_connection_id())
        else {
            self.show_message("请先创建连接", AppMessageKind::Warning, cx);
            return;
        };
        let database = database
            .or_else(|| self.current_query_scope().and_then(|(_, database)| database))
            .or_else(|| {
                self.controller
                    .state()
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == connection_id)
                    .and_then(|connection| connection_default_database(&connection.config))
            });

        let target_dir = self
            .controller
            .state()
            .settings
            .backup_dir
            .trim()
            .to_string();
        // 先借用 database 计算默认全选表集合（同时作为表/视图列表快照），再整体移入数据库字段。
        let all_tables = self.current_backup_tables(connection_id, database.as_deref());
        let all_views = self.current_backup_views(connection_id, database.as_deref());
        // 重新打开时重置对象列表滚动位置，避免残留上次滚动偏移。
        self.backup_objects_scroll
            .scroll_to_item(0, ScrollStrategy::Top);
        // 对象选择默认：表右键预选该表，其它入口全不选（用户手动勾选）。
        // include_views 默认关，确保「全不选」真正为空，开始按钮需先手动勾选才可用。
        let preselected: BTreeSet<String> = preselect_table
            .map(|name| {
                let mut set = BTreeSet::new();
                set.insert(name.to_string());
                set
            })
            .unwrap_or_default();
        self.pending_backup_modal = Some(BackupForm {
            connection_id,
            selected_tables: all_tables
                .intersection(&preselected)
                .cloned()
                .collect(),
            all_table_names: all_tables,
            all_view_names: all_views,
            database,
            mode: BackupMode::Auto,
            target_dir,
            tab: BackupTab::General,
            file_name: String::new(),
            object_search: String::new(),
            include_views: false,
            lock_tables: false,
            single_transaction: true,
            include_routines: true,
            include_schema: true,
            include_data: true,
            note: String::new(),
        });
        self.backup_log_task = None;
        self.backup_file_name_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.backup_object_search_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.backup_note_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// 当前连接+库下所有表名（对象选择页签资源：默认全选）。
    fn current_backup_tables(&self, connection_id: ConnectionId, database: Option<&str>) -> BTreeSet<String> {
        let Some(database) = database else {
            return BTreeSet::new();
        };
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| {
                let names: BTreeSet<String> = group_objects(connection, database, ObjectGroup::Tables)
                    .into_iter()
                    .map(|object| object.path.name.clone())
                    .collect();
                names
            })
            .unwrap_or_default()
    }

    /// 当前连接+库下所有视图名（对象选择页签「视图」分组资源）。
    fn current_backup_views(&self, connection_id: ConnectionId, database: Option<&str>) -> BTreeSet<String> {
        let Some(database) = database else {
            return BTreeSet::new();
        };
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| {
                group_objects(connection, database, ObjectGroup::Views)
                    .into_iter()
                    .map(|object| object.path.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn cancel_backup_modal(&mut self, cx: &mut Context<Self>) {
        self.pending_backup_modal = None;
        self.backup_log_task = None;
        cx.notify();
    }

    fn confirm_backup(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_backup_modal.clone() else {
            return;
        };
        if form.target_dir.trim().is_empty() {
            self.show_message("请先在设置中配置备份目录", AppMessageKind::Warning, cx);
            return;
        }
        let task_id = self.create_backup_task(&form);
        self.pending_backup_modal = None;
        self.backup_log_task = Some(task_id);
        cx.notify();

        let controller = self.controller.clone();
        self._file_picker_task = Some(cx.spawn(async move |view, cx| {
            smol::Timer::after(Duration::from_millis(50)).await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.start_backup(task_id, form, controller, cx);
                });
            });
        }));
    }

    fn create_backup_task(&mut self, form: &BackupForm) -> u64 {
        let task_id = self.backup_task_seq + 1;
        self.backup_task_seq = task_id;
        self.backup_tasks.push(BackupTaskState {
            id: task_id,
            connection_id: form.connection_id,
            database: form.database.clone(),
            mode: form.mode,
            output_path: PathBuf::new(),
            stage: "准备中".to_string(),
            started_at: Instant::now(),
            finished_at: None,
            logs: Vec::new(),
            skipped: Vec::new(),
            error: None,
            cancel_requested: false,
            canceled: false,
        });
        task_id
    }

    fn start_backup(
        &mut self,
        task_id: u64,
        form: BackupForm,
        controller: AppController,
        cx: &mut Context<Self>,
    ) {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self._backup_cancel_flags.insert(task_id, cancel_flag.clone());

        let database = form.database.clone().unwrap_or_default();
        let output_path = backup_output_path(&form);
        if let Some(task) = self.backup_tasks.iter_mut().find(|task| task.id == task_id) {
            task.output_path = output_path.clone();
        }
        let output_path_finish = output_path.clone();

        // 暂存本次备份元数据（表清单/视图开关/备注）；成功后由 backup_finish_on_ui 写入 .meta.json。
        self.backup_pending_metas.insert(
            task_id,
            BackupFileMeta {
                tables: Some(form.selected_tables.iter().cloned().collect()),
                include_views: form.include_views,
                note: form.note.clone(),
            },
        );

        let task = cx.spawn(async move |view, cx| {
            let (sender, receiver) = mpsc::channel::<BackupTaskProgress>();
            let result = cx
                .background_spawn(async move {
                    // 用 catch_unwind 兜底：若 run_backup 内部 panic（如子进程/DLL 异常），
                    // 任务会静默死亡，导致完成信号永不到达、弹框一直停在“正在导出数据…”。
                    // 捕获后转为 Err，保证拿到确定的结束信号并展示“失败”。
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run_backup(controller, form, output_path, cancel_flag, sender)
                    }))
                    .unwrap_or_else(|payload| {
                        let message = if let Some(s) = payload.downcast_ref::<&str>() {
                            (*s).to_string()
                        } else if let Some(s) = payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "未知内部错误".to_string()
                        };
                        Err(anyhow::anyhow!("备份过程内部错误：{message}"))
                    })
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok(progress) = receiver.try_recv() {
                    backup_progress_on_ui(&view, cx, task_id, progress);
                }
                if let Some(result) = result.as_mut().now_or_never() {
                    while let Ok(progress) = receiver.try_recv() {
                        backup_progress_on_ui(&view, cx, task_id, progress);
                    }
                    let result = result.map_err(|error| error.to_string());
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this._backup_tasks.remove(&task_id);
                            this._backup_cancel_flags.remove(&task_id);
                            this.backup_finish_on_ui(task_id, result, database.clone(), output_path_finish.clone(), cx);
                        });
                    });
                    break;
                }
                smol::Timer::after(Duration::from_millis(100)).await;
            }
        });
        self._backup_tasks.insert(task_id, task);
        self.backup_log_task = Some(task_id);
        cx.notify();
    }

    fn cancel_backup_task(&mut self, task_id: u64, cx: &mut Context<Self>) {
        let Some(task) = self.backup_tasks.iter_mut().find(|task| task.id == task_id) else {
            return;
        };
        if !task.running() {
            return;
        }
        task.cancel_requested = true;
        if let Some(flag) = self._backup_cancel_flags.get(&task_id) {
            flag.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    fn clear_backup_task(&mut self, task_id: u64, cx: &mut Context<Self>) {
        if self
            .backup_tasks
            .iter()
            .any(|task| task.id == task_id && task.running())
        {
            self.show_message("任务执行中，暂不能清除", AppMessageKind::Warning, cx);
            return;
        }
        self.backup_tasks.retain(|task| task.id != task_id);
        if self.backup_log_task == Some(task_id) {
            self.backup_log_task = None;
        }
        cx.notify();
    }

    fn backup_finish_on_ui(
        &mut self,
        task_id: u64,
        result: Result<(), String>,
        database: String,
        output_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let (canceled, error) = match result {
            Ok(()) => (false, None),
            Err(message) => (false, Some(message)),
        };
        let task_canceled = self
            .backup_tasks
            .iter()
            .find(|task| task.id == task_id)
            .is_some_and(|task| task.cancel_requested);
        let canceled = canceled || task_canceled;
        if let Some(task) = self.backup_tasks.iter_mut().find(|task| task.id == task_id) {
            task.finished_at = Some(Instant::now());
            task.canceled = canceled;
            task.error = error.clone();
            if error.is_some() {
                task.stage = "失败".to_string();
            } else if canceled {
                task.stage = "已取消".to_string();
            } else {
                task.stage = "完成".to_string();
            }
        }
        // 成功则写旁挂元数据 {文件}.sql.meta.json；失败/取消丢弃，避免残留误导。
        let meta_ok = error.is_none() && !canceled;
        // 失败/取消时备份文件内容不完整（且写入时已截断旧文件），删除残留避免列表出现无效记录。
        if !meta_ok {
            match fs::remove_file(&output_path) {
                Ok(()) => {
                    tracing::info!(path = %output_path.display(), "已清理未完成备份文件");
                }
                Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => {}
                Err(remove_error) => {
                    tracing::warn!(
                        path = %output_path.display(),
                        error = %remove_error,
                        "未完成备份文件清理失败"
                    );
                }
            }
        }
        if let Some(meta) = self.backup_pending_metas.remove(&task_id) {
            if meta_ok {
                if let Err(write_error) = write_backup_meta(&output_path, &meta) {
                    tracing::warn!(
                        path = %output_path.display(),
                        error = %write_error,
                        "备份元数据写入失败"
                    );
                }
            }
        }
        let kind = if canceled {
            AppMessageKind::Info
        } else if error.is_some() {
            AppMessageKind::Error
        } else {
            AppMessageKind::Success
        };
        let message = if canceled {
            format!("备份已取消：{database}")
        } else if let Some(error) = error {
            format!("备份失败：{database}：{error}")
        } else {
            format!("备份完成：{database} → {}", output_path.display())
        };
        self.show_message(message, kind, cx);
        cx.notify();
    }
}

/// 计算最终备份文件名：配合「使用指定的文件名」门控；否则用默认 `库名_时间戳.sql`（保持旧行为）。
fn resolve_backup_file_name(form: &BackupForm) -> String {
    let default = {
        let name = form
            .database
            .clone()
            .unwrap_or_default();
        let name = safe_data_export_filename_segment(&name);
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        format!("{name}_{timestamp}.sql")
    };
    let raw = form.file_name.trim();
    if raw.is_empty() {
        return default;
    }
    // 模板占位替换：{timestamp} / {database}
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let db = safe_data_export_filename_segment(&form.database.clone().unwrap_or_default());
    let mut resolved = raw
        .replace("{timestamp}", &timestamp.to_string())
        .replace("{database}", &db);
    // 净化路径穿越/非法字符
    resolved = resolved
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if !resolved.to_lowercase().ends_with(".sql") {
        resolved.push_str(".sql");
    }
    resolved
}

fn backup_output_path(form: &BackupForm) -> PathBuf {
    let base = form
        .target_dir
        .trim()
        .trim_end_matches('/')
        .trim_end_matches('\\')
        .to_string();
    // 按库建子文件夹：每个库的备份落在 {backup_dir}/{db 安全名}/{文件名}.sql，
    // 供备份菜单按库扫描对应文件夹反推历史（不落 sqlite）。
    let db_safe = safe_data_export_filename_segment(&form.database.clone().unwrap_or_default());
    PathBuf::from(base)
        .join(db_safe)
        .join(resolve_backup_file_name(form))
}

#[derive(Clone, Debug)]
struct BackupTaskProgress {
    stage: String,
    message: String,
    success: bool,
}

fn backup_progress_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    task_id: u64,
    progress: BackupTaskProgress,
) {
    let _ = cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            if let Some(task) = this.backup_tasks.iter_mut().find(|task| task.id == task_id) {
                task.stage = progress.stage.clone();
                task.logs.push(BackupLogEntry {
                    index: task.logs.len() + 1,
                    stage: progress.stage.clone(),
                    success: progress.success,
                    message: progress.message.clone(),
                });
                if task.logs.len() > BACKUP_VISIBLE_LOG_LIMIT {
                    let overflow = task.logs.len() - BACKUP_VISIBLE_LOG_LIMIT;
                    task.logs.drain(..overflow);
                }
                if !progress.success {
                    task.skipped.push(progress.message.clone());
                }
            }
            cx.notify();
        });
    });
}

fn run_backup(
    controller: AppController,
    form: BackupForm,
    output_path: PathBuf,
    cancel_flag: Arc<AtomicBool>,
    sender: mpsc::Sender<BackupTaskProgress>,
) -> anyhow::Result<()> {
    let config = controller
        .connection_configs()
        .into_iter()
        .find(|config| config.id == form.connection_id)
        .ok_or_else(|| anyhow::anyhow!("连接不存在"))?;
    let (host, port, user, password) = resolved_credentials(&config);

    let settings = controller.state().settings.clone();
    let mode = match form.mode {
        BackupMode::Native => BackupMode::Native,
        BackupMode::Logic => BackupMode::Logic,
        BackupMode::Auto => {
            if native_tool_available(&settings, &config.kind) {
                BackupMode::Native
            } else {
                BackupMode::Logic
            }
        }
    };
    let _ = sender.send(BackupTaskProgress {
        stage: "开始".to_string(),
        message: format!(
            "使用 {} 备份数据库：{}",
            match mode {
                BackupMode::Native => "原生工具",
                BackupMode::Logic => "逻辑备份",
                BackupMode::Auto => unreachable!(),
            },
            form.database.clone().unwrap_or_default()
        ),
        success: true,
    });

    // 确保按库子文件夹存在（原生工具直接把备份写到该路径，父目录必须先建好）。
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| anyhow::anyhow!("创建备份目录失败: {err}"))?;
    }

    match mode {
        BackupMode::Native => match config.kind {
            DatabaseKind::MySql | DatabaseKind::TiDb => run_native_mysqldump(
                &config,
                &settings,
                &host,
                port,
                &user,
                &password,
                form.database.as_deref().unwrap_or_default(),
                &output_path,
                &cancel_flag,
                &sender,
                &form,
            ),
            DatabaseKind::Sqlite => {
                run_native_sqlite(&config, &output_path, &cancel_flag, &sender)
            }
            DatabaseKind::MongoDb | DatabaseKind::Redis => {
                anyhow::bail!("当前连接类型不支持原生备份")
            }
        },
        BackupMode::Logic => run_logic_backup(
            &controller,
            &config,
            form.database.as_deref(),
            &output_path,
            &cancel_flag,
            &sender,
            &form,
        ),
        BackupMode::Auto => unreachable!(),
    }
}

fn resolved_credentials(config: &ConnectionConfig) -> (String, u16, String, String) {
    let resolved = config.mysql_resolved();
    let (host, port) = match &resolved.endpoint {
        Endpoint::Tcp { host, port, .. } => (host.clone(), *port),
        Endpoint::SqliteFile { .. } => (String::new(), 0),
        Endpoint::Uri { .. } => (String::new(), 0),
    };
    let user = resolved
        .options
        .get("username")
        .cloned()
        .unwrap_or_else(|| "root".to_string());
    let password = resolved
        .options
        .get("password")
        .cloned()
        .unwrap_or_default();
    (host, port, user, password)
}

fn native_tool_available(settings: &Settings, kind: &DatabaseKind) -> bool {
    let tool = match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            if !settings.mysqldump_path.trim().is_empty() {
                &settings.mysqldump_path
            } else {
                "mysqldump"
            }
        }
        DatabaseKind::Sqlite => {
            if !settings.sqlite3_path.trim().is_empty() {
                &settings.sqlite3_path
            } else {
                "sqlite3"
            }
        }
        _ => return false,
    };
    Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| child.wait())
        .is_ok()
}

fn emit_native_log(
    sender: &mpsc::Sender<BackupTaskProgress>,
    stage: &str,
    message: impl Into<String>,
) -> anyhow::Result<()> {
    let _ = sender.send(BackupTaskProgress {
        stage: stage.to_string(),
        message: message.into(),
        success: true,
    });
    Ok(())
}

fn run_native_mysqldump(
    config: &ConnectionConfig,
    settings: &Settings,
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
    output_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
    sender: &mpsc::Sender<BackupTaskProgress>,
    form: &BackupForm,
) -> anyhow::Result<()> {
    if database.is_empty() {
        anyhow::bail!("未指定数据库，无法进行原生备份");
    }
    let tool = if !settings.mysqldump_path.trim().is_empty() {
        settings.mysqldump_path.as_str()
    } else {
        "mysqldump"
    };
    if cfg!(not(test)) && host.is_empty() {
        anyhow::bail!("MySQL/TiDB 连接缺少主机信息");
    }
    // 数据库 + 可选表过滤：mysqldump 语法 `db [table ...]`。
    // 选中表非空时按表导出（BTreeSet 排序保证可复现）；空集合 = 整库
    // （与逻辑备份的空集合=全选语义一致）。此前该原生路径忽略了 selected_tables，
    // 用户只选一张表却整库落盘；此处修复为透传所选表名给 mysqldump。
    let mut dump_target = vec![database.to_string()];
    if !form.selected_tables.is_empty() {
        dump_target.extend(form.selected_tables.iter().cloned());
    }

    // 第一次按用户的 --routines 开关执行；MySQL 9.x 客户端 + 老服务端时
    // 会因服务端缺少 INFORMATION_SCHEMA.LIBRARIES 报错（Unknown table 'LIBRARIES'），
    // 命中该特征则自动去掉 --routines 重试一次（备份降级为不含存储过程/函数，日志说明原因）。
    let mut stderr_tail = String::new();
    let first = mysqldump_once(
        config,
        tool,
        host,
        port,
        user,
        password,
        &dump_target,
        output_path,
        cancel_flag,
        sender,
        form,
        form.include_routines,
        &mut stderr_tail,
    );
    let Err(error) = first else {
        return Ok(());
    };
    if form.include_routines && stderr_tail.contains("LIBRARIES") {
        emit_native_log(
            sender,
            "提示",
            "mysqldump 与当前服务端不兼容（查询 INFORMATION_SCHEMA.LIBRARIES 失败），\
             已自动去掉 --routines 重试；本次备份不包含存储过程/函数。\
             建议在设置中配置与服务端版本匹配的 mysqldump 路径。"
                .to_string(),
        )?;
        let mut retry_tail = String::new();
        return mysqldump_once(
            config,
            tool,
            host,
            port,
            user,
            password,
            &dump_target,
            output_path,
            cancel_flag,
            sender,
            form,
            false,
            &mut retry_tail,
        );
    }
    Err(error)
}

/// 单次 mysqldump 执行；stderr 尾部回填到 `stderr_tail` 供调用方做兼容性判定。
#[allow(clippy::too_many_arguments)]
fn mysqldump_once(
    config: &ConnectionConfig,
    tool: &str,
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    dump_target: &[String],
    output_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
    sender: &mpsc::Sender<BackupTaskProgress>,
    form: &BackupForm,
    with_routines: bool,
    stderr_tail: &mut String,
) -> anyhow::Result<()> {
    let mut cmd = Command::new(tool);
    cmd.args(["--protocol=tcp", "-h", host, "-P", &port.to_string(), "-u", user]);
    // 高级选项：仅对 MySQL/TiDB 生效（本分支已限定），不影响 SQLite 等其它类型。
    if form.single_transaction {
        cmd.arg("--single-transaction");
    }
    if form.lock_tables {
        cmd.arg("--lock-all-tables");
    }
    // 普通账号无 PROCESS 权限时，mysqldump 查询 INNODB_TABLESPACES 会报
    // "Access denied; you need (at least one of) the PROCESS privilege(s)"；
    // --no-tablespaces 为客户端 flag（仅跳过该查询），恢复不依赖 tablespace 指令，TiDB 亦兼容。
    cmd.arg("--triggers").arg("--no-tablespaces");
    if with_routines {
        cmd.arg("--routines");
    }
    for target in dump_target {
        cmd.arg(target);
    }
    // 密码只走环境变量，避免出现在进程参数列表（ps 泄露）。
    cmd.env("MYSQL_PWD", password);
    if let Endpoint::Uri { uri } = &config.endpoint {
        cmd.arg(format!("--default-auth=")); // 占位避免空 flag 报错；URI 兼容由连接器处理
        let _ = uri;
    }

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| anyhow::anyhow!("启动 mysqldump 失败：{error}"))?;

    let output = child.stdout.take().expect("stdout piped");
    let stderr_handle = if let Some(mut stderr) = child.stderr.take() {
        emit_native_log(sender, "转储", "mysqldump 已启动，正在导出数据…")?;
        // 后台线程持续收集 mysqldump 的 stderr 提示（如 using password on command line）
        let sender2 = sender.clone();
        Some(std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            let mut tail = Vec::new();
            while let Ok(n) = std::io::Read::read(&mut stderr, &mut buffer) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buffer[..n]);
                if tail.len() > 8192 {
                    let overflow = tail.len() - 8192;
                    tail.drain(..overflow);
                }
            }
            if tail.is_empty() {
                return String::new();
            }
            let text = String::from_utf8_lossy(&tail).trim().to_string();
            let _ = sender2.send(BackupTaskProgress {
                stage: "提示".to_string(),
                message: format!("mysqldump: {text}"),
                success: true,
            });
            text
        }))
    } else {
        emit_native_log(sender, "转储", "mysqldump 已启动，正在导出数据…")?;
        None
    };

    // 复制 stdout 到备份文件，同时每批检测取消请求并 kill。
    let mut out_writer = BufWriter::new(fs::File::create(output_path)?);
    let mut buffer = [0u8; 64 * 1024];
    let mut written: u64 = 0;
    let mut stdout_io = std::io::BufReader::new(output);
    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("已取消");
        }
        let n = std::io::Read::read(&mut stdout_io, &mut buffer)?;
        if n == 0 {
            break;
        }
        out_writer.write_all(&buffer[..n])?;
        written += n as u64;
    }
    out_writer.flush()?;

    let status = child.wait()?;
    // 等 stderr 线程收敛后再取尾部（进程已退出，管道到 EOF）。
    if let Some(handle) = stderr_handle {
        *stderr_tail = handle.join().unwrap_or_default();
    }
    if let Some(code) = status.code()
        && code != 0
    {
        anyhow::bail!("mysqldump 退出码 {code}");
    }
    emit_native_log(
        sender,
        "完成",
        format!("原生备份完成，共写入 {} 字节", written),
    )?;
    Ok(())
}

fn run_native_sqlite(
    config: &ConnectionConfig,
    output_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
    sender: &mpsc::Sender<BackupTaskProgress>,
) -> anyhow::Result<()> {
    let source = match &config.endpoint {
        Endpoint::SqliteFile { path, .. } => path.clone(),
        _ => anyhow::bail!("SQLite 连接缺少文件路径"),
    };
    emit_native_log(sender, "转储", "sqlite3 正在执行在线备份…")?;

    // sqlite3 的 .backup 命令以文件路径为参数，使用临时 DB 路径；此处直接把输出路径传给 .backup。
    let backup_cmd = format!(".backup '{}'", output_path.to_string_lossy());
    let status = match Command::new("sqlite3")
        .arg(source)
        .arg(backup_cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // 在线等待期间定期检测取消。
            loop {
                if cancel_flag.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("已取消");
                }
                match child.try_wait()? {
                    Some(status) => break status,
                    None => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
        Err(error) => anyhow::bail!("启动 sqlite3 失败：{error}"),
    };
    if let Some(code) = status.code()
        && code != 0
    {
        anyhow::bail!("sqlite3 退出码 {code}");
    }
    emit_native_log(sender, "完成", "原生备份完成".to_string())?;
    Ok(())
}

fn run_logic_backup(
    controller: &AppController,
    config: &ConnectionConfig,
    database: Option<&str>,
    output_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
    sender: &mpsc::Sender<BackupTaskProgress>,
    form: &BackupForm,
) -> anyhow::Result<()> {
    let Some(database) = database else {
        anyhow::bail!("未指定数据库，无法进行逻辑备份");
    };
    let base_path = ObjectPath {
        connection_id: config.id,
        database: Some(database.to_string()),
        schema: None,
        name: String::new(),
        kind: ObjectKind::Schema,
    };
    let objects = controller
        .list_objects(config.id, Some(&base_path))
        .map_err(|error| anyhow::anyhow!("枚举对象失败：{error}"))?;
    // 对象选择：表按勾选集合精确过滤（空集合 = 不备份任何表，用户需手动勾选）；
    // 视图按 include_views 开关。只有勾选的对象会被导出。
    let tables = objects
        .into_iter()
        .filter(|object| match object.path.kind {
            ObjectKind::Table => form.selected_tables.contains(&object.path.name),
            ObjectKind::View => form.include_views,
            _ => false,
        })
        .collect::<Vec<_>>();

    let mut writer = BufWriter::new(fs::File::create(output_path)?);
    writeln!(writer, "-- fluxDB 逻辑备份：{database}")?;
    writeln!(writer, "-- 生成时间：{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))?;
    writeln!(writer, "SET FOREIGN_KEY_CHECKS = 0;")?;
    writeln!(writer)?;

    let mut skipped = 0usize;
    for object in &tables {
        if cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("已取消");
        }
        // 结构（仅当「包含表结构」开启）
        if form.include_schema {
            let _ = sender.send(BackupTaskProgress {
                stage: "结构".to_string(),
                message: format!("正在导出表结构：{}", object.path.name),
                success: true,
            });
            let ddl = match controller.load_table_ddl(&object.path) {
                Ok(ddl) => ddl,
                Err(error) => {
                    skipped += 1;
                    let _ = sender.send(BackupTaskProgress {
                        stage: "跳过".to_string(),
                        message: format!("{} 结构导出失败：{error}", object.path.name),
                        success: false,
                    });
                    continue;
                }
            };
            writeln!(writer, "{ddl};")?;
            writeln!(writer)?;
        }

        // 数据：仅对 Table 且「包含表数据」开启时；View 只保留结构。
        if !form.include_data || !matches!(object.path.kind, ObjectKind::Table) {
            continue;
        }
        let _ = sender.send(BackupTaskProgress {
            stage: "数据".to_string(),
            message: format!("正在导出数据：{}", object.path.name),
            success: true,
        });
        let mut offset = 0u64;
        loop {
            if cancel_flag.load(Ordering::Relaxed) {
                anyhow::bail!("已取消");
            }
            let page = match controller.load_data_for_export(&object.path, offset, 1000, &[], &[]) {
                Ok(page) => page,
                Err(error) => {
                    skipped += 1;
                    let _ = sender.send(BackupTaskProgress {
                        stage: "跳过".to_string(),
                        message: format!("{} 数据导出失败：{error}", object.path.name),
                        success: false,
                    });
                    break;
                }
            };
            let written_rows = write_page_rows(&mut writer, &object.path, &page)?;
            if written_rows > 0 {
                let _ = sender.send(BackupTaskProgress {
                    stage: "数据".to_string(),
                    message: format!("{} 已导出 {} 行", object.path.name, offset + written_rows),
                    success: true,
                });
            }
            if !page.has_more || page.rows.is_empty() {
                break;
            }
            offset += written_rows;
        }
    }

    if cancel_flag.load(Ordering::Relaxed) {
        anyhow::bail!("已取消");
    }
    writeln!(writer, "SET FOREIGN_KEY_CHECKS = 1;")?;
    writer.flush()?;
    emit_native_log(
        sender,
        "完成",
        format!("逻辑备份完成，共导出 {} 个对象", tables.len()),
    )?;
    if skipped > 0 {
        let _ = sender.send(BackupTaskProgress {
            stage: "跳过".to_string(),
            message: format!("共跳过 {skipped} 个对象"),
            success: false,
        });
    }
    Ok(())
}

fn write_page_rows<W: Write>(
    writer: &mut W,
    object: &ObjectPath,
    page: &fluxdb_core::DataPage,
) -> io::Result<u64> {
    let indexes = page
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| (column.name.clone(), index))
        .collect::<Vec<_>>();
    let mut written = 0;
    for row in &page.rows {
        let fields = indexes
            .iter()
            .enumerate()
            .map(|(field_index, (name, source_index))| {
                let column = &page.columns[*source_index];
                RowFieldSnapshot {
                    index: field_index + 1,
                    name: name.clone(),
                    type_name: column
                        .type_name
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    primary_key: column.primary_key,
                    comment: column.comment.clone(),
                    value: row.values.get(*source_index).cloned().unwrap_or(CellValue::Null),
                }
            })
            .collect::<Vec<_>>();
        writeln!(writer, "{}", row_insert_sql(object, fields.as_slice(), false))?;
        written += 1;
    }
    Ok(written)
}

/// 底栏右侧「备份任务」状态徽标：与 SQL 文件执行一致，展示最新/运行中的备份任务，
/// 点击打开备份日志弹框。空任务占位保持宽度，避免底栏跳动。
fn backup_statusbar_area(
    tasks: &[BackupTaskState],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(task) = tasks.iter().rev().find(|task| task.running()).or_else(|| tasks.last()) else {
        return div().w(px(200.));
    };
    let running_count = tasks.iter().filter(|task| task.running()).count();
    let db = task.database.clone().unwrap_or_default();
    let label = if running_count > 1 {
        format!("{running_count} 个备份任务运行中")
    } else if task.cancel_requested {
        format!("{db} 正在停止 · {}", task.stage)
    } else if task.running() {
        format!("{db} 备份中 · {}", task.stage)
    } else if task.canceled {
        format!("{db} 已取消")
    } else if task.error.is_some() {
        format!("{db} 备份失败")
    } else {
        format!("{db} 备份完成")
    };
    let has_error = task.error.is_some();
    let task_id = task.id;
    div()
        .w(px(240.))
        .h_full()
        .flex()
        .items_center()
        .justify_end()
        .pr_2()
        .child(
            div()
                .max_w(px(228.))
                .h(px(20.))
                .px_2()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.backup_log_task = Some(task_id);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(
                    if task.running() { AppIcon::Play } else { AppIcon::Database },
                    13.,
                    if has_error { rgb(0xd64545) } else { rgb(0x1687ff) },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .truncate()
                        .whitespace_nowrap()
                        .text_size(px(12.))
                        .text_color(colors.text)
                        .child(label),
                ),
        )
}
