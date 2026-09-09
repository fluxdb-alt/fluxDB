const DATA_EXPORT_BATCH_SIZE: u64 = 1000;
const DATA_EXPORT_VISIBLE_LOG_LIMIT: usize = 300;

impl NavicatMain {
    fn open_table_data_export(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((object, columns)) = self.table_data_export_source(tab_id) else {
            self.show_message("请先加载表数据", AppMessageKind::Warning, cx);
            return;
        };
        let selected_fields = columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<BTreeSet<_>>();
        self.pending_data_export = Some(TableDataExportForm {
            tab_id,
            object,
            columns,
            selected_fields,
            format: TableDataExportFormat::Csv,
            scope: TableDataExportScope::CurrentConditions,
            sort: self.data_sort_specs_for_tab(tab_id),
            filters: self.data_filter_specs_for_tab(tab_id),
            custom_filter_rules: Vec::new(),
            custom_sort_rules: Vec::new(),
            active_tab: TableDataExportTab::General,
        });
        self.data_export_custom_conditions_open = false;
        self.refresh_table_data_export_preview(true, cx);
        cx.notify();
    }

    fn open_table_data_export_from_object(
        &mut self,
        object_path: ObjectPath,
        cx: &mut Context<Self>,
    ) {
        let event = self.dispatch(AppCommand::OpenDataEditor(object_path), cx);
        let tab_id = match event {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => Some(tab_id),
            _ => self.controller.state().active_tab,
        };
        let Some(tab_id) = tab_id else {
            self.show_message("打开表数据失败", AppMessageKind::Error, cx);
            return;
        };
        if self.table_data_export_source(tab_id).is_some() {
            self.open_table_data_export(tab_id, cx);
            return;
        }
        self.pending_table_data_export_after_load = Some(tab_id);
        self.show_message("正在加载表字段，加载完成后打开导出", AppMessageKind::Info, cx);
        cx.notify();
    }

    fn table_data_export_source(
        &self,
        tab_id: TabId,
    ) -> Option<(ObjectPath, Vec<TableDataExportColumn>)> {
        let tab = self.controller.state().tabs.iter().find(|tab| tab.id == tab_id)?;
        let TabKind::DataEditor(editor) = &tab.kind else {
            return None;
        };
        let page = editor.page.as_ref()?;
        let columns = page
            .columns
            .iter()
            .map(|column| TableDataExportColumn {
                name: column.name.clone(),
                type_name: column.type_name.clone(),
                nullable: column.nullable,
                primary_key: column.primary_key,
                comment: column.comment.clone(),
            })
            .collect();
        Some((editor.object.clone(), columns))
    }

    fn cancel_table_data_export_modal(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &self.pending_data_export {
            self.cleanup_table_data_export_object_selects(form.tab_id);
        }
        self.cleanup_table_data_export_filter_state();
        self.pending_data_export = None;
        self.data_export_custom_conditions_open = false;
        self.data_export_preview = None;
        cx.notify();
    }

    fn select_table_data_export_tab(&mut self, tab: TableDataExportTab, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_data_export {
            form.active_tab = tab;
            cx.notify();
        }
    }

    fn set_table_data_export_format(
        &mut self,
        format: TableDataExportFormat,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.pending_data_export {
            form.format = format;
            cx.notify();
        }
    }

    fn set_table_data_export_scope(
        &mut self,
        scope: TableDataExportScope,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.pending_data_export {
            form.scope = scope;
            self.refresh_table_data_export_preview(true, cx);
            cx.notify();
        }
    }

    fn refresh_table_data_export_preview(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(form) = self.pending_data_export.clone() else {
            return;
        };
        let key = table_data_export_preview_key(&form);
        if !force
            && self
                .data_export_preview
                .as_ref()
                .is_some_and(|preview| preview.key == key)
        {
            return;
        }
        let (fields, sort, filters) = table_data_export_preview_request(&form);
        self.data_export_preview_seq = self.data_export_preview_seq.saturating_add(1);
        let seq = self.data_export_preview_seq;
        self.data_export_preview = Some(TableDataExportPreviewState {
            key: key.clone(),
            status: TableDataExportPreviewStatus::Loading,
        });
        let controller = self.controller.clone();
        let object = form.object.clone();
        self._data_export_preview_task = Some(cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    controller.preview_data_export(&object, &fields, &sort, &filters)
                })
                .await;
            table_data_export_preview_on_ui(&view, cx, seq, key, result);
        }));
    }

    fn open_table_data_export_custom_conditions(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.pending_data_export else {
            return;
        };
        let export_filter_tab_id = table_data_export_filter_tab_id(form.tab_id);
        self.data_filter_draft_rules
            .insert(export_filter_tab_id, form.custom_filter_rules.clone());
        self.data_sort_draft_rules
            .insert(export_filter_tab_id, form.custom_sort_rules.clone());
        self.data_filter_popover = None;
        self.data_export_custom_conditions_open = true;
        cx.notify();
    }

    fn cancel_table_data_export_custom_conditions(&mut self, cx: &mut Context<Self>) {
        self.cleanup_table_data_export_filter_state();
        self.data_export_custom_conditions_open = false;
        cx.notify();
    }

    fn apply_table_data_export_custom_conditions(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.pending_data_export else {
            return;
        };
        let export_filter_tab_id = table_data_export_filter_tab_id(form.tab_id);
        form.custom_filter_rules = self
            .data_filter_draft_rules
            .get(&export_filter_tab_id)
            .cloned()
            .unwrap_or_default();
        form.custom_sort_rules = self
            .data_sort_draft_rules
            .get(&export_filter_tab_id)
            .cloned()
            .unwrap_or_default();
        form.scope = TableDataExportScope::CustomRules;
        self.cleanup_table_data_export_filter_state();
        self.data_export_custom_conditions_open = false;
        self.refresh_table_data_export_preview(true, cx);
        cx.notify();
    }

    fn clear_table_data_export_custom_conditions(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_data_export {
            form.custom_filter_rules.clear();
            form.custom_sort_rules.clear();
            form.scope = TableDataExportScope::CustomRules;
            self.cleanup_table_data_export_filter_state();
            self.refresh_table_data_export_preview(true, cx);
            cx.notify();
        }
    }

    fn cleanup_table_data_export_filter_state(&mut self) {
        let Some(form) = &self.pending_data_export else {
            return;
        };
        let export_filter_tab_id = table_data_export_filter_tab_id(form.tab_id);
        self.data_filter_draft_rules.remove(&export_filter_tab_id);
        self.data_sort_draft_rules.remove(&export_filter_tab_id);
        self.data_filter_popover = self
            .data_filter_popover
            .filter(|popover| popover.tab_id != export_filter_tab_id);
    }

    fn cleanup_table_data_export_object_selects(&mut self, tab_id: TabId) {
        for key in [
            DataExportObjectSelectKey::Database(tab_id),
            DataExportObjectSelectKey::Table(tab_id),
        ] {
            self.data_export_object_selects.remove(&key);
            self._data_export_object_select_subscriptions.remove(&key);
        }
    }

    fn data_export_object_select(
        &mut self,
        key: DataExportObjectSelectKey,
        options: Vec<String>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<SearchableVec<String>>> {
        if !self.data_export_object_selects.contains_key(&key) {
            let selected_index = data_export_select_index(&options, value);
            let select_options = options.clone();
            let select = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(select_options.clone()),
                    selected_index,
                    window,
                    cx,
                )
                .searchable(matches!(key, DataExportObjectSelectKey::Table(_)))
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                move |this: &mut NavicatMain,
                      _select,
                      event: &SelectEvent<SearchableVec<String>>,
                      _window,
                      cx| {
                    let SelectEvent::Confirm(value) = event;
                    if let (DataExportObjectSelectKey::Table(_), Some(value)) = (key, value) {
                        this.select_table_data_export_table(value.clone(), cx);
                    }
                },
            );
            self.data_export_object_selects.insert(key, select);
            self._data_export_object_select_subscriptions
                .insert(key, subscription);
        }

        let select = self.data_export_object_selects.get(&key).cloned().unwrap();
        let selected_index = data_export_select_index(&options, value);
        select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(options), window, cx);
            select.set_selected_index(selected_index, window, cx);
        });
        select
    }

    fn select_table_data_export_table(&mut self, table_name: String, cx: &mut Context<Self>) {
        let table_name = table_name.trim().to_string();
        let Some(form) = self.pending_data_export.clone() else {
            return;
        };
        if table_name.is_empty() || form.object.name == table_name {
            return;
        }
        let Some(object) = self
            .table_data_export_object_options(&form)
            .into_iter()
            .find(|object| object.name == table_name)
        else {
            self.show_message("未找到可导出的表", AppMessageKind::Warning, cx);
            return;
        };

        self.cleanup_table_data_export_filter_state();
        self.data_export_custom_conditions_open = false;
        let event = self.dispatch(AppCommand::OpenDataEditor(object.clone()), cx);
        let tab_id = match event {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => Some(tab_id),
            _ => self.controller.state().active_tab,
        };
        let Some(tab_id) = tab_id else {
            self.show_message("切换导出表失败", AppMessageKind::Error, cx);
            return;
        };
        if self.table_data_export_source(tab_id).is_some() {
            self.open_table_data_export(tab_id, cx);
            self.show_message(
                format!("已切换导出表：{}", object.name),
                AppMessageKind::Info,
                cx,
            );
            return;
        }
        self.pending_table_data_export_after_load = Some(tab_id);
        self.show_message("正在加载表字段，加载完成后更新导出对象", AppMessageKind::Info, cx);
        cx.notify();
    }

    fn table_data_export_table_select_options(&self, form: &TableDataExportForm) -> Vec<String> {
        self.table_data_export_object_options(form)
            .into_iter()
            .map(|object| object.name)
            .collect()
    }

    fn table_data_export_object_options(&self, form: &TableDataExportForm) -> Vec<ObjectPath> {
        let database = data_export_object_database_label(&form.object);
        let mut objects = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == form.object.connection_id)
            .map(|connection| {
                connection
                    .objects
                    .iter()
                    .filter(|object| {
                        matches!(object.path.kind, ObjectKind::Table | ObjectKind::View)
                            && data_export_object_database_label(&object.path) == database
                    })
                    .map(|object| object.path.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !objects.iter().any(|object| object.name == form.object.name) {
            objects.push(form.object.clone());
        }
        objects.sort_by(|left, right| left.name.cmp(&right.name));
        objects.dedup_by(|left, right| left.name == right.name);
        objects
    }

    fn toggle_table_data_export_field(&mut self, field: String, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_data_export {
            if !form.selected_fields.remove(&field) {
                form.selected_fields.insert(field);
            }
            self.refresh_table_data_export_preview(true, cx);
            cx.notify();
        }
    }

    fn select_all_table_data_export_fields(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_data_export {
            form.selected_fields = form
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect();
            self.refresh_table_data_export_preview(true, cx);
            cx.notify();
        }
    }

    fn invert_table_data_export_fields(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_data_export {
            let mut next = BTreeSet::new();
            for column in &form.columns {
                if !form.selected_fields.contains(&column.name) {
                    next.insert(column.name.clone());
                }
            }
            form.selected_fields = next;
            self.refresh_table_data_export_preview(true, cx);
            cx.notify();
        }
    }

    fn select_visible_table_data_export_fields(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.pending_data_export else {
            return;
        };
        let Some(visible) = self.visible_table_fields.get(&form.tab_id).cloned() else {
            self.select_all_table_data_export_fields(cx);
            return;
        };
        if let Some(form) = &mut self.pending_data_export {
            form.selected_fields = form
                .columns
                .iter()
                .filter(|column| visible.contains(&column.name))
                .map(|column| column.name.clone())
                .collect();
            self.refresh_table_data_export_preview(true, cx);
            cx.notify();
        }
    }

    fn confirm_table_data_export(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pending_data_export.clone() else {
            return;
        };
        if form.selected_fields.is_empty() {
            self.show_message("请选择至少一个字段", AppMessageKind::Warning, cx);
            return;
        }
        let fields = form
            .columns
            .iter()
            .filter(|column| form.selected_fields.contains(&column.name))
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let suggested = table_data_export_suggested_name(&form.object, form.format);
        let receiver = cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested));
        let task_id = self.create_table_data_export_task(&form, fields.len());
        self.data_export_log_task = Some(task_id);
        self.cleanup_table_data_export_filter_state();
        self.pending_data_export = None;
        self.show_message("选择保存位置后开始导出", AppMessageKind::Success, cx);
        self.spawn_table_data_export_task(task_id, receiver, form, fields, cx);
        cx.notify();
    }

    fn create_table_data_export_task(&mut self, form: &TableDataExportForm, field_count: usize) -> u64 {
        let task_id = self.next_data_export_task_id();
        self.data_export_tasks.push(TableDataExportTaskState {
            id: task_id,
            table_name: form.object.name.clone(),
            path: PathBuf::new(),
            format: form.format,
            field_count,
            exported_rows: 0,
            batch_count: 0,
            started_at: Instant::now(),
            finished_at: None,
            logs: vec![TableDataExportLogEntry {
                index: 1,
                success: true,
                elapsed_ms: 0,
                message: format!(
                    "准备导出 {} 个字段，范围：{}",
                    field_count,
                    table_data_export_scope_label(form.scope)
                ),
            }],
            error: None,
            cancel_requested: false,
            canceled: false,
        });
        task_id
    }

    fn spawn_table_data_export_task(
        &mut self,
        task_id: u64,
        receiver: futures::channel::oneshot::Receiver<anyhow::Result<Option<PathBuf>>>,
        form: TableDataExportForm,
        fields: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self._data_export_cancel_flags
            .insert(task_id, cancel_flag.clone());
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let selected = receiver.await;
            let path = match selected {
                Ok(Ok(Some(path))) => Some(safe_table_data_export_path(path, form.format)),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    table_data_export_finish_on_ui(
                        &view,
                        cx,
                        task_id,
                        Err(format!("选择保存位置失败：{error}")),
                        false,
                    );
                    None
                }
                Err(error) => {
                    table_data_export_finish_on_ui(
                        &view,
                        cx,
                        task_id,
                        Err(format!("选择保存位置失败：{error}")),
                        false,
                    );
                    None
                }
            };
            let Some(path) = path else {
                table_data_export_finish_on_ui(&view, cx, task_id, Ok(()), true);
                return;
            };
            table_data_export_set_path_on_ui(&view, cx, task_id, path.clone());

            let (sender, receiver) = mpsc::channel::<TableDataExportProgress>();
            let result = cx
                .background_spawn({
                    let path = path.clone();
                    async move {
                        run_table_data_export(controller, form, fields, path, cancel_flag, sender)
                    }
                })
                .fuse();
            futures::pin_mut!(result);
            loop {
                while let Ok(progress) = receiver.try_recv() {
                    table_data_export_progress_on_ui(&view, cx, task_id, progress);
                }
                if let Some(result) = result.as_mut().now_or_never() {
                    while let Ok(progress) = receiver.try_recv() {
                        table_data_export_progress_on_ui(&view, cx, task_id, progress);
                    }
                    let canceled = matches!(result, Ok(TableDataExportResult { canceled: true }));
                    table_data_export_finish_on_ui(
                        &view,
                        cx,
                        task_id,
                        result.map(|_| ()).map_err(|error| error.to_string()),
                        canceled,
                    );
                    break;
                }
                smol::Timer::after(Duration::from_millis(100)).await;
            }
        });
        self._data_export_tasks.insert(task_id, task);
    }

    fn show_table_data_export_task_log(&mut self, task_id: u64, cx: &mut Context<Self>) {
        if self.data_export_tasks.iter().any(|task| task.id == task_id) {
            self.data_export_log_task = Some(task_id);
            cx.notify();
        }
    }

    fn close_table_data_export_log_modal(&mut self, cx: &mut Context<Self>) {
        self.data_export_log_task = None;
        cx.notify();
    }

    fn cancel_table_data_export_task(&mut self, task_id: u64, cx: &mut Context<Self>) {
        let Some(task) = self.data_export_tasks.iter_mut().find(|task| task.id == task_id) else {
            return;
        };
        if !task.running() {
            return;
        }
        task.cancel_requested = true;
        if let Some(flag) = self._data_export_cancel_flags.get(&task_id) {
            flag.store(true, Ordering::Relaxed);
        }
        self.show_message("已请求取消，当前批次完成后停止", AppMessageKind::Warning, cx);
        cx.notify();
    }

    fn copy_table_data_export_task_log(&mut self, task_id: u64, cx: &mut Context<Self>) {
        let Some(task) = self.data_export_tasks.iter().find(|task| task.id == task_id) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(table_data_export_log_text(task)));
        self.show_message("已复制日志", AppMessageKind::Success, cx);
    }

    fn clear_table_data_export_task(&mut self, task_id: u64, cx: &mut Context<Self>) {
        if self
            .data_export_tasks
            .iter()
            .any(|task| task.id == task_id && task.running())
        {
            self.show_message("任务执行中，暂不能清除", AppMessageKind::Warning, cx);
            return;
        }
        self.data_export_tasks.retain(|task| task.id != task_id);
        if self.data_export_log_task == Some(task_id) {
            self.data_export_log_task = None;
        }
        cx.notify();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TableDataExportModalClose {
    Form,
    Log,
}

impl TableDataExportModalClose {
    fn close(self, this: &mut NavicatMain, cx: &mut Context<NavicatMain>) {
        match self {
            Self::Form => this.cancel_table_data_export_modal(cx),
            Self::Log => this.close_table_data_export_log_modal(cx),
        }
    }
}

#[derive(Clone, Debug)]
struct TableDataExportProgress {
    rows: u64,
    elapsed_ms: u64,
    message: String,
}

#[derive(Clone, Debug)]
struct TableDataExportResult {
    canceled: bool,
}

fn run_table_data_export(
    controller: AppController,
    form: TableDataExportForm,
    fields: Vec<String>,
    path: PathBuf,
    cancel_flag: Arc<AtomicBool>,
    sender: mpsc::Sender<TableDataExportProgress>,
) -> anyhow::Result<TableDataExportResult> {
    let sort = match form.scope {
        TableDataExportScope::CurrentConditions => form.sort,
        TableDataExportScope::AllRows => Vec::new(),
        TableDataExportScope::CustomRules => {
            table_data_export_sort_specs_from_rules(&form.custom_sort_rules)
        }
    };
    let filters = match form.scope {
        TableDataExportScope::CurrentConditions => form.filters,
        TableDataExportScope::AllRows => Vec::new(),
        TableDataExportScope::CustomRules => data_filter_specs_from_rules(&form.custom_filter_rules),
    };
    let mut writer =
        TableDataExportWriter::create(&path, form.format, form.object.clone(), fields)?;
    let mut offset = 0;
    let mut exported = 0;
    let started_at = Instant::now();
    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            writer.finish()?;
            return Ok(TableDataExportResult { canceled: true });
        }
        let page = controller.load_data_for_export(
            &form.object,
            offset,
            DATA_EXPORT_BATCH_SIZE,
            &sort,
            &filters,
        )?;
        let rows = writer.write_page(&page)?;
        exported += rows;
        let _ = sender.send(TableDataExportProgress {
            rows,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            message: format!("已导出第 {} 批，累计 {} 行", offset / DATA_EXPORT_BATCH_SIZE + 1, exported),
        });
        if rows == 0 || !page.has_more {
            break;
        }
        offset += rows;
    }
    writer.finish()?;
    Ok(TableDataExportResult { canceled: false })
}

fn table_data_export_set_path_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    task_id: u64,
    path: PathBuf,
) {
    let _ = cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            if let Some(task) = this.data_export_tasks.iter_mut().find(|task| task.id == task_id) {
                task.path = path;
            }
            cx.notify();
        });
    });
}

fn table_data_export_progress_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    task_id: u64,
    progress: TableDataExportProgress,
) {
    let _ = cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            if let Some(task) = this.data_export_tasks.iter_mut().find(|task| task.id == task_id) {
                task.exported_rows += progress.rows;
                task.batch_count += 1;
                task.logs.push(TableDataExportLogEntry {
                    index: task.logs.len() + 1,
                    success: true,
                    elapsed_ms: progress.elapsed_ms,
                    message: progress.message,
                });
            }
            cx.notify();
        });
    });
}

fn table_data_export_finish_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    task_id: u64,
    result: Result<(), String>,
    canceled: bool,
) {
    let _ = cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            this._data_export_tasks.remove(&task_id);
            this._data_export_cancel_flags.remove(&task_id);
            if let Some(task) = this.data_export_tasks.iter_mut().find(|task| task.id == task_id) {
                task.finished_at = Some(Instant::now());
                task.canceled = canceled;
                if let Err(error) = result {
                    task.error = Some(error.clone());
                    task.logs.push(TableDataExportLogEntry {
                        index: task.logs.len() + 1,
                        success: false,
                        elapsed_ms: task.started_at.elapsed().as_millis() as u64,
                        message: error,
                    });
                } else if canceled {
                    task.logs.push(TableDataExportLogEntry {
                        index: task.logs.len() + 1,
                        success: true,
                        elapsed_ms: task.started_at.elapsed().as_millis() as u64,
                        message: "导出已取消".to_string(),
                    });
                } else {
                    task.logs.push(TableDataExportLogEntry {
                        index: task.logs.len() + 1,
                        success: true,
                        elapsed_ms: task.started_at.elapsed().as_millis() as u64,
                        message: format!("导出完成，共 {} 行", task.exported_rows),
                    });
                }
                let message = table_data_export_task_message(task);
                let kind = if task.error.is_some() {
                    AppMessageKind::Error
                } else if task.canceled {
                    AppMessageKind::Warning
                } else {
                    AppMessageKind::Success
                };
                this.show_message(message, kind, cx);
            }
            cx.notify();
        });
    });
}

fn table_data_export_preview_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    seq: u64,
    key: String,
    result: fluxdb_core::Result<DataExportPreview>,
) {
    let _ = cx.update(|cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        view.update(cx, |this, cx| {
            if this.data_export_preview_seq != seq {
                return;
            }
            this.data_export_preview = Some(TableDataExportPreviewState {
                key,
                status: match result {
                    Ok(preview) => TableDataExportPreviewStatus::Ready {
                        sql: preview.sql,
                        row_count: preview.row_count,
                    },
                    Err(error) => TableDataExportPreviewStatus::Failed(error.to_string()),
                },
            });
            cx.notify();
        });
    });
}

fn table_data_export_preview_request(
    form: &TableDataExportForm,
) -> (Vec<String>, Vec<SortSpec>, Vec<FilterSpec>) {
    let fields = table_data_export_selected_fields(form);
    let (sort, filters) = match form.scope {
        TableDataExportScope::CurrentConditions => (form.sort.clone(), form.filters.clone()),
        TableDataExportScope::AllRows => (Vec::new(), Vec::new()),
        TableDataExportScope::CustomRules => (
            table_data_export_sort_specs_from_rules(&form.custom_sort_rules),
            data_filter_specs_from_rules(&form.custom_filter_rules),
        ),
    };
    (fields, sort, filters)
}

fn table_data_export_selected_fields(form: &TableDataExportForm) -> Vec<String> {
    form.columns
        .iter()
        .filter(|column| form.selected_fields.contains(&column.name))
        .map(|column| column.name.clone())
        .collect()
}

fn table_data_export_preview_key(form: &TableDataExportForm) -> String {
    let (fields, sort, filters) = table_data_export_preview_request(form);
    format!(
        "{:?}|{:?}|{:?}|{:?}",
        form.object, form.scope, fields, sort
    ) + &format!("|{:?}", filters)
}

fn table_data_export_modal(
    form: TableDataExportForm,
    database_select: Entity<SelectState<SearchableVec<String>>>,
    table_select: Entity<SelectState<SearchableVec<String>>>,
    preview: Option<TableDataExportPreviewState>,
    custom_conditions_open: bool,
    custom_filter_rules: Vec<DataFilterRule>,
    custom_sort_rules: Vec<DataSortRule>,
    data_filter_popover: Option<DataFilterPopover>,
    data_filter_value_input: Entity<InputState>,
    data_filter_search_input: Entity<InputState>,
    data_filter_value_search: String,
    data_filter_value_search_loading: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let can_export = !form.selected_fields.is_empty();
    let active_tab = form.active_tab;
    let panel = data_export_modal_panel(TableDataExportModalClose::Form, colors, cx)
        .child(data_export_modal_header(
            "导出数据",
            TableDataExportModalClose::Form,
            colors,
            cx,
        ))
        .child(data_export_tabs(active_tab, None, colors, cx))
        .child(match active_tab {
            TableDataExportTab::General => data_export_general_body(
                form.clone(),
                database_select,
                table_select,
                window,
                colors,
                cx,
            ),
            TableDataExportTab::Fields => data_export_general_body(
                form.clone(),
                database_select,
                table_select,
                window,
                colors,
                cx,
            ),
            TableDataExportTab::Conditions => data_export_conditions_body(
                form.clone(),
                preview.clone(),
                window,
                colors,
                cx,
            ),
            TableDataExportTab::Log => data_export_empty_log_body(colors),
        })
        .child(data_export_modal_footer(can_export, None, colors, cx));
    data_export_modal_shell(panel, TableDataExportModalClose::Form, colors, cx).when(
        custom_conditions_open,
        |this| {
        this.child(table_data_export_custom_conditions_modal(
            form,
            custom_filter_rules,
            custom_sort_rules,
            data_filter_popover,
            data_filter_value_input,
            data_filter_search_input,
            data_filter_value_search,
            data_filter_value_search_loading,
            colors,
            cx,
        ))
    })
}

fn table_data_export_log_modal(
    task: TableDataExportTaskState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let task_id = task.id;
    let finished = !task.running();
    let cancel_requested = task.cancel_requested;
    let panel = data_export_modal_panel(TableDataExportModalClose::Log, colors, cx)
        .child(data_export_modal_header(
            "导出数据",
            TableDataExportModalClose::Log,
            colors,
            cx,
        ))
        .child(data_export_tabs(TableDataExportTab::Log, Some(task_id), colors, cx))
        .child(data_export_log_body(task, colors))
        .child(data_export_modal_footer(
            false,
            Some((task_id, finished, cancel_requested)),
            colors,
            cx,
        ));
    data_export_modal_shell(panel, TableDataExportModalClose::Log, colors, cx)
}

fn data_export_modal_shell(
    panel: Div,
    close_action: TableDataExportModalClose,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
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
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
            close_action.close(this, cx);
            cx.stop_propagation();
        }))
        .child(panel)
}

fn data_export_modal_panel(
    close_action: TableDataExportModalClose,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w(px(780.))
        .max_w(px(780.))
        .h(px(560.))
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
        .text_size(px(13.))
        .text_color(colors.text)
        .key_context("TableDataExportModal")
        .on_action(cx.listener(move |this, _: &CancelDialog, _, cx| {
            close_action.close(this, cx);
            cx.stop_propagation();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn data_export_modal_header(
    title: &'static str,
    close_action: TableDataExportModalClose,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
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
                .gap_3()
                .child(app_icon(AppIcon::Save, 18., colors.text))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
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
                    cx.listener(move |this, _, _, cx| {
                        close_action.close(this, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn data_export_tabs(
    active: TableDataExportTab,
    task_id: Option<u64>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .px_5()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(data_export_tab_button("常规", active == TableDataExportTab::General || active == TableDataExportTab::Fields, TableDataExportTab::General, task_id, colors, cx))
        .child(data_export_tab_button("条件", active == TableDataExportTab::Conditions, TableDataExportTab::Conditions, task_id, colors, cx))
        .child(data_export_tab_button("消息日志", active == TableDataExportTab::Log, TableDataExportTab::Log, task_id, colors, cx))
}

fn data_export_tab_button(
    label: &'static str,
    active: bool,
    tab: TableDataExportTab,
    task_id: Option<u64>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(34.))
        .px_3()
        .flex()
        .items_center()
        .border_b_2()
        .border_color(if active { rgb(0x1687ff) } else { colors.border_soft })
        .text_size(px(13.))
        .text_color(if active { colors.text } else { colors.muted })
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if let Some(task_id) = task_id {
                    this.show_table_data_export_task_log(task_id, cx);
                } else {
                    this.select_table_data_export_tab(tab, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(label)
}

fn data_export_general_body(
    form: TableDataExportForm,
    database_select: Entity<SelectState<SearchableVec<String>>>,
    table_select: Entity<SelectState<SearchableVec<String>>>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .px_5()
        .py_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(data_export_object_picker(
            database_select,
            table_select,
            window,
            colors,
            cx,
        ))
        .child(data_export_format_grid(form.format, colors, cx))
        .child(data_export_fields_picker(form, colors, cx).flex_1().min_h(px(0.)))
}

fn data_export_object_picker(
    database_select: Entity<SelectState<SearchableVec<String>>>,
    table_select: Entity<SelectState<SearchableVec<String>>>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(data_export_form_row_label("对象", colors))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(34.))
                .flex()
                .items_center()
                .gap_3()
                .child(data_export_object_select_field(
                    "库",
                    database_select,
                    true,
                    window,
                    colors,
                    cx,
                ))
                .child(data_export_object_select_field(
                    "表",
                    table_select,
                    false,
                    window,
                    colors,
                    cx,
                )),
        )
}

fn data_export_object_select_field(
    label: &'static str,
    select: Entity<SelectState<SearchableVec<String>>>,
    disabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_none()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(data_export_object_select_box(
            select, disabled, window, colors, cx,
        ))
}

fn data_export_object_select_box(
    select: Entity<SelectState<SearchableVec<String>>>,
    disabled: bool,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = select.read(cx).focus_handle(cx).is_focused(window);
    div()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(create_table_input_border_color(focused && !disabled, colors))
        .bg(if disabled { colors.panel_alt } else { colors.input_bg })
        .overflow_hidden()
        .flex()
        .items_center()
        .when(!disabled, |this| {
            this.hover(move |style| {
                style.border_color(create_table_input_hover_border_color(focused, colors))
            })
        })
        .child(
            Select::new(&select)
                .appearance(false)
                .small()
                .placeholder("选择...")
                .search_placeholder("选择表...")
                .disabled(disabled)
                .w_full()
                .h_full()
                .menu_width(px(320.)),
        )
}

fn data_export_format_grid(
    active: TableDataExportFormat,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let grid = div()
        .flex()
        .items_center()
        .gap_3()
        .child(data_export_form_row_label("格式", colors));
    let mut row = div().flex_1().min_w(px(0.)).flex().gap_2();
    for format in TableDataExportFormat::all() {
        let format_value = *format;
        row = row.child(data_export_choice_chip(format.label(), active == format_value, colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.set_table_data_export_format(format_value, cx);
                cx.stop_propagation();
            }),
        ));
    }
    grid.child(row)
}

fn data_export_fields_picker(
    form: TableDataExportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected = form.selected_fields.len();
    let total = form.columns.len();
    let mut list = div().flex().flex_col().gap_1().p_2();
    for (field_index, column) in form.columns.into_iter().enumerate() {
        let checked = form.selected_fields.contains(&column.name);
        list = list.child(data_export_field_row(field_index, column, checked, colors, cx));
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(data_export_section_label("字段", colors))
                        .child(
                            div()
                                .text_color(colors.muted)
                                .child(format!("{selected}/{total}")),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("data-export-select-all-fields")
                                .xsmall()
                                .label("全选")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.select_all_table_data_export_fields(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("data-export-invert-fields")
                                .xsmall()
                                .label("反选")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.invert_table_data_export_fields(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("data-export-select-visible-fields")
                                .xsmall()
                                .label("仅当前可见字段")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.select_visible_table_data_export_fields(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
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

fn data_export_field_row(
    field_index: usize,
    column: TableDataExportColumn,
    checked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let name = column.name.clone();
    div()
        .min_h(px(32.))
        .px_2()
        .py_1()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_table_data_export_field(name.clone(), cx);
                cx.stop_propagation();
            }),
        )
        .child(Checkbox::new(("data-export-field", field_index)).checked(checked))
        .child(div().w(px(180.)).font_weight(gpui::FontWeight::SEMIBOLD).child(column.name))
        .child(
            div()
                .w(px(140.))
                .text_color(colors.muted)
                .child(column.type_name.unwrap_or_else(|| "unknown".to_string())),
        )
        .when(column.primary_key, |this| this.child(data_export_badge("PK", colors)))
        .when(!column.nullable, |this| this.child(data_export_badge("NOT NULL", colors)))
        .when_some(column.comment, |this, comment| {
            this.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .truncate()
                    .text_color(colors.muted)
                    .child(comment),
            )
        })
}

fn data_export_conditions_body(
    form: TableDataExportForm,
    preview: Option<TableDataExportPreviewState>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .px_5()
        .py_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(data_export_section_label("导出范围", colors))
        .child(data_export_scope_segments(form.scope, colors, cx))
        .when(form.scope == TableDataExportScope::CustomRules, |this| {
            this.child(data_export_custom_summary(&form, colors, cx))
        })
        .child(data_export_preview_card(&form, preview, window, colors, cx))
}

fn data_export_scope_segments(
    active: TableDataExportScope,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(42.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .p_1()
        .flex()
        .gap_1()
        .child(data_export_scope_segment(
            "当前筛选条件",
            active == TableDataExportScope::CurrentConditions,
            TableDataExportScope::CurrentConditions,
            colors,
            cx,
        ))
        .child(data_export_scope_segment(
            "导出全部",
            active == TableDataExportScope::AllRows,
            TableDataExportScope::AllRows,
            colors,
            cx,
        ))
        .child(data_export_scope_segment(
            "自定义条件",
            active == TableDataExportScope::CustomRules,
            TableDataExportScope::CustomRules,
            colors,
            cx,
        ))
}

fn data_export_scope_segment(
    title: &'static str,
    selected: bool,
    scope: TableDataExportScope,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_1()
        .h_full()
        .rounded(colors.radius * 0.5)
        .bg(if selected {
            if colors.is_dark { rgb(0x16345f) } else { rgb(0xe8f2ff) }
        } else {
            colors.input_bg
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if selected { colors.text } else { colors.muted })
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.set_table_data_export_scope(scope, cx);
                cx.stop_propagation();
            }),
        )
        .child(title)
}

fn data_export_preview_card(
    form: &TableDataExportForm,
    preview: Option<TableDataExportPreviewState>,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let expected_key = table_data_export_preview_key(form);
    let status = preview
        .filter(|preview| preview.key == expected_key)
        .map(|preview| preview.status)
        .unwrap_or(TableDataExportPreviewStatus::Loading);
    let (count_label, sql, failed) = match status {
        TableDataExportPreviewStatus::Loading => ("统计中...".to_string(), "正在生成 SQL 预览...".to_string(), false),
        TableDataExportPreviewStatus::Ready { sql, row_count } => {
            (format!("{} 行", table_data_export_format_count(row_count)), sql, false)
        }
        TableDataExportPreviewStatus::Failed(error) => ("未知".to_string(), error, true),
    };
    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("预计导出"))
                        .child(
                            div()
                                .text_color(if failed { rgb(0xd64545) } else { colors.text })
                                .child(count_label),
                        )
                        .child(
                            Button::new("data-export-refresh-preview")
                                .xsmall()
                                .label("刷新")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.refresh_table_data_export_preview(true, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
        .child(data_export_sql_preview_box(
            "data-export-sql-preview",
            sql,
            window,
            colors,
            cx,
        ))
}

fn data_export_sql_preview_box(
    key: &'static str,
    sql: String,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let editor = window.use_keyed_state(key, cx, {
        let sql = sql.clone();
        move |window, cx| {
            InputState::new(window, cx)
                .code_editor(SQL_HIGHLIGHT_LANGUAGE)
                .line_number(false)
                .legacy_soft_wrap(false)
                .default_value(sql)
        }
    });
    editor.update(cx, |state, cx| {
        if state.value().to_string() != sql {
            state.set_value(sql.clone(), window, cx);
        }
    });

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child("SQL 预览"),
        )
        .child(
            div()
                .h(px(102.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border_soft)
                .bg(colors.panel_alt)
                .overflow_hidden()
                .child(
                    Input::new(&editor)
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false)
                        .text_size(px(12.))
                        .font_family(EDITOR_FONT)
                        .p_2()
                        .size_full(),
                ),
        )
}

fn data_export_custom_summary(
    form: &TableDataExportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let filter_count = form.custom_filter_rules.iter().filter(|rule| rule.enabled).count();
    let sort_count = form.custom_sort_rules.iter().filter(|rule| rule.enabled).count();
    let has_rules = filter_count > 0 || sort_count > 0;
    div()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("自定义条件"))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(if has_rules {
                            format!("筛选 {filter_count} 个 · 排序 {sort_count} 个")
                        } else {
                            "未设置筛选和排序，将按全表导出".to_string()
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .when(has_rules, |this| {
                    this.child(
                        Button::new("data-export-clear-custom")
                            .xsmall()
                            .label("清空")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_table_data_export_custom_conditions(cx);
                                cx.stop_propagation();
                            })),
                    )
                })
                .child(
                    Button::new("data-export-edit-custom")
                        .xsmall()
                        .label(if has_rules { "编辑条件" } else { "设置条件" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_table_data_export_custom_conditions(cx);
                            cx.stop_propagation();
                        })),
                ),
        )
}

fn table_data_export_custom_conditions_modal(
    form: TableDataExportForm,
    custom_filter_rules: Vec<DataFilterRule>,
    custom_sort_rules: Vec<DataSortRule>,
    data_filter_popover: Option<DataFilterPopover>,
    data_filter_value_input: Entity<InputState>,
    data_filter_search_input: Entity<InputState>,
    data_filter_value_search: String,
    data_filter_value_search_loading: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let export_filter_tab_id = table_data_export_filter_tab_id(form.tab_id);
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.46)
        } else {
            opaque_grey(0.75, 0.24)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_table_data_export_custom_conditions(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .relative()
                .w(px(720.))
                .h(px(430.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(menu_surface_bg(colors))
                .shadow(vec![box_shadow(
                    px(0.),
                    px(16.),
                    px(34.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.44 } else { 0.18 }),
                )])
                .flex()
                .flex_col()
                .key_context("TableDataExportCustomConditionsModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_table_data_export_custom_conditions(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(data_export_custom_modal_header(colors, cx))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .p_4()
                        .child(data_export_filter_sort_builder(
                            export_filter_tab_id,
                            &form,
                            custom_filter_rules.clone(),
                            custom_sort_rules.clone(),
                            colors,
                            cx,
                        )),
                )
                .child(data_export_custom_modal_footer(colors, cx))
                .when_some(
                    data_filter_popover.filter(|popover| popover.tab_id == export_filter_tab_id),
                    |this, popover| {
                        this.child(data_filter_popover_layer(
                            export_filter_tab_id,
                            &table_data_export_filter_page(&form),
                            custom_filter_rules.as_slice(),
                            custom_sort_rules.as_slice(),
                            popover,
                            data_filter_value_input,
                            data_filter_search_input,
                            data_filter_value_search.as_str(),
                            data_filter_value_search_loading,
                            colors,
                            cx,
                        ))
                    },
                ),
        )
}

fn data_export_custom_modal_header(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .h(px(52.))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("设置导出条件"),
        )
        .child(
            div()
                .size(px(28.))
                .rounded(colors.radius)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 15., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_table_data_export_custom_conditions(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn data_export_custom_modal_footer(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .h(px(56.))
        .flex_none()
        .px_4()
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .child(
            Button::new("data-export-custom-cancel")
                .label("取消")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_table_data_export_custom_conditions(cx);
                    cx.stop_propagation();
                })),
        )
        .child(
            Button::new("data-export-custom-apply")
                .label("应用")
                .primary()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.apply_table_data_export_custom_conditions(cx);
                    cx.stop_propagation();
                })),
        )
}

fn data_export_filter_sort_builder(
    export_filter_tab_id: TabId,
    form: &TableDataExportForm,
    filter_rules: Vec<DataFilterRule>,
    sort_rules: Vec<DataSortRule>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let page = table_data_export_filter_page(form);
    let default_field = page.columns.first().map(|column| column.name.clone());
    div()
        .relative()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(data_export_builder_title("筛选", colors))
        .child(data_filter_builder_rows(
            export_filter_tab_id,
            filter_rules.as_slice(),
            default_field.clone(),
            colors,
            cx,
        ))
        .child(div().h(px(1.)).bg(colors.border_soft))
        .child(data_sort_builder_section(
            export_filter_tab_id,
            sort_rules.as_slice(),
            default_field,
            colors,
            cx,
        ))
}

fn data_export_builder_title(label: &'static str, colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .px_3()
        .flex()
        .items_center()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(label)
}

fn table_data_export_filter_tab_id(tab_id: TabId) -> TabId {
    TabId(u64::MAX - tab_id.0)
}

fn table_data_export_filter_page(form: &TableDataExportForm) -> DataPage {
    DataPage {
        columns: form
            .columns
            .iter()
            .map(|column| GdbColumn {
                name: column.name.clone(),
                type_name: column.type_name.clone(),
                nullable: column.nullable,
                primary_key: column.primary_key,
                comment: column.comment.clone(),
            })
            .collect(),
        rows: Vec::new(),
        offset: 0,
        limit: 0,
        has_more: false,
    }
}

fn data_export_empty_log_body(colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(0.))
        .p_5()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child("开始导出后显示消息日志")
}

fn data_export_log_body(task: TableDataExportTaskState, colors: UiColors) -> Div {
    let elapsed = task
        .finished_at
        .unwrap_or_else(Instant::now)
        .saturating_duration_since(task.started_at)
        .as_millis();
    let mut list = div().flex().flex_col().gap_1().p_3();
    if let Some(error) = &task.error {
        list = list.child(sql_file_log_error(error.clone(), colors));
    }
    let hidden_count = task.logs.len().saturating_sub(DATA_EXPORT_VISIBLE_LOG_LIMIT);
    if hidden_count > 0 {
        list = list.child(
            div()
                .text_color(colors.muted)
                .text_size(px(12.))
                .px_2()
                .py_1()
                .child(format!("已隐藏前 {hidden_count} 条日志，完整日志可复制")),
        );
    }
    for entry in task.logs.iter().skip(hidden_count) {
        list = list.child(data_export_log_row(entry, colors));
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .p_5()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_5()
                .text_size(px(14.))
                .child(format!("表: {}", task.table_name))
                .child(format!("格式: {}", task.format.label()))
                .child(format!("字段: {}", task.field_count))
                .child(format!("行数: {}", task.exported_rows))
                .child(format!("批次: {}", task.batch_count))
                .child(format!("状态: {}", table_data_export_task_status(&task)))
                .child(format!("时间: {elapsed} ms")),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .overflow_hidden()
                .child(if task.path.as_os_str().is_empty() {
                    "等待选择保存位置".to_string()
                } else {
                    task.path.display().to_string()
                }),
        )
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

fn data_export_log_row(entry: &TableDataExportLogEntry, colors: UiColors) -> Div {
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
        .child(div().flex_1().min_w(px(0.)).overflow_hidden().child(entry.message.clone()))
}

fn data_export_modal_footer(
    can_export: bool,
    task: Option<(u64, bool, bool)>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
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
        .when_some(task, |this, (task_id, finished, cancel_requested)| {
            this.child(
                Button::new("data-export-copy-log")
                    .label("复制日志")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.copy_table_data_export_task_log(task_id, cx);
                        cx.stop_propagation();
                    })),
            )
            .when(!finished, |this| {
                this.child(
                    Button::new("data-export-cancel-task")
                        .label(if cancel_requested { "停止中" } else { "取消导出" })
                        .disabled(cancel_requested)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cancel_table_data_export_task(task_id, cx);
                            cx.stop_propagation();
                        })),
                )
            })
            .when(finished, |this| {
                this.child(
                    Button::new("data-export-clear-task")
                        .label("清除任务")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.clear_table_data_export_task(task_id, cx);
                            cx.stop_propagation();
                        })),
                )
            })
        })
        .child(
            Button::new("data-export-close")
                .label("关闭")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_table_data_export_modal(cx);
                    this.data_export_log_task = None;
                    cx.stop_propagation();
                })),
        )
        .when(task.is_none(), |this| {
            this.child(
                Button::new("data-export-confirm")
                    .label("导出")
                    .disabled(!can_export)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_table_data_export(cx);
                        cx.stop_propagation();
                    })),
            )
        })
}

fn data_export_section_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(label)
}

fn data_export_form_row_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .w(px(42.))
        .flex_none()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(label)
}

fn data_export_choice_chip(label: &'static str, selected: bool, colors: UiColors) -> Div {
    div()
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
        .text_size(px(12.))
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn data_export_badge(label: &'static str, colors: UiColors) -> Div {
    div()
        .h(px(20.))
        .px_2()
        .rounded(colors.radius)
        .bg(colors.panel_alt)
        .text_size(px(11.))
        .text_color(colors.muted)
        .flex()
        .items_center()
        .child(label)
}

fn table_data_export_scope_label(scope: TableDataExportScope) -> &'static str {
    match scope {
        TableDataExportScope::CurrentConditions => "当前筛选条件",
        TableDataExportScope::AllRows => "全部数据",
        TableDataExportScope::CustomRules => "自定义筛选 & 排序",
    }
}

fn data_export_object_database_label(object: &ObjectPath) -> String {
    object
        .database
        .clone()
        .unwrap_or_else(|| "默认数据库".to_string())
}

fn data_export_select_index(options: &[String], value: &str) -> Option<IndexPath> {
    options
        .iter()
        .position(|item| item == value)
        .map(IndexPath::new)
}

fn table_data_export_format_count(count: u64) -> String {
    let text = count.to_string();
    let mut formatted = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    formatted.chars().rev().collect()
}

fn table_data_export_sort_specs_from_rules(rules: &[DataSortRule]) -> Vec<SortSpec> {
    rules
        .iter()
        .filter(|rule| rule.enabled)
        .map(|rule| SortSpec {
            field: rule.field.clone(),
            direction: if rule.ascending {
                SortDirection::Asc
            } else {
                SortDirection::Desc
            },
        })
        .collect()
}

fn table_data_export_task_status(task: &TableDataExportTaskState) -> &'static str {
    if task.running() && task.cancel_requested {
        "停止中"
    } else if task.running() {
        "导出中"
    } else if task.canceled {
        "已取消"
    } else if task.error.is_some() {
        "失败"
    } else {
        "完成"
    }
}

fn table_data_export_task_message(task: &TableDataExportTaskState) -> String {
    if task.canceled {
        format!("已取消导出 {}，已写入 {} 行", task.table_name, task.exported_rows)
    } else if task.error.is_some() {
        format!("导出 {} 失败", task.table_name)
    } else if task.path.as_os_str().is_empty() {
        "已取消选择保存位置".to_string()
    } else {
        format!(
            "已导出 {} 行为 {}：{}",
            task.exported_rows,
            task.format.label(),
            task.path.display()
        )
    }
}

fn table_data_export_log_text(task: &TableDataExportTaskState) -> String {
    let mut lines = vec![
        format!("任务: {}", task.id),
        format!("表: {}", task.table_name),
        format!("格式: {}", task.format.label()),
        format!("文件: {}", task.path.display()),
        format!("字段数: {}", task.field_count),
        format!("行数: {}", task.exported_rows),
        format!("状态: {}", table_data_export_task_status(task)),
    ];
    if let Some(error) = &task.error {
        lines.push(format!("错误: {error}"));
    }
    for entry in &task.logs {
        lines.push(format!(
            "#{} [{}] {} ms {}",
            entry.index,
            if entry.success { "成功" } else { "失败" },
            entry.elapsed_ms,
            entry.message
        ));
    }
    lines.join("\n")
}

fn table_data_export_statusbar_area(
    tasks: &[TableDataExportTaskState],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(task) = tasks.iter().rev().find(|task| task.running()).or_else(|| tasks.last()) else {
        return div().w(px(180.));
    };
    let running_count = tasks.iter().filter(|task| task.running()).count();
    let label = if running_count > 1 {
        format!("{running_count} 个导出任务运行中")
    } else if task.cancel_requested {
        format!("导出停止中 {} 行", task.exported_rows)
    } else if task.running() {
        format!("导出中 {} 行", task.exported_rows)
    } else if task.canceled {
        format!("导出已取消 {} 行", task.exported_rows)
    } else if task.error.is_some() {
        format!("{} 导出失败", task.table_name)
    } else {
        format!("{} 导出完成", task.table_name)
    };
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
                        this.show_table_data_export_task_log(task_id, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(
                    if task.running() { AppIcon::Play } else { AppIcon::Save },
                    13.,
                    if task.error.is_some() {
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
