//! # WrapMap：从单体 DisplayMap 拆出的软换行层（DM-220）。
//!
//! 第三层 `LayerSnapshot`：`Input=TabPoint`（tab 展开后的展示列坐标），
//! `Output=WrapPoint`（可视/软换行行坐标）。只负责「源行 → 可视片段」的
//! 整数映射：维护每源行的可视片段数与前缀和，不做驱动 UI 的 measurement。
//!
//! DM-220 阶段机械迁移当前 display_map 的 UTF-16 固定列切分语义
//! （`column_fragment_count`/`fragment_columns`/`visual_row_for_column`），
//! 门面冻结期间不接线；DM-223 再把宽度启发式换成真实 shaping 断点
//! （`LineBreaker`），DM-228 做门面组合。层本身零依赖、hold 纯整数摘要，
//! 字节列↔展示列换算（需行文本）交由上层/消费侧完成。

use std::rc::Rc;

use crate::coordinates::{Biased, TabPoint, WrapPoint};
use crate::layer::{LayerPatch, LayerSnapshot};
use crate::sum_tree::{SumTree, TextSummary};

/// 折行 measurement 接口（DM-222）：core 零依赖、由 UI 侧注入实现。
///
/// 把「一行文本在给定字体与像素宽度下断成哪几段」的测量能力抽象出来，
/// core 只声明契约，不依赖 GPUI/字体堆栈；UI 侧用现有 `LineWrapper`
/// 实现并注入 `WrapSnapshot`。`wrap_line` 结果按文本**字节偏移**返回断点，
/// 应保证不断到字符中间（grapheme/宽字符/emoji 兼容），并覆盖 word/CJK 边界
/// 基础场景。后台 shaping 需跨线程，故要求 `Send + Sync`。
pub trait LineBreaker: Send + Sync {
    /// 返回折行断点的字节偏移（不含 0 与行尾；升序唯一的中间断点）。
    ///
    /// `text` 为待折行来源行可见文本（tab 已展开的展示列空间由调用方换算）；
    /// `wrap_width_px` 为允许像素宽。实现不得返回空白（0 与 `text.len()`）。
    fn break_line(&self, text: &str, wrap_width_px: f32) -> Vec<usize>;
}

/// WrapMap 运行配置（DM-222/223）：UI 侧构建时一次性注入。
///
/// `font_size_px/font_key` 供 DM-224 按行缓存 key 与 revision 区分；`tab_size`
/// 供展示列↔字节换算。
#[derive(Clone, Copy, Debug)]
pub struct WrapConfig {
    /// 允许的折行像素宽（真实 shaping 用）。
    pub wrap_width_px: f32,
    /// 固定 UTF-16 列宽回退（Non-shaping 或估算路径）。
    pub wrap_width_utf16: usize,
    /// 字体像素大小（缓存/measurement 参数）。
    pub font_size_px: f32,
    /// 字体标识（缓存 key 去重；UI 可给字体系列哈希或句柄地址）。
    pub font_key: u64,
    /// tab 宽度（展示列单位）。
    pub tab_size: usize,
}

/// 软换行模式（与 display_map::SoftWrap 对齐的最小集）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoftWrap {
    #[default]
    None,
    EditorWidth,
}

/// 单个可视（软换行）片段的展示列区间。
///
/// `col_start/col_end` 是 **UTF-16 展示列**（tab 已展开的列），DM-220 阶段由
/// 固定宽度除法得到；DM-223 换真实 shaping 断点后仍保持展示列语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrapLine {
    /// 所属源行（TabPoint.row）。
    pub row: usize,
    /// 该片段在源行内起始展示列（含）。
    pub col_start: usize,
    /// 该片段结束展示列（不含，末片段截断到行尾）。
    pub col_end: usize,
    /// 是否源行首片段。
    pub first_fragment: bool,
}

/// WrapMap 层快照：源行(TabPoint) → 可视行(WrapPoint)。
///
/// 视觉行索引的唯一事实来源是 [`SumTree`]，每源行一条摘要：
/// `lines`=该源行可视片段数，`utf16`=该源行展示列宽（post-tab，用于末片段
/// 截断/反向列换算）。前缀和给出 `first_visual_row`，`locate_lines` 给出
/// visual row → 源行 的反向，均不保留全文 `Vec<VisualLine>`。
#[derive(Clone)]
pub struct WrapSnapshot {
    input_version: u64,
    revision: u64,
    wrap_width: usize,
    soft_wrap: SoftWrap,
    /// 每源行 → 可视片段数(lines) + 展示列宽(utf16)。
    lines: SumTree<TextSummary>,
    /// 无软换行时的恒等行数；避免为每行构造摘要树。
    identity_rows: Option<usize>,
    estimated_rows: Option<usize>,
    /// 每源行的精确片段展示列区间（DM-223 真实 shaping 断点产物）。
    ///
    /// 仅当通过 `from_breakpoints` 提供时为 `Some`；固定宽切分路径为 `None`
    /// （仍走 `visual_line_at` 内的除法回退）。`Vec` 并行数组只为降低实现
    /// 复杂度，视口内裁剪/缓存见 DM-224/DM-227（`ponytail:` 现为整行持有）。
    frag_columns: Option<Vec<Vec<(usize, usize)>>>,
}

impl WrapSnapshot {
    /// 从「每源行的可视片段数与展示列宽」构建（机械迁移，DM-220）。
    ///
    /// `wrap_width` 参与 revision 派生与片段切分。
    pub fn new(
        input_version: u64,
        soft_wrap: SoftWrap,
        wrap_width: usize,
        tab_size: usize,
        lines: &[TextSummary],
    ) -> Self {
        let wrap_width = wrap_width.max(1);
        let tab_size = tab_size.max(1);
        let lines = SumTree::from_summaries(lines);
        let revision = compute_revision(input_version, wrap_width, tab_size, &lines);
        Self {
            input_version,
            revision,
            wrap_width,
            soft_wrap,
            lines,
            identity_rows: None,
            estimated_rows: None,
            frag_columns: None,
        }
    }

    /// 便捷构造（DM-223）：从「每源行精确片段展示列区间」构建。
    ///
    /// 片段列区间由 UI 侧 `LineBreaker` 对 tab 展开文本实测 shaping 得出
    /// （每片段字节断点 + 累计展示列宽换算）；`wrap_width/tab_size` 仅参与
    /// revision 派生与回退切分。col 区间须升序、相邻、首片段从 0 起、
    /// 末片段截断到 `display_width`。
    pub fn from_breakpoints(
        input_version: u64,
        wrap_width: usize,
        tab_size: usize,
        display_widths: &[usize],
        breakpoints: &[Vec<(usize, usize)>],
    ) -> Self {
        let wrap_width = wrap_width.max(1);
        let tab_size = tab_size.max(1);
        let summaries: Vec<TextSummary> = display_widths
            .iter()
            .zip(breakpoints.iter())
            .map(|(&w, frags)| TextSummary {
                lines: frags.len().max(1),
                utf16: w,
                ..TextSummary::default()
            })
            .collect();
        let lines = SumTree::from_summaries(&summaries);
        let revision = compute_revision(input_version, wrap_width, tab_size, &lines);
        Self {
            input_version,
            revision,
            wrap_width,
            soft_wrap: SoftWrap::EditorWidth,
            lines,
            identity_rows: None,
            estimated_rows: None,
            frag_columns: Some(breakpoints.to_vec()),
        }
    }

    /// 无软换行的恒等层。行数由上游提供，避免构造 O(行数) 摘要树。
    pub fn identity(input_version: u64, row_count: usize, tab_size: usize) -> Self {
        let rows = row_count.max(1);
        Self {
            input_version,
            revision: compute_identity_revision(input_version, rows, tab_size.max(1)),
            wrap_width: 1,
            soft_wrap: SoftWrap::None,
            lines: SumTree::default(),
            identity_rows: Some(rows),
            estimated_rows: None,
            frag_columns: None,
        }
    }

    /// 轻量软换行占位层：先按每个源行一条可视行估算，精确断点由 DisplayMap 延迟构造。
    pub fn estimated(
        input_version: u64,
        row_count: usize,
        wrap_width: usize,
        tab_size: usize,
    ) -> Self {
        Self {
            input_version,
            revision: compute_identity_revision(input_version, row_count.max(1), tab_size.max(1)),
            wrap_width: wrap_width.max(1),
            soft_wrap: SoftWrap::EditorWidth,
            lines: SumTree::default(),
            identity_rows: None,
            estimated_rows: Some(row_count.max(1)),
            frag_columns: None,
        }
    }

    /// 便捷构造：按固定宽切分算法从「每源行展示列宽」推导片段数（UTF-16 近似，
    /// 与 display_map::rebuild 的 EditorWidth 分支同构）。
    pub fn from_display_widths(
        input_version: u64,
        wrap_width: usize,
        tab_size: usize,
        display_widths: &[usize],
    ) -> Self {
        let wrap_width = wrap_width.max(1);
        let summaries: Vec<TextSummary> = display_widths
            .iter()
            .map(|&w| TextSummary {
                lines: fragment_count(w, wrap_width),
                utf16: w,
                ..TextSummary::default()
            })
            .collect();
        Self::new(
            input_version,
            SoftWrap::EditorWidth,
            wrap_width,
            tab_size,
            &summaries,
        )
    }

    /// 只替换受编辑影响的源行摘要。`SumTree::replace_leaves` 对 dirty path 做
    /// path-copy，前缀和后缀子树继续共享；换行导致行数变化时也只在边界处分裂。
    pub fn sync_rows(
        &self,
        input_version: u64,
        tab_size: usize,
        old_range: std::ops::Range<usize>,
        display_widths: &[usize],
    ) -> Self {
        if let Some(rows) = self.identity_rows.or(self.estimated_rows) {
            let rows = rows
                .saturating_sub(old_range.end.saturating_sub(old_range.start))
                .saturating_add(display_widths.len())
                .max(1);
            return if self.identity_rows.is_some() {
                Self::identity(input_version, rows, tab_size)
            } else {
                Self::estimated(input_version, rows, self.wrap_width, tab_size)
            };
        }
        let start = old_range.start.min(self.lines.leaf_count());
        let end = old_range.end.max(start).min(self.lines.leaf_count());
        let summaries: Vec<TextSummary> = display_widths
            .iter()
            .map(|&width| TextSummary {
                lines: fragment_count(width, self.wrap_width),
                utf16: width,
                ..TextSummary::default()
            })
            .collect();
        let lines = self.lines.replace_leaves(start, end, &summaries);
        let revision = compute_revision(input_version, self.wrap_width, tab_size, &lines);
        let frag_columns = self.frag_columns.as_ref().map(|old| {
            let start = old_range.start.min(old.len());
            let end = old_range.end.max(start).min(old.len());
            let mut next = Vec::with_capacity(
                old.len()
                    .saturating_sub(end.saturating_sub(start))
                    .saturating_add(display_widths.len()),
            );
            next.extend_from_slice(&old[..start]);
            next.extend(
                display_widths
                    .iter()
                    .map(|&width| fallback_ranges(width, self.wrap_width)),
            );
            next.extend_from_slice(&old[end..]);
            next
        });
        Self {
            input_version,
            revision,
            wrap_width: self.wrap_width,
            soft_wrap: self.soft_wrap,
            lines,
            identity_rows: None,
            estimated_rows: None,
            frag_columns,
        }
    }

    /// 同步源行并返回该层 old/new visual-row dirty patch，供上游继续 compose。
    pub fn sync_rows_with_patch(
        &self,
        input_version: u64,
        tab_size: usize,
        old_range: std::ops::Range<usize>,
        display_widths: &[usize],
    ) -> (Self, LayerPatch) {
        let old_start = self.first_visual_row(old_range.start);
        let old_end = self.first_visual_row(old_range.end);
        let next = self.sync_rows(input_version, tab_size, old_range.clone(), display_widths);
        let new_start = next.first_visual_row(
            old_range
                .start
                .min(next.source_row_count().saturating_sub(1)),
        );
        let new_end = next.first_visual_row(
            old_range
                .start
                .saturating_add(display_widths.len())
                .min(next.source_row_count()),
        );
        (
            next,
            LayerPatch::single(
                old_start..old_end.max(old_start + 1),
                new_start..new_end.max(new_start + 1),
            ),
        )
    }

    pub fn soft_wrap(&self) -> SoftWrap {
        self.soft_wrap
    }

    pub fn wrap_width(&self) -> usize {
        self.wrap_width
    }

    pub fn source_row_count(&self) -> usize {
        self.identity_rows
            .or(self.estimated_rows)
            .unwrap_or_else(|| self.lines.leaf_count())
    }

    /// 可视行（visual row）总数。
    pub fn visual_row_count(&self) -> usize {
        if let Some(rows) = self.identity_rows {
            return rows;
        }
        if let Some(rows) = self.estimated_rows {
            return rows;
        }
        if matches!(self.soft_wrap, SoftWrap::None) {
            self.lines.leaf_count().max(1)
        } else {
            self.lines.total().lines.max(1)
        }
    }

    /// 源行 → 首个可视行号。
    pub fn first_visual_row(&self, row: usize) -> usize {
        if self.identity_rows.is_some() || self.estimated_rows.is_some() {
            return row.min(self.visual_row_count().saturating_sub(1));
        }
        if matches!(self.soft_wrap, SoftWrap::None) {
            row
        } else {
            self.lines.summary_before_leaf(row).lines
        }
    }

    /// 源行 → (可视片段数)。
    pub fn row_fragment_count(&self, row: usize) -> usize {
        if self.identity_rows.is_some() || self.estimated_rows.is_some() {
            return usize::from(row < self.visual_row_count());
        }
        if row >= self.lines.leaf_count() {
            return 0;
        }
        let before = self.lines.summary_before_leaf(row).lines;
        let after = self.lines.summary_before_leaf(row + 1).lines;
        after.saturating_sub(before)
    }

    /// 源行 → 展示列宽（post-tab，UTF-16）。
    pub fn row_display_width(&self, row: usize) -> usize {
        if self.identity_rows.is_some() || self.estimated_rows.is_some() {
            return 0;
        }
        if row >= self.lines.leaf_count() {
            0
        } else {
            self.lines.summary_before_leaf(row + 1).utf16
                - self.lines.summary_before_leaf(row).utf16
        }
    }

    /// 可视行号 → 源行。
    pub fn buffer_row_for_visual(&self, visual_row: usize) -> usize {
        if self.identity_rows.is_some() || self.estimated_rows.is_some() {
            return visual_row.min(self.visual_row_count().saturating_sub(1));
        }
        if matches!(self.soft_wrap, SoftWrap::None) {
            return visual_row.min(self.lines.leaf_count().saturating_sub(1));
        }
        let (row, _) = self.lines.locate_lines(visual_row);
        row
    }

    /// 可视行号 → 单个可视片段（展示列区间；优先精确 shaping 断点，否则固定宽切分）。
    pub fn visual_line_at(&self, visual_row: usize) -> Option<WrapLine> {
        if visual_row >= self.visual_row_count() {
            return None;
        }
        let row = self.buffer_row_for_visual(visual_row);
        let first = self.first_visual_row(row);
        let count = self.row_fragment_count(row);
        let width = self.row_display_width(row);
        let frag = visual_row.saturating_sub(first);
        // DM-223：真实 shaping 断点下逐片段列区间精确给出（含 CJK/emoji 宽字符）。
        if let Some(frags) = self.frag_columns.as_ref().and_then(|f| f.get(row)) {
            if let Some(&(cs, ce)) = frags.get(frag) {
                return Some(WrapLine {
                    row,
                    col_start: cs,
                    col_end: ce.max(cs),
                    first_fragment: frag == 0,
                });
            }
        }
        // 回退：固定宽切分（`from_display_widths` 路径 / 无精确断点的末片段兜底）。
        if count <= 1 {
            return Some(WrapLine {
                row,
                col_start: 0,
                col_end: width,
                first_fragment: true,
            });
        }
        let cs = frag * self.wrap_width;
        let ce = ((frag + 1) * self.wrap_width).min(width);
        Some(WrapLine {
            row,
            col_start: cs,
            col_end: ce.max(cs),
            first_fragment: frag == 0,
        })
    }

    /// 源行内展示列 → 所在可视行（优先精确 shaping 断点，越界裁剪到末片段）。
    pub fn visual_row_for_column(&self, row: usize, display_column: usize) -> usize {
        let first = self.first_visual_row(row);
        let count = self.row_fragment_count(row);
        if count <= 1 || matches!(self.soft_wrap, SoftWrap::None) {
            return first;
        }
        // DM-223：按精确片段区间定位（而非固定宽除法）——宽字符列归属正确。
        if let Some(frags) = self.frag_columns.as_ref().and_then(|f| f.get(row)) {
            for (i, &(_, ce)) in frags.iter().enumerate() {
                if display_column < ce {
                    return first + i;
                }
            }
            return first + frags.len() - 1;
        }
        let frag = (display_column / self.wrap_width).min(count - 1);
        first + frag
    }

    /// 指定可视行区间内的片段（供前端只渲染 viewport）。
    pub fn visual_lines(&self, start: usize, end: usize) -> impl Iterator<Item = WrapLine> {
        (start..end).filter_map(move |v| self.visual_line_at(v))
    }
}

impl LayerSnapshot for WrapSnapshot {
    type Input = TabPoint;
    type Output = WrapPoint;

    fn input_version(&self) -> u64 {
        self.input_version
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn map_input_by(&self, input: TabPoint) -> Biased<WrapPoint> {
        let TabPoint { row, column } = input;
        if self.identity_rows.is_some() || self.estimated_rows.is_some() {
            return Biased::from(WrapPoint { row, column });
        }
        if row >= self.lines.leaf_count() {
            return Biased::from(WrapPoint { row, column });
        }
        if matches!(self.soft_wrap, SoftWrap::None) {
            return Biased::from(WrapPoint { row, column });
        }
        let visual_row = self.visual_row_for_column(row, column);
        // 列定位到片段内：保留原始展示列不再进一步裁剪（末片段外行走恒等）。
        Biased::from(WrapPoint {
            row: visual_row,
            column,
        })
    }

    fn map_output_by(&self, output: WrapPoint) -> Biased<TabPoint> {
        let WrapPoint { row, column } = output;
        if self.identity_rows.is_some() || self.estimated_rows.is_some() {
            return Biased::from(TabPoint { row, column });
        }
        if matches!(self.soft_wrap, SoftWrap::None) {
            return Biased::from(TabPoint { row, column });
        }
        if row >= self.visual_row_count() {
            return Biased::from(TabPoint { row, column });
        }
        // WrapPoint.column 即源行内绝对展示列（与 TabPoint.column 同语义）；反向只把
        // 可视行还原为源行，列保持绝对。片段边界/裁剪由 visual_line_at 负责。
        let src = self.buffer_row_for_visual(row);
        Biased::from(TabPoint { row: src, column })
    }
}

/// 某行展示列宽在固定宽切分下的可视片段数。
fn fragment_count(line_display_width: usize, wrap_width: usize) -> usize {
    let wrap_width = wrap_width.max(1);
    if line_display_width == 0 {
        1
    } else {
        (line_display_width + wrap_width - 1) / wrap_width
    }
}

fn fallback_ranges(width: usize, wrap_width: usize) -> Vec<(usize, usize)> {
    let count = fragment_count(width, wrap_width);
    (0..count)
        .map(|i| {
            let start = i * wrap_width;
            (start, ((i + 1) * wrap_width).min(width).max(start))
        })
        .collect()
}

fn compute_identity_revision(input_version: u64, rows: usize, tab_size: usize) -> u64 {
    input_version
        .wrapping_mul(0x100000001b3)
        .wrapping_add(rows as u64)
        .wrapping_mul(0x100000001b3)
        .wrapping_add(tab_size as u64)
}

/// 整形输出不可变值：一源行的精确片段展示列区间。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowWrap {
    /// 该整形缓存项对应的配置 key（DM-224）：违背即失效重算。
    pub key: WrapCacheKey,
    /// 每片段展示列区间 [col_start, col_end)。
    pub ranges: Vec<(usize, usize)>,
}

/// WrapMap 按行整形缓存 key（DM-224）。上游 revision、字体、折行宽任一变化即失效。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrapCacheKey {
    pub version: u64,
    pub font_key: u64,
    pub wrap_width_px: u32,
}

/// 可变的折行整形器（DM-224/227）：按需对「视口内源行」惰性调用 `LineBreaker`，
/// 计算并缓存精确片段列区间；key 不变则复用缓存，key 变（上游/字体/折行宽）即失效
/// 重算。只持有已整形的行，视口外行不缓存（DM-227）——由 UI 侧后台任务填充、
/// 版本守卫提交后产出不可变 `WrapSnapshot`（DM-225）。
///
/// 为便于缓存命中比对，`wrap_width_px` 存为比特等价（`u32`）；`rows` 稀疏缓存
/// 只覆盖已请求/已进入 window 的行。
pub struct WrapMap {
    line_breaker: Rc<dyn LineBreaker>,
    input_version: u64,
    font_key: u64,
    wrap_width_px: u32,
    rows: Vec<Option<RowWrap>>,
}

impl WrapMap {
    pub fn new(line_breaker: Rc<dyn LineBreaker>, config: WrapConfig, input_version: u64) -> Self {
        Self {
            line_breaker,
            input_version,
            font_key: config.font_key,
            wrap_width_px: config.wrap_width_px.to_bits(),
            rows: Vec::new(),
        }
    }

    /// 更新上游版本（DM-224：上游 revision 变化使全部缓存失效）。
    pub fn set_input_version(&mut self, input_version: u64) {
        self.input_version = input_version;
    }

    /// 更新折行宽（修改 key 的一部分；既有缓存靠 key 匹配自然失效，不主动清空）。
    pub fn set_wrap_width(&mut self, wrap_width_px: f32) {
        self.wrap_width_px = wrap_width_px.to_bits();
    }

    fn key(&self) -> WrapCacheKey {
        WrapCacheKey {
            version: self.input_version,
            font_key: self.font_key,
            wrap_width_px: self.wrap_width_px,
        }
    }

    /// 对 `row` 求精确片段列区间：命中缓存直接返回；未命中或 key 变则对
    /// `text`（该源行的可见文本）调用 `LineBreaker` 折行，并把字节断点换算成
    /// 展示列区间。只缓存本行（DM-227 由调用方限制为视口行）。
    pub fn layout_row(&mut self, row: usize, text: &str) -> RowWrap {
        let key = self.key();
        if let Some(rw) = self.rows.get(row) {
            if let Some(rw) = rw {
                if rw.key == key {
                    return rw.clone();
                }
            }
        }
        let breaks = self
            .line_breaker
            .break_line(text, f32::from_bits(self.wrap_width_px));
        let ranges = breaks_to_ranges(text, &breaks);
        let rw = RowWrap { key, ranges };
        if self.rows.len() <= row {
            self.rows.resize(row + 1, None);
        }
        self.rows[row] = Some(rw.clone());
        rw
    }

    /// 丢弃视口（含 overscan）之外的整形缓存（DM-227）：只保留
    /// `[first_source_row, last_source_row]` 内的行，其余置为未整形。
    pub fn shrink_to_viewport(&mut self, first_source_row: usize, last_source_row: usize) {
        for (i, slot) in self.rows.iter_mut().enumerate() {
            if slot.is_some() && (i < first_source_row || i > last_source_row) {
                *slot = None;
            }
        }
    }

    /// 已整形的行数（测试/诊断：DM-227 断言视口外不整形）。
    pub fn shaped_row_count(&self) -> usize {
        self.rows.iter().filter(|r| r.is_some()).count()
    }
}

/// 把字节断点换算成相邻展示列区间（DM-223）。
///
/// 从行首累计每个字符展示列宽（`len_utf16`，与 TabPoint 展示列语义一致），
/// 在每个断点处闭合上一片段区间并新开下一片段；末片段截断到行尾。
pub fn breaks_to_ranges(text: &str, breaks: &[usize]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::with_capacity(breaks.len().saturating_add(1));
    let mut start = 0usize;
    let mut col = 0usize;
    let mut bi = breaks.iter().peekable();
    for (byte, ch) in text.char_indices() {
        if bi.peek().copied() == Some(&byte) {
            ranges.push((start, col));
            start = col;
            bi.next();
        }
        col += ch.len_utf16();
    }
    ranges.push((start, col));
    ranges
}

/// FNV-1a 派生 revision：input_version + wrap/tab 配置。任一影响 output 的变更
/// （上游 TabSnapshot revision、wrap 宽、tab 宽、片段分布）都翻转 revision。
fn compute_revision(
    input_version: u64,
    wrap_width: usize,
    tab_size: usize,
    lines: &SumTree<TextSummary>,
) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100000001b3);
    mix(input_version);
    mix(wrap_width as u64);
    mix(tab_size as u64);
    mix(lines.total().lines as u64);
    for row in 0..lines.leaf_count() {
        mix(lines.summary_before_leaf(row + 1).lines as u64);
    }
    h
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::layer::LayerSnapshot;
    use crate::model::Bias;

    /// 假 LineBreaker（DM-222 契约测试）：按「每 wrap_width_px 单位一个 break」的
    /// 定额 ASCII 宽度断行，模拟真实 shaping 返回字节断点（不中断字符）。
    struct FakeLineBreaker;

    impl LineBreaker for FakeLineBreaker {
        fn break_line(&self, text: &str, wrap_width_px: f32) -> Vec<usize> {
            let step = wrap_width_px.max(1.0) as usize;
            let mut breaks = Vec::new();
            let mut col = 0usize;
            for (byte, ch) in text.char_indices() {
                if col >= step {
                    breaks.push(byte);
                    col = 0;
                }
                col += ch.len_utf16();
            }
            breaks
        }
    }

    /// DM-222：`LineBreaker` 契约 —— 断点为字节偏移、升序、不含 0、不中断字符。
    #[test]
    fn line_breaker_contract_byte_breaks() {
        let lb = FakeLineBreaker;
        // 9 字符 step=4：断在 byte4（第 4 字符 e 前）与 byte8（第 8 字符 i 前）；
        // 不含 0、不含行尾 byte9。
        let breaks = lb.break_line("abcdefghi", 4.0);
        assert_eq!(breaks, vec![4, 8]);
        // 契约：所有断点都是字符边界、且在 (0, len] 内（即便含多字节字符）。
        for text in ["aé", "ééé", "aéé", "😀abcd"] {
            let text = text.to_string();
            let bs = lb.break_line(&text, 2.0);
            for b in &bs {
                assert!(b < &text.len(), "break 不越界: {text} @ {b}");
                assert!(text.is_char_boundary(*b), "break 不中断字符: {text} @ {b}");
                assert!(*b > 0, "不含 0");
            }
        }
    }

    /// 一个展示列宽序列 → WrapSnapshot（EditorWidth 模式）。
    fn wrap_of(widths: &[usize], wrap_width: usize) -> WrapSnapshot {
        WrapSnapshot::from_display_widths(1, wrap_width, 4, widths)
    }

    /// DM-220：短行不折，超宽行按固定宽切 N 段。
    #[test]
    fn fragment_count_splits_wide_lines() {
        let snap = wrap_of(&[5, 40], 10);
        // 行0(宽5) 1 段，行1(宽40) 4 段。
        assert_eq!(snap.row_fragment_count(0), 1);
        assert_eq!(snap.row_fragment_count(1), 4);
        assert_eq!(snap.visual_row_count(), 5);
        assert_eq!(snap.first_visual_row(1), 1);
    }

    /// DM-220：可视图行索引反向 —— visual row → 源行。
    #[test]
    fn visual_row_reverse_locate_source() {
        let snap = wrap_of(&[3, 25, 10], 10);
        assert_eq!(snap.buffer_row_for_visual(0), 0);
        assert_eq!(snap.buffer_row_for_visual(1), 1);
        assert_eq!(snap.buffer_row_for_visual(3), 1); // 行1 第3段仍属源行1
        assert_eq!(snap.buffer_row_for_visual(4), 2);
    }

    /// DM-220：可视行片段列区间（固定宽切分 + 末片段截断）。
    #[test]
    fn visual_line_fragment_columns() {
        let snap = wrap_of(&[0, 25, 10], 10);
        let v0 = snap.visual_line_at(0).unwrap();
        assert_eq!((v0.row, v0.col_start, v0.col_end), (0, 0, 0)); // 空行 1 片段，列 [0,0)
        let v1 = snap.visual_line_at(1).unwrap();
        assert_eq!((v1.row, v1.col_start, v1.col_end), (1, 0, 10));
        assert!(v1.first_fragment);
        let v3 = snap.visual_line_at(3).unwrap(); // 行1 末片段 [20,25)
        assert_eq!((v3.row, v3.col_start, v3.col_end), (1, 20, 25));
        assert!(!v3.first_fragment);
    }

    /// DM-220：展示列 → 可视行，越界列裁剪到末片段。
    #[test]
    fn visual_row_for_column_clamped() {
        let snap = wrap_of(&[25], 10);
        assert_eq!(snap.visual_row_for_column(0, 0), 0);
        assert_eq!(snap.visual_row_for_column(0, 5), 0);
        assert_eq!(snap.visual_row_for_column(0, 10), 1);
        assert_eq!(snap.visual_row_for_column(0, 19), 1);
        assert_eq!(snap.visual_row_for_column(0, 20), 2);
        assert_eq!(snap.visual_row_for_column(0, 999), 2); // 裁剪到末片段
    }

    /// DM-220：input/output 双向 round-trip（展示列保持绝对语义、可视行正确）
    #[test]
    fn map_input_output_round_trip() {
        let snap = wrap_of(&[25, 22], 10);
        // 行0 宽25 → 3 片段(visual 0..3)，行1 宽22 → 3 片段(visual 3..6)。
        // 行1 列 12 落在片段1（[10,20)），可视行 = first(1)=3 + 1 = 4。
        let out = snap.map_input_by(TabPoint { row: 1, column: 12 });
        assert_eq!(out.get(Bias::Left), WrapPoint { row: 4, column: 12 });
        // 反向把可视行还原为源行、列保持绝对 → 回到 TabPoint{1,12}。
        let back = snap.map_output_by(out.get(Bias::Left)).get(Bias::Left);
        assert_eq!(back, TabPoint { row: 1, column: 12 });
    }

    /// DM-220：跨片段点定位到正确可视行（列落在哪个片段由固定宽切分决定）。
    #[test]
    fn map_input_locates_correct_fragment_row() {
        let snap = wrap_of(&[25, 22], 10);
        // first(1)=3。片段：frag0→visual3, frag1→visual4, frag2→visual5。
        assert_eq!(
            snap.map_input_by(TabPoint { row: 1, column: 8 })
                .get(Bias::Left),
            WrapPoint { row: 3, column: 8 }, // 片段0 [0,10)
        );
        assert_eq!(
            snap.map_input_by(TabPoint { row: 1, column: 15 })
                .get(Bias::Left),
            WrapPoint { row: 4, column: 15 }, // 片段1 [10,20)
        );
        assert_eq!(
            snap.map_input_by(TabPoint { row: 1, column: 25 })
                .get(Bias::Left),
            WrapPoint { row: 5, column: 25 }, // 片段2 [20,22) 外，落在末片段
        );
    }

    /// DM-221：可视图行索引唯一来源是 SumTree —— 10 万行文档的 viewport 查询
    /// 只按需定位/产出片段，不物化全文 `Vec<VisualLine>`（`WrapSnapshot` 类型本身
    /// 无并行 `row_layout` 数组，纯由 `locate_lines`/前缀和驱动）。
    #[test]
    fn viewport_query_on_100k_rows_is_index_driven() {
        let n = 100_000usize;
        // 每行宽 25 → 3 片段，共 300_000 可视行。构造（模拟：直接给片段数摘要）。
        let lines: Vec<TextSummary> = (0..n)
            .map(|_| TextSummary {
                lines: 3,
                utf16: 25,
                ..TextSummary::default()
            })
            .collect();
        let snap = WrapSnapshot::new(1, SoftWrap::EditorWidth, 10, 4, &lines);
        assert_eq!(snap.visual_row_count(), n * 3);
        // 只查一个 viewport 窗口（视口+overscan），逐行产出，不生成全文数组。
        let start = 100_000usize;
        let end = start + 40;
        let count = snap.visual_lines(start, end).count();
        assert_eq!(count, 40);
        // 定位正确：视口首行落在正确的源行（100_000 / 3 = 33333 行余 1）。
        let first = snap.visual_line_at(start).unwrap();
        assert_eq!(first.row, 33_333);
        assert_eq!(first.col_start, 10);
        assert_eq!(first.col_end, 20);
        // 反向定位一致。
        assert_eq!(snap.buffer_row_for_visual(start), 33_333);
        assert_eq!(snap.first_visual_row(33_334), 33_334 * 3); // 前缀 = 之前行片段数
    }

    /// 用「按展示列宽断」的假 LineBreaker 把字节断点换算成精确片段列区间。
    ///
    /// 字节断点 → 展示列边界：累计每字符长（ASCII/CJK 各 1/2 演示），相邻边界成片段。
    fn frags_from_text(text: &str, wrap_width_px: f32) -> Vec<(usize, usize)> {
        let breaks = FakeLineBreaker.break_line(text, wrap_width_px);
        let mut bounds = vec![0usize];
        let mut byte = 0usize;
        for &b in &breaks {
            let col: usize = text[byte..b].chars().map(|c| c.len_utf16()).sum();
            byte = b;
            bounds.push(col);
        }
        let total: usize = text.chars().map(|c| c.len_utf16()).sum();
        bounds.push(total);
        bounds.windows(2).map(|w| (w[0], w[1])).collect()
    }

    /// DM-223：真实 shaping 断点替换固定宽切分 —— 宽字符（BMP+代理对）片段
    /// 列边界与列归属均按实测展示宽，而非固定列除法。
    #[test]
    fn shaping_breakpoints_cover_wide_chars() {
        // 行文本含代理对宽字符 emoji（len_utf16=2）。
        let text = "a😀b";
        // 假测量宽：a=1 😀=2 b=1，总 4。step=2 → 断在 😀 之后 byte5。
        let frags = frags_from_text(text, 2.0);
        // 首片段展示列 [0,3)（a+😀），次片段 [3,4)（b）——断点落在 emoji 后，未劈开。
        assert_eq!(frags, vec![(0, 3), (3, 4)]);
        let snap = WrapSnapshot::from_breakpoints(1, 2, 4, &[4], &[frags]);
        assert_eq!(snap.row_fragment_count(0), 2);
        assert_eq!(snap.visual_row_count(), 2);
        // 片段列区间按 shaping。
        let v0 = snap.visual_line_at(0).unwrap();
        assert_eq!((v0.row, v0.col_start, v0.col_end), (0, 0, 3));
        assert!(v0.first_fragment);
        let v1 = snap.visual_line_at(1).unwrap();
        assert_eq!((v1.row, v1.col_start, v1.col_end), (0, 3, 4));
        // 列归属：列 1/2（😀 内）仍首片段；列 3（b）次片段。
        assert_eq!(snap.visual_row_for_column(0, 1), 0);
        assert_eq!(snap.visual_row_for_column(0, 2), 0);
        assert_eq!(snap.visual_row_for_column(0, 3), 1);
        // 越界到末片段。
        assert_eq!(snap.visual_row_for_column(0, 99), 1);
    }

    /// 记录被调用次数的假 LineBreaker（DM-224/227 断言「只整形视口行 / 命中缓存」）。
    struct CountingBreaker {
        calls: AtomicUsize,
    }

    impl CountingBreaker {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl LineBreaker for CountingBreaker {
        fn break_line(&self, text: &str, wrap_width_px: f32) -> Vec<usize> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            FakeLineBreaker.break_line(text, wrap_width_px)
        }
    }

    fn counting_map(wrap: f32) -> (WrapMap, Rc<CountingBreaker>) {
        let lb = Rc::new(CountingBreaker::new());
        let map = WrapMap::new(
            lb.clone(),
            WrapConfig {
                wrap_width_px: wrap,
                wrap_width_utf16: wrap as usize,
                font_size_px: 14.0,
                font_key: 1,
                tab_size: 4,
            },
            1,
        );
        (map, lb)
    }

    /// DM-224：按行整形缓存 —— 命中 key 复用、key 变（折行宽 / 上游版本）失效重算。
    #[test]
    fn wrap_map_caches_layout_keyed_by_config() {
        let (mut map, lb) = counting_map(4.0);
        // 首次整形两行各调一次 break_line。
        let r0 = map.layout_row(0, "abcdefghi");
        let r1 = map.layout_row(1, "abc");
        assert_eq!(lb.calls(), 2);
        assert_eq!(r0.ranges, vec![(0, 4), (4, 8), (8, 9)]); // step4：3 片段
        assert_eq!(r1.ranges, vec![(0, 3)]); // 短行 1 片段
        // 再次请求同 key → 命中缓存，不再调用 break_line。
        let r0b = map.layout_row(0, "abcdefghi");
        assert_eq!(lb.calls(), 2);
        assert_eq!(r0b, r0);
        // 折行宽变化 → key 变 → 该行重算。
        map.set_wrap_width(8.0);
        let r0c = map.layout_row(0, "abcdefghi");
        assert_eq!(lb.calls(), 3);
        assert_eq!(r0c.ranges, vec![(0, 8), (8, 9)]);
        // 上游版本变化 → 全缓存 key 变 → 重算。
        map.set_input_version(2);
        let _ = map.layout_row(0, "abcdefghi");
        assert_eq!(lb.calls(), 4);
    }

    /// DM-227：视口（含 overscan）之外的行不整形、不缓存。
    #[test]
    fn wrap_map_shapes_only_viewport_rows() {
        let (mut map, lb) = counting_map(4.0);
        // 视口只含源行 100..104。
        for row in 100..104 {
            let text = format!("line {row} abcdefghij");
            map.layout_row(row, &text);
        }
        assert_eq!(lb.calls(), 4);
        assert_eq!(map.shaped_row_count(), 4);
        // 缩小视口到 101..102 → 100/103 两端整形缓存被丢弃。
        map.shrink_to_viewport(101, 102);
        assert_eq!(map.shaped_row_count(), 2);
        // 再次请求 100 → 之前已丢，重新整形。
        map.layout_row(100, "line 100 abcdefghij");
        assert_eq!(lb.calls(), 5);
    }

    /// DM-226：超长行整形不下溢出 —— 断点数量/列区间正确，且末片段截断到行尾。
    #[test]
    fn long_line_breaks_without_overflow() {
        let (mut map, _lb) = counting_map(80.0);
        let long = "あ".repeat(100_000); // 100k 个宽字符（BMP，len_utf16=1）
        let rw = map.layout_row(0, &long);
        // step=80 列 → 每片段 80 列，共 1250 片段；首/末区间正确、相邻无缝隙。
        assert_eq!(rw.ranges.len(), 100_000 / 80);
        assert_eq!(rw.ranges[0], (0, 80));
        assert_eq!(*rw.ranges.last().unwrap(), (99_920, 100_000));
        assert!(
            rw.ranges.windows(2).all(|w| w[0].1 == w[1].0),
            "相邻片段无缝隙"
        );
    }

    /// DM-220：revision / is_current —— input_version 或 wrap 配置变化即失效。
    #[test]
    fn revision_and_is_current() {
        let a = wrap_of(&[10, 20], 10);
        let b = wrap_of(&[10, 20], 20); // 不同 wrap 宽
        assert_ne!(a.revision(), b.revision());
        assert!(a.is_current(a.input_version(), a.revision()));
        assert!(!b.is_current(a.input_version(), a.revision()));
        // 同配置同版本 → 同 revision。
        let a2 = wrap_of(&[10, 20], 10);
        assert_eq!(a.revision(), a2.revision());
    }

    #[test]
    fn identity_snapshot_maps_rows_without_summary_tree() {
        let map = WrapSnapshot::identity(7, 100_000, 4);
        assert_eq!(map.visual_row_count(), 100_000);
        assert_eq!(map.first_visual_row(42), 42);
        assert_eq!(map.row_fragment_count(42), 1);
        assert_eq!(map.buffer_row_for_visual(99_999), 99_999);
        assert_eq!(map.visual_row_for_column(42, 123), 42);
        assert!(map.is_current(7, map.revision()));
    }

    #[test]
    fn sync_rows_replaces_only_dirty_path() {
        let widths = vec![1usize; 100];
        let old = WrapSnapshot::from_display_widths(1, 10, 4, &widths);
        let next = old.sync_rows(2, 4, 2..3, &[30]);
        assert_eq!(next.row_fragment_count(2), 3);
        assert_eq!(next.row_fragment_count(99), 1);
        assert!(old.lines.shared_trailing_leaves(&next.lines) > 0);
    }
}
