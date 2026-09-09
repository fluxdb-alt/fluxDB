// 通用终端模拟器核心的「模型与边界」。
//
// 本模块只描述“终端像什么”，不描述“Redis/MySQL/SSH 怎么执行”：
//   - 会话种类 / 状态 / 复用 key / transcript 条目 / 补全项 / adapter trait，全部是纯数据 + trait，
//     不依赖任何 UI 或连接器细节，因此可以被 Redis CLI / MySQL CLI / SSH 等多后端复用。
//   - grid / parser 作为子文件，随本模块 `include!` 汇入 `fluxdb_core::terminal` 命名空间。
pub mod terminal {
    use std::collections::BTreeMap;

    /// 会话种类：终端 core 只识别种类，不做各自语义。
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum TerminalSessionKind {
        RedisCli,
        MySqlCli,
        SshShell,
        GenericShell,
    }

    /// 会话状态：显式状态机，避免用一组布尔值表达。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TerminalSessionState {
        /// 正在启动 PTY / 连接。
        Connecting,
        /// PTY 进程已起来，等待就绪。
        PtyRunning,
        /// 已就绪，可输入。
        Ready,
        /// 执行 / 忙碌中。
        Busy,
        /// 等待危险命令确认。
        AwaitingConfirmation,
        /// 进程已退出。
        Exited,
        /// 启动或运行失败。
        Failed,
    }

    /// 会话复用 key：供 app 层按规则复用 tab，而不是 UI 临时判断。
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub enum TerminalSessionKey {
        Redis { connection_id: u64, database: u32 },
        MySql { connection_id: u64, database: Option<String>, schema: Option<String> },
        Ssh { host: String, port: u16, user: String, profile: String },
        GenericShell { profile_id: String, working_dir: String },
    }

    /// transcript 条目种类（设计文档要求区分 system/prompt/command/output/error/notice/blank）。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TerminalTranscriptKind {
        System,
        Prompt,
        Command,
        Output,
        Error,
        Notice,
        Blank,
    }

    /// 终端 transcript 条目。首期作为“按行渲染 + 滚动回看”的辅助视图，
    /// 由 grid（真实 cell 缓冲）派生，二者解耦；后续可替换为更重的富文档，不影响 core 边界。
    #[derive(Clone, Debug, PartialEq)]
    pub struct TerminalTranscriptEntry {
        pub kind: TerminalTranscriptKind,
        pub text: String,
    }

    /// 补全项种类。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TerminalCompletionKind {
        Keyword,
        Command,
        Subcommand,
        Argument,
        Key,
        Path,
        Function,
    }

    /// 补全优先级。
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub enum TerminalCompletionPriority {
        Low,
        Normal,
        High,
    }

    /// 补全候选：设计文档统一字段（label / insert_text / replace_range / detail / kind / priority）。
    #[derive(Clone, Debug, PartialEq)]
    pub struct TerminalCompletionItem {
        pub label: String,
        pub insert_text: String,
        /// 替换区间（字节偏移），供替换当前 token。
        pub replace_start: usize,
        pub replace_end: usize,
        pub detail: Option<String>,
        pub kind: TerminalCompletionKind,
        pub priority: TerminalCompletionPriority,
    }

    /// adapter 进行补全 / 提示时需要拿到的上下文。
    #[derive(Clone, Debug)]
    pub struct TerminalCommandContext {
        /// 当前（用户已键入的）输入行文本。
        pub input: String,
        /// 光标字节偏移。
        pub cursor: usize,
        /// 当前 prompt 文本。
        pub prompt: String,
        pub kind: TerminalSessionKind,
        /// 连接 / 数据库 / 主机等元信息（key → 值）。
        pub meta: BTreeMap<String, String>,
    }

    /// 补全落地的效果：adapter 决定“直接插入 / 替换 token / 透传 Tab / 只提示”。
    #[derive(Clone, Debug, PartialEq)]
    pub enum CompletionApplyEffect {
        Insert { text: String },
        Replace { start: usize, end: usize, text: String },
        PassThrough,
        ShowHint { message: String },
    }

    /// transport 启动参数：由 session adapter 生成，terminal core 不解释其语义。
    #[derive(Clone, Debug)]
    pub struct TerminalSpawnSpec {
        pub program: String,
        pub args: Vec<String>,
        /// (key, value) 环境变量。
        pub env: Vec<(String, String)>,
        pub cwd: Option<std::path::PathBuf>,
        pub cols: u16,
        pub rows: u16,
    }

    /// PTY 子进程的退出结果：供 adapter 区分「正常退出 / 参数·TLS·认证失败 / 被信号终止」。
    ///
    /// - `code == Some(0)`：正常退出（如用户键入 `quit`）。
    /// - `code == Some(nonzero)`：进程以非零码退出，多为连接失败 / 参数错误。
    /// - `signal.is_some()`：被信号终止（如被杀）。
    /// - 两者皆 None：读取退出状态失败，无法判定（按失败兜底，但不臆造成功）。
    #[derive(Clone, Debug, Default)]
    pub struct TerminalExitStatus {
        pub code: Option<u32>,
        pub signal: Option<String>,
    }

    impl TerminalExitStatus {
        /// 是否成功退出（无信号且退出码为 0；无法判定时视为失败）。
        pub fn is_success(&self) -> bool {
            self.signal.is_none() && self.code == Some(0)
        }
    }

    /// Transport 只管字节流，不懂 Redis / MySQL / SSH。
    pub trait TerminalTransport {
        fn write(&mut self, data: &[u8]);
        fn resize(&mut self, cols: u16, rows: u16);
        fn close(&mut self);
    }

    /// 会话 adapter：负责“启动什么 / 怎么命名 / 输出如何解释 / 退出如何收尾”。
    pub trait TerminalSessionAdapter {
        fn kind(&self) -> TerminalSessionKind;
        fn session_key(&self) -> TerminalSessionKey;
        fn title(&self) -> String;
        /// 根据会话状态给出 header/footer 状态文案。
        fn status_text(&self, state: TerminalSessionState) -> String;
        fn spawn(&self) -> TerminalSpawnSpec;
        fn on_output(&mut self, bytes: &[u8], state: &mut TerminalSessionState);
        /// 子进程退出回调：接收真实退出结果（非固定 0），由 adapter 决定进入 Exited 或 Failed，
        /// 供状态栏展示可操作的失败原因。默认实现按成功与否切换状态。
        fn on_exit(&mut self, status: TerminalExitStatus, state: &mut TerminalSessionState) {
            *state = if status.is_success() {
                TerminalSessionState::Exited
            } else {
                TerminalSessionState::Failed
            };
        }

        /// 当前 prompt 文本：供补全上下文 / transcript / 状态栏表达。
        /// 由 adapter 解析，core 不写死成 `> `。
        fn prompt(&self) -> String {
            "> ".to_string()
        }

        /// 用户界面展示的提示符。默认沿用会话原生 prompt；需要隐藏后端连接地址、
        /// 改成产品统一样式的会话可以覆盖它（例如 Redis CLI 的 `[dbN] > `）。
        fn display_prompt(&self) -> String {
            self.prompt()
        }

        /// 会话元信息（key → 值），供补全上下文与状态展示使用。
        /// 至少应包含 connection / database / host / profile 等。
        fn meta(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }

        /// Ctrl+C 的会话级控制键策略：adapter 决定按下 Ctrl+C 时下发到 PTY 的字节。
        ///
        /// 这是「可定制的会话控制键」扩展点，把 Ctrl+C 从内核的一刀切行为解耦出来：
        /// 返回的字节由组件层原样写入 PTY；返回空切片表示「不下发、不关闭会话」，
        /// 仅由组件层做本地收尾（如取消危险命令确认）。
        ///
        /// 默认返回 `\x03`（SIGINT），保持多数终端（shell / MySQL/SSH CLI）的既有行为；
        /// 需要「Ctrl+C 不退出会话」的后端（如 redis-cli，其在提示符处收到 `\x03` 会直接退出）
        /// 覆盖本方法并依据 `state` 决定是否 / 下发何种字节（见 fluxdb-app 的 RedisCliAdapter）。
        /// 会话状态是否可区分“提示符下输入中”与“命令执行中”，由 adapter 自行消费。
        fn ctrl_c_bytes(&self, _state: TerminalSessionState) -> &'static [u8] {
            b"\x03"
        }
    }

    /// completion adapter：补全是 adapter 能力，不写死在 core。
    pub trait TerminalCompletionAdapter {
        fn supports_completion(&self) -> bool;
        fn complete(&self, ctx: &TerminalCommandContext) -> Vec<TerminalCompletionItem>;
        fn apply(&self, item: &TerminalCompletionItem) -> CompletionApplyEffect;
        /// 输入行是否属于危险命令（adapter 决定是否打断 Enter 并要求确认）。
        fn is_dangerous(&self, input: &str) -> bool;
    }

    /// 内存版 transport（测试 / SSH 预留）：不拉起真实 PTY，
    /// 记录写入的字节与 resize/close 调用，供 core/desktop 单测与后续 SSH 纯透传复用。
    #[derive(Clone, Debug, Default)]
    pub struct MockTransport {
        pub written: Vec<u8>,
        pub resizes: Vec<(u16, u16)>,
        pub closed: bool,
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self::default()
        }

        /// 已写入的字节转成 UTF-8 文本（测试断言用）。
        pub fn written_text(&self) -> String {
            String::from_utf8_lossy(&self.written).into_owned()
        }
    }

    impl TerminalTransport for MockTransport {
        fn write(&mut self, data: &[u8]) {
            self.written.extend_from_slice(data);
        }
        fn resize(&mut self, cols: u16, rows: u16) {
            self.resizes.push((cols, rows));
        }
        fn close(&mut self) {
            self.closed = true;
        }
    }

    /// 把一行输出分类为 transcript 条目种类：语义日志，不等同于 cell 网格。
    ///
    /// 规则（由 adapter 传入 prompt 做解析，core 不写死）：
    /// - 空行/纯空白 → `Blank`
    /// - 以 prompt 结尾 → `Prompt`（若该行还有命令前缀则归为 `Output`，避免误判；见下）
    /// - 以已知错误标记开头 → `Error`
    /// - 常量为 `Output`
    ///
    /// 首期不做 token 级纠结，只按整行判定，供 transcript 记录与后续回看面板使用。
    pub fn classify_transcript_line(line: &str, prompt: &str) -> TerminalTranscriptKind {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            return TerminalTranscriptKind::Blank;
        }
        let p = prompt.trim_end();
        if !p.is_empty() && trimmed.ends_with(p) && trimmed.trim() == p.trim() {
            return TerminalTranscriptKind::Prompt;
        }
        let head = trimmed.trim_start();
        if head.starts_with("(error)")
            || head.starts_with("ERR ")
            || head.starts_with("WRONGTYPE ")
            || head.starts_with("NOAUTH ")
            || head.starts_with("DENIED ")
        {
            return TerminalTranscriptKind::Error;
        }
        TerminalTranscriptKind::Output
    }

    // 真实 cell 网格（缓冲 / 光标 / 滚动 / selection / resize）。
    include!("terminal/grid.rs");
    // ANSI / VT 解析：把 PTY 字节流喂进 grid。
    include!("terminal/parser.rs");
}
