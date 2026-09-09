// sql_editor_adapter/syntax.rs —— SQL 语言定义与简单词法高亮。
//
// `SqlLanguage` 包装方言，实现 fluxdb-editor-core 的 `LanguageDefinition`；
// `tokenize_sql` 为纯函数词法器，产出注释 / 字符串 / 数字 / 关键字高亮。

/// 包装方言的语言定义，实现 fluxdb-editor-core 的 `LanguageDefinition`。
// 语法高亮由同层的 `SqlAdapter`/`SyntaxProvider` 提供；该类型承载语言元数据与折叠规则。
#[allow(dead_code)]
pub struct SqlLanguage {
    /// 当前方言。
    pub dialect: SqlDialect,
    /// 按文档版本缓存折叠范围；布局重绘不会重复扫描全文。
    fold_cache: Mutex<Option<(u64, usize, usize, Vec<Range>)>>,
}

impl SqlLanguage {
    /// 以指定方言构造。
    #[allow(dead_code)]
    pub fn new(dialect: SqlDialect) -> Self {
        Self {
            dialect,
            fold_cache: Mutex::new(None),
        }
    }
}

impl LanguageDefinition for SqlLanguage {
    /// language_id 由方言决定，如 "sql_mysql"。
    fn language_id(&self) -> &str {
        self.dialect.language_id()
    }

    /// 行注释标记。
    fn line_comment(&self) -> Option<&str> {
        self.dialect.line_comment()
    }

    /// 块注释标记。
    fn block_comment(&self) -> Option<(&str, &str)> {
        self.dialect.block_comment()
    }

    /// 折叠区间：对每条**跨行** SQL 语句返回其字节区间，作为可折叠块。
    ///
    /// 只折叠真正的语句块（多行语句 / begin-end 分组），拒绝把每个单行当作折叠区间；
    /// 单行语句不产生折叠。区间为字节偏移（与 snapshot 一致），由前端在每次编辑后
    /// 基于最新 snapshot 重新推导折叠（稳定 range 语义，见整改设计 5.4）。
    fn fold_ranges(&self, snapshot: &BufferSnapshot) -> Vec<Range> {
        let started = Instant::now();
        let version = snapshot.version();
        let mut cache_hit = false;
        let result = match self.fold_cache.lock() {
            Ok(mut cache) => {
                if cache
                    .as_ref()
                    .map(|(cached, bytes, lines, _)| (*cached, *bytes, *lines))
                    != Some((version, snapshot.len(), snapshot.line_count()))
                {
                    let ranges = split_statement_ranges_snapshot(snapshot)
                        .into_iter()
                        .filter(|range| {
                            snapshot.offset_to_point(range.start).row
                                < snapshot.offset_to_point(range.end.saturating_sub(1)).row
                        })
                        .collect::<Vec<_>>();
                    *cache = Some((version, snapshot.len(), snapshot.line_count(), ranges));
                } else {
                    cache_hit = true;
                }
                cache
                    .as_ref()
                    .map(|(_, _, _, ranges)| ranges.clone())
                    .unwrap_or_default()
            }
            Err(_) => {
                split_statement_ranges_snapshot(snapshot)
                    .into_iter()
                    .filter(|range| {
                        snapshot.offset_to_point(range.start).row
                            < snapshot.offset_to_point(range.end.saturating_sub(1)).row
                    })
                    .collect::<Vec<_>>()
            }
        };
        tracing::debug!(
            target: "gdb_sql_perf",
            op = "fold_scan",
            elapsed_us = started.elapsed().as_micros() as u64,
            buffer_version = version,
            text_bytes = snapshot.len(),
            fold_count = result.len(),
            cache_hit,
        );
        result
    }
}

/// SQL 高亮用的 tree-sitter-sequel 查询文本。
///
/// 基于 tree-sitter-sequel 自带的 `queries/highlights.scm` 精简而来，只保留
/// 本编辑器主题关心的 token 类别（注释 / 字符串 / 数字 / 关键字 / 类型 / 操作符 /
/// 布尔 / 参数 / 函数 / 变量）。生产路径使用该查询做真实语法高亮；
/// `tokenize_sql` 的简单词法器仅保留为测试 fallback（见设计 5.3）。
fn sql_highlights_query() -> &'static str {
    r#"
(comment) @comment
(marginalia) @comment

(literal) @string
(parameter) @parameter

[
  (keyword_true)
  (keyword_false)
] @boolean

[
  (keyword_int)
  (keyword_boolean)
  (keyword_binary)
  (keyword_bit)
  (keyword_character)
  (keyword_smallint)
  (keyword_bigint)
  (keyword_tinyint)
  (keyword_decimal)
  (keyword_float)
  (keyword_double)
  (keyword_numeric)
  (keyword_real)
  (keyword_money)
  (keyword_char)
  (keyword_varchar)
  (keyword_text)
  (keyword_uuid)
  (keyword_json)
  (keyword_date)
  (keyword_datetime)
  (keyword_time)
  (keyword_timestamp)
  (keyword_interval)
] @type

[
  (keyword_select)
  (keyword_from)
  (keyword_where)
  (keyword_join)
  (keyword_create)
  (keyword_insert)
  (keyword_update)
  (keyword_delete)
  (keyword_into)
  (keyword_values)
  (keyword_set)
  (keyword_order)
  (keyword_group)
  (keyword_by)
  (keyword_having)
  (keyword_limit)
  (keyword_offset)
  (keyword_table)
  (keyword_as)
  (keyword_with)
  (keyword_distinct)
  (keyword_union)
  (keyword_in)
  (keyword_and)
  (keyword_or)
  (keyword_not)
  (keyword_is)
  (keyword_null)
  (keyword_on)
  (keyword_like)
  (keyword_between)
  (keyword_exists)
  (keyword_primary)
  (keyword_foreign)
  (keyword_key)
  (keyword_constraint)
  (keyword_references)
  (keyword_default)
  (keyword_comment)
  (keyword_collate)
  (keyword_engine)
  (keyword_auto_increment)
  (keyword_unsigned)
  (keyword_unique)
  (keyword_generated)
  (keyword_always)
  (keyword_stored)
  (keyword_virtual)
  (keyword_check)
  (keyword_current_timestamp)
  (keyword_alter)
  (keyword_drop)
  (keyword_add)
  (keyword_index)
  (keyword_view)
] @keyword

(object_reference
  name: (identifier) @identifier)

; 建表列名来自 `column_definition.name`，查询字段名来自 `field.name`。
(field
  name: (identifier) @field)
(column_definition
  name: (identifier) @field)
(table_option
  name: (identifier) @attribute)

; 别名和函数名单独标注，避免被普通对象名规则覆盖。
(relation
  alias: (identifier) @variable)
(term
  alias: (identifier) @variable)
(invocation
  (object_reference
    name: (identifier) @function))
"#
}

/// 判断一段字面量文本是否为纯数字字面量（整数 / 小数，可带正负号）。
///
/// 用于把 tree-sitter 的 `literal` 节点按内容区分为 number 或 string。
fn is_numeric_literal(seg: &str) -> bool {
    let s = seg.trim();
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    if matches!(bytes[0], b'+' | b'-') {
        i = 1;
        if i >= bytes.len() {
            return false;
        }
    }
    let mut digits = 0;
    let mut has_dot = false;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            digits += 1;
        } else if b == b'.' && !has_dot {
            has_dot = true;
        } else {
            return false;
        }
        i += 1;
    }
    digits > 0
}

fn is_numeric_snapshot_range(snapshot: &BufferSnapshot, range: Range) -> bool {
    let mut digits = 0usize;
    let mut has_dot = false;
    let mut first = true;
    for chunk in snapshot.text_chunks_in_range(range) {
        for byte in chunk {
            if first && matches!(*byte, b'+' | b'-') {
                first = false;
                continue;
            }
            first = false;
            if byte.is_ascii_digit() {
                digits += 1;
            } else if *byte == b'.' && !has_dot {
                has_dot = true;
            } else {
                return false;
            }
        }
    }
    digits > 0
}

/// 把字节区间收敛到 UTF-8 字符边界，避免 tree-sitter 的错误节点切进中文字符内部。
///
/// tree-sitter 的字节范围对 ASCII 安全，但在中文输入的错误态里可能落在多字节字符中间；
/// 渲染前先吸附到合法边界，避免 GPUI 按 `TextRun.len` 切片时 panic。
fn clamp_range_to_char_boundaries(
    text: &str,
    range: std::ops::Range<usize>,
) -> Option<Range> {
    let len = text.len();
    let mut start = range.start.min(len);
    let mut end = range.end.min(len);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    while end < len && !text.is_char_boundary(end) {
        end += 1;
    }
    (start < end).then_some(Range::new(start, end))
}

struct SqlSyntaxCache {
    parser: tree_sitter::Parser,
    tree: tree_sitter::Tree,
    /// 语句级快速路径更新高亮后，MySQL 归一化输入无法复用旧树；其它方言会尝试增量更新。
    tree_valid: bool,
    /// parser 输入是否经过 MySQL 兼容归一化；归一化会改变字节流，不能直接套用 InputEdit。
    parser_normalized: bool,
    /// 与语法树对应的不可变快照；局部编辑只替换 Arc 根，不复制全文。
    snapshot: BufferSnapshot,
    version: u64,
    highlights: Vec<Highlight>,
    statement_ranges: Vec<Range>,
    /// statement 级 subtree 高亮缓存；未受影响 statement 的 `Arc` 在编辑后复用。
    statement_highlights: Vec<Arc<Vec<Highlight>>>,
    /// Tree-sitter changed-subtree 缓存，按 root sibling 顺序保存；只要节点未标记变化，
    /// 且类型/长度一致，就复用相对坐标高亮，避免大文档每次输入重新计算内容 hash。
    changed_subtree_highlights: Vec<CachedSubtreeHighlights>,
    /// 大文档首轮 tokenizer 后的 Tree-sitter refinement 游标；`usize::MAX` 表示语法树已建立。
    refinement_cursor: usize,
}

#[derive(Clone, Debug)]
struct CachedSubtreeHighlights {
    kind: u16,
    bytes: usize,
    highlights: Arc<Vec<Highlight>>,
    children: Vec<CachedSubtreeHighlights>,
}

fn changed_subtree_highlight_cache(
    tree: &tree_sitter::Tree,
    highlights: &[Highlight],
) -> Vec<CachedSubtreeHighlights> {
    changed_subtree_highlight_cache_reusing(tree, highlights, None)
}

fn changed_subtree_highlight_cache_reusing(
    tree: &tree_sitter::Tree,
    highlights: &[Highlight],
    previous: Option<&[CachedSubtreeHighlights]>,
) -> Vec<CachedSubtreeHighlights> {
    let root = tree.root_node();
    let mut cache = Vec::with_capacity(root.named_child_count());
    for index in 0..root.named_child_count() {
        let Some(node) = root.named_child(index as u32) else { continue };
        let kind = node.kind_id();
        let bytes = node.end_byte().saturating_sub(node.start_byte());
        if !node.has_changes() {
            if let Some(previous) = previous.and_then(|cache| cache.get(index))
                && previous.kind == kind
                && previous.bytes == bytes
            {
                cache.push(previous.clone());
                continue;
            }
        }
        let range = Range::new(node.start_byte(), node.end_byte());
        let local_highlights = highlights_for_range(highlights, range);
        let relative = local_highlights
            .iter()
            .map(|highlight| Highlight {
                range: Range::new(
                    highlight.range.start.saturating_sub(range.start),
                    highlight.range.end.saturating_sub(range.start),
                ),
                kind: highlight.kind.clone(),
            })
            .collect::<Vec<_>>();
        // Keep an Arc even for empty subtrees: comments/whitespace-only nodes are
        // still reusable and avoid re-running the query after an unrelated edit.
        let children = if bytes <= DEEP_CACHE_MAX_BYTES {
            (0..node.named_child_count())
                .filter_map(|child_index| node.named_child(child_index as u32).map(|child| (child_index, child)))
                .map(|(child_index, child)| {
                    cache_subtree_node(
                        child,
                        &local_highlights,
                        previous
                            .and_then(|cache| cache.get(index))
                            .and_then(|cached| cached.children.get(child_index)),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        cache.push(CachedSubtreeHighlights {
            kind,
            bytes,
            highlights: Arc::new(relative),
            children,
        });
    }
    cache
}

const DEEP_CACHE_MAX_BYTES: usize = 64 * 1024;

fn highlights_for_range(highlights: &[Highlight], range: Range) -> Vec<Highlight> {
    let end = highlights.partition_point(|highlight| highlight.range.start < range.end);
    let mut start = highlights.partition_point(|highlight| highlight.range.start < range.start);
    while start > 0 && highlights[start - 1].range.end > range.start {
        start -= 1;
    }
    highlights[start.min(end)..end].to_vec()
}

fn cache_subtree_node(
    node: tree_sitter::Node<'_>,
    highlights: &[Highlight],
    previous: Option<&CachedSubtreeHighlights>,
) -> CachedSubtreeHighlights {
    let kind = node.kind_id();
    let bytes = node.end_byte().saturating_sub(node.start_byte());
    if !node.has_changes()
        && previous.is_some_and(|cached| cached.kind == kind && cached.bytes == bytes)
    {
        return previous.cloned().unwrap();
    }
    let range = Range::new(node.start_byte(), node.end_byte());
    let local_highlights = highlights_for_range(highlights, range);
    let relative = local_highlights
        .iter()
        .map(|highlight| Highlight {
            range: Range::new(
                highlight.range.start.saturating_sub(range.start),
                highlight.range.end.saturating_sub(range.start),
            ),
            kind: highlight.kind.clone(),
        })
        .collect();
    let children = if bytes <= DEEP_CACHE_MAX_BYTES {
        (0..node.named_child_count())
            .filter_map(|index| node.named_child(index as u32).map(|child| (index, child)))
            .map(|(index, child)| {
                cache_subtree_node(
                    child,
                    &local_highlights,
                    previous.and_then(|cached| cached.children.get(index)),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    CachedSubtreeHighlights {
        kind,
        bytes,
        highlights: Arc::new(relative),
        children,
    }
}

/// 根据 Tree-sitter 增量树的 sibling subtree changed 标记复用高亮。
/// 这是真正的 changed-subtree cache：未改变内容的节点不再执行 QueryCursor，只对
/// changed 节点重新查询；节点移动后的绝对 offset 由当前 node.start_byte() 重新计算。
fn highlight_from_changed_subtrees(
    snapshot: &BufferSnapshot,
    tree: &tree_sitter::Tree,
    old_cache: &[CachedSubtreeHighlights],
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<Vec<Highlight>> {
    let root = tree.root_node();
    if root.named_child_count() == 0 {
        return highlight_sql_tree_snapshot_range_cancellable(
            snapshot,
            tree,
            None,
            latest_request,
            request_id,
        );
    }
    let mut result = Vec::new();
    let mut reused_count = 0usize;
    let mut reparsed_count = 0usize;
    for index in 0..root.named_child_count() {
        let Some(node) = root.named_child(index as u32) else { continue };
        let start = node.start_byte();
        if let Some(cached) = old_cache.get(index).filter(|cached| {
            cached.kind == node.kind_id()
                && cached.bytes == node.end_byte().saturating_sub(node.start_byte())
        }) {
            if !node.has_changes() {
                result.extend(translate_cached_highlights(cached, start));
                reused_count += 1;
                continue;
            }
            let local = highlight_changed_subtree_with_children(
                snapshot,
                tree,
                node,
                cached,
                latest_request,
                request_id,
            )?;
            reparsed_count += 1;
            result.extend(local);
            continue;
        }
        let local = highlight_sql_tree_snapshot_range_cancellable(
            snapshot,
            tree,
            Some(Range::new(start, node.end_byte())),
            latest_request,
            request_id,
        )?;
        reparsed_count += 1;
        result.extend(local);
    }
    result.sort_by_key(|highlight| (highlight.range.start, highlight.range.end));
    tracing::debug!(
        target: "gdb_sql_perf",
        op = "syntax_changed_subtree_cache",
        request_id,
        buffer_version = snapshot.version(),
        reused_count,
        reparsed_count,
    );
    Some(result)
}

fn translate_cached_highlights(
    cached: &CachedSubtreeHighlights,
    start: usize,
) -> Vec<Highlight> {
    cached
        .highlights
        .iter()
        .map(|highlight| Highlight {
            range: Range::new(
                start.saturating_add(highlight.range.start),
                start.saturating_add(highlight.range.end),
            ),
            kind: highlight.kind.clone(),
        })
        .collect()
}

/// 父节点被 Tree-sitter 标记 changed 时，继续复用其中未变化的子树。
/// 父级查询仍保留跨子树的 capture，子树内部 capture 则由旧 Arc 替换，避免重复结果。
fn highlight_changed_subtree_with_children(
    snapshot: &BufferSnapshot,
    tree: &tree_sitter::Tree,
    node: tree_sitter::Node<'_>,
    cached: &CachedSubtreeHighlights,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<Vec<Highlight>> {
    let range = Range::new(node.start_byte(), node.end_byte());
    let mut result = highlight_sql_tree_snapshot_range_cancellable(
        snapshot,
        tree,
        Some(range),
        latest_request,
        request_id,
    )?;
    for index in 0..node.named_child_count() {
        let Some(child) = node.named_child(index as u32) else { continue };
        let Some(child_cache) = cached.children.get(index) else { continue };
        if child.has_changes()
            || child_cache.kind != child.kind_id()
            || child_cache.bytes != child.end_byte().saturating_sub(child.start_byte())
        {
            continue;
        }
        let child_range = Range::new(child.start_byte(), child.end_byte());
        result.retain(|highlight| {
            !(highlight.range.start >= child_range.start
                && highlight.range.end <= child_range.end)
        });
        result.extend(translate_cached_highlights(child_cache, child_range.start));
    }
    result.sort_by_key(|highlight| (highlight.range.start, highlight.range.end));
    Some(result)
}

fn statement_highlight_cache(
    ranges: &[Range],
    highlights: &[Highlight],
) -> Vec<Arc<Vec<Highlight>>> {
    let mut buckets = vec![Vec::new(); ranges.len()];
    let mut range_hint = 0usize;
    for highlight in highlights {
        // Both inputs are sorted by byte offset. Advance monotonically instead of
        // restarting a linear search for every token (important for large SQL files).
        while range_hint < ranges.len() && ranges[range_hint].end <= highlight.range.start {
            range_hint += 1;
        }
        let mut index = range_hint;
        while index < ranges.len() && ranges[index].start < highlight.range.end {
            let range = ranges[index];
            if highlight.range.start < range.end && highlight.range.end > range.start {
                buckets[index].push(highlight.clone());
            }
            index += 1;
        }
    }
    buckets.into_iter().map(|items| Arc::new(items)).collect()
}

/// Tree-sitter 查询使用的 Rope chunk 文本源；节点文本跨 chunk 时仍保持零拷贝。
struct SnapshotTextProvider<'a> {
    snapshot: &'a BufferSnapshot,
}

impl<'a> tree_sitter::TextProvider<&'a [u8]> for SnapshotTextProvider<'a> {
    type I = fluxdb_editor_core::SnapshotChunkIter<'a>;

    fn text(&mut self, node: tree_sitter::Node) -> Self::I {
        self.snapshot.text_chunks_in_range(Range::new(node.start_byte(), node.end_byte()))
    }
}

const TREE_SITTER_FULL_PARSE_LIMIT: usize = 512 * 1024;
const TREE_SITTER_REFINEMENT_BATCH_BYTES: usize = 64 * 1024;

/// 用 tree-sitter-sequel 对 SQL 文本做真实语法高亮。
///
/// 逐段运行 `sql_highlights_query`，把捕获到的语法节点映射为
/// `Highlight { range, kind }`（字节偏移，与 buffer 一致），供编辑器按主题
/// token 颜色渲染。相比 `tokenize_sql`，本实现词法由真实 SQL 语法树驱动，
/// 关键字大小写、字符串转义、注释与参数识别更准确。
#[allow(dead_code)] // 保留无缓存入口供独立调用和测试使用。
pub fn highlight_sql_tree_sitter(text: &str, dialect: SqlDialect) -> Vec<Highlight> {
    use tree_sitter::{Language, Parser};

    let lang = Language::new(tree_sitter_sequel::LANGUAGE);
    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("tree-sitter-sequel language should be valid");
    let parser_text = mysql_parser_text(text, dialect);
    let Some(tree) = parser.parse(parser_text.as_bytes(), None) else {
        return Vec::new();
    };
    highlight_sql_tree(text, tree, dialect)
}

fn parse_tree_with_cancellation(
    parser: &mut tree_sitter::Parser,
    input: &[u8],
    old_tree: Option<&tree_sitter::Tree>,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<tree_sitter::Tree> {
    let mut read = |offset: usize, _position: tree_sitter::Point| {
        input.get(offset..).unwrap_or_default()
    };
    let mut progress = |_state: &tree_sitter::ParseState| {
        // Tree-sitter progress callback 返回 true 表示取消；仅取消已过期请求。
        if latest_request.load(Ordering::Acquire) != request_id {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    };
    parser.parse_with_options(
        &mut read,
        old_tree,
        Some(tree_sitter::ParseOptions::new().progress_callback(&mut progress)),
    )
}

fn parse_snapshot_with_cancellation(
    parser: &mut tree_sitter::Parser,
    snapshot: &BufferSnapshot,
    old_tree: Option<&tree_sitter::Tree>,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<tree_sitter::Tree> {
    let mut read = |offset: usize, _position: tree_sitter::Point| {
        snapshot.text_chunk_bytes_at(offset)
    };
    let mut progress = |_state: &tree_sitter::ParseState| {
        // Tree-sitter progress callback 返回 true 表示取消；仅取消已过期请求。
        if latest_request.load(Ordering::Acquire) != request_id {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    };
    parser.parse_with_options(
        &mut read,
        old_tree,
        Some(tree_sitter::ParseOptions::new().progress_callback(&mut progress)),
    )
}

/// 使用缓存的 statement ranges 校验局部编辑，避免为判断边界复制旧全文。
fn local_statement_edit_cached(
    old_snapshot: &BufferSnapshot,
    new_snapshot: &BufferSnapshot,
    ranges: &[Range],
    changed: &InputEdit,
) -> Option<(Range, Range)> {
    let mut matched = None;
    for range in ranges {
        let intersects = if changed.old_range.is_empty() {
            range.start <= changed.old_range.start && changed.old_range.start <= range.end
        } else {
            range.start < changed.old_range.end && changed.old_range.start < range.end
        };
        if intersects {
            if matched.is_some() {
                return None;
            }
            matched = Some(*range);
        }
    }
    let old_statement = matched?;
    let removed = old_snapshot.text_in_range(changed.old_range);
    if removed.contains(';') || changed.new_text.contains(';') {
        return None;
    }
    let delta = changed.new_text.len() as isize
        - changed.old_range.end.saturating_sub(changed.old_range.start) as isize;
    let new_end = shifted_offset(old_statement.end, delta).min(new_snapshot.len());
    if old_statement.start >= new_end {
        return None;
    }
    // 语句首尾的空白变化会改变 split_statements 的边界，交给窗口/全文路径处理。
    if new_snapshot
        .byte_at(old_statement.start)
        .is_none_or(|byte| byte.is_ascii_whitespace())
        || new_snapshot
            .byte_at(new_end.saturating_sub(1))
            .is_none_or(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    Some((old_statement, Range::new(old_statement.start, new_end)))
}

/// 尝试只从当前快照读取被编辑的 statement；结构变化时返回 None 交给全量路径。
/// 语句范围来自上一次完整切分，因而不会为了单字符输入复制整篇快照。
fn try_local_statement_highlight(
    snapshot: &BufferSnapshot,
    dialect: SqlDialect,
    changed: &InputEdit,
    version: u64,
    cache: &Arc<Mutex<Option<SqlSyntaxCache>>>,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<(Vec<Highlight>, Range, &'static str)> {
    if changed.full_document
        || (changed.old_range.is_empty() && changed.new_text.is_empty())
        || changed.new_text.len() > 256
        || changed.old_range.end.saturating_sub(changed.old_range.start) > 256
    {
        return None;
    }
    let mut cached = cache.lock().ok()?.take()?;
    if cached.version.saturating_add(1) != version || changed.version != version {
        store_syntax_cache(cache, cached);
        return None;
    }
    let mut matched_statement = None;
    for range in &cached.statement_ranges {
        let intersects = if changed.old_range.is_empty() {
            range.start <= changed.old_range.start && changed.old_range.start <= range.end
        } else {
            range.start < changed.old_range.end && changed.old_range.start < range.end
        };
        if intersects {
            if matched_statement.is_some() {
                store_syntax_cache(cache, cached);
                return None;
            }
            matched_statement = Some(*range);
        }
    }
    let Some(old_statement) = matched_statement else {
        store_syntax_cache(cache, cached);
        return None;
    };
    // 分号改变语句拓扑，不能沿用旧 statement 范围；保守回退全量路径。
    let removed_start = changed.old_range.start.min(cached.snapshot.len());
    let removed_end = changed.old_range.end.min(cached.snapshot.len());
    let removed = cached
        .snapshot
        .text_in_range(Range::new(removed_start, removed_end));
    if removed.contains(';') || changed.new_text.contains(';') {
        store_syntax_cache(cache, cached);
        return None;
    }
    let delta = changed.new_text.len() as isize
        - changed.old_range.end.saturating_sub(changed.old_range.start) as isize;
    let new_statement = Range::new(
        old_statement.start,
        shifted_offset(old_statement.end, delta).min(snapshot.len()),
    );
    if new_statement.start >= new_statement.end {
        store_syntax_cache(cache, cached);
        return None;
    }
    let statement_text = snapshot.text_in_range(new_statement);
    let Some(local) = parse_statement_highlights(
        &statement_text,
        dialect,
        latest_request,
        request_id,
    ) else {
        store_syntax_cache(cache, cached);
        return None;
    };
    let local = local
        .into_iter()
        .map(|mut highlight| {
            highlight.range.start += new_statement.start;
            highlight.range.end += new_statement.start;
            highlight
        })
        .collect::<Vec<_>>();
    if cached.statement_highlights.len() == cached.statement_ranges.len() {
        if let Some(index) = cached.statement_ranges.iter().position(|range| *range == old_statement) {
            cached.statement_highlights[index] = Arc::new(local.clone());
        }
    }
    merge_incremental_highlights_in_place(
        &mut cached.highlights,
        local.clone(),
        old_statement,
        new_statement.len(),
        new_statement,
        snapshot.len(),
    );
    for range in &mut cached.statement_ranges {
        if *range == old_statement {
            *range = new_statement;
        } else if range.start >= old_statement.end {
            range.start = shifted_offset(range.start, delta).min(snapshot.len());
            range.end = shifted_offset(range.end, delta).min(snapshot.len());
        }
    }
    // 高亮和语法树使用同一份增量变更。解析在后台执行，过期请求由 progress callback 取消。
    cached.tree_valid = refresh_incremental_tree(
        &mut cached,
        snapshot,
        dialect,
        changed,
        latest_request,
        request_id,
    );
    if cached.tree_valid && !cached.parser_normalized {
        let previous = cached.changed_subtree_highlights.clone();
        cached.changed_subtree_highlights = changed_subtree_highlight_cache_reusing(
            &cached.tree,
            &cached.highlights,
            Some(&previous),
        );
    }
    cached.version = version;
    cached.snapshot = snapshot.clone();
    // DM-401：返回**作用域内**高亮 + 对应 statement 区间，宿主用 `HighlightStore::apply_range`
    // 做增量合并，避免把整份文档高亮一次全量重建索引。
    store_syntax_cache(cache, cached);
    Some((local, new_statement, "statement_snapshot_incremental"))
}

/// 处理分号插入/删除：只重解析受影响语句及相邻语句窗口，避免结构编辑直接触发全文解析。
fn try_local_statement_window_highlight(
    snapshot: &BufferSnapshot,
    dialect: SqlDialect,
    changed: &InputEdit,
    version: u64,
    cache: &Arc<Mutex<Option<SqlSyntaxCache>>>,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<(Vec<Highlight>, Range, &'static str)> {
    if changed.full_document
        || changed.new_text.len() > 256
        || changed.old_range.len() > 256
        || (changed.new_text.is_empty() && changed.old_range.is_empty())
    {
        return None;
    }
    let mut cached = cache.lock().ok()?.take()?;
    if cached.version.saturating_add(1) != version || changed.version != version {
        store_syntax_cache(cache, cached);
        return None;
    }
    let ranges = &cached.statement_ranges;
    if ranges.is_empty() {
        store_syntax_cache(cache, cached);
        return None;
    }
    let mut first = ranges
        .iter()
        .position(|range| changed.old_range.start <= range.end && changed.old_range.end >= range.start)
        .unwrap_or_else(|| {
            ranges
                .iter()
                .position(|range| range.end >= changed.old_range.start)
                .unwrap_or(ranges.len() - 1)
        });
    let mut last = ranges
        .iter()
        .rposition(|range| changed.old_range.start <= range.end && changed.old_range.end >= range.start)
        .unwrap_or(first);
    first = first.saturating_sub(1);
    last = (last + 1).min(ranges.len() - 1);
    let old_window = Range::new(ranges[first].start, ranges[last].end);
    let delta = changed.new_text.len() as isize - changed.old_range.len() as isize;
    let new_window = Range::new(
        old_window.start,
        shifted_offset(old_window.end, delta).min(snapshot.len()),
    );
    if new_window.start >= new_window.end {
        store_syntax_cache(cache, cached);
        return None;
    }
    let window_text = snapshot.text_in_range(new_window);
    let Some(local) = parse_statement_highlights(&window_text, dialect, latest_request, request_id)
    else {
        store_syntax_cache(cache, cached);
        return None;
    };
    let local = local
        .into_iter()
        .map(|mut highlight| {
            highlight.range.start += new_window.start;
            highlight.range.end += new_window.start;
            highlight
        })
        .collect::<Vec<_>>();
    merge_incremental_highlights_in_place(
        &mut cached.highlights,
        local.clone(),
        old_window,
        new_window.len(),
        new_window,
        snapshot.len(),
    );
    let local_ranges = split_statements(&window_text)
        .into_iter()
        .map(|range| Range::new(range.start + new_window.start, range.end + new_window.start));
    let mut updated_ranges = Vec::with_capacity(cached.statement_ranges.len() + 2);
    updated_ranges.extend(cached.statement_ranges[..first].iter().copied());
    updated_ranges.extend(local_ranges);
    updated_ranges.extend(cached.statement_ranges[last + 1..].iter().map(|range| {
        Range::new(
            shifted_offset(range.start, delta).min(snapshot.len()),
            shifted_offset(range.end, delta).min(snapshot.len()),
        )
    }));
    cached.statement_ranges = updated_ranges;
    // 结构编辑会改变 statement 拓扑；窗口内缓存重建，窗口外 subtree 仍可复用。
    cached.statement_highlights = statement_highlight_cache(&cached.statement_ranges, &cached.highlights);
    // 分号改变语句拓扑，但 Tree-sitter 仍可通过 InputEdit 保留未受影响的子树。
    cached.tree_valid = refresh_incremental_tree(
        &mut cached,
        snapshot,
        dialect,
        changed,
        latest_request,
        request_id,
    );
    cached.version = version;
    cached.snapshot = snapshot.clone();
    // DM-401：返回窗口内作用域高亮 + 窗口区间，宿主 `apply_range` 增量合并（同 statement 路径）。
    store_syntax_cache(cache, cached);
    Some((local, new_window, "statement_window_incremental"))
}

fn dirty_range_for_snapshot(snapshot: &BufferSnapshot, changed: &InputEdit) -> Range {
    let start_row = snapshot.offset_to_point(changed.old_range.start).row;
    let end_offset = changed
        .old_range
        .start
        .saturating_add(changed.new_text.len())
        .min(snapshot.len());
    let end_row = snapshot.offset_to_point(end_offset).row;
    Range::new(snapshot.line_start(start_row), snapshot.line_end_offset(end_row))
}

fn dirty_ranges_for_snapshot_edit(snapshot: &BufferSnapshot, changed: &InputEdit) -> Vec<Range> {
    let added_newlines = changed.new_text.bytes().filter(|&byte| byte == b'\n').count();
    let replaced_len = changed.old_range.len();
    if added_newlines > 256 || replaced_len.max(changed.new_text.len()) * 4 > snapshot.len() {
        return vec![Range::new(0, snapshot.len())];
    }
    vec![dirty_range_for_snapshot(snapshot, changed)]
}

/// 对缓存语法树应用一次真实 Tree-sitter InputEdit。
///
/// 只有 parser 输入与 snapshot 字节流一致时才能安全复用旧树；MySQL 兼容归一化会
/// 改写输入，继续走完整 parser fallback。失败时返回 false，由调用方在下次请求中重建。
fn refresh_incremental_tree(
    cached: &mut SqlSyntaxCache,
    snapshot: &BufferSnapshot,
    dialect: SqlDialect,
    changed: &InputEdit,
    latest_request: &AtomicU64,
    request_id: u64,
) -> bool {
    if !cached.tree_valid
        || changed.full_document
        || changed.old_range.end > cached.snapshot.len()
        || (matches!(dialect, SqlDialect::Mysql)
            && (cached.parser_normalized || mysql_parser_needs_normalization(&changed.new_text)))
    {
        return false;
    }
    let old_start = cached.snapshot.offset_to_point(changed.old_range.start);
    let old_end = cached.snapshot.offset_to_point(changed.old_range.end);
    let start_position = tree_sitter::Point {
        row: old_start.row,
        column: old_start.column,
    };
    let old_end_position = tree_sitter::Point {
        row: old_end.row,
        column: old_end.column,
    };
    let new_end_position = tree_point_after_text(start_position, &changed.new_text);
    cached.tree.edit(&tree_sitter::InputEdit {
        start_byte: changed.old_range.start,
        old_end_byte: changed.old_range.end,
        new_end_byte: changed.old_range.start + changed.new_text.len(),
        start_position,
        old_end_position,
        new_end_position,
    });
    let Some(tree) = parse_snapshot_with_cancellation(
        &mut cached.parser,
        snapshot,
        Some(&cached.tree),
        latest_request,
        request_id,
    ) else {
        tracing::debug!(
            target: "gdb_sql_perf",
            op = "syntax_tree_refresh",
            result = "cancelled",
            request_id,
            buffer_version = snapshot.version(),
        );
        return false;
    };
    cached.tree = tree;
    tracing::debug!(
        target: "gdb_sql_perf",
        op = "syntax_tree_refresh",
        result = "incremental",
        request_id,
        buffer_version = snapshot.version(),
        old_bytes = changed.old_range.len(),
        new_bytes = changed.new_text.len(),
    );
    true
}

fn parse_statement_highlights(
    text: &str,
    dialect: SqlDialect,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<Vec<Highlight>> {
    use tree_sitter::{Language, Parser};

    let lang = Language::new(tree_sitter_sequel::LANGUAGE);
    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("tree-sitter-sequel language should be valid");
    let parser_text = mysql_parser_text(text, dialect);
    let tree = parse_tree_with_cancellation(
        &mut parser,
        parser_text.as_bytes(),
        None,
        latest_request,
        request_id,
    )?;
    highlight_sql_tree_cancellable(text, tree, dialect, latest_request, request_id)
}

/// MySQL 兼容语法需要改写 parser 输入时，按 statement 解析，避免复制整篇快照。
fn highlight_normalized_statements_snapshot(
    snapshot: &BufferSnapshot,
    dialect: SqlDialect,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<(Vec<Highlight>, Vec<Range>)> {
    let statement_ranges = split_statement_ranges_snapshot(snapshot);
    // 纯注释（如整篇 `--`/`#`/`//`/`/* */` 注释）按 statement 切分会得到空区间，
    // 导致注释完全失去高亮；此时把整篇当作一个 statement 解析，保证注释仍可着色。
    let statement_ranges = if statement_ranges.is_empty() && snapshot.len() > 0 {
        vec![Range::new(0, snapshot.len())]
    } else {
        statement_ranges
    };
    let mut highlights = Vec::new();
    for range in &statement_ranges {
        if latest_request.load(Ordering::Acquire) != request_id {
            return None;
        }
        let text = snapshot.text_in_range(*range);
        let local = parse_statement_highlights(&text, dialect, latest_request, request_id)?;
        highlights.extend(local.into_iter().map(|mut highlight| {
            highlight.range.start = highlight.range.start.saturating_add(range.start);
            highlight.range.end = highlight.range.end.saturating_add(range.start);
            highlight
        }));
    }
    // 语句切分只覆盖含可见 SQL 的区间；语句外或纯注释文档中的注释会被丢弃。
    // 用轻量词法扫描补齐这些注释（含 tree-sitter-sequel 不识别为注释节点的
    // `#`/`//`），确保整篇注释仍可着色。按 range 去重，避免覆盖已解析结果。
    let covered = highlights.iter().map(|h| h.range).collect::<Vec<_>>();
    for token in tokenize_sql_snapshot(snapshot, dialect) {
        if token.kind == "comment" && !covered.contains(&token.range) {
            highlights.push(token);
        }
    }
    Some((highlights, statement_ranges))
}

/// 在局部高亮路径同步更新增量语法树，保留跨 statement 的上下文。
///
/// MySQL 的 `COLLATE=` 归一化会改变 parser 输入，无法直接使用 Rope chunk；该方言
/// 继续走安全 fallback。其它方言直接从 snapshot 提供 Tree-sitter 输入，不复制全文。
#[allow(dead_code)]
fn highlight_sql_tree_sitter_cached(
    text: &str,
    dialect: SqlDialect,
    changed: &InputEdit,
    version: u64,
    cache: &Arc<Mutex<Option<SqlSyntaxCache>>>,
    latest_request: &AtomicU64,
    request_id: u64,
) -> (Vec<Highlight>, &'static str) {
    let snapshot = fluxdb_editor_core::EditorBuffer::new_from(text).snapshot();
    highlight_sql_tree_sitter_cached_snapshot(
        &snapshot,
        dialect,
        changed,
        version,
        cache,
        latest_request,
        request_id,
    )
}

fn highlight_sql_tree_sitter_cached_snapshot(
    snapshot: &BufferSnapshot,
    dialect: SqlDialect,
    changed: &InputEdit,
    version: u64,
    cache: &Arc<Mutex<Option<SqlSyntaxCache>>>,
    latest_request: &AtomicU64,
    request_id: u64,
) -> (Vec<Highlight>, &'static str) {
    use tree_sitter::{InputEdit as TreeInputEdit, Language, Parser};
    let lang = Language::new(tree_sitter_sequel::LANGUAGE);
    let state = match cache.lock() {
        Ok(mut cache) => {
            // 旧请求不能取走更新版本的缓存，否则它完成后可能回写旧树。
            if cache
                .as_ref()
                .map(|cached| cached.version > version)
                .unwrap_or(false)
            {
                None
            } else {
                cache.take()
            }
        }
        Err(_) => None,
    };
    let Some(mut cached) = state else {
        let mut fresh = Parser::new();
        fresh
            .set_language(&lang)
            .expect("tree-sitter-sequel language should be valid");
        if snapshot.len() > TREE_SITTER_FULL_PARSE_LIMIT {
            // 大文档首轮优先给出完整轻量高亮，避免首次 Tree-sitter 查询长时间占用 CPU。
            // 后续单 statement 编辑仍会进入 statement_incremental；结构性编辑再回退全文树。
            let tree = fresh
                .parse("", None)
                .expect("tree-sitter should parse an empty document");
            let highlights = tokenize_sql_snapshot(snapshot, dialect);
            let statement_ranges = split_statement_ranges_snapshot(snapshot);
            store_syntax_cache(
                cache,
                SqlSyntaxCache {
                    parser: fresh,
                    tree,
                    tree_valid: false,
                    // MySQL 大文档首轮走 statement refinement；兼容归一化输入不能安全
                    // 与完整原文 tree 做增量复用，保持 false 会在每次小编辑后误尝试刷新。
                    parser_normalized: matches!(dialect, SqlDialect::Mysql),
                    snapshot: snapshot.clone(),
                    version,
                    highlights: highlights.clone(),
                    statement_highlights: vec![Arc::new(Vec::new()); statement_ranges.len()],
                    changed_subtree_highlights: Vec::new(),
                    statement_ranges,
                    refinement_cursor: 0,
                },
            );
            return (highlights, "tokenizer_large");
        }
        if matches!(dialect, SqlDialect::Mysql)
            && mysql_parser_needs_normalization_snapshot(snapshot)
        {
            let Some((highlights, statement_ranges)) = highlight_normalized_statements_snapshot(
                snapshot,
                dialect,
                latest_request,
                request_id,
            ) else {
                return (Vec::new(), "normalized_statement_cancelled");
            };
            let tree = fresh
                .parse("", None)
                .expect("tree-sitter should parse an empty document");
            store_syntax_cache(
                cache,
                SqlSyntaxCache {
                    parser: fresh,
                    tree,
                    tree_valid: false,
                    parser_normalized: true,
                    snapshot: snapshot.clone(),
                    version,
                    statement_highlights: statement_highlight_cache(&statement_ranges, &highlights),
                    changed_subtree_highlights: Vec::new(),
                    statement_ranges,
                    refinement_cursor: usize::MAX,
                    highlights: highlights.clone(),
                },
            );
            return (highlights, "statement_normalized");
        }
        let tree = parse_snapshot_with_cancellation(
            &mut fresh,
            snapshot,
            None,
            latest_request,
            request_id,
        );
        let Some(tree) = tree else {
            return (Vec::new(), "full_failed");
        };
        let highlights =
            highlight_sql_tree_snapshot_cancellable(snapshot, &tree, latest_request, request_id);
        let Some(highlights) = highlights else {
            return (Vec::new(), "full_cancelled");
        };
        let statement_ranges = split_statement_ranges_snapshot(snapshot);
        let changed_subtree_highlights =
            changed_subtree_highlight_cache(&tree, &highlights);
        store_syntax_cache(
            cache,
            SqlSyntaxCache {
                parser: fresh,
                tree,
                tree_valid: true,
                parser_normalized: false,
                snapshot: snapshot.clone(),
                version,
                highlights: highlights.clone(),
                statement_highlights: statement_highlight_cache(&statement_ranges, &highlights),
                changed_subtree_highlights,
                statement_ranges,
                refinement_cursor: usize::MAX,
            },
        );
        return (highlights, "full");
    };

    // `changed.version` is the post-edit version. Incremental parsing is valid only
    // when the cached tree is exactly the immediately preceding document version.
    let contiguous_edit = cached.version.saturating_add(1) == version
        && changed.version == version
        && changed.old_range.start <= changed.old_range.end
        && changed.old_range.end <= cached.snapshot.len()
        && cached.snapshot.len() >= changed.old_range.end;
    let parser_normalized_snapshot = matches!(dialect, SqlDialect::Mysql)
        && mysql_parser_needs_normalization_snapshot(snapshot);
    let large_refinement = snapshot.len() > TREE_SITTER_FULL_PARSE_LIMIT
        && changed.old_range.is_empty()
        && changed.new_text.is_empty()
        && !cached.tree_valid;
    if parser_normalized_snapshot && !large_refinement {
        let Some((highlights, statement_ranges)) = highlight_normalized_statements_snapshot(
            snapshot,
            dialect,
            latest_request,
            request_id,
        ) else {
            store_syntax_cache(cache, cached);
            return (Vec::new(), "normalized_statement_cancelled");
        };
        cached.tree = cached
            .parser
            .parse("", None)
            .expect("tree-sitter should parse an empty document");
        cached.tree_valid = false;
        cached.parser_normalized = true;
        cached.snapshot = snapshot.clone();
        cached.version = version;
        cached.highlights = highlights.clone();
        cached.statement_ranges = statement_ranges.clone();
        cached.statement_highlights = statement_highlight_cache(&statement_ranges, &highlights);
        cached.changed_subtree_highlights.clear();
        cached.refinement_cursor = usize::MAX;
        store_syntax_cache(cache, cached);
        tracing::debug!(
            target: "gdb_sql_perf",
            op = "syntax_normalized_statements",
            request_id,
            buffer_version = version,
            text_bytes = snapshot.len(),
            statement_count = statement_ranges.len(),
        );
        return (highlights, "statement_normalized");
    }

    // 大文档首轮只返回 tokenizer 结果；后续同版本空变更按 statement 分批精化，
    // 每批控制在约 64KB，UI 可以在批次之间响应新输入，过期请求由 parser 回调取消。
    if snapshot.len() > TREE_SITTER_FULL_PARSE_LIMIT
        && changed.old_range.is_empty()
        && changed.new_text.is_empty()
        && !cached.tree_valid
    {
        if cached.refinement_cursor < cached.statement_ranges.len() {
            let batch_start = cached.refinement_cursor;
            let mut batch_end = batch_start;
            let mut batch_bytes = 0usize;
            while batch_end < cached.statement_ranges.len() {
                let range = cached.statement_ranges[batch_end];
                let next_bytes = batch_bytes.saturating_add(range.len());
                if batch_end > batch_start && next_bytes > TREE_SITTER_REFINEMENT_BATCH_BYTES {
                    break;
                }
                batch_bytes = next_bytes;
                batch_end += 1;
            }
            for (index, range) in cached.statement_ranges[batch_start..batch_end]
                .iter()
                .enumerate()
            {
                let statement_text = snapshot.text_in_range(*range);
                let Some(local) = parse_statement_highlights(
                    &statement_text,
                    dialect,
                    latest_request,
                    request_id,
                ) else {
                    store_syntax_cache(cache, cached);
                    return (Vec::new(), "refinement_cancelled");
                };
                let local = local
                    .into_iter()
                    .map(|mut highlight| {
                        highlight.range.start += range.start;
                        highlight.range.end += range.start;
                        highlight
                    })
                    .collect::<Vec<_>>();
                cached.statement_highlights[batch_start + index] = Arc::new(local.clone());
                merge_incremental_highlights_in_place(
                    &mut cached.highlights,
                    local,
                    *range,
                    range.len(),
                    *range,
                    snapshot.len(),
                );
            }
            cached.refinement_cursor = batch_end;
            cached.snapshot = snapshot.clone();
            cached.version = version;
            let highlights = cached.highlights.clone();
            store_syntax_cache(cache, cached);
            tracing::debug!(
                target: "gdb_sql_perf",
                op = "syntax_refinement_batch",
                request_id,
                buffer_version = version,
                statement_start = batch_start,
                statement_end = batch_end,
                batch_bytes,
            );
            return (highlights, "statement_refinement");
        }

        // MySQL 兼容归一化需要复制整篇输入。大文档已经完成 statement 高亮后，
        // 完整 tree 不是渲染必需品；保留 statement cache，等结构性编辑再走完整 fallback。
        if matches!(dialect, SqlDialect::Mysql) {
            cached.refinement_cursor = usize::MAX;
            cached.snapshot = snapshot.clone();
            cached.version = version;
            let highlights = cached.highlights.clone();
            store_syntax_cache(cache, cached);
            tracing::debug!(
                target: "gdb_sql_perf",
                op = "syntax_refinement_tree_skipped",
                request_id,
                buffer_version = version,
                text_bytes = snapshot.len(),
                reason = "mysql_normalization",
            );
            return (highlights, "refinement_statement_cache");
        }

        // 所有 statement 已完成局部 query，普通方言建立零拷贝完整语法树，供下一次编辑复用。
        let tree = parse_snapshot_with_cancellation(
            &mut cached.parser,
            snapshot,
            None,
            latest_request,
            request_id,
        );
        let Some(tree) = tree else {
            store_syntax_cache(cache, cached);
            return (Vec::new(), "refinement_cancelled");
        };
        cached.tree = tree;
        cached.tree_valid = true;
        cached.parser_normalized = false;
        cached.changed_subtree_highlights =
            changed_subtree_highlight_cache(&cached.tree, &cached.highlights);
        cached.refinement_cursor = usize::MAX;
        cached.snapshot = snapshot.clone();
        cached.version = version;
        let highlights = cached.highlights.clone();
        store_syntax_cache(cache, cached);
        tracing::debug!(
            target: "gdb_sql_perf",
            op = "syntax_refinement_tree",
            request_id,
            buffer_version = version,
            text_bytes = snapshot.len(),
        );
        return (highlights, "refinement_tree");
    }

    if contiguous_edit && !(changed.old_range.is_empty() && changed.new_text.is_empty()) {
        if let Some((old_statement, new_statement)) = local_statement_edit_cached(
            &cached.snapshot,
            snapshot,
            &cached.statement_ranges,
            changed,
        )
        {
            let statement_text = snapshot.text_in_range(new_statement);
            if let Some(local) = parse_statement_highlights(
                &statement_text,
                dialect,
                latest_request,
                request_id,
            ) {
                let local = local
                    .into_iter()
                    .map(|mut highlight| {
                        highlight.range.start += new_statement.start;
                        highlight.range.end += new_statement.start;
                        highlight
                    })
                    .collect::<Vec<_>>();
                if cached.statement_highlights.len() == cached.statement_ranges.len() {
                    if let Some(index) = cached.statement_ranges.iter().position(|range| *range == old_statement) {
                        cached.statement_highlights[index] = Arc::new(local.clone());
                    }
                }
                merge_incremental_highlights_in_place(
                    &mut cached.highlights,
                    local,
                    old_statement,
                    new_statement.len(),
                    new_statement,
                    snapshot.len(),
                );
                let delta = new_statement.len() as isize - old_statement.len() as isize;
                for range in &mut cached.statement_ranges {
                    if *range == old_statement {
                        *range = new_statement;
                    } else if range.start >= old_statement.end {
                        range.start = shifted_offset(range.start, delta).min(snapshot.len());
                        range.end = shifted_offset(range.end, delta).min(snapshot.len());
                    }
                }
                cached.tree_valid = refresh_incremental_tree(
                    &mut cached,
                    snapshot,
                    dialect,
                    changed,
                    latest_request,
                    request_id,
                );
                if cached.tree_valid && !cached.parser_normalized {
                    let previous = cached.changed_subtree_highlights.clone();
                    cached.changed_subtree_highlights = changed_subtree_highlight_cache_reusing(
                        &cached.tree,
                        &cached.highlights,
                        Some(&previous),
                    );
                }
                let merged = cached.highlights.clone();
                cached.snapshot = snapshot.clone();
                cached.version = version;
                store_syntax_cache(cache, cached);
                return (merged, "statement_incremental");
            }
        }
    }

    // 到达这里表示当前快照无需 MySQL parser 归一化；归一化文档已在上面的
    // statement cache 路径返回，因此普通路径始终可以直接消费 Snapshot chunks。
    let parser_normalized = false;
    let can_incremental = contiguous_edit
        && cached.tree_valid
        && cached.parser_normalized == parser_normalized;
    let cache_mode = if can_incremental { "incremental" } else { "full" };
    let tree = if can_incremental {
        let start = cached.snapshot.offset_to_point(changed.old_range.start);
        let old_end = cached.snapshot.offset_to_point(changed.old_range.end);
        let start_position = tree_sitter::Point {
            row: start.row,
            column: start.column,
        };
        let old_end_position = tree_sitter::Point {
            row: old_end.row,
            column: old_end.column,
        };
        let new_end_position = tree_point_after_text(start_position, &changed.new_text);
        cached.tree.edit(&TreeInputEdit {
            start_byte: changed.old_range.start,
            old_end_byte: changed.old_range.end,
            new_end_byte: changed.old_range.start + changed.new_text.len(),
            start_position,
            old_end_position,
            new_end_position,
        });
        let parsed = parse_snapshot_with_cancellation(
            &mut cached.parser,
            snapshot,
            Some(&cached.tree),
            latest_request,
            request_id,
        );
        parsed.or_else(|| {
            parse_snapshot_with_cancellation(
                &mut cached.parser,
                snapshot,
                None,
                latest_request,
                request_id,
            )
        })
    } else {
        parse_snapshot_with_cancellation(
            &mut cached.parser,
            snapshot,
            None,
            latest_request,
            request_id,
        )
    };
    let Some(tree) = tree else {
        return (Vec::new(), "parse_failed");
    };
    let dirty_ranges = dirty_ranges_for_snapshot_edit(snapshot, changed);
    let highlights = if can_incremental && !parser_normalized {
        let subtree_cache = cached.changed_subtree_highlights.clone();
        if let Some(reused) = highlight_from_changed_subtrees(
            snapshot,
            &tree,
            &subtree_cache,
            latest_request,
            request_id,
        ) {
            reused
        } else if dirty_ranges.len() == 1 {
        let dirty = dirty_ranges[0];
        let local = highlight_sql_tree_snapshot_range_cancellable(
            snapshot,
            &tree,
            Some(dirty),
            latest_request,
            request_id,
        )
        .unwrap_or_default();
        merge_incremental_highlights_in_place(
            &mut cached.highlights,
            local,
            changed.old_range,
            changed.new_text.len(),
            dirty,
            snapshot.len(),
        );
        cached.highlights.clone()
        } else {
            let full = highlight_sql_tree_snapshot_cancellable(snapshot, &tree, latest_request, request_id);
            let Some(full) = full else {
                return (Vec::new(), "highlight_cancelled");
            };
            cached.highlights = full.clone();
            full
        }
    } else {
        let full = highlight_sql_tree_snapshot_cancellable(snapshot, &tree, latest_request, request_id);
        let Some(full) = full else {
            return (Vec::new(), "highlight_cancelled");
        };
        cached.highlights = full.clone();
        full
    };
    cached.tree = tree;
    cached.tree_valid = true;
    cached.parser_normalized = parser_normalized;
    cached.snapshot = snapshot.clone();
    cached.version = version;
    cached.statement_ranges = split_statement_ranges_snapshot(snapshot);
    cached.statement_highlights = statement_highlight_cache(&cached.statement_ranges, &highlights);
    if cached.tree_valid && !cached.parser_normalized {
        let previous = cached.changed_subtree_highlights.clone();
        cached.changed_subtree_highlights = changed_subtree_highlight_cache_reusing(
            &cached.tree,
            &highlights,
            Some(&previous),
        );
    }
    store_syntax_cache(cache, cached);
    (highlights, cache_mode)
}

fn store_syntax_cache(cache: &Arc<Mutex<Option<SqlSyntaxCache>>>, value: SqlSyntaxCache) {
    if let Ok(mut slot) = cache.lock() {
        let should_replace = slot
            .as_ref()
            .map(|current| current.version <= value.version)
            .unwrap_or(true);
        if should_replace {
            *slot = Some(value);
        }
    }
}

fn mysql_parser_needs_normalization(text: &str) -> bool {
    mysql_parser_needs_normalization_bytes(text.as_bytes())
}

fn mysql_parser_needs_normalization_bytes(bytes: &[u8]) -> bool {
    bytes.windows(7).any(|word| word.eq_ignore_ascii_case(b"collate"))
        || ["datetime(", "timestamp(", "time(", "date("]
            .iter()
            .any(|marker| {
                bytes
                    .windows(marker.len())
                    .any(|word| word.eq_ignore_ascii_case(marker.as_bytes()))
            })
        || bytes.windows(5).any(|word| word.eq_ignore_ascii_case(b"using"))
        // `#`/`//` 行注释也要走归一化，才能让 parser 把它们当注释而非语法错误。
        || bytes.contains(&b'#')
        || bytes.windows(2).any(|pair| pair == b"//")
}

fn mysql_parser_needs_normalization_snapshot(snapshot: &BufferSnapshot) -> bool {
    const MAX_MARKER_LEN: usize = 10;
    let mut tail = Vec::with_capacity(MAX_MARKER_LEN - 1);
    for chunk in snapshot.text_chunks_in_range(Range::new(0, snapshot.len())) {
        let mut combined = Vec::with_capacity(tail.len() + chunk.len());
        combined.extend_from_slice(&tail);
        combined.extend_from_slice(chunk);
        if mysql_parser_needs_normalization_bytes(&combined) {
            return true;
        }
        tail.clear();
        tail.extend_from_slice(&combined[combined.len().saturating_sub(MAX_MARKER_LEN - 1)..]);
    }
    false
}

fn mysql_parser_text<'a>(
    text: &'a str,
    dialect: SqlDialect,
) -> std::borrow::Cow<'a, str> {
    let needs_mysql_normalization =
        matches!(dialect, SqlDialect::Mysql) && mysql_parser_needs_normalization(text);
    if needs_mysql_normalization {
        std::borrow::Cow::Owned(normalize_mysql_collate_equals(text))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

fn tree_point_after_text(start: tree_sitter::Point, text: &str) -> tree_sitter::Point {
    let newlines = text.bytes().filter(|&byte| byte == b'\n').count();
    if newlines == 0 {
        return tree_sitter::Point {
            row: start.row,
            column: start.column + text.len(),
        };
    }
    tree_sitter::Point {
        row: start.row + newlines,
        column: text.len() - text.rfind('\n').expect("newline was counted") - 1,
    }
}

fn compiled_sql_highlights_query() -> Option<&'static tree_sitter::Query> {
    use std::sync::OnceLock;
    static QUERY: OnceLock<Option<tree_sitter::Query>> = OnceLock::new();
    QUERY
        .get_or_init(|| {
            let lang = tree_sitter::Language::new(tree_sitter_sequel::LANGUAGE);
            tree_sitter::Query::new(&lang, sql_highlights_query()).ok()
        })
        .as_ref()
}

fn highlight_sql_tree(text: &str, tree: tree_sitter::Tree, dialect: SqlDialect) -> Vec<Highlight> {
    highlight_sql_tree_with_byte_range(text, &tree, dialect, None)
}

fn highlight_sql_tree_cancellable(
    text: &str,
    tree: tree_sitter::Tree,
    dialect: SqlDialect,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<Vec<Highlight>> {
    highlight_sql_tree_with_byte_range_cancellable(
        text,
        &tree,
        dialect,
        None,
        Some((latest_request, request_id)),
    )
}

/// 使用 Rope chunk 作为 Tree-sitter 查询输入，避免为节点文本拼接全文字节数组。
fn highlight_sql_tree_snapshot_cancellable(
    snapshot: &BufferSnapshot,
    tree: &tree_sitter::Tree,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<Vec<Highlight>> {
    highlight_sql_tree_snapshot_range_cancellable(snapshot, tree, None, latest_request, request_id)
}

/// 使用 Rope chunk 查询指定 dirty range，避免局部增量高亮阶段拼接全文。
fn highlight_sql_tree_snapshot_range_cancellable(
    snapshot: &BufferSnapshot,
    tree: &tree_sitter::Tree,
    byte_range: Option<Range>,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<Vec<Highlight>> {
    use tree_sitter::{QueryCursor, StreamingIterator};

    let query = compiled_sql_highlights_query()?;
    let mut cursor = QueryCursor::new();
    let byte_range = byte_range.map(|range| {
        Range::new(range.start.min(snapshot.len()), range.end.min(snapshot.len()))
    });
    if let Some(range) = byte_range {
        cursor.set_byte_range(range.start..range.end);
    }
    let query_root = byte_range
        .and_then(|range| {
            (range.start < range.end)
                .then(|| tree.root_node().descendant_for_byte_range(range.start, range.end))
                .flatten()
        })
        .unwrap_or_else(|| tree.root_node());
    let provider = SnapshotTextProvider { snapshot };
    let mut matches = cursor.matches(query, query_root, provider);
    let capacity = byte_range.map(|range| range.len()).unwrap_or(snapshot.len());
    let mut highlights = Vec::with_capacity((capacity / 16).min(1_000_000));
    while let Some(mat) = matches.next() {
        if latest_request.load(Ordering::Acquire) != request_id {
            return None;
        }
        for capture in mat.captures {
            let node = capture.node;
            let kind = match query.capture_names()[capture.index as usize] {
                "comment" => "comment",
                "string" => "string",
                "parameter" => "parameter",
                "boolean" => "boolean",
                "type" => "type",
                "identifier" => "identifier",
                "field" => "field",
                "attribute" => "attribute",
                "variable" => "variable",
                "function" => "function",
                "keyword" => "keyword",
                _ => continue,
            };
            let start = node.start_byte();
            let end = node.end_byte();
            let kind = if kind == "string"
                && is_numeric_snapshot_range(snapshot, Range::new(start, end))
            {
                "number"
            } else {
                kind
            };
            let start = snapshot.clamp_to_char_boundary(start);
            let end = snapshot.clamp_to_char_boundary(end);
            if start < end {
                highlights.push(Highlight {
                    range: Range::new(start, end),
                    kind: kind.into(),
                });
            }
        }
    }
    Some(highlights)
}

fn highlight_sql_tree_with_byte_range(
    text: &str,
    tree: &tree_sitter::Tree,
    dialect: SqlDialect,
    byte_range: Option<Range>,
) -> Vec<Highlight> {
    highlight_sql_tree_with_byte_range_cancellable(text, tree, dialect, byte_range, None)
        .unwrap_or_default()
}

fn highlight_sql_tree_with_byte_range_cancellable(
    text: &str,
    tree: &tree_sitter::Tree,
    dialect: SqlDialect,
    byte_range: Option<Range>,
    cancellation: Option<(&AtomicU64, u64)>,
) -> Option<Vec<Highlight>> {
    use tree_sitter::{QueryCursor, StreamingIterator};

    let Some(query) = compiled_sql_highlights_query() else {
        let range = byte_range.unwrap_or(Range::new(0, text.len()));
        return Some(
            tokenize_sql(text.get(range.start..range.end).unwrap_or(""), dialect)
                .into_iter()
                .map(|mut highlight| {
                    highlight.range.start += range.start;
                    highlight.range.end += range.start;
                    highlight
                })
                .collect(),
        );
    };
    let root = byte_range
        .and_then(|range| {
            (range.start < range.end)
                .then(|| tree.root_node().descendant_for_byte_range(range.start, range.end))
                .flatten()
        })
        .unwrap_or_else(|| tree.root_node());
    // 大文档通常每个 token 约 4-8 字节；预留容量避免结果收集期间反复扩容。
    let capacity_bytes = byte_range
        .map(|range| range.end.saturating_sub(range.start))
        .unwrap_or(text.len());
    let mut highlights: Vec<Highlight> = Vec::with_capacity((capacity_bytes / 6).min(2_000_000));
    let mut cursor = QueryCursor::new();
    if let Some(range) = byte_range {
        cursor.set_byte_range(range.start..range.end);
    }
    let mut matches = cursor.matches(&query, root, text.as_bytes());
    while let Some(mat) = matches.next() {
        if cancellation.is_some_and(|(latest, request)| latest.load(Ordering::Acquire) != request) {
            return None;
        }
        for capture in mat.captures {
            if cancellation.is_some_and(|(latest, request)| latest.load(Ordering::Acquire) != request) {
                return None;
            }
            let node = capture.node;
            // 只取 named 语法节点（跳过匿名/标点），并忽略 helper 标签。
            if !node.is_named() {
                continue;
            }
            let mut kind = match query.capture_names()[capture.index as usize] {
                "comment" => "comment",
                "string" => "string",
                "parameter" => "parameter",
                "boolean" => "boolean",
                "type" => "type",
                "identifier" => "identifier",
                "field" => "field",
                "attribute" => "attribute",
                "variable" => "variable",
                "function" => "function",
                "keyword" => "keyword",
                _ => continue,
            };
            // 字面量统一标注为 string；若其文本是纯数字，则更精确地标注为 number。
            if kind == "string" {
                let start = node.start_byte();
                let end = node.end_byte();
                let seg = text.get(start..end).unwrap_or("");
                if is_numeric_literal(seg) {
                    kind = "number";
                }
            }
            let Some(range) = clamp_range_to_char_boundaries(text, node.byte_range()) else {
                continue;
            };
            let (start, end) = (range.start, range.end);
            // 避免与短捕获重复（如字面量既命中 string 又命中 number 时取更精确的其一），
            // 这里简单保留全部，渲染层按种类覆盖差异。
            highlights.push(Highlight {
                range: Range::new(start, end),
                kind: kind.into(),
            });
        }
    }
    // 语法树在 MySQL 方言扩展或错误恢复时可能漏掉 token；小文档补齐未被
    // 语法捕获覆盖的词法片段。大文档避免无条件再扫一遍全文，保留 tree-sitter
    // 已产出的结果；只有完全没有捕获时才启用 fallback。
    let fallback_needed = highlights.is_empty()
        || (byte_range.is_none() && tree.root_node().has_error() && text.len() <= 256 * 1024);
    if !fallback_needed {
        // tree-sitter-sequel 不把 `//` / `#` 作为 SQL 注释节点（`#` 在归一化里被
        // 替换为空格后更是没有节点）；仅在文档包含该标记时做一次轻量词法补叠，
        // 避免普通 SQL 的额外扫描。范围取自原始 `text`，与归一化前后字节数一致。
        if text.as_bytes().windows(2).any(|pair| pair == b"//")
            || text.as_bytes().contains(&b'#')
        {
            let fallback_comments = tokenize_sql(text, dialect)
                .into_iter()
                .filter(|highlight| {
                    highlight.kind == "comment"
                        && text
                            .get(highlight.range.start..highlight.range.end)
                            .is_some_and(|value| value.starts_with("//") || value.starts_with('#'))
                });
            for highlight in fallback_comments {
                if !highlights.iter().any(|existing| existing.range == highlight.range) {
                    highlights.push(highlight);
                }
            }
        }
        return Some(highlights);
    }
    if let Some(range) = byte_range {
        return Some(tokenize_sql(text.get(range.start..range.end).unwrap_or(""), dialect)
            .into_iter()
            .map(|mut highlight| {
                highlight.range.start += range.start;
                highlight.range.end += range.start;
                highlight
            })
            .collect());
    }
    // 高亮区间与词法 token 都按起点递增，使用单调指针避免长文档 O(n*m) 扫描。
    highlights.sort_unstable_by_key(|highlight| (highlight.range.start, highlight.range.end));
    let mut covered_index = 0;
    for fallback in tokenize_sql(text, dialect) {
        while covered_index < highlights.len()
            && highlights[covered_index].range.end <= fallback.range.start
        {
            covered_index += 1;
        }
        let covered = covered_index < highlights.len()
            && highlights[covered_index].range.start < fallback.range.end;
        if !covered {
            highlights.push(fallback);
        }
    }
    Some(highlights)
}

#[cfg(test)]
fn merge_incremental_highlights(
    previous: &[Highlight],
    local: Vec<Highlight>,
    old_range: Range,
    inserted_len: usize,
    dirty: Range,
    text_len: usize,
) -> Vec<Highlight> {
    let mut merged = previous.to_vec();
    merge_incremental_highlights_in_place(
        &mut merged,
        local,
        old_range,
        inserted_len,
        dirty,
        text_len,
    );
    merged
}

fn merge_incremental_highlights_in_place(
    highlights: &mut Vec<Highlight>,
    local: Vec<Highlight>,
    old_range: Range,
    inserted_len: usize,
    dirty: Range,
    text_len: usize,
) {
    let delta = inserted_len as isize - old_range.len() as isize;
    highlights.retain_mut(|highlight| {
        if highlight.range.end <= old_range.start {
            // unchanged prefix
        } else if highlight.range.start >= old_range.end {
            highlight.range.start = shifted_offset(highlight.range.start, delta).min(text_len);
            highlight.range.end = shifted_offset(highlight.range.end, delta).min(text_len);
        } else {
            return false;
        }
        if highlight.range.end > dirty.start && highlight.range.start < dirty.end {
            return false;
        }
        highlight.range.start < highlight.range.end
    });
    if let Some(local_start) = local.first().map(|highlight| highlight.range.start) {
        let insert_at = highlights.partition_point(|highlight| highlight.range.start < local_start);
        highlights.splice(insert_at..insert_at, local);
    }
}

fn shifted_offset(offset: usize, delta: isize) -> usize {
    if delta >= 0 {
        offset.saturating_add(delta as usize)
    } else {
        offset.saturating_sub((-delta) as usize)
    }
}

/// 对文本做一次 SQL 语法树解析，收集语法错误节点为诊断。
///
/// 返回的每个诊断对应一个 `ERROR`（语法错误）或 `MISSING`（缺符号）节点：
/// 错误严重度，range 为该节点在 buffer 中的字节区间，message 描述出错类型。
/// 语法树无法解析（极少见）时返回空诊断，不抛错。
#[allow(dead_code)] // 保留无方言调用入口，供现有测试与外部适配层复用。
pub fn sql_diagnostics_tree_sitter(text: &str) -> Vec<Diagnostic> {
    sql_diagnostics_tree_sitter_for_dialect(text, SqlDialect::Mysql)
}

fn sql_diagnostics_tree_sitter_for_dialect(text: &str, dialect: SqlDialect) -> Vec<Diagnostic> {
    let latest_request = AtomicU64::new(1);
    sql_diagnostics_tree_sitter_for_dialect_cancellable(
        text,
        dialect,
        &latest_request,
        1,
    )
    .unwrap_or_default()
}

fn sql_diagnostics_tree_sitter_for_dialect_cancellable(
    text: &str,
    dialect: SqlDialect,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<Vec<Diagnostic>> {
    if latest_request.load(Ordering::Acquire) != request_id {
        return None;
    }
    use tree_sitter::{Language, Parser};

    let lang = Language::new(tree_sitter_sequel::LANGUAGE);
    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("tree-sitter-sequel language should be valid");
    // tree-sitter-sequel accepts `COLLATE name`, while MySQL commonly emits
    // `COLLATE=name`. Normalize only this parser spelling and keep byte offsets unchanged.
    let parser_text = if matches!(dialect, SqlDialect::Mysql) {
        std::borrow::Cow::Owned(normalize_mysql_collate_equals(text))
    } else {
        std::borrow::Cow::Borrowed(text)
    };
    let tree = parse_tree_with_cancellation(
        &mut parser,
        parser_text.as_bytes(),
        None,
        latest_request,
        request_id,
    )?;
    Some(collect_tree_diagnostics(&tree, |range| {
        clamp_range_to_char_boundaries(text, range)
    }))
}

/// 对无需 MySQL 归一化的方言，直接从 Rope chunk 驱动 Tree-sitter，避免拼接全文。
pub(crate) fn sql_diagnostics_snapshot_cancellable(
    snapshot: &BufferSnapshot,
    dialect: SqlDialect,
    latest_request: &AtomicU64,
    request_id: u64,
) -> Option<Vec<Diagnostic>> {
    if matches!(dialect, SqlDialect::Mysql) && mysql_parser_needs_normalization_snapshot(snapshot) {
        // 归一化只影响 parser 输入；按 statement 读取快照，避免为 1MB/10MB 文档
        // 创建一份完整副本。每个局部 parser 的诊断范围再平移回文档坐标。
        let mut diagnostics = Vec::new();
        for statement in split_statement_ranges_snapshot(snapshot) {
            if latest_request.load(Ordering::Acquire) != request_id {
                return None;
            }
            let text = snapshot.text_in_range(statement);
            let local = sql_diagnostics_tree_sitter_for_dialect_cancellable(
                &text,
                dialect,
                latest_request,
                request_id,
            )?;
            diagnostics.extend(local.into_iter().map(|mut diagnostic| {
                diagnostic.range.start = diagnostic.range.start.saturating_add(statement.start);
                diagnostic.range.end = diagnostic.range.end.saturating_add(statement.start);
                diagnostic
            }));
        }
        return Some(diagnostics);
    }
    if latest_request.load(Ordering::Acquire) != request_id {
        return None;
    }
    use tree_sitter::{Language, Parser};
    let lang = Language::new(tree_sitter_sequel::LANGUAGE);
    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("tree-sitter-sequel language should be valid");
    let tree = parse_snapshot_with_cancellation(
        &mut parser,
        snapshot,
        None,
        latest_request,
        request_id,
    )?;
    Some(collect_tree_diagnostics(&tree, |range| {
        let start = snapshot.clamp_to_char_boundary(range.start);
        let end = snapshot.clamp_to_char_boundary(range.end);
        (start < end).then_some(Range::new(start, end))
    }))
}

fn collect_tree_diagnostics<F>(tree: &tree_sitter::Tree, mut clamp: F) -> Vec<Diagnostic>
where
    F: FnMut(std::ops::Range<usize>) -> Option<Range>,
{
    let mut out: Vec<Diagnostic> = Vec::new();
    let mut stack: Vec<tree_sitter::Node> = vec![tree.root_node()];
    // 遍历整棵语法树，收集所有 error / missing 节点。
    while let Some(node) = stack.pop() {
        match node.kind() {
            "ERROR" | "MISSING" => {
                let Some(range) = clamp(node.byte_range()) else {
                    continue;
                };
                let (start, end) = (range.start, range.end);
                let message = if node.kind() == "MISSING" {
                    format!("缺少语法符号：{}", node.kind())
                } else {
                    format!("语法错误附近：{}", node.kind())
                };
                out.push(Diagnostic::error(Range::new(start, end), message));
            }
            _ => {}
        }
        // 继续向下遍历子节点，ERROR 的子节点可能是更细的错误点。
        let mut child = node.child(0);
        while let Some(c) = child {
            stack.push(c);
            child = c.next_sibling();
        }
    }
    out
}

fn normalize_mysql_collate_equals(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b'\'' | b'"' | b'`') {
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == quote {
                    if i + 1 < bytes.len() && bytes[i + 1] == quote {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push_str(&text[start..i]);
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push_str(&text[start..i]);
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push_str(&text[start..i]);
            continue;
        }
        // MySQL 也接受 `#` 与 `//` 作为行注释，但 tree-sitter-sequel 只把 `--`
        // 当作行注释，遇到 `#`/`//` 会产出 ERROR 节点、触发诊断红波浪线。
        // 这里把整段行注释替换为等长空格（字节数不变，避免打乱后续诊断/高亮的
        // 字节偏移），使 parser 不再报错；`--` 与 `/* */` 已在上面原样保留为
        // parser 原生的注释节点。
        if bytes[i] == b'#' || (bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/')) {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.extend(std::iter::repeat_n(' ', i - start));
            continue;
        }
        let is_collate = bytes
            .get(i..i + 7)
            .is_some_and(|word| word.eq_ignore_ascii_case(b"collate"))
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
            && (i + 7 == bytes.len()
                || !bytes[i + 7].is_ascii_alphanumeric() && bytes[i + 7] != b'_');
        if is_collate {
            out.push_str(&text[i..i + 7]);
            i += 7;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'=' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        // tree-sitter-sequel 不接受 MySQL 的 `datetime(6)`、
        // `CURRENT_TIMESTAMP(6)` 等时间精度写法；索引的 `USING BTREE` 也不在
        // grammar 的列/约束规则中。只对已知合法的 MySQL 扩展做等长空格归一化，
        // 保持后续诊断和高亮的字节偏移；其它语法错误仍交给 parser 报告。
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let word_start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[word_start..i];
            if word.eq_ignore_ascii_case("using")
                && text[..word_start].trim_end().ends_with(')')
            {
                let mut value_start = i;
                while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
                    value_start += 1;
                }
                let mut value_end = value_start;
                while value_end < bytes.len()
                    && (bytes[value_end].is_ascii_alphanumeric() || bytes[value_end] == b'_')
                {
                    value_end += 1;
                }
                if text[value_start..value_end].eq_ignore_ascii_case("btree") {
                    out.extend(std::iter::repeat_n(' ', value_end - word_start));
                    i = value_end;
                    continue;
                }
            }
            if matches!(
                word.to_ascii_lowercase().as_str(),
                "date" | "time" | "datetime" | "timestamp" | "current_timestamp"
            ) {
                let mut open = i;
                while open < bytes.len() && bytes[open].is_ascii_whitespace() {
                    open += 1;
                }
                if bytes.get(open) == Some(&b'(') {
                    let mut close = open + 1;
                    while close < bytes.len() && bytes[close].is_ascii_digit() {
                        close += 1;
                    }
                    if close > open + 1 && bytes.get(close) == Some(&b')') {
                        out.push_str(&text[word_start..open]);
                        out.extend(std::iter::repeat_n(' ', close - open + 1));
                        i = close + 1;
                        continue;
                    }
                }
            }
            out.push_str(&text[word_start..i]);
            continue;
        }
        let ch = text[i..].chars().next().expect("valid UTF-8");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// 把一次编辑的脏区间扩展到所在行边界（含新增文本带来的行）。
///
/// 返回覆盖「编辑起点所在行首 → 编辑终点所在行末」的区间，供前端只重绘受影响行。
/// 若文本被整段替换（行数变化较大），保守起见返回整文档区间。
#[allow(dead_code)]
pub fn dirty_ranges_for_edit(
    text: &str,
    old_range: &Range,
    new_text: &str,
) -> Vec<Range> {
    let len = text.len();
    let added_newlines = new_text.bytes().filter(|&b| b == b'\n').count();
    let edit_start = floor_char_boundary(text, old_range.start.min(len));
    let edit_end = floor_char_boundary(
        text,
        old_range.end.saturating_add(added_newlines).min(len),
    );
    // 起点：编辑起点所在行的行首。
    let line_start = text[..edit_start]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    // 终点：按旧区间尾部加上新增换行数定位到当前快照中的安全边界，再取行尾。
    let line_end = text[edit_end..]
        .find('\n')
        .map_or(len, |i| edit_end + i);
    // 极端替换（大量新增行 / 替换量覆盖文档过半）时退化为整文档脏区间，
    // 避免脏区间覆盖不足导致漏高亮；常规单行/多行编辑保持在受影响行范围内。
    let replaced_len = old_range.end.saturating_sub(old_range.start);
    if added_newlines > 256 || replaced_len.max(new_text.len()) * 4 > len {
        return vec![Range::new(0, len)];
    }
    vec![Range::new(line_start, line_end)]
}

#[allow(dead_code)]
fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// 对 SQL 文本做一次简单词法高亮。
///
/// 逐字节扫描，识别：
/// - 注释：`--...`、`#...`（行内）、`/*...*/`（块）
/// - 字符串：`'...'`、`"..."`、`` `...` ``（支持双写转义）
/// - 数字
/// - 关键字 / 内置函数（不区分大小写）
///
/// 返回的高亮区间使用字节偏移，与 buffer 偏移一致。
pub fn tokenize_sql(text: &str, dialect: SqlDialect) -> Vec<Highlight> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut highlights = Vec::new();
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        // 行注释 `--`
        if b == b'-' && i + 1 < n && bytes[i + 1] == b'-' {
            let start = i;
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            highlights.push(Highlight {
                range: Range::new(start, i),
                kind: "comment".into(),
            });
            continue;
        }
        // 行注释 `#`
        if b == b'#' {
            let start = i;
            i += 1;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            highlights.push(Highlight {
                range: Range::new(start, i),
                kind: "comment".into(),
            });
            continue;
        }
        // 行注释 `//`（SQL 方言扩展，兼容代码编辑器输入）
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let start = i;
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            highlights.push(Highlight {
                range: Range::new(start, i),
                kind: "comment".into(),
            });
            continue;
        }
        // 块注释 `/* */`
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < n {
                i += 2;
            } else {
                i = n;
            }
            highlights.push(Highlight {
                range: Range::new(start, i),
                kind: "comment".into(),
            });
            continue;
        }
        // 字符串字面量；MySQL 反引号是标识符，不是字符串。
        if b == b'\'' || b == b'"' || b == b'`' {
            let start = i;
            i += 1;
            while i < n {
                if bytes[i] == b {
                    // 双写转义（如 `''`）
                    if i + 1 < n && bytes[i + 1] == b {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            highlights.push(Highlight {
                range: Range::new(start, i),
                kind: if b == b'`' {
                    "identifier".into()
                } else {
                    "string".into()
                },
            });
            continue;
        }
        // 数字
        if b.is_ascii_digit() {
            let start = i;
            while i < n
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'.'
                    || bytes[i] == b'_')
            {
                i += 1;
            }
            highlights.push(Highlight {
                range: Range::new(start, i),
                kind: "number".into(),
            });
            continue;
        }
        // 标识符 / 词：命中关键字则高亮
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < n
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            let word = &text[start..i];
            if dialect.is_keyword(word) {
                let lower = word.to_ascii_lowercase();
                let kind = if dialect
                    .keywords()
                    .iter()
                    .any(|def| def.word == lower && def.kind == CompletionKind::Function)
                {
                    "function"
                } else if matches!(
                    lower.as_str(),
                    "bit"
                        | "binary"
                        | "varbinary"
                        | "char"
                        | "character"
                        | "nchar"
                        | "varchar"
                        | "nvarchar"
                        | "text"
                        | "tinyint"
                        | "smallint"
                        | "mediumint"
                        | "int"
                        | "integer"
                        | "bigint"
                        | "decimal"
                        | "numeric"
                        | "float"
                        | "double"
                        | "date"
                        | "time"
                        | "datetime"
                        | "timestamp"
                        | "json"
                        | "blob"
                        | "enum"
                ) {
                    "type"
                } else {
                    "keyword"
                };
                highlights.push(Highlight {
                    range: Range::new(start, i),
                    kind: kind.into(),
                });
            }
            continue;
        }
        i += 1;
    }
    highlights
}

/// 在不可变 Snapshot 上执行首轮轻量高亮，不拼接完整文档字符串。
///
/// 该路径只用于大文档首屏 fallback；正式 Tree-sitter refinement 仍负责完整语法语义。
/// `Peekable` 跨 chunk 保持注释、字符串和标识符状态，因此 chunk 边界不会改变 token 范围。
pub fn tokenize_sql_snapshot(
    snapshot: &BufferSnapshot,
    dialect: SqlDialect,
) -> Vec<Highlight> {
    let mut bytes = snapshot
        .text_chunks_in_range(Range::new(0, snapshot.len()))
        .flat_map(|chunk| chunk.iter().copied())
        .enumerate()
        .peekable();
    let mut highlights = Vec::new();
    let document_len = snapshot.len();

    while let Some((start, byte)) = bytes.next() {
        if byte == b'-' && bytes.peek().is_some_and(|(_, next)| *next == b'-') {
            bytes.next();
            while bytes.peek().is_some_and(|(_, next)| *next != b'\n') {
                bytes.next();
            }
            let end = bytes.peek().map(|(offset, _)| *offset).unwrap_or(document_len);
            highlights.push(Highlight {
                range: Range::new(start, end),
                kind: "comment".into(),
            });
            continue;
        }
        if byte == b'#' {
            while bytes.peek().is_some_and(|(_, next)| *next != b'\n') {
                bytes.next();
            }
            let end = bytes.peek().map(|(offset, _)| *offset).unwrap_or(document_len);
            highlights.push(Highlight {
                range: Range::new(start, end),
                kind: "comment".into(),
            });
            continue;
        }
        if byte == b'/' && bytes.peek().is_some_and(|(_, next)| *next == b'/') {
            bytes.next();
            while bytes.peek().is_some_and(|(_, next)| *next != b'\n') {
                bytes.next();
            }
            let end = bytes.peek().map(|(offset, _)| *offset).unwrap_or(document_len);
            highlights.push(Highlight {
                range: Range::new(start, end),
                kind: "comment".into(),
            });
            continue;
        }
        if byte == b'/' && bytes.peek().is_some_and(|(_, next)| *next == b'*') {
            bytes.next();
            let mut previous = None;
            let mut end = (start + 2).min(document_len);
            while let Some((offset, next)) = bytes.next() {
                end = offset + 1;
                if previous == Some(b'*') && next == b'/' {
                    break;
                }
                previous = Some(next);
            }
            highlights.push(Highlight {
                range: Range::new(start, end),
                kind: "comment".into(),
            });
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            let mut end = (start + 1).min(document_len);
            while let Some((offset, next)) = bytes.next() {
                end = offset + 1;
                if next != byte {
                    continue;
                }
                if bytes.peek().is_some_and(|(_, following)| *following == byte) {
                    if let Some((offset, _)) = bytes.next() {
                        end = offset + 1;
                    }
                    continue;
                }
                break;
            }
            highlights.push(Highlight {
                range: Range::new(start, end),
                kind: if byte == b'`' {
                    "identifier".into()
                } else {
                    "string".into()
                },
            });
            continue;
        }
        if byte.is_ascii_digit() {
            let mut end = start + 1;
            while bytes.peek().is_some_and(|(_, next)| {
                next.is_ascii_alphanumeric() || *next == b'.' || *next == b'_'
            }) {
                if let Some((offset, _)) = bytes.next() {
                    end = offset + 1;
                }
            }
            highlights.push(Highlight {
                range: Range::new(start, end),
                kind: "number".into(),
            });
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let mut word = vec![byte];
            let mut end = start + 1;
            while bytes.peek().is_some_and(|(_, next)| {
                next.is_ascii_alphanumeric() || *next == b'_' || *next == b'$'
            }) {
                if let Some((offset, next)) = bytes.next() {
                    word.push(next);
                    end = offset + 1;
                }
            }
            let word = String::from_utf8_lossy(&word);
            if dialect.is_keyword(&word) {
                let lower = word.to_ascii_lowercase();
                let kind = if dialect
                    .keywords()
                    .iter()
                    .any(|def| def.word == lower && def.kind == CompletionKind::Function)
                {
                    "function"
                } else if matches!(
                    lower.as_str(),
                    "bit"
                        | "binary"
                        | "varbinary"
                        | "char"
                        | "character"
                        | "nchar"
                        | "varchar"
                        | "nvarchar"
                        | "text"
                        | "tinyint"
                        | "smallint"
                        | "mediumint"
                        | "int"
                        | "integer"
                        | "bigint"
                        | "decimal"
                        | "numeric"
                        | "float"
                        | "double"
                        | "date"
                        | "time"
                        | "datetime"
                        | "timestamp"
                        | "json"
                        | "blob"
                        | "enum"
                ) {
                    "type"
                } else {
                    "keyword"
                };
                highlights.push(Highlight {
                    range: Range::new(start, end),
                    kind: kind.into(),
                });
            }
        }
    }
    highlights
}

/// 便利：把方言包装成语言定义（用于向编辑器注册 provider）。
#[allow(dead_code)] // 语法 provider 接入时用于注册语言定义，保留。
pub fn sql_language(dialect: SqlDialect) -> SqlLanguage {
    SqlLanguage::new(dialect)
}

#[cfg(test)]
mod syntax_tests {
    use super::*;

    fn kinds(text: &str, dialect: SqlDialect) -> Vec<(String, Range)> {
        tokenize_sql(text, dialect)
            .into_iter()
            .map(|h| (h.kind.into_owned(), h.range))
            .collect()
    }

    #[test]
    fn tokens_keyword_string_comment() {
        let text = "select 'abc' -- 注释\n# hash\n// slash\n /* 块 */ from t";
        let k = kinds(text, SqlDialect::Mysql);
        // 断言存在 keyword、string、comment 类型。
        assert!(k.iter().any(|(t, _)| t == "string"));
        assert!(k.iter().any(|(t, _)| t == "comment"));
        assert!(k.iter().any(|(t, _)| t == "keyword"));
        assert!(k.len() >= 3);
    }

    #[test]
    fn snapshot_tokenizer_matches_string_tokenizer_across_chunks() {
        let text = format!(
            "select {} from `table_name` -- comment\n# hash\n/* block */ where id = 42",
            "'value' ".repeat(300)
        );
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(&text).snapshot();
        assert_eq!(
            tokenize_sql_snapshot(&snapshot, SqlDialect::Mysql),
            tokenize_sql(&text, SqlDialect::Mysql)
        );
    }

    #[test]
    fn tokens_slash_comment_is_supported() {
        let text = "SELECT 1 // editor comment\nFROM users";
        let highlights = tokenize_sql(text, SqlDialect::Mysql);
        assert!(highlights.iter().any(|highlight| {
            highlight.kind == "comment"
                && text
                    .get(highlight.range.start..highlight.range.end)
                    .is_some_and(|value| value.starts_with("//"))
        }));
    }

    #[test]
    fn tokens_number() {
        let k = kinds("select 42 from t", SqlDialect::Mysql);
        assert!(k.iter().any(|(t, _)| t == "number"));
    }

    #[test]
    fn keyword_highlight_within_text() {
        let text = "SELECT name FROM users";
        let k = kinds(text, SqlDialect::Mysql);
        // SELECT 与 FROM 都应命中关键字。
        let kw_count = k.iter().filter(|(t, _)| t == "keyword").count();
        assert_eq!(kw_count, 2);
    }

    #[test]
    fn dialect_is_keyword_case_insensitive() {
        assert!(SqlDialect::Postgres.is_keyword("SELECT"));
        assert!(SqlDialect::Postgres.is_keyword("select"));
        assert!(!SqlDialect::Postgres.is_keyword("notakeyword"));
    }

    #[test]
    fn language_id_matches_dialect() {
        let lang = SqlLanguage::new(SqlDialect::Mysql);
        assert_eq!(lang.language_id(), "sql_mysql");
        assert_eq!(lang.line_comment(), Some("--"));
        assert_eq!(lang.block_comment(), Some(("/*", "*/")));
    }

    #[test]
    fn tree_sitter_highlights_common_sql_tokens() {
        let text = "SELECT u.name, COUNT(*) FROM users u -- 注释\nWHERE u.age > 18 AND u.name = 'Alice';";
        let hs = highlight_sql_tree_sitter(text, SqlDialect::Mysql);
        let kinds: Vec<&str> = hs.iter().map(|h| h.kind.as_ref()).collect();
        assert!(kinds.contains(&"keyword"), "expected keyword, got {kinds:?}");
        assert!(kinds.contains(&"string"), "expected string, got {kinds:?}");
        assert!(kinds.contains(&"comment"), "expected comment, got {kinds:?}");
        assert!(kinds.contains(&"function"), "expected function, got {kinds:?}");
        // object_reference 的 name 单独命中 identifier，避免与数据类型共用颜色。
        assert!(kinds.contains(&"identifier"), "expected identifier, got {kinds:?}");
        // 所有区间须落在文本字节范围内。
        for h in &hs {
            assert!(
                h.range.end <= text.len(),
                "range {:?} out of bounds for len {}",
                h.range,
                text.len()
            );
        }
    }

    #[test]
    fn tree_sitter_highlights_number_literal() {
        // 数字字面量经正则匹配为 number，且区间精确覆盖 "42"。
        let text = "SELECT 42 FROM t";
        let hs = highlight_sql_tree_sitter(text, SqlDialect::Mysql);
        let number = hs.iter().find(|h| h.kind == "number");
        assert!(number.is_some(), "expected number highlight: {hs:?}");
        let h = number.unwrap();
        assert_eq!(&text[h.range.start..h.range.end], "42");
    }

    #[test]
    fn tree_sitter_highlights_mysql_create_table_ddl() {
        let text = "CREATE TABLE `t` (\n  `id` varchar(64) CHARACTER NOT NULL DEFAULT '' COMMENT '主键',\n  `created_at` datetime(6) DEFAULT CURRENT_TIMESTAMP,\n  UNIQUE KEY `uniq_product_line_parent_id_application_code` (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_general_ci;";
        let highlights = highlight_sql_tree_sitter(text, SqlDialect::Mysql);
        let kind_for = |needle: &str| {
            highlights.iter().find_map(|highlight| {
                (text.get(highlight.range.start..highlight.range.end) == Some(needle))
                    .then_some(highlight.kind.as_ref())
            })
        };

        assert_eq!(kind_for("`id`"), Some("field"));
        assert_eq!(kind_for("varchar"), Some("type"));
        assert_eq!(kind_for("CHARACTER"), Some("type"));
        assert_eq!(kind_for("DEFAULT"), Some("keyword"));
        assert_eq!(kind_for("COMMENT"), Some("keyword"));
        assert_eq!(kind_for("ENGINE"), Some("keyword"));
        assert_eq!(kind_for("CHARSET"), Some("attribute"));
        assert_eq!(kind_for("CURRENT_TIMESTAMP"), Some("keyword"));
        assert_eq!(kind_for("'主键'"), Some("string"));
        assert_eq!(
            kind_for("`uniq_product_line_parent_id_application_code`"),
            Some("identifier")
        );
    }

    #[test]
    fn cached_tree_sitter_highlighting_reuses_previous_tree() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT name FROM users";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Mysql,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );

        let updated = "SELECT id FROM users";
        let change = InputEdit::new(Range::new(7, 11), "id".to_string(), 1);
        let (highlights, cache_mode) = highlight_sql_tree_sitter_cached(
            updated,
            SqlDialect::Mysql,
            &change,
            1,
            &cache,
            &latest_request,
            1,
        );
        assert_eq!(cache_mode, "statement_incremental");
        assert!(highlights.iter().any(|highlight| {
            highlight.kind == "field"
                && updated
                    .get(highlight.range.start..highlight.range.end)
                    .is_some_and(|value| value == "id")
        }));
        assert_eq!(cache.lock().unwrap().as_ref().map(|state| state.version), Some(1));
    }

    #[test]
    fn normalized_mysql_highlighting_uses_statement_cache() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "CREATE TABLE t (id INT) COLLATE=utf8mb4_general_ci; SELECT id FROM t";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let (highlights, mode) = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Mysql,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );
        assert_eq!(mode, "statement_normalized");
        assert!(highlights.iter().any(|highlight| highlight.kind == "keyword"));
        assert!(!cache.lock().unwrap().as_ref().unwrap().tree_valid);
    }

    #[test]
    fn snapshot_statement_path_avoids_full_document_copy() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT name FROM users;\nSELECT id FROM orders";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Mysql,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );
        let updated = "SELECT user_name FROM users;\nSELECT id FROM orders";
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(updated).snapshot();
        let change = InputEdit::new(Range::new(7, 11), "user_name".to_string(), 1);
        let (highlights, _, mode) = try_local_statement_highlight(
            &snapshot,
            SqlDialect::Mysql,
            &change,
            1,
            &cache,
            &latest_request,
            1,
        )
        .expect("single-statement edit should stay local");
        assert_eq!(mode, "statement_snapshot_incremental");
        assert!(highlights.iter().any(|highlight| {
            highlight.kind == "field"
                && updated
                    .get(highlight.range.start..highlight.range.end)
                    .is_some_and(|value| value == "user_name")
        }));
    }

    #[test]
    fn snapshot_statement_path_advances_cached_document() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT name FROM users;\nSELECT id FROM orders";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Mysql,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );

        let first = "SELECT user_name FROM users;\nSELECT id FROM orders";
        let first_snapshot = fluxdb_editor_core::EditorBuffer::new_from(first).snapshot();
        let first_change = InputEdit::new(Range::new(7, 11), "user_name".to_string(), 1);
        try_local_statement_highlight(
            &first_snapshot,
            SqlDialect::Mysql,
            &first_change,
            1,
            &cache,
            &latest_request,
            1,
        )
        .expect("first edit should stay local");

        let second = "SELECT user_id FROM users;\nSELECT id FROM orders";
        let second_snapshot = fluxdb_editor_core::EditorBuffer::new_from(second).snapshot();
        let second_change = InputEdit::new(Range::new(12, 16), "id".to_string(), 2);
        let second_result = try_local_statement_highlight(
            &second_snapshot,
            SqlDialect::Mysql,
            &second_change,
            2,
            &cache,
            &latest_request,
            1,
        );
        assert!(second_result.is_some(), "second edit cache={:?}", cache.lock().unwrap().as_ref().map(|state| (state.version, state.snapshot.to_string(), state.statement_ranges.clone())));

        let cached = cache.lock().unwrap();
        assert_eq!(
            cached
                .as_ref()
                .map(|state| state.snapshot.to_string()),
            Some(second.to_string())
        );
        assert_eq!(cached.as_ref().map(|state| state.tree_valid), Some(true));
        assert_eq!(
            cached
                .as_ref()
                .and_then(|state| state.statement_ranges.get(1))
                .copied(),
            Some(Range::new(
                second.find("SELECT id FROM orders").unwrap(),
                second.len(),
            ))
        );
    }

    #[test]
    fn large_initial_document_uses_fast_tokenizer_cache() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let text = "SELECT value FROM users;\n".repeat(
            TREE_SITTER_FULL_PARSE_LIMIT / "SELECT value FROM users;\n".len() + 1,
        );
        let change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let (highlights, cache_mode) = highlight_sql_tree_sitter_cached(
            &text,
            SqlDialect::Mysql,
            &change,
            0,
            &cache,
            &latest_request,
            1,
        );
        assert_eq!(cache_mode, "tokenizer_large");
        assert!(!highlights.is_empty());
        assert_eq!(
            cache.lock().unwrap().as_ref().map(|state| state.tree_valid),
            Some(false)
        );
        let refinement = InputEdit::new(Range::new(0, 0), String::new(), 1);
        let mut refinement_mode = "";
        for _ in 0..128 {
            let (_, mode) = highlight_sql_tree_sitter_cached(
                &text,
                SqlDialect::Mysql,
                &refinement,
                1,
                &cache,
                &latest_request,
                1,
            );
            refinement_mode = mode;
            if cache.lock().unwrap().as_ref().map(|state| state.tree_valid) == Some(true) {
                break;
            }
        }
        assert_eq!(refinement_mode, "refinement_statement_cache");
        assert_eq!(
            cache.lock().unwrap().as_ref().map(|state| state.tree_valid),
            Some(false)
        );
    }

    #[test]
    fn statement_subtree_cache_reuses_unedited_statement_arc() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT name FROM users;\nSELECT id FROM orders";
        let change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Postgres,
            &change,
            0,
            &cache,
            &latest_request,
            1,
        );
        let before = cache
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .statement_highlights[1]
            .clone();
        let updated = "SELECT user_name FROM users;\nSELECT id FROM orders";
        let edit = InputEdit::new(Range::new(7, 11), "user_name".to_string(), 1);
        let _ = highlight_sql_tree_sitter_cached(
            updated,
            SqlDialect::Postgres,
            &edit,
            1,
            &cache,
            &latest_request,
            1,
        );
        let after = cache
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .statement_highlights[1]
            .clone();
        assert!(Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn changed_subtree_cache_reuses_tree_sitter_sibling_arc() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT name FROM users;\nSELECT id FROM orders";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Postgres,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );
        let before = {
            let state = cache.lock().unwrap();
            let state = state.as_ref().unwrap();
            state.changed_subtree_highlights.get(1).unwrap().highlights.clone()
        };
        let updated = "SELECT user_name FROM users;\nSELECT id FROM orders";
        let edit = InputEdit::new(Range::new(7, 11), "user_name".to_string(), 1);
        let _ = highlight_sql_tree_sitter_cached(
            updated,
            SqlDialect::Postgres,
            &edit,
            1,
            &cache,
            &latest_request,
            1,
        );
        let state = cache.lock().unwrap();
        let state = state.as_ref().unwrap();
        let after = &state.changed_subtree_highlights.get(1).unwrap().highlights;
        assert!(Arc::ptr_eq(&before, after));
    }

    #[test]
    fn changed_subtree_cache_keeps_unchanged_nested_child_arc() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT name FROM users WHERE id = 1";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Postgres,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );
        let before = cache.lock().unwrap().as_ref().unwrap().changed_subtree_highlights[0]
            .children.last().map(|child| child.highlights.clone())
            .expect("statement cache should contain nested nodes");
        let updated = "SELECT idxx FROM users WHERE id = 1";
        let edit = InputEdit::new(Range::new(7, 11), "idxx".to_string(), 1);
        let _ = highlight_sql_tree_sitter_cached(
            updated,
            SqlDialect::Postgres,
            &edit,
            1,
            &cache,
            &latest_request,
            1,
        );
        let state = cache.lock().unwrap();
        let after = state.as_ref().unwrap().changed_subtree_highlights[0]
            .children.last().map(|child| child.highlights.clone())
            .expect("nested cache should remain available");
        assert!(Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn statement_incremental_path_preserves_suffix_highlights() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT name FROM users;\nSELECT id FROM orders";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Mysql,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );

        let updated = "SELECT user_name FROM users;\nSELECT id FROM orders";
        let change = InputEdit::new(Range::new(7, 11), "user_name".to_string(), 1);
        let (highlights, cache_mode) = highlight_sql_tree_sitter_cached(
            updated,
            SqlDialect::Mysql,
            &change,
            1,
            &cache,
            &latest_request,
            1,
        );
        assert_eq!(cache_mode, "statement_incremental");
        assert!(highlights.iter().any(|highlight| {
            highlight.kind == "field"
                && updated
                    .get(highlight.range.start..highlight.range.end)
                    == Some("user_name")
        }));
        assert!(highlights.iter().any(|highlight| {
            updated
                .get(highlight.range.start..highlight.range.end)
                == Some("orders")
        }));
    }

    #[test]
    fn statement_incremental_path_falls_back_when_edit_merges_statements() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT 1; SELECT 2";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Mysql,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );

        let updated = "SELECT 1 SELECT 2";
        let change = InputEdit::new(Range::new(8, 9), String::new(), 1);
        let (_, cache_mode) = highlight_sql_tree_sitter_cached(
            updated,
            SqlDialect::Mysql,
            &change,
            1,
            &cache,
            &latest_request,
            1,
        );
        assert_ne!(cache_mode, "statement_incremental");
    }

    #[test]
    fn statement_window_incremental_path_handles_semicolon_edit() {
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let initial = "SELECT 1; SELECT 2; SELECT 3";
        let initial_change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let _ = highlight_sql_tree_sitter_cached(
            initial,
            SqlDialect::Mysql,
            &initial_change,
            0,
            &cache,
            &latest_request,
            1,
        );
        let updated = "SELECT 1 SELECT 2; SELECT 3";
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(updated).snapshot();
        let change = InputEdit::new(Range::new(8, 9), String::new(), 1);
        let (highlights, _, mode) = try_local_statement_window_highlight(
            &snapshot,
            SqlDialect::Mysql,
            &change,
            1,
            &cache,
            &latest_request,
            1,
        )
        .expect("semicolon edit should stay in a local statement window");
        assert_eq!(mode, "statement_window_incremental");
        assert!(highlights.iter().any(|highlight| {
            updated.get(highlight.range.start..highlight.range.end) == Some("SELECT")
        }));
    }

    #[test]
    fn incremental_highlights_shift_suffix_without_rehighlighting_it() {
        let previous = vec![
            Highlight {
                range: Range::new(0, 6),
                kind: "keyword".into(),
            },
            Highlight {
                range: Range::new(10, 14),
                kind: "identifier".into(),
            },
        ];
        let merged = merge_incremental_highlights(
            &previous,
            vec![Highlight {
                range: Range::new(6, 9),
                kind: "identifier".into(),
            }],
            Range::new(6, 6),
            3,
            Range::new(6, 9),
            17,
        );
        assert!(merged.iter().any(|h| h.range == Range::new(13, 17)));
        assert!(merged.iter().any(|h| h.range == Range::new(6, 9)));
    }

    #[test]
    fn diagnostics_flag_grammar_errors() {
        use fluxdb_editor_core::DiagnosticSeverity;
        // 合法语句无诊断。
        let ok = sql_diagnostics_tree_sitter("SELECT name FROM users WHERE id = 1;");
        assert!(ok.is_empty(), "expected no diagnostics, got {ok:?}");
        let mysql_ddl = sql_diagnostics_tree_sitter_for_dialect(
            "CREATE TABLE t (id INT NOT NULL) ENGINE=InnoDB COLLATE=utf8mb4_general_ci;",
            SqlDialect::Mysql,
        );
        assert!(mysql_ddl.is_empty(), "expected MySQL DDL to parse, got {mysql_ddl:?}");
        // 明显残缺（未闭合字符串）应产生至少一个错误诊断，且区间在界内。
        let text = "SELECT 'unterminated FROM t;";
        let bad = sql_diagnostics_tree_sitter(text);
        assert!(!bad.is_empty(), "expected diagnostics for unterminated string");
        for d in &bad {
            assert!(
                d.range.end <= text.len(),
                "range {:?} out of bounds for len {}",
                d.range,
                text.len()
            );
            assert_eq!(d.severity, DiagnosticSeverity::Error);
            assert!(!d.message.is_empty());
        }
    }

    #[test]
    fn diagnostics_parser_stops_for_stale_request() {
        let latest_request = AtomicU64::new(2);
        let text = "SELECT id FROM users WHERE id = 1;\n".repeat(4096);
        let result = sql_diagnostics_tree_sitter_for_dialect_cancellable(
            &text,
            SqlDialect::Mysql,
            &latest_request,
            1,
        );
        assert!(result.is_none());
    }

    #[test]
    fn diagnostics_snapshot_uses_chunked_parser_input() {
        let text = "SELECT id FROM users;\n".repeat(128);
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(&text).snapshot();
        let latest_request = AtomicU64::new(1);
        let result = sql_diagnostics_snapshot_cancellable(
            &snapshot,
            SqlDialect::Postgres,
            &latest_request,
            1,
        )
        .expect("chunked snapshot parser should complete");
        assert!(result.is_empty());
        assert!(result.iter().all(|diagnostic| diagnostic.range.end <= snapshot.len()));
    }

    #[test]
    fn mysql_snapshot_diagnostics_normalize_per_statement() {
        let text = concat!(
            "CREATE TABLE first (id INT) COLLATE=utf8mb4_general_ci;\n",
            "SELECT 'unterminated FROM second;\n",
        );
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(text).snapshot();
        let latest_request = AtomicU64::new(1);
        let diagnostics = sql_diagnostics_snapshot_cancellable(
            &snapshot,
            SqlDialect::Mysql,
            &latest_request,
            1,
        )
        .expect("diagnostic request should complete");
        assert!(!diagnostics.is_empty());
        assert!(diagnostics.iter().all(|diagnostic| diagnostic.range.end <= text.len()));
        assert!(diagnostics.iter().all(|diagnostic| diagnostic.range.start >= text.find("SELECT").unwrap()));
    }

    #[test]
    fn mysql_normalization_probe_reads_across_snapshot_chunks() {
        let text = format!("{}collate=utf8mb4_general_ci", "x".repeat(1022));
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(&text).snapshot();
        assert!(mysql_parser_needs_normalization_snapshot(&snapshot));

        let plain = fluxdb_editor_core::EditorBuffer::new_from("SELECT 1;").snapshot();
        assert!(!mysql_parser_needs_normalization_snapshot(&plain));
    }

    #[test]
    fn tree_sitter_highlight_query_reads_snapshot_chunks() {
        use tree_sitter::{Language, Parser};

        let text = "SELECT value FROM users WHERE id = 42";
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(text).snapshot();
        let lang = Language::new(tree_sitter_sequel::LANGUAGE);
        let mut parser = Parser::new();
        parser.set_language(&lang).expect("valid SQL grammar");
        let latest = AtomicU64::new(1);
        let tree = parse_snapshot_with_cancellation(&mut parser, &snapshot, None, &latest, 1)
            .expect("snapshot parser should produce a tree");
        let highlights = highlight_sql_tree_snapshot_cancellable(&snapshot, &tree, &latest, 1)
            .expect("highlight query should complete");
        assert!(highlights.iter().any(|highlight| {
            highlight.kind == "keyword"
                && text.get(highlight.range.start..highlight.range.end) == Some("SELECT")
        }));
        assert!(highlights.iter().any(|highlight| {
            highlight.kind == "number"
                && text.get(highlight.range.start..highlight.range.end) == Some("42")
        }));
    }

    #[test]
    fn numeric_snapshot_range_does_not_materialize_text() {
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("-42.5").snapshot();
        assert!(is_numeric_snapshot_range(&snapshot, Range::new(0, snapshot.len())));
        let invalid = fluxdb_editor_core::EditorBuffer::new_from("42x").snapshot();
        assert!(!is_numeric_snapshot_range(&invalid, Range::new(0, invalid.len())));
    }

    #[test]
    fn highlight_query_stops_for_stale_request() {
        let latest_request = AtomicU64::new(2);
        let text = "SELECT id FROM users WHERE id = 1;\n".repeat(4096);
        let language = tree_sitter::Language::new(tree_sitter_sequel::LANGUAGE);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).expect("SQL grammar");
        let tree = parser.parse(text.as_bytes(), None).expect("SQL tree");
        let result = highlight_sql_tree_cancellable(&text, tree, SqlDialect::Mysql, &latest_request, 1);
        assert!(result.is_none());
    }

    /// 四种注释（`--`/`/* */`/`#`/`//`）在诊断层都不应产生错误红线。
    ///
    /// `#` 与 `//` 是方言扩展注释，tree-sitter-sequel 只认 `--`；它们经归一化
    /// 替换为等长空格后进入 parser，不再触发 ERROR 节点。`--` 与 `/* */` 仍是
    /// parser 原生注释节点，同样不应报错。
    #[test]
    fn diagnostics_accept_all_comment_styles() {
        let snapshot = |text: &str| fluxdb_editor_core::EditorBuffer::new_from(text).snapshot();
        let mysql_diag = |text: &str| {
            sql_diagnostics_snapshot_cancellable(
                &snapshot(text),
                SqlDialect::Mysql,
                &AtomicU64::new(1),
                1,
            )
            .expect("diagnostic request should complete")
        };
        let cases = [
            ("-- dash line comment", "-- pick a comment\nSELECT 1;"),
            ("/* block comment */", "/* pick a block */\nSELECT 1;"),
            ("# hash line comment", "# pick a comment\nSELECT 1;"),
            ("// slash line comment", "// pick a comment\nSELECT 1;"),
            ("# empty line", "#\nSELECT 1;"),
            ("// empty line", "//\nSELECT 1;"),
            ("# trailing comment", "SELECT 1; # trailing"),
        ];
        for (name, text) in cases {
            let diagnostics = mysql_diag(text);
            assert!(
                diagnostics.is_empty(),
                "{name}: expected no diagnostics, got {diagnostics:?} for {text:?}"
            );
        }
    }

    /// 归一化必须保持字节偏移：`#` 注释行后的真实语法错误仍要在原位置检出。
    #[test]
    fn diagnostics_keep_byte_offset_after_hash_comment() {
        let text = "# note\nSELECT fro\nm t;";
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(text).snapshot();
        let latest_request = AtomicU64::new(1);
        let diagnostics = sql_diagnostics_snapshot_cancellable(
            &snapshot,
            SqlDialect::Mysql,
            &latest_request,
            1,
        )
        .expect("diagnostic request should complete");
        assert!(
            !diagnostics.is_empty(),
            "a genuine error after a # comment must still be reported"
        );
        // 对照不含注释行的真实错误位置（SELECT fro\nm t; → fro\nm 换行），
        // 注释行占 7 字节（`# note\n`），真实错误字节区间应整体后移 7 且长度不变。
        let plain_start = "SELECT fro\nm t;".find('t').unwrap();
        assert_eq!(diagnostics[0].range.start, plain_start + 7);
    }

    #[test]
    #[test]


    #[test]


    #[test]
    fn cached_highlight_covers_all_comment_styles() {
        // 整篇由注释构成（无任何可见 SQL / `;`）时，statement 切分会得到空区间，
        // 旧逻辑因此让注释完全失去高亮；此处回归保证 4 种注释都被标注为 comment。
        let text = "-- dd \n# ds \n/*  dd */\n// dsjd\n";
        let cache = Arc::new(Mutex::new(None));
        let latest_request = AtomicU64::new(1);
        let change = InputEdit::new(Range::new(0, 0), String::new(), 0);
        let (highlights, _) = highlight_sql_tree_sitter_cached(
            text,
            SqlDialect::Mysql,
            &change,
            0,
            &cache,
            &latest_request,
            1,
        );
        let comments: Vec<&str> = highlights
            .iter()
            .filter(|h| h.kind == "comment")
            .map(|h| text.get(h.range.start..h.range.end).unwrap_or(""))
            .collect();
        for expected in ["-- dd ", "# ds ", "/*  dd */", "// dsjd"] {
            assert!(
                comments.iter().any(|got| got.starts_with(expected.trim())),
                "缺少注释高亮: {expected:?}，得到: {comments:?}"
            );
        }
        // 与普通语句混排时，注释也应保留。
        let mixed = "SELECT 1; -- tail\n# hash\n";
        let (highlights, _) = highlight_sql_tree_sitter_cached(
            mixed,
            SqlDialect::Mysql,
            &change,
            0,
            &cache,
            &latest_request,
            1,
        );
        assert!(highlights.iter().any(|h| h.kind == "comment"));
    }

    fn mysql_diagnostic_normalization_handles_chinese_ddl() {
        let ddl = "CREATE TABLE `test` (`id` VARCHAR(64) COMMENT '主键id') COLLATE=utf8mb4_general_ci;";
        let normalized = normalize_mysql_collate_equals(ddl);

        assert_eq!(normalized.len(), ddl.len());
        assert!(normalized.contains("COMMENT '主键id'"));
        assert!(normalized.contains("COLLATE utf8mb4_general_ci"));
    }

    #[test]
    fn mysql_diagnostic_normalization_accepts_common_mysql_ddl_extensions() {
        let ddl = concat!(
            "CREATE TABLE `shining_implant_part_db_version` (",
            "`id` VARCHAR(64) NOT NULL DEFAULT '' COMMENT '主键',",
            "`create_on` DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) COMMENT '创建时间',",
            "`db_type` VARCHAR(32) NOT NULL DEFAULT '' COMMENT '数据库类型',",
            "`version_no` VARCHAR(32) NOT NULL DEFAULT '' COMMENT '版本号',",
            "`deleted_at` BIGINT UNSIGNED NOT NULL DEFAULT 0 COMMENT '删除时间戳',",
            "PRIMARY KEY (`id`) USING BTREE,",
            "UNIQUE KEY `uniq_type_sort` (`db_type`, `version_no`, `deleted_at`) USING BTREE,",
            "KEY `idx_type_sort` (`db_type`, `deleted_at`) USING BTREE",
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_general_ci;",
        );
        let normalized = normalize_mysql_collate_equals(ddl);
        assert_eq!(normalized.len(), ddl.len());
        assert!(normalized.contains("CURRENT_TIMESTAMP "));
        assert!(!normalized.contains("CURRENT_TIMESTAMP(6)"));
        assert!(!normalized.contains("USING BTREE"));

        let diagnostics = sql_diagnostics_tree_sitter_for_dialect(&ddl, SqlDialect::Mysql);
        assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
    }

    #[test]
    fn mysql_diagnostic_normalization_keeps_invalid_primary_key_syntax() {
        let ddl = "CREATE TABLE `broken` (`id` INT, PRIMARY KEY `id` USING BTREE);";
        let diagnostics = sql_diagnostics_tree_sitter_for_dialect(ddl, SqlDialect::Mysql);
        assert!(!diagnostics.is_empty(), "invalid primary key syntax was hidden");

        let query = "SELECT id FROM t USING BTREE;";
        let diagnostics = sql_diagnostics_tree_sitter_for_dialect(query, SqlDialect::Mysql);
        assert!(!diagnostics.is_empty(), "invalid USING BTREE syntax was hidden");
    }

    #[test]
    fn char_boundary_clamp_expands_partial_utf8_ranges_safely() {
        let text = "会";
        assert_eq!(
            clamp_range_to_char_boundaries(text, 1..4),
            Some(Range::new(0, 3))
        );
        assert_eq!(
            clamp_range_to_char_boundaries(text, 1..1),
            Some(Range::new(0, 3))
        );
    }

    #[test]
    fn dirty_ranges_span_edited_line() {
        let text = "SELECT 1;\nSELECT 2;\nSELECT 3;\n";
        // 编辑第一行中间（替换 "1" 为 "999"）-> 脏区间应覆盖第一行整行。
        let dr = dirty_ranges_for_edit(text, &Range::new(7, 8), "999");
        assert_eq!(dr.len(), 1);
        assert_eq!(&text[dr[0].start..dr[0].end], "SELECT 1;");
        // 整文档替换 -> 保守退化为整文档脏区间。
        let dr_full = dirty_ranges_for_edit(text, &Range::new(0, 0), "SELECT 9;\nSELECT 8;\n");
        assert_eq!(dr_full, vec![Range::new(0, text.len())]);
    }

    #[test]
    fn dirty_ranges_handle_chinese_byte_offsets() {
        let text = "搜索到的订单\nSELECT 1;";
        // 8 落在“环”的 UTF-8 字节中间；计算脏区间时应自动回退到字符边界。
        let ranges = dirty_ranges_for_edit(text, &Range::new(8, 8), "环");
        assert_eq!(ranges, vec![Range::new(0, "搜索到的订单".len())]);
    }

    #[test]
    fn sql_fold_ranges_cover_only_multiline_statements() {
        let lang = SqlLanguage::new(SqlDialect::Mysql);
        // 三条语句：第 1/3 条跨行，第 2 条单行。只应折叠跨行语句。
        let text = "SELECT a,\n  b\nFROM t;\nSELECT 2;\nUPDATE u\nSET x=1\nWHERE id=2;";
        let snap = fluxdb_editor_core::EditorBuffer::new_from(text).snapshot();
        let ranges = lang.fold_ranges(&snap);
        // 跨行语句 = 2 个折叠区间（第 1、3 条）。
        assert_eq!(ranges.len(), 2, "got {ranges:?}");
        for r in &ranges {
            // 每个折叠区间的行跨度 >= 2（由首末行号差值体现，这里直接校验字节非空跨行）。
            let start_row = snap.offset_to_point(r.start).row;
            let end_row = snap.offset_to_point(r.end.saturating_sub(1)).row;
            assert!(end_row > start_row, "fold must span >=2 lines: {r:?}");
            // 单行语句（"SELECT 2;"）不应被包含进任何折叠区间内部首行。
            assert!(r.end <= text.len());
        }
        // 纯单行文档不应产生任何折叠。
        let one_line = fluxdb_editor_core::EditorBuffer::new_from("SELECT 1; SELECT 2;").snapshot();
        assert!(lang.fold_ranges(&one_line).is_empty());
    }

    #[test]
    #[ignore = "manual large-document syntax benchmark"]
    fn large_sql_highlight_benchmark() {
        use std::time::Instant;

        let line = "SELECT id, name FROM users WHERE id = 42;\n";
        for target_bytes in [1024 * 1024, 10 * 1024 * 1024] {
            let text = line.repeat(target_bytes / line.len() + 1);
            let started = Instant::now();
            let highlights = highlight_sql_tree_sitter(&text, SqlDialect::Mysql);
            eprintln!(
                "SQL highlight benchmark: bytes={} lines={} highlights={} elapsed_ms={:.3}",
                text.len(),
                text.lines().count(),
                highlights.len(),
                started.elapsed().as_secs_f64() * 1000.0,
            );
            assert!(!highlights.is_empty());
        }
    }

    #[test]
    #[ignore = "manual large-document parser benchmark"]
    fn large_sql_parser_benchmark() {
        use std::time::Instant;
        use tree_sitter::{Language, Parser};

        let line = "SELECT id, name FROM users WHERE id = 42;\n";
        let text = line.repeat(1024 * 1024 / line.len() + 1);
        let language = Language::new(tree_sitter_sequel::LANGUAGE);
        let mut parser = Parser::new();
        parser.set_language(&language).expect("SQL grammar");
        let started = Instant::now();
        let tree = parser.parse(text.as_bytes(), None);
        eprintln!(
            "SQL parse benchmark: bytes={} elapsed_ms={:.3} has_error={}",
            text.len(),
            started.elapsed().as_secs_f64() * 1000.0,
            tree.as_ref().is_some_and(|tree| tree.root_node().has_error()),
        );
        assert!(tree.is_some());
    }

    #[test]
    #[ignore = "manual large-document tokenizer benchmark"]
    fn large_sql_tokenizer_benchmark() {
        use std::time::Instant;

        let line = "SELECT id, name FROM users WHERE id = 42;\n";
        let text = line.repeat(10 * 1024 * 1024 / line.len() + 1);
        let started = Instant::now();
        let highlights = tokenize_sql(&text, SqlDialect::Mysql);
        eprintln!(
            "SQL tokenize benchmark: bytes={} highlights={} elapsed_ms={:.3}",
            text.len(),
            highlights.len(),
            started.elapsed().as_secs_f64() * 1000.0,
        );
        assert!(!highlights.is_empty());
    }
}
