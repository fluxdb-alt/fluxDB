//! 版本化文本 buffer、只读 snapshot、局部 edit 与 undo/redo。
//!
//! - 文本以分块 rope 存储，输入路径只做局部 edit，禁止每次全文 `to_string()`。
//! - snapshot 与 buffer 共享不可变 chunk（Arc），可安全交给后台任务。
//! - 版本号单调递增；后台任务结果必须携带 version，旧结果直接丢弃。
//! - 行索引按 edit 区间增量维护。

use std::sync::Arc;

use crate::line_index::LineIndex;
use crate::model::{Edit, Offset, Point, Range, TextChange};
use crate::sum_tree::{SumTree, TextSummary};

/// 单个 chunk 的上限字节数。
const MAX_CHUNK: usize = 1024;

/// 分块 rope：以不可变字符串为 chunk 的文本容器。
///
/// 快照共享 `Arc<str>` chunk，因此复制一个快照是 O(chunk 数) 的引用计数操作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rope {
    // Snapshots share this vector; edits replace only the vector being mutated.
    chunks: Arc<Vec<Arc<str>>>,
    // Chunk start offsets make random point/range lookup logarithmic.
    chunk_starts: Arc<Vec<Offset>>,
    // Per-chunk summaries are aggregated by SumTree for byte/line/UTF-16 lookup.
    chunk_summaries: Arc<Vec<TextSummary>>,
    sum_tree: SumTree<TextSummary>,
    len: usize,
}

impl Default for Rope {
    fn default() -> Self {
        Self::new()
    }
}

impl Rope {
    fn new() -> Self {
        Self {
            chunks: Arc::new(Vec::new()),
            chunk_starts: Arc::new(vec![0]),
            chunk_summaries: Arc::new(Vec::new()),
            sum_tree: SumTree::default(),
            len: 0,
        }
    }

    fn from_str(text: &str) -> Self {
        let mut rope = Self::new();
        rope.push_str(text);
        rope
    }

    fn push_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let bytes = text.len();
        let base = self.len;
        let mut start = 0;
        while start < bytes {
            // 优先取 MAX_CHUNK 字节；若落在多字节字符中间，回退到上一个字符边界，
            // 保证每个 chunk 都以字符边界起止，快照切片与游标移动永不切到 UTF-8 中间。
            let mut end = (start + MAX_CHUNK).min(bytes);
            if end < bytes {
                while end > start && !text.is_char_boundary(end) {
                    end -= 1;
                }
                debug_assert!(end > start, "MAX_CHUNK 内应总能找到字符边界");
            }
            Arc::make_mut(&mut self.chunks).push(Arc::from(&text[start..end]));
            Arc::make_mut(&mut self.chunk_starts).push(base + end);
            Arc::make_mut(&mut self.chunk_summaries).push(summarize_chunk(&text[start..end]));
            start = end;
        }
        self.sum_tree = SumTree::from_summaries(&self.chunk_summaries);
        self.len += bytes;
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn len(&self) -> usize {
        self.len
    }

    /// 返回绝对偏移所属 chunk 的 `(index, within_offset)`。
    fn chunk_index_for_offset(&self, offset: Offset) -> (usize, usize) {
        if self.chunks.is_empty() {
            return (0, 0);
        }
        let (i, _) = self.sum_tree.locate_byte(offset);
        let i = i.min(self.chunks.len() - 1);
        (i, offset.saturating_sub(self.sum_tree.byte_start(i)))
    }

    /// 回退一个字符到其起始字节偏移。`offset` 可落在字符中间（对外会把此类损坏游标安全吸附）。
    ///
    /// 语义：若光标停在字符起始边界，则返回紧邻其前一个字符的起始；若光标落在字符中间，
    /// 则返回其所处字符的起始（吸附）。因 chunk 均以字符边界起止，单个字符不会跨 chunk。
    fn prev_char_boundary(&self, offset: Offset) -> Offset {
        if offset == 0 {
            return 0;
        }
        let (i, within) = self.chunk_index_for_offset(offset);
        if within == 0 {
            // offset 落在 chunk i 起始（= 字符边界）：前一个字符属于 chunk i-1。
            debug_assert!(i > 0, "offset>0 时 within==0 意味着 i>0");
            let c = &self.chunks[i - 1];
            let outer = self.chunks[i - 1].len();
            let start = floor_boundary_before(c, outer);
            return offset - (outer - start);
        }
        let c = &self.chunks[i];
        let start = if c.is_char_boundary(within) {
            // 严格落在 chunk 内的字符边界：回退到前一个字符的起始。
            floor_boundary_before(c, within)
        } else {
            // 落在字符中间：吸附到当前字符的起始。
            floor_boundary_at_or_before(c, within)
        };
        offset - (within - start)
    }

    /// 前进一个字符到其结束字节偏移（即下一个字符的起始）。`offset` 可落在字符中间（吸附到该字符末尾）。
    fn next_char_boundary(&self, offset: Offset) -> Offset {
        if offset >= self.len {
            return self.len;
        }
        let (i, within) = self.chunk_index_for_offset(offset);
        if within == self.chunks[i].len() {
            // offset 恰在 chunk 末尾（字符边界）：其下一个字符是 chunk i+1 的首字符。
            let nxt = &self.chunks[i + 1];
            let chlen = utf8_len_at(nxt, 0);
            return offset + chlen;
        }
        let c = &self.chunks[i];
        let cs = floor_boundary_at_or_before(c, within);
        let chlen = utf8_len_at(c, cs);
        offset - within + cs + chlen
    }

    /// 把 `[start, end)` 区间写入 target。
    fn append_range_to_string(&self, range: Range, target: &mut String) {
        if range.start >= range.end {
            return;
        }
        let clamped_start = range.start.min(self.len);
        let clamped_end = range.end.min(self.len);
        let (si, soff) = self.chunk_index_for_offset(clamped_start);
        let (ei, eoff) = self.chunk_index_for_offset(clamped_end);
        for (idx, chunk) in self.chunks[si..=ei].iter().enumerate() {
            let idx = idx + si;
            let start_in = if idx == si { soff } else { 0 };
            let end_in = if idx == ei { eoff } else { chunk.len() };
            // 防御：range 端点可能来自跨快照的旧 offset（如 IME 组合期间文档已变化），
            // 落到多字节字符中间会触发切片 panic。此处吸附到字符边界再切，绝不产出半字。
            let start_in = floor_boundary_at_or_before(chunk, start_in);
            let end_in = floor_boundary_at_or_before(chunk, end_in);
            if start_in < end_in {
                target.push_str(&chunk[start_in..end_in]);
            }
        }
    }

    fn to_string_slice(&self, range: Range) -> String {
        let mut out = String::with_capacity(range.end.saturating_sub(range.start));
        self.append_range_to_string(range, &mut out);
        out
    }

    fn append_to_string(&self, target: &mut String) {
        for chunk in self.chunks.iter() {
            target.push_str(chunk);
        }
    }

    /// 计算 `[0, offset)` 区间文本的 UTF-16 长度（`offset` 需为字符边界）。
    ///
    /// 只遍历到目标 offset 所在的 chunk，不构造全文字符串，用于输入法 / 光标换算。
    fn utf16_until(&self, offset: usize) -> usize {
        if self.chunks.is_empty() {
            return 0;
        }
        let offset = offset.min(self.len);
        let (ei, raw_eoff) = self.chunk_index_for_offset(offset);
        // IME 可能暂时携带跨快照的字节 offset；切片前统一吸附到字符边界。
        let eoff = floor_boundary_at_or_before(&self.chunks[ei], raw_eoff);
        let mut total = self.sum_tree.summary_before_leaf(ei).utf16;
        if eoff > 0 {
            total += self.chunks[ei][..eoff]
                .chars()
                .map(|c| c.len_utf16() as usize)
                .sum::<usize>();
        }
        total
    }

    /// 给定文档级 UTF-16 offset，返回其对应的字符边界字节 offset；越界返回 `None`。
    ///
    /// 逐 chunk 累加 UTF-16 长度定位目标，仅在目标所在 chunk 内扫描字符，不构造全文。
    fn utf16_to_byte(&self, utf16: usize) -> Option<usize> {
        if self.chunks.is_empty() {
            return (utf16 == 0).then_some(0);
        }
        if utf16 > self.sum_tree.total().utf16 {
            return None;
        }
        let (i, within) = self.sum_tree.locate_utf16(utf16);
        let mut byte = 0usize;
        let mut count = 0usize;
        for c in self.chunks[i].chars() {
            if count >= within {
                break;
            }
            count += c.len_utf16();
            byte += c.len_utf8();
        }
        Some(self.chunk_starts[i] + byte)
    }

    /// 用 `new_text` 替换 `old_range` 区间，返回被替换的旧文本。
    ///
    /// 做到「真局部编辑」：只重建被编辑区间跨越的 chunk 段（起点 chunk 的前缀 +
    /// `new_text` + 终点 chunk 的后缀），起点之前与终点之后的 chunk 通过 `Arc` 共享原样保留，
    /// 不再像旧实现那样把整个后缀 `append_range_to_string` 全文拷贝。
    fn replace(&mut self, old_range: Range, new_text: &str) -> String {
        let old_len = self.len;
        let old_text = self.to_string_slice(old_range);
        if self.chunks.is_empty() {
            self.push_str(new_text);
            return old_text;
        }
        let start = old_range.start.min(old_len);
        let end = old_range.end.min(old_len).max(start);
        let (si, soff) = self.chunk_index_for_offset(start);
        let (ei, eoff) = self.chunk_index_for_offset(end);

        // 防御：`old_range` 端点可能来自跨快照的旧 offset（如 IME 组合期间文档已变化），
        // 落到多字节字符中间会触发切片 panic。此处吸附到字符边界再切，绝不产出半字。
        let soff = floor_boundary_at_or_before(&self.chunks[si], soff);
        let eoff = floor_boundary_at_or_before(&self.chunks[ei], eoff);

        // 起点之前的所有 chunk 原样共享。
        let mut kept: Vec<Arc<str>> = self.chunks[..si].to_vec();

        // 拼接被编辑区间跨越的段：起点 chunk 前缀 + new_text + 终点 chunk 后缀。
        let mut splice = String::with_capacity(new_text.len() + MAX_CHUNK * 2);
        if soff > 0 {
            splice.push_str(&self.chunks[si][..soff]);
        }
        splice.push_str(new_text);
        if ei < self.chunks.len() && eoff < self.chunks[ei].len() {
            splice.push_str(&self.chunks[ei][eoff..]);
        }
        // 分块追加（字符边界安全）。
        let mut splice_start = 0;
        let mut local_summaries = Vec::new();
        let bytes = splice.len();
        while splice_start < bytes {
            let mut splice_end = (splice_start + MAX_CHUNK).min(bytes);
            if splice_end < bytes {
                while splice_end > splice_start && !splice.is_char_boundary(splice_end) {
                    splice_end -= 1;
                }
            }
            let local_chunk = &splice[splice_start..splice_end];
            kept.push(Arc::from(local_chunk));
            local_summaries.push(summarize_chunk(local_chunk));
            splice_start = splice_end;
        }
        // 终点之后的 chunk 原样共享，不做全文拷贝。
        kept.extend_from_slice(&self.chunks[ei + 1..]);

        // 索引只拼接受影响的局部段；后缀 chunk 的字节起点整体平移。
        let byte_delta = signed_delta(new_text.len(), end - start);
        let new_len = old_len.saturating_sub(end - start) + new_text.len();
        let local_chunk_count = kept
            .len()
            .saturating_sub(si + self.chunks.len().saturating_sub(ei + 1));
        let mut starts = Vec::with_capacity(kept.len() + 1);
        starts.extend_from_slice(&self.chunk_starts[..si]);
        let mut offset = self.chunk_starts[si];
        for chunk in kept.iter().skip(si).take(local_chunk_count) {
            starts.push(offset);
            offset += chunk.len();
        }
        for old_index in (ei + 1)..self.chunks.len() {
            starts.push(shifted_offset(self.chunk_starts[old_index], byte_delta));
        }
        starts.push(new_len);

        debug_assert_eq!(starts.len(), kept.len() + 1);
        self.chunks = Arc::new(kept);
        self.chunk_starts = Arc::new(starts);
        let mut summaries = self.chunk_summaries[..si].to_vec();
        summaries.extend_from_slice(&local_summaries);
        summaries.extend_from_slice(&self.chunk_summaries[ei + 1..]);
        self.chunk_summaries = Arc::new(summaries);
        self.sum_tree = self.sum_tree.replace_leaves(si, ei + 1, &local_summaries);
        self.len = new_len;
        old_text
    }
}

/// 一段已完成的编辑，用于 undo/redo。
#[derive(Clone, Debug)]
struct EditRecord {
    /// undo 时用 old_text 替换的新区间。
    new_range: Range,
    old_text: String,
    new_text: String,
}

/// 一个 undo/redo 事务。
#[derive(Clone, Debug)]
struct EditTransaction {
    records: Vec<EditRecord>,
    mergeable: bool,
    before_anchor: Offset,
    before_cursor: Offset,
    after_anchor: Offset,
    after_cursor: Offset,
}

/// 版本化编辑器 buffer。
#[derive(Debug)]
pub struct EditorBuffer {
    text: Rope,
    version: u64,
    line_starts: LineIndex,
    undo_stack: Vec<EditTransaction>,
    redo_stack: Vec<EditTransaction>,
    undo_limit: usize,
}

impl Default for EditorBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorBuffer {
    pub fn new() -> Self {
        Self::new_from("")
    }

    pub fn new_from(text: &str) -> Self {
        let mut buffer = Self {
            text: Rope::from_str(text),
            version: 0,
            line_starts: LineIndex::from_offsets(&[0]),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            undo_limit: 100,
        };
        buffer.rebuild_line_starts();
        buffer
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 当前全文（仅测试或宿主确实需要时使用）。
    pub fn to_string(&self) -> String {
        let mut s = String::with_capacity(self.text.len());
        self.text.append_to_string(&mut s);
        s
    }

    pub fn text_in_range(&self, range: Range) -> String {
        self.text.to_string_slice(range)
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Materialize line starts when a caller needs the complete index; cursor paths query the tree.
    pub fn line_starts(&self) -> Vec<Offset> {
        self.line_starts.collect()
    }

    fn line_start_offset(&self, row: usize) -> Offset {
        self.line_starts.offset_at(row)
    }

    fn line_of_offset(&self, offset: Offset) -> usize {
        let clamped = offset.min(self.text.len());
        self.line_starts.row_for_offset(clamped)
    }

    pub fn offset_to_point(&self, offset: Offset) -> Point {
        let clamped = offset.min(self.text.len());
        let row = self.line_of_offset(clamped);
        Point::new(row, clamped - self.line_start_offset(row))
    }

    pub fn point_to_offset(&self, point: Point) -> Offset {
        let row = point.row.min(self.line_starts.len().saturating_sub(1));
        let line_start = self.line_start_offset(row);
        (line_start + point.column).min(self.text.len())
    }

    /// 回退一个字符的起始字节偏移（字符边界安全）。
    pub fn prev_char_boundary(&self, offset: Offset) -> Offset {
        self.text.prev_char_boundary(offset.min(self.text.len()))
    }

    /// 前进一个字符的结束字节偏移（字符边界安全）。
    pub fn next_char_boundary(&self, offset: Offset) -> Offset {
        self.text.next_char_boundary(offset.min(self.text.len()))
    }

    /// 返回 `<= offset` 的最大字符边界；若 `offset` 已是字符边界则原样返回（跨快照吸附用）。
    pub fn clamp_to_char_boundary(&self, offset: Offset) -> Offset {
        self.snapshot().clamp_to_char_boundary(offset)
    }

    /// 第 `row` 行文本（不含换行符）。
    pub fn line_text(&self, row: usize) -> String {
        let start = self.line_start_offset(row);
        let raw_end = if row + 1 < self.line_starts.len() {
            self.line_starts.offset_at(row + 1)
        } else {
            self.text.len()
        };
        let end = strip_trailing_newline(&self.text, raw_end);
        self.text.to_string_slice(Range::new(start, end))
    }

    /// 返回第 `row` 行的行尾偏移（含换行符）。若末行无换行则返回文本末尾。
    pub fn line_end_offset(&self, row: usize) -> Offset {
        if row + 1 < self.line_starts.len() {
            self.line_starts.offset_at(row + 1)
        } else {
            self.text.len()
        }
    }

    /// 返回第 `row` 行的起始偏移。`row` 超出范围时收敛到末行。
    pub fn line_start(&self, row: usize) -> Offset {
        self.line_start_offset(row.min(self.line_starts.len().saturating_sub(1)))
    }

    fn rebuild_line_starts(&mut self) {
        let mut line_starts = Vec::with_capacity(1024);
        line_starts.push(0);
        let mut offset = 0;
        for chunk in self.text.chunks.iter() {
            for (i, b) in chunk.bytes().enumerate() {
                if b == b'\n' {
                    line_starts.push(offset + i + 1);
                }
            }
            offset += chunk.len();
        }
        self.line_starts = LineIndex::from_offsets(&line_starts);
    }

    /// 执行一次替换。
    ///
    /// `new_cursor`/`new_anchor` 为宿主期望的编辑后光标/锚点位置（按新文本偏移给出）。
    /// `merge` 为 true 时把本次变更并入上一个事务，使连续输入可一次撤销。
    pub fn edit(
        &mut self,
        old_range: Range,
        new_text: &str,
        new_cursor: Offset,
        new_anchor: Offset,
        merge: bool,
    ) -> Edit {
        let old_range = old_range.sorted();
        self.edit_with_selection(
            old_range,
            new_text,
            new_cursor,
            new_anchor,
            old_range.start,
            old_range.end,
            merge,
        )
    }

    /// 执行替换并记录编辑前后的选区，供 undo/redo 恢复光标位置。
    pub fn edit_with_selection(
        &mut self,
        old_range: Range,
        new_text: &str,
        new_cursor: Offset,
        new_anchor: Offset,
        before_anchor: Offset,
        before_cursor: Offset,
        merge: bool,
    ) -> Edit {
        let old_range = old_range.sorted();
        // 应用文本替换，拿到被替换的旧文本。
        let old_text = self.text.replace(old_range, new_text);
        // 增量更新行起始索引。
        self.update_line_starts(old_range, new_text);

        // 记录 undo 事务。
        let record = EditRecord {
            new_range: Range::new(old_range.start, old_range.start + new_text.len()),
            old_text,
            new_text: new_text.to_string(),
        };
        let can_merge = merge
            && self
                .undo_stack
                .last()
                .map(|top| top.mergeable)
                .unwrap_or(false);
        if can_merge {
            if let Some(top) = self.undo_stack.last_mut() {
                top.records.push(record);
                top.after_anchor = new_anchor;
                top.after_cursor = new_cursor;
            }
        } else {
            self.undo_stack.push(EditTransaction {
                records: vec![record],
                mergeable: merge,
                before_anchor,
                before_cursor,
                after_anchor: new_anchor,
                after_cursor: new_cursor,
            });
        }

        self.finish_edit();
        Edit {
            // 返回本 edit 的增量变更（供宿主映射 Changed 事件）。
            changes: vec![TextChange {
                old_range,
                new_text: new_text.to_string(),
                version: self.version,
                full_document: false,
            }],
            cursor: new_cursor,
            anchor: new_anchor,
        }
    }

    fn finish_edit(&mut self) {
        if self.undo_stack.len() > self.undo_limit {
            self.undo_stack.remove(0);
        }
        if !self.redo_stack.is_empty() {
            self.redo_stack.clear();
        }
        self.version += 1;
    }

    /// 增量更新行起始索引：受影响行之前保持不变，之后重算。
    fn update_line_starts(&mut self, old_range: Range, new_text: &str) {
        self.line_starts.splice(old_range, new_text);
    }

    /// 撤销最近一次事务。
    pub fn undo(&mut self) -> Option<(Offset, Offset)> {
        let tx = self.undo_stack.pop()?;
        let selection = (tx.before_anchor, tx.before_cursor);
        for rec in tx.records.iter().rev() {
            let new_range = Range::new(
                rec.new_range.start,
                rec.new_range.start + rec.new_text.len(),
            );
            self.text.replace(new_range, &rec.old_text);
        }
        self.rebuild_line_starts();
        self.version += 1;
        self.redo_stack.push(tx);
        Some(selection)
    }

    /// 重做最近一次撤销。
    pub fn redo(&mut self) -> Option<(Offset, Offset)> {
        let tx = self.redo_stack.pop()?;
        let selection = (tx.after_anchor, tx.after_cursor);
        for rec in &tx.records {
            self.text.replace(rec.new_range, &rec.new_text);
        }
        self.rebuild_line_starts();
        self.version += 1;
        self.undo_stack.push(tx);
        Some(selection)
    }

    /// UTF-16 列。
    pub fn utf16_column_at(&self, point: Point) -> usize {
        let line = self.text.to_string_slice(Range::new(
            self.line_start_offset(point.row),
            self.point_to_offset(point),
        ));
        count_utf16(&line)
    }

    /// 当前快照（共享 chunk，后台安全）。
    pub fn snapshot(&self) -> BufferSnapshot {
        BufferSnapshot {
            rope: self.text.clone(),
            version: self.version,
            line_starts: self.line_starts.clone(),
        }
    }
}

/// 去掉行尾换行符（含 `\r\n`），使行内容不把 `\r` 计入显示列。
fn strip_trailing_newline(rope: &Rope, end: usize) -> usize {
    let mut end = end;
    if end > 0 && rope.to_string_slice(Range::new(end - 1, end)) == "\n" {
        end -= 1;
    }
    if end > 0 && rope.to_string_slice(Range::new(end - 1, end)) == "\r" {
        end -= 1;
    }
    end
}

fn count_utf16(s: &str) -> usize {
    s.chars().map(|c| c.len_utf16()).sum()
}

fn summarize_chunk(text: &str) -> TextSummary {
    TextSummary {
        bytes: text.len(),
        lines: text.bytes().filter(|byte| *byte == b'\n').count(),
        utf16: count_utf16(text),
    }
}

fn shifted_offset(offset: Offset, delta: isize) -> Offset {
    if delta >= 0 {
        offset.saturating_add(delta as usize)
    } else {
        offset.saturating_sub((-delta) as usize)
    }
}

fn signed_delta(new_value: usize, old_value: usize) -> isize {
    if new_value >= old_value {
        (new_value - old_value) as isize
    } else {
        -((old_value - new_value) as isize)
    }
}

/// `bytes[..upper)` 内最后一个字符边界（strictly < `upper`）。`upper>0` 时必有解。
fn floor_boundary_before(bytes: &str, upper: usize) -> usize {
    debug_assert!(upper > 0);
    (0..upper)
        .rev()
        .find(|&i| bytes.is_char_boundary(i))
        .unwrap_or(0)
}

/// `bytes` 内 `<= idx` 的最大字符边界。
fn floor_boundary_at_or_before(bytes: &str, idx: usize) -> usize {
    let idx = idx.min(bytes.len());
    let mut i = idx;
    while i > 0 && !bytes.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `bytes` 从 `cs`（必为字符边界）起始的第一个字符的 UTF-8 字节长。
fn utf8_len_at(bytes: &str, cs: usize) -> usize {
    bytes[cs..]
        .chars()
        .next()
        .map(|c| c.len_utf8())
        .unwrap_or(0)
}

/// 只读 buffer 快照。克隆廉价，可安全交给后台任务。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferSnapshot {
    rope: Rope,
    version: u64,
    line_starts: LineIndex,
}

impl BufferSnapshot {
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len(&self) -> usize {
        self.rope.len()
    }

    /// 文档摘要：bytes、换行数和 UTF-16 码元数均由 SumTree 聚合得到。
    /// `lines` 是换行符数量；逻辑行数仍为 `lines + 1`（空文档也有一行）。
    pub fn summary(&self) -> TextSummary {
        self.rope.sum_tree.total()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.is_empty()
    }

    pub fn to_string(&self) -> String {
        let mut s = String::with_capacity(self.rope.len());
        self.rope.append_to_string(&mut s);
        s
    }

    pub fn text_in_range(&self, range: Range) -> String {
        self.rope.to_string_slice(range)
    }

    /// 返回从精确字节 offset 开始的当前 Rope chunk 只读字节片段。
    ///
    /// 解析器可能从 UTF-8 字符中间请求输入，因此这里不能吸附字符边界；
    /// 只返回当前 chunk 的原始字节，不拼接也不复制后续文本。
    pub fn text_chunk_bytes_at(&self, offset: Offset) -> &[u8] {
        if offset >= self.rope.len() || self.rope.chunks.is_empty() {
            return &[];
        }
        let (index, within) = self.rope.chunk_index_for_offset(offset);
        self.rope.chunks[index]
            .as_bytes()
            .get(within..)
            .unwrap_or(&[])
    }

    /// 读取单个 UTF-8 原始字节，不创建临时字符串。
    pub fn byte_at(&self, offset: Offset) -> Option<u8> {
        self.text_chunk_bytes_at(offset).first().copied()
    }

    /// 计算范围内 UTF-16 码元数量，不拼接中间字符串。
    pub fn utf16_len_in_range(&self, range: Range) -> usize {
        self.text_chunks_in_range(range)
            .filter_map(|chunk| std::str::from_utf8(chunk).ok())
            .map(|text| text.chars().map(char::len_utf16).sum::<usize>())
            .sum()
    }

    /// 以零拷贝方式遍历指定字节范围覆盖的 Rope chunks。
    ///
    /// 迭代器只借用快照和底层 `Arc<str>`，不会把跨 chunk 文本拼成新的 `String`；
    /// Tree-sitter 的 `TextProvider` 可直接使用它作为节点文本来源。
    pub fn text_chunks_in_range(&self, range: Range) -> SnapshotChunkIter<'_> {
        let start = range.start.min(self.rope.len());
        let end = range.end.min(self.rope.len()).max(start);
        let first = if start < self.rope.len() {
            self.rope.chunk_index_for_offset(start).0
        } else {
            self.rope.chunks.len()
        };
        SnapshotChunkIter {
            chunks: &self.rope.chunks,
            starts: &self.rope.chunk_starts,
            index: first,
            start,
            end,
        }
    }

    pub fn offset_to_point(&self, offset: Offset) -> Point {
        let clamped = offset.min(self.rope.len());
        let row = self.line_starts.row_for_offset(clamped);
        let line_start = self.line_starts.offset_at(row);
        Point::new(row, clamped - line_start)
    }

    pub fn point_to_offset(&self, point: Point) -> Offset {
        let row = point.row.min(self.line_starts.len().saturating_sub(1));
        let line_start = self.line_starts.offset_at(row);
        (line_start + point.column).min(self.rope.len())
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Materialize line starts when a caller needs the complete index; cursor paths query the tree.
    pub fn line_starts(&self) -> Vec<Offset> {
        self.line_starts.collect()
    }

    pub fn line_start(&self, row: usize) -> Offset {
        self.line_starts.offset_at(row)
    }

    /// 第 `row` 行的行尾偏移（含换行符）；末行无换行则返回文本末尾。
    pub fn line_end_offset(&self, row: usize) -> Offset {
        if row + 1 < self.line_starts.len() {
            self.line_starts.offset_at(row + 1)
        } else {
            self.rope.len()
        }
    }

    /// 第 `row` 行文本（不含换行符）。
    pub fn line_text(&self, row: usize) -> String {
        let start = self.line_start(row);
        let raw_end = if row + 1 < self.line_starts.len() {
            self.line_starts.offset_at(row + 1)
        } else {
            self.rope.len()
        };
        let end = strip_trailing_newline(&self.rope, raw_end);
        self.text_in_range(Range::new(start, end))
    }

    pub fn utf16_column_at(&self, point: Point) -> usize {
        let line = self.text_in_range(Range::new(
            self.line_start(point.row),
            self.point_to_offset(point),
        ));
        count_utf16(&line)
    }

    /// 文档级字节 offset → 文档级 UTF-16 offset（`offset` 需为字符边界）。
    ///
    /// 只遍历到目标行 / chunk，不构造全文字符串；供输入法（IME）的 UTF-16 换算使用。
    pub fn byte_to_utf16(&self, offset: usize) -> usize {
        self.rope.utf16_until(offset)
    }

    /// 文档级 UTF-16 offset → 字符边界字节 offset；越界返回 `None`。
    pub fn utf16_to_byte(&self, utf16: usize) -> Option<Offset> {
        self.rope.utf16_to_byte(utf16)
    }

    /// 沿文本方向推进一个字节偏移，遇到非 char 边界时对齐。
    pub fn clamp_offset(&self, offset: Offset) -> Offset {
        offset.min(self.rope.len())
    }

    /// 返回 `<= offset` 的最大字符边界；若 `offset` 已是字符边界则原样返回。
    ///
    /// 用于把「跨快照的旧字节 offset」（如 IME 组合期间文档已变化而保留的 marked 区间）
    /// 吸附到当前文档的合法位置，避免落到多字节字符中间触发切片 panic。
    pub fn clamp_to_char_boundary(&self, offset: Offset) -> Offset {
        let offset = offset.min(self.rope.len());
        if self.rope.len() == 0 {
            return 0;
        }
        let (i, within) = self.rope.chunk_index_for_offset(offset);
        let snapped = floor_boundary_at_or_before(&self.rope.chunks[i], within);
        // 还原为绝对 offset：`within` 是相对 chunk 的偏移，`offset - within` 是 chunk 起始绝对偏移。
        offset - within + snapped
    }

    /// 回退一个字符的起始字节偏移（字符边界安全）。
    pub fn prev_char_boundary(&self, offset: Offset) -> Offset {
        self.rope.prev_char_boundary(offset.min(self.rope.len()))
    }

    /// 前进一个字符的结束字节偏移（字符边界安全）。
    pub fn next_char_boundary(&self, offset: Offset) -> Offset {
        self.rope.next_char_boundary(offset.min(self.rope.len()))
    }
}

/// `BufferSnapshot` 范围的零拷贝 chunk 迭代器。
pub struct SnapshotChunkIter<'a> {
    chunks: &'a [Arc<str>],
    starts: &'a [Offset],
    index: usize,
    start: Offset,
    end: Offset,
}

impl<'a> Iterator for SnapshotChunkIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.chunks.len() {
            let index = self.index;
            self.index += 1;
            let chunk_start = self.starts[index];
            let chunk_end = self.starts[index + 1];
            if chunk_end <= self.start || chunk_start >= self.end {
                continue;
            }
            let start = floor_boundary_at_or_before(
                &self.chunks[index],
                self.start
                    .saturating_sub(chunk_start)
                    .min(self.chunks[index].len()),
            );
            let end = floor_boundary_at_or_before(
                &self.chunks[index],
                self.end
                    .saturating_sub(chunk_start)
                    .min(self.chunks[index].len()),
            );
            if start < end {
                return Some(&self.chunks[index].as_bytes()[start..end]);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rope_edit_and_text() {
        let mut rope = Rope::from_str("hello world");
        rope.replace(Range::new(0, 5), "HELLO");
        let mut s = String::new();
        rope.append_to_string(&mut s);
        assert_eq!(s, "HELLO world");
        assert_eq!(rope.len(), 11);
    }

    #[test]
    fn rope_middle_insert_keeps_suffix() {
        let mut rope = Rope::from_str("abcdef");
        rope.replace(Range::new(2, 3), "XY");
        assert_eq!(rope.to_string_slice(Range::new(0, rope.len())), "abXYdef");
    }

    #[test]
    fn sum_tree_summary_tracks_local_edits() {
        let mut buffer = EditorBuffer::new_from("a\n界🙂");
        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.summary().bytes, snapshot.len());
        assert_eq!(snapshot.summary().lines, 1);
        assert_eq!(
            snapshot.summary().utf16,
            snapshot.to_string().encode_utf16().count()
        );

        buffer.edit(Range::new(1, 2), "\r\n\n", 1, 1, false);
        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.summary().bytes, snapshot.len());
        assert_eq!(snapshot.summary().lines, 2);
        assert_eq!(
            snapshot.summary().utf16,
            snapshot.to_string().encode_utf16().count()
        );
    }

    #[test]
    fn buffer_undo_redo() {
        let mut buf = EditorBuffer::new_from("abc");
        buf.edit(Range::new(3, 3), "123", 6, 6, false);
        assert_eq!(buf.to_string(), "abc123");
        buf.undo().unwrap();
        assert_eq!(buf.to_string(), "abc");
        buf.redo().unwrap();
        assert_eq!(buf.to_string(), "abc123");
    }

    #[test]
    fn point_offset_roundtrip() {
        let buf = EditorBuffer::new_from("ab\ncd\n");
        assert_eq!(buf.offset_to_point(0), Point::new(0, 0));
        assert_eq!(buf.offset_to_point(1), Point::new(0, 1));
        assert_eq!(buf.offset_to_point(3), Point::new(1, 0));
        assert_eq!(buf.offset_to_point(5), Point::new(1, 2));
        assert_eq!(buf.point_to_offset(Point::new(1, 1)), 4);
        assert_eq!(buf.line_count(), 3);
    }

    #[test]
    fn snapshot_isolation() {
        let mut buf = EditorBuffer::new_from("abc");
        let snap = buf.snapshot();
        buf.edit(Range::new(0, 0), "x", 1, 1, false);
        assert_eq!(snap.to_string(), "abc");
        assert_eq!(buf.to_string(), "xabc");
        assert_eq!(snap.version(), 0);
        assert_eq!(buf.version(), 1);
    }

    #[test]
    fn snapshot_chunk_view_reads_without_joining_chunks() {
        let text = "a".repeat(1024) + "尾部";
        let snapshot = EditorBuffer::new_from(&text).snapshot();
        assert_eq!(snapshot.text_chunk_bytes_at(0), &text.as_bytes()[..1024]);
        assert_eq!(snapshot.text_chunk_bytes_at(1024), "尾部".as_bytes());
        assert_eq!(snapshot.text_chunk_bytes_at(snapshot.len()), &[]);
    }

    #[test]
    fn snapshot_range_chunks_cover_requested_bytes() {
        let text = "a".repeat(2048) + "尾部";
        let snapshot = EditorBuffer::new_from(&text).snapshot();
        let range = Range::new(1000, snapshot.len());
        let chunks = snapshot
            .text_chunks_in_range(range)
            .flat_map(|chunk| chunk.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(chunks, text.as_bytes()[range.start..range.end]);
    }

    #[test]
    fn snapshot_range_chunks_snap_mid_utf8_offsets_safely() {
        let text = "a尾部";
        let snapshot = EditorBuffer::new_from(text).snapshot();
        let range = Range::new(2, snapshot.len());
        let chunks = snapshot
            .text_chunks_in_range(range)
            .flat_map(|chunk| chunk.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(chunks, snapshot.text_in_range(range).as_bytes());
    }

    #[test]
    fn snapshot_byte_at_reads_without_materializing_text() {
        let snapshot = EditorBuffer::new_from("ab\n尾").snapshot();
        assert_eq!(snapshot.byte_at(2), Some(b'\n'));
        assert_eq!(snapshot.byte_at(snapshot.len()), None);
    }

    #[test]
    fn snapshot_utf16_len_reads_chunk_ranges() {
        let snapshot = EditorBuffer::new_from("a😀尾").snapshot();
        assert_eq!(
            snapshot.utf16_len_in_range(Range::new(0, snapshot.len())),
            4
        );
        assert_eq!(snapshot.utf16_len_in_range(Range::new(1, 5)), 2);
    }

    #[test]
    fn utf16_column() {
        let buf = EditorBuffer::new_from("a😀b");
        assert_eq!(buf.utf16_column_at(Point::new(0, 1)), 1);
        assert_eq!(buf.utf16_column_at(Point::new(0, 5)), 3);
        assert_eq!(buf.utf16_column_at(Point::new(0, 6)), 4);
    }

    #[test]
    fn multiline_edit_updates_line_index() {
        let mut buf = EditorBuffer::new_from("one\ntwo\nthree\n");
        assert_eq!(buf.line_count(), 4);
        buf.edit(Range::new(4, 7), "TWO\n2", 6, 6, false);
        assert_eq!(buf.to_string(), "one\nTWO\n2\nthree\n");
        assert_eq!(buf.line_count(), 5);
        assert_eq!(buf.offset_to_point(4), Point::new(1, 0));
        assert_eq!(buf.offset_to_point(9), Point::new(2, 1));
    }

    #[test]
    fn edit_returns_cursor() {
        let mut buf = EditorBuffer::new_from("hello");
        let edit = buf.edit(Range::new(5, 5), " world", 11, 11, false);
        assert_eq!(edit.cursor, 11);
        assert_eq!(buf.to_string(), "hello world");
    }

    /// 退格 / 删除键依赖的「删除单个字符区间」语义：
    /// 把光标展开成 [前一字符, 光标] 或 [光标, 后一字符] 选区后，用空文本替换即可整字删除，
    /// 并把光标/锚点收敛到区间起点。此行为与编辑器组件内的 backspace/delete 保持一致。
    #[test]
    fn delete_one_char_range_via_empty_edit() {
        // 退格：删除 [start-1, start]，光标落在 start-1。
        let mut buf = EditorBuffer::new_from("ab😀c");
        let start = 2; // 光标在 "b" 之后、"😀" 之前，退格删除 "b"
        let prev = buf.prev_char_boundary(start);
        assert_eq!(prev, 1);
        let edit = buf.edit(Range::new(prev, start), "", prev, prev, false);
        assert_eq!(buf.to_string(), "a😀c");
        assert_eq!(edit.cursor, prev);
        assert_eq!(edit.anchor, prev);

        // 前进删除：光标在 "a" 之后，删除其后的一个字符 "😀"，光标保持在 start。
        let start = 1;
        let next = buf.next_char_boundary(start);
        assert_eq!(next, 5);
        let edit = buf.edit(Range::new(start, next), "", 1, 1, false);
        assert_eq!(buf.to_string(), "ac");
        assert_eq!(edit.cursor, 1);
    }

    #[test]
    fn line_text() {
        let buf = EditorBuffer::new_from("ab\ncde\n");
        assert_eq!(buf.line_text(0), "ab");
        assert_eq!(buf.line_text(1), "cde");
        assert_eq!(buf.line_text(2), "");
    }

    #[test]
    fn multiline_paste_keeps_line_starts_ordered() {
        let text = "/*!40111 SET @OLD_SQL_NOTES=@@SQL_NOTES, SQL_NOTES=0 */;\n\n# Dump of table 3d_application_info\n# ----------------------------------\n\nCREATE TABLE `3d_application_info` (\n  `id` varchar(64) NOT NULL,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB;\n\n/*!40111 SET @OLD_SQL_NOTES=@@SQL_NOTES, SQL_NOTES=0 */;\n# Dump of table 3d_application_info\n# ----------------------------------\n";
        let mut buf = EditorBuffer::new_from("");
        buf.edit(Range::new(0, 0), text, text.len(), text.len(), false);

        assert_eq!(buf.to_string(), text);
        let line_starts = buf.line_starts.collect();
        assert!(line_starts.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(buf.line_count(), text.split('\n').count());
        for (row, expected) in text.split('\n').enumerate() {
            assert_eq!(buf.line_text(row), expected);
        }

        let prefix = "/*!40111 SET @OLD_SQL_NOTES=@@SQL_NOTES, SQL_NOTES=0 */;\n";
        let tail = "# Dump of table 3d_application_info\n# ----------------------------------\n";
        let mut appended = EditorBuffer::new_from(prefix);
        let end = appended.len();
        appended.edit(
            Range::new(end, end),
            tail,
            end + tail.len(),
            end + tail.len(),
            false,
        );
        let combined = format!("{prefix}{tail}");
        assert_eq!(appended.to_string(), combined);
        assert!(
            appended
                .line_starts
                .collect()
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert_eq!(appended.line_count(), combined.split('\n').count());
        for (row, expected) in combined.split('\n').enumerate() {
            assert_eq!(appended.line_text(row), expected);
        }
    }

    #[test]
    fn lazy_line_index_matches_reference_after_random_edits() {
        let mut seed = 0x1234_5678_u64;
        let mut text = String::from("head\nbody\ntail\n");
        let mut buffer = EditorBuffer::new_from(&text);
        for _ in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let start = (seed as usize) % (text.len() + 1);
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let end = start + ((seed as usize) % (text.len() - start + 1));
            let replacement = match seed % 4 {
                0 => "x",
                1 => "\nline\n",
                2 => "",
                _ => "abc\n",
            };
            text.replace_range(start..end, replacement);
            buffer.edit(
                Range::new(start, end),
                replacement,
                start + replacement.len(),
                start + replacement.len(),
                false,
            );
            let expected = std::iter::once(0)
                .chain(
                    text.bytes()
                        .enumerate()
                        .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
                )
                .collect::<Vec<_>>();
            assert_eq!(buffer.line_starts(), expected);
            assert_eq!(buffer.to_string(), text);
        }
    }

    #[test]
    fn adjacent_typing_merges_into_one_undo() {
        let mut buf = EditorBuffer::new_from("");
        buf.edit(Range::new(0, 0), "a", 1, 1, true);
        buf.edit(Range::new(1, 1), "b", 2, 2, true);
        assert_eq!(buf.to_string(), "ab");
        buf.undo().unwrap();
        assert_eq!(buf.to_string(), "");
    }

    #[test]
    fn large_text_no_full_parse_in_edit() {
        // 生成 10 万行文本，验证编辑不会退化为退化行为且结果正确。
        let mut text = String::new();
        for i in 0..100_000 {
            text.push_str(&format!("line {i}\n"));
        }
        let mut buf = EditorBuffer::new_from(&text);
        // 在末尾追加。
        buf.edit(
            Range::new(buf.len(), buf.len()),
            "tail",
            buf.len() + 4,
            buf.len() + 4,
            false,
        );
        assert!(buf.to_string().ends_with("line 99999\ntail"));
        assert_eq!(buf.line_count(), 100_001);
    }

    #[test]
    #[ignore = "manual large-document benchmark"]
    fn ten_mb_snapshot_and_tail_edit_benchmark() {
        use std::time::Instant;

        let line = "SELECT id, name FROM users WHERE id = 42;\n";
        let text = line.repeat(10 * 1024 * 1024 / line.len() + 1);
        let started = Instant::now();
        let mut buffer = EditorBuffer::new_from(&text);
        let snapshot = buffer.snapshot();
        let snapshot_ms = started.elapsed().as_secs_f64() * 1000.0;

        let edit_started = Instant::now();
        let end = buffer.len();
        buffer.edit(
            Range::new(end, end),
            "SELECT 1;\n",
            end + 10,
            end + 10,
            false,
        );
        let edit_ms = edit_started.elapsed().as_secs_f64() * 1000.0;

        eprintln!(
            "10MB editor benchmark: bytes={} lines={} snapshot_ms={snapshot_ms:.3} edit_ms={edit_ms:.3}",
            snapshot.len(),
            snapshot.line_count(),
        );
        assert_eq!(snapshot.version(), 0);
        assert_eq!(buffer.version(), 1);
        assert_eq!(buffer.line_count(), snapshot.line_count() + 1);
        assert!(buffer.to_string().ends_with("SELECT 1;\n"));
    }

    #[test]
    fn chunking_never_splits_multibyte_char() {
        // 中文单字 3 字节、emoji 4 字节：填满超过一个 MAX_CHUNK，断言任何 chunk 切片都在字符边界。
        let mut rope = Rope::new();
        let chars: String = "中".repeat(500) + &"😀".repeat(400);
        rope.push_str(&chars);
        assert_eq!(rope.to_string_slice(Range::new(0, rope.len())), chars);
        for chunk in rope.chunks.iter() {
            assert!(
                chunk.is_char_boundary(chunk.len()),
                "chunk 末尾必须是字符边界，got chunk len {}",
                chunk.len()
            );
        }
        // 拼接得到与原文一致的字节长度。
        let mut s = String::new();
        rope.append_to_string(&mut s);
        assert_eq!(s, chars);
    }

    #[test]
    fn char_boundary_moves_snap_over_multibyte() {
        // “a😀中b”：😀 占 4 字节、中 占 3 字节。逐字符移动光标必须落在各自起始，绝不落在字符中间。
        let buf = EditorBuffer::new_from("a😀中b");
        let offsets: Vec<usize> = {
            let mut v = Vec::new();
            let mut o = 0;
            let mut guard = 0;
            while o < buf.len() && guard < 20 {
                v.push(o);
                o = buf.next_char_boundary(o);
                guard += 1;
            }
            v.push(buf.len());
            v
        };
        // “a😀中b” 各字符起始：[0, 1, 5, 8, 9]
        assert_eq!(offsets, vec![0, 1, 5, 8, 9]);
        // 从末尾逐步回退。
        let mut back = Vec::new();
        let mut o = buf.len();
        while o > 0 {
            back.push(o);
            o = buf.prev_char_boundary(o);
        }
        back.push(0);
        back.reverse();
        assert_eq!(back, offsets);
    }

    #[test]
    fn mid_char_offset_snaps_to_char_start() {
        // 故意把游标放在“😀”中间（offset 3，真 ASCII 中间），回退/前进都要吸附到字符边界。
        let buf = EditorBuffer::new_from("x😀y");
        assert_eq!(buf.prev_char_boundary(3), 1); // 吸附到 😀 起始
        assert_eq!(buf.next_char_boundary(3), 5); // 吸附到 😀 末尾
    }

    #[test]
    fn clamp_to_char_boundary_keeps_valid_and_snaps_mid_char() {
        let buf = EditorBuffer::new_from("你你颜x"); // 你0..3 你3..6 颜6..9 x9..10
        // 已是字符边界 → 原样返回。
        assert_eq!(buf.clamp_to_char_boundary(6), 6);
        assert_eq!(buf.clamp_to_char_boundary(0), 0);
        assert_eq!(buf.clamp_to_char_boundary(10), 10); // len 边界
        // 落在 '颜' 中间 → 吸附回字符起点。
        assert_eq!(buf.clamp_to_char_boundary(7), 6);
        assert_eq!(buf.clamp_to_char_boundary(8), 6);
        // 越界 → 收敛到 len。
        assert_eq!(buf.clamp_to_char_boundary(99), 10);
        // 空文档。
        let empty = EditorBuffer::new_from("");
        assert_eq!(empty.clamp_to_char_boundary(5), 0);
    }

    #[test]
    fn emoji_spanning_former_chunk_boundary_roundtrip() {
        // 构造会让 chunk 边界恰落在 emoji 中间的大量多字节内容，验证局部编辑与全文一致。
        let head: String = "汉".repeat(340); // 1020 字节
        let body = "😀";
        let tail: String = "字".repeat(340); // 1020 字节
        let mut text = String::new();
        text.push_str(&head);
        text.push_str(body);
        text.push_str(&tail);
        let mut buf = EditorBuffer::new_from(&text);
        // 在中间（emoji 附近）插入一个 ASCII 字符。
        // 头 340 个“汉”=1020 字节，恰让首个 chunk 边界贴近 emoji；用字符边界定点光标。
        let at = buf.next_char_boundary(head.len()); // 😀 起始
        buf.edit(Range::new(at, at), "!", at + 1, at + 1, false);
        let expect = text[..at].to_string() + "!" + &text[at..];
        assert_eq!(buf.to_string(), expect);
        // 删除回退。
        buf.undo().unwrap();
        assert_eq!(buf.to_string(), text);
    }

    #[test]
    fn local_edit_keeps_suffix_shared_after_edit() {
        // 局部编辑只重建被编辑段，尾部长文本 chunk 应原样共享（指针共享，非全文拷贝）。
        let suffix: String = "tailcol".repeat(200); // 3200 字节尾缀
        let mut text = String::new();
        text.push_str("head.");
        text.push_str(&suffix);
        let mut buf = EditorBuffer::new_from(&text);
        let before_chunks: Vec<Arc<str>> = buf.text.chunks.to_vec();

        let insert_at = 5; // "head." 之后
        buf.edit(
            Range::new(insert_at, insert_at),
            "X",
            insert_at + 1,
            insert_at + 1,
            false,
        );

        let after_chunks = &buf.text.chunks;
        // 末尾 chunk 应与其在编辑前的引用完全相同（Arc 相同、未重建），证明后缀未全文拷贝重建。
        assert_eq!(
            before_chunks.last(),
            after_chunks.last(),
            "尾缀 chunk 应共享而非重建"
        );
        let expect = format!("head.X{suffix}");
        assert_eq!(buf.to_string(), expect);
    }

    #[test]
    fn single_char_insert_does_not_rebuild_full_document() {
        // 整改设计 4.1：单字符输入不得构造全文字符串。用 chunk 指针共享作为代理指标：
        // 一次单字符插入只重建被编辑段附近，占文档主体的尾缀 chunk 的 Arc 指针保持不变，
        // 且总 chunk 数不会因全文重建而增长。
        let head = "head.";
        let tail: String = "tailcol".repeat(300); // 4800 字节，跨多个 MAX_CHUNK
        let mut text = String::new();
        text.push_str(head);
        text.push_str(&tail);
        let mut buf = EditorBuffer::new_from(&text);
        let before: Vec<Arc<str>> = buf.text.chunks.to_vec();
        let before_count = before.len();
        assert!(before_count > 1, "前提：尾缀应被切成多个 chunk");

        let insert_at = head.len(); // "head." 之后插入单个字符
        buf.edit(
            Range::new(insert_at, insert_at),
            "!",
            insert_at + 1,
            insert_at + 1,
            false,
        );

        let after = &buf.text.chunks;
        // 占主体的尾缀 chunk 应保持 Arc 指针共享（OpenRope 局部编辑而非全文重建）。
        assert_eq!(before.last(), after.last(), "尾部 chunk 应共享而非重建");
        assert!(
            after.len() <= before_count + 1,
            "单字符编辑不应让 chunk 数因全文重建而显著增长"
        );
        let expect = format!("{head}!{tail}");
        assert_eq!(buf.to_string(), expect);
    }

    #[test]
    fn local_edits_never_produce_full_document_changes() {
        // DM-507 / DM-503：缩进/行注释等局部编辑走 `after_edit` 增量变更，绝不得产出
        // `full_document` 变更（那会让宿主走整文档 `to_string()` 同步）。此属性守护
        // 顶层把 `buffer.edit(...).changes` 直接喂给 `after_edit` 的机制。
        let mut buf = EditorBuffer::new_from("a\nb\nc\nd\n");
        let mut versions: Vec<u64> = Vec::new();
        // 模拟多行缩进：逐行行首插入，多次局部 edit。自底向上插入（同 indent 的 `.rev()`），
        // 保证上方插入不会使后续目标行 `line_start` 偏移。
        let rows = [2usize, 1, 0];
        for row in rows {
            let offset = buf.line_start(row);
            let edit = buf.edit(
                Range::new(offset, offset),
                "  ",
                offset + 2,
                offset + 2,
                false,
            );
            assert!(
                !edit.changes.is_empty(),
                "每次局部 edit 至少要产出一条增量变更"
            );
            for change in &edit.changes {
                assert!(
                    !change.full_document,
                    "局部编辑不得产出整文档变更: {:?}",
                    change
                );
                // 增量变更应描述被替换的精确旧区间（非全文档的 0..0）。
                assert!(
                    change.old_range.end <= buf.len() + 2,
                    "变更旧区间必须落在文档内"
                );
                versions.push(change.version);
            }
        }
        // 每次 edit 版本单调递增，最终文本正确。
        assert_eq!(buf.to_string(), "  a\n  b\n  c\nd\n");
        let mut prev = 0;
        for v in versions {
            assert!(v >= prev, "版本号应单调不减");
            prev = v;
        }
    }

    #[test]
    fn doc_utf16_byte_conversions_roundtrip() {
        // 中文(3 字节 UTF-8 / 1 UTF-16)、emoji(4 字节 / 2 UTF-16)、默认生效 BMP 之外字符。
        let text = "a汉b😀c";
        let snap = EditorBuffer::new_from(text).snapshot();
        // 逐字符边界：a(byte0,utf16 0) 汉(1,1) b(4,2) 😀(5,3) c(9,5) 末(10,6)
        let cases: &[(usize, usize)] = &[(0, 0), (1, 1), (4, 2), (5, 3), (9, 5), (10, 6)];
        for &(byte, utf16) in cases {
            assert_eq!(
                snap.byte_to_utf16(byte),
                utf16,
                "byte {byte} -> utf16 {utf16}"
            );
            assert_eq!(
                snap.utf16_to_byte(utf16),
                Some(byte),
                "utf16 {utf16} -> byte {byte}"
            );
        }
        // 越界 UTF-16 返回 None。
        assert_eq!(snap.utf16_to_byte(7), None);
        assert_eq!(
            snap.utf16_to_byte(6),
            Some(10),
            "文档级末尾 UTF-16 应映射到文本末尾"
        );
    }

    #[test]
    fn doc_utf16_conversion_matches_reference() {
        // 与朴素全文换算结果一致，确保不构造全文时结果不偏差。
        let text = "SELECT '汉😀x' FROM t;";
        let snap = EditorBuffer::new_from(text).snapshot();
        let mut byte = 0usize;
        for c in text.chars() {
            assert_eq!(snap.utf16_to_byte(snap.byte_to_utf16(byte)).unwrap(), byte);
            byte += c.len_utf8();
        }
        // 全文 UTF-16 长度核对。
        let total_utf16: usize = text.chars().map(|c| c.len_utf16() as usize).sum();
        assert_eq!(snap.byte_to_utf16(text.len()), total_utf16);
    }

    #[test]
    fn byte_to_utf16_is_safe_for_mid_character_offsets() {
        let snap = EditorBuffer::new_from("a汉b").snapshot();
        // 汉字占 1..4 字节；任意中间 offset 都应回退到它的起点，而不是 panic。
        assert_eq!(snap.byte_to_utf16(2), 1);
        assert_eq!(snap.byte_to_utf16(3), 1);
    }
}
