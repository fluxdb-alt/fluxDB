fn task_center_status_color(status: TaskCenterStatus, colors: UiColors) -> gpui::Rgba {
    match status {
        TaskCenterStatus::Running => colors.info,
        TaskCenterStatus::Success => colors.success,
        TaskCenterStatus::Failed => colors.danger,
        TaskCenterStatus::Canceled => colors.muted,
    }
}

fn task_center_elapsed(started_at: Instant, finished_at: Option<Instant>) -> String {
    let elapsed = finished_at.unwrap_or_else(Instant::now).saturating_duration_since(started_at);
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

/// 状态栏只保留一个任务中心入口，运行中显示计数，避免多个任务芯片挤占空间。
fn task_center_statusbar_area(
    task_center: &TaskCenterState,
    backup_tasks: &[BackupTaskState],
    data_export_tasks: &[TableDataExportTaskState],
    sql_file_tasks: &[SqlFileExecutionTaskState],
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let derived_running = backup_tasks
        .iter()
        .filter(|task| task.running())
        .count()
        + data_export_tasks
            .iter()
            .filter(|task| task.running())
            .count()
        + sql_file_tasks.iter().filter(|task| task.running()).count();
    let record_running = task_center
        .records
        .iter()
        .filter(|record| record.status.is_running())
        .count();
    let running = derived_running + record_running;
    let total = backup_tasks.len() + data_export_tasks.len() + sql_file_tasks.len() + task_center.records.len();
    let label = if running > 0 {
        tr("app.task_center.statusbar.running").replace("%{count}", &running.to_string())
    } else {
        tr("app.task_center.statusbar.idle").to_string()
    };
    div()
        .w(px(230.))
        .h_full()
        .flex()
        .items_center()
        .justify_end()
        .pr_2()
        .child(
            div()
                .debug_selector(|| "task-center-statusbar-entry".into())
                .max_w(px(218.))
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
                        this.open_task_center(cx);
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(
                    if running > 0 {
                        AppIcon::Activity
                    } else {
                        AppIcon::List
                    },
                    13.,
                    if running > 0 { colors.info } else { colors.muted },
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
                )
                .when(total > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(total.to_string()),
                    )
                }),
        )
}

/// 任务中心面板：遮罩 + 面板；右侧抽屉只跟随选中行，不改变列表滚动位置。
fn task_center_overlay(
    task_center: &TaskCenterState,
    rows: &[TaskCenterRow],
    selected: Option<&TaskCenterRow>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let running = rows.iter().filter(|row| row.status.is_running()).count();
    let failed = rows
        .iter()
        .filter(|row| row.status == TaskCenterStatus::Failed)
        .count();
    let visible: Vec<&TaskCenterRow> = rows
        .iter()
        .filter(|row| match task_center.filter {
            TaskCenterFilter::All => true,
            TaskCenterFilter::Running => row.status.is_running(),
            TaskCenterFilter::Failed => row.status == TaskCenterStatus::Failed,
        })
        .collect();

    let mut list = div()
        .id("task-center-list")
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();
    if visible.is_empty() {
        list = list.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(tr("app.task_center.empty")),
        );
    } else {
        for row in visible {
            list = list.child(task_center_row(
                row,
                task_center.selected == Some(row.key),
                colors,
                cx,
            ));
        }
    }

    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.55)
        } else {
            opaque_grey(0.75, 0.28)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_task_center(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .debug_selector(|| "task-center-panel".into())
                .w(px(1040.))
                .max_w(px(1040.))
                .h(px(620.))
                .rounded(px(12.))
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
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
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(54.))
                        .flex_none()
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(15.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(tr("app.task_center.heading")),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(colors.muted)
                                        .child(
                                            tr("app.task_center.subtitle")
                                                .replace("%{total}", &rows.len().to_string())
                                                .replace("%{running}", &running.to_string()),
                                        ),
                                ),
                        )
                        .child(
                            Button::new("task-center-close")
                                .icon(IconName::Close)
                                .ghost()
                                .compact()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_task_center(cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .h(px(42.))
                        .flex_none()
                        .px_5()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(task_center_filter_button(
                            tr("app.task_center.filter.all"),
                            rows.len(),
                            TaskCenterFilter::All,
                            task_center.filter,
                            colors,
                            cx,
                        ))
                        .child(task_center_filter_button(
                            tr("app.task_center.filter.running"),
                            running,
                            TaskCenterFilter::Running,
                            task_center.filter,
                            colors,
                            cx,
                        ))
                        .child(task_center_filter_button(
                            tr("app.task_center.filter.failed"),
                            failed,
                            TaskCenterFilter::Failed,
                            task_center.filter,
                            colors,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .flex()
                        .child(list)
                        .when_some(selected, |this, row| {
                            this.child(task_center_detail(row, colors, cx))
                        }),
                ),
        )
}

fn task_center_filter_button(
    label: SharedString,
    count: usize,
    filter: TaskCenterFilter,
    current: TaskCenterFilter,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let active = filter == current;
    div()
        .h(px(24.))
        .px_2()
        .rounded(colors.radius)
        .bg(if active { colors.hover } else { colors.panel_bg })
        .text_color(if active { colors.text } else { colors.muted })
        .text_size(px(12.))
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.task_center.filter = filter;
                cx.notify();
                cx.stop_propagation();
            }),
        )
        .child(label)
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(count.to_string()),
        )
}

fn task_center_row(
    row: &TaskCenterRow,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let key = row.key;
    let status_color = task_center_status_color(row.status, colors);
    div()
        .debug_selector(move || {
            format!(
                "task-center-row:{}:{}:{}",
                key.kind as u8, key.id, key.source as u8
            )
        })
        .h(px(62.))
        .flex_none()
        .px_4()
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .bg(if selected { colors.hover } else { colors.panel_bg })
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.task_center.selected = Some(key);
                cx.notify();
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(50.))
                .flex_none()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(app_icon(row.kind.icon(), 15., status_color))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(colors.muted)
                        .child(row.kind.label()),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(12.5))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(row.title.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(status_color)
                                .child(row.status.label(row.cancel_requested)),
                        ),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(if row.stage.is_empty() {
                            row.target.clone()
                        } else {
                            format!("{} · {}", row.stage, row.target)
                        }),
                ),
        )
        .child(task_center_row_progress(row, colors))
        .child(
            div()
                .w(px(58.))
                .flex_none()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(task_center_elapsed(row.started_at, row.finished_at)),
        )
}

fn task_center_row_progress(row: &TaskCenterRow, colors: UiColors) -> Div {
    let percent = row.progress.unwrap_or(0.);
    div().w(px(150.)).flex_none().child(
        Progress::new(SharedString::from(format!(
            "task-center-progress-{}-{}-{}",
            row.key.kind as u8, row.key.id, row.key.source as u8
        )))
        .value(percent * 100.)
        .loading(row.status.is_running() && row.progress.is_none())
        .color(task_center_status_color(row.status, colors))
        .xsmall()
        .w_full(),
    )
}

fn task_center_detail(
    row: &TaskCenterRow,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let key = row.key;
    let status_color = task_center_status_color(row.status, colors);
    let mut log_list = div()
        .id("task-center-detail-logs")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_1();
    if row.logs.is_empty() {
        log_list = log_list.child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(tr("app.task_center.detail.no_logs")),
        );
    } else {
        for line in &row.logs {
            log_list = log_list.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .whitespace_normal()
                    .child(line.clone()),
            );
        }
    }

    div()
        .debug_selector(|| "task-center-detail".into())
        .w(px(360.))
        .flex_none()
        .h_full()
        .border_l_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .flex_col()
        .child(
            div()
                .px_4()
                .py_3()
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(row.title.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(status_color)
                                .child(row.status.label(row.cancel_requested)),
                        ),
                )
                .child(task_center_detail_line(
                    tr("app.task_center.detail.target"),
                    row.target.clone(),
                    colors,
                ))
                .child(task_center_detail_line(
                    tr("app.task_center.detail.stage"),
                    row.stage.clone(),
                    colors,
                ))
                .child(task_center_detail_line(
                    tr("app.task_center.detail.elapsed"),
                    task_center_elapsed(row.started_at, row.finished_at),
                    colors,
                ))
                .when_some(row.error.clone(), |this, error| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.danger)
                            .whitespace_normal()
                            .child(error),
                    )
                }),
        )
        .child(
            div()
                .px_4()
                .pt_3()
                .flex_none()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(tr("app.task_center.detail.logs")),
        )
        .child(div().px_4().py_2().flex_1().min_h(px(0.)).flex().child(log_list))
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
                .when(row.status.is_running() && row.cancel_requested, |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.warning)
                            .child(tr("app.task_center.status.stopping")),
                    )
                })
                .when(row.status.is_running() && !row.cancel_requested, |this| {
                    this.child(
                        Button::new("task-center-cancel-task")
                            .label(tr("app.task_center.action.cancel"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.cancel_task_center_task(key, cx);
                            })),
                    )
                })
                .when(!row.status.is_running(), |this| {
                    this.child(
                        div()
                            .flex_none()
                            .debug_selector(|| "task-center-delete-task".into())
                            .child(
                                Button::new("task-center-delete-task")
                                .label(tr("app.task_center.action.delete"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.delete_task_center_task(key, window, cx);
                                })),
                        ),
                    )
                })
                .child(
                    Button::new("task-center-detail-close")
                        .label(tr("app.common.close"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.task_center.selected = None;
                            cx.notify();
                        })),
                ),
        )
}

fn task_center_detail_line(label: SharedString, value: String, colors: UiColors) -> Div {
    div()
        .flex()
        .gap_2()
        .text_size(px(11.))
        .child(div().w(px(52.)).flex_none().text_color(colors.muted).child(label))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_color(colors.text)
                .child(value),
        )
}
