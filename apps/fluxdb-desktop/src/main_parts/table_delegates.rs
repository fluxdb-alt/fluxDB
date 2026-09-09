impl DataPageTableDelegate {
    fn from_page_with_rule(
        view: WeakEntity<NavicatMain>,
        tab_id: TabId,
        page: &DataPage,
        sort_rules: Option<&[DataSortRule]>,
        visible_fields: Option<&BTreeSet<String>>,
        column_choices: &BTreeMap<String, Vec<ColumnChoice>>,
        changes: Option<&DataChangeSet>,
        search_query: Option<&str>,
        active_search_match: Option<DataSearchMatch>,
        highlight_search_matches: bool,
        highlighted_column: Option<String>,
        query_result_page_index: Option<usize>,
        edit_input: Entity<InputState>,
        editing_cell: Option<DataCellEditState>,
        temporal_part_input: Entity<InputState>,
        temporal_part_editing: Option<TemporalPartEditState>,
        cells_editable: bool,
        redis_table_width: Option<Pixels>,
        on_sort: DataTableSortHandler,
    ) -> Self {
        let redis_page = page
            .columns
            .iter()
            .all(|column| column.type_name.as_deref() == Some("redis"));
        let header_actions = !redis_page;
        let show_row_index = !redis_page;
        let visible_column_indexes = page
            .columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                visible_fields
                    .map(|fields| fields.contains(&column.name))
                    .unwrap_or(true)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        let mut columns = Vec::new();
        let mut source_column_indexes = Vec::new();
        let mut column_types = Vec::new();
        let mut column_meta = Vec::new();
        if show_row_index {
            columns.push(
                TableColumn::new("__row_index", "#")
                    .width(px(54.))
                    .paddings(data_table_cell_padding())
                    .fixed_left()
                    .resizable(false)
                    .movable(false)
                    .selectable(false),
            );
            source_column_indexes.push(None);
            column_types.push(String::new());
            column_meta.push(DataTableColumnMeta {
                name: String::new(),
                type_name: String::new(),
                nullable: false,
                primary_key: false,
                choices: Vec::new(),
            });
        }
        for index in &visible_column_indexes {
            let column = &page.columns[*index];
            let mut table_column = table_column_from_data_column(column);
            if redis_page {
                table_column.width = redis_table_column_width(
                    column.name.as_str(),
                    redis_table_width.unwrap_or(px(1000.)),
                );
                table_column.paddings = Some(redis_table_cell_padding());
            }
            columns.push(table_column);
            source_column_indexes.push(Some(*index));
            column_types.push(
                page.columns[*index]
                    .type_name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            );
            let column = &page.columns[*index];
            column_meta.push(DataTableColumnMeta {
                name: column.name.clone(),
                type_name: column
                    .type_name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                nullable: column.nullable,
                primary_key: column.primary_key,
                choices: column_choices
                    .get(&column.name)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        if redis_page {
            columns.push(
                TableColumn::new("__redis_actions", "操作")
                    .width(redis_table_column_width(
                        "__redis_actions",
                        redis_table_width.unwrap_or(px(1000.)),
                    ))
                    .paddings(redis_table_cell_padding())
                    .movable(false),
            );
            source_column_indexes.push(None);
            column_types.push(String::new());
            column_meta.push(DataTableColumnMeta {
                name: "操作".to_string(),
                type_name: "redis_action".to_string(),
                nullable: false,
                primary_key: false,
                choices: Vec::new(),
            });
        }

        let rows = page.rows.iter().enumerate().collect::<Vec<_>>();

        let sorts = sort_rules
            .unwrap_or(&[])
            .iter()
            .filter(|rule| rule.enabled)
            .filter_map(|rule| {
                page.columns
                    .iter()
                    .position(|column| column.name == rule.field)
                    .and_then(|column_index| {
                        visible_column_indexes
                            .iter()
                            .position(|index| *index == column_index)
                            .map(|visible_index| DataTableSort {
                                col_ix: visible_index + usize::from(show_row_index),
                                direction: if rule.ascending {
                                    DataTableSortDirection::Ascending
                                } else {
                                    DataTableSortDirection::Descending
                                },
                            })
                    })
            })
            .collect::<Vec<_>>();
        let source_row_indexes = rows.iter().map(|(index, _)| *index).collect::<Vec<_>>();
        let dirty_cells = data_change_dirty_cells(page, changes);
        let deleted_rows = data_change_deleted_rows(page, changes);
        let null_cells = rows
            .iter()
            .flat_map(|(row_index, row)| {
                visible_column_indexes
                    .iter()
                    .filter_map(|column_index| {
                        matches!(row.values.get(*column_index), Some(CellValue::Null))
                            .then_some((*row_index, *column_index))
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>();
        let rows: Vec<Vec<SharedString>> = rows
            .into_iter()
            .map(|(_, row)| {
                visible_column_indexes
                    .iter()
                    .filter_map(|index| row.values.get(*index))
                    .map(|value| {
                        SharedString::from(cell_value_label(value).replace('\r', " ").replace('\n', " "))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let search_matches = data_search_matches(rows.as_slice(), search_query.unwrap_or_default());
        let active_search_match = active_search_match.filter(|active| {
            search_matches
                .iter()
                .any(|search_match| search_match == active)
        });

        Self {
            view,
            tab_id,
            columns,
            source_column_indexes,
            column_types,
            column_meta,
            rows,
            source_row_indexes,
            null_cells,
            dirty_cells,
            deleted_rows,
            sorts,
            selected_cell: None,
            selected_row: None,
            selected_cells: BTreeSet::new(),
            selected_rows: BTreeSet::new(),
            selection_anchor: None,
            hovered_cell: None,
            search_matches,
            active_search_match,
            highlight_search_matches,
            highlighted_column,
            query_result_page_index,
            edit_input,
            editing_cell,
            temporal_part_input,
            temporal_part_editing,
            cells_editable,
            header_actions,
            show_row_index,
            redis_page,
            on_sort,
        }
    }

    fn redis_table_width(&self) -> Option<Pixels> {
        self.redis_page.then(|| {
            px(self
                .columns
                .iter()
                .map(|column| f32::from(column.width))
                .sum::<f32>())
        })
    }

    fn sync_redis_table_width(&mut self, table_width: Pixels) -> bool {
        if !self.redis_page || table_width <= px(0.) {
            return false;
        }
        if self
            .redis_table_width()
            .is_some_and(|width| (f32::from(width) - f32::from(table_width)).abs() < 1.)
        {
            return false;
        }
        for column in &mut self.columns {
            column.width = redis_table_column_width(column.key.as_ref(), table_width);
        }
        true
    }
}

fn redis_table_column_width(name: &str, table_width: Pixels) -> Pixels {
    let weight = match name {
        "键" => 0.12,
        "类型" => 0.10,
        "值" => 0.45,
        "大小" => 0.08,
        "TTL" => 0.08,
        "__redis_actions" => 0.12,
        _ => 0.10,
    };
    px(f32::from(table_width) * weight / 0.95)
}

fn redis_table_fit_width(table_width: Pixels) -> Pixels {
    (table_width - px(16.)).max(px(0.))
}

fn redis_table_cell_padding() -> gpui::Edges<Pixels> {
    gpui::Edges {
        top: px(0.),
        right: px(0.),
        bottom: px(0.),
        left: px(0.),
    }
}

fn apply_data_table_column_widths(
    columns: &mut [TableColumn],
    widths: &BTreeMap<String, Pixels>,
) {
    for column in columns {
        if let Some(width) = widths.get(column.key.as_ref()) {
            column.width = *width;
        }
    }
}

fn apply_data_table_column_widths_from_list(columns: &mut [TableColumn], widths: &[Pixels]) {
    for (column, width) in columns.iter_mut().zip(widths.iter()) {
        column.width = *width;
    }
}

fn apply_data_table_source_row_indexes(
    delegate: &mut DataPageTableDelegate,
    source_row_indexes: &[usize],
) {
    if source_row_indexes.len() != delegate.rows.len() {
        return;
    }

    delegate.dirty_cells = delegate
        .dirty_cells
        .iter()
        .filter_map(|(row, column)| {
            source_row_indexes
                .get(*row)
                .map(|source_row| (*source_row, *column))
        })
        .collect();
    delegate.deleted_rows = delegate
        .deleted_rows
        .iter()
        .filter_map(|row| source_row_indexes.get(*row).copied())
        .collect();
    delegate.source_row_indexes = source_row_indexes.to_vec();
}

#[derive(Clone)]
struct TableInfoTableDelegate {
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    columns: Vec<TableColumn>,
    rows: Vec<TableInfoTableRow>,
}

#[derive(Clone)]
struct TableInfoTableRow {
    cells: Vec<TableInfoTableCell>,
    highlighted: bool,
    column_action: Option<String>,
}

#[derive(Clone)]
struct TableInfoTableCell {
    title: String,
    badge: Option<&'static str>,
    mono: bool,
    strong: bool,
}

impl TableInfoTableCell {
    fn text(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            badge: None,
            mono: false,
            strong: false,
        }
    }

    fn mono(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            badge: None,
            mono: true,
            strong: false,
        }
    }
}

impl TableDelegate for TableInfoTableDelegate {
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
        div()
            .size_full()
            .flex()
            .items_center()
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground)
            .child(self.columns[col_ix].name.clone())
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let highlighted = self.rows.get(row_ix).is_some_and(|row| row.highlighted);

        div()
            .id(("table-info-row", row_ix))
            .when(highlighted, |row| row.bg(cx.theme().table_hover))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let cell = self
            .rows
            .get(row_ix)
            .and_then(|row| row.cells.get(col_ix))
            .cloned()
            .unwrap_or_else(|| TableInfoTableCell::text(""));
        let action = self
            .rows
            .get(row_ix)
            .and_then(|row| row.column_action.clone());
        let view = self.view.clone();
        let tab_id = self.tab_id;

        div()
            .size_full()
            .min_w(px(0.))
            .overflow_hidden()
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                if let Some(column) = action.clone() {
                    let _ = view.update(cx, |this, cx| {
                        this.dispatch(
                            AppCommand::HighlightDataColumn {
                                tab_id,
                                column: column.clone(),
                            },
                            cx,
                        );
                        this.scroll_to_data_column(tab_id, &column, cx);
                    });
                }
                cx.stop_propagation();
            })
            .child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .flex_1()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap_2()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .font_weight(if cell.strong {
                                gpui::FontWeight::SEMIBOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(cx.theme().foreground)
                            .when(cell.mono, |this| this.font_family("Menlo"))
                            .child(cell.title),
                    )
                    .when_some(cell.badge, |this, badge| {
                        this.child(
                            div()
                                .flex_none()
                                .px_1()
                                .rounded(ComponentTheme::global(cx).radius * 0.5)
                                .bg(cx.theme().warning.opacity(0.18))
                                .text_size(px(10.))
                                .text_color(cx.theme().warning)
                                .child(badge),
                        )
                    }),
            )
    }
}

fn data_change_dirty_cells(
    page: &DataPage,
    changes: Option<&DataChangeSet>,
) -> BTreeSet<(usize, usize)> {
    let Some(changes) = changes else {
        return BTreeSet::new();
    };
    let mut dirty_cells = BTreeSet::new();
    for update in &changes.updates {
        let Some(row_index) = page.rows.iter().enumerate().find_map(|(row_index, _)| {
            (data_page_row_identity(page, row_index).as_ref() == Some(&update.identity))
                .then_some(row_index)
        }) else {
            continue;
        };
        for cell in &update.cells {
            if let Some(column_index) = page
                .columns
                .iter()
                .position(|column| column.name == cell.column)
            {
                dirty_cells.insert((row_index, column_index));
            }
        }
    }
    dirty_cells
}

fn data_change_deleted_rows(page: &DataPage, changes: Option<&DataChangeSet>) -> BTreeSet<usize> {
    let Some(changes) = changes else {
        return BTreeSet::new();
    };

    changes
        .deletes
        .iter()
        .filter_map(|identity| {
            page.rows.iter().enumerate().find_map(|(row_index, _)| {
                (data_page_row_identity(page, row_index).as_ref() == Some(identity))
                    .then_some(row_index)
            })
        })
        .collect()
}

fn data_cell_selection_should_fill_background(dirty_cell: bool, deleted_row: bool) -> bool {
    !dirty_cell && !deleted_row
}

fn data_cell_content_opacity(deleted_row: bool) -> f32 {
    if deleted_row { 0.46 } else { 1.0 }
}

fn data_cell_edit_event_should_commit(event: &InputEvent) -> bool {
    matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur)
}

fn data_cell_blur_should_commit(meta: &DataTableColumnMeta, editing: DataCellEditState) -> bool {
    editing.temporal_kind.is_none()
        && data_cell_editor_kind(meta.type_name.as_str()) == DataCellEditorKind::Text
        && meta.choices.is_empty()
}

fn data_cell_edit_should_commit_before_cell_change(
    current: Option<DataCellEditState>,
    target: DataCellEditState,
) -> bool {
    current.is_some_and(|editing| editing != target)
}

fn data_page_row_identity(page: &DataPage, row_index: usize) -> Option<RowIdentity> {
    let row = page.rows.get(row_index)?;
    let mut values = BTreeMap::new();
    for (index, column) in page.columns.iter().enumerate() {
        if column.primary_key
            && let Some(value) = row.values.get(index)
        {
            values.insert(column.name.clone(), value.clone());
        }
    }
    if values.is_empty()
        && let Some(column) = page.columns.first()
        && let Some(value) = row.values.first()
    {
        values.insert(column.name.clone(), value.clone());
    }
    (!values.is_empty()).then_some(RowIdentity { values })
}

fn data_type_color(type_name: &str) -> Option<gpui::Rgba> {
    let ty = type_name.to_ascii_lowercase();
    if ty.is_empty() {
        None
    } else if ty.contains("char")
        || ty.contains("text")
        || ty.contains("json")
        || ty.contains("enum")
        || ty.contains("set")
    {
        Some(rgb(0x21d07a))
    } else if ty.contains("date") || ty.contains("time") || ty.contains("year") {
        Some(rgb(0xc65cff))
    } else if ty.contains("bool") {
        Some(rgb(0x28c7d7))
    } else if ty.contains("int")
        || ty.contains("decimal")
        || ty.contains("numeric")
        || ty.contains("float")
        || ty.contains("double")
        || ty.contains("real")
        || ty.contains("bit")
    {
        Some(rgb(0x4aa3ff))
    } else if fluxdb_core::is_binary_type_name(&ty) {
        Some(rgb(0xf59e0b))
    } else {
        None
    }
}

fn data_type_supports_empty_string(type_name: &str) -> bool {
    let ty = type_name.to_ascii_lowercase();
    ty.contains("char") || ty.contains("text")
}

fn data_type_is_binary(type_name: &str) -> bool {
    fluxdb_core::is_binary_type_name(type_name)
}

fn data_cell_editor_kind(type_name: &str) -> DataCellEditorKind {
    let ty = type_name.trim().to_ascii_lowercase();
    let base = ty
        .split(|ch: char| ch == '(' || ch.is_whitespace())
        .next()
        .unwrap_or_default();

    if base == "bool" || base == "boolean" || ty.starts_with("tinyint(1)") {
        DataCellEditorKind::Boolean
    } else if base == "enum" {
        DataCellEditorKind::Enum
    } else if base == "set" {
        DataCellEditorKind::Set
    } else {
        DataCellEditorKind::Text
    }
}

fn data_cell_enum_set_options(type_name: &str) -> Vec<String> {
    let Some(start) = type_name.find('(') else {
        return Vec::new();
    };
    let Some(end) = type_name.rfind(')') else {
        return Vec::new();
    };
    let mut options = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;

    for ch in type_name[start + 1..end].chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '\'' => {
                if in_quote {
                    options.push(current.clone());
                    current.clear();
                }
                in_quote = !in_quote;
            }
            _ if in_quote => current.push(ch),
            _ => {}
        }
    }

    options
}

fn data_cell_choice_options(kind: DataCellEditorKind, meta: &DataTableColumnMeta) -> Vec<ColumnChoice> {
    match kind {
        DataCellEditorKind::Boolean => ["true", "false"]
            .into_iter()
            .map(|value| ColumnChoice {
                value: value.to_string(),
                label: String::new(),
            })
            .collect(),
        DataCellEditorKind::Enum | DataCellEditorKind::Set => data_cell_enum_set_options(
            meta.type_name.as_str(),
        )
        .into_iter()
        .map(|value| ColumnChoice {
            value,
            label: String::new(),
        })
        .collect(),
        DataCellEditorKind::Text => meta.choices.clone(),
    }
}

fn data_cell_value_from_text(meta: &DataTableColumnMeta, text: &str) -> Result<CellValue, String> {
    if data_type_is_binary(meta.type_name.as_str()) {
        return Err(format!("字段 {} 是二进制类型，不能修改", meta.name));
    }

    let trimmed = text.trim();
    if meta.nullable && trimmed.eq_ignore_ascii_case("null") {
        return Ok(CellValue::Null);
    }
    if !meta.nullable && trimmed.eq_ignore_ascii_case("null") {
        return Err(format!("字段 {} 不允许 NULL", meta.name));
    }

    match data_cell_editor_kind(meta.type_name.as_str()) {
        DataCellEditorKind::Boolean => match trimmed.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "y" | "on" => Ok(CellValue::Bool(true)),
            "0" | "false" | "no" | "n" | "off" => Ok(CellValue::Bool(false)),
            _ => Err(format!("字段 {} 需要布尔值", meta.name)),
        },
        DataCellEditorKind::Enum => {
            let options = data_cell_enum_set_options(meta.type_name.as_str());
            if options.iter().any(|option| option == trimmed) {
                Ok(CellValue::Text(trimmed.to_string()))
            } else {
                Err(format!("字段 {} 不支持该枚举值", meta.name))
            }
        }
        DataCellEditorKind::Set => {
            let options = data_cell_enum_set_options(meta.type_name.as_str());
            let selected = trimmed
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            if selected
                .iter()
                .all(|value| options.iter().any(|option| option == *value))
            {
                Ok(CellValue::Text(selected.join(",")))
            } else {
                Err(format!("字段 {} 不支持该 SET 值", meta.name))
            }
        }
        DataCellEditorKind::Text => data_cell_text_value_for_type(meta, trimmed),
    }
}

fn data_cell_text_value_for_type(
    meta: &DataTableColumnMeta,
    text: &str,
) -> Result<CellValue, String> {
    let ty = meta.type_name.to_ascii_lowercase();
    let base = ty
        .split(|ch: char| ch == '(' || ch.is_whitespace())
        .next()
        .unwrap_or_default();

    if matches!(
        base,
        "tinyint"
            | "smallint"
            | "mediumint"
            | "int"
            | "integer"
            | "bigint"
            | "serial"
            | "bigserial"
    ) {
        return text
            .parse::<i64>()
            .map(CellValue::I64)
            .map_err(|_| format!("字段 {} 需要整数", meta.name));
    }
    if matches!(base, "decimal" | "numeric" | "float" | "double" | "real") {
        return text
            .parse::<f64>()
            .map(CellValue::F64)
            .map_err(|_| format!("字段 {} 需要数字", meta.name));
    }
    if let Some(kind) = data_cell_temporal_kind(meta.type_name.as_str()) {
        let valid = match kind {
            DataCellTemporalKind::Date => NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok(),
            DataCellTemporalKind::Time => NaiveTime::parse_from_str(text, "%H:%M:%S").is_ok(),
            DataCellTemporalKind::DateTime => {
                parse_temporal_date_part(text).is_some() && temporal_time_part(text).is_some()
            }
        };
        if !valid {
            return Err(format!("字段 {} 的时间格式不正确", meta.name));
        }
    }

    Ok(CellValue::Text(text.to_string()))
}
