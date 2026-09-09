/// Redis Hash 字段分页查询结果：一页 field/value/ttl + 下次游标 + 总数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisHashFieldPage {
    pub fields: Vec<(String, String, String)>,
    pub next_cursor: String,
    pub total: usize,
}

fn redis_hscan_hash_fields(
    config: &ConnectionConfig,
    object: &ObjectPath,
    query: &str,
    cursor: &str,
    limit: usize,
) -> fluxdb_core::Result<RedisHashFieldPage> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Hash 查询缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let total = match connection.command(&["HLEN", object.name.as_str()])? {
        RedisValue::Int(n) => n.max(0) as usize,
        _ => return Err(Error::new(ErrorKind::Query, "Redis HLEN 返回格式异常")),
    };
    if limit == 0 {
        return Ok(RedisHashFieldPage {
            fields: Vec::new(),
            next_cursor: "0".to_string(),
            total,
        });
    }
    // 对齐 RedisInsight：空串拉全量，普通输入做精确字段查找，显式 glob 才走 HSCAN MATCH。
    match redis_hash_field_query_mode(query) {
        RedisHashFieldQueryMode::All => redis_scan_hash_fields(
            &mut connection,
            object.name.as_str(),
            None,
            cursor,
            limit,
            total,
        ),
        RedisHashFieldQueryMode::Exact(field) => {
            redis_load_exact_hash_field(&mut connection, object.name.as_str(), &field, total)
        }
        RedisHashFieldQueryMode::Pattern(pattern) => redis_scan_hash_fields(
            &mut connection,
            object.name.as_str(),
            Some(&pattern),
            cursor,
            limit,
            total,
        ),
    }
}

fn redis_hash_field_query_mode(query: &str) -> RedisHashFieldQueryMode {
    let query = query.trim();
    if query.is_empty() {
        RedisHashFieldQueryMode::All
    } else if redis_query_looks_like_glob(query) {
        RedisHashFieldQueryMode::Pattern(query.to_string())
    } else {
        RedisHashFieldQueryMode::Exact(query.to_string())
    }
}

fn redis_scan_hash_fields(
    connection: &mut RedisConnection,
    key: &str,
    pattern: Option<&str>,
    cursor: &str,
    limit: usize,
    total: usize,
) -> fluxdb_core::Result<RedisHashFieldPage> {
    let mut next_cursor = if cursor.trim().is_empty() {
        "0".to_string()
    } else {
        cursor.trim().to_string()
    };
    let mut fields = Vec::new();
    loop {
        if fields.len() >= limit {
            break;
        }
        let remaining = limit - fields.len();
        let mut args = vec![
            "HSCAN".to_string(),
            key.to_string(),
            next_cursor.clone(),
            "COUNT".to_string(),
            REDIS_SCAN_COUNT.to_string(),
        ];
        if let Some(pattern) = pattern {
            args.push("MATCH".to_string());
            args.push(pattern.to_string());
        }
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        let (cursor, batch) = redis_parse_scan_pairs(connection.command(&args)?, remaining)?;
        next_cursor = cursor;
        fields.extend(batch);
        if next_cursor == "0" {
            break;
        }
    }
    let field_names = fields
        .iter()
        .map(|(field, _)| field.clone())
        .collect::<Vec<_>>();
    let ttls = redis_hpttl_hash_fields(connection, key, &field_names)?;
    // 截断必须在 field_names 提取之后：HPTTL 需要用完整字段名取 TTL。
    let fields = fields
        .into_iter()
        .zip(ttls)
        .map(|((field, value), ttl)| (field, redis_hash_truncate(value), ttl))
        .collect::<Vec<_>>();
    Ok(RedisHashFieldPage {
        fields,
        next_cursor,
        total,
    })
}

fn redis_load_exact_hash_field(
    connection: &mut RedisConnection,
    key: &str,
    field: &str,
    total: usize,
) -> fluxdb_core::Result<RedisHashFieldPage> {
    let mut fields = Vec::new();
    if let Some(value) = redis_hash_field_value_text(connection.command(&["HGET", key, field])?) {
        // 精确查找路径同样截断大值，避免大字段绕开截断（搜索命中时）。
        fields.push((field.to_string(), redis_hash_truncate(value)));
    }
    let field_names = fields
        .iter()
        .map(|(field, _)| field.clone())
        .collect::<Vec<_>>();
    let ttls = redis_hpttl_hash_fields(connection, key, &field_names)?;
    let fields = fields
        .into_iter()
        .zip(ttls)
        .map(|((field, value), ttl)| (field, value, ttl))
        .collect::<Vec<_>>();
    Ok(RedisHashFieldPage {
        fields,
        next_cursor: "0".to_string(),
        total,
    })
}

/// 完整值弹框专用：HGET 单字段返回原始值，不经过截断（与 `redis_load_exact_hash_field` 相反）。
/// 字段不存在返回 None。供 UI 查看/编辑 >1MB 被截断的完整值。
fn redis_load_exact_hash_field_full(
    connection: &mut RedisConnection,
    key: &str,
    field: &str,
) -> fluxdb_core::Result<Option<String>> {
    Ok(redis_hash_field_value_text(
        connection.command(&["HGET", key, field])?,
    ))
}

fn redis_hash_field_value_text(value: RedisValue) -> Option<String> {
    match value {
        RedisValue::Simple(value) => Some(value),
        RedisValue::Int(value) => Some(value.to_string()),
        RedisValue::Bulk(Some(value)) => Some(redis_text_from_bytes(value)),
        RedisValue::Bulk(None) => None,
        RedisValue::Array(_) => None,
    }
}

fn redis_hpttl_hash_fields(
    connection: &mut RedisConnection,
    key: &str,
    fields: &[String],
) -> fluxdb_core::Result<Vec<String>> {
    if fields.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec![
        "HPTTL".to_string(),
        key.to_string(),
        "FIELDS".to_string(),
        fields.len().to_string(),
    ];
    args.extend(fields.iter().cloned());
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    match connection.command(&args) {
        Ok(RedisValue::Array(items)) => {
            if items.len() != fields.len() {
                return Err(Error::new(ErrorKind::Query, "Redis HPTTL 返回格式异常"));
            }
            items
                .into_iter()
                .map(redis_hash_field_ttl_text)
                .collect::<fluxdb_core::Result<Vec<_>>>()
        }
        Ok(_) => Err(Error::new(ErrorKind::Query, "Redis HPTTL 返回格式异常")),
        Err(error) if redis_hash_ttl_unsupported(&error) => {
            Ok(vec!["无 TTL".to_string(); fields.len()])
        }
        Err(error) => Err(error),
    }
}

fn redis_hash_field_ttl_text(value: RedisValue) -> fluxdb_core::Result<String> {
    let ttl = match value {
        RedisValue::Int(ttl) => ttl,
        other => redis_value_text(other)
            .parse::<i64>()
            .map_err(|_| Error::new(ErrorKind::Query, "Redis HPTTL 返回格式异常"))?,
    };
    Ok(match ttl {
        -2 => "已过期".to_string(),
        -1 => "无 TTL".to_string(),
        ttl => {
            // Hash 字段 TTL 只展示到秒，和 key TTL 保持一致。
            format!("{}s", (ttl / 1000).max(1))
        }
    })
}

fn redis_hash_ttl_unsupported(error: &Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("unknown command")
        || message.contains("unknown subcommand")
        || message.contains("wrong number of arguments")
}

fn redis_hset_hash_field(
    config: &ConnectionConfig,
    object: &ObjectPath,
    field: &str,
    value: &str,
    ttl: RedisHashFieldTtl,
    allow_truncated: bool,
) -> fluxdb_core::Result<()> {
    redis_reject_binary_placeholder(field)?;
    redis_reject_binary_placeholder(value)?;
    // 行内编辑（allow_truncated=false）拒绝截断标记回写，防止片段覆盖完整数据；
    // 完整值弹框保存（allow_truncated=true）携带的是完整原始值，放行。
    if !allow_truncated {
        redis_reject_truncated_value(value)?;
    }
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Hash 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    // Redis 7.4 起 HSET 会清掉字段原有 TTL，Keep 语义必须在写入前读出来、写入后补回。
    let kept_ttl_ms = match ttl {
        RedisHashFieldTtl::Keep => {
            redis_hpttl_hash_field_ms(&mut connection, object.name.as_str(), field)?
        }
        _ => None,
    };
    match connection.command(&["HSET", object.name.as_str(), field, value])? {
        RedisValue::Int(_) => {}
        _ => return Err(Error::new(ErrorKind::Query, "Redis HSET 返回格式异常")),
    }
    match ttl {
        RedisHashFieldTtl::Keep => {
            if let Some(ttl_ms) = kept_ttl_ms {
                redis_hpexpire_hash_field(&mut connection, object.name.as_str(), field, ttl_ms)?;
            }
        }
        // HSET 已经把 TTL 清掉了，这里再显式 HPERSIST 一次，兼容不清 TTL 的旧版本服务端。
        RedisHashFieldTtl::Persist => {
            redis_hpersist_hash_field(&mut connection, object.name.as_str(), field)?;
        }
        RedisHashFieldTtl::Seconds(seconds) => {
            let ttl_ms = redis_hash_field_ttl_ms_from_seconds(seconds)?;
            redis_hpexpire_hash_field(&mut connection, object.name.as_str(), field, ttl_ms)?;
        }
    }
    Ok(())
}

/// 仅更新 Hash 字段 TTL，不写 value（纯 TTL 命令，Redis 7.4+ 字段级 TTL）。
/// `Keep` 表示 TTL 未变，直接 no-op（不写值则 TTL 本就保持）；实际 UI 不会提交 Keep。
fn redis_hset_hash_field_ttl(
    config: &ConnectionConfig,
    object: &ObjectPath,
    field: &str,
    ttl: RedisHashFieldTtl,
) -> fluxdb_core::Result<()> {
    redis_reject_binary_placeholder(field)?;
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Hash 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    match ttl {
        RedisHashFieldTtl::Keep => Ok(()),
        RedisHashFieldTtl::Persist => {
            redis_hpersist_hash_field(&mut connection, object.name.as_str(), field)
        }
        RedisHashFieldTtl::Seconds(seconds) => {
            let ttl_ms = redis_hash_field_ttl_ms_from_seconds(seconds)?;
            redis_hpexpire_hash_field(&mut connection, object.name.as_str(), field, ttl_ms)
        }
    }
}

/// 清除字段 TTL。服务端不支持字段级 TTL（7.4 以下）时按无操作处理。
fn redis_hpersist_hash_field(
    connection: &mut RedisConnection,
    key: &str,
    field: &str,
) -> fluxdb_core::Result<()> {
    match connection.command(&["HPERSIST", key, "FIELDS", "1", field]) {
        // 1=已清除，-1=本来就没有 TTL，都算成功；-2=字段不存在。
        Ok(RedisValue::Array(items)) => match items.as_slice() {
            [RedisValue::Int(1)] | [RedisValue::Int(-1)] => Ok(()),
            [RedisValue::Int(-2)] => Err(Error::new(ErrorKind::Query, "Redis 字段不存在")),
            _ => Err(Error::new(ErrorKind::Query, "Redis HPERSIST 返回格式异常")),
        },
        Ok(_) => Err(Error::new(ErrorKind::Query, "Redis HPERSIST 返回格式异常")),
        Err(error) if redis_hash_ttl_unsupported(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn redis_rename_hash_field(
    config: &ConnectionConfig,
    object: &ObjectPath,
    old_field: &str,
    new_field: &str,
    value: &str,
) -> fluxdb_core::Result<()> {
    redis_reject_binary_placeholder(new_field)?;
    redis_reject_binary_placeholder(value)?;
    redis_reject_truncated_value(value)?;
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Hash 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let ttl_ms = redis_hpttl_hash_field_ms(&mut connection, object.name.as_str(), old_field)?;
    match connection.command(&["HSET", object.name.as_str(), new_field, value])? {
        RedisValue::Int(_) => {}
        _ => return Err(Error::new(ErrorKind::Query, "Redis HSET 返回格式异常")),
    }
    match connection.command(&["HDEL", object.name.as_str(), old_field])? {
        RedisValue::Int(_) => {
            if let Some(ttl_ms) = ttl_ms {
                redis_hpexpire_hash_field(&mut connection, object.name.as_str(), new_field, ttl_ms)?;
            }
            Ok(())
        }
        _ => Err(Error::new(ErrorKind::Query, "Redis HDEL 返回格式异常")),
    }
}

fn redis_hash_field_ttl_ms_from_seconds(seconds: u64) -> fluxdb_core::Result<u64> {
    if seconds == 0 {
        return Err(Error::new(ErrorKind::Query, "Redis TTL 必须大于 0 秒或留空"));
    }
    seconds
        .checked_mul(1000)
        .ok_or_else(|| Error::new(ErrorKind::Query, "Redis TTL 超出可设置范围"))
}

fn redis_hpexpire_hash_field(
    connection: &mut RedisConnection,
    key: &str,
    field: &str,
    ttl_ms: u64,
) -> fluxdb_core::Result<()> {
    match connection.command(&[
        "HPEXPIRE",
        key,
        &ttl_ms.to_string(),
        "FIELDS",
        "1",
        field,
    ])? {
        RedisValue::Array(items) => match items.as_slice() {
            [RedisValue::Int(1)] => Ok(()),
            [RedisValue::Int(0)] => Err(Error::new(ErrorKind::Query, "Redis 字段 TTL 设置失败")),
            [RedisValue::Int(-2)] => Err(Error::new(ErrorKind::Query, "Redis 字段不存在")),
            _ => Err(Error::new(ErrorKind::Query, "Redis HPEXPIRE 返回格式异常")),
        },
        _ => Err(Error::new(ErrorKind::Query, "Redis HPEXPIRE 返回格式异常")),
    }
}

fn redis_hpttl_hash_field_ms(
    connection: &mut RedisConnection,
    key: &str,
    field: &str,
) -> fluxdb_core::Result<Option<u64>> {
    let args = ["HPTTL", key, "FIELDS", "1", field];
    match connection.command(&args) {
        Ok(RedisValue::Array(items)) => match items.as_slice() {
            [RedisValue::Int(ttl)] if *ttl > 0 => Ok(Some(*ttl as u64)),
            [RedisValue::Int(-1)] | [RedisValue::Int(-2)] => Ok(None),
            [RedisValue::Int(_)] => Ok(None),
            _ => Err(Error::new(ErrorKind::Query, "Redis HPTTL 返回格式异常")),
        },
        Ok(_) => Err(Error::new(ErrorKind::Query, "Redis HPTTL 返回格式异常")),
        Err(error) if redis_hash_ttl_unsupported(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn redis_hdel_hash_field(
    config: &ConnectionConfig,
    object: &ObjectPath,
    field: &str,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Hash 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    match connection.command(&["HDEL", object.name.as_str(), field])? {
        RedisValue::Int(_) => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis HDEL 返回格式异常")),
    }
}

/// Hash 字段值超过该字节数就截断（对齐 RedisInsight 的 1MB 阈值，默认开启）。
/// 只在 hash 读取层生效，不影响 list / zset / string。
const REDIS_HASH_VALUE_TRUNCATE_LIMIT_BYTES: usize = 1024 * 1024;

/// 截断标记前缀，与 RedisInsight 文案一致：用户读到它就知道该值被截断过、不可编辑。
const REDIS_HASH_TRUNCATED_MARKER: &str = "[Truncated due to length]";

/// 截断后可保留的前导字符数（按字符计，避免切坏多字节 UTF-8）。
const REDIS_HASH_TRUNCATED_CHARS: usize = 30;

/// 把超限的 Hash 字段值替换成「截断标记 + 前 30 字符 + ...」。
/// 未超限时原样返回（常见路径，零分配）；截断串本身 < 1MB，回灌本函数是幂等的。
fn redis_hash_truncate(value: String) -> String {
    if value.len() <= REDIS_HASH_VALUE_TRUNCATE_LIMIT_BYTES {
        return value;
    }
    let head = value.chars().take(REDIS_HASH_TRUNCATED_CHARS).collect::<String>();
    format!("{REDIS_HASH_TRUNCATED_MARKER} {head}...")
}

/// 判断一段文本是否带截断标记前缀。
fn redis_is_hash_truncated(value: &str) -> bool {
    value.starts_with(REDIS_HASH_TRUNCATED_MARKER)
}

/// 写入前的保护：被截断的值只是原值片段，回写它等于用片段覆盖完整数据。
fn redis_reject_truncated_value(value: &str) -> fluxdb_core::Result<()> {
    if redis_is_hash_truncated(value) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "该值因超过 1MB 被截断，无法在表格中编辑；请改用支持大值的客户端写入",
        ));
    }
    Ok(())
}
