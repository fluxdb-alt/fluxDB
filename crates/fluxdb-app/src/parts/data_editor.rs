impl Default for AppController {
    fn default() -> Self {
        Self::new()
    }
}

fn should_clear_completion_cache(command: &AppCommand) -> bool {
    matches!(
        command,
        AppCommand::LoadConnections
            | AppCommand::ReplaceConnections(_)
            | AppCommand::CreateConnection(_)
            | AppCommand::UpdateConnection(_)
            | AppCommand::OpenConnection(_)
            | AppCommand::DisconnectConnection(_)
            | AppCommand::DeleteConnection(_)
            | AppCommand::LoadObjectChildren(_)
            | AppCommand::RefreshObject(_)
            | AppCommand::ApplyCreateTable(_)
            | AppCommand::RenameTable { .. }
            | AppCommand::CopyTable { .. }
            | AppCommand::DropTable { .. }
            | AppCommand::TruncateTable { .. }
            | AppCommand::ExecuteQuery(_)
            | AppCommand::ExecuteQueryText { .. }
            | AppCommand::ExecuteQueryTextWithOptions { .. }
    )
}

fn edit_data_cell(
    editor: &mut DataEditorState,
    row: usize,
    column: usize,
    value: CellValue,
) -> fluxdb_core::Result<()> {
    let insert_index = inserted_row_change_index(editor, row);
    let page = editor
        .page
        .as_mut()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    let column_name = page
        .columns
        .get(column)
        .map(|column| column.name.clone())
        .ok_or_else(|| Error::new(ErrorKind::Internal, "列不存在"))?;
    let identity = row_identity(page, row)?;
    let original_value = editor
        .original_page
        .as_ref()
        .and_then(|original_page| cell_value_at(original_page, row, column).ok());
    let row_values = page
        .rows
        .get_mut(row)
        .ok_or_else(|| Error::new(ErrorKind::Internal, "行不存在"))?;
    let cell = row_values
        .values
        .get_mut(column)
        .ok_or_else(|| Error::new(ErrorKind::Internal, "单元格不存在"))?;

    *cell = value.clone();
    editor.editing_cell = Some(CellPosition { row, column });
    if let Some(insert_index) = insert_index
        && let Some(insert) = editor
            .changes
            .as_mut()
            .and_then(|changes| changes.inserts.get_mut(insert_index))
    {
        if let Some(insert_cell) = insert.values.get_mut(column) {
            *insert_cell = value;
        }
        return Ok(());
    }

    if original_value.as_ref() == Some(&value) {
        if let Some(changes) = editor.changes.as_mut() {
            remove_cell_update(changes, &identity, &column_name);
        }
        if editor.changes.as_ref().is_some_and(DataChangeSet::is_empty) {
            editor.changes = None;
        }
    } else {
        upsert_cell_update(
            editor.changes.get_or_insert_with(|| DataChangeSet {
                object: editor.object.clone(),
                inserts: Vec::new(),
                updates: Vec::new(),
                deletes: Vec::new(),
            }),
            identity,
            column_name,
            value,
        );
    }

    Ok(())
}

fn binary_preview(
    editor: &DataEditorState,
    row: usize,
    column: usize,
) -> fluxdb_core::Result<BinaryPreviewResponse> {
    ensure_binary_cell(editor, row, column)?;
    match current_cell_value(editor, row, column)? {
        CellValue::Bytes(bytes) => {
            let preview_size = bytes.len().min(64);
            Ok(BinaryPreviewResponse {
                byte_length: bytes.len() as u64,
                preview_hex: bytes[..preview_size]
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<String>(),
                preview_size: preview_size as u64,
            })
        }
        CellValue::BinarySummary(summary) => {
            let preview_hex = summary.preview_hex.unwrap_or_default();
            Ok(BinaryPreviewResponse {
                byte_length: summary.byte_length,
                preview_size: (preview_hex.len() / 2) as u64,
                preview_hex,
            })
        }
        CellValue::Null => Ok(BinaryPreviewResponse {
            byte_length: 0,
            preview_hex: String::new(),
            preview_size: 0,
        }),
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            "当前单元格不是二进制字段",
        )),
    }
}

fn update_binary_cell(
    editor: &mut DataEditorState,
    row: usize,
    column: usize,
    payload: BinaryUpdatePayload,
) -> fluxdb_core::Result<()> {
    ensure_binary_cell(editor, row, column)?;
    let value = match payload {
        BinaryUpdatePayload::SetNull => CellValue::Null,
        BinaryUpdatePayload::Hex(hex) => {
            let bytes = parse_hex_bytes(&hex)?;
            if bytes.len() as u64 > HEX_EDIT_LIMIT {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "大二进制值不能直接 Hex 编辑，请使用下载、上传替换或设为 NULL",
                ));
            }
            CellValue::Bytes(bytes)
        }
        BinaryUpdatePayload::FilePath(path) => {
            let metadata = std::fs::metadata(&path)
                .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
            if let Some(max_bytes) = binary_column_max_bytes(editor, column)
                && metadata.len() > max_bytes
            {
                let column_name = editor
                    .page
                    .as_ref()
                    .and_then(|page| page.columns.get(column))
                    .map(|column| column.name.as_str())
                    .unwrap_or("当前字段");
                let type_name = editor
                    .page
                    .as_ref()
                    .and_then(|page| page.columns.get(column))
                    .and_then(|column| column.type_name.as_deref())
                    .unwrap_or("二进制字段");
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!(
                        "导入失败：{column_name} 是 {type_name}，最多支持 {}，当前文件 {}",
                        fluxdb_core::format_byte_length(max_bytes),
                        fluxdb_core::format_byte_length(metadata.len())
                    ),
                ));
            }
            if metadata.len() > BINARY_FILE_UPLOAD_LIMIT {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!(
                        "上传文件超过 {} MB，请等待大文件流式上传支持",
                        BINARY_FILE_UPLOAD_LIMIT / 1024 / 1024
                    ),
                ));
            }
            let bytes = std::fs::read(path)
                .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
            CellValue::Bytes(bytes)
        }
    };

    edit_data_cell(editor, row, column, value)
}

fn binary_column_max_bytes(editor: &DataEditorState, column: usize) -> Option<u64> {
    editor
        .page
        .as_ref()?
        .columns
        .get(column)?
        .type_name
        .as_deref()
        .and_then(fluxdb_core::binary_type_max_bytes)
}

fn ensure_binary_cell(editor: &DataEditorState, row: usize, column: usize) -> fluxdb_core::Result<()> {
    let page = editor
        .page
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    let type_is_binary = page
        .columns
        .get(column)
        .and_then(|column| column.type_name.as_deref())
        .is_some_and(fluxdb_core::is_binary_type_name);
    let value_is_binary = matches!(
        cell_value_at(page, row, column)?,
        CellValue::Bytes(_) | CellValue::BinarySummary(_)
    );

    if type_is_binary || value_is_binary {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::Unsupported,
            "当前单元格不是二进制字段",
        ))
    }
}

fn apply_data_editor_edit(
    tab: &mut TabState,
    result_index: Option<usize>,
    edit: impl FnOnce(&mut DataEditorState) -> fluxdb_core::Result<()>,
) -> fluxdb_core::Result<()> {
    match &mut tab.kind {
        TabKind::DataEditor(editor) => edit(editor)?,
        TabKind::QueryEditor(query_editor) => {
            let page_index = result_index
                .or(query_editor.active_result_editor)
                .ok_or_else(|| Error::new(ErrorKind::Internal, "查询结果不存在"))?;
            let editor = query_editor
                .result_editors
                .get_mut(&page_index)
                .ok_or_else(|| Error::new(ErrorKind::Internal, "查询结果不可编辑"))?;
            edit(editor)?;
            if let Some(page) = editor.page.clone()
                && let Some(result) = query_editor.results.get_mut(page_index)
            {
                *result = page;
            }
        }
        _ => return Err(Error::new(ErrorKind::Internal, "数据结果不存在")),
    }
    tab.dirty = tab_has_data_changes(tab);
    Ok(())
}

fn tab_has_data_changes(tab: &TabState) -> bool {
    match &tab.kind {
        TabKind::DataEditor(editor) => editor
            .changes
            .as_ref()
            .is_some_and(|changes| !changes.is_empty()),
        TabKind::QueryEditor(editor) => editor.result_editors.values().any(|editor| {
            editor
                .changes
                .as_ref()
                .is_some_and(|changes| !changes.is_empty())
        }),
        _ => false,
    }
}

fn insert_data_row(
    editor: &mut DataEditorState,
    after_row: Option<usize>,
) -> fluxdb_core::Result<()> {
    let page = editor
        .page
        .as_mut()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    let row = Row {
        values: page.columns.iter().map(|_| CellValue::Null).collect(),
    };
    let insert_at = row_insert_index(page.rows.len(), after_row);

    page.rows.insert(insert_at, row.clone());
    editor.editing_cell = Some(CellPosition { row: insert_at, column: 0 });
    editor
        .changes
        .get_or_insert_with(|| DataChangeSet {
            object: editor.object.clone(),
            inserts: Vec::new(),
            updates: Vec::new(),
            deletes: Vec::new(),
        })
        .inserts
        .push(row);

    Ok(())
}

fn clone_data_row(
    editor: &mut DataEditorState,
    row: usize,
    after_row: Option<usize>,
) -> fluxdb_core::Result<()> {
    let page = editor
        .page
        .as_mut()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    let original = page
        .rows
        .get(row)
        .cloned()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "行不存在"))?;
    let cloned = Row {
        values: page
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                if column.primary_key {
                    CellValue::Null
                } else {
                    original
                        .values
                        .get(index)
                        .cloned()
                        .unwrap_or(CellValue::Null)
                }
            })
            .collect(),
    };
    let insert_at = row_insert_index(page.rows.len(), after_row);

    page.rows.insert(insert_at, cloned.clone());
    editor.editing_cell = Some(CellPosition { row: insert_at, column: 0 });
    editor
        .changes
        .get_or_insert_with(|| DataChangeSet {
            object: editor.object.clone(),
            inserts: Vec::new(),
            updates: Vec::new(),
            deletes: Vec::new(),
        })
        .inserts
        .push(cloned);

    Ok(())
}

fn row_insert_index(row_count: usize, after_row: Option<usize>) -> usize {
    after_row.map_or(row_count, |row| row.saturating_add(1).min(row_count))
}

fn inserted_row_change_index(editor: &DataEditorState, row: usize) -> Option<usize> {
    let page = editor.page.as_ref()?;
    page.rows.get(row)?;
    let original = editor.original_page.as_ref()?;
    let changes = editor.changes.as_ref()?;
    let inserted_before_or_at = page
        .rows
        .iter()
        .enumerate()
        .take(row.saturating_add(1))
        .filter(|(row_index, _)| {
            row_identity(page, *row_index)
                .ok()
                .map_or(true, |identity| !page_contains_identity(original, &identity))
        })
        .count();
    inserted_before_or_at
        .checked_sub(1)
        .filter(|index| *index < changes.inserts.len())
}

fn page_contains_identity(page: &DataPage, identity: &RowIdentity) -> bool {
    page.rows.iter().enumerate().any(|(row_index, _)| {
        row_identity(page, row_index)
            .as_ref()
            .is_ok_and(|row_identity| row_identity == identity)
    })
}

fn delete_data_row(editor: &mut DataEditorState, row: usize) -> fluxdb_core::Result<()> {
    let page = editor
        .page
        .as_mut()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    let identity = row_identity(page, row)?;
    page.rows
        .get(row)
        .ok_or_else(|| Error::new(ErrorKind::Internal, "行不存在"))?;
    editor.editing_cell = None;
    let changes = editor.changes.get_or_insert_with(|| DataChangeSet {
        object: editor.object.clone(),
        inserts: Vec::new(),
        updates: Vec::new(),
        deletes: Vec::new(),
    });
    changes.updates.retain(|update| update.identity != identity);
    if !changes.deletes.contains(&identity) {
        changes.deletes.push(identity);
    }

    Ok(())
}

fn save_cell_detail_edit(editor: &mut DataEditorState) -> fluxdb_core::Result<()> {
    let active_cell = editor
        .cell_detail_panel
        .active_cell
        .ok_or_else(|| Error::new(ErrorKind::Internal, "未选择单元格"))?;
    let current_value = current_cell_value(editor, active_cell.row, active_cell.column)?;
    let next_value =
        cell_value_from_edit_text(&current_value, &editor.cell_detail_panel.edit_value)?;
    set_cell_detail_value(editor, next_value)
}

fn set_cell_detail_value(editor: &mut DataEditorState, value: CellValue) -> fluxdb_core::Result<()> {
    let active_cell = editor
        .cell_detail_panel
        .active_cell
        .ok_or_else(|| Error::new(ErrorKind::Internal, "未选择单元格"))?;
    edit_data_cell(editor, active_cell.row, active_cell.column, value.clone())?;
    editor.cell_detail_panel.mode = CellDetailMode::View;
    editor.cell_detail_panel.edit_value = cell_value_edit_text(&value);
    Ok(())
}

fn active_detail_cell_value(editor: &DataEditorState) -> fluxdb_core::Result<CellValue> {
    let active_cell = editor
        .cell_detail_panel
        .active_cell
        .ok_or_else(|| Error::new(ErrorKind::Internal, "未选择单元格"))?;
    current_cell_value(editor, active_cell.row, active_cell.column)
}

fn active_original_cell_value(editor: &DataEditorState) -> fluxdb_core::Result<CellValue> {
    let active_cell = editor
        .cell_detail_panel
        .active_cell
        .ok_or_else(|| Error::new(ErrorKind::Internal, "未选择单元格"))?;
    let page = editor
        .original_page
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "原始数据页未加载"))?;
    cell_value_at(page, active_cell.row, active_cell.column)
}

fn current_cell_value(
    editor: &DataEditorState,
    row: usize,
    column: usize,
) -> fluxdb_core::Result<CellValue> {
    let page = editor
        .page
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    cell_value_at(page, row, column)
}

fn cell_value_at(page: &DataPage, row: usize, column: usize) -> fluxdb_core::Result<CellValue> {
    page.rows
        .get(row)
        .and_then(|row| row.values.get(column))
        .cloned()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "单元格不存在"))
}

fn cell_value_edit_text(value: &CellValue) -> String {
    match value {
        CellValue::Null => String::new(),
        CellValue::Bool(value) => value.to_string(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) => value.to_string(),
        CellValue::Text(value) | CellValue::Json(value) => value.clone(),
        CellValue::Bytes(value) => value
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" "),
        CellValue::BinarySummary(summary) => summary.preview_hex.clone().unwrap_or_default(),
    }
}

fn cell_value_from_edit_text(original: &CellValue, value: &str) -> fluxdb_core::Result<CellValue> {
    let value = match original {
        CellValue::Null | CellValue::Text(_) => CellValue::Text(value.to_string()),
        CellValue::Bool(_) => value
            .trim()
            .parse::<bool>()
            .map(CellValue::Bool)
            .unwrap_or_else(|_| CellValue::Text(value.to_string())),
        CellValue::I64(_) => value
            .trim()
            .parse::<i64>()
            .map(CellValue::I64)
            .unwrap_or_else(|_| CellValue::Text(value.to_string())),
        CellValue::F64(_) => value
            .trim()
            .parse::<f64>()
            .map(CellValue::F64)
            .unwrap_or_else(|_| CellValue::Text(value.to_string())),
        CellValue::Bytes(_) | CellValue::BinarySummary(_) => {
            CellValue::Bytes(parse_hex_bytes(value)?)
        }
        CellValue::Json(_) => CellValue::Json(value.to_string()),
    };

    Ok(value)
}

fn parse_hex_bytes(value: &str) -> fluxdb_core::Result<Vec<u8>> {
    let hex = value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if hex.len() % 2 != 0 {
        return Err(Error::new(ErrorKind::Query, "Hex 长度必须是偶数"));
    }

    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16)
                .map_err(|_| Error::new(ErrorKind::Query, "Hex 只能包含 0-9、A-F"))
        })
        .collect()
}

fn row_identity(page: &DataPage, row_index: usize) -> fluxdb_core::Result<RowIdentity> {
    let row = page
        .rows
        .get(row_index)
        .ok_or_else(|| Error::new(ErrorKind::Internal, "行不存在"))?;
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

    if values.is_empty() {
        return Err(Error::new(ErrorKind::Internal, "无法定位行身份"));
    }

    Ok(RowIdentity { values })
}

fn upsert_cell_update(
    changes: &mut DataChangeSet,
    identity: RowIdentity,
    column: String,
    value: CellValue,
) {
    if let Some(update) = changes
        .updates
        .iter_mut()
        .find(|update| update.identity == identity)
    {
        if let Some(cell) = update.cells.iter_mut().find(|cell| cell.column == column) {
            cell.value = value;
        } else {
            update.cells.push(CellUpdate { column, value });
        }
        return;
    }

    changes.updates.push(RowUpdate {
        identity,
        cells: vec![CellUpdate { column, value }],
    });
}

fn remove_cell_update(changes: &mut DataChangeSet, identity: &RowIdentity, column: &str) {
    if let Some(update) = changes
        .updates
        .iter_mut()
        .find(|update| &update.identity == identity)
    {
        update.cells.retain(|cell| cell.column != column);
    }
    changes.updates.retain(|update| !update.cells.is_empty());
}
