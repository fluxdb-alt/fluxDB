//! 通用 provider 协议与补全控制器。
//!
//! `CompletionProvider` / `HoverProvider` / `SignatureProvider` / `InlineHintProvider` /
//! `DiagnosticProvider` / `DecorationProvider` / `ExecutionAdapter` 全部只操作
//! 只读 snapshot 与中立数据，不能直接修改 buffer；由宿主（GPUI 前端 + adapter）
//! 负责运行、取消和把结果落回 UI。
//!
//! `CompletionController` 提供本地过滤、缓存、request id + buffer version 保护，
//! 保证旧请求结果不会覆盖新文本或新选区。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::buffer::BufferSnapshot;
use crate::model::{
    CompletionItem, CompletionKind, CompletionRequest, CompletionResult, Diagnostic, ExecuteMode,
    Offset, Point, Range, Selection,
};
use crate::syntax::InputEdit;

/// 补全 provider 的异步结果未来。
pub type CompletionFuture =
    Pin<Box<dyn Future<Output = Result<CompletionResult, CompletionError>> + Send>>;

/// 补全错误。core 只定义中立错误，不携带连接细节。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionError {
    Cancelled,
    Interrupted,
    Provider,
}

/// 补全 session 生命周期状态（DM-701，对齐 VS Code SuggestModel）。
///
/// 状态由 editor 从现有隐式标志（visible/loading/continuation/token）派生，本枚举只做
/// 类型化 + 稳定标签供划分日志与断言，不新增调度机制（`ponytail:` 单 provider 现况下
/// 用派生态而非完整状态机对象；出现真实多 provider/分页再补迁移守卫对象）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionSession {
    /// 无活动补全。
    Idle,
    /// 已请求、等待 provider 返回（loading）。
    Triggering,
    /// 有可见结果。
    Active,
    /// 在已有结果上继续输入触发刷新。
    Retriggering,
    /// 被取消/隐藏。
    Cancelled,
}

impl CompletionSession {
    /// 稳定字符串标签，供结构化日志与断言共享。
    pub fn as_str(&self) -> &'static str {
        match self {
            CompletionSession::Idle => "idle",
            CompletionSession::Triggering => "triggering",
            CompletionSession::Active => "active",
            CompletionSession::Retriggering => "retriggering",
            CompletionSession::Cancelled => "cancelled",
        }
    }
}

/// 不透明 continuation 游标（DM-704）：替代裸 `has_more: bool` 的能力上限。
///
/// provider 在部分结果时返回 `Some(游标)`，宿主在下次请求把它原样回传给 provider 续载
/// 更多；`Send + Sync + Copy`。当前 SQL 单 provider 不真正分页（由 `has_more` 映射，
/// has_more=false → None），本类型只承担协议承载与后续分页升级，不预建分页消费端
/// （YAGNI，参见 §11 禁止无消费者类型系统）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompletionContinuation(pub u64);

/// 补全触发决策判断。
pub trait CompletionProvider: Send + Sync {
    /// 判断给定请求是否应触发补全。
    fn should_trigger(&self, request: &CompletionRequest) -> TriggerDecision;

    /// 使当前 provider 请求失效。默认实现适用于没有外部资源的 provider。
    fn cancel_pending(&self) {}

    /// 发起补全。可返回本地结果（立即）或异步结果。
    fn complete(&self, request: CompletionRequest) -> CompletionFuture;

    /// 解析候选项右侧 metadata 详情（F005）。宿主在后台线程调用，latest-wins 取消。
    /// 返回 `None` 表示本 provider 不支持详情，编辑器回退到候选自带内联文档。
    fn documentation(&self, _request: DocumentationRequest) -> Option<DocumentationState> {
        None
    }
}

/// 触发决策。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerDecision {
    No,
    Yes,
    DependsOnPrefix(usize),
}

/// 补全候选项「右侧 metadata 详情」请求（F005）。由选中项/悬停切换触发，宿主在
/// 后台线程异步解析；`latest_request`/`request_id` 用于 latest-wins 取消——请求 id
/// 变化即丢弃旧结果，保证旧详情不覆盖新选中项。
#[derive(Clone, Debug)]
pub struct DocumentationRequest {
    pub kind: CompletionKind,
    pub label: String,
    /// 候选自带的内联注释（仅 Column 有意义）。不缓存全文快照，沿用补全可得的
    /// 注释文本；`None` 表示无内联注释。
    pub comment: Option<String>,
    /// 候选对象所属 schema（由候选项携带，`None` 表示未知/不适用）。
    /// 详情解析据此按 (库, schema, 表) 查询完整身份。
    pub schema: Option<String>,
    pub latest_request: Arc<std::sync::atomic::AtomicU64>,
    pub request_id: u64,
}

/// 详情解析异步状态（F005）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentationState {
    /// 加载中，或被新选择取消（latest-wins 提前返回）。
    Loading,
    /// 解析成功；`String` 为渲染文本（表/视图→列清单，列→注释，函数等→名称）。
    Ready(String),
    /// 解析失败；`String` 为原因（对象不在索引 / 无注释 / 无可用文档）。
    Error(String),
}

/// hover 内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverContent {
    pub text: String,
    pub range: Range,
}

/// hover provider。
pub trait HoverProvider: Send + Sync {
    fn hover(&self, snapshot: &BufferSnapshot, position: Point) -> Option<HoverContent>;
}

/// 签名/命令提示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureInfo {
    pub label: String,
    pub documentation: String,
    /// 当前激活参数索引。
    pub active_parameter: Option<usize>,
    /// 显式参数范围：label 内每个参数的 `[start, end)` UTF-8 byte 区间。
    ///
    /// 供以空格/方括号分隔参数骨架的语言（如 Redis `SET <key> <value> [NX|XX]`）
    /// 精确高亮当前参数标注。空 Vec 表示未提供，渲染层回退到按 `()`+逗号解析 label
    /// 的既有规则（SQL 函数签名），保持兼容。
    pub parameter_ranges: Vec<(usize, usize)>,
}

/// 签名 provider。
pub trait SignatureProvider: Send + Sync {
    fn signature(&self, snapshot: &BufferSnapshot, position: Point) -> Option<SignatureInfo>;
}

// ---------- 函数签名参数推断（P1.9） ----------

/// 光标处所在函数调用派生的签名信息（P1.9）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSignature {
    /// 函数名（不包含 schema 限定）。
    pub name: String,
    /// 当前正在编辑的参数下标。
    pub active_parameter: usize,
}

/// 计算某函数调用括号内当前 active 参数的下标（P1.9）。
///
/// `inside` 为最近开括号之后到光标之间的文本。统计深度 0（即不属于嵌套括号）的逗号，
/// 并忽略字符串字面量（单/双引号、反引号）内的逗号，以确定正在编辑的参数下标。
/// 空文本或括号内无参数返回 0。
pub fn active_parameter_index(inside: &str) -> usize {
    let mut depth = 0usize;
    let mut commas = 0usize;
    let mut chars = inside.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '\'' | '"' | '`' => {
                let quote = c;
                // 跳过字符串字面量；单/双引号支持相邻重复字符作为转义。
                while let Some(n) = chars.next() {
                    if n == quote {
                        if quote != '`' && chars.peek() == Some(&quote) {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
            }
            ',' if depth == 0 => commas += 1,
            _ => {}
        }
    }
    commas
}

/// 推断光标所处函数调用的签名（P1.9）：定位最近包围光标的「identifier(」，据括号内文本
/// 计算 active 参数下标。
///
/// - 兼容嵌套调用（取最近的包围括号）与 schema 限定（只取末段标识符）。
/// - 括号左侧不是标识符（如控制流 `if (`）返回 `None`。
/// - 光标不在任何括号内（移出括号）返回 `None`。
pub fn signature_at(snapshot: &BufferSnapshot, position: Point) -> Option<CallSignature> {
    let cursor = snapshot.point_to_offset(position);
    if cursor == 0 {
        return None;
    }
    // 签名只依赖光标附近尚未闭合的调用上下文；避免在大文档中复制全文。
    const SIGNATURE_CONTEXT_BYTES: usize = 64 * 1024;
    let context_start = cursor.saturating_sub(SIGNATURE_CONTEXT_BYTES);
    let text = snapshot.text_in_range(Range::new(context_start, cursor));
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = text.len();
    loop {
        if i == 0 {
            return None;
        }
        i = text[..i]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    // 找到最里的包围开括号：仅当左侧紧邻标识符（函数调用）时返回签名；
                    // 括号左侧不是标识符（如控制流 `if (`）或不在括号内则返回 None。
                    let name = preceding_identifier(bytes, i);
                    if name.is_empty() {
                        return None;
                    }
                    let inside = &text[i + 1..text.len()];
                    return Some(CallSignature {
                        name,
                        active_parameter: active_parameter_index(inside),
                    });
                }
                depth -= 1;
            }
            _ => {}
        }
    }
}

/// 返回紧邻 `index`（字节，指向某字符首字节）左侧的标识符（`[a-zA-Z0-9_]`），
/// 不含 schema 限定符；无标识符返回空串。
fn preceding_identifier(bytes: &[u8], index: usize) -> String {
    let end = index;
    let mut start = end;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    if start == end {
        return String::new();
    }
    String::from_utf8_lossy(&bytes[start..end]).into_owned()
}

/// 行内 hint。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineHint {
    pub position: Offset,
    pub text: String,
}

/// 行内 hint provider。
pub trait InlineHintProvider: Send + Sync {
    fn hints(&self, snapshot: &BufferSnapshot, visible: Range) -> Vec<InlineHint>;

    /// provider 自身配置版本（DM-303）。当 provider 的配置/设置变化（如开关、
    /// 阈值）应递增 revision，使已经产出的 inlay 结果被判定过期，UI 重新请求。
    /// 返回 0 表示 provider 无自身版本依赖。
    fn revision(&self) -> u64 {
        0
    }
}

/// 诊断 provider。
pub trait DiagnosticProvider: Send + Sync {
    fn diagnostics(&self, snapshot: &BufferSnapshot) -> Vec<Diagnostic>;

    /// 对单次编辑提供增量诊断入口；默认实现回退到完整诊断。
    fn diagnostics_incremental(
        &self,
        snapshot: &BufferSnapshot,
        changed: InputEdit,
    ) -> Vec<Diagnostic> {
        let _ = changed;
        self.diagnostics(snapshot)
    }
}

/// 编辑器上方的可点击代码镜头（CodeLens）。
///
/// provider 返回独立镜头；渲染层按 anchor 所在行合并多个镜头，并决定显示分隔符。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeLens {
    /// 镜头关联的源码范围。通常使用 `start` 作为显示 anchor。
    pub range: Range,
    /// 显示文案，例如 `Run` 或 `Select`。
    pub title: String,
    /// 中性动作键，由宿主解释，不携带 SQL/数据库语义。
    pub action: String,
}

/// CodeLens provider。
pub trait CodeLensProvider: Send + Sync {
    fn code_lenses(&self, snapshot: &BufferSnapshot, visible: Range) -> Vec<CodeLens>;
}

/// 装饰：文本/行背景、gutter 图标、行首按钮、行尾 widget 与可点击区域。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoration {
    /// 文本前景高亮。
    TextBackground(Range, String),
    /// 文本下划线。
    TextUnderline(Range, String),
    /// 行背景（按 visual/buffer 行）。
    LineBackground(usize, String),
    /// 行号区图标 (buffer_row, icon key)。
    GutterIcon(usize, String),
    /// 行首按钮 (buffer_row, label, action key)。
    StatementAction(usize, String, String),
    /// 行尾 widget (buffer_row, text)。
    LineEndWidget(usize, String),
    /// 命中区域（可点击）。
    HitRegion(Range, String),
}

/// 装饰集合。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecorationSet {
    pub decorations: Vec<Decoration>,
}

/// 装饰 provider。
pub trait DecorationProvider: Send + Sync {
    fn decorations(&self, snapshot: &BufferSnapshot, visible: Range) -> DecorationSet;
}

/// 执行单元。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionUnit {
    pub range: Range,
    pub text: String,
    pub mode: ExecuteMode,
}

/// 执行 adapter：只把 snapshot + selection 转成执行单元，不执行外部命令。
pub trait ExecutionAdapter: Send + Sync {
    fn execution_units(
        &self,
        snapshot: &BufferSnapshot,
        selection: Selection,
    ) -> Vec<ExecutionUnit>;
}

// ---------- 补全控制器 ----------

/// 管理补全请求生命周期：request id 递增、按 buffer version 丢弃旧结果、本地缓存。
pub struct CompletionController {
    /// 下次分配的 request id。
    next_request_id: u64,
    /// 最新一次请求的 id；旧请求迟到返回时可据此判定过期并丢弃。
    latest_request_id: u64,
    /// 最新一次请求的 buffer version。
    latest_buffer_version: u64,
    /// 已经返回的补全结果缓存（query → items）。
    cache: HashMap<String, Vec<CompletionItem>>,
    /// 上一次查询（用于后缀复用）。
    last_query: String,
    /// 上一次 provider 结果对应的编辑位置和 buffer 版本；用于限制后缀复用的上下文。
    last_result_cursor: Option<Offset>,
    last_result_version: Option<u64>,
}

impl Default for CompletionController {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletionController {
    pub fn new() -> Self {
        Self {
            next_request_id: 1,
            latest_request_id: 0,
            latest_buffer_version: 0,
            cache: HashMap::new(),
            last_query: String::new(),
            last_result_cursor: None,
            last_result_version: None,
        }
    }

    /// 分配一个新的补全请求，并把它记为“最新请求”。
    ///
    /// 每次分配都会推进 `latest_request_id`，因此同一版本（或任意版本）下迟到的旧请求
    /// 都能通过 [`Self::is_latest_request`] 判定为过期并丢弃，避免旧结果覆盖新文本/新选区。
    pub fn new_request(
        &mut self,
        buffer_version: u64,
        cursor: Offset,
        query: String,
        explicit: bool,
    ) -> CompletionRequest {
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.latest_request_id = id;
        self.latest_buffer_version = buffer_version;
        CompletionRequest {
            request_id: id,
            buffer_version,
            cursor,
            query,
            explicit,
            document: None,
            edit_id: 0,
            continuation: None,
        }
    }

    /// 最新一次请求的 id（用于丢弃旧请求的迟到结果）。
    pub fn latest_request_id(&self) -> u64 {
        self.latest_request_id
    }

    /// 最新一次请求的 buffer version。
    pub fn latest_buffer_version(&self) -> u64 {
        self.latest_buffer_version
    }

    /// `request_id` 是否仍是最新请求。旧请求迟到返回时返回 `false`，调用方应丢弃结果。
    pub fn is_latest_request(&self, request_id: u64) -> bool {
        request_id == self.latest_request_id
    }

    /// 若新查询是旧查询的后缀，优先本地过滤已有候选，避免重复访问外部 provider。
    /// 返回 `Some(候选)` 表示可复用的本地项；`None` 表示需要走 provider。
    pub fn try_reuse_local(
        &mut self,
        new_query: &str,
        new_buffer_version: u64,
        last_buffer_version: u64,
    ) -> Option<Vec<CompletionItem>> {
        self.try_reuse_local_at(new_query, new_buffer_version, last_buffer_version, None)
    }

    /// 在同一光标上下文中复用后缀候选；连续输入最多允许跨一个版本。
    pub fn try_reuse_local_at(
        &mut self,
        new_query: &str,
        new_buffer_version: u64,
        last_buffer_version: u64,
        new_cursor: Option<Offset>,
    ) -> Option<Vec<CompletionItem>> {
        if let Some(result_version) = self.last_result_version {
            if new_buffer_version < result_version
                || new_buffer_version > result_version.saturating_add(1)
                || last_buffer_version < result_version
            {
                return None;
            }
        }
        let suffix_bytes = new_query.len().saturating_sub(self.last_query.len());
        let insertion_cursor = match (self.last_result_cursor, new_cursor) {
            (Some(previous_cursor), Some(cursor)) => {
                if cursor < previous_cursor || cursor - previous_cursor != suffix_bytes {
                    return None;
                }
                Some(previous_cursor)
            }
            _ => None,
        };
        if new_query.len() <= self.last_query.len() {
            // 不是后缀（变短了），需要刷新。
            return None;
        }
        if !new_query.starts_with(&self.last_query) {
            return None;
        }
        let candidates = self.cache.get(&self.last_query)?.clone();
        let suffix = &new_query[self.last_query.len()..];
        // 复用只发生在光标处追加后缀的场景；把缓存里的替换区间按这次插入平移，
        // 否则整行候选（Mongo shell 的 `db["…"]`）会退化成按词前缀插入，留下 `db.db[...]`。
        // 没有旧光标时无法确定插入点，仍清空区间交给宿主重建。
        let filtered: Vec<CompletionItem> = candidates
            .iter()
            .filter(|item| {
                let full = if item.filter_text.is_empty() {
                    &item.label
                } else {
                    &item.filter_text
                };
                full.to_lowercase().contains(&suffix.to_lowercase())
            })
            .cloned()
            .map(|mut item| {
                item.replace_range = match (item.replace_range, insertion_cursor) {
                    (Some(range), Some(cursor)) => {
                        let shift = |offset: Offset| {
                            if offset >= cursor {
                                offset + suffix_bytes
                            } else {
                                offset
                            }
                        };
                        Some(Range::new(shift(range.start), shift(range.end)))
                    }
                    _ => None,
                };
                item
            })
            .collect();
        if filtered.is_empty() {
            return None;
        }
        self.last_query = new_query.to_string();
        self.last_result_version = Some(new_buffer_version);
        self.last_result_cursor = new_cursor;
        self.cache.insert(self.last_query.clone(), filtered.clone());
        Some(filtered)
    }

    /// 记录一次 provider 返回，供后续复用。
    pub fn store_result(&mut self, query: &str, items: Vec<CompletionItem>) {
        self.store_result_at(query, items, None, None);
    }

    /// 记录 provider 结果及其编辑上下文，供 VS Code 式后缀复用判断。
    pub fn store_result_at(
        &mut self,
        query: &str,
        items: Vec<CompletionItem>,
        buffer_version: Option<u64>,
        cursor: Option<Offset>,
    ) {
        self.last_query = query.to_string();
        self.cache.insert(query.to_string(), items);
        self.last_result_version = buffer_version;
        self.last_result_cursor = cursor;
        // 简单缓存上限，防止无限增长。
        if self.cache.len() > 100 {
            // 保留最近条目：收集键并移除最旧的。
            let mut keys: Vec<String> = self.cache.keys().cloned().collect();
            keys.sort();
            let excess = keys.len() - 100;
            for k in keys.into_iter().take(excess) {
                self.cache.remove(&k);
            }
        }
    }
}

/// 便捷：计算给定 offset 前的补全前缀（词字符序列）。
pub fn completion_prefix(
    snapshot: &BufferSnapshot,
    cursor: Offset,
    word_char: &dyn Fn(char) -> bool,
) -> String {
    let point = snapshot.offset_to_point(cursor);
    let line_start = snapshot.line_start(point.row);
    let before = snapshot.text_in_range(Range::new(line_start, cursor));
    before
        .chars()
        .rev()
        .take_while(|c| word_char(*c))
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect()
}

/// 便捷：返回 `[start, end)` 排序后的区间（供 ReplaceRange 使用）。
pub fn sorted_range(start: Offset, end: Offset) -> Range {
    if start <= end {
        Range::new(start, end)
    } else {
        Range::new(end, start)
    }
}

#[cfg(test)]
mod signature_tests {
    use super::*;
    use crate::buffer::EditorBuffer;

    /// 在文本末尾调用 `signature_at`；`text` 的末尾即光标位置。
    fn signature_at_end(text: &str) -> Option<CallSignature> {
        let snapshot = EditorBuffer::new_from(text).snapshot();
        let point = snapshot.offset_to_point(text.len());
        signature_at(&snapshot, point)
    }

    fn call(name: &str, active: usize) -> Option<CallSignature> {
        Some(CallSignature {
            name: name.to_string(),
            active_parameter: active,
        })
    }

    #[test]
    fn signature_uses_innermost_call_and_counts_arguments() {
        assert_eq!(signature_at_end("SELECT COUNT("), call("COUNT", 0));
        assert_eq!(signature_at_end("SELECT CONCAT(a, "), call("CONCAT", 1));
        // 嵌套：取最内层未闭合的调用。
        assert_eq!(signature_at_end("SELECT SUM(ROUND(a, "), call("ROUND", 1));
        // schema 限定只取末段标识符。
        assert_eq!(
            signature_at_end("SELECT pg_catalog.count("),
            call("count", 0)
        );
    }

    #[test]
    fn signature_ignores_commas_in_string_literals() {
        assert_eq!(signature_at_end("SELECT CONCAT('a,b', "), call("CONCAT", 1));
    }

    #[test]
    fn signature_requires_unclosed_call_parenthesis() {
        // 括号已闭合、不在任何括号内、以及括号左侧不是标识符（分组括号）都不算调用。
        assert_eq!(signature_at_end("SELECT COUNT(a) "), None);
        assert_eq!(signature_at_end("SELECT 1"), None);
        assert_eq!(signature_at_end("SELECT (a + "), None);
        // 空文档（光标 0）。
        assert_eq!(signature_at_end(""), None);
    }
}

#[cfg(test)]
mod reuse_tests {
    use super::*;

    fn line_item(start: usize, end: usize) -> CompletionItem {
        let mut item = CompletionItem::new("db[\"customers\"].", CompletionKind::Command);
        item.replace_range = Some(Range::new(start, end));
        item
    }

    #[test]
    fn reused_ranges_follow_typing_at_cursor() {
        let mut ctrl = CompletionController::new();
        // 光标在 `db.` 行尾，整行替换区间 0..3；再键入 `c` 形成 `db.c`。
        ctrl.store_result_at("", vec![line_item(0, 3)], Some(1), Some(3));
        let reused = ctrl
            .try_reuse_local_at("c", 2, 1, Some(4))
            .expect("suffix reuse");
        assert_eq!(reused[0].replace_range, Some(Range::new(0, 4)));
    }

    #[test]
    fn reused_ranges_shift_residue_after_cursor() {
        let mut ctrl = CompletionController::new();
        // 光标在行中：区间覆盖光标右侧残留，插入一个字符后整体后移。
        ctrl.store_result_at("c", vec![line_item(0, 7)], Some(1), Some(4));
        let reused = ctrl
            .try_reuse_local_at("cu", 2, 1, Some(5))
            .expect("suffix reuse");
        assert_eq!(reused[0].replace_range, Some(Range::new(0, 8)));
    }
}
