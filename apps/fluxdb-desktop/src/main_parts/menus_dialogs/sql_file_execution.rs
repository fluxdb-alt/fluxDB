const SQL_FILE_VISIBLE_LOG_LIMIT: usize = 300;
const SQL_FILE_START_DELAY_MS: u64 = 50;

/// 「执行 SQL 文件」Dialog builder 需要的共享句柄。
///
/// dialog builder 在 `NavicatMain::render` 期间被同步执行（NavicatMain 处于 lease 状态），
/// builder 内部不能 `view.read/update(cx)`，实时状态统一走 `data`（Rc<RefCell<..>>）；
/// `view` 只在事件回调里使用（事件派发期不在渲染 lease 内，安全）。
#[derive(Clone)]
struct SqlFileDialogViews {
    view: Entity<NavicatMain>,
    data: Rc<std::cell::RefCell<SqlFileModalData>>,
    connection_select: Entity<SelectState<SearchableVec<SqlFileConnectionItem>>>,
    database_select: Entity<SelectState<SearchableVec<SqlFileDatabaseItem>>>,
    encoding_select: Entity<SelectState<SearchableVec<SqlFileEncodingItem>>>,
    path_input: Entity<InputState>,
}

impl NavicatMain {
    fn show_sql_file_execution_modal(
        &mut self,
        connection_id: Option<ConnectionId>,
        database: Option<String>,
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

        {
            let mut data = self.sql_file_modal.borrow_mut();
            data.form = Some(SqlFileExecutionForm {
                connection_id,
                database,
                path: None,
                encoding: SqlFileEncoding::Utf8,
                continue_on_error: true,
                split_statements: true,
                active_tab: SqlFileExecutionTab::General,
            });
            data.log_task = None;
        }
        self.sql_file_path_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.refresh_sql_file_selects(window, cx);
        // 先把焦点放回主视图：Dialog 打开时会记录 previous_focused_handle，关闭后焦点还原到这里
        self.focus_handle.focus(window, cx);
        self.open_sql_file_dialog(window, cx);
    }

    /// 打开（或刷新）「执行 SQL 文件」gpui-component Dialog。
    ///
    /// 已打开时只 `cx.notify()`：`Root::render_dialog_layer` 每帧重跑 builder，内容自然更新。
    /// 主题色在打开时捕获；打开期间切换主题的弹框内颜色不会即时刷新（下一次打开生效）。
    fn open_sql_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        {
            let mut data = self.sql_file_modal.borrow_mut();
            if data.dialog_open {
                cx.notify();
                return;
            }
            data.dialog_open = true;
        }
        let colors = ui_colors_from_theme(self.theme_mode, cx);
        let views = SqlFileDialogViews {
            view: cx.entity(),
            data: self.sql_file_modal.clone(),
            connection_select: self.sql_file_connection_select.clone(),
            database_select: self.sql_file_database_select.clone(),
            encoding_select: self.sql_file_encoding_select.clone(),
            path_input: self.sql_file_path_input.clone(),
        };
        window.open_dialog(cx, move |dialog, window, _cx| {
            sql_file_build_dialog(dialog, views.clone(), colors, window)
        });
    }

    /// 程序化关闭（页脚「关闭」/清除当前查看任务后）：X、Esc、遮罩由 Dialog 内部关闭并回调 on_close 清理。
    fn request_close_sql_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = {
            let mut data = self.sql_file_modal.borrow_mut();
            let was_open = data.dialog_open;
            data.form = None;
            data.log_task = None;
            data.dialog_open = false;
            was_open
        };
        if was_open {
            window.close_dialog(cx);
        }
        cx.notify();
    }

    fn refresh_sql_file_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 快照后立刻释放 RefCell 借用：后续 set_items/set_selected_value 可能同步触发
        // Select 订阅（订阅内会 borrow_mut 同一 RefCell），持锁调用会 panic
        let form = { self.sql_file_modal.borrow().form.clone() };
        let Some(form) = form else {
            return;
        };
        self.sql_file_connection_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(sql_file_connection_items(self.controller.state())),
                window,
                cx,
            );
            select.set_selected_value(&form.connection_id, window, cx);
        });
        self.sql_file_database_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(sql_file_database_items(
                    self.controller.state(),
                    form.connection_id,
                )),
                window,
                cx,
            );
            select.set_selected_value(&form.database, window, cx);
        });
        self.sql_file_encoding_select.update(cx, |select, cx| {
            select.set_selected_value(&form.encoding, window, cx);
        });
    }

    fn choose_sql_file_for_execution(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 SQL 文件".into()),
        });
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let _ = cx.update(|_, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择 SQL 文件失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                        });
                    });
                    None
                }
                Err(error) => {
                    let _ = cx.update(|_, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!("选择 SQL 文件失败：{error}"),
                                AppMessageKind::Error,
                                cx,
                            );
                        });
                    });
                    None
                }
            };
            let Some(path) = path else {
                return;
            };
            let _ = cx.update(|window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    // 快照路径文本并释放借用后再 set_value：
                    // set_value 会同步触发输入框订阅（内部 borrow_mut），持锁调用会 panic
                    let path_display = {
                        let mut data = this.sql_file_modal.borrow_mut();
                        match &mut data.form {
                            Some(form) => {
                                form.path = Some(path.clone());
                                Some(path.display().to_string())
                            }
                            None => None,
                        }
                    };
                    if let Some(display) = path_display {
                        this.sql_file_path_input.update(cx, |input, cx| {
                            input.set_value(display, window, cx);
                        });
                    }
                    cx.notify();
                });
            });
        }));
    }

    /// 状态栏任务芯片点击：打开 Dialog 并直接展示该任务的「消息日志」。
    fn show_sql_file_task_log(
        &mut self,
        task_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        {
            let mut data = self.sql_file_modal.borrow_mut();
            if !data.tasks.iter().any(|task| task.id == task_id) {
                return;
            }
            data.form = None;
            data.log_task = Some(task_id);
        }
        self.open_sql_file_dialog(window, cx);
    }

    fn confirm_sql_file_execution(&mut self, cx: &mut Context<Self>) {
        // 先释放借用：后续 create_sql_file_task 会 borrow_mut 同一 RefCell
        let form = { self.sql_file_modal.borrow().form.clone() };
        let Some(form) = form else {
            return;
        };
        let Some(path) = form.path.clone() else {
            self.show_message("请选择 SQL 文件", AppMessageKind::Warning, cx);
            return;
        };
        let task_id = self.create_sql_file_task(&form, path.clone());
        {
            let mut data = self.sql_file_modal.borrow_mut();
            data.form = None;
            data.log_task = Some(task_id);
        }
        cx.notify();

        self._file_picker_task = Some(cx.spawn(async move |view, cx| {
            smol::Timer::after(Duration::from_millis(SQL_FILE_START_DELAY_MS)).await;
            let result = cx
                .background_spawn(read_sql_file(path.clone(), form.encoding))
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| match result {
                    Ok(text) => this.start_sql_file_execution(task_id, form, path, text, cx),
                    Err(error) => {
                        let message = format!("读取 SQL 文件失败：{error}");
                        this.finish_sql_file_task(
                            task_id,
                            Err(fluxdb_core::UserFacingError {
                                title: "读取失败".to_string(),
                                message: message.clone(),
                                detail: None,
                                retryable: false,
                            }),
                            false,
                        );
                        this.show_message(message, AppMessageKind::Error, cx);
                        cx.notify();
                    }
                });
            });
        }));
    }

    fn create_sql_file_task(&mut self, form: &SqlFileExecutionForm, path: PathBuf) -> u64 {
        let mut data = self.sql_file_modal.borrow_mut();
        let task_id = data.task_seq + 1;
        data.task_seq = task_id;
        data.tasks.push(SqlFileExecutionTaskState {
            id: task_id,
            file_name: sql_file_name(&path),
            path,
            connection_id: form.connection_id,
            database: form.database.clone(),
            tab_id: None,
            total: 1,
            processed: 0,
            errors: 0,
            started_at: Instant::now(),
            finished_at: None,
            logs: Vec::new(),
            error: None,
            cancel_requested: false,
            canceled: false,
        });
        task_id
    }

    fn start_sql_file_execution(
        &mut self,
        task_id: u64,
        form: SqlFileExecutionForm,
        path: PathBuf,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if text.trim().is_empty() {
            self.finish_sql_file_task(
                task_id,
                Err(fluxdb_core::UserFacingError {
                    title: "SQL 文件为空".to_string(),
                    message: "SQL 文件为空".to_string(),
                    detail: None,
                    retryable: false,
                }),
                false,
            );
            self.show_message("SQL 文件为空", AppMessageKind::Warning, cx);
            cx.notify();
            return;
        }

        let file_name = sql_file_name(&path);
        let total = sql_file_execution_statement_count(&text, form.split_statements).max(1);
        // 取消标志可能在「任务还没开始执行」时就已被用户点过，需要带回后台任务
        let cancel_requested = {
            let mut data = self.sql_file_modal.borrow_mut();
            let cancel_requested = data
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .is_some_and(|task| task.cancel_requested);
            if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
                task.file_name = file_name;
                task.path = path;
                task.tab_id = None;
                task.total = total;
            }
            data.log_task = Some(task_id);
            cancel_requested
        };

        let connection_id = form.connection_id;
        let database = form.database.clone();
        let options = QueryExecutionOptions {
            continue_on_error: form.continue_on_error,
            split_statements: form.split_statements,
            ..QueryExecutionOptions::default()
        };
        let cancel_flag = Arc::new(AtomicBool::new(cancel_requested));
        self._sql_file_cancel_flags
            .insert(task_id, cancel_flag.clone());
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let (summary_tx, summary_rx) = mpsc::channel::<fluxdb_core::QueryExecutionSummary>();
            let query_cancel_flag = cancel_flag.clone();
            let mut result_task = cx.background_spawn(async move {
                let mut on_summary = move |summary| {
                    let _ = summary_tx.send(summary);
                };
                controller
                    .execute_query_text_for_scope_with_progress(
                        connection_id,
                        database,
                        text,
                        options,
                        &mut on_summary,
                        &|| query_cancel_flag.load(Ordering::Relaxed),
                    )
                    .map_err(fluxdb_core::UserFacingError::from)
            });
            let result = loop {
                if let Some(result) = (&mut result_task).now_or_never() {
                    break result;
                }
                let summaries = summary_rx.try_iter().collect::<Vec<_>>();
                if !summaries.is_empty() {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.apply_sql_file_progress(task_id, summaries, cx);
                        });
                    });
                }
                smol::Timer::after(Duration::from_millis(50)).await;
            };
            let summaries = summary_rx.try_iter().collect::<Vec<_>>();
            if !summaries.is_empty() {
                let _ = cx.update(|cx| {
                    let Some(view) = view.upgrade() else {
                        return;
                    };
                    view.update(cx, |this, cx| {
                        this.apply_sql_file_progress(task_id, summaries, cx);
                    });
                });
            }
            let canceled = cancel_flag.load(Ordering::Relaxed);

            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._sql_file_execute_tasks.remove(&task_id);
                    this._sql_file_cancel_flags.remove(&task_id);
                    this.finish_sql_file_task(task_id, result.clone(), canceled);
                    let message = this
                        .sql_file_modal
                        .borrow()
                        .tasks
                        .iter()
                        .find(|task| task.id == task_id)
                        .map(sql_file_task_message)
                        .unwrap_or_else(|| "SQL 文件执行完成".to_string());
                    let kind = if message.contains("失败") {
                        AppMessageKind::Warning
                    } else {
                        AppMessageKind::Success
                    };
                    this.show_message(message, kind, cx);
                    this.open_connection_from_sidebar(connection_id, cx);
                    cx.notify();
                });
            });
        });
        self._sql_file_execute_tasks.insert(task_id, task);
        cx.notify();
    }

    fn finish_sql_file_task(
        &mut self,
        task_id: u64,
        result: std::result::Result<fluxdb_core::QueryExecutionResult, fluxdb_core::UserFacingError>,
        canceled: bool,
    ) {
        let mut data = self.sql_file_modal.borrow_mut();
        let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) else {
            return;
        };
        task.finished_at = Some(Instant::now());
        task.canceled = canceled;
        match result {
            Ok(_) => {}
            Err(error) => {
                if task.error.is_none() {
                    task.error = Some(error.message);
                }
            }
        }
    }

    fn record_sql_file_statement_summary(
        &mut self,
        task_id: u64,
        summary: fluxdb_core::QueryExecutionSummary,
    ) -> Option<usize> {
        let mut data = self.sql_file_modal.borrow_mut();
        let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) else {
            return None;
        };
        let index = task.processed + 1;
        task.processed = task.processed.saturating_add(1);
        if !summary.success {
            task.errors = task.errors.saturating_add(1);
        }
        task.logs.push(SqlFileExecutionLogEntry {
            index,
            success: summary.success,
            elapsed_ms: summary.elapsed_ms,
            message: summary.message,
            sql: sql_file_preview(&summary.sql),
        });
        Some(index)
    }

    fn apply_sql_file_progress(
        &mut self,
        task_id: u64,
        summaries: Vec<fluxdb_core::QueryExecutionSummary>,
        cx: &mut Context<Self>,
    ) {
        for summary in summaries {
            self.record_sql_file_statement_summary(task_id, summary);
        }
        cx.notify();
    }

    fn cancel_sql_file_task(&mut self, task_id: u64, cx: &mut Context<Self>) {
        // 借用只覆盖状态写入，show_message/flags 操作在释放后进行
        {
            let mut data = self.sql_file_modal.borrow_mut();
            let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) else {
                return;
            };
            if !task.running() {
                return;
            }
            task.cancel_requested = true;
        }
        if let Some(flag) = self._sql_file_cancel_flags.get(&task_id) {
            flag.store(true, Ordering::Relaxed);
        }
        self.show_message("已请求取消，当前 SQL 完成后停止", AppMessageKind::Warning, cx);
        cx.notify();
    }

    fn copy_sql_file_task_log(&mut self, task_id: u64, cx: &mut Context<Self>) {
        // 快照并释放借用后再走剪贴板/消息，避免 RefCell 借用跨越副作用调用
        let task = {
            self.sql_file_modal
                .borrow()
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
        };
        let Some(task) = task else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(sql_file_task_log_text(&task)));
        self.show_message("已复制日志", AppMessageKind::Success, cx);
    }

    fn clear_sql_file_task(&mut self, task_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let is_running = self
            .sql_file_modal
            .borrow()
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.running());
        if is_running {
            self.show_message("任务执行中，暂不能清除", AppMessageKind::Warning, cx);
            return;
        }
        let clears_current_view = {
            let mut data = self.sql_file_modal.borrow_mut();
            data.tasks.retain(|task| task.id != task_id);
            let mut clears = false;
            if data.log_task == Some(task_id) {
                data.log_task = None;
                // 清除的正是当前查看的任务且没有表单视图时，弹框无内容可展示 → 关闭
                clears = data.form.is_none();
            }
            clears
        };
        if clears_current_view {
            self.request_close_sql_file_dialog(window, cx);
        } else {
            cx.notify();
        }
    }
}

/// 弹框尺寸与「数据库备份」弹框对齐（600×480）。
const SQL_FILE_DIALOG_WIDTH: Pixels = px(600.);
const SQL_FILE_DIALOG_HEIGHT: Pixels = px(480.);

/// 构建「执行 SQL 文件」Dialog。builder 每帧被 `Root::render_dialog_layer` 重放，
/// 内容快照（form/task）从 Rc 克隆，保证与 NavicatMain 渲染 lease 隔离。
fn sql_file_build_dialog(
    dialog: Dialog,
    views: SqlFileDialogViews,
    colors: UiColors,
    window: &mut Window,
) -> Dialog {
    // 先取出视图快照并立即释放借用，避免构建子元素期间持有 Ref 借用
    let (form, log_task) = {
        let data = views.data.borrow();
        (data.form.clone(), data.log_task)
    };
    let task = log_task.and_then(|task_id| {
        views
            .data
            .borrow()
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
    });

    let (tab_bar, body, footer) = if let Some(task) = task {
        // 任务日志视图（从状态栏芯片重开，或执行后自动切换）
        (
            sql_file_dialog_tabs(&views, SqlFileExecutionTab::Log, true).flex_none(),
            sql_file_log_body(task.clone(), colors),
            sql_file_dialog_footer(
                false,
                Some((task.id, !task.running(), task.cancel_requested)),
                &views,
            ),
        )
    } else if let Some(form) = form {
        let active = form.active_tab;
        let can_execute = form.path.is_some();
        let body = match active {
            SqlFileExecutionTab::General => sql_file_general_body(form, &views, colors),
            SqlFileExecutionTab::Log => div()
                .flex_1()
                .min_h(px(0.))
                .py_4()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("执行后显示消息日志"),
        };
        (
            sql_file_dialog_tabs(&views, active, false).flex_none(),
            body,
            sql_file_dialog_footer(can_execute, None, &views),
        )
    } else {
        // 理论上不可达：form 与 task 同时为空说明弹框应当已关闭
        (
            sql_file_dialog_tabs(&views, SqlFileExecutionTab::General, false).flex_none(),
            div(),
            sql_file_dialog_footer(false, None, &views),
        )
    };

    // 备份弹框为全屏遮罩 flex 居中；Dialog 默认 y=视口高/10 偏顶，这里显式算 margin_top 垂直居中。
    // 窗口过矮时贴顶留 8px，避免负 margin 把弹框推出可视区
    let remaining = window.viewport_size().height - SQL_FILE_DIALOG_HEIGHT;
    let margin_top = if remaining > px(16.) { remaining / 2. } else { px(8.) };
    let on_close_data = views.data.clone();
    // 右上角关闭按钮：与备份弹框头部一致，直接走统一关闭 helper（清 Rc + close_dialog）
    let close_view = views.view.clone();
    dialog
        .title(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::FileSql, 18., colors.text))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("执行 SQL 文件"),
                )
                .child(div().flex_1())
                .child(
                    Button::new("sql-file-title-close")
                        .ghost()
                        .small()
                        .icon(IconName::Close)
                        .on_click(move |_, window, cx| {
                            let _ = close_view.update(cx, |this, cx| {
                                this.request_close_sql_file_dialog(window, cx);
                            });
                        }),
                ),
        )
        // 标题行已放显式关闭按钮，关闭内置悬浮 X，避免右上重复
        .close_button(false)
        .w(SQL_FILE_DIALOG_WIDTH)
        .h(SQL_FILE_DIALOG_HEIGHT)
        .margin_top(margin_top)
        // Enter 键会触发 Confirm → on_ok，默认返回 true 会直接关窗；路径输入框回车必须不关闭
        .on_ok(|_, _, _| false)
        // X / Esc / 点击遮罩 → Cancel → request_close → on_close：统一清理弹框状态
        .on_close(move |_, _, _| {
            let mut data = on_close_data.borrow_mut();
            data.form = None;
            data.log_task = None;
            data.dialog_open = false;
        })
        .child(tab_bar)
        .child(body)
        .footer(footer)
}

/// 常规 / 消息日志 两页签（gpui-component TabBar）。
/// 日志视图（form 为 None）时「常规」无内容可回退，点击保持日志页。
fn sql_file_dialog_tabs(
    views: &SqlFileDialogViews,
    active: SqlFileExecutionTab,
    log_view: bool,
) -> TabBar {
    let selected_index = if log_view {
        1
    } else {
        match active {
            SqlFileExecutionTab::General => 0,
            SqlFileExecutionTab::Log => 1,
        }
    };
    let view = views.view.clone();
    TabBar::new("sql-file-tabs")
        .segmented()
        .small()
        .selected_index(selected_index)
        .child(Tab::from("常规"))
        .child(Tab::from("消息日志"))
        .on_click(move |index, _window, cx| {
            let _ = view.update(cx, |this, cx| {
                if let Some(form) = &mut this.sql_file_modal.borrow_mut().form {
                    form.active_tab = if *index == 0 {
                        SqlFileExecutionTab::General
                    } else {
                        SqlFileExecutionTab::Log
                    };
                    cx.notify();
                }
            });
        })
}

fn sql_file_general_body(
    form: SqlFileExecutionForm,
    views: &SqlFileDialogViews,
    colors: UiColors,
) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .py_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(sql_file_path_field(views, colors))
        .child(
            div()
                .flex()
                .gap_6()
                .child(sql_file_select_field(
                    "连接",
                    views.connection_select.clone(),
                    "选择连接",
                    colors,
                ))
                .child(sql_file_select_field(
                    "数据库",
                    views.database_select.clone(),
                    "选择数据库",
                    colors,
                )),
        )
        .child(sql_file_select_field(
            "编码",
            views.encoding_select.clone(),
            "选择编码",
            colors,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(sql_file_section_label("选项", colors))
                .child(sql_file_check_row(
                    "遇到错误时继续",
                    form.continue_on_error,
                    SqlFileOption::ContinueOnError,
                    views,
                ))
                .child(sql_file_check_row(
                    "按分号拆分多个查询",
                    form.split_statements,
                    SqlFileOption::SplitStatements,
                    views,
                )),
        )
}

fn sql_file_path_field(views: &SqlFileDialogViews, colors: UiColors) -> Div {
    let path_input = views.path_input.clone();
    let view = views.view.clone();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(sql_file_section_label("文件", colors))
        .child(
            div()
                .flex()
                .gap_3()
                .child(
                    div()
                        .h(px(34.))
                        .flex_1()
                        .min_w(px(0.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .overflow_hidden()
                        .child(
                            Input::new(&path_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .text_size(px(13.)),
                        ),
                )
                .child(
                    Button::new("sql-file-browse")
                        .label("浏览")
                        .w(px(96.))
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.choose_sql_file_for_execution(window, cx);
                            });
                        }),
                ),
        )
}

fn sql_file_select_field<D>(
    label: &'static str,
    select: Entity<SelectState<SearchableVec<D>>>,
    placeholder: &'static str,
    colors: UiColors,
) -> Div
where
    D: SelectItem + 'static,
{
    div()
        .w(px(260.))
        .flex()
        .flex_col()
        .gap_2()
        .child(sql_file_section_label(label, colors))
        // 改用 Select 自带外观：触发器本身 flex + items_center，直接给 34px 高度，
        // 选中值/占位文字垂直居中。原「外框 + appearance(false)」时触发器只有内容高度
        // 且贴顶摆放，选中后文字看起来不居中。边框/背景/文字色随 ActiveTheme 双主题生效。
        .child(
            Select::new(&select)
                .placeholder(placeholder)
                .w_full()
                .h(px(34.))
                .menu_width(px(260.)),
        )
}

#[derive(Clone, Copy)]
enum SqlFileOption {
    ContinueOnError,
    SplitStatements,
}

fn sql_file_check_row(
    label: &'static str,
    checked: bool,
    option: SqlFileOption,
    views: &SqlFileDialogViews,
) -> Checkbox {
    let id = match option {
        SqlFileOption::ContinueOnError => "sql-file-continue-on-error",
        SqlFileOption::SplitStatements => "sql-file-split-statements",
    };
    let view = views.view.clone();
    Checkbox::new(id)
        .checked(checked)
        .label(label)
        .text_size(px(13.))
        .on_click(move |new_checked, _window, cx| {
            let value = *new_checked;
            let _ = view.update(cx, |this, cx| {
                if let Some(form) = &mut this.sql_file_modal.borrow_mut().form {
                    match option {
                        SqlFileOption::ContinueOnError => form.continue_on_error = value,
                        SqlFileOption::SplitStatements => form.split_statements = value,
                    }
                }
                cx.notify();
            });
        })
}

fn sql_file_section_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(label)
}

fn sql_file_log_body(task: SqlFileExecutionTaskState, colors: UiColors) -> Div {
    let elapsed = task
        .finished_at
        .unwrap_or_else(Instant::now)
        .saturating_duration_since(task.started_at)
        .as_millis();
    let mut list = div().flex().flex_col().gap_1().p_3();
    if let Some(error) = &task.error {
        list = list.child(sql_file_log_error(error.clone(), colors));
    } else if task.logs.is_empty() {
        list = list.child(
            div()
                .text_color(colors.muted)
                .text_size(px(13.))
                .child("正在执行，消息日志会实时追加"),
        );
    } else {
        let hidden_count = task.logs.len().saturating_sub(SQL_FILE_VISIBLE_LOG_LIMIT);
        if hidden_count > 0 {
            list = list.child(
                div()
                    .text_color(colors.muted)
                    .text_size(px(12.))
                    .px_2()
                    .py_1()
                    .child(format!(
                        "已隐藏前 {hidden_count} 条日志，完整日志可复制"
                    )),
            );
        }
        for entry in task.logs.iter().skip(hidden_count) {
            list = list.child(sql_file_log_row(entry, colors));
        }
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .py_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child(format!("连接: {}", task.connection_id.0))
                .child(format!(
                    "数据库: {}",
                    task.database.clone().unwrap_or_else(|| "默认数据库".to_string())
                )),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .overflow_hidden()
                .child(task.path.display().to_string()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_5()
                .text_size(px(14.))
                .child(format!("查询: {}", task.total.max(task.processed)))
                .child(format!("已处理: {}", task.processed))
                .child(format!("错误: {}", task.errors))
                .child(format!("状态: {}", sql_file_task_status(&task)))
                .child(format!("时间: {elapsed} ms")),
        )
        .child(sql_file_progress(&task))
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_y_scrollbar()
                .child(list),
        )
}

fn sql_file_log_error(error: String, colors: UiColors) -> Div {
    div()
        .rounded(colors.radius)
        .bg(if colors.is_dark { rgb(0x3a2020) } else { rgb(0xffeeee) })
        .p_2()
        .text_size(px(13.))
        .text_color(rgb(0xd64545))
        .child(error)
}

fn sql_file_log_row(entry: &SqlFileExecutionLogEntry, colors: UiColors) -> Div {
    let status = if entry.success { "成功" } else { "失败" };
    let status_color = if entry.success {
        rgb(0x22a06b)
    } else {
        rgb(0xd64545)
    };
    div()
        .rounded(colors.radius)
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_3()
        .text_size(px(13.))
        .hover(move |style| style.bg(colors.hover))
        .child(div().w(px(34.)).text_color(colors.muted).child(format!("#{}", entry.index)))
        .child(div().w(px(42.)).text_color(status_color).child(status))
        .child(
            div()
                .w(px(58.))
                .text_color(colors.muted)
                .child(format!("{} ms", entry.elapsed_ms)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .child(format!("{} · {}", entry.message, entry.sql)),
        )
}

/// 进度条（gpui-component Progress）：运行中且尚未处理任何语句时用不确定动画；有错误时红色。
fn sql_file_progress(task: &SqlFileExecutionTaskState) -> Progress {
    let total = task.total.max(task.processed).max(1);
    let running = task.running();
    let percent_value = if running {
        if task.processed == 0 {
            8.
        } else {
            task.processed as f32 / total as f32 * 100.
        }
    } else {
        100.
    };
    Progress::new("sql-file-progress")
        .value(percent_value)
        .loading(running && task.processed == 0)
        .color(if task.errors > 0 || task.error.is_some() {
            rgb(0xd64545)
        } else {
            rgb(0x1687ff)
        })
        .xsmall()
        .w_full()
}

/// 弹框页脚：自定义 footer 后 Dialog 不再渲染默认 OK/Cancel。
/// 顺序与备份弹框一致：「关闭」在最左，任务操作/主操作（执行）靠右。
/// 注意 Dialog 的 footer 外层已带 16px 左右内边距，这里不再重复 px_5。
fn sql_file_dialog_footer(
    can_execute: bool,
    log_task: Option<(u64, bool, bool)>,
    views: &SqlFileDialogViews,
) -> Div {
    let view = views.view.clone();
    div()
        .w_full()
        .h(px(54.))
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .child(
            Button::new("sql-file-close")
                .label("关闭")
                .small()
                .w(px(78.))
                .on_click({
                    let view = view.clone();
                    move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| {
                            this.request_close_sql_file_dialog(window, cx);
                        });
                    }
                }),
        )
        .when_some(log_task, |this, (task_id, finished, cancel_requested)| {
            this.child(
                Button::new("sql-file-copy-log")
                    .label("复制日志")
                    .small()
                    .w(px(92.))
                    .on_click({
                        let view = view.clone();
                        move |_, _window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.copy_sql_file_task_log(task_id, cx);
                            });
                        }
                    }),
            )
            .when(!finished, |this| {
                this.child(
                    Button::new("sql-file-cancel-task")
                        .label(if cancel_requested { "停止中" } else { "取消执行" })
                        .small()
                        .w(px(92.))
                        .disabled(cancel_requested)
                        .on_click({
                            let view = view.clone();
                            move |_, _window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.cancel_sql_file_task(task_id, cx);
                                });
                            }
                        }),
                )
            })
            .when(finished, |this| {
                this.child(
                    Button::new("sql-file-clear-task")
                        .label("清除任务")
                        .small()
                        .w(px(92.))
                        .on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.clear_sql_file_task(task_id, window, cx);
                                });
                            }
                        }),
                )
            })
        })
        .when(log_task.is_none(), |this| {
            this.child(
                Button::new("sql-file-execute")
                    .label("执行")
                    .primary()
                    .small()
                    .w(px(86.))
                    .disabled(!can_execute)
                    .on_click({
                        let view = view.clone();
                        move |_, _window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.confirm_sql_file_execution(cx);
                            });
                        }
                    }),
            )
        })
}

fn sql_file_statusbar_area(
    tasks: &[SqlFileExecutionTaskState],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(task) = tasks.iter().rev().find(|task| task.running()).or_else(|| tasks.last()) else {
        return div().w(px(180.));
    };
    let running_count = tasks.iter().filter(|task| task.running()).count();
    let label = if running_count > 1 {
        format!("{running_count} 个任务运行中")
    } else if task.cancel_requested {
        format!("正在停止 {}/{}", task.processed, task.total)
    } else if task.running() {
        format!("执行中 {}/{}", task.processed, task.total)
    } else if task.canceled {
        format!("已取消 {}/{}", task.processed, task.total)
    } else if task.errors > 0 || task.error.is_some() {
        format!("{}  {} 失败", task.file_name, task.errors.max(1))
    } else {
        format!("{} 执行完成", task.file_name)
    };
    let task_id = task.id;
    div()
        .w(px(260.))
        .h_full()
        .flex()
        .items_center()
        .justify_end()
        .pr_2()
        .child(
            div()
                .max_w(px(248.))
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
                    cx.listener(move |this, _, window, cx| {
                        this.show_sql_file_task_log(task_id, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(
                    if task.running() { AppIcon::Play } else { AppIcon::FileSql },
                    13.,
                    if task.errors > 0 || task.error.is_some() {
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

fn sql_file_connection_items(state: &AppState) -> Vec<SqlFileConnectionItem> {
    state
        .connections
        .iter()
        .map(|connection| SqlFileConnectionItem {
            label: connection.config.name.clone(),
            value: connection.config.id,
        })
        .collect()
}

fn sql_file_database_items(
    state: &AppState,
    connection_id: ConnectionId,
) -> Vec<SqlFileDatabaseItem> {
    let mut items = vec![SqlFileDatabaseItem {
        label: "默认数据库".to_string(),
        value: None,
    }];
    if let Some(connection) = state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
    {
        items.extend(all_connection_database_names(connection).into_iter().map(|database| {
            SqlFileDatabaseItem {
                label: database.clone(),
                value: Some(database),
            }
        }));
    }
    items
}

fn sql_file_encoding_items() -> Vec<SqlFileEncodingItem> {
    [
        SqlFileEncoding::Utf8,
        SqlFileEncoding::Utf8Bom,
        SqlFileEncoding::Gbk,
        SqlFileEncoding::Gb18030,
        SqlFileEncoding::Utf16Le,
        SqlFileEncoding::Utf16Be,
    ]
    .into_iter()
    .map(|encoding| SqlFileEncodingItem {
        label: encoding.label().to_string(),
        value: encoding,
    })
    .collect()
}

fn sql_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("query.sql")
        .to_string()
}

fn sql_file_preview(sql: &str) -> String {
    let text = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 90 {
        format!("{}...", text.chars().take(90).collect::<String>())
    } else {
        text
    }
}

fn sql_file_task_message(task: &SqlFileExecutionTaskState) -> String {
    if task.canceled {
        return format!(
            "{} 已取消，已处理 {}/{}",
            task.file_name, task.processed, task.total
        );
    }
    if task.error.is_some() {
        return format!("{} 执行失败", task.file_name);
    }
    if task.errors > 0 {
        format!("{} 执行完成，{} 条失败", task.file_name, task.errors)
    } else {
        format!("{} 执行完成", task.file_name)
    }
}

fn sql_file_execution_statement_count(text: &str, split_statements: bool) -> usize {
    if !split_statements {
        return 1;
    }
    sql_editor_adapter::build_statement_runs(text).len()
}

async fn read_sql_file(path: PathBuf, encoding: SqlFileEncoding) -> io::Result<String> {
    let bytes = fs::read(path)?;
    match encoding {
        SqlFileEncoding::Utf8 | SqlFileEncoding::Utf8Bom => String::from_utf8(bytes)
            .map(|text| text.trim_start_matches('\u{feff}').to_string())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        SqlFileEncoding::Gbk => Ok(decode_sql_file_bytes(&encoding_rs::GBK, &bytes)),
        SqlFileEncoding::Gb18030 => Ok(decode_sql_file_bytes(&encoding_rs::GB18030, &bytes)),
        SqlFileEncoding::Utf16Le => Ok(decode_sql_file_bytes(&encoding_rs::UTF_16LE, &bytes)),
        SqlFileEncoding::Utf16Be => Ok(decode_sql_file_bytes(&encoding_rs::UTF_16BE, &bytes)),
    }
}

fn decode_sql_file_bytes(encoding: &'static encoding_rs::Encoding, bytes: &[u8]) -> String {
    let (text, _, _) = encoding.decode(bytes);
    text.trim_start_matches('\u{feff}').to_string()
}

fn sql_file_task_log_text(task: &SqlFileExecutionTaskState) -> String {
    let elapsed = task
        .finished_at
        .unwrap_or_else(Instant::now)
        .saturating_duration_since(task.started_at)
        .as_millis();
    let mut text = format!(
        "文件: {}\n路径: {}\n连接: {}\n数据库: {}\n查询: {}\n已处理: {}\n错误: {}\n时间: {elapsed} ms\n",
        task.file_name,
        task.path.display(),
        task.connection_id.0,
        task.database.clone().unwrap_or_else(|| "默认数据库".to_string()),
        task.total.max(task.processed),
        task.processed,
        task.errors,
    );
    if let Some(error) = &task.error {
        text.push_str(&format!("错误: {error}\n"));
    }
    if task.canceled {
        text.push_str("状态: 已取消\n");
    } else if task.cancel_requested {
        text.push_str("状态: 正在停止\n");
    }
    for entry in &task.logs {
        let status = if entry.success { "成功" } else { "失败" };
        text.push_str(&format!(
            "#{} {status} {} ms {} | {}\n",
            entry.index, entry.elapsed_ms, entry.message, entry.sql
        ));
    }
    text
}

fn sql_file_task_status(task: &SqlFileExecutionTaskState) -> &'static str {
    if task.canceled {
        "已取消"
    } else if task.cancel_requested {
        "正在停止"
    } else if task.running() {
        "执行中"
    } else {
        "已完成"
    }
}
