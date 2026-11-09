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
        stable: None,
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

fn matches_completion_filter(value: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty()
        || value
            .to_ascii_lowercase()
            .starts_with(&filter.to_ascii_lowercase())
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
            stable: None,
        })
        .collect()
}

fn mysql_select_expr(column: &Column) -> String {
    let name = fluxdb_core::MySqlDialect.quote_identifier(&column.name);
    if is_binary_column(column) {
        let is_null = fluxdb_core::MySqlDialect.quote_identifier(&binary_alias(&column.name, "is_null"));
        let byte_length = fluxdb_core::MySqlDialect.quote_identifier(&binary_alias(&column.name, "byte_length"));
        let preview_hex = fluxdb_core::MySqlDialect.quote_identifier(&binary_alias(&column.name, "preview_hex"));
        return format!(
            "CAST({name} IS NULL AS UNSIGNED) AS {is_null}, COALESCE(OCTET_LENGTH({name}), 0) AS {byte_length}, UPPER(HEX(SUBSTRING({name}, 1, 64))) AS {preview_hex}"
        );
    }

    format!("CAST({name} AS CHAR) AS {name}")
}

fn sqlite_select_expr(column: &Column) -> String {
    let name = fluxdb_core::SqliteDialect.quote_identifier(&column.name);
    if is_binary_column(column) {
        let is_null = fluxdb_core::SqliteDialect.quote_identifier(&binary_alias(&column.name, "is_null"));
        let byte_length = fluxdb_core::SqliteDialect.quote_identifier(&binary_alias(&column.name, "byte_length"));
        let preview_hex = fluxdb_core::SqliteDialect.quote_identifier(&binary_alias(&column.name, "preview_hex"));
        return format!(
            "{name} IS NULL AS {is_null}, COALESCE(length({name}), 0) AS {byte_length}, upper(hex(substr({name}, 1, 64))) AS {preview_hex}"
        );
    }

    // 不统一 CAST 成 TEXT：SQLite 的值类型按行存储，表浏览需要保留
    // INTEGER/REAL/NULL 的真实类型，才能与查询编辑器及写回身份保持一致。
    format!("{name} AS {name}")
}

/// SQL Server 的二进制列投影：与 MySQL/SQLite 同一口径，正文永不进结果页。
///
/// `DATALENGTH` 对 `varchar`/`nvarchar`/`varbinary` 都给真实字节数（`LEN` 会把
/// `nvarchar` 算成字符数、且忽略尾随空格，不能用）；`CONVERT(..., 2)` 是十六进制
/// 样式，`SUBSTRING` 一基且对二进制按字节切。`image` 不能直接进 `SUBSTRING`，
/// 先 `CAST(... AS varbinary(max))`（对 `binary`/`varbinary` 也无副作用）。
fn sqlserver_select_expr(column: &Column) -> String {
    let name = fluxdb_core::SqlServerDialect.quote_identifier(&column.name);
    if is_binary_column(column) {
        let is_null =
            fluxdb_core::SqlServerDialect.quote_identifier(&binary_alias(&column.name, "is_null"));
        let byte_length =
            fluxdb_core::SqlServerDialect.quote_identifier(&binary_alias(&column.name, "byte_length"));
        let preview_hex =
            fluxdb_core::SqlServerDialect.quote_identifier(&binary_alias(&column.name, "preview_hex"));
        return format!(
            "CAST(CASE WHEN {name} IS NULL THEN 1 ELSE 0 END AS int) AS {is_null}, \
             COALESCE(DATALENGTH({name}), 0) AS {byte_length}, \
             CONVERT(varchar(max), SUBSTRING(CAST({name} AS varbinary(max)), 1, 64), 2) AS {preview_hex}"
        );
    }

    sqlserver_single_select_expr(column)
}

/// 单列投影：`sql_variant` 转文本（驱动解码是 `todo!()`，直接取会 abort 进程）。
fn sqlserver_single_select_expr(column: &Column) -> String {
    let quoted = fluxdb_core::SqlServerDialect.quote_identifier(&column.name);
    if column.type_name.as_deref().is_some_and(|type_name| {
        type_name
            .trim()
            .split(['(', ' '])
            .next()
            .is_some_and(|base| base.eq_ignore_ascii_case("sql_variant"))
    }) {
        format!("CAST({quoted} AS nvarchar(max)) AS {quoted}")
    } else {
        quoted
    }
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

/// 单格超过阈值就外置，行本身不按整页字节预算截断。
fn spill_large_cells(owner: &str, values: &mut [CellValue]) -> fluxdb_core::Result<()> {
    for value in values.iter_mut() {
        if let Some(text) = cell_full_text(value) {
            if text.len() > fluxdb_core::CELL_SPILL_THRESHOLD_BYTES {
                let kind = if matches!(value, CellValue::Json(_)) {
                    fluxdb_core::LargeValueKind::Json
                } else {
                    fluxdb_core::LargeValueKind::Text
                };
                *value = fluxdb_core::spill_large_value(owner, kind, text.as_bytes())?;
            }
        } else if let CellValue::Bytes(bytes) = value
            && bytes.len() > fluxdb_core::CELL_SPILL_THRESHOLD_BYTES
        {
            let bytes = std::mem::take(bytes);
            *value = fluxdb_core::spill_large_value(owner, fluxdb_core::LargeValueKind::Binary, &bytes)?;
        }
    }
    Ok(())
}

/// 单元格仍是完整文本时取出引用（外置前用）。
fn cell_full_text(value: &CellValue) -> Option<&str> {
    match value {
        CellValue::Text(text) | CellValue::Json(text) => Some(text),
        _ => None,
    }
}

/// 按行顺序收页：单格先外置，页容量只由行数决定。
///
/// `rows` 由调用方按容量截好；本函数只负责逐行决定收或不收。
///
/// 生产路径都用流式的 [`collect_sqlx_page`]，这个内存版只服务于单测里
/// 可造大批行、不依赖驱动的场景。
#[cfg(test)]
fn rows_with_large_cells<T>(
    columns: Vec<Column>,
    rows: impl Iterator<Item = T>,
    pagination: Pagination,
    owner: &str,
    cell_value: impl Fn(&T, usize, &Column) -> CellValue,
) -> fluxdb_core::Result<DataPage> {
    let mut page = DataPage {
        columns,
        rows: Vec::new(),
        offset: pagination.offset,
        limit: pagination.limit,
        has_more: false,
        incomplete: None,
    };
    for row in rows {
        let mut values = page
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| cell_value(&row, index, column))
            .collect::<Vec<_>>();
        spill_large_cells(owner, &mut values)?;
        page.rows.push(Row { values });
    }
    Ok(page)
}

/// 驱动的行流边读边收进一页：**不再 `fetch_all` 整个结果集**。
///
/// 这是大字段卡顿/爆内存的根因修复：`fetch_all` 会把每一行的完整值先堆进
/// `Vec<Row>`，一条 8 MB 的行乘以页大小就是几百 MB，整表结果更是几个 GB——
/// 预算与单格外置都在它之后才生效，救不了那次物化。改成流式后，单格超阈值
/// 立刻外置落盘，页容量由行数决定。
///
/// `skip_rows` 是流里已经取到、但不属于本页的行数（SQL 已带 OFFSET 时传 0，
/// 否则传 `pagination.offset`）。页容量由 `pagination.limit` 决定（0 = 不限制）。
/// `columns` 为空时按第一行的服务端描述补全
/// （查询编辑器不知道列结构，表浏览已知）。
/// 返回 `(页, 从流里读到的行数)`；后者只用于执行摘要，不代表「结果总行数」。
async fn collect_sqlx_page<S, R>(
    stream: S,
    columns: Vec<Column>,
    pagination: Pagination,
    skip_rows: u64,
    owner: &str,
    cell_value: impl Fn(&R, usize, &Column) -> CellValue,
    map_error: impl Fn(sqlx::Error) -> fluxdb_core::Error,
) -> fluxdb_core::Result<(DataPage, u64)>
where
    S: futures_util::Stream<Item = Result<R, sqlx::Error>>,
    R: sqlx::Row,
{
    futures_util::pin_mut!(stream);
    let mut page = DataPage {
        columns,
        rows: Vec::new(),
        offset: pagination.offset,
        limit: pagination.limit,
        has_more: false,
        incomplete: None,
    };
    let row_capacity = query_execution_row_capacity(pagination.limit);
    let mut read_rows = 0u64;
    while let Some(item) = futures_util::StreamExt::next(&mut stream).await {
        let row = item.map_err(&map_error)?;
        read_rows += 1;
        if page.columns.is_empty() {
            page.columns = sqlx_row_columns(&row);
        }
        if read_rows <= skip_rows {
            continue;
        }
        if page.rows.len() >= row_capacity {
            // 读满页容量：再往后还有行，这是「正常还有更多」，不是资源截断。
            page.has_more = true;
            break;
        }
        let mut values = page
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| cell_value(&row, index, column))
            .collect::<Vec<_>>();
        spill_large_cells(owner, &mut values)?;
        page.rows.push(Row { values });
    }
    if page.incomplete.is_some() {
        // 预算截断与「正常还有更多」互斥。
        page.has_more = false;
    }
    Ok((page, read_rows))
}

/// 按服务端描述构造列定义（查询编辑器结果用）。
fn sqlx_row_columns<R: sqlx::Row>(row: &R) -> Vec<Column> {
    row.columns()
        .iter()
        .map(|column| {
            Column {
                name: column.name().to_string(),
                type_name: Some(column.type_info().name().to_string()),
                nullable: true,
                primary_key: false,
                comment: None,
            }
        })
        .collect()
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
        return mysql_unsigned_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value.map(CellValue::I64).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return mysql_float_cell_value(value);
    }
    if let Ok(value) = row.try_get::<Option<BigDecimal>, _>(index) {
        // DECIMAL 保精：不经 f64 往返；无小数的精确整数投影为 I64，其余保留十进制文本。
        return mysql_decimal_cell_value(value);
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

/// MySQL 无符号整型投影：类型化数值让下游能区分真实数值与文本（产物数值列判定、
/// 回滚字面量等）。`CellValue` 只有有符号 `I64`，超出范围的值保留十进制文本以免丢精度。
fn mysql_unsigned_cell_value(value: Option<u64>) -> CellValue {
    match value {
        Some(value) => i64::try_from(value)
            .map(CellValue::I64)
            .unwrap_or_else(|_| CellValue::Text(value.to_string())),
        None => CellValue::Null,
    }
}

/// MySQL DECIMAL 投影：不经 f64 往返，保精度。
///
/// 无小数部分的精确整数（如 `SUM(int)`、`COUNT`、`FLOOR` 等以 DECIMAL 返回的聚合/函数结果）
/// 在 i64 范围内投影为类型化 `I64`：这些值本就是精确整数，若统一保留文本会被下游当作非数值列，
/// 无法作为图表/指标来源。含小数部分或超出 i64 范围（含 scale 为负的超大整数）的值仍保留真实
/// 十进制文本，避免丢精度。
fn mysql_decimal_cell_value(value: Option<BigDecimal>) -> CellValue {
    let Some(value) = value else {
        return CellValue::Null;
    };
    if value.fractional_digit_count() <= 0 {
        if let Ok(integer) = value.to_string().parse::<i64>() {
            return CellValue::I64(integer);
        }
    }
    CellValue::Text(value.to_string())
}

/// MySQL 浮点投影：有限值返回类型化 `F64`；非有限值不能作为合法 JSON 数字，
/// 保留类型化文本（与 PG 路径的处理一致）。
fn mysql_float_cell_value(value: Option<f64>) -> CellValue {
    match value {
        Some(value) if value.is_finite() => CellValue::F64(value),
        Some(value) => CellValue::Text(value.to_string()),
        None => CellValue::Null,
    }
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

    // SQLite 的列类型只是 affinity，真实存储类型按行决定。表浏览必须与
    // 查询编辑器一样先读取实际值类型，不能把所有非二进制值都转成文本：
    // 否则 INTEGER 主键在编辑器里是 I64、在表格里却是 Text，排序、编辑身份
    // 和 AI 结果投影会看到两套语义。
    if let Ok(value) = row.try_get::<Option<i64>, _>(column.name.as_str()) {
        return value.map(CellValue::I64).unwrap_or(CellValue::Null);
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(column.name.as_str()) {
        return value.map(CellValue::F64).unwrap_or(CellValue::Null);
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

#[cfg(test)]
mod page_collection_tests {
    use super::{rows_with_large_cells, spill_large_cells};
    use fluxdb_core::{
        CellContentMeta, CellContentStore, CellValue, Column, LargeValueKind, Pagination,
    };
    use std::sync::Arc;

    struct MemoryStore;

    impl CellContentStore for MemoryStore {
        fn put(&self, owner: &str, _kind: LargeValueKind, bytes: &[u8]) -> fluxdb_core::Result<String> {
            Ok(format!("{owner}-{}", bytes.len()))
        }

        fn stat(&self, _content_id: &str) -> Option<CellContentMeta> {
            None
        }

        fn read(
            &self,
            _content_id: &str,
            _offset: u64,
            _len: usize,
        ) -> fluxdb_core::Result<Vec<u8>> {
            Ok(Vec::new())
        }

        fn release(&self, _content_id: &str) {}

        fn release_owner(&self, _owner: &str) {}
    }

    /// 内容归属只做标识；内存桩不区分归属，固定一个即可。
    const OWNER: &str = "result-once-7";

    fn register_store() {
        fluxdb_core::register_cell_content_store(Arc::new(MemoryStore));
    }

    /// 单行超大文本外置成内容引用后仍保留这一行。
    #[test]
    fn oversized_text_row_is_kept_as_content_reference() {
        register_store();
        let text = "x".repeat(fluxdb_core::CELL_SPILL_THRESHOLD_BYTES + 1);
        let mut values = vec![CellValue::Text(text.clone())];
        spill_large_cells(OWNER, &mut values).expect("spill succeeds");
        let large = values[0].large_value().expect("内容引用");
        assert_eq!(large.byte_length, Some(text.len() as u64));
        assert!(large.preview_truncated, "预览必须标明只是片段");
        assert!(large.preview.len() < 4096, "页里只留预览");
    }

    /// JSON 与二进制同样只留预览 + 内容引用，行不能丢。
    #[test]
    fn oversized_json_and_binary_rows_are_kept_as_references() {
        register_store();
        let json = format!("{{\"note\":\"{}\"}}", "y".repeat(fluxdb_core::CELL_SPILL_THRESHOLD_BYTES));
        let binary = vec![0xABu8; fluxdb_core::CELL_SPILL_THRESHOLD_BYTES + 1];
        for (value, kind) in [
            (CellValue::Json(json.clone()), LargeValueKind::Json),
            (CellValue::Bytes(binary.clone()), LargeValueKind::Binary),
        ] {
            let mut values = vec![value];
            spill_large_cells(OWNER, &mut values).expect("spill succeeds");
            let large = values[0].large_value().expect("内容引用");
            assert_eq!(large.kind, kind);
            assert!(large.preview_truncated);
            // 引用里只有预览，不能塞整份内容：预览必须短于原值。
            let original_len = match kind {
                LargeValueKind::Json => json.len(),
                _ => binary.len(),
            };
            assert!((large.preview.len() as u64) < original_len as u64);
        }
    }

    /// 普通小字段保持原值，不因字段数量触发外置。
    #[test]
    fn fitting_row_keeps_page_complete() {
        register_store();
        let mut values = vec![CellValue::Text("ok".into())];
        spill_large_cells(OWNER, &mut values).unwrap();
        assert_eq!(values, vec![CellValue::Text("ok".into())]);
    }

    /// 页预算还很宽裕时，单格超过阈值也必须外置。
    ///
    /// 这是卡顿的根因：只有「整行超预算」才外置的话，一条 8 MB 单元格会作为
    /// 普通 `Text` 常驻页里，随后被渲染、事件、快照反复整串克隆。
    #[test]
    fn oversized_cell_spills_even_when_budget_is_ample() {
        register_store();
        let text = "x".repeat(8 * 1024 * 1024);
        let mut values = vec![CellValue::Text(text.clone())];
        spill_large_cells(OWNER, &mut values).unwrap();
        assert!(
            values[0].large_value().is_some(),
            "预算宽裕也必须把 8 MB 单元格换成内容引用"
        );
        let large = values[0].large_value().expect("内容引用");
        assert!(large.preview.len() < 8192, "页里只留预览");
    }

    /// 小字段合计超过旧 16 MiB 页预算也完整收页，不再制造 incomplete 短页。
    #[test]
    fn small_cells_over_old_page_budget_stay_complete() {
        register_store();
        let columns = vec![Column {
            name: "note".into(),
            type_name: Some("VARCHAR".into()),
            nullable: true,
            primary_key: false,
            comment: None,
        }];
        // 每行都低于单格外置阈值（所以都不会被外置），但加起来超过整页预算：
        // 收下的行数由预算决定，剩下的必须留给下一页。
        const ROW_BYTES: usize = 60_000;
        let expected_rows = (16 * 1024 * 1024 / ROW_BYTES) + 5;
        let rows = (0..expected_rows + 5)
            .map(|_| CellValue::Text("y".repeat(ROW_BYTES)))
            .collect::<Vec<_>>()
            .into_iter();
        let page = rows_with_large_cells(
            columns,
            rows,
            Pagination::new(0, 500),
            OWNER,
            |value: &CellValue, _index: usize, _column: &Column| value.clone(),
        )
        .expect("collect rows");
        assert_eq!(page.rows.len(), expected_rows + 5);
        assert!(page.incomplete.is_none(), "小字段总量不再截断页");
        assert!(!page.has_more);
    }
}
