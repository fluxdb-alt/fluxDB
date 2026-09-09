//! 用户折叠的稳定状态（DM-106）。
//!
//! 折叠在旧实现里以「行号集合」保存（用户折叠哪些候选行），编辑发生在折叠点
//! 之前时行号会错位，可能匹配到另一个语句。本模块把折叠状态迁移为基于
//! [`AnchorRange`]（DM-105 稳定锚点区间）的唯一标识折叠，编辑后显式重定位，
//! 不再依赖行号猜测。
//!
//! `DisplayMap` 在 Phase 2 前的 facade 仍消费行区间 `Fold`（DM-108）；因此
//! `FoldSet` 在需要时以当前快照把稳定区间解析回行区间 `Fold`，保证 UI 迁移期
//! 不感知内部层变化。

use crate::display_map::Fold;
use crate::model::{AnchorRange, Bias, Range};
use crate::sum_tree::{IntervalSummary, SumTree, SumTreeItem};

/// 折叠的唯一稳定 id。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FoldId(pub u64);

/// 一个稳定折叠：内容绑定到 `AnchorRange`，编辑后重定位（DM-105）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldEntry {
    pub id: FoldId,
    /// 折叠的字节区间（半开，两端稳定锚点）。start 用 `Bias::Left`、end 用
    /// `Bias::Right`：在边界插入时折叠保持包含原内容。
    pub range: AnchorRange,
}

/// 折叠条目在 [`FoldSet`] 内部 SumTree 中的叶子项。
///
/// 摘要携带区间包络 `[start, end)`（字节偏移）与条目数，使 add/remove/edit 只替换
/// 相交叶子区间，未触碰的子树节点全部 `Arc` 共享（持久，DM-202）。
#[derive(Clone, Debug)]
struct FoldSumItem {
    entry: FoldEntry,
}

/// [`FoldSumItem`] 的子树叶摘要：`start`/`end` 为该叶子（或子树）所有条目区间的
/// 最小起点 / 最大终点包络。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FoldSummary {
    start: usize,
    end: usize,
    count: usize,
}

impl crate::sum_tree::Summary for FoldSummary {
    fn add(&self, other: &Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            count: self.count + other.count,
        }
    }
}

impl IntervalSummary for FoldSummary {
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
}

impl SumTreeItem for FoldSumItem {
    type Summary = FoldSummary;

    fn summary(&self) -> Self::Summary {
        Self::Summary {
            start: self.entry.range.start.offset,
            end: self.entry.range.end.offset,
            count: 1,
        }
    }
}

/// 稳定折叠集合：按起始偏移排序、互不重叠。
///
/// 构造时以当前快照的字节区间建立（语言 `fold_ranges` / 旧行区间均可迁移），
/// 之后每次编辑调用 [`FoldSet::relocate`] 重定位，用户折叠即绑定到内容而非行号。
///
/// 条目存储于按起始偏移排序的 [`SumTree`]：add/remove/edit 只替换相交的叶子区间
/// （persistent `replace_leaves`，未触碰子树 `Arc` 共享），而不是整体重建 —— 即
/// DM-202「fold add/remove/edit 只替换相交 transform path」的稳定状态侧实现。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldSet {
    next_id: u64,
    tree: SumTree<FoldSumItem>,
}

impl FoldSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.total().count == 0
    }

    pub fn len(&self) -> usize {
        self.tree.total().count
    }

    /// 迁移入口：从「当前快照字节偏移」的区间序列建立稳定折叠状态（DM-106）。
    ///
    /// 过滤退化区间（空 / 单字节），并去除包含关系与重叠，按起点排序。
    /// 来源可为语言 `fold_ranges` 或把旧行区间 `Fold` 解析到字节后传入。
    pub fn from_byte_ranges(ranges: impl IntoIterator<Item = Range>) -> Self {
        let mut set = FoldSet::new();
        let mut ranges: Vec<Range> = ranges.into_iter().filter(|r| !r.is_empty()).collect();
        ranges.sort_unstable_by_key(|r| (r.start, r.end));
        for r in ranges {
            set.insert_normalized(r);
        }
        set
    }

    /// 迁移入口：从旧行区间折叠（`Vec<Fold>`）建立稳定状态。
    ///
    /// `row_to_offset(row) -> 行首字节` 由调用方提供（基于当前快照）。
    /// 折叠终点取 `end_row` 的下一行行首，使折叠覆盖整段内容行；无法解析时跳过。
    pub fn from_row_folds(folds: &[Fold], row_to_offset: impl Fn(usize) -> Option<usize>) -> Self {
        let mut set = FoldSet::new();
        let mut entries: Vec<(usize, usize)> = Vec::new(); // (start_offset, end_offset)
        for f in folds {
            let (Some(start), Some(end)) =
                (row_to_offset(f.start_row), row_to_offset(f.end_row + 1))
            else {
                continue;
            };
            if end > start {
                entries.push((start, end));
            }
        }
        entries.sort_unstable();
        for (start, end) in entries {
            set.insert_normalized(Range::new(start, end.max(start + 1)));
        }
        set
    }

    /// 插入一个区间，规范化内部重叠/包含（含相触合并），保持排序，不重复 id 分配。
    /// 新条目总会获得一个新 [`FoldId`]。
    ///
    /// DM-202：只替换「与新区间相交（含相触）」的叶子区间 ——
    /// [`SumTree::intersecting_leaf_span`] 定位受影响的下标范围，被合并后的条目经
    /// [`SumTree::replace_leaves`] 在该区间原地替换；未触碰子树节点 `Arc` 共享
    /// （持久 path），不整体重建。无相交时是纯插入：以 `lower_bound_start` 定插入点。
    fn insert_normalized(&mut self, range: Range) {
        if range.is_empty() {
            return;
        }
        let id = FoldId(self.next_id);
        self.next_id += 1;
        let entry = FoldEntry {
            id,
            range: to_anchor_range(range),
        };
        let item = FoldSumItem { entry };
        match self.tree.intersecting_leaf_span((range.start, range.end)) {
            Some((first, last_excl)) => {
                // 与相交条目合并：取所有相交叶子的真实区间包络。
                let mut merged = range;
                for idx in first..last_excl {
                    if let Some(it) = self.tree.get(idx) {
                        let e = &it.entry;
                        merged = Range::new(
                            merged.start.min(e.range.start.offset),
                            merged.end.max(e.range.end.offset),
                        );
                    }
                }
                self.tree = self.tree.replace_leaves(
                    first,
                    last_excl,
                    &[FoldSumItem {
                        entry: FoldEntry {
                            id,
                            range: to_anchor_range(merged),
                        },
                    }],
                );
            }
            None => {
                // 纯插入：找第一个 start >= range.start 位置，原地插一个叶子。
                let idx = self.tree.lower_bound_start(range.start);
                self.tree = self.tree.replace_leaves(idx, idx, &[item]);
            }
        }
    }

    /// 切换一个字节区间对应的折叠：存在则移除，不存在则加入。
    /// 返回是否有变化。
    pub fn toggle_range(&mut self, range: Range) -> bool {
        if self.contains(range) {
            self.remove(range)
        } else {
            let before = self.len();
            self.insert_normalized(range);
            self.len() != before
        }
    }

    /// 该字节区间当前是否处于折叠状态。
    pub fn contains(&self, range: Range) -> bool {
        let idx = self.tree.lower_bound_start(range.start);
        idx < self.len()
            && self.tree.get(idx).map_or(false, |item| {
                item.entry.range.start.offset == range.start
                    && item.entry.range.end.offset == range.end
            })
    }

    /// 无条件移除一个字节区间对应的折叠（若存在）。用于「展开某折叠」类语义：
    /// 目标区间不在集合中也视为成功（幂等），不新增。返回是否发生了移除。
    ///
    /// DM-202：精确定位该条目叶子（起始偏移二分 + 校验首叶），只替换单个叶子区间。
    pub fn remove(&mut self, range: Range) -> bool {
        let idx = self.tree.lower_bound_start(range.start);
        if idx >= self.len() {
            return false;
        }
        let exact = self.tree.get(idx).map_or(false, |item| {
            item.entry.range.start.offset == range.start && item.entry.range.end.offset == range.end
        });
        if !exact {
            return false;
        }
        self.tree = self.tree.replace_leaves(idx, idx + 1, &[]);
        true
    }

    /// 按一次编辑重定位所有折叠（DM-105）。`old_range` 为被替换区间，`new_len`
    /// 为新文本长度。折叠绑定内容，编辑发生在折叠之前时不会漂移。
    ///
    /// DM-203：重定位会移动各折叠的起止，可能使原本分开的折叠变成**重叠/相邻**，
    /// 故重定位后重新规范化——丢弃退化为空的折叠、合并重叠/相邻条目并保序。合并
    /// 时保留该组最早条目的 [`FoldId`]（折叠身份不漂移；`relocate` 不分配新 id）。
    pub fn relocate(&mut self, old_range: Range, new_len: usize) {
        let items: Vec<FoldEntry> = self
            .tree
            .to_vec()
            .into_iter()
            .map(|item| item.entry)
            .collect();
        // 重定位后仍按起点升序（sum_tree 保序重定位每个条目），丢弃退化，再归并
        // 相邻/重叠。首条 id 保留、后续相邻条目的区间并入第一条。
        let mut merged: Vec<FoldSumItem> = Vec::new();
        for mut e in items {
            e.range = e.range.relocate(old_range, new_len);
            if e.range.end.offset <= e.range.start.offset {
                continue; // 塌缩为空，丢弃
            }
            if let Some(last) = merged.last_mut() {
                // 相触（end 相邻）或重叠 → 并入前一条（保持首条 id）。
                if e.range.start.offset <= last.entry.range.end.offset {
                    // 终点锚点取两者偏移较大者（原地复用已重定位的锚点，保持 bias）。
                    if e.range.end.offset > last.entry.range.end.offset {
                        last.entry.range.end = e.range.end;
                    }
                    continue;
                }
            }
            merged.push(FoldSumItem { entry: e });
        }
        // 重定位可能改变条目数（合并/塌缩），全量重建一次（relocate 罕见）。
        self.tree = SumTree::from_items(&merged);
    }

    /// 按当前快照把稳定区间解析为行区间 `Fold`，供现有 `DisplayMap` facade 消费。
    ///
    /// `offset_to_row(offset) -> 行号` 由调用方提供（如
    /// `|o| snapshot.offset_to_point(o).row`）；区间终点回退到所在行，退化为
    /// 同一行的区间被丢弃。
    pub fn resolve_rows(&self, offset_to_row: impl Fn(usize) -> usize) -> Vec<Fold> {
        self.entries()
            .into_iter()
            .filter_map(|e| {
                let start_row = offset_to_row(e.range.start.offset);
                // 终点用闭区间语义：至少覆盖到其所在行。
                let end_offset = e.range.end.offset.saturating_sub(1);
                let end_row = offset_to_row(end_offset);
                if end_row > start_row {
                    Some(Fold { start_row, end_row })
                } else {
                    None
                }
            })
            .collect()
    }

    /// 遍历稳定折叠（供 FoldMap/diagnostic 直接消费条目）。按起始偏移升序。
    pub fn iter(&self) -> impl Iterator<Item = FoldEntry> {
        self.entries().into_iter()
    }

    /// 当前稳定折叠条目（按起始偏移升序）。拷贝自 SumTree（O(n)，用于消费端）。
    pub fn entries(&self) -> Vec<FoldEntry> {
        self.tree
            .to_vec()
            .into_iter()
            .map(|item| item.entry)
            .collect()
    }
}

fn to_anchor_range(range: Range) -> AnchorRange {
    AnchorRange::new(
        crate::model::Anchor::new(range.start, Bias::Left),
        crate::model::Anchor::new(range.end, Bias::Right),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模拟一段每行 6 字节的文档：行首分别在 0,6,12,...。行长固定便于断言。
    /// 用于 `resolve_rows` 的 offset→row 映射。
    fn offset_to_row_6(offset: usize) -> usize {
        offset / 6
    }
    /// 用于 `from_row_folds` 的 row→offset 映射。
    fn row_to_offset_6(row: usize) -> Option<usize> {
        Some(row * 6)
    }

    /// DM-106：从字节区间迁移建立稳定 fold 状态，自动排序并丢弃退化区间。
    #[test]
    fn migrates_from_byte_ranges_sorted_and_dedupes() {
        let set = FoldSet::from_byte_ranges([
            Range::new(24, 42),
            Range::new(0, 6),
            Range::new(6, 6), // 空区间，丢弃
        ]);
        assert_eq!(set.len(), 2);
        let starts: Vec<usize> = set.iter().map(|e| e.range.start.offset).collect();
        assert_eq!(starts, vec![0, 24]);
        // 每个条目获得唯一 id。
        let ids: Vec<FoldId> = set.iter().map(|e| e.id).collect();
        assert_ne!(ids[0], ids[1]);
    }

    /// DM-106：移除一个区间后再次加入会分配新 id（id 不保证复用）。
    #[test]
    fn toggle_reassigns_new_id() {
        let mut set = FoldSet::new();
        set.toggle_range(Range::new(6, 12));
        let first_id = set.iter().next().unwrap().id;
        set.toggle_range(Range::new(6, 12)); // 移除
        assert!(set.is_empty());
        set.toggle_range(Range::new(6, 12)); // 再加
        let second_id = set.iter().next().unwrap().id;
        assert_ne!(first_id, second_id);
        // contains 反映当前状态。
        assert!(set.contains(Range::new(6, 12)));
        assert!(!set.contains(Range::new(12, 18)));
    }

    /// DM-106：迁移时不变量——重叠/包含区间合并为不重叠稳定条目。
    #[test]
    fn overlapping_ranges_merge_into_disjoint_entries() {
        let mut set = FoldSet::new();
        set.insert_normalized(Range::new(6, 18));
        set.insert_normalized(Range::new(12, 24));
        assert_eq!(set.len(), 1);
        let e = set.iter().next().unwrap();
        assert_eq!((e.range.start.offset, e.range.end.offset), (6, 24));
    }

    /// DM-106/DM-107：编辑发生在折叠之前，折叠绑定内容而非行号——
    /// 顶部插入两行后，同一折叠仍覆盖同一段内容，行号整体后移。
    #[test]
    fn fold_survives_insert_before_it() {
        // 原文档 10 行（行长 6）。折叠 [18,42)（第 3..6 行）。
        let mut set = FoldSet::from_byte_ranges([Range::new(18, 42)]);
        // 顶部插入 2 行（12 字节）：old_range=0..0, new_len=12。
        set.relocate(Range::new(0, 0), 12);
        let rows = set.resolve_rows(offset_to_row_6);
        assert_eq!(rows.len(), 1);
        // 内容下移 2 行：原第 3 行 -> 第 5 行，原第 6 行 -> 第 8 行。
        assert_eq!((rows[0].start_row, rows[0].end_row), (5, 8));
    }

    /// DM-106/DM-107：删除折叠之前的行后，折叠仍绑定内容、行号前移。
    #[test]
    fn fold_survives_delete_before_it() {
        let mut set = FoldSet::from_byte_ranges([Range::new(18, 42)]);
        // 删除前两行（12 字节）：old_range=0..12, new_len=0。
        set.relocate(Range::new(0, 12), 0);
        let rows = set.resolve_rows(offset_to_row_6);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].start_row, rows[0].end_row), (1, 4));
    }

    /// DM-107：编辑发生在折叠 *内部*，折叠起点/终点各自吸附，仍覆盖折叠内容。
    #[test]
    fn fold_survives_edit_inside_it() {
        let mut set = FoldSet::from_byte_ranges([Range::new(18, 42)]);
        // 内部替换 [24,30) -> 空（删除 6 字节）。
        set.relocate(Range::new(24, 30), 0);
        // 起点 18 不变，终点 42-6=36。
        let e = set.iter().next().unwrap();
        assert_eq!((e.range.start.offset, e.range.end.offset), (18, 36));
        assert!(!set.is_empty());
    }

    /// DM-107：整段被删除导致折叠塌缩为空时，丢弃退化折叠。
    #[test]
    fn collapsed_fold_is_dropped() {
        let mut set = FoldSet::from_byte_ranges([Range::new(6, 12)]);
        // 删除整段 + 周围：old 覆盖 [6,18)。
        set.relocate(Range::new(6, 18), 0);
        assert!(set.is_empty());
    }

    /// DM-106：从旧行区间 `Vec<Fold>` 迁移为稳定状态，再解析回行区间应一致。
    #[test]
    fn migrates_from_row_folds_round_trips_rows() {
        let folds = vec![Fold {
            start_row: 2,
            end_row: 4,
        }];
        let set = FoldSet::from_row_folds(&folds, row_to_offset_6);
        assert_eq!(set.len(), 1);
        let rows = set.resolve_rows(offset_to_row_6);
        assert_eq!((rows[0].start_row, rows[0].end_row), (2, 4));
    }

    /// DM-202：增量 add/edit 只替换「相交叶子区间」——首段合并只重建被命中路径上的
    /// 叶子节点，其后未触碰的叶子节点（每节点 ≤ FANOUT 条折叠）整体 `Arc` 共享
    /// （持久 path），证明未触碰的后缀子树未重建。
    ///
    /// `shared_trailing_leaves` 统计末尾连续共享的**叶子节点**数（每片叶子含多条折叠）；
    /// 100 条折叠按 FANOUT=32 打包成 4 个叶子节点，首节点合并只重建节点 0，
    /// 剩余 3 个节点（96 条折叠）全程共享。
    #[test]
    fn incremental_add_shares_untouched_suffix() {
        let mut set = FoldSet::new();
        // 100 个互不相触、升序的折叠（间距 12 > 单折叠宽 6 → 互不相触）。
        for i in 0..100 {
            set.insert_normalized(Range::new(i * 12, i * 12 + 6));
        }
        assert_eq!(set.len(), 100);
        let before = set.tree.clone();
        // 首个叶子内的合并 [6,12)：与 [0,6)、[12,18) 相触 → [0,18)。只重建首叶子
        // 节点，其余叶子节点（兄弟子树）共享。
        set.insert_normalized(Range::new(6, 12));
        assert_eq!(set.len(), 99);
        let shared = set.tree.shared_trailing_leaves(&before);
        assert!(
            shared >= 2,
            "共享叶节点数 = {shared}，应 ≥2（未触碰后缀子树持久共享）"
        );
        // 归并结果正确：首个折叠合并为 [0,18)。
        let mut it = set.iter();
        let (fs, fe) = {
            let e = it.next().unwrap();
            (e.range.start.offset, e.range.end.offset)
        };
        assert_eq!((fs, fe), (0, 18));
    }

    /// DM-202：增量 add/edit 归并语义与旧 Vec 实现一致——相触区间合并为一个条目，
    /// 顺序保持升序，未受影响条目保留。
    #[test]
    fn incremental_add_merges_adjacent_and_keeps_sorted() {
        let mut set = FoldSet::new();
        set.insert_normalized(Range::new(6, 12));
        set.insert_normalized(Range::new(18, 24));
        assert_eq!(set.len(), 2);
        // 相触 [12,18)：与左右均相触 → 合并 [6,24] 成单个条目。
        set.insert_normalized(Range::new(12, 18));
        assert_eq!(set.len(), 1);
        let e = set.iter().next().unwrap();
        assert_eq!((e.range.start.offset, e.range.end.offset), (6, 24));
        // 远处插入保持升序。
        set.insert_normalized(Range::new(42, 48));
        let starts: Vec<usize> = set.iter().map(|e| e.range.start.offset).collect();
        assert_eq!(starts, vec![6, 42]);
    }

    /// DM-202：remove 精确移除单条折叠（只替换单个叶子），不影响其余条目。
    #[test]
    fn remove_targets_exact_entry_only() {
        let mut set =
            FoldSet::from_byte_ranges([Range::new(6, 12), Range::new(18, 24), Range::new(30, 36)]);
        assert!(set.remove(Range::new(18, 24)));
        assert!(!set.contains(Range::new(18, 24)));
        assert_eq!(set.len(), 2);
        // 其余条目保留。
        assert!(set.contains(Range::new(6, 12)));
        assert!(set.contains(Range::new(30, 36)));
        // 幂等：移除不存在区间视为成功但无变化（返回 false）。
        assert!(!set.remove(Range::new(99, 102)));
    }

    /// DM-203：relocate 重定位后重新规范化——删除两折叠之间间隙使它们滑拢成
    /// 相邻（相触）区间，合并为一条，且保留最早条目的折叠 id（身份不漂移）。
    #[test]
    fn relocate_renormalizes_adjacent_merged() {
        let mut set = FoldSet::from_byte_ranges([Range::new(0, 6), Range::new(12, 18)]);
        assert_eq!(set.len(), 2);
        let first_id = set.iter().next().unwrap().id;
        // 删除两折叠之间的间隙 [6,12)：后一条整体前移 6 字节 → [6,12)，与 [0,6)
        // 首尾相触（6 == 6）。
        set.relocate(Range::new(6, 12), 0);
        assert_eq!(set.len(), 1, "相邻折叠重定位后应合并为一条");
        let e = set.iter().next().unwrap();
        assert_eq!((e.range.start.offset, e.range.end.offset), (0, 12));
        assert_eq!(e.id, first_id, "合并保留最早条目的 id");
    }

    /// DM-203：relocate 重定位后重新规范化——被整段删除的折叠仍被丢弃（既有
    /// 语义），其余不相邻折叠保持不合并。
    #[test]
    fn relocate_keeps_non_adjacent_separate() {
        let mut set = FoldSet::from_byte_ranges([Range::new(0, 6), Range::new(18, 24)]);
        // 删除中间的 [6,18)（含一个正在被移除的区间）：丢失 [6,18) 全部 →
        // 后一条 [18,24) 前移 12 → [6,12)；[0,6) 与 [6,12) 相触 → 合并 [0,12)。
        set.relocate(Range::new(6, 18), 0);
        assert_eq!(set.len(), 1);
        let e = set.iter().next().unwrap();
        assert_eq!((e.range.start.offset, e.range.end.offset), (0, 12));
        // 无间隙且不相邻的折叠不合并（各自保序）。
        let mut set2 = FoldSet::from_byte_ranges([Range::new(0, 6), Range::new(18, 24)]);
        set2.relocate(Range::new(30, 36), 0); // 删除两者之外的区间，无影响
        assert_eq!(set2.len(), 2);
    }

    /// DM-202：toggle（add/remove 的 UI 入口）在 SumTree 存储下行为不变。
    #[test]
    fn toggle_round_trips_on_sumtree() {
        let mut set = FoldSet::new();
        assert!(set.toggle_range(Range::new(6, 12))); // add
        assert!(set.contains(Range::new(6, 12)));
        assert!(set.toggle_range(Range::new(6, 12))); // 精确匹配 → remove
        assert!(!set.contains(Range::new(6, 12)));
        assert!(set.is_empty());
        assert!(set.toggle_range(Range::new(6, 12))); // 再加
        assert!(set.toggle_range(Range::new(18, 24))); // add 第二
        assert_eq!(set.len(), 2);
        assert!(set.toggle_range(Range::new(6, 12))); // remove 第一
        assert_eq!(set.len(), 1);
        assert!(!set.contains(Range::new(6, 12)));
        assert!(set.contains(Range::new(18, 24)));
    }
}
