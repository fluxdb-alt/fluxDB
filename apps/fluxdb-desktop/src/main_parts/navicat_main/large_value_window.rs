impl NavicatMain {
    /// 详情面板渲染前确保大字段窗口存在：首次打开时读第一块，换单元格则换窗口。
    ///
    /// 在渲染期按「面板当前单元格 + 内容引用」对齐，避免依赖某个打开入口先跑一遍；
    /// 同一窗口重复渲染不会重复发起读取。
    fn ensure_large_value_window(
        &mut self,
        tab_id: TabId,
        row: usize,
        column: usize,
        large: &fluxdb_core::LargeCellValue,
        cx: &mut Context<Self>,
    ) -> LargeValueWindows {
        let stale = self
            .large_value_windows
            .get(&tab_id)
            .is_none_or(|window| {
                window.row != row
                    || window.column != column
                    || window.view.content_id != large.content_id
            });
        if stale {
            let view = fluxdb_app::LargeValueView::new(large);
            self.large_value_windows.insert(
                tab_id,
                LargeValueWindows {
                    row,
                    column,
                    view,
                },
            );
            self.spawn_cell_content_chunk(tab_id, row, column, 0, cx);
        }
        self.large_value_windows
            .get(&tab_id)
            .cloned()
            .unwrap_or_else(|| LargeValueWindows {
                row,
                column,
                view: fluxdb_app::LargeValueView::new(large),
            })
    }

    /// 详情面板关闭或换到普通单元格时丢弃窗口，别让「读取更多」读到上一格。
    fn drop_large_value_window(&mut self, tab_id: TabId) {
        self.large_value_windows.remove(&tab_id);
    }

    /// 后台读一块。同一单元格只保留一个在途任务，连点不会并发读同一窗口。
    fn spawn_cell_content_chunk(
        &mut self,
        tab_id: TabId,
        row: usize,
        column: usize,
        offset: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(window) = self.large_value_windows.get_mut(&tab_id) else {
            return;
        };
        let content_id = window.view.content_id.clone();
        // 去重按内容引用而不是单元格：同一格换了内容（重新加载结果）时旧任务的结果必须丢弃。
        let key = (tab_id, content_id.clone());
        if self._cell_content_chunk_tasks.contains_key(&key) {
            return;
        }
        let kind = window.view.kind;
        window.view.loading = true;
        window.view.error = None;
        let mut controller = self.controller.clone();
        let task_key = key.clone();
        let task = cx.spawn(async move |view, cx| {
            let requested = content_id.clone();
            let result = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::LoadCellContentChunk {
                        tab_id,
                        row,
                        column,
                        content_id,
                        kind,
                        offset,
                    })
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._cell_content_chunk_tasks.remove(&task_key);
                    // 窗口已换内容：结果属于旧引用，丢弃。
                    let current = this
                        .large_value_windows
                        .get(&tab_id)
                        .is_some_and(|window| window.view.content_id == requested);
                    match &result {
                        _ if !current => {}
                        AppEvent::CellContentChunkLoaded { .. } => {
                            this.apply_app_event(&result, cx);
                        }
                        AppEvent::Failed(error) => {
                            this.set_large_value_error(tab_id, row, column, &error.message);
                            this.show_message(error.message.clone(), AppMessageKind::Error, cx);
                        }
                        _ => {
                            this.set_large_value_error(
                                tab_id,
                                row,
                                column,
                                tr("app.cell_detail.large_load_failed").as_ref(),
                            );
                        }
                    }
                    cx.notify();
                });
            });
        });
        self._cell_content_chunk_tasks.insert(key, task);
    }

    fn set_large_value_error(&mut self, tab_id: TabId, row: usize, column: usize, message: &str) {
        if let Some(window) = self
            .large_value_windows
            .get_mut(&tab_id)
            .filter(|window| window.row == row && window.column == column)
        {
            window.view.loading = false;
            window.view.error = Some(message.to_string());
        }
    }

    /// 「读取更多」：接着窗口末尾读，读完（空块）就停。
    fn load_next_large_value_chunk(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(window) = self.large_value_windows.get(&tab_id) else {
            return;
        };
        if window.view.loading || window.view.is_complete() {
            return;
        }
        let (row, column, offset) = (
            window.row,
            window.column,
            window.view.next_read_offset(),
        );
        self.spawn_cell_content_chunk(tab_id, row, column, offset, cx);
    }

    /// 把完整内容流式写到用户选定位置；所选路径只用于写文件，不进日志与事件。
    fn save_large_value_to_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(window) = self.large_value_windows.get(&tab_id) else {
            return;
        };
        if self
            ._cell_content_save_tasks
            .contains_key(&(tab_id, window.row, window.column))
        {
            return;
        }
        let key = (tab_id, window.row, window.column);
        let content_id = window.view.content_id.clone();
        let extension = match window.view.kind {
            fluxdb_core::LargeValueKind::Json => "json",
            fluxdb_core::LargeValueKind::Binary => "bin",
            fluxdb_core::LargeValueKind::Text => "txt",
        };
        let suggested = format!("cell-{}-{}-{}.{extension}", tab_id.0, key.1 + 1, key.2 + 1);
        let receiver = cx.prompt_for_new_path(&default_data_export_directory(), Some(&suggested));
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => {
                    // 用户取消：不算失败，安静收尾。
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this._cell_content_save_tasks.remove(&key);
                            cx.notify();
                        });
                    });
                    return;
                }
                Ok(Err(error)) => {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this._cell_content_save_tasks.remove(&key);
                            this.show_message(
                                tr("app.cell_detail.large_save_choose_failed")
                                    .replace("%{error}", &error.to_string()),
                                AppMessageKind::Error,
                                cx,
                            );
                            cx.notify();
                        });
                    });
                    return;
                }
                Err(error) => {
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this._cell_content_save_tasks.remove(&key);
                            this.show_message(
                                tr("app.cell_detail.large_save_choose_failed")
                                    .replace("%{error}", &error.to_string()),
                                AppMessageKind::Error,
                                cx,
                            );
                            cx.notify();
                        });
                    });
                    return;
                }
            };
            let result = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::SaveCellContentToFile {
                        tab_id,
                        content_id,
                        path,
                    })
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._cell_content_save_tasks.remove(&key);
                    this.apply_app_event(&result, cx);
                    if let AppEvent::Failed(error) = &result {
                        this.show_message(error.message.clone(), AppMessageKind::Error, cx);
                    }
                    cx.notify();
                });
            });
        });
        self._cell_content_save_tasks.insert(key, task);
    }
}

/// 读取中的转圈图标。
///
/// 复用 `spinner.svg`，但动画 id 必须自成一份：侧边栏的加载动画也在这个 id 空间里，
/// 共用会让两处动画互相重启，表现为「转一下停一下」。
fn large_value_spinner(size: f32, color: gpui::Rgba) -> impl IntoElement {
    svg()
        .size(px(size))
        .path("spinner.svg")
        .text_color(color)
        .with_animation(
            "large_value_loading_spinner",
            Animation::new(Duration::from_secs_f64(0.75))
                .repeat()
                .with_easing(ease_in_out),
            |this, delta| this.with_transformation(Transformation::rotate(percentage(delta))),
        )
}

/// 大字段标题行里的状态条：总量、已读窗口、覆盖比例与加载状态。
///
/// 详情区高度紧张，全部压在「值」标题同一行：总量做成浅底小标签、窗口范围一段灰字、
/// 覆盖比例用细进度轨表达、状态用小色点 + 短词。比「完整大小 X 当前范围 Y / X 仅显示…」
/// 少一半字数，也不再把总量重复两遍。
const LARGE_STATUS_TRACK_WIDTH: f32 = 56.;

fn large_value_status(
    window: Option<&LargeValueWindows>,
    colors: UiColors,
) -> Option<gpui::AnyElement> {
    let view = &window?.view;
    let (state, tone) = match (&view.error, view.loading, view.is_complete()) {
        (Some(error), _, _) => (error.clone(), colors.danger),
        (None, true, _) => (
            tr("app.cell_detail.large_loading").to_string(),
            colors.info,
        ),
        (None, false, true) => (
            tr("app.cell_detail.large_complete").to_string(),
            colors.success,
        ),
        // 「仅片段」是大字段的常态，不是警告：用中性色，别让每次打开都像出了问题。
        (None, false, false) => (
            tr("app.cell_detail.large_partial").to_string(),
            colors.muted,
        ),
    };
    // 总量未知（库端拿不到大小）时不画进度轨，也不能拿窗口长度冒充总量。
    let total = view.byte_length.filter(|total| *total > 0);
    let window_range = view.window();

    let track = total.zip(window_range).map(|(total, (start, end))| {
        let scale = |bytes: u64| (bytes as f32 / total as f32).clamp(0., 1.) * LARGE_STATUS_TRACK_WIDTH;
        div()
            .flex_none()
            .w(px(LARGE_STATUS_TRACK_WIDTH))
            .h(px(4.))
            .rounded_full()
            .bg(colors.border_soft)
            // 进度轨固定用信息色：它表达「已读多少」，颜色变化留给右侧状态点。
            .child(
                div()
                    .ml(px(scale(start)))
                    // 50 MB 里只读了 256 KiB 时按比例只有 1px，肉眼等于没有；
                    // 给个最小可见宽度，保证「读过一点」看得出来。
                    .w(px(scale(end.saturating_sub(start)).max(2.)))
                    .h_full()
                    .rounded_full()
                    .bg(colors.info),
            )
    });
    let range = window_range.map(|(start, end)| {
        format!(
            "{} – {}",
            fluxdb_core::format_byte_length(start),
            fluxdb_core::format_byte_length(end)
        )
    });
    let range_tooltip = tr("app.cell_detail.large_range").replace("%{range}", &view.range_label());

    Some(
        div()
            .debug_selector(|| "large-value-status".into())
            .flex()
            .flex_1()
            .min_w(px(0.))
            .items_center()
            .gap_2()
            .when_some(total.map(fluxdb_core::format_byte_length), |this, total| {
                // 总量单独成标签：和后面的窗口范围一眼能分开，不写成「完整大小 X」的长句。
                // 标签只写数字，靠悬浮说明点明这是「完整大小」，省下标题行十几个字的宽度。
                this.child(
                    div()
                        .id("large-value-total")
                        .flex_none()
                        .h(px(16.))
                        .px_1p5()
                        .rounded(colors.radius * 0.5)
                        .bg(colors.panel_alt)
                        .text_size(ui_text_size(10.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .flex()
                        .items_center()
                        .tooltip({
                            let tip = tr("app.cell_detail.large_total_tip")
                                .replace("%{size}", &total);
                            move |window, cx| Tooltip::new(tip.clone()).build(window, cx)
                        })
                        .child(total),
                )
            })
            .when_some(track, |this, track| this.child(track))
            .when_some(range, |this, range| {
                this.child(
                    div()
                        .id("large-value-range")
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .tooltip(move |window, cx| {
                            Tooltip::new(range_tooltip.clone()).build(window, cx)
                        })
                        .child(range),
                )
            })
            .child(
                // 状态永远可见：范围可以被挤掉，但「加载中 / 仅片段 / 已到末尾 / 失败」不能被挤掉。
                div()
                    .flex_none()
                    .max_w(px(200.))
                    .flex()
                    .items_center()
                    .gap_1()
                    // 读取中把色点换成转圈：用户点完「读取更多」能立刻看到「在动」，
                    // 静态色点看不出请求是否还在进行。
                    .child(if view.loading {
                        div()
                            .flex_none()
                            .size(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(large_value_spinner(10., colors.info))
                            .into_any_element()
                    } else {
                        div()
                            .flex_none()
                            .size(px(6.))
                            .rounded_full()
                            .bg(tone)
                            .into_any_element()
                    })
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(ui_text_size(11.))
                            .text_color(colors.muted)
                            .child(state),
                    ),
            )
            .into_any_element(),
    )
}

/// 详情面板里的大字段分段只读区域：只有内容本身。
///
/// 不提供复制与格式化：窗口只是片段，拿它当完整值用等于丢数据。
fn large_value_window_view(
    window: Option<&LargeValueWindows>,
    colors: UiColors,
) -> Div {
    let border = if colors.is_dark {
        rgb(0x2f3642)
    } else {
        colors.border_soft
    };
    let bg = if colors.is_dark {
        rgb(0x13171d)
    } else {
        rgb(0xffffff)
    };

    let Some(window) = window else {
        return div()
            .debug_selector(|| "large-value-window".into())
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .child(div().text_size(ui_text_size(13.)).text_color(colors.muted).child(tr(
                "app.cell_detail.large_unavailable",
            )));
    };

    let window_text = window.view.window_text();

    div()
        .debug_selector(|| "large-value-window".into())
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .rounded(colors.radius * 0.5)
                .border_1()
                .border_color(border)
                .bg(bg)
                .px_2()
                .py_2()
                // 只给纵向滚动：横向滚动区会把可用宽度变成未定值，长内容就不换行了。
                .overflow_y_scrollbar()
                .text_size(ui_text_size(13.))
                .text_color(colors.text)
                .child(
                    div()
                        .debug_selector(|| "large-value-text".into())
                        .w_full()
                        .min_w(px(0.))
                        .whitespace_normal()
                        .child(window_text),
                ),
        )
}
