/// Redis Set 成员分页查询结果：一页成员 + 下次游标 + 集合总数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisSetMemberPage {
    pub members: Vec<String>,
    /// 下一次 SSCAN 游标；为 "0" 表示已遍历完。
    pub next_cursor: String,
    /// SCARD 得到的集合成员总数（与 MATCH 无关）。
    pub total: usize,
}

fn redis_srem_set_member(
    config: &ConnectionConfig,
    object: &ObjectPath,
    member: &str,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Set 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    match connection.command(&["SREM", object.name.as_str(), member])? {
        RedisValue::Int(_) => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis SREM 返回格式异常")),
    }
}

fn redis_sadd_set_member(
    config: &ConnectionConfig,
    object: &ObjectPath,
    member: &str,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Set 操作缺少 Key"));
    }
    if member.trim().is_empty() {
        return Err(Error::new(ErrorKind::Query, "Member 不能为空"));
    }
    redis_reject_binary_placeholder(member)?;
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    match connection.command(&["SADD", object.name.as_str(), member])? {
        RedisValue::Int(_) => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis SADD 返回格式异常")),
    }
}

/// 单次 SSCAN 分页查询 Set 成员（对齐 Redis Insight：一次请求一页，游标回传给客户端）。
/// query 非空时按 `*glob_escape(query)*` 做服务端 MATCH 过滤；SCARD 总数一次只读命令回传。
fn redis_sscan_set_members(
    config: &ConnectionConfig,
    object: &ObjectPath,
    query: &str,
    cursor: &str,
    limit: usize,
) -> fluxdb_core::Result<RedisSetMemberPage> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis Set 查询缺少 Key"));
    }
    if limit == 0 {
        return Ok(RedisSetMemberPage {
            members: Vec::new(),
            next_cursor: "0".to_string(),
            total: 0,
        });
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let cursor = if cursor.trim().is_empty() { "0" } else { cursor.trim() };
    let pattern = if query.trim().is_empty() {
        None
    } else {
        Some(format!("*{}*", redis_glob_escape(query.trim())))
    };
    let mut args = vec![
        "SSCAN".to_string(),
        object.name.clone(),
        cursor.to_string(),
        "COUNT".to_string(),
        REDIS_SCAN_COUNT.to_string(),
    ];
    if let Some(pattern) = pattern.as_ref() {
        args.push("MATCH".to_string());
        args.push(pattern.clone());
    }
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    // SSCAN 是首个命令：若键服务端类型已不是 Set（被删除后重建为其他类型、或类型探测结果过期），
    // 会返回 WRONGTYPE。这里把它转成清晰的中文提示，避免把原始 Redis 错误直接抛给用户。
    let RedisValue::Array(items) = (match connection.command(&args) {
        Ok(value) => value,
        Err(error) if redis_is_wrongtype(&error) => {
            return Err(redis_wrongtype_set_message(object, database, &error));
        }
        Err(error) => return Err(error),
    }) else {
        return Err(Error::new(ErrorKind::Query, "Redis SSCAN 返回格式异常"));
    };
    let [next_cursor, member_values] = items.as_slice() else {
        return Err(Error::new(ErrorKind::Query, "Redis SSCAN 返回格式异常"));
    };
    let next_cursor = redis_value_text(next_cursor.clone());
    let mut members = Vec::new();
    if let RedisValue::Array(items) = member_values {
        for member in items {
            members.push(redis_value_text(member.clone()));
            if members.len() >= limit {
                break;
            }
        }
    }
    // SCARD 与游标无关；失败回退为已取成员数，避免整页查询失败。
    // SSCAN 与 SCARD 之间键类型发生变化（极端竞态）时，同样按非 Set 提示而非全页失败。
    let total = match connection.command(&["SCARD", object.name.as_str()]) {
        Ok(RedisValue::Int(n)) => n.max(0) as usize,
        Ok(_) => members.len(),
        Err(error) if redis_is_wrongtype(&error) => {
            return Err(redis_wrongtype_set_message(object, database, &error));
        }
        Err(_) => members.len(),
    };
    Ok(RedisSetMemberPage {
        members,
        next_cursor,
        total,
    })
}

fn redis_set_members(
    connection: &mut RedisConnection,
    key: &str,
    value: &str,
) -> fluxdb_core::Result<()> {
    let desired = redis_set_members_from_text(value)?;
    if desired.is_empty() {
        return Err(Error::new(ErrorKind::Query, "Redis Set 至少需要一个成员"));
    }

    let current = match connection.command(&["SMEMBERS", key])? {
        RedisValue::Array(items) => items
            .into_iter()
            .map(redis_value_text)
            .collect::<std::collections::BTreeSet<_>>(),
        _ => return Err(Error::new(ErrorKind::Query, "Redis SMEMBERS 返回格式异常")),
    };
    let desired = desired.into_iter().collect::<std::collections::BTreeSet<_>>();

    for member in current.difference(&desired) {
        match connection.command(&["SREM", key, member])? {
            RedisValue::Int(_) => {}
            _ => return Err(Error::new(ErrorKind::Query, "Redis SREM 返回格式异常")),
        }
    }
    let added = desired.difference(&current).cloned().collect::<Vec<_>>();
    if !added.is_empty() {
        let mut args = vec!["SADD".to_string(), key.to_string()];
        args.extend(added);
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        match connection.command(&args)? {
            RedisValue::Int(_) => {}
            _ => return Err(Error::new(ErrorKind::Query, "Redis SADD 返回格式异常")),
        }
    }
    Ok(())
}

fn redis_set_preview(length: Option<RedisValue>, members: Option<RedisValue>) -> String {
    let members = redis_array_text(members);
    let length = match length {
        Some(RedisValue::Int(length)) => length,
        _ => members.len() as i64,
    };
    if members.is_empty() {
        format!("{length} 成员")
    } else {
        format!("{length} 成员\n{}", members.join("\n"))
    }
}

fn redis_set_members_from_text(value: &str) -> fluxdb_core::Result<Vec<String>> {
    // ponytail: 先按行存成员，暂不支持成员里包含换行；真要支持时再换成转义编码。
    let members = value
        .lines()
        .map(str::to_string)
        .filter(|member| !member.trim().is_empty())
        .collect::<Vec<_>>();
    Ok(members)
}
