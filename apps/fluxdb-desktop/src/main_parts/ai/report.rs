// T040 会话产物卡 → 独立报告弹框（只读快照）与宿主安全导出。
//
// 边界（设计 §27、§31）：
// - 弹框只读 `AiReportView`（产物行 + 已生成快照），打开不重查库、不调模型、不刷新来源版本。
// - renderer 缺失/快照损坏时仍展示可解释元数据（标题/类型/renderer/来源/口径），不伪造正文。
// - 导出目标来自用户在系统保存对话框里选择的路径（路径授权），写临时文件后改名并回读核对；
//   失败不显示成功、不留下半截文件；HTML 导出为转义后的静态文档，不含脚本与远程资源。
/// 报告弹框的共享状态：dialog builder 在 `NavicatMain::render` 期间同步执行，此时主视图处于
/// lease 状态，builder 内不能 `view.read(cx)`，实时数据统一放 `Rc<RefCell<..>>`
/// （先例：`SqlFileModalData`）。
#[derive(Default)]
struct AiReportModalData {
    dialog_open: bool,
    report: Option<fluxdb_app::AiReportView>,
    /// 弹框内「来源 SQL」区域的展开态（默认折叠）。
    sql_open: bool,
    /// 导出进行中：按钮 loading/disabled，防止重复选择路径与重复写文件。
    exporting: bool,
}

impl AiReportModalData {
    /// 关闭弹框后的统一清理：Dialog 内置关闭（X / Esc / 点击遮罩）与页脚「关闭」都必须清掉
    /// `dialog_open`，否则残留标志会让产物卡后续「打开报告」被静默拦下。
    fn reset(&mut self) {
        self.dialog_open = false;
        self.report = None;
        self.sql_open = false;
    }
}

#[derive(Clone)]
struct AiReportDialogViews {
    view: Entity<NavicatMain>,
    data: Rc<std::cell::RefCell<AiReportModalData>>,
    colors: UiColors,
}

/// 产物卡打开报告的意图：原型卡片有「SQL / 导出 / 打开报告」三个动作，
/// 它们都进入同一弹框，只是初始视图/动作不同。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiReportOpenIntent {
    Report,
    Sql,
    Export,
}

impl NavicatMain {
    /// 从会话产物卡打开报告：后台读产物行 + 快照，成功后打开独立弹框（不重查数据库）。
    fn open_ai_artifact_report(
        &mut self,
        artifact_id: String,
        intent: AiReportOpenIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_report_modal.borrow().dialog_open {
            return;
        }
        let Ok(storage) = self.controller.ai_storage() else {
            self.show_message(
                tr("app.ai_report.storage_missing"),
                AppMessageKind::Error,
                cx,
            );
            return;
        };
        self.show_message(tr("app.ai_report.loading_snapshot"), AppMessageKind::Success, cx);
        let task = cx.spawn_in(window, async move |view, cx| {
            let loaded = cx
                .background_spawn({
                    let artifact_id = artifact_id.clone();
                    async move { fluxdb_app::load_ai_report(&storage, &artifact_id) }
                })
                .await;
            let _ = view.update_in(cx, |this, window, cx| match loaded {
                Ok(report) => this.open_ai_report_dialog(report, intent, window, cx),
                Err(error) => this.show_message(
                    tr("app.ai_report.open_failed").replace("%{error}", &error.message),
                    AppMessageKind::Error,
                    cx,
                ),
            });
        });
        self._ai_report_task = Some(task);
    }

    fn open_ai_report_dialog(
        &mut self,
        report: fluxdb_app::AiReportView,
        intent: AiReportOpenIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        {
            let mut data = self.ai_report_modal.borrow_mut();
            data.dialog_open = true;
            data.sql_open = intent == AiReportOpenIntent::Sql;
            data.report = Some(report);
            data.exporting = false;
        }
        let colors = ui_colors_from_theme(self.theme_mode, cx);
        let views = AiReportDialogViews {
            view: cx.entity(),
            data: self.ai_report_modal.clone(),
            colors,
        };
        // 焦点先落到主视图：Dialog 打开时记录 previous_focused_handle，关闭后还原到这里。
        self.focus_handle.focus(window, cx);
        window.open_dialog(cx, move |dialog, window, _cx| {
            // 弹框宽度为 min(960, 视口 90%)；正文可用宽度去掉弹框与卡片内边距，
            // 供分类轴标签按真实可用宽度自适应。
            let viewport = f32::from(window.viewport_size().width);
            let content_px = ((viewport * 0.9).min(960.) - 56.).max(160.);
            ai_report_build_dialog(dialog, views.clone(), content_px)
        });
        // 卡片「导出」直达：弹框先打开，再走同一路径选择与安全写文件链路。
        if intent == AiReportOpenIntent::Export {
            self.begin_ai_report_export(fluxdb_app::AiReportExportFormat::Html, window, cx);
        }
    }

    /// 关闭报告弹框（关闭按钮/Esc/遮罩由 Dialog 内部处理时，on_close 走同一清理）。
    fn close_ai_report_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = {
            let mut data = self.ai_report_modal.borrow_mut();
            let was_open = data.dialog_open;
            data.reset();
            was_open
        };
        if was_open {
            window.close_dialog(cx);
        }
        cx.notify();
    }

    /// 选择导出路径并安全写出；写失败不显示成功、不留下半截文件。
    fn begin_ai_report_export(
        &mut self,
        format: fluxdb_app::AiReportExportFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let report = {
            let data = self.ai_report_modal.borrow();
            if data.exporting {
                return;
            }
            match data.report.clone() {
                Some(report) => report,
                None => return,
            }
        };
        if report.metadata_only() {
            self.show_message(
                tr("app.ai_report.export_no_body"),
                AppMessageKind::Warning,
                cx,
            );
            return;
        }
        let content = match fluxdb_app::render_ai_report_export(&report, format) {
            Ok(content) => content,
            Err(error) => {
                self.show_message(
                    tr("app.ai_report.export_failed").replace("%{error}", &error.message),
                    AppMessageKind::Error,
                    cx,
                );
                return;
            }
        };
        let suggested = format!(
            "{}.{}",
            safe_ai_report_file_stem(&report.title),
            format.extension()
        );
        let receiver = cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested));
        self.ai_report_modal.borrow_mut().exporting = true;
        cx.notify();
        let task = cx.spawn_in(window, async move |view, cx| {
            let selected = receiver.await;
            match selected {
                Ok(Ok(Some(path))) => {
                    let write_path = path.clone();
                    let written = cx
                        .background_spawn(async move {
                            fluxdb_app::write_ai_report_export(&write_path, &content)
                        })
                        .await;
                    let _ = view.update_in(cx, |this, _window, cx| {
                        this.ai_report_modal.borrow_mut().exporting = false;
                        match written {
                            Ok(fact) => this.show_message(
                                tr("app.ai_report.exported")
                                    .replace("%{bytes}", &fact.byte_count.to_string())
                                    .replace("%{hash}", &fact.content_hash)
                                    .replace("%{path}", &path.display().to_string()),
                                AppMessageKind::Success,
                                cx,
                            ),
                            Err(error) => this.show_message(
                                tr("app.ai_report.export_failed")
                                    .replace("%{error}", &error.message),
                                AppMessageKind::Error,
                                cx,
                            ),
                        }
                        cx.notify();
                    });
                }
                Ok(Ok(None)) => {
                    let _ = view.update_in(cx, |this, _, cx| {
                        this.ai_report_modal.borrow_mut().exporting = false;
                        cx.notify();
                    });
                }
                Ok(Err(error)) => {
                    let _ = view.update_in(cx, |this, _, cx| {
                        this.ai_report_modal.borrow_mut().exporting = false;
                        this.show_message(
                            tr("app.ai_report.pick_path_failed")
                                .replace("%{error}", &error.to_string()),
                            AppMessageKind::Error,
                            cx,
                        );
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = view.update_in(cx, |this, _, cx| {
                        this.ai_report_modal.borrow_mut().exporting = false;
                        this.show_message(
                            tr("app.ai_report.pick_path_failed")
                                .replace("%{error}", &error.to_string()),
                            AppMessageKind::Error,
                            cx,
                        );
                        cx.notify();
                    });
                }
            }
        });
        self._ai_report_export_task = Some(task);
    }
}

/// 导出文件名主干：去掉路径分隔符/控制字符，避免越界或非法文件名。
fn safe_ai_report_file_stem(title: &str) -> String {
    let stem: String = title
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' | '\n' | '\r' | '\t' => {
                '-'
            }
            _ => ch,
        })
        .take(64)
        .collect();
    let trimmed = stem.trim().trim_end_matches(['.', ' ']).to_string();
    if trimmed.is_empty() {
        "artifact-report".to_string()
    } else {
        trimmed
    }
}

fn ai_report_build_dialog(
    dialog: gpui_component::dialog::Dialog,
    views: AiReportDialogViews,
    available_px: f32,
) -> gpui_component::dialog::Dialog {
    let (report, sql_open, exporting) = {
        let data = views.data.borrow();
        (data.report.clone(), data.sql_open, data.exporting)
    };
    let Some(report) = report else {
        return dialog
            .title(tr("app.ai_report.artifact_title"))
            .child(tr("app.ai_report.unavailable"));
    };
    let colors = views.colors;
    let title = tr("app.ai_report.title_with_name").replace("%{title}", &report.title);
    let view_sql = views.view.clone();
    let view_export_html = views.view.clone();
    let view_close = views.view.clone();
    let data_sql = views.data.clone();
    let data_close = views.data.clone();
    let body = ai_report_body(&report, sql_open, exporting, available_px, colors);
    dialog
        .w(px(960.))
        .max_w(px(960.))
        .close_button(true)
        .overlay_closable(true)
        .keyboard(true)
        // X / Esc / 点击遮罩由 Dialog 内部关闭，不经过页脚「关闭」按钮；这里必须清掉
        // `dialog_open`，否则产物卡后续「打开报告」会被静默拦下。
        .on_close(move |_, _, _| data_close.borrow_mut().reset())
        .title(title)
        .child(body)
        .footer(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ai_report_footer_meta(&report, colors))
                .child(div().flex_1())
                .child(
                    Button::new("ai-report-sql-toggle")
                        .debug_selector(|| "ai-report-sql-toggle".into())
                        .small()
                        .outline()
                        .cursor_pointer()
                        .child(
                            tr("app.ai_report.source_sql_count")
                                .replace("%{count}", &report.sources.len().to_string()),
                        )
                        .on_click(move |_, _window, _cx| {
                            // 弹框 builder 每帧重读共享状态：翻转后请求重绘即可刷新折叠区域。
                            {
                                let mut data = data_sql.borrow_mut();
                                data.sql_open = !data.sql_open;
                            }
                            let _ = view_sql.update(_cx, |_this, cx| cx.notify());
                        }),
                )
                .child(
                    Button::new("ai-report-export-html")
                        .debug_selector(|| "ai-report-export-html".into())
                        .small()
                        .outline()
                        .cursor_pointer()
                        .disabled(exporting || report.metadata_only())
                        .child(app_icon(AppIcon::FolderUp, 13., colors.muted))
                        // 导出与弹框同构：自包含 HTML（含图表），不再提供 Markdown。
                        .child(tr("app.ai_report.export_html"))
                        .on_click(move |_, window, cx| {
                            let _ = view_export_html.update(cx, |this, cx| {
                                this.begin_ai_report_export(
                                    fluxdb_app::AiReportExportFormat::Html,
                                    window,
                                    cx,
                                );
                            });
                        }),
                )
                .child(
                    Button::new("ai-report-close")
                        .debug_selector(|| "ai-report-close".into())
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .child(tr("app.common.close"))
                        .on_click(move |_, window, cx| {
                            let _ = view_close.update(cx, |this, cx| {
                                this.close_ai_report_dialog(window, cx);
                            });
                        }),
                ),
        )
}

fn ai_report_footer_meta(report: &fluxdb_app::AiReportView, colors: UiColors) -> gpui::AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .text_size(ui_text_size(10.))
        .text_color(colors.muted)
        .child(app_icon(AppIcon::Activity, 12., colors.success))
        .child(format!(
            "{} · renderer {} v{} · rev {}{}",
            report.status,
            report.renderer,
            report.renderer_version,
            report.revision,
            if report.truncated {
                tr("app.ai_report.sampled_suffix").to_string()
            } else {
                String::new()
            }
        ))
        .into_any_element()
}

/// 报告正文：只消费已读取的快照视图，不再访库。
fn ai_report_body(
    report: &fluxdb_app::AiReportView,
    sql_open: bool,
    exporting: bool,
    available_px: f32,
    colors: UiColors,
) -> gpui::AnyElement {
    let mut column = div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .text_size(ui_text_size(12.));
    column = column.child(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1p5()
            .child(ai_report_tag(
                tr("app.ai_report.artifact_with_status")
                    .replace("%{status}", &report.status),
                if report.status == "ready" {
                    colors.success
                } else {
                    colors.warning
                },
                colors,
            ))
            .child(ai_report_tag(
                tr("app.ai_report.renderer_tag").replace("%{renderer}", &report.renderer),
                colors.info,
                colors,
            ))
            .child(ai_report_tag(
                tr("app.ai_report.snapshot_tag").to_string(),
                colors.muted,
                colors,
            )),
    );
    if let Some(issue) = &report.data_issue {
        column = column.child(
            gpui_component::alert::Alert::error("ai-report-data-issue", issue.clone()).small(),
        );
    }
    // 关键指标条：数字由宿主从真实快照点汇总，不是模型编造，也不是未查询的结论。
    if let Some(metrics) = ai_report_metrics(report, colors) {
        column = column.child(metrics);
    }
    if let Some(metric) = &report.spec.metric {
        let mut meta = tr("app.ai_report.metric_meta")
            .replace("%{name}", &metric.name)
            .replace("%{timezone}", &metric.timezone);
        if let Some(assumption) = &metric.assumption {
            meta.push_str(
                &tr("app.ai_report.metric_assumption").replace("%{assumption}", assumption),
            );
        }
        column = column.child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(meta),
        );
    }

    for block in &report.spec.blocks {
        match block {
            fluxdb_core::ArtifactBlock::Chart {
                result_ref,
                chart,
                x,
                y,
            } => {
                column = column.child(ai_report_chart_block(
                    report,
                    result_ref,
                    *chart,
                    x,
                    y,
                    available_px,
                    colors,
                ));
            }
            fluxdb_core::ArtifactBlock::Table {
                result_ref,
                columns,
            } => {
                column = column.child(ai_report_table_block(report, result_ref, columns, colors));
            }
            fluxdb_core::ArtifactBlock::Note { text } => {
                column = column.child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.ai_report.model_note")
                                .replace("%{text}", text),
                        ),
                );
            }
        }
    }
    if report.spec.blocks.is_empty() {
        column = column.child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.muted)
                .child(tr("app.ai_report.no_blocks")),
        );
    }

    column = column.child(ai_report_provenance(report, colors));
    if sql_open {
        column = column.child(ai_report_sql_section(report, colors));
    }
    if exporting {
        column = column.child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(tr("app.ai_report.exporting")),
        );
    }
    column.into_any_element()
}

/// 关键指标条：汇总真实图表点（求和/极值/点数），只做算术，不推断业务结论。
/// 没有可汇总的数值点时返回 `None`，不显示伪造指标。
fn ai_report_metrics(
    report: &fluxdb_app::AiReportView,
    colors: UiColors,
) -> Option<gpui::AnyElement> {
    // 汇总口径由 App 层给出：图表、指标条与导出的 HTML 共用同一份真实点算术。
    let summary = report.chart_summary()?;
    let number = |value: f64| {
        if (value.fract()).abs() < f64::EPSILON && value.abs() < 1e15 {
            format!("{}", value as i64)
        } else {
            format!("{value:.2}")
        }
    };
    let cell = |label: &str, value: String, note: String| {
        div()
            .flex_1()
            .min_w(px(0.))
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.border_soft)
            .bg(colors.panel_alt)
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(div().text_size(ui_text_size(10.)).text_color(colors.muted).child(label.to_string()))
            .child(
                div()
                    .text_size(ui_text_size(18.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .truncate()
                    .child(value),
            )
            .child(div().text_size(ui_text_size(10.)).text_color(colors.muted).truncate().child(note))
    };
    Some(
        div()
            .debug_selector(|| "ai-report-metrics".into())
            .w_full()
            .flex()
            .gap_2()
            .child(cell(
                &tr("app.ai_report.metric_sum").replace("%{value}", &summary.y),
                number(summary.sum),
                tr("app.ai_report.metric_points")
                    .replace("%{count}", &summary.count.to_string()),
            ))
            .child(cell(
                &tr("app.ai_report.metric_peak").replace("%{value}", &summary.y),
                number(summary.max),
                if summary.max_label.is_empty() {
                    tr("app.ai_report.metric_source_value").to_string()
                } else {
                    tr("app.ai_report.metric_max_label")
                        .replace("%{label}", &summary.max_label)
                },
            ))
            .into_any_element(),
    )
}

fn ai_report_tag(label: String, tone: gpui::Rgba, colors: UiColors) -> gpui::AnyElement {
    div()
        .flex_none()
        .px_1p5()
        .py(px(2.))
        .rounded(px(999.))
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .text_color(tone)
        .text_size(ui_text_size(10.))
        .child(label)
        .into_any_element()
}

fn ai_report_source<'a>(
    report: &'a fluxdb_app::AiReportView,
    declared: &str,
) -> Option<&'a fluxdb_app::AiReportSource> {
    report
        .sources
        .iter()
        .find(|source| source.declared_ref.as_deref() == Some(declared))
}

fn ai_report_table_block(
    report: &fluxdb_app::AiReportView,
    result_ref: &str,
    columns: &[String],
    colors: UiColors,
) -> gpui::AnyElement {
    let header = div()
        .text_size(ui_text_size(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(
            tr("app.ai_report.table_header")
                .replace("%{count}", &columns.len().to_string()),
        );
    let body = match ai_report_source(report, result_ref) {
        None => ai_report_placeholder(tr("app.ai_report.table_source_unresolved"), colors),
        Some(_) if columns.is_empty() => {
            ai_report_placeholder(tr("app.ai_report.table_no_columns"), colors)
        }
        Some(source) => {
            let mut head = div()
                .flex()
                .bg(colors.panel_alt)
                .border_1()
                .border_color(colors.border_soft);
            for column in columns {
                head = head.child(ai_report_cell(column, true, colors));
            }
            let mut rows = div().flex().flex_col();
            let mut shown = 0usize;
            for item in &source.items {
                let Some(object) = item.as_object() else {
                    continue;
                };
                if shown >= 50 {
                    break;
                }
                let mut row = div()
                    .flex()
                    .border_1()
                    .border_color(colors.border_soft)
                    .border_t_0();
                for column in columns {
                    let value = object
                        .get(column)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let text = if value.is_null() {
                        String::new()
                    } else if let Some(text) = value.as_str() {
                        text.to_string()
                    } else {
                        value.to_string()
                    };
                    row = row.child(ai_report_cell(&text, false, colors));
                }
                rows = rows.child(row);
                shown += 1;
            }
            let note = if source.items.len() > shown || source.truncated {
                div()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.warning)
                    .child(
                        tr("app.ai_report.table_sampled")
                            .replace("%{shown}", &shown.to_string())
                            .replace(
                                "%{total}",
                                &source.rows.max(source.items.len()).to_string(),
                            )
                    )
                    .into_any_element()
            } else {
                div()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.muted)
                    .child(
                        tr("app.ai_report.table_rows")
                            .replace("%{count}", &shown.to_string()),
                    )
                    .into_any_element()
            };
            div()
                .w_full()
                .child(head)
                .child(rows)
                .child(div().mt_1().child(note))
                .into_any_element()
        }
    };
    div()
        .w_full()
        .debug_selector(|| "ai-report-table".into())
        .flex()
        .flex_col()
        .gap_2()
        .child(header)
        .child(body)
        .into_any_element()
}

fn ai_report_cell(text: &str, header: bool, colors: UiColors) -> gpui::AnyElement {
    div()
        .flex_1()
        .min_w(px(0.))
        .px_2()
        .py_1()
        .text_size(ui_text_size(11.))
        .when(header, |cell| {
            cell.font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
        })
        .when(!header, |cell| cell.text_color(colors.text))
        .child(div().truncate().child(text.to_string()))
        .into_any_element()
}

fn ai_report_placeholder(text: impl Into<SharedString>, colors: UiColors) -> gpui::AnyElement {
    let text = text.into();
    div()
        .py_3()
        .text_size(ui_text_size(11.))
        .text_color(colors.muted)
        .child(text)
        .into_any_element()
}

/// 来源/口径/截断事实：始终可见，即使 renderer 卸载也能解释产物。
fn ai_report_provenance(report: &fluxdb_app::AiReportView, colors: UiColors) -> gpui::AnyElement {
    let mut column = div()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .pt_2()
        .border_t_1()
        .border_color(colors.border_soft)
        .text_size(ui_text_size(10.))
        .text_color(colors.muted)
        .child(if report.metadata_only() {
            tr("app.ai_report.provenance_metadata_only")
        } else {
            tr("app.ai_report.provenance")
        });
    for source in &report.sources {
        let sampled = if source.truncated {
            tr("app.ai_report.sampled_marker").to_string()
        } else {
            String::new()
        };
        column = column.child(
            div()
                .debug_selector(|| "ai-report-source".into())
                .child(
                    tr("app.ai_report.source_line")
                        .replace(
                            "%{ref}",
                            source.declared_ref.as_deref().unwrap_or("-"),
                        )
                        .replace("%{rows}", &source.rows.to_string())
                        .replace("%{sampled}", &sampled)
                        .replace(
                            "%{call}",
                            &source
                                .tool_call_id
                                .as_ref()
                                .map(|call| {
                                    tr("app.ai_report.source_call")
                                        .replace("%{call}", call)
                                })
                                .unwrap_or_default(),
                        )
                        .replace(
                            "%{result}",
                            &source
                                .result_id
                                .as_ref()
                                .map(|id| {
                                    tr("app.ai_report.source_result")
                                        .replace("%{id}", id)
                                })
                                .unwrap_or_default(),
                        ),
                ),
        );
    }
    if report.metadata_only() {
        column = column
            .child(tr("app.ai_report.metadata_only_note"));
    }
    column.into_any_element()
}

fn ai_report_sql_section(report: &fluxdb_app::AiReportView, colors: UiColors) -> gpui::AnyElement {
    let mut column = div()
        .w_full()
        .debug_selector(|| "ai-report-sql".into())
        .flex()
        .flex_col()
        .gap_2()
        .pt_2()
        .border_t_1()
        .border_color(colors.border_soft);
    let mut any = false;
    for source in &report.sources {
        if let Some(sql) = &source.sql {
            any = true;
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(colors.muted)
                            .child(
                                tr("app.ai_report.source_sql_readonly").replace(
                                    "%{ref}",
                                    source.declared_ref.as_deref().unwrap_or("-"),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .rounded(colors.radius)
                            .border_1()
                            .border_color(colors.border_soft)
                            .bg(colors.panel_alt)
                            .p_2()
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(colors.text)
                                    .child(sql.clone()),
                            ),
                    ),
            );
        }
    }
    if !any {
        column = column.child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(tr("app.ai_report.no_sql")),
        );
    }
    column.into_any_element()
}

/// 会话产物卡（T040）：把持久化的 `artifact_ref` 片段渲染成可打开的产物容器，
/// 提供「SQL / 导出 / 打开报告」三个动作；三个动作都进入独立报告弹框（只读快照）。
///
/// 卡片只消费片段里的持久化事实（标题/状态/renderer/来源数/截断），不重新读库；
/// 状态为 failed 时明确说明渲染失败，且不显示导出成功之类的假反馈。
fn ai_chat_artifact_card(
    part: &fluxdb_app::AiConversationPart,
    payload: &serde_json::Value,
    text: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let artifact_id = payload
        .get("artifact_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let title = payload
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| tr("app.ai_report.artifact_title").to_string());
    let status = payload
        .get("artifact_status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(part.status.as_str())
        .to_string();
    let renderer = payload
        .get("renderer")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let source_count = payload
        .get("source_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let truncated = payload
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let failure_code = payload
        .get("failure_code")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let (status_label, status_color) = match status.as_str() {
        "ready" => (tr("app.ai_report.status_ready"), colors.success),
        "failed" => (tr("app.ai_report.status_failed"), colors.danger),
        "expired" => (tr("app.ai_report.status_expired"), colors.muted),
        "draft" | "staged" => (tr("app.ai_report.status_preparing"), colors.muted),
        _ => (tr("app.ai_report.status_running"), colors.info),
    };
    // 每个动作打开同一弹框，只是初始视图/动作不同（与原型卡片的三个按钮一致）。
    let report_id = artifact_id.clone();
    let sql_id = artifact_id.clone();
    let export_id = artifact_id.clone();
    let report_click_id = artifact_id.clone();
    let can_open = status == "ready" || status == "failed" || status == "expired";
    let mut actions = div().flex().items_center().gap_1p5().flex_wrap();
    if !artifact_id.is_empty() {
        actions = actions
            .child(
                Button::new(SharedString::from(format!("ai-artifact-sql:{report_click_id}")))
                    .debug_selector({
                        let id = sql_id.clone();
                        move || format!("ai-artifact-sql:{id}")
                    })
                    .xsmall()
                    .outline()
                    .cursor_pointer()
                    .child(app_icon(AppIcon::FileSql, 12., colors.muted))
                    .child("SQL")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_ai_artifact_report(
                            sql_id.clone(),
                            AiReportOpenIntent::Sql,
                            window,
                            cx,
                        );
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("ai-artifact-export:{export_id}")))
                    .debug_selector({
                        let id = export_id.clone();
                        move || format!("ai-artifact-export:{id}")
                    })
                    .xsmall()
                    .outline()
                    .cursor_pointer()
                    .disabled(!can_open)
                    .child(app_icon(AppIcon::FolderUp, 12., colors.muted))
                    .child(tr("app.context.export"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_ai_artifact_report(
                            export_id.clone(),
                            AiReportOpenIntent::Export,
                            window,
                            cx,
                        );
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("ai-artifact-open:{report_id}")))
                    .debug_selector({
                        let id = report_id.clone();
                        move || format!("ai-artifact-open:{id}")
                    })
                    .xsmall()
                    .primary()
                    .cursor_pointer()
                    .disabled(!can_open)
                    .child(app_icon(AppIcon::FileSearch, 12., colors.muted))
                    .child(tr("app.ai_report.open_report"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_ai_artifact_report(
                            report_id.clone(),
                            AiReportOpenIntent::Report,
                            window,
                            cx,
                        );
                    })),
            );
    }
    div()
        .id(SharedString::from(format!("ai-artifact:{}", part.id)))
        .debug_selector({
            let id = part.id.clone();
            move || format!("ai-artifact:{id}")
        })
        .w_full()
        .min_w(px(0.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(app_icon(AppIcon::Activity, 13., colors.success))
                        .child(
                            div()
                                .min_w(px(0.))
                                .text_size(ui_text_size(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .truncate()
                                .child(title),
                        ),
                )
                .child(ai_report_tag(
                    tr("app.ai_report.artifact_with_status")
                        .replace("%{status}", status_label.as_str()),
                    status_color,
                    colors,
                )),
        )
        .child(
            div()
                .w_full()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(format!(
                    "{}{}{}",
                    if text.is_empty() {
                        String::new()
                    } else {
                        format!("{text} · ")
                    },
                    renderer,
                    if truncated {
                        tr("app.ai_report.sampled_suffix").to_string()
                    } else {
                        String::new()
                    },
                )),
        )
        .when_some(failure_code, |card, code| {
            card.child(
                div()
                    .text_size(ui_text_size(10.))
                    .text_color(colors.danger)
                    .child(
                        tr("app.ai_report.renderer_failed").replace("%{code}", &code)
                    ),
            )
        })
        .child(
            div()
                .text_size(ui_text_size(10.))
                .text_color(colors.muted)
                .child(
                    tr("app.ai_report.source_count")
                        .replace("%{count}", &source_count.to_string()),
                ),
        )
        .child(actions)
        .into_any_element()
}
