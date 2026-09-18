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
        // 对象选择默认：表右键预选该表并按对象清单导出；其它入口默认整库备份（全部对象）。
        let scope_all = preselect_table.is_none();
        let preselected: BTreeSet<String> = preselect_table
            .map(|name| {
                let mut set = BTreeSet::new();
                set.insert(name.to_string());
                set
            })
            .unwrap_or_default();
        // 打开弹框时把设置里的备份目录填入输入框（默认值，用户可更换）。
        self.backup_target_dir_input
            .update(cx, |input, cx| input.set_value(target_dir.clone(), window, cx));
        self.pending_backup_modal = Some(BackupForm {
            connection_id,
            database_kind: self.controller.connection_configs().iter().find(|c| c.id == connection_id).map(|c| c.kind).unwrap_or(DatabaseKind::MySql),
            selected_tables: all_tables.intersection(&preselected).cloned().collect(),
            scope_all,
            all_table_names: all_tables,
            all_view_names: all_views,
            database,
            mode: BackupMode::Auto,
            target_dir,
            tab: BackupTab::General,
            file_name: String::new(),
            object_search: String::new(),
            include_views: scope_all,
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

    /// 选择本次备份的输出目录，并将结果同时回填输入框和表单状态。
    fn choose_backup_target_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择备份目录".into()),
        });
        let input = self.backup_target_dir_input.clone();
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = receiver.await;
            let _ = cx.update(|window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            let value = path.display().to_string();
                            input.update(cx, |input, cx| {
                                input.set_value(value.clone(), window, cx);
                            });
                            if let Some(form) = &mut this.pending_backup_modal {
                                form.target_dir = value;
                            }
                            cx.notify();
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.show_message(
                        format!("选择备份目录失败：{error}"),
                        AppMessageKind::Error,
                        cx,
                    ),
                    Err(error) => this.show_message(
                        format!("选择备份目录失败：{error}"),
                        AppMessageKind::Error,
                        cx,
                    ),
                });
            });
        }));
    }

    fn confirm_backup(&mut self, cx: &mut Context<Self>) {
        let Some(mut form) = self.pending_backup_modal.clone() else {
            return;
        };
        // 备份目录输入留空则回退到设置中的 backup_dir（两者都空才拦截提示）。
        if form.target_dir.trim().is_empty() {
            form.target_dir = self.controller.state().settings.backup_dir.trim().to_string();
        }
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

    /// 备份记录唯一标识：unix 纳秒时间戳的十六进制串，单机本机生成足够唯一。
    fn new_backup_record_id() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{nanos:016x}")
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

        // 暂存本次备份记录（归属连接/库 + 表清单/视图开关/备注）；
        // 成功后由 backup_finish_on_ui 补齐全路径/时间/大小并写入 sqlite。
        self.backup_pending_metas.insert(
            task_id,
            BackupFileMeta {
                id: Self::new_backup_record_id(),
                connection_id: form.connection_id,
                database: database.clone(),
                output_path: String::new(),
                created_unix: 0,
                size: 0,
                // 整库备份记录为空表清单（Some([]) = 整库）；否则记录勾选的表。
                tables: Some(if form.scope_all {
                    Vec::new()
                } else {
                    form.selected_tables.iter().cloned().collect()
                }),
                include_views: form.include_views,
                note: form.note.clone(),
                manifest: None,
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
                        run_backup_task(controller, form, output_path, cancel_flag, sender)
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
        result: Result<(PathBuf, fluxdb_app::BackupManifest), String>,
        database: String,
        output_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let mut output_path = output_path;
        let (canceled, error) = match result {
            Ok((path, manifest)) => {
                output_path = path;
                if let Some(meta) = self.backup_pending_metas.get_mut(&task_id) {
                    meta.manifest = Some(manifest);
                }
                if let Some(task) = self.backup_tasks.iter_mut().find(|task| task.id == task_id) {
                    task.output_path = output_path.clone();
                }
                (false, None)
            },
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
        // 失败时保留现场，不删除可能在预检查前已存在的同名文件。
        if let Some(mut meta) = self.backup_pending_metas.remove(&task_id) {
            if meta_ok {
                // 补全真实文件信息后写入 sqlite 备份记录（真实数据仍在磁盘，这里只存记录）。
                meta.output_path = output_path.to_string_lossy().to_string();
                meta.created_unix = chrono::Local::now().timestamp();
                meta.size = fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
                self.persist_backup_record(meta)
                    .unwrap_or_else(|fail| {
                        tracing::warn!(path = %output_path.display(), error = %fail, "备份记录写入失败");
                    });
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
    let format = if form.database_kind == DatabaseKind::Sqlite { fluxdb_app::BackupFormat::SqliteBinary } else { fluxdb_app::BackupFormat::Sql };
    let default = {
        let name = form.database.clone().unwrap_or_default();
        let name = safe_data_export_filename_segment(&name);
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        format!("{name}_{timestamp}.sql")
    };
    let raw = form.file_name.trim();
    if raw.is_empty() {
        return fluxdb_app::normalize_backup_file_name(default, format);
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
    fluxdb_app::normalize_backup_file_name(resolved, format)
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
