// 终端真实 cell 网格（作为 `fluxdb_core::terminal` 的子文件被 include）。
//
// 屏幕缓冲：定宽列 × 高的 cell 网格 + 环形 scrollback（顶部为更早的行）。
// 光标 / 视口滚动 / resize-reflow / selection 都属于内核层，不属于 Redis/MySQL/SSH adapter。
// 本文件只做纯数据与纯操作，不依赖 GPUI。

use std::collections::VecDeque;

/// 网格单元格：字符 + 可选的 24 位前景 / 背景色 + 加粗。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct TermCell {
    pub ch: char,
    pub fg: Option<(u8, u8, u8)>,
    pub bg: Option<(u8, u8, u8)>,
    pub bold: bool,
}

impl TermCell {
    /// 空 cell 统一用空格表示（未写入 / 被擦除都是空格），便于渲染、复制与行尾空白截断。
    fn blank() -> Self {
        TermCell {
            ch: ' ',
            ..TermCell::default()
        }
    }
}

/// 一行（总长度恒为 cols，未写到的位置为空格）。
type TermRow = Vec<TermCell>;

/// 终端网格：屏幕可视区 + scrollback。
///
/// 持有一个 `vte::Parser`（TERM-001 持久化解析状态），它既不 `Clone` 也不 `Debug`，
/// 故 `TermGrid` 不再派生这两个 trait，改为手写：`Clone` 视为「快照」，克隆出新解析器；
/// `Debug` 不打印解析器内部。
pub struct TermGrid {
    cols: u16,
    rows: u16,
    /// 可视区（rows 行 × cols 列），index = y（0..rows）。
    screen: Vec<TermRow>,
    /// scrollback，front 为最早的行；每行长度恒为 cols。
    scrollback: VecDeque<TermRow>,
    /// 光标（x, y），y 为可视区内 0..rows。
    cursor: TermPoint,
    /// 用户滚回看时的偏移（0 = 底部），以可视区行为单位。
    scroll_offset: usize,
    /// 备用屏标记（CSI ? 1049h/l）。
    pub alternate: bool,
    _saved_cursor: Option<TermPoint>,
    /// scrollback 上限（行数），超出丢弃最旧的。
    max_scrollback: usize,
    /// scrollback 里累计存储的原始行数（含被丢弃的），用于 follow 语义。
    scrolled_total: u64,
    /// 保存的光标（ESC 7 / ESC 8），主屏用。
    pub(crate) saved_cursor: Option<TermPoint>,
    /// SGR 解析后的“下一个打印字符”样式。
    pending_fg: Option<(u8, u8, u8)>,
    pending_bg: Option<(u8, u8, u8)>,
    pending_bold: bool,
    /// 用户鼠标选区（anchor + 拖动端点），None 表示无选区。
    selection: Option<TermSelection>,
    /// 持久化 ANSI/VT 解析器：跨多次 `feed_bytes` 保留下一条控制序列 / UTF-8 的
    /// 半途状态，避免 PTY read 把一条序列或一个多字节字符拆到两次 read 时损坏。
    parser: vte::Parser,
}

impl TermGrid {
    pub fn new(cols: u16, rows: u16) -> Self {
        assert!(cols >= 2 && rows >= 1, "网格尺寸非法: {cols}x{rows}");
        let screen = vec![blank_row(cols); rows as usize];
        TermGrid {
            cols,
            rows,
            screen,
            scrollback: VecDeque::new(),
            cursor: TermPoint { x: 0, y: 0 },
            scroll_offset: 0,
            alternate: false,
            _saved_cursor: None,
            max_scrollback: 5000,
            scrolled_total: 0,
            saved_cursor: None,
            pending_fg: None,
            pending_bg: None,
            pending_bold: false,
            selection: None,
            parser: vte::Parser::new(),
        }
    }

    /// 把一段字节流喂进持久化解析器（逐字节喂给 vte，vte 内部按 UTF-8 累积，
    /// 且在多次调用间保留半途的转义序列 / UTF-8 状态）。
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        // 先把 parser 结构体 take 出 `self`，避免 `self.parser` 与 `GridPerformer{ grid: self }`
        // 同时可变借用 `self`；跑完再把 parser 放回，保证下一次 read 继续复用同一解析状态。
        let mut parser = std::mem::take(&mut self.parser);
        let mut performer = GridPerformer { grid: self };
        parser.advance(&mut performer, bytes);
        self.parser = parser;
    }
}

impl std::clone::Clone for TermGrid {
    fn clone(&self) -> Self {
        // 快照语义：解析器的半途状态不跨克隆延续，新实例用全新解析器（对调用方无影响）。
        let mut cloned = TermGrid::new(self.cols, self.rows);
        cloned.screen = self.screen.clone();
        cloned.scrollback = self.scrollback.clone();
        cloned.cursor = self.cursor;
        cloned.scroll_offset = self.scroll_offset;
        cloned.alternate = self.alternate;
        cloned.max_scrollback = self.max_scrollback;
        cloned.scrolled_total = self.scrolled_total;
        cloned.saved_cursor = self.saved_cursor;
        cloned.pending_fg = self.pending_fg;
        cloned.pending_bg = self.pending_bg;
        cloned.pending_bold = self.pending_bold;
        cloned.selection = self.selection;
        cloned
    }
}

impl std::fmt::Debug for TermGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermGrid")
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("cursor", &self.cursor)
            .field("alternate", &self.alternate)
            .field("scrolled_total", &self.scrolled_total)
            .finish()
    }
}

impl TermGrid {
    /// 读取 / 写入“下一个打印字符”样式（由 SGR 解析驱动）。
    pub(crate) fn set_pending_style(
        &mut self,
        fg: Option<(u8, u8, u8)>,
        bg: Option<(u8, u8, u8)>,
        bold: bool,
    ) {
        self.pending_fg = fg;
        self.pending_bg = bg;
        self.pending_bold = bold;
    }

    /// 读取某单元格（供 SGR 读取当前样式作为该串字符的样式）。
    pub(crate) fn screen_cell(&self, y: usize, x: usize) -> TermCell {
        self.screen
            .get(y)
            .and_then(|r| r.get(x))
            .copied()
            .unwrap_or_default()
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// 底部可视行的序号（scroll_to_bottom 后 y==bottom）。
    pub fn bottom(&self) -> usize {
        self.rows as usize - 1
    }

    /// 当前光标（可视区内坐标）。
    pub fn cursor(&self) -> TermPoint {
        self.cursor
    }

    /// 可视区首行在「scrollback + screen」拼接序列中的下标（滚回看时向前偏移）。
    fn visible_start(&self) -> usize {
        if self.scroll_offset == 0 {
            return self.scrollback.len();
        }
        // scroll_offset 可视行数；滚起时从 scrollback 里挑若干行 + 全部/部分屏幕。
        if self.scrollback.len() >= self.scroll_offset {
            self.scrollback.len() - self.scroll_offset
        } else {
            (self.scrollback.len() + self.screen.len()).saturating_sub(self.rows as usize)
        }
    }

    /// 视口第 `y` 行（含滚回看偏移）的 cell 行。
    ///
    /// 选区 / 复制都用视口坐标：滚回看时 `screen[0]` 不是用户看到的第一行，
    /// 只有走这里才能和渲染出来的内容对上。
    fn viewport_row(&self, y: usize) -> Option<&TermRow> {
        let index = self.visible_start().checked_add(y)?;
        if index < self.scrollback.len() {
            self.scrollback.get(index)
        } else {
            self.screen.get(index - self.scrollback.len())
        }
    }

    /// 可见内容 = scrollback 的可见部分 + 当前屏幕，按 top→bottom 返回，供渲染。
    /// `follow` 为 true 表示跟随底部（scroll_offset=0）。
    pub fn visible_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let total_lines = self.scrollback.len() + self.screen.len();
        let start = self.visible_start();
        // 保证输出不超过 rows 行，避免渲染器溢出。
        let end = (start + self.rows as usize).min(total_lines);
        for i in start..end {
            let row = if i < self.scrollback.len() {
                &self.scrollback[i]
            } else {
                &self.screen[i - self.scrollback.len()]
            };
            out.push(row_text_raw(row));
        }
        out
    }

    /// 是否跟随底部（无用户滚回看）。
    pub fn following_bottom(&self) -> bool {
        self.scroll_offset == 0
    }

    /// 重置视口滚动到跟随底部。
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// 用户向上滚动 n 行（不回看时会把底部行推入可视，保证有内容卷动）。
    /// 返回实际滚动的行数（已到顶时为 0），拖选自动滚动据此同步选区坐标。
    pub fn scroll_up(&mut self, n: usize) -> usize {
        let max = self.scrollback.len();
        let before = self.scroll_offset;
        self.scroll_offset = (self.scroll_offset + n).min(if max >= self.rows as usize {
            self.rows as usize
        } else {
            max
        });
        self.scroll_offset - before
    }

    /// 用户向下滚动 n 行。返回实际滚动的行数（同 `scroll_up`）。
    pub fn scroll_down(&mut self, n: usize) -> usize {
        let before = self.scroll_offset;
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        before - self.scroll_offset
    }

    /// 视口内容整体移动 `rows` 行（正 = 内容下移 = 向上回看）后同步选区坐标。
    ///
    /// 选区用视口坐标存储，滚回看会让同一段文字落到不同的行号上；自动滚动后
    /// 不搬坐标的话，选区会“粘”在屏幕位置而不是粘在内容上。
    pub fn shift_selection_rows(&mut self, rows: i32) {
        let Some(selection) = self.selection.as_mut() else {
            return;
        };
        let last_row = self.rows.saturating_sub(1) as i32;
        for point in [&mut selection.anchor, &mut selection.end] {
            point.y = (point.y as i32 + rows).clamp(0, last_row) as u16;
        }
    }

    // ---- 鼠标选区（内核层状态；渲染 / 复制都消费它） ----

    /// 开始一次选区（鼠标按下）。
    pub fn begin_selection(&mut self, point: TermPoint) {
        self.selection = Some(TermSelection { anchor: point, end: point });
    }

    /// 拖动更新选区端点（鼠标移动）。未开始选区时为 no-op。
    pub fn update_selection(&mut self, point: TermPoint) {
        if let Some(sel) = self.selection.as_mut() {
            sel.end = point;
        }
    }

    /// 结束 / 清除选区。
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// 当前选区是否激活（供渲染高亮）。
    ///
    /// 有选区即激活：单字符词（`a`、标点）的 anchor 与 end 相同，不能再靠“两端不同”
    /// 判断。单击不留选区由桌面端保证——它在越过拖动阈值后才 `begin_selection`。
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// 规范化选区为 `(左上, 右下)`：按 (行, 列) 排序，反方向拖动也得到同一区间。
    pub fn selection_region(&self) -> Option<(TermPoint, TermPoint)> {
        let sel = self.selection?;
        if (sel.anchor.y, sel.anchor.x) <= (sel.end.y, sel.end.x) {
            Some((sel.anchor, sel.end))
        } else {
            Some((sel.end, sel.anchor))
        }
    }

    /// 某可视单元格是否落在选区内（用于绘制高亮背景）。
    ///
    /// 选区是**线性区间**而非矩形：跨行时首行从锚点列到行尾、中间整行、末行到端点列。
    /// 早期按矩形判定，高亮范围与 `copy_region` 复制出的文本对不上。
    pub fn is_cell_selected(&self, x: usize, y: usize) -> bool {
        let Some((start, end)) = self.selection_region() else {
            return false;
        };
        let (y, x) = (y as u16, x as u16);
        if y < start.y || y > end.y {
            return false;
        }
        if start.y == end.y {
            return x >= start.x && x <= end.x;
        }
        if y == start.y {
            return x >= start.x;
        }
        if y == end.y {
            return x <= end.x;
        }
        true
    }

    /// 双击选词：把 `point` 所在“词”的整段设为选区（线性区间，单行）。
    ///
    /// 词边界按 alacritty 语义选区的近似规则：空白与常见标点视作分隔符。点在分隔符/
    /// 空白上时就选中该单个 cell（宽字符占两格全选），不会出现“双击没反应”。
    /// 返回 false 表示该行取不到内容（超出视口）。
    pub fn select_word_at(&mut self, point: TermPoint) -> bool {
        let Some(row) = self.viewport_row(point.y as usize) else {
            return false;
        };
        let chars: Vec<char> = row.iter().map(|c| c.ch).collect();
        let real = row_real_chars(&chars);
        let x = point.x as usize;
        let Some(hit) = real
            .iter()
            .position(|rc| (rc.cell..rc.cell + rc.cell_len as usize).contains(&x))
        else {
            return false;
        };
        // 分隔符 / 空白：只选中它自己。
        let (first, last) = if !is_word_char(real[hit].c) {
            (hit, hit)
        } else {
            let mut first = hit;
            while first > 0 && is_word_char(real[first - 1].c) {
                first -= 1;
            }
            let mut last = hit;
            while last + 1 < real.len() && is_word_char(real[last + 1].c) {
                last += 1;
            }
            (first, last)
        };
        let anchor = TermPoint { x: real[first].cell as u16, y: point.y };
        let end = TermPoint {
            x: (real[last].cell + real[last].cell_len as usize - 1) as u16,
            y: point.y,
        };
        self.selection = Some(TermSelection { anchor, end });
        true
    }

    /// 三击选行：把整行设为选区（首列 → 末列，复制时行尾空白仍会被裁掉）。
    pub fn select_line_at(&mut self, y: u16) -> bool {
        if y >= self.rows {
            return false;
        }
        self.selection = Some(TermSelection {
            anchor: TermPoint { x: 0, y },
            end: TermPoint { x: self.cols.saturating_sub(1), y },
        });
        true
    }

    /// 提取选中文本（视口坐标、线性区间；行尾空白去除、行间 `\n`）。
    pub fn selected_text(&self) -> String {
        let Some((tl, br)) = self.selection_region() else {
            return String::new();
        };
        self.copy_region(tl, br)
    }

    /// 打印一个可见字符（覆盖光标处并右移一位，应用 SGR pending 样式；到行尾则折行）。
    pub fn print_char(&mut self, c: char) {
        let row = &mut self.screen[self.cursor.y as usize];
        let x = (self.cursor.x as usize).min(self.cols as usize - 1);
        row[x] = TermCell {
            ch: c,
            fg: self.pending_fg,
            bg: self.pending_bg,
            bold: self.pending_bold,
        };
        // CJK 等宽字符占两个终端单元，避免中文后退格只回退到半个字符的位置。
        let width = char_cell_width(c);
        if self.cursor.x + width < self.cols {
            self.cursor.x += width;
        } else if self.cursor.y + 1 < self.rows {
            self.cursor.y += 1;
            self.cursor.x = 0;
        }
    }

    /// 供渲染读取某可视行的字符与逐列样式。
    pub fn row_view(&self, y: usize) -> TermRowView {
        let mut chars = Vec::new();
        let mut fg = Vec::new();
        let mut bg = Vec::new();
        let mut bold = Vec::new();
        for cell in self.screen.get(y).cloned().unwrap_or_default() {
            chars.push(cell.ch);
            fg.push(cell.fg);
            bg.push(cell.bg);
            bold.push(cell.bold);
        }
        TermRowView { chars, fg, bg, bold, y }
    }

    /// 回车：回到行首。
    pub fn carriage_return(&mut self) {
        self.cursor.x = 0;
    }

    /// 换行（LF）：若在最后一行则上滚一格，否则下移一行。
    pub fn linefeed(&mut self) {
        if self.cursor.y as usize == self.bottom() {
            self.push_scrollback();
        } else {
            self.cursor.y += 1;
        }
        // LF 后不回行首（由 CR 或 CRLF 处理），与真实终端一致。
    }

    /// 光标退格一列；行首不退。
    pub fn backspace(&mut self) {
        if self.cursor.x > 0 {
            self.cursor.x -= 1;
            let row = &mut self.screen[self.cursor.y as usize];
            let x = self.cursor.x as usize;
            let previous = row[x].ch;
            row[x] = TermCell::blank();
            // 宽字符的第二个单元本身是空白，继续回退并清理第一个单元。
            if previous == ' ' && x > 0 && is_wide_char(row[x - 1].ch) {
                row[x - 1] = TermCell::blank();
                self.cursor.x -= 1;
            }
        }
    }

    /// 光标前移/后移/上/下（clamp 在可视区内）。
    pub fn cursor_forward(&mut self, n: u16) {
        self.cursor.x = (self.cursor.x + n).min(self.cols - 1);
    }
    pub fn cursor_back(&mut self, n: u16) {
        self.cursor.x = self.cursor.x.saturating_sub(n);
    }
    pub fn cursor_up(&mut self, n: u16) {
        self.cursor.y = self.cursor.y.saturating_sub(n);
    }
    pub fn cursor_down(&mut self, n: u16) {
        self.cursor.y = (self.cursor.y + n).min(self.rows - 1);
    }

    /// 置光标到绝对行列（0 基）。
    pub fn set_cursor(&mut self, x: u16, y: u16) {
        self.cursor.x = x.min(self.cols - 1);
        self.cursor.y = y.min(self.rows - 1);
    }

    /// 保存 / 恢复光标。
    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some(self.cursor);
    }
    pub fn restore_cursor(&mut self) {
        if let Some(p) = self.saved_cursor {
            self.cursor = p;
        }
    }

    /// 光标行推入 scrollback：把可视首行送进历史，再垫一行空白到底部。
    fn push_scrollback(&mut self) {
        let top = self.screen.remove(0);
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.pop_front();
        }
        self.scrollback.push_back(top);
        self.screen.push(blank_row(self.cols));
        self.scrolled_total += 1;
        // 若用户正滚回看，维持视口偏移不跟。
    }

    /// 清屏（保持 scrollback）。
    pub fn clear_screen(&mut self) {
        for r in self.screen.iter_mut() {
            r.fill(TermCell::blank());
        }
        self.cursor = TermPoint { x: 0, y: 0 };
    }

    /// CSI J 0：从光标位置清到屏幕末尾（当前行光标到行尾 + 其后所有行），保留光标。
    pub fn clear_from_cursor_to_end(&mut self) {
        self.erase_to_eol();
        let cy = self.cursor.y as usize;
        for row in self.screen.iter_mut().skip(cy + 1) {
            row.fill(TermCell::blank());
        }
    }

    /// CSI J 1：从屏幕开头清到光标位置（其前所有行 + 当前行行首到光标，含光标格），保留光标。
    ///
    /// 规范：ED 1 是“清到**包含**光标”为止，故当前行取 `0..=cursor`（比 ED 0 的行尾含光标一致）。
    pub fn clear_to_cursor(&mut self) {
        let cy = self.cursor.y as usize;
        let cx = self.cursor.x as usize;
        for row in self.screen.iter_mut().take(cy) {
            row.fill(TermCell::blank());
        }
        let up_to = (cx + 1).min(self.screen[cy].len());
        for cell in self.screen[cy].iter_mut().take(up_to) {
            *cell = TermCell::blank();
        }
    }

    /// CSI J 2/3：全屏清空，但保留当前光标位置（清屏与复位光标解耦）。
    pub fn clear_all_preserve(&mut self) {
        for r in self.screen.iter_mut() {
            r.fill(TermCell::blank());
        }
    }

    /// 清当前行从光标到行尾（ERASE LINE 0）。
    pub fn erase_to_eol(&mut self) {
        let x0 = self.cursor.x as usize;
        let row = &mut self.screen[self.cursor.y as usize];
        for cell in row.iter_mut().skip(x0) {
            *cell = TermCell::blank();
        }
    }

    /// 清当前行全部（ERASE LINE 2）。
    pub fn erase_line(&mut self) {
        let row = &mut self.screen[self.cursor.y as usize];
        row.fill(TermCell::blank());
    }

    /// 换到备用屏 / 回到主屏。备用屏用简易模型：清空可视、记录主屏内容现场暂存。
    pub fn enter_alternate(&mut self) {
        self.alternate = true;
        self.save_cursor();
        self.clear_screen();
    }
    pub fn leave_alternate(&mut self) {
        self.alternate = false;
        self.restore_cursor();
        self.scrollback.clear();
        for r in self.screen.iter_mut() {
            r.fill(TermCell::blank());
        }
    }

    /// 调整行/列（reflow）：把「scrollback + 当前屏幕」按新列宽重新折行，
    /// 行数变化保留底部内容、顶部溢出进历史；光标收敛回可视区并返回跟随底部。
    pub fn resize(&mut self, new_cols: u16, new_rows: u16) {
        if new_cols < 2 || new_rows < 1 {
            return;
        }
        // 把 scrollback + 屏幕按“实字符 + 单元宽”采集，再按新列宽重新折行。
        // 折行必须按 cell 宽度（宽字符=2 格）切，绝不能按 Rust `char` 数量在宽字符中间切断，
        // 否则 `中文`/emoji 会被切成半个字符导致 `�` 与光标漂移。
        //
        // 先去掉源末尾的整行空白：screen 底部未写到的行是“填充”，不是内容，参与折行只会
        // 占住底部位置、把真实内容整体挤进 scrollback（缩放后再拉宽也无法还原）。空白行在
        // reflow 末尾会以 blank 行重建，去掉它们不影响可视区行数。
        let mut source: Vec<TermRow> = self
            .scrollback
            .iter()
            .cloned()
            .chain(self.screen.iter().cloned())
            .collect();
        while source.last().is_some_and(|row| row.iter().all(|c| c.ch == ' ')) {
            source.pop();
        }
        let spans: Vec<Vec<(char, u16)>> = source.iter().map(row_char_spans).collect();
        let reflowed = reflow_into_rows(&spans, new_cols);
        let rows_out: Vec<TermRow> = if reflowed.len() > new_rows as usize {
            // 保留底部 new_rows 行，顶部溢出作为 scrollback。
            let split = reflowed.len() - new_rows as usize;
            let (history, bottom) = reflowed.split_at(split);
            self.scrollback.clear();
            for row in history {
                if self.scrollback.len() >= self.max_scrollback {
                    self.scrollback.pop_front();
                }
                self.scrollback.push_back(row.clone());
            }
            bottom.to_vec()
        } else {
            self.scrollback.clear();
            reflowed
        };

        self.cols = new_cols;
        self.rows = new_rows;
        self.screen = rows_out;
        while self.screen.len() < new_rows as usize {
            self.screen.push(blank_row(new_cols));
        }
        // 光标收敛回可视区内并回到底部跟随。
        self.cursor.x = (self.cursor.x as usize).min(new_cols as usize - 1) as u16;
        self.cursor.y = (self.cursor.y as usize).min(new_rows as usize - 1) as u16;
        self.scroll_offset = 0;
        // 选区坐标随尺寸失效，直接清掉，避免越界高亮 / 复制错位。
        self.selection = None;
    }

    /// 复制当前可视内容为纯文本（每行去尾部空格，行间 \n）。
    pub fn content_text(&self) -> String {
        let mut out = String::new();
        // 收集非空行，末尾的空行不参与输出（避免多余的 `\n`）。
        let rendered: Vec<String> = self
            .screen
            .iter()
            .map(|row| row_text(row))
            .filter(|t| !t.is_empty())
            .collect();
        for (i, text) in rendered.iter().enumerate() {
            out.push_str(text);
            if i + 1 != rendered.len() {
                out.push('\n');
            }
        }
        out
    }

    /// 提取行列区间 `[start, end]`（视口坐标、含两端列）的文本。
    ///
    /// 线性区间语义：首行从 `start.x` 到行尾、中间整行、末行从行首到 `end.x`；
    /// 同一行时取 `start.x..=end.x`。每行去尾部空白，行间 `\n`。
    pub fn copy_region(&self, start: TermPoint, end: TermPoint) -> String {
        let (start, end) = if (start.y, start.x) <= (end.y, end.x) {
            (start, end)
        } else {
            (end, start)
        };
        let mut out = String::new();
        for y in start.y..=end.y {
            let Some(row) = self.viewport_row(y as usize) else {
                continue;
            };
            let chars: Vec<char> = row.iter().map(|c| c.ch).collect();
            let (c0, c1) = if start.y == end.y {
                (start.x as usize, end.x as usize + 1)
            } else if y == start.y {
                (start.x as usize, chars.len())
            } else if y == end.y {
                (0, end.x as usize + 1)
            } else {
                (0, chars.len())
            };
            let c0 = c0.min(chars.len());
            let c1 = c1.min(chars.len());
            if c0 < c1 {
                // 按真实字符取，不按下标切 cell 数组：宽字符占两格，第二格是填充，
                // 直接切片会把 `中文` 复制成 `中 文`。
                let seg: String = row_real_chars(&chars)
                    .into_iter()
                    .filter(|rc| rc.cell >= c0 && rc.cell < c1)
                    .map(|rc| rc.c)
                    .collect();
                out.push_str(seg.trim_end());
            }
            if y != end.y {
                out.push('\n');
            }
        }
        out
    }
}

/// 字符占用终端单元的宽度：CJK 等宽字符占 2 单元，其余占 1 单元。
/// model.rs（桌面端）在发送退格/方向键前据此对齐"整字"，故设为 `pub`。
pub fn char_cell_width(c: char) -> u16 {
    if is_wide_char(c) {
        2
    } else {
        1
    }
}

/// 行内"实字符"信息：用于把 redis-cli 的**字节**光标与 grid 的**单元**布局双向换算。
/// redis 按字节编辑/回显（宽字符 `的` 计 3 字节、grid 计 2 格），二者在此对齐，
/// 退格/方向键补偿与光标吸附都基于这份表。
pub struct RowCharInfo {
    pub c: char,
    /// 该字符在行内的字节起始偏移（含提示符，ASCII 提示符字节=单元，故基线一致）。
    pub byte: usize,
    /// utf8 字节长（ASCII=1、中文=3）。
    pub byte_len: usize,
    /// 该字符在行内的单元列起始（含提示符）。
    pub cell: usize,
    /// 单元宽（ASCII=1、宽字符=2）。
    pub cell_len: u16,
}

/// 遍历一行 cell，跳过宽字符的第 2 空白格与行尾填充格，产出"实字符"表。
/// 行内真实空格（宽度 1）保留；宽字符的 padding 空格由 `i += cell_len` 跳过。
pub fn row_real_chars(cells: &[char]) -> Vec<RowCharInfo> {
    let mut out = Vec::new();
    let mut byte = 0usize;
    let mut cell = 0usize;
    let mut i = 0usize;
    while i < cells.len() {
        let c = cells[i];
        let w = char_cell_width(c) as usize;
        out.push(RowCharInfo {
            c,
            byte,
            byte_len: c.len_utf8(),
            cell,
            cell_len: w as u16,
        });
        byte += c.len_utf8();
        cell += w;
        i += w;
    }
    out
}

/// 把"字节光标 x"（redis 回显的 `ESC[nC`）换算成**单元列**光标：
/// 光标落在某字符字节区间内 → 吸附到该字符起始列；落在末尾边界 → 返回内容末尾单元列。
pub fn byte_cursor_to_cell(cells: &[char], byte_cursor: usize) -> usize {
    let chars = row_real_chars(cells);
    if let Some(rc) = chars.iter().find(|rc| rc.byte + rc.byte_len > byte_cursor) {
        rc.cell
    } else {
        chars
            .last()
            .map(|rc| rc.cell + rc.cell_len as usize)
            .unwrap_or(0)
    }
}

/// 语义选词（双击）用的“词内字符”判定：空白与常见标点视作分隔符。
///
/// 对齐 alacritty/Zed 的 `semantic_escape_chars`（Zed 额外把制表线 `─` 当分隔符），
/// 而不是只按空白切分——否则双击 `db.users.find()` 会把整行当成一个词。
fn is_word_char(c: char) -> bool {
    !c.is_whitespace()
        && !matches!(
            c,
            ',' | ';' | ':' | '=' | '+' | '-' | '*' | '/' | '\\' | '|' | '`' | '\'' | '"' | '!'
                | '?' | '&' | '%' | '$' | '#' | '@' | '~' | '^' | '(' | ')' | '[' | ']' | '{'
                | '}' | '<' | '>' | '─' | '│' | '.'
        )
}

fn is_wide_char(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115f | 0x2329..=0x232a | 0x2e80..=0xa4cf |
        0xac00..=0xd7a3 | 0xf900..=0xfaff | 0xfe10..=0xfe19 |
        0xfe30..=0xfe6f | 0xff00..=0xff60 | 0xffe0..=0xffe6 |
        0x1f300..=0x1faff)
}

fn blank_row(cols: u16) -> TermRow {
    vec![TermCell::blank(); cols as usize]
}

/// 行 → 纯文本（去掉行尾空白不影响复制语义的展示用）。
/// 空 cell 即空格（未写入 / 被擦除统一为空格），行尾空格去掉。
/// 供 `content_text`/`copy_region`/resize 等需要"去尾部空白"语义的路径使用。
fn row_text(row: &TermRow) -> String {
    let mut s: String = row.iter().map(|c| c.ch).collect();
    while s.ends_with(' ') {
        s.pop();
    }
    s
}

/// 行 → 纯文本，**保留**行尾空格。供渲染路径（`visible_lines`）使用：
/// redis-cli 输入行末尾的空格是"输入内容"，裁剪会让光标被 `input_len` 夹回提示符（Bug 1）。
fn row_text_raw(row: &TermRow) -> String {
    row.iter().map(|c| c.ch).collect()
}

/// 把一行 stored row 还原成“实字符 + 单元宽”的有序序列（跳过宽字符的填充空白格）。
///
/// 只取到最后一个非空格为止（右侧未写到的空白格是“填充”，非内容），否则整屏每一行都被
/// `cols` 个 padding 空格撑成整版行，reflow 时真实内容会被挤到 scrollback 外而丢失。
fn row_char_spans(row: &TermRow) -> Vec<(char, u16)> {
    let chars: Vec<char> = row.iter().map(|c| c.ch).collect();
    let end = chars.iter().rposition(|c| *c != ' ').map(|i| i + 1).unwrap_or(0);
    row_real_chars(&chars[..end])
        .into_iter()
        .map(|rc| (rc.c, rc.cell_len))
        .collect()
}

/// 按 cell 宽度把多行“实字符”序列重新折行：每个逻辑行独立，超宽则溢出成多行。
/// 宽字符占用 2 格，若剩余宽度不足则整体换到下一行，绝不在宽字符中间切断。
fn reflow_into_rows(rows: &[Vec<(char, u16)>], cols: u16) -> Vec<TermRow> {
    let colw = cols as usize;
    let mut out: Vec<TermRow> = Vec::new();
    for spans in rows {
        if spans.is_empty() {
            out.push(blank_row(cols));
            continue;
        }
        let mut cur = blank_row(cols);
        let mut x = 0usize;
        for (c, w) in spans {
            let w = *w as usize;
            if x + w > colw {
                out.push(std::mem::replace(&mut cur, blank_row(cols)));
                x = 0;
            }
            cur[x] = TermCell { ch: *c, ..TermCell::blank() };
            x += w;
        }
        out.push(cur);
    }
    out
}

/// 光标 / 选区坐标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermPoint {
    pub x: u16,
    pub y: u16,
}

/// 鼠标选区：anchor（按下点）+ end（当前拖动点），均为可视区内坐标。
/// 高亮与 `selected_text` 都按 (行, 列) 排序后的线性区间解释，反方向拖动也一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSelection {
    pub anchor: TermPoint,
    pub end: TermPoint,
}

/// 供渲染读取一行的字符与逐列样式。
#[derive(Clone, Debug)]
pub struct TermRowView {
    pub chars: Vec<char>,
    pub fg: Vec<Option<(u8, u8, u8)>>,
    pub bg: Vec<Option<(u8, u8, u8)>>,
    pub bold: Vec<bool>,
    pub y: usize,
}


#[cfg(test)]
mod grid_selection_tests {
    use super::*;

    /// 造一个 16 列、至少 4 行、内容为指定文本的网格（不足补空格）。
    fn grid_with(lines: &[&str]) -> TermGrid {
        let mut grid = TermGrid::new(16, lines.len().max(4) as u16);
        let mut bytes = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                bytes.push_str("\r\n");
            }
            bytes.push_str(line);
        }
        grid.feed_bytes(bytes.as_bytes());
        grid
    }

    fn point(x: u16, y: u16) -> TermPoint {
        TermPoint { x, y }
    }

    #[test]
    fn selection_requires_explicit_begin() {
        // 内核不替调用方决定“单击算不算选区”（桌面端越过拖动阈值才 begin）：
        // 没有 begin 时拖动不改任何状态，begin 之后哪怕两端相同也是有效选区。
        let mut grid = grid_with(&["hello"]);
        assert!(!grid.has_selection());
        grid.update_selection(point(4, 0));
        assert!(!grid.has_selection(), "未开始选区时拖动不应产生选区");
        grid.begin_selection(point(2, 0));
        assert!(grid.has_selection(), "单格选区也必须算激活");
        assert_eq!(grid.selected_text(), "l");
        grid.update_selection(point(4, 0));
        assert_eq!(grid.selected_text(), "llo");
    }

    #[test]
    fn reversed_drag_copies_same_text() {
        let mut grid = grid_with(&["hello world"]);
        grid.begin_selection(point(10, 0));
        grid.update_selection(point(6, 0));
        assert_eq!(grid.selected_text(), "world");
        // 反向拖动的高亮与正向一致：6..=10 列都亮。
        assert!(grid.is_cell_selected(6, 0));
        assert!(grid.is_cell_selected(10, 0));
        assert!(!grid.is_cell_selected(5, 0));
    }

    #[test]
    fn multi_line_selection_is_linear_not_rectangular() {
        let mut grid = grid_with(&["aaaa", "bbbb", "cccc"]);
        // 从第 0 行第 2 列拖到第 2 行第 1 列。
        grid.begin_selection(point(2, 0));
        grid.update_selection(point(1, 2));
        // 中间整行都亮；首行只从锚点列起，末行只到端点列。
        assert!(!grid.is_cell_selected(1, 0), "首行锚点左侧不应高亮");
        assert!(grid.is_cell_selected(2, 0));
        assert!(grid.is_cell_selected(0, 1));
        assert!(grid.is_cell_selected(1, 2));
        assert!(!grid.is_cell_selected(2, 2), "末行端点右侧不应高亮");
        assert_eq!(grid.selected_text(), "aa\nbbbb\ncc");
    }

    #[test]
    fn shift_selection_rows_keeps_selection_on_content() {
        let mut grid = grid_with(&["hello"]);
        grid.begin_selection(point(0, 0));
        grid.update_selection(point(4, 0));
        // 视口内容下移一行（向上回看）后，选区应跟着落到第 1 行。
        grid.shift_selection_rows(1);
        assert!(grid.is_cell_selected(2, 1));
        assert!(!grid.is_cell_selected(2, 0));
    }

    #[test]
    fn select_word_at_stops_at_punctuation() {
        let mut grid = grid_with(&["db.users.find()"]);
        // 点在 "users" 中间。
        assert!(grid.select_word_at(point(5, 0)));
        assert_eq!(grid.selected_text(), "users");
        // 边界外侧不亮。
        assert!(!grid.is_cell_selected(2, 0), "`.` 左侧不应高亮");
        assert!(!grid.is_cell_selected(11, 0), "`.` 右侧不应高亮");
    }

    #[test]
    fn select_word_at_separator_selects_single_cell() {
        let mut grid = grid_with(&["a b"]);
        assert!(grid.select_word_at(point(1, 0)));
        assert!(grid.has_selection());
        assert_eq!(grid.selected_text(), "");
        assert_eq!(grid.selection_region(), Some((point(1, 0), point(1, 0))));
    }

    #[test]
    fn select_word_at_takes_whole_wide_char() {
        let mut grid = grid_with(&["中文 ok"]);
        // 点在宽字符的第二个单元上，仍应选中整个字符（两格）。
        assert!(grid.select_word_at(point(1, 0)));
        assert_eq!(grid.selected_text(), "中文");
        assert!(grid.is_cell_selected(0, 0) && grid.is_cell_selected(3, 0));
    }

    #[test]
    fn select_line_at_selects_full_row() {
        let mut grid = grid_with(&["aaaa", "bbbb"]);
        assert!(grid.select_line_at(1));
        assert_eq!(grid.selected_text(), "bbbb");
        assert!(grid.is_cell_selected(0, 1));
        assert!(!grid.is_cell_selected(0, 0));
        assert!(!grid.select_line_at(99), "越界行不应改动选区");
    }
}
