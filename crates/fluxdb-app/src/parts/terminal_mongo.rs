// MongoDB 命令行终端：库内执行 shell 语句（`use` / `show dbs` / `db.<集合>.<方法>(...)`）的 REPL。
//
// mongosh 不一定安装在本机，且它需要真实 PTY 与会话协议；这里改为「自有行编辑 + 连接器执行」：
//   - `MongoReplTransport`：一个后台线程接收按键字节、维护输入行与历史、把语句交给连接器执行，
//     并把回显 / 结果 / prompt 作为终端字节流回传，交给桌面端既有 TermGrid 渲染。
//   - `MongoCliAdapter`：会话命名 / prompt / 状态文案（供终端 surface 展示）。
//   - `MongoCliCompletion`：库名与集合名 / 方法名补全（Tab）。
//
// 边界：不经过 PTY，密码不会出现在任何子进程 argv；语句文本与结果只在本地终端内呈现。
// 语法与只读范围由连接器的 `mongo_shell` 决定，这里只负责行编辑与回显。

// 本文件随 `include!` 汇入 crate 根作用域，与 `terminal_redis.rs` 共享命名空间；
// 为避免 `use` 重名冲突，统一用模块别名限定终端核心类型。
use fluxdb_core::terminal as mterm;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

const MONGO_TERM_COLS: u16 = 120;
const MONGO_TERM_ROWS: u16 = 32;

/// 顶层语句补全候选：与连接器 shell 解析器支持的关键字保持一致。
const MONGO_SHELL_KEYWORDS: &[&str] = &[
    "use ",
    "show dbs",
    "show collections",
    "db.",
    "db.getCollectionNames()",
];

/// 集合方法补全候选（`db.<集合>.` 之后）。
const MONGO_SHELL_METHODS: &[&str] = &[
    "find",
    "findOne",
    "count",
    "countDocuments",
    "estimatedDocumentCount",
    "distinct",
    "aggregate",
    "getIndexes",
    "stats",
];

/// REPL 与 adapter 共享的会话状态。
///
/// 当前库由 `use` 切换，它同时决定 prompt —— 终端渲染拿 adapter 的 prompt 换算光标列，
/// 两者必须永远一致，所以放在同一份共享状态里，而不是各存一份。
#[derive(Clone, Default)]
pub struct MongoCliSession {
    database: std::sync::Arc<std::sync::Mutex<String>>,
    /// 当前库的集合名（补全用）；`use` 切换后重新读取。
    collections: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// 所有库名（`use <库>` 补全用）；每个会话读一次。
    databases: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// 后台列表刷新状态：CLI 与命令工作台共用，避免每次补全都重新建连。
    refresh: std::sync::Arc<std::sync::Mutex<MongoListRefresh>>,
}

/// 补全列表的刷新记账。
#[derive(Default)]
struct MongoListRefresh {
    /// 库名是否已经请求过（失败也不重试，避免每按键一次重连）。
    databases_requested: bool,
    /// 集合名最近请求过的库；换库才重新请求。
    collections_for: Option<String>,
}

impl MongoCliSession {
    pub fn new(database: String) -> Self {
        Self {
            database: std::sync::Arc::new(std::sync::Mutex::new(database)),
            collections: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            databases: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            refresh: std::sync::Arc::new(std::sync::Mutex::new(MongoListRefresh::default())),
        }
    }

    pub fn database(&self) -> String {
        self.database
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default()
    }

    pub fn set_database(&self, database: String) {
        if let Ok(mut current) = self.database.lock() {
            *current = database;
        }
    }

    pub fn collections(&self) -> Vec<String> {
        self.collections
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default()
    }

    fn set_collections(&self, names: Vec<String>) {
        if let Ok(mut current) = self.collections.lock() {
            *current = names;
        }
    }

    pub fn databases(&self) -> Vec<String> {
        self.databases
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default()
    }

    fn set_databases(&self, names: Vec<String>) {
        if let Ok(mut current) = self.databases.lock() {
            *current = names;
        }
    }

    /// 终端提示符：`<库名>> `，与 Compass / mongosh 的当前库提示一致。
    fn prompt(&self) -> String {
        format!("{}> ", self.database())
    }

    /// 后台刷新补全用的库名与当前库集合名（CLI 与命令工作台共用）。
    ///
    /// 读取列表要建连，可能慢到超时，所以永远不阻塞调用方：本次先用旧候选，
    /// 结果到了再由下一次补全看到。库名每个会话只请求一次；集合名换库才重读，
    /// 失败也不重试——补全少几条候选远好过每次按键都重连。
    pub fn request_refresh(&self, config: &fluxdb_core::ConnectionConfig) {
        self.refresh_databases(config);
        self.refresh_collections(config);
    }

    /// 后台读一次全部库名（`use <库>` 补全用）。
    fn refresh_databases(&self, config: &fluxdb_core::ConnectionConfig) {
        {
            let Ok(mut state) = self.refresh.lock() else {
                return;
            };
            if state.databases_requested {
                return;
            }
            state.databases_requested = true;
        }
        let connection_id = config.id;
        let config = config.clone();
        let session = self.clone();
        let spawned = std::thread::Builder::new()
            .name("fluxdb-mongo-cli-databases".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                if let Ok(objects) = fluxdb_connectors::connector_for(&config)
                    .and_then(|connector| connector.list_objects(None))
                {
                    session
                        .set_databases(objects.into_iter().map(|object| object.path.name).collect());
                }
            });
        if spawned.is_err() {
            tracing::debug!(
                connection_id = connection_id.0,
                "MongoDB 无法启动库名读取线程，use 补全退化为无候选"
            );
        }
    }

    /// 后台重读当前库的集合名（补全用）。
    fn refresh_collections(&self, config: &fluxdb_core::ConnectionConfig) {
        let database = self.database();
        if database.trim().is_empty() {
            return;
        }
        {
            let Ok(mut state) = self.refresh.lock() else {
                return;
            };
            if state.collections_for.as_deref() == Some(database.as_str()) {
                return;
            }
            state.collections_for = Some(database.clone());
        }
        let connection_id = config.id;
        let config = config.clone();
        let session = self.clone();
        let requested_database = database.clone();
        let spawned = std::thread::Builder::new()
            .name("fluxdb-mongo-cli-collections".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let objects = fluxdb_connectors::connector_for(&config).and_then(|connector| {
                    connector.list_objects(Some(&fluxdb_core::ObjectPath {
                        connection_id: config.id,
                        database: Some(requested_database.clone()),
                        schema: None,
                        name: String::new(),
                        kind: fluxdb_core::ObjectKind::Database,
                    }))
                });
                // 读取期间又切了库：这份结果属于旧库，丢掉，别覆盖新库的候选。
                if let Ok(objects) = objects
                    && session.database() == requested_database
                {
                    session.set_collections(
                        objects.into_iter().map(|object| object.path.name).collect(),
                    );
                }
            });
        if spawned.is_err() {
            tracing::debug!(
                connection_id = connection_id.0,
                database,
                "MongoDB 无法启动集合名读取线程，集合补全退化为无候选"
            );
        }
    }
}

/// MongoDB REPL 传输：UI 线程按键字节 → 后台线程 → 输出字节流。
pub struct MongoReplTransport {
    input: Sender<Vec<u8>>,
    output: Receiver<Vec<u8>>,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl MongoReplTransport {
    /// 启动 REPL 线程。`session` 与 adapter 共享当前库与集合列表。
    pub fn new(config: fluxdb_core::ConnectionConfig, session: MongoCliSession) -> Self {
        let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (output_tx, output_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let worker_alive = alive.clone();
        let connection_id = config.id;
        let worker = std::thread::Builder::new()
            .name("fluxdb-mongo-cli".to_string())
            // 连接器在放大栈的线程里跑驱动，但 REPL 自己也会压栈（JSON 解析 + 排版），
            // 与其它 mongo 调用一致给 8 MiB，避免 debug 构建下擦到保护页。
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut repl = MongoRepl::new(config, session);
                repl.greet(&output_tx);
                while worker_alive.load(std::sync::atomic::Ordering::Relaxed) {
                    let bytes = match input_rx.recv() {
                        Ok(bytes) => bytes,
                        Err(_) => break,
                    };
                    repl.feed(&bytes, &output_tx);
                }
                tracing::debug!(connection_id = connection_id.0, "MongoDB 命令行线程退出");
            })
            .ok();
        Self {
            input: input_tx,
            output: output_rx,
            alive,
            worker,
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.input.send(bytes.to_vec());
    }

    /// 非阻塞取一段输出；无输出返回 `None`。
    pub fn try_recv(&self) -> Option<Vec<u8>> {
        match self.output.try_recv() {
            Ok(bytes) => Some(bytes),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    pub fn close(&mut self) {
        self.alive.store(false, std::sync::atomic::Ordering::Relaxed);
        // 关闭输入通道让阻塞的 recv 立即返回；工作线程随后自然退出。
        let (dead_tx, _dead_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let _ = std::mem::replace(&mut self.input, dead_tx);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for MongoReplTransport {
    fn drop(&mut self) {
        self.alive.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// REPL 本体：输入行、历史与连接器执行。
struct MongoRepl {
    config: fluxdb_core::ConnectionConfig,
    session: MongoCliSession,
    line: String,
    /// 光标在 `line` 内的字节偏移，永远落在字符边界上（左 / 右方向键与中途插入靠它）。
    cursor: usize,
    /// 历史命令（旧 → 新）；`history_cursor` 为回看位置（`len` = 当前输入行）。
    history: Vec<String>,
    history_cursor: usize,
    /// 跨 chunk 未完整的 UTF-8 尾部。
    pending: Vec<u8>,
    /// 转义序列中间态（方向键等）。
    escape: Vec<u8>,
}

impl MongoRepl {
    fn new(config: fluxdb_core::ConnectionConfig, session: MongoCliSession) -> Self {
        Self {
            config,
            session,
            line: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_cursor: 0,
            pending: Vec::new(),
            escape: Vec::new(),
        }
    }

    fn prompt(&self) -> String {
        self.session.prompt()
    }

    /// 会话开场：只给提示符，不打用途说明。
    ///
    /// 顶部信息条已经写明连接 / 库，再刷一段 banner 只是把真正的第一行输出往下挤。
    fn greet(&mut self, out: &Sender<Vec<u8>>) {
        let prompt = self.prompt();
        let _ = out.send(prompt.into_bytes());
        // 补全列表只影响补全，晚一点到没关系；放到后台读，别让首次提示符等建连。
        self.session.request_refresh(&self.config);
    }

    /// 处理一段按键字节。
    fn feed(&mut self, bytes: &[u8], out: &Sender<Vec<u8>>) {
        let mut echo: Vec<u8> = Vec::new();
        let mut commands: Vec<String> = Vec::new();
        for byte in bytes {
            match byte {
                b'\r' | b'\n' => {
                    if self.escape.is_empty() {
                        echo.extend_from_slice(b"\r\n");
                        commands.push(std::mem::take(&mut self.line));
                        self.cursor = 0;
                    } else {
                        self.escape.clear();
                    }
                }
                // 退格：删光标左侧整个字符。光标不在行尾时整行重画，否则只抹掉那几个单元。
                0x7f | 0x08 => {
                    if !self.escape.is_empty() {
                        self.escape.clear();
                    } else if let Some(start) = self.char_before(self.cursor) {
                        let width = self.line[start..self.cursor]
                            .chars()
                            .map(|ch| fluxdb_core::terminal::char_cell_width(ch).max(1) as usize)
                            .sum::<usize>();
                        self.line.replace_range(start..self.cursor, "");
                        self.cursor = start;
                        if self.cursor == self.line.len() {
                            for _ in 0..width {
                                echo.extend_from_slice(b"\x08 \x08");
                            }
                        } else {
                            self.redraw(&mut echo);
                        }
                    }
                }
                // Ctrl+C：放弃当前输入并另起一行（与 node/mongosh REPL 一致，`\r\n` 后重画提示符）。
                // Ctrl+U：就地清空当前行，不换行。
                0x03 | 0x15 => {
                    self.escape.clear();
                    self.line.clear();
                    self.cursor = 0;
                    if *byte == 0x03 {
                        // 只换行，**不擦除**被放弃的输入：node/mongosh REPL 实测就是 `\r\r\n` + 重画提示符，
                        // 旧行保留可见。加了 `\x1b[K` 会把整行擦成空白，用户看到的就是「按完什么都没了」。
                        echo.extend_from_slice(b"\r\n");
                    } else {
                        echo.extend_from_slice(b"\r");
                    }
                    echo.extend_from_slice(self.prompt().as_bytes());
                    echo.extend_from_slice(b"\x1b[K");
                }
                // Ctrl+L：清屏并重画当前行。
                0x0c => {
                    self.escape.clear();
                    echo.extend_from_slice(b"\x1b[2J\x1b[H");
                    echo.extend_from_slice(self.prompt().as_bytes());
                    echo.extend_from_slice(self.line.as_bytes());
                    // 光标回到输入行内的原位置（重画后光标停在行尾）。
                    let back: usize = self.line[self.cursor..]
                        .chars()
                        .map(|ch| fluxdb_core::terminal::char_cell_width(ch).max(1) as usize)
                        .sum();
                    self.move_cursor_left(&mut echo, back);
                }
                // Ctrl+D：结束会话（与 shell 一致）。
                0x04 => {
                    echo.extend_from_slice(b"\r\n");
                    let _ = out.send(echo);
                    let _ = out.send(Vec::new());
                    return;
                }
                // Tab：补全走桌面端的补全浮层（`MongoCliCompletion`），这里不重复实现。
                b'\t' => {}
                0x1b => {
                    self.escape.clear();
                    self.escape.push(*byte);
                }
                _ if !self.escape.is_empty() => {
                    self.escape.push(*byte);
                    // CSI 序列以 0x40..=0x7e 的字节结束（`ESC[A`、`ESC[3~` 都是），
                    // 按终止字节收尾才能等到 4 字节的 `ESC[3~`；固定长度会把后续按键一起吞掉。
                    // 至少 `ESC[` + 1 字节才算完整：`[` 自己也在 0x40..=0x7e 区间里。
                    let complete = self.escape.len() >= 3
                        && self
                            .escape
                            .last()
                            .is_some_and(|byte| matches!(byte, 0x40..=0x7e));
                    if complete {
                        let sequence = std::mem::take(&mut self.escape);
                        match sequence.as_slice() {
                            b"\x1b[A" => self.history_previous(&mut echo),
                            b"\x1b[B" => self.history_next(&mut echo),
                            b"\x1b[C" => self.cursor_right(&mut echo),
                            b"\x1b[D" => self.cursor_left(&mut echo),
                            b"\x1b[H" => self.cursor_home(&mut echo),
                            b"\x1b[F" => self.cursor_end(&mut echo),
                            b"\x1b[3~" => self.delete_at_cursor(&mut echo),
                            _ => {}
                        }
                    } else if !self.escape.starts_with(b"\x1b[") || self.escape.len() > 8 {
                        self.escape.clear();
                    }
                }
                _ => {
                    self.pending.push(*byte);
                    // 只有完整 UTF-8 前缀才消费；不完整尾巴留到下一段。
                    let valid = match std::str::from_utf8(&self.pending) {
                        Ok(text) => Some(text.to_string()),
                        Err(error) if error.error_len().is_none() => None,
                        Err(_) => {
                            // 非法字节（非 UTF-8）：丢弃，避免污染输入行。
                            self.pending.clear();
                            None
                        }
                    };
                    if let Some(text) = valid {
                        self.pending.clear();
                        for ch in text.chars() {
                            if ch.is_control() {
                                continue;
                            }
                            self.line.insert(self.cursor, ch);
                            self.cursor += ch.len_utf8();
                            let mut buffer = [0u8; 4];
                            echo.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
                            // 光标不在行尾：后面的内容要重新画一遍，并把光标退回插入点之后。
                            if self.cursor < self.line.len() {
                                let rest = self.line[self.cursor..].to_string();
                                echo.extend_from_slice(rest.as_bytes());
                                echo.extend_from_slice(b"\x1b[K");
                                let back: usize = rest
                                    .chars()
                                    .map(|c| fluxdb_core::terminal::char_cell_width(c).max(1) as usize)
                                    .sum();
                                self.move_cursor_left(&mut echo, back);
                            }
                        }
                    } else if self.pending.len() > 4 {
                        self.pending.clear();
                    }
                }
            }
        }
        if !echo.is_empty() {
            let _ = out.send(echo);
        }
        for command in commands {
            let rendered = self.execute(&command);
            let _ = out.send(rendered);
        }
    }

    /// 执行一条语句并返回终端要回显的字节（结果 / 错误 + 新 prompt）。
    fn execute(&mut self, command: &str) -> Vec<u8> {
        let command = command.trim().to_string();
        let mut out = String::new();
        if command.is_empty() {
            out.push_str(&self.prompt());
            return out.into_bytes();
        }
        if command.eq_ignore_ascii_case("exit") || command.eq_ignore_ascii_case("quit") {
            out.push_str("MongoDB shell exited.\r\n");
            return out.into_bytes();
        }
        if self.history.last() != Some(&command) {
            self.history.push(command.clone());
        }
        self.history_cursor = self.history.len();
        let database = self.session.database();
        match fluxdb_connectors::connector_for(&self.config).and_then(|connector| {
            connector.mongo_shell(self.config.id, &database, &command)
        }) {
            Ok(result) => {
                if let Some(database) = result.database {
                    // `use` 只切换本会话的库：prompt 与补全列表跟着换，连接配置不动。
                    self.session.set_database(database);
                    self.session.request_refresh(&self.config);
                }
                if !result.output.is_empty() {
                    // 连接器给的排版文本用 `\n`；终端里必须补 `\r`，否则下一行会接着上一行的列。
                    for line in result.output.trim_end_matches('\n').split('\n') {
                        out.push_str(line);
                        out.push_str("\r\n");
                    }
                }
            }
            Err(error) => {
                // 错误按行回显：多行提示（用法说明）在终端里逐行换行才对齐。
                for line in error.message.lines() {
                    out.push_str("\x1b[31m");
                    out.push_str(line.trim_end());
                    out.push_str("\x1b[0m\r\n");
                }
            }
        }
        out.push_str(&self.prompt());
        out.into_bytes()
    }

    fn history_previous(&mut self, echo: &mut Vec<u8>) {
        if self.history.is_empty() || self.history_cursor == 0 {
            return;
        }
        self.history_cursor -= 1;
        self.line = self.history[self.history_cursor].clone();
        self.cursor = self.line.len();
        self.redraw(echo);
    }

    fn history_next(&mut self, echo: &mut Vec<u8>) {
        if self.history_cursor >= self.history.len() {
            return;
        }
        self.history_cursor += 1;
        self.line = self
            .history
            .get(self.history_cursor)
            .cloned()
            .unwrap_or_default();
        self.cursor = self.line.len();
        self.redraw(echo);
    }

    /// 回车重画当前输入行（历史翻页 / 中途编辑用），光标回到 `cursor` 处。
    fn redraw(&self, echo: &mut Vec<u8>) {
        echo.extend_from_slice(b"\r");
        echo.extend_from_slice(self.prompt().as_bytes());
        echo.extend_from_slice(self.line.as_bytes());
        echo.extend_from_slice(b"\x1b[K");
        let back: usize = self.line[self.cursor..]
            .chars()
            .map(|ch| fluxdb_core::terminal::char_cell_width(ch).max(1) as usize)
            .sum();
        self.move_cursor_left(echo, back);
    }

    /// `cursor` 左侧那个字符的字节起点；已在行首返回 `None`。
    fn char_before(&self, cursor: usize) -> Option<usize> {
        self.line[..cursor].char_indices().next_back().map(|(i, _)| i)
    }

    fn cursor_left(&mut self, echo: &mut Vec<u8>) {
        let Some(start) = self.char_before(self.cursor) else {
            return;
        };
        let width = self.line[start..self.cursor]
            .chars()
            .map(|ch| fluxdb_core::terminal::char_cell_width(ch).max(1) as usize)
            .sum();
        self.cursor = start;
        self.move_cursor_left(echo, width);
    }

    /// Delete：删光标右侧整个字符。
    fn delete_at_cursor(&mut self, echo: &mut Vec<u8>) {
        let Some(ch) = self.line[self.cursor..].chars().next() else {
            return;
        };
        self.line
            .replace_range(self.cursor..self.cursor + ch.len_utf8(), "");
        self.redraw(echo);
    }

    fn cursor_right(&mut self, echo: &mut Vec<u8>) {
        let Some(ch) = self.line[self.cursor..].chars().next() else {
            return;
        };
        let width = fluxdb_core::terminal::char_cell_width(ch).max(1) as usize;
        self.cursor += ch.len_utf8();
        // 宽字符占两格，向右移动要按单元列走。
        echo.extend_from_slice(format!("\x1b[{width}C").as_bytes());
    }

    fn cursor_home(&mut self, echo: &mut Vec<u8>) {
        let width: usize = self.line[..self.cursor]
            .chars()
            .map(|ch| fluxdb_core::terminal::char_cell_width(ch).max(1) as usize)
            .sum();
        self.cursor = 0;
        self.move_cursor_left(echo, width);
    }

    fn cursor_end(&mut self, echo: &mut Vec<u8>) {
        let rest = self.line[self.cursor..].to_string();
        let width: usize = rest
            .chars()
            .map(|ch| fluxdb_core::terminal::char_cell_width(ch).max(1) as usize)
            .sum();
        self.cursor = self.line.len();
        if width > 0 {
            echo.extend_from_slice(format!("\x1b[{width}C").as_bytes());
        }
    }

    fn move_cursor_left(&self, echo: &mut Vec<u8>, cells: usize) {
        if cells > 0 {
            echo.extend_from_slice(format!("\x1b[{cells}D").as_bytes());
        }
    }
}

/// MongoDB shell 会话 adapter：命名 / prompt / 状态文案（spawn 参数不会被使用）。
pub struct MongoCliAdapter {
    connection_id: fluxdb_core::ConnectionId,
    connection_name: String,
    session: MongoCliSession,
}

impl MongoCliAdapter {
    pub fn new(
        config: &fluxdb_core::ConnectionConfig,
        session: MongoCliSession,
    ) -> Self {
        Self {
            connection_id: config.id,
            connection_name: config.name.clone(),
            session,
        }
    }
}

impl mterm::TerminalSessionAdapter for MongoCliAdapter {
    fn kind(&self) -> mterm::TerminalSessionKind {
        mterm::TerminalSessionKind::MongoCli
    }

    fn session_key(&self) -> mterm::TerminalSessionKey {
        mterm::TerminalSessionKey::Mongo {
            connection_id: self.connection_id.0,
            database: self.session.database(),
        }
    }

    fn title(&self) -> String {
        format!("MongoDB Shell - {}", self.session.database())
    }

    fn status_text(&self, state: mterm::TerminalSessionState) -> String {
        match state {
            mterm::TerminalSessionState::Connecting => "连接中…".to_string(),
            mterm::TerminalSessionState::PtyRunning => "会话已建立".to_string(),
            mterm::TerminalSessionState::Ready => {
                format!("{} · 就绪", self.session.database())
            }
            mterm::TerminalSessionState::Busy => "执行中…".to_string(),
            mterm::TerminalSessionState::AwaitingConfirmation => "等待确认".to_string(),
            mterm::TerminalSessionState::Exited => "已退出".to_string(),
            mterm::TerminalSessionState::Failed => "启动失败".to_string(),
        }
    }

    /// 不拉起子进程：桌面端按 `kind()` 创建 `MongoReplTransport`；这里的参数只是占位，
    /// 保证 trait 契约（绝不执行，cmd 不存在）。
    fn spawn(&self) -> mterm::TerminalSpawnSpec {
        mterm::TerminalSpawnSpec {
            program: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            cols: MONGO_TERM_COLS,
            rows: MONGO_TERM_ROWS,
        }
    }

    /// REPL 自己负责回显与 prompt，这里不改状态（状态由桌面端按输出置位）。
    fn on_output(&mut self, _bytes: &[u8], _state: &mut mterm::TerminalSessionState) {}

    fn prompt(&self) -> String {
        self.session.prompt()
    }

    /// Ctrl+C 下发 `\x03`：由 REPL 自己当作「清空当前输入行」处理，会话保持存活。
    ///
    /// 不能返回空切片——组件层收到空切片就完全不写后端，REPL 的 `0x03` 分支永远收不到，
    /// 表现就是「按 Ctrl+C 什么都清不掉」。进程内 REPL 没有信号语义，不会因此退出。
    fn ctrl_c_bytes(&self, _state: mterm::TerminalSessionState) -> &'static [u8] {
        b"\x03"
    }

    fn meta(&self) -> std::collections::BTreeMap<String, String> {
        let mut meta = std::collections::BTreeMap::new();
        meta.insert("connection_id".to_string(), self.connection_id.0.to_string());
        meta.insert("database".to_string(), self.session.database());
        meta.insert("profile".to_string(), self.connection_name.clone());
        meta
    }
}

/// MongoDB shell 补全：顶层关键字 / `db.` 后的集合名 / `db.<集合>.` 后的方法名。
pub struct MongoCliCompletion {
    session: MongoCliSession,
}

impl MongoCliCompletion {
    pub fn new(session: MongoCliSession) -> Self {
        Self { session }
    }
}

/// 当前 token 的替换区间：输入行末尾连续标识符字符。
fn mongo_shell_token_range(input: &str) -> (usize, usize) {
    let end = input.len();
    let start = input
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_')
        .last()
        .map(|(index, _)| index)
        .unwrap_or(end);
    (start, end)
}

/// 按输入行的形状给出候选（只补名字，不猜参数）。
///
/// 集合名一律用 `db["name"]` 方括号形式（与 Compass 一致）：集合名里合法地可以带点，
/// `db.a.b.find(` 在解析器眼里是「集合 a、方法 b」，只有方括号能无歧义表达。
fn mongo_shell_candidates(
    input: &str,
    databases: &[String],
    collections: &[String],
) -> Vec<String> {
    // `use fl`：补库名（后面还有别的词就说明库名写完了）。
    if let Some(rest) = input.strip_prefix("use ")
        && !rest.contains(char::is_whitespace)
    {
        return databases
            .iter()
            .map(|name| format!("use {name}"))
            .collect();
    }
    if let Some(rest) = input.strip_prefix("db[") {
        return mongo_shell_bracket_candidates(rest, collections);
    }
    if let Some(rest) = input.strip_prefix("db.") {
        if let Some(dot) = rest.find('.') {
            let collection = &rest[..dot];
            let after = &rest[dot + 1..];
            // 走到 `(` 之后就不给候选了：参数结构没法按名字猜。
            if !collection.is_empty() && !after.contains('(') {
                return MONGO_SHELL_METHODS
                    .iter()
                    .map(|name| format!("db[\"{collection}\"].{name}("))
                    .collect();
            }
        }
        // `db.eve` / `db.`：补集合名。
        return collections
            .iter()
            .map(|name| format!("db[\"{name}\"]."))
            .collect();
    }
    MONGO_SHELL_KEYWORDS
        .iter()
        .map(|keyword| (*keyword).to_string())
        .collect()
}

/// 把 `db.<集合>.` 的输入改写成方括号写法，供候选前缀比对。
///
/// 用户用点写法打出来的行必须与方括号候选比得上，否则一个候选都出不来。集合名的边界
/// 是它后面的 `.`：还没出现点（`db.eve`）就说明集合名还在输入中，保持引号未闭合。
fn mongo_shell_canonical_input(input: &str) -> String {
    let Some(rest) = input.strip_prefix("db.") else {
        return input.to_string();
    };
    // 已经带 `(`：语句写完了，不再补。
    if rest.contains('(') {
        return input.to_string();
    }
    match rest.find('.') {
        Some(dot) => format!("db[\"{}\"].{}", &rest[..dot], &rest[dot + 1..]),
        None => format!("db[\"{rest}"),
    }
}

/// `db["…"]` 之后的候选：`"]` 还没出现就是还在写集合名，出现之后补方法名。
fn mongo_shell_bracket_candidates(rest: &str, collections: &[String]) -> Vec<String> {
    match rest.split_once("\"]") {
        // `db[` / `db["eve`：补集合名。
        None => collections
            .iter()
            .map(|name| format!("db[\"{name}\"]."))
            .collect(),
        // `db["events"].` / `db["events"].fi`：补方法名。
        Some((name, after)) if !after.contains('(') => {
            let name = name.trim_start_matches('"');
            MONGO_SHELL_METHODS
                .iter()
                .map(|method| format!("db[\"{name}\"].{method}("))
                .collect()
        }
        Some(_) => Vec::new(),
    }
}

/// MongoDB shell 补全候选：终端与命令工作台共用同一套形状判断与前缀过滤。
///
/// 返回的候选都是**整行**文本（带 `db.` / `use` 前缀），调用方按各自的替换区间落字：
/// 终端替换整行，工作台替换光标所在行。
pub fn mongo_shell_completions(
    input: &str,
    databases: &[String],
    collections: &[String],
) -> Vec<String> {
    // 候选统一用方括号拼集合名，用户敲的却是 `db.events.`。比对前把输入也转成
    // 方括号形式，否则前缀永远对不上（`db.events.` vs `db["events"].`）会一个候选都不出。
    let typed = mongo_shell_canonical_input(input);
    let (start, end) = mongo_shell_token_range(&typed);
    let prefix = &typed[..start];
    let token = &typed[start..end];
    mongo_shell_candidates(&typed, databases, collections)
        .into_iter()
        .filter(|candidate| {
            candidate.starts_with(prefix) && candidate[prefix.len()..].starts_with(token)
        })
        .collect()
}

impl mterm::TerminalCompletionAdapter for MongoCliCompletion {
    fn supports_completion(&self) -> bool {
        true
    }

    fn complete(&self, ctx: &mterm::TerminalCommandContext) -> Vec<mterm::TerminalCompletionItem> {
        // 候选是**整行**文本（带着 `db.` / 集合名前缀），因此替换区间是整行。
        //
        // 区间用字节偏移（组件层按字节发退格）。多发的退格落在空输入行上会被忽略，
        // 不会吃掉提示符，因此含中文的输入也安全。
        mongo_shell_completions(
            &ctx.input,
            &self.session.databases(),
            &self.session.collections(),
        )
            .into_iter()
            .map(|candidate| mterm::TerminalCompletionItem {
                label: candidate.clone(),
                insert_text: candidate.clone(),
                replace_start: 0,
                replace_end: ctx.input.len(),
                detail: None,
                kind: mterm::TerminalCompletionKind::Command,
                priority: mterm::TerminalCompletionPriority::Normal,
            })
            .collect()
    }

    fn apply(&self, item: &mterm::TerminalCompletionItem) -> mterm::CompletionApplyEffect {
        mterm::CompletionApplyEffect::Replace {
            start: item.replace_start,
            end: item.replace_end,
            text: item.insert_text.clone(),
        }
    }

    /// 写操作已在连接器解析层拒绝（只读终端），这里不需要二次确认。
    fn is_dangerous(&self, _input: &str) -> bool {
        false
    }

    /// 边打字边出候选（mongosh / Compass 的行为）。候选来自本地关键字与集合名，
    /// 不用等任何请求，所以能跟着回显逐字刷新。
    fn completion_on_typing(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod mongo_repl_input_tests {
    use super::*;

    /// 构造一个不连库的 REPL（配置指向不可达地址；本组用例只测输入处理与补全）。
    fn repl() -> (MongoRepl, Sender<Vec<u8>>, Receiver<Vec<u8>>) {
        let config = fluxdb_core::ConnectionConfig {
            id: fluxdb_core::ConnectionId(1),
            name: "unit".into(),
            kind: fluxdb_core::DatabaseKind::MongoDb,
            endpoint: fluxdb_core::Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1,
                database: Some("demo".into()),
            },
            options: std::collections::BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        (
            MongoRepl::new(config, MongoCliSession::new("demo".into())),
            tx,
            rx,
        )
    }

    fn drain(rx: &Receiver<Vec<u8>>) -> Vec<u8> {
        let mut bytes = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            bytes.extend_from_slice(&chunk);
        }
        bytes
    }

    #[test]
    fn prompt_shows_the_current_database_like_compass() {
        let (repl, _tx, _rx) = repl();
        assert_eq!(repl.prompt(), "demo> ");
    }

    #[test]
    fn greeting_is_only_the_prompt() {
        let (mut repl, tx, rx) = repl();
        repl.greet(&tx);
        // 开场不打用途说明：第一行就是提示符，别把函数签名塞进终端。
        assert_eq!(String::from_utf8_lossy(&drain(&rx)), "demo> ");
    }

    #[test]
    fn use_switches_the_prompt_and_stays_in_the_session() {
        let (mut repl, _tx, _rx) = repl();
        let rendered = String::from_utf8_lossy(&repl.execute("use other_db")).to_string();
        assert!(rendered.contains("switched to db other_db"), "{rendered}");
        assert!(rendered.ends_with("other_db> "), "{rendered}");
        // 库只属于会话：adapter 的 prompt 必须同步，否则渲染层光标会错位。
        assert_eq!(repl.session.prompt(), "other_db> ");
    }

    #[test]
    fn backspace_erases_one_cell_per_character_width() {
        let (mut repl, tx, rx) = repl();
        repl.feed("ab".as_bytes(), &tx);
        let _ = drain(&rx);
        // 半角字符退格一次；宽字符（中文）占两列，需要抹两格。
        repl.feed(b"\x7f", &tx);
        assert_eq!(drain(&rx), b"\x08 \x08".to_vec());
        assert_eq!(repl.line, "a");

        repl.feed("中".as_bytes(), &tx);
        let _ = drain(&rx);
        repl.feed(b"\x7f", &tx);
        assert_eq!(drain(&rx), b"\x08 \x08\x08 \x08".to_vec());
        assert_eq!(repl.line, "a");
    }

    /// adapter 必须把 Ctrl+C 真的下发到 REPL：返回空切片时组件层不会写后端，
    /// REPL 的清行分支就成了死代码（用户看到的现象是「按 Ctrl+C 没反应」）。
    #[test]
    fn adapter_forwards_ctrl_c_to_the_repl() {
        let adapter = MongoCliAdapter::new(
            &fluxdb_core::ConnectionConfig {
                id: fluxdb_core::ConnectionId(1),
                name: "unit".into(),
                kind: fluxdb_core::DatabaseKind::MongoDb,
                endpoint: fluxdb_core::Endpoint::Tcp {
                    host: "127.0.0.1".into(),
                    port: 1,
                    database: Some("demo".into()),
                },
                options: std::collections::BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
                dameng_profile: None,
                sqlserver_profile: None,
                mongo_profile: None,
                oracle_profile: None,
            },
            MongoCliSession::new("demo".into()),
        );
        assert_eq!(
            mterm::TerminalSessionAdapter::ctrl_c_bytes(&adapter, mterm::TerminalSessionState::Ready),
            b"\x03",
            "Ctrl+C 必须下发 \\x03，否则 REPL 的清行分支收不到任何输入"
        );
    }

    #[test]
    fn ctrl_c_and_ctrl_u_clear_the_current_line_without_executing() {
        // Ctrl+C 换一行再重画提示符（node/mongosh REPL 实测行为），Ctrl+U 就地清空。
        let (mut repl, tx, rx) = repl();
        repl.feed("db.events.find(".as_bytes(), &tx);
        let _ = drain(&rx);
        repl.feed(b"\x03", &tx);
        // 只有 `\r\n` + 提示符：不能带 `\x1b[K` —— 那会把被放弃的输入擦成空白。
        assert_eq!(drain(&rx), b"\r\ndemo> \x1b[K".to_vec());
        assert!(repl.line.is_empty());
        assert!(repl.history.is_empty(), "清行不应产生历史记录");

        repl.feed("show dbs".as_bytes(), &tx);
        let _ = drain(&rx);
        repl.feed(b"\x15", &tx);
        assert_eq!(drain(&rx), b"\rdemo> \x1b[K".to_vec());
        assert!(repl.line.is_empty());
        assert!(repl.history.is_empty(), "Ctrl+U 不应产生历史记录");
    }

    #[test]
    fn ctrl_d_ends_the_session_with_an_empty_frame() {
        let (mut repl, tx, rx) = repl();
        repl.feed(b"\x04", &tx);
        let frames = {
            let mut frames = Vec::new();
            while let Ok(chunk) = rx.try_recv() {
                frames.push(chunk);
            }
            frames
        };
        assert_eq!(frames.last().map(Vec::len), Some(0), "空帧表示会话结束");
    }

    #[test]
    fn utf8_split_across_chunks_is_kept_until_complete() {
        let (mut repl, tx, rx) = repl();
        let bytes = "中".as_bytes();
        repl.feed(&bytes[..2], &tx);
        assert!(repl.line.is_empty(), "不完整的 UTF-8 不能进输入行");
        assert!(drain(&rx).is_empty());
        repl.feed(&bytes[2..], &tx);
        assert_eq!(repl.line, "中");
    }

    #[test]
    fn blank_line_only_redraws_the_prompt() {
        let (mut repl, tx, rx) = repl();
        repl.feed(b"\r", &tx);
        let output = String::from_utf8_lossy(&drain(&rx)).to_string();
        assert_eq!(output, "\r\ndemo> ");
    }

    #[test]
    fn history_navigation_redraws_with_erase_to_end_of_line() {
        let (mut repl, tx, rx) = repl();
        repl.history = vec!["show dbs".into(), "show collections".into()];
        repl.history_cursor = repl.history.len();
        repl.feed(b"\x1b[A", &tx);
        let output = String::from_utf8_lossy(&drain(&rx)).to_string();
        assert_eq!(repl.line, "show collections");
        assert!(output.ends_with("\x1b[K"), "{output}");
        repl.feed(b"\x1b[A", &tx);
        assert_eq!(repl.line, "show dbs");
        repl.feed(b"\x1b[B", &tx);
        assert_eq!(repl.line, "show collections");
    }

    #[test]
    fn arrow_keys_move_the_cursor_and_typing_inserts_at_it() {
        let (mut repl, tx, rx) = repl();
        repl.feed(b"find()", &tx);
        let _ = drain(&rx);

        // ← 两次：光标落在 `find` 之后、`()` 之前。
        repl.feed(b"\x1b[D\x1b[D", &tx);
        let output = String::from_utf8_lossy(&drain(&rx)).to_string();
        assert_eq!(output, "\x1b[1D\x1b[1D", "每次只动一格: {output}");
        assert_eq!(repl.cursor, 4);

        // 中途插入：后面的内容重画一遍，光标退回插入点之后。
        repl.feed(b"X", &tx);
        assert_eq!(repl.line, "findX()");
        assert_eq!(repl.cursor, 5);
        let output = String::from_utf8_lossy(&drain(&rx)).to_string();
        assert!(output.contains("()\x1b[K"), "行尾内容要重画: {output}");
        assert!(output.ends_with("\x1b[2D"), "光标退回插入点: {output}");

        // → 回到行尾后就停住，不越界。
        repl.feed(b"\x1b[C\x1b[C\x1b[C", &tx);
        assert_eq!(repl.cursor, "findX()".len());
    }

    #[test]
    fn cursor_moves_by_cells_for_wide_characters() {
        let (mut repl, tx, rx) = repl();
        repl.feed("中a".as_bytes(), &tx);
        let _ = drain(&rx);
        // 先跨过半角的 `a`：一格。
        repl.feed(b"\x1b[D", &tx);
        assert_eq!(String::from_utf8_lossy(&drain(&rx)), "\x1b[1D");
        assert_eq!(repl.cursor, "中".len());
        // 再跨过 `中`：两个单元格、三个字节，按键只发一次。
        repl.feed(b"\x1b[D", &tx);
        assert_eq!(String::from_utf8_lossy(&drain(&rx)), "\x1b[2D");
        assert_eq!(repl.cursor, 0);
    }

    #[test]
    fn backspace_and_delete_work_at_the_cursor() {
        let (mut repl, tx, rx) = repl();
        repl.feed("abc".as_bytes(), &tx);
        let _ = drain(&rx);
        repl.feed(b"\x1b[D", &tx); // 光标在 b 之后
        let _ = drain(&rx);
        repl.feed(b"\x7f", &tx);
        assert_eq!(repl.line, "ac", "退格删光标左侧字符");
        assert_eq!(repl.cursor, 1);

        repl.feed(b"\x1b[3~", &tx);
        assert_eq!(repl.line, "a", "Delete 删光标右侧字符（4 字节序列不能被吞）");
    }

    #[test]
    fn home_and_end_jump_to_the_line_edges() {
        let (mut repl, tx, rx) = repl();
        repl.feed(b"show dbs", &tx);
        let _ = drain(&rx);
        repl.feed(b"\x1b[H", &tx);
        assert_eq!(repl.cursor, 0);
        assert_eq!(String::from_utf8_lossy(&drain(&rx)), "\x1b[8D");
        repl.feed(b"\x1b[F", &tx);
        assert_eq!(repl.cursor, 8);
        assert_eq!(String::from_utf8_lossy(&drain(&rx)), "\x1b[8C");
    }

    #[test]
    fn exit_command_reports_without_touching_the_database() {
        let (mut repl, _tx, _rx) = repl();
        let rendered = String::from_utf8_lossy(&repl.execute(" quit ")).to_string();
        assert!(rendered.contains("exited"), "{rendered}");
        assert!(repl.history.is_empty());
    }

    #[test]
    fn syntax_errors_are_echoed_line_by_line_in_red() {
        let (mut repl, _tx, _rx) = repl();
        let rendered = String::from_utf8_lossy(&repl.execute("db.events.insertOne({})")).to_string();
        assert!(rendered.contains("read-only"), "{rendered}");
        // 错误行带 ANSI 红色，且以 CRLF 结束——否则下一行会接着上一行的列。
        assert!(rendered.contains("\x1b[31m"), "{rendered}");
        assert!(rendered.contains("\r\n"), "{rendered}");
        assert!(rendered.ends_with("demo> "), "{rendered}");
    }

    #[test]
    fn completion_offers_keywords_while_typing_a_new_statement() {
        let completion = MongoCliCompletion::new(MongoCliSession::new("demo".into()));
        let items = completion.complete(&context("sh"));
        assert!(
            items.iter().any(|item| item.insert_text == "show dbs"),
            "{items:?}"
        );
        assert!(items.iter().any(|item| item.insert_text == "show collections"));
    }

    #[test]
    fn completion_offers_collections_then_methods_and_replaces_the_whole_line() {
        let session = MongoCliSession::new("demo".into());
        let mut database = session.database.lock().unwrap();
        *database = "demo".to_string();
        drop(database);
        session.set_collections(vec!["events".into(), "orders".into()]);
        let completion = MongoCliCompletion::new(session);

        let collections = completion.complete(&context("db.ev"));
        assert_eq!(collections.len(), 1, "{collections:?}");
        // 集合名用方括号：集合名允许带点，`db.a.b.find(` 会被解析成「集合 a、方法 b」。
        assert_eq!(collections[0].insert_text, "db[\"events\"].");
        assert_eq!(collections[0].replace_start, 0);
        assert_eq!(collections[0].replace_end, "db.ev".len(), "整行替换");

        let methods = completion.complete(&context("db.events.co"));
        assert!(
            methods
                .iter()
                .any(|item| item.insert_text == "db[\"events\"].countDocuments("),
            "{methods:?}"
        );
        assert!(
            methods
                .iter()
                .all(|item| item.replace_end == "db.events.co".len()),
            "方法候选同样替换整行"
        );
    }

    #[test]
    fn completion_walks_the_bracket_form_and_database_names() {
        let session = MongoCliSession::new("demo".into());
        session.set_collections(vec!["events".into()]);
        session.set_databases(vec!["admin".into(), "fluxdb_demo".into()]);
        let completion = MongoCliCompletion::new(session);

        // `use fl` 补库名（开场后台读一次，切库不重读）。
        let databases = completion.complete(&context("use fl"));
        assert_eq!(databases.len(), 1, "{databases:?}");
        assert_eq!(databases[0].insert_text, "use fluxdb_demo");

        // 方括号进行中：还没闭合引号就补集合名。
        let opening = completion.complete(&context("db[\"eve"));
        assert_eq!(opening[0].insert_text, "db[\"events\"].", "{opening:?}");

        // 方括号闭合后按方法名补。
        let methods = completion.complete(&context("db[\"events\"].fi"));
        assert!(
            methods
                .iter()
                .any(|item| item.insert_text == "db[\"events\"].find("),
            "{methods:?}"
        );
    }

    /// 构造补全上下文（补全按整行给出候选）。
    fn context(text: &str) -> mterm::TerminalCommandContext {
        mterm::TerminalCommandContext {
            input: text.to_string(),
            cursor: text.len(),
            prompt: "demo> ".to_string(),
            kind: mterm::TerminalSessionKind::MongoCli,
            meta: std::collections::BTreeMap::new(),
        }
    }
}
