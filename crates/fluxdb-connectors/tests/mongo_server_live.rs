//! MongoDB 服务器面板真实链路集成测试。
//!
//! 通过环境变量指定一个可写 MongoDB（建议副本集，`$currentOp` / `killOp` 依赖）：
//!
//! ```text
//! FLUXDB_MONGO_SERVER=host:port
//! cargo test --locked -p fluxdb-connectors --test mongo_server_live -- --ignored --nocapture
//! ```
//!
//! 用例只操作自己创建的 `fluxdb_probe` 库，结束时删除。

use fluxdb_connectors::MongoConnector;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MongoServerActivity, MongoTopology,
};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn config(id: u64) -> ConnectionConfig {
    let raw = std::env::var("FLUXDB_MONGO_SERVER").expect("FLUXDB_MONGO_SERVER=host:port");
    let (host, port) = raw.rsplit_once(':').expect("环境变量应为 host:port");
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("mongo-server-{id}"),
        kind: DatabaseKind::MongoDb,
        endpoint: Endpoint::Tcp {
            host: host.to_string(),
            port: port.parse().expect("MongoDB 端口"),
            database: None,
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: None,
        oracle_profile: None,
    }
}

const PROBE_DB: &str = "fluxdb_probe";

/// 准备一条可以阻塞 20 秒的查询：`$where` 里的 `sleep` 在服务端执行。
const SLOW_FIND: &str = r#"{"find":"probe","filter":{"$where":"sleep(20000) || true"}}"#;

fn prepare_probe_database(connector: &MongoConnector) {
    connector
        .run_command(PROBE_DB, r#"{"dropDatabase":1}"#)
        .expect("清理探测库失败");
    connector
        .run_command(
            PROBE_DB,
            r#"{"insert":"probe","documents":[{"_id":1,"value":"x"}]}"#,
        )
        .expect("写入探测文档失败");
}

fn wait_for_probe_operation(
    connector: &MongoConnector,
    deadline: Duration,
) -> (MongoServerActivity, String) {
    let stop = Instant::now() + deadline;
    loop {
        let activity = connector.server_activity().expect("读取服务器状态失败");
        let opid = activity
            .operations
            .iter()
            .find(|operation| operation.namespace.contains(PROBE_DB) && !operation.opid.is_empty())
            .map(|operation| operation.opid.clone());
        if let Some(opid) = opid {
            return (activity, opid);
        }
        assert!(
            Instant::now() < stop,
            "慢查询未出现在 $currentOp 中：{:?}",
            activity.operations
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn wait_until_gone(connector: &MongoConnector, opid: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let activity = connector.server_activity().expect("读取服务器状态失败");
        if !activity
            .operations
            .iter()
            .any(|operation| operation.opid == opid)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "操作 {opid} 终止后仍在 $currentOp 中"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[test]
#[ignore = "需要真实 MongoDB（FLUXDB_MONGO_SERVER=host:port）"]
fn mongo_server_overview_and_kill_operation() {
    let connector = MongoConnector::with_config(config(9_981));
    let activity = connector.server_activity().expect("读取服务器状态失败");
    assert!(!activity.version.is_empty(), "serverStatus 缺少 version");
    assert!(!activity.edition.is_empty(), "serverStatus 缺少发行版信息");
    assert_ne!(activity.topology, MongoTopology::Unknown, "无法判定拓扑");
    assert!(activity.uptime_secs.is_some_and(|uptime| uptime > 0));
    assert!(activity.connections_current >= 1, "至少包含本连接");
    assert!(!activity.metrics.is_empty(), "概览指标为空");
    assert!(
        activity.operations_error.is_none(),
        "$currentOp 失败：{:?}",
        activity.operations_error
    );

    prepare_probe_database(&connector);
    let victim = MongoConnector::with_config(config(9_981));
    let handle = std::thread::spawn(move || {
        // 被 killOp 打断时驱动返回错误，这里只关心它中途退出。
        let _ = victim.run_command(PROBE_DB, SLOW_FIND);
    });

    let (activity, opid) = wait_for_probe_operation(&connector, Duration::from_secs(15));
    let operation = activity
        .operations
        .iter()
        .find(|operation| operation.opid == opid)
        .expect("刚读到的操作不见了");
    assert!(
        operation.is_slow(),
        "sleep 查询应被判定为慢操作：{}s",
        operation.secs_running
    );
    assert_eq!(operation.op, "query");

    connector.kill_operation(&opid).expect("终止操作失败");
    wait_until_gone(&connector, &opid);
    let _ = handle.join();

    connector
        .run_command(PROBE_DB, r#"{"dropDatabase":1}"#)
        .expect("清理探测库失败");
}
