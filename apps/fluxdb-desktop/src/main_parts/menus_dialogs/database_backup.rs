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
            .or_else(|| {
                self.current_query_scope()
                    .map(|(connection_id, _)| connection_id)
            })
            .or_else(|| self.first_connection_id())
        else {
            self.show_message("请先创建连接", AppMessageKind::Warning, cx);
            return;
        };
        let database = database
            .or_else(|| {
                self.current_query_scope()
                    .and_then(|(_, database)| database)
            })
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
            selected_tables: all_tables.intersection(&preselected).cloned().collect(),
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
            pg_include_owner: false,
            pg_include_acl: false,
        });
        // 打开即预检 PostgreSQL 客户端：缺工具时在对话框顶部提示，不用等点了备份才报错。
        // 只查文件存在性（不执行 --version），保证打开对话框不被子进程拖慢。
        self.backup_pg_client_missing = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .is_some_and(|connection| connection.config.kind == DatabaseKind::Postgres)
            && !fluxdb_app::pg_client_tool_present(
                &self.controller.state().settings,
                fluxdb_app::PgClientTool::Dump,
            );
        self.backup_mysql_client_missing = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .is_some_and(|connection| {
                matches!(
                    connection.config.kind,
                    DatabaseKind::MySql | DatabaseKind::TiDb
                )
            })
            && !fluxdb_app::mysql_client_tool_present(
                &self.controller.state().settings,
                fluxdb_app::MySqlClientTool::Dump,
            );
        if self.backup_pg_client_missing {
            tracing::warn!("打开备份对话框时未发现 PostgreSQL 客户端工具");
        }
        if self.backup_mysql_client_missing {
            tracing::warn!("打开备份对话框时未发现 MySQL 客户端工具");
        }
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
    fn current_backup_tables(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
    ) -> BTreeSet<String> {
        let Some(database) = database else {
            return BTreeSet::new();
        };
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| {
                let names: BTreeSet<String> =
                    group_objects(connection, database, None, ObjectGroup::Tables)
                        .into_iter()
                        .map(|object| object.path.name.clone())
                        .collect();
                names
            })
            .unwrap_or_default()
    }

    /// 当前连接+库下所有视图名（对象选择页签「视图」分组资源）。
    fn current_backup_views(
        &self,
        connection_id: ConnectionId,
        database: Option<&str>,
    ) -> BTreeSet<String> {
        let Some(database) = database else {
            return BTreeSet::new();
        };
        self.controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| {
                group_objects(connection, database, None, ObjectGroup::Views)
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
        if !form.include_schema && !form.include_data {
            self.show_message("表结构和数据至少需要包含一项", AppMessageKind::Warning, cx);
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
        self._backup_cancel_flags
            .insert(task_id, cancel_flag.clone());

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
                            this.backup_finish_on_ui(
                                task_id,
                                result,
                                database.clone(),
                                output_path_finish.clone(),
                                cx,
                            );
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
        let name = form.database.clone().unwrap_or_default();
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
    let mut mode = match form.mode {
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
    // PostgreSQL 的逻辑备份即原生 pg_dump（本质是 SQL dump，且带准确 schema 范围）；
    // 显式选「逻辑备份」也归一为原生 pg_dump，避免落入 MySQL 专用的逐表 run_logic_backup
    // （其 `SET FOREIGN_KEY_CHECKS` 为 MySQL 语法，对 PG 非法）。
    if config.kind == DatabaseKind::Postgres && mode == BackupMode::Logic {
        mode = BackupMode::Native;
    }
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
            DatabaseKind::Sqlite => run_native_sqlite(&config, &output_path, &cancel_flag, &sender),
            DatabaseKind::MongoDb | DatabaseKind::Redis => {
                anyhow::bail!("当前连接类型不支持原生备份")
            }
            DatabaseKind::Postgres => {
                let ssl_mode = config
                    .postgres_profile
                    .as_ref()
                    .map(|profile| profile.tls.ssl_mode)
                    .unwrap_or(PostgresSslMode::Prefer);
                // CA/客户端证书/私钥路径沿连接档案下发给 pg_dump（经 libpq env，不入 argv）。
                let tls_paths = config
                    .postgres_profile
                    .as_ref()
                    .map(|profile| fluxdb_app::pg_native_tls_paths(&profile.tls))
                    .unwrap_or_default();
                // 工具解析：设置目录 → 应用下载目录 → 系统安装路径 → PATH（见 pg_client_tools）。
                // 解析不到时直接给带安装引导的错误，而不是等 spawn 失败后抛裸 OS 错误。
                let server_major = fluxdb_app::pg_server_major_version(&config).ok().flatten();
                let Some(resolved) = fluxdb_app::resolve_pg_client_tool(
                    &settings,
                    fluxdb_app::PgClientTool::Dump,
                    server_major,
                ) else {
                    anyhow::bail!("{}", fluxdb_app::pg_client_install_hint());
                };
                // 版本校验：pg_dump 客户端主版本不得低于服务端主版本（PG 禁止更旧客户端备份）。
                // 工具版本从 `pg_dump --version` 解析；服务端版本经连接器读取；任一未知则放行（不误拦）。
                let tool_major = resolved.major_version;
                if !fluxdb_app::pg_dump_version_compatible(tool_major, server_major) {
                    anyhow::bail!(
                        "pg_dump 版本过旧：客户端主版本 {} 低于服务端主版本 {}。\
                         请在「设置 → 数据 → 备份」中下载匹配版本的 PostgreSQL 客户端，\
                         或指定不低于服务端版本的客户端目录",
                        tool_major.unwrap_or(0),
                        server_major.unwrap_or(0)
                    );
                }
                // SSH 隧道（若启用）：复用连接器的 libssh2 把远端映射到本地端口，工具经
                // 127.0.0.1:local 拨号（PGHOSTADDR），-h 保持真实远端供 TLS 校验；保持通道至工具结束。
                let ssh = config
                    .postgres_profile
                    .as_ref()
                    .and_then(|profile| profile.ssh());
                if let Some(ssh) = ssh {
                    return run_native_pg_dump_via_ssh(
                        &resolved.program,
                        ssh,
                        config.postgres_profile.as_ref().map(|profile| profile.connect_timeout_secs()).unwrap_or(5),
                        &host,
                        port,
                        &user,
                        &password,
                        ssl_mode,
                        &tls_paths,
                        form.database.as_deref().unwrap_or_default(),
                        &output_path,
                        &cancel_flag,
                        &sender,
                        &form,
                    );
                }
                run_native_pg_dump(
                    &resolved.program,
                    &host,
                    port,
                    &user,
                    &password,
                    ssl_mode,
                    &tls_paths,
                    form.database.as_deref().unwrap_or_default(),
                    &output_path,
                    &cancel_flag,
                    &sender,
                    &form,
                    None,
                )
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
    // PG 与 MySQL 凭据分属不同档案/options 键（PG 用 host/maintenance_database/username/password，
    // MySQL 用 root/剩下扁平键），按连接类型选择对应归一化，避免 PG 拿到 MySQL 默认 root。
    let resolved = match config.kind {
        DatabaseKind::Postgres => config.postgres_resolved(),
        _ => config.mysql_resolved(),
    };
    let (host, port) = match &resolved.endpoint {
        Endpoint::Tcp { host, port, .. } => (host.clone(), *port),
        Endpoint::SqliteFile { .. } => (String::new(), 0),
        Endpoint::Uri { .. } => (String::new(), 0),
    };
    let user = resolved
        .options
        .get("username")
        .cloned()
        .unwrap_or_else(|| {
            if config.kind == DatabaseKind::Postgres {
                String::new()
            } else {
                "root".to_string()
            }
        });
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
            return fluxdb_app::resolve_mysql_client_tool(
                settings,
                fluxdb_app::MySqlClientTool::Dump,
            )
            .is_some();
        }
        DatabaseKind::Sqlite => {
            if !settings.sqlite3_path.trim().is_empty() {
                &settings.sqlite3_path
            } else {
                "sqlite3"
            }
        }
        // PostgreSQL 走统一的客户端解析（含应用下载目录与系统标准安装路径）。
        DatabaseKind::Postgres => {
            return fluxdb_app::resolve_pg_client_tool(
                settings,
                fluxdb_app::PgClientTool::Dump,
                None,
            )
            .is_some();
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
    // 与 DBeaver 一致：mysqldump 会直接截断同名文件，原生备份执行前拒绝覆盖。
    if output_path.exists() {
        anyhow::bail!("备份文件已存在，请修改文件名：{}", output_path.display());
    }
    let Some(client) =
        fluxdb_app::resolve_mysql_client_tool(settings, fluxdb_app::MySqlClientTool::Dump)
    else {
        anyhow::bail!("{}", fluxdb_app::mysql_client_install_hint());
    };
    let tool = client.program.display().to_string();
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
        &tool,
        &client.version,
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
            &tool,
            &client.version,
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
    tool: &str,
    client_version: &fluxdb_app::MySqlClientVersion,
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
    let database = dump_target.first().map(String::as_str).unwrap_or_default();
    let tables = dump_target.get(1..).unwrap_or_default();
    let invocation = fluxdb_app::mysql_dump_invocation(
        tool,
        client_version,
        host,
        port,
        user,
        password,
        database,
        tables,
        fluxdb_app::MySqlDumpOptions {
            include_schema: form.include_schema,
            include_data: form.include_data,
            include_routines: with_routines,
            single_transaction: form.single_transaction,
            lock_tables: form.lock_tables,
        },
    );
    let mut cmd = Command::new(&invocation.program);
    cmd.args(&invocation.args);
    for (key, value) in &invocation.env {
        cmd.env(key, value);
    }
    prepare_tree_kill_command(&mut cmd);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            anyhow::anyhow!(
                "启动 mysqldump 失败：{error}。{}",
                fluxdb_app::mysql_client_install_hint()
            )
        })?;

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
            kill_child_tree(&mut child);
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
    let mut sqlite3_cmd = Command::new("sqlite3");
    sqlite3_cmd.arg(source).arg(backup_cmd).stdout(Stdio::null()).stderr(Stdio::null());
    prepare_tree_kill_command(&mut sqlite3_cmd);
    let status = match sqlite3_cmd.spawn() {
        Ok(mut child) => {
            // 在线等待期间定期检测取消。
            loop {
                if cancel_flag.load(Ordering::Relaxed) {
                    kill_child_tree(&mut child);
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

/// PostgreSQL 原生备份：调用 pg_dump（plain+inserts 单文件 .sql，可直接用 psql 恢复）。
///
/// 密码只经 `PGPASSWORD` 环境变量，不进 argv（避免 `ps` 泄露，设计 §11.2）。表过滤走 `-t`
/// 透传所选表名（schema 限定时原样下发）；空集合 = 整库。stdout 流式写文件并逐批检测取消。
#[allow(clippy::too_many_arguments)]
fn run_native_pg_dump(
    tool: &str,
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    ssl_mode: PostgresSslMode,
    tls_paths: &fluxdb_app::NativeTlsPaths,
    database: &str,
    output_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
    sender: &mpsc::Sender<BackupTaskProgress>,
    form: &BackupForm,
    // SSH 隧道场景的拨号地址（隧道本地 127.0.0.1）；host 仍为真实远端供 TLS 校验。
    hostaddr: Option<&str>,
) -> anyhow::Result<()> {
    if database.is_empty() {
        anyhow::bail!("未指定数据库，无法进行原生备份");
    }
    // 直接使用预检已验证的路径，直连与 SSH 均不重新选择客户端。
    if cfg!(not(test)) && host.is_empty() {
        anyhow::bail!("PostgreSQL 连接缺少主机信息");
    }

    // 备份范围：结构/数据/完整（复用表单 include_schema/include_data）。
    let scope = match (form.include_schema, form.include_data) {
        (true, false) => fluxdb_app::PgDumpScope::SchemaOnly,
        (false, true) => fluxdb_app::PgDumpScope::DataOnly,
        _ => fluxdb_app::PgDumpScope::Full,
    };
    let tables: Vec<String> = form.selected_tables.iter().cloned().collect();
    // 参数与凭据/传输由连接器统一构造（可单测）；密码/TLS 只入 env，argv 无凭据、无 shell。
    let invocation = fluxdb_app::pg_dump_invocation(
        &tool,
        host,
        port,
        user,
        database,
        Some(password),
        ssl_mode,
        tls_paths,
        scope,
        form.pg_include_owner,
        form.pg_include_acl,
        &tables,
    );
    let mut cmd = Command::new(&invocation.program);
    cmd.args(&invocation.args);
    for (key, value) in &invocation.env {
        cmd.env(key, value);
    }
    // SSH 隧道：host 保持真实远端（TLS 校验用），PGHOSTADDR 指向隧道本地地址实际拨号。
    if let Some(addr) = hostaddr {
        for (key, value) in fluxdb_app::pg_hostaddr_env(addr, port) {
            cmd.env(key, value);
        }
    }
    prepare_tree_kill_command(&mut cmd);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            anyhow::anyhow!(
                "启动 pg_dump 失败：{error}。{}",
                fluxdb_app::pg_client_install_hint()
            )
        })?;

    let output = child.stdout.take().expect("stdout piped");
    // stderr 尾部收集（pg_dump 的错误/提示，如 connection 相关）。
    let stderr_handle = if let Some(mut stderr) = child.stderr.take() {
        emit_native_log(sender, "转储", "pg_dump 已启动，正在导出数据…")?;
        Some(std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
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
            String::from_utf8_lossy(&tail).trim().to_string()
        }))
    } else {
        emit_native_log(sender, "转储", "pg_dump 已启动，正在导出数据…")?;
        None
    };

    // 复制 stdout 到备份文件，同时每批检测取消请求并 kill。
    let mut out_writer = BufWriter::new(fs::File::create(output_path)?);
    let mut buffer = [0u8; 64 * 1024];
    let mut written: u64 = 0;
    let mut stdout_io = std::io::BufReader::new(output);
    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            kill_child_tree(&mut child);
            // 清理取消产生的半成品备份，避免残留部分 dump 被误当成功备份。
            let _ = fs::remove_file(output_path);
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
    let stderr_tail = if let Some(handle) = stderr_handle {
        handle.join().unwrap_or_default()
    } else {
        String::new()
    };
    if let Some(code) = status.code()
        && code != 0
    {
        if !stderr_tail.is_empty() {
            let _ = sender.send(BackupTaskProgress {
                stage: "转储".to_string(),
                message: format!("pg_dump: {stderr_tail}"),
                success: false,
            });
        }
        // 失败也清理半成品输出，避免留下残缺备份被扫描为「历史备份」。
        let _ = fs::remove_file(output_path);
        anyhow::bail!("pg_dump 退出码 {code}");
    }
    emit_native_log(
        sender,
        "完成",
        format!("原生备份完成，共写入 {} 字节", written),
    )?;
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
    writeln!(
        writer,
        "-- 生成时间：{}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    )?;
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
            let written_rows = write_page_rows(&mut writer, &object.path, &page, config.kind)?;
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
    db_kind: DatabaseKind,
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
                    value: row
                        .values
                        .get(*source_index)
                        .cloned()
                        .unwrap_or(CellValue::Null),
                }
            })
            .collect::<Vec<_>>();
        writeln!(
            writer,
            "{}",
            row_insert_sql(object, fields.as_slice(), false, db_kind)
        )?;
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
    let Some(task) = tasks
        .iter()
        .rev()
        .find(|task| task.running())
        .or_else(|| tasks.last())
    else {
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
                    if task.running() {
                        AppIcon::Play
                    } else {
                        AppIcon::Database
                    },
                    13.,
                    if has_error {
                        rgb(0xd64545)
                    } else {
                        rgb(0x1687ff)
                    },
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

/// 经连接器的 libssh2 隧道做 PG 原生备份：PGHOSTADDR 指向本地端口，
/// `-h` 保留远端 TLS 身份；pg_dump 结束、失败或取消后释放隧道。
#[allow(clippy::too_many_arguments)]
fn run_native_pg_dump_via_ssh(
    tool: &str,
    ssh: &fluxdb_core::PostgresSshOptions,
    inherited_connect_timeout_secs: u32,
    remote_host: &str,
    remote_port: u16,
    user: &str,
    password: &str,
    ssl_mode: PostgresSslMode,
    tls_paths: &fluxdb_app::NativeTlsPaths,
    database: &str,
    output_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
    sender: &mpsc::Sender<BackupTaskProgress>,
    form: &BackupForm,
) -> anyhow::Result<()> {
    emit_native_log(sender, "隧道", format!("正在经 SSH {} 建立隧道", ssh.host))?;
    if cancel_flag.load(Ordering::Relaxed) {
        anyhow::bail!("备份已取消");
    }
    let tunnel = fluxdb_app::pg_open_native_ssh_tunnel(
        ssh,
        remote_host,
        remote_port,
        inherited_connect_timeout_secs,
    )
    .map_err(|error| anyhow::anyhow!("SSH 隧道建立失败：{error}"))?;
    let local_port = tunnel.local_port();
    if cancel_flag.load(Ordering::Relaxed) {
        anyhow::bail!("备份已取消");
    }

    let result = run_native_pg_dump(
        tool,
        remote_host,
        local_port,
        user,
        password,
        ssl_mode,
        tls_paths,
        database,
        output_path,
        cancel_flag,
        sender,
        form,
        Some("127.0.0.1"),
    );

    // 句柄离开作用域时连接器会关闭监听并等待桥线程退出。
    drop(tunnel);
    result
}
