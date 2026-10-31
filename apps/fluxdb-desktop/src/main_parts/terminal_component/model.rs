// TerminalComponent 的模型与生命周期：PTY 会话 + grid + adapter + 输入状态。
//
// 一个标签页对应一个 TerminalComponent。会话由 fluxdb-app 的 adapter 描述（spawn 参数/命名/
// 状态文案/危险命令识别），桌面端只负责真实 PTY 进程调度、字节搬运、渲染与键盘输入。
//
// 键盘模型与 sql_editor 一致：特殊键（Enter/Backspace/方向键/Tab/Esc/Ctrl 组合）通过
// `Action` + keymap 下发到对应 handler；普通可打印字符 / IME 走 EntityInputHandler 的
// `replace_text_in_range`。终端内没有“文档”，可打印字符直接以 UTF-8 交给 redis-cli 自行回显。

use fluxdb_core::terminal::{
    char_cell_width, row_real_chars, CompletionApplyEffect, TermGrid, TermPoint,
    TerminalCommandContext, TerminalCompletionAdapter, TerminalCompletionItem,
    TerminalSessionAdapter, TerminalSessionState, TerminalTranscriptEntry, TerminalTranscriptKind,
    classify_transcript_line, feed_bytes,
};

/// 终端键盘动作（危险命令确认 / 特殊键）。
#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = terminal, no_json)]
pub struct TermEnter {
    pub secondary: bool,
}

actions!(
    terminal,
    [
        TermBackspace,
        TermDelete,
        TermLeft,
        TermRight,
        TermUp,
        TermDown,
        TermHome,
        TermEnd,
        TermTab,
        TermShiftTab,
        TermEscape,
        TermCtrlC,
        TermCtrlD,
        TermCopy,
        TermPaste,
        TermClear,
    ]
);

/// 终端组件的 keymap 上下文名。
pub(crate) const TERMINAL_CONTEXT: &str = "Terminal";

/// 绑定终端键盘动作到 keymap（必须在 app boot 时调用）。
pub(crate) fn register_terminal_shortcuts(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new(
            "enter",
            TermEnter { secondary: false },
            Some(TERMINAL_CONTEXT),
        ),
        gpui::KeyBinding::new(
            "secondary-enter",
            TermEnter { secondary: true },
            Some(TERMINAL_CONTEXT),
        ),
        gpui::KeyBinding::new("backspace", TermBackspace, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("delete", TermDelete, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("left", TermLeft, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("right", TermRight, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("up", TermUp, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("down", TermDown, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("home", TermHome, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("end", TermEnd, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("tab", TermTab, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("shift-tab", TermShiftTab, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("escape", TermEscape, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("ctrl-c", TermCtrlC, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("ctrl-d", TermCtrlD, Some(TERMINAL_CONTEXT)),
        // 复制 / 粘贴 / 清屏（cmd 为 mac 习惯，ctrl-shift 为跨平台终端习惯）。
        // 注意 `ctrl-c` 已保留给 SIGINT；复制用 `cmd-c` / `ctrl-shift-c`。
        gpui::KeyBinding::new("cmd-c", TermCopy, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("ctrl-shift-c", TermCopy, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("cmd-v", TermPaste, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("ctrl-shift-v", TermPaste, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("cmd-l", TermClear, Some(TERMINAL_CONTEXT)),
        gpui::KeyBinding::new("ctrl-l", TermClear, Some(TERMINAL_CONTEXT)),
    ]);
}

/// 当前输入行上光标左/右两侧的定位方向（供整字删除/移动补偿用）。
#[derive(Clone, Copy)]
enum EdgeSide {
    Left,
    Right,
}

/// 每个终端标签页对应的组件实体。
pub(crate) struct TerminalComponent {
    pub focus_handle: FocusHandle,
    /// 真实 cell 缓冲（来自 fluxdb-core），是渲染与输入的唯一事实来源。
    pub grid: TermGrid,
    /// 会话 adapter（spawn 参数 / 命名 / 状态文案 / 危险命令）。
    pub adapter: Box<dyn TerminalSessionAdapter>,
    /// 补全 / 危险命令 adapter（复用 fluxdb-app 的 Redis 命令知识）。
    pub completion: Box<dyn TerminalCompletionAdapter>,
    /// 会话状态机。
    pub state: TerminalSessionState,
    /// 终端后端（PTY 或进程内 REPL；None = 尚未拉起或已关闭）。
    pub transport: Option<TerminalBackend>,
    /// PTY 创建失败时保留底层错误，供状态栏给出可操作的诊断信息。
    pub failure_reason: Option<String>,
    /// 等待危险命令确认的原始命令（非空时渲染确认条并拦截 Enter / 输入）。
    pub pending_dangerous: Option<String>,
    /// 是否需要请求一帧重绘（读线程数据到达时置位）。
    pub dirty: bool,
    /// IME 输入法组合进行中（Mac 拼音等未确认态）。
    /// 组合期间确认键（Return）只应提交输入法、不得当成"执行命令"发送 `\r` 给 redis，
    /// 否则回车换行后后续输入落到左下角。
    pub ime_composing: bool,
    /// 最近一次绘制时输入光标的矩形（window 坐标）。`bounds_for_range` 返回它，
    /// macOS 据此把输入法候选栏定位到光标旁边；否则候选栏落到屏幕左下角。
    pub ime_caret_bounds: Option<Bounds<Pixels>>,
    /// 鼠标左键按下拖动选中（区分“按下未拖动”与“正在拖选”）。
    pub dragging_selection: bool,
    /// prepaint 测得的 cell 尺寸 / 状态栏高缓存，供鼠标事件做「像素 → 单元格」换算。
    pub last_cell_w: f32,
    pub last_line_h: f32,
    pub last_status_h: f32,
    /// 终端画布可用像素高（排除状态栏），供滚动到顶 / 底判定。
    pub last_content_h: f32,
    /// 终端画布左上角（window 坐标），供鼠标事件换算相对坐标。
    pub last_origin_x: f32,
    pub last_origin_y: f32,
    /// 补全浮层候选（非空 + `completion_index.is_some()` 时显示）。
    pub completion_items: Vec<TerminalCompletionItem>,
    /// 当前选中候选下标；`None` 只表示「没有高亮项」（边打字边提示时就是这样）。
    pub completion_index: Option<usize>,
    /// 补全浮层候选列表的滚动句柄（候选多于可视行时，键盘选中要跟着滚）。
    pub completion_scroll: ScrollHandle,
    /// 会话语义日志（prompt / command / output / error / notice），
    /// 是「终端做了什么」的语义层记录；首期承接状态展示，后续供回看 / 搜索面板复用。
    pub transcript: Vec<TerminalTranscriptEntry>,
}

impl TerminalComponent {
    pub fn new(
        mut adapter: Box<dyn TerminalSessionAdapter>,
        completion: Box<dyn TerminalCompletionAdapter>,
        cx: &mut App,
    ) -> Self {
        let spec = adapter.spawn();
        let grid = TermGrid::new(spec.cols, spec.rows);
        let mut state = TerminalSessionState::Connecting;
        adapter.on_output(&[], &mut state);
        Self {
            focus_handle: cx.focus_handle(),
            grid,
            adapter,
            completion,
            state,
            transport: None,
            failure_reason: None,
            pending_dangerous: None,
            dirty: false,
            ime_composing: false,
            ime_caret_bounds: None,
            dragging_selection: false,
            last_cell_w: 8.0,
            last_line_h: 15.0,
            last_status_h: 24.0,
            last_content_h: 0.0,
            last_origin_x: 0.0,
            last_origin_y: 0.0,
            completion_items: Vec::new(),
            completion_index: None,
            completion_scroll: ScrollHandle::new(),
            transcript: Vec::new(),
        }
    }

    /// 向会话语义日志追加一条记录（带回退上限防止无界增长）。
    fn record_transcript(&mut self, kind: TerminalTranscriptKind, text: String) {
        if self.transcript.len() >= 5000 {
            self.transcript.remove(0);
        }
        self.transcript.push(TerminalTranscriptEntry { kind, text });
    }

    /// 记录 prepaint 实测的布局参数（渲染 / 鼠标事件共享，避免重复计算）。
    pub fn set_layout(
        &mut self,
        cell_w: f32,
        line_h: f32,
        status_h: f32,
        content_h: f32,
        origin_x: f32,
        origin_y: f32,
    ) {
        self.last_cell_w = cell_w;
        self.last_line_h = line_h;
        self.last_status_h = status_h;
        self.last_content_h = content_h;
        self.last_origin_x = origin_x;
        self.last_origin_y = origin_y;
    }

    /// 把窗口坐标系里的像素点换算成终端可视区 cell 坐标 `(x, y)`（越界则 clamp 进可视区）。
    pub fn window_point_to_cell(&self, pos: Point<Pixels>) -> TermPoint {
        let origin = point(px(self.last_origin_x), px(self.last_origin_y));
        self.point_to_cell(pos, origin)
    }

    /// 把窗口内像素点换算成终端可视区 cell 坐标 `(x, y)`（越界则 clamp 进可视区）。
    /// `window_origin` 为终端内容区（不含状态栏）左上角。
    pub fn point_to_cell(&self, point: Point<Pixels>, window_origin: Point<Pixels>) -> TermPoint {
        let x = ((f32::from(point.x) - f32::from(window_origin.x)) / self.last_cell_w)
            .floor()
            .max(0.0) as u16;
        let x = x.min(self.grid.cols().saturating_sub(1));
        let y = ((f32::from(point.y) - f32::from(window_origin.y)) / self.last_line_h)
            .floor()
            .max(0.0) as u16;
        let y = y.min(self.grid.rows().saturating_sub(1));
        TermPoint { x, y }
    }

    /// 拉起 MongoDB 命令行 REPL（进程内，无子进程；仅一次）。
    ///
    /// 与 PTY 会话共用 grid / 输入 / 渲染：REPL 自己负责回显与 prompt，输出按终端字节流回放。
    pub fn spawn_mongo_repl(
        &mut self,
        config: fluxdb_core::ConnectionConfig,
        session: fluxdb_app::MongoCliSession,
        cx: &mut Context<Self>,
    ) {
        if self.transport.is_some() {
            return;
        }
        let repl = fluxdb_app::MongoReplTransport::new(config, session);
        self.state = TerminalSessionState::Ready;
        self.failure_reason = None;
        self.transport = Some(TerminalBackend::Repl(repl));
        cx.notify();
    }

    /// 拉起真实 PTY 子进程（仅一次）。
    pub fn spawn_process(&mut self, cx: &mut Context<Self>) {
        if self.transport.is_some() {
            return;
        }
        let spec = self.adapter.spawn();
        match TerminalPty::spawn(&spec) {
            Ok(pty) => {
                self.state = TerminalSessionState::PtyRunning;
                self.failure_reason = None;
                self.grid.resize(spec.cols, spec.rows);
                self.transport = Some(TerminalBackend::Pty(pty));
                self.adapter.on_output(&[], &mut self.state);
            }
            Err(err) => {
                eprintln!("[terminal] 启动子进程失败: {err}");
                self.state = TerminalSessionState::Failed;
                self.failure_reason = Some(err.to_string());
                self.record_transcript(TerminalTranscriptKind::Error, tr("app.terminal.transcript.start_failed").replace("%{error}", &err.to_string()));
            }
        }
        cx.notify();
    }

    /// 消费后台读事件：把增量字节喂给 adapter + grid，并触发重绘。
    pub fn pump_transport(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        // 进程退出时先把 `&mut self.transport` 的借用收回（退出分支里不能同时写回 transport）。
        let mut exited = false;
        // transcript 条目先收集到局部，待 transport 借用结束后再写入字段，避免借重。
        let mut pending_transcript: Vec<(TerminalTranscriptKind, String)> = Vec::new();
        if let Some(pty) = self.transport.as_mut() {
            while !exited {
                let Some(event) = pty.try_recv() else {
                    break;
                };
                match event {
                    TerminalTransportEvent::Output(bytes) => {
                        // adapter 先解读（更新状态），再把字节写进 grid 供渲染。
                        self.adapter.on_output(&bytes, &mut self.state);
                        feed_bytes(&mut self.grid, &bytes);
                        // transcript：按行分类输出（语义日志，非 cell 网格副产品）。
                        let prompt = self.adapter.prompt();
                        for line in String::from_utf8_lossy(&bytes).lines() {
                            let kind = classify_transcript_line(line, &prompt);
                            pending_transcript.push((kind, line.to_string()));
                        }
                        changed = true;
                    }
                    TerminalTransportEvent::Exited => {
                        // 先取回真实退出状态（正常/失败/信号），交给 adapter 落状态；拿不到时
                        // 按“无法判定”兜底，绝不臆造成功退出。
                        let status = pty
                            .takeover_exit_status()
                            .map(|(code, signal)| fluxdb_core::terminal::TerminalExitStatus {
                                code: Some(code),
                                signal,
                            })
                            .unwrap_or_default();
                        // 状态栏失败原因：非零退出时给出，便于排障（不打印密码）。
                        if !status.is_success() {
                            self.failure_reason = status
                                .signal
                                .clone()
                                .map(|s| tr("app.terminal.transcript.killed_by_signal").replace("%{signal}", &s))
                                .or_else(|| status.code.map(|c| tr("app.terminal.transcript.exit_code").replace("%{code}", &c.to_string())))
                                .or_else(|| Some(tr("app.terminal.transcript.abnormal_exit").to_string()));
                        }
                        self.adapter.on_exit(status, &mut self.state);
                        pending_transcript
                            .push((TerminalTranscriptKind::System, tr("app.terminal.transcript.session_exited").to_string()));
                        changed = true;
                        exited = true;
                    }
                }
            }
        }
        if exited {
            self.transport = None;
        }
        for (kind, text) in pending_transcript {
            self.record_transcript(kind, text);
        }
        // 每帧都重算，不只在有新输出时：Mongo 的集合名由后台线程异步填进会话状态，
        // 到位时没有任何字节进 grid。只在 `changed` 分支刷新会让「打开就敲 db.」永远不出候选。
        // 候选来自本地状态，重算很便宜；输入行没变时结果也相同，不会闪。
        let before = self.completion_items.clone();
        self.sync_typing_completion();
        if changed || before != self.completion_items {
            self.dirty = true;
            cx.notify();
        }
    }

    /// 终止会话：关闭 PTY（杀掉子进程），状态置为 Exited。
    /// 在标签页关闭（apply_closed_tabs）时调用，避免 redis-cli 残留驻留。
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        if let Some(pty) = self.transport.as_mut() {
            pty.close();
        }
        self.transport = None;
        self.pending_dangerous = None;
        self.state = TerminalSessionState::Exited;
        self.record_transcript(TerminalTranscriptKind::System, tr("app.terminal.transcript.session_closed").to_string());
        cx.notify();
    }

    /// 下发字节到 PTY（未退出时才写）并请求重绘。
    fn send_bytes(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if let Some(pty) = self.transport.as_mut() {
            pty.write(bytes);
        }
        cx.notify();
    }

    /// 读取当前输入行文本（剥去 redis-cli 的 `host:port> ` 提示前缀），用于危险命令识别。
    fn current_input_line(&self) -> String {
        let cursor = self.grid.cursor();
        let view = self.grid.row_view(cursor.y as usize);
        let end = (cursor.x as usize).min(view.chars.len());
        let text: String = view.chars[..end].iter().collect();
        let prompt_text = self.adapter.prompt();
        let prompt = prompt_text.trim_end();
        text.strip_prefix(prompt)
            .map(|input| input.strip_prefix(' ').unwrap_or(input).to_string())
            .unwrap_or_else(|| text.trim_start().to_string())
    }

    /// 当前后端是否按字符编辑输入行（进程内 REPL 是，redis-cli 按字节）。
    ///
    /// 按字节编辑的后端要把一个宽字符的 3 个字节逐个删掉；按字符编辑的后端只认一次按键。
    fn backend_edits_by_char(&self) -> bool {
        self.transport.as_ref().is_some_and(TerminalBackend::edits_by_char)
    }

    /// 定位当前输入行上、处于 grid 光标左/右侧的完整字符（按终端单元宽度对齐）。
    ///
    /// 宽字符占 2 个单元格，其第 2 个单元是空白 ` `，不能按"字符数组相邻格"直接取；
    /// 这里遍历本行未裁剪的 `row_view().chars`，用 `char_cell_width` 累加单元宽度跳过
    /// 宽字符的空白第 2 格，得到"字符 → 起始单元格"的跨度表。
    ///
    /// 注意：redis-cli 的回显按"字节数"定位光标（如 `的` 占 3 字节 → `ESC[19C`），与 grid
    /// 对宽字符按 2 单元的实际布局不一致，会在宽字符右缘多出幻影空白格。因此不能只取
    /// `start < cursor_x` 的相邻跨度，而要**跳过空白/行尾填充格**，取光标左/右的**首个
    /// 实字符**（redis 行缓冲区里真实存在的输入内容）。返回该字符的字节数（`len_utf8()`，
    /// ASCII 为 1、中文为 3）。
    fn cursor_edge_byte_len(&self, cursor_x: usize, side: EdgeSide) -> Option<usize> {
        let view = self.grid.row_view(self.grid.cursor().y as usize);
        // redis 的退格/方向键按"字节"编辑，grid 光标 x 也是 redis 回显的字节位置，
        // 故统一按**字节区间**定位光标左/右侧整字（而非按单元宽，宽字符 2 格会算错）。
        let chars = row_real_chars(&view.chars);
        match side {
            // 光标左侧最近字符：字节起始 < 光标字节；返回其字节数（ASCII=1、中文=3）。
            EdgeSide::Left => chars
                .iter()
                .rev()
                .find(|rc| rc.byte < cursor_x)
                .map(|rc| rc.byte_len),
            // 光标所在/右侧最近字符：字节起始 >= 光标字节。
            EdgeSide::Right => chars
                .iter()
                .find(|rc| rc.byte >= cursor_x)
                .map(|rc| rc.byte_len),
        }
    }

    // ---- 补全（浮层 + 应用到 PTY） ----

    /// 补全候选行高，与编辑器补全浮层同一个常量（`COMPLETION_ROW_HEIGHT`）。
    pub const COMPLETION_ROW_HEIGHT: f32 = crate::editor_component::COMPLETION_ROW_HEIGHT;

    /// 补全浮层高度（像素）：候选行数 × 行高 + 内边距，超过上限就滚动。
    ///
    /// 行高直接复用编辑器的 `COMPLETION_ROW_HEIGHT`，外壳内边距与 `render.rs` 的 `p_1`
    /// （上下各 4px）一致。两边是同一个数，不能各算一套——算小了外层一裁，文字整块消失。
    pub fn completion_popup_height(items: usize) -> f32 {
        /// 外壳上下内边距 4+4。
        const SHELL_PADDING: f32 = 8.0;
        /// 超过这个高度就滚动，不把整屏占满。
        const MAX_H: f32 = 260.0;
        (items as f32 * Self::COMPLETION_ROW_HEIGHT + SHELL_PADDING).min(MAX_H)
    }

    /// 是否正显示补全浮层。
    ///
    /// 只看候选，不看高亮下标：边打字边提示时没有选中项，浮层照样要显示。
    pub fn completion_active(&self) -> bool {
        !self.completion_items.is_empty()
    }

    /// 补全浮层的窗口坐标锚点（像素）：输入光标正下方那一格。
    ///
    /// 用窗口坐标而不是相对容器偏移：浮层走 `deferred(anchored())` 画在容器裁剪之外，
    /// 相对定位会被终端面板的 `overflow_hidden` 切掉（截图上就是只剩一条边）。
    /// 横向跟着光标列走（以前钉在面板左边缘，光标在行尾时浮层和光标离着半个屏幕）。
    /// 下方放不下就翻到光标行上方，最后夹进内容区——不夹的话面板比窗口高时浮层跑到窗口外；
    /// 右侧越界由 `anchored().snap_to_window_with_margin` 兜底。
    pub fn completion_popup_anchor(&self) -> Point<Pixels> {
        let height = Self::completion_popup_height(self.completion_items.len());
        let line_h = self.last_line_h.max(1.0);
        let cursor_y = f32::from(self.grid.cursor().y);
        let below = (cursor_y + 1.0) * line_h;
        let top = if below + height <= self.last_content_h {
            below
        } else {
            cursor_y * line_h - height
        };
        let top = top.clamp(0.0, (self.last_content_h - height).max(0.0));
        let left = self.cursor_cell_x() as f32 * self.last_cell_w;
        point(
            px(self.last_origin_x + left),
            px(self.last_origin_y + top),
        )
    }

    /// 输入光标在内容区里的单元格列（含提示符前缀）。
    ///
    /// 与 `render.rs` 画光标块用的是同一套换算：后端提示符被换成了展示提示符，
    /// 所以光标列不能直接用 `cursor.x`，否则 `fluxdb_demo> ` 这种长前缀下浮层会偏左若干格。
    fn cursor_cell_x(&self) -> usize {
        let cursor = self.grid.cursor();
        let view = self.grid.row_view(cursor.y as usize);
        let line: String = view.chars.iter().collect();
        let native = self.adapter.prompt();
        let display = self.adapter.display_prompt();
        match normalize_prompt_line(&line, &native, &display) {
            Some((_, native_prefix_len, input_len)) => {
                let input_cursor = byte_cursor_to_cell(&view.chars, cursor.x as usize)
                    .saturating_sub(native_prefix_len);
                display.chars().count() + input_cursor.min(input_len)
            }
            None => cursor.x as usize,
        }
    }

    /// 浮层可见时用 ↑/↓ 移动高亮；返回 true 表示按键已被浮层消费。
    ///
    /// 没有高亮（边打字边提示）时 ↓ 选第一项、↑ 选最后一项，和编辑器补全一致。
    fn move_completion_selection(&mut self, delta: i32, cx: &mut Context<Self>) -> bool {
        let n = self.completion_items.len();
        if n == 0 {
            return false;
        }
        let n_i = n as i32;
        let next = match self.completion_index {
            Some(idx) => (idx as i32 + delta).rem_euclid(n_i),
            None if delta > 0 => 0,
            None => n_i - 1,
        } as usize;
        self.completion_index = Some(next);
        // 候选超过可视高度时要跟着滚，否则键盘选到浮层外什么也看不见。
        self.completion_scroll.scroll_to_item(next);
        cx.notify();
        true
    }

    /// 当前命令的只读参数提示。参数不会写入 PTY，实际光标仍停在已输入命令末尾。
    pub fn inline_completion_hint(&self) -> Option<String> {
        if self.completion_active()
            || !matches!(
                self.state,
                TerminalSessionState::PtyRunning | TerminalSessionState::Ready
            )
        {
            return None;
        }

        let input = self.current_input_line();
        let command = input.split_whitespace().next()?;
        if command.is_empty()
            || input.split_whitespace().count() != 1
            || input.chars().last().is_some_and(char::is_whitespace)
        {
            return None;
        }

        let ctx = self.completion_context();
        let item = self
            .completion
            .complete(&ctx)
            .into_iter()
            .find(|item| item.label.eq_ignore_ascii_case(command))?;
        let detail = item.detail?;
        let mut args = detail.split_whitespace();
        args.next();
        let hint = args
            .map(|arg| arg.trim_matches(['<', '>', '[', ']']))
            .filter(|arg| !arg.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        (!hint.is_empty()).then_some(hint)
    }

    /// 构造传给 completion adapter 的上下文：当前输入行 + 光标在行尾（byte 偏移）。
    /// prompt / meta 均由 adapter 提供，不写死成 `>` / 空表。
    fn completion_context(&self) -> TerminalCommandContext {
        let input = self.current_input_line();
        TerminalCommandContext {
            cursor: input.len(),
            input,
            prompt: self.adapter.prompt(),
            kind: self.adapter.kind(),
            meta: self.adapter.meta(),
        }
    }

    /// 鼠标划过候选行：把高亮挪过去（与 ↑/↓ 共用同一个下标）。
    pub fn select_completion(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.completion_items.len() && self.completion_index != Some(index) {
            self.completion_index = Some(index);
            cx.notify();
        }
    }

    /// 鼠标点击候选行：直接应用（等价于选中后回车，不必再按一次）。
    pub fn accept_completion(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.completion_items.get(index).cloned() else {
            return;
        };
        self.close_completion();
        self.apply_completion(&item, cx);
        cx.notify();
    }

    /// 关闭并清空补全浮层。
    fn close_completion(&mut self) {
        self.completion_items.clear();
        self.completion_index = None;
        // 滚回顶部：下一次打开若沿用上次的滚动位置，第一屏候选会凭空少几行。
        self.completion_scroll.set_offset(point(px(0.), px(0.)));
    }

    /// 按当前输入行刷新候选（边打字边提示用）。
    ///
    /// 只在用户没有正在用 Tab 选候选时刷新：否则每来一段回显都会把高亮重置回第一项，
    /// 用户没法继续往下选。候选来源是本地状态，不需要额外请求。
    fn sync_typing_completion(&mut self) {
        if !self.completion.completion_on_typing() || self.completion_index.is_some() {
            return;
        }
        if !matches!(
            self.state,
            TerminalSessionState::Ready | TerminalSessionState::PtyRunning
        ) {
            return;
        }
        let ctx = self.completion_context();
        // 空行不弹：mongosh 在空提示符下也不刷候选，否则一进终端就是满屏列表。
        if ctx.input.trim().is_empty() {
            self.close_completion();
            return;
        }
        let input = ctx.input.trim().to_string();
        // 已经打完的候选不再提示：否则刚用回车写回 `show dbs`，下一个回显又把它自己弹出来。
        self.completion_items = self
            .completion
            .complete(&ctx)
            .into_iter()
            .filter(|item| item.insert_text.trim() != input)
            .collect();
    }

    /// 应用一个补全候选：按 adapter 给的效果改写当前输入行。
    ///
    /// 多数为 `Replace{start,end,text}`：替换区间是相对输入行的 byte 偏移（光标在行尾），
    /// 我们发若干退格删掉区间内的内容、再键入 `text`——后端行编辑器会回显抹除 + 新字符，
    /// grid 由这些回显字节自行同步，故不会错位。退格次数按后端语义算：redis-cli 的
    /// linenoise 按字节删，进程内 REPL 按字符删。
    fn apply_completion(&mut self, item: &TerminalCompletionItem, cx: &mut Context<Self>) {
        if !matches!(
            self.state,
            TerminalSessionState::Ready | TerminalSessionState::PtyRunning
        ) {
            return;
        }
        let effect = self.completion.apply(item);
        match effect {
            CompletionApplyEffect::Insert { text } => {
                self.send_bytes(text.as_bytes(), cx);
            }
            CompletionApplyEffect::Replace { start, end, text } => {
                let input = self.current_input_line();
                // `start..end` 是相对输入行的**字节**偏移。redis-cli 的 linenoise 按 UTF-8 **字节**
                // 删除（`中`=3 字节、emoji=4 字节），因此要发 `end-start` 个 `\x7f`。上一版按
                // Unicode 字符数发 DEL，会把中文/emoji 残留半个字符产生 `�`。
                let start = start.min(input.len());
                let end = end.min(input.len());
                let delete_bytes = end.saturating_sub(start);
                // 进程内 REPL 自己按字符维护输入行，一个退格删一个字符；redis-cli 按字节。
                let delete_count = if self.backend_edits_by_char() {
                    input[start..end].chars().count()
                } else {
                    delete_bytes
                };
                for _ in 0..delete_count {
                    self.send_bytes(b"\x7f", cx);
                }
                self.send_bytes(text.as_bytes(), cx);
            }
            CompletionApplyEffect::PassThrough => {
                self.send_bytes(b"\t", cx);
            }
            CompletionApplyEffect::ShowHint { message: _ } => { /* 只提示，不落地 */ }
        }
    }

    // ---- Action handlers（签名与 sql_editor 一致，供 window.listener_for 使用） ----

    pub fn enter(&mut self, _action: &TermEnter, _window: &mut Window, cx: &mut Context<Self>) {
        // IME 组合中的确认键（Return）只提交输入法，不得当作"执行命令"向 redis 发 `\r`，
        // 否则回车换行后后续输入会落到左下角。
        if self.ime_composing {
            return;
        }
        // 补全浮层打开时：回车 = 应用当前高亮候选，并关闭浮层（不立即执行命令）。
        if let Some(idx) = self.completion_index {
            let item = self.completion_items.get(idx).cloned();
            self.close_completion();
            if let Some(item) = item {
                self.apply_completion(&item, cx);
            }
            cx.notify();
            return;
        }
        if self.pending_dangerous.take().is_some() {
            // 已有待确认危险命令时再次回车即“确认并执行”。
            self.state = TerminalSessionState::Busy;
            self.record_transcript(
                TerminalTranscriptKind::Notice,
                tr("app.terminal.transcript.danger_confirmed").to_string(),
            );
            self.send_bytes(b"\r", cx);
            return;
        }
        let line = self.current_input_line();
        if self.completion.is_dangerous(&line) {
            self.pending_dangerous = Some(line.clone());
            self.state = TerminalSessionState::AwaitingConfirmation;
            self.record_transcript(
                TerminalTranscriptKind::Notice,
                tr("app.terminal.transcript.danger_pending").replace("%{line}", &line),
            );
            cx.notify();
            return;
        }
        if !line.is_empty() {
            self.record_transcript(TerminalTranscriptKind::Command, line);
        }
        self.send_bytes(b"\r", cx);
    }

    pub fn escape(&mut self, _action: &TermEscape, _window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_dangerous.take().is_some() {
            // 取消危险命令：清确认态并下发 Ctrl-C 清掉 redis-cli 命令行。
            self.state = TerminalSessionState::Ready;
            self.send_bytes(b"\x03", cx);
        } else if self.completion_active() {
            // 关闭补全浮层，不把 Esc 透传给 PTY。
            // 用 `completion_active` 而不是高亮下标：边打字边提示的浮层没有高亮项。
            self.close_completion();
            cx.notify();
        } else {
            self.send_bytes(b"\x1b", cx); // redis-cli / VIM 等需要原始 ESC。
        }
    }

    pub fn ctrl_c(&mut self, _action: &TermCtrlC, _window: &mut Window, cx: &mut Context<Self>) {
        // 危险命令确认态：Ctrl+C 仅取消确认、回到可输入态，不退出会话。
        if self.pending_dangerous.take().is_some() {
            self.state = TerminalSessionState::Ready;
        }
        // 「Ctrl+C 会否退出 / 下发什么字节」由会话级策略（adapter）决定，不写死在组件层：
        // - 默认（shell / MySQL/SSH 等）仍下发 `\x03`（SIGINT），保持既有行为；
        // - redis-cli 覆盖为「清空当前行 / 仅中断执行」，不发送会退出进程的字节。
        // 返回空切片时不下发任何字节（仅做上方本地收尾），也绝不让会话进入 Exited。
        let bytes = self.adapter.ctrl_c_bytes(self.state);
        if !bytes.is_empty() {
            self.send_bytes(bytes, cx);
        }
    }

    pub fn ctrl_d(&mut self, _action: &TermCtrlD, _window: &mut Window, cx: &mut Context<Self>) {
        self.send_bytes(b"\x04", cx);
    }

    pub fn backspace(
        &mut self,
        _action: &TermBackspace,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_backspace(cx);
    }

    /// 按光标左侧完整字符字节数连发 `\x7f` 退格，删除整个字符而不切半（redis 的 linenoise
    /// 按"字节"编辑，中文等多字节字符若只发 1 个退格会切到字符中间产生乱码 `�`）。
    /// 退格 action 与 IME 的 `\u{8}`/`\x7f` 退格回调都走这里，保证中文、emoji 整字删除。
    fn send_backspace(&mut self, cx: &mut Context<Self>) {
        self.close_completion(); // 编辑输入行，候选已失效。
        let cursor_x = self.grid.cursor().x as usize;
        let n = self
            .cursor_edge_byte_len(cursor_x, EdgeSide::Left)
            .filter(|_| !self.backend_edits_by_char())
            .unwrap_or(1);
        self.send_bytes(&vec![0x7f; n], cx);
    }

    pub fn delete(&mut self, _action: &TermDelete, _window: &mut Window, cx: &mut Context<Self>) {
        self.close_completion();
        // redis-cli 的 Delete 键（`ESC[3~`）只删光标右侧 **1 字节**；对宽字符（中=3B）会
        // 切出半字 `�`。与 backspace/move_* 一致，按光标右侧完整字符的字节数连发 `ESC[3~`，
        // 让一次 Delete 删掉整个字符，绝不残留半字。
        let cursor_x = self.grid.cursor().x as usize;
        let n = self
            .cursor_edge_byte_len(cursor_x, EdgeSide::Right)
            .filter(|_| !self.backend_edits_by_char())
            .unwrap_or(1);
        self.send_bytes(&vec![0x1b, b'[', b'3', b'~'].repeat(n), cx);
    }

    pub fn move_left(&mut self, _action: &TermLeft, _window: &mut Window, cx: &mut Context<Self>) {
        self.close_completion();
        // 同上：中文按字节移动会落进字符中间，按左侧整字字节数连发 `ESC[D` 跨过整个字符。
        let cursor_x = self.grid.cursor().x as usize;
        let n = self
            .cursor_edge_byte_len(cursor_x, EdgeSide::Left)
            .filter(|_| !self.backend_edits_by_char())
            .unwrap_or(1);
        self.send_bytes(&vec![0x1b, b'[', b'D'].repeat(n), cx);
    }

    pub fn move_right(
        &mut self,
        _action: &TermRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_completion();
        // 同上：按光标右侧整字字节数连发 `ESC[C` 跨过整个字符。
        let cursor_x = self.grid.cursor().x as usize;
        let n = self
            .cursor_edge_byte_len(cursor_x, EdgeSide::Right)
            .filter(|_| !self.backend_edits_by_char())
            .unwrap_or(1);
        self.send_bytes(&vec![0x1b, b'[', b'C'].repeat(n), cx);
    }

    pub fn move_up(&mut self, _action: &TermUp, _window: &mut Window, cx: &mut Context<Self>) {
        // 浮层打开时 ↑/↓ 在候选间移动，不透传给 PTY（透传会去翻 redis-cli 历史行，
        // 用户在挑候选却被换掉了整行输入）。
        if self.move_completion_selection(-1, cx) {
            return;
        }
        self.close_completion();
        self.send_bytes(b"\x1b[A", cx);
    }

    pub fn move_down(&mut self, _action: &TermDown, _window: &mut Window, cx: &mut Context<Self>) {
        if self.move_completion_selection(1, cx) {
            return;
        }
        self.close_completion();
        self.send_bytes(b"\x1b[B", cx);
    }

    pub fn move_home(&mut self, _action: &TermHome, _window: &mut Window, cx: &mut Context<Self>) {
        self.close_completion();
        // `ESC[H` / `ESC[F` 不带数量，无法像退格那样按字节放大；
        // 按字节编辑的后端（redis-cli）认这两个；REPL 也认，按字符处理。
        self.send_bytes(b"\x1b[H", cx);
    }

    pub fn move_end(&mut self, _action: &TermEnd, _window: &mut Window, cx: &mut Context<Self>) {
        self.close_completion();
        self.send_bytes(b"\x1b[F", cx);
    }

    pub fn tab(&mut self, _action: &TermTab, _window: &mut Window, cx: &mut Context<Self>) {
        // 浮层已打开（含边打字边提示的无高亮态）：Tab 把高亮带到下一项。
        if !self.completion_items.is_empty() {
            let n = self.completion_items.len();
            self.completion_index = Some(match self.completion_index {
                Some(idx) => (idx + 1) % n,
                None => 0,
            });
            cx.notify();
            return;
        }
        // 未打开：问 completion adapter 是否可补，能补则开浮层（拦截 Tab），否则透传 \t。
        if self.pending_dangerous.is_none() && self.completion.supports_completion() {
            let ctx = self.completion_context();
            let items = self.completion.complete(&ctx);
            if !items.is_empty() {
                self.completion_items = items;
                self.completion_index = Some(0);
                cx.notify();
                return; // 拦截 Tab，不发给 PTY。
            }
        }
        self.send_bytes(b"\t", cx);
    }

    pub fn shift_tab(
        &mut self,
        _action: &TermShiftTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 浮层打开：Shift-Tab 反向循环；未打开时透传。
        if let Some(idx) = self.completion_index {
            let n = self.completion_items.len();
            self.completion_index = Some(if n == 0 { idx } else { (idx + n - 1) % n });
            cx.notify();
        } else {
            self.send_bytes(b"\x1b[Z", cx);
        }
    }

    /// 把住进来的可打印字符下发到 PTY（redis-cli 自行回显 + 行编辑）。
    pub fn insert_typed_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.pending_dangerous.is_some() {
            return; // 确认期间忽略输入，只能回车 / 取消。
        }
        if matches!(
            self.state,
            TerminalSessionState::Exited | TerminalSessionState::Failed
        ) {
            return;
        }
        // 用户继续输入，补全上下文已变化，关闭浮层避免残留过期候选。
        self.close_completion();
        self.send_bytes(text.as_bytes(), cx);
    }

    // ---- 选区：按下 / 拖动 / 抬起（像素坐标系 → cell 坐标） ----

    /// 落地一次“按下”：把窗口像素点换算进 grid（clamp 进可视区）后开始选区。
    pub fn begin_mouse_selection(&mut self, point: Point<Pixels>) {
        // 先随用户点击回到底部跟随（与主流终端一致：点击即回到最新输出）。
        self.grid.scroll_to_bottom();
        self.grid.begin_selection(self.window_point_to_cell(point));
        self.dragging_selection = true;
    }

    /// 拖动更新选中端点。
    pub fn update_mouse_selection(&mut self, point: Point<Pixels>) {
        if self.dragging_selection {
            self.grid.update_selection(self.window_point_to_cell(point));
        }
    }

    /// 抬起结束拖动（保留已选中的选区供复制）。
    pub fn end_mouse_selection(&mut self) {
        self.dragging_selection = false;
    }

    // ---- 滚动回看（滚轮） ----

    /// 按滚轮像素增量滚动视口；向上滚动回看历史，向下回到底部。
    pub fn scroll_by_delta(&mut self, delta_y: f32) {
        if delta_y == 0.0 {
            return;
        }
        let lines = (delta_y / self.last_line_h.max(1.0)).round() as i32;
        if lines > 0 {
            self.grid.scroll_up(lines as usize);
        } else {
            self.grid.scroll_down((-lines) as usize);
        }
    }

    // ---- 复制 / 粘贴 / 清屏（Action handlers） ----

    /// 复制当前选区（无选区时 no-op）。
    pub fn copy(&mut self, _action: &TermCopy, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.grid.has_selection() {
            return;
        }
        let text = self.grid.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// 把剪贴板文本作为输入下发到 PTY（相当于用户键入，交由 redis-cli 行编辑回显）。
    pub fn paste(&mut self, _action: &TermPaste, _window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_dangerous.is_some()
            || matches!(
                self.state,
                TerminalSessionState::Exited | TerminalSessionState::Failed
            )
        {
            return;
        }
        let Some(text) = read_clipboard_text(cx) else {
            return;
        };
        self.send_bytes(text.as_bytes(), cx);
    }

    /// 清屏（Ctrl-L 清空主屏并回到底部）。
    pub fn clear(&mut self, _action: &TermClear, _window: &mut Window, cx: &mut Context<Self>) {
        self.send_bytes(b"\x0c", cx);
        self.grid.scroll_to_bottom();
    }
}

/// 读剪贴板纯文本（gpui 空剪贴板可能没有 text()）。
fn read_clipboard_text(cx: &Context<TerminalComponent>) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

impl Focusable for TerminalComponent {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// 终端输入没有“文档”：可打印字符 / IME 直接以 UTF-8 下发给 PTY（redis-cli 自行回显 + 行编辑）。
// 除 replace_text_in_range 外的方法都返回 None / 空操作。
impl gpui::EntityInputHandler for TerminalComponent {
    fn text_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _actual_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        // IME 组合中必须返回非空区间：macOS 靠 `hasMarkedText`（=markedTextRange 是否 Some）
        // 判断是否处于组合态；若返回 None，确认键（Return）会被当成普通按键派发给 `enter()`
        // 而向 redis 发送 `\r`，导致回车换行、后续输入落到左下角。
        if self.ime_composing {
            Some(0..1)
        } else {
            None
        }
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        // 组合结束（取消/提交），恢复可执行命令的 Enter。
        self.ime_composing = false;
    }

    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // IME 提交最终文本的回调（本次落地的 char 已定型，组合随之结束）。实测 macOS 提交
        // 后未必调用 `unmark_text`，若不在这里清 `ime_composing`，后续退格/方向键会被 IME 当作
        // 组合态键吃掉：退格变成单字节 `\u{8}` 直接进 redis，删中文切半字、左右键失灵。
        self.ime_composing = false;
        // IME 删除"带组合标记字符"的退格以 `\u{8}`（BACKSPACE）形式回调。它代表用户想删掉
        // 已上屏的字符，必须走字节补偿的 `send_backspace` 整字删除，而不是把单个退格当普通
        // 文本下发（那会对中文切半个字）。
        if text.as_bytes() == b"\x08" || text.as_bytes() == b"\x7f" {
            self.send_backspace(cx);
            return;
        }
        self.insert_typed_text(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range_utf16: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // IME 组合期不下发，避免半组合字符污染 PTY；只标记"正在组合"，
        // 让确认键（Return）先用于提交输入法而不是执行命令。
        if !new_text.is_empty() {
            self.ime_composing = true;
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // 返回最近一次绘制时光标所在矩形（window 坐标），macOS 用 `firstRect(forCharacterRange:)`
        // 把它转成屏幕坐标来放置输入法候选栏：None 会让候选栏出现在屏幕左下角。
        self.ime_caret_bounds
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
