// 数据同步执行记录回看：左侧最近任务列表，右侧选中任务的块明细与行级样本。
//
// 只读本地账本（SQLite），不触碰源/目标库；列表与详情都在后台线程读取，
// 弹框内只渲染快照。

use fluxdb_storage::SyncRunRecord;
use fluxdb_storage::sync_run_store::KEEP_RUNS;

/// 执行记录弹框状态；`detail` 为空且 `loading` 时显示加载态。
#[derive(Clone)]
struct SyncRunViewState {
    /// 最近若干次任务，新的在前。
    runs: Vec<SyncRunRecord>,
    /// 当前选中的任务 ID。
    selected: Option<i64>,
    loading: bool,
    error: Option<String>,
    detail: Option<SyncRunDetail>,
}

impl NavicatMain {
    /// 打开执行记录弹框。`run_id` 为空时选最新一条；没有记录时显示空态。
    fn open_sync_run_view(&mut self, run_id: Option<i64>, cx: &mut Context<Self>) {
        let runs = self.controller.load_sync_runs(KEEP_RUNS as usize);
        let selected = run_id.or_else(|| runs.first().map(|run| run.id));
        let loading = selected.is_some();
        self.sync_run_view = Some(SyncRunViewState {
            runs,
            selected,
            loading,
            error: None,
            detail: None,
        });
        if let Some(selected) = selected {
            self.spawn_sync_run_detail(selected, cx);
        }
        cx.notify();
    }

    /// 切换左侧列表选中的任务，并重新加载详情。
    fn select_sync_run(&mut self, run_id: i64, cx: &mut Context<Self>) {
        let Some(state) = self.sync_run_view.as_mut() else {
            return;
        };
        if state.selected == Some(run_id) {
            return;
        }
        state.selected = Some(run_id);
        state.loading = true;
        state.error = None;
        state.detail = None;
        self.spawn_sync_run_detail(run_id, cx);
        cx.notify();
    }

    /// 后台读取选中任务；结果按 `run_id` 归属校验，过期结果直接丢弃。
    fn spawn_sync_run_detail(&mut self, run_id: i64, cx: &mut Context<Self>) {
        let controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move { controller.load_sync_run_detail(run_id) })
                .await;
            let _ = view.update(cx, |this, cx| {
                let Some(state) = this.sync_run_view.as_mut() else {
                    return;
                };
                if state.selected != Some(run_id) {
                    return;
                }
                state.loading = false;
                match result {
                    Ok(detail) => state.detail = Some(detail),
                    Err(error) => state.error = Some(error.message),
                }
                cx.notify();
            });
        });
        self._sync_run_view_task = Some(task);
    }

    fn close_sync_run_view(&mut self, cx: &mut Context<Self>) {
        self.sync_run_view = None;
        self._sync_run_view_task = None;
        cx.notify();
    }
}

fn sync_run_record_modal(
    view: &SyncRunViewState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_sync_run_view(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .debug_selector(|| "sync-run-record-modal".into())
                .w(px(960.))
                .h(px(620.))
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .text_color(colors.text)
                .key_context("SyncRunRecord")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.close_sync_run_view(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(52.))
                        .flex_none()
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .child(
                            div()
                                .text_size(ui_text_size(16.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.sync.record_list_title")),
                        )
                        .child(
                            div()
                                .debug_selector(|| "sync-run-record-close".into())
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
                                        this.close_sync_run_view(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .flex()
                        .child(sync_run_record_list(view, colors, cx))
                        .child(sync_run_record_detail_pane(view, colors)),
                ),
        )
}

/// 左栏：最近任务列表；点击切换右侧详情。
fn sync_run_record_list(
    view: &SyncRunViewState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let list = div()
        .debug_selector(|| "sync-run-record-list".into())
        .w(px(260.))
        .flex_none()
        .flex()
        .flex_col()
        .border_r_1()
        .border_color(colors.border_soft);
    if view.runs.is_empty() {
        return list.child(sync_run_centered_note(tr("app.sync.record_list_empty"), colors));
    }
    let mut rows = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_1()
        .p_2();
    for run in &view.runs {
        rows = rows.child(sync_run_record_row(
            run,
            view.selected == Some(run.id),
            colors,
            cx,
        ));
    }
    list.child(rows)
}

fn sync_run_record_row(
    run: &SyncRunRecord,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let run_id = run.id;
    div()
        .debug_selector(move || format!("sync-run-record-run:{run_id}"))
        .flex()
        .flex_col()
        .gap(px(2.))
        .px_2()
        .py_2()
        .rounded(colors.radius)
        .cursor_pointer()
        .when(selected, |this| this.bg(colors.tree_selected))
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.select_sync_run(run_id, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!(
                            "#{}  {}",
                            run.id,
                            sync_run_started_label(run.started_unix_ms)
                        )),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(sync_run_status_color(&run.status, colors))
                        .child(sync_run_status_label(&run.status)),
                ),
        )
        .child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .overflow_hidden()
                .whitespace_nowrap()
                .child(format!("{} → {}", run.source, run.target)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(sync_run_metric("+", run.inserted, colors.success, colors))
                .child(sync_run_metric("~", run.updated, colors.warning, colors))
                .child(sync_run_metric("-", run.deleted, colors.danger, colors)),
        )
}

/// 右栏：选中任务的加载态 / 错误 / 明细。
fn sync_run_record_detail_pane(view: &SyncRunViewState, colors: UiColors) -> impl IntoElement {
    let body = if view.loading {
        sync_run_centered_note(tr("app.sync.record_loading"), colors)
    } else if let Some(detail) = view.detail.as_ref() {
        sync_run_detail_body(detail, colors)
    } else if let Some(error) = view.error.as_ref() {
        sync_run_centered_note(error.clone(), colors)
    } else {
        sync_run_centered_note(tr("app.sync.record_list_empty"), colors)
    };
    div()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .child(body)
}

fn sync_run_detail_body(detail: &SyncRunDetail, colors: UiColors) -> Div {
    let run = &detail.run;
    let mut body = div()
        .flex()
        .flex_col()
        .child(sync_run_detail_header(run, colors));
    if detail.tables.is_empty() {
        return body.child(sync_run_centered_note(
            tr("app.sync.record_no_tables"),
            colors,
        ));
    }
    for table in &detail.tables {
        let chunks = detail
            .chunks
            .iter()
            .filter(|chunk| chunk.table_key == table.table_key);
        let samples = detail
            .samples
            .iter()
            .filter(|sample| sample.table_key == table.table_key);
        let failures = detail
            .failures
            .iter()
            .filter(|failure| failure.table_key == table.table_key);
        body = body.child(sync_run_table_section(
            table, chunks, samples, failures, colors,
        ));
    }
    body
}

/// 详情头部：标题 + 状态徽标 + 两端 + 增改删计数，替代原来一行塞满的灰字摘要。
fn sync_run_detail_header(run: &SyncRunRecord, colors: UiColors) -> Div {
    let elapsed = run.finished_unix_ms.saturating_sub(run.started_unix_ms);
    div()
        .px_4()
        .py_3()
        .flex_none()
        .flex()
        .flex_col()
        .gap_1()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!(
                            "#{}  {}",
                            run.id,
                            sync_run_started_label(run.started_unix_ms)
                        )),
                )
                .child(sync_run_status_pill(&run.status, colors))
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.sync.record_elapsed")
                                .replace("%{value}", &sync_run_duration_label(elapsed)),
                        ),
                ),
        )
        .child(
            div()
                .text_size(ui_text_size(10.5))
                .text_color(colors.muted)
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .child(format!("{} → {}", run.source, run.target)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(sync_run_metric("+", run.inserted, colors.success, colors))
                .child(sync_run_metric("~", run.updated, colors.warning, colors))
                .child(sync_run_metric("-", run.deleted, colors.danger, colors)),
        )
}

fn sync_run_table_section<'a>(
    table: &fluxdb_storage::SyncRunTableRecord,
    chunks: impl Iterator<Item = &'a fluxdb_storage::SyncRunChunkRecord>,
    samples: impl Iterator<Item = &'a fluxdb_storage::SyncRunSampleRecord>,
    failures: impl Iterator<Item = &'a fluxdb_storage::SyncRunFailureRecord>,
    colors: UiColors,
) -> Div {
    let chunks = chunks.collect::<Vec<_>>();
    let samples = samples.collect::<Vec<_>>();
    let failures = failures.collect::<Vec<_>>();
    let mut section = div()
        .debug_selector({
            let key = table.table_key.clone();
            move || format!("sync-run-record-table:{key}")
        })
        .flex()
        .flex_col()
        .gap_1()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(ui_text_size(12.))
                        .child(table.name.clone()),
                )
                .child(sync_run_pill(
                    sync_run_table_state_label(&table.state),
                    sync_run_table_state_color(&table.state, colors),
                    colors,
                ))
                .child(div().flex_1())
                .child(sync_run_metric("+", table.inserted, colors.success, colors))
                .child(sync_run_metric("~", table.updated, colors.warning, colors))
                .child(sync_run_metric("-", table.deleted, colors.danger, colors)),
        )
        .when_some(table.error.clone(), |this, error| {
            this.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(colors.hover)
                    .text_size(ui_text_size(10.))
                    .text_color(colors.danger)
                    .whitespace_normal()
                    .child(error),
            )
        });
    if chunks.is_empty() {
        section = section.child(sync_run_muted_line(
            tr("app.sync.record_empty_chunks"),
            colors,
        ));
    } else {
        section = section.child(sync_run_chunk_grid(&chunks, colors));
    }
    if samples.is_empty() {
        section = section.child(sync_run_muted_line(tr("app.sync.record_no_samples"), colors));
    } else {
        section = section.child(
            div()
                .pt_1()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(
                    tr("app.sync.record_samples")
                        .replace("%{count}", &samples.len().to_string()),
                ),
        );
        for (index, sample) in samples.iter().enumerate() {
            section = section.child(sync_run_sample_row(sample, index, colors));
        }
    }
    if !failures.is_empty() {
        section = section.child(
            div()
                .pt_1()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(
                    tr("app.sync.record_failures")
                        .replace("%{count}", &failures.len().to_string()),
                ),
        );
        for (index, failure) in failures.iter().enumerate() {
            section = section.child(sync_run_failure_row(failure, index, colors));
        }
    }
    section
}

/// 失败行：键 + 数据库原文，单行省略；完整原因可从日志或账本导出。
fn sync_run_failure_row(
    failure: &fluxdb_storage::SyncRunFailureRecord,
    index: usize,
    colors: UiColors,
) -> Div {
    let zebra = index % 2 == 1;
    div()
        .debug_selector({
            let table_key = failure.table_key.clone();
            move || format!("sync-run-failure:{table_key}:{index}")
        })
        .flex()
        .items_center()
        .gap_2()
        .h(px(24.))
        .px_2()
        .rounded(px(4.))
        .when(zebra, |this| this.bg(colors.panel_alt))
        .child(
            div()
                .flex_none()
                .px_1p5()
                .py_0p5()
                .rounded(px(4.))
                .bg(colors.hover)
                .text_size(ui_text_size(10.))
                .text_color(colors.danger)
                .child(tr("app.sync.record_failure_badge")),
        )
        .child(
            div()
                .flex_none()
                .max_w(px(220.))
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(ui_text_size(10.5))
                .text_color(colors.text)
                .child(failure.row_key.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .text_size(ui_text_size(10.))
                .text_color(colors.danger)
                .child(failure.message.clone()),
        )
}

/// 块明细：固定列宽的行式列表，避免长键把计数和耗时挤成一行流水账。
fn sync_run_chunk_grid(chunks: &[&fluxdb_storage::SyncRunChunkRecord], colors: UiColors) -> Div {
    let mut grid = div()
        .flex()
        .flex_col()
        .rounded(px(4.))
        .border_1()
        .border_color(colors.border_soft)
        .overflow_hidden();
    for (index, chunk) in chunks.iter().enumerate() {
        let zebra = index % 2 == 1;
        grid = grid.child(
            div()
                .debug_selector({
                    let table_key = chunk.table_key.clone();
                    let ordinal = chunk.ordinal;
                    move || format!("sync-run-chunk:{table_key}:{ordinal}")
                })
                .flex()
                .items_center()
                .gap_2()
                .h(px(24.))
                .px_2()
                .when(zebra, |this| this.bg(colors.panel_alt))
                .child(
                    div()
                        .w(px(40.))
                        .flex_none()
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(format!("#{}", chunk.ordinal)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .truncate()
                        .font_family(crate::PLATFORM_MONOSPACE_FONT)
                        .text_size(ui_text_size(10.5))
                        .text_color(colors.text)
                        .child(sync_run_chunk_range(chunk)),
                )
                .child(sync_run_count_cell("+", chunk.inserted, colors.success, colors))
                .child(sync_run_count_cell("~", chunk.updated, colors.warning, colors))
                .child(sync_run_count_cell("-", chunk.deleted, colors.danger, colors))
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_align(gpui::TextAlign::Right)
                        .text_size(ui_text_size(10.))
                        .text_color(colors.muted)
                        .child(sync_run_duration_label(chunk.elapsed_ms)),
                ),
        );
    }
    grid
}

fn sync_run_chunk_range(chunk: &fluxdb_storage::SyncRunChunkRecord) -> String {
    match (&chunk.key_from, &chunk.key_to) {
        (Some(from), Some(to)) => format!(
            "{}  …  {}",
            sync_run_short_key(from),
            sync_run_short_key(to)
        ),
        (Some(from), None) => format!("{}  …", sync_run_short_key(from)),
        (None, Some(to)) => format!("…  {}", sync_run_short_key(to)),
        (None, None) => "—".to_string(),
    }
}

/// 超长键（UUID、复合键）保留头尾、中间省略：范围仍可辨认，又不会把右侧计数挤掉。
fn sync_run_short_key(key: &str) -> String {
    const HEAD: usize = 12;
    const TAIL: usize = 6;
    let length = key.chars().count();
    if length <= HEAD + TAIL + 1 {
        return key.to_string();
    }
    let head: String = key.chars().take(HEAD).collect();
    let tail: String = key.chars().skip(length - TAIL).collect();
    format!("{head}…{tail}")
}

/// 行级样本：动作做徽标，键用等宽字体，字段变更跟在后面；超长内容截断。
fn sync_run_sample_row(
    sample: &fluxdb_storage::SyncRunSampleRecord,
    index: usize,
    colors: UiColors,
) -> Div {
    let (label, color) = match sample.action.as_str() {
        "insert" => (tr("app.sync.inserted"), colors.success),
        "update" => (tr("app.sync.updated"), colors.warning),
        _ => (tr("app.sync.deleted"), colors.danger),
    };
    let zebra = index % 2 == 1;
    let detail = match (&sample.column, &sample.source_value, &sample.target_value) {
        (Some(column), Some(source), Some(target)) => {
            format!("{column}: {source} → {target}")
        }
        _ => String::new(),
    };
    div()
        .debug_selector({
            let table_key = sample.table_key.clone();
            move || format!("sync-run-sample:{table_key}:{index}")
        })
        .flex()
        .items_center()
        .gap_2()
        .h(px(24.))
        .px_2()
        .rounded(px(4.))
        .when(zebra, |this| this.bg(colors.panel_alt))
        .child(
            div()
                .flex_none()
                .px_1p5()
                .py_0p5()
                .rounded(px(4.))
                .bg(colors.hover)
                .text_size(ui_text_size(10.))
                .text_color(color)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .font_family(crate::PLATFORM_MONOSPACE_FONT)
                .text_size(ui_text_size(10.5))
                .text_color(colors.text)
                .child(sample.row_key.clone()),
        )
        .when(!detail.is_empty(), |this| {
            this.child(
                div()
                    .flex_none()
                    .max_w(px(300.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(detail),
            )
        })
}

/// 状态徽标：底色走 hover，文字走状态色，比自己拼一串彩色文字更好扫。
fn sync_run_status_pill(status: &str, colors: UiColors) -> Div {
    sync_run_pill(
        sync_run_status_label(status),
        sync_run_status_color(status, colors),
        colors,
    )
}

fn sync_run_pill(label: SharedString, color: gpui::Rgba, colors: UiColors) -> Div {
    div()
        .flex_none()
        .px_1p5()
        .py_0p5()
        .rounded(px(4.))
        .bg(colors.hover)
        .text_size(ui_text_size(10.))
        .text_color(color)
        .child(label)
}

/// `+插入` / `~更新` / `-删除` 计数；为 0 时整组转灰，让真实变化的数字自己跳出来。
fn sync_run_metric(sign: &str, count: u64, color: gpui::Rgba, colors: UiColors) -> Div {
    let text_color = if count == 0 { colors.muted } else { color };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .text_size(ui_text_size(10.5))
        .child(div().text_color(text_color).child(sign.to_string()))
        .child(
            div()
                .font_weight(if count == 0 {
                    gpui::FontWeight::NORMAL
                } else {
                    gpui::FontWeight::SEMIBOLD
                })
                .text_color(if count == 0 { colors.muted } else { colors.text })
                .child(count.to_string()),
        )
}

/// 块列表里的计数格：定宽右对齐，等宽字体保证多行数字上下对齐。
fn sync_run_count_cell(sign: &str, count: u64, color: gpui::Rgba, colors: UiColors) -> Div {
    div()
        .w(px(56.))
        .flex_none()
        .text_align(gpui::TextAlign::Right)
        .font_family(crate::PLATFORM_MONOSPACE_FONT)
        .text_size(ui_text_size(10.5))
        .text_color(if count == 0 { colors.muted } else { color })
        .child(format!("{sign}{count}"))
}

/// 耗时展示：毫秒级给整数毫秒，秒级保留一位小数，分钟级换 `m ss`。
fn sync_run_duration_label(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1_000.)
    } else {
        let seconds = ms / 1_000;
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

fn sync_run_muted_line(text: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .text_size(ui_text_size(10.))
        .text_color(colors.muted)
        .child(text.into())
}

fn sync_run_centered_note(text: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_h(px(120.))
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_size(ui_text_size(11.))
        .text_color(colors.muted)
        .child(text.into())
}

fn sync_run_status_label(status: &str) -> SharedString {
    match status {
        "success" => tr("app.sync.record_status_success"),
        "failed" => tr("app.sync.record_status_failed"),
        "canceled" => tr("app.sync.record_status_canceled"),
        _ => tr("app.sync.record_status_running"),
    }
}

fn sync_run_status_color(status: &str, colors: UiColors) -> gpui::Rgba {
    match status {
        "success" => colors.success,
        "failed" => colors.danger,
        "canceled" => colors.warning,
        _ => colors.muted,
    }
}

fn sync_run_table_state_label(state: &str) -> SharedString {
    match state {
        "synced" => tr("app.sync.record_table_synced"),
        "skipped" => tr("app.sync.record_table_skipped"),
        _ => tr("app.sync.record_table_failed"),
    }
}

fn sync_run_table_state_color(state: &str, colors: UiColors) -> gpui::Rgba {
    match state {
        "synced" => colors.success,
        "skipped" => colors.warning,
        _ => colors.danger,
    }
}

/// 记录头部的时间展示：`MM-DD HH:MM`；无效值回落 `—`。
fn sync_run_started_label(started_unix_ms: u64) -> String {
    i64::try_from(started_unix_ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".to_string())
}
