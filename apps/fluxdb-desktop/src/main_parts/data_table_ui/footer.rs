fn data_editor_footer(
    tab_id: TabId,
    page: &DataPage,
    sql: String,
    selection: SqlTextSelection,
    page_input: Entity<InputState>,
    change_count: Option<usize>,
    change_sql_preview_open: bool,
    selected_source_row: Option<usize>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let page_size = data_page_limit(page.limit);
    let page_no = data_page_number(page.offset, page_size);
    let can_previous = page.offset > 0;
    let can_next = page.has_more && page_no < DATA_EDITOR_MAX_PAGE;
    let can_last = can_next;
    let previous_offset = page.offset.saturating_sub(page_size);
    let next_offset = page.offset.saturating_add(page_size);
    let last_offset = data_page_offset_for_supported_page(DATA_EDITOR_MAX_PAGE, page_size);
    let has_changes = change_count.is_some_and(|count| count > 0);

    div()
        .relative()
        .occlude()
        .h(px(30.))
        .w_full()
        .flex_none()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .pl_0()
        .pr_2()
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(
            div()
                .flex()
                .items_center()
                .flex_none()
                .child(
                    data_editor_footer_icon_button(AppIcon::Plus, "新增行", true, colors)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.dispatch(
                                    AppCommand::InsertDataRow {
                                        tab_id,
                                        result_index: None,
                                        after_row: None,
                                    },
                                    cx,
                                );
                                this.refresh_active_data_table(tab_id, cx);
                                cx.stop_propagation();
                            }),
                        ),
                )
                .child(
                    data_editor_footer_icon_button(
                        AppIcon::Minus,
                        "删除行",
                        selected_source_row.is_some(),
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(row) = selected_source_row {
                                this.dispatch(
                                    AppCommand::DeleteDataRow {
                                        tab_id,
                                        result_index: None,
                                        row,
                                    },
                                    cx,
                                );
                                this.refresh_active_data_table(tab_id, cx);
                            }
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(
                    data_editor_footer_icon_button(AppIcon::Check, "提交更改", has_changes, colors)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                if has_changes {
                                    this.request_apply_data_changes(tab_id, cx);
                                }
                                cx.stop_propagation();
                            }),
                        ),
                )
                .child(
                    data_editor_footer_icon_button(AppIcon::Close, "取消更改", has_changes, colors)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                if has_changes {
                                    this.dispatch(AppCommand::DiscardDataChanges(tab_id), cx);
                                    this.data_change_sql_preview_tabs.remove(&tab_id);
                                    if this.pending_apply_data_changes == Some(tab_id) {
                                        this.pending_apply_data_changes = None;
                                    }
                                    this.refresh_active_data_table(tab_id, cx);
                                }
                                cx.stop_propagation();
                            }),
                        ),
                )
                .child(
                    data_editor_footer_icon_button(AppIcon::Refresh, "刷新", true, colors)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.request_data_editor_refresh(tab_id, cx);
                                cx.stop_propagation();
                            }),
                        ),
                )
                .child(data_editor_footer_icon_button(
                    AppIcon::Square,
                    "停止",
                    false,
                    colors,
                )),
        )
        .child(if let Some(count) = change_count {
            data_editor_change_status(tab_id, count, change_sql_preview_open, colors, cx)
        } else {
            data_editor_sql_strip(sql, selection, colors, cx)
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .flex_none()
                .child(
                    data_editor_page_button("⏮", can_previous, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if can_previous {
                                this.sync_data_page_input_value(0, page_size, window, cx);
                                this.request_data_editor_pagination(tab_id, 0, page_size, cx);
                            }
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(
                    data_editor_page_button("‹", can_previous, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if can_previous {
                                this.sync_data_page_input_value(
                                    previous_offset,
                                    page_size,
                                    window,
                                    cx,
                                );
                                this.request_data_editor_pagination(
                                    tab_id,
                                    previous_offset,
                                    page_size,
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(data_editor_page_input(
                    page_no, page_input, colors, window, cx,
                ))
                .child(
                    data_editor_page_button("›", can_next, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if can_next {
                                this.sync_data_page_input_value(next_offset, page_size, window, cx);
                                this.request_data_editor_pagination(
                                    tab_id,
                                    next_offset,
                                    page_size,
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(
                    data_editor_page_button("⏭", can_last, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if can_last {
                                this.sync_data_page_input_value(last_offset, page_size, window, cx);
                                this.request_data_editor_pagination(
                                    tab_id,
                                    last_offset,
                                    page_size,
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
}

const DATA_EDITOR_MAX_PAGE: u64 = 100;

fn data_page_limit(limit: u64) -> u64 {
    limit.max(1)
}

fn data_page_number(offset: u64, limit: u64) -> u64 {
    offset / data_page_limit(limit) + 1
}

fn data_page_offset_for_page(page_no: u64, limit: u64) -> u64 {
    page_no
        .saturating_sub(1)
        .saturating_mul(data_page_limit(limit))
}

fn data_page_supported_number(page_no: u64) -> u64 {
    page_no.clamp(1, DATA_EDITOR_MAX_PAGE)
}

fn data_page_offset_for_supported_page(page_no: u64, limit: u64) -> u64 {
    data_page_offset_for_page(data_page_supported_number(page_no), limit)
}

fn data_editor_page_input(
    page_no: u64,
    input: Entity<InputState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let expected = page_no.to_string();
    let input_focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !input_focused && input.read(cx).value().to_string() != expected {
        input.update(cx, |input, cx| {
            input.set_value(expected.clone(), window, cx);
        });
    }

    div()
        .h(px(22.))
        .w(px(48.))
        .rounded(colors.radius * 0.5)
        .bg(colors.input_bg)
        .border_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .justify_center()
        .cursor_text()
        .text_color(colors.text)
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .px_0()
                .text_size(px(12.))
                .text_color(colors.text),
        )
}

fn data_editor_change_status(
    tab_id: TabId,
    count: usize,
    open: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let accent = if colors.is_dark {
        rgb(0x8ab4ff)
    } else {
        rgb(0x1a73e8)
    };
    div()
        .h(px(26.))
        .flex_1()
        .min_w(px(0.))
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(if open { accent } else { colors.border_soft })
        .bg(if open {
            if colors.is_dark {
                rgb(0x182235)
            } else {
                rgb(0xeaf2ff)
            }
        } else if colors.is_dark {
            rgb(0x111418)
        } else {
            rgb(0xf8fafc)
        })
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .overflow_hidden()
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(if open { accent } else { colors.text })
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon(
            AppIcon::List,
            14.,
            if open { accent } else { colors.muted },
        ))
        .child(
            div()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(format!("待提交 {count} 处修改")),
        )
        .child(
            div()
                .ml_auto()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(if open { "收起" } else { "查看 SQL" }),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_data_change_sql_preview(tab_id, cx);
                cx.stop_propagation();
            }),
        )
}

fn data_change_sql_preview_drawer(
    tab_id: TabId,
    sql: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql_for_copy = sql.to_string();
    let mut sql_lines = div()
        .flex()
        .flex_col()
        .gap_1()
        .font_family("Menlo")
        .text_size(px(11.))
        .line_height(px(17.))
        .text_color(colors.text);
    for line in sql.lines() {
        sql_lines = sql_lines.child(div().child(line.to_string()));
    }

    div()
        .occlude()
        .border_t_1()
        .border_color(colors.border)
        .bg(if colors.is_dark {
            rgb(0x101418)
        } else {
            rgb(0xf7f9fc)
        })
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .h(px(32.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(colors.border_soft)
                .text_size(px(12.))
                .text_color(colors.text)
                .child(app_icon(AppIcon::List, 14., colors.muted))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("待提交 SQL"),
                )
                .child(div().flex_1())
                .child(
                    data_editor_footer_icon_button(AppIcon::Copy, "复制待提交 SQL", true, colors)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    sql_for_copy.clone(),
                                ));
                                this.show_message("已复制待提交 SQL", AppMessageKind::Success, cx);
                                cx.stop_propagation();
                            }),
                        ),
                )
                .child(
                    data_editor_footer_icon_button(AppIcon::Close, "收起", true, colors)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_data_change_sql_preview(tab_id, cx);
                                cx.stop_propagation();
                            }),
                        ),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .px_3()
                .py_2()
                .child(sql_lines),
        )
}

fn data_search_bar(
    tab_id: TabId,
    input: Entity<InputState>,
    matches: &[DataSearchMatch],
    active_match: Option<DataSearchMatch>,
    highlight_all: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let match_count = matches.len();
    let match_label = data_search_match_label(matches, active_match);
    div()
        .relative()
        .occlude()
        .h(px(30.))
        .w_full()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .text_color(colors.text)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .size(px(20.))
                .rounded(colors.radius * 0.5)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(15.))
                .text_color(colors.muted)
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 14., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.close_data_search_panel(tab_id, window, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            div()
                .h(px(22.))
                .w(px(520.))
                .rounded(colors.radius * 0.5)
                .border_1()
                .border_color(colors.border_soft)
                .bg(colors.input_bg)
                .px_2()
                .flex()
                .items_center()
                .gap_1()
                .child(app_icon(AppIcon::Search, 13., colors.muted))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child("查找数据:"),
                )
                .child(
                    Input::new(&input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(12.)),
                ),
        )
        .child(
            data_search_button("下一个", match_count > 0, false, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.select_next_data_search_match(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_search_button(match_label, match_count > 0, highlight_all, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if match_count > 0 {
                        this.toggle_data_search_highlight_all(tab_id, cx);
                    }
                    cx.stop_propagation();
                }),
            ),
        )
}

fn data_search_button(
    label: impl Into<SharedString>,
    enabled: bool,
    active: bool,
    colors: UiColors,
) -> Div {
    let label = label.into();
    let active_border = if colors.is_dark {
        rgb(0x5b8cff)
    } else {
        rgb(0x2f6fed)
    };
    let active_bg = if colors.is_dark {
        rgb(0x1f2d46)
    } else {
        rgb(0xe7f0ff)
    };
    div()
        .h(px(22.))
        .px_3()
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(if active && enabled {
            active_border
        } else {
            colors.border
        })
        .bg(if active && enabled {
            active_bg
        } else {
            colors.panel_alt
        })
        .text_size(px(12.))
        .text_color(if active && enabled {
            active_border
        } else if enabled {
            colors.text
        } else {
            colors.muted
        })
        .opacity(if enabled { 1.0 } else { 0.45 })
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .child(label)
        .hover(move |style| {
            if enabled {
                style.bg(colors.hover)
            } else {
                style
            }
        })
}

fn data_editor_sql_input(
    sql: String,
    input: Entity<InputState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let input_focused = input.read(cx).focus_handle(cx).is_focused(window);
    if !input_focused && input.read(cx).value().to_string() != sql {
        input.update(cx, |input, cx| {
            input.set_value(sql.clone(), window, cx);
        });
    }
    let input_for_copy = input.clone();

    div()
        .h(px(26.))
        .flex_1()
        .min_w(px(0.))
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(colors.border_soft)
        .bg(if colors.is_dark {
            rgb(0x111418)
        } else {
            rgb(0xf8fafc)
        })
        .px_3()
        .flex()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(
            data_editor_footer_button("⧉", "复制 SQL", true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        input_for_copy.read(cx).value().to_string(),
                    ));
                    this.show_message("已复制 SQL", AppMessageKind::Success, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .text_size(px(12.))
                .font_family("Menlo"),
        )
}

fn data_editor_sql_strip(
    sql: String,
    selection: SqlTextSelection,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sql_for_copy = sql.clone();
    let sql_for_down = sql.clone();
    let sql_for_move = sql.clone();
    let sql_for_bounds = sql.clone();
    let sql_for_canvas = sql.clone();
    let view = cx.entity().downgrade();
    let selection_for_canvas = selection.clone();

    div()
        .h(px(26.))
        .flex_1()
        .min_w(px(0.))
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(colors.border_soft)
        .bg(if colors.is_dark {
            rgb(0x111418)
        } else {
            rgb(0xf8fafc)
        })
        .px_3()
        .flex()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(
            data_editor_footer_button("⧉", "复制 SQL", true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(sql_for_copy.clone()));
                    this.show_message("已复制 SQL", AppMessageKind::Success, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .font_family("Menlo")
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .child(
                            canvas(
                                move |bounds, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        let mut changed = false;
                                        if this.data_sql_footer_selection.text != sql_for_bounds {
                                            this.data_sql_footer_selection.text =
                                                sql_for_bounds.clone();
                                            this.data_sql_footer_selection.anchor = 0;
                                            this.data_sql_footer_selection.cursor = 0;
                                            this.data_sql_footer_selection.selecting = false;
                                            changed = true;
                                        }
                                        if this.data_sql_footer_selection.bounds.as_ref()
                                            != Some(&bounds)
                                        {
                                            this.data_sql_footer_selection.bounds =
                                                Some(bounds.clone());
                                            changed = true;
                                        }
                                        if changed {
                                            cx.notify();
                                        }
                                    });
                                    bounds
                                },
                                move |bounds, _, window, _| {
                                    if let Some((start, end)) =
                                        selection_for_canvas.selected_range()
                                    {
                                        let start = sql_prefix_char_count(&sql_for_canvas, start);
                                        let end = sql_prefix_char_count(&sql_for_canvas, end);
                                        let x = bounds.left() + sql_selection_char_width() * start;
                                        let width = sql_selection_char_width() * (end - start);
                                        window.paint_quad(gpui::fill(
                                            Bounds::new(
                                                point(x, bounds.top()),
                                                size(width, bounds.size.height),
                                            ),
                                            hsla(211. / 360., 0.83, 0.52, 0.35),
                                        ));
                                    }
                                },
                            )
                            .size_full(),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(sql),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        if event.click_count >= 2 {
                            this.select_all_footer_sql(&sql_for_down, cx);
                        } else {
                            this.start_footer_sql_selection(&sql_for_down, event.position, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if event.dragging() {
                        this.update_footer_sql_selection(&sql_for_move, event.position, cx);
                        cx.stop_propagation();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        this.finish_footer_sql_selection(cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn data_editor_footer_button(
    label: &'static str,
    tooltip: &'static str,
    enabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(tooltip)
        .size(px(24.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(15.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(if enabled { colors.text } else { colors.muted })
        .hover(move |style| {
            style
                .bg(colors.hover)
                .text_color(if enabled { colors.text } else { colors.muted })
        })
        .when(enabled, |this| this.cursor_pointer())
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(label)
}

fn data_editor_footer_icon_button(
    icon: AppIcon,
    tooltip: &'static str,
    enabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let icon_color = if enabled { colors.text } else { colors.muted };
    div()
        .id(tooltip)
        .size(px(24.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .opacity(if enabled { 1.0 } else { 0.48 })
        .hover(move |style| {
            if enabled {
                style.bg(colors.hover)
            } else {
                style
            }
        })
        .when(enabled, |this| this.cursor_pointer())
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(app_icon(icon, 15., icon_color))
}

fn data_editor_page_button(label: &'static str, enabled: bool, colors: UiColors) -> Div {
    div()
        .size(px(22.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
        })
        .child(label)
}

fn center_message(text: impl Into<String>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(15.))
        .text_color(colors.muted)
        .child(text.into())
}

fn center_loading_message(text: impl Into<String>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(px(15.))
        .text_color(colors.muted)
        .child(loading_spinner_with_color(20., colors.muted))
        .child(text.into())
}
