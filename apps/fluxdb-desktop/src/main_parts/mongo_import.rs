// MongoDB 集合的文档导入弹窗：选文件 → 预览字段类型 → 批量写入。
//
// 与表数据导入（`menus_dialogs/data_import.rs`）共用交互骨架，但语义不同：
//   - 目标是「一篇文档」而不是「一行 × 目标列」，因此没有字段映射，只有 CSV 的
//     字段类型推断与逐字段改选；
//   - 坏文档「跳过并继续」，失败明细在结束后随报告展示；
//   - 长任务可中止：每批写入前检查取消标记，已写入的批次保留。

/// 导入弹窗的一次会话状态。
#[derive(Clone, Debug)]
struct MongoImportForm {
    /// 结果条所属的集合标签页：导入完成后用它刷新结果。
    tab_id: TabId,
    object: ObjectPath,
    source: Option<PathBuf>,
    options: fluxdb_core::MongoImportOptions,
    preview: Option<fluxdb_core::MongoImportPreview>,
    loading: bool,
    running: bool,
    error: Option<String>,
    /// 最近一条进度文案（阶段 + 说明）。
    progress: Option<String>,
    /// 结束后的结果；`running` 为假且此项为 `Some` 表示已跑完。
    report: Option<fluxdb_core::MongoImportReport>,
    /// 运行中用于中止；未运行时为空。
    cancel: Option<Arc<AtomicBool>>,
    /// 当前页签：常规配置 / 消息日志。
    tab: ImportDialogTab,
    /// 消息日志：每条进度一行，与表数据导入同一版式。
    logs: Vec<ImportDialogLogEntry>,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
    /// 任务中心记录 id；弹窗关闭后仍可查看本次导入结果。
    task_center_id: Option<u64>,
}

impl MongoImportForm {
    fn new(tab_id: TabId, object: ObjectPath) -> Self {
        Self {
            tab_id,
            object,
            source: None,
            options: fluxdb_core::MongoImportOptions::new(),
            preview: None,
            loading: false,
            running: false,
            error: None,
            progress: None,
            report: None,
            cancel: None,
            tab: ImportDialogTab::General,
            logs: Vec::new(),
            started_at: None,
            finished_at: None,
            task_center_id: None,
        }
    }

    /// 日志页需要的会话快照；行数单位是「文档」，措辞单独取。
    fn log_view(&self) -> ImportDialogLogView<'_> {
        ImportDialogLogView {
            running: self.running,
            finished: self.finished_at.is_some(),
            imported_label: tr("app.mongo.import.log_documents").replace(
                "%{count}",
                &self
                    .report
                    .as_ref()
                    .map(|report| report.inserted.to_string())
                    .unwrap_or_else(|| "0".to_string()),
            ),
            started_at: self.started_at,
            finished_at: self.finished_at,
            logs: &self.logs,
        }
    }

    /// 追加一条日志，耗时相对本次导入开始时间。
    fn push_log(&mut self, stage: String, message: String, success: bool) {
        let elapsed_ms = self
            .started_at
            .map(|start| start.elapsed().as_millis() as u64)
            .unwrap_or(0);
        self.logs.push(ImportDialogLogEntry {
            stage,
            message,
            success,
            elapsed_ms,
        });
    }

    fn source_name(&self) -> Option<String> {
        self.source
            .as_ref()?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }

    /// CSV 才需要用户确认字段类型；JSON/JSONL 的类型由文档自身携带。
    fn is_csv(&self) -> bool {
        self.preview
            .as_ref()
            .map(|preview| preview.format.has_field_types())
            .unwrap_or(false)
    }
}

impl NavicatMain {
    /// 结果条「导入」入口：先确认当前标签页是集合，再打开弹窗。
    pub(crate) fn open_mongo_import_dialog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self
            .pending_mongo_import
            .as_ref()
            .is_some_and(|form| form.running)
        {
            self.show_message(
                tr("app.mongo.import.running").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let object = mongo_collection_state(self.controller.state(), tab_id)
            .map(|state| state.object.clone());
        let Some(object) = object else {
            self.show_message(
                tr("app.mongo.import.open_failed").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        };
        self.pending_mongo_import = Some(MongoImportForm::new(tab_id, object));
        cx.notify();
    }

    /// 弹窗内选择文件；选好后立即解析预览。
    fn choose_mongo_import_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("app.mongo.import.choose_file")),
        });
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = cx.update(|_, cx| {
                        if let Some(view) = view.upgrade() {
                            view.update(cx, |this, cx| {
                                this.show_message(
                                    format!(
                                        "{}: {message}",
                                        tr("app.mongo.import.choose_file_failed")
                                    ),
                                    AppMessageKind::Error,
                                    cx,
                                );
                            });
                        }
                    });
                    None
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = cx.update(|_, cx| {
                        if let Some(view) = view.upgrade() {
                            view.update(cx, |this, cx| {
                                this.show_message(
                                    format!(
                                        "{}: {message}",
                                        tr("app.mongo.import.choose_file_failed")
                                    ),
                                    AppMessageKind::Error,
                                    cx,
                                );
                            });
                        }
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
                    let Some(form) = this.pending_mongo_import.as_mut() else {
                        return;
                    };
                    form.source = Some(path);
                    form.preview = None;
                    form.error = None;
                    form.report = None;
                    form.progress = None;
                    this.reload_mongo_import_preview(cx);
                    cx.notify();
                });
            });
        }));
    }

    /// 解析文件样本并刷新预览；不写数据库。
    fn reload_mongo_import_preview(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_mongo_import.clone() else {
            return;
        };
        let Some(source) = form.source.clone() else {
            return;
        };
        if form.loading {
            return;
        }
        if let Some(current) = self.pending_mongo_import.as_mut() {
            current.loading = true;
            current.error = None;
        }
        let controller = self.controller.clone();
        let object = form.object.clone();
        let options = form.options.clone();
        let task = cx.spawn(async move |view, cx| {
            let preview = cx
                .background_spawn(async move {
                    controller.preview_mongo_import(&object, &source, options)
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if let Some(current) = this.pending_mongo_import.as_mut() {
                        current.loading = false;
                        match preview {
                            Ok(preview) => current.preview = Some(preview),
                            Err(error) => {
                                current.preview = None;
                                current.error =
                                    Some(crate::localize_backend_error_text(&error.message));
                            }
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._mongo_import_task = Some(task);
    }

    /// 改解析选项后重新解析：分隔符与空值策略都会改变类型推断结果。
    fn set_mongo_import_options(
        &mut self,
        update: impl FnOnce(&mut fluxdb_core::MongoImportOptions),
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_mongo_import.as_mut() {
            update(&mut form.options);
        }
        self.reload_mongo_import_preview(cx);
        cx.notify();
    }

    /// 逐字段改类型：只改覆盖表，不重新解析（预览原文不变，类型在渲染时取生效值）。
    fn set_mongo_import_field_type(
        &mut self,
        name: String,
        type_name: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.pending_mongo_import.as_mut() {
            match form
                .options
                .field_overrides
                .iter_mut()
                .find(|item| item.name == name)
            {
                Some(item) => item.type_name = type_name,
                None => form
                    .options
                    .field_overrides
                    .push(fluxdb_core::MongoImportFieldOverride { name, type_name }),
            }
        }
        cx.notify();
    }

    /// 执行导入；坏文档跳过并继续，结束后刷新结果条。
    fn confirm_mongo_import(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_mongo_import.clone() else {
            return;
        };
        if form.running {
            return;
        }
        let Some(source) = form.source.clone() else {
            self.set_mongo_import_error(tr("app.mongo.import.choose_first").to_string(), cx);
            return;
        };
        if form
            .preview
            .as_ref()
            .map(|preview| preview.rows.is_empty())
            .unwrap_or(true)
        {
            self.set_mongo_import_error(tr("app.mongo.import.no_documents").to_string(), cx);
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let task_center_id = self.task_center.begin(
            TaskCenterKind::MongoImport,
            tr("app.task_center.title.mongo_import").replace("%{collection}", &form.object.name),
            source.display().to_string(),
            Some(cancel.clone()),
        );
        if let Some(current) = self.pending_mongo_import.as_mut() {
            current.running = true;
            current.error = None;
            current.report = None;
            current.progress = Some(tr("app.mongo.import.stage_prepare").to_string());
            current.cancel = Some(cancel.clone());
            current.task_center_id = Some(task_center_id);
            current.logs.clear();
            current.started_at = Some(Instant::now());
            current.finished_at = None;
            // 开始时切到日志页，与表数据导入一致：进度直接可见，而不是埋在底部小字里。
            current.tab = ImportDialogTab::Log;
            current.push_log(
                tr("app.data.data_import.stage_prepare").to_string(),
                tr("app.mongo.import.log_starting").to_string(),
                true,
            );
        }

        let controller = self.controller.clone();
        let object = form.object.clone();
        let options = form.options.clone();
        let tab_id = form.tab_id;
        let task = cx.spawn(async move |view, cx| {
            // 进度在后台线程产生，经通道回到 UI 线程；写入期间界面保持可中止。
            let (progress_tx, progress_rx) = std::sync::mpsc::channel();
            let result = cx
                .background_spawn(async move {
                    controller.dispatch_database_task(
                        AppCommand::ImportMongoDocuments {
                            object,
                            source,
                            options,
                        },
                        &cancel,
                        &mut |progress: fluxdb_app::DatabaseTaskProgress| {
                            let _ = progress_tx.send(progress);
                        },
                    )
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok(progress) = progress_rx.try_recv() {
                    cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            if let Some(current) = this.pending_mongo_import.as_mut() {
                                current.progress =
                                    Some(format!("{} · {}", progress.stage, progress.message));
                                // 批次进度同时进日志页；底部那行只留最新一条。
                                current.push_log(
                                    progress.stage.clone(),
                                    progress.message.clone(),
                                    true,
                                );
                            }
                            this.task_center.update_progress(
                                task_center_id,
                                None,
                                Some(progress.stage.clone()),
                                Some(format!("{}：{}", progress.stage, progress.message)),
                            );
                            cx.notify();
                        });
                    });
                }
                if let Some(event) = result.as_mut().now_or_never() {
                    while let Ok(progress) = progress_rx.try_recv() {
                        cx.update(|cx| {
                            let Some(view) = view.upgrade() else {
                                return;
                            };
                            // 收尾阶段不必重绘：紧随其后的 finish_mongo_import 会通知一次。
                            view.update(cx, |this, _cx| {
                                if let Some(current) = this.pending_mongo_import.as_mut() {
                                    current.progress =
                                        Some(format!("{} · {}", progress.stage, progress.message));
                                    current.push_log(
                                        progress.stage.clone(),
                                        progress.message.clone(),
                                        true,
                                    );
                                }
                                this.task_center.update_progress(
                                    task_center_id,
                                    None,
                                    Some(progress.stage.clone()),
                                    Some(format!("{}：{}", progress.stage, progress.message)),
                                );
                            });
                        });
                    }
                    cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this._mongo_import_task = None;
                            this.finish_mongo_import(event, tab_id, cx);
                        });
                    });
                    break;
                }
                smol::Timer::after(Duration::from_millis(100)).await;
            }
        });
        self._mongo_import_task = Some(task);
        cx.notify();
    }

    fn set_mongo_import_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(current) = self.pending_mongo_import.as_mut() {
            current.error = Some(message);
        }
        cx.notify();
    }

    /// 任务收尾：成功则提示 + 刷新结果；失败保留弹窗让用户看到原因。
    fn finish_mongo_import(&mut self, event: AppEvent, tab_id: TabId, cx: &mut Context<Self>) {
        let task_center_id = self
            .pending_mongo_import
            .as_ref()
            .and_then(|form| form.task_center_id);
        match event {
            AppEvent::MongoDocumentsImported { report, .. } => {
                let template = if report.cancelled {
                    tr("app.mongo.import.done_cancelled")
                } else {
                    tr("app.mongo.import.done")
                };
                let message = template
                    .replace("%{inserted}", &report.inserted.to_string())
                    .replace("%{failed}", &report.failed.to_string());
                if let Some(current) = self.pending_mongo_import.as_mut() {
                    current.running = false;
                    current.cancel = None;
                    current.progress = None;
                    current.report = Some(report.clone());
                    current.finished_at = Some(Instant::now());
                    current.task_center_id = None;
                    current.tab = ImportDialogTab::Log;
                    current.push_log(
                        tr("app.data.data_import.stage_done").to_string(),
                        message.clone(),
                        report.failed == 0 && !report.cancelled,
                    );
                    // 失败明细进日志：报告本身只保留前若干条，正好够逐条展示。
                    for error in report.errors.iter() {
                        let error = crate::localize_backend_error_text(error);
                        current.push_log(
                            tr("app.data.data_import.stage_failed").to_string(),
                            error,
                            false,
                        );
                    }
                }
                let kind = if report.failed == 0 && !report.cancelled {
                    AppMessageKind::Success
                } else {
                    AppMessageKind::Warning
                };
                self.show_message(message, kind, cx);
                if let Some(id) = task_center_id {
                    self.finish_task_center_record(
                        id,
                        if report.cancelled {
                            TaskCenterStatus::Canceled
                        } else if report.failed == 0 {
                            TaskCenterStatus::Success
                        } else {
                            TaskCenterStatus::Failed
                        },
                        if report.cancelled {
                            tr("app.task_center.status.canceled").to_string()
                        } else {
                            tr("app.task_center.status.success").to_string()
                        },
                        (report.failed > 0).then(|| {
                            tr("app.task_center.status.failed").to_string()
                        }),
                    );
                }
                // 已写入的文档要立刻出现在结果里；标签页可能已被关闭，内部会自行忽略。
                self.run_mongo_query(tab_id, cx);
            }
            event => {
                let message = match event {
                    AppEvent::Failed(error) => crate::localize_backend_error_text(&error.message),
                    _ => tr("app.mongo.import.no_result").to_string(),
                };
                if let Some(current) = self.pending_mongo_import.as_mut() {
                    current.running = false;
                    current.cancel = None;
                    current.progress = None;
                    current.finished_at = Some(Instant::now());
                    current.task_center_id = None;
                    current.tab = ImportDialogTab::Log;
                    current.push_log(
                        tr("app.data.data_import.stage_failed").to_string(),
                        message.clone(),
                        false,
                    );
                    current.error = Some(message.clone());
                }
                if let Some(id) = task_center_id {
                    self.finish_task_center_record(
                        id,
                        TaskCenterStatus::Failed,
                        tr("app.task_center.status.failed").to_string(),
                        Some(message),
                    );
                }
            }
        }
        cx.notify();
    }

    /// 中止：已写入的批次保留，当前批之后的文档不再写入。
    fn cancel_mongo_import_run(&mut self, cx: &mut Context<Self>) {
        let mut task_center_id = None;
        if let Some(form) = self.pending_mongo_import.as_mut()
            && let Some(flag) = &form.cancel
        {
            flag.store(true, Ordering::Relaxed);
            form.progress = Some(tr("app.mongo.import.stopping").to_string());
            task_center_id = form.task_center_id;
        }
        if let Some(id) = task_center_id {
            self.task_center.mark_cancel_requested(id);
        }
        cx.notify();
    }

    fn close_mongo_import_dialog(&mut self, cx: &mut Context<Self>) {
        if self
            .pending_mongo_import
            .as_ref()
            .is_some_and(|form| form.running)
        {
            self.show_message(
                tr("app.mongo.import.running_no_close").as_ref(),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        self.pending_mongo_import = None;
        self._mongo_import_task = None;
        cx.notify();
    }

    /// 把导入日志复制为纯文本，格式与表数据导入保持一致。
    fn copy_mongo_import_log(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_mongo_import.as_ref() else {
            return;
        };
        let mut lines = vec![
            tr("app.mongo.import.log_collection").replace("%{name}", &form.object.name),
            tr("app.data.data_import.log_file").replace(
                "%{name}",
                &form
                    .source_name()
                    .unwrap_or_else(|| tr("app.mongo.import.no_file").to_string()),
            ),
            form.log_view().imported_label,
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
}

/// 导入弹窗：单页完成选文件、类型确认与预览，避免多步来回切换。
fn mongo_import_modal(
    form: MongoImportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_import = form.preview.is_some()
        && !form.loading
        && !form.running
        && form
            .preview
            .as_ref()
            .is_some_and(|preview| !preview.rows.is_empty());

    let panel = div()
        .w(px(980.))
        .max_w(px(980.))
        .h(px(660.))
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
        .text_size(px(13.))
        .text_color(colors.text)
        .key_context("MongoImportModal")
        .debug_selector(|| "mongo-import-modal".into())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(mongo_import_header(&form, colors, cx))
        .child(import_dialog_tabs(
            form.tab,
            ImportDialogOwner::MongoDocuments,
            colors,
            cx,
        ))
        .child(match form.tab {
            ImportDialogTab::General => div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .child(mongo_import_summary(&form, colors, cx))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .px_6()
                        .pb_4()
                        .flex()
                        .gap_4()
                        .when(form.is_csv(), |this| {
                            this.child(mongo_import_fields_panel(&form, colors, cx))
                        })
                        .child(mongo_import_preview_panel(&form, colors)),
                )
                .child(mongo_import_options_bar(&form, colors, cx))
                .into_any_element(),
            ImportDialogTab::Log => {
                import_dialog_log_body(form.log_view(), colors).into_any_element()
            }
        })
        .child(mongo_import_footer(&form, can_import, colors, cx));

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
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_mongo_import_dialog(cx);
                cx.stop_propagation();
            }),
        )
        .child(panel)
}

fn mongo_import_header(
    form: &MongoImportForm,
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
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.mongo.import.title")),
                ),
        )
        .child(
            Button::new("mongo-import-close")
                .debug_selector(|| "mongo-import-close".into())
                .ghost()
                .compact()
                .disabled(form.running)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_mongo_import_dialog(cx);
                    cx.stop_propagation();
                })),
        )
}

/// 目标集合 + 选文件 + 三项概览。
fn mongo_import_summary(
    form: &MongoImportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let preview_count = form
        .preview
        .as_ref()
        .map(|preview| preview.rows.len().to_string())
        .unwrap_or_else(|| "—".to_string());
    let format = form
        .preview
        .as_ref()
        .map(|preview| preview.format.label().to_string())
        .unwrap_or_else(|| "—".to_string());
    div()
        .flex_none()
        .px_6()
        .py_4()
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
                                .text_size(px(10.5))
                                .text_color(colors.muted)
                                .child(tr("app.mongo.import.target")),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(13.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(form.object.name.clone()),
                        ),
                )
                .child(
                    Button::new("mongo-import-choose-source")
                        .debug_selector(|| "mongo-import-choose-source".into())
                        .label(tr("app.mongo.import.choose_file"))
                        .disabled(form.running)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.choose_mongo_import_source(window, cx);
                            cx.stop_propagation();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .gap_3()
                .child(table_data_import_stat(
                    tr("app.mongo.import.summary_file").as_ref(),
                    &form
                        .source_name()
                        .unwrap_or_else(|| tr("app.mongo.import.no_file").to_string()),
                    colors,
                ))
                .child(table_data_import_stat(
                    tr("app.mongo.import.format").as_ref(),
                    &format,
                    colors,
                ))
                .child(table_data_import_stat(
                    tr("app.mongo.import.summary_rows").as_ref(),
                    &preview_count,
                    colors,
                )),
        )
        .when(form.loading, |this| {
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(app_icon(AppIcon::Loader, 13., colors.info))
                    .child(tr("app.mongo.import.parsing")),
            )
        })
        .when_some(form.error.clone(), |this, error| {
            this.child(mongo_import_notice(error, colors.danger))
        })
        .when_some(form.report.clone(), |this, report| {
            let template = if report.cancelled {
                tr("app.mongo.import.done_cancelled")
            } else {
                tr("app.mongo.import.done")
            };
            let summary = template
                .replace("%{inserted}", &report.inserted.to_string())
                .replace("%{failed}", &report.failed.to_string());
            let tone = if report.failed == 0 {
                colors.success
            } else {
                colors.warning
            };
            let mut block = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(mongo_import_notice(summary, tone));
            for error in report.errors.iter() {
                block = block.child(
                    div()
                        .text_size(px(10.5))
                        .text_color(colors.danger)
                        .child(error.clone()),
                );
            }
            this.child(block)
        })
}

/// 左侧字段类型：每个 CSV 字段一行，点击类型下拉改选。
fn mongo_import_fields_panel(
    form: &MongoImportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div().flex().flex_col().gap_1().px_3().py_2();
    if let Some(preview) = &form.preview {
        for field in &preview.fields {
            list = list.child(mongo_import_field_row(
                field.clone(),
                form.options.field_type(&field.name, &field.detected),
                colors,
                cx,
            ));
        }
    }

    div()
        .w(px(360.))
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
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.mongo.import.fields")),
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

fn mongo_import_field_row(
    field: fluxdb_core::MongoImportField,
    current: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let name = field.name.clone();
    let weak = cx.entity().downgrade();
    let selected = current.clone();
    div()
        .h(px(40.))
        .px_2()
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .child(field.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(10.))
                        .text_color(colors.muted)
                        .child(if field.sample.is_empty() {
                            tr("app.mongo.import.sample_empty").to_string()
                        } else {
                            field.sample.clone()
                        }),
                ),
        )
        .child(
            Button::new(SharedString::from(format!(
                "mongo-import-type:{}",
                field.name
            )))
            .label(current)
            .outline()
            .dropdown_caret(true)
            .compact()
            .w(px(110.))
            // 行位于弹窗中部，向下展开才不会盖住上方内容。
            .dropdown_menu_with_anchor(gpui::Anchor::TopLeft, move |menu, _, _| {
                let mut menu = menu.min_w(px(150.));
                for type_name in fluxdb_core::MONGO_IMPORT_FIELD_TYPES {
                    let view = weak.clone();
                    let field_name = name.clone();
                    let value = type_name.to_string();
                    menu = menu.item(
                        PopupMenuItem::new(type_name)
                            .checked(selected == type_name)
                            .on_click(move |_, _, cx| {
                                if let Some(view) = view.upgrade() {
                                    let field_name = field_name.clone();
                                    let value = value.clone();
                                    view.update(cx, |this, cx| {
                                        this.set_mongo_import_field_type(field_name, value, cx);
                                    });
                                }
                            }),
                    );
                }
                menu
            }),
        )
}

/// 右侧预览：CSV 按列对齐，JSON 每行一篇文档原文。
fn mongo_import_preview_panel(form: &MongoImportForm, colors: UiColors) -> Div {
    let mut content = div().flex().flex_col().px_3().py_2();
    match form.preview.as_ref() {
        Some(preview) if !preview.columns.is_empty() => {
            let mut header = div().flex().flex_none().h(px(32.));
            for column in &preview.columns {
                header = header.child(
                    div()
                        .w(px(150.))
                        .flex_none()
                        .px_2()
                        .flex()
                        .items_center()
                        .truncate()
                        .text_size(px(11.))
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
                    line = line.child(
                        div()
                            .w(px(150.))
                            .flex_none()
                            .px_2()
                            .flex()
                            .items_center()
                            .truncate()
                            .text_size(px(11.5))
                            .child(row.get(column_index).cloned().unwrap_or_default()),
                    );
                }
                content = content.child(line);
            }
        }
        Some(preview) => {
            for (row_index, row) in preview.rows.iter().enumerate() {
                content = content.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_1()
                        .rounded(px(6.))
                        .when(row_index % 2 == 1, |this| this.bg(colors.panel_alt))
                        .text_size(px(11.))
                        .child(row.first().cloned().unwrap_or_default()),
                );
            }
        }
        None => {
            content = content.child(
                div()
                    .p_4()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(tr("app.mongo.import.preview_empty")),
            );
        }
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
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.mongo.import.preview")),
                )
                .when_some(
                    form.preview.as_ref().filter(|preview| preview.truncated),
                    |this, _| {
                        this.child(div().text_size(px(10.5)).text_color(colors.muted).child(
                            tr("app.mongo.import.preview_truncated").replace("%{count}", "50"),
                        ))
                    },
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_x_scrollbar()
                .child(content),
        )
}

/// 选项行：分隔符与空值策略；CSV 之外的分隔符选择无意义，保持可见但禁用。
fn mongo_import_options_bar(
    form: &MongoImportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let csv = form.is_csv();
    div()
        .flex_none()
        .px_6()
        .pb_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(div().h(px(1.)).w_full().bg(colors.border_soft))
        .child(
            div()
                .flex()
                // 分隔符是「标签 + 控件」两行，开关只有一行；居中对齐会让开关
                // 飘在标签与控件之间，底部对齐才和下拉框同一水平线。
                .items_end()
                .gap_5()
                .child(
                    div()
                        .debug_selector(|| "mongo-import-delimiter-field".into())
                        .child(table_data_import_field(
                            tr("app.mongo.import.delimiter").as_ref(),
                            mongo_import_delimiter_dropdown(form, csv, cx).into_any_element(),
                            colors,
                        )),
                )
                // 开关自带文案，不再套一层「标签 + 控件」，否则同一句话出现两次。
                .child(mongo_import_option_toggle(
                    "mongo-import-empty-null",
                    tr("app.mongo.import.empty_as_null").to_string(),
                    form.options.empty_as_null,
                    csv,
                    colors,
                    cx,
                ))
                .child(div().flex_1())
                .when_some(form.progress.clone(), |this, progress| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .max_w(px(520.))
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(app_icon(AppIcon::Loader, 12., colors.info))
                            .child(div().truncate().child(progress)),
                    )
                }),
        )
}

fn mongo_import_delimiter_dropdown(
    form: &MongoImportForm,
    csv: bool,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let weak = cx.entity().downgrade();
    let current = form.options.delimiter.clone();
    Button::new("mongo-import-delimiter")
        .label(mongo_import_delimiter_label(&current))
        .outline()
        .dropdown_caret(true)
        .compact()
        .disabled(!csv || form.running)
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu.min_w(px(160.));
            for value in ["", ";", "\t", "|"] {
                let view = weak.clone();
                let option = value.to_string();
                let checked = current == value;
                menu = menu.item(
                    PopupMenuItem::new(mongo_import_delimiter_label(value))
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            if let Some(view) = view.upgrade() {
                                let option = option.clone();
                                view.update(cx, |this, cx| {
                                    this.set_mongo_import_options(
                                        move |options| options.delimiter = option,
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

/// 分隔符展示名：空值代表默认逗号。
fn mongo_import_delimiter_label(value: &str) -> SharedString {
    match value {
        "" => tr("app.mongo.import.delimiter_comma"),
        ";" => tr("app.mongo.import.delimiter_semicolon"),
        "\t" => tr("app.mongo.import.delimiter_tab"),
        "|" => tr("app.mongo.import.delimiter_pipe"),
        other => SharedString::from(other.to_string()),
    }
}

fn mongo_import_option_toggle(
    id: &'static str,
    label: String,
    selected: bool,
    enabled: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(28.))
        .px_3()
        .rounded(px(14.))
        .border_1()
        .border_color(if selected { colors.info } else { colors.border })
        .when(selected, |this| {
            this.bg(colors.panel_bg)
                .font_weight(gpui::FontWeight::SEMIBOLD)
        })
        .when(!selected, |this| {
            this.bg(colors.input_bg).text_color(colors.muted)
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
        })
        .when(!enabled, |this| this.opacity(0.4).cursor_default())
        .text_size(px(11.))
        .flex()
        .items_center()
        .gap_1p5()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if enabled {
                    this.set_mongo_import_options(
                        move |options| options.empty_as_null = !options.empty_as_null,
                        cx,
                    );
                }
                cx.stop_propagation();
            }),
        )
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

fn mongo_import_notice(text: String, tone: gpui::Rgba) -> Div {
    div()
        .debug_selector(|| "mongo-import-notice".into())
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
        .child(div().text_size(px(10.5)).text_color(tone).child(text))
}

fn mongo_import_footer(
    form: &MongoImportForm,
    can_import: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
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
        // 日志页：运行中同样要给「停止」，开始后界面自动切到这里。
        .when(form.tab == ImportDialogTab::Log, |this| {
            this.when(form.running, |this| {
                this.child(
                    Button::new("mongo-import-stop-log")
                        .debug_selector(|| "mongo-import-stop-log".into())
                        .label(tr("app.mongo.import.stop"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_mongo_import_run(cx);
                            cx.stop_propagation();
                        })),
                )
            })
            .child(
                Button::new("mongo-import-copy-log")
                    .debug_selector(|| "mongo-import-copy-log".into())
                    .label(tr("app.data.data_import.copy_log"))
                    .disabled(form.logs.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.copy_mongo_import_log(cx);
                        cx.stop_propagation();
                    })),
            )
            .child(
                Button::new("mongo-import-close-log")
                    .label(tr("app.common.close"))
                    .disabled(form.running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_mongo_import_dialog(cx);
                        cx.stop_propagation();
                    })),
            )
        })
        .when(form.tab == ImportDialogTab::General, |this| {
            this.when(form.running, |this| {
                this.child(
                    Button::new("mongo-import-stop")
                        .debug_selector(|| "mongo-import-stop".into())
                        .label(tr("app.mongo.import.stop"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_mongo_import_run(cx);
                            cx.stop_propagation();
                        })),
                )
            })
            .child(
                Button::new("mongo-import-cancel")
                    .label(if form.running {
                        tr("app.mongo.import.running_short")
                    } else {
                        tr("app.common.close")
                    })
                    .disabled(form.running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_mongo_import_dialog(cx);
                        cx.stop_propagation();
                    })),
            )
            .child(
                Button::new("mongo-import-start")
                    .debug_selector(|| "mongo-import-start".into())
                    .label(tr("app.mongo.import.start"))
                    .primary()
                    .disabled(!can_import)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_mongo_import(cx);
                        cx.stop_propagation();
                    })),
            )
        })
}
