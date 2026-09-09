// ============================================================
// 备份列表 tab（TabKind::BackupList）
// 侧边栏「备份」节点单击打开（按库一个 tab）：
//   列 = 名称 / 修改时间 / 备份表（查看弹框）/ 文件大小 / 备注（编辑弹框）/ 操作（编辑、删除确认弹框）。
// 数据来源：
//   - 历史备份：扫描 {backup_dir}/{库安全名}/*.sql，修改时间与大小实时读文件系统；
//   - 备份表清单与备注：读旁挂元数据 {文件}.sql.meta.json（仅新备份写入，缺失显示「无记录」）；
//   - 运行中任务：内存 backup_tasks，展示阶段并提供日志/取消入口。
// 渲染期做小目录磁盘扫描（备份数量有限，可接受）；刷新按钮/右键菜单通过 cx.notify 触发重扫。
// ============================================================

use std::time::UNIX_EPOCH;

/// 备份文件在 tab 中的一行（磁盘文件视角）。
struct BackupFileRow {
    path: PathBuf,
    file_name: String,
    modified: String,
    size: u64,
    meta: Option<BackupFileMeta>,
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

    /// 保存备注：与已有 meta 合并后写 {文件}.sql.meta.json（不影响表清单字段）。
    fn save_backup_note_modal(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.backup_note_modal_path.clone() else {
            return;
        };
        let note = self.backup_note_edit_input.read(cx).value().trim().to_string();
        let mut meta = read_backup_meta(&path).unwrap_or_default();
        meta.note = note;
        match write_backup_meta(&path, &meta) {
            Ok(()) => {
                self.backup_note_modal_path = None;
                self.show_message("备注已保存".to_string(), AppMessageKind::Success, cx);
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "备份备注写入失败");
                self.show_message(format!("备注保存失败：{error}"), AppMessageKind::Error, cx);
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

    /// 确认删除：删除备份 .sql 文件及旁挂 .meta.json（残留元数据一并清理）。
    fn confirm_backup_delete(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.pending_delete_backup.take() else {
            return;
        };
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        // 主文件删除失败则终止；元数据删除失败仅记日志（主文件已移除即视为删除成功）。
        if let Err(error) = fs::remove_file(&path) {
            tracing::warn!(path = %path.display(), %error, "备份文件删除失败");
            self.show_message(format!("删除失败：{error}"), AppMessageKind::Error, cx);
            cx.notify();
            return;
        }
        let meta_path = backup_meta_path(&path);
        if let Err(error) = fs::remove_file(&meta_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %meta_path.display(), %error, "备份元数据删除失败");
            }
        }
        tracing::info!(path = %path.display(), "备份文件已删除");
        self.show_message(
            format!("已删除备份：{file_name}"),
            AppMessageKind::Success,
            cx,
        );
        cx.notify();
    }
}

/// 备份文件旁挂元数据路径：{文件}.sql → {文件}.sql.meta.json。
fn backup_meta_path(sql_path: &Path) -> PathBuf {
    sql_path.with_extension("sql.meta.json")
}

/// 读取旁挂元数据；不存在或解析失败返回 None（视为无记录）。
fn read_backup_meta(sql_path: &Path) -> Option<BackupFileMeta> {
    let raw = fs::read_to_string(backup_meta_path(sql_path)).ok()?;
    serde_json::from_str::<BackupFileMeta>(&raw).ok()
}

/// 写入旁挂元数据（pretty JSON），返回可读错误信息供日志与提示。
fn write_backup_meta(sql_path: &Path, meta: &BackupFileMeta) -> Result<(), String> {
    let json = serde_json::to_string_pretty(meta).map_err(|error| error.to_string())?;
    fs::write(backup_meta_path(sql_path), json).map_err(|error| error.to_string())
}

/// 该库的备份目录：{backup_dir}/{库安全名}；未配置备份目录时 None。
fn backup_tab_dir(state: &AppState, database: &str) -> Option<PathBuf> {
    let dir = state.settings.backup_dir.trim();
    if dir.is_empty() {
        return None;
    }
    Some(PathBuf::from(dir).join(safe_data_export_filename_segment(database)))
}

fn file_mtime_unix(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// 扫描备份目录下的 .sql 文件，按修改时间新→旧排序，并合并旁挂元数据。
fn scan_backup_rows(dir: &Path) -> Vec<BackupFileRow> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows: Vec<BackupFileRow> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
                return None;
            }
            let file_name = path.file_name()?.to_string_lossy().to_string();
            let metadata = fs::metadata(&path).ok()?;
            let size = metadata.len();
            let modified_unix = metadata
                .modified()
                .ok()
                .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0);
            Some(BackupFileRow {
                modified: format_backup_mtime(modified_unix),
                path,
                file_name,
                size,
                meta: read_backup_meta(&entry.path()),
            })
        })
        .collect();
    // 新→旧排序（重读 mtime 作排序键，避免格式化字符串参与比较）。
    rows.sort_by(|a, b| file_mtime_unix(&b.path).cmp(&file_mtime_unix(&a.path)));
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
        Some(tables) if tables.is_empty() => "整库".to_string(),
        Some(tables) => format!("{} 张", tables.len()),
        None => "无记录".to_string(),
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
    let rows = match backup_tab_dir(state, &list.database) {
        Some(dir) => scan_backup_rows(&dir),
        None => Vec::new(),
    };
    let dir_configured = !state.settings.backup_dir.trim().is_empty();
    let database_for_new = list.database.clone();
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
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(format!("备份 · {}", list.database)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(backup_text_button(
                            "新建备份",
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
                            "刷新",
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
                .when(!dir_configured, |this| {
                    this.child(backup_list_hint("请先在设置中配置备份目录", colors))
                })
                .when(dir_configured && rows.is_empty() && running_tasks.is_empty(), |this| {
                    this.child(backup_list_hint("该库暂无备份", colors))
                })
                // 运行中任务置顶，带阶段/日志/取消。
                .children(running_tasks.into_iter().map(|task| {
                    backup_running_row(task, colors, cx)
                }))
                .children(rows.into_iter().map(|row| backup_file_row(row, colors, cx))),
        )
}

fn backup_list_hint(text: &'static str, colors: UiColors) -> Div {
    div()
        .py_6()
        .flex()
        .justify_center()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(text)
}

/// 通用小按钮（纯 div + App 级监听，cx.listener 负责回写实体状态）。
fn backup_text_button(
    label: &'static str,
    colors: UiColors,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .h(px(22.))
        .px_2()
        .flex()
        .items_center()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .text_size(px(12.))
        .text_color(colors.text)
        .cursor_pointer()
        .hover(|style| style.bg(colors.hover))
        .child(label)
        .on_mouse_down(MouseButton::Left, on_click)
}

/// 列宽：名称与备注弹性；其余固定。
const BACKUP_COL_MODIFIED: f32 = 150.;
const BACKUP_COL_TABLES: f32 = 118.;
const BACKUP_COL_SIZE: f32 = 90.;
const BACKUP_COL_ACTION: f32 = 56.;

fn backup_list_header_row(colors: UiColors) -> Div {
    div()
        .flex_none()
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .bg(colors.panel_alt)
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(div().flex_1().min_w_0().child("名称"))
        .child(div().w(px(BACKUP_COL_MODIFIED)).child("修改时间"))
        .child(div().w(px(BACKUP_COL_TABLES)).child("备份表"))
        .child(div().w(px(BACKUP_COL_SIZE)).child("文件大小"))
        .child(div().flex_1().min_w_0().child("备注"))
        .child(div().w(px(BACKUP_COL_ACTION * 2. + 6.)).child("操作"))
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
        .unwrap_or_else(|| format!("备份 #{}", task.id));
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
        .text_size(px(12.))
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
                .child(format!("{stage}（进行中）")),
        )
        .child(div().w(px(BACKUP_COL_TABLES)).child("—"))
        .child(div().w(px(BACKUP_COL_SIZE)).child("—"))
        .child(div().flex_1().min_w_0().child(""))
        .child(
            div()
                .w(px(BACKUP_COL_ACTION * 2. + 6.))
                .flex()
                .items_center()
                .gap_2()
                .child(backup_text_button(
                    "日志",
                    colors,
                    cx.listener(move |this, _, _, cx| {
                        this.backup_log_task = Some(task_id);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                ))
                .child(backup_text_button(
                    "取消",
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
    let path_for_tables = row.path.clone();
    let file_name_for_tables = row.file_name.clone();
    let meta_for_tables = row.meta.clone();
    let path_for_note = row.path.clone();
    let path_for_delete = row.path.clone();
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
        .text_size(px(12.))
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
                    "查看",
                    colors,
                    cx.listener(move |this, _, _, cx| {
                        // 点击时重读 meta，保证编辑备注后清单展示最新。
                        let meta = read_backup_meta(&path_for_tables).or_else(|| meta_for_tables.clone());
                        this.open_backup_tables_modal(file_name_for_tables.clone(), meta, cx);
                        cx.stop_propagation();
                    }),
                )),
        )
        .child(div().w(px(BACKUP_COL_SIZE)).child(format_backup_size(row.size)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(note_display),
        )
        // 操作列宽度与表头/运行中任务行保持一致（两个按钮 + 间距），否则列错位。
        .child(
            div()
                .w(px(BACKUP_COL_ACTION * 2. + 6.))
                .flex()
                .items_center()
                .gap_2()
                .child(backup_text_button(
                    "编辑",
                    colors,
                    cx.listener(move |this, _, window, cx| {
                        let note = read_backup_meta(&path_for_note)
                            .map(|meta| meta.note)
                            .unwrap_or_default();
                        this.open_backup_note_modal(path_for_note.clone(), note, window, cx);
                        cx.stop_propagation();
                    }),
                ))
                .child(backup_text_button(
                    "删除",
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
            .child("该备份没有表清单记录（备份元数据功能上线前的老备份）"),
        Some(tables) if tables.is_empty() => div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .text_color(colors.muted)
            .child("整库备份（备份了当时的全部表）"),
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
                    .text_size(px(12.))
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
                .text_size(px(13.))
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
                                .child("备份表清单")
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .font_weight(gpui::FontWeight::NORMAL)
                                        .text_color(colors.muted)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(file_label),
                                ),
                        )
                        .child(backup_text_button(
                            "关闭",
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
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child("包含视图"),
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
                .text_size(px(13.))
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
                        .child("删除备份"),
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
                                .child(format!("确定要删除备份「{file_name}」吗？")),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child("将同时删除备份文件与其元数据，删除后不可恢复。"),
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
                                .label("取消")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_backup_delete_confirm(cx);
                                })),
                        )
                        .child(
                            Button::new("backup-delete-confirm")
                                .label("删除")
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
                .text_size(px(13.))
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
                        .child("编辑备注")
                        .child(
                            div()
                                .text_size(px(11.))
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
                                        .text_size(px(13.)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child("备注随 {备份文件}.sql.meta.json 保存；留空表示清除备注。"),
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
                                .label("取消")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_backup_note_modal(cx);
                                })),
                        )
                        .child(
                            Button::new("backup-note-save")
                                .label("保存")
                                .primary()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_backup_note_modal(cx);
                                })),
                        ),
                ),
        )
}
