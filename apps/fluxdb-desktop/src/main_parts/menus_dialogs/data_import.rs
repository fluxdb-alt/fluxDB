// Included in crate-root scope by ../menus_dialogs.rs; 表数据导入向导弹窗。

/// 导入目标方式；“创建新表”暂未支持，保留为可见但置灰的预留项。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableDataImportTarget {
    Existing,
    #[allow(dead_code)] // 预留：后续开放“从文件创建新表”时启用。
    Create,
}

/// 导入弹窗页签：与导出保持一致的“常规 + 消息日志”结构。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImportDialogTab {
    General,
    Log,
}

/// 单条导入日志；批量写入时每条进度都会追加一行。
#[derive(Clone, Debug)]
struct ImportDialogLogEntry {
    stage: String,
    message: String,
    success: bool,
    /// 相对开始时间的毫秒数，用于和导出日志一样展示每步耗时。
    elapsed_ms: u64,
}

/// 导入状态。
#[derive(Clone, Debug)]
struct TableDataImportForm {
    object: ObjectPath,
    /// 文件与格式在弹框内选择；未选择时仍可看到解析选项和置灰的预留项。
    source: Option<PathBuf>,
    format: Option<fluxdb_core::TableImportFormat>,
    options: fluxdb_core::TableImportOptions,
    target: TableDataImportTarget,
    preview: Option<fluxdb_core::TableImportPreview>,
    mappings: Vec<fluxdb_core::TableImportColumnMapping>,
    error: Option<String>,
    running: bool,
    loading: bool,
    tab: ImportDialogTab,
    logs: Vec<ImportDialogLogEntry>,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
    imported_rows: u64,
    /// 运行中用于跨线程中止；结束后清空。此前传的是临时 AtomicBool，界面无法真正停止导入。
    cancel: Option<Arc<AtomicBool>>,
    /// 任务中心记录 id；弹窗关闭后任务中心仍能查看进度与日志。
    task_center_id: Option<u64>,
}

impl TableDataImportForm {
    fn new(object: ObjectPath) -> Self {
        Self {
            object,
            source: None,
            format: None,
            options: fluxdb_core::TableImportOptions::new(),
            target: TableDataImportTarget::Existing,
            preview: None,
            mappings: Vec::new(),
            error: None,
            running: false,
            loading: false,
            tab: ImportDialogTab::General,
            logs: Vec::new(),
            started_at: None,
            finished_at: None,
            imported_rows: 0,
            cancel: None,
            task_center_id: None,
        }
    }

    fn source_name(&self) -> Option<String> {
        self.source
            .as_ref()?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }

    /// 清空模式在部分数据库无法回滚，需要在界面上明确提示。
    fn truncate_warning(
        &self,
        kind: DatabaseKind,
    ) -> Option<String> {
        if self.options.mode != fluxdb_core::TableImportMode::Truncate {
            return None;
        }
        Some(match kind {
            DatabaseKind::Postgres | DatabaseKind::Kingbase | DatabaseKind::Sqlite => {
                tr("app.data.data_import.truncate_rollback_safe").to_string()
            }
            _ => tr("app.data.data_import.truncate_rollback_unsafe").to_string(),
        })
    }
}

impl NavicatMain {
    /// 从数据编辑页打开导入向导（工具栏入口）。
    fn open_table_import_wizard_for_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let object = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::DataEditor(editor) => Some(editor.object.clone()),
                _ => None,
            });
        let Some(object) = object else {
            self.show_message(
                tr("app.data.data_import.open_failed").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        };
        self.open_table_import_wizard(object, window, cx);
    }

    /// 打开导入向导。弹框先出现，文件在框内选择。
    fn open_table_import_wizard(
        &mut self,
        object: ObjectPath,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self._data_import_task.is_some() {
            self.show_message(
                tr("app.table.import.running").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.pending_data_import = Some(TableDataImportForm::new(object));
        cx.notify();
    }

    /// 在弹框内选择导入文件；选好后解析预览。
    fn choose_table_import_source(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(object) = self
            .pending_data_import
            .as_ref()
            .map(|form| form.object.clone())
        else {
            return;
        };
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("app.table.import.choose_file")),
        });
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = cx.update(|_, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!(
                                    "{}: {message}",
                                    tr("app.table.import.choose_file_failed")
                                ),
                                AppMessageKind::Error,
                                cx,
                            );
                        });
                    });
                    None
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = cx.update(|_, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.show_message(
                                format!(
                                    "{}: {message}",
                                    tr("app.table.import.choose_file_failed")
                                ),
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
            let _ = cx.update(|_, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    let Some(format) = fluxdb_app::TableImportFormat::from_path(&path) else {
                        this.show_message(
                            tr("app.table.import.format_unsupported").as_ref(),
                            AppMessageKind::Warning,
                            cx,
                        );
                        return;
                    };
                    let Some(form) = this.pending_data_import.as_mut() else {
                        return;
                    };
                    if form.object != object {
                        return;
                    }
                    form.source = Some(path);
                    form.format = Some(format);
                    form.preview = None;
                    form.mappings.clear();
                    this.reload_table_import_preview(cx);
                    cx.notify();
                });
            });
        }));
    }

    /// 解析文件样本并刷新预览；不写数据库。
    fn reload_table_import_preview(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_data_import.clone() else {
            return;
        };
        let (Some(source), Some(_format)) = (form.source.clone(), form.format) else {
            return;
        };
        if form.loading {
            return;
        }
        if let Some(current) = self.pending_data_import.as_mut() {
            current.loading = true;
            current.error = None;
        }
        let controller = self.controller.clone();
        let object = form.object.clone();
        let options = form.options.clone();
        let task = cx.spawn(async move |view, cx| {
            let preview = cx
                .background_spawn(async move {
                    controller.preview_table_import(&object, &source, options)
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if let Some(current) = this.pending_data_import.as_mut() {
                        current.loading = false;
                        match preview {
                            Ok(preview) => {
                                current.mappings = preview
                                    .columns
                                    .iter()
                                    .map(|source| fluxdb_core::TableImportColumnMapping {
                                        source: source.clone(),
                                        target: preview
                                            .target_columns
                                            .iter()
                                            .find(|column| {
                                                column.name.eq_ignore_ascii_case(source)
                                            })
                                            .map(|column| column.name.clone()),
                                    })
                                    .collect();
                                current.preview = Some(preview);
                            }
                            Err(error) => {
                                current.error = Some(error.message);
                                current.preview = None;
                            }
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._data_import_task = Some(task);
    }

    fn cancel_table_import_wizard(&mut self, cx: &mut Context<Self>) {
        // 运行中关闭只会让任务失去进度显示，因此改为请求停止并保留弹窗到任务收尾。
        if let Some(form) = self.pending_data_import.as_mut()
            && form.running
        {
            if let Some(cancel) = &form.cancel {
                cancel.store(true, Ordering::Relaxed);
            }
            if let Some(id) = form.task_center_id {
                self.task_center.mark_cancel_requested(id);
            }
            self.show_message(
                tr("app.task_center.cancel_requested"),
                AppMessageKind::Warning,
                cx,
            );
            cx.notify();
            return;
        }
        self.pending_data_import = None;
        self._data_import_task = None;
        cx.notify();
    }

    /// 把导入日志复制为纯文本，与导出的日志格式保持一致。
    fn copy_table_import_log(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_data_import.as_ref() else {
            return;
        };
        let mut lines = vec![
            tr("app.data.data_import.log_table")
                .replace("%{name}", &form.object.name),
            tr("app.data.data_import.log_file").replace(
                "%{name}",
                &form
                    .source_name()
                    .unwrap_or_else(|| tr("app.data.data_import.no_file").to_string()),
            ),
            tr("app.data.data_import.log_rows")
                .replace("%{count}", &form.imported_rows.to_string()),
        ];
        if let Some(start) = form.started_at {
            let end = form.finished_at.unwrap_or_else(Instant::now);
            lines.push(
                tr("app.data.data_import.log_elapsed").replace(
                    "%{ms}",
                    &end.saturating_duration_since(start).as_millis().to_string(),
                ),
            );
        }
        if let Some(error) = &form.error {
            lines.push(error.clone());
        }
        for (index, entry) in form.logs.iter().enumerate() {
            lines.push(format!(
                "#{} [{}] {} ms {}",
                index + 1,
                entry.stage,
                entry.elapsed_ms,
                entry.message
            ));
        }
        cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")));
        self.show_message(
            tr("app.data.data_import.log_copied"),
            AppMessageKind::Success,
            cx,
        );
    }

    /// 清除已完成任务：关闭弹框；运行中不允许清除，避免丢失进度。
    fn clear_table_import_task(&mut self, cx: &mut Context<Self>) {
        let running = self
            .pending_data_import
            .as_ref()
            .map(|form| form.running)
            .unwrap_or(false);
        if running {
            self.show_message(
                tr("app.data.data_import.running_no_clear"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.pending_data_import = None;
        self._data_import_task = None;
        cx.notify();
    }

    fn set_table_import_mode(
        &mut self,
        mode: fluxdb_core::TableImportMode,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_data_import.as_mut() {
            form.options.mode = mode;
        }
        cx.notify();
    }

    fn set_table_import_mapping(
        &mut self,
        source: String,
        target: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_data_import.as_mut() {
            if let Some(mapping) = form
                .mappings
                .iter_mut()
                .find(|mapping| mapping.source == source)
            {
                mapping.target = target;
            }
        }
        cx.notify();
    }

    /// 更新解析选项后重新解析；只有影响解析的字段变化时才需要重载预览。
    fn set_table_import_options(
        &mut self,
        update: impl FnOnce(&mut fluxdb_core::TableImportOptions),
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_data_import.as_mut() {
            update(&mut form.options);
        }
        self.reload_table_import_preview(cx);
        cx.notify();
    }

    /// 执行导入；确认步骤由调用方负责。
    fn confirm_table_import(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_data_import.clone() else {
            return;
        };
        if form.running {
            return;
        }
        let Some(source) = form.source.clone() else {
            if let Some(current) = self.pending_data_import.as_mut() {
                current.error = Some(tr("app.data.data_import.choose_first").to_string());
            }
            cx.notify();
            return;
        };
        if form
            .preview
            .as_ref()
            .map(|preview| preview.rows.is_empty())
            .unwrap_or(true)
        {
            if let Some(current) = self.pending_data_import.as_mut() {
                current.error = Some(tr("app.data.data_import.no_rows").to_string());
            }
            cx.notify();
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let task_center_id = self.task_center.begin(
            TaskCenterKind::TableImport,
            tr("app.task_center.title.table_import").replace("%{table}", &form.object.name),
            source.display().to_string(),
            Some(cancel.clone()),
        );
        if let Some(current) = self.pending_data_import.as_mut() {
            current.running = true;
            current.error = None;
            current.logs.clear();
            current.started_at = Some(Instant::now());
            current.finished_at = None;
            current.imported_rows = 0;
            // 持久化取消标记：此前每次执行都新建 AtomicBool，界面无法停止正在写入的任务。
            current.cancel = Some(cancel.clone());
            current.task_center_id = Some(task_center_id);
            // 开始时切到日志页，让用户直接看到写入进度。
            current.tab = ImportDialogTab::Log;
            current.logs.push(ImportDialogLogEntry {
                stage: tr("app.data.data_import.stage_prepare").to_string(),
                message: tr("app.data.data_import.log_starting").to_string(),
                success: true,
                elapsed_ms: 0,
            });
        }
        let controller = self.controller.clone();
        let object = form.object.clone();
        let options = form.options.clone();
        let mappings = form.mappings.clone();
        let cancel_for_status = cancel.clone();
        let task = cx.spawn(async move |view, cx| {
            // 进度在后台线程产生，经通道逐条回到 UI 线程；导入期间界面保持可中止。
            let (progress_tx, progress_rx) = mpsc::channel::<(String, String)>();
            let result = cx
                .background_spawn(async move {
                    controller.dispatch_database_task(
                        AppCommand::ImportTableData {
                            object: object.clone(),
                            source,
                            options,
                            column_mapping: mappings,
                        },
                        cancel.as_ref(),
                        &mut |progress| {
                            let _ = progress_tx.send((progress.stage, progress.message));
                        },
                    )
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok((stage, message)) = progress_rx.try_recv() {
                    cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            if let Some(current) = this.pending_data_import.as_mut() {
                                let started_at = current.started_at.unwrap_or_else(Instant::now);
                                current.logs.push(ImportDialogLogEntry {
                                    stage: stage.clone(),
                                    message: message.clone(),
                                    success: true,
                                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                                });
                            }
                            this.task_center.update_progress(
                                task_center_id,
                                None,
                                Some(stage.clone()),
                                Some(format!("{stage}：{message}")),
                            );
                            cx.notify();
                        });
                    });
                }
                if let Some(event) = result.as_mut().now_or_never() {
                    cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this._data_import_task = None;
                            // 通道收尾：完成前最后一批进度仍需落日志。
                            while let Ok((stage, message)) = progress_rx.try_recv() {
                                if let Some(current) = this.pending_data_import.as_mut() {
                                    let started_at =
                                        current.started_at.unwrap_or_else(Instant::now);
                                    current.logs.push(ImportDialogLogEntry {
                                        stage: stage.clone(),
                                        message: message.clone(),
                                        success: true,
                                        elapsed_ms: started_at.elapsed().as_millis() as u64,
                                    });
                                }
                                this.task_center.update_progress(
                                    task_center_id,
                                    None,
                                    Some(stage.clone()),
                                    Some(format!("{stage}：{message}")),
                                );
                            }
                            match event {
                                AppEvent::TableDataImported { object, report } => {
                                    let message = tr("app.table.import.succeeded")
                                        .replace("%{rows}", &report.imported_rows.to_string());
                                    if let Some(current) = this.pending_data_import.as_mut() {
                                        current.running = false;
                                        current.finished_at = Some(Instant::now());
                                        current.imported_rows = report.imported_rows;
                                        current.cancel = None;
                                        current.tab = ImportDialogTab::Log;
                                        current.logs.push(ImportDialogLogEntry {
                                            stage: tr("app.data.data_import.stage_done").to_string(),
                                            message: message.clone(),
                                            success: true,
                                            elapsed_ms: current
                                                .started_at
                                                .map(|start| start.elapsed().as_millis() as u64)
                                                .unwrap_or(0),
                                        });
                                        for warning in &report.warnings {
                                            current.logs.push(ImportDialogLogEntry {
                                                stage: tr("app.data.data_import.stage_warning")
                                                    .to_string(),
                                                message: crate::localize_backend_error_text(warning),
                                                success: false,
                                                elapsed_ms: current
                                                    .started_at
                                                    .map(|start| start.elapsed().as_millis() as u64)
                                                    .unwrap_or(0),
                                            });
                                        }
                                    }
                                    this.finish_task_center_record(
                                        task_center_id,
                                        TaskCenterStatus::Success,
                                        tr("app.task_center.status.success").to_string(),
                                        None,
                                    );
                                    let mut message = message;
                                    if !report.warnings.is_empty() {
                                        message.push_str(": ");
                                        message.push_str(
                                            &report
                                                .warnings
                                                .iter()
                                                .map(|warning| {
                                                    crate::localize_backend_error_text(warning)
                                                })
                                                .collect::<Vec<_>>()
                                                .join("; "),
                                        );
                                    }
                                    this.show_message(message, AppMessageKind::Success, cx);
                                    this.request_data_editor_refresh_for_object(&object, cx);
                                }
                                event => {
                                    let message = match event {
                                        AppEvent::Failed(error) => error.message,
                                        _ => tr("app.table.import.no_result").to_string(),
                                    };
                                    let canceled = cancel_for_status.load(Ordering::Relaxed);
                                    let status = if canceled {
                                        TaskCenterStatus::Canceled
                                    } else {
                                        TaskCenterStatus::Failed
                                    };
                                    if let Some(current) = this.pending_data_import.as_mut() {
                                        current.running = false;
                                        current.finished_at = Some(Instant::now());
                                        current.cancel = None;
                                        current.tab = ImportDialogTab::Log;
                                        current.error =
                                            (!canceled).then_some(message.clone());
                                        current.logs.push(ImportDialogLogEntry {
                                            stage: if canceled {
                                                tr("app.data.data_import.stage_canceled")
                                                    .to_string()
                                            } else {
                                                tr("app.data.data_import.stage_failed").to_string()
                                            },
                                            message: message.clone(),
                                            success: false,
                                            elapsed_ms: current
                                                .started_at
                                                .map(|start| start.elapsed().as_millis() as u64)
                                                .unwrap_or(0),
                                        });
                                    }
                                    this.finish_task_center_record(
                                        task_center_id,
                                        status,
                                        status
                                            .label(false)
                                            .to_string(),
                                        if canceled { None } else { Some(message) },
                                    );
                                }
                            }
                            cx.notify();
                        });
                    });
                    break;
                }
                smol::Timer::after(Duration::from_millis(100)).await;
            }
        });
        self._data_import_task = Some(task);
    }
}

/// 导入弹窗：单页完成配置、映射与预览，避免多步骤来回切换。
fn table_data_import_modal(
    form: TableDataImportForm,
    kind: DatabaseKind,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let has_preview = form.preview.is_some();
    let can_import = has_preview
        && !form.loading
        && !form.running
        && form.mappings.iter().any(|mapping| mapping.target.is_some());

    let panel = div()
        .w(px(1080.))
        .max_w(px(1080.))
        .h(px(700.))
        .rounded(px(12.))
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .shadow(vec![box_shadow(
            px(0.),
            px(28.),
            px(70.),
            px(0.),
            hsla(0., 0., 0., if colors.is_dark { 0.55 } else { 0.2 }),
        )])
        .flex()
        .flex_col()
        .overflow_hidden()
        .text_size(ui_text_size(13.))
        .text_color(colors.text)
        .key_context("TableDataImportModal")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(table_data_import_header(&form, colors, cx))
        .child(import_dialog_tabs(
            form.tab,
            ImportDialogOwner::TableData,
            colors,
            cx,
        ))
        .child(match form.tab {
            ImportDialogTab::General => div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .child(table_data_import_summary(&form, colors, cx))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .px_6()
                        .pb_4()
                        .flex()
                        .gap_4()
                        .child(table_data_import_mapping_panel(&form, colors, cx))
                        .child(table_data_import_preview_panel(&form, colors)),
                )
                .child(table_data_import_options_bar(&form, kind, colors, cx))
                .into_any_element(),
            ImportDialogTab::Log => {
                import_dialog_log_body(table_data_import_log_view(&form), colors)
                    .into_any_element()
            }
        })
        .child(table_data_import_footer(&form, can_import, colors, cx));

    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.6)
        } else {
            opaque_grey(0.75, 0.3)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.cancel_table_import_wizard(cx);
            cx.stop_propagation();
        }))
        .child(panel)
}

/// 页签栏归属哪个导入弹窗：两个弹窗共用版式，只有改哪个表单状态的差别。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImportDialogOwner {
    TableData,
    MongoDocuments,
}

/// 页签栏：沿用导出弹框的样式，表数据导入与文档导入共用。
fn import_dialog_tabs(
    active: ImportDialogTab,
    owner: ImportDialogOwner,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .px_6()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(import_dialog_tab_button(
            tr("app.data.data_import.tab_general"),
            active == ImportDialogTab::General,
            ImportDialogTab::General,
            owner,
            colors,
            cx,
        ))
        .child(import_dialog_tab_button(
            tr("app.data.data_import.tab_log"),
            active == ImportDialogTab::Log,
            ImportDialogTab::Log,
            owner,
            colors,
            cx,
        ))
}

fn import_dialog_tab_button(
    label: impl Into<SharedString>,
    active: bool,
    tab: ImportDialogTab,
    owner: ImportDialogOwner,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = label.into();
    div()
        .debug_selector(move || format!("import-dialog-tab:{tab:?}"))
        .h(px(34.))
        .px_3()
        .flex()
        .items_center()
        .border_b_2()
        .border_color(if active {
            rgb(0x1687ff)
        } else {
            colors.border_soft
        })
        .text_size(ui_text_size(13.))
        .text_color(if active { colors.text } else { colors.muted })
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                match owner {
                    ImportDialogOwner::TableData => {
                        if let Some(form) = this.pending_data_import.as_mut() {
                            form.tab = tab;
                        }
                    }
                    ImportDialogOwner::MongoDocuments => {
                        if let Some(form) = this.pending_mongo_import.as_mut() {
                            form.tab = tab;
                        }
                    }
                }
                cx.notify();
                cx.stop_propagation();
            }),
        )
        .child(label)
}

/// 消息日志：阶段 + 文案，失败项用错误色区分。
/// 日志页正文所需的会话快照；表数据导入与文档导入各自组装。
struct ImportDialogLogView<'a> {
    running: bool,
    finished: bool,
    /// 已导入行数/文档数，由调用方带单位拼好（行 vs 文档）。
    imported_label: String,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
    logs: &'a [ImportDialogLogEntry],
}

fn table_data_import_log_view(form: &TableDataImportForm) -> ImportDialogLogView<'_> {
    ImportDialogLogView {
        running: form.running,
        finished: form.finished_at.is_some(),
        imported_label: tr("app.data.data_import.log_rows")
            .replace("%{count}", &form.imported_rows.to_string()),
        started_at: form.started_at,
        finished_at: form.finished_at,
        logs: &form.logs,
    }
}

fn import_dialog_log_body(view: ImportDialogLogView<'_>, colors: UiColors) -> Div {
    let status = if view.running {
        tr("app.data.data_import.log_running").to_string()
    } else if view.finished {
        tr("app.data.data_import.log_finished").to_string()
    } else {
        tr("app.data.data_import.log_idle").to_string()
    };
    let mut list = div().flex().flex_col().gap_1().px_6().py_4();
    // 失败原因已经作为“失败”阶段日志写入；顶部再渲染一次会重复同一错误。
    if view.logs.is_empty() {
        list = list.child(
            div()
                .text_size(ui_text_size(11.5))
                .text_color(colors.muted)
                .child(tr("app.data.data_import.log_empty")),
        );
    }
    for entry in view.logs.iter() {
        list = list.child(
            div()
                .flex()
                .items_start()
                .gap_3()
                .px_2()
                .py_1()
                .child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .text_size(ui_text_size(10.5))
                        .text_color(if entry.success {
                            colors.muted
                        } else {
                            colors.danger
                        })
                        .child(entry.stage.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(ui_text_size(11.5))
                        .text_color(if entry.success {
                            colors.text
                        } else {
                            colors.danger
                        })
                        .child(entry.message.clone()),
                ),
        );
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(
            // 运行概览与导出日志页保持一致的信息密度。
            div()
                .flex_none()
                .px_6()
                .pt_4()
                .flex()
                .items_center()
                .gap_5()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(view.imported_label.clone())
                .child(
                    tr("app.data.data_import.log_status")
                        .replace("%{status}", &status),
                )
                .when_some(view.started_at.zip(view.finished_at), |this, (start, end)| {
                    this.child(
                        tr("app.data.data_import.log_elapsed").replace(
                            "%{ms}",
                            &end.saturating_duration_since(start)
                                .as_millis()
                                .to_string(),
                        ),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(list),
        )
}

fn table_data_import_header(
    form: &TableDataImportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(58.))
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
                .child(app_icon(AppIcon::FolderInput, 17., colors.text))
                .child(
                    div()
                        .text_size(ui_text_size(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.data.data_import.title")),
                ),
        )
        .child(
            Button::new("data-import-close")
                .ghost()
                .compact()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_table_import_wizard(cx);
                    cx.stop_propagation();
                })),
        )
        .when(form.running, |this| {
            this.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(tr("app.data.data_import.running")),
            )
        })
}

/// 目标表 + 选择文件 + 三项概览，对应 dbx 的首屏信息密度。
fn table_data_import_summary(
    form: &TableDataImportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mapped = form
        .mappings
        .iter()
        .filter(|mapping| mapping.target.is_some())
        .count();
    let total_columns = form
        .preview
        .as_ref()
        .map(|preview| preview.columns.len())
        .unwrap_or(0);
    let preview_rows = form
        .preview
        .as_ref()
        .map(|preview| preview.rows.len())
        .unwrap_or(0);

    div()
        .flex_none()
        .px_6()
        .pb_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_stretch()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(colors.border_soft)
                        .bg(colors.panel_alt)
                        .px_4()
                        .py_3()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(ui_text_size(10.5))
                                .text_color(colors.muted)
                                .child(tr("app.data.data_import.target_table")),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(ui_text_size(13.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(form.object.name.clone()),
                        ),
                )
                .child(
                    Button::new("data-import-choose-source")
                        .label(if form.source.is_some() {
                            tr("app.data.data_import.choose_file")
                        } else {
                            tr("app.data.data_import.choose_file")
                        })
                                .on_click(cx.listener(|this, _, window, cx| {
                            this.choose_table_import_source(window, cx);
                            cx.stop_propagation();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .gap_3()
                .child(table_data_import_stat(
                    tr("app.data.data_import.summary_file").as_ref(),
                    &form
                        .source_name()
                        .unwrap_or_else(|| tr("app.data.data_import.no_file").to_string()),
                    colors,
                ))
                .child(table_data_import_stat(
                    tr("app.data.data_import.summary_rows").as_ref(),
                    &if form.preview.is_some() {
                        preview_rows.to_string()
                    } else {
                        "—".to_string()
                    },
                    colors,
                ))
                .child(table_data_import_stat(
                    tr("app.data.data_import.summary_mapped").as_ref(),
                    &if form.preview.is_some() {
                        format!("{mapped} / {total_columns}")
                    } else {
                        "—".to_string()
                    },
                    colors,
                )),
        )
        .when(form.loading, |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(app_icon(AppIcon::Loader, 13., colors.info))
                    .child(tr("app.data.data_import.parsing")),
            )
        })
        .when_some(form.error.clone(), |this, error| {
            this.child(table_data_import_notice(error, colors.danger))
        })
}

fn table_data_import_stat(label: &str, value: &str, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .rounded(px(10.))
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .px_4()
        .py_3()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .truncate()
                .text_size(ui_text_size(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(value.to_string()),
        )
}

/// 左侧字段映射：源字段 → 目标字段下拉，行高压紧避免滚动疲劳。
fn table_data_import_mapping_panel(
    form: &TableDataImportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div()
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2();
    if let Some(preview) = &form.preview {
        for source in &preview.columns {
            let current = form
                .mappings
                .iter()
                .find(|mapping| &mapping.source == source)
                .and_then(|mapping| mapping.target.clone());
            list = list.child(table_data_import_mapping_row(
                source,
                current,
                preview.target_columns.clone(),
                cx,
                colors,
            ));
        }
    } else {
        list = list.child(
            div()
                .p_4()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.data.data_import.mapping_empty")),
        );
    }

    div()
        .w(px(430.))
        .flex_none()
        .min_h(px(0.))
        .rounded(px(10.))
        .border_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .h(px(38.))
                .px_4()
                .border_b_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.data.data_import.mapping_title")),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.muted)
                        .child(tr("app.data.data_import.mapping_hint")),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(list),
        )
}

fn table_data_import_mapping_row(
    source: &str,
    current: Option<String>,
    columns: Vec<fluxdb_core::TableImportTargetColumn>,
    cx: &mut Context<NavicatMain>,
    colors: UiColors,
) -> Div {
    let skipped = current.is_none();
    let label = current
        .clone()
        .unwrap_or_else(|| tr("app.data.data_import.skip").to_string());
    let source_name = source.to_string();
    let selected = current.clone();
    let weak = cx.entity().downgrade();
    let menu_columns = columns.clone();

    div()
        .h(px(40.))
        .px_2()
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(140.))
                .min_w(px(0.))
                .truncate()
                .text_size(ui_text_size(12.))
                .child(source_name.clone()),
        )
        .child(
            Button::new(SharedString::from(format!("data-import-map:{source}")))
                .label(label)
                .outline()
                .dropdown_caret(true)
                .compact()
                .w(px(210.))
                .when(skipped, |this| this.text_color(colors.muted))
                // 映射行位于表格中部，向下展开才不会盖住上方内容。
                .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
                    let mut menu = menu.min_w(px(240.)).max_w(px(320.)).check_side(gpui_component::Side::Right);

                    let skip_view = weak.clone();
                    let skip_source = source_name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(tr("app.data.data_import.skip"))
                            .checked(selected.is_none())
                            .on_click(move |_, _, cx| {
                                if let Some(view) = skip_view.upgrade() {
                                    view.update(cx, |this, cx| {
                                        this.set_table_import_mapping(skip_source.clone(), None, cx);
                                    });
                                }
                            }),
                    );

                    for column in &menu_columns {
                        let target = column.name.clone();
                        let map_view = weak.clone();
                        let map_source = source_name.clone();
                        let item_label = match &column.type_name {
                            Some(type_name) => format!("{} · {}", column.name, type_name),
                            None => column.name.clone(),
                        };
                        let checked = selected.as_deref() == Some(column.name.as_str());
                        menu = menu.item(
                            PopupMenuItem::new(item_label)
                                .checked(checked)
                                .on_click(move |_, _, cx| {
                                    if let Some(view) = map_view.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.set_table_import_mapping(
                                                map_source.clone(),
                                                Some(target.clone()),
                                                cx,
                                            );
                                        });
                                    }
                                }),
                        );
                    }
                    menu
                }),
        )
}

/// 右侧数据预览：等宽列 + 斑马纹，最多展示预览行数。
fn table_data_import_preview_panel(form: &TableDataImportForm, colors: UiColors) -> Div {
    let mut content = div().flex().flex_col().px_3().py_2();
    if let Some(preview) = &form.preview {
        let mut header = div().flex().flex_none().h(px(32.));
        for column in &preview.columns {
            header = header.child(
                div()
                    .w(px(160.))
                    .flex_none()
                    .px_2()
                    .flex()
                    .items_center()
                    .truncate()
                    .text_size(ui_text_size(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.muted)
                    .child(column.clone()),
            );
        }
        content = content.child(header);
        for (row_index, row) in preview.rows.iter().enumerate() {
            let mut line = div()
                .flex()
                .flex_none()
                .h(px(30.))
                .rounded(px(6.))
                .when(row_index % 2 == 1, |this| this.bg(colors.panel_alt));
            for column_index in 0..preview.columns.len() {
                let value = row.get(column_index).cloned().unwrap_or_default();
                line = line.child(
                    div()
                        .w(px(160.))
                        .flex_none()
                        .px_2()
                        .flex()
                        .items_center()
                        .truncate()
                        .text_size(ui_text_size(11.5))
                        .child(value),
                );
            }
            content = content.child(line);
        }
    } else {
        content = content.child(
            div()
                .p_4()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.data.data_import.preview_empty")),
        );
    }

    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .rounded(px(10.))
        .border_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .h(px(38.))
                .px_4()
                .border_b_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.data.data_import.preview_title")),
                )
                .when_some(form.preview.as_ref(), |this, preview| {
                    this.child(
                        div()
                            .text_size(ui_text_size(10.5))
                            .text_color(colors.muted)
                            .child(if preview.truncated {
                                tr("app.data.data_import.preview_truncated").to_string()
                            } else {
                                format!("{} · {}", preview.encoding, preview.format)
                            }),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_x_scrollbar()
                .child(content),
        )
}

/// 底部选项条：导入模式与解析选项集中一行，减少纵向堆叠。
fn table_data_import_options_bar(
    form: &TableDataImportForm,
    kind: DatabaseKind,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let delimited = matches!(
        form.format,
        Some(fluxdb_core::TableImportFormat::Txt | fluxdb_core::TableImportFormat::Csv)
    );
    let row = div()
        .flex_none()
        .px_6()
        .pb_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            // 选项区与上方表格用分隔线切开，避免看起来是表格的一部分。
            div().h(px(1.)).w_full().bg(colors.border_soft),
        )
        .child(
            div()
                .flex()
                // 字段是「标签 + 控件」两行，开关只有一行；居中对齐会让开关飘在
                // 标签与控件之间，底部对齐才和下拉框同一水平线。
                .items_end()
                .gap_5()
                .child(table_data_import_field(
                    tr("app.data.data_import.mode").as_ref(),
                    {
                        let mut cluster = div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(table_data_import_mode_dropdown(form, cx));
                        // 清空风险紧跟模式选择，避免提示孤立在角落。
                        if let Some(warning) = form.truncate_warning(kind) {
                            cluster = cluster.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_size(ui_text_size(10.5))
                                    .text_color(colors.warning)
                                    .child(app_icon(AppIcon::Info, 12., colors.warning))
                                    .child(warning),
                            );
                        }
                        cluster.into_any_element()
                    },
                    colors,
                ))
                .child(table_data_import_field(
                    tr("app.data.data_import.target_mode").as_ref(),
                    table_data_import_target_dropdown(form, cx).into_any_element(),
                    colors,
                ))
                .child(table_data_import_field(
                    tr("app.data.data_import.encoding").as_ref(),
                    table_data_import_encoding_dropdown(form, cx)
                        .into_any_element(),
                    colors,
                ))
                .child(div().flex_1())
                .when(delimited, |this| {
                    this.child(table_data_import_option_toggle(
                        "data-import-has-header",
                        tr("app.data.data_import.has_header").to_string(),
                        form.options.has_header,
                        colors,
                        cx,
                    ))
                    .child(table_data_import_option_toggle(
                        "data-import-trim",
                        tr("app.data.data_import.trim").to_string(),
                        form.options.trim_whitespace,
                        colors,
                        cx,
                    ))
                    .child(table_data_import_option_toggle(
                        "data-import-empty-null",
                        tr("app.data.data_import.empty_as_null").to_string(),
                        form.options.empty_as_null,
                        colors,
                        cx,
                    ))
                }),
        );

    row
}

/// 目标方式下拉：保留“创建新表”为置灰预留项，而不是隐藏。
fn table_data_import_target_dropdown(
    form: &TableDataImportForm,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let weak = cx.entity().downgrade();
    let current = form.target;
    Button::new("data-import-target")
        .label(match current {
            TableDataImportTarget::Existing => {
                tr("app.data.data_import.target_existing")
            }
            TableDataImportTarget::Create => tr("app.data.data_import.target_create"),
        })
        .outline()
        .dropdown_caret(true)
        .compact()
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu.min_w(px(240.));
            let exist_view = weak.clone();
            menu = menu.item(
                PopupMenuItem::new(tr("app.data.data_import.target_existing"))
                    .checked(current == TableDataImportTarget::Existing)
                    .on_click(move |_, _, cx| {
                        if let Some(view) = exist_view.upgrade() {
                            view.update(cx, |this, cx| {
                                if let Some(form) = this.pending_data_import.as_mut() {
                                    form.target = TableDataImportTarget::Existing;
                                }
                                cx.notify();
                            });
                        }
                    }),
            );
            menu.item(
                PopupMenuItem::new(tr("app.data.data_import.target_create"))
                    .disabled(true)
                    .checked(false),
            )
        })
}

/// 文件编码下拉：选项固定且互斥，改选后立即重新解析预览。
fn table_data_import_encoding_dropdown(
    form: &TableDataImportForm,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let weak = cx.entity().downgrade();
    let current = if form.options.encoding.trim().is_empty() {
        "UTF-8".to_string()
    } else {
        form.options.encoding.trim().to_ascii_uppercase()
    };
    Button::new("data-import-encoding")
        .label(current.clone())
        .outline()
        .dropdown_caret(true)
        .compact()
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu.min_w(px(180.));
            for encoding in ["UTF-8", "GB18030"] {
                let view = weak.clone();
                let value = encoding.to_string();
                let label = encoding.to_string();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(current == encoding)
                        .on_click(move |_, _, cx| {
                            if let Some(view) = view.upgrade() {
                                let value = value.clone();
                                view.update(cx, |this, cx| {
                                    this.set_table_import_options(
                                        move |options| {
                                            // UTF-8 是默认编码，空值表示默认，避免多存一份状态。
                                            options.encoding = if value == "UTF-8" {
                                                String::new()
                                            } else {
                                                value.clone()
                                            };
                                        },
                                        cx,
                                    );
                                });
                            }
                        }),
                );
            }
            menu
        })
}

fn table_data_import_mode_dropdown(
    form: &TableDataImportForm,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let weak = cx.entity().downgrade();
    let mode = form.options.mode;
    let label = match mode {
        fluxdb_core::TableImportMode::Append => tr("app.data.data_import.mode_append"),
        fluxdb_core::TableImportMode::Truncate => tr("app.data.data_import.mode_truncate"),
    };
    Button::new("data-import-mode")
        .label(label)
        .outline()
        .dropdown_caret(true)
        .compact()
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu.min_w(px(200.));
            for option in [
                fluxdb_core::TableImportMode::Append,
                fluxdb_core::TableImportMode::Truncate,
            ] {
                let option_label = match option {
                    fluxdb_core::TableImportMode::Append => {
                        tr("app.data.data_import.mode_append")
                    }
                    fluxdb_core::TableImportMode::Truncate => {
                        tr("app.data.data_import.mode_truncate")
                    }
                };
                let view = weak.clone();
                menu = menu.item(
                    PopupMenuItem::new(option_label)
                        .checked(option == mode)
                        .on_click(move |_, _, cx| {
                            if let Some(view) = view.upgrade() {
                                view.update(cx, |this, cx| {
                                    this.set_table_import_mode(option, cx);
                                });
                            }
                        }),
                );
            }
            menu
        })
}

fn table_data_import_option_toggle(
    id: &'static str,
    label: String,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let key = id.to_string();
    div()
        .id(id)
        .h(px(28.))
        .px_3()
        .rounded(px(14.))
        .border_1()
        .border_color(if selected {
            colors.info
        } else {
            colors.border
        })
        .when(selected, |this| {
            this.bg(colors.panel_bg)
                .font_weight(gpui::FontWeight::SEMIBOLD)
        })
        .when(!selected, |this| {
            this.bg(colors.input_bg).text_color(colors.muted)
        })
        .cursor_pointer()
        .text_size(ui_text_size(11.))
        .flex()
        .items_center()
        .gap_1p5()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                let key = key.clone();
                this.set_table_import_options(
                    move |options| match key.as_str() {
                        "data-import-has-header" => options.has_header = !options.has_header,
                        "data-import-trim" => {
                            options.trim_whitespace = !options.trim_whitespace
                        }
                        "data-import-empty-null" => {
                            options.empty_as_null = !options.empty_as_null
                        }
                        _ => {}
                    },
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        // 勾选图标让开关状态一眼可辨，而不是只靠颜色。
        .child(app_icon(
            if selected {
                AppIcon::Check
            } else {
                AppIcon::Circle
            },
            11.,
            if selected { colors.info } else { colors.muted },
        ))
        .child(label)
}

/// 表单字段：上方小标签、下方控件，让控件归属明确。
fn table_data_import_field(
    label: &str,
    control: gpui::AnyElement,
    colors: UiColors,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .child(label.to_string()),
        )
        .child(control)
}

fn table_data_import_notice(text: String, tone: gpui::Rgba) -> Div {
    div()
        .debug_selector(|| "data-import-notice".into())
        // 错误只占自身宽度，避免横跨整行压过主要内容。
        .max_w(px(760.))
        .rounded(px(6.))
        .border_1()
        .border_color(tone)
        .px_2p5()
        .py_1p5()
        .flex()
        .items_center()
        .gap_1p5()
        .child(app_icon(AppIcon::Info, 12., tone))
        .child(
            div()
                .text_size(ui_text_size(10.5))
                .text_color(tone)
                .child(text),
        )
}

fn table_data_import_footer(
    form: &TableDataImportForm,
    can_import: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let finished = form.finished_at.is_some();
    div()
        .h(px(64.))
        .flex_none()
        .px_5()
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        // 日志页与导出一致：复制日志 / 清除任务 / 关闭。
        .when(form.tab == ImportDialogTab::Log, |this| {
            this.child(
                Button::new("data-import-copy-log")
                    .label(tr("app.data.data_import.copy_log"))
                    .disabled(form.logs.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.copy_table_import_log(cx);
                        cx.stop_propagation();
                    })),
            )
            .when(finished, |this| {
                this.child(
                    Button::new("data-import-clear-task")
                        .label(tr("app.data.data_import.clear_task"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_table_import_task(cx);
                            cx.stop_propagation();
                        })),
                )
            })
            .child(
                Button::new("data-import-close-log")
                    .label(tr("app.common.close"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_table_import_wizard(cx);
                        cx.stop_propagation();
                    })),
            )
        })
        .when(form.tab == ImportDialogTab::General, |this| {
            this.child(
                Button::new("data-import-cancel")
                    .label(tr("app.common.cancel"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_table_import_wizard(cx);
                        cx.stop_propagation();
                    })),
            )
            .child(
                Button::new("data-import-start")
                    .label(tr("app.data.data_import.start"))
                    .primary()
                    .disabled(!can_import || form.running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_table_import(cx);
                        cx.stop_propagation();
                    })),
            )
        })
}
