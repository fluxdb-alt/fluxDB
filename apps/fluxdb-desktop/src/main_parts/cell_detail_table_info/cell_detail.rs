fn cell_detail_panel(
    tab_id: TabId,
    editor: &DataEditorState,
    page: &DataPage,
    large_window: Option<&LargeValueWindows>,
    panel_height: f32,
    input: Entity<InputState>,
    temporal_part_input: Entity<InputState>,
    temporal_part_editing: Option<TemporalPartEditState>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(active_cell) = editor.cell_detail_panel.active_cell else {
        return div();
    };
    let Some(column) = page.columns.get(active_cell.column) else {
        return div();
    };
    let value = page
        .rows
        .get(active_cell.row)
        .and_then(|row| row.values.get(active_cell.column))
        .cloned()
        .unwrap_or(CellValue::Null);
    // 完整文本已按需加载时用它（复制/格式化/编辑都以完整值为准），否则显示带标记的预览。
    // 只在需要时克隆：`full_value` 可能是接近缓存上限的整串，每帧复制会拖慢面板。
    let value_text = editor
        .cell_detail_panel
        .full_value
        .clone()
        .unwrap_or_else(|| cell_value_label(&value));
    let is_large = matches!(value, CellValue::LargeValue(_));
    let is_json = matches!(value, CellValue::Json(_)) || looks_like_json(&value_text);
    let is_binary = matches!(value, CellValue::Bytes(_) | CellValue::BinarySummary(_));
    let row_number = page.offset + active_cell.row as u64 + 1;
    let type_name = column
        .type_name
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    // 上传替换按列类型放开：大二进制与大文本共用同一条入口（短文本类型也允许，
    // 但按钮只放在大字段/二进制上，避免普通字段的标题行挤满操作）。
    let can_upload = (is_binary || fluxdb_core::is_large_text_type_name(&type_name))
        && fluxdb_core::upload_replace_kind(column.type_name.as_deref(), &value).is_some();
    let type_color = data_type_color(&type_name).unwrap_or(colors.text);
    let length = cell_value_length(&value);
    let mode = editor.cell_detail_panel.mode;
    // 编辑态才需要草稿文本：只读态每帧克隆一份大文本没有意义。
    let detail_value = if mode == CellDetailMode::Edit {
        editor.cell_detail_panel.edit_value.clone()
    } else {
        String::new()
    };
    let temporal_kind = data_cell_temporal_kind(&type_name);

    if mode == CellDetailMode::Edit {
        sync_input_value(&input, &detail_value, window, cx);
    }

    let drawer_bg = if colors.is_dark {
        rgb(0x171b21)
    } else {
        rgb(0xfbfcfe)
    };
    let card_bg = if colors.is_dark {
        rgb(0x20252d)
    } else {
        rgb(0xffffff)
    };
    let field_bg = if colors.is_dark {
        rgb(0x13171d)
    } else {
        rgb(0xffffff)
    };
    let soft_border = if colors.is_dark {
        rgb(0x2f3642)
    } else {
        colors.border_soft
    };

    div()
        .h_full()
        .w_full()
        .flex_none()
        .border_t_1()
        .border_color(soft_border)
        .bg(drawer_bg)
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(cell_detail_drawer_resize_handle(
            tab_id,
            panel_height,
            colors,
            cx,
        ))
        .child(
            div()
                .h(px(54.))
                .w_full()
                .rounded(colors.radius * 0.5)
                .border_1()
                .border_color(soft_border)
                .bg(card_bg)
                .flex()
                .overflow_hidden()
                .child(cell_detail_meta_item(
                    tr("app.table_info.column_name"),
                    column.name.clone(),
                    colors,
                ))
                .child(cell_detail_meta_item(
                    tr("app.content.row_number"),
                    row_number.to_string(),
                    colors,
                ))
                .child(cell_detail_meta_item_colored(
                    tr("app.history.meta_type"),
                    type_name,
                    type_color,
                    colors,
                ))
                .child(cell_detail_meta_item(
                    "NULL",
                    matches!(value, CellValue::Null).to_string(),
                    colors,
                ))
                .child(cell_detail_meta_item(
                    tr("app.content.length"),
                    length.to_string(),
                    colors,
                ))
                .child(cell_detail_meta_item(
                    tr("app.table_info.comment"),
                    column
                        .comment
                        .clone()
                        .unwrap_or_else(|| tr("app.history.none").to_string()),
                    colors,
                ))
                .child(
                    div()
                        .w(px(42.))
                        .h_full()
                        .flex_none()
                        .flex()
                        .items_start()
                        .justify_center()
                        .pt_2()
                        .child(
                            cell_detail_icon_button(AppIcon::Close, tr("app.common.close"), colors)
                                .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::CloseCellDetail(tab_id), cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
        .child(cell_detail_value_header(
            tab_id,
            mode,
            value_text.clone(),
            is_json,
            is_binary,
            is_large,
            can_upload,
            large_window,
            colors,
            cx,
        ))
        .child(if mode == CellDetailMode::Edit {
            if let Some(kind) = temporal_kind {
                div().w(px(match kind {
                    DataCellTemporalKind::Date => 290.,
                    DataCellTemporalKind::Time => 252.,
                    DataCellTemporalKind::DateTime => 300.,
                }))
                .child(cell_detail_temporal_editor(
                    kind,
                    input.clone(),
                    temporal_part_input,
                    temporal_part_editing,
                    tab_id,
                    cx.entity().downgrade(),
                    column.nullable,
                    colors.is_dark,
                    detail_value,
                ))
            } else {
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .rounded(colors.radius * 0.5)
                    .border_1()
                    .border_color(soft_border)
                    .bg(field_bg)
                    .overflow_hidden()
                    .child(
                        Input::new(&input)
                            .appearance(false)
                            .focus_bordered(false)
                            .w_full()
                            .h_full()
                            .text_size(ui_text_size(13.)),
                    )
            }
        } else if is_large {
            // 大字段只能分段只读：窗口有上限，复制/格式化/编辑都禁用。
            large_value_window_view(large_window, colors)
        } else {
            cell_detail_value_preview(tab_id, &value, colors, cx)
        })
        .when(mode == CellDetailMode::Edit, |this| {
            this.child(cell_detail_footer(tab_id, colors, cx))
        })
}

fn cell_detail_drawer_resize_handle(
    tab_id: TabId,
    panel_height: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id("cell-detail-drawer-resize-handle")
        .h(px(8.))
        .mt(px(-6.))
        .mx(px(-8.))
        .cursor_ns_resize()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .h(px(1.))
                .w_full()
                .bg(colors.border_soft)
                .hover(|style| style.bg(rgb(0x3478f6))),
        )
        .hover(|style| style.bg(hsla(212. / 360., 0.92, 0.58, 0.10)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.cell_detail_drawer_resize_start = Some(CellDetailDrawerResizeStart {
                    tab_id,
                    y: f32::from(event.position.y),
                    height: panel_height,
                });
                this.sync_table_hover_overlay_block(cx);
                cx.stop_propagation();
            }),
        )
        .on_drag(CellDetailDrawerResizeDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<CellDetailDrawerResizeDrag>, _, cx| {
                if let Some(start) = this.cell_detail_drawer_resize_start {
                    if start.tab_id == tab_id {
                        let delta = start.y - f32::from(event.event.position.y);
                        this.cell_detail_drawer_heights.insert(
                            tab_id,
                            clamp_cell_detail_drawer_height(start.height + delta),
                        );
                        cx.notify();
                    }
                }
                cx.stop_propagation();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cell_detail_drawer_resize_start = None;
                cx.stop_propagation();
            }),
        )
}

fn clamp_cell_detail_drawer_height(height: f32) -> f32 {
    height.clamp(CELL_DETAIL_DRAWER_MIN_HEIGHT, CELL_DETAIL_DRAWER_MAX_HEIGHT)
}

#[cfg(test)]
mod cell_detail_drawer_height_tests {
    use super::*;

    #[test]
    fn minimum_height_keeps_edit_controls_and_value_area_visible() {
        let outer_padding = 16.;
        let flex_gaps = 4. * 8.;
        let fixed_rows = 8. + 54. + 22. + 26.;
        let minimum_value_area = 56.;

        assert!(
            CELL_DETAIL_DRAWER_MIN_HEIGHT
                >= outer_padding + flex_gaps + fixed_rows + minimum_value_area
        );
        assert_eq!(
            clamp_cell_detail_drawer_height(0.),
            CELL_DETAIL_DRAWER_MIN_HEIGHT
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn cell_detail_value_header(
    tab_id: TabId,
    mode: CellDetailMode,
    value_text: String,
    is_json: bool,
    is_binary: bool,
    is_large: bool,
    // 该列能否整值上传替换（大二进制/大文本）。
    can_upload: bool,
    large_window: Option<&LargeValueWindows>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 按钮回调要 `'static`，用 `Rc` 共享同一份文本，而不是给每个按钮各复制一遍。
    let value_text = std::rc::Rc::new(value_text);
    let copy_value_text = std::rc::Rc::clone(&value_text);
    let format_value_text = std::rc::Rc::clone(&value_text);
    // 大字段的大小/范围/完整性放在标题行里，不再单独占一行。
    let status = large_value_status(large_window, colors);

    div()
        .h(px(22.))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            // 左组必须让出空间：状态里的错误文案很长时，按钮不能被挤出详情面板。
            div()
                .flex()
                .flex_1()
                .items_center()
                .gap_2()
                .min_w(px(0.))
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(tr("app.placeholder.value")),
                )
                .when_some(status, |this, status| this.child(status)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(mode == CellDetailMode::View && is_large, |this| {
                    // 大字段不开放格式化与编辑：窗口只是片段，不是完整值。
                    // 操作按钮与其它类型统一放在「值」标题行，窗口内不再重复一排放底部。
                    // 未读完就一直留着「读取更多」：读取中把它换成转圈而不是让按钮消失，
                    // 否则点完一下按钮凭空不见，看不出是在读取还是出错了。
                    let can_load_more =
                        large_window.is_some_and(|window| !window.view.is_complete());
                    let load_more_loading =
                        large_window.is_some_and(|window| window.view.loading);
                    // 标题行空间紧张，大字段的操作统一用图标 + 悬浮说明。
                    this.child(
                        // 下载整份内容：用下载图标（箭头入托盘），
                        // 磁盘图标会被当成「保存改动」，`»` 会被当成「跳到末尾」。
                        cell_detail_icon_button(
                            AppIcon::Download,
                            tr("app.cell_detail.large_download_tip"),
                            colors,
                        )
                        .debug_selector(|| "large-value-save".into())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.save_large_value_to_file(tab_id, cx);
                                cx.stop_propagation();
                            }),
                        ),
                    )
                    .when(can_upload, |this| {
                        // 大字段的正文不在内存里，只能用文件整值替换。
                        this.child(
                            cell_detail_icon_button(
                                AppIcon::Upload,
                                tr("app.cell_detail.large_upload_tip"),
                                colors,
                            )
                            .debug_selector(|| "large-value-upload".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.upload_active_binary_cell(window, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                    })
                    .when(can_load_more, |this| {
                        let button = cell_detail_icon_button(
                            AppIcon::ChevronsDown,
                            if load_more_loading {
                                tr("app.cell_detail.large_loading")
                            } else {
                                tr("app.cell_detail.large_load_more_tip")
                            },
                            colors,
                        )
                        .debug_selector(|| "large-value-load-more".into());
                        // 加载中只转圈、不响应点击：重复点会再排一次读同一段的请求。
                        // 不能往原按钮里再塞一个子元素——那会和箭头并排显示成两个图标。
                        if load_more_loading {
                            this.child(
                                cell_detail_icon_spinner_button(
                                    tr("app.cell_detail.large_loading"),
                                    colors,
                                )
                                .debug_selector(|| "large-value-load-more".into()),
                            )
                        } else {
                            // 接着往下读：用向下的双箭头表示「继续加载」，
                            // `»` 在文件列表里是「跳到末尾」，和这里语义相反。
                            this.child(button.on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.load_next_large_value_chunk(tab_id, cx);
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ))
                        }
                    })
                })
                .when(mode == CellDetailMode::View && !is_large, |this| {
                    this.when(is_json, |this| {
                        this.child(
                            cell_detail_action_button(tr("app.cell_detail.format_json"), colors)
                                .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(formatted) = format_json_text(&format_value_text) {
                                        this.dispatch(AppCommand::StartCellDetailEdit(tab_id), cx);
                                        this.dispatch(
                                            AppCommand::UpdateCellDetailEditValue {
                                                tab_id,
                                                value: formatted,
                                            },
                                            cx,
                                        );
                                    }
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                    })
                    .when(is_binary, |this| {
                        this.child(cell_detail_action_button(tr("app.cell_detail.download"), colors).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.download_active_cell(cx);
                                cx.stop_propagation();
                            }),
                        ))
                        .child(cell_detail_action_button(tr("app.cell_detail.replace_upload"), colors).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.upload_active_binary_cell(window, cx);
                                cx.stop_propagation();
                            }),
                        ))
                        .child(
                            cell_detail_action_button(tr("app.cell_detail.set_null"), colors)
                                .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.set_active_binary_cell_null(tab_id, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                    })
                    // 文本大字段：值还没外置时也能用文件整值替换，不要求先撑到 64 KiB。
                    .when(can_upload && !is_binary, |this| {
                        this.child(
                            cell_detail_icon_button(
                                AppIcon::Upload,
                                tr("app.cell_detail.upload_tip"),
                                colors,
                            )
                            .debug_selector(|| "text-value-upload".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.upload_active_binary_cell(window, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                    })
                    .when(!is_binary, |this| {
                        this.child(
                            cell_detail_icon_button(
                                AppIcon::Edit,
                                tr("app.cell_detail.edit"),
                                colors,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::StartCellDetailEdit(tab_id), cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                    })
                    .child(
                        cell_detail_icon_button(AppIcon::Copy, tr("app.context.copy"), colors)
                            .debug_selector(|| "cell-detail-copy".into())
                            .on_mouse_down(
                            MouseButton::Left,
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    (*copy_value_text).clone(),
                                ));
                                cx.stop_propagation();
                            },
                        ),
                    )
                })
                .when(mode == CellDetailMode::Edit, |this| {
                    this.when(is_json, |this| {
                        this.child(
                            cell_detail_action_button(tr("app.cell_detail.format_json"), colors)
                                .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    let Some(text) = this.active_cell_detail_edit_value(cx) else {
                                        return;
                                    };
                                    if let Some(formatted) = format_json_text(&text) {
                                        this.dispatch(
                                            AppCommand::UpdateCellDetailEditValue {
                                                tab_id,
                                                value: formatted,
                                            },
                                            cx,
                                        );
                                    }
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                    })
                    .child(
                        cell_detail_action_button(tr("app.cell_detail.set_null"), colors)
                            .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                if is_binary {
                                    this.set_active_binary_cell_null(tab_id, cx);
                                } else {
                                    this.dispatch(AppCommand::SetCellDetailNull(tab_id), cx);
                                    this.refresh_active_data_table(tab_id, cx);
                                }
                                cx.stop_propagation();
                            }),
                        ),
                    )
                }),
        )
}

fn cell_detail_value_preview(
    tab_id: TabId,
    value: &CellValue,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let (text, text_color) = match value {
        CellValue::Bytes(bytes) => (
            tr("app.cell_detail.binary_hint")
                .replace("%{bytes}", &bytes.len().to_string()),
            colors.muted,
        ),
        CellValue::BinarySummary(summary) if summary.is_null => ("NULL".to_string(), colors.text),
        CellValue::BinarySummary(summary) => (
            summary
                .preview_hex
                .as_ref()
                .map(|preview| {
                    tr("app.cell_detail.hex_preview").replace("%{preview}", preview)
                })
                .unwrap_or_else(|| tr("app.cell_detail.no_hex_preview").to_string()),
            colors.muted,
        ),
        // 预览是片段：完整值还在后台加载时明确标出来，别让人以为已经看全。
        CellValue::TruncatedText { total_chars, .. } => (
            tr("app.cell_detail.truncated_loading")
                .replace("%{chars}", &total_chars.to_string()),
            colors.muted,
        ),
        _ => {
            (cell_value_label(value), colors.text)
        }
    };

    div()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(if colors.is_dark {
            rgb(0x2f3642)
        } else {
            colors.border_soft
        })
        .bg(if colors.is_dark {
            rgb(0x13171d)
        } else {
            rgb(0xffffff)
        })
        .px_2()
        .py_2()
        .overflow_hidden()
        .cursor_pointer()
        .flex()
        .items_start()
        .text_size(ui_text_size(13.))
        .text_color(text_color)
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::StartCellDetailEdit(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .size_full()
                // 只给纵向滚动：横向滚动区会把可用宽度变成未定值，长内容就不换行了。
                .overflow_y_scrollbar()
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .whitespace_normal()
                        .child(text),
                ),
        )
}

fn cell_detail_footer(
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(26.))
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .child(cell_detail_primary_button(tr("app.common.save"), colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                // 二进制不进编辑态（见 `StartCellDetailEdit`），这里只会是普通值。
                this.dispatch(AppCommand::SaveCellDetailEdit(tab_id), cx);
                this.refresh_active_data_table(tab_id, cx);
                cx.stop_propagation();
            }),
        ))
        .child(cell_detail_action_button(tr("app.common.cancel"), colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::CancelCellDetailEdit(tab_id), cx);
                cx.stop_propagation();
            }),
        ))
        .child(cell_detail_action_button(tr("app.cell_detail.restore_original"), colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::RestoreCellDetailOriginalValue(tab_id), cx);
                cx.stop_propagation();
            }),
        ))
}

fn cell_detail_meta_item(
    label: impl Into<SharedString>,
    value: String,
    colors: UiColors,
) -> Div {
    cell_detail_meta_item_colored(label, value, colors.text, colors)
}

fn cell_detail_meta_item_colored(
    label: impl Into<SharedString>,
    value: String,
    value_color: gpui::Rgba,
    colors: UiColors,
) -> Div {
    let label = label.into();
    div()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .px_3()
        .py_1()
        .flex()
        .flex_col()
        .justify_center()
        .gap_1()
        .text_size(ui_text_size(12.))
        .child(div().text_color(colors.muted).child(label))
        .child(
            div()
                .text_color(value_color)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(value),
        )
}

fn cell_detail_icon_button(
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    colors: UiColors,
) -> Stateful<Div> {
    let tooltip = tooltip.into();
    div()
        .id(tooltip.clone())
        .size(px(22.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(app_icon(icon, 15., colors.muted))
}

/// 与 `cell_detail_icon_button` 同一尺寸、同一悬浮底的「读取中」占位按钮。
///
/// 只用来显示进度，不绑定点击：读取中的按钮既不能消失也不能重复触发。
fn cell_detail_icon_spinner_button(tooltip: impl Into<SharedString>, colors: UiColors) -> Stateful<Div> {
    let tooltip = tooltip.into();
    div()
        .id(tooltip.clone())
        .size(px(22.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(large_value_spinner(15., colors.muted))
}

fn cell_detail_action_button(label: impl Into<SharedString>, colors: UiColors) -> Div {
    let label = label.into();
    div()
        .h(px(24.))
        .px_2()
        .rounded(colors.radius * 0.5)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(ui_text_size(12.))
        .text_color(colors.text)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn cell_detail_primary_button(label: impl Into<SharedString>, colors: UiColors) -> Div {
    cell_detail_action_button(label, colors)
        .border_color(if colors.is_dark {
            rgb(0x2f75ff)
        } else {
            rgb(0x2c6bed)
        })
        .bg(if colors.is_dark {
            rgb(0x173766)
        } else {
            rgb(0xe7f0ff)
        })
        .text_color(if colors.is_dark {
            rgb(0xa9c9ff)
        } else {
            rgb(0x1757bd)
        })
}
