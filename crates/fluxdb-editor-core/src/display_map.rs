//! Display map：buffer row ↔ visual row 的双向映射，软换行、折叠、viewport 摘要。
//!
//! `DisplayMap` 是分层管线
//! **（Buffer → Inlay → Fold → Tab → Wrap）**之上的**精简门面**（DM-228/DM-304）：
//! 内部委托四个独立的 [`LayerSnapshot`] —— [`InlaySnapshot`]（BufferPoint→InlayPoint）、
//! [`FoldSnapshot`]（InlayPoint→FoldPoint）、
//! [`TabSnapshot`]（FoldPoint→TabPoint）、[`WrapSnapshot`]（TabPoint→WrapPoint），
//! 单调体的 fold/tab/wrap 字段与 rebuild/apply_change 交织分支已删除。
//!
//! 门面保留对 UI 的稳定字节语义接口（[`VisualLine`] 的 `column_start/column_end` 仍是
//! **buffer 字节列**，由「展示列 → 字节列」换算），外部消费端（fluxdb-desktop）的
//! `visual_row_count` / `visual_line_at` / `visual_row_for_column` / `soft_wrap` /
//! `apply_change` 签名不变，diff 收敛到构造与重建路径。
//!
//! `ponytail:` 门面当前在 `new_new_with_tab_size` 时 **eagerly** 构建三个快照
//! （TabSnapshot 按展示行持有可视文本、WrapSnapshot 持有每行片段列区间）——对超大文档
//! 是 O(总文本) 的构造成本，违背 DM-007「构造不读全文」的旧性质。真正的 **viewport
//! 惰性整形**已由 `crates/fluxdb-editor-core/src/wrap_map.rs` 的 `WrapMap`（按需 + 视口
//! 裁剪）提供；待 UI 侧以 `LineBreaker` 后台整形填充后再经 `WrapSnapshot::from_breakpoints`
//! 原子提交（DM-225/DM-227）。此处先用 UTF-16 近似列宽（`from_display_widths`）保证
//! 正确性与既有 golden 测试全绿，不回归渲染链路。

use crate::block_map::{Block, BlockSnapshot};
use crate::buffer::BufferSnapshot;
use crate::coordinates::BufferPoint;
use crate::edit::FullRebuild;
use crate::fold_map::FoldSnapshot;
use crate::inlay_map::InlaySnapshot;
use crate::layer::{LayerPatch, LayerSnapshot};
use crate::model::{Bias, Range, TextChange};
use crate::tab_map::{TabLine, TabSnapshot};
use crate::wrap_map::WrapSnapshot;
use std::cell::OnceCell;
use std::rc::Rc;

/// 软换行模式（与 model 复用，避免重复定义）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoftWrap {
    #[default]
    None,
    EditorWidth,
}

/// display map 内的一个可视行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualLine {
    /// 所属 buffer 行。
    pub buffer_row: usize,
    /// 在 buffer 行内的字节列起始（wrap 时从 0 或上次换行处开始）。
    pub column_start: usize,
    /// 该可视行覆盖的 buffer 行内字节列范围。
    pub column_end: usize,
    /// 是否为 buffer 行的首片段。
    pub first_fragment: bool,
}

/// 一条 DisplayMap edit，等价于 Zed `Patch<WrapRow>` 中的一项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayEdit {
    pub old_rows: std::ops::Range<usize>,
    pub new_rows: std::ops::Range<usize>,
}

impl DisplayEdit {
    pub fn is_empty(&self) -> bool {
        self.old_rows.is_empty() && self.new_rows.is_empty()
    }

    pub fn old_len(&self) -> usize {
        self.old_rows.len()
    }

    pub fn new_len(&self) -> usize {
        self.new_rows.len()
    }
}

/// DisplayMap 的可组合 dirty patch。每层把自己的坐标变换结果追加为一项，UI
/// 只需遍历这些区间失效对应的 visual-row/layout 缓存。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisplayPatch {
    edits: Vec<DisplayEdit>,
}

impl DisplayPatch {
    pub fn new(edits: Vec<DisplayEdit>) -> Self {
        debug_assert!(
            edits
                .windows(2)
                .all(|pair| pair[0].old_rows.start <= pair[1].old_rows.start)
        );
        Self { edits }
    }

    pub fn single(old_rows: std::ops::Range<usize>, new_rows: std::ops::Range<usize>) -> Self {
        Self {
            edits: vec![DisplayEdit { old_rows, new_rows }],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    pub fn edits(&self) -> &[DisplayEdit] {
        &self.edits
    }

    pub fn into_edits(self) -> Vec<DisplayEdit> {
        self.edits
    }

    /// Convert a layer-local patch at the final visual-row boundary into the
    /// public UI patch type. Intermediate layers stay independent of DisplayMap.
    pub fn from_layer_patch(patch: &LayerPatch) -> Self {
        Self::new(
            patch
                .edits()
                .iter()
                .map(|edit| DisplayEdit {
                    old_rows: edit.old.clone(),
                    new_rows: edit.new.clone(),
                })
                .collect(),
        )
    }

    /// 追加一个非空 edit。edit 必须按 old 坐标递增传入。
    pub fn push(&mut self, old_rows: std::ops::Range<usize>, new_rows: std::ops::Range<usize>) {
        let edit = DisplayEdit { old_rows, new_rows };
        if edit.is_empty() {
            return;
        }
        self.push_maybe_empty(edit);
    }

    /// 追加 edit，即使它表示纯插入/删除。对应 Zed `push_maybe_empty`。
    pub fn push_maybe_empty(&mut self, edit: DisplayEdit) {
        if let Some(last) = self.edits.last_mut()
            && last.old_rows.end >= edit.old_rows.start
        {
            last.old_rows.end = last.old_rows.end.max(edit.old_rows.end);
            last.new_rows.end = last.new_rows.end.max(edit.new_rows.end);
            return;
        }
        self.edits.push(edit);
    }

    pub fn extend(&mut self, other: Self) {
        for edit in other.edits {
            self.push_maybe_empty(edit);
        }
    }

    /// 将当前 patch（old -> mid）与后续 patch（mid -> new）合成为一个
    /// old -> new patch。坐标变换按 Zed `Patch<T>::compose` 的 sweep 语义执行，
    /// 不把中间层的行号误当成原始 buffer 行号。
    pub fn compose(&self, next: impl IntoIterator<Item = DisplayEdit>) -> Self {
        let mut old_edits = self.edits.clone();
        let mut new_edits: Vec<DisplayEdit> = next.into_iter().collect();
        let mut old_index = 0;
        let mut new_index = 0;
        let mut old_start = 0usize;
        let mut new_start = 0usize;
        let mut composed = Self::default();

        loop {
            let old_edit = old_edits.get(old_index).cloned();
            let new_edit = new_edits.get(new_index).cloned();

            if let Some(old_edit) = old_edit.as_ref()
                && new_edit
                    .as_ref()
                    .is_none_or(|new_edit| old_edit.new_rows.end < new_edit.old_rows.start)
            {
                let catchup = old_edit.old_rows.start.saturating_sub(old_start);
                old_start += catchup;
                new_start += catchup;
                let old_end = old_start + old_edit.old_len();
                let new_end = new_start + old_edit.new_len();
                composed.push(old_start..old_end, new_start..new_end);
                old_start = old_end;
                new_start = new_end;
                old_index += 1;
                continue;
            }

            if let Some(new_edit) = new_edit.as_ref()
                && old_edit
                    .as_ref()
                    .is_none_or(|old_edit| new_edit.old_rows.end < old_edit.new_rows.start)
            {
                let catchup = new_edit.new_rows.start.saturating_sub(new_start);
                old_start += catchup;
                new_start += catchup;
                let old_end = old_start + new_edit.old_len();
                let new_end = new_start + new_edit.new_len();
                composed.push(old_start..old_end, new_start..new_end);
                old_start = old_end;
                new_start = new_end;
                new_index += 1;
                continue;
            }

            let (Some(old_edit), Some(new_edit)) = (old_edit, new_edit) else {
                break;
            };

            if old_edit.new_rows.start < new_edit.old_rows.start {
                let catchup = old_edit.old_rows.start.saturating_sub(old_start);
                old_start += catchup;
                new_start += catchup;
                let overshoot = new_edit.old_rows.start - old_edit.new_rows.start;
                let old_end = (old_start + overshoot).min(old_edit.old_rows.end);
                let new_end = new_start + overshoot;
                composed.push(old_start..old_end, new_start..new_end);
                old_start = old_end;
                new_start = new_end;
                old_edits[old_index].old_rows.start = old_end;
                old_edits[old_index].new_rows.start += overshoot;
            } else {
                let catchup = new_edit.new_rows.start.saturating_sub(new_start);
                old_start += catchup;
                new_start += catchup;
                let overshoot = old_edit.new_rows.start - new_edit.old_rows.start;
                let old_end = old_start + overshoot;
                let new_end = (new_start + overshoot).min(new_edit.new_rows.end);
                composed.push(old_start..old_end, new_start..new_end);
                old_start = old_end;
                new_start = new_end;
                new_edits[new_index].old_rows.start += overshoot;
                new_edits[new_index].new_rows.start = new_end;
            }

            let old_edit = old_edits[old_index].clone();
            let new_edit = new_edits[new_index].clone();
            if old_edit.new_rows.end > new_edit.old_rows.end {
                let old_end = old_start + old_edit.old_len().min(new_edit.old_len());
                let new_end = new_start + new_edit.new_len();
                composed.push(old_start..old_end, new_start..new_end);
                old_edits[old_index].old_rows.start = old_end;
                old_edits[old_index].new_rows.start = new_edit.old_rows.end;
                old_start = old_end;
                new_start = new_end;
                new_index += 1;
            } else {
                let old_end = old_start + old_edit.old_len();
                let new_end = new_start + old_edit.new_len().min(new_edit.new_len());
                composed.push(old_start..old_end, new_start..new_end);
                new_edits[new_index].old_rows.start = old_edit.new_rows.end;
                new_edits[new_index].new_rows.start = new_end;
                old_start = old_end;
                new_start = new_end;
                old_index += 1;
            }
        }

        composed
    }

    /// 将 old 坐标中的位置映射到 new 坐标；落在替换区间内部时定位到新区间起点。
    pub fn old_to_new(&self, old: usize) -> usize {
        let index = match self
            .edits
            .binary_search_by(|edit| edit.old_rows.start.cmp(&old))
        {
            Ok(index) => index,
            Err(0) => return old,
            Err(index) => index - 1,
        };
        let Some(edit) = self.edits.get(index) else {
            return old;
        };
        if old >= edit.old_rows.end {
            edit.new_rows.end + old.saturating_sub(edit.old_rows.end)
        } else {
            edit.new_rows.start
        }
    }

    /// 返回触及 old 坐标的 edit；未命中时返回零长度恒等 edit。
    pub fn edit_for_old_position(&self, old: usize) -> DisplayEdit {
        let index = match self
            .edits
            .binary_search_by(|edit| edit.old_rows.start.cmp(&old))
        {
            Ok(index) => index,
            Err(0) => {
                return DisplayEdit {
                    old_rows: old..old,
                    new_rows: old..old,
                };
            }
            Err(index) => index - 1,
        };
        let Some(edit) = self.edits.get(index) else {
            return DisplayEdit {
                old_rows: old..old,
                new_rows: old..old,
            };
        };
        if old > edit.old_rows.end {
            let translated = edit.new_rows.end + old.saturating_sub(edit.old_rows.end);
            DisplayEdit {
                old_rows: old..old,
                new_rows: translated..translated,
            }
        } else {
            edit.clone()
        }
    }

    pub fn invert(&mut self) -> &mut Self {
        for edit in &mut self.edits {
            std::mem::swap(&mut edit.old_rows, &mut edit.new_rows);
        }
        self.edits.sort_by_key(|edit| edit.old_rows.start);
        self
    }

    pub fn clear(&mut self) {
        self.edits.clear();
    }

    pub fn old_row_bounds(&self) -> Option<std::ops::Range<usize>> {
        let first = self.edits.first()?;
        let last = self.edits.last()?;
        Some(first.old_rows.start..last.old_rows.end)
    }

    pub fn new_row_bounds(&self) -> Option<std::ops::Range<usize>> {
        let first = self.edits.first()?;
        let last = self.edits.last()?;
        Some(first.new_rows.start..last.new_rows.end)
    }

    pub fn row_delta(&self) -> isize {
        self.edits.iter().fold(0, |delta, edit| {
            delta + edit.new_rows.len() as isize - edit.old_rows.len() as isize
        })
    }
}

impl IntoIterator for DisplayPatch {
    type Item = DisplayEdit;
    type IntoIter = std::vec::IntoIter<DisplayEdit>;

    fn into_iter(self) -> Self::IntoIter {
        self.edits.into_iter()
    }
}

impl<'a> IntoIterator for &'a DisplayPatch {
    type Item = DisplayEdit;
    type IntoIter = std::iter::Cloned<std::slice::Iter<'a, DisplayEdit>>;

    fn into_iter(self) -> Self::IntoIter {
        self.edits.iter().cloned()
    }
}

/// 折叠区间（buffer 行内）。门面把此形态转交给 [`FoldSnapshot`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fold {
    /// 折叠开始的 buffer 行（含）。
    pub start_row: usize,
    /// 折叠结束的 buffer 行（含）。
    pub end_row: usize,
}

/// 一个 display map。组合独立 LayerSnapshot，对外提供稳定的字节语义查询。
pub struct DisplayMap {
    snapshot: BufferSnapshot,
    soft_wrap: SoftWrap,
    inlay_snap: InlaySnapshot,
    fold_snap: FoldSnapshot,
    tab_snap: TabSnapshot,
    wrap_snap: WrapSnapshot,
    lazy_layout: Option<LazyLayoutState>,
    /// 第 5 层 Block：CodeLens 等「整行 widget」的坐标（DM-310~316）。
    block_snap: BlockSnapshot,
    /// DM-007：视图查询路径（展示列 → 字节列换算）读取的字符工作量计数，仅供测试。
    #[cfg(test)]
    viewport_scan_work: std::cell::Cell<usize>,
}

const LAZY_LAYOUT_MIN_BYTES: usize = 256 * 1024;

#[derive(Clone)]
struct MaterializedLayout {
    tab_snap: TabSnapshot,
    wrap_snap: WrapSnapshot,
}

struct LazyLayoutState {
    cell: OnceCell<MaterializedLayout>,
    snapshot: BufferSnapshot,
    fold_snap: FoldSnapshot,
    soft_wrap: SoftWrap,
    wrap_width: usize,
    tab_size: usize,
}

impl LazyLayoutState {
    fn build(&self) -> MaterializedLayout {
        let display_count = visible_display_rows(&self.snapshot, &self.fold_snap);
        let tab_lines: Vec<TabLine> = (0..display_count)
            .map(|disp| {
                let row = self.fold_snap.buffer_row_for_display(disp);
                TabLine {
                    text: self
                        .snapshot
                        .text_in_range(line_range(&self.snapshot, row))
                        .into(),
                    is_placeholder: false,
                }
            })
            .collect();
        let tab_snap = TabSnapshot::new(self.fold_snap.revision(), self.tab_size, tab_lines);
        let widths: Vec<usize> = (0..display_count)
            .map(|row| tab_snap.line_display_width(row))
            .collect();
        let wrap_snap = match self.soft_wrap {
            SoftWrap::None => {
                WrapSnapshot::identity(tab_snap.revision(), display_count, self.tab_size)
            }
            SoftWrap::EditorWidth => WrapSnapshot::from_display_widths(
                tab_snap.revision(),
                self.wrap_width,
                self.tab_size,
                &widths,
            ),
        };
        MaterializedLayout {
            tab_snap,
            wrap_snap,
        }
    }
}

impl DisplayMap {
    pub fn soft_wrap(&self) -> SoftWrap {
        self.soft_wrap
    }

    fn layout(&self) -> (&TabSnapshot, &WrapSnapshot) {
        if let Some(state) = &self.lazy_layout {
            let layout = state.cell.get_or_init(|| state.build());
            (&layout.tab_snap, &layout.wrap_snap)
        } else {
            (&self.tab_snap, &self.wrap_snap)
        }
    }

    fn materialize_layout(&mut self) {
        let Some(state) = self.lazy_layout.take() else {
            return;
        };
        let layout = state.cell.get().cloned().unwrap_or_else(|| state.build());
        self.tab_snap = layout.tab_snap;
        self.wrap_snap = layout.wrap_snap;
    }

    /// 构造 display map。`folds` 需已按 start_row 排序且互不重叠（实现会规范化）。
    pub fn new(
        snapshot: BufferSnapshot,
        soft_wrap: SoftWrap,
        wrap_width_utf16: usize,
        folds: Vec<Fold>,
    ) -> Self {
        Self::new_with_tab_size(snapshot, soft_wrap, wrap_width_utf16, folds, 4)
    }

    /// 构造带 TabMap 语义的 display map（DM-228 组合管线门面）。
    ///
    /// tab 不改变 buffer 文本，只在显示坐标中展开到下一个 tab stop；因此编辑、
    /// Tree-sitter 和补全继续使用原始字节偏移，只有 wrap/layout 消费显示列。
    pub fn new_with_tab_size(
        snapshot: BufferSnapshot,
        soft_wrap: SoftWrap,
        wrap_width_utf16: usize,
        folds: Vec<Fold>,
        tab_size: usize,
    ) -> Self {
        let tab_size = tab_size.max(1);
        let wrap_width = wrap_width_utf16.max(1);
        let version = snapshot.version();

        // 第 1 层：Buffer → Fold（规范化折叠，隐藏内部行）。
        let fold_snap = FoldSnapshot::new(version, folds);
        // 展示行（fold 之后可见）总数。
        let display_count = visible_display_rows(&snapshot, &fold_snap);
        if display_count == 0 {
            // 空文档：至少 1 个展示行（末尾空行），占位结构。
            let empty_tab = TabSnapshot::new(
                fold_snap.revision(),
                tab_size,
                vec![TabLine {
                    text: Rc::from(""),
                    is_placeholder: false,
                }],
            );
            let empty_wrap = WrapSnapshot::from_display_widths(
                empty_tab.revision(),
                wrap_width,
                tab_size,
                &[0usize],
            );
            return Self {
                snapshot,
                soft_wrap,
                // DM-304：门面管线插入空 Inlay 层（恒等映射）。
                inlay_snap: InlaySnapshot::new(version, 0, Vec::new(), |_| (0, 0)),
                fold_snap,
                tab_snap: empty_tab,
                wrap_snap: empty_wrap,
                lazy_layout: None,
                // DM-310：空 Block 层（恒等映射，无块）。
                block_snap: BlockSnapshot::new(version, 0, Vec::new(), 1),
                #[cfg(test)]
                viewport_scan_work: std::cell::Cell::new(0),
            };
        }

        if matches!(soft_wrap, SoftWrap::EditorWidth)
            && (snapshot.len() >= LAZY_LAYOUT_MIN_BYTES || display_count >= 5_000)
        {
            let tab_snap = TabSnapshot::identity(fold_snap.revision(), display_count, tab_size);
            let wrap_snap =
                WrapSnapshot::estimated(tab_snap.revision(), display_count, wrap_width, tab_size);
            let block_snap =
                BlockSnapshot::new(version, 0, Vec::new(), wrap_snap.visual_row_count());
            return Self {
                snapshot: snapshot.clone(),
                soft_wrap,
                inlay_snap: InlaySnapshot::new(version, 0, Vec::new(), |_| (0, 0)),
                fold_snap: fold_snap.clone(),
                tab_snap,
                wrap_snap,
                lazy_layout: Some(LazyLayoutState {
                    cell: OnceCell::new(),
                    snapshot,
                    fold_snap,
                    soft_wrap,
                    wrap_width,
                    tab_size,
                }),
                block_snap,
                #[cfg(test)]
                viewport_scan_work: std::cell::Cell::new(0),
            };
        }

        // 第 2 层：Fold → Tab。无软换行时 tab 不影响行数或输出区间，
        // 不需要为每一行物化文本；TabMap 的行文本只在需要列映射时由
        // `visual_row_for_column`/`visual_line_at` 直接读取 buffer。
        let tab_lines = if matches!(soft_wrap, SoftWrap::None) {
            (0..display_count)
                .map(|_| TabLine {
                    text: Rc::from(""),
                    is_placeholder: false,
                })
                .collect()
        } else {
            let mut lines = Vec::with_capacity(display_count);
            for disp in 0..display_count {
                let buffer_row = fold_snap.buffer_row_for_display(disp);
                let byte_start = snapshot.line_start(buffer_row);
                let bytes = line_len_bytes(&snapshot, buffer_row);
                lines.push(TabLine {
                    text: snapshot
                        .text_in_range(Range::new(byte_start, byte_start + bytes))
                        .into(),
                    is_placeholder: false,
                });
            }
            lines
        };
        let tab_snap = TabSnapshot::new(fold_snap.revision(), tab_size, tab_lines);

        // 第 3 层：Tab → Wrap。按展示行显示宽推导片段（可换行时切分）。
        let wrap_snap = match soft_wrap {
            SoftWrap::None => WrapSnapshot::identity(tab_snap.revision(), display_count, tab_size),
            SoftWrap::EditorWidth => {
                let widths: Vec<usize> = (0..display_count)
                    .map(|disp| tab_snap.line_display_width(disp))
                    .collect();
                WrapSnapshot::from_display_widths(
                    tab_snap.revision(),
                    wrap_width,
                    tab_size,
                    &widths,
                )
            }
        };

        let block_snap = BlockSnapshot::new(version, 0, Vec::new(), wrap_snap.visual_row_count());
        Self {
            snapshot,
            soft_wrap,
            // DM-304：门面管线插入空 Inlay 层（恒等映射，inlay 由 UI 经
            // inline_hints_to_snapshot 注入后才参与 fold/tab/wrap）。
            inlay_snap: InlaySnapshot::new(version, 0, Vec::new(), |_| (0, 0)),
            fold_snap,
            tab_snap,
            wrap_snap,
            lazy_layout: None,
            // DM-310：空 Block 层（恒等映射，无块；base_rows = wrap 行数）。
            // 外部经 set_blocks 把 CodeLens 等块注入为顶层坐标层（DM-312）。
            block_snap,
            #[cfg(test)]
            viewport_scan_work: std::cell::Cell::new(0),
        }
    }

    /// 应用一次编辑并返回跨层传播后的 dirty patch。
    ///
    /// 普通编辑走 Buffer → Inlay → Fold → Tab → Wrap → Block：只替换受影响源行的
    /// 摘要叶子，SumTree 对前后缀做 path-copy。全文替换、版本跳变和折叠结构变化
    /// 才允许显式全量重建。
    pub fn apply_change(
        &mut self,
        snapshot: BufferSnapshot,
        change: &TextChange,
        folds: Vec<Fold>,
    ) {
        let _ = self.apply_change_with_patch(snapshot, change, folds);
    }

    pub fn apply_change_with_patch(
        &mut self,
        snapshot: BufferSnapshot,
        change: &TextChange,
        folds: Vec<Fold>,
    ) -> DisplayPatch {
        self.materialize_layout();
        let old_snapshot = self.snapshot.clone();
        let old_rows = self.visual_row_range_for_change(&old_snapshot, change.old_range);
        let new_end = change
            .old_range
            .start
            .saturating_add(change.new_text.len())
            .min(snapshot.len());
        let new_rows = self
            .visual_row_range_for_change(&snapshot, Range::new(change.old_range.start, new_end));

        let structural_folds = !self.fold_snap.same_folds(&folds);
        let full = matches!(
            FullRebuild::for_change(change, old_snapshot.version()),
            FullRebuild::Needed(_)
        );
        if full {
            let old = self.soft_wrap;
            *self = Self::new_with_tab_size(
                snapshot,
                old,
                self.wrap_snap.wrap_width(),
                folds,
                self.tab_snap.tab_size(),
            );
            return DisplayPatch::single(old_rows, new_rows);
        }

        if structural_folds {
            return self.apply_fold_change(snapshot, change, folds, old_rows, new_rows);
        }

        // Relocate the inlay anchors first, then propagate the buffer row patch through
        // Fold, Tab, Wrap and Block without rebuilding untouched suffix leaves.
        let old_start = old_snapshot.offset_to_point(change.old_range.start).row;
        let old_end = old_snapshot
            .offset_to_point(change.old_range.end.min(old_snapshot.len()))
            .row
            .saturating_add(1)
            .min(old_snapshot.line_count());
        let new_start = snapshot
            .offset_to_point(change.old_range.start.min(snapshot.len()))
            .row;
        let new_end_row = snapshot
            .offset_to_point(new_end)
            .row
            .saturating_add(1)
            .min(snapshot.line_count());
        let old_range = old_start..old_end.max(old_start + 1).min(old_snapshot.line_count());
        let new_range = new_start..new_end_row.max(new_start + 1).min(snapshot.line_count());

        let inlay_snap = self.inlay_snap.sync_change(
            snapshot.version(),
            &snapshot,
            change.old_range,
            change.new_text.len(),
        );
        let fold_snap = self.fold_snap.sync_input(snapshot.version());
        let tab_lines = (new_range.clone())
            .map(|row| TabLine {
                text: snapshot.text_in_range(line_range(&snapshot, row)).into(),
                is_placeholder: false,
            })
            .collect();
        let (tab_snap, tab_patch) =
            self.tab_snap
                .sync_rows_with_patch(fold_snap.revision(), old_range.clone(), tab_lines);
        let tab_edit =
            tab_patch
                .edits()
                .first()
                .cloned()
                .unwrap_or_else(|| crate::layer::LayerEdit {
                    old: old_range.clone(),
                    new: new_range.clone(),
                });
        let widths: Vec<usize> = new_range
            .clone()
            .map(|row| {
                display_width_for_line(&snapshot, row, self.tab_snap.tab_size())
                    + inlay_snap.row_extra_width(row)
            })
            .collect();
        let (wrap_snap, wrap_patch) = self.wrap_snap.sync_rows_with_patch(
            tab_snap.revision(),
            self.tab_snap.tab_size(),
            tab_edit.old,
            &widths,
        );
        let wrap_edit = wrap_patch
            .edits()
            .first()
            .expect("sync_rows_with_patch always returns one edit");
        let old_wrap_start = wrap_edit.old.start;
        let old_wrap_end = wrap_edit.old.end;
        let new_wrap_start = wrap_edit.new.start;
        let new_wrap_end = wrap_edit.new.end;
        let had_blocks = self.block_snap.has_blocks();
        let (block_snap, block_patch) = if had_blocks {
            self.block_snap.sync_rows_with_patch(
                wrap_snap.revision(),
                old_wrap_start..old_wrap_end.max(old_wrap_start + 1),
                new_wrap_end.saturating_sub(new_wrap_start).max(1),
            )
        } else {
            (
                self.block_snap
                    .sync_input(wrap_snap.revision(), wrap_snap.visual_row_count()),
                crate::layer::LayerPatch::single(
                    old_wrap_start..old_wrap_end,
                    new_wrap_start..new_wrap_end,
                ),
            )
        };
        self.snapshot = snapshot;
        self.inlay_snap = inlay_snap;
        self.fold_snap = fold_snap;
        self.tab_snap = tab_snap;
        self.wrap_snap = wrap_snap;
        self.block_snap = block_snap;
        DisplayPatch::from_layer_patch(&block_patch)
    }

    fn visual_row_range_for_change(
        &self,
        snapshot: &BufferSnapshot,
        range: Range,
    ) -> std::ops::Range<usize> {
        let start = snapshot
            .offset_to_point(range.start.min(snapshot.len()))
            .row;
        let end = snapshot
            .offset_to_point(range.end.min(snapshot.len()))
            .row
            .saturating_add(1);
        start..end.max(start + 1).min(snapshot.line_count().max(1))
    }

    fn apply_fold_change(
        &mut self,
        snapshot: BufferSnapshot,
        change: &TextChange,
        folds: Vec<Fold>,
        old_rows: std::ops::Range<usize>,
        new_rows: std::ops::Range<usize>,
    ) -> DisplayPatch {
        let old_fold = &self.fold_snap;
        let (next_fold, fold_patch) = old_fold.sync_folds_with_patch(
            snapshot.version(),
            folds,
            self.snapshot.line_count(),
            snapshot.line_count(),
        );
        let next_inlay = self.inlay_snap.sync_change(
            snapshot.version(),
            &snapshot,
            change.old_range,
            change.new_text.len(),
        );
        let (old_prefix, new_prefix) = fold_patch
            .edits()
            .first()
            .map(|edit| (edit.old.start, edit.new.start))
            .unwrap_or((old_rows.start, new_rows.start));
        let new_display_count = visible_display_rows(&snapshot, &next_fold);
        let old_source_count = self.tab_snap.row_count();
        let inserted_lines: Vec<TabLine> = (new_prefix..new_display_count)
            .map(|display_row| {
                if matches!(self.soft_wrap, SoftWrap::None) {
                    return TabLine {
                        text: Rc::from(""),
                        is_placeholder: false,
                    };
                }
                let row = next_fold.buffer_row_for_display(display_row);
                TabLine {
                    text: snapshot.text_in_range(line_range(&snapshot, row)).into(),
                    is_placeholder: next_fold.is_folded_display_row(display_row),
                }
            })
            .collect();
        let (tab_snap, tab_patch) = self.tab_snap.sync_rows_with_patch(
            next_fold.revision(),
            old_prefix..old_source_count,
            inserted_lines,
        );
        let tab_edit =
            tab_patch
                .edits()
                .first()
                .cloned()
                .unwrap_or_else(|| crate::layer::LayerEdit {
                    old: old_prefix..old_source_count,
                    new: new_prefix..new_display_count,
                });
        let widths: Vec<usize> = (new_prefix..new_display_count)
            .map(|row| {
                let buffer_row = next_fold.buffer_row_for_display(row);
                tab_snap.line_display_width(row) + next_inlay.row_extra_width(buffer_row)
            })
            .collect();
        let (wrap_snap, wrap_patch) = self.wrap_snap.sync_rows_with_patch(
            tab_snap.revision(),
            tab_snap.tab_size(),
            tab_edit.old,
            &widths,
        );
        let wrap_edit = wrap_patch
            .edits()
            .first()
            .expect("sync_rows_with_patch always returns one edit");
        let old_wrap_start = wrap_edit.old.start;
        let old_wrap_end = wrap_edit.old.end;
        let new_wrap_start = wrap_edit.new.start;
        let had_blocks = self.block_snap.has_blocks();
        let (block_snap, block_patch) = if had_blocks {
            let old_wrap_end = self.wrap_snap.visual_row_count();
            self.block_snap.sync_rows_with_patch(
                wrap_snap.revision(),
                old_wrap_start..old_wrap_end.max(old_wrap_start + 1),
                wrap_snap.visual_row_count().saturating_sub(new_wrap_start),
            )
        } else {
            (
                self.block_snap
                    .sync_input(wrap_snap.revision(), wrap_snap.visual_row_count()),
                crate::layer::LayerPatch::single(
                    old_wrap_start..old_wrap_end,
                    new_wrap_start..wrap_snap.visual_row_count(),
                ),
            )
        };
        let block_edit = block_patch
            .edits()
            .first()
            .expect("block sync always returns one edit");
        let old_block_rows = block_edit.old.clone();
        let new_block_rows = block_edit.new.clone();
        self.snapshot = snapshot;
        self.inlay_snap = next_inlay;
        self.fold_snap = next_fold;
        self.tab_snap = tab_snap;
        self.wrap_snap = wrap_snap;
        self.block_snap = block_snap;
        let (propagated_old, propagated_new) = if had_blocks {
            (old_block_rows, new_block_rows)
        } else {
            let edit = wrap_patch
                .edits()
                .first()
                .expect("wrap sync always returns one edit");
            (edit.old.clone(), edit.new.clone())
        };
        DisplayPatch::single(
            old_rows.start.min(propagated_old.start)..propagated_old.end.max(old_rows.end),
            new_rows.start.min(propagated_new.start)..propagated_new.end.max(new_rows.end),
        )
    }

    /// buffer 行 → 可视行数（首可视行, 片段数）。
    pub fn row_layout(&self, buffer_row: usize) -> (u32, u32) {
        let (_, wrap_snap) = self.layout();
        let disp = self.fold_snap.display_row(buffer_row);
        let first = wrap_snap.first_visual_row(disp);
        let count = wrap_snap.row_fragment_count(disp);
        (first as u32, count as u32)
    }

    /// buffer 行 → 首个可视行号。
    pub fn first_visual_row(&self, buffer_row: usize) -> u32 {
        let (_, wrap_snap) = self.layout();
        let disp = self.fold_snap.display_row(buffer_row);
        wrap_snap.first_visual_row(disp) as u32
    }

    /// 将 buffer 行内的 UTF-8 字节列映射到所在 visual row，兼容软换行片段。
    ///
    /// 沿四层管线逐层映射：Buffer → Inlay → Fold → Tab → Wrap，
    /// 确定该点在 inlay 展开显示列内、折叠展示行内、tab 展开显示列内所属的软换行片段行
    /// （DM-304：Inlay 层插在 Buffer 与 Fold 之间，空 inlay 时恒等）。
    pub fn visual_row_for_column(&self, buffer_row: usize, byte_column: usize) -> u32 {
        let (tab_snap, wrap_snap) = self.layout();
        if matches!(self.soft_wrap, SoftWrap::None) {
            let inlay = self
                .inlay_snap
                .map_input_by(BufferPoint::new(buffer_row, byte_column));
            return self
                .fold_snap
                .map_input_by(inlay.get(Bias::Left))
                .get(Bias::Left)
                .row as u32;
        }
        let inlay = self
            .inlay_snap
            .map_input_by(BufferPoint::new(buffer_row, byte_column));
        let fold = self.fold_snap.map_input_by(inlay.get(Bias::Left));
        let tab = tab_snap.map_input_by(fold.get(Bias::Left));
        let wrap = wrap_snap.map_input_by(tab.get(Bias::Left));
        wrap.get(Bias::Left).row as u32
    }

    /// 可视行总数。
    pub fn visual_row_count(&self) -> usize {
        self.layout().1.visual_row_count()
    }

    /// 应用一份与当前 Buffer 版本匹配的 Inlay 快照，并只向 Wrap/Block 传播受影响行。
    /// Inlay 只改变行内显示宽度，不改变 buffer 行数，因此 Tab/Fold 坐标可直接复用。
    pub fn set_inlays(&mut self, next: InlaySnapshot) -> DisplayPatch {
        self.materialize_layout();
        if next.input_version() != self.snapshot.version() {
            return DisplayPatch::default();
        }
        let scope = next.row_scope();
        let next = self.inlay_snap.merge_scoped(&next);
        let row_count = self
            .snapshot
            .line_count()
            .max(self.inlay_snap.row_count())
            .max(next.row_count());
        let scan = scope.unwrap_or(0..row_count);
        let mut dirty_start = None;
        let mut dirty_end = 0usize;
        for row in scan.start.min(row_count)..scan.end.min(row_count) {
            if self.inlay_snap.row_extra_width(row) != next.row_extra_width(row) {
                dirty_start.get_or_insert(row);
                dirty_end = row + 1;
            }
        }
        self.inlay_snap = next;
        let Some(dirty_start) = dirty_start else {
            return DisplayPatch::default();
        };

        let display_start = self.fold_snap.display_row(dirty_start);
        let display_end = self
            .fold_snap
            .display_row(dirty_end)
            .max(display_start + 1)
            .min(self.tab_snap.row_count());
        let old_wrap_start = self.wrap_snap.first_visual_row(display_start);
        let old_wrap_end = self
            .wrap_snap
            .first_visual_row(display_end.min(self.wrap_snap.source_row_count()));
        let had_blocks = self.block_snap.has_blocks();
        let widths: Vec<usize> = (display_start..display_end)
            .map(|display_row| {
                let buffer_row = self.fold_snap.buffer_row_for_display(display_row);
                self.tab_snap.line_display_width(display_row)
                    + self.inlay_snap.row_extra_width(buffer_row)
            })
            .collect();
        let (wrap_snap, wrap_patch) = self.wrap_snap.sync_rows_with_patch(
            self.tab_snap.revision(),
            self.tab_snap.tab_size(),
            display_start..display_end,
            &widths,
        );
        let wrap_edit = wrap_patch
            .edits()
            .first()
            .expect("sync_rows_with_patch always returns one edit");
        let new_wrap_start = wrap_edit.new.start;
        let new_wrap_end = wrap_edit.new.end;
        let (block_snap, block_patch) = if had_blocks {
            self.block_snap.sync_rows_with_patch(
                wrap_snap.revision(),
                old_wrap_start..old_wrap_end.max(old_wrap_start + 1),
                new_wrap_end.saturating_sub(new_wrap_start).max(1),
            )
        } else {
            (
                self.block_snap
                    .sync_input(wrap_snap.revision(), wrap_snap.visual_row_count()),
                crate::layer::LayerPatch::single(
                    old_wrap_start..old_wrap_end,
                    new_wrap_start..new_wrap_end,
                ),
            )
        };
        self.wrap_snap = wrap_snap;
        self.block_snap = block_snap;
        DisplayPatch::from_layer_patch(&block_patch)
    }

    /// DM-310~316：把已 resolve 到 wrap 行的 blocks（CodeLens 等）注入为顶层
    /// Block 坐标层。`provider_revision` 由调用方绑定（版本/revision 变则 is_current
    /// 判过期）。wrap_row 需 < base_rows，越界会钳制。
    pub fn set_blocks(&mut self, provider_revision: u64, blocks: Vec<Block>) {
        self.materialize_layout();
        let base_rows = self.wrap_snap.visual_row_count();
        let input_version = self.wrap_snap.revision();
        // 若输入版本（wrap 行数/wrap revision）未变，走增量 replace_blocks，仅重建
        // 相交行区间（DM-315 path 共享）；否则全量重建。
        self.block_snap = if self.block_snap.input_version() == input_version
            && self.block_snap.base_rows() == base_rows
        {
            self.block_snap
                .replace_blocks(input_version, provider_revision, blocks)
        } else {
            BlockSnapshot::new(input_version, provider_revision, blocks, base_rows)
        };
    }

    /// 清除所有 block（回到恒等 Block 层）。
    pub fn clear_blocks(&mut self) {
        self.materialize_layout();
        let base_rows = self.wrap_snap.visual_row_count();
        self.block_snap = BlockSnapshot::new(self.wrap_snap.revision(), 0, Vec::new(), base_rows);
    }

    /// 访问当前 Block 层（供 UI 查询块坐标/命中）。
    pub fn block_snapshot(&self) -> &BlockSnapshot {
        &self.block_snap
    }

    /// wrap 行 `r` 的内容（after before-blocks）所在的 Block 显示行，供像素换算
    /// （DM-312 UI 消费；stride 计文本行、块高单独计）。
    pub fn block_content_row(&self, wrap_row: usize) -> usize {
        let _ = self.layout();
        self.block_snap.content_display_row(wrap_row)
    }

    /// block 前缀高度（以额外显示行计）。布局、渲染和命中测试统一使用该摘要。
    pub fn block_extra_rows_before(&self, wrap_row: usize) -> usize {
        let _ = self.layout();
        self.block_snap.extra_rows_before(wrap_row)
    }

    /// Block 显示行的总行数 = 基础行数 + Σ 块高；滚动条/总高度以此为顶层行数。
    pub fn block_total_rows(&self) -> usize {
        let (_, wrap_snap) = self.layout();
        if self.block_snap.has_blocks() {
            self.block_snap.total_height_rows()
        } else {
            wrap_snap.visual_row_count()
        }
    }

    /// 任意可视行号对应的行信息（按需计算，组合三层反向映射 + 字节换算）。
    pub fn visual_line_at(&self, visual_row: usize) -> Option<VisualLine> {
        let (tab_snap, wrap_snap) = self.layout();
        let wrap = wrap_snap.visual_line_at(visual_row)?;
        // wrap.row 是 TabPoint.row == FoldPoint.row（展示行；tab 不改行号）。
        let display_row = wrap.row;
        let buffer_row = self.fold_snap.buffer_row_for_display(display_row);
        let tab_size = tab_snap.tab_size();
        // 读一次该行文本，兼顾两端点换算（DM-007：视口查询只扫描视口内行）。
        let start = self.snapshot.line_start(buffer_row);
        let bytes = line_len_bytes(&self.snapshot, buffer_row);
        let text = self
            .snapshot
            .text_in_range(Range::new(start, start + bytes));
        #[cfg(test)]
        self.viewport_scan_work
            .set(self.viewport_scan_work.get() + text.len());
        let (column_start, column_end) = if matches!(self.soft_wrap, SoftWrap::None) {
            (0, bytes)
        } else {
            (
                display_column_to_byte(&text, wrap.col_start, tab_size).min(bytes),
                display_column_to_byte(&text, wrap.col_end, tab_size)
                    .max(display_column_to_byte(&text, wrap.col_start, tab_size))
                    .min(bytes),
            )
        };
        Some(VisualLine {
            buffer_row,
            column_start,
            column_end,
            first_fragment: wrap.first_fragment,
        })
    }

    /// 指定可视行区间内的行信息（供前端只渲染 viewport）。
    pub fn visual_lines(&self, start: usize, end: usize) -> impl Iterator<Item = VisualLine> {
        (start..end).filter_map(move |v| self.visual_line_at(v))
    }

    /// 折叠状态（委托 FoldSnapshot）。
    pub fn folds(&self) -> &[Fold] {
        self.fold_snap.folds()
    }

    /// DM-007：视图查询累计扫描的字符数（测试专用，`#[cfg(test)]` 时才存在）。
    #[cfg(test)]
    pub fn viewport_scan_work(&self) -> usize {
        self.viewport_scan_work.get()
    }
}

/// 展示行（fold 之后可见）总数 = buffer 行数 − 各折叠隐藏的内部行数。
fn visible_display_rows(snapshot: &BufferSnapshot, fold_snap: &FoldSnapshot) -> usize {
    let line_count = snapshot.line_count().max(1);
    let hidden: usize = fold_snap
        .folds()
        .iter()
        .map(|fold| fold.end_row.saturating_sub(fold.start_row))
        .sum();
    line_count.saturating_sub(hidden).max(1)
}

fn line_range(snapshot: &BufferSnapshot, row: usize) -> Range {
    let start = snapshot.line_start(row);
    let end = if row + 1 < snapshot.line_count() {
        snapshot.line_start(row + 1)
    } else {
        snapshot.len()
    };
    let mut end = end;
    if end > start && snapshot.byte_at(end - 1) == Some(b'\n') {
        end -= 1;
    }
    if end > start && snapshot.byte_at(end - 1) == Some(b'\r') {
        end -= 1;
    }
    Range::new(start, end)
}

fn display_width_for_line(snapshot: &BufferSnapshot, row: usize, tab_size: usize) -> usize {
    let text = snapshot.text_in_range(line_range(snapshot, row));
    let tab_size = tab_size.max(1);
    let mut column = 0usize;
    for ch in text.chars() {
        column = if ch == '\t' {
            (column / tab_size + 1) * tab_size
        } else {
            column + ch.len_utf16()
        };
    }
    column
}

/// buffer 行内容字节数（去掉末尾 `\n`/`\r\n`）。
fn line_len_bytes(snapshot: &BufferSnapshot, buffer_row: usize) -> usize {
    let start = snapshot.line_start(buffer_row);
    let end = if buffer_row + 1 < snapshot.line_count() {
        snapshot.line_start(buffer_row + 1)
    } else {
        snapshot.len()
    };
    let mut end = end;
    if end > start && end > 0 && snapshot.byte_at(end - 1) == Some(b'\n') {
        end -= 1;
    }
    if end > start && end > 0 && snapshot.byte_at(end - 1) == Some(b'\r') {
        end -= 1;
    }
    end.saturating_sub(start)
}

fn display_column_to_byte(text: &str, target: usize, tab_size: usize) -> usize {
    let tab_size = tab_size.max(1);
    let mut column = 0usize;
    for (byte, character) in text.char_indices() {
        if column >= target {
            return byte;
        }
        let next = if character == '\t' {
            (column / tab_size + 1) * tab_size
        } else {
            column + character.len_utf16()
        };
        if next > target {
            return byte;
        }
        column = next;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::EditorBuffer;

    fn snapshot_of(text: &str) -> BufferSnapshot {
        EditorBuffer::new_from(text).snapshot()
    }

    #[test]
    fn no_wrap_one_visual_line_per_row() {
        let snap = snapshot_of("a\nbb\nccc\n");
        let map = DisplayMap::new(snap, SoftWrap::None, 80, Vec::new());
        // "a\nbb\nccc\n" → 3 内容行 + 1 末尾空行 = 4 可视行。
        assert_eq!(map.visual_row_count(), 4);
        assert_eq!(map.row_layout(0), (0, 1));
        assert_eq!(map.row_layout(2), (2, 1));
    }

    #[test]
    fn folding_skips_internal_rows() {
        let snap = snapshot_of("x\ny\nz\na\n");
        let map = DisplayMap::new(
            snap,
            SoftWrap::None,
            80,
            vec![Fold {
                start_row: 1,
                end_row: 2,
            }],
        );
        // 可视行：0(x)、1(折叠行 y..z)、3(a)、4(末尾空行) → 共 4 行。
        assert_eq!(map.visual_row_count(), 4);
        let first = map.first_visual_row(3);
        assert_eq!(first, 2);
        let rows: Vec<usize> = (0..map.visual_row_count())
            .filter_map(|v| map.visual_line_at(v))
            .map(|vl| vl.buffer_row)
            .collect();
        assert_eq!(rows, vec![0, 1, 3, 4]);
    }

    #[test]
    fn visual_line_at_maps_fragments_correctly() {
        let snap = snapshot_of("aaa\nbb\n");
        let map = DisplayMap::new(snap, SoftWrap::None, 80, Vec::new());
        let v0 = map.visual_line_at(0).unwrap();
        assert_eq!(
            (
                v0.buffer_row,
                v0.first_fragment,
                v0.column_start,
                v0.column_end
            ),
            (0, true, 0, 3)
        );
        let v1 = map.visual_line_at(1).unwrap();
        assert_eq!((v1.buffer_row, v1.first_fragment), (1, true));
        assert!(map.visual_line_at(99).is_none());
    }

    #[test]
    fn soft_wrap_splits_long_lines() {
        let long = "a".repeat(200);
        let snap = snapshot_of(&format!("{long}\n"));
        let map = DisplayMap::new(snap, SoftWrap::EditorWidth, 80, Vec::new());
        assert_eq!(map.row_layout(0).1, 3);
        let f0 = map.visual_line_at(0).unwrap();
        let f1 = map.visual_line_at(1).unwrap();
        let f2 = map.visual_line_at(2).unwrap();
        assert_eq!(
            (f0.column_start, f0.column_end, f0.first_fragment),
            (0, 80, true)
        );
        assert_eq!(
            (f1.column_start, f1.column_end, f1.first_fragment),
            (80, 160, false)
        );
        assert_eq!(
            (f2.column_start, f2.column_end, f2.first_fragment),
            (160, 200, false)
        );
        assert_eq!(map.visual_line_at(3).unwrap().buffer_row, 1);
    }

    #[test]
    fn soft_wrap_chinese_mixed_line_fragments_cover_line() {
        let line = "你好，世界 hello 中文abcdefghij".repeat(4);
        let bytes = line.len();
        let snap = snapshot_of(&format!("{line}\n"));
        let map = DisplayMap::new(snap, SoftWrap::EditorWidth, 40, Vec::new());
        let total = map.row_layout(0).1 as usize;
        assert!(total > 1, "long line should wrap into multiple fragments");
        let frags: Vec<VisualLine> = (0..total).filter_map(|v| map.visual_line_at(v)).collect();
        assert_eq!(frags[0].column_start, 0);
        assert_eq!(frags[total - 1].column_end, bytes);
        for w in frags.windows(2) {
            assert_eq!(
                w[0].column_end, w[1].column_start,
                "fragments must be contiguous"
            );
            assert!(w[0].column_end >= w[0].column_start);
        }
        assert!(frags.iter().all(|f| f.buffer_row == 0));
    }

    #[test]
    fn tab_map_expands_tabs_for_wrap_and_column_mapping() {
        let snap = snapshot_of("\tabc\n");
        let map = DisplayMap::new_with_tab_size(snap, SoftWrap::EditorWidth, 4, Vec::new(), 4);
        assert_eq!(map.row_layout(0).1, 2);
        let first = map.visual_line_at(0).unwrap();
        let second = map.visual_line_at(1).unwrap();
        assert_eq!((first.column_start, first.column_end), (0, 1));
        assert_eq!((second.column_start, second.column_end), (1, 4));
        assert_eq!(map.visual_row_for_column(0, 1), 1);
    }

    #[test]
    fn single_line_change_updates_wrap_index_without_rebuilding_rows() {
        let mut buffer = EditorBuffer::new_from("short\n".to_owned().as_str());
        let mut map = DisplayMap::new(buffer.snapshot(), SoftWrap::EditorWidth, 3, Vec::new());
        let edit = buffer.edit(Range::new(0, 5), "abcdefgh", 8, 8, true);
        map.apply_change(buffer.snapshot(), &edit.changes[0], Vec::new());
        assert_eq!(map.row_layout(0), (0, 3));
        assert_eq!(map.visual_row_count(), 4);
        assert_eq!(map.visual_line_at(2).unwrap().column_end, 8);
        assert_eq!(map.visual_line_at(3).unwrap().buffer_row, 1);
    }

    #[test]
    fn incremental_change_returns_dirty_patch_and_preserves_suffix() {
        let mut buffer = EditorBuffer::new_from("a\nb\nc\n");
        let mut map = DisplayMap::new(buffer.snapshot(), SoftWrap::EditorWidth, 80, Vec::new());
        let edit = buffer.edit(Range::new(2, 3), "long", 4, 4, true);
        let patch = map.apply_change_with_patch(buffer.snapshot(), &edit.changes[0], Vec::new());
        assert_eq!(patch.edits()[0].old_rows, 1..2);
        assert_eq!(patch.edits()[0].new_rows, 1..2);
        assert_eq!(map.visual_line_at(2).unwrap().buffer_row, 2);
        assert_eq!(map.visual_line_at(3).unwrap().buffer_row, 3);
    }

    #[test]
    fn fold_change_rebuilds_only_suffix_and_keeps_mapping_correct() {
        let mut buffer = EditorBuffer::new_from("a\nb\nc\nd\ne\n");
        let mut map = DisplayMap::new(
            buffer.snapshot(),
            SoftWrap::None,
            80,
            vec![Fold {
                start_row: 2,
                end_row: 3,
            }],
        );
        let edit = buffer.edit(Range::new(0, 0), "x\n", 2, 2, true);
        let patch = map.apply_change_with_patch(
            buffer.snapshot(),
            &edit.changes[0],
            vec![Fold {
                start_row: 3,
                end_row: 4,
            }],
        );
        assert!(patch.edits()[0].new_rows.start <= 1);
        let rows: Vec<usize> = (0..map.visual_row_count())
            .filter_map(|row| map.visual_line_at(row))
            .map(|line| line.buffer_row)
            .collect();
        assert_eq!(rows, vec![0, 1, 2, 3, 5, 6]);
    }

    #[test]
    fn display_patch_consolidates_adjacent_edits() {
        let mut patch = DisplayPatch::default();
        patch.push(1..2, 1..3);
        patch.push(2..4, 3..4);
        patch.push(8..9, 8..9);
        assert_eq!(patch.edits().len(), 2);
        assert_eq!(patch.edits()[0].old_rows, 1..4);
        assert_eq!(patch.edits()[0].new_rows, 1..4);
        assert_eq!(patch.old_row_bounds(), Some(1..9));
        assert_eq!(patch.new_row_bounds(), Some(1..9));
        assert_eq!(patch.row_delta(), 0);
    }

    #[test]
    fn display_patch_composes_insert_and_delete_in_mid_coordinates() {
        // old -> mid inserts one visual row at 2.
        let first = DisplayPatch::single(2..2, 2..3);
        // mid -> new deletes the inserted row at 2 and then replaces mid row 5.
        let second = DisplayPatch {
            edits: vec![
                DisplayEdit {
                    old_rows: 2..3,
                    new_rows: 2..2,
                },
                DisplayEdit {
                    old_rows: 5..6,
                    new_rows: 5..7,
                },
            ],
        };
        let composed = first.compose(&second);
        assert_eq!(composed.edits(), &[second.edits()[1].clone()]);
        assert_eq!(composed.old_to_new(2), 2);
        assert_eq!(composed.old_to_new(5), 5);
    }

    #[test]
    fn display_patch_composes_row_shift_into_following_edit() {
        let first = DisplayPatch::single(2..2, 2..3);
        let second = DisplayPatch::single(3..4, 3..5);
        let composed = first.compose(&second);
        assert_eq!(
            composed.edits(),
            &[DisplayEdit {
                old_rows: 2..3,
                new_rows: 2..5,
            }]
        );
    }

    #[test]
    fn display_patch_composes_overlapping_replacements() {
        let first = DisplayPatch::single(10..14, 10..12);
        let second = DisplayPatch::single(10..12, 10..16);
        let composed = first.compose(&second);
        assert_eq!(
            composed.edits(),
            &[DisplayEdit {
                old_rows: 10..14,
                new_rows: 10..16,
            }]
        );
    }

    #[test]
    fn display_patch_composes_disjoint_edits_across_row_delta() {
        let first = DisplayPatch::new(vec![
            DisplayEdit {
                old_rows: 1..3,
                new_rows: 1..4,
            },
            DisplayEdit {
                old_rows: 8..12,
                new_rows: 9..11,
            },
        ]);
        let second = DisplayPatch::new(vec![
            DisplayEdit {
                old_rows: 0..0,
                new_rows: 0..4,
            },
            DisplayEdit {
                old_rows: 3..10,
                new_rows: 7..9,
            },
        ]);
        let composed = first.compose(&second);
        assert_eq!(
            composed.edits(),
            &[
                DisplayEdit {
                    old_rows: 0..0,
                    new_rows: 0..4,
                },
                DisplayEdit {
                    old_rows: 1..12,
                    new_rows: 5..10,
                },
            ]
        );
    }

    #[test]
    fn display_patch_maps_positions_and_inverts() {
        let mut patch = DisplayPatch::default();
        patch.push(2..4, 2..5);
        patch.push(8..9, 9..9);
        assert_eq!(patch.old_to_new(1), 1);
        assert_eq!(patch.old_to_new(2), 2);
        assert_eq!(patch.old_to_new(3), 2);
        assert_eq!(patch.old_to_new(4), 5);
        assert_eq!(patch.old_to_new(9), 9);
        assert_eq!(patch.edit_for_old_position(6).old_rows, 6..6);
        patch.invert();
        assert_eq!(patch.edits()[0].old_rows, 2..5);
        assert_eq!(patch.edits()[0].new_rows, 2..4);
    }

    #[test]
    fn set_inlays_updates_only_affected_wrap_rows() {
        let buffer = EditorBuffer::new_from("abcd\nxy");
        let snapshot = buffer.snapshot();
        let mut map = DisplayMap::new(snapshot.clone(), SoftWrap::EditorWidth, 4, Vec::new());
        let next = InlaySnapshot::new(
            snapshot.version(),
            9,
            vec![crate::inlay_map::Inlay::new(
                crate::inlay_map::InlayId(1),
                2,
                Bias::Left,
                "hint",
                4,
                None,
            )],
            |offset| {
                let point = snapshot.offset_to_point(offset);
                (point.row, point.column)
            },
        );
        let patch = map.set_inlays(next);
        assert_eq!(patch.edits()[0].old_rows, 0..1);
        assert!(patch.edits()[0].new_rows.end > patch.edits()[0].new_rows.start);
        assert!(map.visual_row_count() >= 3);
    }

    #[test]
    fn incremental_patch_uses_block_display_coordinates_when_blocks_exist() {
        let mut buffer = EditorBuffer::new_from("a\nb\n");
        let mut map = DisplayMap::new(buffer.snapshot(), SoftWrap::None, 80, Vec::new());
        map.set_blocks(
            1,
            vec![Block {
                id: crate::block_map::BlockId(1),
                wrap_row: 1,
                before: true,
                height_rows: 1,
                payload: 0,
            }],
        );
        let edit = buffer.edit(Range::new(2, 3), "long", 6, 6, false);
        let patch = map.apply_change_with_patch(buffer.snapshot(), &edit.changes[0], Vec::new());
        assert_eq!(patch.edits()[0].old_rows, 1..3);
        assert_eq!(patch.edits()[0].new_rows, 1..2);
    }

    #[test]
    fn multiline_change_falls_back_to_full_index_rebuild() {
        let mut buffer = EditorBuffer::new_from("one\ntwo\n");
        let mut map = DisplayMap::new(buffer.snapshot(), SoftWrap::EditorWidth, 80, Vec::new());
        let edit = buffer.edit(Range::new(3, 3), "x\n", 5, 5, true);
        map.apply_change(buffer.snapshot(), &edit.changes[0], Vec::new());
        assert_eq!(map.visual_row_count(), buffer.line_count());
        assert_eq!(map.visual_line_at(1).unwrap().buffer_row, 1);
    }

    #[test]
    fn cross_line_same_newline_count_rebuilds_both_rows() {
        let mut buffer = EditorBuffer::new_from("a\nb\n");
        let mut map = DisplayMap::new(buffer.snapshot(), SoftWrap::EditorWidth, 1, Vec::new());
        let edit = buffer.edit(Range::new(0, 3), "long\nx", 6, 6, true);
        map.apply_change(buffer.snapshot(), &edit.changes[0], Vec::new());
        assert_eq!(map.row_layout(0).1, 4);
        assert_eq!(map.row_layout(1).1, 1);
    }

    /// DM-006：CRLF 换行的可视行数与列区间正确（不把 `\r` 计入行内容错位）。
    #[test]
    fn crlf_documents_map_rows_correctly() {
        let snap = snapshot_of("a\r\nb\r\nc\r\n");
        let map = DisplayMap::new(snap, SoftWrap::None, 80, Vec::new());
        assert_eq!(map.visual_row_count(), 4);
        let v0 = map.visual_line_at(0).unwrap();
        assert_eq!((v0.buffer_row, v0.column_start, v0.column_end), (0, 0, 1));
        let v1 = map.visual_line_at(1).unwrap();
        assert_eq!((v1.buffer_row, v1.column_start, v1.column_end), (1, 0, 1));
    }

    /// DM-006：fold + tab + wrap 组合的 golden 行为。
    /// 折叠行内部可视行被隐藏，Tab 按 tab stop 展开列，软换行按列切片。
    #[test]
    fn fold_tab_wrap_combined_golden() {
        let snap = snapshot_of("abcd\tef\nhidden\na\nz\n");
        let map = DisplayMap::new_with_tab_size(
            snap,
            SoftWrap::EditorWidth,
            7,
            vec![Fold {
                start_row: 1,
                end_row: 2,
            }],
            4,
        );
        assert_eq!(map.visual_row_count(), 5);
        let rows: Vec<usize> = (0..map.visual_row_count())
            .filter_map(|v| map.visual_line_at(v))
            .map(|vl| vl.buffer_row)
            .collect();
        assert_eq!(rows, vec![0, 0, 1, 3, 4]);
        // 第 0 行 `abcd\tef`：显示列 = 4 + 跳到8 + 2 = 10；wrap 7 → 2 片段。
        assert_eq!(map.row_layout(0).1, 2);
    }

    #[test]
    fn large_soft_wrap_map_builds_layout_lazily() {
        let mut doc = String::with_capacity(6_000 * 202);
        for _ in 0..6_000 {
            doc.push_str(&"x".repeat(200));
            doc.push('\n');
        }
        let snap = snapshot_of(&doc);
        let map = DisplayMap::new(snap, SoftWrap::EditorWidth, 80, Vec::new());
        let lazy = map
            .lazy_layout
            .as_ref()
            .expect("large documents use lazy layout");
        assert!(
            lazy.cell.get().is_none(),
            "constructor must not build all wrap rows"
        );
        assert_eq!(map.visual_row_count(), 6_000 * 3 + 1);
        assert!(
            lazy.cell.get().is_some(),
            "first layout query materializes exact rows"
        );
    }

    /// DM-007：viewport 查询不物化/不扫描全文 VisualLine。
    #[test]
    fn viewport_query_does_not_materialize_full_document() {
        let line = "x".repeat(320);
        let mut doc = String::with_capacity(100_000 * 321);
        for _ in 0..100_000 {
            doc.push_str(&line);
            doc.push('\n');
        }
        let snap = snapshot_of(&doc);
        let map = DisplayMap::new(snap, SoftWrap::EditorWidth, 80, Vec::new());
        let total = map.visual_row_count();
        assert_eq!(total, 100_000 * 4 + 1);
        let view_start = 100_000usize;
        let view_end = view_start + 8;
        let rows: Vec<VisualLine> = map.visual_lines(view_start, view_end).collect();
        assert_eq!(rows.len(), 8);
        let scanned = map.viewport_scan_work();
        assert!(
            scanned <= 8 * 320,
            "viewport query scanned {scanned} chars, expected ≤{viewport}",
            viewport = 8 * 320
        );
    }

    #[test]
    fn fold_and_inlay_do_not_remap_highlight_buffer_range() {
        // DM-407：语法高亮始终按 buffer 字节坐标存储与查询，fold/inlay 只在
        // 输出坐标层裁剪，不改动高亮 range。这里验证「折叠行 + inlay 块同时存在时，
        // 对折叠可视行的 buffer 字节区间做 `iter_intersecting` 仍能命中横跨折叠区的高亮」。
        let snap = snapshot_of("aa\nbb\ncc\ndd\n");
        // 折叠第 1..2 行（bb、cc），可视行变为 0(aa)、1(折叠 bb..cc)、3(dd)、4(末尾)。
        let mut map = DisplayMap::new(
            snap.clone(),
            SoftWrap::None,
            80,
            vec![Fold {
                start_row: 1,
                end_row: 2,
            }],
        );
        // 在第 2 个可视行上方插入一个 inlay 块（CodeLens 形态），验证不影响高亮坐标层。
        map.set_blocks(
            1,
            vec![Block {
                id: crate::block_map::BlockId(0),
                // 折叠行的可视行号 = 1。
                wrap_row: 1,
                before: true,
                height_rows: 1,
                payload: 7,
            }],
        );

        // 一条横跨折叠区（bb..cc 两行）的高亮：buffer 字节范围 [3, 11)。
        // "aa\nbb\ncc\ndd\n" → 下标 0-2='aa'，3='\n'，4-5='bb'，6='\n'，7-8='cc'，9='\n'...
        let highlights = crate::syntax::HighlightStore::from_highlights(
            std::sync::Arc::new(vec![crate::Highlight {
                range: Range::new(3, 11),
                kind: "comment".into(),
            }]),
            snap.len(),
        );

        // 折叠可视行（buffer_row=1）的 buffer 字节区间；
        // 即便内容折叠、即便该行上方有 inlay 块，画到 buffer 字节区仍应命中该高亮。
        let folded_line = map.visual_line_at(1).unwrap();
        assert_eq!(folded_line.buffer_row, 1);
        let line_start = snap.point_to_offset(crate::model::Point {
            row: folded_line.buffer_row,
            column: 0,
        });
        let line_end_off = snap.point_to_offset(crate::model::Point {
            row: folded_line.buffer_row + 1,
            column: 0,
        });
        let hit = highlights.iter_intersecting(Range::new(line_start, line_end_off));
        assert_eq!(hit.len(), 1, "折叠/内联不应破坏高亮 buffer 区间命中");
        assert_eq!(hit[0].range, Range::new(3, 11));
    }
}
