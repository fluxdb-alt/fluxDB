// Oracle 取值与绑定：把驱动返回值映射成 `CellValue`，把编辑值映射成真绑定。
//
// 三条硬约束：
// 1. `NUMBER` 一律走 `OracleNumber::to_string()`，绝不经过 f64（19 位精度会丢）。
// 2. `DATE` 自带时分秒，按时间戳展示，不依赖会话 `NLS_DATE_FORMAT`。
// 3. CLOB/BLOB 只取有界预览，绝不把整个 LOB 读进内存或事件负载。

/// CLOB 预览的最大字符数；超出部分标记截断，完整内容走 `load_cell_text` 按需读取。
const ORACLE_LOB_TEXT_PREVIEW_CHARS: usize = 4_000;
/// BLOB 预览的十六进制字节数。
const ORACLE_LOB_BINARY_PREVIEW_BYTES: usize = 256;
/// 按需读取完整字符 LOB 的字节上限。超过直接报错，不把无界内容读进内存。
const ORACLE_LOB_TEXT_FULL_MAX_BYTES: usize = 8 * 1024 * 1024;

fn oracle_meta_text(row: &OracleRow, index: usize) -> Option<String> {
    if let Ok(Some(value)) = row.get::<Option<String>>(index) {
        return Some(value);
    }
    if let Ok(Some(value)) = row.get::<Option<OracleNumber>>(index) {
        return Some(value.to_string());
    }
    if let Ok(Some(value)) = row.get::<Option<OracleTimestamp>>(index) {
        return Some(oracle_timestamp_text(&value));
    }
    None
}

fn oracle_meta_u64(row: &OracleRow, index: usize) -> Option<u64> {
    if let Ok(Some(value)) = row.get::<Option<OracleNumber>>(index) {
        return value.to_string().trim().parse().ok();
    }
    oracle_meta_text(row, index)?.trim().parse().ok()
}

fn oracle_meta_i64(row: &OracleRow, index: usize) -> Option<i64> {
    if let Ok(Some(value)) = row.get::<Option<OracleNumber>>(index) {
        return value.to_string().trim().parse().ok();
    }
    oracle_meta_text(row, index)?.trim().parse().ok()
}

/// 单个单元格取值。`row` 需要可变引用，因为 CLOB/BLOB 只能通过 `take` 拿到所有权。
fn oracle_cell_value(row: &mut OracleRow, index: usize, column: &Column) -> CellValue {
    if oracle_column_is_lob(row, index) {
        return oracle_lob_cell_value(row, index, column);
    }
    // `Option<T>` 探针：NULL 列对所有探针都返回 `Ok(None)`，类型不符返回 `Err`。
    // 因此第一个成功的 `Ok(None)` 就代表这一列确实是 NULL。
    match row.get::<Option<String>>(index) {
        Ok(Some(value)) => return CellValue::Text(value),
        Ok(None) => return CellValue::Null,
        Err(_) => {}
    }
    match row.get::<Option<OracleNumber>>(index) {
        // NUMBER 的精确文本表示：既不丢精度，也不需要猜测列声明的标度。
        Ok(Some(value)) => return CellValue::Text(value.to_string()),
        Ok(None) => return CellValue::Null,
        Err(_) => {}
    }
    match row.get::<Option<OracleTimestamp>>(index) {
        Ok(Some(value)) => return CellValue::Text(oracle_timestamp_text(&value)),
        Ok(None) => return CellValue::Null,
        Err(_) => {}
    }
    match row.get::<Option<f64>>(index) {
        Ok(Some(value)) if value.is_finite() => return CellValue::F64(value),
        Ok(Some(value)) => return CellValue::Text(value.to_string()),
        Ok(None) => return CellValue::Null,
        Err(_) => {}
    }
    match row.get::<Option<bool>>(index) {
        Ok(Some(value)) => return CellValue::Bool(value),
        Ok(None) => return CellValue::Null,
        Err(_) => {}
    }
    match row.get::<Option<Vec<u8>>>(index) {
        Ok(Some(value)) => return oracle_bytes_cell_value(column, &value),
        Ok(None) => return CellValue::Null,
        Err(_) => {}
    }
    match row.get::<Option<OracleJsonValue>>(index) {
        Ok(Some(value)) => return CellValue::Json(oracle_json_text(&value)),
        Ok(None) => return CellValue::Null,
        Err(_) => {}
    }
    match row.get::<Option<oracledb::Vector>>(index) {
        Ok(Some(value)) => return CellValue::Text(oracle_vector_text(&value)),
        Ok(None) => return CellValue::Null,
        Err(_) => {}
    }
    // 无法转换不等于 NULL：明确标注类型，避免把数据静默显示成空值。
    CellValue::Text(format!(
        "(无法展示的类型：{})",
        column.type_name.as_deref().unwrap_or("UNKNOWN")
    ))
}

fn oracle_column_is_lob(row: &OracleRow, index: usize) -> bool {
    let Some(metadata) = row.columns().get(index) else {
        return false;
    };
    oracle_db_type_is_character_lob(metadata.db_type()) || oracle_db_type_is_lob(metadata.db_type())
}

fn oracle_db_type_is_lob(db_type: &oracledb::DbType) -> bool {
    *db_type == *oracledb::DB_TYPE_CLOB
        || *db_type == *oracledb::DB_TYPE_NCLOB
        || *db_type == *oracledb::DB_TYPE_BLOB
}

/// 字符 LOB（CLOB/NCLOB）：按字符取预览，不是二进制摘要。
fn oracle_db_type_is_character_lob(db_type: &oracledb::DbType) -> bool {
    *db_type == *oracledb::DB_TYPE_CLOB || *db_type == *oracledb::DB_TYPE_NCLOB
}

fn oracle_lob_cell_value(row: &mut OracleRow, index: usize, column: &Column) -> CellValue {
    let is_character = row
        .columns()
        .get(index)
        .is_some_and(|metadata| oracle_db_type_is_character_lob(metadata.db_type()));
    let Ok(value) = row.take::<Option<OracleLob>>(index) else {
        return CellValue::Text("(LOB 读取失败)".to_string());
    };
    let Some(mut lob) = value else {
        return CellValue::Null;
    };
    let byte_length = lob.get_size().unwrap_or(0) as u64;
    if is_character {
        let (text, truncated) = read_lob_text(&mut lob, ORACLE_LOB_TEXT_PREVIEW_CHARS);
        // 字符 LOB 的 `get_size()` 是字符数，这里按同一口径展示，不做字节换算。
        if truncated {
            // 预览不能伪装成完整值：用独立类型表达，导出/备份/写回据此拒绝。
            return CellValue::TruncatedText {
                preview: text,
                total_chars: byte_length,
            };
        }
        return CellValue::Text(text);
    }
    let preview = read_lob_bytes(&mut lob, ORACLE_LOB_BINARY_PREVIEW_BYTES);
    CellValue::BinarySummary(BinaryCellSummary {
        type_name: column
            .type_name
            .clone()
            .unwrap_or_else(|| "BLOB".to_string()),
        is_null: false,
        byte_length,
        preview_hex: Some(preview),
    })
}

/// 有界读取字符 LOB；返回文本与是否被截断。
///
/// 先按字节读满预算再整体解码：逐块 `from_utf8_lossy` 会把跨块的多字节字符切成
/// 替换字符，预览里凭空多出 `�`。
///
/// 预算取 `4 * max_chars`（UTF-8 单字符最多 4 字节）：超过这个字节数必然超过字符上限，
/// 因此「预算内且读到 EOF」就等价于完整值，不必再读一次；恰好 `max_chars` 的值也不会
/// 被误判成截断。读取出错时保守返回 `true`——半截内容当成完整值写回去会丢数据。
fn read_lob_text(lob: &mut impl std::io::Read, max_chars: usize) -> (String, bool) {
    let budget = max_chars.saturating_mul(4).saturating_add(4);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut eof = false;
    while bytes.len() < budget {
        match std::io::Read::read(lob, &mut chunk) {
            Ok(0) => {
                eof = true;
                break;
            }
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            Err(error) => {
                tracing::warn!(target: "fluxdb_connectors", message = %error, "Oracle CLOB 读取中断，按预览处理");
                let text = String::from_utf8_lossy(&bytes).into_owned();
                return (text.chars().take(max_chars).collect(), true);
            }
        }
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let chars = text.chars().count();
    if chars > max_chars {
        return (text.chars().take(max_chars).collect(), true);
    }
    (text, !eof)
}

/// 无界读取字符 LOB，超过 `max_bytes` 直接失败。
///
/// 详情面板/编辑需要完整值，但「完整」不等于「无限」：超限宁可报错让用户走导出，
/// 也不把可能上 GB 的 LOB 读进内存。
fn read_lob_text_unbounded(lob: &mut impl std::io::Read, max_bytes: usize) -> fluxdb_core::Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match std::io::Read::read(lob, &mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                if buffer.len() + read > max_bytes {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        format!("文本超过 {} MiB，请改用导出查看完整内容", max_bytes / (1024 * 1024)),
                    ));
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            Err(error) => {
                // 读到一半失败不能返回半截内容：调用方会把它当完整值写回。
                return Err(Error::new(
                    ErrorKind::Query,
                    format!("读取 CLOB 失败：{error}"),
                ));
            }
        }
    }
    String::from_utf8(buffer)
        .map_err(|_| Error::new(ErrorKind::Query, "CLOB 内容不是合法 UTF-8"))
}

fn read_lob_bytes(lob: &mut OracleLob, max_bytes: usize) -> String {
    let mut buffer = vec![0u8; max_bytes];
    let mut filled = 0usize;
    while filled < max_bytes {
        match std::io::Read::read(lob, &mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => break,
        }
    }
    buffer[..filled]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

fn oracle_bytes_cell_value(column: &Column, value: &[u8]) -> CellValue {
    // RAW/LONG RAW 是定长二进制，按字节展示；BLOB 已在 LOB 分支处理。
    if value.len() > ORACLE_LOB_BINARY_PREVIEW_BYTES {
        return CellValue::BinarySummary(BinaryCellSummary {
            type_name: column
                .type_name
                .clone()
                .unwrap_or_else(|| "RAW".to_string()),
            is_null: false,
            byte_length: value.len() as u64,
            preview_hex: Some(
                value[..ORACLE_LOB_BINARY_PREVIEW_BYTES]
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect(),
            ),
        });
    }
    CellValue::Bytes(value.to_vec())
}

/// 时间戳展示文本。Oracle `DATE` 与 `TIMESTAMP` 都按 `YYYY-MM-DD HH:MM:SS[.fffffffff]`
/// 展示。驱动 26.0.0-beta.5 不再暴露时区偏移（只有 `is_utc` 标记），因此带时区的列
/// 按驱动返回的墙上时间原样展示，不追加可能错误的偏移。
fn oracle_timestamp_text(value: &OracleTimestamp) -> String {
    let mut text = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        value.year(),
        value.month(),
        value.day(),
        value.hour(),
        value.minute(),
        value.second()
    );
    let nanos = value.nanoseconds();
    if nanos > 0 {
        let fraction = format!("{nanos:09}");
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    text
}

/// JSON 值转文本。驱动只给出结构体，没有 `Display`/`Serialize`，这里自己渲染。
fn oracle_json_text(value: &OracleJsonValue) -> String {
    match value {
        OracleJsonValue::Null => "null".to_string(),
        OracleJsonValue::Boolean(value) => value.to_string(),
        OracleJsonValue::Number(value) => value.to_string(),
        OracleJsonValue::BinaryDouble(value) => value.to_string(),
        OracleJsonValue::BinaryFloat(value) => value.to_string(),
        OracleJsonValue::String(value) => quote_json_string(value),
        OracleJsonValue::Timestamp(value) => quote_json_string(&oracle_timestamp_text(value)),
        OracleJsonValue::IntervalDS(value) => quote_json_string(&value.to_string()),
        OracleJsonValue::IntervalYM(value) => quote_json_string(&value.to_string()),
        OracleJsonValue::JsonArray(items) => {
            let items = items.iter().map(oracle_json_text).collect::<Vec<_>>();
            format!("[{}]", items.join(","))
        }
        OracleJsonValue::JsonObject(fields) => {
            let mut entries = fields
                .iter()
                .map(|(key, value)| format!("{}:{}", quote_json_string(key), oracle_json_text(value)))
                .collect::<Vec<_>>();
            entries.sort();
            format!("{{{}}}", entries.join(","))
        }
        // 二进制节点（OSON id / raw / vector）没有稳定的文本形式，用占位说明。
        OracleJsonValue::JsonId(_) => "\"(json id)\"".to_string(),
        OracleJsonValue::Raw(bytes) => {
            format!("\"(raw {} bytes)\"", bytes.len())
        }
        OracleJsonValue::Vector(vector) => quote_json_string(&oracle_vector_text(vector)),
    }
}

fn quote_json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn oracle_vector_text(vector: &oracledb::Vector) -> String {
    // 驱动的 `Vector::num_dimensions` 是私有的，这里按公开变体取维数。
    let dimensions = match vector {
        oracledb::Vector::Dense(data) => match data {
            oracledb::VectorData::Float32(values) => values.len(),
            oracledb::VectorData::Float64(values) => values.len(),
            oracledb::VectorData::Int8(values) => values.len(),
            oracledb::VectorData::Binary(values) => values.len(),
        },
        oracledb::Vector::Sparse(sparse) => sparse.num_dimensions(),
    };
    format!("VECTOR({dimensions} 维)")
}

/// 编辑值 → 绑定值。Oracle 的 SQL 层没有布尔字面量，布尔按 1/0 写回。
fn oracle_bind_cell(value: &CellValue) -> OracleBindValue {
    match value {
        CellValue::Null => OracleBindValue::Null,
        CellValue::Bool(value) => OracleBindValue::Integer(i64::from(*value)),
        CellValue::I64(value) => OracleBindValue::Integer(*value),
        CellValue::F64(value) if value.is_finite() => OracleBindValue::Float(*value),
        CellValue::F64(value) => OracleBindValue::Text(value.to_string()),
        CellValue::Text(value) | CellValue::Json(value) => OracleBindValue::Text(value.clone()),
        // 预览不是完整值：绑进去等于用片段覆盖真值。
        CellValue::TruncatedText { .. } => OracleBindValue::Null,
        // 大字段只有预览：同样不能拿片段覆盖真值。
        CellValue::LargeValue(_) => OracleBindValue::Null,
        CellValue::Bytes(value) => OracleBindValue::Bytes(value.clone()),
        // 二进制预览不是真值，只有完整字节才能写回。
        CellValue::BinarySummary(_) => OracleBindValue::Null,
    }
}

fn oracle_bind_filter(value: &fluxdb_core::FilterValue) -> OracleBindValue {
    use fluxdb_core::FilterValue;
    match value {
        FilterValue::Null => OracleBindValue::Null,
        FilterValue::Text(value) => OracleBindValue::Text(value.clone()),
        FilterValue::Integer(value) => OracleBindValue::Integer(*value),
        FilterValue::Float(value) if value.is_finite() => OracleBindValue::Float(*value),
        FilterValue::Float(value) => OracleBindValue::Text(value.to_string()),
        FilterValue::Boolean(value) => OracleBindValue::Integer(i64::from(*value)),
    }
}

/// 参数化执行绑定。十进制文本按整数或 `OracleNumber` 绑定，绝不转 f64。
fn oracle_bind_parameter(
    value: &fluxdb_core::SqlParameter,
) -> fluxdb_core::Result<OracleBindValue> {
    use fluxdb_core::SqlParameter;
    Ok(match value {
        SqlParameter::Null => OracleBindValue::Null,
        SqlParameter::Bool(value) => OracleBindValue::Integer(i64::from(*value)),
        SqlParameter::I64(value) | SqlParameter::U64(value) | SqlParameter::Decimal(value) => {
            oracle_bind_decimal(value)?
        }
        SqlParameter::F64(value) if value.is_finite() => OracleBindValue::Float(*value),
        SqlParameter::F64(_) => {
            return Err(Error::new(ErrorKind::Query, "参数化绑定拒绝非有限浮点数"));
        }
        SqlParameter::Text(value) => OracleBindValue::Text(value.clone()),
        SqlParameter::Bytes(value) => OracleBindValue::Bytes(value.clone()),
    })
}

fn oracle_bind_decimal(value: &str) -> fluxdb_core::Result<OracleBindValue> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::new(ErrorKind::Query, "参数化绑定缺少数值"));
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return Ok(OracleBindValue::Integer(integer));
    }
    // 超出 i64 或带小数：交给驱动的定长十进制解析，保持精确值。
    trimmed
        .parse::<OracleNumber>()
        .map(OracleBindValue::Number)
        .map_err(|_| Error::new(ErrorKind::Query, format!("无法绑定的数值参数：{trimmed}")))
}

/// 结果集列描述。类型名以驱动元数据为准；表数据页会用目录里的声明覆盖它，
/// 因为驱动按字节给出 CHAR/VARCHAR2 长度，而目录区分 BYTE/CHAR 语义。
fn oracle_column(metadata: &OracleMetadata, primary_key: bool) -> Column {
    Column {
        name: metadata.name().to_string(),
        type_name: Some(metadata.data_type()),
        nullable: metadata.nullable(),
        primary_key,
        comment: None,
    }
}

fn oracle_row(row: &mut OracleRow, columns: &[Column]) -> Row {
    let values = (0..columns.len())
        .map(|index| oracle_cell_value(row, index, &columns[index]))
        .collect();
    Row { values }
}

#[cfg(test)]
mod oracle_lob_text_tests {
    use super::{read_lob_text, read_lob_text_unbounded, ORACLE_LOB_TEXT_PREVIEW_CHARS};

    /// 恰好读满预览上限的值是完整的：必须多读一次到 EOF 才能判定，不能因为
    /// 「已经读够上限」就标成截断（否则 4000 字符的 CLOB 永远无法编辑/导出）。
    #[test]
    fn exactly_preview_limit_is_not_truncated() {
        let text = "a".repeat(ORACLE_LOB_TEXT_PREVIEW_CHARS);
        let (read, truncated) = read_lob_text(&mut text.as_bytes(), ORACLE_LOB_TEXT_PREVIEW_CHARS);
        assert_eq!(read, text);
        assert!(!truncated);
    }

    #[test]
    fn one_char_past_preview_limit_is_truncated() {
        let text = "a".repeat(ORACLE_LOB_TEXT_PREVIEW_CHARS + 1);
        let (read, truncated) = read_lob_text(&mut text.as_bytes(), ORACLE_LOB_TEXT_PREVIEW_CHARS);
        assert_eq!(read.chars().count(), ORACLE_LOB_TEXT_PREVIEW_CHARS);
        assert!(truncated);
    }

    /// 多字节字符按字符计数：按字节截断会切出乱码，且上限口径与 `get_size()` 不一致。
    #[test]
    fn truncation_counts_characters_not_bytes() {
        let text = "中".repeat(ORACLE_LOB_TEXT_PREVIEW_CHARS + 1);
        let (read, truncated) = read_lob_text(&mut text.as_bytes(), ORACLE_LOB_TEXT_PREVIEW_CHARS);
        assert_eq!(read.chars().count(), ORACLE_LOB_TEXT_PREVIEW_CHARS);
        assert!(read.chars().all(|ch| ch == '中'));
        assert!(truncated);
    }

    #[test]
    fn full_read_returns_everything() {
        let text = "内容".repeat(100);
        assert_eq!(
            read_lob_text_unbounded(&mut text.as_bytes(), 1024).unwrap(),
            text
        );
    }

    /// 超限必须报错：截断返回会让调用方把半截当完整值写回。
    #[test]
    fn full_read_rejects_values_over_the_cap() {
        let text = "a".repeat(64);
        assert!(read_lob_text_unbounded(&mut text.as_bytes(), 32).is_err());
    }
}
