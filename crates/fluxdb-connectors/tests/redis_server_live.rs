//! Redis 服务器面板真实链路集成测试。
//!
//! 通过环境变量指定一个可写的 Redis：
//!
//! ```text
//! FLUXDB_REDIS_SERVER=host:port:password:database
//! cargo test --locked -p fluxdb-connectors --test redis_server_live -- --ignored --nocapture
//! ```
//!
//! 用例只操作自己创建的客户端连接与慢日志配置，结束时恢复 `slowlog-log-slower-than`。

use fluxdb_connectors::RedisConnector;
use fluxdb_core::{
    CommandExecutionItem, CommandExecutionSource, CommandExecutionStatus, CommandExecutionTarget,
    CommandResultsMode, CommandRunMode, CommandWorkbenchRequest, ConnectionConfig, ConnectionId,
    DatabaseKind, Endpoint, RedisBasicOptions, RedisConnectionProfile, SecretRef,
};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn parts() -> (String, u16, String, usize) {
    let raw = std::env::var("FLUXDB_REDIS_SERVER")
        .expect("FLUXDB_REDIS_SERVER=host:port:password:database");
    let fields: Vec<&str> = raw.splitn(4, ':').collect();
    assert_eq!(fields.len(), 4, "环境变量应为 host:port:password:database");
    (
        fields[0].to_string(),
        fields[1].parse().expect("Redis 端口"),
        fields[2].to_string(),
        fields[3].parse().expect("Redis 逻辑库序号"),
    )
}

fn config(id: u64) -> ConnectionConfig {
    let (host, port, password, database) = parts();
    let profile = RedisConnectionProfile {
        basic: RedisBasicOptions {
            host: host.clone(),
            port,
            database: Some(database.to_string()),
            username: None,
            password: SecretRef::inline(password),
        },
        ..Default::default()
    };
    ConnectionConfig {
        id: ConnectionId(id),
        name: format!("redis-server-{id}"),
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

/// 绕过连接池直接开一条 Redis 连接，模拟「另一个客户端」。
///
/// 连接器按 host/port 复用连接池，用第二个连接器取到的是同一条连接，
/// 无法验证 KILL；这里手写最小 RESP 握手。
fn raw_client(host: &str, port: u16, password: &str) -> (TcpStream, String) {
    let mut stream = TcpStream::connect((host, port)).expect("连接 Redis 失败");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("设置读超时失败");
    let mut reader = BufReader::new(stream.try_clone().expect("复制 socket 失败"));
    let command = |args: &[&str]| {
        let mut buf = format!("*{}\r\n", args.len());
        for arg in args {
            buf.push_str(&format!("${}\r\n{}\r\n", arg.len(), arg));
        }
        buf
    };
    if !password.is_empty() {
        stream
            .write_all(command(&["AUTH", password]).as_bytes())
            .expect("发送 AUTH 失败");
        let mut line = String::new();
        reader.read_line(&mut line).expect("读取 AUTH 回复失败");
        assert!(line.starts_with("+OK"), "AUTH 失败：{line}");
    }
    stream
        .write_all(command(&["CLIENT", "ID"]).as_bytes())
        .expect("发送 CLIENT ID 失败");
    let mut line = String::new();
    reader.read_line(&mut line).expect("读取 CLIENT ID 失败");
    let id = line
        .trim()
        .strip_prefix(':')
        .unwrap_or_else(|| panic!("CLIENT ID 回复异常：{line}"))
        .to_string();
    (stream, id)
}

fn wait_until_gone(connector: &RedisConnector, victim_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let activity = connector.server_activity().expect("读取服务器状态失败");
        if !activity.clients.iter().any(|client| client.id == victim_id) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "客户端 {victim_id} 终止后仍在 CLIENT LIST 中"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
#[ignore = "需要真实 Redis（FLUXDB_REDIS_SERVER=host:port:password:database）"]
fn redis_server_overview_clients_and_slowlog() {
    let connector = RedisConnector::with_config(config(9_971));
    let mut activity = connector.server_activity().expect("读取服务器状态失败");
    assert!(!activity.version.is_empty(), "INFO 缺少 redis_version");
    assert!(activity.connected_clients >= 1, "至少包含本连接");
    assert!(
        !activity.clients.is_empty(),
        "CLIENT LIST 应至少返回本连接：{:?}",
        activity.clients_error
    );
    assert!(
        activity.clients.iter().any(|client| client.is_self),
        "CLIENT LIST 应标记面板自身连接"
    );
    assert!(
        activity.slowlog_error.is_none(),
        "SLOWLOG GET 失败：{:?}",
        activity.slowlog_error
    );
    assert!(
        activity.slowlog_log_slower_than_micros.is_some(),
        "CONFIG GET slowlog-log-slower-than 失败"
    );

    // 面板自身连接不允许被终止。
    let self_id = activity.current_client_id.clone();
    assert!(!self_id.is_empty());
    let error = connector.kill_client(&self_id).expect_err("不应允许杀自身");
    assert!(
        error.message.contains("自身"),
        "错误信息应说明原因：{}",
        error.message
    );

    // 另开一条连接作为「外部客户端」，从面板终止它。
    let (host, port, password, _) = parts();
    let (_victim_socket, victim_id) = raw_client(&host, port, &password);
    activity = connector.server_activity().expect("读取服务器状态失败");
    assert!(
        activity.clients.iter().any(|client| client.id == victim_id),
        "外部客户端 {victim_id} 未出现在 CLIENT LIST"
    );
    connector
        .kill_client(&victim_id)
        .expect("终止外部客户端失败");
    wait_until_gone(&connector, &victim_id);

    // 用最低阈值制造慢日志，再验证清空动作。
    let old_threshold = activity.slowlog_log_slower_than_micros.expect("慢日志阈值");
    run(&connector, "CONFIG SET slowlog-log-slower-than 0");
    run(
        &connector,
        "SET fluxdb:slowlog:probe 1\nGET fluxdb:slowlog:probe",
    );
    activity = connector.server_activity().expect("读取服务器状态失败");
    assert!(!activity.slowlog.is_empty(), "阈值为 0 后应产生慢日志记录");

    // 先恢复阈值再清空：阈值为 0 时 SLOWLOG RESET 之后的命令也会立刻入队，
    // 清空后 LEN 不可能为 0。实例原本就是 0（或上次中断没恢复）时用默认值兜底。
    let restore_threshold = if old_threshold <= 0 {
        10_000
    } else {
        old_threshold
    };
    run(
        &connector,
        &format!(
            "CONFIG SET slowlog-log-slower-than {restore_threshold}\nDEL fluxdb:slowlog:probe"
        ),
    );
    connector.clear_slowlog().expect("清空慢日志失败");
    activity = connector.server_activity().expect("读取服务器状态失败");
    assert_eq!(activity.slowlog_len, 0, "清空后 SLOWLOG LEN 应为 0");
    assert!(activity.slowlog.is_empty(), "清空后不应再返回记录");
}
