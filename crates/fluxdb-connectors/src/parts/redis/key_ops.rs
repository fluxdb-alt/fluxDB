/// 新建 Key 的类型。与键列表「类型」列的取值一一对应，也决定值区如何解释：见 [`RedisAddKeyRequest`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedisAddKeyKind {
    String,
    Json,
    List,
    Set,
    Hash,
    ZSet,
    Stream,
}

/// List 新增元素时的插入方向：尾（RPUSH，默认）或头（LPUSH）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedisListDirection {
    Tail,
    Head,
}

/// 「新增 Key」抽屉提交的建 Key 请求（对齐 RedisInsight AddKey 的数据模型）。
///
/// Redis 没有空集合，集合类必须带至少一个元素；值区按类型解释：
/// - `String` / `Json`：`value` 整段文本就是值
/// - `List` / `Set`：`value` 按行拆分，每行一个元素；List 用 `list_direction` 决定从尾/头插入
/// - `Hash` / `Stream`：`pairs` 每项 `(field, field_value)`，作为字段（Stream 为首条 Entry 的字段）
/// - `ZSet`：`pairs` 每项 `(member, score)`，score 必须是数字
/// - `Stream`：`stream_entry_id` 非空时用作首条 Entry 的 id，空/None 走 `*` 自动生成
#[derive(Clone, Debug, PartialEq)]
pub struct RedisAddKeyRequest {
    pub key: String,
    pub kind: RedisAddKeyKind,
    pub value: String,
    pub pairs: Vec<(String, String)>,
    pub ttl: String,
    /// Hash 各字段的字段级 TTL（秒）：与 `pairs` 平行，仅 Hash 使用；
    /// `Vec` 项 `None` 表示该字段不过期。非 Hash 类型传 `None`。
    pub hash_field_ttls: Option<Vec<Option<u64>>>,
    /// List 插入方向；`None` 等价于 `Tail`（兼容 grid 插入路径）。
    pub list_direction: Option<RedisListDirection>,
    /// Stream 首条 Entry 的显式 id；`None`/空串走 `*` 自动生成。
    pub stream_entry_id: Option<String>,
}

/// 建 Key 入口：连接并选中目标库后执行创建（供「新增 Key」抽屉调用）。
/// 与 grid 插入 `redis_apply_changes` 走同一套建 Key 核心，行为保持一致。
fn redis_create_key(
    config: &ConnectionConfig,
    object: &ObjectPath,
    request: &RedisAddKeyRequest,
) -> fluxdb_core::Result<()> {
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let kind = match request.kind {
        RedisAddKeyKind::String => "string",
        RedisAddKeyKind::Json => "json",
        RedisAddKeyKind::List => "list",
        RedisAddKeyKind::Set => "set",
        RedisAddKeyKind::Hash => "hash",
        RedisAddKeyKind::ZSet => "zset",
        RedisAddKeyKind::Stream => "stream",
    };
    redis_emit_key_create(&mut connection, request.key.as_str(), kind, request, request.ttl.as_str())
}

/// 新建 Key 核心：EXISTS 防覆盖 → 按类型执行建命令 → 应用 TTL。
/// `redis_apply_key_insert`（grid 插入）与 `redis_create_key`（新增 Key 抽屉）共用，
/// 确保两种入口的类型解释与校验完全一致。
fn redis_emit_key_create(
    connection: &mut RedisConnection,
    key: &str,
    type_name: &str,
    request: &RedisAddKeyRequest,
    ttl: &str,
) -> fluxdb_core::Result<()> {
    // 覆盖已有 Key 一定是误操作，直接拒绝。
    match connection.command(&["EXISTS", key])? {
        RedisValue::Int(0) => {}
        RedisValue::Int(_) => {
            return Err(Error::new(ErrorKind::Query, "Redis Key 已存在"));
        }
        _ => return Err(Error::new(ErrorKind::Query, "Redis EXISTS 返回格式异常")),
    }

    let lines = || {
        request
            .value
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
    };
    let require_lines = |what: &str| {
        let items = lines();
        if items.is_empty() {
            Err(Error::new(
                ErrorKind::Query,
                format!("新建 {what} 至少需要一个元素"),
            ))
        } else {
            Ok(items)
        }
    };

    match type_name {
        "string" => redis_expect_ok(connection.command(&["SET", key, request.value.as_str()])?)?,
        "json" | "rejson-rl" => {
            redis_expect_ok(connection.command(&["JSON.SET", key, ".", request.value.as_str()])?)?
        }
        "list" => {
            let items = require_lines("List")?;
            // 按方向选择命令：Tail=RPUSH（默认），Head=LPUSH。
            let command = match request.list_direction {
                Some(RedisListDirection::Head) => "LPUSH",
                _ => "RPUSH",
            };
            let mut args = vec![command.to_string(), key.to_string()];
            args.extend(items.into_iter().map(str::to_string));
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            connection.command(&args)?;
        }
        "set" => {
            let items = require_lines("Set")?;
            let mut args = vec!["SADD".to_string(), key.to_string()];
            args.extend(items.into_iter().map(str::to_string));
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            connection.command(&args)?;
        }
        "hash" => {
            if request.pairs.is_empty() {
                return Err(Error::new(
                    ErrorKind::Query,
                    "新建 Hash 至少需要一个字段",
                ));
            }
            let mut args = vec!["HSET".to_string(), key.to_string()];
            for (field, field_value) in &request.pairs {
                args.push(field.clone());
                args.push(field_value.clone());
            }
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            connection.command(&args)?;
            // 逐字段应用字段级 TTL（Redis 7.4+）：None → HPERSIST 明确永久（兼容旧版），
            // Some(secs) → HPEXPIRE 设秒级过期；未提供 hash_field_ttls（如 grid 插入路径）则跳过。
            if let Some(ttls) = &request.hash_field_ttls {
                for ((field, _), ttl) in request.pairs.iter().zip(ttls.iter()) {
                    if let Some(seconds) = ttl {
                        let ttl_ms = redis_hash_field_ttl_ms_from_seconds(*seconds)?;
                        redis_hpexpire_hash_field(&mut *connection, key, field, ttl_ms)?;
                    } else {
                        redis_hpersist_hash_field(&mut *connection, key, field)?;
                    }
                }
            }
        }
        "zset" => {
            if request.pairs.is_empty() {
                return Err(Error::new(
                    ErrorKind::Query,
                    "新建 ZSet 至少需要一个成员",
                ));
            }
            let mut args = vec!["ZADD".to_string(), key.to_string()];
            for (member, score) in &request.pairs {
                if score.parse::<f64>().is_err() {
                    return Err(Error::new(ErrorKind::Query, "ZSet 的 score 必须是数字"));
                }
                args.push(score.clone());
                args.push(member.clone());
            }
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            connection.command(&args)?;
        }
        "stream" => {
            if request.pairs.is_empty() {
                return Err(Error::new(
                    ErrorKind::Query,
                    "新建 Stream 至少需要一个字段",
                ));
            }
            // 首条 Entry 的 id：显式提供则直接使用（对齐 RedisInsight），否则 `*` 自动生成。
            let entry_id = request
                .stream_entry_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .unwrap_or("*");
            let mut args = vec![
                "XADD".to_string(),
                key.to_string(),
                entry_id.to_string(),
            ];
            for (field, field_value) in &request.pairs {
                args.push(field.clone());
                args.push(field_value.clone());
            }
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            connection.command(&args)?;
        }
        other => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("Redis 暂不支持新建 {other} 类型的 Key"),
            ));
        }
    }

    if !ttl.trim().is_empty() {
        // 复用与「改 TTL」一致的解析与命令，行为保持统一。
        redis_set_key_ttl(connection, key, ttl.trim())?;
    }
    Ok(())
}

fn redis_apply_changes(config: &ConnectionConfig, changes: &DataChangeSet) -> fluxdb_core::Result<()> {
    let database = redis_path_database(&changes.object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    for insert in &changes.inserts {
        redis_apply_key_insert(&mut connection, &changes.object, insert)?;
    }
    for update in &changes.updates {
        redis_apply_key_update(&mut connection, update)?;
    }
    for identity in &changes.deletes {
        let key = identity
            .values
            .get("键")
            .map(CellValue::display_label)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| Error::new(ErrorKind::Internal, "Redis 删除缺少 Key"))?;
        match connection.command(&["DEL", key.as_str()])? {
            RedisValue::Int(_) => {}
            _ => return Err(Error::new(ErrorKind::Query, "Redis DEL 返回格式异常")),
        }
    }
    Ok(())
}

/// 新建 Key。行按 Key 列表的列序给出：键 / 类型 / 值 / 大小 / TTL（大小列只读，忽略）。
///
/// Redis 没有「空集合」，集合类必须带至少一个元素才能建出来，因此值列按类型解释：
/// - string / json：整段文本就是值
/// - list / set：按行拆分，每行一个元素
/// - hash：每行 `field=value`
/// - zset：每行 `member=score`，score 必须是数字
/// - stream：每行 `field=value`，一起作为首条 Entry 的字段
fn redis_apply_key_insert(
    connection: &mut RedisConnection,
    object: &ObjectPath,
    row: &Row,
) -> fluxdb_core::Result<()> {
    let cell = |index: usize| {
        row.values
            .get(index)
            .filter(|value| !matches!(value, CellValue::Null))
            .map(CellValue::display_label)
            .unwrap_or_default()
    };
    let key = cell(0);
    let key = key.trim();
    if key.is_empty() {
        return Err(Error::new(ErrorKind::Query, "Redis Key 名称不能为空"));
    }
    redis_reject_binary_placeholder(key)?;
    let raw_type = cell(1);
    let type_name = if raw_type.trim().is_empty() {
        "string".to_string()
    } else {
        raw_type.trim().to_ascii_lowercase()
    };
    let value = cell(2);
    redis_reject_binary_placeholder(&value)?;
    let ttl = cell(4);

    // 把行的值列归一化成对应的请求结构，再交给统一的建 Key 核心，
    // 与「新增 Key」抽屉（redis_create_key）走同一套校验与命令。
    let split_pair = |line: &str, what: &str| -> fluxdb_core::Result<(String, String)> {
        line.split_once('=')
            .map(|(left, right)| (left.trim().to_string(), right.trim().to_string()))
            .filter(|(left, _)| !left.is_empty())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Query,
                    format!("新建 {what} 的每行需要写成 `名称=值`"),
                )
            })
    };
    let request = match type_name.as_str() {
        "list" | "set" | "string" | "json" | "rejson-rl" => RedisAddKeyRequest {
            key: key.to_string(),
            kind: type_to_add_key_kind(&type_name),
            value,
            pairs: Vec::new(),
            ttl,
            hash_field_ttls: None,
            list_direction: None,
            stream_entry_id: None,
        },
        "hash" | "stream" | "zset" => {
            let pairs = value
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| split_pair(line, &type_name))
                .collect::<fluxdb_core::Result<Vec<_>>>()?;
            RedisAddKeyRequest {
                key: key.to_string(),
                kind: type_to_add_key_kind(&type_name),
                value: String::new(),
                pairs,
                ttl,
                hash_field_ttls: None,
                list_direction: None,
                stream_entry_id: None,
            }
        }
        other => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("Redis 暂不支持新建 {other} 类型的 Key"),
            ));
        }
    };
    let _ = object;
    redis_emit_key_create(connection, key, &type_name, &request, &request.ttl)
}

/// 把键列表「类型」列字符串（含别名）映射到 [`RedisAddKeyKind`]。
/// 未知类型返回 `None`，由调用方用 [`ErrorKind::Unsupported`] 报告。
fn type_to_add_key_kind(type_name: &str) -> RedisAddKeyKind {
    match type_name {
        "string" => RedisAddKeyKind::String,
        "json" | "rejson-rl" => RedisAddKeyKind::Json,
        "list" => RedisAddKeyKind::List,
        "set" => RedisAddKeyKind::Set,
        "hash" => RedisAddKeyKind::Hash,
        "zset" => RedisAddKeyKind::ZSet,
        "stream" => RedisAddKeyKind::Stream,
        _ => RedisAddKeyKind::String,
    }
}

fn redis_apply_key_update(
    connection: &mut RedisConnection,
    update: &RowUpdate,
) -> fluxdb_core::Result<()> {
    let mut key = update
        .identity
        .values
        .get("键")
        .map(CellValue::display_label)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Internal, "Redis 更新缺少 Key"))?;

    for cell in &update.cells {
        if cell.column == "键" {
            let new_key = redis_text_cell(&cell.value, "Redis Key 名称必须是文本")?;
            redis_rename_key(connection, key.as_str(), new_key)?;
            key = new_key.to_string();
        }
    }
    for cell in &update.cells {
        match cell.column.as_str() {
            "键" | "TTL" => {}
            "值" => redis_set_key_value(
                connection,
                key.as_str(),
                redis_text_cell(&cell.value, "Redis 值修改仅支持文本内容")?,
            )?,
            _ => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!("Redis 暂不支持修改 {} 列", cell.column),
                ));
            }
        }
    }
    for cell in &update.cells {
        if cell.column == "TTL" {
            redis_set_key_ttl(
                connection,
                key.as_str(),
                redis_text_cell(&cell.value, "Redis TTL 必须是文本")?,
            )?;
        }
    }
    Ok(())
}

fn redis_text_cell<'a>(value: &'a CellValue, message: &str) -> fluxdb_core::Result<&'a str> {
    match value {
        CellValue::Text(value) | CellValue::Json(value) => Ok(value.as_str()),
        _ => Err(Error::new(ErrorKind::Unsupported, message)),
    }
}

fn redis_rename_key(
    connection: &mut RedisConnection,
    key: &str,
    new_key: &str,
) -> fluxdb_core::Result<()> {
    if new_key.trim().is_empty() {
        return Err(Error::new(ErrorKind::Query, "Redis Key 名称不能为空"));
    }
    if key == new_key {
        return Ok(());
    }
    match connection.command(&["RENAMENX", key, new_key])? {
        RedisValue::Int(1) => Ok(()),
        RedisValue::Int(0) => Err(Error::new(ErrorKind::Query, "目标 Redis Key 已存在")),
        _ => Err(Error::new(ErrorKind::Query, "Redis RENAMENX 返回格式异常")),
    }
}

fn redis_set_key_ttl(
    connection: &mut RedisConnection,
    key: &str,
    ttl: &str,
) -> fluxdb_core::Result<()> {
    let ttl = ttl.trim();
    if ttl.is_empty() {
        return match connection.command(&["PERSIST", key])? {
            RedisValue::Int(_) => Ok(()),
            _ => Err(Error::new(ErrorKind::Query, "Redis PERSIST 返回格式异常")),
        };
    }
    let seconds = ttl
        .parse::<u64>()
        .map_err(|_| Error::new(ErrorKind::Query, "Redis TTL 必须是秒数"))?;
    if seconds == 0 {
        return Err(Error::new(ErrorKind::Query, "Redis TTL 必须大于 0 秒或留空"));
    }
    match connection.command(&["EXPIRE", key, &seconds.to_string()])? {
        RedisValue::Int(1) => Ok(()),
        RedisValue::Int(0) => Err(Error::new(ErrorKind::Query, "Redis Key 不存在")),
        _ => Err(Error::new(ErrorKind::Query, "Redis EXPIRE 返回格式异常")),
    }
}

fn redis_set_key_value(
    connection: &mut RedisConnection,
    key: &str,
    value: &str,
) -> fluxdb_core::Result<()> {
    redis_reject_binary_placeholder(value)?;
    let raw_type = redis_value_text(connection.command(&["TYPE", key])?);
    // 对齐 RedisInsight「编辑值后保留 TTL」语义：SET/JSON.SET/替换 set members 等写值命令
    // 会清掉 key 的过期时间，这里先在写入前记录既有 TTL（无 TTL 记 None），写值后补回，
    // 避免用户只是改值却意外把 key 的过期时间清掉；若本次更新同时显式改了 TTL，
    // redis_apply_key_update 的 TTL 循环会在之后覆盖，显式 TTL 优先。
    let ttl = redis_key_ttl_seconds(connection, key)?;
    match raw_type.to_ascii_lowercase().as_str() {
        "string" => redis_expect_ok(connection.command(&["SET", key, value])?)?,
        "rejson-rl" | "json" => {
            redis_expect_ok(connection.command(&["JSON.SET", key, ".", value])?)?
        }
        "set" => redis_set_members(connection, key, value)?,
        "none" => return Err(Error::new(ErrorKind::Query, "Redis Key 不存在")),
        _ => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("Redis 暂不支持修改 {raw_type} 类型的值"),
            ))
        }
    }
    redis_restore_key_ttl(connection, key, ttl)
}

/// 读取 key 的剩余 TTL（秒）：`Some(secs)` 表示有过期时间，`None` 表示永不超时（-1）
/// 或已不存在（-2）。写值命令会清掉过期时间，调用方用它做「值变更后保留 TTL」。
fn redis_key_ttl_seconds(
    connection: &mut RedisConnection,
    key: &str,
) -> fluxdb_core::Result<Option<u64>> {
    match connection.command(&["TTL", key])? {
        RedisValue::Int(ttl) if ttl > 0 => Ok(Some(ttl as u64)),
        RedisValue::Int(-1) | RedisValue::Int(-2) => Ok(None),
        _ => Err(Error::new(ErrorKind::Query, "Redis TTL 返回格式异常")),
    }
}

/// 把 key 的过期时间补回：`Some(secs)` 用 EXPIRE 恢复，`None` 保持永不超时（不需要动作）。
fn redis_restore_key_ttl(
    connection: &mut RedisConnection,
    key: &str,
    ttl: Option<u64>,
) -> fluxdb_core::Result<()> {
    match ttl {
        Some(seconds) if seconds > 0 => {
            match connection.command(&["EXPIRE", key, &seconds.to_string()])? {
                RedisValue::Int(1) => Ok(()),
                RedisValue::Int(0) => Err(Error::new(ErrorKind::Query, "Redis Key 不存在")),
                _ => Err(Error::new(ErrorKind::Query, "Redis EXPIRE 返回格式异常")),
            }
        }
        _ => Ok(()),
    }
}
