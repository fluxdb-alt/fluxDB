// Included in crate-root scope by ../data_editor_model.rs; grouped by table export formatting.

impl DataRowExportFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Json => "JSON",
            Self::Markdown => "Markdown",
            Self::SqlInsert => "SQL INSERT",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
            Self::Markdown => "md",
            Self::SqlInsert => "sql",
        }
    }
}

fn default_data_export_directory() -> PathBuf {
    // 默认导出目录由 fluxdb-storage 统一解析（AI-01，方案 §12.4）：
    // 平台下载目录（Known Folder / XDG）→ 用户主目录 → 临时目录；
    // 不再回退到进程工作目录/安装目录（Windows 下可能是 System32 等不可写位置）。
    fluxdb_storage::FileStorage::default_download_dir()
}

fn data_row_export_suggested_name(object: &ObjectPath, format: DataRowExportFormat) -> String {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    format!(
        "{}-{timestamp}.{}",
        safe_data_export_filename_segment(&object.name),
        format.extension()
    )
}

fn table_data_export_suggested_name(
    object: &ObjectPath,
    format: TableDataExportFormat,
) -> String {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    format!(
        "{}-{timestamp}.{}",
        safe_data_export_filename_segment(&object.name),
        format.extension()
    )
}

fn safe_data_export_path(path: PathBuf, format: DataRowExportFormat) -> PathBuf {
    safe_data_export_path_with_extension(path, format.extension())
}

fn safe_data_export_path_with_extension(mut path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension(extension);
    }
    path
}

fn safe_table_data_export_path(path: PathBuf, format: TableDataExportFormat) -> PathBuf {
    safe_data_export_path_with_extension(path, format.extension())
}

fn safe_data_export_filename_segment(value: &str) -> String {
    let mut segment = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else if ch.is_whitespace() {
                '_'
            } else {
                '_'
            }
        })
        .collect::<String>();
    while segment.contains("__") {
        segment = segment.replace("__", "_");
    }
    let segment = segment.trim_matches(['_', '.', '-']).to_string();
    if segment.is_empty() {
        "data".to_string()
    } else {
        segment
    }
}

fn write_data_row_export_file(
    path: &Path,
    format: DataRowExportFormat,
    object: Option<&ObjectPath>,
    rows: &[Vec<RowFieldSnapshot>],
    db_kind: DatabaseKind,
) -> io::Result<()> {
    let file = fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    write_data_row_export(&mut writer, format, object, rows, db_kind)?;
    writer.flush()
}

fn write_data_selection_export_file(path: &Path, export: &DataTableSelectionExport) -> io::Result<()> {
    let file = fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(export.text.as_bytes())?;
    writer.flush()
}

struct TableDataExportWriter {
    writer: BufWriter<fs::File>,
    format: TableDataExportFormat,
    object: ObjectPath,
    fields: Vec<String>,
    /// 查询结果导出按列下标取值：结果列允许重名，不能再用列名匹配。
    source_indexes: Option<Vec<usize>>,
    wrote_rows: bool,
    /// 方言：SQL INSERT 字面量/标识符按此渲染（PG 双引号 + `'\x..'::bytea`）。
    db_kind: DatabaseKind,
}

impl TableDataExportWriter {
    fn create(
        path: &Path,
        format: TableDataExportFormat,
        object: ObjectPath,
        fields: Vec<String>,
        db_kind: DatabaseKind,
    ) -> io::Result<Self> {
        Self::create_with_source_indexes(path, format, object, fields, None, db_kind)
    }

    /// 按列下标导出（查询结果）：`fields` 与 `source_indexes` 一一对应。
    fn create_indexed(
        path: &Path,
        format: TableDataExportFormat,
        object: ObjectPath,
        fields: Vec<String>,
        source_indexes: Vec<usize>,
        db_kind: DatabaseKind,
    ) -> io::Result<Self> {
        Self::create_with_source_indexes(
            path,
            format,
            object,
            fields,
            Some(source_indexes),
            db_kind,
        )
    }

    fn create_with_source_indexes(
        path: &Path,
        format: TableDataExportFormat,
        object: ObjectPath,
        fields: Vec<String>,
        source_indexes: Option<Vec<usize>>,
        db_kind: DatabaseKind,
    ) -> io::Result<Self> {
        let file = fs::File::create(path)?;
        let mut export = Self {
            writer: BufWriter::new(file),
            format,
            object,
            fields,
            source_indexes,
            wrote_rows: false,
            db_kind,
        };
        export.write_header()?;
        Ok(export)
    }

    fn write_header(&mut self) -> io::Result<()> {
        match self.format {
            TableDataExportFormat::Sql => Ok(()),
            TableDataExportFormat::Txt => {
                writeln!(
                    self.writer,
                    "{}",
                    self.fields
                        .iter()
                        .map(|field| tsv_cell(field))
                        .collect::<Vec<_>>()
                        .join("\t")
                )
            }
            TableDataExportFormat::Csv => {
                write_csv_bom(&mut self.writer)?;
                writeln!(
                    self.writer,
                    "{}",
                    self.fields
                        .iter()
                        .map(|field| csv_cell(field))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            TableDataExportFormat::Json => self.writer.write_all(b"[\n"),
            TableDataExportFormat::Xml => {
                writeln!(self.writer, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
                writeln!(self.writer, "<rows>")
            }
        }
    }

    fn write_page(&mut self, page: &DataPage) -> io::Result<u64> {
        let indexes = match &self.source_indexes {
            Some(source_indexes) => source_indexes
                .iter()
                .enumerate()
                .filter_map(|(field_index, source_index)| {
                    page.columns.get(*source_index).map(|_| {
                        (
                            self.fields
                                .get(field_index)
                                .cloned()
                                .unwrap_or_else(|| source_index.to_string()),
                            *source_index,
                        )
                    })
                })
                .collect::<Vec<_>>(),
            None => self
                .fields
                .iter()
                .filter_map(|field| {
                    page.columns
                        .iter()
                        .position(|column| column.name == *field)
                        .map(|index| (field.clone(), index))
                })
                .collect::<Vec<_>>(),
        };
        let mut written = 0;
        for row in &page.rows {
            let fields = indexes
                .iter()
                .enumerate()
                .map(|(field_index, (name, source_index))| {
                    let column = &page.columns[*source_index];
                    RowFieldSnapshot {
                        index: field_index + 1,
                        name: name.clone(),
                        type_name: column
                            .type_name
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                        primary_key: column.primary_key,
                        comment: column.comment.clone(),
                        value: row.values.get(*source_index).cloned().unwrap_or(CellValue::Null),
                    }
                })
                .collect::<Vec<_>>();
            self.write_row(fields.as_slice())?;
            written += 1;
        }
        Ok(written)
    }

    fn write_row(&mut self, fields: &[RowFieldSnapshot]) -> io::Result<()> {
        reject_truncated_fields(fields)?;
        match self.format {
            TableDataExportFormat::Sql => writeln!(
                self.writer,
                "{}",
                row_insert_sql(&self.object, fields, false, self.db_kind)
            )?,
            TableDataExportFormat::Txt => writeln!(
                self.writer,
                "{}",
                fields
                    .iter()
                    .map(|field| tsv_cell(cell_value_label(&field.value).as_str()))
                    .collect::<Vec<_>>()
                    .join("\t")
            )?,
            TableDataExportFormat::Csv => writeln!(
                self.writer,
                "{}",
                fields
                    .iter()
                    .map(|field| csv_cell(cell_value_label(&field.value).as_str()))
                    .collect::<Vec<_>>()
                    .join(",")
            )?,
            TableDataExportFormat::Json => {
                if self.wrote_rows {
                    self.writer.write_all(b",\n")?;
                }
                let row = row_json_value(fields);
                let text = serde_json::to_string_pretty(&row).unwrap_or_default();
                for line in text.lines() {
                    self.writer.write_all(b"  ")?;
                    self.writer.write_all(line.as_bytes())?;
                    self.writer.write_all(b"\n")?;
                }
            }
            TableDataExportFormat::Xml => {
                writeln!(self.writer, "  <row>")?;
                for field in fields {
                    writeln!(
                        self.writer,
                        "    <field name=\"{}\">{}</field>",
                        xml_escape(field.name.as_str()),
                        xml_escape(cell_value_label(&field.value).as_str())
                    )?;
                }
                writeln!(self.writer, "  </row>")?;
            }
        }
        self.wrote_rows = true;
        Ok(())
    }

    fn finish(mut self) -> io::Result<()> {
        match self.format {
            TableDataExportFormat::Json => {
                if self.wrote_rows {
                    self.writer.write_all(b"]\n")?;
                } else {
                    self.writer.write_all(b"]\n")?;
                }
            }
            TableDataExportFormat::Xml => {
                writeln!(self.writer, "</rows>")?;
            }
            _ => {}
        }
        self.writer.flush()
    }
}

/// 导出前拒绝只加载了预览的文本。
///
/// 预览是片段：写进导出文件等于交付一份静默截断的数据，而导出本身看起来「成功」。
/// 这里直接失败并提示改用完整读取，不把半截内容混进结果。
fn reject_truncated_fields(fields: &[RowFieldSnapshot]) -> io::Result<()> {
    if let Some(field) = fields.iter().find(|field| field.value.is_truncated()) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            tr("app.data_export.error.truncated_value").replace("%{column}", &field.name),
        ));
    }
    Ok(())
}

/// 结果列允许重名（如 `SELECT a.id, b.id`）：按出现顺序追加 `_2`、`_3` 生成唯一表头。
///
/// 只保证导出列内部唯一，不改写查询结果本身；`id_2` 已存在时继续递增后缀。
fn unique_data_export_labels<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut used = std::collections::HashSet::new();
    names
        .into_iter()
        .map(|name| {
            let mut label = name.to_string();
            let mut suffix = 2;
            while !used.insert(label.clone()) {
                label = format!("{name}_{suffix}");
                suffix += 1;
            }
            label
        })
        .collect()
}

fn write_data_row_export<W: Write>(
    writer: &mut W,
    format: DataRowExportFormat,
    object: Option<&ObjectPath>,
    rows: &[Vec<RowFieldSnapshot>],
    db_kind: DatabaseKind,
) -> io::Result<()> {
    for fields in rows {
        reject_truncated_fields(fields)?;
    }
    match format {
        DataRowExportFormat::Csv => write_data_row_csv_export(writer, rows),
        DataRowExportFormat::Json => write_data_row_json_export(writer, rows),
        DataRowExportFormat::Markdown => write_data_row_markdown_export(writer, rows),
        DataRowExportFormat::SqlInsert => {
            let Some(object) = object else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    tr("app.data_export.error.insert_needs_table").as_ref(),
                ));
            };
            write_data_row_insert_export(writer, object, rows, db_kind)
        }
    }
}

fn write_data_row_csv_export<W: Write>(
    writer: &mut W,
    rows: &[Vec<RowFieldSnapshot>],
) -> io::Result<()> {
    let Some(fields) = rows.first() else {
        return Ok(());
    };
    write_csv_bom(writer)?;
    writeln!(
        writer,
        "{}",
        fields
            .iter()
            .map(|field| csv_cell(field.name.as_str()))
            .collect::<Vec<_>>()
            .join(",")
    )?;
    for fields in rows {
        writeln!(
            writer,
            "{}",
            fields
                .iter()
                .map(|field| csv_cell(cell_value_label(&field.value).as_str()))
                .collect::<Vec<_>>()
                .join(",")
        )?;
    }
    Ok(())
}

fn write_data_row_json_export<W: Write>(
    writer: &mut W,
    rows: &[Vec<RowFieldSnapshot>],
) -> io::Result<()> {
    writer.write_all(b"[\n")?;
    for (index, fields) in rows.iter().enumerate() {
        if index > 0 {
            writer.write_all(b",\n")?;
        }
        let row = row_json_text(fields.as_slice());
        for line in row.lines() {
            writer.write_all(b"  ")?;
            writer.write_all(line.as_bytes())?;
            writer.write_all(b"\n")?;
        }
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_data_row_markdown_export<W: Write>(
    writer: &mut W,
    rows: &[Vec<RowFieldSnapshot>],
) -> io::Result<()> {
    let Some(fields) = rows.first() else {
        return Ok(());
    };
    writeln!(
        writer,
        "| {} |",
        fields
            .iter()
            .map(|field| markdown_cell(field.name.as_str()))
            .collect::<Vec<_>>()
            .join(" | ")
    )?;
    writeln!(
        writer,
        "| {} |",
        std::iter::repeat("---")
            .take(fields.len())
            .collect::<Vec<_>>()
            .join(" | ")
    )?;
    for fields in rows {
        writeln!(
            writer,
            "| {} |",
            fields
                .iter()
                .map(|field| markdown_cell(cell_value_label(&field.value).as_str()))
                .collect::<Vec<_>>()
                .join(" | ")
        )?;
    }
    Ok(())
}

fn write_data_row_insert_export<W: Write>(
    writer: &mut W,
    object: &ObjectPath,
    rows: &[Vec<RowFieldSnapshot>],
    db_kind: DatabaseKind,
) -> io::Result<()> {
    for fields in rows {
        writeln!(writer, "{}", row_insert_sql(object, fields.as_slice(), false, db_kind))?;
    }
    Ok(())
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// 写 CSV 的 UTF-8 BOM（EF BB BF）。
///
/// 跨平台适配 §12.3.1：无 BOM 的 UTF-8 CSV 在 Windows 简体中文环境的 Excel 按 ANSI(CP936)
/// 打开，中文全部乱码；带 BOM 后 Excel 能识别为 UTF-8。默认统一带 BOM，标签/文档已说明；
/// 若有工具不接受 BOM 再作为独立选项决策。
fn write_csv_bom<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(b"\xEF\xBB\xBF")
}

fn markdown_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "<br>")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod unique_data_export_labels_tests {
    use super::unique_data_export_labels;

    #[test]
    fn deduplicates_duplicate_column_names() {
        assert_eq!(
            unique_data_export_labels(["id", "name", "id", "id"]),
            vec!["id", "name", "id_2", "id_3"]
        );
    }

    #[test]
    fn skips_suffix_already_taken_by_real_column() {
        assert_eq!(
            unique_data_export_labels(["id", "id_2", "id"]),
            vec!["id", "id_2", "id_3"]
        );
    }
}

#[cfg(test)]
mod indexed_data_export_tests {
    use super::*;

    #[test]
    fn writes_selected_duplicate_columns_by_position() {
        let path = std::env::temp_dir().join(format!(
            "fluxdb-export-test-{}-{}.csv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        let page = DataPage {
            columns: vec![
                GdbColumn {
                    name: "id".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
                GdbColumn {
                    name: "id".to_string(),
                    type_name: None,
                    nullable: false,
                    primary_key: false,
                    comment: None,
                },
            ],
            rows: vec![Row {
                values: vec![CellValue::I64(1), CellValue::I64(2)],
            }],
            offset: 0,
            limit: 10,
            has_more: false,
            incomplete: None,
        };
        let object = ObjectPath {
            connection_id: ConnectionId(1),
            database: None,
            schema: None,
            name: "query-result".to_string(),
            kind: ObjectKind::Table,
        };
        let mut writer = TableDataExportWriter::create_indexed(
            &path,
            TableDataExportFormat::Csv,
            object,
            vec!["id".to_string(), "id_2".to_string()],
            vec![0, 1],
            DatabaseKind::MySql,
        )
        .expect("create export writer");
        assert_eq!(writer.write_page(&page).expect("write page"), 1);
        writer.finish().expect("finish export");
        let text = std::fs::read_to_string(&path).expect("read exported csv");
        let _ = std::fs::remove_file(&path);
        assert!(text.contains("id,id_2"), "header: {text}");
        assert!(text.contains("1,2"), "row: {text}");
    }
}
