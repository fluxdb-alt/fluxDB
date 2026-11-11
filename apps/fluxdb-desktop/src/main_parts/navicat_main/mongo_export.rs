// MongoDB 集合导出：与表数据导出同一套外壳（常规 / 消息日志）。
//
// 布局对齐 Compass：条件不单独占页签，常规页选「当前筛选条件」后直接展示对应的 shell 查询，
// 条件值取集合工作台已生效的 filter/projection/sort（只读，要改回工作台改）。
// 分页用 `mongo_find_page` 按 skip 翻页，写入先落 `.partial` 临时文件，成功后改名，
// 取消或失败不留半截文件；取消标记每页检查一次。

/// 每页拉取条数；连接器还会再夹一次上限（`MONGO_MAX_DOCUMENTS_PER_QUERY`）。
const MONGO_EXPORT_PAGE_SIZE: u64 = 500;

impl NavicatMain {
    /// 右键「导出」：按集合工作台已生效的查询预填条件（与 Compass 一致，条件只读展示）。
    fn open_mongo_export_dialog(&mut self, object_path: ObjectPath, _window: &mut Window, cx: &mut Context<Self>) {
        let (query, object) = self.mongo_export_seed(&object_path);
        self.pending_mongo_export = Some(MongoExportForm {
            object,
            format: MongoExportFormat::Json,
            scope: MongoExportScope::All,
            active_tab: MongoExportTab::General,
            filter: query.filter,
            projection: query.projection,
            sort: query.sort,
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            exported_documents: 0,
            logs: Vec::new(),
            error: None,
            finished: false,
            task_center_id: None,
        });
        cx.notify();
    }

    /// 条件预填来源：已打开的集合标签页用其已生效查询，否则用默认草稿。
    fn mongo_export_seed(&self, object_path: &ObjectPath) -> (MongoQueryDraft, ObjectPath) {
        let tab = self.controller.state().tabs.iter().find(|tab| match &tab.kind {
            TabKind::MongoCollection(state) => state.object == *object_path,
            _ => false,
        });
        match tab.map(|tab| &tab.kind) {
            Some(TabKind::MongoCollection(state)) => (state.applied_query.clone(), state.object.clone()),
            _ => (
                MongoQueryDraft::new(self.controller.state().settings.page_size),
                object_path.clone(),
            ),
        }
    }

    fn select_mongo_export_tab(&mut self, tab: MongoExportTab, cx: &mut Context<Self>) {
        if let Some(form) = self.pending_mongo_export.as_mut() {
            form.active_tab = tab;
            cx.notify();
        }
    }

    fn set_mongo_export_format(&mut self, format: MongoExportFormat, cx: &mut Context<Self>) {
        if let Some(form) = self.pending_mongo_export.as_mut() {
            form.format = format;
            cx.notify();
        }
    }

    fn set_mongo_export_scope(&mut self, scope: MongoExportScope, cx: &mut Context<Self>) {
        if let Some(form) = self.pending_mongo_export.as_mut() {
            form.scope = scope;
            cx.notify();
        }
    }

    fn cancel_mongo_export_dialog(&mut self, cx: &mut Context<Self>) {
        if self
            .pending_mongo_export
            .as_ref()
            .is_some_and(|form| form.running)
        {
            self.show_message(
                tr("app.mongo.export.running_no_close"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.pending_mongo_export = None;
        cx.notify();
    }

    fn request_cancel_mongo_export(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_mongo_export.as_mut() else {
            return;
        };
        if !form.running {
            return;
        }
        form.cancel.store(true, Ordering::Relaxed);
        if let Some(id) = form.task_center_id {
            self.task_center.mark_cancel_requested(id);
        }
        self.show_message(
            tr("app.data_export.cancel_requested"),
            AppMessageKind::Warning,
            cx,
        );
        cx.notify();
    }

    /// 选路径后开始导出；条件文本在这里读一次，之后后台只认这份快照。
    fn confirm_mongo_export(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_mongo_export.clone() else {
            return;
        };
        if form.running || self._mongo_export_task.is_some() {
            return;
        }
        // 条件在这里定稿：之后后台只认这份快照，弹框里的表单再变也不影响已在跑的导出。
        let query = match form.scope {
            MongoExportScope::Conditions => MongoExportQuery {
                filter: form.filter.clone(),
                projection: form.projection.clone(),
                sort: form.sort.clone(),
            },
            MongoExportScope::All => MongoExportQuery::default(),
        };
        let suggested = mongo_export_suggested_name(&form.object, form.format);
        let receiver = cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested));
        let cancel = form.cancel.clone();
        cancel.store(false, Ordering::Relaxed);
        let task_center_id = self.task_center.begin(
            TaskCenterKind::MongoExport,
            tr("app.task_center.title.mongo_export")
                .replace("%{collection}", &form.object.name),
            mongo_export_object_label(&form.object),
            Some(cancel.clone()),
        );
        if let Some(pending) = self.pending_mongo_export.as_mut() {
            pending.running = true;
            pending.finished = false;
            pending.error = None;
            pending.exported_documents = 0;
            pending.task_center_id = Some(task_center_id);
            pending.active_tab = MongoExportTab::Log;
            pending.logs = vec![ImportDialogLogEntry {
                stage: tr("app.mongo.export.stage.prepare").to_string(),
                message: tr("app.mongo.export.log_started")
                    .replace("%{collection}", &form.object.name)
                    .replace("%{format}", form.format.label()),
                success: true,
                elapsed_ms: 0,
            }];
        }
        let task = self.spawn_mongo_export(receiver, form, query, cancel, cx);
        self._mongo_export_task = Some(task);
        cx.notify();
    }

    fn spawn_mongo_export(
        &mut self,
        receiver: futures::channel::oneshot::Receiver<anyhow::Result<Option<PathBuf>>>,
        form: MongoExportForm,
        query: MongoExportQuery,
        cancel: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let controller = self.controller.clone();
        cx.spawn(async move |view, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => mongo_export_safe_path(path, form.format),
                Ok(Ok(None)) => {
                    mongo_export_reset_on_ui(&view, cx, false);
                    return;
                }
                Ok(Err(error)) => {
                    mongo_export_log_on_ui(
                        &view,
                        cx,
                        false,
                        tr("app.data_export.choose_path_failed")
                            .replace("%{error}", &error.to_string()),
                    );
                    mongo_export_reset_on_ui(&view, cx, false);
                    return;
                }
                Err(error) => {
                    mongo_export_log_on_ui(
                        &view,
                        cx,
                        false,
                        tr("app.data_export.choose_path_failed")
                            .replace("%{error}", &error.to_string()),
                    );
                    mongo_export_reset_on_ui(&view, cx, false);
                    return;
                }
            };
            mongo_export_log_on_ui(
                &view,
                cx,
                true,
                tr("app.mongo.export.log_path")
                    .replace("%{path}", &path.display().to_string()),
            );
            let (sender, receiver) = mpsc::channel::<MongoExportProgress>();
            let result = cx
                .background_spawn({
                    let path = path.clone();
                    async move {
                        run_mongo_export(controller, form, query, path, cancel, sender)
                    }
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok(progress) = receiver.try_recv() {
                    mongo_export_progress_on_ui(&view, cx, progress);
                }
                if let Some(result) = result.as_mut().now_or_never() {
                    while let Ok(progress) = receiver.try_recv() {
                        mongo_export_progress_on_ui(&view, cx, progress);
                    }
                    mongo_export_finish_on_ui(&view, cx, result);
                    break;
                }
                smol::Timer::after(Duration::from_millis(100)).await;
            }
        })
    }
}

#[derive(Clone, Debug)]
struct MongoExportProgress {
    documents: u64,
    elapsed_ms: u64,
    message: String,
}

struct MongoExportResult {
    documents: u64,
    canceled: bool,
}

/// 一次导出使用的查询条件（Extended JSON 文本；空表示不限制）。
#[derive(Clone, Debug, Default)]
struct MongoExportQuery {
    filter: String,
    projection: String,
    sort: String,
}

fn mongo_export_suggested_name(object: &ObjectPath, format: MongoExportFormat) -> String {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    format!(
        "{}-{timestamp}.{}",
        safe_data_export_filename_segment(&object.name),
        format.extension()
    )
}

fn mongo_export_safe_path(path: PathBuf, format: MongoExportFormat) -> PathBuf {
    safe_data_export_path_with_extension(path, format.extension())
}

/// 标题旁的目标标签：带上库名，避免同名集合分不清来源。
fn mongo_export_object_label(object: &ObjectPath) -> String {
    match object.database.as_deref() {
        Some(database) if !database.is_empty() => format!("{database}.{}", object.name),
        _ => object.name.clone(),
    }
}

/// 后台导出：按 skip 翻页取全量，逐页写文件；每页前检查取消标记。
///
/// CSV 需要先求全量列并集，因此走两段：先把整份结果落到一个临时 EJSONL 文件并收集列，
/// 再按固定列序写目标文件（与 Compass 的 DOWNLOAD / WRITE 两阶段一致）。
fn run_mongo_export(
    controller: AppController,
    form: MongoExportForm,
    query: MongoExportQuery,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<MongoExportProgress>,
) -> anyhow::Result<MongoExportResult> {
    let started = Instant::now();
    let temp = table_data_export_temp_path(&path);
    let write_result = (|| -> anyhow::Result<MongoExportResult> {
        if form.format == MongoExportFormat::Csv {
            return run_mongo_export_csv(&controller, &form, &query, &temp, &cancel, &sender, started);
        }
        let file = fs::File::create(&temp)?;
        let mut writer = BufWriter::new(file);
        if form.format == MongoExportFormat::Json {
            writer.write_all(b"[\n")?;
        }
        let mut skip = 0u64;
        let mut documents = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(MongoExportResult { documents, canceled: true });
            }
            let page = mongo_export_fetch_page(&controller, &form, &query, skip, &cancel)?;
            for document in &page.documents {
                if form.format == MongoExportFormat::Json {
                    if documents > 0 {
                        writer.write_all(b",\n")?;
                    }
                    writer.write_all(document.json.as_bytes())?;
                } else {
                    // JSONL：一条文档一行，压掉换行避免破坏「一行一记录」。
                    writer.write_all(document.json.replace('\n', " ").as_bytes())?;
                    writer.write_all(b"\n")?;
                }
                documents += 1;
            }
            if !mongo_export_advance(&page, &mut skip) {
                break;
            }
            mongo_export_report_progress(&sender, &started, documents, "log_page");
        }
        if form.format == MongoExportFormat::Json {
            writer.write_all(b"\n]\n")?;
        }
        writer.flush()?;
        drop(writer);
        Ok(MongoExportResult { documents, canceled: false })
    })();

    match write_result {
        Ok(result) if !result.canceled => {
            fs::rename(&temp, &path)?;
            Ok(result)
        }
        Ok(result) => {
            let _ = fs::remove_file(&temp);
            Ok(result)
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

fn mongo_export_fetch_page(
    controller: &AppController,
    form: &MongoExportForm,
    query: &MongoExportQuery,
    skip: u64,
    cancel: &Arc<AtomicBool>,
) -> anyhow::Result<fluxdb_core::MongoDocumentsPage> {
    let request = fluxdb_core::MongoFindRequest {
        collection: form.object.name.clone(),
        filter: query.filter.clone(),
        projection: query.projection.clone(),
        sort: query.sort.clone(),
        skip,
        limit: MONGO_EXPORT_PAGE_SIZE,
        ..Default::default()
    };
    Ok(controller.mongo_find_page(&form.object, &request, &|| cancel.load(Ordering::Relaxed))?)
}

/// 推进 skip；返回 `false` 表示已到末尾。
fn mongo_export_advance(page: &fluxdb_core::MongoDocumentsPage, skip: &mut u64) -> bool {
    if page.returned == 0 || !page.has_more {
        return false;
    }
    *skip += page.returned;
    true
}

fn mongo_export_report_progress(
    sender: &mpsc::Sender<MongoExportProgress>,
    started: &Instant,
    documents: u64,
    key: &str,
) {
    let _ = sender.send(MongoExportProgress {
        documents,
        elapsed_ms: started.elapsed().as_millis() as u64,
        message: tr(match key {
            "log_download" => "app.mongo.export.log_download",
            _ => "app.mongo.export.log_page",
        })
        .replace("%{count}", &documents.to_string()),
    });
}

/// CSV 第一段：翻页拉全量，写临时 EJSONL 文件并记录列并集（每行一条紧凑文档）。
fn run_mongo_export_csv(
    controller: &AppController,
    form: &MongoExportForm,
    query: &MongoExportQuery,
    temp: &Path,
    cancel: &Arc<AtomicBool>,
    sender: &mpsc::Sender<MongoExportProgress>,
    started: Instant,
) -> anyhow::Result<MongoExportResult> {
    let raw = temp.with_extension("ejsonl");
    let result = (|| -> anyhow::Result<MongoExportResult> {
        let mut columns = MongoCsvColumns::default();
        let mut skip = 0u64;
        let mut documents = 0u64;
        {
            let mut writer = BufWriter::new(fs::File::create(&raw)?);
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(MongoExportResult { documents, canceled: true });
                }
                let page = mongo_export_fetch_page(controller, form, query, skip, cancel)?;
                for document in &page.documents {
                    // 展示用的 JSON 是格式化文本；解析后用紧凑序列化写一行，便于逐行回读。
                    // 不能按空白字符压缩——那会把字符串值内部的空格/换行一起吃掉。
                    let value: serde_json::Value = serde_json::from_str(&document.json)?;
                    columns.add_document(&value);
                    writer.write_all(serde_json::to_string(&value)?.as_bytes())?;
                    writer.write_all(b"\n")?;
                    documents += 1;
                }
                if !mongo_export_advance(&page, &mut skip) {
                    break;
                }
                mongo_export_report_progress(sender, &started, documents, "log_download");
            }
            writer.flush()?;
        }
        {
            let mut writer = BufWriter::new(fs::File::create(temp)?);
            // 与表数据导出保持一致：带 UTF-8 BOM，Windows Excel 打开中文不乱码。
            write_csv_bom(&mut writer)?;
            // 空结果也写一个只有表头的文件（列为空时就是一行空表头），不留零字节文件。
            writer.write_all(columns.header_names().join(",").as_bytes())?;
            writer.write_all(b"\n")?;
            mongo_csv_read_rows(&raw, |value| {
                let row = columns.row(&value);
                writer.write_all(row.join(",").as_bytes())?;
                writer.write_all(b"\n")?;
                Ok(())
            })?;
            writer.flush()?;
        }
        Ok(MongoExportResult { documents, canceled: false })
    })();
    let _ = fs::remove_file(&raw);
    result
}

fn mongo_export_log_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    success: bool,
    message: String,
) {
    cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            let task_center_id = this
                .pending_mongo_export
                .as_ref()
                .and_then(|form| form.task_center_id);
            if let Some(form) = this.pending_mongo_export.as_mut() {
                form.logs.push(ImportDialogLogEntry {
                    stage: tr("app.mongo.export.stage.write").to_string(),
                    message: message.clone(),
                    success,
                    elapsed_ms: 0,
                });
            }
            if let Some(id) = task_center_id {
                this.task_center.update_progress(
                    id,
                    None,
                    Some(tr("app.mongo.export.stage.write").to_string()),
                    Some(message),
                );
            }
            cx.notify();
        });
    });
}

fn mongo_export_progress_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    progress: MongoExportProgress,
) {
    cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            let task_center_id = this
                .pending_mongo_export
                .as_ref()
                .and_then(|form| form.task_center_id);
            if let Some(form) = this.pending_mongo_export.as_mut() {
                form.exported_documents = progress.documents;
                form.logs.push(ImportDialogLogEntry {
                    stage: tr("app.mongo.export.stage.page").to_string(),
                    message: progress.message.clone(),
                    success: true,
                    elapsed_ms: progress.elapsed_ms,
                });
            }
            if let Some(id) = task_center_id {
                this.task_center.update_progress(
                    id,
                    None,
                    Some(tr("app.mongo.export.stage.page").to_string()),
                    Some(progress.message),
                );
            }
            cx.notify();
        });
    });
}

/// 选路径被取消：退出运行态，不改日志结论。
fn mongo_export_reset_on_ui(view: &WeakEntity<NavicatMain>, cx: &mut gpui::AsyncApp, finished: bool) {
    cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            this._mongo_export_task = None;
            let task_center_id = this
                .pending_mongo_export
                .as_ref()
                .and_then(|form| form.task_center_id);
            if let Some(form) = this.pending_mongo_export.as_mut() {
                form.running = false;
                form.finished |= finished;
                form.task_center_id = None;
            }
            if let Some(id) = task_center_id {
                this.finish_task_center_record(
                    id,
                    TaskCenterStatus::Canceled,
                    tr("app.task_center.status.canceled").to_string(),
                    None,
                );
            }
            cx.notify();
        });
    });
}

fn mongo_export_finish_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    result: anyhow::Result<MongoExportResult>,
) {
    cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            this._mongo_export_task = None;
            let Some(form) = this.pending_mongo_export.as_mut() else {
                return;
            };
            let task_center_id = form.task_center_id.take();
            form.running = false;
            form.finished = true;
            match result {
                Ok(export) => {
                    form.exported_documents = export.documents;
                    let (message, kind) = if export.canceled {
                        (
                            tr("app.data_export.toast.cancelled").to_string(),
                            AppMessageKind::Warning,
                        )
                    } else {
                        (
                            tr("app.data_export.toast.completed")
                                .replace("%{count}", &export.documents.to_string()),
                            AppMessageKind::Success,
                        )
                    };
                    form.logs.push(ImportDialogLogEntry {
                        stage: tr("app.mongo.export.stage.done").to_string(),
                        message: message.clone(),
                        success: true,
                        elapsed_ms: 0,
                    });
                    if let Some(id) = task_center_id {
                        this.finish_task_center_record(
                            id,
                            if export.canceled {
                                TaskCenterStatus::Canceled
                            } else {
                                TaskCenterStatus::Success
                            },
                            tr("app.mongo.export.stage.done").to_string(),
                            None,
                        );
                    }
                    this.show_message(message, kind, cx);
                }
                Err(error) => {
                    let message = error.to_string();
                    form.error = Some(message.clone());
                    form.logs.push(ImportDialogLogEntry {
                        stage: tr("app.mongo.export.stage.done").to_string(),
                        message: message.clone(),
                        success: false,
                        elapsed_ms: 0,
                    });
                    if let Some(id) = task_center_id {
                        this.finish_task_center_record(
                            id,
                            TaskCenterStatus::Failed,
                            tr("app.task_center.status.failed").to_string(),
                            Some(message.clone()),
                        );
                    }
                    this.show_message(message, AppMessageKind::Error, cx);
                }
            }
            cx.notify();
        });
    });
}

/// 导出弹框：沿用表数据导出的外壳尺寸与三页签样式。
fn mongo_export_modal(
    form: MongoExportForm,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let active = form.active_tab;
    // 全量导出只有格式与范围两行，没有查询预览：沿用 560 的固定高度会空出大半个弹框。
    // 按内容收紧到刚好容纳「标题 + 页签 + 两行选项 + 提示 + 底栏」；条件导出和日志页
    // 分别有查询预览与滚动的消息列表，仍用满高。
    let height = if active == MongoExportTab::General && form.scope == MongoExportScope::All {
        px(320.)
    } else {
        px(560.)
    };
    let body = match active {
        MongoExportTab::General => mongo_export_general_body(&form, colors, cx),
        MongoExportTab::Log => import_dialog_log_body(
            ImportDialogLogView {
                running: form.running,
                finished: form.finished,
                imported_label: tr("app.mongo.export.log_documents")
                    .replace("%{count}", &form.exported_documents.to_string()),
                started_at: None,
                finished_at: None,
                logs: &form.logs,
            },
            colors,
        ),
    };
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.58)
        } else {
            opaque_grey(0.75, 0.28)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_mongo_export_dialog(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(780.))
                .max_w(px(780.))
                .h(height)
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(menu_surface_bg(colors))
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .flex()
                .flex_col()
                .overflow_hidden()
                .text_size(ui_text_size(13.))
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .debug_selector(|| "mongo-export-modal".into())
                .key_context("MongoExportModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_mongo_export_dialog(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(mongo_export_header(&form, colors, cx))
                .child(mongo_export_tabs(active, colors, cx))
                .child(body)
                .child(mongo_export_footer(&form, colors, cx)),
        )
}

fn mongo_export_header(form: &MongoExportForm, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .h(px(56.))
        .flex_none()
        .px_5()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2p5()
                .min_w(px(0.))
                .child(app_icon(AppIcon::Save, 17., colors.text))
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_text_size(16.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.mongo.export.title")),
                )
                // 目标集合做成胶囊标签，和标题拉开层次，也不占标题的视觉重量。
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .px_2()
                        .py(px(2.))
                        .rounded(colors.radius)
                        .bg(colors.panel_alt)
                        .font_family(EDITOR_FONT)
                        .text_size(ui_text_size(11.5))
                        .text_color(colors.muted)
                        .child(mongo_export_object_label(&form.object)),
                ),
        )
        .child(
            div()
                .size(px(30.))
                .rounded(colors.radius)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 16., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_mongo_export_dialog(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn mongo_export_tabs(active: MongoExportTab, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .px_5()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(mongo_export_tab_button(
            tr("app.data_export.tab.general"),
            active == MongoExportTab::General,
            MongoExportTab::General,
            colors,
            cx,
        ))
        .child(mongo_export_tab_button(
            tr("app.data_export.tab.log"),
            active == MongoExportTab::Log,
            MongoExportTab::Log,
            colors,
            cx,
        ))
}

fn mongo_export_tab_button(
    label: impl Into<SharedString>,
    active: bool,
    tab: MongoExportTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    div()
        .debug_selector(move || format!("mongo-export-tab:{tab:?}"))
        .h(px(34.))
        .px_3()
        .flex()
        .items_center()
        .border_b_2()
        .border_color(if active { rgb(0x1687ff) } else { colors.border_soft })
        .text_size(ui_text_size(13.))
        .text_color(if active { colors.text } else { colors.muted })
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.select_mongo_export_tab(tab, cx);
                cx.stop_propagation();
            }),
        )
        .child(label)
}

fn mongo_export_general_body(
    form: &MongoExportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let running = form.running;
    let mut formats = div().flex().gap_2();
    for format in MongoExportFormat::all() {
        let selected = form.format == *format;
        formats = formats.child(
            div()
                .debug_selector(move || format!("mongo-export-format:{}", format.label()))
                .h(px(28.))
                .px_3()
                .rounded(colors.radius)
                .border_1()
                .border_color(if selected { rgb(0x1687ff) } else { colors.border })
                .bg(if selected {
                    if colors.is_dark { rgb(0x16345f) } else { rgb(0xe8f2ff) }
                } else {
                    colors.input_bg
                })
                .flex()
                .items_center()
                .justify_center()
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if selected { colors.text } else { colors.muted })
                .cursor_pointer()
                .when(running, |this| this.opacity(0.6))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if !running {
                            this.set_mongo_export_format(*format, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(format.label()),
        );
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .px_5()
        .py_4()
        .flex()
        .flex_col()
        .gap_3()
        // 格式与范围并排一行：两者都是二级选择，不需要各自独占一段标题。
        .child(
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_size(ui_text_size(12.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(tr("app.mongo.export.format")),
                )
                .child(formats)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(ui_text_size(11.5))
                        .text_color(colors.muted)
                        .child(tr(format_hint_key(form.format))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_size(ui_text_size(12.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(tr("app.data_export.scope")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .h(px(32.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .p_1()
                        .flex()
                        .gap_1()
                        .child(mongo_export_scope_segment(
                            tr("app.mongo.export.scope_all"),
                            form.scope == MongoExportScope::All,
                            MongoExportScope::All,
                            running,
                            colors,
                            cx,
                        ))
                        .child(mongo_export_scope_segment(
                            tr("app.data_export.scope_current"),
                            form.scope == MongoExportScope::Conditions,
                            MongoExportScope::Conditions,
                            running,
                            colors,
                            cx,
                        )),
                ),
        )
        .child(mongo_export_scope_note(form, colors))
        // 与 Compass 一致：只有「当前筛选条件」才展示查询预览；全量是 find({})，
        // 展示一条固定的空查询既无信息量，也让人误以为还要配条件。
        .when(form.scope == MongoExportScope::Conditions, |this| {
            this.child(mongo_export_query_preview(form, colors, cx))
        })
}

fn format_hint_key(format: MongoExportFormat) -> &'static str {
    match format {
        MongoExportFormat::Json => "app.mongo.export.format_hint_json",
        MongoExportFormat::Csv => "app.mongo.export.format_hint_csv",
        MongoExportFormat::JsonLines => "app.mongo.export.format_hint_jsonl",
    }
}

/// 范围下方的解释行：说明条件从哪来，并把工作台实际生效的 filter / projection / sort
/// 做成小标签列出来，用户一眼能确认「当前筛选条件」到底带了什么。
fn mongo_export_scope_note(form: &MongoExportForm, colors: UiColors) -> Div {
    let mut note = div()
        // 与上方两行标签列（64px + 16px 间距）左对齐。
        .pl(px(80.))
        .flex()
        .flex_col()
        .gap_1()
        .text_size(ui_text_size(11.5))
        .text_color(colors.muted)
        .child(tr("app.mongo.export.scope_hint"));
    if form.scope != MongoExportScope::Conditions {
        return note;
    }
    // 空串与 `{}` 都按「未填写」处理，与工作台执行时的口径一致。
    let conditions = [
        (tr("app.mongo.query.filter"), form.filter.trim()),
        (tr("app.mongo.query.projection"), form.projection.trim()),
        (tr("app.mongo.query.sort"), form.sort.trim()),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty() && *value != "{}")
    .map(|(label, _)| label)
    .collect::<Vec<_>>();
    if conditions.is_empty() {
        return note.child(
            div()
                .text_size(ui_text_size(11.5))
                .text_color(colors.muted)
                .child(tr("app.mongo.export.conditions_empty")),
        );
    }
    let mut chips = div().flex().items_center().gap_1();
    for label in conditions {
        chips = chips.child(
            div()
                .px_2()
                .py(px(1.))
                .rounded(colors.radius)
                .bg(colors.panel_alt)
                .text_size(ui_text_size(10.5))
                .text_color(colors.text)
                .child(label),
        );
    }
    note = note.child(chips);
    note
}

/// 导出范围对应的 shell 查询预览（与 Compass 的「Export results from the query below」一致）。
/// 全量导出展示 `find({})`；条件导出把 filter / projection / sort 拼进同一条语句。
fn mongo_export_query_preview(
    form: &MongoExportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let statement = mongo_export_query_statement(form);

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(data_export_section_label(
                    tr("app.mongo.export.query_preview"),
                    colors,
                ))
                // 只读预览也允许复制：用户常要把这条语句贴到 mongosh 里核对条数。
                .child(
                    div()
                        .debug_selector(|| "mongo-export-query-copy".into())
                        .size(px(24.))
                        .rounded(colors.radius)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(colors.hover))
                        .child(app_icon(AppIcon::Copy, 13., colors.muted))
                        .on_mouse_down(
                            MouseButton::Left,
                            {
                                let copy_text = statement.clone();
                                cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_text.clone(),
                                    ));
                                    this.show_message(
                                        tr("app.mongo.copy_done").as_ref(),
                                        AppMessageKind::Success,
                                        cx,
                                    );
                                    cx.stop_propagation();
                                })
                            },
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(tr("app.mongo.export.query_hint")),
                ),
        )
        .child(
            div()
                .debug_selector(|| "mongo-export-query-preview".into())
                .flex_1()
                .min_h(px(72.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border_soft)
                .bg(colors.panel_alt)
                .px_3()
                .py_2()
                .overflow_x_scrollbar()
                .font_family(EDITOR_FONT)
                .text_size(ui_text_size(12.))
                .line_height(ui_text_size(19.))
                .text_color(colors.text)
                // 长条件不折行：横向滚动，语句结构保持一行可读，也不把弹框撑高。
                .whitespace_nowrap()
                .child(statement),
        )
}

/// 拼出预览用的 shell 语句；全量与条件导出共用，复制与展示读同一份结果。
fn mongo_export_query_statement(form: &MongoExportForm) -> String {
    let (filter, projection, sort) = match form.scope {
        MongoExportScope::All => ("{}", "", ""),
        MongoExportScope::Conditions => (
            form.filter.trim(),
            form.projection.trim(),
            form.sort.trim(),
        ),
    };
    let mut statement = format!("db.getCollection(\"{}\").find(", form.object.name);
    statement.push_str(if filter.is_empty() { "{}" } else { filter });
    if !projection.is_empty() {
        statement.push_str(", ");
        statement.push_str(projection);
    }
    statement.push(')');
    if !sort.is_empty() {
        statement.push_str(".sort(");
        statement.push_str(sort);
        statement.push(')');
    }
    statement.push(';');
    statement
}

fn mongo_export_scope_segment(
    label: impl Into<SharedString>,
    active: bool,
    scope: MongoExportScope,
    running: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    div()
        .flex_1()
        .h_full()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .justify_center()
        .text_size(ui_text_size(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if active { colors.text } else { colors.muted })
        // 选中态与格式胶囊一致：淡蓝底 + 强调色描边；未选中保留同宽透明边框，
        // 切换时文字不因边框出现/消失而抖动。
        .border_1()
        .border_color(if active { rgb(0x1687ff) } else { colors.input_bg })
        .bg(if active {
            if colors.is_dark { rgb(0x16345f) } else { rgb(0xe8f2ff) }
        } else {
            colors.input_bg
        })
        .cursor_pointer()
        .hover(move |style| {
            style.bg(if active {
                if colors.is_dark { rgb(0x16345f) } else { rgb(0xe8f2ff) }
            } else {
                colors.hover
            })
        })
        .when(running, |this| this.opacity(0.6))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if !running {
                    this.set_mongo_export_scope(scope, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(label)
}

fn mongo_export_footer(form: &MongoExportForm, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    let running = form.running;
    // 底部左侧常驻一行摘要：确认「导出什么、导到哪」，不用回头翻上面的选项。
    let scope = match form.scope {
        MongoExportScope::All => tr("app.mongo.export.scope_all"),
        MongoExportScope::Conditions => tr("app.data_export.scope_current"),
    };
    let summary = tr("app.mongo.export.footer_summary")
        .replace("%{format}", form.format.label())
        .replace("%{scope}", &scope);
    div()
        .h(px(58.))
        .flex_none()
        .px_5()
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        // 摘要占满剩余宽度，按钮始终贴右。
        .child(
            div()
                .debug_selector(|| "mongo-export-summary".into())
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_size(ui_text_size(11.5))
                .text_color(colors.muted)
                .child(summary),
        )
        .when(running, |this| {
            this.child(
                Button::new("mongo-export-cancel-task")
                    .label(tr("app.data_export.cancel_export"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.request_cancel_mongo_export(cx);
                        cx.stop_propagation();
                    })),
            )
        })
        .child(
            Button::new("mongo-export-close")
                .label(tr("app.common.close"))
                .disabled(running)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_mongo_export_dialog(cx);
                    cx.stop_propagation();
                })),
        )
        .when(!running, |this| {
            this.child(
                Button::new("mongo-export-confirm")
                    .debug_selector(|| "mongo-export-confirm".into())
                    .label(tr("app.data_export.export"))
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_mongo_export(cx);
                        cx.stop_propagation();
                    })),
            )
        })
}
