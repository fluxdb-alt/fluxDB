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

    /// 可见内容 = scrollback 的可见部分 + 当前屏幕，按 top→bottom 返回，供渲染。
    /// `follow` 为 true 表示跟随底部（scroll_offset=0）。
    pub fn visible_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let total_lines = self.scrollback.len() + self.screen.len();
        let start = {
            if self.scroll_offset == 0 {
                self.scrollback.len()
            } else {
                // scroll_offset 可视行数；滚起时从 scrollback 里挑若干行 + 全部/部分屏幕。
                let visible = self.rows as usize;
                if self.scrollback.len() >= self.scroll_offset {
                    self.scrollback.len() - self.scroll_offset.min(self.scrollback.len())
                } else {
                    total_lines.saturating_sub(visible)
                }
            }
        };
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
    pub fn scroll_up(&mut self, n: usize) {
        let max = self.scrollback.len();
        self.scroll_offset = (self.scroll_offset + n).min(if max >= self.rows as usize {
            self.rows as usize
        } else {
            max
        });
    }

    /// 用户向下滚动 n 行。
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
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
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// 规范化选区为 `(左上, 右下)`。
    pub fn selection_region(&self) -> Option<(TermPoint, TermPoint)> {
        let sel = self.selection?;
        let (sx, sy) = (sel.anchor.x.min(sel.end.x), sel.anchor.y.min(sel.end.y));
        let (ex, ey) = (sel.anchor.x.max(sel.end.x), sel.anchor.y.max(sel.end.y));
        Some((TermPoint { x: sx, y: sy }, TermPoint { x: ex, y: ey }))
    }

    /// 某可视单元格是否落在选区内（用于绘制高亮背景）。
    pub fn is_cell_selected(&self, x: usize, y: usize) -> bool {
        let Some((tl, br)) = self.selection_region() else {
            return false;
        };
        y >= tl.y as usize && y <= br.y as usize && x >= tl.x as usize && x <= br.x as usize
    }

    /// 提取选中文本（复用行列区间的复制语义，行尾空白去除、行间 `\n`）。
    /// 选区的右下端按“含该列”处理（标准终端选区语义），故 `copy_region` 的末行右界取 `br.x+1`。
    pub fn selected_text(&self) -> String {
        let Some((tl, br)) = self.selection_region() else {
            return String::new();
        };
        // copy_region 末行右界为“不含”，传入 +1 使包含端点列（内部按行长截断，安全）。
        let end = TermPoint { x: br.x.saturating_add(1), y: br.y };
        self.copy_region(tl, end)
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

    /// 提取行列区间 `(start, end)`（可视区内、行优先）的文本。
    /// 首期按“行”文本提取：整行时取全部有效字符，部分行裁掉左右列区间之外的部分，行间 `\n`。
    pub fn copy_region(&self, start: TermPoint, end: TermPoint) -> String {
        let (sx, sy) = (start.x as usize, start.y as usize);
        let (ex, ey) = (end.x as usize, end.y as usize);
        let (y0, y1) = (sy.min(ey), sy.max(ey));
        let mut out = String::new();
        for y in y0..=y1 {
            let row_text = match self.screen.get(y) {
                Some(row) => row_text(row),
                None => continue,
            };
            let chars: Vec<char> = row_text.chars().collect();
            // 首/末行的边界列（列区间可视化）：整行时取全行，首行从头、末行截到 ex。
            let (c0, c1) = if y == y0 {
                (sx.min(chars.len()), chars.len())
            } else if y == y1 {
                (0, ex.min(chars.len()))
            } else {
                (0, chars.len())
            };
            if c0 <= c1 {
                let seg: String = chars[c0..c1].iter().collect();
                out.push_str(seg.trim_end());
            }
            if y != y1 {
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
/// 渲染高亮 / `selected_text` 复制都基于规范化后的矩形覆盖。
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
mod tests {
    use super::*;

    // 抽象出一个 20 列 × 5 行的网格，减少样板。
    fn grid20() -> TermGrid {
        TermGrid::new(20, 5)
    }

    // ---- TERM-001：持久化解析状态（控制序列 / 多字节 UTF-8 被拆到两次 feed） ----

    #[test]
    fn ansi_sequence_split_across_feeds_persists() {
        // 一条 SGR 被 PTY read 拆成两段：`\x1b[31` 与 `m`。若解析器不持久化，
        // 前半段会被当作乱码打印；持久化后应正确解析成红色，且不留垃圾字符。
        let mut g = grid20();
        g.feed_bytes(b"ab\x1b[31");
        // 中间不 flush / 不复位，直接继续喂下一段。
        g.feed_bytes(b"mcd");
        assert_eq!(g.content_text(), "abcd");

        // 再验证 256 色完整序列拆分也不损坏。
        let mut g = grid20();
        g.feed_bytes(b"x\x1b[38;");
        g.feed_bytes(b"5;196m");
        g.feed_bytes(b"yz");
        assert_eq!(g.content_text(), "xyz");
        // 第二个字符应为红色（196 号色），确认 SGR 状态也跨段保持。
        let view = g.row_view(0);
        assert_eq!(view.chars[1], 'y');
        assert!(view.fg[1].is_some(), "SGR 颜色应跨分段保持");
    }

    #[test]
    fn utf8_cjk_split_across_feeds_single_char() {
        // `中` 的 UTF-8 = E4 B8 AD。一次只喂 2 字节，另 1 字节下次补上。
        // 应拼成一个 `中`（占 2 格），而不是两个残缺字符。
        let mut g = grid20();
        g.feed_bytes(b"\xe4\xb8");
        g.feed_bytes(b"\xad");
        assert_eq!(g.content_text(), "中");
        assert_eq!(g.cursor().x, 2, "宽字符占 2 格");
    }

    #[test]
    fn utf8_emoji_split_across_feeds_single_char() {
        // 😀 = F0 9F 98 80（宽字符，占 2 格）。
        let mut g = grid20();
        g.feed_bytes(b"\xf0\x9f\x98"); // 前 3 字节
        g.feed_bytes(b"\x80"); // 第 4 字节
        assert_eq!(g.content_text(), "😀");
        assert_eq!(g.cursor().x, 2, "emoji 占 2 格");
    }

    // ---- TERM-002：TAB 制表位 / CSI J 清屏光标语义 ----

    #[test]
    fn tab_advances_to_eight_col_tab_stop() {
        // 行首 TAB → 净前进 8（到第 8 列）。
        let mut g = grid20();
        g.feed_bytes(b"\t");
        assert_eq!(g.cursor().x, 8, "行首 TAB 到第 8 列");

        // `ab` 后 TAB → 距下一个 8 倍数列 6 格 → x=8。
        let mut g = grid20();
        g.feed_bytes(b"ab\t");
        assert_eq!(g.cursor().x, 8);

        // 恰好停在制表位上再 TAB → 前进整 8。
        let mut g = grid20();
        g.feed_bytes(b"12345678\t");
        assert_eq!(g.cursor().x, 16);
    }

    #[test]
    fn csi_j0_clears_to_end_preserves_cursor() {
        let mut g = grid20();
        g.feed_bytes(b"1234567890\x1b[3G"); // 光标到 col3(0 基 x=2)
        assert_eq!(g.cursor(), TermPoint { x: 2, y: 0 });
        g.feed_bytes(b"\x1b[J"); // J 0
        // 从 col2(含) 清到行尾 → 只剩 "12"。
        assert_eq!(g.content_text(), "12");
        assert_eq!(g.cursor(), TermPoint { x: 2, y: 0 }, "清屏不移动光标");
    }

    #[test]
    fn csi_j1_clears_from_start_preserves_cursor() {
        let mut g = grid20();
        g.feed_bytes(b"1234567890\x1b[3G");
        g.feed_bytes(b"\x1b[1J"); // J 1
        // ED 1 清到“含光标”为止：col0..=2（'1','2','3'）清空 → 剩 "4567890"
        // （清空的 3 格是空格，故 `compact` 去掉空格后恰为 "4567890"）。
        assert_eq!(compact(&g.content_text()), "4567890");
        assert_eq!(g.cursor(), TermPoint { x: 2, y: 0 }, "清屏不移动光标");
    }

    #[test]
    fn csi_j2_full_clear_preserves_cursor() {
        let mut g = grid20();
        g.feed_bytes(b"1234567890\x1b[3G");
        g.feed_bytes(b"\x1b[2J");
        assert_eq!(g.content_text(), "", "J2 全清");
        assert_eq!(g.cursor(), TermPoint { x: 2, y: 0 }, "J2 不清光标");
    }

    #[test]
    fn csi_j3_full_clear_preserves_cursor() {
        let mut g = grid20();
        g.feed_bytes(b"1234567890\x1b[3G");
        g.feed_bytes(b"\x1b[3J");
        assert_eq!(g.content_text(), "", "J3 全清");
        assert_eq!(g.cursor(), TermPoint { x: 2, y: 0 }, "J3 不清光标");
    }

    // ---- TERM-004：按 cell 宽度的 resize / reflow（宽字符绝不被切断） ----

    // cell 网格里宽字符的第二个格是存进 buffer 的占位空格；`content_text` 会原样保留这些空格，
    // 因此比较 reflow 结果时去掉所有空白（含换行），只比对“实字符序列是否一致 / 有无损坏”：
    // reflow 允许在宽字符之间的占位空格存在，也必须保证不含替换符 `�`、且字符顺序不变。
    fn compact(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn resize_reflows_by_cell_width_no_garbage() {
        // `中文abc` = 2+2+3 = 7 格。缩到 5 列（溢出进 scrollback）再放回宽列，
        // 实字符序列应完整还原、无 `�`——reflow 绝不丢弃实内容、绝不把宽字符切断。
        let mut g = grid20();
        g.feed_bytes("中文abc".as_bytes());
        g.resize(5, 5);
        assert!(!g.content_text().contains('\u{fffd}'), "不得出现替换符");
        // 重新拉宽到能容纳全部内容的一行，验证此前的缩小没有丢数据（溢出行已回到可视）。
        g.resize(50, 5);
        assert_eq!(compact(&g.content_text()), "中文abc");
    }

    #[test]
    fn resize_never_splits_wide_char_across_lines() {
        // 4 列放下 `中文`（各 2 格恰好）。缩到 3 列：`中`(2 格)后 `文`(2 格)放不下、
        // 必须整字符换行，绝不能在宽字符中间切断成半个。
        let mut g = TermGrid::new(4, 5);
        g.feed_bytes("中文".as_bytes());
        g.resize(3, 5);
        assert!(!g.content_text().contains('\u{fffd}'), "不得出现替换符");
        // 拉宽回来，两个宽字符都必须完整存在（一旦被切断就会变 `�`）。
        g.resize(50, 5);
        assert_eq!(compact(&g.content_text()), "中文", "宽字符不得被从中间切断");
    }

    #[test]
    fn repeated_resizes_keep_ascii_content_stable() {
        let mut g = grid20();
        g.feed_bytes("hello world from fluxdb terminal".as_bytes());
        g.resize(8, 3);
        g.resize(100, 2);
        g.resize(15, 4);
        assert!(!g.content_text().contains('\u{fffd}'));
        // 最后拉成单行宽度，多次 reflow 后 ASCII 字符序列应无损。
        g.resize(50, 10);
        assert_eq!(compact(&g.content_text()), "helloworldfromfluxdbterminal");
    }

    // ---- TERM-010：Unicode / 坐标换算回归 ----

    #[test]
    fn content_widths_cjk_and_ascii() {
        assert_eq!(char_cell_width('中'), 2);
        assert_eq!(char_cell_width('a'), 1);
        assert_eq!(char_cell_width('😀'), 2);
        assert_eq!(char_cell_width('\u{301}'), 1, "组合附加符号宽 1");
    }

    #[test]
    fn row_real_chars_skips_wide_padding() {
        // `中a` 占 3 格：中(格0-1)、a(格2)。row_real_chars 对前半段应产出 2 个"实字符"，
        // 宽字符的 padding 格（占位空格）不算独立字符。
        let mut g = grid20();
        g.feed_bytes("中a".as_bytes());
        let cells: Vec<char> = g.row_view(0).chars;
        let info = row_real_chars(&cells);
        // 行尾未写到的空白格是填充空格（宽度 1），故总长 >2；只校验“实内容”起始段。
        assert_eq!((info[0].c, info[0].cell, info[0].cell_len), ('中', 0, 2));
        assert_eq!((info[1].c, info[1].cell, info[1].cell_len), ('a', 2, 1));
        assert_eq!(info[1].byte, 3, "中=3 字节，a 从字节 3 起");
    }

    #[test]
    fn byte_cursor_to_cell_maps_redis_byte_cursor() {
        // redis 用字节编辑：在 `中a` 里，字节光标 1（`中` 中间）应吸附到 `中` 起始格 0；
        // 字节光标 3（`a` 前）→ 格 2；末尾字节 4 → 格 3（内容末尾）。
        let mut g = grid20();
        g.feed_bytes("中a".as_bytes());
        let cells: Vec<char> = g.row_view(0).chars;
        assert_eq!(byte_cursor_to_cell(&cells, 1), 0, "宽字符内部吸附到起始格");
        assert_eq!(byte_cursor_to_cell(&cells, 3), 2, "a 的起始格");
        assert_eq!(byte_cursor_to_cell(&cells, 4), 3, "末尾字节 → 内容末尾格");
    }

    #[test]
    fn right_edge_byte_len_mirrors_delete_compensation() {
        // `delete()`（桌面端）按"光标右侧整字的字节数"连发 `ESC[3~`，避免 redis 只删 1 字节
        // 把宽字符切出半字。这里锁定这个契约：光标停在宽字符 `中` 起始字节（16，提示符后），
        // RowCharInfo 应报 `中` 的 byte_len=3（→ 发 3 次 `ESC[3~` 删掉整字），而不是垃圾桶。
        let mut g = TermGrid::new(80, 5);
        g.feed_bytes("中a".as_bytes());
        let cells: Vec<char> = g.row_view(0).chars;
        let info = row_real_chars(&cells);
        // 找到 `中`（byte 0）与 `a`（byte 3）：模拟 redis 光标落在 中 起始字节 0（光标右侧是 中）。
        let mid = info.iter().find(|rc| rc.c == '中').expect("应含 `中`");
        assert_eq!(mid.byte_len, 3, "宽字符 byte_len=3 → Delete 需补 3 个 `ESC[3~`");
        // EdgeSide::Right 的字节区间判断：byte >= cursor_byte 命中 `中` 自身。
        let right = info.iter().find(|rc| rc.byte >= mid.byte).unwrap();
        assert_eq!(right.c, '中');
        // 光标落在 `中` 末尾字节（3）右侧 `a`（byte 3≥3）→ 命中 `a`（byte_len 1）。
        let right_a = info.iter().find(|rc| rc.byte >= 3).unwrap();
        assert_eq!((right_a.c, right_a.byte_len), ('a', 1));
    }

    #[test]
    fn backspace_handles_wide_char_two_cells() {
        // 输入 `中`（占 2 格）后退格，应一次清干净（含第二个 padding 格），光标回列 0。
        let mut g = grid20();
        g.feed_bytes("中".as_bytes());
        assert_eq!(g.cursor().x, 2);
        g.feed_bytes(b"\x08"); // BS
        assert_eq!(g.cursor().x, 0, "宽字符退格回退 2 格");
        assert_eq!(g.content_text(), "", "宽字符两个格都清空");
    }

    #[test]
    fn combining_mark_renders_after_base_char() {
        // e + 组合尖音符 → 视觉一个字符，占 1 格（组合附加符号宽 1），不退格漂移。
        let mut g = grid20();
        g.feed_bytes("e\u{301}".as_bytes());
        assert_eq!(g.content_text(), "e\u{301}");
        assert_eq!(g.cursor().x, 2, "两个窄字符各占 1 格");
    }

    // ---- 端到端：模拟 redis-cli 的字节式行编辑器（实机抓包驱动） ----
    //
    // redis-cli 的 linenoise 按**字节**编辑与回显（7.2 deps/linenoise.c）：
    //   - 每按一个键都会整行重绘：`\r ESC[0K <prompt><line> \r ESC[<byteCusor>C`
    //     （`ESC[0K` 清行、`ESC[nC` 把光标放到“字节位置”，提示符 16 字节）。
    //   - 一个退格 `\x7f` 只删 1 字节；Delete 键 `ESC[3~` 只删光标右侧 1 字节。
    //   - 因此删除多字节字符（中=3B）会切出半个字符 → `�`（这就是上面两个 delete bug 的来源）。
    //
    // 下面用同样的字节刷新协议喂 TermGrid，验证：grid 对 redis 的字节重绘能正确同步、
    // 删除宽字符不会让 grid 越界 / 残留半字 / panic。

    const REDIS_PROMPT: &str = "127.0.0.1:6379> ";

    /// 按 redis 刷新协议把“一个逻辑行 + 字节光标”重绘进 grid。
    /// `buf` 为输入字节（可含无效 UTF-8，如删了 1 字节的广字符），`pos` 为字节光标位置。
    fn redis_refresh(g: &mut TermGrid, buf: &[u8], pos: usize) {
        let mut out = Vec::new();
        out.extend_from_slice(b"\r\x1b[0K");
        out.extend_from_slice(REDIS_PROMPT.as_bytes());
        // redis 直接吐原始字节（可能含半字），grid 的持久化 parser 按 UTF-8 收敛。
        out.extend_from_slice(buf);
        let cursor_byte = REDIS_PROMPT.len() + pos;
        out.extend_from_slice(b"\r\x1b[");
        out.extend_from_slice(cursor_byte.to_string().as_bytes());
        out.extend_from_slice(b"C");
        g.feed_bytes(&out);
    }

    /// 输入一段 UTF-8 文本（逐字节触发 redis 重绘，还原“mid-UTF8 也重绘”的真实行为）。
    fn redis_type(g: &mut TermGrid, text: &str) {
        let mut buf: Vec<u8> = Vec::new();
        for b in text.as_bytes() {
            buf.push(*b);
            redis_refresh(g, &buf, buf.len());
        }
    }

    #[test]
    fn redis_byte_editor_delete_wide_char_leaves_half_char() {
        // 复现 bug：redis 的 Delete 键只删 1 字节，把 `中`（3B）切成 `�`。
        // 这属于 redis 的**真实字节行为**——grid 必须如实呈现（半字应作为替换符出现，
        // 而不是 panic / 越界）。桌面端 delete() 负责在发送层用 `ESC[3~`×字节数 补齐，
        // 让“整字删除”不发生在这里。
        let mut g = grid20();
        redis_type(&mut g, "a"); // 先输入 a，光标字节 1
        // 手动构造：a + 中 + b，光标落在 中 之前（字节 1），按一次 Delete。
        let mut buf = "a中b".as_bytes().to_vec();
        let mut pos = 1usize; // 光标在 'a' 后，'中' 前
        redis_refresh(&mut g, &buf, pos);
        // Delete x1：删掉 '中' 的第 1 字节 → 剩下 a + 2 个残留字节 + b。
        buf.remove(pos);
        redis_refresh(&mut g, &buf, pos);
        // grid 应如实呈现“半字”（替换符），且不越界 / 不 panic。
        assert!(
            g.content_text().contains('\u{fffd}'),
            "redis 字节删除应体现为替换符，实际: {:?}",
            g.content_text()
        );
    }

    #[test]
    fn redis_byte_editor_delete_at_end_no_panic() {
        // 复现 bug 2：在行尾按 Delete（或对空输入退格），redis 什么都不回显
        // （linenoise 守卫 `pos < len`），grid 不应因此越界 / panic / 产生垃圾。
        let mut g = TermGrid::new(80, 5);
        redis_type(&mut g, "中");
        // 光标已到行尾（字节 6）。行尾 Delete：redis 无操作，不重绘 → 只喂一个空刷新验证稳定。
        redis_refresh(&mut g, "中".as_bytes(), 6);
        // 再在“内容末尾边界”反复退格 / 删除字节到空，都不能 panic。
        let mut buf: Vec<u8> = "中".as_bytes().to_vec();
        while !buf.is_empty() {
            buf.pop();
            redis_refresh(&mut g, &buf, buf.len());
        }
        redis_refresh(&mut g, &[], 0);
        // 逐字节清空后，输入行不得残留半字（`�`）。提示符属于行内正常内容，去空白后比对。
        assert!(
            !compact(&g.content_text()).contains('\u{fffd}'),
            "行尾删除到空不得残留垃圾，实际 content: {:?}",
            g.content_text()
        );
    }

    #[test]
    fn redis_byte_editor_move_and_erase_keep_grid_synced() {
        // 方向键 / 退格也是逐字节：整行重绘 + 光标字节位。grid 必须跟着字节光标走，
        // 且不数组越界。这里模拟“输入 你好，光标移到行首，再逐字节退格”。
        let mut g = TermGrid::new(80, 5);
        redis_type(&mut g, "你好");
        let mut buf: Vec<u8> = "你好".as_bytes().to_vec();
        let mut pos = buf.len();
        // 移到行首（逐字节左移重绘）。
        while pos > 0 {
            pos -= 1;
            redis_refresh(&mut g, &buf, pos);
        }
        // 从行首逐字节退格（删到空）。
        while !buf.is_empty() {
            buf.remove(0);
            redis_refresh(&mut g, &buf, 0);
        }
        redis_refresh(&mut g, &[], 0);
        assert!(
            !compact(&g.content_text()).contains('\u{fffd}'),
            "逐字节清空不得留半字: {:?}",
            g.content_text()
        );
    }
}
