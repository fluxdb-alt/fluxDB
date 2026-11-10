// Redis 连接级服务器面板：INFO / CLIENT LIST / SLOWLOG 读取与处置。
//
// 只做服务端只读观测；终止客户端与清空慢日志在 UI、控制器和连接器三处拦截只读连接。

/// 慢日志一次最多读取的条数。SLOWLOG GET 无分页，条数过大时整包回传会拖慢界面。
const REDIS_SLOWLOG_PAGE_LIMIT: i64 = 128;

fn redis_connector_config(
    config: Option<&ConnectionConfig>,
) -> fluxdb_core::Result<&ConnectionConfig> {
    config.ok_or_else(|| Error::new(ErrorKind::Connection, "Redis 服务器状态需要连接配置上下文"))
}

/// 读取 INFO / CLIENT LIST / SLOWLOG，三个分区独立容错。
fn redis_load_server_activity(
    config: &ConnectionConfig,
) -> fluxdb_core::Result<RedisServerActivity> {
    let mut connection = redis_connect(config)?;
    let RedisValue::Bulk(Some(bytes)) = connection.command(&["INFO"])? else {
        return Err(Error::new(ErrorKind::Query, "Redis INFO 返回格式异常"));
    };
    let info = redis_info_text_map(&redis_text_from_bytes(bytes));

    let current_client_id = match connection.command(&["CLIENT", "ID"]) {
        Ok(RedisValue::Int(id)) => id.to_string(),
        Ok(value) => redis_value_text(value),
        // 旧版本或 ACL 限制读不到自身 ID 时不阻断概览；终止客户端前会再核对。
        Err(_) => String::new(),
    };

    let mut activity = RedisServerActivity {
        version: info.get("redis_version").cloned().unwrap_or_default(),
        mode: info.get("redis_mode").cloned().unwrap_or_default(),
        role: info.get("role").cloned().unwrap_or_default(),
        uptime_secs: redis_server_info_u64(&info, "uptime_in_seconds"),
        connected_clients: redis_server_info_u64(&info, "connected_clients").unwrap_or(0),
        used_memory_bytes: redis_server_info_u64(&info, "used_memory").unwrap_or(0),
        metrics: redis_overview_metrics(&info),
        current_client_id,
        ..RedisServerActivity::default()
    };

    match connection.command(&["CLIENT", "LIST"]) {
        Ok(RedisValue::Bulk(Some(bytes))) => {
            let text = redis_text_from_bytes(bytes);
            activity.clients = redis_parse_client_list(&text, &activity.current_client_id);
        }
        Ok(value) => {
            // 老版本 CLIENT LIST 也可能返回 Simple；按文本再解析一次。
            let text = redis_value_text(value);
            activity.clients = redis_parse_client_list(&text, &activity.current_client_id);
        }
        Err(error) => activity.clients_error = Some(error.message),
    }

    match connection.command(&[
        "SLOWLOG",
        "GET",
        &REDIS_SLOWLOG_PAGE_LIMIT.to_string(),
    ]) {
        Ok(RedisValue::Array(entries)) => {
            activity.slowlog = entries
                .into_iter()
                .filter_map(redis_parse_slowlog_entry)
                .collect();
        }
        Ok(_) => activity.slowlog_error = Some("SLOWLOG GET 返回格式异常".into()),
        Err(error) => activity.slowlog_error = Some(error.message),
    }

    activity.slowlog_len = match connection.command(&["SLOWLOG", "LEN"]) {
        Ok(RedisValue::Int(len)) if len >= 0 => len as u64,
        // LEN 失败不影响 GET 已读到的内容，保留空值由展示层显示 `-`。
        _ => activity.slowlog.len() as u64,
    };

    activity.slowlog_log_slower_than_micros =
        redis_config_int(&mut connection, "slowlog-log-slower-than");
    activity.slowlog_max_len = redis_config_int(&mut connection, "slowlog-max-len");

    Ok(activity)
}

fn redis_server_info_u64(info: &BTreeMap<String, String>, key: &str) -> Option<u64> {
    info.get(key)?.parse::<u64>().ok()
}

/// 概览指标：只放稳定存在、排障常用的字段；界面按 key 本地化。
fn redis_overview_metrics(info: &BTreeMap<String, String>) -> Vec<ServerMetric> {
    const KEYS: &[(&str, bool)] = &[
        ("redis_version", false),
        ("redis_mode", false),
        ("role", false),
        ("uptime_in_seconds", false),
        ("connected_clients", false),
        ("blocked_clients", true),
        ("used_memory", false),
        ("used_memory_human", false),
        ("used_memory_peak_human", false),
        ("maxmemory_human", false),
        ("maxmemory_policy", false),
        ("mem_fragmentation_ratio", false),
        ("instantaneous_ops_per_sec", false),
        ("total_connections_received", false),
        ("total_commands_processed", false),
        ("keyspace_hits", false),
        ("keyspace_misses", false),
        ("expired_keys", false),
        ("evicted_keys", true),
        ("rejected_connections", true),
        ("rdb_last_bgsave_status", true),
        ("aof_last_write_status", true),
    ];
    KEYS.iter()
        .filter_map(|(key, warning)| {
            info.get(*key).map(|value| ServerMetric {
                key: (*key).to_string(),
                value: value.clone(),
                // 关键状态字段只有异常值才标黄；数量类字段非零才标黄。
                warning: *warning && redis_metric_needs_attention(key, value),
            })
        })
        .collect()
}

fn redis_metric_needs_attention(key: &str, value: &str) -> bool {
    match key {
        "rdb_last_bgsave_status" | "aof_last_write_status" => {
            !value.eq_ignore_ascii_case("ok")
        }
        _ => value.parse::<u64>().is_ok_and(|n| n > 0),
    }
}

/// 解析 `CLIENT LIST` 文本。每行 `key=value` 以空格分隔，
/// `cmd=client|list` 这类值里可能包含 `|`，不影响按空白切分。
fn redis_parse_client_list(text: &str, current_client_id: &str) -> Vec<RedisClient> {
    text.lines()
        .filter_map(|line| {
            let mut fields = BTreeMap::new();
            for token in line.split_whitespace() {
                if let Some((key, value)) = token.split_once('=') {
                    fields.insert(key, value);
                }
            }
            let id = fields.get("id")?.to_string();
            Some(RedisClient {
                is_self: !current_client_id.is_empty() && id == current_client_id,
                id,
                addr: fields.get("addr").unwrap_or(&"").to_string(),
                local_addr: fields.get("laddr").unwrap_or(&"").to_string(),
                name: redis_client_list_value(fields.get("name")),
                user: fields.get("user").unwrap_or(&"").to_string(),
                database: fields.get("db").unwrap_or(&"").to_string(),
                command: fields.get("cmd").unwrap_or(&"").to_string(),
                age_secs: redis_client_list_u64(fields.get("age")),
                idle_secs: redis_client_list_u64(fields.get("idle")),
                flags: fields.get("flags").unwrap_or(&"").to_string(),
            })
        })
        .collect()
}

fn redis_client_list_value(value: Option<&&str>) -> String {
    value
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn redis_client_list_u64(value: Option<&&str>) -> u64 {
    value.and_then(|value| value.parse().ok()).unwrap_or(0)
}

/// 解析一条 `SLOWLOG GET` 记录：[id, timestamp, duration, args, client addr, client name]。
fn redis_parse_slowlog_entry(value: RedisValue) -> Option<RedisSlowLogEntry> {
    let RedisValue::Array(items) = value else {
        return None;
    };
    let mut items = items.into_iter();
    let id = redis_slowlog_u64(items.next()?)?;
    let timestamp_unix_secs = redis_slowlog_u64(items.next()?)? as i64;
    let duration_micros = redis_slowlog_u64(items.next()?)?;
    let command = match items.next()? {
        RedisValue::Array(args) => args
            .into_iter()
            .map(redis_value_text)
            .collect::<Vec<_>>()
            .join(" "),
        value => redis_value_text(value),
    };
    let client_addr = items.next().map(redis_value_text).unwrap_or_default();
    let client_name = items.next().map(redis_value_text).unwrap_or_default();
    Some(RedisSlowLogEntry {
        id,
        timestamp_unix_secs,
        duration_micros,
        command,
        client_addr,
        client_name,
    })
}

fn redis_slowlog_u64(value: RedisValue) -> Option<u64> {
    match value {
        RedisValue::Int(value) if value >= 0 => Some(value as u64),
        RedisValue::Bulk(Some(bytes)) => redis_text_from_bytes(bytes).parse().ok(),
        _ => None,
    }
}

/// `CONFIG GET <name>` 返回 `[name, value]`；配置被禁用/重命名时返回 `None`。
fn redis_config_int(connection: &mut RedisConnection, name: &str) -> Option<i64> {
    let value = connection.command(&["CONFIG", "GET", name]).ok()?;
    let RedisValue::Array(items) = value else {
        return None;
    };
    let text = items.get(1).map(|value| redis_value_text(value.clone()))?;
    text.parse().ok()
}

fn redis_kill_client(config: &ConnectionConfig, client_id: &str) -> fluxdb_core::Result<()> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许终止 Redis 客户端",
        ));
    }
    if client_id.trim().is_empty() {
        return Err(Error::new(ErrorKind::Query, "客户端 ID 不能为空"));
    }
    let mut connection = redis_connect(config)?;
    // 面板自身连接不能杀：先取自身 ID，避免把状态页自己的连接断掉。
    if let Ok(RedisValue::Int(self_id)) = connection.command(&["CLIENT", "ID"])
        && self_id.to_string() == client_id
    {
        return Err(Error::new(
            ErrorKind::Query,
            "不能终止服务器面板自身使用的连接",
        ));
    }
    match connection.command(&["CLIENT", "KILL", "ID", client_id])? {
        RedisValue::Int(killed) if killed > 0 => Ok(()),
        RedisValue::Int(_) => Err(Error::new(ErrorKind::Query, "客户端不存在或已断开")),
        RedisValue::Simple(value) if value.eq_ignore_ascii_case("OK") => Ok(()),
        _ => Err(Error::new(ErrorKind::Query, "Redis CLIENT KILL 返回格式异常")),
    }
}

fn redis_clear_slowlog(config: &ConnectionConfig) -> fluxdb_core::Result<()> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许清空 Redis 慢日志",
        ));
    }
    let mut connection = redis_connect(config)?;
    redis_expect_ok(connection.command(&["SLOWLOG", "RESET"])?)
}

#[cfg(test)]
mod redis_server_tests {
    use super::*;

    fn bulk(text: &str) -> RedisValue {
        RedisValue::Bulk(Some(text.as_bytes().to_vec()))
    }

    #[test]
    fn client_list_parses_fields_and_marks_self() {
        let text = "id=7 addr=127.0.0.1:5000 laddr=127.0.0.1:6379 fd=9 name= age=12 idle=0 flags=N db=0 sub=0 psub=0 ssub=0 multi=-1 watch=0 qbuf=26 cmd=client|list user=default redir=-1 resp=2\n\
                    id=8 addr=127.0.0.1:5001 laddr=127.0.0.1:6379 fd=10 name=worker age=3 idle=90 flags=N db=2 cmd=get user=app\n";
        let clients = redis_parse_client_list(text, "7");
        assert_eq!(clients.len(), 2);
        assert!(clients[0].is_self);
        assert_eq!(clients[0].id, "7");
        assert_eq!(clients[0].command, "client|list");
        assert_eq!(clients[1].id, "8");
        assert!(!clients[1].is_self);
        assert_eq!(clients[1].name, "worker");
        assert_eq!(clients[1].database, "2");
        assert_eq!(clients[1].idle_secs, 90);
    }

    #[test]
    fn slowlog_entry_parses_timestamp_duration_and_args() {
        let entry = redis_parse_slowlog_entry(RedisValue::Array(vec![
            RedisValue::Int(3),
            RedisValue::Int(1_700_000_000),
            RedisValue::Int(250),
            RedisValue::Array(vec![bulk("SET"), bulk("k"), bulk("v")]),
            bulk("127.0.0.1:5000"),
            bulk("worker"),
        ]))
        .expect("slowlog entry");
        assert_eq!(entry.id, 3);
        assert_eq!(entry.timestamp_unix_secs, 1_700_000_000);
        assert_eq!(entry.duration_micros, 250);
        assert_eq!(entry.command, "SET k v");
        assert_eq!(entry.client_addr, "127.0.0.1:5000");
        assert_eq!(entry.client_name, "worker");
    }

    #[test]
    fn slowlog_entry_ignores_malformed_value() {
        assert!(redis_parse_slowlog_entry(RedisValue::Int(1)).is_none());
        assert!(
            redis_parse_slowlog_entry(RedisValue::Array(vec![RedisValue::Int(-1)])).is_none(),
            "负 id 不是合法记录"
        );
    }

    #[test]
    fn overview_metrics_warn_only_on_abnormal_values() {
        let info: BTreeMap<String, String> = [
            ("redis_version", "7.2.0"),
            ("rdb_last_bgsave_status", "ok"),
            ("aof_last_write_status", "err"),
            ("evicted_keys", "0"),
            ("rejected_connections", "2"),
            ("unknown_field", "x"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let metrics = redis_overview_metrics(&info);
        let find = |key: &str| {
            metrics
                .iter()
                .find(|metric| metric.key == key)
                .unwrap_or_else(|| panic!("缺少指标 {key}"))
        };
        assert!(!find("redis_version").warning);
        assert!(!find("rdb_last_bgsave_status").warning, "ok 不告警");
        assert!(find("aof_last_write_status").warning, "非 ok 告警");
        assert!(!find("evicted_keys").warning, "0 不告警");
        assert!(find("rejected_connections").warning, "非零数量告警");
        assert!(
            !metrics.iter().any(|metric| metric.key == "unknown_field"),
            "只输出白名单指标"
        );
    }

    fn config(read_only: bool) -> ConnectionConfig {
        let mut options = BTreeMap::new();
        if read_only {
            options.insert("read_only".to_string(), "true".to_string());
        }
        ConnectionConfig {
            id: ConnectionId(1),
            name: "redis".into(),
            kind: DatabaseKind::Redis,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1,
                database: None,
            },
            options,
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    #[test]
    fn read_only_connection_rejects_actions_before_connecting() {
        let config = config(true);
        assert_eq!(
            redis_kill_client(&config, "7").unwrap_err().kind,
            ErrorKind::Unsupported
        );
        assert_eq!(
            redis_clear_slowlog(&config).unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }

    #[test]
    fn empty_client_id_rejected_before_connecting() {
        let config = config(false);
        assert_eq!(
            redis_kill_client(&config, "  ").unwrap_err().kind,
            ErrorKind::Query
        );
    }
}
