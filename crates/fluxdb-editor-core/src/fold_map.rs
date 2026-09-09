//! FoldMap：把 inlay→ 行列映射为折叠后的展示行列（DM-200，Phase 2；DM-304 上游
//! 调整为 InlayPoint）。
//!
//! 显示管线中第二个真正的 [`LayerSnapshot`]：显示顺序为 Buffer → Inlay → Fold →
//! Tab → Wrap（DM-304 起在 Fold 前插入 Inlay 层）。因此本层输入 [`InlayPoint`]
//! （已含 inlay 加宽的展示列，DM-304 后），输出 [`FoldPoint`]（折叠替换后，含
//! placeholder）。折叠把内部行折叠进折叠起始行，占位符为展示行 `disp(start)` 列
//! 0 处的单个展示列槽（见 [`FOLD_PLACEHOLDER_WIDTH`]）。跨折叠边界返回
//! [`Biased`] 左右候选，调用方按 [`Bias`] 取其一。
//!
//! 本层是独立自包含展示层，仅依赖 `coordinates` / `model` / `display_map::Fold` /
//! `fold::FoldSet` / `layer::LayerSnapshot`，不依赖任何 UI 类型。折叠语义与
//! `DisplayMap` 保持一致（同样的排序、去空、合并交叉/相邻规范化；折叠起始行仍
//! 可视，仅内部行隐藏入内）。非折叠行对 InlayPoint 列恒等透传（inlay 已先加宽
//! 展示列，Fold 不再改动其列）。DisplayMap 门面保持精简组合（DM-228 起）。

use crate::coordinates::{Biased, FoldPoint, InlayPoint};
use crate::display_map::Fold;
use crate::fold::FoldSet;
use crate::layer::{LayerPatch, LayerSnapshot};
use crate::sum_tree::{IntervalSummary, SumTree, SumTreeItem, Summary};

/// 折叠占位符宽度（展示列）。
///
/// 整行折叠没有任何 buffer 列语义（内部行有自己的多列内容且被整体隐藏），因此把
/// 占位符建模为折叠起始行 `disp(start)` 列 0 处的一个 **单列槽**：left 指向占位符
/// 起点（列 0），right 指向占位符终点（列 1）。这让折叠边界处 [`Biased`] 有真实
/// 左右可区分（`is_identical()` 为假，DM-203 测试依赖）。
///
/// `ponytail:` 列 1 是合成占位符终点哨兵，不是真实 buffer 列。等字节级折叠
/// （DM-203/204）需要精确折叠列时再改为导出列；当前整行折叠用固定常量即可。
const FOLD_PLACEHOLDER_WIDTH: usize = 1;

/// 折叠展示层的不可变快照。
#[derive(Clone)]
pub struct FoldSnapshot {
    /// 绑定的输入（buffer 快照）版本号，构建时记录。
    input_version: u64,
    /// 本层自身 revision：由 input_version + fold 状态派生（FNV-1a）。
    revision: u64,
    /// 规范化折叠：排序、去空、合并交叉/相邻。`start_row` 为折叠起始（可视），
    /// `end_row` 为折叠结束（含）。
    folds: std::sync::Arc<Vec<Fold>>,
    tree: SumTree<FoldRowItem>,
}

#[derive(Clone, Debug)]
struct FoldRowItem {
    fold: Fold,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FoldRowSummary {
    start: usize,
    end: usize,
    hidden: usize,
}

impl Summary for FoldRowSummary {
    fn add(&self, other: &Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            hidden: self.hidden.saturating_add(other.hidden),
        }
    }
}

impl IntervalSummary for FoldRowSummary {
    fn start(&self) -> usize {
        self.start
    }

    fn end(&self) -> usize {
        self.end
    }
}

impl SumTreeItem for FoldRowItem {
    type Summary = FoldRowSummary;

    fn summary(&self) -> Self::Summary {
        FoldRowSummary {
            start: self.fold.start_row,
            end: self.fold.end_row,
            hidden: self.fold.end_row.saturating_sub(self.fold.start_row),
        }
    }
}

impl FoldSnapshot {
    /// 行区间构造（DisplayMap 现有的折叠形态）。内部规范化。`input_version` 取
    /// 构建时 `BufferSnapshot::version()`。
    pub fn new(input_version: u64, folds: Vec<Fold>) -> Self {
        let folds = normalize_folds(&folds);
        let revision = compute_revision(input_version, &folds);
        Self {
            input_version,
            revision,
            tree: SumTree::from_items(&row_items(&folds)),
            folds: std::sync::Arc::new(folds),
        }
    }

    /// 从稳定 [`FoldSet`] 解析构造。复用 [`FoldSet::resolve_rows`] 把稳定区间解析
    /// 为行区间 `Fold`，再走 `new` 规范化。`offset_to_row` 由调用方提供（如
    /// `|o| snapshot.offset_to_point(o).row`）。
    pub fn from_fold_set(
        input_version: u64,
        set: &FoldSet,
        offset_to_row: impl Fn(usize) -> usize,
    ) -> Self {
        let folds = set.resolve_rows(offset_to_row);
        Self::new(input_version, folds)
    }

    /// 当前规范化折叠列表（只读）。
    pub fn folds(&self) -> &[Fold] {
        self.folds.as_slice()
    }

    pub fn same_folds(&self, folds: &[Fold]) -> bool {
        normalize_folds(folds).as_slice() == self.folds.as_slice()
    }

    pub fn sync_input(&self, input_version: u64) -> Self {
        Self {
            input_version,
            revision: compute_revision(input_version, self.folds.as_slice()),
            folds: self.folds.clone(),
            tree: self.tree.clone(),
        }
    }

    /// Replace the fold set and return the affected Fold-coordinate suffix.
    ///
    /// Fold rows are prefix-summed: changing one fold can move every following
    /// display row, so the smallest correct patch starts at the first changed
    /// buffer row and ends at the old/new visible-row counts.
    pub fn sync_folds_with_patch(
        &self,
        input_version: u64,
        folds: Vec<Fold>,
        old_buffer_rows: usize,
        new_buffer_rows: usize,
    ) -> (Self, LayerPatch) {
        let normalized = normalize_folds(&folds);
        if self.folds.as_slice() == normalized.as_slice() && old_buffer_rows == new_buffer_rows {
            let next = Self {
                input_version,
                revision: compute_revision(input_version, &normalized),
                folds: std::sync::Arc::new(normalized),
                tree: self.tree.clone(),
            };
            return (next, LayerPatch::default());
        }
        let dirty_buffer_row = first_changed_fold_row(self.folds.as_slice(), &normalized);
        let first_changed = first_changed_fold_index(self.folds.as_slice(), &normalized);
        let shared_suffix = common_fold_suffix(self.folds.as_slice(), &normalized);
        let old_end = self.folds.len().saturating_sub(shared_suffix);
        let new_end = normalized.len().saturating_sub(shared_suffix);
        let tree = self.tree.replace_leaves(
            first_changed,
            old_end,
            &row_items(&normalized[first_changed..new_end]),
        );
        let next = Self {
            input_version,
            revision: compute_revision(input_version, &normalized),
            folds: std::sync::Arc::new(normalized),
            tree,
        };
        let old_start = self.display_row(dirty_buffer_row);
        let new_start = next.display_row(dirty_buffer_row);
        let old_end = visible_rows(old_buffer_rows, self.folds.as_slice());
        let new_end = visible_rows(new_buffer_rows, next.folds.as_slice());
        (
            next,
            LayerPatch::single(
                old_start..old_end.max(old_start),
                new_start..new_end.max(new_start),
            ),
        )
    }

    // ===== DM-204：四个消费端（placeholder / selection / caret / hit test）
    // ===== 共享的同一套映射 API。所有消费端只走这几个原语 + `map_input_by` /
    // ===== `map_output_by`，不再各自 `.iter().find` 重推导折叠数学。

    /// buffer 行 → 所在折叠（含起始行边界 `start_row <= row <= end_row`，折叠起始行
    /// 自身可视仍属该折叠）。placeholder / selection / caret / hit test 的 **buffer 侧**
    /// 折叠归属查询都用这一个原语（`map_input_by` 也复用它，见下）。
    fn fold_containing(&self, buffer_row: usize) -> Option<&Fold> {
        let index = self.tree.lower_bound_start(buffer_row.saturating_add(1));
        index
            .checked_sub(1)
            .and_then(|index| self.folds.get(index))
            .filter(|fold| fold.start_row <= buffer_row && buffer_row <= fold.end_row)
    }

    /// 该 buffer 行之前的隐藏行数。
    fn hidden_rows_before(&self, row: usize) -> usize {
        let index = self.tree.lower_bound_start(row);
        self.tree.summary_before_leaf(index).hidden
    }

    /// buffer 行 → 展示行号（该行与其前隐藏行的差）。
    ///
    /// 供门面侧 `row_layout`/`first_visual_row` 反向推导（DM-228 组合）。
    pub fn display_row(&self, buffer_row: usize) -> usize {
        buffer_row.saturating_sub(self.hidden_rows_before(buffer_row))
    }

    /// 展示行 → 所在折叠（该展示行是某折叠的占位符行）。折叠互不重叠且非相邻
    /// （已合并），故 `disp(start)` 在所有折叠上互不相同，可唯一确定。hit test：
    /// 「该展示行是折叠占位符行」即本查询非空。
    pub fn fold_at_display_row(&self, disp: usize) -> Option<&Fold> {
        self.folds
            .iter()
            .find(|fold| self.display_row(fold.start_row) == disp)
    }

    /// hit test：该展示行是否落在某折叠的占位符行上（要画折叠标记 / 拦截点击）。
    /// 与 [`FoldSnapshot::fold_at_display_row`] 完全相同，是 placeholder 与 hit test
    /// 共享的同一归属查询。
    pub fn is_folded_display_row(&self, disp: usize) -> bool {
        self.fold_at_display_row(disp).is_some()
    }

    /// 折叠占位符所在展示行（= 折叠起始行的展示行）。placeholder 消费端用它确定
    /// 折叠标记应绘制在哪一行。
    pub fn placeholder_display_row(&self, fold: &Fold) -> usize {
        self.display_row(fold.start_row)
    }

    /// selection 消费端：把 buffer 行区间（起止点）映射到展示区间。
    ///
    /// 起止两个端点都用同一套 `map_input_by` 逐个映射（selection 的可视化边界与
    /// placeholder/caret 用的是同一映射，保证视觉上选中区域与折叠标记/光标对齐）。
    /// 折叠把内部行折叠进占位符；若选区端点之一落在折叠内部，其展示位置即占位符
    /// 位置。返回 `(起始展示点, 结束展示点)`，排序关系由调用方（`Bias`）解释。
    ///
    /// 注意这是**端点可视位置**映射；选区实际覆盖的 buffer 内容仍由调用方持原始
    /// buffer 区间，FoldMap 只折叠端点的展示位置（折叠不改写选区语义）。
    pub fn map_input_range(
        &self,
        start: InlayPoint,
        end: InlayPoint,
    ) -> (Biased<FoldPoint>, Biased<FoldPoint>) {
        (self.map_input_by(start), self.map_input_by(end))
    }

    /// 展示行 → buffer 行（反向）。仅用于非占位符行；占位符行在
    /// `map_output_by` 中已先短路处理。
    ///
    /// 折叠互不重叠且已排序/合并，可见 buffer 行即「非折叠内部行」，展示行是可见
    /// 行的序数。因此按序扫描折叠：每段可见区间为 `[buffer ..= fold.start_row]`
    /// （折叠起始行自身可视），段内可见行逐一与目标展示行比对；越过折叠后跳过其
    /// 内部隐藏行。无需 buffer 总行数即可取值。caret / selection 的反向映射消费端
    /// 复用它。
    ///
    /// `ponytail:` 线性扫描（O(折叠数)）定位区间。折叠数通常远小于行数，可接受；
    /// 若未来折叠规模变大，可在构造时预置 SumTree 前缀和走二分。
    pub fn buffer_row_for_display(&self, disp: usize) -> usize {
        let mut buffer = 0usize; // 本段可见区起始 buffer 行
        let mut seen = 0usize; // 已计入的可见行数（展示行序数）
        for fold in self.folds.iter() {
            // 该折叠前的一段可见区 [buffer ..= fold.start_row]。
            let visible_in_segment = fold.start_row.saturating_sub(buffer) + 1;
            if seen + visible_in_segment > disp {
                return buffer + (disp - seen);
            }
            seen += visible_in_segment;
            // 越过该折叠，跳过其内部隐藏行（start+1..=end），下一段从 end+1 起。
            buffer = fold.end_row.saturating_add(1);
        }
        buffer + (disp - seen)
    }
}

impl LayerSnapshot for FoldSnapshot {
    type Input = InlayPoint;
    type Output = FoldPoint;

    fn input_version(&self) -> u64 {
        self.input_version
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn map_input_by(&self, input: InlayPoint) -> Biased<FoldPoint> {
        let InlayPoint { row: r, column: c } = input;
        // 折叠归属与 placeholder/hit test 共享同一 `fold_containing` 原语（DM-204），
        // 不在此单独 `.iter().find`。
        if let Some(fold) = self.fold_containing(r) {
            let disp = self.placeholder_display_row(fold);
            if r == fold.start_row && c > 0 {
                return Biased::from(FoldPoint {
                    row: disp,
                    column: c,
                });
            }
            // 内部行（start < r <= end）或折叠行边界（列 0）：映射到占位符。
            return Biased {
                left: FoldPoint {
                    row: disp,
                    column: 0,
                },
                right: FoldPoint {
                    row: disp,
                    column: FOLD_PLACEHOLDER_WIDTH,
                },
            };
        }
        // 折叠外部：恒等映射。
        Biased::from(FoldPoint {
            row: self.display_row(r),
            column: c,
        })
    }

    fn map_output_by(&self, output: FoldPoint) -> Biased<InlayPoint> {
        let FoldPoint { row: o, column: c } = output;
        // 占位符行：折叠边界（列 0，占位符单列槽）反向到折叠边界，left 在折叠起始
        // 行、right 到折叠后首行（折叠非相邻/已合并，`end+1` 恒为可见间隙行）；列
        // ≥ 占位符宽度的输出列属于折叠起始行的可见内容，按列恒等反向（与
        // `map_input_by` 起始行列 >0 恒等映射互逆，保证 round-trip，DM-201）。
        if let Some(fold) = self.fold_at_display_row(o) {
            if c >= FOLD_PLACEHOLDER_WIDTH {
                return Biased::from(InlayPoint::new(fold.start_row, c));
            }
            return Biased {
                left: InlayPoint::new(fold.start_row, 0),
                right: InlayPoint::new(fold.end_row.saturating_add(1), 0),
            };
        }
        // 普通行：恒等反向。
        Biased::from(InlayPoint::new(self.buffer_row_for_display(o), c))
    }
}

/// 派生本层 revision：FNV-1a 混合 input_version 与折叠状态。
///
/// 折叠已规范化排序，故相同折叠状态 → 相同 revision；折叠或 input_version 任一
/// 变化都会翻转 revision，使 [`LayerSnapshot::is_current`]（layer.rs 默认实现）
/// 正确判定过期。revision 是缓存有效性键而非身份标识，理论哈希碰撞可接受。
fn compute_revision(input_version: u64, folds: &[Fold]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100000001b3);
    mix(input_version);
    mix(folds.len() as u64);
    for f in folds {
        mix(f.start_row as u64);
        mix(f.end_row as u64);
    }
    h
}

fn visible_rows(buffer_rows: usize, folds: &[Fold]) -> usize {
    let hidden = folds
        .iter()
        .map(|fold| fold.end_row.saturating_sub(fold.start_row))
        .sum::<usize>();
    buffer_rows.max(1).saturating_sub(hidden).max(1)
}

fn row_items(folds: &[Fold]) -> Vec<FoldRowItem> {
    folds
        .iter()
        .copied()
        .map(|fold| FoldRowItem { fold })
        .collect()
}

fn first_changed_fold_index(old: &[Fold], new: &[Fold]) -> usize {
    old.iter()
        .zip(new.iter())
        .position(|(a, b)| a != b)
        .unwrap_or(old.len().min(new.len()))
}

fn first_changed_fold_row(old: &[Fold], new: &[Fold]) -> usize {
    old.iter()
        .zip(new.iter())
        .position(|(a, b)| a != b)
        .map_or_else(
            || {
                old.get(new.len()).map_or_else(
                    || new.get(old.len()).map_or(0, |f| f.start_row),
                    |f| f.start_row,
                )
            },
            |i| old[i].start_row.min(new[i].start_row),
        )
}

fn common_fold_suffix(old: &[Fold], new: &[Fold]) -> usize {
    let mut count = 0;
    while count < old.len()
        && count < new.len()
        && old[old.len() - 1 - count] == new[new.len() - 1 - count]
    {
        count += 1;
    }
    count
}

/// 规范化折叠（DM-203 明确政策）：按 `start_row` 排序；丢弃空/退化区间；合并
/// **交叉/嵌套/相邻（相触）** 区间。相邻区间（`last.end_row + 1 >= f.start_row`）
/// 合并，与 [`FoldSet`] 的增量插入（相触即合并、`FoldSet` 永不存相邻条目）政策一致，
/// 保证 `from_fold_set` 解析出的产物与 `FoldSet` 内部状态同构。
///
/// 这与 `DisplayMap::normalize_folds`（display_map.rs）的旧副本行为仅在「相邻是否
/// 合并」上有差异——旧副本保留相邻。DisplayMap 门面冻结至 DM-228，故在其副本处
/// 保留旧语义、不在这里改，避免破坏冻结中的 facade。
fn normalize_folds(folds: &[Fold]) -> Vec<Fold> {
    let mut out: Vec<Fold> = Vec::new();
    let mut sorted: Vec<Fold> = folds.to_vec();
    sorted.sort_by_key(|f| f.start_row);
    for f in sorted {
        if f.end_row <= f.start_row {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if f.start_row <= last.end_row + 1 {
                last.end_row = last.end_row.max(f.end_row);
                continue;
            }
        }
        out.push(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Bias;

    /// 用固定行区间构建审视映射。`input_version` 取 1。
    fn snapshot_with(folds: Vec<Fold>) -> FoldSnapshot {
        FoldSnapshot::new(1, folds)
    }

    /// DM-200：折叠前/间/后点 buffer → fold → buffer 恒等往返，`is_identical` 为真。
    #[test]
    fn outside_folds_round_trip_identity() {
        // 折叠 [1,2]：buffer 行 0 可视、1 为折叠行、2 隐藏、3 可视。
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 2,
        }]);
        // 折叠前（行 0）。
        let p0 = snap.map_input_by(InlayPoint::new(0, 3));
        assert!(p0.is_identical());
        assert_eq!(snap.map_output_by(p0.left).left, InlayPoint::new(0, 3));
        // 折叠后（行 3：其展示行 = 3 - 1 隐藏 = 2）。
        let p3 = snap.map_input_by(InlayPoint::new(3, 1));
        assert!(p3.is_identical());
        assert_eq!(p3.left, FoldPoint { row: 2, column: 1 });
        assert_eq!(
            snap.map_output_by(p3.left).left,
            InlayPoint::new(3, 1),
            "展示行 2 反向到 buffer 行 3"
        );
    }

    /// DM-200：折叠内部行 `start < r <= end` 映射到占位符起始行，left 列 0。
    #[test]
    fn inside_fold_maps_to_placeholder_start() {
        // 折叠 [1,3]：隐藏 2..3（共 2 行），占位符在折叠起始行展示行
        // disp(1) = 1（折叠行自身仍可视，内部行折叠进它）。
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 3,
        }]);
        for r in 2..=3 {
            let biased = snap.map_input_by(InlayPoint::new(r, 7));
            assert_eq!(
                biased.left,
                FoldPoint { row: 1, column: 0 },
                "内部行 {r} left 落占位符起点（展示行 1，列 0）"
            );
            assert_eq!(
                biased.right,
                FoldPoint {
                    row: 1,
                    column: FOLD_PLACEHOLDER_WIDTH
                },
                "内部行 {r} right 落占位符终点之后"
            );
        }
    }

    /// DM-200：折叠行列 0 边界与内部行的左右偏置——`Bias::Left` 在占位符起点，
    /// `Bias::Right` 越过其后；边界处 `is_identical` 为假。
    #[test]
    fn boundary_bias_left_right() {
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 2,
        }]);
        // 折叠行（start=1）列 0：折叠边界，偏置到占位符两侧（占位符在展示行 1）。
        let line_boundary = snap.map_input_by(InlayPoint::new(1, 0));
        assert!(!line_boundary.is_identical());
        assert_eq!(
            line_boundary.get(Bias::Left),
            FoldPoint { row: 1, column: 0 }
        );
        assert_eq!(
            line_boundary.get(Bias::Right),
            FoldPoint { row: 1, column: 1 }
        );
        // 折叠行列 >0：折叠行内容恒等（仍可视）。
        let line_content = snap.map_input_by(InlayPoint::new(1, 3));
        assert!(line_content.is_identical());
        assert_eq!(line_content.left, FoldPoint { row: 1, column: 3 });
        // 内部行（2）：折叠行 row=1 映射到展示行 1；左/右偏置各异。
        let internal = snap.map_input_by(InlayPoint::new(2, 0));
        assert!(!internal.is_identical());
        assert_eq!(internal.get(Bias::Left), FoldPoint { row: 1, column: 0 });
        assert_eq!(internal.get(Bias::Right), FoldPoint { row: 1, column: 1 });
    }

    /// DM-200：`map_output_by` 在占位符展示行反向到折叠边界——left 在折叠起始，
    /// right 到折叠后首行。
    #[test]
    fn reverse_maps_placeholder_back_to_fold_bounds() {
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 2,
        }]);
        // 占位符展示行 = disp(start) = 1 - 0 = 1。
        let biased = snap.map_output_by(FoldPoint { row: 1, column: 0 });
        assert!(!biased.is_identical());
        assert_eq!(biased.get(Bias::Left), InlayPoint::new(1, 0));
        assert_eq!(biased.get(Bias::Right), InlayPoint::new(3, 0));
    }

    /// DM-200：规范化丢弃空区间、合并交叉/相邻、排序（镜像 display_map 的
    /// normalize_folds 测试）。
    #[test]
    fn normalization_drops_empty_and_merges() {
        let folds = vec![
            Fold {
                start_row: 5,
                end_row: 8,
            },
            Fold {
                start_row: 1,
                end_row: 3,
            },
            Fold {
                start_row: 2,
                end_row: 4,
            },
            Fold {
                start_row: 9,
                end_row: 9,
            }, // 空区间，丢弃
            Fold {
                start_row: 3,
                end_row: 2,
            }, // 退化，丢弃
            Fold {
                start_row: 3,
                end_row: 6,
            }, // 与 [1,4]、[5,8] 均重叠，合并
        ];
        let snap = snapshot_with(folds);
        // [1,3]→[1,4]→[1,6]→[1,8]，空/退化区间被丢弃，全部合并为一个区间。
        assert_eq!(
            snap.folds(),
            &[Fold {
                start_row: 1,
                end_row: 8
            }]
        );
    }

    /// DM-200：revision —— 同状态同 revision；改折叠或 input_version 都翻转；
    /// `is_current` 仅当两者都匹配时为真。
    #[test]
    fn revision_and_is_current() {
        let folds = vec![Fold {
            start_row: 1,
            end_row: 2,
        }];
        let a = FoldSnapshot::new(1, folds.clone());
        let b = FoldSnapshot::new(1, folds.clone());
        assert_eq!(a.revision(), b.revision());
        // 同一 snapshot 自身 is_current。
        assert!(a.is_current(a.input_version(), a.revision()));
        // 改 input_version → revision 变化，旧结果过期。
        let c = FoldSnapshot::new(2, folds.clone());
        assert_ne!(a.revision(), c.revision());
        assert!(!c.is_current(a.input_version(), a.revision()));
        assert!(!a.is_current(c.input_version(), c.revision()));
        // 改折叠 → revision 变化。
        let d = FoldSnapshot::new(
            1,
            vec![Fold {
                start_row: 2,
                end_row: 4,
            }],
        );
        assert_ne!(a.revision(), d.revision());
        assert!(!d.is_current(a.input_version(), a.revision()));
    }

    /// DM-200：从稳定 FoldSet 解析构造与行区间构造一致（via `from_fold_set`）。
    #[test]
    fn from_fold_set_resolves_rows() {
        let mut set = FoldSet::new();
        set.toggle_range(crate::model::Range::new(6, 18));
        // 每行 6 字节：offset 6..18 → 行 1..3（end_offset 17 → 行 2），折叠 [1,2]。
        let snap = FoldSnapshot::from_fold_set(7, &set, |o| o / 6);
        assert_eq!(snap.input_version(), 7);
        assert_eq!(
            snap.map_input_by(InlayPoint::new(2, 0)).left,
            FoldPoint { row: 1, column: 0 },
            "内部行（行 2，折叠 [1,2] 内）映射到占位符展示行 1"
        );
    }

    /// DM-201：可见行（折叠外）input ↔ output 双向 round-trip 恒等，含列保真。
    #[test]
    fn visible_rows_bidirectional_round_trip() {
        let snap = snapshot_with(vec![
            Fold {
                start_row: 1,
                end_row: 2,
            },
            Fold {
                start_row: 5,
                end_row: 6,
            },
        ]);
        // 折叠前可见行：buffer 0（output row 0）。
        let out = snap.map_input_by(InlayPoint::new(0, 7));
        assert!(out.is_identical());
        assert_eq!(snap.map_output_by(out.right).right, InlayPoint::new(0, 7));
        // 两折叠间可见行：buffer 3（其前行 1..2 隐藏 1 行 → output row 2）、
        // buffer 4（output row 3）。
        assert_eq!(
            snap.map_output_by(snap.map_input_by(InlayPoint::new(3, 2)).left)
                .left,
            InlayPoint::new(3, 2)
        );
        assert_eq!(
            snap.map_output_by(snap.map_input_by(InlayPoint::new(4, 0)).left)
                .left,
            InlayPoint::new(4, 0)
        );
        // 折叠后可见行：buffer 7（前两折叠共隐藏 2 行 → output row 5）。
        let out7 = snap.map_input_by(InlayPoint::new(7, 1));
        assert_eq!(out7.left, FoldPoint { row: 5, column: 1 });
        assert_eq!(snap.map_output_by(out7.left).left, InlayPoint::new(7, 1));
    }

    /// DM-201：折叠起始行的可见内容列不能被反向映射吞掉——
    /// 列 ≥ 占位符宽度时按列恒等 round-trip（修复：反向丢失折叠行内容列）。
    #[test]
    fn fold_line_content_column_round_trips() {
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 3,
        }]);
        // 折叠起始行（buffer 1）列 3：正存为 (disp(1), 3) 恒等，反存回 (1, 3)。
        let out = snap.map_input_by(InlayPoint::new(1, 3));
        assert!(out.is_identical());
        assert_eq!(out.left, FoldPoint { row: 1, column: 3 });
        let back = snap.map_output_by(out.left);
        assert!(
            back.is_identical(),
            "折叠行内容列为恒等映射，不应退化为折叠边界 Biased"
        );
        assert_eq!(back.left, InlayPoint::new(1, 3));
    }

    /// DM-201：折叠边界处 bias 决定落点——`map_input_by` 边界候选非恒等，
    /// left 在占位符起点、right 越过其后；反向 boundary 与 bias 组成一对。
    #[test]
    fn bias_selects_candidate_at_boundaries() {
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 2,
        }]);
        // 折叠起始行 col 0：折叠边界，Bias::Left/Right 落在占位符两侧。
        let b = snap.map_input_by(InlayPoint::new(1, 0));
        assert!(!b.is_identical());
        assert_eq!(b.get(Bias::Left), FoldPoint { row: 1, column: 0 });
        assert_eq!(b.get(Bias::Right), FoldPoint { row: 1, column: 1 });
        // 内部行 col 0：同样折叠进占位符，bias 决定占位符起点还是终点。
        let i = snap.map_input_by(InlayPoint::new(2, 0));
        assert!(!i.is_identical());
        assert_eq!(i.get(Bias::Left), FoldPoint { row: 1, column: 0 });
        assert_eq!(i.get(Bias::Right), FoldPoint { row: 1, column: 1 });
        // 反向：占位符展示行（disp(start)=1）col 0 → 折叠边界，left=折叠起点、
        // right=折叠后首行。映射出的显示点反向取回 buffer 边界。
        let rev = snap.map_output_by(FoldPoint { row: 1, column: 0 });
        assert!(!rev.is_identical());
        assert_eq!(rev.get(Bias::Left), InlayPoint::new(1, 0));
        assert_eq!(rev.get(Bias::Right), InlayPoint::new(3, 0));
    }

    /// DM-201：外部可见行 `is_identical`（零宽）为真，折叠边界为假——bias 只在
    /// 折叠/占位边界有意义，普通行无需偏置。
    #[test]
    fn identical_only_where_no_fold_boundary() {
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 2,
        }]);
        // 外部可见行恒等（零宽）。
        assert!(snap.map_input_by(InlayPoint::new(0, 4)).is_identical());
        assert!(snap.map_input_by(InlayPoint::new(3, 4)).is_identical());
        // 折叠边界非恒等。
        assert!(!snap.map_input_by(InlayPoint::new(1, 0)).is_identical());
        assert!(!snap.map_input_by(InlayPoint::new(2, 0)).is_identical());
        // 折叠起始行内容列恒等（可见内容，非边界）。
        assert!(snap.map_input_by(InlayPoint::new(1, 4)).is_identical());
    }

    /// DM-203：规范化的「空/退化」——空区间、反向区间被丢弃，不影响其余。
    #[test]
    fn normalize_drops_empty_and_degenerate() {
        let folds = normalize_folds(&[
            Fold {
                start_row: 3,
                end_row: 3,
            }, // 空
            Fold {
                start_row: 3,
                end_row: 2,
            }, // 反向/退化
            Fold {
                start_row: 5,
                end_row: 9,
            },
        ]);
        assert_eq!(
            folds,
            vec![Fold {
                start_row: 5,
                end_row: 9
            }]
        );
    }

    /// DM-203：规范化「相邻」——两条首尾相接的折叠（`next.start == last.end + 1`）
    /// 合并为一条（与 FoldSet 相触合并政策一致）。
    #[test]
    fn normalize_merges_adjacent_folds() {
        let folds = normalize_folds(&[
            Fold {
                start_row: 1,
                end_row: 2,
            },
            Fold {
                start_row: 3,
                end_row: 5,
            }, // start 3 == last.end(2)+1 → 相触
        ]);
        assert_eq!(
            folds,
            vec![Fold {
                start_row: 1,
                end_row: 5
            }]
        );
    }

    /// DM-203：规范化「嵌套」——被包含折叠并入外层，不产生更小/重叠条目。
    #[test]
    fn normalize_merges_nested_folds() {
        let folds = normalize_folds(&[
            Fold {
                start_row: 2,
                end_row: 6,
            }, // 外层
            Fold {
                start_row: 3,
                end_row: 4,
            }, // 完全被包含
        ]);
        assert_eq!(
            folds,
            vec![Fold {
                start_row: 2,
                end_row: 6
            }]
        );
    }

    /// DM-203：规范化「交叉」——部分重叠并成覆盖两者的并集；乱序输入仍按起点排序。
    #[test]
    fn normalize_merges_crossing_and_sorts() {
        let folds = normalize_folds(&[
            Fold {
                start_row: 9,
                end_row: 11,
            },
            Fold {
                start_row: 4,
                end_row: 6,
            },
            Fold {
                start_row: 5,
                end_row: 10,
            }, // 与 [4,6)、[9,11) 均交叉
            Fold {
                start_row: 1,
                end_row: 2,
            },
        ]);
        // [1,2]；(4→ [1,2] 不相邻 4>3)；[4,6]→[5,10] 交叉→[4,10]→与[9,11]交叉→[4,11]。
        assert_eq!(
            folds,
            vec![
                Fold {
                    start_row: 1,
                    end_row: 2
                },
                Fold {
                    start_row: 4,
                    end_row: 11
                },
            ]
        );
    }

    /// DM-204：hit test 与 placeholder 共享同一归属查询——占位符展示行
    /// `is_folded_display_row` 为真且 `fold_at_display_row` 命中，普通可见行不为真。
    #[test]
    fn hit_test_matches_placeholder_rows() {
        // 折叠 [1,2]、[5,7]（各隐藏 1、2 行）。占位符展示行分别为
        // disp(1)=1、disp(5)=4(5-1 隐藏)=4。
        let snap = snapshot_with(vec![
            Fold {
                start_row: 1,
                end_row: 2,
            },
            Fold {
                start_row: 5,
                end_row: 7,
            },
        ]);
        // 占位符展示行：hit test 命中，且拿到的折叠起始行正确。
        assert!(snap.is_folded_display_row(1));
        assert_eq!(
            snap.fold_at_display_row(1).unwrap().start_row,
            1,
            "展示行 1 是折叠 [1,2] 的占位符行"
        );
        assert!(snap.is_folded_display_row(4));
        assert_eq!(
            snap.fold_at_display_row(4).unwrap().start_row,
            5,
            "展示行 4 是折叠 [5,7] 的占位符行（前面隐藏 1 行）"
        );
        // 普通可见行不是折叠占位符行。
        assert!(!snap.is_folded_display_row(0));
        assert!(!snap.is_folded_display_row(2));
        assert!(!snap.is_folded_display_row(3));
        // placeholder 展示行与 hit test 一致（同一 `placeholder_display_row` 原语）。
        assert_eq!(
            snap.placeholder_display_row(&Fold {
                start_row: 5,
                end_row: 7
            }),
            4
        );
    }

    /// DM-204：caret 与 placeholder 共享同一映射——展示行折叠边界处
    /// `map_output_by` 反向到折叠边界；buffer 折叠内点 `map_input_by` 落占位符。
    #[test]
    fn caret_and_placeholder_share_mapping() {
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 3,
        }]);
        // caret 放在展示行 1（折叠占位符行）列 0：反向到折叠边界（同 placeholder
        // 的 `map_output_by`）。
        let boundary = snap.map_output_by(FoldPoint { row: 1, column: 0 });
        assert!(!boundary.is_identical());
        assert_eq!(boundary.get(Bias::Left), InlayPoint::new(1, 0));
        assert_eq!(boundary.get(Bias::Right), InlayPoint::new(4, 0));
        // 把 caret 打进折叠内部 buffer 行（行 2）列 5：正向落到占位符（同 placeholder
        // 的 `map_input_by`），二者共享同一映射。
        let inside = snap.map_input_by(InlayPoint::new(2, 5));
        assert_eq!(inside.get(Bias::Left), FoldPoint { row: 1, column: 0 });
        assert_eq!(inside.get(Bias::Right), FoldPoint { row: 1, column: 1 });
    }

    /// DM-204：selection 端点映射与 placeholder/caret 一致——`map_input_range` 的端点
    /// 结果等于逐个 `map_input_by`，且跨越折叠的选区端点在折叠处落到占位符两侧。
    #[test]
    fn selection_uses_same_endpoint_mapping() {
        let snap = snapshot_with(vec![
            Fold {
                start_row: 1,
                end_row: 2,
            },
            Fold {
                start_row: 5,
                end_row: 6,
            },
        ]);
        // 选区从 buffer 行 0 列 0（折叠前）到 buffer 行 6 列 2（折叠[5,6]内部）。
        let (s, e) = snap.map_input_range(InlayPoint::new(0, 0), InlayPoint::new(6, 2));
        // 起始端：折叠前可见，恒等。
        assert_eq!(s.get(Bias::Left), FoldPoint { row: 0, column: 0 });
        // 结束端：落在折叠 [5,6] 内部 → 占位符展示行 disp(5)=4（前面隐藏 1 行）。
        // 与单独 map_input_by 一致（同一映射）。
        assert_eq!(
            e,
            snap.map_input_by(InlayPoint::new(6, 2)),
            "selection 结束端与 caret 纵向映射一致"
        );
        assert_eq!(e.get(Bias::Left), FoldPoint { row: 4, column: 0 });
        assert_eq!(e.get(Bias::Right), FoldPoint { row: 4, column: 1 });
        // 顺序保持：起始端展示行序数 < 结束端。
        assert!(s.get(Bias::Left).row <= e.get(Bias::Left).row);
        // buffer 侧反向取回边界（caret 反查同一映射）。
        assert_eq!(
            snap.map_output_by(e.get(Bias::Left)).get(Bias::Right),
            InlayPoint::new(7, 0),
            "占位符展示行 4 反查到折叠 [5,6] 后首行 = end_row+1 = 7"
        );
    }

    /// DM-204：`fold_containing`（buffer 侧）与占位符映射一致——被 `fold_containing`
    /// 命中的行 `map_input_by` 必落占位符，未命中的可见行为恒等。证明 placeholder /
    /// caret / selection / hit test 的 buffer 侧查询都收敛到同一折叠归属。
    #[test]
    fn fold_containing_agrees_with_mapping() {
        let snap = snapshot_with(vec![Fold {
            start_row: 1,
            end_row: 3,
        }]);
        // 折叠内部/边界行全部被 fold_containing 命中。
        for r in 1..=3 {
            assert!(
                snap.fold_containing(r).is_some(),
                "buffer 行 {r} 应命中折叠 [1,3]"
            );
            // 命中 → map_input_by 落占位符（非恒等边界或占位符内容）。
            let m = snap.map_input_by(InlayPoint::new(r, 0));
            assert_eq!(
                m.get(Bias::Left),
                FoldPoint { row: 1, column: 0 },
                "行 {r} 落占位符起点"
            );
        }
        // 可见行未命中 → map_input_by 恒等。
        for r in [0usize, 4, 8] {
            assert!(snap.fold_containing(r).is_none());
            assert!(snap.map_input_by(InlayPoint::new(r, 4)).is_identical());
        }
    }

    /// DM-203：交叉合并的链式传导——新折叠把中间若干相邻/交叉折叠一次并成一个并集
    /// （与 FoldSet 相交路径合并语义一致，FoldSnapshot 构造即规范化）。
    #[test]
    fn normalize_chains_through_bridge() {
        let folds = normalize_folds(&[
            Fold {
                start_row: 1,
                end_row: 3,
            },
            Fold {
                start_row: 3,
                end_row: 5,
            }, // 与 [1,3] 相邻 → [1,5]
            Fold {
                start_row: 5,
                end_row: 7,
            }, // 与 [1,5] 相邻 → [1,7]
        ]);
        assert_eq!(
            folds,
            vec![Fold {
                start_row: 1,
                end_row: 7
            }]
        );
    }

    #[test]
    fn sync_folds_with_patch_returns_suffix_in_fold_coordinates() {
        let old = snapshot_with(vec![Fold {
            start_row: 2,
            end_row: 4,
        }]);
        let (next, patch) = old.sync_folds_with_patch(
            2,
            vec![Fold {
                start_row: 3,
                end_row: 5,
            }],
            8,
            9,
        );
        assert_eq!(
            next.folds(),
            &[Fold {
                start_row: 3,
                end_row: 5
            }]
        );
        let edit = &patch.edits()[0];
        assert_eq!(edit.old.start, old.display_row(2));
        assert_eq!(edit.new.start, next.display_row(2));
        assert!(edit.old.end >= edit.old.start);
        assert!(edit.new.end >= edit.new.start);
    }

    #[test]
    fn sync_folds_with_patch_shares_untouched_suffix_tree() {
        let old_folds: Vec<Fold> = (0..100)
            .map(|index| Fold {
                start_row: index * 4,
                end_row: index * 4 + 1,
            })
            .collect();
        let old = FoldSnapshot::new(1, old_folds.clone());
        let mut next_folds = old_folds;
        next_folds[0].end_row = 2;
        let (next, patch) = old.sync_folds_with_patch(2, next_folds, 500, 500);
        assert!(!patch.is_empty());
        assert!(old.tree.shared_trailing_leaves(&next.tree) >= 2);
    }
}
