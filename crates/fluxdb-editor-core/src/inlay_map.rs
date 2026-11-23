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

/// 估算 inlay 文本的显示列宽（纯文本，按字符展示宽度；不含 tab 展开）。
pub fn inlay_text_width(text: &str) -> usize {
    text.chars().map(crate::tab_map::char_display_width).sum()
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
