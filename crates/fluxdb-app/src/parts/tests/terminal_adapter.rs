// Redis CLI 终端 adapter 纯逻辑单测（无 UI / 无真实 PTY）。
// 覆盖：session_key、prompt、meta、spawn 参数、on_output 状态机过渡。

use std::collections::BTreeMap;

use fluxdb_core::{ConnectionConfig, ConnectionId, DatabaseKind, Endpoint};

use fluxdb_core::terminal::{TerminalSessionAdapter, TerminalSessionKey, TerminalSessionState};

/// 构造一个指向 127.0.0.1:6379 的 Redis 连接配置（不含密码，避免测试里出现明文密钥）。
fn redis_config(id: u64) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(id),
        name: "local-redis".to_string(),
        kind: DatabaseKind::Redis,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".to_string(),
            port: 6379,
            database: Some("0".to_string()),
        },
        credential_ref: None,
        options: std::collections::BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
    }
}

#[test]
fn redis_adapter_session_key_is_connection_plus_database() {
    let adapter = RedisCliAdapter::new(redis_config(7), 3);
    assert_eq!(
        adapter.session_key(),
        TerminalSessionKey::Redis {
            connection_id: 7,
            database: 3
        }
    );
}

#[test]
fn redis_adapter_prompt_and_meta_follow_config() {
    let adapter = RedisCliAdapter::new(redis_config(7), 2);
    assert_eq!(adapter.prompt(), "127.0.0.1:6379[2]> ");
    assert_eq!(adapter.display_prompt(), "127.0.0.1:6379[2]> ");

    let meta = adapter.meta();
    assert_eq!(meta.get("connection_id").map(String::as_str), Some("7"));
    assert_eq!(meta.get("database").map(String::as_str), Some("2"));
    assert_eq!(meta.get("host").map(String::as_str), Some("127.0.0.1"));
    assert_eq!(meta.get("port").map(String::as_str), Some("6379"));
    assert_eq!(meta.get("profile").map(String::as_str), Some("local-redis"));
}

#[test]
fn redis_adapter_display_prompt_matches_native_prompt() {
    let adapter = RedisCliAdapter::new(redis_config(7), 0);
    assert_eq!(adapter.prompt(), "127.0.0.1:6379> ");
    assert_eq!(adapter.display_prompt(), "127.0.0.1:6379> ");
}

#[test]
fn redis_adapter_spawn_contains_host_port_database() {
    let adapter = RedisCliAdapter::new(redis_config(7), 4);
    let spec = adapter.spawn();
    assert_eq!(spec.program, "redis-cli");
    assert!(spec.args.iter().any(|a| a == "-h"));
    assert!(spec.args.iter().any(|a| a == "127.0.0.1"));
    assert!(spec.args.iter().any(|a| a == "-p"));
    assert!(spec.args.iter().any(|a| a == "6379"));
    assert!(spec.args.iter().any(|a| a == "-n"));
    assert!(spec.args.iter().any(|a| a == "4"));
}

#[test]
fn redis_adapter_on_output_prompt_sets_ready() {
    let mut adapter = RedisCliAdapter::new(redis_config(7), 0);
    let prompt = adapter.prompt();

    // 命令执行结束：输出以 prompt 收尾 → Ready。
    let mut state = TerminalSessionState::Busy;
    let out = format!("OK\n{prompt}");
    adapter.on_output(out.as_bytes(), &mut state);
    assert_eq!(state, TerminalSessionState::Ready);

    // 输出中没有 prompt（命令仍在执行）→ 保持当前态（不回退）。
    let mut state = TerminalSessionState::Busy;
    adapter.on_output(b"SCAN\r\n", &mut state);
    assert_eq!(state, TerminalSessionState::Busy);
}

#[test]
fn redis_adapter_ctrl_c_clears_line_at_prompt_but_sigint_when_busy() {
    let adapter = RedisCliAdapter::new(redis_config(7), 0);
    // redis-cli 在提示符下（Ready / 输入中 / 等待确认）Ctrl+C 应清空当前行而非退出，
    // 下发 \x15 (Ctrl+U)；因此不会发送会退出 redis-cli 的 \x03。
    for state in [
        TerminalSessionState::Ready,
        TerminalSessionState::PtyRunning,
        TerminalSessionState::AwaitingConfirmation,
    ] {
        assert_eq!(adapter.ctrl_c_bytes(state), b"\x15", "state {state:?}");
    }
    // 命令真正执行（Busy）时才发送 \x03 去中断，且仍不关闭会话（由 on_output 拉回 Ready）。
    assert_eq!(adapter.ctrl_c_bytes(TerminalSessionState::Busy), b"\x03");
}

#[test]
fn redis_adapter_status_text_covers_all_states() {
    let adapter = RedisCliAdapter::new(redis_config(7), 1);
    for state in [
        TerminalSessionState::Connecting,
        TerminalSessionState::PtyRunning,
        TerminalSessionState::Ready,
        TerminalSessionState::Busy,
        TerminalSessionState::AwaitingConfirmation,
        TerminalSessionState::Exited,
        TerminalSessionState::Failed,
    ] {
        let text = adapter.status_text(state);
        assert!(!text.is_empty(), "状态 {state:?} 应有非空文案");
    }
}

/// 构造一个带指定 options 的 Redis 连接配置（供 URI / 凭据 / TLS 场景）。
fn redis_config_with_options(id: u64, endpoint: Endpoint, options: BTreeMap<String, String>) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(id),
        name: "local-redis".to_string(),
        kind: DatabaseKind::Redis,
        endpoint,
        credential_ref: None,
        options,
        redis_profile: None,
        mysql_profile: None,
    }
}

#[test]
fn redis_adapter_uri_spawn_keeps_username_tls_database() {
    // TERM-006：旧实现的 URI 分支会提前返回，把下方解析出的 username/tls/database 全部丢掉。
    // 现在 `-u <uri>` 只是参数基础，username/tls/`-n` 仍会叠加（不早退）。
    let mut opts = BTreeMap::new();
    opts.insert("username".to_string(), "admin".to_string());
    opts.insert("tls".to_string(), "true".to_string());
    opts.insert("tls_insecure".to_string(), "true".to_string());
    let adapter = RedisCliAdapter::new(
        redis_config_with_options(
            7,
            Endpoint::Uri {
                uri: "redis://cache.example.com:16379/2".to_string(),
            },
            opts,
        ),
        3,
    );
    let spec = adapter.spawn();
    // URI 作为基础端点。
    assert!(spec.args.iter().any(|a| a == "-u"));
    assert!(spec.args.iter().any(|a| a == "redis://cache.example.com:16379/2"));
    // 叠加的独立配置不再丢失。
    assert!(spec.args.iter().any(|a| a == "--user"), "username 参数丢失");
    assert!(spec.args.iter().any(|a| a == "admin"));
    assert!(spec.args.iter().any(|a| a == "--tls"), "tls 参数丢失");
    assert!(spec.args.iter().any(|a| a == "--insecure"), "insecure 参数丢失");
    // 数据库与连接配置里的 database 分开：这里用 adapter 的 database 字段。
    assert!(spec.args.iter().any(|a| a == "-n"));
    assert!(spec.args.iter().any(|a| a == "3"));
}

#[test]
fn redis_adapter_password_goes_to_env_not_argv() {
    // TERM-007：密码经 REDISCLI_AUTH 注入，绝不进入 argv（进程参数 / 日志可见列）。
    let mut opts = BTreeMap::new();
    opts.insert("password".to_string(), "s3cr3t-password-凭证".to_string());
    let adapter = RedisCliAdapter::new(redis_config_with_options(7, redis_config(7).endpoint, opts), 0);
    let spec = adapter.spawn();
    // argv 里绝不能出现密码本体。
    assert!(!spec.args.iter().any(|a| a.contains("s3cr3t")), "密码泄漏进 argv");
    // 密码只出现在环境变量 REDISCLI_AUTH 中。
    let auth = spec
        .env
        .iter()
        .find(|(k, _)| k == "REDISCLI_AUTH")
        .map(|(_, v)| v.clone());
    assert_eq!(auth.as_deref(), Some("s3cr3t-password-凭证"));
}

#[test]
fn redis_adapter_rc_file_cleaned_up_on_drop() {
    // TERM-007：每个会话独立 rc 文件，Drop 时清理，不残留 / 不跨会话覆盖。
    let adapter = RedisCliAdapter::new(redis_config(7), 0);
    let rc_path = adapter.rc_path.clone().expect("应有 rc 文件写入成功");
    assert!(rc_path.exists(), "rc 文件应已写出");
    drop(adapter);
    assert!(!rc_path.exists(), "Drop 后 rc 文件应被清理");
}

#[test]
fn redis_adapter_rc_file_isolation_between_concurrent_tabs() {
    // TERM-007：并发打开多个 Redis tab，各自的 rc 路径互不相同（文件名带连接/序号差异）。
    let a = RedisCliAdapter::new(redis_config(1), 0);
    let b = RedisCliAdapter::new(redis_config(1), 0);
    let pa = a.rc_path.clone().expect("a 应有 rc");
    let pb = b.rc_path.clone().expect("b 应有 rc");
    assert_ne!(pa, pb, "两个并发会话不得共用同一 rc 文件");
    assert!(pa.exists());
    assert!(pb.exists());
}

#[test]
fn redis_adapter_exit_status_maps_to_state() {
    // TERM-005：真实退出码/信号决定状态，不再固定 on_exit(0)。
    let mut adapter = RedisCliAdapter::new(redis_config(7), 0);
    use fluxdb_core::terminal::TerminalExitStatus;

    let mut state = TerminalSessionState::Busy;
    adapter.on_exit(TerminalExitStatus { code: Some(0), signal: None }, &mut state);
    assert_eq!(state, TerminalSessionState::Exited);

    let mut state = TerminalSessionState::Busy;
    // 连接失败 / 参数错误 → 非零退出码。
    adapter.on_exit(TerminalExitStatus { code: Some(1), signal: None }, &mut state);
    assert_eq!(state, TerminalSessionState::Failed);

    let mut state = TerminalSessionState::Busy;
    // 被信号终止。
    adapter.on_exit(TerminalExitStatus { code: None, signal: Some("SIGKILL".to_string()) }, &mut state);
    assert_eq!(state, TerminalSessionState::Failed);
}
