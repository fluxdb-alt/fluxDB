// Included in crate-root scope by ../menus_dialogs.rs; 查询结果导出（SQL 即导出条件）。
//
// 与表数据导出共用弹框外壳、格式网格、任务记录与 `.partial` 写盘语义，
// 但取数走「按 SQL 分批执行」，不构造表过滤条件。

impl NavicatMain {
    /// 从查询结果页工具栏打开导出：条件预填该结果实际执行的 SQL，可在弹框内编辑。
    fn open_query_result_export(
        &mut self,
        key: QueryPaneKey,
        result_index: usize,
        sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((connection_id, database, schema, columns)) = self
            .controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == key.tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::QueryEditor(editor) => editor.pane(key.pane_id).map(|pane| {
                    let columns = query_result_page_index(pane, result_index)
                        .and_then(|page_index| pane.results.get(page_index))
                        .map(|page| {
                            page.columns
                                .iter()
                                .map(|column| TableDataExportColumn {
                                    name: column.name.clone(),
                                    type_name: column.type_name.clone(),
                                    nullable: column.nullable,
                                    primary_key: column.primary_key,
                                    comment: column.comment.clone(),
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    (
                        editor.connection_id,
                        editor.database.clone(),
                        editor.schema.clone(),
                        columns,
                    )
                }),
                _ => None,
            })
        else {
            return;
        };

        self.cancel_table_data_export_modal(cx);
        // 默认导出全部结果列；字段选择按列下标记录，允许重名列。
        let selected_fields = (0..columns.len()).collect();
        self.pending_query_result_export = Some(QueryResultExportForm {
            connection_id,
            database,
            schema,
            sql: sql.clone(),
            columns,
            selected_fields,
            format: TableDataExportFormat::Csv,
            active_tab: TableDataExportTab::General,
        });
        let editor = self.create_query_result_export_editor(sql, connection_id, window, cx);
        let subscription = cx.subscribe(&editor, |this, editor, event, cx| {
            if matches!(event, editor_component::EditorEvent::Changed(_))
                && let Some(form) = &mut this.pending_query_result_export
            {
                form.sql = editor.read(cx).text();
                cx.notify();
            }
        });
        self.query_result_export_editor = Some(editor);
        self._query_result_export_editor_subscription = Some(subscription);
        self.refresh_query_result_export_preview(true, cx);
        cx.notify();
    }

    /// 导出 SQL 编辑器：复用通用编辑器内核（行号/折行/查找），但不挂 SQL 执行快捷键，
    /// 避免弹框内误触执行。内容以编辑器为准，表单里的 `sql` 仅作快照。
    fn create_query_result_export_editor(
        &mut self,
        sql: String,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<editor_component::Editor> {
        let database_kind = self.connection_database_kind(connection_id);
        let dialect = sql_editor_adapter::SqlDialect::from_database_kind(database_kind);
        let settings = self.controller.state().settings.clone();
        // 只挂语法 provider：导出 SQL 不需要补全/执行，避免弹框内触发查询副作用。
        let adapter = Arc::new(sql_editor_adapter::SqlAdapter::new(dialect, database_kind));
        let language_registry = Arc::new(fluxdb_editor_core::LanguageRegistry::new());
        language_registry.register(
            dialect.language_id(),
            Arc::new(sql_editor_adapter::SqlLanguage::new(dialect, database_kind)),
            Some(adapter.clone() as _),
        );
        let providers = editor_component::Providers {
            language_registry: Some(language_registry),
            syntax: Some(adapter as _),
            ..Default::default()
        };
        let profile = fluxdb_editor_core::EditorProfile {
            language_id: dialect.language_id().to_string(),
            show_line_numbers: true,
            show_folding: false,
            soft_wrap: fluxdb_editor_core::SoftWrapMode::EditorWidth,
            tab_size: settings.editor_tab_width.max(1) as usize,
            ..Default::default()
        };
        let config = fluxdb_editor_core::EditorConfig {
            profile,
            font: code_font_family(&settings),
            font_size: settings.editor_font_size.clamp(10, 24) as f32,
            line_height: settings.editor_line_height.clamp(13, 28) as f32,
            gutter_line_numbers: true,
        };
        let editor = cx.new(|cx| {
            editor_component::Editor::new(sql, providers, Some(config), window, cx)
        });
        let theme = self.editor_theme_for(cx);
        editor.update(cx, |editor, cx| editor.set_theme(theme, cx));
        editor
    }

    fn cancel_query_result_export_modal(&mut self, cx: &mut Context<Self>) {
        self.pending_query_result_export = None;
        self.query_result_export_editor = None;
        self._query_result_export_editor_subscription = None;
        if self.pending_data_export.is_none() {
            self.data_export_preview = None;
        }
        cx.notify();
    }

    fn select_query_result_export_tab(
        &mut self,
        tab: TableDataExportTab,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.pending_query_result_export {
            form.active_tab = tab;
            cx.notify();
        }
    }

    fn set_query_result_export_format(
        &mut self,
        format: TableDataExportFormat,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.pending_query_result_export {
            form.format = format;
            cx.notify();
        }
    }

    fn toggle_query_result_export_field(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_query_result_export {
            if !form.selected_fields.remove(&index) {
                form.selected_fields.insert(index);
            }
            cx.notify();
        }
    }

    fn select_all_query_result_export_fields(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_query_result_export {
            form.selected_fields = (0..form.columns.len()).collect();
            cx.notify();
        }
    }

    fn invert_query_result_export_fields(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.pending_query_result_export {
            let next = (0..form.columns.len())
                .filter(|index| !form.selected_fields.contains(index))
                .collect();
            form.selected_fields = next;
            cx.notify();
        }
    }

    fn query_result_export_sql(&self, cx: &App) -> Option<String> {
        self.query_result_export_editor
            .as_ref()
            .map(|editor| editor.read(cx).text())
            .or_else(|| {
                self.pending_query_result_export
                    .as_ref()
                    .map(|form| form.sql.clone())
            })
    }

    /// 统计当前 SQL 的结果行数；SQL 不可包裹成子查询时按失败展示，不影响导出。
    fn refresh_query_result_export_preview(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(form) = self.pending_query_result_export.clone() else {
            return;
        };
        let Some(sql) = self.query_result_export_sql(cx) else {
            return;
        };
        let key = query_result_export_preview_key(&sql);
        if !force
            && self
                .data_export_preview
                .as_ref()
                .is_some_and(|preview| preview.key == key)
        {
            return;
        }
        let database_kind = self.connection_database_kind(form.connection_id);
        let count_sql = query_result_export_count_sql(&sql, database_kind);
        self.data_export_preview_seq = self.data_export_preview_seq.saturating_add(1);
        let seq = self.data_export_preview_seq;
        self.data_export_preview = Some(TableDataExportPreviewState {
            key: key.clone(),
            status: TableDataExportPreviewStatus::Loading,
        });
        let controller = self.controller.clone();
        self._data_export_preview_task = Some(cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    controller
                        .execute_query_text_page_for_export(
                            form.connection_id,
                            form.database,
                            form.schema,
                            count_sql,
                            0,
                            1,
                            &|| false,
                        )
                        .map(|execution| query_export_count_from_execution(&execution))
                })
                .await;
            query_result_export_preview_on_ui(&view, cx, seq, key, result);
        }));
    }

    fn confirm_query_result_export(&mut self, cx: &mut Context<Self>) {
        let Some(mut form) = self.pending_query_result_export.clone() else {
            return;
        };
        let sql = self
            .query_result_export_sql(cx)
            .unwrap_or_else(|| form.sql.clone())
            .trim()
            .to_string();
        if sql.is_empty() {
            self.show_message(
                tr("app.data_export.query_sql_empty"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        if form.selected_fields.is_empty() {
            self.show_message(
                tr("app.data_export.select_field"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        form.sql = sql;
        let suggested = query_result_export_suggested_name(&form);
        let receiver = cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested));
        let task_id = self.create_query_result_export_task(&form);
        self.data_export_log_task = Some(task_id);
        self.pending_query_result_export = None;
        self.query_result_export_editor = None;
        self._query_result_export_editor_subscription = None;
        self.data_export_preview = None;
        self.show_message(
            tr("app.data_export.choose_path"),
            AppMessageKind::Success,
            cx,
        );
        self.spawn_data_export_task(
            task_id,
            receiver,
            DataExportJob::Query(form),
            cx,
        );
        cx.notify();
    }

    /// 查询结果导出任务：日志里的表名用「查询结果」，范围标记为 SQL。
    fn create_query_result_export_task(&mut self, form: &QueryResultExportForm) -> u64 {
        self.create_data_export_task(
            tr("app.data_export.query_result").to_string(),
            form.format,
            form.selected_fields.len(),
            tr("app.data_export.scope_query").to_string(),
        )
    }
}

fn run_query_result_export(
    controller: AppController,
    form: QueryResultExportForm,
    path: PathBuf,
    cancel_flag: Arc<AtomicBool>,
    sender: mpsc::Sender<TableDataExportProgress>,
) -> anyhow::Result<TableDataExportResult> {
    let db_kind = controller
        .state()
        .connections
        .iter()
        .find(|c| c.config.id == form.connection_id)
        .map(|c| c.config.kind)
        .unwrap_or(DatabaseKind::MySql);
    let object = ObjectPath {
        connection_id: form.connection_id,
        database: form.database.clone(),
        schema: form.schema.clone(),
        name: tr("app.data_export.query_result").to_string(),
        kind: ObjectKind::Table,
    };
    let temp_path = table_data_export_temp_path(&path);
    // 表头/字段以首屏结果的列为准：用户可能改过 SQL，弹框打开时缓存的列已过期。
    let mut writer: Option<TableDataExportWriter> = None;
    let mut offset = 0u64;
    let mut exported = 0u64;
    let started_at = Instant::now();
    let canceled = loop {
        if cancel_flag.load(Ordering::Relaxed) {
            break true;
        }
        let execution = controller.execute_query_text_page_for_export(
            form.connection_id,
            form.database.clone(),
            form.schema.clone(),
            form.sql.clone(),
            offset,
            DATA_EXPORT_BATCH_SIZE,
            &|| cancel_flag.load(Ordering::Relaxed),
        )?;
        let page = execution
            .results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!(tr("app.data_export.query_no_result")))?;
        if let Some(incomplete) = &page.incomplete {
            let _ = fs::remove_file(&temp_path);
            return Err(anyhow::anyhow!(
                tr("app.data_export.error.incomplete_page")
                    .replace("%{reason}", &incomplete.reason)
            ));
        }
        if writer.is_none() {
            // 字段下标来自打开弹框时的结果列；用户改过 SQL 时只导出仍然存在的列。
            let source_indexes = form
                .selected_fields
                .iter()
                .copied()
                .filter(|index| *index < page.columns.len())
                .collect::<Vec<_>>();
            if source_indexes.is_empty() {
                let _ = fs::remove_file(&temp_path);
                return Err(anyhow::anyhow!(
                    tr("app.data_export.select_field").to_string()
                ));
            }
            let fields = unique_data_export_labels(
                source_indexes
                    .iter()
                    .map(|index| page.columns[*index].name.as_str()),
            );
            writer = Some(TableDataExportWriter::create_indexed(
                &temp_path,
                form.format,
                object.clone(),
                fields,
                source_indexes,
                db_kind,
            )?);
        }
        let rows = match writer.as_mut() {
            Some(writer) => writer.write_page(&page)?,
            None => 0,
        };
        exported += rows;
        let _ = sender.send(TableDataExportProgress {
            rows,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            message: tr("app.data_export.toast.exported")
                .replace("%{count}", &exported.to_string()),
        });
        if rows == 0 || !page.has_more {
            break cancel_flag.load(Ordering::Relaxed);
        }
        offset = page.next_offset();
    };
    let Some(writer) = writer else {
        // 首屏取出前就被取消：临时文件尚未创建。
        return Ok(TableDataExportResult { canceled: true });
    };
    writer.finish()?;
    if canceled {
        let _ = fs::remove_file(&temp_path);
        Ok(TableDataExportResult { canceled: true })
    } else {
        fs::rename(&temp_path, &path).map_err(|error| {
            anyhow::anyhow!(
                tr("app.data_export.error.write").replace("%{error}", &error.to_string())
            )
        })?;
        Ok(TableDataExportResult { canceled: false })
    }
}

/// 结果行数统计 SQL：把用户 SQL 包成子查询。Oracle 子查询不能带 `AS` 别名。
fn query_result_export_count_sql(sql: &str, kind: DatabaseKind) -> String {
    let sql = sql.trim().trim_end_matches(';');
    if kind == DatabaseKind::Oracle {
        format!("SELECT COUNT(*) FROM ({sql})")
    } else {
        format!("SELECT COUNT(*) FROM ({sql}) AS fluxdb_export_count")
    }
}

fn query_export_count_from_execution(execution: &fluxdb_core::QueryExecutionResult) -> u64 {
    execution
        .results
        .first()
        .and_then(|page| page.rows.first())
        .and_then(|row| row.values.first())
        .map(|value| match value {
            CellValue::I64(value) => (*value).max(0) as u64,
            CellValue::F64(value) => (*value).max(0.0) as u64,
            CellValue::Text(value) => value.parse().unwrap_or(0),
            _ => 0,
        })
        .unwrap_or(0)
}

fn query_result_export_preview_key(sql: &str) -> String {
    format!("query|{sql}")
}

fn query_result_export_preview_on_ui(
    view: &WeakEntity<NavicatMain>,
    cx: &mut gpui::AsyncApp,
    seq: u64,
    key: String,
    result: fluxdb_core::Result<u64>,
) {
    cx.update(|cx| {
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
                    Ok(row_count) => TableDataExportPreviewStatus::Ready {
                        sql: String::new(),
                        row_count,
                    },
                    Err(error) => TableDataExportPreviewStatus::Failed(error.to_string()),
                },
            });
            cx.notify();
        });
    });
}

fn query_result_export_suggested_name(form: &QueryResultExportForm) -> String {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    format!("query-result-{timestamp}.{}", form.format.extension())
}

fn query_result_export_modal(
    form: QueryResultExportForm,
    editor: Option<Entity<editor_component::Editor>>,
    preview: Option<TableDataExportPreviewState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let can_export = !form.sql.trim().is_empty() && !form.selected_fields.is_empty();
    let active_tab = form.active_tab;
    let panel = data_export_modal_panel(
        TableDataExportModalClose::QueryForm,
        colors,
        cx,
    )
    .child(data_export_modal_header(
        tr("app.data_export.query_title"),
        TableDataExportModalClose::QueryForm,
        colors,
        cx,
    ))
    .child(data_export_query_tabs(active_tab, colors, cx))
    .child(match active_tab {
        TableDataExportTab::Conditions => {
            query_result_export_conditions_body(&form, editor, preview, colors, cx)
        }
        TableDataExportTab::Log => data_export_empty_log_body(colors),
        _ => query_result_export_general_body(&form, colors, cx),
    })
    .child(data_export_modal_footer(
        can_export,
        None,
        DataExportFooterTarget::Query,
        colors,
        cx,
    ));
    data_export_modal_shell(panel, TableDataExportModalClose::QueryForm, colors, cx)
}

fn data_export_query_tabs(
    active: TableDataExportTab,
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
        .child(data_export_tab_button(
            tr("app.data_export.tab.general"),
            active == TableDataExportTab::General,
            TableDataExportTab::General,
            None,
            colors,
            cx,
            |this, tab, cx| this.select_query_result_export_tab(tab, cx),
        ))
        .child(data_export_tab_button(
            tr("app.data_export.tab.conditions"),
            active == TableDataExportTab::Conditions,
            TableDataExportTab::Conditions,
            None,
            colors,
            cx,
            |this, tab, cx| this.select_query_result_export_tab(tab, cx),
        ))
        .child(data_export_tab_button(
            tr("app.data_export.tab.log"),
            active == TableDataExportTab::Log,
            TableDataExportTab::Log,
            None,
            colors,
            cx,
            |this, tab, cx| this.select_query_result_export_tab(tab, cx),
        ))
}

fn query_result_export_general_body(
    form: &QueryResultExportForm,
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
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(data_export_form_row_label(tr("app.data_export.query_source"), colors))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_color(colors.muted)
                        .child(tr("app.data_export.query_result_detail").replace(
                            "%{columns}",
                            &form.columns.len().to_string(),
                        )),
                ),
        )
        .child(query_result_export_fields_picker(form, colors, cx))
        .child(data_export_format_grid(
            form.format,
            TableDataExportFormat::query_result_all(),
            colors,
            cx,
            |this, format, cx| this.set_query_result_export_format(format, cx),
        ))
        .child(
            div()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.data_export.query_scope_hint")),
        )
}

/// 查询结果字段选择：按列下标勾选，界面上给重名列显示去重后的导出表头。
fn query_result_export_fields_picker(
    form: &QueryResultExportForm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected = form.selected_fields.len();
    let total = form.columns.len();
    let labels = unique_data_export_labels(form.columns.iter().map(|column| column.name.as_str()));
    let mut list = div().flex().flex_col().gap_1().p_2();
    for (index, column) in form.columns.iter().enumerate() {
        let checked = form.selected_fields.contains(&index);
        let label = labels
            .get(index)
            .cloned()
            .unwrap_or_else(|| column.name.clone());
        let type_name = column
            .type_name
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        list = list.child(
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
                        this.toggle_query_result_export_field(index, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(Checkbox::new(("query-export-field", index)).checked(checked))
                .child(
                    div()
                        .w(px(220.))
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(label),
                )
                .child(
                    div()
                        .w(px(140.))
                        .truncate()
                        .text_color(colors.muted)
                        .child(type_name),
                ),
        );
    }
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
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(data_export_section_label(tr("app.data_export.fields"), colors))
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
                            Button::new("query-export-select-all-fields")
                                .xsmall()
                                .label(tr("app.data_export.select_all"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.select_all_query_result_export_fields(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("query-export-invert-fields")
                                .xsmall()
                                .label(tr("app.data_export.invert"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.invert_query_result_export_fields(cx);
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

fn query_result_export_conditions_body(
    form: &QueryResultExportForm,
    editor: Option<Entity<editor_component::Editor>>,
    preview: Option<TableDataExportPreviewState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let key = query_result_export_preview_key(&form.sql);
    let count = preview
        .filter(|preview| preview.key == key)
        .map(|preview| match preview.status {
            TableDataExportPreviewStatus::Loading => tr("app.data_export.counting").to_string(),
            TableDataExportPreviewStatus::Ready { row_count, .. } => {
                tr("app.data.count.rows").replace("%{count}", &table_data_export_format_count(row_count))
            }
            TableDataExportPreviewStatus::Failed(_) => tr("app.data_export.unknown").to_string(),
        });
    // 编辑器缺失只可能是弹框刚打开尚未创建；保持布局稳定。
    let editor_box = div()
        .debug_selector(|| "query-result-export-sql".to_string())
        .flex_1()
        .min_h(px(0.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .overflow_hidden()
        .p_3();
    let editor_box = match editor {
        Some(editor) => editor_box.child(editor),
        None => editor_box.child(
            div()
                .text_color(colors.muted)
                .child(tr("app.data_export.generating_preview")),
        ),
    };
    div()
        .flex_1()
        .min_h(px(0.))
        .px_5()
        .py_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(data_export_section_label(tr("app.data_export.query_sql"), colors))
        .child(editor_box)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(data_export_section_label(tr("app.data_export.estimated"), colors))
                .child(
                    div()
                        .text_color(colors.muted)
                        .child(count.unwrap_or_else(|| tr("app.data_export.unknown").to_string())),
                )
                .child(
                    Button::new("query-export-refresh-preview")
                        .xsmall()
                        .label(tr("app.common.refresh"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_query_result_export_preview(true, cx);
                            cx.stop_propagation();
                        })),
                ),
        )
        .child(
            div()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.data_export.query_sql_hint")),
        )
}

#[cfg(test)]
mod query_result_export_tests {
    use super::*;

    #[test]
    fn count_sql_wraps_dialect_alias() {
        assert_eq!(
            query_result_export_count_sql("SELECT * FROM users;", DatabaseKind::MySql),
            "SELECT COUNT(*) FROM (SELECT * FROM users) AS fluxdb_export_count"
        );
        // Oracle 子查询不能带 AS 别名。
        assert_eq!(
            query_result_export_count_sql("SELECT * FROM users", DatabaseKind::Oracle),
            "SELECT COUNT(*) FROM (SELECT * FROM users)"
        );
    }

    #[test]
    fn count_reads_first_cell() {
        let execution = fluxdb_core::QueryExecutionResult {
            summaries: Vec::new(),
            results: vec![DataPage {
                columns: vec![GdbColumn {
                    name: "count".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: false,
                    comment: None,
                }],
                rows: vec![Row {
                    values: vec![CellValue::I64(42)],
                }],
                offset: 0,
                limit: 1,
                has_more: false,
                incomplete: None,
            }],
            rollback_snapshots: Vec::new(),
        };
        assert_eq!(query_export_count_from_execution(&execution), 42);
    }
}
