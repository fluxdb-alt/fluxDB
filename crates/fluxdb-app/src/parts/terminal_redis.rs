// Redis CLI 终端 adapter：把「某个连接的某个库」翻译成语义，供桌面端真实终端（PTY + grid）落地。
//
// 作为 `fluxdb_app` 的 crate-root 子文件被 include。它只实现边界（spawn 参数、命名、状态文案、
// 补全、危险命令识别），不依赖任何 UI，因此可单测，也天然可供未来 MySQL / SSH 复用同一套
// `fluxdb_core::terminal` 边界实现各自的 adapter。

// `BTreeMap` 来自 crate 根作用域（lib.rs 顶部统一导入），本文件随 include! 汇入同名命名空间。

use fluxdb_core::terminal::{
    CompletionApplyEffect, TerminalCommandContext, TerminalCompletionAdapter,
    TerminalCompletionItem, TerminalCompletionKind, TerminalCompletionPriority,
    TerminalSessionAdapter, TerminalSessionKey, TerminalSessionKind, TerminalSessionState,
    TerminalSpawnSpec,
};

/// redis-cli 启动参数缺省矩阵：桌面端在拿到真实布局尺寸前先给一个常见默认，
/// 布局定稿后由 transport.resize 下发真实尺寸。
const DEFAULT_TERM_COLS: u16 = 120;
const DEFAULT_TERM_ROWS: u16 = 32;

/// 危险命令（首期白名单）：执行前应打断 Enter 并请求确认。
const DANGEROUS_REDIS_COMMANDS: &[&str] = &[
    "FLUSHALL",
    "FLUSHDB",
    "SHUTDOWN",
    "DEBUG", // DEBUG JMEET / OBJECT 等可影响服务端稳定性
    "CONFIG",
    "MONITOR",
    "SLAVEOF",
    "REPLICAOF",
    "BGREWRITEAOF", // 重写 AOF 期间可能显著占用
];

/// 每个 Redis CLI 会话独立的 rc 文件序号源：并发打开多个 Redis tab 时各有一个独立路径，
/// 互不覆盖（旧实现按进程 PID 单一文件，两个 tab 会共用、甚至互相覆盖）。
static RC_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 从连接配置取目标数据库（options 里显式的 database 优先，其次 URI 自带不可知——留空）。
fn database_ref(config: &fluxdb_core::ConnectionConfig) -> String {
    config
        .options
        .get("database")
        .filter(|v| !v.is_empty())
        .cloned()
        .unwrap_or_default()
}

/// Redis CLI 会话 adapter：持有已解析凭据的连接配置 + 目标库，负责生成 redis-cli 启动参数。
pub struct RedisCliAdapter {
    config: fluxdb_core::ConnectionConfig,
    database: u32,
    /// 本会话独立的 rc 文件路径（None = 未写入成功，回退到不让 redis-cli 读任何 rc）。
    rc_path: Option<std::path::PathBuf>,
    /// 只读连接的本地 RESP 代理；存在时 redis-cli 只连本机代理，写入在代理层拒绝。
    read_only_proxy: Option<ReadOnlyRedisProxy>,
    /// 只读代理启动失败的原因（非空时终端直接显示错误并退出，避免无护栏放行）。
    read_only_proxy_error: Option<String>,
}

impl RedisCliAdapter {
    pub fn new(config: fluxdb_core::ConnectionConfig, database: u32) -> Self {
        // 只读判定要在 `redis_resolved()` 之前做：档案归一化只关心拨号参数。
        let read_only = connection_is_read_only(&config);
        // 解析结构化档案（TLS/用户名/URI 等）为扁平 endpoint + options，
        // 让 spawn/prompt/meta 读到的都是最终拨号视角，避免 redis-cli 与直连参数不一致。
        let config = config.redis_resolved();
        // 会话结束（tab 关闭 / 组件 drop）时由 Drop 清理本会话独立 rc 文件。
        let rc_path = Self::write_unique_rc_file(&config);
        let (read_only_proxy, read_only_proxy_error) = if read_only {
            match ReadOnlyRedisProxy::start(config.clone(), database) {
                Ok(proxy) => (Some(proxy), None),
                Err(err) => (None, Some(format!("只读代理启动失败：{err}"))),
            }
        } else {
            (None, None)
        };
        Self {
            config,
            database,
            rc_path,
            read_only_proxy,
            read_only_proxy_error,
        }
    }

    /// 从连接配置里的 options 取任一步长密钥配置，值字符串原样返回。
    fn option(&self, key: &str) -> Option<String> {
        self.config
            .options
            .get(key)
            .cloned()
            .filter(|v| !v.is_empty())
    }

    /// redis-cli 默认会启用自己的在线参数提示（块状光标会落在提示文本上）。
    /// 用**每个会话独立**的 rc 文件关闭它，提示由桌面端渲染层统一绘制，避免两个光标/提示叠加。
    /// 文件名带（PID + connection_id + 会话序号 + 数据库号），并发打开多个 tab 互不覆盖。
    fn write_unique_rc_file(
        config: &fluxdb_core::ConnectionConfig,
    ) -> Option<std::path::PathBuf> {
        let seq = RC_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "fluxdb-redis-cli-{}-{}-{}-{}.redisclirc",
            std::process::id(),
            config.id.0,
            database_ref(config),
            seq
        ));
        match std::fs::write(&path, ":set nohints\n") {
            Ok(()) => Some(path),
            Err(err) => {
                // 只记录，不因 rc 写入失败阻塞终端；回退为不使用 rc。
                tracing::warn!(path = %path.display(), err = %err, "redis-cli rc 文件写入失败，将不使用 rc");
                None
            }
        }
    }
}

impl Drop for RedisCliAdapter {
    fn drop(&mut self) {
        // 会话结束（tab 关闭 / 组件销毁）时清理本会话独立的 rc 临时文件，避免残留。
        if let Some(path) = self.rc_path.take() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

impl TerminalSessionAdapter for RedisCliAdapter {
    fn kind(&self) -> TerminalSessionKind {
        TerminalSessionKind::RedisCli
    }

    fn session_key(&self) -> TerminalSessionKey {
        TerminalSessionKey::Redis {
            connection_id: self.config.id.0,
            database: self.database,
        }
    }

    fn title(&self) -> String {
        format!("Redis CLI - DB {}", self.database)
    }

    fn status_text(&self, state: TerminalSessionState) -> String {
        match state {
            TerminalSessionState::Connecting => "连接中…".to_string(),
            TerminalSessionState::PtyRunning => "进程已启动".to_string(),
            TerminalSessionState::Ready => format!("Redis DB {} · 就绪", self.database),
            TerminalSessionState::Busy => "执行中…".to_string(),
            TerminalSessionState::AwaitingConfirmation => "等待确认".to_string(),
            TerminalSessionState::Exited => "已退出".to_string(),
            TerminalSessionState::Failed => "启动失败".to_string(),
        }
    }

    /// 生成 redis-cli 启动参数。密码经 `REDISCLI_AUTH` 环境变量注入（不进入 argv，避免
    /// 出现在进程参数 / 日志里；见 TERM-007），并追加 `--no-auth-warning` 抑制明文告警
    /// （`--no-color` redis-cli 不识别，已去掉）。
    ///
    /// URI / Tcp 两种 endpoint 统一在末尾一次性生成完整参数，**不提前返回**：URI 分支也要
    /// 叠加下方解析出的用户名 / 密码 / TLS / 数据库等配置，避免 `-u` 早退丢失这些参数（TERM-006）。
    fn spawn(&self) -> TerminalSpawnSpec {
        // rc 路径写进环境（REDISCLI_RCFILE）；无 rc（写入失败）时给出空环境，不阻塞。
        let mut env: Vec<(String, String)> = Vec::new();
        if let Some(path) = &self.rc_path {
            env.push(("REDISCLI_RCFILE".to_string(), path.to_string_lossy().into_owned()));
        }

        // 只读连接：redis-cli 只连本机代理，凭据由代理持有；写入命令在代理层被拒绝。
        if let Some(proxy) = &self.read_only_proxy {
            env.push(("REDISCLI_AUTH".to_string(), proxy.token.clone()));
            return TerminalSpawnSpec {
                program: "redis-cli".to_string(),
                args: vec![
                    "-h".to_string(),
                    "127.0.0.1".to_string(),
                    "-p".to_string(),
                    proxy.port.to_string(),
                    "-n".to_string(),
                    self.database.to_string(),
                    "--no-auth-warning".to_string(),
                ],
                env,
                cwd: None,
                cols: DEFAULT_TERM_COLS,
                rows: DEFAULT_TERM_ROWS,
            };
        }
        if let Some(message) = &self.read_only_proxy_error {
            // 代理没起来时不能回退成直连（会绕过只读），直接提示并退出。
            let message = message.replace('\'', "'\\''");
            return TerminalSpawnSpec {
                program: "sh".to_string(),
                args: vec![
                    "-c".to_string(),
                    format!("printf '%s\\n' '{message}' >&2; exit 1"),
                ],
                env,
                cwd: None,
                cols: DEFAULT_TERM_COLS,
                rows: DEFAULT_TERM_ROWS,
            };
        }

        // 密码绝不进 argv：优先注入环境变量 REDISCLI_AUTH。
        if let Some(password) = self.option("password") {
            env.push(("REDISCLI_AUTH".to_string(), password));
        }

        let mut args: Vec<String> = Vec::new();
        match &self.config.endpoint {
            fluxdb_core::Endpoint::Tcp { host, port, .. } => {
                args.push("-h".to_string());
                args.push(host.clone());
                args.push("-p".to_string());
                args.push(port.to_string());
            }
            fluxdb_core::Endpoint::Uri { uri } => {
                // URI 内联 host/port/库/凭据，作为基础；仍需叠加下方独立配置的 username/tls/库。
                // redis-cli `-u <uri>` 会将 URI 里自带的凭据作为事实来源；独立配置只做补充。
                args.push("-u".to_string());
                args.push(uri.clone());
            }
            _ => {
                args.push("-h".to_string());
                args.push("127.0.0.1".to_string());
                args.push("-p".to_string());
                args.push("6379".to_string());
            }
        }
        if let Some(user) = self.option("username") {
            args.push("--user".to_string());
            args.push(user);
        }
        if self.option("tls").is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "true" | "y" | "yes" | "1" | "on"
            )
        }) {
            args.push("--tls".to_string());
        }
        if self.option("tls_insecure").is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "true" | "y" | "yes" | "1" | "on"
            )
        }) {
            args.push("--insecure".to_string());
        }
        args.push("-n".to_string());
        args.push(self.database.to_string());
        // 只保留 redis-cli 真正支持的参数。
        // `--no-auth-warning` 合法；`--no-color` redis-cli 不识别，会直接报 “Unrecognized option”
        // 并以退出码 1 结束会话（导致终端打开即报错、无法输入），故这里必须去掉。
        args.push("--no-auth-warning".to_string());

        TerminalSpawnSpec {
            program: "redis-cli".to_string(),
            args,
            env,
            cwd: None,
            cols: DEFAULT_TERM_COLS,
            rows: DEFAULT_TERM_ROWS,
        }
    }

    fn on_output(&mut self, bytes: &[u8], state: &mut TerminalSessionState) {
        // 输出回流：若本次增量以 redis prompt 收尾，说明命令已执行完、重新回到可输入态；
        // 否则保持当前态（Busy 时输出持续回流，不回退到 Ready）。
        //
        // 不硬编码 Redis 之外的细节：prompt 由本 adapter 的 `prompt()` 提供，core 只消费结果。
        let output = String::from_utf8_lossy(bytes);
        let trailing = output
            .rsplit('\n')
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim_end().to_string())
            .unwrap_or_default();
        let prompt = self.prompt().trim_end().to_string();
        if !prompt.is_empty() && trailing.ends_with(&prompt) {
            *state = TerminalSessionState::Ready;
        } else if matches!(*state, TerminalSessionState::PtyRunning) {
            // 尚无 prompt 可识别（启动阶段）但已有输出时，视为准备就绪。
            *state = TerminalSessionState::Ready;
        }
    }

    fn on_exit(&mut self, status: fluxdb_core::terminal::TerminalExitStatus, state: &mut TerminalSessionState) {
        // 真实退出结果：正常（redis-cli 里 `quit`/正常结束）→ Exited；
        // 非零或信号（连接失败 / 参数·TLS·认证错误 / 被杀）→ Failed，状态栏据此提示。
        // 与设计文档 TERM-005 一致：不再固定调用 on_exit(0)。
        if status.is_success() {
            *state = TerminalSessionState::Exited;
        } else {
            *state = TerminalSessionState::Failed;
            // 不打印密码 / secret：这里只记录退出码与信号，供状态栏展示。
            tracing::warn!(
                code = ?status.code,
                signal = ?status.signal,
                "redis-cli 会话非正常退出"
            );
        }
    }

    /// Redis CLI 专属的 Ctrl+C 策略：把 Ctrl+C 当作「会话控制键」，永不因它退出会话。
    ///
    /// redis-cli 在提示符处（没有命令在执行）收到 `\x03`（SIGINT）会直接退出交互会话，
    /// 而不是“取消”。因此：
    /// - 处于 Busy（命令正在执行/阻塞）时，才发送 `\x03` 去中断当前操作——这正是 redis-cli
    ///   “中断当前命令、回到可继续输入”的语义，中断后新 prompt 回流，`on_output` 会把状态拉回 Ready。
    /// - 其余时刻（提示符下输入中 / 等待危险命令确认 / 刚就绪），改发 `\x15`（Ctrl+U）：redis-cli
    ///   的 linenoise 行编辑器用它清空当前输入行并回画一个新提示，既不退出进程、又能“清空输入”。
    fn ctrl_c_bytes(&self, state: TerminalSessionState) -> &'static [u8] {
        if matches!(state, TerminalSessionState::Busy) {
            b"\x03"
        } else {
            b"\x15"
        }
    }

    /// redis-cli 提示符：`host:port[db]> `，供补全上下文 / transcript 使用。
    fn prompt(&self) -> String {
        let (host, port) = match &self.config.endpoint {
            fluxdb_core::Endpoint::Tcp { host, port, .. } => (host.clone(), *port),
            _ => ("127.0.0.1".to_string(), 6379u16),
        };
        if self.database == 0 {
            format!("{host}:{port}> ")
        } else {
            format!("{host}:{port}[{}]> ", self.database)
        }
    }

    fn display_prompt(&self) -> String {
        self.prompt()
    }

    /// 会话元信息：把连接 / 库 / 主机等喂给补全上下文与状态展示。
    fn meta(&self) -> BTreeMap<String, String> {
        let mut meta = BTreeMap::new();
        meta.insert("connection_id".to_string(), self.config.id.0.to_string());
        meta.insert("database".to_string(), self.database.to_string());
        if let fluxdb_core::Endpoint::Tcp { host, port, .. } = &self.config.endpoint {
            meta.insert("host".to_string(), host.clone());
            meta.insert("port".to_string(), port.to_string());
        }
        if !self.config.name.is_empty() {
            meta.insert("profile".to_string(), self.config.name.clone());
        }
        meta
    }
}

/// 只读连接的本地 RESP 代理：redis-cli 只连本机代理，代理按只读白名单逐条校验，
/// 通过现有 Redis 连接器执行；写入命令直接回 `-READONLY`，不触达服务端。
///
/// 只监听 `127.0.0.1` 并要求随机令牌认证（经 `REDISCLI_AUTH` 注入），
/// 避免同机其他进程借代理读取数据。
struct ReadOnlyRedisProxy {
    port: u16,
    token: String,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ReadOnlyRedisProxy {
    fn start(
        config: fluxdb_core::ConnectionConfig,
        database: u32,
    ) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let token = format!(
            "{:x}{:x}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        );
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = std::sync::Arc::clone(&shutdown);
        let thread_token = token.clone();
        let handle = std::thread::Builder::new()
            .name("redis-readonly-proxy".to_string())
            .spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let connector =
                        fluxdb_connectors::RedisConnector::with_config(config.clone());
                    let token = thread_token.clone();
                    // 一个 CLI 会话对应一个客户端连接，交给独立线程处理。
                    std::thread::spawn(move || {
                        serve_read_only_client(stream, connector, database, token)
                    });
                }
            })?;
        Ok(Self {
            port,
            token,
            shutdown,
            handle: Some(handle),
        })
    }
}

impl Drop for ReadOnlyRedisProxy {
    fn drop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // 唤醒阻塞中的 accept，让代理线程退出。
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// 处理一个 redis-cli 连接：逐条解析 RESP 命令，按只读策略放行或拒绝。
fn serve_read_only_client(
    stream: std::net::TcpStream,
    connector: fluxdb_connectors::RedisConnector,
    mut database: u32,
    token: String,
) {
    let Ok(read_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = std::io::BufReader::new(read_stream);
    let mut writer = stream;
    // redis-cli 通过 REDISCLI_AUTH 先发 AUTH；未认证前只允许 AUTH。
    let mut authenticated = false;
    while let Some(argv) = read_resp_command(&mut reader) {
        if argv.is_empty() {
            continue;
        }
        let command = argv[0].to_ascii_lowercase();
        if command == "auth" {
            let supplied = argv.last().cloned().unwrap_or_default();
            authenticated = supplied == token;
            let reply = if authenticated {
                CommandReply::Status("OK".to_string())
            } else {
                CommandReply::Error("WRONGPASS invalid token".to_string())
            };
            if write_reply(&mut writer, &reply).is_err() {
                break;
            }
            continue;
        }
        if !authenticated {
            let reply = CommandReply::Error("NOAUTH Authentication required.".to_string());
            if write_reply(&mut writer, &reply).is_err() {
                break;
            }
            continue;
        }
        let reply = match command.as_str() {
            "quit" => {
                let _ = write_reply(&mut writer, &CommandReply::Status("OK".to_string()));
                break;
            }
            // redis-cli 连接初始化时可能发 CLIENT SETINFO/SETNAME，直接确认；
            // 其他 CLIENT 子命令仍按只读策略拒绝。
            "client" => {
                let subcommand = argv
                    .get(1)
                    .map(|value| value.to_ascii_lowercase())
                    .unwrap_or_default();
                if matches!(subcommand.as_str(), "setinfo" | "setname") {
                    CommandReply::Status("OK".to_string())
                } else {
                    CommandReply::Error("READONLY 连接已设为只读，未执行".to_string())
                }
            }
            "select" => match argv.get(1).and_then(|value| value.parse::<u32>().ok()) {
                Some(db) => {
                    database = db;
                    CommandReply::Status("OK".to_string())
                }
                None => CommandReply::Error("ERR invalid DB index".to_string()),
            },
            _ if !redis_argv_allowed(&argv) => {
                CommandReply::Error("READONLY 连接已设为只读，未执行".to_string())
            }
            _ => match connector.execute_argv(database, &argv) {
                Ok(reply) => reply,
                Err(error) => CommandReply::Error(format!("ERR {}", error.message)),
            },
        };
        if write_reply(&mut writer, &reply).is_err() {
            break;
        }
    }
}

/// 读一条 RESP 命令：支持 `*N` 多段批量与 inline 文本两种形式。
fn read_resp_command(reader: &mut impl std::io::BufRead) -> Option<Vec<String>> {
    let mut line = String::new();
    if std::io::BufRead::read_line(reader, &mut line).ok()? == 0 {
        return None;
    }
    let line = line.trim_end_matches(['\r', '\n']);
    let Some(count) = line.strip_prefix('*') else {
        return Some(line.split_whitespace().map(str::to_string).collect());
    };
    let count = count.parse::<usize>().ok()?;
    let mut argv = Vec::with_capacity(count);
    for _ in 0..count {
        let mut header = String::new();
        if std::io::BufRead::read_line(reader, &mut header).ok()? == 0 {
            return None;
        }
        let header = header.trim_end_matches(['\r', '\n']);
        let length = header.strip_prefix('$')?.parse::<usize>().ok()?;
        let mut buffer = vec![0u8; length + 2];
        std::io::Read::read_exact(reader, &mut buffer).ok()?;
        buffer.truncate(length);
        argv.push(String::from_utf8_lossy(&buffer).into_owned());
    }
    Some(argv)
}

/// 把连接器回包写成 RESP2，供 redis-cli 渲染。
fn write_reply(writer: &mut impl std::io::Write, reply: &CommandReply) -> std::io::Result<()> {
    match reply {
        CommandReply::Nil => writer.write_all(b"$-1\r\n"),
        CommandReply::Status(text) => {
            let text = text.replace(['\r', '\n'], " ");
            writer.write_all(format!("+{text}\r\n").as_bytes())
        }
        CommandReply::Integer(value) => writer.write_all(format!(":{value}\r\n").as_bytes()),
        CommandReply::Float(value) => write_bulk(writer, value.to_string().as_bytes()),
        CommandReply::Bulk(bulk) => match (bulk.text.as_deref(), bulk.bytes_preview_hex.as_deref()) {
            (Some(text), _) => write_bulk(writer, text.as_bytes()),
            (None, Some(hex)) => write_bulk(writer, hex.as_bytes()),
            (None, None) => write_bulk(writer, b""),
        },
        CommandReply::Array(items) => {
            writer.write_all(format!("*{}\r\n", items.len()).as_bytes())?;
            for item in items {
                write_reply(writer, item)?;
            }
            Ok(())
        }
        CommandReply::Map(pairs) => {
            writer.write_all(format!("*{}\r\n", pairs.len() * 2).as_bytes())?;
            for (key, value) in pairs {
                write_reply(writer, key)?;
                write_reply(writer, value)?;
            }
            Ok(())
        }
        CommandReply::Error(message) => {
            let message = message.replace(['\r', '\n'], " ");
            writer.write_all(format!("-{message}\r\n").as_bytes())
        }
        CommandReply::Text(text) => write_bulk(writer, text.as_bytes()),
    }
}

fn write_bulk(writer: &mut impl std::io::Write, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(format!("${}\r\n", bytes.len()).as_bytes())?;
    writer.write_all(bytes)?;
    writer.write_all(b"\r\n")
}

/// Redis CLI 补全 / 危险命令 adapter：复用 fluxdb-app 既有的 redis 命令补全能力。
pub struct RedisCliCompletion;

impl RedisCliCompletion {
    fn map_kind(kind: fluxdb_core::QueryCompletionKind) -> TerminalCompletionKind {
        use fluxdb_core::QueryCompletionKind::*;
        match kind {
            RedisCommand => TerminalCompletionKind::Command,
            RedisSubCommand => TerminalCompletionKind::Subcommand,
            RedisArgument | Column => TerminalCompletionKind::Argument,
            Function | Procedure => TerminalCompletionKind::Function,
            Keyword | Snippet | Schema | Table | View | Trigger => TerminalCompletionKind::Keyword,
        }
    }

    fn priority(kind: TerminalCompletionKind) -> TerminalCompletionPriority {
        match kind {
            TerminalCompletionKind::Command => TerminalCompletionPriority::High,
            TerminalCompletionKind::Subcommand => TerminalCompletionPriority::Normal,
            _ => TerminalCompletionPriority::Normal,
        }
    }

    fn is_dangerous_impl(input: &str) -> bool {
        let first_word = input
            .trim_start()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches([';'])
            .to_ascii_uppercase();
        DANGEROUS_REDIS_COMMANDS
            .iter()
            .any(|dangerous| dangerous == &first_word)
    }
}

impl TerminalCompletionAdapter for RedisCliCompletion {
    fn supports_completion(&self) -> bool {
        true
    }

    fn complete(&self, ctx: &TerminalCommandContext) -> Vec<TerminalCompletionItem> {
        let result = redis_completion_result(&ctx.input, ctx.cursor);
        result
            .items
            .into_iter()
            .map(|item| {
                let kind = Self::map_kind(item.kind);
                let priority = Self::priority(kind);
                TerminalCompletionItem {
                    label: item.label,
                    insert_text: item.insert_text,
                    replace_start: result.replace_start,
                    replace_end: result.replace_end,
                    detail: item.detail,
                    kind,
                    priority,
                }
            })
            .collect()
    }

    fn apply(&self, item: &TerminalCompletionItem) -> CompletionApplyEffect {
        if item.insert_text.is_empty() {
            return CompletionApplyEffect::PassThrough;
        }
        CompletionApplyEffect::Replace {
            start: item.replace_start,
            end: item.replace_end,
            text: item.insert_text.clone(),
        }
    }

    fn is_dangerous(&self, input: &str) -> bool {
        Self::is_dangerous_impl(input)
    }
}

#[cfg(test)]
mod read_only_proxy_tests {
    use super::*;

    #[test]
    fn resp_command_parses_multibulk_and_inline() {
        let mut reader = std::io::BufReader::new(&b"*2\r\n$3\r\nGET\r\n$1\r\nk\r\nPING\r\n"[..]);
        assert_eq!(
            read_resp_command(&mut reader),
            Some(vec!["GET".to_string(), "k".to_string()])
        );
        assert_eq!(read_resp_command(&mut reader), Some(vec!["PING".to_string()]));
        assert_eq!(read_resp_command(&mut reader), None);
    }

    #[test]
    fn read_only_classification_reuses_workbench_policy() {
        assert!(redis_argv_allowed(&["get".to_string(), "k".to_string()]));
        assert!(redis_argv_allowed(&["PING".to_string()]));
        assert!(!redis_argv_allowed(&[
            "set".to_string(),
            "k".to_string(),
            "v".to_string()
        ]));
        assert!(!redis_argv_allowed(&["del".to_string(), "k".to_string()]));
    }

    #[test]
    fn reply_serialization_is_resp2() {
        let mut output = Vec::new();
        write_reply(&mut output, &CommandReply::Status("OK".to_string())).unwrap();
        write_reply(&mut output, &CommandReply::Integer(7)).unwrap();
        write_reply(&mut output, &CommandReply::Nil).unwrap();
        write_reply(&mut output, &CommandReply::Error("ERR boom".to_string())).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "+OK\r\n:7\r\n$-1\r\n-ERR boom\r\n"
        );
    }
}
