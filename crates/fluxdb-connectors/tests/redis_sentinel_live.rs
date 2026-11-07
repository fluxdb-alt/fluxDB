//! Redis Sentinel 真实链路集成测试。
//!
//! 需要一套可用的哨兵环境（主从 + 至少 1 个哨兵），通过环境变量指定：
//!
//! ```text
//! FLUXDB_REDIS_SENTINEL_ENDPOINTS=127.0.0.1:26379,127.0.0.1:26380
//! FLUXDB_REDIS_SENTINEL_MASTER=fluxmaster
//! cargo test --locked -p fluxdb-connectors --test redis_sentinel_live -- --ignored --nocapture
//! ```
//!
//! 覆盖两点：
//! 1. 节点列表里第一个哨兵不可达时，仍能回退到后续哨兵解析出主库；
//! 2. 手动 failover 后，新连接跟随哨兵切到新主库（`ROLE` 为 master）。
//!
//! 环境可以从任意位置经 SSH 端口转发提供，哨兵返回的主库地址必须是测试进程可达的。

use fluxdb_connectors::RedisConnector;
use fluxdb_core::{
    CommandExecutionItem, CommandExecutionSource, CommandExecutionStatus, CommandExecutionTarget,
    CommandReply, CommandResultsMode, CommandRunMode, CommandWorkbenchRequest, ConnectionConfig,
    ConnectionId, DatabaseKind, Endpoint, RedisBasicOptions, RedisClusterOptions,
    RedisConnectionProfile, RedisSentinelOptions, RedisTopology, RedisTopologyKind,
};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// 不存在的哨兵端口，用于验证列表回退。
const DEAD_SENTINEL: &str = "127.0.0.1:26399";

fn sentinels() -> Vec<String> {
    std::env::var("FLUXDB_REDIS_SENTINEL_ENDPOINTS")
        .unwrap_or_else(|_| "127.0.0.1:26379,127.0.0.1:26380".to_string())
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn master_name() -> String {
    std::env::var("FLUXDB_REDIS_SENTINEL_MASTER").unwrap_or_else(|_| "fluxmaster".to_string())
}

fn node(raw: &str) -> (String, u16) {
    let (host, port) = raw
        .rsplit_once(':')
        .unwrap_or_else(|| panic!("节点地址应为 host:port，实际为 {raw}"));
    let port = port
        .parse()
        .unwrap_or_else(|_| panic!("节点端口非法：{raw}"));
    (host.to_string(), port)
}

/// 构造哨兵模式配置；`endpoints` 决定候选哨兵列表。
fn config(id: u64, endpoints: Vec<String>) -> ConnectionConfig {
    let (host, port) = endpoints
        .first()
        .map(|item| node(item))
        .unwrap_or_else(|| ("127.0.0.1".to_string(), 26379));
    let profile = RedisConnectionProfile {
        basic: RedisBasicOptions {
            host: host.clone(),
            port,
            ..Default::default()
        },
        topology: RedisTopology {
            kind: RedisTopologyKind::Sentinel,
            sentinel: RedisSentinelOptions {
                master_name: master_name(),
                endpoints,
                ..Default::default()
            },
            cluster: RedisClusterOptions::default(),
            ..Default::default()
        },
        ..Default::default()
    };
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("redis-sentinel-{id}"),
        kind: DatabaseKind::Redis,
        endpoint: Endpoint::Tcp {
            host,
            port,
            database: None,
        },
        options: BTreeMap::new(),
        redis_profile: Some(profile),
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
}

fn run(connector: &RedisConnector, text: &str) -> Vec<CommandExecutionItem> {
    let execution = connector
        .execute_command_workbench(&CommandWorkbenchRequest {
            target: CommandExecutionTarget::Redis {
                connection_id: ConnectionId(0),
                database: 0,
            },
            text: text.to_string(),
            run_mode: CommandRunMode::Text,
            results_mode: CommandResultsMode::Default,
            batch_size: 1,
            continue_on_error: false,
            source: CommandExecutionSource::Workbench,
        })
        .expect("Workbench 执行失败");
    for item in &execution.commands {
        assert_eq!(
            item.status,
            CommandExecutionStatus::Success,
            "命令 `{}` 执行失败：{:?}",
            item.command,
            item.reply
        );
    }
    execution.commands
}

fn bulk(item: &CommandExecutionItem) -> String {
    match &item.reply {
        CommandReply::Bulk(bulk) => bulk
            .text
            .clone()
            .unwrap_or_else(|| panic!("`{}` 的 bulk 不是 UTF-8 文本", item.command)),
        other => panic!("`{}` 期望 bulk 回复，实际 {other:?}", item.command),
    }
}

fn array_first(item: &CommandExecutionItem) -> &CommandReply {
    match &item.reply {
        CommandReply::Array(items) => items
            .first()
            .unwrap_or_else(|| panic!("`{}` 返回空数组", item.command)),
        other => panic!("`{}` 期望数组回复，实际 {other:?}", item.command),
    }
}

/// 直接向哨兵发一条命令并读回字符串数组（避免依赖 redis-cli）。
fn sentinel_command(endpoint: &str, args: &[&str]) -> std::io::Result<Vec<String>> {
    let (host, port) = node(endpoint);
    let mut stream = TcpStream::connect((host.as_str(), port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = format!("*{}\r\n", args.len());
    for arg in args {
        request.push_str(&format!("${}\r\n{}\r\n", arg.len(), arg));
    }
    stream.write_all(request.as_bytes())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    match line.chars().next() {
        Some('*') => {
            let count: usize = line[1..].trim().parse().unwrap_or(0);
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                let mut head = String::new();
                reader.read_line(&mut head)?;
                if let Some(len) = head.strip_prefix('$') {
                    let len: usize = len.trim().parse().unwrap_or(0);
                    let mut value = vec![0u8; len];
                    reader.read_exact(&mut value)?;
                    let mut crlf = [0u8; 2];
                    reader.read_exact(&mut crlf)?;
                    values.push(String::from_utf8_lossy(&value).into_owned());
                } else {
                    values.push(head.trim().to_string());
                }
            }
            Ok(values)
        }
        _ => Ok(vec![line.trim().to_string()]),
    }
}

fn current_master(endpoint: &str) -> Option<String> {
    let args = ["SENTINEL", "get-master-addr-by-name", &master_name()];
    let reply = sentinel_command(endpoint, &args).ok()?;
    match reply.as_slice() {
        [host, port] => Some(format!("{host}:{port}")),
        _ => None,
    }
}

/// 第一个哨兵不可达时，必须回退到后续哨兵并正常读写。
#[test]
#[ignore = "需要真实 Redis Sentinel（FLUXDB_REDIS_SENTINEL_ENDPOINTS / FLUXDB_REDIS_SENTINEL_MASTER）"]
fn sentinel_falls_back_to_next_node() {
    let mut endpoints = vec![DEAD_SENTINEL.to_string()];
    endpoints.extend(sentinels());

    let key = "fluxdb:sentinel:fallback";
    let connector = RedisConnector::with_config(config(9_911, endpoints));
    let items = run(&connector, &format!("SET {key} fallback-ok\nGET {key}"));
    assert_eq!(bulk(&items[1]), "fallback-ok");
}

/// 手动 failover 后，新连接应跟随哨兵切到新主库。
#[test]
#[ignore = "需要真实 Redis Sentinel（FLUXDB_REDIS_SENTINEL_ENDPOINTS / FLUXDB_REDIS_SENTINEL_MASTER）"]
fn sentinel_follows_failover() {
    let endpoints = sentinels();
    let first = endpoints.first().expect("至少一个哨兵地址");
    let before = current_master(first).expect("读取当前主库地址");
    let key = "fluxdb:sentinel:failover";

    // 先写入当前主库，确保切换后仍能读到。
    let old = RedisConnector::with_config(config(9_912, endpoints.clone()));
    let items = run(&old, &format!("SET {key} failover-ok"));
    assert_eq!(items.len(), 1);

    let reply = sentinel_command(first, &["SENTINEL", "FAILOVER", &master_name()])
        .expect("发送 SENTINEL FAILOVER");
    assert!(
        reply
            .first()
            .is_some_and(|line| line.starts_with("+OK") || line == "OK"),
        "SENTINEL FAILOVER 未被接受：{reply:?}"
    );

    // 等哨兵公布新主库；最多 30 秒。
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut after = before.clone();
    while Instant::now() < deadline {
        after = current_master(first).unwrap_or_else(|| before.clone());
        if after != before {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert_ne!(after, before, "failover 后哨兵仍未切换主库");

    // 反向列表顺序制造独立连接池分桶，避免复用 failover 前的旧连接。
    let mut reversed = endpoints.clone();
    reversed.reverse();
    let connector = RedisConnector::with_config(config(9_913, reversed));
    let items = run(&connector, &format!("ROLE\nGET {key}"));
    assert!(
        matches!(array_first(&items[0]), CommandReply::Bulk(bulk) if bulk.text.as_deref() == Some("master")),
        "新连接应落在新主库，ROLE 实际为 {:?}",
        items[0].reply
    );
    assert_eq!(bulk(&items[1]), "failover-ok");
}
