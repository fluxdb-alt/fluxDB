/// Redis Stream 单条条目：流 ID + 由 ID 推导的本地时间 + 字段列表（保留服务端顺序）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisStreamEntryRow {
    pub id: String,
    /// 由流 ID 的毫秒时间戳格式化得到；ID 非法时为空串。
    pub time: String,
    pub fields: Vec<(String, String)>,
}

/// Stream 条目的时间范围过滤（毫秒时间戳，两端都可选、都含端点）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RedisStreamRange {
    /// 只看这个时刻之后（含）的条目。
    pub since_ms: Option<u64>,
    /// 只看这个时刻之前（含）的条目。
    pub until_ms: Option<u64>,
}

/// Stream 消费者组概览（只读）：XINFO GROUPS 的一行 + 该组的消费者明细。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RedisStreamGroup {
    pub name: String,
    /// 组内消费者数量。
    pub consumers: u64,
    /// 已投递未确认的条目数（XPENDING 总量）。
    pub pending: u64,
    /// 该组已消费到的最后一个 Entry ID。
    pub last_delivered_id: String,
    /// 组内各消费者：(名称, 未确认条目数, 空闲毫秒数)。
    pub consumer_detail: Vec<(String, u64, u64)>,
}

/// Redis Stream 条目分页查询结果：一页条目（从新到旧）+ 下次游标 + 条目总数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisStreamEntryPage {
    pub entries: Vec<RedisStreamEntryRow>,
    /// 下一页 XREVRANGE 的起始 ID（含）；为 "0" 表示已翻到最旧一条。
    pub next_cursor: String,
    /// XLEN 得到的条目总数。
    pub total: usize,
}

fn redis_xadd_stream_entry(
    config: &ConnectionConfig,
    object: &ObjectPath,
    id: &str,
    fields: &[(String, String)],
    maxlen: Option<u64>,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Stream 操作缺少 Key"));
    }
    if fields.is_empty() {
        return Err(Error::new(ErrorKind::Query, "Stream Entry 至少需要一个字段"));
    }
    let id = id.trim();
    let id = if id.is_empty() { "*" } else { id };
    let mut args = vec!["XADD".to_string(), object.name.clone()];
    if let Some(maxlen) = maxlen {
        if maxlen == 0 {
            return Err(Error::new(ErrorKind::Query, "Stream MAXLEN 必须大于 0"));
        }
        args.push("MAXLEN".to_string());
        args.push("~".to_string());
        args.push(maxlen.to_string());
    }
    args.push(id.to_string());
    for (field, value) in fields {
        let field = field.trim();
        if field.is_empty() {
            return Err(Error::new(ErrorKind::Query, "Stream 字段名不能为空"));
        }
        args.push(field.to_string());
        args.push(value.clone());
        redis_reject_binary_placeholder(value)?;
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    match connection.command(&args)? {
        RedisValue::Bulk(Some(_)) | RedisValue::Simple(_) => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis XADD 返回格式异常")),
    }
}

fn redis_xdel_stream_entry(
    config: &ConnectionConfig,
    object: &ObjectPath,
    entry_id: &str,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Stream 操作缺少 Key"));
    }
    if entry_id.trim().is_empty() {
        return Err(Error::new(ErrorKind::Query, "Stream Entry ID 不能为空"));
    }
    // 删除前先校验 ID 格式，避免把解析异常得到的垃圾文本发给 Redis，
    // 否则 Redis 会返回 "Invalid stream ID specified as stream command argument"。
    if !redis_stream_id_format_valid(entry_id) {
        return Err(Error::new(ErrorKind::Query, "Stream Entry ID 格式异常，无法删除"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    match connection.command(&["XDEL", object.name.as_str(), entry_id.trim()])? {
        RedisValue::Int(_) => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis XDEL 返回格式异常")),
    }
}

/// 单次 XREVRANGE 分页查询 Stream 条目（对齐 Set/Hash/ZSet/List：一次请求一页，游标回传给客户端）。
/// 游标为空取最新一页；否则从上一页多取出的那条 ID（含）继续向更旧方向翻页，
/// 用「多取一条」判断是否还有下一页，避免依赖 Redis 6.2 才支持的排他区间语法。
///
/// `range` 给出可选的时间范围（毫秒时间戳），直接下推成 XREVRANGE 的 start/end，
/// 由服务端过滤，不需要把范围外的条目拉回来。
fn redis_load_stream_entries(
    config: &ConnectionConfig,
    object: &ObjectPath,
    range: RedisStreamRange,
    cursor: &str,
    limit: usize,
) -> fluxdb_core::Result<RedisStreamEntryPage> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Stream 查询缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let total = match connection.command(&["XLEN", object.name.as_str()])? {
        RedisValue::Int(total) => total.max(0) as usize,
        _ => return Err(Error::new(ErrorKind::Query, "Redis XLEN 返回格式异常")),
    };
    if limit == 0 {
        return Ok(RedisStreamEntryPage {
            entries: Vec::new(),
            next_cursor: "0".to_string(),
            total,
        });
    }
    // XREVRANGE 是从新到旧，所以 start 取范围上界、end 取范围下界。
    // 时间戳按 Redis 流 ID 规则补齐序列号：上界补 -18446744073709551615 会超范围，
    // 直接用毫秒时间戳本身（等价于该毫秒的 0 号序列），上界则用 `ms` 让同毫秒的条目都落在范围内。
    let upper = match (cursor.is_empty(), range.until_ms) {
        (false, _) => cursor.to_string(),
        (true, Some(until)) => until.to_string(),
        (true, None) => "+".to_string(),
    };
    let lower = match range.since_ms {
        Some(since) => format!("{since}-0"),
        None => "-".to_string(),
    };
    let count = limit.saturating_add(1).to_string();
    let args = [
        "XREVRANGE",
        object.name.as_str(),
        upper.as_str(),
        lower.as_str(),
        "COUNT",
        count.as_str(),
    ];
    let mut entries = redis_parse_stream_entries(connection.command(&args)?)?;
    let next_cursor = if entries.len() > limit {
        // 第 limit+1 条只用于产生下一页游标，不展示。
        let next = entries[limit].id.clone();
        entries.truncate(limit);
        next
    } else {
        "0".to_string()
    };
    Ok(RedisStreamEntryPage {
        entries,
        next_cursor,
        total,
    })
}

/// 读取消费者组：XINFO GROUPS 拿组列表，再对每个组 XINFO CONSUMERS 拿消费者明细。
/// 老版本服务端没有这两个子命令时按「没有消费者组」处理，而不是报错。
fn redis_load_stream_groups(
    config: &ConnectionConfig,
    object: &ObjectPath,
) -> fluxdb_core::Result<Vec<RedisStreamGroup>> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Stream 查询缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let groups = match connection.command(&["XINFO", "GROUPS", object.name.as_str()]) {
        Ok(RedisValue::Array(items)) => items,
        Ok(_) => return Err(Error::new(ErrorKind::Query, "Redis XINFO GROUPS 返回格式异常")),
        Err(error) if redis_hash_ttl_unsupported(&error) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut result = Vec::with_capacity(groups.len());
    for group in groups {
        let fields = redis_info_map(group);
        let name = fields.get("name").cloned().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let consumers = redis_info_u64(&fields, "consumers");
        let pending = redis_info_u64(&fields, "pending");
        let last_delivered_id = fields.get("last-delivered-id").cloned().unwrap_or_default();
        let consumer_detail = match connection.command(&[
            "XINFO",
            "CONSUMERS",
            object.name.as_str(),
            name.as_str(),
        ]) {
            Ok(RedisValue::Array(items)) => items
                .into_iter()
                .map(|consumer| {
                    let fields = redis_info_map(consumer);
                    (
                        fields.get("name").cloned().unwrap_or_default(),
                        redis_info_u64(&fields, "pending"),
                        redis_info_u64(&fields, "idle"),
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
        result.push(RedisStreamGroup {
            name,
            consumers,
            pending,
            last_delivered_id,
            consumer_detail,
        });
    }
    Ok(result)
}

/// 把 XINFO 的扁平 [name, value, name, value, ...] 回包转成键值表。
fn redis_info_map(value: RedisValue) -> BTreeMap<String, String> {
    let RedisValue::Array(items) = value else {
        return BTreeMap::new();
    };
    items
        .chunks(2)
        .filter_map(|pair| match pair {
            [name, value] => Some((
                redis_value_text(name.clone()),
                redis_value_text(value.clone()),
            )),
            _ => None,
        })
        .collect()
}

fn redis_info_u64(fields: &BTreeMap<String, String>, key: &str) -> u64 {
    fields
        .get(key)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default()
}

/// 解析 XREVRANGE/XRANGE 返回的 [[id, [field, value, ...]], ...] 结构。
fn redis_parse_stream_entries(value: RedisValue) -> fluxdb_core::Result<Vec<RedisStreamEntryRow>> {
    let RedisValue::Array(items) = value else {
        return Err(Error::new(ErrorKind::Query, "Redis XREVRANGE 返回格式异常"));
    };
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        let RedisValue::Array(mut entry) = item else {
            return Err(Error::new(ErrorKind::Query, "Redis XREVRANGE 返回格式异常"));
        };
        if entry.len() < 2 {
            return Err(Error::new(ErrorKind::Query, "Redis XREVRANGE 返回格式异常"));
        }
        let fields = entry.remove(1);
        let id = redis_value_text(entry.remove(0));
        entries.push(RedisStreamEntryRow {
            time: redis_stream_id_time(&id).unwrap_or_default(),
            id,
            fields: redis_stream_entry_fields(fields),
        });
    }
    Ok(entries)
}

/// 把扁平的 [field, value, field, value, ...] 数组还原为字段对；落单的字段名按空值处理。
fn redis_stream_entry_fields(value: RedisValue) -> Vec<(String, String)> {
    let RedisValue::Array(items) = value else {
        return Vec::new();
    };
    items
        .chunks(2)
        .map(|pair| match pair {
            [field, value] => (
                redis_value_text(field.clone()),
                redis_value_text(value.clone()),
            ),
            [field] => (redis_value_text(field.clone()), String::new()),
            _ => (String::new(), String::new()),
        })
        .collect()
}

fn redis_stream_preview(length: Option<RedisValue>, entries: Option<RedisValue>) -> String {
    let length = match length {
        Some(RedisValue::Int(length)) => length,
        _ => 0,
    };
    let entries = redis_stream_entries_text(entries);
    if entries.is_empty() {
        format!("{length} 条目")
    } else {
        format!("{length} 条目\n{entries}")
    }
}

fn redis_stream_entries_text(value: Option<RedisValue>) -> String {
    let Some(RedisValue::Array(entries)) = value else {
        return String::new();
    };
    entries
        .into_iter()
        .filter_map(redis_stream_entry_text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn redis_stream_entry_text(value: RedisValue) -> Option<String> {
    let RedisValue::Array(mut entry) = value else {
        return None;
    };
    if entry.len() < 2 {
        return None;
    }
    let fields = entry.pop()?;
    let id = redis_value_text(entry.remove(0));
    let time = redis_stream_id_time(&id).unwrap_or_default();
    let mut lines = vec![if time.is_empty() {
        id
    } else {
        format!("{time}  {id}")
    }];
    lines.extend(redis_stream_field_lines(fields));
    Some(lines.join("\n"))
}

/// 转义流字段名/值中的换行，避免多行内容破坏预览的块分隔符（\n\n）。
/// 否则前端会把多行值误拆成多个 Entry，产生无效的流 ID，删除时报 "Invalid stream ID"。
fn redis_stream_escape_text(text: String) -> String {
    text.replace('\r', "\\r").replace('\n', "\\n")
}

fn redis_stream_field_lines(value: RedisValue) -> Vec<String> {
    let RedisValue::Array(items) = value else {
        return Vec::new();
    };
    items
        .chunks(2)
        .map(|pair| match pair {
            [field, value] => format!(
                "  {}: {}",
                redis_stream_escape_text(redis_value_text(field.clone())),
                redis_stream_escape_text(redis_value_text(value.clone()))
            ),
            [field] => format!(
                "  {}",
                redis_stream_escape_text(redis_value_text(field.clone()))
            ),
            _ => String::new(),
        })
        .collect()
}

/// 校验流 ID 是否为 Redis 接受的 `timestamp-sequence`（两部分均为 u64 十进制数字）。
fn redis_stream_id_format_valid(id: &str) -> bool {
    let Some((timestamp, sequence)) = id.trim().split_once('-') else {
        return false;
    };
    !timestamp.is_empty()
        && !sequence.is_empty()
        && timestamp.parse::<u64>().is_ok()
        && sequence.parse::<u64>().is_ok()
}

fn redis_stream_id_time(id: &str) -> Option<String> {
    let millis = id.split('-').next()?.parse::<i64>().ok()?;
    sqlx::types::chrono::DateTime::from_timestamp_millis(millis)
        .map(|time| time.with_timezone(&sqlx::types::chrono::Local))
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
}
