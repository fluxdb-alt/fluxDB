//! 统一的 layer 编辑传播与脏区协议（DM-104）。
//!
//! 一次编辑在多显示层间传播时使用统一「事务记录」，各层顺序消费并扩展脏区
//! （设计 10.1/10.2）。全量重建只有显式原因（设计 10.3）才允许，且必须带
//! reason 日志，不允许静默回退。
//!
//! 本模块只定义**协议本身的类型与语义**（事务记录、脏区扩大、全量重建门控），
//! 具体层（Fold/Tab/Wrap/Inlay/Block）在对应阶段消费它并实现自身扩展规则。

use crate::model::{Range, TextChange};

/// 一次编辑的统一事务记录（设计 10.1）。
///
/// 一次用户编辑产生一条该记录，携带全局定位（editor/edit id）与版本迁移
/// （`old_version -> new_version`）及文本变更，按固定顺序传给各层：
/// Anchor 索引 → Inlay → Fold → Tab → Wrap → Block → DisplaySnapshot。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditTransaction {
    /// 产生编辑的编辑器 id。
    pub editor_id: u64,
    /// 本次编辑的全局递增 id。
    pub edit_id: u64,
    /// 编辑前 input snapshot 版本。
    pub old_version: u64,
    /// 编辑后 input snapshot 版本（应等于消费方新版 input_version）。
    pub new_version: u64,
    /// 文本变更本身。
    pub change: TextChange,
    /// 编辑前选区（可选）。
    pub selection_before: Option<Range>,
    /// 编辑后选区（可选）。
    pub selection_after: Option<Range>,
}

impl EditTransaction {
    pub fn new(
        editor_id: u64,
        edit_id: u64,
        old_version: u64,
        new_version: u64,
        change: TextChange,
    ) -> Self {
        Self {
            editor_id,
            edit_id,
            old_version,
            new_version,
            change,
            selection_before: None,
            selection_after: None,
        }
    }
}

/// 输入坐标空间中的一个脏区（半开区间，字节 offset）。
///
/// 各层把 EditTransaction 中与自身相关的部分展开为脏区，供「只重算受影响
/// 摘要/path」使用；脏区会按 10.2 规则扩大。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRange {
    pub start: u64,
    pub end: u64,
}

impl DirtyRange {
    pub fn new(start: u64, end: u64) -> Self {
        Self {
            start: start.min(end),
            end: start.max(end),
        }
    }

    /// 从一次文本变更的旧区间得到初始脏区（10.2 第一层：字节编辑）。
    pub fn from_change(change: &TextChange) -> Self {
        let start = change.old_range.start as u64;
        // 中新文本长度作为替换后的区间长度下限，保证覆盖插入扩展。
        let end = start + change.new_text.len() as u64;
        Self::new(start, end)
    }

    /// 扩大脏区到包含另一区间（10.2：扩大到受影响逻辑行 / 相邻行等）。
    pub fn include(&mut self, other: DirtyRange) {
        self.start = self.start.min(other.start);
        self.end = self.end.max(other.end);
    }

    /// 把脏区边界对齐到给定的行边界集（把字节脏区扩大到整个受影响行）。
    ///
    /// `line_boundaries` 按升序给出各逻辑行首字节 offset；把 `self` 的
    /// start 左移到所在行行首、end 右移到下一行行首（或文档末尾）。
    /// 返回「是否实际扩大」（用于判断是否需要级联失效相邻行）。
    pub fn expand_to_lines(&mut self, boundaries: &[u64]) -> bool {
        let end_doc = boundaries.last().copied().unwrap_or(0);
        // 找包含 start 的那一行（最后一个 <= start 的行首）。
        let line_start = match boundaries.iter().rev().find(|&&b| b <= self.start) {
            Some(&b) => b,
            None => 0,
        };
        // 找 line_end：下一行行首；start 恰在某行首时 end 至少到下一行。
        let line_end = boundaries
            .iter()
            .find(|&&b| b > self.start)
            .copied()
            .unwrap_or(end_doc)
            .max(end_doc);
        let start0 = self.start;
        let end0 = self.end;
        self.start = self.start.min(line_start);
        self.end = self.end.max(line_end).max(self.end);
        self.start != start0 || self.end != end0
    }
}

/// 全量重建的显式原因（设计 10.3）。
///
/// 只有这些原因允许层显式重建；重建调用方必须记录该 reason，不得静默回退。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullRebuildReason {
    /// 初次加载。
    InitialLoad,
    /// 整篇外部替换（如 open/覆盖）。
    FullDocumentReplace,
    /// 不兼容的 Snapshot/version gap。
    VersionGap,
    /// 字体系统或主题改变字体度量。
    FontMetricsChanged,
    /// debug 一致性校验发现 transform 损坏。
    TransformCorruption,
}

/// 全量重建的门控决策。
///
/// 层在消费一次编辑时先调用 [`FullRebuild::for_change`]：若该文本变更本身
/// 触发全量重建（整篇替换 / 版本跳变），返回对应 reason；否则返回 `None`，
/// 表示应走增量脏区路径。层自身的配置变化（tab size/字体/宽度）由层单独
/// 传入对应 reason。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullRebuild {
    /// 需要全量重建，携带原因。
    Needed(FullRebuildReason),
    /// 走增量路径（不需要全量重建）。
    Incremental,
}

impl FullRebuild {
    /// 由文本变更判定是否需要全量重建。
    pub fn for_change(change: &TextChange, old_version: u64) -> FullRebuild {
        if change.full_document {
            return FullRebuild::Needed(FullRebuildReason::FullDocumentReplace);
        }
        // 版本跳变（差值 > 1）视为不兼容 gap：增量摘要无法保证一致。
        if change.version > old_version.saturating_add(1) {
            return FullRebuild::Needed(FullRebuildReason::VersionGap);
        }
        FullRebuild::Incremental
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change_for(old: Range, new: &str) -> TextChange {
        TextChange::new(old, new.to_string(), 2)
    }

    /// DM-104：编辑事务携带统一的定位与版本迁移信息。
    #[test]
    fn edit_transaction_carries_unified_identity() {
        let tx = EditTransaction::new(7, 100, 1, 2, change_for(Range::new(3, 6), "xyz"));
        assert_eq!(tx.editor_id, 7);
        assert_eq!(tx.edit_id, 100);
        assert_eq!((tx.old_version, tx.new_version), (1, 2));
        assert_eq!(tx.change.old_range, Range::new(3, 6));
        assert_eq!(tx.change.new_text, "xyz");
    }

    /// DM-104：字节编辑的初始脏区覆盖替换区间且纳入新文本长度。
    #[test]
    fn dirty_range_from_change_covers_insertion() {
        let change = change_for(Range::new(10, 12), "abcd");
        let d = DirtyRange::from_change(&change);
        // start 沿用旧区间起点；end 覆盖新文本长度。
        assert_eq!((d.start, d.end), (10, 10 + 4));
        assert_eq!(d.start, 10);
        assert_eq!(d.end, 14);
    }

    /// DM-104：脏区 include 为并集。
    #[test]
    fn dirty_range_include_unions() {
        let mut d = DirtyRange::new(5, 8);
        d.include(DirtyRange::new(1, 3));
        d.include(DirtyRange::new(20, 22));
        assert_eq!((d.start, d.end), (1, 22));
        // 完全被包含的区间不改变范围。
        d.include(DirtyRange::new(2, 21));
        assert_eq!((d.start, d.end), (1, 22));
    }

    /// DM-104：脏区扩大到整行（10.2 扩大到受影响逻辑行）。
    #[test]
    fn dirty_range_expands_to_whole_line() {
        // 行首：0, 6, 12。
        let lines = &[0u64, 6, 12];
        let mut d = DirtyRange::new(8, 9); // 位于第 1 行（行首 6）
        assert!(d.expand_to_lines(lines));
        // 第 1 行覆盖 [6,12)，扩大到下一行行首 12。
        assert_eq!((d.start, d.end), (6, 12));
        // 再次扩大到同一行应无变化。
        assert!(!d.expand_to_lines(lines));
        // 末尾行换行变化需包含相邻行：start 对齐到第 3 行行首 12，
        // end 保留插入后的新文本范围（14）。
        let mut e = DirtyRange::new(13, 14);
        e.expand_to_lines(lines);
        assert_eq!((e.start, e.end), (12, 14));
    }

    /// DM-104：全量重建门槛——整篇替换 / 版本跳变触发，否则增量。
    #[test]
    fn full_rebuild_is_gated_by_explicit_reasons() {
        // 普通增量编辑：不触发。
        assert_eq!(
            FullRebuild::for_change(&change_for(Range::new(1, 2), "x"), 1),
            FullRebuild::Incremental
        );
        // 整篇替换：触发。
        let full = TextChange::full_document("new".to_string(), 2);
        assert_eq!(
            FullRebuild::for_change(&full, 1),
            FullRebuild::Needed(FullRebuildReason::FullDocumentReplace)
        );
        // 版本跳变（1 -> 3）：触发 version gap。
        let jump = TextChange {
            version: 3,
            ..change_for(Range::new(1, 1), "x")
        };
        assert_eq!(
            FullRebuild::for_change(&jump, 1),
            FullRebuild::Needed(FullRebuildReason::VersionGap)
        );
        // 连续版本（0 -> 1）不触发。
        let cont = TextChange {
            version: 1,
            ..change_for(Range::new(1, 1), "x")
        };
        assert_eq!(FullRebuild::for_change(&cont, 0), FullRebuild::Incremental);
    }
}
