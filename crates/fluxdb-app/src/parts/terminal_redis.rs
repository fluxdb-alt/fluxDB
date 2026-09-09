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
}

impl RedisCliAdapter {
    pub fn new(config: fluxdb_core::ConnectionConfig, database: u32) -> Self {
        // 解析结构化档案（TLS/用户名/URI 等）为扁平 endpoint + options，
        // 让 spawn/prompt/meta 读到的都是最终拨号视角，避免 redis-cli 与直连参数不一致。
        let config = config.redis_resolved();
        // 会话结束（tab 关闭 / 组件 drop）时由 Drop 清理本会话独立 rc 文件。
        let rc_path = Self::write_unique_rc_file(&config);
        Self { config, database, rc_path }
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
