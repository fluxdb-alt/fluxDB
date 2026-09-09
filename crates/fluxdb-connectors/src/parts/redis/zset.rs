/// Redis ZSet 成员分页查询结果：一页 member/score + 下次游标 + 总数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedisZSetMemberPage {
    pub members: Vec<(String, String)>,
    pub next_cursor: String,
    pub total: usize,
}

fn redis_load_zset_members(
    config: &ConnectionConfig,
    object: &ObjectPath,
    query: &str,
    cursor: &str,
    limit: usize,
) -> fluxdb_core::Result<RedisZSetMemberPage> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis ZSet 查询缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    let total = match connection.command(&["ZCARD", object.name.as_str()])? {
        RedisValue::Int(n) => n.max(0) as usize,
        _ => return Err(Error::new(ErrorKind::Query, "Redis ZCARD 返回格式异常")),
    };
    if limit == 0 {
        return Ok(RedisZSetMemberPage {
            members: Vec::new(),
            next_cursor: "0".to_string(),
            total,
        });
    }
    if query.trim().is_empty() {
        let start = redis_cursor_to_index(cursor)?;
        if start >= total {
            return Ok(RedisZSetMemberPage {
                members: Vec::new(),
                next_cursor: "0".to_string(),
                total,
            });
        }
        let end = start.saturating_add(limit).saturating_sub(1);
        let args = [
            "ZRANGE".to_string(),
            object.name.clone(),
            start.to_string(),
            end.to_string(),
            "WITHSCORES".to_string(),
        ];
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        let members = redis_parse_range_pairs(connection.command(&args)?)?;
        let next_cursor = if start.saturating_add(members.len()) >= total {
            "0".to_string()
        } else {
            (start + members.len()).to_string()
        };
        return Ok(RedisZSetMemberPage {
            members,
            next_cursor,
            total,
        });
    }

    let cursor = if cursor.trim().is_empty() { "0" } else { cursor.trim() };
    let args = vec![
        "ZSCAN".to_string(),
        object.name.clone(),
        cursor.to_string(),
        "COUNT".to_string(),
        REDIS_SCAN_COUNT.to_string(),
        "MATCH".to_string(),
        format!("*{}*", redis_glob_escape(query.trim())),
    ];
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let (next_cursor, members) = redis_parse_scan_pairs(connection.command(&args)?, limit)?;
    Ok(RedisZSetMemberPage {
        members,
        next_cursor,
        total,
    })
}

fn redis_zadd_member(
    config: &ConnectionConfig,
    object: &ObjectPath,
    member: &str,
    score: &str,
) -> fluxdb_core::Result<()> {
    redis_reject_binary_placeholder(member)?;
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis ZSet 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    match connection.command(&["ZADD", object.name.as_str(), score, member])? {
        RedisValue::Int(_) => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis ZADD 返回格式异常")),
    }
}

fn redis_zrem_member(
    config: &ConnectionConfig,
    object: &ObjectPath,
    member: &str,
) -> fluxdb_core::Result<()> {
    if object.kind != ObjectKind::RedisKey {
        return Err(Error::new(ErrorKind::Internal, "Redis ZSet 操作缺少 Key"));
    }
    let database = redis_path_database(object)?;
    let mut connection = redis_connect(config)?;
    redis_select(&mut connection, database)?;
    match connection.command(&["ZREM", object.name.as_str(), member])? {
        RedisValue::Int(_) => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis ZREM 返回格式异常")),
    }
}
