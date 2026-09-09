// ANSI / VT 解析：把 PTY 字节流喂进 `TermGrid`（作为 `fluxdb_core::terminal` 的子文件被 include）。
//
// 基于 `vte` crate。覆盖 redis-cli / 常见 CLI 交互所需的子集：
//   - 可见字符 / 换行 / 回车 / 退格 / 制表
//   - 清屏（J）/ 清行（K）/ 光标移动（A-F/G-H）/ 保存恢复光标（ESC 7/8）
//   - SGR（m）：reset / bold / 16 色 / 256 色 / 24 位色，映射到 cell 的 fg/bg
//   - 备用屏切换（CSI ?1049 h/l），供全屏类程序使用
//   - OSC（设置标题等）`put` 存起来，供上层取用
// 不追求完整 xterm；grid + parser 的边界为后续全量 ANSI 预留。

use vte::{Params, Perform};

// `TermGrid` 定义于 grid.rs，随模块 include 共享命名空间，此处无需 import。

/// 把一段字节流解析进网格（透传给 `TermGrid::feed_bytes`，由后者复用持久化 parser 状态）。
pub fn feed_bytes(grid: &mut TermGrid, bytes: &[u8]) {
    grid.feed_bytes(bytes);
}

/// 单个 UTF-8 序列不适合直接逐字节拿 char，`Perform::print(c)` 已由 vte 按 UTF-8 解出 char。
struct GridPerformer<'a> {
    grid: &'a mut TermGrid,
}

impl Perform for GridPerformer<'_> {
    fn print(&mut self, c: char) {
        self.grid.print_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte as char {
            '\n' => self.grid.linefeed(),
            '\r' => self.grid.carriage_return(),
            '\u{8}' => self.grid.backspace(), // BS
            '\u{7f}' => self.grid.backspace(), // DEL（linenoise 退格回显）
            '\t' => {
                // 前进到下一个 8 列制表位（在制表位上的光标净前进 8，含自身已就位时为 0 由终端
                // 语义决定——这里统一按“前进到下一制表位”处理，上一版 `.min(1)` 会把 8 夹成 1）。
                let x = self.grid.cursor().x as usize;
                let advance = 8 - x % 8;
                self.grid.cursor_forward(advance as u16);
            }
            '\u{c}' => self.grid.clear_screen(), // FF
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, action: char) {
        // vte 0.14 把分号分隔的值以“子参数”形式存放在一个参数里，故展平为连续的 u16 序列。
        let ps: Vec<u16> = params.iter().flat_map(|p| p.iter().copied()).collect();
        let default = || ps.first().copied().unwrap_or(1).max(1) as u16;
        match action {
            'A' => self.grid.cursor_up(default()),
            'B' => self.grid.cursor_down(default()),
            'C' => self.grid.cursor_forward(default()),
            'D' => self.grid.cursor_back(default()),
            'E' => {
                self.grid.cursor_down(default());
                self.grid.carriage_return();
            }
            'F' => {
                self.grid.cursor_up(default());
                self.grid.carriage_return();
            }
            'G' => {
                let x = ps.first().copied().unwrap_or(1).max(1) as u16;
                self.grid.set_cursor((x - 1).min(self.grid.cols() - 1), self.grid.cursor().y);
            }
            'H' | 'f' => {
                // 行;列（1 基）。
                let row = ps.first().copied().unwrap_or(1).max(1) as u16;
                let col = ps.get(1).copied().unwrap_or(1).max(1) as u16;
                self.grid
                    .set_cursor((col - 1).min(self.grid.cols() - 1), (row - 1).min(self.grid.rows() - 1));
            }
            'J' => {
                // CSI J：0=从光标清到屏幕末尾，1=从屏幕开头清到光标，2/3=全清。
                // 清屏应保留当前光标位置（清屏与“光标复位到原点”是两件事，上一版混在一起
                // 导致 `\x1b[J`/`\x1b[2J` 后光标被拉回 (0,0)，redis-cli 重绘提示符会错位）。
                match ps.first().copied().unwrap_or(0) {
                    0 => self.grid.clear_from_cursor_to_end(),
                    1 => self.grid.clear_to_cursor(),
                    2 | 3 => self.grid.clear_all_preserve(),
                    _ => {}
                }
            }
            'K' => match ps.first().copied().unwrap_or(0) {
                0 => self.grid.erase_to_eol(),
                2 => self.grid.erase_line(),
                _ => {}
            },
            'm' => self.apply_sgr(&ps),
            'h' | 'l' => {
                // 备用屏开关在 CSI ? ... h/l（参数里带 1049）。
                if ps.contains(&1049) {
                    if action == 'h' {
                        self.grid.enter_alternate();
                    } else {
                        self.grid.leave_alternate();
                    }
                }
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // 首期忽略 OSC（窗口标题等），标题由 adapter 提供。
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte as char {
            // 保存 / 恢复光标。
            '\u{37}' => self.grid.save_cursor(), // ESC 7
            '\u{38}' => self.grid.restore_cursor(), // ESC 8
            _ => {}
        }
    }
}

impl GridPerformer<'_> {
    /// SGR：把 ANSI 颜色 / 加粗写进当前光标单元格样式（供后续字符着色）。
    fn apply_sgr(&mut self, params: &[u16]) {
        // 当前光标处的样式作为“后续字符串”的 fg/bg。
        let cell = self.grid.screen_cell(self.grid.cursor().y as usize, self.grid.cursor().x as usize);
        let mut fg = cell.fg;
        let mut bg = cell.bg;
        let mut bold = cell.bold;

        let mut i = 0;
        let ps: Vec<u16> = if params.is_empty() { vec![0] } else { params.to_vec() };
        while i < ps.len() {
            let p = ps[i];
            match p {
                0 => {
                    fg = None;
                    bg = None;
                    bold = false;
                }
                1 | 22 => bold = p == 1,
                30..=37 => fg = Some(ansi_color(p as u8 - 30, false)),
                90..=97 => fg = Some(ansi_color(p as u8 - 90, true)),
                39 => fg = None,
                40..=47 => bg = Some(ansi_color(p as u8 - 40, false)),
                100..=107 => bg = Some(ansi_color(p as u8 - 100, true)),
                49 => bg = None,
                38 | 48 => {
                    // 38;5;n or 38;2;r;g;b
                    let is_fg = p == 38;
                    if let Some(sub) = ps.get(i + 1) {
                        match *sub {
                            5 => {
                                if let Some(n) = ps.get(i + 2) {
                                    let c = xterm_256(*n as u16);
                                    if is_fg {
                                        fg = Some(c);
                                    } else {
                                        bg = Some(c);
                                    }
                                    i += 2;
                                }
                            }
                            2 => {
                                if let (Some(r), Some(g), Some(b)) =
                                    (ps.get(i + 2), ps.get(i + 3), ps.get(i + 4))
                                {
                                    let c = (*r as u8, *g as u8, *b as u8);
                                    if is_fg {
                                        fg = Some(c);
                                    } else {
                                        bg = Some(c);
                                    }
                                    i += 4;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        // 记录“下一个打印字符”应使用的样式：暂存到网格的 pending 样式。
        self.grid.set_pending_style(fg, bg, bold);
    }
}

/// 16 色（含明亮）到 RGB。
fn ansi_color(idx: u8, bright: bool) -> (u8, u8, u8) {
    // 标准 16 色调色板（xterm）。
    const COLORS: [(u8, u8, u8); 8] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
    ];
    let base = COLORS[idx as usize % 8];
    if bright {
        match base {
            (0, 0, 0) => (127, 127, 127),
            (229, 229, 229) => (255, 255, 255),
            other => {
                let (r, g, b) = other;
                (r.saturating_add(60).min(255), g.saturating_add(60).min(255), b.saturating_add(60).min(255))
            }
        }
    } else {
        base
    }
}

/// xterm 256 色调色板。
fn xterm_256(n: u16) -> (u8, u8, u8) {
    match n {
        0..=7 => ansi_color(n as u8, false),
        8..=15 => ansi_color(n as u8 - 8, true),
        16..=231 => {
            let v = n - 16;
            let r = v / 36;
            let g = (v % 36) / 6;
            let b = v % 6;
            let cube = |x: u16| -> u8 {
                match x {
                    0 => 0,
                    _ => (55 + x * 40) as u8,
                }
            };
            (cube(r), cube(g), cube(b))
        }
        232..=255 => {
            let v = n - 232;
            let c = (8 + v * 10) as u8;
            (c, c, c)
        }
        _ => (0, 0, 0),
    }
}

// （`TermRowView` / `TermCell` 定义于 grid.rs，随模块 include 共享命名空间。）
