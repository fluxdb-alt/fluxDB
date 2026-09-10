/// 避免超长/多语句查询刷屏且不泄露过多内容。
fn truncate_sql_for_log(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 400 {
        let mut truncated: String = compact.chars().take(400).collect();
        truncated.push_str("…[截断]");
        truncated
    } else {
        compact
    }
}

/// 等待驱动 future，同时轮询取消回调。
/// 返回 `Ok(None)` 表示 future 被取消；调用方必须丢弃当前连接，避免复用半途状态。
async fn await_with_cancel<T, E, F>(
    future: F,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Option<T>, E>
where
    F: Future<Output = Result<T, E>>,
{
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => {
                let value = result?;
                return if should_cancel() { Ok(None) } else { Ok(Some(value)) };
            },
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                if should_cancel() {
                    return Ok(None);
                }
            }
        }
    }
}

fn database_object(connection_id: ConnectionId, database: &str) -> ObjectSummary {
    ObjectSummary {
        path: ObjectPath {
            connection_id,
            database: Some(database.to_string()),
            schema: None,
            name: database.to_string(),
            kind: ObjectKind::Database,
        },
        rows: None,
        modified_at: None,
        comment: None,
    }
}

fn completion_database(
    config: &ConnectionConfig,
    database: Option<&str>,
) -> fluxdb_core::Result<String> {
    database
        .map(str::to_string)
        .or_else(|| match &config.endpoint {
            Endpoint::Tcp { database, .. } => database.clone(),
            _ => None,
        })
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Connection, "缺少数据库上下文"))
}

fn completion_like_filter(filter: &str) -> String {
    format!("{}%", filter.trim().to_ascii_lowercase())
}

fn completion_fuzzy_like_filter(filter: &str) -> String {
    let filter = filter.trim().to_ascii_lowercase();
    if filter.is_empty() {
        return "%".to_string();
    }

    let mut pattern = String::from("%");
    for ch in filter.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
        pattern.push('%');
    }
    pattern
}

fn matches_completion_filter(value: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty()
        || value
            .to_ascii_lowercase()
            .starts_with(&filter.to_ascii_lowercase())
}

fn matches_completion_fuzzy_filter(value: &str, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }

    let value = value.to_ascii_lowercase();
    let mut value_chars = value.chars();
    filter
        .to_ascii_lowercase()
        .chars()
        .all(|filter_char| value_chars.any(|value_char| value_char == filter_char))
}

fn columns_to_completion(
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
    columns: Vec<Column>,
) -> Vec<CompletionColumn> {
    let database = database.map(str::to_string);
    let schema = schema.map(str::to_string);
    columns
        .into_iter()
        .map(|column| CompletionColumn {
            database: database.clone(),
            schema: schema.clone(),
            table: table.to_string(),
            name: column.name,
            type_name: column.type_name,
            nullable: column.nullable,
            primary_key: column.primary_key,
            comment: column.comment,
        })
        .collect()
}

fn mysql_select_expr(column: &Column) -> String {
    let name = mysql_quote_identifier(&column.name);
    if is_binary_column(column) {
        let is_null = mysql_quote_identifier(&binary_alias(&column.name, "is_null"));
        let byte_length = mysql_quote_identifier(&binary_alias(&column.name, "byte_length"));
        let preview_hex = mysql_quote_identifier(&binary_alias(&column.name, "preview_hex"));
        return format!(
            "CAST({name} IS NULL AS UNSIGNED) AS {is_null}, COALESCE(OCTET_LENGTH({name}), 0) AS {byte_length}, UPPER(HEX(SUBSTRING({name}, 1, 64))) AS {preview_hex}"
        );
    }

    format!("CAST({name} AS CHAR) AS {name}")
}

fn sqlite_select_expr(column: &Column) -> String {
    let name = sqlite_quote_identifier(&column.name);
    if is_binary_column(column) {
        let is_null = sqlite_quote_identifier(&binary_alias(&column.name, "is_null"));
        let byte_length = sqlite_quote_identifier(&binary_alias(&column.name, "byte_length"));
        let preview_hex = sqlite_quote_identifier(&binary_alias(&column.name, "preview_hex"));
        return format!(
            "{name} IS NULL AS {is_null}, COALESCE(length({name}), 0) AS {byte_length}, upper(hex(substr({name}, 1, 64))) AS {preview_hex}"
        );
    }

    format!("CAST({name} AS TEXT) AS {name}")
}

fn is_binary_column(column: &Column) -> bool {
    column.type_name.as_deref().is_some_and(is_binary_type_name)
}

fn binary_alias(column: &str, suffix: &str) -> String {
    format!("{column}__{suffix}")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn mysql_rows_to_page(
    columns: Vec<Column>,
    rows: Vec<MySqlRow>,
    pagination: Pagination,
) -> DataPage {
    let has_more = rows.len() > pagination.limit as usize;
    let rows = rows
        .into_iter()
        .take(pagination.limit as usize)
        .map(|row| Row {
            values: (0..columns.len())
                .map(|index| mysql_cell_value(&row, &columns[index]))
                .collect(),
        })
        .collect();

    DataPage {
        columns,
        rows,
        offset: pagination.offset,
        limit: pagination.limit,
        has_more,
    }
}

fn sqlite_rows_to_page(
    columns: Vec<Column>,
    rows: Vec<SqliteRow>,
    pagination: Pagination,
) -> DataPage {
    let has_more = rows.len() > pagination.limit as usize;
    let rows = rows
        .into_iter()
        .take(pagination.limit as usize)
        .map(|row| Row {
            values: (0..columns.len())
                .map(|index| sqlite_cell_value(&row, &columns[index]))
                .collect(),
        })
        .collect();

    DataPage {
        columns,
        rows,
        offset: pagination.offset,
        limit: pagination.limit,
        has_more,
    }
}

fn mysql_cell_value(row: &MySqlRow, column: &Column) -> CellValue {
    if is_binary_column(column) {
        return mysql_binary_summary(row, column);
    }

    mysql_plain_cell_value(row, column)
}

fn mysql_plain_cell_value(row: &MySqlRow, column: &Column) -> CellValue {
    mysql_plain_cell_value_by(row, column.name.as_str())
}

fn mysql_plain_cell_value_by<I>(row: &MySqlRow, index: I) -> CellValue
where
    I: ColumnIndex<MySqlRow> + Copy,
{
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<u64>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<BigDecimal>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<DateTime<Utc>>, _>(index) {
        return mysql_datetime_cell_value(value.map(|value| value.naive_utc()));
    }
    if let Ok(value) = row.try_get::<Option<NaiveDateTime>, _>(index) {
        return mysql_datetime_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<NaiveDate>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<NaiveTime>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<MySqlTime>, _>(index) {
        return mysql_optional_display_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value
            .map(mysql_query_bytes_cell_value)
            .unwrap_or(CellValue::Null);
    }

    CellValue::Null
}

fn mysql_optional_display_cell_value(value: Option<impl ToString>) -> CellValue {
    value
        .map(|value| CellValue::Text(value.to_string()))
        .unwrap_or(CellValue::Null)
}

fn sqlite_cell_value(row: &SqliteRow, column: &Column) -> CellValue {
    if is_binary_column(column) {
        return sqlite_binary_summary(row, column);
    }

    row.try_get::<Option<String>, _>(column.name.as_str())
        .map(|value| value.map(CellValue::Text).unwrap_or(CellValue::Null))
        .unwrap_or_else(|_| {
            row.try_get::<Option<Vec<u8>>, _>(column.name.as_str())
                .map(|value| value.map(CellValue::Bytes).unwrap_or(CellValue::Null))
                .unwrap_or(CellValue::Null)
        })
}

fn mysql_binary_summary(row: &MySqlRow, column: &Column) -> CellValue {
    let is_null_alias = binary_alias(&column.name, "is_null");
    let byte_length_alias = binary_alias(&column.name, "byte_length");
    let preview_hex_alias = binary_alias(&column.name, "preview_hex");
    let is_null = row
        .try_get::<u64, _>(is_null_alias.as_str())
        .map(|value| value != 0)
        .unwrap_or(false);
    let byte_length = row
        .try_get::<u64, _>(byte_length_alias.as_str())
        .unwrap_or_default();
    let preview_hex = row
        .try_get::<Option<String>, _>(preview_hex_alias.as_str())
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());

    CellValue::BinarySummary(binary_summary(column, is_null, byte_length, preview_hex))
}

fn sqlite_binary_summary(row: &SqliteRow, column: &Column) -> CellValue {
    let is_null_alias = binary_alias(&column.name, "is_null");
    let byte_length_alias = binary_alias(&column.name, "byte_length");
    let preview_hex_alias = binary_alias(&column.name, "preview_hex");
    let is_null = row
        .try_get::<i64, _>(is_null_alias.as_str())
        .map(|value| value != 0)
        .unwrap_or(false);
    let byte_length = row
        .try_get::<i64, _>(byte_length_alias.as_str())
        .map(|value| value.max(0) as u64)
        .unwrap_or_default();
    let preview_hex = row
        .try_get::<Option<String>, _>(preview_hex_alias.as_str())
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());

    CellValue::BinarySummary(binary_summary(column, is_null, byte_length, preview_hex))
}

fn binary_summary(
    column: &Column,
    is_null: bool,
    byte_length: u64,
    preview_hex: Option<String>,
) -> BinaryCellSummary {
    BinaryCellSummary {
        type_name: column
            .type_name
            .clone()
            .unwrap_or_else(|| "BINARY".to_string()),
        is_null,
        byte_length,
        preview_hex,
    }
}
