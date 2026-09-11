//! 编辑器内核的纯逻辑领域模型。
//!
//! 本模块只包含与业务解耦的通用编辑类型：位置、选区、文本变更、事件、
//! 补全/诊断/装饰/执行等协议。不依赖 GPUI、数据库驱动或任何业务类型。

use std::fmt;

use crate::buffer::BufferSnapshot;
use crate::completion::CompletionContinuation;

/// 字节偏移与行/列坐标。
///
/// - `Offset` 以 UTF-8 字节偏移为底层不变量。
/// - `Point` 的 `column` 按字节计数（与 Zed 的 UTF-8 坐标一致）；需要
///   UTF-16 列时使用 [`BufferSnapshot::utf16_column_at`] 等转换方法。
pub type Offset = usize;

/// 行、列坐标。`column` 按字节计数，`row` 从 0 开始。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}

impl Point {
    pub const fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }

    pub fn is_zero(&self) -> bool {
        self.row == 0 && self.column == 0
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.row, self.column)
    }
}

/// `[start, end)` 半开区间，按字节偏移。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Offset,
    pub end: Offset,
}

impl Range {
    pub const fn new(start: Offset, end: Offset) -> Self {
        Self { start, end }
    }

    pub const fn empty(point: Offset) -> Self {
        Self {
            start: point,
            end: point,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, offset: Offset) -> bool {
        offset >= self.start && offset < self.end
    }

    pub fn contains_inclusive(&self, offset: Offset) -> bool {
        offset >= self.start && offset <= self.end
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// 按给定偏置排序（start <= end 的规范化形式）。
    pub fn sorted(&self) -> Range {
        if self.start <= self.end {
            *self
        } else {
            Range::new(self.end, self.start)
        }
    }

    pub fn intersection(&self, other: &Range) -> Option<Range> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if start <= end {
            Some(Range::new(start, end))
        } else {
            None
        }
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}

/// 锚点：某个偏移的稳定描述。
///
/// 锚点绑定字节偏移 + [`Bias`]（重叠点偏向）。调用方（display map、SQL adapter）
/// 通过 [`Anchor::relocate`] 按每次编辑把锚点重定位到最新文本上，从而在编辑发生在
/// 折叠/inlay/block 之前或范围内时不漂移。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Anchor {
    pub offset: Offset,
    pub bias: Bias,
}

impl Anchor {
    pub const fn new(offset: Offset, bias: Bias) -> Self {
        Self { offset, bias }
    }

    /// 把锚点按一次编辑重定位（DM-105）。`old_range` 为被替换区间，`new_len` 为
    /// 新文本字节长度。
    ///
    /// 语义：
    /// - 编辑区间之前（`offset < start`）不变；`offset == start && bias == Left`
    ///   也不变（锚点在插入点左边）；
    /// - 删/替换区间起点边界按 bias 吸附；区间内部按 bias 吸附到新文本起点或终点；
    /// - 编辑区间终点上（`offset == end`）映射到新文本终点；
    /// - 区间之后整体平移 `new_len - old_len`（含单位）：即后续内容前移/后移。
    ///
    /// 返回的偏移可能越过最新缓冲区长度（见 `usize` 平移）；调用方应再
    /// `clamp` 到字符边界，版本不连续时可视作显式重建信号。
    pub fn relocate(self, old_range: Range, new_len: usize) -> Anchor {
        let Range { start, end } = old_range;
        match self.offset.cmp(&start) {
            std::cmp::Ordering::Less => self,
            std::cmp::Ordering::Equal => match self.bias {
                Bias::Left => self,
                Bias::Right => Anchor::new(start + new_len, Bias::Right),
            },
            std::cmp::Ordering::Greater => {
                if self.offset < end {
                    return match self.bias {
                        Bias::Left => Anchor::new(start, Bias::Left),
                        Bias::Right => Anchor::new(start + new_len, Bias::Right),
                    };
                }
                if self.offset == end {
                    return Anchor::new(start + new_len, self.bias);
                }
                let delta = new_len as isize - (end - start) as isize;
                let shifted = (self.offset as isize + delta).max(0) as usize;
                Anchor::new(shifted, self.bias)
            }
        }
    }
}

/// 重叠点锚点的偏向：向左（前一个字符）或向右（后一个字符）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bias {
    Left,
    #[default]
    Right,
}

/// 由两个锚点界定的区间（`[start, end)`）。锚点按编辑自动重定位，
/// 用于折叠/block/inlay 等需要跨编辑保持绑定的持久范围（DM-105）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AnchorRange {
    pub start: Anchor,
    pub end: Anchor,
}

impl AnchorRange {
    pub const fn new(start: Anchor, end: Anchor) -> Self {
        Self { start, end }
    }

    /// 把整个区间按一次编辑重定位（见 [`Anchor::relocate`]）。
    pub fn relocate(self, old_range: Range, new_len: usize) -> AnchorRange {
        AnchorRange {
            start: self.start.relocate(old_range, new_len),
            end: self.end.relocate(old_range, new_len),
        }
    }
}

/// 文本选区。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Selection {
    /// 选区的起点（锚点）。
    pub anchor: Offset,
    /// 光标位置（焦点）。
    pub cursor: Offset,
}

impl Selection {
    pub const fn new(anchor: Offset, cursor: Offset) -> Self {
        Self { anchor, cursor }
    }

    pub const fn point(cursor: Offset) -> Self {
        Self {
            anchor: cursor,
            cursor,
        }
    }

    /// 返回规范化的 `[start, end)` 区间（不含 fold bias）。
    pub fn range(&self) -> Range {
        Range {
            start: self.anchor.min(self.cursor),
            end: self.anchor.max(self.cursor),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}

/// 单次文本变更。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChange {
    /// 被替换的旧区间。
    pub old_range: Range,
    /// 替换后的新文本。
    pub new_text: String,
    /// 变更后的 buffer 版本号。
    pub version: u64,
    /// 是否表示用 `new_text` 替换整个文档；用于撤销/重做等非增量编辑同步。
    pub full_document: bool,
}

impl TextChange {
    pub fn new(old_range: Range, new_text: String, version: u64) -> Self {
        Self {
            old_range,
            new_text,
            version,
            full_document: false,
        }
    }

    pub fn full_document(new_text: String, version: u64) -> Self {
        Self {
            old_range: Range::new(0, 0),
            new_text,
            version,
            full_document: true,
        }
    }
}

/// 一次编辑的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    /// 对 buffer 的变更。
    pub changes: Vec<TextChange>,
    /// 编辑后的光标位置。
    pub cursor: Offset,
    /// 编辑后的选中锚点。
    pub anchor: Offset,
}

/// 编辑操作结束后向宿主发起的选区描述（与 buffer 版本无关的快照）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionSnapshot {
    pub anchor: Offset,
    pub cursor: Offset,
}

/// 编辑器事件。
#[derive(Clone, Debug, PartialEq)]
pub enum EditorEvent {
    /// 文本已变更。携带变更信息，宿主可据此同步到 AppCommand。
    Changed(TextChange),
    /// 选区已变更。
    SelectionChanged(Selection),
    /// 用户触发执行（Run/Select/Explain 等）。`mode` 由宿主 adapter 解释。
    Execute { range: Range, mode: ExecuteMode },
    /// 用户请求格式化。
    FormatRequested,
    /// 补全项被接受。
    CompletionAccepted(CompletionItem),
    /// 用户请求 hover 信息。
    HoverRequested(Point),
    /// 用户点击 CodeLens；动作键由宿主解释。
    CodeLensActivated { range: Range, action: String },
}

/// 执行模式。具体的 Run/Select/Explain 语义由 adapter 层解释，
/// core 只传递中立标识。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecuteMode {
    /// 执行当前语句或选区。
    #[default]
    Execute,
    /// 只查询（不修改）。
    Select,
    /// 解释执行计划。
    Explain,
}

/// 软换行模式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoftWrapMode {
    /// 不软换行，横向滚动。
    #[default]
    None,
    /// 按视口宽度软换行。
    EditorWidth,
}

/// 最后一行之后还能继续滚动多少（对齐 Zed 的 `scroll_beyond_last_line`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollBeyondLastLine {
    /// 预留一屏：最后一行可以滚到视口顶部。
    ///
    /// 编辑场景（Zed 默认）留出输入呼吸空间，但内容短于视口时会凭空多出一屏可滚空白。
    #[default]
    OnePage,
    /// 不预留：可滚范围恰好到内容末尾，内容装得下就没有可滚动的余量。
    /// 只读预览用这个，避免短文本仍能滚动。
    None,
}

/// 回车提交模式：单行输入框回车如何响应。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SubmitMode {
    /// 回车提交（适用于单行输入）。
    Submit,
    /// 回车插入换行。
    #[default]
    InsertNewline,
}

/// 补全触发方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// 仅在显式触发（Ctrl+Space）时弹出。
    #[default]
    Manual,
    /// 输入触发字符时自动弹出。
    Auto,
}

/// 编辑器能力配置：与 UI 无关，允许 clone 后单独调整。
#[derive(Clone, Debug, PartialEq)]
pub struct EditorProfile {
    pub language_id: String,
    pub multiline: bool,
    pub read_only: bool,
    pub soft_wrap: SoftWrapMode,
    pub tab_size: usize,
    pub use_spaces: bool,
    pub show_line_numbers: bool,
    pub show_folding: bool,
    /// 内容末尾之外的可滚动余量；只读预览应设为 `None`。
    pub scroll_beyond_last_line: ScrollBeyondLastLine,
    pub auto_close_pairs: bool,
    pub submit_mode: SubmitMode,
    pub completion_trigger: CompletionTrigger,
    /// 触发补全的字符（如 SQL 的 ' '、'.' 等）；普通 word 字符仍按最小前缀触发。
    pub completion_trigger_chars: Vec<char>,
    /// 最小前缀长度（触发自动补全所需的前缀长度）。
    pub completion_min_prefix: usize,
}

impl EditorProfile {
    /// 判断一次增量编辑是否应触发自动补全。
    ///
    /// 这里只处理通用编辑语义：单字符插入、触发字符和前缀长度；具体语言的
    /// qualifier/上下文判断仍交给 CompletionProvider::should_trigger。
    pub fn should_auto_complete_after_edit(
        &self,
        change: &TextChange,
        query_char_count: usize,
        is_word_char: bool,
    ) -> bool {
        if self.completion_trigger != CompletionTrigger::Auto
            || change.full_document
            || change.new_text.chars().count() != 1
        {
            return false;
        }
        let Some(last_char) = change.new_text.chars().next() else {
            return false;
        };
        self.completion_trigger_chars.contains(&last_char)
            || (is_word_char && query_char_count >= self.completion_min_prefix)
    }
}

impl Default for EditorProfile {
    fn default() -> Self {
        Self {
            language_id: "plain".into(),
            multiline: true,
            read_only: false,
            soft_wrap: SoftWrapMode::None,
            tab_size: 4,
            use_spaces: true,
            show_line_numbers: false,
            show_folding: false,
            scroll_beyond_last_line: ScrollBeyondLastLine::OnePage,
            auto_close_pairs: true,
            submit_mode: SubmitMode::InsertNewline,
            completion_trigger: CompletionTrigger::Manual,
            completion_trigger_chars: Vec::new(),
            completion_min_prefix: 1,
        }
    }
}

/// 编辑器配置：扩展开的运行时配置集合。
#[derive(Clone, Debug, PartialEq)]
pub struct EditorConfig {
    pub profile: EditorProfile,
    /// 字体缩写（如 "Menlo"），具体由 GPUI 前端解析。core 不解析。
    pub font: String,
    pub font_size: f32,
    pub line_height: f32,
    /// gutter 是否显示行号。
    pub gutter_line_numbers: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            profile: EditorProfile::default(),
            font: "Menlo".into(),
            font_size: 12.0,
            line_height: 20.0,
            gutter_line_numbers: true,
        }
    }
}

/// 补全项插入格式。
///
/// 默认 `PlainText`：`insert_text` 原样插入，不解析占位符。仅当 provider 显式声明
/// `Snippet` 时才解析 `$1`、`${1:placeholder}` 等 tabstop 语法。避免把 SQL 中可能
/// 出现的 `$1`、`${name}` 字面量误解析为 snippet（参见设计文档 §14.3 兼容策略 2）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InsertTextFormat {
    #[default]
    PlainText,
    Snippet,
}

/// 补全项：使用中立字段，供 SQL / Redis / 代码等 provider 复用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub insert_text: String,
    /// 插入格式：默认 PlainText 原样插入；Snippet 时解析 tabstop 语法。
    pub insert_text_format: InsertTextFormat,
    /// 可选的替换区间（相对当前光标，由 provider 返回）。
    pub replace_range: Option<Range>,
    /// 类型：keyword / function / table / column / schema / snippet / command。
    pub kind: CompletionKind,
    pub detail: String,
    pub documentation: String,
    pub filter_text: String,
    pub sort_text: String,
    pub priority: i32,
    pub commit_characters: Vec<char>,
}

impl CompletionItem {
    pub fn new(label: impl Into<String>, kind: CompletionKind) -> Self {
        let label = label.into();
        Self {
            insert_text: label.clone(),
            insert_text_format: InsertTextFormat::PlainText,
            label,
            replace_range: None,
            kind,
            detail: String::new(),
            documentation: String::new(),
            filter_text: String::new(),
            sort_text: String::new(),
            priority: 0,
            commit_characters: Vec::new(),
        }
    }

    /// 计算过滤得分：前缀匹配时给出更高分，便于本地排序。
    pub fn match_score(&self, query: &str) -> Option<u32> {
        if query.is_empty() {
            return Some(0);
        }
        let filter = self.filter_text_full();
        let filter = filter.to_lowercase();
        let query = query.to_lowercase();
        if let Some(prefix_offset) = filter.find(&query) {
            // 前缀匹配得分最高，越靠前越高。
            let offset_penalty = (prefix_offset * 2) as u32;
            Some(10_000 - offset_penalty + self.priority.clamp(0, 1000) as u32)
        } else {
            // provider 可能已经按更宽松的 fuzzy 规则筛选过候选；core 继续支持
            // 通用的子序列匹配，避免桌面层二次过滤掉 `Pdt -> Product` 这类结果。
            let mut search_from = 0;
            let mut first = None;
            let mut last = None;
            for query_char in query.chars() {
                let Some(relative) = filter[search_from..]
                    .char_indices()
                    .find(|(_, value_char)| *value_char == query_char)
                    .map(|(index, _)| index)
                else {
                    return None;
                };
                let index = search_from + relative;
                first.get_or_insert(index);
                last = Some(index);
                search_from = index + query_char.len_utf8();
            }
            let first = first.unwrap_or(0);
            let span = last.unwrap_or(first).saturating_sub(first);
            Some(
                8_000u32
                    .saturating_sub((first * 2) as u32)
                    .saturating_sub(span as u32)
                    + self.priority.clamp(0, 1000) as u32,
            )
        }
    }

    fn filter_text_full(&self) -> &str {
        if !self.filter_text.is_empty() {
            &self.filter_text
        } else {
            &self.label
        }
    }
}

/// 补全项类型。
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum CompletionKind {
    #[default]
    Text,
    Keyword,
    Function,
    Class,
    Method,
    Module,
    Table,
    Column,
    Schema,
    Snippet,
    Command,
    Value,
}

/// 补全请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionRequest {
    /// 用于取消/丢弃旧结果的递增请求 id。
    pub request_id: u64,
    /// 发起请求时的 buffer 版本。
    pub buffer_version: u64,
    /// 光标位置（UTF-8 字节偏移）。
    pub cursor: Offset,
    /// 光标前后的触发文本上下文。
    pub query: String,
    /// 是否显式触发。
    pub explicit: bool,
    /// 发起请求时的文档快照。仅 provider 需要完整 SQL 上下文时读取；普通本地补全可忽略。
    pub document: Option<BufferSnapshot>,
    /// 触发请求的编辑序号；用于和编辑器性能日志关联，旧调用默认为 0。
    pub edit_id: u64,
    /// 续载游标（DM-704）：续载上一批部分结果时原样回传；首请求为 None。
    #[allow(dead_code)] // 单 provider 现况无分页消费，协议承载 + 测试。
    pub continuation: Option<CompletionContinuation>,
}

/// 补全结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompletionResult {
    pub items: Vec<CompletionItem>,
    /// 是否有更多结果尚未加载（用于分页）。
    pub has_more: bool,
    /// 续载游标（DM-704）：`has_more` 为真时携带，宿主下次请求回传。
    pub continuation: Option<CompletionContinuation>,
}

/// 诊断严重级别。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    #[default]
    Hint,
    Information,
    Warning,
    Error,
}

/// 诊断：错误 / 警告 / 提示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub message: String,
    /// (start, end) 可选的消息来源标签（如驱动名）。
    pub source: String,
}

impl Diagnostic {
    pub fn error(range: Range, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            source: String::new(),
        }
    }

    pub fn warning(range: Range, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: DiagnosticSeverity::Warning,
            message: message.into(),
            source: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_sorted_and_intersection() {
        let r = Range::new(10, 2);
        assert_eq!(r.sorted(), Range::new(2, 10));
        assert_eq!(
            Range::new(0, 5).intersection(&Range::new(3, 8)),
            Some(Range::new(3, 5))
        );
        assert_eq!(Range::new(0, 2).intersection(&Range::new(3, 5)), None);
    }

    #[test]
    fn completion_match_score_prefers_prefix() {
        let kw = CompletionItem::new("select", CompletionKind::Keyword);
        let tbl = CompletionItem::new("t_select", CompletionKind::Table);
        let kw_score = kw.match_score("sel").unwrap();
        let tbl_score = tbl.match_score("sel").unwrap();
        assert!(kw_score > tbl_score);
        assert!(tbl.match_score("zzz").is_none());
    }

    /// DM-105：编辑区间之前的锚点不变（无论 bias）。
    #[test]
    fn anchor_before_edit_is_unaffected() {
        let a = Anchor::new(3, Bias::Right);
        assert_eq!(a.relocate(Range::new(10, 15), 12), a);
        // 起点 == 编辑起点 + Left：锚点在插入点左边，保持不动。
        let a = Anchor::new(10, Bias::Left);
        assert_eq!(a.relocate(Range::new(10, 15), 12), a);
    }

    /// DM-105：编辑起点边界按 bias 吸附。
    #[test]
    fn anchor_at_edit_start_follows_bias() {
        let a = Anchor::new(10, Bias::Left);
        // Left 保持起点。
        assert_eq!(
            a.relocate(Range::new(10, 20), 5),
            Anchor::new(10, Bias::Left)
        );
        // Right 吸附到新文本终点。
        let a = Anchor::new(10, Bias::Right);
        assert_eq!(
            a.relocate(Range::new(10, 20), 5),
            Anchor::new(15, Bias::Right)
        );
    }

    /// DM-105：删除区间内部按 bias 吸附到起点或终点；后续内容整体平移。
    #[test]
    fn anchor_inside_and_after_edit() {
        // 区间内部，删除 10..12（new_len=0）。
        let inside_left = Anchor::new(11, Bias::Left);
        assert_eq!(
            inside_left.relocate(Range::new(10, 12), 0),
            Anchor::new(10, Bias::Left)
        );
        let inside_right = Anchor::new(11, Bias::Right);
        assert_eq!(
            inside_right.relocate(Range::new(10, 12), 0),
            Anchor::new(10, Bias::Right)
        );
        // 编辑终点上 → 新文本终点。
        let end_point = Anchor::new(12, Bias::Left);
        assert_eq!(
            end_point.relocate(Range::new(10, 12), 0),
            Anchor::new(10, Bias::Left)
        );
        // 区间之后：删除 3 字节 → 前移。
        let after = Anchor::new(20, Bias::Right);
        assert_eq!(
            after.relocate(Range::new(10, 13), 0),
            Anchor::new(17, Bias::Right)
        );
        // 区间之后：插入 3 字节 → 后移。
        let after = Anchor::new(20, Bias::Left);
        assert_eq!(
            after.relocate(Range::new(10, 10), 3),
            Anchor::new(23, Bias::Left)
        );
    }

    /// DM-105：内部替换（new_len > 0）把内部锚点吸附到新文本终点。
    #[test]
    fn anchor_inside_replacement_moves_to_new_end() {
        let inside = Anchor::new(15, Bias::Right);
        assert_eq!(
            inside.relocate(Range::new(10, 20), 8),
            Anchor::new(18, Bias::Right)
        );
        let inside_left = Anchor::new(15, Bias::Left);
        assert_eq!(
            inside_left.relocate(Range::new(10, 20), 8),
            Anchor::new(10, Bias::Left)
        );
    }

    /// DM-105：AnchorRange 两端随编辑一起重定位，折叠点前插入不漂移。
    #[test]
    fn anchor_range_relocates_both_ends() {
        let r = AnchorRange::new(Anchor::new(10, Bias::Right), Anchor::new(20, Bias::Right));
        // 在折叠范围(10..20)之前插入 5 字节。
        let shifted = r.relocate(Range::new(5, 5), 5);
        assert_eq!(shifted.start.offset, 15);
        assert_eq!(shifted.end.offset, 25);
        // 折叠范围内编辑：起点（==编辑起点,Right）吸附到新文本终点 13；
        // 终点(20)在删除区间之后，净增 1 字节 → 21。
        let inside = r.relocate(Range::new(10, 12), 3);
        assert_eq!(inside.start.offset, 13);
        assert_eq!(inside.end.offset, 21);
    }

    #[test]
    fn completion_match_score_supports_subsequence() {
        let item = CompletionItem::new("Product", CompletionKind::Table);
        assert!(item.match_score("Pdt").is_some());
        assert!(item.match_score("Pzx").is_none());
    }
}
