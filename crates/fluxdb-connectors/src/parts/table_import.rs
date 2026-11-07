use fluxdb_core::{
    ParameterizedQueryRequest, QueryExecutionOptions, QueryMode, SqlParameter, TableDataImport,
    TableImportColumnMapping, TableImportFormat, TableImportMode, TableImportPreview,
    TableImportPreviewRequest, TableImportProgress, TableImportReport, TableImportRequest,
    TableImportTargetColumn,
};
use std::{
    collections::BTreeSet,
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

/// 单批写入行数上限，避免单条 INSERT 参数过多；行数本身不再设硬上限。
const TABLE_IMPORT_BATCH_ROWS: usize = 500;
/// 预览最多读取的行数。
const TABLE_IMPORT_PREVIEW_ROWS: usize = 50;

pub fn table_data_import(kind: DatabaseKind) -> fluxdb_core::Result<Box<dyn TableDataImport>> {
    match kind {
        DatabaseKind::MySql => Ok(Box::new(MySqlTableImport)),
        DatabaseKind::TiDb => Ok(Box::new(TiDbTableImport)),
        DatabaseKind::MariaDb => Ok(Box::new(MariaDbTableImport)),
        DatabaseKind::Postgres | DatabaseKind::Kingbase => Ok(Box::new(PostgresTableImport)),
        DatabaseKind::Sqlite => Ok(Box::new(SqliteTableImport)),
        DatabaseKind::Dameng => Ok(Box::new(DamengTableImport)),
        DatabaseKind::SqlServer => Ok(Box::new(SqlServerTableImport)),
        DatabaseKind::Oracle => Ok(Box::new(OracleTableImport)),
        DatabaseKind::Redis | DatabaseKind::MongoDb => Err(Error::new(
            ErrorKind::Unsupported,
            "当前连接类型没有可导入的表数据",
        )),
    }
}

struct MySqlTableImport;
struct TiDbTableImport;
struct MariaDbTableImport;
struct PostgresTableImport;
struct SqliteTableImport;
struct DamengTableImport;
struct SqlServerTableImport;
struct OracleTableImport;

impl TableDataImport for MySqlTableImport {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MySql
    }

    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> fluxdb_core::Result<TableImportReport> {
        import_relational(DatabaseKind::MySql, request, cancel, progress)
    }

    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> fluxdb_core::Result<TableImportPreview> {
        preview_relational(DatabaseKind::MySql, request)
    }
}

impl TableDataImport for TiDbTableImport {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::TiDb
    }

    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> fluxdb_core::Result<TableImportReport> {
        import_relational(DatabaseKind::TiDb, request, cancel, progress)
    }

    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> fluxdb_core::Result<TableImportPreview> {
        preview_relational(DatabaseKind::TiDb, request)
    }
}

impl TableDataImport for MariaDbTableImport {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::MariaDb
    }

    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> fluxdb_core::Result<TableImportReport> {
        import_relational(DatabaseKind::MariaDb, request, cancel, progress)
    }

    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> fluxdb_core::Result<TableImportPreview> {
        preview_relational(DatabaseKind::MariaDb, request)
    }
}

impl TableDataImport for PostgresTableImport {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Postgres
    }

    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> fluxdb_core::Result<TableImportReport> {
        import_relational(DatabaseKind::Postgres, request, cancel, progress)
    }

    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> fluxdb_core::Result<TableImportPreview> {
        preview_relational(DatabaseKind::Postgres, request)
    }
}

impl TableDataImport for SqliteTableImport {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Sqlite
    }

    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> fluxdb_core::Result<TableImportReport> {
        import_relational(DatabaseKind::Sqlite, request, cancel, progress)
    }

    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> fluxdb_core::Result<TableImportPreview> {
        preview_relational(DatabaseKind::Sqlite, request)
    }
}

impl TableDataImport for DamengTableImport {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Dameng
    }

    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> fluxdb_core::Result<TableImportReport> {
        import_relational(DatabaseKind::Dameng, request, cancel, progress)
    }

    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> fluxdb_core::Result<TableImportPreview> {
        preview_relational(DatabaseKind::Dameng, request)
    }
}

impl TableDataImport for OracleTableImport {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Oracle
    }

    fn import(
        &self,
        request: &TableImportRequest,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(TableImportProgress),
    ) -> fluxdb_core::Result<TableImportReport> {
        import_relational(DatabaseKind::Oracle, request, cancel, progress)
    }

    fn preview(
        &self,
        request: &TableImportPreviewRequest,
    ) -> fluxdb_core::Result<TableImportPreview> {
        preview_relational(DatabaseKind::Oracle, request)
    }
}

include!("table_import/sqlserver.rs");

#[derive(Debug)]
struct ParsedTable {
    columns: Vec<String>,
    rows: Vec<Vec<CellValue>>,
}

fn import_relational(
    kind: DatabaseKind,
    request: &TableImportRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(TableImportProgress),
) -> fluxdb_core::Result<TableImportReport> {
    if request.config.kind != kind {
        return Err(Error::new(ErrorKind::Internal, "导入执行器与连接类型不匹配"));
    }
    if request.object.kind != ObjectKind::Table {
        return Err(Error::new(ErrorKind::Unsupported, "仅表对象支持数据导入"));
    }
    check_cancel(cancel)?;
    progress(TableImportProgress {
        stage: "读取".into(),
        message: format!("正在读取 {} 文件", request.format.label()),
    });
    let parsed = parse_import_file(request, cancel, progress)?;
    if parsed.rows.is_empty() {
        return Ok(TableImportReport {
            total_rows: 0,
            imported_rows: 0,
            warnings: Vec::new(),
        });
    }
    let connector = connector_for(&request.config)?;
    let target_columns = connector.list_completion_columns(
        request.object.database.as_deref(),
        request.object.schema.as_deref(),
        &request.object.name,
    )?;
    if target_columns.is_empty() {
        return Err(Error::new(ErrorKind::Query, "目标表没有可导入字段"));
    }
    let mapping = map_import_columns(
        &parsed.columns,
        &target_columns,
        &request.column_mapping,
    )?;

    let ddl = connector.table_ddl(&request.object).ok();
    let (excluded, identity_columns) = ddl_import_column_flags(kind, ddl.as_deref());
    let mut warnings = Vec::new();
    let mut selected = Vec::new();
    for (import_index, target_index) in mapping {
        let Some(target_index) = target_index else {
            continue;
        };
        let column = &target_columns[target_index];
        let upper = column.name.to_ascii_uppercase();
        if excluded.contains(&upper) {
            warnings.push(format!("字段 {} 由数据库生成，已跳过", column.name));
            continue;
        }
        selected.push((import_index, target_index));
    }
    if selected.is_empty() {
        return Err(Error::new(ErrorKind::Query, "导入文件没有可写入字段"));
    }
    check_cancel(cancel)?;
    let table = import_qualified_table(kind, request)?;
    let column_list = selected
        .iter()
        .map(|(_, target_index)| {
            import_quote_identifier(kind, &target_columns[*target_index].name)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let overriding = if kind == DatabaseKind::Postgres
        && selected.iter().any(|(_, target_index)| {
            identity_columns.contains(&target_columns[*target_index].name.to_ascii_uppercase())
        })
    {
        " OVERRIDING SYSTEM VALUE"
    } else {
        ""
    };
    if request.options.mode == TableImportMode::Truncate {
        // MySQL/达梦的 TRUNCATE 是 DDL 且隐式提交，失败无法回滚；
        // 因此必须在确认文件可解析且存在可写列之后再清空。
        progress(TableImportProgress {
            stage: "清空".into(),
            message: format!("正在清空 {table}"),
        });
        check_cancel(cancel)?;
        let truncate = match kind {
            DatabaseKind::Sqlite => format!("DELETE FROM {table}"),
            _ => format!("TRUNCATE TABLE {table}"),
        };
        let result = connector.execute(&QueryRequest {
            connection_id: request.config.id,
            database: request.object.database.clone(),
            schema: request.object.schema.clone(),
            session_id: None,
            text: truncate,
            mode: QueryMode::All,
            options: QueryExecutionOptions::default(),
        })?;
        if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
            return Err(Error::new(ErrorKind::Query, summary.message.clone()));
        }
    }

    let total_rows = parsed.rows.len();
    let total_batches = total_rows.div_ceil(TABLE_IMPORT_BATCH_ROWS);
    let mut imported_rows = 0u64;
    for (batch_index, chunk) in parsed.rows.chunks(TABLE_IMPORT_BATCH_ROWS).enumerate() {
        check_cancel(cancel)?;
        let written = (batch_index * TABLE_IMPORT_BATCH_ROWS + chunk.len()).min(total_rows);
        progress(TableImportProgress {
            stage: "写入".into(),
            message: format!(
                "第 {}/{} 批，正在写入第 {}-{} 行（共 {} 行）",
                batch_index + 1,
                total_batches,
                written.saturating_sub(chunk.len()) + 1,
                written,
                total_rows,
            ),
        });
        let (sql, parameters) = build_import_batch_sql(
            kind,
            &table,
            &column_list,
            overriding,
            &selected,
            &target_columns,
            chunk,
        )?;
        let result = connector.execute_parameterized(&ParameterizedQueryRequest {
            request: QueryRequest {
                connection_id: request.config.id,
                database: request.object.database.clone(),
                schema: request.object.schema.clone(),
                session_id: None,
                text: sql,
                mode: QueryMode::All,
                options: QueryExecutionOptions::default(),
            },
            parameters,
        })?;
        if let Some(summary) = result.summaries.iter().find(|summary| !summary.success) {
            return Err(Error::new(ErrorKind::Query, summary.message.clone()));
        }
        imported_rows += chunk.len() as u64;
    }
    Ok(TableImportReport {
        total_rows: total_rows as u64,
        imported_rows,
        warnings,
    })
}

/// 解析有限行样本并读取目标表字段；不写数据库，用于导入前预览。
fn preview_relational(
    kind: DatabaseKind,
    request: &TableImportPreviewRequest,
) -> fluxdb_core::Result<TableImportPreview> {
    if request.config.kind != kind {
        return Err(Error::new(ErrorKind::Internal, "预览执行器与连接类型不匹配"));
    }
    let parsed = parse_import_source(
        &request.source,
        request.format,
        &request.options,
        &AtomicBool::new(false),
        &mut |_| {},
    )?;
    let target_columns = connector_for(&request.config)?
        .list_completion_columns(
            request.object.database.as_deref(),
            request.object.schema.as_deref(),
            &request.object.name,
        )?
        .into_iter()
        .map(|column| TableImportTargetColumn {
            name: column.name,
            type_name: column.type_name,
            nullable: column.nullable,
        })
        .collect::<Vec<_>>();
    let truncated = parsed.rows.len() > TABLE_IMPORT_PREVIEW_ROWS;
    let rows = parsed
        .rows
        .iter()
        .take(TABLE_IMPORT_PREVIEW_ROWS)
        .map(|row| row.iter().map(preview_cell_text).collect::<Vec<_>>())
        .collect();
    Ok(TableImportPreview {
        format: request.format.label().to_string(),
        encoding: if request.options.encoding.trim().is_empty() {
            "UTF-8".to_string()
        } else {
            request.options.encoding.trim().to_ascii_uppercase()
        },
        columns: parsed.columns,
        rows,
        truncated,
        target_columns,
    })
}

fn preview_cell_text(value: &CellValue) -> String {
    match value {
        CellValue::Null => "(NULL)".to_string(),
        CellValue::Bool(value) => value.to_string(),
        CellValue::I64(value) => value.to_string(),
        CellValue::F64(value) => value.to_string(),
        CellValue::Text(value) | CellValue::Json(value) => value.clone(),
        CellValue::TruncatedText { total_chars, .. } => format!("<截断文本：共 {total_chars} 字符>"),
        CellValue::LargeValue(large) => match large.byte_length {
            Some(length) => format!("<超大字段：共 {length} 字节>"),
            None => "<超大字段：大小未知>".to_string(),
        },
        CellValue::Bytes(value) => format!("<{} 字节>", value.len()),
        CellValue::BinarySummary(_) => "<二进制>".to_string(),
    }
}

/// 构造单批 INSERT；批大小受 `TABLE_IMPORT_BATCH_ROWS` 约束，避免参数过多。
fn build_import_batch_sql(
    kind: DatabaseKind,
    table: &str,
    column_list: &str,
    overriding: &str,
    selected: &[(usize, usize)],
    target_columns: &[fluxdb_core::CompletionColumn],
    rows: &[Vec<CellValue>],
) -> fluxdb_core::Result<(String, Vec<SqlParameter>)> {
    let mut parameters = Vec::with_capacity(selected.len() * rows.len());
    // Oracle 不支持 `VALUES (..),(..)`；用 `INSERT ALL ... SELECT 1 FROM DUAL`
    // 把整批做成一条语句，绑定仍是唯一的 `:N`。
    if kind == DatabaseKind::Oracle {
        let mut into_clauses = Vec::with_capacity(rows.len());
        for row in rows {
            let mut placeholders = Vec::with_capacity(selected.len());
            for (import_index, target_index) in selected {
                let value = row.get(*import_index).cloned().unwrap_or(CellValue::Null);
                let parameter =
                    import_sql_parameter_for_column(&value, &target_columns[*target_index])?;
                parameters.push(parameter);
                placeholders.push(format!(":{}", parameters.len()));
            }
            into_clauses.push(format!(
                " INTO {table} ({column_list}) VALUES ({})",
                placeholders.join(", ")
            ));
        }
        return Ok((
            format!("INSERT ALL{}{overriding} SELECT 1 FROM DUAL", into_clauses.join("")),
            parameters,
        ));
    }
    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let mut placeholders = Vec::with_capacity(selected.len());
        for (import_index, target_index) in selected {
            let value = row.get(*import_index).cloned().unwrap_or(CellValue::Null);
            let parameter = import_sql_parameter_for_column(&value, &target_columns[*target_index])?;
            placeholders.push(match kind {
                DatabaseKind::Postgres => {
                    parameters.push(parameter);
                    let index = parameters.len();
                    let column = &target_columns[*target_index];
                    if matches!(value, CellValue::Null)
                        || pg_import_needs_text_cast(&value, column)
                    {
                        format!(
                            "CAST(CAST(${index} AS text) AS {})",
                            column.type_name.as_deref().unwrap_or("text")
                        )
                    } else {
                        format!("${index}")
                    }
                }
                DatabaseKind::SqlServer => {
                    let column = &target_columns[*target_index];
                    if matches!(value, CellValue::Null)
                        && fluxdb_core::is_binary_type_name(
                            column.type_name.as_deref().unwrap_or(""),
                        )
                    {
                        // 无类型 NULL 参数默认声明为 nvarchar；与同一 VALUES 中的
                        // varbinary 参数混用时 SQL Server 会拒绝隐式转换（257）。
                        "NULL".to_string()
                    } else {
                        parameters.push(parameter);
                        format!("@P{}", parameters.len())
                    }
                }
                _ => {
                    parameters.push(parameter);
                    "?".to_string()
                }
            });
        }
        values.push(format!("({})", placeholders.join(", ")));
    }
    Ok((
        format!(
            "INSERT INTO {table} ({column_list}){overriding} VALUES {}",
            values.join(", ")
        ),
        parameters,
    ))
}

fn parse_import_file(
    request: &TableImportRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(TableImportProgress),
) -> fluxdb_core::Result<ParsedTable> {
    parse_import_source(
        &request.source,
        request.format,
        &request.options,
        cancel,
        progress,
    )
}

/// 统一解析入口：预览与正式导入共用同一套规则，避免两边行为不一致。
fn parse_import_source(
    source: &std::path::Path,
    format: TableImportFormat,
    options: &fluxdb_core::TableImportOptions,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(TableImportProgress),
) -> fluxdb_core::Result<ParsedTable> {
    let metadata = fs::metadata(source)
        .map_err(|error| Error::new(ErrorKind::Internal, format!("读取导入文件失败：{error}")))?;
    if !metadata.is_file() {
        return Err(Error::new(ErrorKind::Internal, "导入源不是普通文件"));
    }
    let bytes = fs::read(source)
        .map_err(|error| Error::new(ErrorKind::Internal, format!("读取导入文件失败：{error}")))?;
    let text = decode_import_text(&bytes, &options.encoding)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match format {
        TableImportFormat::Txt => {
            parse_delimited(text, options.delimiter_byte(), options, cancel, progress)
        }
        TableImportFormat::Csv => parse_delimited(text, b',', options, cancel, progress),
        TableImportFormat::Json => parse_json(text, cancel, progress),
        TableImportFormat::Xml => parse_xml(text, cancel, progress),
    }
}

/// 按指定编码解码导入文件；空编码按 UTF-8 严格解码。
fn decode_import_text(bytes: &[u8], encoding: &str) -> fluxdb_core::Result<String> {
    let encoding = encoding.trim();
    if encoding.is_empty()
        || encoding.eq_ignore_ascii_case("utf-8")
        || encoding.eq_ignore_ascii_case("utf8")
    {
        return std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| Error::new(ErrorKind::Query, "导入文件必须是 UTF-8 文本"));
    }
    let encoding = encoding_rs::Encoding::for_label(encoding.as_bytes())
        .ok_or_else(|| Error::new(ErrorKind::Query, format!("不支持的导入编码：{encoding}")))?;
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|text| text.into_owned())
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Query,
                format!("导入文件不是合法的 {} 文本", encoding.name()),
            )
        })
}

fn parse_delimited(
    text: &str,
    delimiter: u8,
    options: &fluxdb_core::TableImportOptions,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(TableImportProgress),
) -> fluxdb_core::Result<ParsedTable> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .quote(options.quote_byte())
        .has_headers(options.has_header)
        .flexible(false)
        .from_reader(text.as_bytes());
    let headers = if options.has_header {
        reader
            .headers()
            .map_err(|error| Error::new(ErrorKind::Query, format!("导入表头无效：{error}")))?
            .iter()
            .map(|value| value.trim().to_string())
            .collect::<Vec<_>>()
    } else {
        // 无表头时按列序号生成稳定列名。
        let mut plain = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .quote(options.quote_byte())
            .has_headers(false)
            .flexible(false)
            .from_reader(text.as_bytes());
        let count = plain
            .records()
            .next()
            .transpose()
            .map_err(|error| Error::new(ErrorKind::Query, format!("导入首行无效：{error}")))?
            .map(|record| record.len())
            .unwrap_or(0);
        if count == 0 {
            return Err(Error::new(ErrorKind::Query, "导入文件没有字段"));
        }
        (1..=count).map(|index| format!("column_{index}")).collect()
    };
    validate_import_columns(&headers)?;
    // 无表头时重新创建 reader 从头读取；有表头时沿用已消费首行的 reader。
    let mut reader = if options.has_header {
        reader
    } else {
        csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .quote(options.quote_byte())
            .has_headers(false)
            .flexible(false)
            .from_reader(text.as_bytes())
    };
    let mut rows = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let line_offset = if options.has_header { 2 } else { 1 };
        check_cancel(cancel)?;
        let record = record.map_err(|error| {
            Error::new(
                ErrorKind::Query,
                format!("导入第 {} 行无效：{error}", index + line_offset),
            )
        })?;
        if record.len() != headers.len() {
            return Err(Error::new(
                ErrorKind::Query,
                format!("导入第 {} 行字段数与表头不一致", index + line_offset),
            ));
        }
        rows.push(
            record
                .iter()
                .map(|value| import_text_cell_with(value, options))
                .collect::<Vec<_>>(),
        );
        if (index + 1) % 1000 == 0 {
            progress(TableImportProgress {
                stage: "读取".into(),
                message: format!("已读取 {} 行", index + 1),
            });
        }
    }
    Ok(ParsedTable {
        columns: headers,
        rows,
    })
}

fn parse_json(
    text: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(TableImportProgress),
) -> fluxdb_core::Result<ParsedTable> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| Error::new(ErrorKind::Query, format!("JSON 导入文件无效：{error}")))?;
    let rows = value
        .as_array()
        .ok_or_else(|| Error::new(ErrorKind::Query, "JSON 导入文件顶层必须是数组"))?;
    let Some(first) = rows.first().and_then(serde_json::Value::as_object) else {
        return Err(Error::new(
            ErrorKind::Query,
            "JSON 导入文件必须包含对象数组",
        ));
    };
    let columns = first.keys().cloned().collect::<Vec<_>>();
    validate_import_columns(&columns)?;
    let mut parsed = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        check_cancel(cancel)?;
        let row = row.as_object().ok_or_else(|| {
            Error::new(
                ErrorKind::Query,
                format!("JSON 第 {} 行不是对象", index + 1),
            )
        })?;
        if row.len() != columns.len()
            || columns.iter().any(|column| !row.contains_key(column))
        {
            return Err(Error::new(
                ErrorKind::Query,
                format!("JSON 第 {} 行字段与首行不一致", index + 1),
            ));
        }
        parsed.push(
            columns
                .iter()
                .map(|column| import_json_cell(&row[column]))
                .collect(),
        );
        if (index + 1) % 1000 == 0 {
            progress(TableImportProgress {
                stage: "读取".into(),
                message: format!("已读取 {} 行", index + 1),
            });
        }
    }
    Ok(ParsedTable {
        columns,
        rows: parsed,
    })
}

fn parse_xml(
    text: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(TableImportProgress),
) -> fluxdb_core::Result<ParsedTable> {
    let document = roxmltree::Document::parse(text)
        .map_err(|error| Error::new(ErrorKind::Query, format!("XML 导入文件无效：{error}")))?;
    let root = document.root_element();
    let row_nodes = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "row")
        .collect::<Vec<_>>();
    let Some(first) = row_nodes.first() else {
        return Err(Error::new(ErrorKind::Query, "XML 导入文件没有 row 节点"));
    };
    let columns = xml_row_columns(first);
    validate_import_columns(&columns)?;
    let mut rows = Vec::with_capacity(row_nodes.len());
    for (index, row) in row_nodes.iter().enumerate() {
        check_cancel(cancel)?;
        let values = xml_row_values(row, &columns).map_err(|message| {
            tracing::debug!(
                target: "fluxdb_connectors",
                row = index + 1,
                %message,
                "XML 导入行解析失败"
            );
            Error::new(ErrorKind::Query, format!("XML 第 {} 行格式无效", index + 1))
        })?;
        rows.push(values);
        if (index + 1) % 1000 == 0 {
            progress(TableImportProgress {
                stage: "读取".into(),
                message: format!("已读取 {} 行", index + 1),
            });
        }
    }
    Ok(ParsedTable { columns, rows })
}

fn xml_row_columns(row: &roxmltree::Node<'_, '_>) -> Vec<String> {
    row.children()
        .filter(|node| node.is_element() && node.tag_name().name() == "field")
        .filter_map(|node| node.attribute("name").map(str::to_string))
        .collect()
}

fn xml_row_values(
    row: &roxmltree::Node<'_, '_>,
    columns: &[String],
) -> Result<Vec<CellValue>, String> {
    let mut values = std::collections::BTreeMap::new();
    for field in row
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "field")
    {
        let name = field
            .attribute("name")
            .ok_or_else(|| "field 缺少 name 属性".to_string())?;
        if field.children().any(|node| node.is_element()) {
            return Err(format!("field {name} 包含嵌套元素，暂不支持"));
        }
        if values.insert(name.to_string(), import_text_cell(field.text().unwrap_or(""))).is_some() {
            return Err(format!("field {name} 重复"));
        }
    }
    if values.len() != columns.len() {
        return Err("字段数与首行不一致".to_string());
    }
    columns
        .iter()
        .map(|column| {
            values
                .remove(column)
                .ok_or_else(|| format!("缺少字段 {column}"))
        })
        .collect()
}

fn validate_import_columns(columns: &[String]) -> fluxdb_core::Result<()> {
    if columns.is_empty() {
        return Err(Error::new(ErrorKind::Query, "导入文件没有字段"));
    }
    let mut seen = BTreeSet::new();
    for column in columns {
        if column.trim().is_empty() {
            return Err(Error::new(ErrorKind::Query, "导入字段名不能为空"));
        }
        if !seen.insert(column.to_ascii_lowercase()) {
            return Err(Error::new(
                ErrorKind::Query,
                format!("导入字段名重复：{column}"),
            ));
        }
    }
    Ok(())
}

fn map_import_columns(
    imported: &[String],
    target: &[CompletionColumn],
    explicit: &[TableImportColumnMapping],
) -> fluxdb_core::Result<Vec<(usize, Option<usize>)>> {
    imported
        .iter()
        .enumerate()
        .map(|(index, name)| {
            // 显式映射优先：`Some(target)` 指定目标列，`None` 表示跳过该列。
            if let Some(mapping) = explicit
                .iter()
                .find(|mapping| mapping.source.eq_ignore_ascii_case(name))
            {
                let Some(target_name) = mapping.target.as_deref() else {
                    return Ok((index, None));
                };
                return resolve_target_column(target, target_name, name).map(|target| (index, Some(target)));
            }
            let exact = target
                .iter()
                .position(|column| column.name == *name);
            if let Some(target_index) = exact {
                return Ok((index, Some(target_index)));
            }
            let matches = target
                .iter()
                .enumerate()
                .filter(|(_, column)| column.name.eq_ignore_ascii_case(name))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [target_index] => Ok((index, Some(*target_index))),
                [] => Err(Error::new(
                    ErrorKind::Query,
                    format!("目标表不存在字段：{name}"),
                )),
                _ => Err(Error::new(
                    ErrorKind::Query,
                    format!("目标表字段名大小写不唯一：{name}"),
                )),
            }
        })
        .collect()
}

fn resolve_target_column(
    target: &[CompletionColumn],
    target_name: &str,
    source: &str,
) -> fluxdb_core::Result<usize> {
    if let Some(index) = target.iter().position(|column| column.name == target_name) {
        return Ok(index);
    }
    let matches = target
        .iter()
        .enumerate()
        .filter(|(_, column)| column.name.eq_ignore_ascii_case(target_name))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => Err(Error::new(
            ErrorKind::Query,
            format!("目标表不存在字段：{target_name}（源列 {source}）"),
        )),
        _ => Err(Error::new(
            ErrorKind::Query,
            format!("目标表字段名大小写不唯一：{target_name}"),
        )),
    }
}

fn ddl_import_column_flags(
    kind: DatabaseKind,
    ddl: Option<&str>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut excluded = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let Some(ddl) = ddl else {
        return (excluded, identities);
    };
    for entry in ddl_column_entries(ddl) {
        let Some(name) = ddl_entry_name(&entry) else {
            continue;
        };
        let upper = entry.to_ascii_uppercase();
        if matches!(kind, DatabaseKind::Postgres | DatabaseKind::SqlServer)
            && upper.contains("IDENTITY")
        {
            identities.insert(name.to_ascii_uppercase());
        } else if kind == DatabaseKind::Oracle && upper.contains("GENERATED ALWAYS AS IDENTITY")
        {
            // `GENERATED ALWAYS` 拒绝显式值（ORA-32795）；`BY DEFAULT` 允许，照常写入。
            excluded.insert(name.to_ascii_uppercase());
        } else if (kind == DatabaseKind::Dameng && upper.contains("IDENTITY"))
            || (upper.contains("GENERATED")
                && !upper.contains("IDENTITY")
                && !upper.contains("AUTO_INCREMENT"))
        {
            excluded.insert(name.to_ascii_uppercase());
        }
    }
    (excluded, identities)
}

fn ddl_column_entries(ddl: &str) -> Vec<String> {
    let Some(open) = ddl.find('(') else {
        return Vec::new();
    };
    let mut depth = 0usize;
    let mut quote = None;
    let mut start = open + 1;
    let mut entries = Vec::new();
    for (index, ch) in ddl.char_indices().filter(|(index, _)| *index >= open) {
        if let Some(end_quote) = quote {
            if ch == end_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => {
                entries.push(ddl[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = ddl[start..].trim();
    if !tail.is_empty() {
        entries.push(tail.to_string());
    }
    entries
}

fn ddl_entry_name(entry: &str) -> Option<String> {
    let trimmed = entry.trim();
    let upper = trimmed.to_ascii_uppercase();
    if ["CONSTRAINT", "PRIMARY", "FOREIGN", "CHECK", "UNIQUE"]
        .iter()
        .any(|keyword| upper.starts_with(keyword))
    {
        return None;
    }
    let first = trimmed.chars().next()?;
    if matches!(first, '"' | '`') {
        let end = trimmed[1..].find(first)? + 2;
        return Some(trimmed[1..end - 1].to_string());
    }
    if first == '[' {
        let end = trimmed[1..].find(']')? + 2;
        return Some(trimmed[1..end - 1].replace("]]", "]"));
    }
    trimmed.split_whitespace().next().map(str::to_string)
}

fn import_qualified_table(
    kind: DatabaseKind,
    request: &TableImportRequest,
) -> fluxdb_core::Result<String> {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            let database = request
                .object
                .database
                .as_deref()
                .ok_or_else(|| Error::new(ErrorKind::Query, "缺少数据库名称"))?;
            Ok(format!(
                "{}.{}",
                fluxdb_core::MySqlDialect.quote_identifier(database),
                fluxdb_core::MySqlDialect.quote_identifier(&request.object.name)
            ))
        }
        DatabaseKind::Postgres | DatabaseKind::Kingbase => {
            let schema = request.object.schema.as_deref().unwrap_or("public");
            Ok(format!(
                "{}.{}",
                fluxdb_core::PostgresDialect.quote_identifier(schema),
                fluxdb_core::PostgresDialect.quote_identifier(&request.object.name)
            ))
        }
        DatabaseKind::Sqlite => Ok(fluxdb_core::SqliteDialect
            .quote_identifier(&request.object.name)),
        DatabaseKind::Dameng => {
            let schema = request
                .object
                .schema
                .as_deref()
                .or_else(|| {
                    request
                        .config
                        .dameng_profile
                        .as_ref()
                        .map(|profile| profile.basic.schema.as_str())
                })
                .ok_or_else(|| Error::new(ErrorKind::Query, "缺少达梦 schema 上下文"))?;
            Ok(format!(
                "{}.{}",
                fluxdb_core::DamengDialect.quote_identifier(schema),
                fluxdb_core::DamengDialect.quote_identifier(&request.object.name)
            ))
        }
        DatabaseKind::SqlServer => {
            let schema = request
                .object
                .schema
                .as_deref()
                .map(str::trim)
                .filter(|schema| !schema.is_empty())
                .or_else(|| {
                    request
                        .config
                        .sqlserver_profile
                        .as_ref()
                        .map(|profile| profile.basic.schema.trim())
                        .filter(|schema| !schema.is_empty())
                })
                .unwrap_or("dbo");
            Ok(format!(
                "{}.{}",
                fluxdb_core::SqlServerDialect.quote_identifier(schema),
                fluxdb_core::SqlServerDialect.quote_identifier(&request.object.name)
            ))
        }
        DatabaseKind::Oracle => {
            // Oracle 的对象 Owner 就是 schema；当前 Schema 只影响名称解析，
            // 因此这里优先用对象自带的 Owner，其次才回落到连接档案的默认 Schema。
            let schema = request
                .object
                .schema
                .as_deref()
                .map(str::trim)
                .filter(|schema| !schema.is_empty())
                .or_else(|| {
                    request
                        .config
                        .oracle_profile
                        .as_ref()
                        .map(|profile| profile.basic.schema.trim())
                        .filter(|schema| !schema.is_empty())
                })
                .ok_or_else(|| Error::new(ErrorKind::Query, "缺少 Oracle schema 上下文"))?;
            Ok(format!(
                "{}.{}",
                fluxdb_core::OracleDialect.quote_identifier(schema),
                fluxdb_core::OracleDialect.quote_identifier(&request.object.name)
            ))
        }
        DatabaseKind::Redis | DatabaseKind::MongoDb => Err(Error::new(
            ErrorKind::Unsupported,
            "当前连接类型没有可导入的表数据",
        )),
    }
}

fn import_quote_identifier(kind: DatabaseKind, identifier: &str) -> String {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            fluxdb_core::MySqlDialect.quote_identifier(identifier)
        }
        DatabaseKind::Postgres | DatabaseKind::Kingbase => {
            fluxdb_core::PostgresDialect.quote_identifier(identifier)
        }
        DatabaseKind::Sqlite => fluxdb_core::SqliteDialect.quote_identifier(identifier),
        DatabaseKind::Dameng => fluxdb_core::DamengDialect.quote_identifier(identifier),
        DatabaseKind::SqlServer => fluxdb_core::SqlServerDialect.quote_identifier(identifier),
        DatabaseKind::Oracle => fluxdb_core::OracleDialect.quote_identifier(identifier),
        DatabaseKind::Redis | DatabaseKind::MongoDb => identifier.to_string(),
    }
}

fn import_text_cell(value: &str) -> CellValue {
    if value.eq_ignore_ascii_case("NULL") {
        CellValue::Null
    } else {
        CellValue::Text(value.to_string())
    }
}

/// 按解析选项转换文本单元格：可选去空白，空串可按 NULL 写入。
fn import_text_cell_with(
    value: &str,
    options: &fluxdb_core::TableImportOptions,
) -> CellValue {
    let value = if options.trim_whitespace {
        value.trim()
    } else {
        value
    };
    if options.empty_as_null && value.is_empty() {
        return CellValue::Null;
    }
    import_text_cell(value)
}

fn import_json_cell(value: &serde_json::Value) -> CellValue {
    match value {
        serde_json::Value::Null => CellValue::Null,
        serde_json::Value::Bool(value) => CellValue::Bool(*value),
        serde_json::Value::Number(value) => value
            .as_i64()
            .map(CellValue::I64)
            .or_else(|| value.as_f64().map(CellValue::F64))
            .unwrap_or_else(|| CellValue::Text(value.to_string())),
        serde_json::Value::String(value) => CellValue::Text(value.clone()),
        other => CellValue::Json(other.to_string()),
    }
}

fn import_sql_parameter(value: &CellValue) -> fluxdb_core::Result<SqlParameter> {
    Ok(match value {
        CellValue::Null => SqlParameter::Null,
        CellValue::Bool(value) => SqlParameter::Bool(*value),
        CellValue::I64(value) => SqlParameter::I64(value.to_string()),
        CellValue::F64(value) if value.is_finite() => SqlParameter::F64(*value),
        CellValue::F64(_) => {
            return Err(Error::new(ErrorKind::Query, "导入数据包含非有限浮点数"));
        }
        CellValue::Text(value) | CellValue::Json(value) => SqlParameter::Text(value.clone()),
        // 预览是片段，导入会用它覆盖目标表的完整值。
        CellValue::TruncatedText { .. } => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "导入不支持只加载了预览的文本（截断的 CLOB），请先加载完整值",
            ));
        }
        // 大字段只有预览：导入会用它覆盖目标表的完整值。
        CellValue::LargeValue(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "导入不支持只加载了预览的超大字段，请先按内容引用读取完整值",
            ));
        }
        CellValue::Bytes(value) => SqlParameter::Bytes(value.clone()),
        CellValue::BinarySummary(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "导入不支持未完整加载的二进制字段",
            ));
        }
    })
}

fn import_sql_parameter_for_column(
    value: &CellValue,
    column: &CompletionColumn,
) -> fluxdb_core::Result<SqlParameter> {
    let type_name = column.type_name.as_deref().unwrap_or("").trim();
    // 数组列在 PG 中恒走文本桥接（`CAST(CAST($n AS text) AS <type>[])`），
    // 内层 `$n` 必须是 text 参数，否则与非文本标签绑定冲突。
    if type_name.ends_with("[]")
        && matches!(
            value,
            CellValue::Text(_)
                | CellValue::Json(_)
                | CellValue::I64(_)
                | CellValue::F64(_)
                | CellValue::Bool(_)
        )
    {
        return Ok(SqlParameter::Text(match value {
            CellValue::Text(text) | CellValue::Json(text) => text.clone(),
            other => other.display_label(),
        }));
    }
    let base = type_name
        .split('(')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    match value {
        CellValue::Null
        | CellValue::Bool(_)
        | CellValue::Bytes(_)
        | CellValue::BinarySummary(_)
        | CellValue::TruncatedText { .. }
        | CellValue::LargeValue(_) => import_sql_parameter(value),
        CellValue::I64(value)
            if matches!(
                base.as_str(),
                "numeric" | "decimal" | "number"
            ) =>
        {
            Ok(SqlParameter::Decimal(value.to_string()))
        }
        CellValue::F64(value)
            if matches!(base.as_str(), "numeric" | "decimal" | "number") && value.is_finite() =>
        {
            Ok(SqlParameter::Decimal(value.to_string()))
        }
        CellValue::Text(value) | CellValue::Json(value) => {
            if fluxdb_core::is_binary_type_name(&base) {
                let hex = value
                    .trim()
                    .strip_prefix("0x")
                    .or_else(|| value.trim().strip_prefix("0X"))
                    .unwrap_or(value.trim())
                    .chars()
                    .filter(|ch| !ch.is_whitespace())
                    .collect::<String>();
                if hex.len() % 2 != 0 {
                    return Err(Error::new(ErrorKind::Query, "导入二进制值格式无效"));
                }
                let bytes = (0..hex.len())
                    .step_by(2)
                    .map(|index| {
                        u8::from_str_radix(&hex[index..index + 2], 16)
                            .map_err(|_| Error::new(ErrorKind::Query, "导入二进制值格式无效"))
                    })
                    .collect::<fluxdb_core::Result<Vec<_>>>()?;
                Ok(SqlParameter::Bytes(bytes))
            } else if matches!(
                base.as_str(),
                "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "int2"
                    | "int4" | "int8" | "serial" | "bigserial" | "smallserial"
            ) {
                value
                    .trim()
                    .parse::<i64>()
                    .map(|value| SqlParameter::I64(value.to_string()))
                    .map_err(|_| Error::new(ErrorKind::Query, "导入整数值格式无效"))
            } else if matches!(base.as_str(), "numeric" | "decimal" | "number") {
                Ok(SqlParameter::Decimal(value.trim().to_string()))
            } else if matches!(
                base.as_str(),
                "real" | "float4" | "float8" | "double" | "double precision" | "float"
            ) {
                value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .map(SqlParameter::F64)
                    .ok_or_else(|| Error::new(ErrorKind::Query, "导入浮点值格式无效"))
            } else if matches!(base.as_str(), "boolean" | "bool") {
                match value.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "t" | "yes" => Ok(SqlParameter::Bool(true)),
                    "0" | "false" | "f" | "no" => Ok(SqlParameter::Bool(false)),
                    _ => Err(Error::new(ErrorKind::Query, "导入布尔值格式无效")),
                }
            } else {
                Ok(SqlParameter::Text(value.clone()))
            }
        }
        _ => import_sql_parameter(value),
    }
}

fn pg_import_needs_text_cast(value: &CellValue, column: &CompletionColumn) -> bool {
    let type_name = column.type_name.as_deref().unwrap_or("").trim();
    // 数组列必须走文本桥接：`$n` 会被服务端推断为元素数组类型（如 `_numeric`），
    // 客户端只能以 text 参数绑定，再由外层 `CAST(... AS <type>[])` 还原。
    // 判定须在剥 `(modifiers)` 之前，否则 `numeric(12,4)[]` 会被当成标量 numeric 直绑。
    if type_name.ends_with("[]") {
        return matches!(
            value,
            CellValue::Text(_)
                | CellValue::Json(_)
                | CellValue::I64(_)
                | CellValue::F64(_)
                | CellValue::Bool(_)
        );
    }
    if !matches!(value, CellValue::Text(_) | CellValue::Json(_)) {
        return false;
    }
    let base = type_name
        .split('(')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    !matches!(
        base.as_str(),
        "text"
            | "varchar"
            | "character varying"
            | "char"
            | "character"
            | "bpchar"
            | "name"
            | "citext"
            | "smallint"
            | "integer"
            | "int"
            | "int2"
            | "int4"
            | "int8"
            | "bigint"
            | "real"
            | "float"
            | "float4"
            | "float8"
            | "double"
            | "double precision"
            | "boolean"
            | "bool"
            | "numeric"
            | "decimal"
    )
}

fn check_cancel(cancel: &AtomicBool) -> fluxdb_core::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::new(ErrorKind::Unsupported, "导入已取消"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod table_import_tests {
    use super::*;
    use std::io::Write;

    fn parse_sample(
        format: TableImportFormat,
        extension: &str,
        text: &str,
    ) -> ParsedTable {
        let dir = std::env::temp_dir().join(format!(
            "fluxdb-import-test-{}-{}",
            std::process::id(),
            extension
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("sample.{extension}"));
        let mut file = std::fs::File::create(&path).expect("sample file");
        file.write_all(text.as_bytes()).expect("write sample");
        let request = TableImportRequest {
            config: crate::ConnectionConfig {
                id: crate::ConnectionId(1),
                name: "import test".into(),
                kind: crate::DatabaseKind::Sqlite,
                endpoint: crate::Endpoint::SqliteFile {
                    path: path.clone(),
                    read_only: true,
                },
                options: std::collections::BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: None,
                mongo_profile: None,
                sqlserver_profile: None,
                oracle_profile: None,
            },
            object: crate::ObjectPath {
                connection_id: crate::ConnectionId(1),
                database: None,
                schema: None,
                name: "sample".into(),
                kind: crate::ObjectKind::Table,
            },
            source: path,
            format,
            options: fluxdb_core::TableImportOptions::new(),
            column_mapping: Vec::new(),
        };
        parse_import_file(&request, &AtomicBool::new(false), &mut |_| {}).expect("parse")
    }

    #[test]
    fn parses_exported_txt_and_csv_with_null_sentinel() {
        for (format, extension, text) in [
            (TableImportFormat::Txt, "txt", "ID\tNAME\n1\talpha\n2\tNULL\n"),
            (TableImportFormat::Csv, "csv", "ID,NAME\n1,alpha\n2,NULL\n"),
        ] {
            let parsed = parse_sample(format, extension, text);
            assert_eq!(parsed.columns, vec!["ID", "NAME"]);
            assert_eq!(parsed.rows[0][1], CellValue::Text("alpha".into()));
            assert_eq!(parsed.rows[1][1], CellValue::Null);
        }
    }

    #[test]
    fn parses_exported_json_and_xml_shapes() {
        let json = parse_sample(
            TableImportFormat::Json,
            "json",
            r#"[{"ID":1,"NAME":"alpha"},{"ID":2,"NAME":null}]"#,
        );
        assert_eq!(json.rows[0][0], CellValue::I64(1));
        assert_eq!(json.rows[1][1], CellValue::Null);
        let xml = parse_sample(
            TableImportFormat::Xml,
            "xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><rows><row><field name="ID">1</field><field name="NAME">alpha</field></row><row><field name="ID">2</field><field name="NAME">NULL</field></row></rows>"#,
        );
        assert_eq!(xml.rows[0][1], CellValue::Text("alpha".into()));
        assert_eq!(xml.rows[1][1], CellValue::Null);
    }

    fn parse_sample_with_options(
        format: TableImportFormat,
        extension: &str,
        label: &str,
        bytes: &[u8],
        options: fluxdb_core::TableImportOptions,
    ) -> fluxdb_core::Result<ParsedTable> {
        let dir = std::env::temp_dir().join(format!(
            "fluxdb-import-options-{}-{label}",
            std::process::id(),
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("sample.{extension}"));
        std::fs::write(&path, bytes).expect("write sample");
        parse_import_source(&path, format, &options, &AtomicBool::new(false), &mut |_| {})
    }

    #[test]
    fn parses_gb18030_text_with_custom_delimiter() {
        // "姓名;城市\n张三;北京" 的 GB18030 编码。
        let mut text = Vec::new();
        text.extend_from_slice(&[0xd0, 0xd5, 0xc3, 0xfb]); // 姓名
        text.push(b';');
        text.extend_from_slice(&[0xb3, 0xc7, 0xca, 0xd0]); // 城市
        text.push(b'\n');
        text.extend_from_slice(&[0xd5, 0xc5, 0xc8, 0xfd]); // 张三
        text.push(b';');
        text.extend_from_slice(&[0xb1, 0xb1, 0xbe, 0xa9]); // 北京
        text.push(b'\n');

        let options = fluxdb_core::TableImportOptions {
            encoding: "gb18030".into(),
            delimiter: ";".into(),
            ..fluxdb_core::TableImportOptions::new()
        };
        let parsed = parse_sample_with_options(TableImportFormat::Txt, "txt", "gb18030", &text, options)
            .expect("parse gb18030");
        assert_eq!(parsed.columns, vec!["姓名", "城市"]);
        assert_eq!(parsed.rows[0][0], CellValue::Text("张三".into()));
        assert_eq!(parsed.rows[0][1], CellValue::Text("北京".into()));
    }

    #[test]
    fn parses_headerless_csv_with_generated_columns() {
        let options = fluxdb_core::TableImportOptions {
            has_header: false,
            ..fluxdb_core::TableImportOptions::new()
        };
        let parsed = parse_sample_with_options(
            TableImportFormat::Csv,
            "csv",
            "headerless",
            b"1,alpha\n2,beta\n",
            options,
        )
        .expect("parse headerless");
        assert_eq!(parsed.columns, vec!["column_1", "column_2"]);
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[1][1], CellValue::Text("beta".into()));
    }

    #[test]
    fn trims_values_and_maps_empty_to_null() {
        let options = fluxdb_core::TableImportOptions {
            trim_whitespace: true,
            empty_as_null: true,
            ..fluxdb_core::TableImportOptions::new()
        };
        let parsed = parse_sample_with_options(
            TableImportFormat::Csv,
            "csv",
            "trimmed",
            b"ID,NAME\n1,  alpha  \n2,\n",
            options,
        )
        .expect("parse trimmed");
        assert_eq!(parsed.rows[0][1], CellValue::Text("alpha".into()));
        assert_eq!(parsed.rows[1][1], CellValue::Null);
    }

    #[test]
    fn explicit_mapping_can_skip_and_rename_columns() {
        let target = vec![
            completion_column("ID", "INT"),
            completion_column("NAME", "TEXT"),
        ];
        let mapping = map_import_columns(
            &["id".to_string(), "full_name".to_string(), "ignored".to_string()],
            &target,
            &[
                TableImportColumnMapping {
                    source: "full_name".into(),
                    target: Some("NAME".into()),
                },
                TableImportColumnMapping {
                    source: "ignored".into(),
                    target: None,
                },
            ],
        )
        .expect("map columns");
        assert_eq!(mapping, vec![(0, Some(0)), (1, Some(1)), (2, None)]);
    }

    fn completion_column(name: &str, type_name: &str) -> fluxdb_core::CompletionColumn {
        fluxdb_core::CompletionColumn {
            database: None,
            schema: None,
            table: "sample".into(),
            name: name.into(),
            type_name: Some(type_name.into()),
            nullable: true,
            primary_key: false,
            comment: None,
            stable: None,
        }
    }

    #[test]
    fn sqlserver_binary_text_values_are_bound_as_bytes() {
        for (type_name, value, expected) in [
            ("varbinary(8)", "0x00FF10", vec![0x00, 0xff, 0x10]),
            ("binary(2)", "00ff", vec![0x00, 0xff]),
            ("image", "0X01A2", vec![0x01, 0xa2]),
            ("varbinary(max)", "0x", Vec::new()),
        ] {
            let parameter = import_sql_parameter_for_column(
                &CellValue::Text(value.into()),
                &completion_column("PAYLOAD", type_name),
            )
            .expect("binary import value");
            assert_eq!(parameter, SqlParameter::Bytes(expected), "{type_name}");
        }
        for value in ["not-hex", "0x123", "0xGG"] {
            let error = import_sql_parameter_for_column(
                &CellValue::Text(value.into()),
                &completion_column("PAYLOAD", "varbinary(max)"),
            )
            .expect_err("invalid binary text must be rejected");
            assert_eq!(error.message, "导入二进制值格式无效", "{value}");
        }
        assert_eq!(
            import_sql_parameter_for_column(
                &CellValue::Null,
                &completion_column("PAYLOAD", "varbinary(max)"),
            )
            .expect("NULL binary"),
            SqlParameter::Null
        );
    }

    #[test]
    fn sqlserver_binary_null_uses_typed_sql_null() {
        let columns = [
            completion_column("PAYLOAD", "varbinary(8)"),
            completion_column("NOTE", "nvarchar(8)"),
        ];
        let (sql, parameters) = build_import_batch_sql(
            DatabaseKind::SqlServer,
            "[dbo].[sample]",
            "[PAYLOAD], [NOTE]",
            "",
            &[(0, 0), (1, 1)],
            &columns,
            &[vec![CellValue::Null, CellValue::Null]],
        )
        .expect("build SQL Server import SQL");
        assert_eq!(sql, "INSERT INTO [dbo].[sample] ([PAYLOAD], [NOTE]) VALUES (NULL, @P1)");
        assert_eq!(parameters, vec![SqlParameter::Null]);
    }

    #[test]
    fn text_columns_keep_hex_looking_text_as_text() {
        assert_eq!(
            import_sql_parameter_for_column(
                &CellValue::Text("0x1234".into()),
                &completion_column("CODE", "nvarchar(20)"),
            )
            .expect("text value"),
            SqlParameter::Text("0x1234".into())
        );
    }

    #[test]
    fn postgres_array_columns_use_text_cast() {
        // `numeric(12,4)[]` 曾被当成标量 numeric 直绑 `$n`，服务端把占位符推断为
        // `_numeric`（元素数组），客户端 text 参数无法直绑 → 导入报类型不兼容。
        let columns = [
            completion_column("AMOUNTS", "numeric(12,4)[]"),
            completion_column("AMOUNT", "numeric(12,4)"),
        ];
        let (sql, parameters) = build_import_batch_sql(
            DatabaseKind::Postgres,
            "\"s\".\"t\"",
            "\"AMOUNTS\", \"AMOUNT\"",
            "",
            &[(0, 0), (1, 1)],
            &columns,
            &[vec![
                CellValue::Text("{1.0000,2.5000}".into()),
                CellValue::Text("1.0000".into()),
            ]],
        )
        .expect("build PostgreSQL import SQL");
        assert_eq!(
            sql,
            "INSERT INTO \"s\".\"t\" (\"AMOUNTS\", \"AMOUNT\") VALUES \
             (CAST(CAST($1 AS text) AS numeric(12,4)[]), $2)"
        );
        assert_eq!(
            parameters,
            vec![
                SqlParameter::Text("{1.0000,2.5000}".into()),
                SqlParameter::Decimal("1.0000".into())
            ]
        );
    }

    #[test]
    fn batch_progress_reports_one_based_batches() {
        // 1200 行按 500 行分批：三批，边界行号必须闭合，不能越界或重复。
        let total_rows = 1200usize;
        let total_batches = total_rows.div_ceil(TABLE_IMPORT_BATCH_ROWS);
        assert_eq!(total_batches, 3);

        let mut last_written = 0usize;
        for (batch_index, chunk) in (0..total_rows).collect::<Vec<_>>().chunks(TABLE_IMPORT_BATCH_ROWS).enumerate() {
            let written = (batch_index * TABLE_IMPORT_BATCH_ROWS + chunk.len()).min(total_rows);
            let first = written.saturating_sub(chunk.len()) + 1;
            assert_eq!(first, last_written + 1, "批次 {batch_index} 起点不连续");
            assert!(written <= total_rows, "批次 {batch_index} 越界");
            last_written = written;
        }
        assert_eq!(last_written, total_rows);
    }
}
