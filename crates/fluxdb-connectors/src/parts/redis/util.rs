fn redis_glob_escape(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// 选库；复用的连接若已在目标 DB 上则跳过，省掉一次往返。
fn redis_select(connection: &mut RedisConnection, database: u32) -> fluxdb_core::Result<()> {
    if connection.database == Some(database) {
        return Ok(());
    }
    let result = redis_expect_ok(connection.command(&["SELECT", &database.to_string()])?);
    connection.database = result.is_ok().then_some(database);
    result
}

/// 解析对象路径里的目标数据库号。
/// `database` 未显式指定（None）时默认落在 0 号库；绝不能把 key 名当库号——
/// 否则当 key 名恰好是纯数字时会把 SELECT 打到错误的库，从而在错库里
/// 对同名 key 执行 LLEN/LINDEX 触发 WRONGTYPE。
fn redis_path_database(path: &ObjectPath) -> fluxdb_core::Result<u32> {
    path.database
        .as_deref()
        .unwrap_or("0")
        .parse::<u32>()
        .map_err(|_| Error::new(ErrorKind::Query, "Redis DB 必须是数字"))
}

fn redis_expect_ok(value: RedisValue) -> fluxdb_core::Result<()> {
    match value {
        RedisValue::Simple(value) if value.eq_ignore_ascii_case("OK") || value == "PONG" => Ok(()),
        _ => Err(Error::new(ErrorKind::Connection, "Redis 返回格式异常")),
    }
}

fn redis_value_text(value: RedisValue) -> String {
    match value {
        RedisValue::Simple(value) => value,
        RedisValue::Int(value) => value.to_string(),
        RedisValue::Bulk(Some(value)) => redis_text_from_bytes(value),
        RedisValue::Bulk(None) => String::new(),
        RedisValue::Array(_) => String::new(),
    }
}

/// 二进制内容占位符前缀，用 U+FFFC（对象替换符）打头：
/// 用户键盘敲不出来，因此可以拿它可靠地识别「这条值没有被真实解码过」。
const REDIS_BINARY_MARKER: char = '\u{FFFC}';

/// 把 Redis 返回的字节解码成可展示文本。
/// 非 UTF-8 的内容（protobuf / gzip / 序列化对象等）不做 lossy 转换——
/// lossy 会把不可解码字节换成 U+FFFD，一旦回写就把原始数据写坏了。
fn redis_text_from_bytes(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let len = error.as_bytes().len();
            format!("{REDIS_BINARY_MARKER}<二进制 {len} 字节，不可编辑>")
        }
    }
}

/// String 预览的字节上限，对齐 RedisInsight 的 5000 字节缓冲（GETRANGE 0..=4999）。
/// 不可配置：这是 string 详情「值是否完整」判定的固定阈值。
const REDIS_STRING_MAX_LENGTH: i64 = 4999;

/// 把 `GETRANGE` 取回的字节片段解码成文本，容忍末尾被切坏的多字节 UTF-8 序列。
///
/// GETRANGE 按字节切片，可能把一个多字节字符从中间切开；此时只需丢掉末尾 1~3 个
/// 不完整的续字节就能还原前缀。若丢到 3 字节后仍无法解码，说明值本身不是合法 UTF-8，
/// 按二进制占位符处理（与 [`redis_text_from_bytes`] 语义一致）。
fn redis_utf8_bounded_text(bytes: Vec<u8>) -> String {
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        return text;
    }
    // UTF-8 单字符最长 4 字节：最多回退 3 字节即可修好被切坏的末字符。
    let mut cut = 0;
    while cut <= 3 && cut < bytes.len() {
        if let Ok(text) = String::from_utf8(bytes[..bytes.len() - cut].to_vec()) {
            return text;
        }
        cut += 1;
    }
    let len = bytes.len();
    format!("{REDIS_BINARY_MARKER}<二进制 {len} 字节，不可编辑>")
}

/// 判断一段文本是否是上面那个占位符。
fn redis_is_binary_placeholder(value: &str) -> bool {
    value.starts_with(REDIS_BINARY_MARKER)
}

/// 写入前的保护：占位符代表原值是二进制且未被解码，回写它等于毁数据。
fn redis_reject_binary_placeholder(value: &str) -> fluxdb_core::Result<()> {
    if redis_is_binary_placeholder(value) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "该内容是二进制数据，无法在表格中编辑；请改用支持二进制的客户端写入",
        ));
    }
    Ok(())
}

/// 检查字符串值里是否含有「非法控制字符」，返回第一个命中字符。
///
/// 对齐 RedisInsight「不要把二进制垃圾 / 异常控制字符写进 key」的意图：文本值编辑器应
/// 拒绝无法被正常表示或会造成数据损坏的控制字符。文本中合法的制表符 / 换行 / 回车
/// （`\t` `\n` `\r`）予以放行，其余 C0/C1/DEL 等控制字符（含 NUL、ESC）一律拦截。
/// 返回 `None` 表示值合法。
pub fn redis_value_illegal_control_char(value: &str) -> Option<char> {
    value
        .chars()
        .find(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
}

fn redis_bulk_text(value: RedisValue) -> Option<String> {
    match value {
        RedisValue::Bulk(Some(value)) => Some(redis_text_from_bytes(value)),
        RedisValue::Simple(value) => Some(value),
        _ => None,
    }
}

fn redis_bulk_text_opt(value: Option<RedisValue>) -> Option<String> {
    value.and_then(redis_bulk_text)
}

fn redis_parse_scan_pairs(
    value: RedisValue,
    limit: usize,
) -> fluxdb_core::Result<(String, Vec<(String, String)>)> {
    let RedisValue::Array(items) = value else {
        return Err(Error::new(ErrorKind::Query, "Redis 扫描返回格式异常"));
    };
    let [next_cursor, values] = items.as_slice() else {
        return Err(Error::new(ErrorKind::Query, "Redis 扫描返回格式异常"));
    };
    let next_cursor = redis_value_text(next_cursor.clone());
    let RedisValue::Array(values) = values.clone() else {
        return Err(Error::new(ErrorKind::Query, "Redis 扫描返回格式异常"));
    };
    let pairs = redis_parse_pairs_array(values, limit)?;
    Ok((next_cursor, pairs))
}

fn redis_parse_range_values(value: RedisValue) -> fluxdb_core::Result<Vec<String>> {
    let RedisValue::Array(items) = value else {
        return Err(Error::new(ErrorKind::Query, "Redis 范围返回格式异常"));
    };
    Ok(items.into_iter().map(redis_value_text).collect())
}

fn redis_parse_range_pairs(value: RedisValue) -> fluxdb_core::Result<Vec<(String, String)>> {
    let RedisValue::Array(items) = value else {
        return Err(Error::new(ErrorKind::Query, "Redis 范围返回格式异常"));
    };
    redis_parse_pairs_array(items, usize::MAX)
}

fn redis_parse_pairs_array(
    items: Vec<RedisValue>,
    limit: usize,
) -> fluxdb_core::Result<Vec<(String, String)>> {
    if items.len() % 2 != 0 {
        return Err(Error::new(ErrorKind::Query, "Redis 返回格式异常"));
    }
    let mut pairs = Vec::new();
    for pair in items.chunks(2).take(limit) {
        if let [first, second] = pair {
            pairs.push((redis_value_text(first.clone()), redis_value_text(second.clone())));
        }
    }
    Ok(pairs)
}

fn redis_read_value(
    reader: &mut std::io::BufReader<RedisStream>,
) -> fluxdb_core::Result<RedisValue> {
    let mut prefix = [0_u8; 1];
    use std::io::Read;
    reader.read_exact(&mut prefix).map_err(redis_io_error)?;
    match prefix[0] {
        b'+' => Ok(RedisValue::Simple(redis_read_line(reader)?)),
        b'-' => Err(Error::new(ErrorKind::Query, redis_read_line(reader)?)),
        b':' => redis_read_line(reader)?
            .parse::<i64>()
            .map(RedisValue::Int)
            .map_err(|_| Error::new(ErrorKind::Query, "Redis 整数返回格式异常")),
        b'$' => {
            let len = redis_read_line(reader)?
                .parse::<isize>()
                .map_err(|_| Error::new(ErrorKind::Query, "Redis Bulk 返回格式异常"))?;
            if len < 0 {
                return Ok(RedisValue::Bulk(None));
            }
            let mut bytes = vec![0_u8; len as usize];
            reader.read_exact(&mut bytes).map_err(redis_io_error)?;
            let mut crlf = [0_u8; 2];
            reader.read_exact(&mut crlf).map_err(redis_io_error)?;
            Ok(RedisValue::Bulk(Some(bytes)))
        }
        b'*' => {
            let len = redis_read_line(reader)?
                .parse::<isize>()
                .map_err(|_| Error::new(ErrorKind::Query, "Redis Array 返回格式异常"))?;
            if len < 0 {
                return Ok(RedisValue::Array(Vec::new()));
            }
            let mut values = Vec::with_capacity(len as usize);
            for _ in 0..len {
                values.push(redis_read_value(reader)?);
            }
            Ok(RedisValue::Array(values))
        }
        _ => Err(Error::new(ErrorKind::Query, "Redis 返回格式异常")),
    }
}

fn redis_read_line(reader: &mut std::io::BufReader<RedisStream>) -> fluxdb_core::Result<String> {
    let mut bytes = Vec::new();
    use std::io::BufRead;
    reader.read_until(b'\n', &mut bytes).map_err(redis_io_error)?;
    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn redis_io_error(error: std::io::Error) -> Error {
    Error::new(ErrorKind::Connection, error.to_string())
}
