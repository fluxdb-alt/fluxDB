//! Redis Cluster「允许从节点读」真实链路集成测试。
//!
//! 需要一套已就绪的 3 主 3 从集群（本仓库验证时由 `scripts` 外的临时环境搭建），
//! 通过环境变量指定一个主节点与它对应的从节点：
//!
//! ```text
//! FLUXDB_REDIS_CLUSTER_MASTER=127.0.0.1:7000
//! FLUXDB_REDIS_CLUSTER_REPLICA=127.0.0.1:7005
//! cargo test --locked -p fluxdb-connectors --test redis_cluster_live -- --ignored --nocapture
//! ```
//!
//! 关键点：普通 GET 在从节点上即使没发 `READONLY` 也会被 `MOVED` 重定向后成功，
//! 所以不能只看「读到了值」。这里用同一条 Workbench 连接执行
//! `CLIENT ID → ROLE → GET → CLIENT ID`：
//! - 关闭只读：GET 触发重定向，连接被替换，前后 CLIENT ID 不同；
//! - 开启只读：连接建立时已发 `READONLY`，GET 本地返回，CLIENT ID 不变且 `ROLE` 为 slave。
//!
//! 两段用例共用同一从节点地址，因此也顺带验证连接池按 `cluster_readonly` 分桶：
//! 若未分桶，第二段会借到第一段重定向后指向主库的连接，`ROLE` 断言会失败。

use fluxdb_connectors::RedisConnector;
use fluxdb_core::{
    CommandExecutionItem, CommandExecutionSource, CommandExecutionStatus, CommandExecutionTarget,
    CommandReply, CommandResultsMode, CommandRunMode, CommandWorkbenchRequest, ConnectionConfig,
    ConnectionId, DatabaseKind, Endpoint, RedisBasicOptions, RedisClusterOptions,
    RedisConnectionProfile, RedisTopology, RedisTopologyKind,
};
use std::collections::BTreeMap;

/// 测试键固定落在 7000 的槽位（1810），7005 是其副本。
const KEY: &str = "fluxdb:cluster:readonly:2";
const VALUE: &str = "readonly-ok";

fn node(raw: &str) -> (String, u16) {
    let (host, port) = raw
        .rsplit_once(':')
        .unwrap_or_else(|| panic!("节点地址应为 host:port，实际为 {raw}"));
    let port = port
        .parse()
        .unwrap_or_else(|_| panic!("节点端口非法：{raw}"));
    (host.to_string(), port)
}

/// 构造走结构化档案的集群配置；`allow_readonly` 即界面上的「允许重定向到从节点」。
fn config(id: u64, address: &str, allow_readonly: bool) -> ConnectionConfig {
    let (host, port) = node(address);
    let profile = RedisConnectionProfile {
        basic: RedisBasicOptions {
            host: host.clone(),
            port,
            ..Default::default()
        },
        topology: RedisTopology {
            kind: RedisTopologyKind::Cluster,
            cluster: RedisClusterOptions {
                start_nodes: vec![address.to_string()],
                allow_readonly,
            },
            ..Default::default()
        },
        ..Default::default()
    };
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("redis-cluster-{id}"),
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

fn integer(item: &CommandExecutionItem) -> i64 {
    match item.reply {
        CommandReply::Integer(value) => value,
        ref other => panic!("`{}` 期望整数回复，实际 {other:?}", item.command),
    }
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

#[test]
#[ignore = "需要真实 Redis Cluster（FLUXDB_REDIS_CLUSTER_MASTER / FLUXDB_REDIS_CLUSTER_REPLICA）"]
fn cluster_readonly_keeps_reads_on_replica() {
    let master = std::env::var("FLUXDB_REDIS_CLUSTER_MASTER")
        .unwrap_or_else(|_| "127.0.0.1:7000".to_string());
    let replica = std::env::var("FLUXDB_REDIS_CLUSTER_REPLICA")
        .unwrap_or_else(|_| "127.0.0.1:7005".to_string());

    // 先写主库并等待至少一个副本确认，避免从节点读到旧值。
    let seeder = RedisConnector::with_config(config(9_901, &master, false));
    let seeded = run(&seeder, &format!("SET {KEY} {VALUE}\nWAIT 1 5000"));
    assert_eq!(
        integer(&seeded[1]),
        1,
        "没有副本确认写入，无法验证从节点读取"
    );

    // 关闭只读：GET 被 MOVED 重定向到主库，连接被替换。
    let plain = RedisConnector::with_config(config(9_902, &replica, false));
    let items = run(&plain, &format!("CLIENT ID\nGET {KEY}\nCLIENT ID"));
    assert_eq!(items.len(), 3);
    let before = integer(&items[0]);
    assert_eq!(bulk(&items[1]), VALUE);
    let after = integer(&items[2]);
    assert_ne!(
        before, after,
        "未开启只读时 GET 应触发重定向并替换连接，CLIENT ID 不应相同"
    );

    // 开启只读：连接建立时发送 READONLY，GET 在副本本地完成，连接保持不变。
    let readonly = RedisConnector::with_config(config(9_903, &replica, true));
    let items = run(&readonly, &format!("CLIENT ID\nROLE\nGET {KEY}\nCLIENT ID"));
    assert_eq!(items.len(), 4);
    let before = integer(&items[0]);
    assert!(
        matches!(array_first(&items[1]), CommandReply::Bulk(bulk) if bulk.text.as_deref() == Some("slave")),
        "开启只读后连接应停留在从节点，ROLE 实际为 {:?}",
        items[1].reply
    );
    assert_eq!(bulk(&items[2]), VALUE);
    let after = integer(&items[3]);
    assert_eq!(
        before, after,
        "开启只读时 GET 不应触发重定向，CLIENT ID 应保持不变"
    );
}

/// 结构化档案回写的键名是 `cluster_readonly`；这里确认档案到扁平参数的转换没有回退。
#[test]
fn profile_writes_canonical_cluster_readonly_key() {
    let resolved = config(9_904, "127.0.0.1:7005", true).redis_resolved();
    assert_eq!(
        resolved.options.get("cluster_readonly").map(String::as_str),
        Some("true")
    );
    assert!(!resolved.options.contains_key("cluster_allow_readonly"));
    assert_eq!(
        resolved.options.get("cluster_enabled").map(String::as_str),
        Some("true")
    );
}
