//! 语言能力与解析协议。
//!
//! 通用语言定义（词字符、注释、括号、折叠、缩进）与版本化语法解析协议。
//! 不包含任何 SQL / Redis / 业务类型，由 adapter（dialect + tree-sitter）实现具体的
//! `SyntaxProvider`。

use std::pin::Pin;
use std::{borrow::Cow, future::Future, sync::Arc};

use crate::buffer::BufferSnapshot;
use crate::language::SyntaxInjection;
use crate::model::Range;
use crate::sum_tree::{IntervalSummary, OffsetItem, SumTree, SumTreeItem, Summary};

/// 换行缩进请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndentRequest {
    /// 当前行文本。
    pub line_text: String,
    /// 缩进单位（按 profile.tab_size）。
    pub tab_size: usize,
}

/// 通用语言定义。由 adapter 实现具体语言的词法/括号/折叠/缩进规则。
pub trait LanguageDefinition: Send + Sync {
    fn language_id(&self) -> &str;

    /// 是否为词字符（用于单词移动与补全前缀提取）。
    fn word_char(&self, ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    /// 行注释标记。默认 `//`；接入方可按语言用 `line_comment()` 覆盖，
    /// 如 SQL 用 `--`、MySQL 亦可配 `#`。
    fn line_comment(&self) -> Option<&str> {
        Some("//")
    }

    /// 块注释标记 (start, end)。
    fn block_comment(&self) -> Option<(&str, &str)> {
        None
    }

    /// 自动配对括号。
    fn brackets(&self) -> &[(char, char)] {
        &[('(', ')'), ('[', ']'), ('{', '}')]
    }

    /// 折叠区间列表（字节区间）。
    fn fold_ranges(&self, _snapshot: &BufferSnapshot) -> Vec<Range> {
        Vec::new()
    }

    /// 换行后的缩进。
    fn indent_for_newline(&self, _request: IndentRequest) -> Option<String> {
        None
    }
}

/// 语法解析的异步结果，携带 buffer version，用于丢弃旧结果。
#[derive(Clone, Debug)]
pub struct SyntaxResult {
    pub buffer_version: u64,
    /// 语法元素；provider 返回的是其 `dirty_ranges` 范围内重解析出的高亮（增量 patch）。
    /// 宿主把该 patch 用 `HighlightStore::apply_range` 合并进主存储（DM-400/401）。
    pub highlights: Vec<Highlight>,
    /// 本次解析覆盖的区间：`highlights` 精确落在这些区间内，宿主据此做 dirty-range 替换。
    /// 空表示该结果是一次**全量**重建（宿主直接用 `from_highlights` 复建主存储）。
    pub dirty_ranges: Vec<Range>,
    /// 结果是否只是快速首轮结果，需要宿主在同一版本继续请求正式解析。
    pub needs_refinement: bool,
}

/// 将字节范围映射到覆盖它们的 buffer 行，供 provider 在后台预构建渲染索引。
pub fn build_range_line_index<I>(snapshot: &BufferSnapshot, ranges: I) -> Vec<Vec<usize>>
where
    I: IntoIterator<Item = Range>,
{
    let mut index = vec![Vec::new(); snapshot.line_count()];
    let text_len = snapshot.len();
    let line_count = snapshot.line_count();
    let mut hint_row = 0usize;
    let mut hint_offset = 0usize;
    for (item_index, range) in ranges.into_iter().enumerate() {
        let start = range.start.min(text_len);
        let end = range.end.min(text_len);
        if start >= end {
            continue;
        }
        let first_row = if start >= hint_offset {
            while hint_row + 1 < line_count && snapshot.line_start(hint_row + 1) <= start {
                hint_row += 1;
            }
            hint_row
        } else {
            snapshot.offset_to_point(start).row
        };
        hint_offset = start;
        let mut last_row = first_row;
        while last_row + 1 < line_count && snapshot.line_start(last_row + 1) <= end - 1 {
            last_row += 1;
        }
        hint_row = last_row;
        for row in first_row..=last_row {
            if let Some(items) = index.get_mut(row) {
                items.push(item_index);
            }
        }
    }
    index
}

/// 一次语法高亮片段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Range,
    /// 语义标签（如 "keyword"、"string"、"comment"），由前端映射到主题 token 颜色。
    pub kind: Cow<'static, str>,
}

/// 持久化分块高亮索引。底层复用通用 `SumTree`，后缀编辑通过节点级 lazy delta
/// 平移；局部 dirty chunk 只复制从根到叶子的路径，未触碰的 Arc 节点保持共享。
#[derive(Clone, Debug)]
pub struct HighlightStore {
    tree: SumTree<HighlightChunk>,
    text_len: usize,
}

#[derive(Clone, Debug)]
struct HighlightChunk {
    highlights: Arc<Vec<Highlight>>,
    delta: isize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HighlightSummary {
    min_start: usize,
    max_end: usize,
}

impl Summary for HighlightSummary {
    fn add(&self, other: &Self) -> Self {
        Self {
            min_start: self.min_start.min(other.min_start),
            max_end: self.max_end.max(other.max_end),
        }
    }
}

impl IntervalSummary for HighlightSummary {
    fn start(&self) -> usize {
        self.min_start
    }

    fn end(&self) -> usize {
        self.max_end
    }
}

impl SumTreeItem for HighlightChunk {
    type Summary = HighlightSummary;

    fn summary(&self) -> Self::Summary {
        let Some(first) = self.highlights.first() else {
            return HighlightSummary::default();
        };
        let min_start = shift_offset(first.range.start, self.delta);
        let max_end = self
            .highlights
            .last()
            .map(|highlight| shift_offset(highlight.range.end, self.delta))
            .unwrap_or(min_start);
        HighlightSummary { min_start, max_end }
    }
}

impl OffsetItem for HighlightChunk {
    fn range(summary: &Self::Summary) -> Option<(usize, usize)> {
        if summary.min_start == usize::MAX || summary.min_start >= summary.max_end {
            None
        } else {
            Some((summary.min_start, summary.max_end))
        }
    }

    fn shift_summary(summary: &Self::Summary, delta: isize) -> Self::Summary {
        if summary.min_start == usize::MAX {
            *summary
        } else {
            Self::Summary {
                min_start: shift_offset(summary.min_start, delta),
                max_end: shift_offset(summary.max_end, delta),
            }
        }
    }

    fn apply_delta(&mut self, delta: isize) {
        self.delta = self.delta.saturating_add(delta);
    }

    fn apply_edit(&mut self, old_start: usize, old_end: usize, byte_delta: isize) -> bool {
        let insertion = old_start == old_end;
        let mut updated = Vec::with_capacity(self.highlights.len());
        for highlight in self.highlights.iter() {
            let start = shift_offset(highlight.range.start, self.delta);
            let end = shift_offset(highlight.range.end, self.delta);
            let overlaps_replaced = start < old_end && end > old_start;
            let insertion_splits_token = insertion && start < old_start && old_start < end;
            if overlaps_replaced || insertion_splits_token {
                continue;
            }
            let mut highlight = highlight.clone();
            highlight.range = if start >= old_end {
                Range::new(
                    shift_offset(start, byte_delta),
                    shift_offset(end, byte_delta),
                )
            } else {
                Range::new(start, end)
            };
            updated.push(highlight);
        }
        self.highlights = Arc::new(updated);
        self.delta = 0;
        !self.highlights.is_empty()
    }
}

const HIGHLIGHTS_PER_CHUNK: usize = 1024;

impl Default for HighlightStore {
    fn default() -> Self {
        Self {
            tree: SumTree::default(),
            text_len: 0,
        }
    }
}

impl HighlightStore {
    pub fn from_highlights(highlights: Arc<Vec<Highlight>>, text_len: usize) -> Self {
        let chunks = highlights
            .chunks(HIGHLIGHTS_PER_CHUNK)
            .map(|slice| HighlightChunk {
                highlights: Arc::new(slice.to_vec()),
                delta: 0,
            })
            .collect::<Vec<_>>();
        Self {
            tree: SumTree::from_items(&chunks),
            text_len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tree.total().min_start == usize::MAX
    }

    /// 返回与可见区相交的高亮；树摘要先剪枝，分配量与可见 token 数相关。
    pub fn iter_intersecting(&self, range: Range) -> Vec<Highlight> {
        self.tree
            .iter_intersecting((range.start, range.end))
            .into_iter()
            .flat_map(|chunk| {
                let delta = chunk.delta;
                chunk
                    .highlights
                    .iter()
                    .map(move |highlight| {
                        let mut highlight = highlight.clone();
                        highlight.range = Range::new(
                            shift_offset(highlight.range.start, delta),
                            shift_offset(highlight.range.end, delta),
                        );
                        highlight
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|highlight| {
                highlight.range.end > range.start && highlight.range.start < range.end
            })
            .collect()
    }

    /// 应用字节编辑。后缀 chunk 通过持久节点 lazy delta 平移，dirty 叶子才 materialize。
    pub fn apply_edit(&mut self, old_range: Range, new_len: usize, text_len: usize) {
        let old_start = old_range.start.min(self.text_len);
        let old_end = old_range.end.min(self.text_len).max(old_start);
        self.tree.apply_edit((old_start, old_end), new_len);
        self.text_len = text_len;
    }

    /// 用 `new_highlights` 替换落在 `old_range` 内的高亮（DM-400/402）。语义：旧高亮
    /// 与该区间相交（token 被重解析）者移除，由 provider 的 `new_highlights` 重建；未相交
    /// 的边界高亮保留。只读取并重建相交叶子区间，未触碰的后缀子树保持 `Arc` 共享，因此
    /// 单字符/局部编辑不会重走全部高亮。
    ///
    /// 前置：调用方需先 `apply_edit(old_range, new_len, text_len)` 平移坐标；本方法输入的
    /// `new_highlights` 与旧高亮均须处于同一（编辑后）绝对坐标空间。
    pub fn apply_range(&self, old_range: Range, new_highlights: &[Highlight]) -> Self {
        let old_start = old_range.start.min(self.text_len);
        let old_end = old_range.end.min(self.text_len).max(old_start);
        // 定位与 dirty 区间相交的叶子区间；无相交则落在插入点处（纯插入，替换空区间）。
        let span = self.tree.intersecting_leaf_span((old_start, old_end));
        let (f, l) = match span {
            Some(span) => span,
            None => {
                let p = self
                    .tree
                    .lower_bound_start(old_start)
                    .min(self.tree.leaf_count());
                (p, p)
            }
        };
        // 展开相交叶子内高亮（绝对坐标），剔除与 dirty 区间相交的旧 token。
        let kept: Vec<Highlight> = self
            .tree
            .leaf_span_items(f, l)
            .into_iter()
            .flat_map(Self::chunk_highlights_abs)
            .filter(|h| !(h.range.start < old_end && h.range.end > old_start))
            .collect();
        // 合并 kept + new_highlights 并按字节序重排，重切 chunk（绝对坐标，delta=0）。
        let mut merged = Vec::with_capacity(kept.len() + new_highlights.len());
        merged.extend(kept);
        merged.extend_from_slice(new_highlights);
        merged.sort_by_key(|h| (h.range.start, h.range.end));
        let inserted: Vec<HighlightChunk> = merged
            .chunks(HIGHLIGHTS_PER_CHUNK)
            .map(|slice| HighlightChunk {
                highlights: Arc::new(slice.to_vec()),
                delta: 0,
            })
            .collect();
        Self {
            tree: self.tree.replace_leaves(f, l, &inserted),
            text_len: self.text_len,
        }
    }

    /// 把 chunk 内高亮从「chunk 相对 delta」展开为绝对坐标（供替换合并用）。
    fn chunk_highlights_abs(chunk: HighlightChunk) -> Vec<Highlight> {
        let delta = chunk.delta;
        chunk
            .highlights
            .iter()
            .map(|highlight| {
                let mut highlight = highlight.clone();
                highlight.range = Range::new(
                    shift_offset(highlight.range.start, delta),
                    shift_offset(highlight.range.end, delta),
                );
                highlight
            })
            .collect()
    }

    /// 与本 store 共享的尾部连续叶子数（DM-202/400/402 用量检测，仅测试用）。
    #[cfg(test)]
    fn shared_trailing_leaves(&self, other: &Self) -> usize {
        self.tree.shared_trailing_leaves(&other.tree)
    }
}

fn shift_offset(offset: usize, delta: isize) -> usize {
    if delta >= 0 {
        offset.saturating_add(delta as usize)
    } else {
        offset.saturating_sub(delta.unsigned_abs())
    }
}

/// 异步工作类型：使用标准 Future，禁止依赖 GPUI Task。
pub type AsyncWork<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// 一次编辑产生的脏区间（供 provider 做增量解析与局部重绘）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputEdit {
    /// 被替换的旧文本区间。
    pub old_range: Range,
    /// 替换后的新文本（可为空，如删除）。
    pub new_text: String,
    /// 变更后的 buffer 版本。
    pub version: u64,
    /// 是否代表整篇文档替换；此类变更不能走 statement 局部路径。
    pub full_document: bool,
    /// 产生这次变更的编辑序号；跨层日志关联用，旧调用默认为 0。
    pub edit_id: u64,
}

impl InputEdit {
    pub fn new(old_range: Range, new_text: String, version: u64) -> Self {
        Self {
            old_range,
            new_text,
            version,
            full_document: false,
            edit_id: 0,
        }
    }

    pub fn full_document(new_text: String, version: u64) -> Self {
        Self {
            old_range: Range::new(0, 0),
            new_text,
            version,
            full_document: true,
            edit_id: 0,
        }
    }
}

/// 通用语法 provider。
pub trait SyntaxProvider: Send + Sync {
    /// 对给定快照解析语法，并携带上一次编辑的脏区间 `changed`。
    ///
    /// 实现可据此做增量解析，把结果（含 `dirty_ranges`）写回 `SyntaxResult`；
    /// core/buffer 版本号作为宿主丢弃过期结果的依据。简单实现可忽略 `changed`
    /// 直接全量解析，但不能作为最终 SQL 高亮实现（见设计 5.3）。
    fn parse(&self, snapshot: &BufferSnapshot, changed: InputEdit) -> AsyncWork<SyntaxResult>;

    /// 在完整解析结果到达前，为可见区提供轻量级临时高亮。
    ///
    /// 默认不提供 fallback；语言适配器可以实现一个不依赖完整语法树的词法器。
    /// 返回的 range 必须使用 snapshot 的全文字节偏移。
    fn highlight_visible(&self, _snapshot: &BufferSnapshot, _visible: Range) -> Vec<Highlight> {
        Vec::new()
    }

    /// 触发解析的能力类型（是否支持自动脚本语言高亮）。
    fn highlight_enabled(&self) -> bool {
        true
    }

    /// 返回当前快照中的嵌套语言注入。默认没有注入，SQL/纯文本等单语言
    /// provider 无需实现；HTML/模板等 adapter 可只为受影响范围返回注入。
    fn injections(&self, _snapshot: &BufferSnapshot, _visible: Range) -> Vec<SyntaxInjection> {
        Vec::new()
    }
}

/// 版本化语法快照容器，供前端按 buffer version 合并结果。
#[derive(Clone, Debug, Default)]
pub struct SyntaxSnapshot {
    pub buffer_version: u64,
    pub highlights: Arc<Vec<Highlight>>,
    pub highlight_store: Arc<HighlightStore>,
}

impl SyntaxSnapshot {
    /// 检查结果版本是否仍与给定版本一致（不一致则丢弃）。
    pub fn is_current(&self, buffer_version: u64) -> bool {
        self.buffer_version == buffer_version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestLang;

    impl LanguageDefinition for TestLang {
        fn language_id(&self) -> &str {
            "test"
        }
    }

    #[test]
    fn default_word_char() {
        let lang = TestLang;
        assert!(lang.word_char('a'));
        assert!(lang.word_char('_'));
        assert!(!lang.word_char(' '));
    }

    #[test]
    fn syntax_snapshot_version_guard() {
        let snap = SyntaxSnapshot {
            buffer_version: 3,
            highlights: Arc::new(Vec::new()),
            highlight_store: Arc::new(HighlightStore::default()),
        };
        assert!(snap.is_current(3));
        assert!(!snap.is_current(4));
    }

    #[test]
    fn highlight_store_lazily_shifts_suffix() {
        let highlights = Arc::new(vec![
            Highlight {
                range: Range::new(10, 12),
                kind: "keyword".into(),
            },
            Highlight {
                range: Range::new(100, 110),
                kind: "identifier".into(),
            },
        ]);
        let mut store = HighlightStore::from_highlights(highlights, 200);
        store.apply_edit(Range::new(0, 0), 3, 203);
        let shifted = store.iter_intersecting(Range::new(0, 203));
        assert_eq!(shifted[0].range, Range::new(13, 15));
        assert_eq!(shifted[1].range, Range::new(103, 113));
    }

    #[test]
    fn highlight_store_drops_tokens_touched_by_edit() {
        let highlights = Arc::new(vec![Highlight {
            range: Range::new(10, 20),
            kind: "string".into(),
        }]);
        let mut store = HighlightStore::from_highlights(highlights, 30);
        store.apply_edit(Range::new(15, 15), 1, 31);
        assert!(store.iter_intersecting(Range::new(0, 31)).is_empty());
    }

    #[test]
    fn highlight_store_indexes_later_chunks_after_edit() {
        let highlights = Arc::new(
            (0..2048)
                .map(|index| Highlight {
                    range: Range::new(index * 4, index * 4 + 2),
                    kind: "identifier".into(),
                })
                .collect(),
        );
        let mut store = HighlightStore::from_highlights(highlights, 8192);
        store.apply_edit(Range::new(0, 0), 10, 8202);
        let visible = store.iter_intersecting(Range::new(4106, 4110));
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].range, Range::new(4106, 4108));
    }

    #[test]
    fn highlight_store_apply_range_replaces_dirty_highlights() {
        // 三处高亮：早于 dirty、相交 dirty、晚于 dirty。
        let highlights = Arc::new(vec![
            Highlight {
                range: Range::new(0, 4),
                kind: "kw".into(),
            },
            Highlight {
                range: Range::new(10, 14),
                kind: "old".into(),
            },
            Highlight {
                range: Range::new(50, 54),
                kind: "id".into(),
            },
        ]);
        let store = HighlightStore::from_highlights(highlights, 100);
        // dirty 区间 [8,20) 重解析：移除相交的 [10,14]，保留 [0,4] 与 [50,54]，
        // 并写入两个新高亮。
        let next = store.apply_range(
            Range::new(8, 20),
            &[
                Highlight {
                    range: Range::new(8, 11),
                    kind: "str".into(),
                },
                Highlight {
                    range: Range::new(14, 20),
                    kind: "str".into(),
                },
            ],
        );
        let mut out = next.iter_intersecting(Range::new(0, 100));
        out.sort_by_key(|h| h.range.start);
        let ranges: Vec<_> = out.iter().map(|h| (h.range, h.kind.as_ref())).collect();
        assert_eq!(
            ranges,
            vec![
                (Range::new(0, 4), "kw"),
                (Range::new(8, 11), "str"),
                (Range::new(14, 20), "str"),
                (Range::new(50, 54), "id"),
            ]
        );
    }

    #[test]
    fn highlight_store_apply_range_no_touch_rebuilds_suffix() {
        // >32*1024 条高亮组成多个叶子；对靠前的局部 dirty 区间做替换，末尾叶子必须 Arc 共享。
        let count = 40000usize;
        let highlights = Arc::new(
            (0..count)
                .map(|i| Highlight {
                    range: Range::new(i * 4, i * 4 + 2),
                    kind: "id".into(),
                })
                .collect(),
        );
        let store = HighlightStore::from_highlights(highlights, count * 4);
        let next = store.apply_range(
            Range::new(40, 48),
            &[Highlight {
                range: Range::new(40, 46),
                kind: "str".into(),
            }],
        );
        // 局部替换后，未触碰的尾部叶子仍与旧树共享（未全量重建）。
        assert!(store.shared_trailing_leaves(&next) > 0);
        // 内容正确：新高亮出现在 dirty 区间，其余保持。
        let visible = next.iter_intersecting(Range::new(36, 52));
        let mut starts: Vec<_> = visible.iter().map(|h| h.range.start).collect();
        starts.sort_unstable();
        assert_eq!(starts, vec![36, 40, 48]);
    }

    #[test]
    fn highlight_store_apply_range_pure_insert_no_intersection() {
        // dirty 区间落在高亮间隙（无相交叶子）时退化为纯插入。
        let highlights = Arc::new(vec![Highlight {
            range: Range::new(0, 4),
            kind: "a".into(),
        }]);
        let store = HighlightStore::from_highlights(highlights, 100);
        let next = store.apply_range(
            Range::new(20, 30),
            &[Highlight {
                range: Range::new(25, 30),
                kind: "b".into(),
            }],
        );
        let out = next.iter_intersecting(Range::new(0, 100));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn highlight_store_large_viewport_query_scales_with_viewport_only() {
        // DM-408：数万条高亮（多叶子 SumTree）下，viewport 查询只返回视口内命中，
        // 不随全文高亮总量线性放大——热路径工作量与可见 token 数同量级。
        let count = 60_000usize; // 60k 高亮 → 58 chunks → 多个叶子
        let highlights = Arc::new(
            (0..count)
                .map(|i| Highlight {
                    range: Range::new(i * 200, i * 200 + 8),
                    kind: "id".into(),
                })
                .collect(),
        );
        let store = HighlightStore::from_highlights(highlights, count * 200);

        // 视口聚焦到文档中部某个稀疏区（约命中 1~3 条）。
        let view = Range::new(50_000 * 200 - 1, 50_000 * 200 + 3 * 200);
        let hits = store.iter_intersecting(view);
        assert!(!hits.is_empty());
        assert!(
            hits.len() <= 4,
            "viewport 查询命中 {} 条，应只随视口 token 数增长",
            hits.len()
        );
        for h in &hits {
            assert!(h.range.start < view.end && h.range.end > view.start);
        }
    }

    #[test]
    fn highlight_store_large_local_edit_keeps_suffix_shared() {
        // DM-408：数万条高亮（多叶子）上对靠前做局部 dirty 替换，未触碰尾部叶子
        // 保持 Arc 共享（增量收敛，非全量重建），且替换后视口内容正确。
        let count = 60_000usize;
        let highlights = Arc::new(
            (0..count)
                .map(|i| Highlight {
                    range: Range::new(i * 4, i * 4 + 2),
                    kind: "id".into(),
                })
                .collect(),
        );
        let store = HighlightStore::from_highlights(highlights, count * 4);
        let next = store.apply_range(
            Range::new(40, 48),
            &[Highlight {
                range: Range::new(40, 46),
                kind: "str".into(),
            }],
        );
        assert!(
            store.shared_trailing_leaves(&next) > 0,
            "大 store 局部编辑必须保留尾部叶子 Arc 共享"
        );
        let visible = next.iter_intersecting(Range::new(36, 52));
        let mut starts: Vec<_> = visible.iter().map(|h| h.range.start).collect();
        starts.sort_unstable();
        assert_eq!(starts, vec![36, 40, 48]);
    }
}
