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
    if let Some(home) = std::env::var_os("HOME") {
        let downloads = PathBuf::from(home).join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
    }

    std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
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
) -> io::Result<()> {
    let file = fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    write_data_row_export(&mut writer, format, object, rows)?;
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
    wrote_rows: bool,
}

impl TableDataExportWriter {
    fn create(
        path: &Path,
        format: TableDataExportFormat,
        object: ObjectPath,
        fields: Vec<String>,
    ) -> io::Result<Self> {
        let file = fs::File::create(path)?;
        let mut export = Self {
            writer: BufWriter::new(file),
            format,
            object,
            fields,
            wrote_rows: false,
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
        let indexes = self
            .fields
            .iter()
            .filter_map(|field| {
                page.columns
                    .iter()
                    .position(|column| column.name == *field)
                    .map(|index| (field.clone(), index))
            })
            .collect::<Vec<_>>();
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
        match self.format {
            TableDataExportFormat::Sql => writeln!(
                self.writer,
                "{}",
                row_insert_sql(&self.object, fields, false)
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

fn write_data_row_export<W: Write>(
    writer: &mut W,
    format: DataRowExportFormat,
    object: Option<&ObjectPath>,
    rows: &[Vec<RowFieldSnapshot>],
) -> io::Result<()> {
    match format {
        DataRowExportFormat::Csv => write_data_row_csv_export(writer, rows),
        DataRowExportFormat::Json => write_data_row_json_export(writer, rows),
        DataRowExportFormat::Markdown => write_data_row_markdown_export(writer, rows),
        DataRowExportFormat::SqlInsert => {
            let Some(object) = object else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "SQL INSERT 导出需要表对象信息",
                ));
            };
            write_data_row_insert_export(writer, object, rows)
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
) -> io::Result<()> {
    for fields in rows {
        writeln!(writer, "{}", row_insert_sql(object, fields.as_slice(), false))?;
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
