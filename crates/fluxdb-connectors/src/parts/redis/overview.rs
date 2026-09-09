/// 把 `INFO` 的扁平文本解析为 `字段名 -> 字符串值` 映射。
/// INFO 返回以 `# 分区名` 分组、每行 `key:value` 的空行分隔文本；
/// 这里不关心分区，只取字段名（不同分区里的字段名不会冲突）。
fn redis_info_text_map(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

/// 读取 Redis 连接级运行概览：版本（`redis_version`）、内存（`memory.used_memory`，
/// 单位字节）与 CPU 采样（`cpu.used_cpu_sys/user` + `server.uptime_in_seconds`）。
/// 一次 `INFO` 覆盖所有分区，字段跨区不冲突，单条命令即可取全。
fn redis_load_overview(connection: &mut RedisConnection) -> fluxdb_core::Result<ConnectionOverview> {
    let RedisValue::Bulk(Some(bytes)) = connection.command(&["INFO"])? else {
        return Ok(ConnectionOverview::default());
    };
    let info = redis_info_text_map(&redis_text_from_bytes(bytes));

    let cpu = (|| {
        let sys = info.get("used_cpu_sys")?.parse::<f64>().ok()?;
        let user = info.get("used_cpu_user")?.parse::<f64>().ok()?;
        let uptime = info.get("uptime_in_seconds")?.parse::<f64>().ok()?;
        Some(fluxdb_core::CpuStats {
            sys_seconds: sys,
            user_seconds: user,
            uptime_seconds: uptime,
        })
    })();

    Ok(ConnectionOverview {
        version: info.get("redis_version").cloned().unwrap_or_default(),
        used_memory_bytes: info
            .get("used_memory")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0),
        cpu,
    })
}

fn redis_list_databases(
    connection_id: ConnectionId,
    connection: &mut RedisConnection,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let mut databases = Vec::new();
    for database in 0..REDIS_DATABASE_COUNT {
        if let Err(error) = redis_select(connection, database) {
            if database == 0 {
                return Err(error);
            }
            break;
        }
        let rows = match connection.command(&["DBSIZE"])? {
            RedisValue::Int(size) if size >= 0 => Some(size as u64),
            _ => None,
        };
        let name = database.to_string();
        databases.push(ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some(name.clone()),
                schema: None,
                name,
                kind: ObjectKind::RedisDb,
            },
            rows,
            modified_at: None,
            comment: None,
        });
    }
    Ok(databases)
}
