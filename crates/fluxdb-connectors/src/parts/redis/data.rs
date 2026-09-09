fn redis_load_data(
    config: &ConnectionConfig,
    path: &ObjectPath,
    offset: u64,
    limit: u64,
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataPage> {
    let mut connection = redis_connect(config)?;
    let database = redis_path_database(path)?;
    redis_select(&mut connection, database)?;

    if path.kind == ObjectKind::RedisKey {
        let rows = vec![redis_key_row(&mut connection, path.name.as_str())?];
        return Ok(DataPage {
            columns: redis_key_columns(),
            rows,
            offset,
            limit,
            has_more: false,
        });
    }
    if path.kind != ObjectKind::RedisDb {
        return Err(Error::new(ErrorKind::Unsupported, "Redis 仅支持 DB 和 Key 数据读取"));
    }

    let filter = redis_scan_filter_from(filters);
    let scan_key = RedisScanCursorKey {
        connection_id: path.connection_id,
        database,
        filter: filter.clone(),
    };
    // 顺序翻页时从上一页结束的位置续扫（游标 + 上一页没用完的 key），
    // 避免每页都从头 SCAN（原实现是 O(offset)）。
    // 跳页 / 回翻等续扫点对不上的情况，退化成从头扫到 offset，与原行为一致。
    // cursor 用 Option 表达，避免和 SCAN 的 "0" 混淆：
    // Some(c) = 还能从游标 c 继续扫（c 为 "0" 表示从头开始），None = 已经扫完。
    let (mut cursor, mut pending) = match redis_scan_cursor_take(&scan_key, offset) {
        Some(resume) => (resume.cursor, resume.leftover),
        None => {
            let mut cursor = Some("0".to_string());
            let mut collected = Vec::new();
            if offset > 0 {
                let (skipped, next_cursor) =
                    redis_scan_keys_page(&mut connection, &filter, "0", offset as usize)?;
                // 服务端 key 数少于 offset：这一页本来就没有数据。
                if skipped.len() < offset as usize {
                    return Ok(DataPage {
                        columns: redis_key_columns(),
                        rows: Vec::new(),
                        offset,
                        limit,
                        has_more: false,
                    });
                }
                cursor = (next_cursor != "0").then_some(next_cursor);
                collected = skipped.into_iter().skip(offset as usize).collect();
            }
            (cursor, collected)
        }
    };

    let limit = limit as usize;
    // 多取一条用于判断 has_more，不展示。
    let need = limit.saturating_add(1);
    while pending.len() < need
        && let Some(current) = cursor.clone()
    {
        let (mut keys, next_cursor) =
            redis_scan_keys_page(&mut connection, &filter, &current, need - pending.len())?;
        pending.append(&mut keys);
        cursor = (next_cursor != "0").then_some(next_cursor);
    }
    let has_more = pending.len() > limit || cursor.is_some();
    let leftover = pending.split_off(pending.len().min(limit));
    let keys = pending;
    redis_scan_cursor_store(&scan_key, offset + keys.len() as u64, cursor, leftover);

    // 首屏只返回键名（类型/值/大小/TTL 留空），对齐 RedisInsight「先拿 key 名、再对可见行懒加载
    // metadata」：SCAN 拿键名很轻，但逐个 key 前置拉 TYPE / preview / MEMORY USAGE / TTL 成本很高，
    // 是打开 Redis 库慢的根因。元信息由 `redis_key_metadata` 按可见行批量补齐。
    let rows = keys.into_iter().map(|key| redis_key_name_row(&key)).collect();

    Ok(DataPage {
        columns: redis_key_columns(),
        rows,
        offset,
        limit: limit as u64,
        has_more,
    })
}

/// 键名专用行：只填「键」列，其余（类型/值/大小/TTL）留空。
/// 首屏返回这种行避免为每个 key 前置查询元信息；后续由 `redis_key_metadata` 惰性补齐。
fn redis_key_name_row(key: &str) -> Row {
    Row {
        values: vec![
            CellValue::Text(key.to_string()),
            CellValue::Text(String::new()),
            CellValue::Text(String::new()),
            CellValue::Text(String::new()),
            CellValue::Text(String::new()),
        ],
    }
}

/// 批量补齐一批键的元信息（类型 / 预览 / 大小 / TTL）。
/// 与 `redis_key_rows` 一致压成两轮 pipeline（TYPE + 预览/内存/TTL），供列表惰性补全复用。
fn redis_key_metadata(
    config: &ConnectionConfig,
    path: &ObjectPath,
    keys: &[String],
) -> fluxdb_core::Result<DataPage> {
    let mut connection = redis_connect(config)?;
    let database = redis_path_database(path)?;
    redis_select(&mut connection, database)?;
    let rows = redis_key_rows(&mut connection, keys)?;
    Ok(DataPage {
        columns: redis_key_columns(),
        rows,
        offset: 0,
        limit: keys.len() as u64,
        has_more: false,
    })
}

/// 把数据网格的过滤条件翻译成 SCAN 的服务端过滤：
/// 「键」列 → MATCH（Contains 走 `*x*`，Eq 走精确匹配），「类型」列 Eq → TYPE。
/// 其余列/操作符不下推，交给上层（Redis 不支持按值过滤）。
fn redis_scan_filter_from(filters: &[FilterSpec]) -> RedisScanFilter {
    let mut result = RedisScanFilter::default();
    for filter in filters.iter().filter(|filter| filter.enabled) {
        let Some(value) = filter.values.first().map(CellValue::display_label) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        match (filter.field.as_str(), filter.op) {
            ("键", FilterOp::Contains) => {
                result.pattern = Some(format!("*{}*", redis_glob_escape(&value)));
            }
            ("键", FilterOp::Eq) => {
                result.pattern = Some(redis_glob_escape(&value));
            }
            ("类型", FilterOp::Eq) => {
                result.type_name = Some(value.to_ascii_lowercase());
            }
            _ => {}
        }
    }
    result
}

/// SCAN 续扫游标的缓存键：同一连接 + 同一 DB + 同一过滤条件下的游标才可复用。
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RedisScanCursorKey {
    connection_id: ConnectionId,
    database: u32,
    filter: RedisScanFilter,
}

/// 缓存条数上限：只服务「顺序翻页」这一种场景，不需要留很多。
const REDIS_SCAN_CURSOR_CACHE_MAX: usize = 64;

/// 顺序翻页的续扫点：扫到哪个 offset、下次从哪个游标继续、
/// 以及上一页多扫出来但没展示的 key（SCAN 不能在批次中间截断，多出来的必须留着）。
#[derive(Clone, Debug)]
struct RedisScanResume {
    offset: u64,
    /// Some(游标) 表示还能继续扫，None 表示已经扫到底。
    cursor: Option<String>,
    leftover: Vec<String>,
}

fn redis_scan_cursor_cache() -> &'static Mutex<HashMap<RedisScanCursorKey, RedisScanResume>> {
    static CACHE: OnceLock<Mutex<HashMap<RedisScanCursorKey, RedisScanResume>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取出「正好扫到 offset 处」的续扫点；对不上就返回 None，由调用方从头扫。
fn redis_scan_cursor_take(key: &RedisScanCursorKey, offset: u64) -> Option<RedisScanResume> {
    if offset == 0 {
        return Some(RedisScanResume {
            offset: 0,
            cursor: Some("0".to_string()),
            leftover: Vec::new(),
        });
    }
    let cache = redis_scan_cursor_cache().lock().ok()?;
    let resume = cache.get(key)?;
    (resume.offset == offset).then(|| resume.clone())
}

fn redis_scan_cursor_store(
    key: &RedisScanCursorKey,
    offset: u64,
    cursor: Option<String>,
    leftover: Vec<String>,
) {
    let Ok(mut cache) = redis_scan_cursor_cache().lock() else {
        return;
    };
    if cache.len() >= REDIS_SCAN_CURSOR_CACHE_MAX && !cache.contains_key(key) {
        cache.clear();
    }
    cache.insert(
        key.clone(),
        RedisScanResume {
            offset,
            cursor,
            leftover,
        },
    );
}

fn redis_key_columns() -> Vec<Column> {
    ["键", "类型", "值", "大小", "TTL"]
        .into_iter()
        .map(|name| Column {
            name: name.to_string(),
            type_name: Some("redis".to_string()),
            nullable: true,
            primary_key: name == "键",
            comment: None,
        })
        .collect()
}

enum RedisHashFieldQueryMode {
    All,
    Exact(String),
    Pattern(String),
}

/// 批量取一页 Key 的元信息。
/// 相比逐行 4 次往返（TYPE / 预览 / MEMORY USAGE / TTL），这里压成 2 次往返：
/// 第一轮批量 TYPE，第二轮把每个 key 的预览 + 大小 + TTL 拼成一个 pipeline。
fn redis_key_rows(connection: &mut RedisConnection, keys: &[String]) -> fluxdb_core::Result<Vec<Row>> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let type_commands = keys
        .iter()
        .map(|key| vec!["TYPE".to_string(), key.clone()])
        .collect::<Vec<_>>();
    let raw_types = connection
        .command_pipeline(&type_commands)?
        .into_iter()
        .map(redis_value_text)
        .collect::<Vec<_>>();
    if raw_types.len() != keys.len() {
        return Err(Error::new(ErrorKind::Query, "Redis TYPE 返回数量不匹配"));
    }

    // 逐个 key 拼命令，并记下每个 key 占了多少条响应，便于按边界切回去。
    let mut commands = Vec::new();
    let mut spans = Vec::new();
    for (key, raw_type) in keys.iter().zip(raw_types.iter()) {
        let preview = redis_key_preview_commands(key, raw_type);
        let preview_len = preview.len();
        commands.extend(preview);
        commands.push(vec!["MEMORY".to_string(), "USAGE".to_string(), key.clone()]);
        commands.push(vec!["TTL".to_string(), key.clone()]);
        spans.push(preview_len);
    }
    let mut responses = connection.command_pipeline(&commands)?.into_iter();

    let mut rows = Vec::with_capacity(keys.len());
    for ((key, raw_type), preview_len) in keys.iter().zip(raw_types.iter()).zip(spans) {
        let preview_values = (0..preview_len)
            .map(|_| responses.next())
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Error::new(ErrorKind::Query, "Redis 批量查询返回数量不匹配"))?;
        let memory = responses.next();
        let ttl = responses
            .next()
            .ok_or_else(|| Error::new(ErrorKind::Query, "Redis 批量查询返回数量不匹配"))?;
        let (type_name, value) = redis_key_preview_from(raw_type, preview_values);
        rows.push(redis_key_row_from(key, raw_type, type_name, value, memory, ttl));
    }
    Ok(rows)
}

fn redis_key_row(connection: &mut RedisConnection, key: &str) -> fluxdb_core::Result<Row> {
    redis_key_rows(connection, std::slice::from_ref(&key.to_string()))?
        .pop()
        .ok_or_else(|| Error::new(ErrorKind::Query, "Redis Key 元信息查询没有返回结果"))
}

/// 由已经取回的响应拼出 Key 列表的一行。
fn redis_key_row_from(
    key: &str,
    raw_type: &str,
    type_name: String,
    value: String,
    memory: Option<RedisValue>,
    ttl: RedisValue,
) -> Row {
    let size = match memory {
        // 字节数换算成可读大小（KB/MB/GB），列表「大小」列与详情标题栏共用。
        Some(RedisValue::Int(bytes)) => fluxdb_core::format_byte_length(bytes.max(0) as u64),
        _ => String::new(),
    };
    let ttl = match ttl {
        RedisValue::Int(-2) => "已过期".to_string(),
        RedisValue::Int(-1) => "(No TTL)".to_string(),
        RedisValue::Int(seconds) => format!("{seconds}s"),
        _ => String::new(),
    };
    // 流预览只拉取最多 5 条 Entry，且详情页会按块重新解析出 Entry ID，
    // 不能再做 200 字符通用截断——截断若落在行中间，会得到无效的 Entry ID。
    let display_value = if raw_type.eq_ignore_ascii_case("stream") {
        value
    } else {
        redis_preview(value)
    };
    Row {
        values: vec![
            CellValue::Text(key.to_string()),
            CellValue::Text(type_name),
            CellValue::Text(display_value),
            CellValue::Text(size),
            CellValue::Text(ttl),
        ],
    }
}

/// Key 列表的服务端过滤条件：MATCH 模式 + TYPE 限定。
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
struct RedisScanFilter {
    /// SCAN MATCH 的 glob 模式；None 表示不过滤。
    pattern: Option<String>,
    /// SCAN TYPE 的类型名（string/hash/list/set/zset/stream）；None 表示不过滤。
    type_name: Option<String>,
}

/// 扫描 Key：从 `cursor` 开始反复 SCAN，直到攒够至少 `need` 个或遍历完。
/// 返回 (收集到的 keys, 下次游标)；下次游标为 "0" 表示已经到底。
///
/// 两个约束：
/// 1. SCAN 的 COUNT 只是提示，带 MATCH/TYPE 时单轮可能一个都不返回，所以必须循环；
/// 2. **绝不在一批结果中间截断**——截断掉的那部分 key 无法用游标再取回来，会整段丢失。
///    因此返回的数量可能略多于 `need`，多出来的由调用方缓存成「下一页的开头」。
fn redis_scan_keys_page(
    connection: &mut RedisConnection,
    filter: &RedisScanFilter,
    cursor: &str,
    need: usize,
) -> fluxdb_core::Result<(Vec<String>, String)> {
    let mut cursor = if cursor.is_empty() {
        "0".to_string()
    } else {
        cursor.to_string()
    };
    let mut keys = Vec::new();
    if need == 0 {
        return Ok((keys, cursor));
    }
    loop {
        let mut args = vec!["SCAN".to_string(), cursor.clone()];
        if let Some(pattern) = filter.pattern.as_deref() {
            args.push("MATCH".to_string());
            args.push(pattern.to_string());
        }
        args.push("COUNT".to_string());
        args.push(REDIS_SCAN_COUNT.to_string());
        if let Some(type_name) = filter.type_name.as_deref() {
            args.push("TYPE".to_string());
            args.push(type_name.to_string());
        }
        let args_ref = args.iter().map(String::as_str).collect::<Vec<_>>();
        let value = connection.command(&args_ref)?;
        let RedisValue::Array(items) = value else {
            return Err(Error::new(ErrorKind::Query, "Redis SCAN 返回格式异常"));
        };
        let [next_cursor, key_values] = items.as_slice() else {
            return Err(Error::new(ErrorKind::Query, "Redis SCAN 返回格式异常"));
        };
        cursor = redis_value_text(next_cursor.clone());
        if let RedisValue::Array(items) = key_values {
            for key in items {
                keys.push(redis_value_text(key.clone()));
            }
        }
        if keys.len() >= need || cursor == "0" {
            return Ok((keys, cursor));
        }
    }
}

fn redis_scan_keys(
    connection: &mut RedisConnection,
    max_keys: usize,
) -> fluxdb_core::Result<Vec<String>> {
    let (keys, _) = redis_scan_keys_page(connection, &RedisScanFilter::default(), "0", max_keys)?;
    Ok(keys)
}
