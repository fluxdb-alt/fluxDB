//! BlockMap：把非行内内容（CodeLens、诊断详情、行前/行后 widget）作为「额外的
//! 整行」插入显示流（DM-310，Phase 3）。
//!
//! 显示管线第五个 [`LayerSnapshot`]：输入 [`WrapPoint`]（soft-wrap 后的展示行），
//! 输出 [`BlockPoint`]（块插入后的显示行，参与滚动 y）。Block 是合成视图：它**增加
//! 显示行数与总高度**，但不改变任何 buffer byte 语义、不写入 Buffer、不参与 undo
//! （设计 §8.5）。典型消费者：CodeLens、诊断详情行、行首/行尾 widget。
//!
//! 本层是独立自包含展示层，仅依赖 `coordinates` / `model` / `layer` / `sum_tree`，
//! 不依赖 UI。每个 block 归属某个 [`WrapPoint`] 行（`wrap_row`），提供
//! `before/after` placement、高度（以**额外行数** `height_rows` 计）与可渲染 payload
//! id（core 只存 id，不存 GPUI element，DM-311）。
//!
//! 几何模型：对 wrap 行 `r`，其**内容**所在 block 行 = `r + Σ(位于 r 及其 before 前缀
//! 的 block 高)`；该行上方（before 前缀）的 block 行占据两者之间。块高与会话行一并
//! 存入 [`SumTree`]（每 wrap 行一叶，`lines` = 1 个内容行 + 该行块高），`total` /
//! `summary_before_leaf` / `locate_lines` 让 `total_height`、scroll-to-cursor 与
//! hit test **共享同一份摘要**（DM-314）。
//!
//! `ponytail:` 本层先以「已 resolve 的 block 位置」（wrap 行）直接驱动；惰性后台
//! provider 填充 / overscan 与 Inlay 同款升级路径（本轮 CodeLens 同步查询足够）。
//! raw [`Anchor`] 编辑自动 relocate 待接入整链时统一处理。

use crate::coordinates::{Biased, BlockPoint, WrapPoint};
use crate::layer::{LayerPatch, LayerSnapshot};
use crate::sum_tree::{IntervalSummary, SumTree, SumTreeItem, Summary};

/// 单个 block 的唯一标识。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u64);

/// 一个已 resolve 到 wrap 行的 block（DM-310）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
    /// 归属的 wrap 行（`WrapPoint.row`）。
    pub wrap_row: usize,
    /// placement：`true` = before（行上方），`false` = after（行下方）。
    pub before: bool,
    /// 该 block 消耗的**额外显示行数**（CodeLens 固定 1）。
    pub height_rows: u32,
    /// UI 可渲染 payload id（core 只存 id，不解释；DM-311）。
    pub payload: u64,
}

/// 按 wrap 行归组、且已按 id 升序（稳定）的 block。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowBlock {
    /// 该 block 消耗的额外显示行数。
    height: u32,
    id: BlockId,
    payload: u64,
}

/// 高度摘要的叶子条目：每个 wrap 行一项（DM-314）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockRowItem {
    /// wrap 行序号（按此升序，区间包络 = `[row, row+1)`）。
    row: usize,
    /// 该行消耗的额外显示行数（Σ 该行块高）。
    height_rows: usize,
}

impl SumTreeItem for BlockRowItem {
    type Summary = BlockRowSummary;

    fn summary(&self) -> Self::Summary {
        BlockRowSummary {
            start: self.row,
            end: self.row + 1,
            lines: 1 + self.height_rows, // 1 个内容行 + 块高
        }
    }
}

/// 行高摘要：`lines` = 显示行数（内容 + 块高）；`[start, end)` = 行号区间包络。
/// 区间包络使 DM-315 局部编辑只替换相交 path、未触碰后缀 subtree `Arc` 共享。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BlockRowSummary {
    start: usize,
    end: usize,
    lines: usize,
}

impl Summary for BlockRowSummary {
    fn add(&self, other: &Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            lines: self.lines + other.lines,
        }
    }
}

impl IntervalSummary for BlockRowSummary {
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
}

/// Block 显示层的不可变快照。
pub struct BlockSnapshot {
    /// 绑定的输入（WrapSnapshot）版本号，构建时记录。
    input_version: u64,
    /// 本层自身 revision：由 input_version + provider revision + 每行 block 派生。
    revision: u64,
    /// 每 wrap 行的 blocks，已按 id 升序。
    rows: Vec<Vec<RowBlock>>,
    /// 以每个 wrap 行为叶的行高摘要（内容 1 行 + 块高；行号区间包络）。
    /// `summary_before_leaf` / `total` 让 `total_height`、scroll 与 hit **共享同一摘要**
    /// （DM-314）；`replace_leaves` 只为相交行区间重建（DM-315）。
    heights: SumTree<BlockRowItem>,
    /// 基础（wrap）行总数。
    base_rows: usize,
}

impl BlockSnapshot {
    /// 按 wrap 版本 + provider revision + 已 resolve block 列表构造（DM-303 同款：
    /// 结果同时绑定版本）。`rows` 总基础行数 = 输入的 wrap 行数，供 `total_height_rows`
    /// 与反查使用。构造成按行归组、稳定排序并建立高度摘要。
    pub fn new(
        input_version: u64,
        provider_revision: u64,
        blocks: Vec<Block>,
        base_rows: usize,
    ) -> Self {
        let (rows, height_items) = build_rows(blocks, base_rows);
        let heights = SumTree::from_items(&height_items);
        let revision = compute_revision(input_version, provider_revision, &rows);
        let base_rows = rows
            .len()
            .max(base_rows)
            .max(height_items.last().map_or(0, |i| i.row + 1));
        Self {
            input_version,
            revision,
            rows,
            heights,
            base_rows,
        }
    }

    /// DM-315：仅重建**相交行区间**上的高度摘要，未触碰的后缀 wrap 行 subtree
    /// `Arc` 共享（持久 tree；`shared_trailing_leaves` 测试佐证）。blocks 直接以
    /// 构建期为准，`input_version`/`provider_revision` 沿用（调用方负责换版本）。
    pub fn replace_blocks(
        &self,
        input_version: u64,
        provider_revision: u64,
        blocks: Vec<Block>,
    ) -> Self {
        // 先照旧按行归组 + 稳定排序（代价在每组；行数大的部分未触碰时靠下方
        // replace_leaves 的 path 共享省掉整树重建）。
        let (rows, height_items) = build_rows(blocks, self.base_rows);
        // 只替换变化的行区间：老树有多少行，新树就覆盖到与老树行数之较大者。
        let new_count = height_items.len();
        // 变更区间：逐行比较第一个与最后一个不同的 item，只重建这段（未触碰
        // 前缀与后缀 subtree 隐含保留 —— `replace_leaves` 是 path-copying 持久树）。
        let old_count = self.rows.len().max(self.base_rows);
        let mut span_start = new_count.min(old_count);
        let mut span_end = 0usize;
        for i in 0..new_count.min(old_count) {
            // `row` 是叶子序号摘要，行插入后后缀可能保留旧值；布局只依赖
            // height_rows，避免 stale row metadata 让整个 suffix 被误判为 dirty。
            if self.row_item(i).map(|item| item.height_rows) != Some(height_items[i].height_rows) {
                span_start = span_start.min(i);
                span_end = i + 1;
            }
        }
        // 无差异时 span_end=0 < span_start，无需重建（replace_leaves 空插入 = 恒等）。
        // 仅对「行数不变、块高/块集变化」做局部替换（CodeLens 同 buffer 版本下
        // wrap 行数恒定即此场景）；行数增长的整链 edits 走 `new()` 全量重建。
        let new_heights = if span_start < span_end {
            self.heights
                .replace_leaves(span_start, span_end, &height_items[span_start..span_end])
        } else {
            self.heights.clone()
        };
        let revision = compute_revision(input_version, provider_revision, &rows);
        let base_rows = rows
            .len()
            .max(self.base_rows)
            .max(height_items.last().map_or(0, |i| i.row + 1));
        Self {
            input_version,
            revision,
            rows,
            heights: new_heights,
            base_rows,
        }
    }

    /// 上游 wrap revision 变化但行坐标未变化时，仅更新输入版本并复用 block 高度树。
    pub fn sync_input(&self, input_version: u64, base_rows: usize) -> Self {
        if self.base_rows != base_rows {
            return Self::new(input_version, 0, Vec::new(), base_rows);
        }
        let revision = compute_revision(input_version, 0, &self.rows);
        Self {
            input_version,
            revision,
            rows: self.rows.clone(),
            heights: self.heights.clone(),
            base_rows,
        }
    }

    /// 上游 wrap 行区间发生插入/删除时迁移 block 行。dirty 行上的 block 交给
    /// provider 下一轮刷新，未触碰前后缀保持原有 payload 与高度摘要。
    pub fn sync_rows(
        &self,
        input_version: u64,
        old_range: std::ops::Range<usize>,
        new_row_count: usize,
    ) -> Self {
        let start = old_range.start.min(self.base_rows);
        let end = old_range.end.max(start).min(self.base_rows);
        let mut rows = Vec::with_capacity(
            self.base_rows
                .saturating_sub(end.saturating_sub(start))
                .saturating_add(new_row_count),
        );
        let mut old_rows = self.rows.clone();
        old_rows.resize(self.base_rows, Vec::new());
        rows.extend_from_slice(&old_rows[..start]);
        rows.extend((0..new_row_count).map(|_| Vec::new()));
        rows.extend_from_slice(&old_rows[end..]);
        let items: Vec<BlockRowItem> = (start..start.saturating_add(new_row_count))
            .map(|row| BlockRowItem {
                row,
                height_rows: 0,
            })
            .collect();
        let heights = self.heights.replace_leaves(start, end, &items);
        let base_rows = rows.len().max(1);
        let revision = compute_revision(input_version, 0, &rows);
        Self {
            input_version,
            revision,
            rows,
            heights,
            base_rows,
        }
    }

    /// 同步 Wrap 行并返回 Block 坐标空间中的 dirty edit。
    pub fn sync_rows_with_patch(
        &self,
        input_version: u64,
        old_range: std::ops::Range<usize>,
        new_row_count: usize,
    ) -> (Self, LayerPatch) {
        let old_display = self.display_range_for_wrap_rows(old_range.clone());
        let next = self.sync_rows(input_version, old_range, new_row_count);
        let new_start = old_display.start.min(next.total_height_rows());
        let new_end = (new_start + new_row_count).min(next.total_height_rows());
        (
            next,
            LayerPatch::single(old_display, new_start..new_end.max(new_start)),
        )
    }

    pub fn has_blocks(&self) -> bool {
        self.rows.iter().any(|row| !row.is_empty())
    }

    /// 高度摘要第 `row` 项（供 DM-315 前缀比较）。
    fn row_item(&self, row: usize) -> Option<BlockRowItem> {
        // 摘要树按 row 升序且一个 wrap 行恰一项，故 `row` 即叶子位置。
        let it = self.heights.get(row)?;
        Some(BlockRowItem {
            row: it.row,
            height_rows: it.height_rows,
        })
    }

    /// wrap 行总数（不含 block 插入前的行数）。
    pub fn base_rows(&self) -> usize {
        self.base_rows
    }

    /// 总显示行数 = 基础行数 + Σ(所有 block 的额外行高)。
    pub fn total_height_rows(&self) -> usize {
        self.heights.total().lines
    }

    /// 把 wrap 行 dirty 区间转换为顶层 block display 行区间。区间末端包含
    /// 最后一行的 before-block/content，确保块高度变化和后缀位移都被失效。
    pub fn display_range_for_wrap_rows(
        &self,
        range: std::ops::Range<usize>,
    ) -> std::ops::Range<usize> {
        let start = range.start.min(self.base_rows);
        let end = range.end.min(self.base_rows);
        if start >= end {
            let row = self
                .map_input_by(WrapPoint {
                    row: start,
                    column: 0,
                })
                .left
                .row
                .min(self.total_height_rows());
            return row..row;
        }
        let old_start = self
            .map_input_by(WrapPoint {
                row: start,
                column: 0,
            })
            .left
            .row;
        let last = end - 1;
        let old_end =
            self.map_input_by(WrapPoint {
                row: last,
                column: 0,
            })
            .right
            .row + 1;
        old_start..old_end.max(old_start + 1)
    }

    /// `wrap_row` 内容行之前插入的 block 总高度（以显示行计）。
    ///
    /// 这是 UI 计算像素坐标时唯一应使用的 block 几何摘要；调用方不应再次
    /// 扫描 block 列表或按行号自行计数。
    pub fn extra_rows_before(&self, wrap_row: usize) -> usize {
        self.content_block_row(wrap_row).saturating_sub(wrap_row)
    }

    /// `wrap_row` 对应的内容显示行（含该行 before block）。
    pub fn content_display_row(&self, wrap_row: usize) -> usize {
        self.content_block_row(wrap_row)
    }

    /// wrap 行 `r` 的**内容**起始 block 行（即该行 before-blocks 之后）。
    fn content_block_row(&self, r: usize) -> usize {
        let before = self
            .heights
            .summary_before_leaf(r.min(self.base_rows))
            .lines;
        before + self.rows_extra(r)
    }

    /// wrap 行 `r` 的 before 前缀 block 总高（落在该行与其上方的块高）。
    fn rows_extra(&self, r: usize) -> usize {
        self.rows
            .get(r)
            .map(|line| line.iter().map(|b| b.height as usize).sum())
            .unwrap_or(0)
    }

    /// 定位 display 行（block 空间）所属的 wrap 行。沿高度摘要前缀二分，返回首个
    /// 覆盖该 display 行的 wrap 行序号（DM-314 使滚动/命中共享同一摘要）。
    fn block_to_wrap_row(&self, display_row: usize) -> usize {
        let target = display_row.min(self.total_height_rows().saturating_sub(1));
        let mut lo = 0usize;
        let mut hi = self.base_rows;
        // 首个前缀累计 > target 的 wrap 行即归属行（内容行起点 = 前缀）。
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.heights.summary_before_leaf(mid).lines > target {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        lo.saturating_sub(1).min(self.base_rows.saturating_sub(1))
    }

    /// hit test（DM-310）：给定一个 block 展示行，若落在某 before-block 的占用行内，
    /// 返回该 block 的 id；落在文本行不命中。设计 §8.5：hit 返回 block target。
    pub fn block_at(&self, point: BlockPoint) -> Option<BlockId> {
        let r = self.block_to_wrap_row(point.row);
        let content_start = self.content_block_row(r);
        let extra = self.rows_extra(r);
        let line = self.rows.get(r)?;
        // 位于该 wrap 行 before-block 占用行的 display 行区间。
        for (i, b) in line.iter().enumerate() {
            let seg_start =
                content_start - extra + line[..i].iter().map(|x| x.height as usize).sum::<usize>();
            if point.row >= seg_start && point.row < seg_start + b.height as usize {
                return Some(b.id);
            }
        }
        None
    }
}

impl LayerSnapshot for BlockSnapshot {
    type Input = WrapPoint;
    type Output = BlockPoint;

    fn input_version(&self) -> u64 {
        self.input_version
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    /// wrap 行 → block 行。落在某 wrap 行的 before-blocks 处（内容行起点）Biased 分
    /// 左右：left = 该行块前、right = 该行内容起点（跳过其 before 块高）。
    fn map_input_by(&self, input: Self::Input) -> Biased<Self::Output> {
        let left_row = self
            .heights
            .summary_before_leaf(input.row.min(self.base_rows))
            .lines;
        let right_row = left_row + self.rows_extra(input.row);
        Biased {
            left: BlockPoint::new(left_row, input.column),
            right: BlockPoint::new(right_row, input.column),
        }
    }

    /// block 行 → wrap 行（反解，去掉块高）。落入块行内则归到其 wrap 行（与 hit 一致）。
    fn map_output_by(&self, output: Self::Output) -> Biased<Self::Input> {
        let r = self.block_to_wrap_row(output.row);
        Biased::from(WrapPoint {
            row: r,
            column: output.column,
        })
    }
}

/// 把已 resolve 的 blocks 归入每 wrap 行并按 id 稳定排序，同时生成等高摘要 item。
fn build_rows(blocks: Vec<Block>, base_rows: usize) -> (Vec<Vec<RowBlock>>, Vec<BlockRowItem>) {
    let mut rows: Vec<Vec<RowBlock>> = Vec::with_capacity(base_rows);
    for b in blocks {
        if b.wrap_row >= rows.len() {
            rows.resize(b.wrap_row + 1, Vec::new());
        }
        rows[b.wrap_row].push(RowBlock {
            height: b.height_rows,
            id: b.id,
            payload: b.payload,
        });
    }
    for line in rows.iter_mut() {
        line.sort_by_key(|b| b.id);
    }
    let n = base_rows.max(rows.len());
    let mut items = Vec::with_capacity(n);
    for r in 0..n {
        let extra: usize = rows
            .get(r)
            .map(|line| line.iter().map(|b| b.height as usize).sum())
            .unwrap_or(0);
        items.push(BlockRowItem {
            row: r,
            height_rows: extra,
        });
    }
    (rows, items)
}

/// 派生本层 revision：FNV-1a 混合 input_version、provider revision 与每行 block
/// （行号/高度/id/payload）。input_version 变化（上游编辑）或 provider_revision 变化
/// 都会翻转 revision，使 [`LayerSnapshot::is_current`] 判过期。
fn compute_revision(input_version: u64, provider_revision: u64, rows: &[Vec<RowBlock>]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100000001b3);
    mix(input_version);
    mix(provider_revision);
    mix(rows.len() as u64);
    for line in rows {
        for b in line {
            mix(b.id.0);
            mix(b.height as u64);
            mix(b.payload);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: u64, wrap_row: usize, height: u32, payload: u64) -> Block {
        Block {
            id: BlockId(id),
            wrap_row,
            before: true,
            height_rows: height,
            payload,
        }
    }

    fn snap(blocks: Vec<Block>, base_rows: usize) -> BlockSnapshot {
        BlockSnapshot::new(1, 0, blocks, base_rows)
    }

    /// DM-310：无 block 时映射恒等，总行数 = 基础行数。
    #[test]
    fn empty_is_identity() {
        let s = snap(vec![], 5);
        assert_eq!(s.total_height_rows(), 5);
        let out = s.map_input_by(WrapPoint { row: 3, column: 2 });
        assert!(out.is_identical());
        assert_eq!(out.left, BlockPoint::new(3, 2));
    }

    /// DM-310：before 块（高 H）将目标行及其后所有行下移 H；内容行在块行之后。
    #[test]
    fn before_block_shifts_down_target_and_after() {
        // 在 wrap 行 2 上方插一个 HIGH=2 的块：内容行 2 应从 block 2 挪到 4。
        let s = snap(vec![block(1, 2, 2, 7)], 6);
        assert_eq!(s.total_height_rows(), 8, "6 内容 + 2 块高");
        let below = s.map_input_by(WrapPoint { row: 2, column: 0 });
        assert_eq!(below.left.column, 0);
        assert_eq!(below.right.row, 4, "内容行 2 起点 = 2 + 2 块高");
        // 行 2 上方边界：left = 2（块前），right = 4（内容起点）
        assert_eq!(below.left.row, 2);
        assert_eq!(below.right.row, 4);
        // 行 2 之前不受影响
        let before = s.map_input_by(WrapPoint { row: 1, column: 0 });
        assert_eq!(before.left.row, 1);
    }

    /// DM-310：多块同行稳定排序（按 id）；content row = base + Σ 块高。
    #[test]
    fn multiple_blocks_same_row_sum_with_stable_order() {
        let s = snap(vec![block(5, 2, 1, 1), block(3, 2, 2, 2)], 6);
        assert_eq!(s.total_height_rows(), 9, "6 + (1+2)");
        let b = s.map_input_by(WrapPoint { row: 2, column: 0 });
        assert_eq!(b.left.row, 2);
        assert_eq!(b.right.row, 5, "内容行 2 = 2 + 3 块高");
    }

    /// DM-310：落在某 block 占用行内 hit 返回该 id；文本行不命中。
    #[test]
    fn hit_test_block_rows() {
        // wrap 行 1 上方插一个高 1 的块，其占用 block 行 = 1（内容行 1 起点=1+1=2，块占行 1）。
        let s = snap(vec![block(9, 1, 1, 42)], 4);
        // 内容行：r0→0, r1→2, r2→3, r3→4 ；块行 1 归属 wrap 行 1。
        assert_eq!(s.block_at(BlockPoint::new(1, 0)), Some(BlockId(9)));
        assert_eq!(s.block_at(BlockPoint::new(0, 0)), None, "内容行不命中");
        assert_eq!(s.block_at(BlockPoint::new(2, 0)), None, "内容行 1 不命中");
    }

    /// DM-310：双向 round-trip——block 空间的内容行反解回原 wrap 行。
    #[test]
    fn bidirectional_round_trip() {
        let s = snap(vec![block(1, 2, 1, 0), block(2, 5, 2, 0)], 8);
        // 内容行映射：r=0→0,1→1,2→3(id1),3→4,4→5,5→8(id1+id2),6→9,7→10
        for (r, expect) in [
            (0usize, 0usize),
            (1, 1),
            (2, 3),
            (3, 4),
            (4, 5),
            (5, 8),
            (6, 9),
            (7, 10),
        ] {
            let content = s.map_input_by(WrapPoint { row: r, column: 0 }).right;
            assert_eq!(content.row, expect, "wrap 行 {r} 内容行应为 {expect}");
            let back = s.map_output_by(BlockPoint::new(content.row, 0));
            assert_eq!(
                back.left.row, r,
                "内容行 {} 反解回 wrap 行 {r}",
                content.row
            );
        }
    }

    /// DM-314：高度摘要前缀正确——summary_before_leaf 累加的是「该行之前」(含内容行)。
    fn row_lines_prefix(s: &BlockSnapshot, r: usize) -> usize {
        s.heights.summary_before_leaf(r).lines
    }

    /// DM-314：滚动/命中反查——block_to_wrap_row 沿高度摘要二分定位所属 wrap 行。
    #[test]
    fn locate_display_row_to_wrap_row() {
        let s = snap(vec![block(1, 2, 1, 0), block(2, 5, 2, 0)], 8);
        // 显示行序（含块行）：内容行 0,1；块行 2(归属 wrap2), 内容3,4,5；块行6,7(归属5),内容8,9,10
        // 更正：行高摘要每叶 lines = 1+extra。r0:1,r1:1,r2:2,r3:1,r4:1,r5:3,r6:1,r7:1
        assert_eq!(row_lines_prefix(&s, 0), 0);
        assert_eq!(row_lines_prefix(&s, 2), 2, "r0,r1 各 1");
        assert_eq!(row_lines_prefix(&s, 5), 6, "r0..r4 = 1+1+2+1+1 = 6");
        // 反查：display 行 3（r2 内容）→ wrap 2；display 行 2（r2 块）→ wrap 2
        assert_eq!(s.block_to_wrap_row(3), 2);
        assert_eq!(s.block_to_wrap_row(2), 2);
        assert_eq!(s.block_to_wrap_row(10), 7);
    }

    /// DM-303：provider revision 变化 → 同一 buffer 旧结果判过期。
    #[test]
    fn provider_revision_invalidates_results() {
        let a = snap(vec![block(1, 0, 1, 0)], 2);
        let b = BlockSnapshot::new(1, 5, vec![block(1, 0, 1, 0)], 2);
        assert_ne!(a.revision(), b.revision());
        assert!(a.is_current(1, a.revision()));
        assert!(!b.is_current(1, a.revision()));
    }

    /// DM-315：局部块变更用 `replace_blocks` 只重建相交行区间；未触碰的后缀行
    /// subtree 全程 `Arc` 共享（`shared_trailing_leaves` ≥ 1），且不整树重建。
    #[test]
    fn local_block_update_shares_trailing_leaves() {
        // 50 个 wrap 行无块（> FANOUT，树有分支）。
        let no_blocks = snap(vec![], 50);
        assert_eq!(no_blocks.total_height_rows(), 50);
        // 在 wrap 行 3 插入一个高 1 块 → 只有行 3 及之后路径受影响，末尾后缀共享。
        let changed = no_blocks.replace_blocks(1, 0, vec![block(1, 3, 1, 0)]);
        assert_eq!(changed.total_height_rows(), 51, "50 + 1 块高");
        // 行 3 内容行 = 3 + 1 = 4；行 49 → 50。
        assert_eq!(
            changed
                .map_input_by(WrapPoint { row: 3, column: 0 })
                .right
                .row,
            4
        );
        assert_eq!(
            changed
                .map_input_by(WrapPoint { row: 49, column: 0 })
                .right
                .row,
            50
        );
        // 末尾未触碰的连续叶子共享（持久 tree）。
        assert!(
            changed.heights.shared_trailing_leaves(&no_blocks.heights) >= 1,
            "行 3 局部更新后末尾叶节点应共享"
        );
    }

    #[test]
    fn sync_rows_shifts_blocks_after_inserted_wrap_rows() {
        let old = snap(vec![block(7, 40, 1, 0)], 80);
        let next = old.sync_rows(2, 10..11, 3);
        assert_eq!(next.base_rows(), 82);
        // dirty 行被 provider 刷新为空，原 block 随后缀整体后移两行。
        assert_eq!(next.block_at(BlockPoint::new(42, 0)), Some(BlockId(7)));
    }
}
