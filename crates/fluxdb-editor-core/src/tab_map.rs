//! TabMap：把 fold 坐标展开为 tab 展开后的显示坐标（DM-210，Phase 2）。
//!
//! 显示管线中第二个真正的 [`LayerSnapshot`] 实现：输入 [`FoldPoint`]（折叠后的
//! buffer 展示行列），输出 [`TabPoint`]（`\t` 展开到下一个 tab stop 后的显示列）。
//!
//! TabMap **不改写 buffer 文本**：`\t` 只是显示坐标里的展开，字节偏移不变，编辑 /
//! Tree-sitter / 补全继续用原始字节偏移（与 [`display_map`] 现有语义一致）。
//! 本层只做「同一行内、列到列」的变换——行号原样穿过（`TabPoint.row == FoldPoint.row`），
//! 只把 `FoldPoint.column`（该展示行内字节列）换算成 `TabPoint.column`（显示列）。
//!
//! 行文本来源：TabMap 按 fold 展示行持有其**可视行文本**（折叠已把内部行压缩进占位
//! 符；占位符行无真实文本）。折叠内部行的隐藏内容不参与 tab 展开。grapheme 宽字符的
//! 像素宽度由 shaping/wrap 层统一处理，TabMap 只做 tab 的固定 tab-stop 展开，不猜字体
//! 宽度（设计 8.3）。
//!
//! 本层独立自包含展示层，仅依赖 `coordinates` / `model` / `layer::LayerSnapshot`，
//! 不依赖任何 UI 类型。DisplayMap 门面冻结至 DM-228，故本层暂不接入，作为独立
//! Snapshot 供下游 WrapMap 与 DM-228 统一时消费。

use std::rc::Rc;

use crate::coordinates::{Biased, FoldPoint, TabPoint};
use crate::layer::{LayerPatch, LayerSnapshot};
use crate::sum_tree::{IntervalSummary, SumTree, SumTreeItem, Summary};

/// 一个 fold 展示行的可视行描述。
///
/// `text` 是该展示行在 tab 展开前的可视文本（不含换行）；折叠占位符行无真实文本
/// （`text` 为空 + `is_placeholder` 为真），其占位符宽度内不做 tab 展开。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabLine {
    /// 该展示行的可视文本（不含换行）。placeholder 行为空串。
    pub text: Rc<str>,
    /// 是否折叠占位符展示行（无真实文本，不参与 tab 展开）。
    pub is_placeholder: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TabLineSummary {
    start: usize,
    end: usize,
    width: usize,
}

impl Summary for TabLineSummary {
    fn add(&self, other: &Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            width: self.width.saturating_add(other.width),
        }
    }
}

impl IntervalSummary for TabLineSummary {
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TabLineItem {
    line: TabLine,
    width: usize,
}

impl SumTreeItem for TabLineItem {
    type Summary = TabLineSummary;

    fn summary(&self) -> Self::Summary {
        TabLineSummary {
            start: 0,
            end: 1,
            width: self.width,
        }
    }
}

/// tab 展开后的显示层不可变快照。
#[derive(Clone)]
pub struct TabSnapshot {
    /// 绑定的输入（上游 fold 快照）版本号，构建时记录。
    input_version: u64,
    /// 本层自身 revision：由 input_version + tab_size 派生（FNV-1a）。**tab size
    /// 改变只翻转本层 revision**，下游按它失效；行文本变化属上游 input_version。
    revision: u64,
    /// tab stop 宽度（`\t` 展开到该宽度的整数倍）。
    tab_size: usize,
    /// 每个 fold 展示行的可视行。持久树只复制 dirty path。
    lines: SumTree<TabLineItem>,
    identity_rows: Option<usize>,
}

impl TabSnapshot {
    /// 构造。`tab_size` 至少为 1（0 视为 1）。`input_version` 取构建时上游 fold
    /// 快照版本。
    pub fn new(input_version: u64, tab_size: usize, lines: Vec<TabLine>) -> Self {
        let tab_size = tab_size.max(1);
        let revision = compute_revision(input_version, tab_size);
        let items: Vec<TabLineItem> = lines
            .into_iter()
            .map(|line| {
                let width = if line.is_placeholder {
                    0
                } else {
                    text_display_width(&line.text, tab_size)
                };
                TabLineItem { line, width }
            })
            .collect();
        Self {
            input_version,
            revision,
            tab_size,
            lines: SumTree::from_items(&items),
            identity_rows: None,
        }
    }

    /// 轻量恒等 Tab 层。真实行文本和 tab 宽度在 DisplayMap 首次需要精确布局时填充。
    pub fn identity(input_version: u64, row_count: usize, tab_size: usize) -> Self {
        Self {
            input_version,
            revision: compute_revision(input_version, tab_size.max(1)),
            tab_size: tab_size.max(1),
            lines: SumTree::default(),
            identity_rows: Some(row_count.max(1)),
        }
    }

    /// 当前 tab stop 宽度。
    pub fn tab_size(&self) -> usize {
        self.tab_size
    }

    pub fn row_count(&self) -> usize {
        self.identity_rows
            .unwrap_or_else(|| self.lines.leaf_count())
    }

    /// 某展示行的显示总宽度（含 tab 展开）。placeholder 行宽度为 0。
    pub fn line_display_width(&self, row: usize) -> usize {
        if self.identity_rows.is_some() {
            return 0;
        }
        self.lines.get(row).map_or(0, |line| line.width)
    }

    /// 某展示行可视文本（只读）。
    pub fn line_text(&self, row: usize) -> &str {
        if self.identity_rows.is_some() {
            return "";
        }
        self.lines
            .get_ref(row)
            .map_or("", |line| line.line.text.as_ref())
    }

    /// 增量替换受影响的 fold 展示行。文本通过 `Rc` 共享，未触碰的行不复制内容；
    /// 行数变化时只拼接 prefix/suffix，供 DisplayMap 把 dirty patch 向下传播。
    pub fn sync_rows(
        &self,
        input_version: u64,
        old_range: std::ops::Range<usize>,
        inserted: Vec<TabLine>,
    ) -> Self {
        if let Some(rows) = self.identity_rows {
            let rows = rows
                .saturating_sub(old_range.end.saturating_sub(old_range.start))
                .saturating_add(inserted.len());
            return Self::identity(input_version, rows, self.tab_size);
        }
        let inserted: Vec<TabLineItem> = inserted
            .into_iter()
            .map(|line| {
                let width = if line.is_placeholder {
                    0
                } else {
                    text_display_width(&line.text, self.tab_size)
                };
                TabLineItem { line, width }
            })
            .collect();
        let start = old_range.start.min(self.lines.leaf_count());
        let end = old_range.end.max(start).min(self.lines.leaf_count());
        Self {
            input_version,
            revision: compute_revision(input_version, self.tab_size),
            tab_size: self.tab_size,
            lines: self.lines.replace_leaves(start, end, &inserted),
            identity_rows: None,
        }
    }

    /// 同步 Fold 行并返回 Tab 坐标空间中的 dirty edit。
    pub fn sync_rows_with_patch(
        &self,
        input_version: u64,
        old_range: std::ops::Range<usize>,
        inserted: Vec<TabLine>,
    ) -> (Self, LayerPatch) {
        let new_range = old_range.start..old_range.start + inserted.len();
        let next = self.sync_rows(input_version, old_range.clone(), inserted);
        (next, LayerPatch::single(old_range, new_range))
    }
}

impl LayerSnapshot for TabSnapshot {
    type Input = FoldPoint;
    type Output = TabPoint;

    fn input_version(&self) -> u64 {
        self.input_version
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn map_input_by(&self, input: FoldPoint) -> Biased<TabPoint> {
        let FoldPoint { row, column } = input;
        if self.identity_rows.is_some() {
            return Biased::from(TabPoint { row, column });
        }
        let Some(line) = self.lines.get(row) else {
            return Biased::from(TabPoint { row, column });
        };
        // 占位符行：无真实文本，不展开 tab，占位符显示列原样穿过。
        if line.line.is_placeholder {
            return Biased::from(TabPoint { row, column });
        }
        let text = line.line.text.as_ref();
        // `column` 是字节列：取该字节列之前文本的显示宽度（tab 展开）。
        // `display_width_prefix` 用 `index >= byte` 在字符边界打断，字节列在
        // UTF-8 中间字节（宽字符内部）时也安全（在该字符起点拦截）。
        let base = display_width_prefix(text, column, self.tab_size);
        // 字节列是否正落在某个制表符上（列恰为某字符起点且该字符是 \t）：
        // 是则偏置到 tab 前后（base ↔ 下一个 tab stop）。
        let at_tab = text
            .char_indices()
            .find(|(index, _)| *index == column)
            .map_or(false, |(_, c)| c == '\t');
        if at_tab {
            return Biased {
                left: TabPoint { row, column: base },
                right: TabPoint {
                    row,
                    column: (base / self.tab_size + 1) * self.tab_size,
                },
            };
        }
        Biased::from(TabPoint { row, column: base })
    }

    fn map_output_by(&self, output: TabPoint) -> Biased<FoldPoint> {
        let TabPoint { row, column } = output;
        let Some(line) = self.lines.get(row) else {
            return Biased::from(FoldPoint { row, column });
        };
        // 占位符行：显示列原样穿回 fold 列。
        if line.line.is_placeholder {
            return Biased::from(FoldPoint { row, column });
        }
        let text = line.line.text.as_ref();
        // 显示列 → 字节列：落在 tab 展开区间内 → 左右偏置到 tab 前 / tab 后字节。
        byte_for_display_column(text, column, self.tab_size)
            .map(|byte| Biased {
                left: FoldPoint {
                    row,
                    column: byte.0,
                },
                right: FoldPoint {
                    row,
                    column: byte.1,
                },
            })
            .unwrap_or_else(|| {
                Biased::from(FoldPoint {
                    row,
                    column: text.len(),
                })
            })
    }
}

/// 派生本层 revision：FNV-1a 混合 input_version 与 tab_size（DM-212）。
///
/// 行文本变化反映在上游 input_version；**tab size 单独混入**，故改 tab size 时即使
/// input_version 不变，本层 revision 也翻转，使下游（WrapMap/DisplayMap）只按 TabMap
/// 的 revision 失效、无需连带失效上游。同 (input_version, tab_size) → 同 revision。
fn compute_revision(input_version: u64, tab_size: usize) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100000001b3);
    mix(input_version);
    mix(tab_size as u64);
    h
}

/// 单个字符占用的展示列数。
///
/// 与等宽字体/终端口径一致：CJK、全角字符与 emoji 占 2 列，组合符占 0 列，
/// 其余占 1 列。`\t` 的展开依赖当前位置，由调用方按 tab stop 单独处理；
/// 控制字符宽度未知，沿用历史口径按 1 列计。
pub fn char_display_width(character: char) -> usize {
    match character {
        '\t' => 1,
        _ => unicode_width::UnicodeWidthChar::width(character).unwrap_or(1),
    }
}

/// 一行文本（已含 tab）展开后的总显示列宽（DM-210，从 display_map 迁移）。
pub fn text_display_width(text: &str, tab_size: usize) -> usize {
    let tab_size = tab_size.max(1);
    let mut column = 0usize;
    for character in text.chars() {
        if character == '\t' {
            column = (column / tab_size + 1) * tab_size;
        } else {
            column += char_display_width(character);
        }
    }
    column
}

/// 取文本 `[..byte]` 前缀的显示列宽（tab 展开）。
pub fn display_width_prefix(text: &str, byte: usize, tab_size: usize) -> usize {
    let tab_size = tab_size.max(1);
    let mut column = 0usize;
    for (index, character) in text.char_indices() {
        if index >= byte {
            break;
        }
        if character == '\t' {
            column = (column / tab_size + 1) * tab_size;
        } else {
            column += char_display_width(character);
        }
    }
    column
}

/// 显示列 → 字节列（DM-211，从 display_map 迁移 + 补 bias）。
///
/// 返回 `Option<(byte_before, byte_after)>`：显示列精确落在某制表符展开区间的起点
/// 或内部时，左右候选为 tab 前 / tab 后字节；否则两候选相同（普通字符边界）。
/// 越界（显示列 ≥ 行尾）返回 `None` 表示应钳到行尾。
fn byte_for_display_column(text: &str, target: usize, tab_size: usize) -> Option<(usize, usize)> {
    let tab_size = tab_size.max(1);
    let mut column = 0usize;
    for (byte, character) in text.char_indices() {
        if column > target {
            break;
        }
        if column == target {
            // 边界正好在字节 `byte` 起点：该字节若是 tab，偏置到 tab 前/后。
            return Some(if character == '\t' {
                (byte, byte + character.len_utf8())
            } else {
                (byte, byte)
            });
        }
        let next = if character == '\t' {
            (column / tab_size + 1) * tab_size
        } else {
            column + char_display_width(character)
        };
        if next > target {
            // 目标列落在此字符展开区间内部 → 只有 tab 才有非零宽展开区间。
            return Some(if character == '\t' {
                (byte, byte + character.len_utf8())
            } else {
                (byte, byte)
            });
        }
        column = next;
    }
    // 目标列到达或越过行尾。
    if target >= column {
        return None;
    }
    // 理论上到不了（column <= target 恒成立）；兜底。
    Some((text.len(), text.len()))
}
