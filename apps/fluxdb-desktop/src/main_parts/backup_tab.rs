// ============================================================
// 备份列表 tab（TabKind::BackupList）
// 侧边栏「备份」节点单击打开（按库一个 tab）：
//   列 = 名称 / 备份时间 / 备份表（查看弹框）/ 文件大小 / 路径 / 备注（编辑弹框）/ 操作（编辑、删除确认弹框）。
// 数据来源：
//   - 备份记录：存 sqlite（真实数据在磁盘），按 connection_id + database 筛选，备份时间/大小存记录；
//   - 运行中任务：内存 backup_tasks，展示阶段并提供日志/取消入口。
// 刷新按钮/右键菜单通过 cx.notify 触发重读。
// ============================================================


/// 备份记录在 tab 中的一行（存 sqlite，真实文件在磁盘）。
struct BackupFileRow {
    path: PathBuf,
    file_name: String,
    modified: String,
    size: u64,
    meta: Option<BackupFileMeta>,
}

impl BackupFileRow {
    /// 备份时间（unix 秒），用于新→旧排序。
    fn created_unix(&self) -> i64 {
        self.meta.as_ref().map(|m| m.created_unix).unwrap_or(0)
    }
}

impl NavicatMain {
    /// 打开「备份表」查看弹框；meta 为 None 表示该备份无表清单记录（老备份）。
    fn open_backup_tables_modal(
        &mut self,
        file_name: String,
        meta: Option<BackupFileMeta>,
        cx: &mut Context<Self>,
    ) {
        let (tables, include_views) = match &meta {
            Some(meta) => (meta.tables.clone(), meta.include_views),
            None => (None, false),
        };
        self.backup_tables_modal = Some(BackupTablesModal { file_name, tables, include_views });
        cx.notify();
    }

    fn close_backup_tables_modal(&mut self, cx: &mut Context<Self>) {
        self.backup_tables_modal = None;
        cx.notify();
    }

    /// 打开「恢复记录」弹框：按备份 id 过滤关联的恢复记录，新→旧排序。
    /// 老备份记录没有 id（空串），一律按无记录展示。
    fn open_restore_records_modal(
        &mut self,
        file_name: String,
        backup_id: String,
        cx: &mut Context<Self>,
    ) {
        let mut records = if backup_id.is_empty() {
            Vec::new()
        } else {
            match self.storage.load_restore_records() {
                Ok(records) => records
                    .into_iter()
                    .filter(|record| record.backup_id == backup_id)
                    .collect(),
                Err(error) => {
                    tracing::warn!(%error, "加载恢复记录失败");
                    Vec::new()
                }
            }
        };
        records.sort_by(|a, b| b.finished_unix.cmp(&a.finished_unix));
        self.restore_records_modal = Some(RestoreRecordsModal { file_name, records });
        cx.notify();
    }

    fn close_restore_records_modal(&mut self, cx: &mut Context<Self>) {
        self.restore_records_modal = None;
        cx.notify();
    }

    /// 打开备注编辑弹框，回填当前备注。
    fn open_backup_note_modal(
        &mut self,
        path: PathBuf,
        note: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.backup_note_modal_path = Some(path);
        self.backup_note_edit_input
            .update(cx, |input, cx| input.set_value(note, window, cx));
        cx.notify();
    }

    fn close_backup_note_modal(&mut self, cx: &mut Context<Self>) {
        self.backup_note_modal_path = None;
        cx.notify();
    }

    /// 保存备注：按输出路径更新 sqlite 中该备份记录的注记（不影响表清单等其它字段）。
    fn save_backup_note_modal(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.backup_note_modal_path.clone() else {
            return;
        };
        let note = self.backup_note_edit_input.read(cx).value().trim().to_string();
        let output_path = path.to_string_lossy().to_string();
        let result = self
            .storage
            .load_backup_records()
            .map_err(|error| error.to_string())
            .and_then(|mut records| {
                let Some(record) = records
                    .iter_mut()
                    .find(|record| record.output_path == output_path)
                else {
                    return Err(tr("app.backup.record_not_found").to_string());
                };
                record.note = note;
                self.storage
                    .save_backup_records(&records)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                self.backup_note_modal_path = None;
                self.show_message(
                    tr("app.backup.note_saved"),
                    AppMessageKind::Success,
                    cx,
                );
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "备份备注写入失败");
                self.show_message(
                    format!("{}: {error}", tr("app.backup.note_save_failed")),
                    AppMessageKind::Error,
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// 打开删除备份确认弹框。
    fn open_backup_delete_confirm(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.pending_delete_backup = Some(path);
        cx.notify();
    }

    fn close_backup_delete_confirm(&mut self, cx: &mut Context<Self>) {
        self.pending_delete_backup = None;
        cx.notify();
    }

    /// 确认删除：先删磁盘真实 .sql 文件（文件已不存在则忽略），再删 sqlite 备份记录。
    /// 磁盘文件缺失时仅删记录，符合「已删数据则只删记录」的语义。
    fn confirm_backup_delete(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.pending_delete_backup.take() else {
            return;
        };
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        // 主文件删除失败（除 NotFound）才终止；否则继续删记录。
        if let Err(error) = fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %path.display(), %error, "备份文件删除失败");
                self.show_message(
                    format!("{}: {error}", tr("app.backup.delete_failed")),
                    AppMessageKind::Error,
                    cx,
                );
                cx.notify();
                return;
            }
        }
        // 按完整路径从 sqlite 记录中移除该备份，真实数据是否已删都不影响记录清理。
        let output_path = path.to_string_lossy().to_string();
        match self.remove_backup_record(&output_path) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "已删除备份及其记录");
                self.show_message(
                    tr("app.backup.deleted").replace("%{name}", &file_name),
                    AppMessageKind::Success,
                    cx,
                );
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "备份记录删除失败");
                self.show_message(
                    format!("{}: {error}", tr("app.backup.record_delete_failed")),
                    AppMessageKind::Error,
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// 追加或按 `output_path` 更新一条备份记录并写回 sqlite。
    fn persist_backup_record(&mut self, record: BackupFileMeta) -> Result<(), String> {
        let mut records = self
            .storage
            .load_backup_records()
            .map_err(|error| error.to_string())?;
        if let Some(existing) = records
            .iter_mut()
            .find(|r| r.output_path == record.output_path)
        {
            *existing = record;
        } else {
            records.push(record);
        }
        self.storage
            .save_backup_records(&records)
            .map_err(|error| error.to_string())
    }

    /// 按完整路径从 sqlite 记录中移除一条备份并写回。
    fn remove_backup_record(&mut self, output_path: &str) -> Result<(), String> {
        let mut records = self
            .storage
            .load_backup_records()
            .map_err(|error| error.to_string())?;
        records.retain(|record| record.output_path != output_path);
        self.storage
            .save_backup_records(&records)
            .map_err(|error| error.to_string())
    }
}

/// 加载某连接 + 库的备份记录，按备份时间新→旧排序。备份历史纯存 sqlite，不再扫磁盘。
fn load_backup_records_for(storage: &FileStorage, connection_id: ConnectionId, database: &str) -> Vec<BackupFileRow> {
    let mut rows = match storage.load_backup_records() {
        Ok(records) => records
            .into_iter()
            .filter_map(|record| {
                if record.connection_id != connection_id || record.database != database {
                    return None;
                }
                let path = PathBuf::from(&record.output_path);
                let file_name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default();
                Some(BackupFileRow {
                    modified: format_backup_mtime(record.created_unix),
                    path: path.clone(),
                    file_name,
                    size: record.size,
                    meta: Some(record),
                })
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "加载备份记录失败");
            Vec::new()
        }
    };
    // 新→旧排序。
    rows.sort_by(|a, b| b.created_unix().cmp(&a.created_unix()));
    rows
}

/// unix 秒 → 本地时间字符串；0 视为不可得。
fn format_backup_mtime(secs: i64) -> String {
    if secs <= 0 {
        return "—".to_string();
    }
    Local
        .timestamp_opt(secs, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "—".to_string())
}

/// 文件大小人类可读展示。
fn format_backup_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    }
}

/// 「备份表」列的数量文案：Some(空)=整库；Some(n)=n 张；None=无记录。
fn backup_tables_cell_text(meta: &Option<BackupFileMeta>) -> String {
    match meta.as_ref().and_then(|meta| meta.tables.as_ref()) {
        Some(tables) if tables.is_empty() => tr("app.backup.whole_database").to_string(),
        Some(tables) => tr("app.backup.table_count").replace("%{count}", &tables.len().to_string()),
        None => tr("app.backup.no_record").to_string(),
    }
}

/// 备份列表 tab 主体内容。
fn backup_list_content(
    state: &AppState,
    this: &mut NavicatMain,
    list: &BackupListState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let running_tasks: Vec<BackupTaskState> = this
        .backup_tasks
        .iter()
        .filter(|task| {
            task.running()
                && task.connection_id == list.connection_id
                && task.database.as_deref() == Some(list.database.as_str())
        })
        .cloned()
        .collect();
    let rows = load_backup_records_for(&this.storage, list.connection_id, &list.database);
    let dir_configured = !state.settings.backup_dir.trim().is_empty();
    let database_for_new = list.database.clone();
    let restore_connection = list.connection_id;
    let connection_for_new = list.connection_id;

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(colors.content_bg)
        .text_color(colors.text)
        // 顶部工具栏：标题 + 新建备份 + 刷新（磁盘扫描发生在渲染期，notify 即重扫）。
        .child(
            div()
                .flex_none()
                .h(px(40.))
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(colors.border_soft)
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(
                            tr("app.backup.title_for_db")
                                .replace("%{database}", &list.database),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(backup_text_button(
                            tr("app.backup.new"),
                            colors,
                            cx.listener(move |this, _, window, cx| {
                                this.show_backup_modal(
                                    Some(connection_for_new),
                                    Some(database_for_new.clone()),
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ))
                        .child(backup_text_button(
                            tr("app.backup.restore_from_file"),
                            colors,
                            cx.listener(move |this, _, window, cx| {
                                this.open_restore_dialog(restore_connection, None, None, window, cx);
                            }),
                        ))
                        .child(backup_text_button(
                            tr("app.common.refresh"),
                            colors,
                            cx.listener(|_, _, _, cx| {
                                cx.notify();
                            }),
                        )),
                ),
        )
        // 表头。
        .child(backup_list_header_row(colors))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .flex()
                .flex_col()
                .when(!dir_configured && rows.is_empty(), |this| {
                    this.child(backup_list_hint(tr("app.backup.configure_dir"), colors))
                })
                .when(dir_configured && rows.is_empty() && running_tasks.is_empty(), |this| {
                    this.child(backup_list_hint(tr("app.backup.empty"), colors))
                })
                // 运行中任务置顶，带阶段/日志/取消。
                .children(running_tasks.into_iter().map(|task| {
                    backup_running_row(task, colors, cx)
                }))
                .children(rows.into_iter().map(|row| backup_file_row(row, colors, cx))),
        )
}

fn backup_list_hint(text: impl Into<SharedString>, colors: UiColors) -> Div {
    let text = text.into();
    div()
        .py_6()
        .flex()
        .justify_center()
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(text)
}

/// 通用小按钮（纯 div + App 级监听，cx.listener 负责回写实体状态）。
fn backup_text_button(
    label: impl Into<SharedString>,
    colors: UiColors,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let label = label.into();
    div()
        .h(px(22.))
        .px_2()
        .flex()
        .items_center()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .text_size(ui_text_size(12.))
        .text_color(colors.text)
        .cursor_pointer()
        .hover(|style| style.bg(colors.hover))
        .child(label)
        .on_mouse_down(MouseButton::Left, on_click)
}

/// 列宽：名称与备注弹性；其余固定。
const BACKUP_COL_MODIFIED: f32 = 140.;
const BACKUP_COL_TABLES: f32 = 118.;
const BACKUP_COL_SIZE: f32 = 84.;
const BACKUP_COL_PATH: f32 = 220.;
const BACKUP_COL_ACTION: f32 = 56.;
/// 操作列总宽：四个按钮位 + 间距，表头/运行中行/文件行共用保证对齐。
const BACKUP_COL_ACTION_TOTAL: f32 = BACKUP_COL_ACTION * 4. + 40.;

fn backup_list_header_row(colors: UiColors) -> Div {
    div()
        .flex_none()
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .bg(colors.panel_alt)
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(div().flex_1().min_w_0().child(tr("app.backup.header.name")))
        .child(div().w(px(BACKUP_COL_MODIFIED)).child(tr("app.backup.header.modified")))
        .child(div().w(px(BACKUP_COL_TABLES)).child(tr("app.backup.header.tables")))
        .child(div().w(px(BACKUP_COL_SIZE)).child(tr("app.backup.header.size")))
        .child(div().w(px(BACKUP_COL_PATH)).child(tr("app.backup.header.path")))
        .child(div().flex_1().min_w_0().child(tr("app.backup.header.note")))
        .child(div().w(px(BACKUP_COL_ACTION_TOTAL)).child(tr("app.backup.header.actions")))
}

/// 运行中任务行：名称 / 阶段 / — / — / — / 日志+取消。
fn backup_running_row(
    task: BackupTaskState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let display_name = task
        .output_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| tr("app.backup.task_fallback").replace("%{id}", &task.id.to_string()));
    let task_id = task.id;
    let stage = task.stage.clone();
    div()
        .flex_none()
        .min_h(px(32.))
        .px_3()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(colors.border_soft)
        .text_size(ui_text_size(12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(display_name),
        )
        .child(
            div()
                .w(px(BACKUP_COL_MODIFIED))
                .text_color(colors.muted)
                .child(tr("app.backup.stage_running").replace("%{stage}", &stage)),
        )
        .child(div().w(px(BACKUP_COL_TABLES)).child("—"))
        .child(div().w(px(BACKUP_COL_SIZE)).child("—"))
        .child(div().w(px(BACKUP_COL_PATH)).child("—"))
        .child(div().flex_1().min_w_0().child(""))
        .child(
            div()
                .w(px(BACKUP_COL_ACTION_TOTAL))
                .flex()
                .items_center()
                .gap_2()
                .child(backup_text_button(
                    tr("app.backup.log"),
                    colors,
                    cx.listener(move |this, _, _, cx| {
                        this.backup_log_task = Some(task_id);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                ))
                .child(backup_text_button(
                    tr("app.backup.cancel"),
                    colors,
                    cx.listener(move |this, _, _, cx| {
                        this.cancel_backup_task(task_id, cx);
                        cx.stop_propagation();
                    }),
                )),
        )
}

/// 磁盘备份文件行。
fn backup_file_row(row: BackupFileRow, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    let path_display = row.path.to_string_lossy().to_string();
    let file_name_for_tables = row.file_name.clone();
    let meta_for_tables = row.meta.clone();
    let path_for_note = row.path.clone();
    let path_for_delete = row.path.clone();
    let restore_path = row.path.clone();
    let restore_meta = row.meta.clone();
    let file_name_for_records = row.file_name.clone();
    let backup_id_for_records = row
        .meta
        .as_ref()
        .map(|meta| meta.id.clone())
        .unwrap_or_default();
    let note_for_edit = row
        .meta
        .as_ref()
        .map(|meta| meta.note.clone())
        .unwrap_or_default();
    let note = row
        .meta
        .as_ref()
        .map(|meta| meta.note.clone())
        .unwrap_or_default();
    let note_display = if note.is_empty() { "—".to_string() } else { note.clone() };
    div()
        .flex_none()
        .min_h(px(32.))
        .px_3()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(colors.border_soft)
        .text_size(ui_text_size(12.))
        .hover(|style| style.bg(colors.hover))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(row.file_name),
        )
        .child(
            div()
                .w(px(BACKUP_COL_MODIFIED))
                .text_color(colors.muted)
                .child(row.modified),
        )
        .child(
            div()
                .w(px(BACKUP_COL_TABLES))
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_color(colors.muted).child(backup_tables_cell_text(&row.meta)))
                .child(backup_text_button(
                    tr("app.backup.view"),
                    colors,
                    cx.listener(move |this, _, _, cx| {
                        this.open_backup_tables_modal(file_name_for_tables.clone(), meta_for_tables.clone(), cx);
                        cx.stop_propagation();
                    }),
                )),
        )
        .child(div().w(px(BACKUP_COL_SIZE)).child(format_backup_size(row.size)))
        .child(
            div()
                .w(px(BACKUP_COL_PATH))
                .overflow_hidden()
                .text_ellipsis()
                .text_color(colors.muted)
                .child(path_display.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(note_display),
        )
        // 操作列宽度与表头/运行中任务行保持一致（BACKUP_COL_ACTION_TOTAL），否则列错位。
        .child(
            div()
                .w(px(BACKUP_COL_ACTION_TOTAL))
                .flex()
                .items_center()
                .gap_2()
                .child(backup_text_button(
                    tr("app.backup.restore"),
                    colors,
                    cx.listener(move |this, _, window, cx| {
                        if let Some(meta) = restore_meta.clone() {
                            this.open_restore_dialog(meta.connection_id, Some(restore_path.clone()), Some(meta), window, cx);
                        }
                    }),
                ))
                .child(backup_text_button(
                    tr("app.backup.records"),
                    colors,
                    cx.listener(move |this, _, _, cx| {
                        this.open_restore_records_modal(
                            file_name_for_records.clone(),
                            backup_id_for_records.clone(),
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                ))
                .child(backup_text_button(
                    tr("app.backup.edit"),
                    colors,
                    cx.listener(move |this, _, window, cx| {
                        this.open_backup_note_modal(path_for_note.clone(), note_for_edit.clone(), window, cx);
                        cx.stop_propagation();
                    }),
                ))
                .child(backup_text_button(
                    tr("app.common.delete"),
                    colors,
                    cx.listener(move |this, _, _, cx| {
                        this.open_backup_delete_confirm(path_for_delete.clone(), cx);
                        cx.stop_propagation();
                    }),
                )),
        )
}

/// 「备份表」查看弹框：表清单 / 整库 / 无记录 三种状态。
fn backup_tables_modal(
    modal: BackupTablesModal,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body = match &modal.tables {
        None => div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.muted)
            .child(tr("app.backup.no_table_manifest")),
        Some(tables) if tables.is_empty() => div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.muted)
            .child(tr("app.backup.whole_database_note")),
        Some(tables) => div()
            .flex_1()
            .min_h_0()
            .py_2()
            .flex()
            .flex_col()
            .children(tables.iter().enumerate().map(|(index, table)| {
                div()
                    .h(px(26.))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(ui_text_size(12.))
                    .child(
                        div()
                            .w(px(32.))
                            .text_color(colors.muted)
                            .child(format!("{}.", index + 1)),
                    )
                    .child(div().overflow_hidden().text_ellipsis().child(table.clone()))
            })),
    };
    let include_views_hint = modal.include_views
        && matches!(&modal.tables, Some(tables) if !tables.is_empty());
    let file_label = modal.file_name.clone();
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
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_backup_tables_modal(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(420.))
                .h(px(360.))
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
                .key_context("BackupTablesModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_backup_tables_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(48.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(tr("app.backup.table_manifest"))
                                .child(
                                    div()
                                        .text_size(ui_text_size(11.))
                                        .font_weight(gpui::FontWeight::NORMAL)
                                        .text_color(colors.muted)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(file_label),
                                ),
                        )
                        .child(backup_text_button(
                            tr("app.common.close"),
                            colors,
                            cx.listener(|this, _, _, cx| {
                                this.close_backup_tables_modal(cx);
                                cx.stop_propagation();
                            }),
                        )),
                )
                .when(include_views_hint, |this| {
                    this.child(
                        div()
                            .flex_none()
                            .px_4()
                            .pt_2()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(tr("app.backup.includes_views")),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .px_4()
                        .flex()
                        .flex_col()
                        .overflow_y_scrollbar()
                        .child(body),
                ),
        )
}

/// 「恢复记录」查看弹框：该备份关联的恢复记录（新→旧），无记录时居中提示。
fn restore_records_modal(
    modal: RestoreRecordsModal,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body = if modal.records.is_empty() {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.muted)
            .child(tr("app.backup.no_restore_records"))
    } else {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .children(modal
                .records
                .iter()
                .map(|record| restore_record_row(record, colors)))
    };
    let file_label = modal.file_name.clone();
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
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_restore_records_modal(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(560.))
                .h(px(420.))
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
                .key_context("RestoreRecordsModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_restore_records_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(48.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(tr("app.backup.restore_records"))
                                .child(
                                    div()
                                        .text_size(ui_text_size(11.))
                                        .font_weight(gpui::FontWeight::NORMAL)
                                        .text_color(colors.muted)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(file_label),
                                ),
                        )
                        .child(backup_text_button(
                            tr("app.common.close"),
                            colors,
                            cx.listener(|this, _, _, cx| {
                                this.close_restore_records_modal(cx);
                                cx.stop_propagation();
                            }),
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .px_4()
                        .flex()
                        .flex_col()
                        .overflow_y_scrollbar()
                        .child(body),
                ),
        )
}

/// 单条恢复记录：时间 / 状态 / 目标（第一行）+ 恢复结果（第二行，可换行）。
fn restore_record_row(record: &fluxdb_storage::RestoreRecord, colors: UiColors) -> Div {
    let time = if record.finished_unix > 0 {
        Local
            .timestamp_opt(record.finished_unix as i64, 0)
            .single()
            .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "—".to_string())
    } else {
        "—".to_string()
    };
    let (status, status_color) = if record.canceled {
        (tr("app.backup.status.canceled"), colors.muted)
    } else if record.success {
        (tr("app.backup.status.success"), colors.text)
    } else {
        (tr("app.backup.status.failed"), rgb(0xe5484d))
    };
    let object_count = record
        .outcome
        .as_ref()
        .filter(|outcome| outcome.total_objects > 0)
        .map(|outcome| tr("app.backup.restore_summary.object_count").replace("%{count}", &outcome.total_objects.to_string()))
        .unwrap_or_default();
    div()
        .flex_none()
        .py_2()
        .border_b_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(ui_text_size(12.))
                .child(div().text_color(colors.muted).child(time))
                .child(div().font_weight(gpui::FontWeight::BOLD).text_color(status_color).child(status))
                .child(div().overflow_hidden().text_ellipsis().child(tr("app.backup.restore_summary.to_target").replace("%{target}", &record.target).replace("%{objects}", &object_count))),
        )
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(record.result.clone()),
        )
}

/// 「删除备份」确认弹框：确认后删除 .sql 与 .meta.json，不可恢复。
fn backup_delete_confirm_modal(
    path: PathBuf,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
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
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_backup_delete_confirm(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(420.))
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
                .key_context("BackupDeleteModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_backup_delete_confirm(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(48.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(tr("app.backup.delete_title")),
                )
                .child(
                    div()
                        .px_4()
                        .pb_4()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(
                                    tr("app.backup.delete_confirm")
                                        .replace("%{name}", &file_name),
                                ),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(tr("app.backup.delete_detail")),
                        ),
                )
                .child(
                    div()
                        .h(px(52.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .child(
                            Button::new("backup-delete-cancel")
                                .label(tr("app.common.cancel"))
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_backup_delete_confirm(cx);
                                })),
                        )
                        .child(
                            Button::new("backup-delete-confirm")
                                .label(tr("app.common.delete"))
                                .danger()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_backup_delete(cx);
                                })),
                        ),
                ),
        )
}

/// 「备注」编辑弹框。input/file_label 由挂载点从 self 读出传入。
fn backup_note_modal(
    input: Entity<InputState>,
    file_label: String,
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
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.close_backup_note_modal(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(420.))
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
                .key_context("BackupNoteModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_backup_note_modal(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(48.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .gap_2()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(tr("app.backup.edit_note"))
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .font_weight(gpui::FontWeight::NORMAL)
                                .text_color(colors.muted)
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(file_label),
                        ),
                )
                .child(
                    div()
                        .px_4()
                        .pb_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .h(px(34.))
                                .rounded(colors.radius)
                                .border_1()
                                .border_color(colors.border)
                                .bg(colors.input_bg)
                                .overflow_hidden()
                                .child(
                                    Input::new(&input)
                                        .appearance(false)
                                        .focus_bordered(false)
                                        .w_full()
                                        .h_full()
                                        .text_size(ui_text_size(13.)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(tr("app.backup.note_help")),
                        ),
                )
                .child(
                    div()
                        .h(px(52.))
                        .flex_none()
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .child(
                            Button::new("backup-note-cancel")
                                .label(tr("app.common.cancel"))
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_backup_note_modal(cx);
                                })),
                        )
                        .child(
                            Button::new("backup-note-save")
                                .label(tr("app.common.save"))
                                .primary()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_backup_note_modal(cx);
                                })),
                        ),
                ),
        )
}
