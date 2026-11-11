impl DataPageTableDelegate {
    fn from_page_with_rule(
        view: WeakEntity<NavicatMain>,
        tab_id: TabId,
        page: Arc<DataPage>,
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
                comment: None,
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
                comment: column.comment.clone(),
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
                TableColumn::new("__redis_actions", tr("app.data.header.actions"))
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
                name: tr("app.data.header.actions").to_string(),
                type_name: "redis_action".to_string(),
                comment: None,
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
        let dirty_cells = data_change_dirty_cells(page.as_ref(), changes);
        let deleted_rows = data_change_deleted_rows(page.as_ref(), changes);
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
        let search_matches = data_search_matches(
            &page,
            visible_column_indexes.as_slice(),
            search_query.unwrap_or_default(),
        );
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
            page,
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

/// 单元格显示文本：把换行折成空格，避免撑破单行网格。
/// 只在可见格渲染/搜索/导出时调用，不预生成整页文本副本。
fn data_cell_display_text(value: &CellValue) -> String {
    match value {
        // 直接折叠，不先整串 clone 再 replace：复制量随字段大小线性增长，
        // 一帧里每个可见格都做一次就足以让 8 MB 单元格拖垮滚动。
        CellValue::Text(text) | CellValue::Json(text) => fold_cell_newlines(text),
        _ => fold_cell_newlines(&cell_value_label(value)),
    }
}

fn fold_cell_newlines(text: &str) -> String {
    if text.contains(['\r', '\n']) {
        text.replace(['\r', '\n'], " ")
    } else {
        text.to_string()
    }
}

/// 单元格可见文本上限，单位是字符而不是字节。
/// 列宽通常几百像素，再多也画不出来；与 dbdelve 的 `CELL_DISPLAY_LIMIT` 一致。
/// 完整值不丢，编辑/右键/复制走 `data_cell_full_text()` 再取。
const DATA_CELL_DISPLAY_LIMIT: usize = 300;

/// 可见格文本：按字符截断并折叠换行，超长 TEXT/JSON 不再每帧复制整串。
/// 截断后补 `…`，让“还有内容”在视觉上成立；多余内容仍可通过交互读取。
fn data_cell_clipped_text(value: &CellValue) -> String {
    match value {
        CellValue::Text(text) | CellValue::Json(text) => clip_cell_text(text),
        // 预览最长 4000 字符，不裁剪会每帧复制整段；补标记说明不是完整值。
        CellValue::TruncatedText { preview, .. } => {
            format!("{} …（仅预览）", clip_cell_text(preview))
        }
        // 超大字段同理：只有有界预览，标记不能省。
        CellValue::LargeValue(large) => {
            format!("{} …（仅预览）", clip_cell_text(&large.preview))
        }
        _ => data_cell_display_text(value),
    }
}

/// 只扫描前 `DATA_CELL_DISPLAY_LIMIT` 个字符：按 `char` 迭代保证不会切在多字节
/// 字符中间，扫描量也因此有界，不受字段总长度影响。
///
/// 直接接收 `&str`（不经过 `cell_value_label`）：那里会 `clone()` 整串，
/// 8 MB 字段每帧复制一次就是滚动卡顿的主因。
fn clip_cell_text(text: &str) -> String {
    let mut shown = String::new();
    for (count, ch) in text.chars().enumerate() {
        if count == DATA_CELL_DISPLAY_LIMIT {
            shown.push('…');
            return shown;
        }
        shown.push(if matches!(ch, '\r' | '\n') { ' ' } else { ch });
    }
    shown
}

/// 选项值比较：不复制完整单元格文本（枚举/布尔列的选项匹配）。
fn cell_value_equals(value: &CellValue, expected: &str) -> bool {
    match value {
        CellValue::Null | CellValue::Bytes(_) | CellValue::BinarySummary(_) => false,
        CellValue::Bool(value) => value.to_string() == expected,
        CellValue::I64(value) => value.to_string() == expected,
        CellValue::F64(value) => value.to_string() == expected,
        CellValue::Text(value) | CellValue::Json(value) => value == expected,
        // 预览是前缀，只可能匹配到以它开头的选项。
        CellValue::TruncatedText { preview, .. } => preview == expected,
        CellValue::LargeValue(large) => large.preview == expected,
    }
}

impl DataPageTableDelegate {
    /// 单元格完整显示文本（换行折叠）。只在编辑/右键/复制等交互时调用。
    fn data_cell_full_text(&self, row_ix: usize, col_ix: usize) -> String {
        if self.show_row_index && col_ix == 0 {
            return (row_ix + 1).to_string();
        }
        let fallback = if self.show_row_index {
            col_ix.saturating_sub(1)
        } else {
            col_ix
        };
        let source_col = self
            .source_column_indexes
            .get(col_ix)
            .and_then(|index| *index)
            .unwrap_or(fallback);
        self.page
            .rows
            .get(row_ix)
            .and_then(|row| row.values.get(source_col))
            .map(data_cell_display_text)
            .unwrap_or_default()
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
    if source_row_indexes.len() != delegate.page.rows.len() {
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
            .text_size(ui_text_size(12.))
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
                            .text_size(ui_text_size(12.))
                            .font_weight(if cell.strong {
                                gpui::FontWeight::SEMIBOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(cx.theme().foreground)
                            .when(cell.mono, |this| this.font_family(crate::PLATFORM_MONOSPACE_FONT))
                            .child(cell.title),
                    )
                    .when_some(cell.badge, |this, badge| {
                        this.child(
                            div()
                                .flex_none()
                                .px_1()
                                .rounded(ComponentTheme::global(cx).radius * 0.5)
                                .bg(cx.theme().warning.opacity(0.18))
                                .text_size(ui_text_size(10.))
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
        let Some(row_index) = data_page_row_for_identity(page, &update.identity, &update.cells)
        else {
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
            // 删除行可能同时被编辑过（含主键列），带上该行改动里的新值才能定位。
            let changed_cells = changes
                .updates
                .iter()
                .find(|update| &update.identity == identity)
                .map(|update| update.cells.as_slice())
                .unwrap_or_default();
            data_page_row_for_identity(page, identity, changed_cells)
        })
        .collect()
}

/// 找到改动集里某个原始行身份对应的页面行。
///
/// 身份值来自改动前的原始行，而页面行会被就地编辑（主键列也允许改），
/// 直接用身份比较会失配：改动格不高亮、删除行不置灰。
/// 身份列按「本次改动过 → 比对新值，否则 → 比对原值」判定。
fn data_page_row_for_identity(
    page: &DataPage,
    identity: &RowIdentity,
    changed_cells: &[CellUpdate],
) -> Option<usize> {
    page.rows.iter().enumerate().find_map(|(row_index, _)| {
        data_page_row_matches_identity(page, row_index, identity, changed_cells).then_some(row_index)
    })
}

fn data_page_row_matches_identity(
    page: &DataPage,
    row_index: usize,
    identity: &RowIdentity,
    changed_cells: &[CellUpdate],
) -> bool {
    let Some(row) = page.rows.get(row_index) else {
        return false;
    };
    identity.values.iter().all(|(name, original)| {
        let Some(column_index) = page.columns.iter().position(|column| &column.name == name) else {
            return false;
        };
        let Some(current) = row.values.get(column_index) else {
            return false;
        };
        match changed_cells.iter().find(|cell| &cell.column == name) {
            Some(cell) => cell.value.placeholder() == current,
            None => current == original,
        }
    })
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
        return Err(tr("app.data.error.binary_edit").replace("%{field}", &meta.name));
    }

    let trimmed = text.trim();
    if meta.nullable && trimmed.eq_ignore_ascii_case("null") {
        return Ok(CellValue::Null);
    }
    if !meta.nullable && trimmed.eq_ignore_ascii_case("null") {
        return Err(tr("app.data.error.not_null").replace("%{field}", &meta.name));
    }

    match data_cell_editor_kind(meta.type_name.as_str()) {
        DataCellEditorKind::Boolean => match trimmed.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "y" | "on" => Ok(CellValue::Bool(true)),
            "0" | "false" | "no" | "n" | "off" => Ok(CellValue::Bool(false)),
            _ => Err(tr("app.data.error.boolean").replace("%{field}", &meta.name)),
        },
        DataCellEditorKind::Enum => {
            let options = data_cell_enum_set_options(meta.type_name.as_str());
            if options.iter().any(|option| option == trimmed) {
                Ok(CellValue::Text(trimmed.to_string()))
            } else {
                Err(tr("app.data.error.enum").replace("%{field}", &meta.name))
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
                Err(tr("app.data.error.set").replace("%{field}", &meta.name))
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
            .map_err(|_| tr("app.data.error.integer").replace("%{field}", &meta.name));
    }
    if matches!(base, "decimal" | "numeric" | "float" | "double" | "real") {
        return text
            .parse::<f64>()
            .map(CellValue::F64)
            .map_err(|_| tr("app.data.error.number").replace("%{field}", &meta.name));
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
            return Err(tr("app.data.error.time_format").replace("%{field}", &meta.name));
        }
    }

    Ok(CellValue::Text(text.to_string()))
}

#[cfg(test)]
mod data_cell_clip_tests {
    use super::*;

    #[test]
    fn clip_flattens_newlines_and_keeps_short_text() {
        assert_eq!(clip_cell_text("a\r\nb"), "a  b");
        assert_eq!(clip_cell_text("short"), "short");
    }

    #[test]
    fn clip_keeps_limit_and_marks_truncation() {
        let exact = "x".repeat(DATA_CELL_DISPLAY_LIMIT);
        assert_eq!(clip_cell_text(&exact), exact);

        let over = "x".repeat(DATA_CELL_DISPLAY_LIMIT + 1);
        let clipped = clip_cell_text(&over);
        assert_eq!(clipped.chars().count(), DATA_CELL_DISPLAY_LIMIT + 1);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn clip_never_splits_multibyte_chars() {
        let value = "🌍".repeat(DATA_CELL_DISPLAY_LIMIT + 10);
        let clipped = clip_cell_text(&value);
        assert_eq!(
            clipped.chars().filter(|ch| *ch == '🌍').count(),
            DATA_CELL_DISPLAY_LIMIT
        );
        assert!(clipped.ends_with('…'));
    }
}

#[cfg(test)]
mod dirty_cell_tests {
    use super::*;
    use fluxdb_core::RowUpdate;

    fn column(name: &str, primary_key: bool) -> GdbColumn {
        GdbColumn {
            name: name.to_string(),
            type_name: Some("int".to_string()),
            nullable: false,
            primary_key,
            comment: None,
        }
    }

    /// 复合主键表：tenant_id + customer_id 都是主键。
    fn page(customer_id: CellValue) -> DataPage {
        DataPage {
            columns: vec![column("tenant_id", true), column("customer_id", true), column("name", false)],
            rows: vec![Row {
                values: vec![CellValue::I64(1), customer_id, CellValue::Text("Old Alice".to_string())],
            }],
            offset: 0,
            limit: 100,
            has_more: false,
            incomplete: None,
        }
    }

    fn update(identity: &[(&str, CellValue)], cells: &[(&str, CellValue)]) -> DataChangeSet {
        DataChangeSet {
            object: ObjectPath {
                connection_id: ConnectionId(0),
                database: Some("db".to_string()),
                schema: None,
                name: "customers".to_string(),
                kind: ObjectKind::Table,
            },
            inserts: Vec::new(),
            updates: vec![RowUpdate {
                identity: RowIdentity {
                    values: identity
                        .iter()
                        .map(|(name, value)| (name.to_string(), value.clone()))
                        .collect(),
                },
                cells: cells
                    .iter()
                    .map(|(name, value)| CellUpdate::new(*name, value.clone()))
                    .collect(),
            }],
            deletes: Vec::new(),
            insert_intents: None,
        }
    }

    #[test]
    fn marks_dirty_cell_after_primary_key_edit() {
        // 页面行已是改后值，改动集身份仍是改前值：仍要高亮被改的主键格。
        let changes = update(
            &[("tenant_id", CellValue::I64(1)), ("customer_id", CellValue::I64(101))],
            &[("customer_id", CellValue::I64(1012))],
        );
        let page = page(CellValue::I64(1012));

        let dirty = data_change_dirty_cells(&page, Some(&changes));

        assert_eq!(dirty, BTreeSet::from([(0, 1)]));
    }

    #[test]
    fn marks_deleted_row_after_primary_key_edit() {
        let mut changes = update(
            &[("tenant_id", CellValue::I64(1)), ("customer_id", CellValue::I64(101))],
            &[("customer_id", CellValue::I64(1012))],
        );
        changes.deletes.push(RowIdentity {
            values: changes.updates[0].identity.values.clone(),
        });
        let page = page(CellValue::I64(1012));

        assert_eq!(data_change_deleted_rows(&page, Some(&changes)), BTreeSet::from([0]));
    }

    #[test]
    fn does_not_mark_unrelated_row() {
        let changes = update(
            &[("tenant_id", CellValue::I64(1)), ("customer_id", CellValue::I64(101))],
            &[("customer_id", CellValue::I64(1012))],
        );
        let page = page(CellValue::I64(999));

        assert!(data_change_dirty_cells(&page, Some(&changes)).is_empty());
    }
}
