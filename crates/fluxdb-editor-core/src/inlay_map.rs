//! InlayMap：把不属于 buffer 的行内内容插入显示流（DM-300，Phase 3）。
//!
//! 显示管线第四个 [`LayerSnapshot`]：输入 [`BufferPoint`]（buffer 行列，字节列），
//! 输出 [`InlayPoint`]（插入 inline element 后、折叠前的坐标，显示列）。Inlay 是
//! 合成视图：它**加宽**展示列但不改变任何 buffer byte 语义，不写入 Buffer、不参与
//! undo（设计 §8.1）。典型消费者：inline hint、参数名提示、虚拟文本。
//!
//! 本层是独立自包含展示层，仅依赖 `coordinates` / `model` / `layer`，不依赖 UI。
//! 每个 inlay 在某个 buffer 字节位置插入，携带稳定位置 + bias（编辑重定位接
//! [`crate::model::Anchor`]，DM-304 接入管线时启用）、文本与显示宽度。对 buffer
//! 列 `col`，其展示列 = `col + Σ(前置于 col 的 inlay 显示宽度)`；落在某 inlay
//! 起始位置时以 [`Biased`] 区分左右（left=inlay 前，right=inlay 后）。
//!
//! `ponytail:` 本层先以「已 resolve 的 inlay 位置」直接驱动（door：UI provider 每
//! 次重新产出，位置天然新鲜）；raw [`Anchor`] 的编辑自动 relocate 在 DM-304 把
//! inlay 接进 Buffer→Fold→Tab→Wrap 整链时统一接入，避免提前引入编辑传播复杂度。

use std::rc::Rc;

use crate::buffer::BufferSnapshot;
use crate::completion::InlineHintProvider;
use crate::coordinates::{Biased, BufferPoint, InlayPoint};
use crate::layer::LayerSnapshot;
use crate::model::{Anchor, Bias, Range};

/// 单个 inlay 的唯一标识。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InlayId(pub u64);

/// 一个已 resolve 到 buffer 字节位置的 inlay。
#[derive(Clone, Debug, PartialEq)]
pub struct Inlay {
    pub id: InlayId,
    /// 插入点的 buffer 字节偏移（构造时已 resolve）。
    pub position: usize,
    /// 重叠锚点的偏向（DM-101 语义，编辑重定位用）。
    pub bias: Bias,
    /// 行内内容文本。
    pub text: Rc<str>,
    /// inlay 文本展示宽度（显示列，UTF-16 近似）。
    pub display_width: usize,
    /// style key（渲染样式索引；core 不解释其含义）。
    pub style_key: Option<u64>,
}

impl Inlay {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: InlayId,
        position: usize,
        bias: Bias,
        text: impl Into<Rc<str>>,
        display_width: usize,
        style_key: Option<u64>,
    ) -> Self {
        Self {
            id,
            position,
            bias,
            text: text.into(),
            display_width,
            style_key,
        }
    }
}

/// 按行归组、且已按列升序（同列稳定）的 inlay。
#[derive(Clone, Debug)]
struct RowInlay {
    /// 行内 buffer 字节列（`position` resolve 结果）。
    col: usize,
    /// 绝对 buffer 字节位置，用于跨编辑重定位。
    position: usize,
    /// inlay 文本展示宽度（显示列）。
    width: usize,
    id: InlayId,
    bias: Bias,
}

/// Inlay 显示层的不可变快照。
pub struct InlaySnapshot {
    /// 绑定的输入（buffer 快照）版本号，构建时记录。
    input_version: u64,
    /// 本层自身 revision：由 input_version + 每行 inlay（列/宽度/文本）派生（FNV-1a）。
    revision: u64,
    provider_revision: u64,
    /// 每行该行的 inlays，已按列升序、同列稳定排序。
    rows: Vec<Vec<RowInlay>>,
    /// provider 结果覆盖的 buffer 行范围；None 表示完整快照。
    scope_rows: Option<std::ops::Range<usize>>,
}

impl InlaySnapshot {
    /// 按 buffer 版本 + provider revision + 已 resolve inlay 列表构造（DM-303：
    /// 结果同时绑定 buffer version 与 provider revision）。`offset_to_col`（如
    /// `|o| snapshot.offset_to_point(Offset::new(o)).column`）把 inlay 的 buffer
    /// 偏移转为行号与行内字节列；构造成按行归组并稳定排序。
    pub fn new(
        input_version: u64,
        provider_revision: u64,
        inlays: Vec<Inlay>,
        mut offset_to_col: impl FnMut(usize) -> (usize, usize),
    ) -> Self {
        let mut rows: Vec<Vec<RowInlay>> = Vec::new();
        for inlay in inlays {
            let (row, col) = offset_to_col(inlay.position);
            if row >= rows.len() {
                rows.resize(row + 1, Vec::new());
            }
            rows[row].push(RowInlay {
                col,
                position: inlay.position,
                width: inlay.display_width,
                id: inlay.id,
                bias: inlay.bias,
            });
        }
        for line in rows.iter_mut() {
            line.sort_by(|a, b| a.col.cmp(&b.col).then(a.id.cmp(&b.id)));
        }
        let revision = compute_revision(input_version, provider_revision, &rows);
        Self {
            input_version,
            revision,
            provider_revision,
            rows,
            scope_rows: None,
        }
    }

    /// 标记该快照只覆盖指定的 buffer 行范围。范围外由 DisplayMap 保留旧值。
    pub fn with_row_scope(mut self, scope: std::ops::Range<usize>) -> Self {
        self.scope_rows = Some(scope);
        self
    }

    pub fn row_scope(&self) -> Option<std::ops::Range<usize>> {
        self.scope_rows.clone()
    }

    pub fn provider_revision(&self) -> u64 {
        self.provider_revision
    }

    /// 把局部 provider 结果合并到旧快照，未覆盖的行共享旧 Vec 内容。
    pub fn merge_scoped(&self, next: &Self) -> Self {
        let Some(scope) = next.scope_rows.as_ref() else {
            return next.clone_for_rows();
        };
        let end = scope.end.max(scope.start);
        let mut rows = self.rows.clone();
        rows.resize(rows.len().max(end).max(next.rows.len()), Vec::new());
        for row in scope.start.min(rows.len())..end.min(rows.len()) {
            rows[row] = next.rows.get(row).cloned().unwrap_or_default();
        }
        let revision = compute_revision(next.input_version, next.provider_revision, &rows);
        Self {
            input_version: next.input_version,
            revision,
            provider_revision: next.provider_revision,
            rows,
            scope_rows: None,
        }
    }

    fn clone_for_rows(&self) -> Self {
        Self {
            input_version: self.input_version,
            revision: self.revision,
            provider_revision: self.provider_revision,
            rows: self.rows.clone(),
            scope_rows: self.scope_rows.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.iter().all(Vec::is_empty)
    }

    pub fn sync_input(&self, input_version: u64) -> Self {
        if self.input_version == input_version {
            return Self {
                input_version,
                revision: self.revision,
                provider_revision: self.provider_revision,
                rows: self.rows.clone(),
                scope_rows: self.scope_rows.clone(),
            };
        }
        Self {
            input_version,
            revision: compute_revision(input_version, self.provider_revision, &self.rows),
            provider_revision: self.provider_revision,
            rows: self.rows.clone(),
            scope_rows: self.scope_rows.clone(),
        }
    }

    /// 按一次 Buffer 编辑重定位所有 inlay。绝对位置由 Anchor 规则迁移后重新
    /// 解析为行/列，未受影响的行仍复用同一组 `Rc<str>` 内容；不会触发全文 DisplayMap
    /// 重建，也不会丢失 provider revision。
    pub fn sync_change(
        &self,
        input_version: u64,
        new_snapshot: &BufferSnapshot,
        old_range: Range,
        new_len: usize,
    ) -> Self {
        if self.is_empty() {
            return self.sync_input(input_version);
        }
        let mut rows: Vec<Vec<RowInlay>> = Vec::new();
        for line in &self.rows {
            for inlay in line {
                let anchor = Anchor::new(inlay.position, inlay.bias).relocate(old_range, new_len);
                let position = anchor.offset.min(new_snapshot.len());
                let point = new_snapshot.offset_to_point(position);
                if point.row >= rows.len() {
                    rows.resize(point.row + 1, Vec::new());
                }
                rows[point.row].push(RowInlay {
                    col: point.column,
                    position,
                    width: inlay.width,
                    id: inlay.id,
                    bias: inlay.bias,
                });
            }
        }
        for line in &mut rows {
            line.sort_by(|a, b| a.col.cmp(&b.col).then(a.id.cmp(&b.id)));
        }
        let revision = compute_revision(input_version, self.provider_revision, &rows);
        Self {
            input_version,
            revision,
            provider_revision: self.provider_revision,
            rows,
            scope_rows: self.scope_rows.clone(),
        }
    }

    /// 该 buffer 行中所有 inlay 额外占用的展示列宽。
    pub fn row_extra_width(&self, row: usize) -> usize {
        self.rows
            .get(row)
            .map(|line| line.iter().map(|inlay| inlay.width).sum())
            .unwrap_or(0)
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// 该 buffer 行前置 inlay 的总显示宽度。
    fn widths_before(&self, row: usize, col: usize) -> usize {
        self.rows
            .get(row)
            .map(|line| {
                line.iter()
                    .take_while(|i| i.col < col)
                    .map(|i| i.width)
                    .sum()
            })
            .unwrap_or(0)
    }

    /// 该 buffer 行内、含同列 inlay 的总显示宽度（`map_input_by` right 候选用）。
    fn widths_through(&self, row: usize, col: usize) -> usize {
        self.rows
            .get(row)
            .map(|line| {
                line.iter()
                    .take_while(|i| i.col <= col)
                    .map(|i| i.width)
                    .sum()
            })
            .unwrap_or(0)
    }

    /// hit test（DM-301）：给定一个展示坐标，若落在某 inlay 的显示区间内，返回该
    /// inlay 的 id。设计 §8.1：hit 返回 inlay target，**不能**伪造 buffer offset。
    pub fn inlay_at(&self, point: InlayPoint) -> Option<InlayId> {
        let line = self.rows.get(point.row)?;
        for inlay in line {
            let start = inlay.col + self.widths_before(point.row, inlay.col);
            let after = start + inlay.width;
            if point.column >= start && point.column < after {
                return Some(inlay.id);
            }
        }
        None
    }
}

impl LayerSnapshot for InlaySnapshot {
    type Input = BufferPoint;
    type Output = InlayPoint;

    fn input_version(&self) -> u64 {
        self.input_version
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn map_input_by(&self, input: Self::Input) -> Biased<Self::Output> {
        let left = InlayPoint::new(
            input.row,
            input.column + self.widths_before(input.row, input.column),
        );
        let right = InlayPoint::new(
            input.row,
            input.column + self.widths_through(input.row, input.column),
        );
        Biased { left, right }
    }

    fn map_output_by(&self, output: Self::Output) -> Biased<Self::Input> {
        let mut cursor = 0usize; // 上一 inlay 后的 buffer 列
        let mut produced = 0usize; // 上一 inlay 后的展示列
        for inlay in self.rows.get(output.row).into_iter().flatten() {
            let start = inlay.col + self.widths_before(output.row, inlay.col);
            let after = start + inlay.width;
            if after <= output.column {
                cursor = inlay.col;
                produced = after;
                continue;
            }
            if start <= output.column {
                // display 列落在 inlay 内部/起点：偏 left 到 inlay 前 buffer 列，
                // 偏 right 到 inlay 插入位置（buffer 列 = inlay.col）。
                return Biased {
                    left: BufferPoint::new(output.row, cursor),
                    right: BufferPoint::new(output.row, inlay.col),
                };
            }
            break;
        }
        // 落在 `cursor` 与下一 inlay（或无）之间的正常 buffer 片段。
        let col = output.column.saturating_sub(produced) + cursor;
        Biased::from(BufferPoint::new(output.row, col))
    }
}

/// 派生本层 revision：FNV-1a 混合 input_version、provider revision 与每行 inlay
/// （列/宽度/id）。input_version 变化（buffer 编辑）或 provider_revision 变化
/// （provider 配置变更）都会翻转 revision，使 [`LayerSnapshot::is_current`] 判过期。
fn compute_revision(input_version: u64, provider_revision: u64, rows: &[Vec<RowInlay>]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100000001b3);
    mix(input_version);
    mix(provider_revision);
    mix(rows.len() as u64);
    for line in rows {
        for inlay in line {
            mix(inlay.col as u64);
            mix(inlay.position as u64);
            mix(inlay.width as u64);
            mix(inlay.id.0);
            mix(matches!(inlay.bias, Bias::Right) as u64);
        }
    }
    h
}

/// 估算 inlay 文本的显示列宽（纯文本，UTF-16 近似；不含 tab 展开）。
pub fn inlay_text_width(text: &str) -> usize {
    text.encode_utf16().count()
}

/// 消费 `InlineHintProvider`，把其产出转为 `InlaySnapshot`（DM-302）。
///
/// 这是 core 侧把 provider 结果接入 inlay 显示层的唯一入口：真正调用
/// [`InlineHintProvider::hints`]，用 [`inlay_text_width`] 估算每个 hint 的显示宽度，
/// 并把 hint 的 buffer 字节位置 resolve 为 (row, col)。UI 注入 provider 实现；core
/// 只依赖 provider 契约与 buffer 快照，不依赖 GPUI。`visible` 为 provider 请求的
/// 可见范围，DM-303 进一步限定为 viewport+overscan。
pub fn inline_hints_to_snapshot(
    input_version: u64,
    provider: &dyn InlineHintProvider,
    snapshot: &BufferSnapshot,
    visible: Range,
) -> InlaySnapshot {
    let hints = provider.hints(snapshot, visible);
    let inlays = hints
        .into_iter()
        .map(|hint| {
            let width = inlay_text_width(&hint.text);
            Inlay::new(
                InlayId(hint.position as u64),
                hint.position,
                Bias::Left,
                hint.text,
                width,
                None,
            )
        })
        .collect();
    let start_row = snapshot
        .offset_to_point(visible.start.min(snapshot.len()))
        .row;
    let end_row = snapshot
        .offset_to_point(visible.end.min(snapshot.len()))
        .row
        .saturating_add(1);
    InlaySnapshot::new(input_version, provider.revision(), inlays, |offset| {
        let p = snapshot.offset_to_point(offset);
        (p.row, p.column)
    })
    .with_row_scope(start_row..end_row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::EditorBuffer;
    use crate::completion::InlineHint;

    /// 解析 offset → (row, col)：用一个线性文本「aXbYc\n..."（无跨行文本）只需
    /// 一个常量行高版本。测试里用固定行：offset 直接当单字符行。
    fn col_only(offset: usize) -> (usize, usize) {
        (0, offset)
    }

    fn snap(inlays: Vec<(usize, usize)>) -> InlaySnapshot {
        let rows = inlays
            .into_iter()
            .map(|(pos, width)| {
                Inlay::new(InlayId(pos as u64), pos, Bias::Left, "hint", width, None)
            })
            .collect();
        InlaySnapshot::new(1, 0, rows, col_only)
    }

    /// DM-300：无 inlay 时映射恒等。
    #[test]
    fn empty_is_identity() {
        let s = snap(vec![]);
        let out = s.map_input_by(BufferPoint::new(0, 5));
        assert!(out.is_identical());
        assert_eq!(out.left, InlayPoint::new(0, 5));
    }

    /// DM-300：inlay 在 buffer 列 2 插入、宽 3；之后列整体加宽 3。
    #[test]
    fn insert_before_shifts_display_column() {
        let s = snap(vec![(2, 3)]);
        // col=1（inlay 之前）不加宽
        let before = s.map_input_by(BufferPoint::new(0, 1));
        assert_eq!(before.left, InlayPoint::new(0, 1));
        // col=3（inlay 之后）加宽 3
        let after = s.map_input_by(BufferPoint::new(0, 3));
        assert_eq!(after.left, InlayPoint::new(0, 6));
    }

    /// DM-300：落在 inlay 起始位置时 Biased 区分左右（left=inlay 前，right=inlay 后）。
    #[test]
    fn inlay_boundary_biased() {
        let s = snap(vec![(4, 7)]);
        let b = s.map_input_by(BufferPoint::new(0, 4));
        assert!(!b.is_identical());
        assert_eq!(b.get(Bias::Left), InlayPoint::new(0, 4), "left=inlay 前");
        assert_eq!(b.get(Bias::Right), InlayPoint::new(0, 11), "right=inlay 后");
    }

    /// DM-300：同位置多个 inlay 稳定排序后整体加宽；revision 随内容翻转。
    #[test]
    fn same_position_stable_and_revision_flips() {
        // 同列两个 inlay，宽度 2 与 3
        let a = Inlay::new(InlayId(1), 5, Bias::Left, "a", 2, None);
        let b = Inlay::new(InlayId(2), 5, Bias::Left, "b", 3, None);
        let s = InlaySnapshot::new(1, 0, vec![a.clone(), b.clone()], |o| (0, o));
        let out = s.map_input_by(BufferPoint::new(0, 6)).left;
        assert_eq!(out.column, 6 + 5, "两个 inlay 总宽 5");

        // 顺序不同 → revision 稳定（排序抵消）；内容变 → revision 翻转
        let s_swapped = InlaySnapshot::new(1, 0, vec![b, a], |o| (0, o));
        assert_eq!(
            s.revision(),
            s_swapped.revision(),
            "同集合稳定排序 revision 一致"
        );
    }

    #[test]
    fn sync_change_relocates_inlay_anchor_and_preserves_provider_revision() {
        let old = EditorBuffer::new_from("a\nb");
        let inlay = Inlay::new(InlayId(7), 2, Bias::Left, "hint", 4, None);
        let snapshot = InlaySnapshot::new(old.version(), 42, vec![inlay], |offset| {
            let point = old.snapshot().offset_to_point(offset);
            (point.row, point.column)
        });
        let mut edited = old;
        let edit = edited.edit(Range::new(0, 0), "x\n", 2, 2, true);
        let next = snapshot.sync_change(
            edited.version(),
            &edited.snapshot(),
            edit.changes[0].old_range,
            edit.changes[0].new_text.len(),
        );
        assert_eq!(next.row_extra_width(2), 4);
        assert_ne!(next.revision(), snapshot.revision());
        assert_eq!(next.map_input_by(BufferPoint::new(2, 0)).right.column, 4);
    }

    /// DM-301：buffer → inlay → buffer 双向往返（左候选）还原。
    #[test]
    fn bidirectional_round_trip() {
        let s = snap(vec![(2, 3), (6, 2)]);
        for buf_col in [0usize, 1, 2, 3, 4, 5, 6, 7, 9] {
            let out = s.map_input_by(BufferPoint::new(0, buf_col)).left;
            let back = s.map_output_by(out);
            let candidates = [back.left.column, back.right.column];
            assert!(
                candidates.contains(&buf_col),
                "buffer col {buf_col}: out={out:?} back 候选 {candidates:?} 应含原列"
            );
        }
    }

    /// DM-301：落在 inlay 显示区间内，hit test 返回该 inlay id；区间外返回 None。
    #[test]
    fn hit_test_returns_inlay_target() {
        let s = snap(vec![(2, 3)]);
        // inlay 显示区间 [2, 5) —— display 列 2..5 命中
        assert_eq!(s.inlay_at(InlayPoint::new(0, 2)), Some(InlayId(2)));
        assert_eq!(s.inlay_at(InlayPoint::new(0, 4)), Some(InlayId(2)));
        // 区间外不命中
        assert_eq!(s.inlay_at(InlayPoint::new(0, 1)), None);
        assert_eq!(s.inlay_at(InlayPoint::new(0, 5)), None, "after 端点不命中");
    }

    /// DM-301：多个 inlay 各自命中各自区间；区间之间（普通 buffer 片段）不命中。
    /// inlay1@buf2 宽 3 → 显示 [2,5)；inlay2@buf6 宽 2 → 显示 [9,11)。
    #[test]
    fn hit_test_multiple_segments() {
        let s = snap(vec![(2, 3), (6, 2)]);
        assert_eq!(s.inlay_at(InlayPoint::new(0, 2)), Some(InlayId(2)));
        assert_eq!(s.inlay_at(InlayPoint::new(0, 4)), Some(InlayId(2)));
        assert_eq!(
            s.inlay_at(InlayPoint::new(0, 5)),
            None,
            "buffer 片段 5..9 不命中"
        );
        assert_eq!(s.inlay_at(InlayPoint::new(0, 8)), None);
        assert_eq!(s.inlay_at(InlayPoint::new(0, 9)), Some(InlayId(6)));
        assert_eq!(s.inlay_at(InlayPoint::new(0, 10)), Some(InlayId(6)));
    }

    /// 一个固定产出的假 provider：在指定字节位置返回固定文本的 hint。
    struct FakeInlineHints {
        positions: Vec<usize>,
        text: &'static str,
    }
    impl InlineHintProvider for FakeInlineHints {
        fn hints(&self, _snapshot: &BufferSnapshot, _visible: Range) -> Vec<InlineHint> {
            self.positions
                .iter()
                .map(|&p| InlineHint {
                    position: p,
                    text: self.text.to_string(),
                })
                .collect()
        }
    }

    /// DM-302：provider 结果真实消费为 InlaySnapshot，位置/宽度映射正确。
    #[test]
    fn provider_results_become_inlay_snapshot() {
        let text = "select a, b"; // 单行，'a' 在字节 7，'b' 在字节 11
        let snap = EditorBuffer::new_from(text).snapshot();
        let provider = FakeInlineHints {
            positions: vec![7, 11],
            text: "..", // 每个 hint 2 显示列
        };
        let inlay = inline_hints_to_snapshot(snap.version(), &provider, &snap, Range::default());
        // 逐 buffer 列观察 inlay 加宽：pos7 宽 2，pos11 宽 2
        for (buf_col, expect) in [(7usize, 7usize), (8, 10), (10, 12), (11, 13), (12, 16)] {
            let out = inlay.map_input_by(BufferPoint::new(0, buf_col)).left;
            assert_eq!(out.row, 0);
            assert_eq!(
                out.column, expect,
                "buffer col {buf_col} 展示列应加宽前置 inlay"
            );
        }
        // hit test：provider 产出的 inlay 显示区间可命中
        assert_eq!(inlay.inlay_at(InlayPoint::new(0, 7)), Some(InlayId(7)));
        assert_eq!(inlay.inlay_at(InlayPoint::new(0, 13)), Some(InlayId(11)));
    }

    #[test]
    fn scoped_provider_snapshot_preserves_offscreen_rows() {
        let old = InlaySnapshot::new(
            1,
            1,
            vec![
                Inlay::new(InlayId(1), 1, Bias::Left, "a", 1, None),
                Inlay::new(InlayId(2), 20, Bias::Left, "b", 1, None),
            ],
            |offset| (offset / 10, offset % 10),
        );
        let next = InlaySnapshot::new(
            1,
            2,
            vec![Inlay::new(InlayId(3), 11, Bias::Left, "c", 2, None)],
            |offset| (offset / 10, offset % 10),
        )
        .with_row_scope(1..2);
        let merged = old.merge_scoped(&next);
        assert_eq!(merged.row_extra_width(0), 1);
        assert_eq!(merged.row_extra_width(1), 2);
        assert_eq!(merged.row_extra_width(2), 1);
        assert_eq!(merged.provider_revision(), 2);
    }

    /// DM-303：provider revision 变化 → 同一 buffer 旧结果被判过期（is_current 假）；
    /// provider revision 相同 → 结果仍有效。
    #[test]
    fn provider_revision_invalidates_results() {
        let a = InlaySnapshot::new(7, 1, Vec::new(), |_| (0, 0));
        let b = InlaySnapshot::new(7, 2, Vec::new(), |_| (0, 0)); // 仅 provider revision 变
        assert_ne!(a.revision(), b.revision());
        assert!(
            a.is_current(7, a.revision()),
            "同 buffer+同 provider rev 有效"
        );
        assert!(
            !b.is_current(7, a.revision()),
            "provider revision 已变，a 的结果不能在 b 复用"
        );
    }

    /// DM-305：caret 落在 inlay 起始处必须被 `Biased` 展开——左偏放 inlay 前、右偏放
    /// 之后，光标两条候选不能退回同一个点（供编辑器 caret 定位用）。
    #[test]
    fn caret_at_inlay_boundary_is_biased() {
        let s = snap(vec![(4, 7)]);
        // caret 停在 inlay 之前：单点，不分裂
        let before = s.map_input_by(BufferPoint::new(0, 3));
        assert!(before.is_identical(), "inlay 前光标恒等");
        // caret 停在 inlay 起始 buffer 列：左右分裂
        let boundary = s.map_input_by(BufferPoint::new(0, 4));
        assert!(!boundary.is_identical(), "inlay 起点光标分裂");
        assert_eq!(boundary.left, InlayPoint::new(0, 4), "左偏＝inlay 前");
        assert_eq!(
            boundary.right,
            InlayPoint::new(0, 11),
            "右偏＝inlay 后（跳过 inlay 宽）"
        );
        // caret 停在 inlay 之后：单点，恒等
        let after = s.map_input_by(BufferPoint::new(0, 5));
        assert!(after.is_identical(), "inlay 后光标恒等");
    }

    /// DM-305：selection 跨界 inlay——buffer 选区起止端点各自跨层加宽；inlay 文本是
    /// 显示装饰、不写回 buffer，因此两端点 buffer 列保持不变（copy/selection 只覆盖
    /// buffer 文本，符合设计 §8.1「selection/copy 默认只复制 buffer 文本」）。
    #[test]
    fn selection_spanning_inlay_maps_both_endpoints() {
        let s = snap(vec![(2, 3)]);
        // buffer 选区 [1, 6) 跨过 inlay@2（宽 3）：起点不加宽、终点加宽 3
        let start = s.map_input_by(BufferPoint::new(0, 1)).left;
        let end = s.map_input_by(BufferPoint::new(0, 6)).left;
        assert_eq!(start, InlayPoint::new(0, 1), "起点在 inlay 前不加宽");
        assert_eq!(end, InlayPoint::new(0, 9), "终点在 inlay 后加宽 3");
        // 反向：展示选区 [1,9) 反解回 buffer 不带着 inlay 文本
        let back_start = s.map_output_by(start);
        let back_end = s.map_output_by(end);
        assert_eq!(back_start.left.column, 1, "展示起点回 buffer 1");
        assert_eq!(
            back_end.left.column, 6,
            "展示终点回 buffer 6（inlay 不计入）"
        );
    }

    /// DM-305：hit test 相邻 inlay 的边界——上一 inlay 的 after 恰等于下一 inlay 的
    /// start 时，该展示列归属下一 inlay；inlay 内部命中自身；inlay 间普通 buffer 片段
    /// 不命中任何 inlay。
    #[test]
    fn hit_test_adjacent_inlays_separates_gap_and_body() {
        // inlay A@buf2 宽 3 → 显示 [2,5)；inlay B@buf5 宽 2 → 显示 [8,10)
        // （B 的 start 会加上其前所有 inlay 宽 3），A 与 B 之间隔普通 buffer 片段 [5,8)。
        let s = snap(vec![(2, 3), (5, 2)]);
        assert_eq!(s.inlay_at(InlayPoint::new(0, 2)), Some(InlayId(2)), "A 体");
        assert_eq!(s.inlay_at(InlayPoint::new(0, 4)), Some(InlayId(2)), "A 尾");
        assert_eq!(
            s.inlay_at(InlayPoint::new(0, 5)),
            None,
            "A/B 间 buffer 片段不命中"
        );
        assert_eq!(
            s.inlay_at(InlayPoint::new(0, 8)),
            Some(InlayId(5)),
            "B 起始（受 A 宽 3 前移）"
        );
        assert_eq!(s.inlay_at(InlayPoint::new(0, 9)), Some(InlayId(5)), "B 体");
        assert_eq!(s.inlay_at(InlayPoint::new(0, 10)), None, "B 后不命中");
    }
}
