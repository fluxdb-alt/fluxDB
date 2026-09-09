fn data_cell_temporal_kind(type_name: &str) -> Option<DataCellTemporalKind> {
    let normalized = type_name
        .trim()
        .to_ascii_lowercase()
        .split(|ch: char| ch == '(' || ch.is_whitespace())
        .next()
        .unwrap_or_default()
        .to_string();

    match normalized.as_str() {
        "timestamp" | "datetime" | "timestamptz" | "datetime2" => {
            Some(DataCellTemporalKind::DateTime)
        }
        "date" => Some(DataCellTemporalKind::Date),
        "time" => Some(DataCellTemporalKind::Time),
        _ => None,
    }
}

fn parse_temporal_date_part(value: &str) -> Option<NaiveDate> {
    let date = value.trim().get(0..10)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

fn temporal_time_part(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((_, time)) = trimmed.split_once(' ') {
        return Some(time.to_string());
    }
    if let Some((_, time)) = trimmed.split_once('T') {
        return Some(time.to_string());
    }
    if trimmed.contains(':') {
        return Some(trimmed.to_string());
    }
    None
}

fn redis_type_tag(label: String, dark: bool) -> Div {
    let (text, bg, border) = redis_type_tag_colors(label.as_str(), dark);
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(22.))
        .min_w(px(54.))
        .max_w(px(84.))
        .px_2()
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(text)
        .overflow_hidden()
        .text_ellipsis()
        .child(label)
}

fn redis_type_tag_colors(kind: &str, dark: bool) -> (gpui::Rgba, Hsla, gpui::Rgba) {
    let (text, light_bg, dark_bg) = match kind.to_ascii_lowercase().as_str() {
        "string" => (rgb(0x16a34a), hsla(142. / 360., 0.72, 0.94, 1.0), 0x143323),
        "list" => (rgb(0x2563eb), hsla(217. / 360., 0.87, 0.95, 1.0), 0x172b4d),
        "set" => (rgb(0x9333ea), hsla(270. / 360., 0.76, 0.95, 1.0), 0x2b1746),
        "zset" => (rgb(0xc2410c), hsla(24. / 360., 0.90, 0.95, 1.0), 0x3a2114),
        "hash" => (rgb(0x0f766e), hsla(173. / 360., 0.58, 0.93, 1.0), 0x123433),
        "stream" => (rgb(0x0891b2), hsla(192. / 360., 0.74, 0.93, 1.0), 0x12323d),
        "json" => (rgb(0xdb2777), hsla(330. / 360., 0.81, 0.95, 1.0), 0x3b1728),
        _ => (rgb(0x64748b), hsla(210. / 360., 0.16, 0.93, 1.0), 0x262b33),
    };
    let bg = if dark { rgb(dark_bg).into() } else { light_bg };
    (text, bg, text)
}

fn redis_action_cell_buttons(
    tab_id: TabId,
    source_row: usize,
    view: WeakEntity<NavicatMain>,
    deleted: bool,
    dark: bool,
    radius: gpui::Pixels,
) -> Div {
    div()
        .flex()
        .w_full()
        .items_center()
        .justify_between()
        .child(redis_action_button("修改", AppIcon::Edit, false, dark, radius))
        .child(
            redis_action_button("删除", AppIcon::Trash, !deleted, dark, radius).when(!deleted, |this| {
                this.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.dispatch(
                            AppCommand::DeleteDataRow {
                                tab_id,
                                result_index: None,
                                row: source_row,
                            },
                            cx,
                        );
                        this.refresh_active_data_table(tab_id, cx);
                    });
                    cx.stop_propagation();
                })
            }),
        )
}

fn redis_action_button(
    label: &'static str,
    icon: AppIcon,
    enabled: bool,
    dark: bool,
    radius: gpui::Pixels,
) -> Div {
    let text_color = if enabled {
        rgb(0x1677ff)
    } else if dark {
        rgb(0x6f7785)
    } else {
        rgb(0x9aa3af)
    };
    div()
        .h(px(24.))
        .px_2()
        .rounded(radius)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(text_color)
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| {
                style.bg(if dark { rgb(0x1d2b3d) } else { rgb(0xeaf3ff) })
            })
        })
        .child(app_icon(icon, 13., text_color))
        .child(label)
}

fn temporal_date_part(value: &str) -> Option<String> {
    parse_temporal_date_part(value).map(|date| date.format("%Y-%m-%d").to_string())
}

fn replace_temporal_date_part(
    current: &str,
    kind: Option<DataCellTemporalKind>,
    date: NaiveDate,
) -> String {
    match kind {
        Some(DataCellTemporalKind::Date) => date.format("%Y-%m-%d").to_string(),
        Some(DataCellTemporalKind::DateTime) => format!(
            "{} {}",
            date.format("%Y-%m-%d"),
            temporal_time_part(current).unwrap_or_else(|| "00:00:00".to_string())
        ),
        _ => current.to_string(),
    }
}

fn replace_temporal_time_part(
    current: &str,
    kind: Option<DataCellTemporalKind>,
    time: &str,
) -> String {
    match kind {
        Some(DataCellTemporalKind::Time) => time.to_string(),
        Some(DataCellTemporalKind::DateTime) => format!(
            "{} {}",
            temporal_date_part(current).unwrap_or_else(|| "1970-01-01".to_string()),
            time
        ),
        _ => current.to_string(),
    }
}

fn temporal_days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .expect("next month first day should be valid");
    (next - ChronoDuration::days(1)).day()
}

fn temporal_shift_month(date: NaiveDate, delta: i32) -> NaiveDate {
    let month_index = date.year() * 12 + date.month() as i32 - 1 + delta;
    let year = month_index.div_euclid(12);
    let month = month_index.rem_euclid(12) as u32 + 1;
    let day = date.day().min(temporal_days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).expect("shifted date should be valid")
}

fn data_cell_edit_matches_table(
    editing: &DataCellEditState,
    tab_id: TabId,
    query_result_page_index: Option<usize>,
    row_ix: usize,
    col_ix: usize,
) -> bool {
    editing.tab_id == tab_id
        && editing.query_result_page_index == query_result_page_index
        && editing.visible_row == row_ix
        && editing.col_ix == col_ix
}

impl TableDelegate for DataPageTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        self.columns[col_ix].clone()
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let column = &self.columns[col_ix];
        let type_name = self.column_types.get(col_ix).cloned().unwrap_or_default();
        let meta = self
            .column_meta
            .get(col_ix)
            .cloned()
            .unwrap_or_else(|| DataTableColumnMeta {
                name: column.name.to_string(),
                type_name: type_name.clone(),
                nullable: false,
                primary_key: false,
                choices: Vec::new(),
            });
        let type_color =
            data_type_color(type_name.as_str()).unwrap_or(cx.theme().muted_foreground.into());
        let sort = self
            .sorts
            .iter()
            .find(|sort| sort.col_ix == col_ix)
            .copied();
        let sort_icon = match sort.map(|sort| sort.direction) {
            Some(DataTableSortDirection::Ascending) => "↑",
            Some(DataTableSortDirection::Descending) => "↓",
            None => "↕",
        };
        let column_name = column.name.to_string();
        let copy_column_name = column_name.clone();
        let action_column_name = column_name.clone();
        let tooltip_meta = meta.clone();
        let tab_id = self.tab_id;
        let on_sort = self.on_sort.clone();
        let view = self.view.clone();
        let table_entity = cx.entity();
        let header_icon_color = data_table_header_icon_color(cx.theme().is_dark());
        if self.show_row_index && col_ix == 0 {
            return div().relative().size_full();
        }
        let highlighted = self
            .highlighted_column
            .as_ref()
            .is_some_and(|name| column.key.as_ref() == name);

        div()
            .relative()
            .size_full()
            .bg(if highlighted {
                cx.theme().table_hover
            } else {
                cx.theme().transparent
            })
            .flex()
            .items_center()
            .gap_1()
            .when(self.redis_page, |this| this.px_3())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .id(("data-table-header-text", col_ix))
                    .tooltip(move |window, cx| {
                        data_table_header_tooltip(tooltip_meta.clone(), type_color)
                            .build(window, cx)
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(column.name.clone()),
                    )
                    .when(self.header_actions && !type_name.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(px(9.))
                                .text_color(type_color)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(type_name.clone()),
                        )
                    }),
            )
            .when((!self.show_row_index || col_ix > 0) && self.header_actions, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_0p5()
                        .flex_none()
                        .child(
                            data_table_header_text_button(
                                ("copy-data-column", col_ix),
                                "⧉",
                                "复制列名",
                                data_table_header_icon_color(cx.theme().is_dark()),
                            )
                            .on_click(move |_, _window, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    copy_column_name.clone(),
                                ));
                                let _ = view.update(cx, |this, cx| {
                                    this.show_message("已复制", AppMessageKind::Success, cx);
                                });
                                cx.stop_propagation();
                            }),
                        )
                        .child(data_table_header_sort_popover(
                            tab_id,
                            col_ix,
                            column_name.clone(),
                            sort_icon,
                            sort.map(|sort| sort.direction),
                            on_sort,
                            header_icon_color,
                        ))
                        .child(data_table_header_action_popover(
                            tab_id,
                            col_ix,
                            action_column_name,
                            self.view.clone(),
                            table_entity,
                            header_icon_color,
                        )),
                )
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let is_row_index_col = self.show_row_index && col_ix == 0;
        let row_value_col_ix = if self.show_row_index {
            col_ix.saturating_sub(1)
        } else {
            col_ix
        };
        let text: SharedString = if is_row_index_col {
            SharedString::from((row_ix + 1).to_string())
        } else {
            self.rows
                .get(row_ix)
                .and_then(|row| row.get(row_value_col_ix))
                .cloned()
                .unwrap_or_default()
        };
        let source_row_ix = self
            .source_row_indexes
            .get(row_ix)
            .copied()
            .unwrap_or(row_ix);
        let source_col_ix = self
            .source_column_indexes
            .get(col_ix)
            .and_then(|index| *index)
            .unwrap_or(row_value_col_ix);
        let null_cell =
            !is_row_index_col && self.null_cells.contains(&(source_row_ix, source_col_ix));
        let dirty_cell =
            !is_row_index_col && self.dirty_cells.contains(&(source_row_ix, source_col_ix));
        let deleted_row = self.deleted_rows.contains(&source_row_ix);
        let editing_cell = self.editing_cell.as_ref().is_some_and(|editing| {
            data_cell_edit_matches_table(
                editing,
                self.tab_id,
                self.query_result_page_index,
                row_ix,
                col_ix,
            )
        });
        let search_match = self
            .search_matches
            .iter()
            .any(|search_match| search_match.row_ix == row_ix && search_match.col_ix == col_ix);
        let active_search = self
            .active_search_match
            .is_some_and(|active| active.row_ix == row_ix && active.col_ix == col_ix);
        let selected_cell = self.has_cell_selected(row_ix, col_ix) || active_search;
        let selected_row = self.has_row_selected(row_ix);
        let hovered = !is_row_index_col
            && self
                .source_column_indexes
                .get(col_ix)
                .and_then(|index| *index)
                .is_some()
            && self.hovered_cell == Some((row_ix, col_ix));
        let highlighted = self.highlighted_column.as_ref().is_some_and(|name| {
            !is_row_index_col
                && self
                    .columns
                    .get(col_ix)
                    .is_some_and(|column| column.key.as_ref() == name)
        });
        let cell_bg: Hsla = if cx.theme().is_dark() {
            rgb(0x222832).into()
        } else {
            rgb(0xdce8f8).into()
        };
        let cell_border: Hsla = if cx.theme().is_dark() {
            rgb(0xa0a8b8).into()
        } else {
            rgb(0x5f7fb8).into()
        };
        let dirty_bg: Hsla = if cx.theme().is_dark() {
            hsla(42. / 360., 0.45, 0.24, 0.64)
        } else {
            hsla(44. / 360., 0.90, 0.82, 0.68)
        };
        let deleted_bg: Hsla = if cx.theme().is_dark() {
            hsla(0. / 360., 0.55, 0.22, 0.52)
        } else {
            hsla(0. / 360., 0.82, 0.88, 0.76)
        };
        let deleted_line = if cx.theme().is_dark() {
            rgb(0xc45d5d)
        } else {
            rgb(0xb74343)
        };
        let edit_input = self.edit_input.clone();
        let tab_id = self.tab_id;
        let cells_editable = self.cells_editable;
        let view_for_edit = self.view.clone();
        let view_for_menu = self.view.clone();
        let view_for_info = self.view.clone();
        let edit_value = if null_cell {
            String::new()
        } else {
            text.to_string()
        };
        let meta_for_menu =
            self.column_meta
                .get(col_ix)
                .cloned()
                .unwrap_or_else(|| DataTableColumnMeta {
                    name: String::new(),
                    type_name: String::new(),
                    nullable: false,
                    primary_key: false,
                    choices: Vec::new(),
                });
        let temporal_kind = data_cell_temporal_kind(&meta_for_menu.type_name);
        let editor_kind = data_cell_editor_kind(&meta_for_menu.type_name);
        let choice_picker = !data_cell_choice_options(editor_kind, &meta_for_menu).is_empty();
        let choice_label = (!null_cell)
            .then(|| {
                meta_for_menu
                    .choices
                    .iter()
                    .find(|choice| choice.value == text.as_ref())
                    .map(|choice| choice.label.trim().to_string())
            })
            .flatten()
            .filter(|label| !label.is_empty());
        let redis_type_cell = !is_row_index_col
            && meta_for_menu.type_name == "redis"
            && meta_for_menu.name == "类型"
            && !text.is_empty();
        let redis_action_cell = meta_for_menu.type_name == "redis_action";
        let redis_cell = meta_for_menu.type_name == "redis" || redis_action_cell;
        let redis_row_selection = self.redis_page && selected_row;
        let redis_type_dark = cx.theme().is_dark();
        let editable_cell = self.cells_editable
            && !is_row_index_col
            && !redis_cell
            && !deleted_row
            && !data_type_is_binary(meta_for_menu.type_name.as_str());
        let editing_input_value = edit_input.read(cx).value().to_string();
        let edit_state = DataCellEditState {
            tab_id,
            query_result_page_index: self.query_result_page_index,
            visible_row: row_ix,
            source_row: source_row_ix,
            col_ix,
            source_col: source_col_ix,
            temporal_kind,
        };
        let context_menu = DataCellContextMenu {
            tab_id,
            position: point(px(0.), px(0.)),
            source_row: source_row_ix,
            query_result_page_index: self.query_result_page_index,
            source_col: source_col_ix,
            column_name: meta_for_menu.name.clone(),
            type_name: meta_for_menu.type_name.clone(),
            nullable: meta_for_menu.nullable,
            value: text.to_string(),
            submenu: None,
            selection_row_count: 0,
            selection_copy_label: None,
            selection_export_label: None,
        };
        let row_context_menu = DataRowContextMenu {
            tab_id,
            position: point(px(0.), px(0.)),
            source_row: source_row_ix,
            query_result_page_index: self.query_result_page_index,
            rows_editable: cells_editable,
            row_object_available: cells_editable,
            submenu: None,
            selection_row_count: 0,
            selection_copy_label: None,
            selection_export_label: None,
        };

        div()
            .id(("data-cell", row_ix.saturating_mul(10_000) + col_ix))
            .relative()
            .size_full()
            .bg(if highlighted {
                cx.theme().table_hover
            } else {
                cx.theme().transparent
            })
            .flex()
            .items_center()
            .text_size(px(12.))
            .text_color(if is_row_index_col {
                cx.theme().muted_foreground
            } else if null_cell {
                cx.theme().muted_foreground
            } else {
                cx.theme().foreground
            })
            .when(
                self.highlight_search_matches && search_match && !active_search,
                |this| {
                    this.child(
                        div()
                            .absolute()
                            .top(px(-4.))
                            .right(px(-8.))
                            .bottom(px(-4.))
                            .left(px(-8.))
                            .bg(if cx.theme().is_dark() {
                                hsla(48. / 360., 0.80, 0.34, 0.50)
                            } else {
                                hsla(55. / 360., 1.0, 0.82, 0.72)
                            }),
                    )
                },
            )
            .when(deleted_row && !active_search, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(-4.))
                        .right(px(-8.))
                        .bottom(px(-4.))
                        .left(px(-8.))
                        .bg(deleted_bg),
                )
            })
            .when(dirty_cell && !deleted_row && !active_search, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(-4.))
                        .right(px(-8.))
                        .bottom(px(-4.))
                        .left(px(-8.))
                        .bg(dirty_bg),
                )
            })
            // 单元格覆盖层只按单元格级选中绘制：它是相对内容区的绝对定位块，
            // 左右只外扩 8px 而单元格内边距是 12px，整行绘制时相邻块之间会露出缝隙，
            // 因此整行选中统一交给 render_tr 的行底色处理。
            .when(selected_cell, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(-4.))
                        .right(px(-8.))
                        .bottom(px(-4.))
                        .left(px(-8.))
                        .when(!redis_row_selection, |this| {
                            this.border_1().border_color(cell_border)
                        })
                        .when(
                            data_cell_selection_should_fill_background(dirty_cell, deleted_row),
                            |this| this.bg(cell_bg),
                        ),
                )
            })
            .when(!editing_cell, |this| {
                this.child(
                    div()
                        .relative()
                        .w_full()
                        .when(redis_cell, |this| this.px_3())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .opacity(data_cell_content_opacity(deleted_row))
                        .when(null_cell, |this| this.italic())
                        .child(
                            if redis_action_cell {
                                redis_action_cell_buttons(
                                    tab_id,
                                    source_row_ix,
                                    self.view.clone(),
                                    deleted_row,
                                    cx.theme().is_dark(),
                                    ComponentTheme::global(cx).radius,
                                )
                            } else if redis_type_cell {
                                redis_type_tag(text.to_string(), redis_type_dark)
                            } else {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(text),
                                    )
                                    .when_some(choice_label, |this, label| {
                                        this.child(
                                            div()
                                                .flex_none()
                                                .max_w(px(96.))
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .text_size(px(11.))
                                                .text_color(cx.theme().muted_foreground)
                                                .child(label),
                                        )
                                    })
                            },
                        )
                        .when(deleted_row, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .top(px(8.))
                                    .h(px(1.))
                                    .bg(deleted_line),
                            )
                        }),
                )
            })
            .when(editing_cell, |this| {
                // “当前单元格本身”切换为可编辑态，而不是在 cell 里再套一个更小的盒子。
                // render_cell 会对每个 cell 施加内边距（左右 12px、上下 5px），render_td 的 size_full 盒
                // 只占内边距内部的内容区；本覆盖层相对 render_td 用负偏移补齐这些内边距，
                // 使外框严格贴合 cell 边界（与 cell 同尺寸、同边界感）。
                // 单元格本身是直角网格，因此不加 rounded_sm，避免圆角与 cell 不一致。
                // 本覆盖层作为渲染顺序上最后添加的子元素，其不透明背景会盖住选择/脏数据等覆盖层。
                this.child(
                    div()
                        .absolute()
                        .top(px(-5.))
                        .right(px(-12.))
                        .bottom(px(-5.))
                        .left(px(-12.))
                        .bg(cx.theme().background)
                        .border_1()
                        .border_color(cell_border)
                        // 只留 2px 左右内边距，输入区贴合 cell、文本离边框近但不贴死。
                        .px_0p5()
                        .flex()
                        .items_center()
                        .when(temporal_kind.is_none(), |this| {
                            this.child(
                                Input::new(&edit_input)
                                    .appearance(false)
                                    .focus_bordered(false)
                                    // Input 只负责内容输入并填满本覆盖层，清掉其自带 medium size 的水平内边距。
                                    .px_0()
                                    .w_full()
                                    .h_full()
                                    .text_size(px(12.)),
                            )
                        })
                        .when(temporal_kind.is_some(), |this| {
                            this.child(
                                div()
                                    .w_full()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .font_family("Menlo")
                                    .child(editing_input_value),
                            )
                        }),
                )
                .when_some(temporal_kind, |this, kind| {
                    this.child(data_cell_temporal_picker(
                        kind,
                        edit_input.clone(),
                        self.temporal_part_input.clone(),
                        self.temporal_part_editing,
                        edit_state,
                        self.view.clone(),
                        meta_for_menu.nullable,
                        cx,
                    ))
                })
                .when(
                    choice_picker && temporal_kind.is_none() && !is_row_index_col,
                    |this| {
                        this.child(data_cell_choice_picker(
                            editor_kind,
                            meta_for_menu.clone(),
                            edit_input.clone(),
                            self.view.clone(),
                            cx,
                        ))
                    },
                )
            })
            .when(hovered && !redis_cell, |this| {
                this.child(data_table_cell_hover_action(cx).on_mouse_down(
                    MouseButton::Left,
                    move |_, _, cx| {
                        if !is_row_index_col {
                            let _ = view_for_info.update(cx, |this, cx| {
                                if cells_editable {
                                    this.dispatch(
                                        AppCommand::OpenCellDetail {
                                            tab_id,
                                            row: source_row_ix,
                                            column: source_col_ix,
                                        },
                                        cx,
                                    );
                                } else {
                                    this.query_result_cell_detail.insert(
                                        tab_id,
                                        QueryResultCellDetail {
                                            row: source_row_ix,
                                            column: source_col_ix,
                                        },
                                    );
                                    cx.notify();
                                }
                            });
                        }
                        cx.stop_propagation();
                    },
                ))
            })
            .on_mouse_move(cx.listener(move |table, _, _, cx| {
                let delegate = table.delegate_mut();
                if delegate.hovered_cell != Some((row_ix, col_ix)) {
                    delegate.hovered_cell = Some((row_ix, col_ix));
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(move |table, hovered: &bool, _, cx| {
                if !*hovered && table.delegate().hovered_cell == Some((row_ix, col_ix)) {
                    table.delegate_mut().hovered_cell = None;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                    let can_change_cell = view_for_edit
                        .update(cx, |this, cx| {
                            this.commit_data_cell_edit_before_cell_change(
                                edit_state.clone(),
                                window,
                                cx,
                            )
                        })
                        .unwrap_or(true);
                    if !can_change_cell {
                        cx.stop_propagation();
                        return;
                    }
                    table.clear_selection(cx);
                    let delegate = table.delegate_mut();
                    let additive = event.modifiers.secondary();
                    let range_select = event.modifiers.shift;
                    if is_row_index_col || redis_cell {
                        if range_select {
                            delegate.select_row_range_for_click(row_ix);
                        } else {
                            delegate.select_row_for_click(row_ix, additive);
                        }
                    } else {
                        if range_select {
                            delegate.select_cell_range_for_click(row_ix, col_ix);
                        } else {
                            delegate.select_cell_for_click(row_ix, col_ix, additive);
                        }
                    }
                    if !is_row_index_col && event.click_count >= 2 {
                        if redis_cell {
                            // Redis 列暂不提供单元格详情/编辑入口，只保留操作列删除。
                        } else if editable_cell {
                            let _ = view_for_edit.update(cx, |this, cx| {
                                this.begin_data_cell_edit(
                                    edit_state.clone(),
                                    edit_value.clone(),
                                    window,
                                    cx,
                                );
                            });
                            table.delegate_mut().editing_cell = Some(edit_state.clone());
                            table.refresh(cx);
                        } else if !cells_editable {
                            let _ = view_for_edit.update(cx, |this, cx| {
                                this.query_result_cell_detail.insert(
                                    tab_id,
                                    QueryResultCellDetail {
                                        row: source_row_ix,
                                        column: source_col_ix,
                                    },
                                );
                                cx.notify();
                            });
                        } else {
                            let _ = view_for_edit.update(cx, |this, cx| {
                                this.dispatch(
                                    AppCommand::OpenCellDetail {
                                        tab_id,
                                        row: source_row_ix,
                                        column: source_col_ix,
                                    },
                                    cx,
                                );
                                let event = this
                                    .controller
                                    .dispatch(AppCommand::StartCellDetailEdit(tab_id));
                                if let AppEvent::Failed(error) = &event {
                                    this.show_message(
                                        error.message.clone(),
                                        AppMessageKind::Warning,
                                        cx,
                                    );
                                }
                                this.apply_app_event(&event, cx);
                            });
                        }
                    }
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                    let can_change_cell = view_for_menu
                        .update(cx, |this, cx| {
                            this.commit_data_cell_edit_before_cell_change(
                                edit_state.clone(),
                                window,
                                cx,
                            )
                        })
                        .unwrap_or(true);
                    if !can_change_cell {
                        cx.stop_propagation();
                        return;
                    }
                    table.clear_selection(cx);
                    let delegate = table.delegate_mut();
                    if is_row_index_col {
                        if !delegate.has_row_selected(row_ix) {
                            delegate.select_row_for_click(row_ix, false);
                        } else {
                            delegate.selected_row = Some(row_ix);
                        }
                        let mut menu = row_context_menu.clone();
                        menu.position = event.position;
                        menu.selection_row_count = delegate.selected_row_count();
                        menu.selection_copy_label = delegate.selection_copy_label();
                        menu.selection_export_label = delegate.selection_export_label();
                        let _ = view_for_menu.update(cx, |this, cx| {
                            this.show_data_row_context_menu(menu, window, cx);
                        });
                    } else {
                        if !delegate.has_cell_selected(row_ix, col_ix) {
                            delegate.select_cell_for_click(row_ix, col_ix, false);
                        } else {
                            delegate.selected_cell = Some((row_ix, col_ix));
                        }
                        let mut menu = context_menu.clone();
                        menu.position = event.position;
                        menu.selection_row_count = delegate.selected_row_count();
                        menu.selection_copy_label = delegate.selection_copy_label();
                        menu.selection_export_label = delegate.selection_export_label();
                        let _ = view_for_menu.update(cx, |this, cx| {
                            this.show_data_cell_context_menu(menu, window, cx);
                        });
                    }
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(|table, _, _, cx| {
                table.clear_selection(cx);
                cx.stop_propagation();
            }))
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let selected_row_bg: Hsla = if cx.theme().is_dark() {
            rgb(0x131923).into()
        } else {
            rgb(0xeef4ff).into()
        };
        let hovered_row_bg: Hsla = if cx.theme().is_dark() {
            rgb(0x252525).into()
        } else {
            rgb(0xf1f4f8).into()
        };
        let selected_row = self.row_has_selected_cell(row_ix) || self.has_row_selected(row_ix);
        let hovered_row = self.hovered_cell.is_some_and(|(row, _)| row == row_ix);
        div()
            .id(("row", row_ix))
            .relative()
            .when(hovered_row && !selected_row, |this| this.bg(hovered_row_bg))
            // gpui-component 会在行元素上再追加一层 hover 背景（它只在自身 selected_row 时跳过，
            // 而本表格的选中态由 delegate 自己维护），直接设置 bg 会在鼠标悬停时被覆盖掉。
            // 因此整行选中用绝对定位覆盖层绘制：它排在单元格之前渲染，压住 hover 背景又不挡文本和点击。
            .when(selected_row, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .bg(selected_row_bg),
                )
            })
    }

}
