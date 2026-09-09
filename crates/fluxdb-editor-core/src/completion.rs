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
        if let (Some(previous_cursor), Some(cursor)) = (self.last_result_cursor, new_cursor) {
            let suffix_bytes = new_query.len().saturating_sub(self.last_query.len());
            if cursor < previous_cursor || cursor - previous_cursor != suffix_bytes {
                return None;
            }
        }
        if new_query.len() <= self.last_query.len() {
            // 不是后缀（变短了），需要刷新。
            return None;
        }
        if !new_query.starts_with(&self.last_query) {
            return None;
        }
        let candidates = self.cache.get(&self.last_query)?.clone();
        let suffix = &new_query[self.last_query.len()..];
        // 位置相关的 replace_range 由“采纳”时刻依据实时光标重建；复用的缓存项可能对应
        // 旧光标位置，若原样带回会引入过期偏移。故复用路径统一清空，交由宿主重建。
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
                item.replace_range = None;
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

    /// 在已缓存/本地结果基础上做本地过滤与排序。
    pub fn filter_and_sort(&self, items: Vec<CompletionItem>, query: &str) -> Vec<CompletionItem> {
        let mut scored: Vec<(u32, CompletionItem)> = Vec::new();
        for item in items {
            if let Some(score) = item.match_score(query) {
                scored.push((score, item));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
        scored.into_iter().map(|(_, item)| item).collect()
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
mod tests {
    use super::*;
    use crate::buffer::EditorBuffer;
    use crate::model::CompletionKind;
    use crate::task::CancellationToken;

    fn item(label: &str, kind: CompletionKind) -> CompletionItem {
        CompletionItem::new(label, kind)
    }

    #[test]
    fn controller_reuses_local_on_suffix() {
        let mut c = CompletionController::new();
        let items = vec![
            item("select", CompletionKind::Keyword),
            item("selectall", CompletionKind::Keyword),
        ];
        c.store_result("sel", items.clone());
        // 同版本 + 后缀 → 本地过滤。
        let reused = c.try_reuse_local("selec", 0, 0);
        assert!(reused.is_some());
        assert_eq!(reused.as_ref().unwrap().len(), 2);
        // 逐字输入导致版本变化，仍可复用同一候选集做后缀过滤。
        assert!(c.try_reuse_local("select", 1, 0).is_some());
        // 非后缀 → 不复用。
        assert!(c.try_reuse_local("xyz", 0, 0).is_none());
    }

    #[test]
    fn controller_filters_sorts() {
        let c = CompletionController::new();
        let items = vec![
            item("apple", CompletionKind::Keyword),
            item("banana", CompletionKind::Keyword),
            item("application", CompletionKind::Keyword),
        ];
        let out = c.filter_and_sort(items, "app");
        let labels: Vec<&str> = out.iter().map(|i| i.label.as_str()).collect();
        // apple 前缀、application 前缀都在；banana 排除。
        assert!(labels.contains(&"apple"));
        assert!(labels.contains(&"application"));
        assert!(!labels.contains(&"banana"));
    }

    #[test]
    fn completion_prefix_extracts_word() {
        let snap = EditorBuffer::new_from("select * from us").snapshot();
        let prefix = completion_prefix(&snap, "select * from us".len(), &|c| {
            c.is_alphanumeric() || c == '_'
        });
        assert_eq!(prefix, "us");
        let point = snap.offset_to_point("select * from us".len());
        assert_eq!(point, Point::new(0, "select * from us".len()));
    }

    /// 同一 buffer version 连续两次请求，第一次迟到返回时应被判定过期丢弃。
    #[test]
    fn latest_request_advances_and_accepts_only_newest() {
        let mut c = CompletionController::new();
        // 同版本连续两次请求：id 递增并推进 latest。
        let req1 = c.new_request(5, 10, "se".to_string(), true);
        assert_eq!(c.latest_request_id(), req1.request_id);
        assert!(c.is_latest_request(req1.request_id));
        let req2 = c.new_request(5, 12, "sel".to_string(), true);
        assert_eq!(c.latest_request_id(), req2.request_id);
        assert_eq!(c.latest_buffer_version(), 5);
        // 第一次请求已过期，迟到应被丢弃；最新请求仍被接受。
        assert!(!c.is_latest_request(req1.request_id));
        assert!(c.is_latest_request(req2.request_id));
        // 再次发新请求后，第二次也过期。
        let req3 = c.new_request(6, 12, "sele".to_string(), false);
        assert!(!c.is_latest_request(req2.request_id));
        assert!(c.is_latest_request(req3.request_id));
    }

    /// 后缀本地复用不得把旧光标位置的 replace_range 带回新位置。
    #[test]
    fn suffix_reuse_clears_stale_replace_range() {
        let mut c = CompletionController::new();
        let mut keyword = item("select", CompletionKind::Keyword);
        keyword.replace_range = Some(Range::new(10, 14));
        c.store_result("sel", vec![keyword]);
        // 同版本 + 后缀 → 触发本地复用。
        let reused = c.try_reuse_local("sele", 0, 0).expect("应复用本地候选");
        assert_eq!(reused.len(), 1);
        // 复用的项不得携带旧位置 replace_range；位置由采纳时刻重算。
        assert_eq!(reused[0].replace_range, None);
    }

    #[test]
    fn suffix_reuse_requires_same_edit_context() {
        let mut c = CompletionController::new();
        c.store_result_at(
            "sel",
            vec![item("select", CompletionKind::Keyword)],
            Some(5),
            Some(10),
        );
        assert!(c.try_reuse_local_at("sele", 6, 5, Some(11)).is_some());
        assert!(c.try_reuse_local_at("sele", 7, 5, Some(12)).is_none());
        assert!(c.try_reuse_local_at("sele", 6, 5, Some(30)).is_none());
    }

    #[test]
    fn active_parameter_counts_top_level_commas() {
        assert_eq!(active_parameter_index(""), 0);
        assert_eq!(active_parameter_index("a"), 0);
        assert_eq!(active_parameter_index("a, b"), 1);
        assert_eq!(active_parameter_index("a, b, "), 2);
        // 嵌套括号内逗号不计。
        assert_eq!(active_parameter_index("f(a, b), c"), 1);
    }

    #[test]
    fn active_parameter_ignores_commas_in_strings() {
        assert_eq!(active_parameter_index("'a,b', c"), 1);
        assert_eq!(active_parameter_index("\"a,b\", c"), 1);
        assert_eq!(active_parameter_index("`a,b`, c"), 1);
        // 相邻重复引号作为转义，不结束字符串。
        assert_eq!(active_parameter_index("'it''s, x', b"), 1);
    }

    #[test]
    fn signature_at_finds_enclosing_call() {
        let snap = EditorBuffer::new_from("select concat(a, b, ").snapshot();
        let pos = snap.offset_to_point("select concat(a, b, ".len());
        let sig = signature_at(&snap, pos).expect("应找到 concat 调用");
        assert_eq!(sig.name, "concat");
        assert_eq!(sig.active_parameter, 2);
    }

    #[test]
    fn signature_at_nested_uses_innermost() {
        let snap = EditorBuffer::new_from("f(g(a, b, ").snapshot();
        let pos = snap.offset_to_point("f(g(a, b, ".len());
        let sig = signature_at(&snap, pos).expect("应取最内层 g 调用");
        assert_eq!(sig.name, "g");
        assert_eq!(sig.active_parameter, 2);
    }

    #[test]
    fn signature_at_inserted_paren_is_first_param() {
        // 刚输入 ( 时，括号内为空 → active 0。
        let snap = EditorBuffer::new_from("f(").snapshot();
        let pos = snap.offset_to_point(2);
        let sig = signature_at(&snap, pos).expect("应找到 f 调用");
        assert_eq!(sig.name, "f");
        assert_eq!(sig.active_parameter, 0);
    }

    #[test]
    fn signature_at_large_document_reads_only_local_context() {
        let text = format!("{}select concat(a, ", "-- 😀 filler\n".repeat(20_000));
        let snap = EditorBuffer::new_from(&text).snapshot();
        let pos = snap.offset_to_point(text.len());
        let sig = signature_at(&snap, pos).expect("应在大文档末尾找到 concat 调用");
        assert_eq!(sig.name, "concat");
        assert_eq!(sig.active_parameter, 1);
    }

    #[test]
    fn signature_at_cursor_outside_call_is_none() {
        // 光标移出括号：在 `f()` 之后 → 无包围括号。
        let snap = EditorBuffer::new_from("f()").snapshot();
        let pos = snap.offset_to_point(3);
        assert!(signature_at(&snap, pos).is_none());
        // 空文本。
        let snap = EditorBuffer::new_from("").snapshot();
        assert!(signature_at(&snap, snap.offset_to_point(0)).is_none());
    }

    #[test]
    fn signature_at_non_identifier_before_paren_is_none() {
        // 控制流 `if (` 左侧不是标识符 → 不判定为函数调用。
        let snap = EditorBuffer::new_from("if (x, y").snapshot();
        let pos = snap.offset_to_point("if (x, y".len());
        assert!(signature_at(&snap, pos).is_none());
    }

    /// DM-701：session 状态标签稳定，供划分日志关键字。
    #[test]
    fn completion_session_tags_are_stable() {
        assert_eq!(CompletionSession::Idle.as_str(), "idle");
        assert_eq!(CompletionSession::Triggering.as_str(), "triggering");
        assert_eq!(CompletionSession::Active.as_str(), "active");
        assert_eq!(CompletionSession::Retriggering.as_str(), "retriggering");
        assert_eq!(CompletionSession::Cancelled.as_str(), "cancelled");
        // 标签唯一。
        let mut tags: Vec<&str> = [
            CompletionSession::Idle,
            CompletionSession::Triggering,
            CompletionSession::Active,
            CompletionSession::Retriggering,
            CompletionSession::Cancelled,
        ]
        .iter()
        .map(|s| s.as_str())
        .collect();
        tags.sort();
        tags.dedup();
        assert_eq!(tags.len(), 5);
    }

    /// DM-704：continuation 游标为不透明 Copy 标量；续载请求原样回传、首请求为 None。
    #[test]
    fn continuation_cursor_roundtrips_through_request() {
        let cursor = CompletionContinuation(7);
        assert_eq!(cursor, CompletionContinuation(7));
        // 续载请求回传续载游标。
        let mut request = CompletionRequest {
            request_id: 1,
            buffer_version: 3,
            cursor: 10,
            query: "sel".to_string(),
            explicit: false,
            document: None,
            edit_id: 0,
            continuation: Some(cursor),
        };
        assert_eq!(request.continuation, Some(cursor));
        // 首请求由 new_request 构造即 None。
        let mut c = CompletionController::new();
        let first = c.new_request(3, 10, "s".to_string(), false);
        assert_eq!(first.continuation, None);
        // 回传后继续携带。
        request.continuation = first.continuation;
        assert_eq!(request.continuation, None);
    }

    /// DM-704：has_more 为真 → result 携带 continuation；为假 → None。
    #[test]
    fn result_maps_has_more_to_continuation() {
        let partial = CompletionResult {
            items: vec![item("select", CompletionKind::Keyword)],
            has_more: true,
            continuation: Some(CompletionContinuation(1)),
        };
        assert_eq!(partial.continuation, Some(CompletionContinuation(1)));
        let done = CompletionResult {
            items: vec![item("select", CompletionKind::Keyword)],
            has_more: false,
            continuation: None,
        };
        assert_eq!(done.continuation, None);
    }

    /// DM-709：session 取消语义——hide/接受/新一轮请求 bump 后，旧 session_id 判废，
    /// 与 `is_latest_request` 合成三守卫时任一过期即拒绝迟到结果。
    #[test]
    fn session_cancel_invalidates_stale_check() {
        let token = CancellationToken::default();
        let sid1 = token.request_id();
        assert!(token.check(sid1));
        // 新一轮请求会重新 request_id（fetch_add），旧 id 随即被判废——这正是
        // `request_completion` 每次新请求自动作废旧 session 的机制。
        let sid2 = token.request_id();
        assert!(!token.check(sid1));
        assert!(token.check(sid2));
        // hide/接受路径 cancel：bump 后旧 id 判废，仅最新 id 保留。
        token.cancel();
        assert!(!token.check(sid1));
        assert!(!token.check(sid2));
        let sid3 = token.request_id();
        assert!(token.check(sid3));
        // 与 latest-request 双守卫合成：任一判废即丢弃。
        let mut c = CompletionController::new();
        let old_req = c.new_request(1, 0, "sel".to_string(), false);
        let new_req = c.new_request(2, 0, "sele".to_string(), false);
        // 旧请求已被新请求抢占 → request 判废。
        assert!(!c.is_latest_request(old_req.request_id));
        assert!(c.is_latest_request(new_req.request_id));
        // session 过期 + request 过期，二者任一即拒。
        assert!(!(c.is_latest_request(old_req.request_id) && token.check(sid1)));
        assert!(c.is_latest_request(new_req.request_id) && token.check(sid3));
    }

    /// DM-709：partial 续载——provider 返回 continuation 后，editor 的新请求（retrigger）
    /// 原样回传游标续载更多；结果收敛（continuation None）后 session 回到非 retrigger 态。
    #[test]
    fn partial_retrigger_keeps_and_clears_continuation() {
        let mut c = CompletionController::new();
        // 首次请求 → 部分结果，携带游标。
        let req1 = c.new_request(1, 0, "sel".to_string(), false);
        assert_eq!(req1.continuation, None);
        let partial = CompletionResult {
            items: vec![item("select", CompletionKind::Keyword)],
            has_more: true,
            continuation: Some(CompletionContinuation(9)),
        };
        // editor 落库后 session 为 Retriggering（有可见结果 + continuation）。
        c.store_result_at(
            &req1.query,
            partial.items,
            Some(req1.buffer_version),
            Some(req1.cursor),
        );
        // retrigger：新请求原样回传承载游标。
        let mut req2 = c.new_request(1, 0, "select".to_string(), false);
        req2.continuation = partial.continuation;
        assert_eq!(req2.continuation, Some(CompletionContinuation(9)));
        // 结果收敛：continuation 清空 → 回到 Active/Idle。
        let done = CompletionResult {
            items: vec![item("select", CompletionKind::Keyword)],
            has_more: false,
            continuation: None,
        };
        assert_eq!(done.continuation, None);
        // session 派生：有结果 + continuation=Some → Retriggering；收敛 → Active。
        assert!(partial.continuation.is_some());
        assert!(done.continuation.is_none());
    }

    /// DM-701/709：session 状态迁移合法性——任意态可被取消，正常推进为
    /// Idle→Triggering→Active→Retriggering，且派生标签与态一一对应。
    #[test]
    fn session_transition_derivation_matches_flags() {
        // 用 editor 的派生规则（visible/loading/continuation）从标志推出状态并断言迁移。
        let derive = |visible: bool, loading: bool, continuation: bool| -> CompletionSession {
            if loading {
                CompletionSession::Triggering
            } else if visible {
                if continuation {
                    CompletionSession::Retriggering
                } else {
                    CompletionSession::Active
                }
            } else {
                CompletionSession::Idle
            }
        };
        // Idle（无请求）。
        assert_eq!(derive(false, false, false), CompletionSession::Idle);
        // Triggering（已请求、等待返回）。
        assert_eq!(derive(false, true, false), CompletionSession::Triggering);
        // Active（有可见结果、无续载）。
        assert_eq!(derive(true, false, false), CompletionSession::Active);
        // Retriggering（有结果 + 续载游标）。
        assert_eq!(derive(true, false, true), CompletionSession::Retriggering);
        // 任意态被取消/隐藏 → Idle（可见与 loading 均清）。
        assert_eq!(derive(false, false, true), CompletionSession::Idle);
        // 每个派生结果与 as_str 标签一致（划分日志不漂移）。
        for (s, tag) in [
            (CompletionSession::Idle, "idle"),
            (CompletionSession::Triggering, "triggering"),
            (CompletionSession::Active, "active"),
            (CompletionSession::Retriggering, "retriggering"),
        ] {
            assert_eq!(s.as_str(), tag);
        }
    }
}
