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
    )
}

fn edit_data_cell(
    editor: &mut DataEditorState,
    row: usize,
    column: usize,
    value: CellValue,
) -> fluxdb_core::Result<()> {
    edit_data_cell_value(
        editor,
        row,
        column,
        CellUpdateValue::Value(value),
    )
}

/// 记一次单元格改动。`value` 可能是内存里的值，也可能是「已暂存、待流式写入」的引用：
/// 页面里只能放占位值，权威内容留在受控内容存储里，由改动集持有引用。
fn edit_data_cell_value(
    editor: &mut DataEditorState,
    row: usize,
    column: usize,
    value: CellUpdateValue,
) -> fluxdb_core::Result<()> {
    let insert_index = inserted_row_change_index(editor, row);
    let identity = if insert_index.is_none() { Some(original_editor_row_identity(editor, row)?) } else { None };
    let page = editor
        .page
        .as_mut()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    let column_name = page
        .columns
        .get(column)
        .map(|column| column.name.clone())
        .ok_or_else(|| Error::new(ErrorKind::Internal, "列不存在"))?;

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

    // 页面只显示占位值：流式写入时值不在内存里，也没有合法预览。
    *cell = value.placeholder().clone();
    editor.editing_cell = Some(CellPosition { row, column });
    if let Some(insert_index) = insert_index {
        // 新插入的行没有可追加的行身份，只能整值写入；流式值到这里必须先落成值。
        let CellUpdateValue::Value(value) = value else {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "新增行不支持流式写入，请先提交后再修改该字段",
            ));
        };
        if let Some(insert) = editor
            .changes
            .as_mut()
            .and_then(|changes| changes.inserts.get_mut(insert_index))
            && let Some(insert_cell) = insert.values.get_mut(column)
        {
            *insert_cell = value;
        }
        return Ok(());
    }

    let identity = identity.ok_or_else(|| Error::new(ErrorKind::Internal, "原始行身份缺失"))?;
    let matches_original = match &value {
        CellUpdateValue::Value(value) => original_value.as_ref() == Some(value),
        // 流式写入的内容不可能已经等于原值（它来自一次新的上传）。
        CellUpdateValue::Stream(_) => false,
    };
    if matches_original {
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
                insert_intents: None,
            }),
            identity,
            column_name,
            value,
        );
    }

    Ok(())
}

/// 上传替换：把本地文件（或 NULL）写进一个可承载整值的大字段。
///
/// 小文件整值写入（单条语句，最快路径）；达到阈值才暂存 + 分块追加。
/// `streamed_supported` 由数据库类型决定，必须在暂存**之前**判定，
/// 否则会先落几百 MiB 再在提交阶段失败。
fn update_binary_cell(
    editor: &mut DataEditorState,
    row: usize,
    column: usize,
    payload: UploadPayload,
    streamed_supported: bool,
) -> fluxdb_core::Result<()> {
    let kind = ensure_upload_cell(editor, row, column)?;
    let value = match payload {
        UploadPayload::SetNull => CellUpdateValue::Value(CellValue::Null),
        UploadPayload::FilePath(path) => {
            let metadata = std::fs::metadata(&path)
                .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
            let byte_length = metadata.len();
            reject_oversized_upload(editor, column, kind, byte_length)?;
            if byte_length >= fluxdb_core::BLOB_UPLOAD_STREAM_THRESHOLD_BYTES {
                if !streamed_supported {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        format!(
                            "当前数据库不支持流式写入大字段，单个文件请控制在 {} 以内",
                            fluxdb_core::format_byte_length(
                                fluxdb_core::BLOB_UPLOAD_STREAM_THRESHOLD_BYTES
                            )
                        ),
                    ));
                }
                // 大文件不进内存：一边读一边落到受控内容存储，提交时按块追加。
                stage_upload_file(editor, column, kind, &path, byte_length)?
            } else {
                let bytes = std::fs::read(&path)
                    .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
                CellUpdateValue::Value(file_contents_as_cell_value(kind, bytes)?)
            }
        }
    };

    edit_data_cell_value(editor, row, column, value)
}

/// 把文件字节转成该列可写的完整值。
///
/// 文本列必须是合法 UTF-8：非法字节按替换字符写进去是静默改数据，
/// 这里直接拒绝，让用户自己确认编码。
fn file_contents_as_cell_value(
    kind: fluxdb_core::LargeValueKind,
    bytes: Vec<u8>,
) -> fluxdb_core::Result<CellValue> {
    if matches!(kind, fluxdb_core::LargeValueKind::Binary) {
        return Ok(CellValue::Bytes(bytes));
    }
    String::from_utf8(bytes).map(CellValue::Text).map_err(|_| {
        Error::new(
            ErrorKind::Unsupported,
            "文件不是合法的 UTF-8 文本，无法写入文本字段",
        )
    })
}

/// 列类型上限 + 上传上限校验。两道限制任一超出都必须拒绝并说清原因：
/// 早拒绝好过暂存几百 MiB 后才在提交阶段失败。
fn reject_oversized_upload(
    editor: &DataEditorState,
    column: usize,
    kind: fluxdb_core::LargeValueKind,
    byte_length: u64,
) -> fluxdb_core::Result<()> {
    let limits = editor.page.as_ref().and_then(|page| {
        let column = page.columns.get(column)?;
        let type_name = column.type_name.as_deref()?;
        Some((
            column.name.clone(),
            type_name.to_string(),
            upload_type_max_bytes(kind, type_name),
        ))
    });
    if let Some((column_name, type_name, Some(max_bytes))) = limits
        && byte_length > max_bytes
    {
        return Err(Error::new(
            ErrorKind::Unsupported,
            format!(
                "上传失败：{column_name} 是 {type_name}，最多支持 {}，当前文件 {}",
                fluxdb_core::format_byte_length(max_bytes),
                fluxdb_core::format_byte_length(byte_length)
            ),
        ));
    }
    if byte_length > BINARY_FILE_UPLOAD_LIMIT {
        // 阈值之上本来就靠暂存 + 分块追加，这里拦的是「连暂存配额都装不下」的文件。
        return Err(Error::new(
            ErrorKind::Unsupported,
            format!(
                "上传文件超过 {} MB，超过单个大字段的暂存上限，请拆分或缩小文件后重试",
                BINARY_FILE_UPLOAD_LIMIT / 1024 / 1024
            ),
        ));
    }
    Ok(())
}

fn upload_type_max_bytes(kind: fluxdb_core::LargeValueKind, type_name: &str) -> Option<u64> {
    match kind {
        fluxdb_core::LargeValueKind::Binary => fluxdb_core::binary_type_max_bytes(type_name),
        _ => fluxdb_core::text_type_max_bytes(type_name),
    }
}

/// 把文件流式暂存到受控内容存储，返回带占位值（类型 + 大小）的流式写入引用。
///
/// 内存只占一个固定拷贝缓冲：文件多大都不影响进程占用。暂存内容由
/// [`fluxdb_core::StagedContent`] 的引用计数释放，改动集丢弃时自动清理。
fn stage_upload_file(
    editor: &DataEditorState,
    column: usize,
    kind: fluxdb_core::LargeValueKind,
    path: &std::path::Path,
    byte_length: u64,
) -> fluxdb_core::Result<CellUpdateValue> {
    let store = fluxdb_core::cell_content_store().ok_or_else(|| {
        Error::new(ErrorKind::Internal, "大字段内容存储未初始化，无法读写完整内容")
    })?;
    let mut file = std::fs::File::open(path)
        .map_err(|error| Error::new(ErrorKind::Internal, format!("打开文件失败：{error}")))?;
    let content_id = store.put_from_reader(
        crate::cell_content_store::UPLOAD_STAGING_OWNER,
        kind,
        &mut file,
    )?;
    let type_name = editor
        .page
        .as_ref()
        .and_then(|page| page.columns.get(column))
        .and_then(|column| column.type_name.clone())
        .unwrap_or_else(|| "blob".to_string());
    // 占位值只在界面/历史里出现，正文留在内容存储里，不能写成可回放的字面量。
    //
    // 文本也必须用 `LargeValue` 而不是 `Text`：`Text` 会被界面当成真实正文，
    // 于是给出编辑/复制按钮、长度算成占位串的字符数，用户还会把占位串当数据。
    // `LargeValue` 自带内容引用，详情面板能按块读出刚选的文件，复制/导出/编辑
    // 统一按「只是预览」拒绝。
    let placeholder = match kind {
        fluxdb_core::LargeValueKind::Binary => {
            CellValue::BinarySummary(fluxdb_core::BinaryCellSummary {
                type_name,
                is_null: false,
                byte_length,
                preview_hex: None,
            })
        }
        _ => CellValue::LargeValue(fluxdb_core::LargeCellValue {
            content_id: content_id.clone(),
            kind,
            byte_length: Some(byte_length),
            // 预览位只放类型名，表格里显示成 `LONGTEXT…（共 50.0 MB）`；
            // 真正的正文在内容存储里，打开详情按块读取。
            preview: type_name.to_ascii_uppercase(),
            preview_truncated: true,
        }),
    };
    Ok(CellUpdateValue::Stream(fluxdb_core::StreamedValue::new(
        std::sync::Arc::new(fluxdb_core::StagedContent::new(content_id, byte_length)),
        kind,
        placeholder,
    )))
}

/// 该单元格能否整值上传替换；返回要写入的内容种类。
///
/// 比旧实现宽：文本大字段（TEXT/LONGTEXT/CLOB…）也能上传替换，JSON 列和
/// 普通短文本以外的类型不放开（见 [`fluxdb_core::upload_replace_kind`]）。
fn ensure_upload_cell(
    editor: &DataEditorState,
    row: usize,
    column: usize,
) -> fluxdb_core::Result<fluxdb_core::LargeValueKind> {
    let page = editor
        .page
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    let type_name = page.columns.get(column).and_then(|column| column.type_name.as_deref());
    let value = cell_value_at(page, row, column)?;
    fluxdb_core::upload_replace_kind(type_name, &value).ok_or_else(|| {
        Error::new(
            ErrorKind::Unsupported,
            "当前单元格不支持上传替换，请使用文本或二进制大字段",
        )
    })
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
            insert_intents: None,
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
            insert_intents: None,
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
    let identity = original_editor_row_identity(editor, row)?;
    let page = editor
        .page
        .as_mut()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    page.rows
        .get(row)
        .ok_or_else(|| Error::new(ErrorKind::Internal, "行不存在"))?;
    editor.editing_cell = None;
    let changes = editor.changes.get_or_insert_with(|| DataChangeSet {
        object: editor.object.clone(),
        inserts: Vec::new(),
        updates: Vec::new(),
        deletes: Vec::new(),
        insert_intents: None,
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
    // 截断值以按需加载的完整文本为准：拿预览当原值既判不出「没改」，也会把片段写回。
    let original = match &current_value {
        CellValue::TruncatedText { .. } => CellValue::Text(detail_full_text(editor)?),
        // 超大字段只按内容引用分段只读：把预览当原值写回等于用片段覆盖真值。
        CellValue::LargeValue(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "超大字段不支持单元格编辑；请使用分段查看或流式保存",
            ));
        }
        other => other.clone(),
    };
    let next_value = cell_value_from_edit_text(&original, &editor.cell_detail_panel.edit_value)?;
    if next_value == original {
        // 内容没变：只退出编辑态，不产生一条把完整值重写回去的更新。
        editor.cell_detail_panel.mode = CellDetailMode::View;
        return Ok(());
    }
    set_cell_detail_value(editor, next_value)
}

/// 详情面板里已加载的完整文本；没有就说明预览还没换成完整值，不能编辑。
fn detail_full_text(editor: &DataEditorState) -> fluxdb_core::Result<String> {
    editor
        .cell_detail_panel
        .full_value
        .clone()
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Unsupported,
                "该单元格只加载了预览（截断的 CLOB），请先加载完整值再编辑",
            )
        })
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
        // 预览可以展示、不能编辑：编辑框里只有片段，保存时会被拒绝（见
        // `cell_value_from_edit_text`）。完整值需先走详情加载。
        CellValue::TruncatedText { preview, .. } => preview.clone(),
        // 同上：预览可展示、不可编辑。
        CellValue::LargeValue(large) => large.preview.clone(),
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
        // 二进制不提供就地编辑：编辑框里只有 64 字节 hex 预览，保存等于用预览覆盖整值。
        // 改内容请走「上传替换」，清空请走「设为 NULL」。
        CellValue::Bytes(_) | CellValue::BinarySummary(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "二进制字段不支持就地编辑，请使用上传替换或设为 NULL",
            ));
        }
        CellValue::Json(_) => CellValue::Json(value.to_string()),
        // 编辑框里只有预览片段，拿它生成新值等于用片段覆盖完整数据。
        CellValue::TruncatedText { .. } => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "该单元格只加载了预览（截断的 CLOB），请先加载完整值再编辑",
            ));
        }
        // 超大字段默认禁用编辑：整份内容不进 UI 进程，编辑框里只有预览。
        CellValue::LargeValue(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "超大字段不支持单元格编辑；请使用分段查看或流式保存",
            ));
        }
    };

    Ok(value)
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

    if values.is_empty() {
        // 无主键时保留完整原始值，不能用第一列冒充唯一身份。
        for (column, value) in page.columns.iter().zip(&row.values) {
            if matches!(value, CellValue::BinarySummary(_)) {
                return Err(Error::new(ErrorKind::Unsupported, "无唯一键且包含二进制摘要，无法可靠定位原始行"));
            }
            values.insert(column.name.clone(), value.clone());
        }
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
    value: CellUpdateValue,
) {
    if let Some(update) = changes
        .updates
        .iter_mut()
        .find(|update| update.identity == identity)
    {
        if let Some(cell) = update.cells.iter_mut().find(|cell| cell.column == column) {
            // 旧值（可能是上一份暂存内容）在这里被丢弃，引用计数归零时自动释放。
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

/// 根据已有草稿还原显示行对应的原始身份；改主键或连续改无键行也不能换成新值定位。
fn original_editor_row_identity(editor: &DataEditorState, row: usize) -> fluxdb_core::Result<RowIdentity> {
    let page = editor.page.as_ref().ok_or_else(|| Error::new(ErrorKind::Internal, "数据页未加载"))?;
    let current = page.rows.get(row).ok_or_else(|| Error::new(ErrorKind::Internal, "行不存在"))?;
    let original = editor.original_page.as_ref().unwrap_or(page);
    // 会话内运行时追加列（如单元格 BLOB 详情列）会使 original 与当前页列结构不同，
    // 整行相等比较失去列对齐前提（expected 比 current 少列）——此时退回按主键/首列身份定位，
    // 主键行的防漂移保护（草稿态不换新值）不受影响，无键行整行比较原语义不变。
    let columns_differ =
        original.columns.len() != page.columns.len()
            || original.columns.iter().any(|col| !page.columns.contains(col));
    if columns_differ {
        for (index, _) in original.rows.iter().enumerate() {
            let identity = row_identity(original, index)?;
            if page_contains_identity(page, &identity) {
                return Ok(identity);
            }
        }
        return Err(Error::new(ErrorKind::Query, "无法匹配原始行，请刷新后重试"));
    }
    for (index, original_row) in original.rows.iter().enumerate() {
        let identity = row_identity(original, index)?;
        let mut expected = original_row.clone();
        if let Some(update) = editor.changes.as_ref().and_then(|c| c.updates.iter().find(|u| u.identity == identity)) {
            for cell in &update.cells {
                if let Some(column) = original.columns.iter().position(|col| col.name == cell.column) {
                    // 只比占位值：流式写入的正文不在内存里，页面显示的也是占位值。
                    expected.values[column] = cell.value.placeholder().clone();
                }
            }
        }
        if &expected == current { return Ok(identity); }
    }
    Err(Error::new(ErrorKind::Query, "无法匹配原始行，请刷新后重试"))
}

/// PG 更新/删除带上读取时的原值，服务端可检测同主键行的并发修改。
///
/// 只加载了预览的值（超大字段、截断文本、二进制摘要）不能参与并发校验：
/// 页里保存的只是前缀/摘要，绑进 `WHERE` 永远匹配不到数据库里的完整内容，
/// 会把正常提交误报成“目标行已不存在”。无主键行的初始身份可能整行都是这类值，
/// 移除后若没有可比较的身份，必须明确拒绝，不能退化成无身份更新。
fn postgres_changes_with_original_values(changes: &DataChangeSet, page: &DataPage) -> fluxdb_core::Result<DataChangeSet> {
    let mut changes = changes.clone();
    for identity in changes.updates.iter_mut().map(|u| &mut u.identity).chain(changes.deletes.iter_mut()) {
        let candidates = page.rows.iter().filter(|row| identity.values.iter().all(|(name, value)|
            page.columns.iter().position(|col| &col.name == name).and_then(|index| row.values.get(index)) == Some(value))).collect::<Vec<_>>();
        if candidates.len() != 1 { return Err(Error::new(ErrorKind::Query, "原始行身份不唯一，已取消提交")); }
        for (col, value) in page.columns.iter().zip(&candidates[0].values) {
            if !matches!(value, CellValue::BinarySummary(_)) && !value.is_truncated() {
                identity.values.insert(col.name.clone(), value.clone());
            }
        }
        // 无主键行最初把整行值当身份，其中可能已有预览值；只保留能忠实比较的完整值。
        identity.values.retain(|_, value| !value.is_truncated());
        if identity.values.is_empty() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "行身份包含未完整加载的大字段，无法安全定位行；请刷新后重试或改用主键定位",
            ));
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod cell_detail_truncated_tests {
    use super::cell_value_edit_text;
    use super::cell_value_from_edit_text;
    use fluxdb_core::CellValue;

    fn truncated() -> CellValue {
        CellValue::TruncatedText {
            preview: "前 4000 字符".to_string(),
            total_chars: 100_000,
        }
    }

    /// 预览可以显示（否则详情面板空白），但绝不能变成可保存的完整值。
    #[test]
    fn preview_is_displayable_but_not_editable() {
        assert_eq!(cell_value_edit_text(&truncated()), "前 4000 字符");
        assert!(cell_value_from_edit_text(&truncated(), "改过的文本").is_err());
    }

    /// 完整文本照常编辑，不受截断保护影响。
    #[test]
    fn complete_text_round_trips() {
        let value = CellValue::Text("完整内容".to_string());
        let edited = cell_value_from_edit_text(&value, "新内容").expect("complete text is editable");
        assert_eq!(edited, CellValue::Text("新内容".to_string()));
    }
}

#[cfg(test)]
mod data_editor_upload_tests {
    include!("data_editor_upload_tests.rs");
}

#[cfg(test)]
mod postgres_original_value_identity_tests {
    use super::*;

    fn column(name: &str, primary_key: bool) -> Column {
        Column {
            name: name.to_string(),
            type_name: Some("text".to_string()),
            nullable: true,
            primary_key,
            comment: None,
        }
    }

    fn large_value() -> CellValue {
        CellValue::LargeValue(fluxdb_core::LargeCellValue {
            content_id: "content-1".to_string(),
            kind: fluxdb_core::LargeValueKind::Text,
            byte_length: Some(128 * 1024),
            preview: "preview".to_string(),
            preview_truncated: true,
        })
    }

    fn page(columns: Vec<Column>, values: Vec<CellValue>) -> DataPage {
        DataPage {
            columns,
            rows: vec![Row { values }],
            offset: 0,
            limit: 100,
            has_more: false,
            incomplete: None,
        }
    }

    fn update(identity: std::collections::BTreeMap<String, CellValue>) -> DataChangeSet {
        DataChangeSet {
            object: ObjectPath {
                connection_id: ConnectionId(1),
                database: Some("db".to_string()),
                schema: Some("public".to_string()),
                name: "t".to_string(),
                kind: ObjectKind::Table,
            },
            inserts: Vec::new(),
            updates: vec![RowUpdate {
                identity: RowIdentity { values: identity },
                cells: vec![CellUpdate::new("note", CellValue::Text("changed".to_string()))],
            }],
            deletes: Vec::new(),
            insert_intents: None,
        }
    }

    /// 有主键时，超大字段只有预览，不能进并发校验身份；其余完整列照常保留。
    #[test]
    fn skips_large_value_preview_from_optimistic_identity() {
        let page = page(
            vec![column("id", true), column("payload", false), column("note", false)],
            vec![CellValue::I64(1), large_value(), CellValue::Text("ok".to_string())],
        );
        let changes = update(std::collections::BTreeMap::from([(
            "id".to_string(),
            CellValue::I64(1),
        )]));

        let guarded = postgres_changes_with_original_values(&changes, &page).expect("identity");

        let identity = &guarded.updates[0].identity.values;
        assert_eq!(identity.get("id"), Some(&CellValue::I64(1)));
        assert_eq!(identity.get("note"), Some(&CellValue::Text("ok".to_string())));
        assert!(
            !identity.contains_key("payload"),
            "大字段预览不能作为 WHERE 身份，否则必然匹配不到完整值"
        );
    }

    /// 无主键时初始身份可能包含整行预览值：移除预览，保留仍可比较的完整值。
    #[test]
    fn no_pk_identity_drops_truncated_values_but_keeps_complete_ones() {
        let large = large_value();
        let page = page(
            vec![column("payload", false), column("note", false)],
            vec![large.clone(), CellValue::Text("ok".to_string())],
        );
        let changes = update(std::collections::BTreeMap::from([
            ("payload".to_string(), large),
            ("note".to_string(), CellValue::Text("ok".to_string())),
        ]));

        let guarded = postgres_changes_with_original_values(&changes, &page).expect("identity");

        let identity = &guarded.updates[0].identity.values;
        assert_eq!(identity.get("note"), Some(&CellValue::Text("ok".to_string())));
        assert!(!identity.contains_key("payload"));
    }

    /// 无主键行身份只剩预览值时不能继续生成无条件 UPDATE，必须明确拒绝。
    #[test]
    fn no_pk_identity_with_only_truncated_values_is_rejected() {
        let large = large_value();
        let page = page(vec![column("payload", false)], vec![large.clone()]);
        let changes = update(std::collections::BTreeMap::from([(
            "payload".to_string(),
            large,
        )]));

        let error = postgres_changes_with_original_values(&changes, &page)
            .expect_err("不能把预览绑定成行身份");

        assert_eq!(error.kind, ErrorKind::Unsupported);
        assert!(error.message.contains("大字段"), "{}", error.message);
    }
}
