/// Redis string / JSON 值详情加载结果：值文本 + 服务端字节长度 + 是否已拿到完整值。
///
/// `loaded_all` 为 true 时 `value` 是完整值（可直接编辑/复制）；为 false 时 `value`
/// 只是前 `REDIS_STRING_MAX_LENGTH` 字节的预览片段，回写它会用片段覆盖完整数据。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisStringValue {
    pub value: String,
    /// STRLEN 返回的字节长度（JSON 类型用字符数近似）。
    pub len: u64,
    /// 当前 `value` 是否已包含完整值（未截断）。
    pub loaded_all: bool,
}

/// 加载 string / JSON 值详情（见 [`RedisConnector::load_string_value`]）。
///
/// 先 `TYPE` 重新判定真实类型：`string` 走 STRLEN + GETRANGE（预览）或 GET（完整），
/// `rejson-rl`/`json` 走 `JSON.GET` 整取。其它类型不在本方法职责内，返回不支持。
fn redis_load_string_value(
    connection: &mut RedisConnection,
    key: &str,
    full: bool,
) -> fluxdb_core::Result<RedisStringValue> {
    let raw_type = redis_value_text(connection.command(&["TYPE", key])?);
    match raw_type.to_ascii_lowercase().as_str() {
        "string" => {
            let len = match connection.command(&["STRLEN", key])? {
                RedisValue::Int(len) => len.max(0) as u64,
                _ => return Err(Error::new(ErrorKind::Query, "Redis STRLEN 返回格式异常")),
            };
            if full {
                // 完整值：GET 整体取回，loaded_all 恒为 true。
                let value = redis_bulk_text(connection.command(&["GET", key])?).unwrap_or_default();
                Ok(RedisStringValue { value, len, loaded_all: true })
            } else {
                // 预览：GETRANGE 只取前 REDIS_STRING_MAX_LENGTH 字节；不足上限即视为完整。
                let end = (len as i64 - 1).min(REDIS_STRING_MAX_LENGTH);
                let value = match connection.command(&["GETRANGE", key, "0", &end.to_string()])? {
                    RedisValue::Bulk(Some(bytes)) => redis_utf8_bounded_text(bytes),
                    RedisValue::Bulk(None) => String::new(),
                    RedisValue::Simple(value) => value,
                    _ => return Err(Error::new(ErrorKind::Query, "Redis GETRANGE 返回格式异常")),
                };
                let loaded_all = len as i64 <= REDIS_STRING_MAX_LENGTH + 1;
                Ok(RedisStringValue { value, len, loaded_all })
            }
        }
        "rejson-rl" | "json" => {
            // JSON 文档走 JSON.GET 整取，天然是完整值。
            let value = redis_bulk_text(connection.command(&["JSON.GET", key])?).unwrap_or_default();
            let len = value.chars().count() as u64;
            Ok(RedisStringValue { value, len, loaded_all: true })
        }
        "none" => Err(Error::new(ErrorKind::Query, "Redis Key 不存在")),
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            format!("Redis 暂不支持读取 {raw_type} 类型的值"),
        )),
    }
}

/// 下载 string / JSON 原始字节（见 [`RedisConnector::download_string_value`]）。
///
/// 先 `TYPE` 重新判定真实类型：`string` 走 `GET`（返回原始 bytes），
/// `rejson-rl`/`json` 走 `JSON.GET`。其它类型不在本方法职责内，返回不支持。
fn redis_download_string_value(
    connection: &mut RedisConnection,
    key: &str,
) -> fluxdb_core::Result<Vec<u8>> {
    let raw_type = redis_value_text(connection.command(&["TYPE", key])?);
    match raw_type.to_ascii_lowercase().as_str() {
        "string" => match connection.command(&["GET", key])? {
            RedisValue::Bulk(Some(bytes)) => Ok(bytes),
            RedisValue::Bulk(None) => Ok(Vec::new()),
            RedisValue::Simple(value) if value.is_empty() => Ok(Vec::new()),
            RedisValue::Simple(value) => Ok(value.into_bytes()),
            _ => Err(Error::new(ErrorKind::Query, "Redis GET 返回格式异常")),
        },
        "rejson-rl" | "json" => match connection.command(&["JSON.GET", key])? {
            RedisValue::Bulk(Some(bytes)) => Ok(bytes),
            RedisValue::Bulk(None) => Ok(Vec::new()),
            RedisValue::Simple(value) => Ok(value.into_bytes()),
            _ => Err(Error::new(ErrorKind::Query, "Redis JSON.GET 返回格式异常")),
        },
        "none" => Err(Error::new(ErrorKind::Query, "Redis Key 不存在")),
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            format!("Redis 暂不支持读取 {raw_type} 类型的值"),
        )),
    }
}
