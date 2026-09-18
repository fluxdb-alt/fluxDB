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

/// —— 服务端可控尺寸的上限 ——
///
/// 对端可能是用户填错的端口、被中间人改写、或本身就是恶意服务：声明长度、元素个数和嵌套深度
/// 都不能直接用来决定本地分配或递归深度，否则一条 `$9223372036854775807` 就能把整个进程打挂
/// （分配失败与栈溢出都是 abort，`catch_unwind` 拦不住；UI 主线程上 panic 同样会 abort）。
/// bulk 上限对齐 Redis 自身的 `proto-max-bulk-len` 默认值（512MB），不会拒掉合法大 value。
const REDIS_MAX_BULK_LEN: u64 = 512 * 1024 * 1024;
/// 单行（前缀行 / 简单字符串 / 错误消息）上限：真实回包都是短行，留足余量即可。
const REDIS_MAX_LINE_LEN: usize = 1024 * 1024;
/// 数组嵌套深度上限：RESP2 只在 MULTI/EXEC 下嵌套，真实深度个位数。
const REDIS_MAX_DEPTH: usize = 64;

fn redis_read_value(
    reader: &mut std::io::BufReader<RedisStream>,
) -> fluxdb_core::Result<RedisValue> {
    redis_read_value_at(reader, 0)
}

fn redis_read_value_at(
    reader: &mut std::io::BufReader<RedisStream>,
    depth: usize,
) -> fluxdb_core::Result<RedisValue> {
    if depth >= REDIS_MAX_DEPTH {
        return Err(protocol_error("Redis 回包嵌套层级异常"));
    }
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
            let len = u64::try_from(len).map_err(|_| protocol_error("Redis Bulk 返回长度异常"))?;
            if len > REDIS_MAX_BULK_LEN {
                return Err(protocol_error(&format!(
                    "Redis Bulk 返回长度 {len} 超出上限 {REDIS_MAX_BULK_LEN}"
                )));
            }
            // 只按实际到达的字节增长缓冲，不信任对端声明的长度：上限内的荒谬值最多多读到超时。
            let mut bytes = Vec::new();
            reader
                .by_ref()
                .take(len)
                .read_to_end(&mut bytes)
                .map_err(redis_io_error)?;
            if bytes.len() as u64 != len {
                return Err(protocol_error("Redis Bulk 返回长度与声明不一致"));
            }
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
            // 不 with_capacity(对端声明值)：元素随实际读到的数量增长，谎报大数只会在读到下一个
            // 元素时报错。真实大集合（数百万 member）仍按需要自然增长。
            let mut values = Vec::with_capacity(std::cmp::min(len as usize, 64));
            for _ in 0..len {
                values.push(redis_read_value_at(reader, depth + 1)?);
            }
            Ok(RedisValue::Array(values))
        }
        // 首字节不是任何 RESP 前缀：真实场景几乎都是端口填错（连到了 MySQL/HTTP 等），
        // 也可能是流已错位；两种都不能再复用这条连接。
        other => Err(Error::new(
            ErrorKind::Connection,
            format!(
                "未收到 Redis 协议响应（首字节 0x{other:02x}），请确认填写的是 Redis 服务端口（默认 6379）"
            ),
        )),
    }
}

fn redis_read_line(reader: &mut std::io::BufReader<RedisStream>) -> fluxdb_core::Result<String> {
    let mut bytes = Vec::new();
    use std::io::{BufRead, Read};
    // 对端不发 \n 时 read_until 会一直累积，必须带上限，否则内存随字节流无界增长。
    reader
        .by_ref()
        .take(REDIS_MAX_LINE_LEN as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(redis_io_error)?;
    if bytes.len() > REDIS_MAX_LINE_LEN {
        return Err(protocol_error("Redis 回包单行超出上限"));
    }
    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// 协议层异常统一按连接错误返回：调用方据此作废整条连接，不能当普通查询错误继续复用，
/// 否则半条响应还留在缓冲里，后续每条命令都会读到错位数据。
fn protocol_error(message: &str) -> Error {
    Error::new(ErrorKind::Connection, message.to_string())
}

fn redis_io_error(error: std::io::Error) -> Error {
    Error::new(ErrorKind::Connection, error.to_string())
}

#[cfg(test)]
mod resp_size_guard_tests {
    use super::*;

    /// 用回环 socket 造一个「按原样吐字节然后等一会儿再关」的假 Redis 端点，返回客户端读端。
    fn reader_for(payload: &[u8]) -> std::io::BufReader<RedisStream> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("假 Redis 端点应可监听");
        let port = listener.local_addr().expect("假端点地址").port();
        let payload = payload.to_vec();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            use std::io::Write;
            let _ = stream.write_all(&payload);
            let _ = stream.flush();
            // 留出时间让客户端读完，避免 EOF 抢在解析完成之前。
            std::thread::sleep(Duration::from_millis(300));
        });
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("连接假 Redis 端点");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("设置读超时");
        std::io::BufReader::new(RedisStream::Plain(stream))
    }

    fn is_connection_error(result: fluxdb_core::Result<RedisValue>) {
        let error = result.err().expect("畸形回包必须报错");
        assert_eq!(error.kind, ErrorKind::Connection, "协议异常要作废连接: {error}");
    }

    #[test]
    fn legit_replies_still_parse() {
        assert_eq!(
            redis_read_value(&mut reader_for(b"$5\r\nhello\r\n")).expect("合法 bulk"),
            RedisValue::Bulk(Some(b"hello".to_vec()))
        );
        assert_eq!(
            redis_read_value(&mut reader_for(b"$-1\r\n")).expect("合法 nil"),
            RedisValue::Bulk(None)
        );
        // 真实嵌套（MULTI/EXEC）只有一两层，必须照常解析。
        let nested = redis_read_value(&mut reader_for(b"*2\r\n$1\r\na\r\n*1\r\n:5\r\n"))
            .expect("合法嵌套数组");
        assert_eq!(
            nested,
            RedisValue::Array(vec![
                RedisValue::Bulk(Some(b"a".to_vec())),
                RedisValue::Array(vec![RedisValue::Int(5)]),
            ])
        );
    }

    #[test]
    fn large_array_from_real_server_still_reads() {
        // 大集合（数千 member）不能被尺寸守卫误伤：元素个数按实际到达增长。
        let mut payload = b"*1000\r\n".to_vec();
        for _ in 0..1000 {
            payload.extend_from_slice(b":7\r\n");
        }
        let value = redis_read_value(&mut reader_for(&payload)).expect("合法大数组");
        let RedisValue::Array(items) = value else {
            panic!("应返回数组，实际 {value:?}");
        };
        assert_eq!(items.len(), 1000);
    }

    #[test]
    fn absurd_bulk_length_is_rejected_without_allocating() {
        is_connection_error(redis_read_value(&mut reader_for(
            b"$9223372036854775807\r\nhello",
        )));
    }

    #[test]
    fn short_bulk_is_rejected_instead_of_blocking_on_preallocated_buffer() {
        is_connection_error(redis_read_value(&mut reader_for(b"$100\r\nabc")));
    }

    #[test]
    fn absurd_array_count_is_rejected_without_preallocating() {
        is_connection_error(redis_read_value(&mut reader_for(
            b"*9223372036854775807\r\n+OK\r\n",
        )));
    }

    #[test]
    fn deep_nesting_is_rejected_before_stack_overflow() {
        let payload = format!("{}+OK\r\n", "*1\r\n".repeat(REDIS_MAX_DEPTH + 50));
        is_connection_error(redis_read_value(&mut reader_for(payload.as_bytes())));
    }

    #[test]
    fn endless_line_is_rejected_at_the_cap() {
        let payload = format!("+{}", "a".repeat(REDIS_MAX_LINE_LEN + 1));
        is_connection_error(redis_read_value(&mut reader_for(payload.as_bytes())));
    }
}
