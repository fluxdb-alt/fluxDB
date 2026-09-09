/// Redis List 元素分页查询结果：一页 (绝对索引, 值) + 下次游标 + 总数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisListItemPage {
    pub items: Vec<(usize, String)>,
    pub next_cursor: String,
    pub total: usize,
}

/// List 分页 / 按索引跳转查询。
///
/// `query` 为空时按 LRANGE 分页（游标为下一个绝对索引，照常透传）；
/// `query` 非空时语义为「按索引跳转」（对齐 RedisInsight）：解析成下标后用
/// LINDEX 精确读取单个元素，未命中返回空页。List 无服务端扫描命令，故不做内容过滤。
fn redis_load_list_items(
    config: &ConnectionConfig,
    object: &ObjectPath,
    query: &str,
    cursor: &str,
    limit: usize,
) -> fluxdb_core::Result<RedisListItemPage> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis List 查询缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    // LLEN 是首个命令：若键服务端类型已不是 List（被删除后重建为其他类型、或类型探测结果过期），
    // 会返回 WRONGTYPE。这里把它转成清晰的中文提示，避免把原始 Redis 错误直接抛给用户。
    let total = match connection.command(&["LLEN", object.name.as_str()]) {
        Ok(RedisValue::Int(n)) => n.max(0) as usize,
        Ok(_) => return Err(Error::new(ErrorKind::Query, "Redis LLEN 返回格式异常")),
        Err(error) if redis_is_wrongtype(&error) => {
            // 服务端确认该 key 不是 List。附带 database 与 key 便于核对是否串到了别的库/键。
            return Err(Error::new(
                ErrorKind::Query,
                format!(
                    "键「{}」在 DB {} 当前类型不是 List，可能已被删除/重建为其他类型或已串库（原始: {}）",
                    object.name,
                    database,
                    error.message.trim()
                ),
            ))
        }
        Err(error) => return Err(error),
    };
    let make_page = |items: Vec<(usize, String)>| RedisListItemPage {
        items,
        next_cursor: "0".to_string(),
        total,
    };
    // 搜索语义对齐 RedisInsight：List 没有原生扫描命令，搜索按「下标跳转」（LINDEX）
    // 精确读取单个元素，不做内容包含过滤；清空搜索词则退回普通分页（LRANGE）。
    let query = query.trim();
    if !query.is_empty() {
        let index = match query.parse::<usize>() {
            Ok(index) => index,
            // 输入无法解析成合法下标时不命中任何元素。
            Err(_) => return Ok(make_page(Vec::new())),
        };
        let value = match redis_list_item_at(&mut connection, object.name.as_str(), index) {
            Ok(Some(value)) => value,
            // 下标越界或 key 为空：LINDEX 返回空 bulk，视为未命中。
            Ok(None) => return Ok(make_page(Vec::new())),
            Err(error) if redis_is_wrongtype(&error) => {
                // LLEN 与 LINDEX 之间键类型发生变化（极端竞态），同样按非 List 提示。
                return Err(Error::new(
                    ErrorKind::Query,
                    format!(
                        "键「{}」在 DB {} 当前类型不是 List，可能已被删除/重建为其他类型或已串库（原始: {}）",
                        object.name,
                        database,
                        error.message.trim()
                    ),
                ));
            }
            Err(error) => return Err(error),
        };
        return Ok(make_page(vec![(index, value)]));
    }
    if limit == 0 {
        return Ok(make_page(Vec::new()));
    }
    let mut start = redis_cursor_to_index(cursor)?;
    if start >= total {
        return Ok(make_page(Vec::new()));
    }

    let mut items = Vec::with_capacity(limit);
    while items.len() < limit && start < total {
        let batch = limit - items.len();
        let end = start.saturating_add(batch).saturating_sub(1).min(total - 1);
        let args = [
            "LRANGE".to_string(),
            object.name.clone(),
            start.to_string(),
            end.to_string(),
        ];
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        let values = redis_parse_range_values(connection.command(&args)?)?;
        if values.is_empty() {
            start = total;
            break;
        }
        for (offset, value) in values.into_iter().enumerate() {
            items.push((start + offset, value));
        }
        start = end + 1;
    }

    Ok(RedisListItemPage {
        items,
        next_cursor: if start >= total {
            "0".to_string()
        } else {
            start.to_string()
        },
        total,
    })
}

fn redis_push_list_items(
    config: &ConnectionConfig,
    object: &ObjectPath,
    items: &[String],
    head: bool,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis List 操作缺少 Key"));
    }
    if items.is_empty() {
        return Err(Error::new(ErrorKind::Query, "List 至少需要一个元素"));
    }
    for value in items {
        redis_reject_binary_placeholder(value)?;
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let mut args = vec![
        if head { "LPUSH" } else { "RPUSH" }.to_string(),
        object.name.clone(),
    ];
    if head {
        for value in items.iter().rev() {
            args.push(value.clone());
        }
    } else {
        for value in items {
            args.push(value.clone());
        }
    }
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    match connection.command(&args)? {
        RedisValue::Int(_) => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis LPUSH/RPUSH 返回格式异常")),
    }
}

fn redis_set_list_item(
    config: &ConnectionConfig,
    object: &ObjectPath,
    index: usize,
    expected_old: Option<&str>,
    value: &str,
) -> fluxdb_core::Result<()> {
    redis_reject_binary_placeholder(value)?;
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis List 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    if let Some(expected_old) = expected_old {
        let current = redis_list_item_at(&mut connection, object.name.as_str(), index)?;
        if current.as_deref() != Some(expected_old) {
            return Err(Error::new(ErrorKind::Query, "列表已变化，请刷新后重试"));
        }
    }
    match connection.command(&["LSET", object.name.as_str(), &index.to_string(), value])? {
        RedisValue::Simple(value) if value.eq_ignore_ascii_case("OK") => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis LSET 返回格式异常")),
    }
}

fn redis_delete_list_item(
    config: &ConnectionConfig,
    object: &ObjectPath,
    index: usize,
    expected_old: Option<&str>,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis List 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let current = redis_list_item_at(&mut connection, object.name.as_str(), index)?;
    if let Some(expected_old) = expected_old
        && current.as_deref() != Some(expected_old)
    {
        return Err(Error::new(ErrorKind::Query, "列表已变化，请刷新后重试"));
    }
    let sentinel = format!(
        "__gdb_list_delete_sentinel_{}_{}__",
        index,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| Error::new(ErrorKind::Connection, error.to_string()))?
            .as_nanos()
    );
    match connection.command(&["LSET", object.name.as_str(), &index.to_string(), &sentinel])? {
        RedisValue::Simple(value) if value.eq_ignore_ascii_case("OK") => {}
        _ => return Err(Error::new(ErrorKind::Query, "Redis LSET 返回格式异常")),
    }
    match connection.command(&["LREM", object.name.as_str(), "1", &sentinel])? {
        RedisValue::Int(removed) if removed > 0 => Ok(()),
        RedisValue::Int(_) => Err(Error::new(
            ErrorKind::Query,
            format!("删除未完成，列表中残留占位值 {sentinel}，请刷新后手动删除"),
        )),
        _ => Err(Error::new(ErrorKind::Query, "Redis LREM 返回格式异常")),
    }
}

/// 从 List 的头部/尾部弹出若干元素（对齐 RedisInsight 的 Remove elements）。
///
/// 对齐 RedisInsight 语义：从头/尾按「数量」弹出，而不是按下标删除。
/// - `count == 1`：发 `LPOP key` / `RPOP key`（所有版本可用）。
/// - `count > 1`：发 `LPOP key count` / `RPOP key count`，需 Redis ≥ 6.2；
///   低版本返回清晰中文提示，避免把裸的 wrong number of arguments 抛给用户。
/// List 元素本身无类型约束，直接解析返回数量（Int 或 Array），不校验内容。
fn redis_pop_list_items(
    config: &ConnectionConfig,
    object: &ObjectPath,
    head: bool,
    count: usize,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis List 操作缺少 Key"));
    }
    if count == 0 {
        return Err(Error::new(ErrorKind::Query, "删除数量必须大于 0"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let command_name = if head { "LPOP" } else { "RPOP" };
    // 多数量弹出的 count 参数自 Redis 6.2 起支持；读到版本号后按能力门禁。
    if count > 1 {
        let version = match connection.command(&["INFO", "server"])? {
            RedisValue::Bulk(Some(bytes)) => redis_info_text_map(&redis_text_from_bytes(bytes))
                .get("redis_version")
                .and_then(|raw| RedisServerVersion::parse(raw)),
            _ => None,
        };
        if let Some(version) = version
            && !version.at_least(6, 2)
        {
            return Err(Error::new(
                ErrorKind::Query,
                format!(
                    "同时删除多个 List 元素需要 Redis 6.2 及以上版本（当前 {}.{}.{}），请改为单次删除 1 个",
                    version.major, version.minor, version.patch
                ),
            ));
        }
    }
    let args = vec![
        command_name.to_string(),
        object.name.clone(),
        count.to_string(),
    ];
    // 无论 count 为 1 还是多个都带 count 参数（`LPOP key N`），返回 Array，解析统一。
    // LPOP/RPOP 对「成功、或列表为空返回空/nil」都返回合法值，命令级错误由 command() 以 Err 抛出，
    // 这里只需确认命令执行成功即可，不校验返回值内容。
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    connection.command(&args)?;
    Ok(())
}

fn redis_list_item_at(
    connection: &mut RedisConnection,
    key: &str,
    index: usize,
) -> fluxdb_core::Result<Option<String>> {
    match connection.command(&["LINDEX", key, &index.to_string()])? {
        RedisValue::Bulk(Some(value)) => Ok(Some(redis_text_from_bytes(value))),
        RedisValue::Simple(value) => Ok(Some(value)),
        RedisValue::Bulk(None) => Ok(None),
        _ => Err(Error::new(ErrorKind::Query, "Redis LINDEX 返回格式异常")),
    }
}

fn redis_cursor_to_index(cursor: &str) -> fluxdb_core::Result<usize> {
    let cursor = cursor.trim();
    if cursor.is_empty() {
        return Ok(0);
    }
    cursor
        .parse::<usize>()
        .map_err(|_| Error::new(ErrorKind::Query, "Redis 游标格式异常"))
}
