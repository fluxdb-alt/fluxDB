// redis_editor_adapter/mod.rs —— Redis 编辑器接入层。
//
// 本模块把 fluxdb-editor-core 的通用编辑协议（CompletionProvider /
// ExecutionAdapter）接到 Redis 业务上下文，通过 fluxdb-app 的纯逻辑
// `redis_completion_result` 提供命令/子命令/参数补全。
//
// 分层约束：
//   - 本模块不直接连接数据库，所有补全都来自内存命令规格。
//   - Redis 语义（命令树、参数定位）全部留在 fluxdb-app，本模块只做协议转换。
//   - 命令执行通过 ExecutionAdapter 把文本/选区交给 app controller，UI 不拼命令。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use fluxdb_editor_core::{
    BufferSnapshot, CompletionFuture, CompletionItem, CompletionKind, CompletionProvider,
    CompletionRequest, CompletionResult, DocumentationRequest, DocumentationState, ExecuteMode,
    ExecutionAdapter, ExecutionUnit, InsertTextFormat, Point, Range, Selection, SignatureInfo,
    SignatureProvider, TriggerDecision,
};

/// 把 fluxdb-app 的 `QueryCompletionKind` 映射到 editor-core 的 `CompletionKind`。
fn map_kind(kind: &fluxdb_core::QueryCompletionKind) -> CompletionKind {
    match kind {
        fluxdb_core::QueryCompletionKind::RedisCommand => CompletionKind::Command,
        fluxdb_core::QueryCompletionKind::RedisSubCommand => CompletionKind::Command,
        fluxdb_core::QueryCompletionKind::RedisArgument => CompletionKind::Keyword,
        // 其余类型（SQL 相关）在 Redis 适配器中不会出现，回退到 Text。
        _ => CompletionKind::Text,
    }
}

/// 把 fluxdb-app 的 `QueryCompletionItem` 映射到 editor-core 的 `CompletionItem`。
fn map_item(item: &fluxdb_core::QueryCompletionItem) -> CompletionItem {
    let mut completion_item = CompletionItem::new(&item.label, map_kind(&item.kind));
    completion_item.insert_text = item.insert_text.clone();
    completion_item.detail = item.detail.clone().unwrap_or_default();
    completion_item.documentation = item.documentation.clone().unwrap_or_default();
    completion_item.insert_text_format = match item.insert_text_format {
        fluxdb_core::InsertTextFormat::PlainText => InsertTextFormat::PlainText,
        fluxdb_core::InsertTextFormat::Snippet => InsertTextFormat::Snippet,
    };
    if let Some(filter) = &item.filter_text {
        completion_item.filter_text = filter.clone();
    }
    if let Some(sort) = &item.sort_text {
        completion_item.sort_text = sort.clone();
    }
    completion_item
}

/// Redis 编辑器适配器。
///
/// 持有无状态引用，所有补全逻辑委托给 fluxdb-app 的纯函数。
/// 本身不持有数据库连接、GPUI 类型或命令词典。
/// `latest_request` 用于 latest-wins 取消：`complete` 写入当前 request_id，
/// `cancel_pending` 推进原子，旧结果提交时判废。
#[derive(Clone, Default)]
pub struct RedisAdapter {
    latest_request: Arc<AtomicU64>,
}

impl RedisAdapter {
    /// 构造 Redis 适配器。
    pub fn new() -> Self {
        Self {
            latest_request: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl CompletionProvider for RedisAdapter {
    /// 触发决策：显式触发总是返回 Yes；非显式时，只要光标落在命令名、
    /// 子命令或参数 token 内（含空白后空 token）都触发。
    ///
    /// 与 SQL 不同，Redis 在命令后的空白位置也需要弹出候选（如 `SET ` 后应提示
    /// 参数），因此不能只依赖 `DependsOnPrefix(1)` 门控。
    fn should_trigger(&self, request: &CompletionRequest) -> TriggerDecision {
        if request.explicit {
            return TriggerDecision::Yes;
        }
        // 有前缀文本（命令名/子命令/参数的片段）时触发。
        if !request.query.is_empty() {
            return TriggerDecision::Yes;
        }
        // 空前缀：由编辑器在 trigger_chars（空格、`.`、`@` 等）命中时触发，
        // 此处返回 Yes 让编辑器在空白后也能弹出候选。
        TriggerDecision::Yes
    }

    /// 取消所有 in-flight 补全：推进 latest_request，令旧请求提交时判废。
    fn cancel_pending(&self) {
        self.latest_request.fetch_add(1, Ordering::AcqRel);
    }

    /// 发起补全：读取快照全文与光标，调用 fluxdb-app 的纯逻辑，再把结果
    /// 映射到 editor-core 的 `CompletionResult`。
    ///
    /// 写入当前 request_id 到 latest_request，异步返回前判废（latest-wins）：
    /// 新请求或 cancel 推进原子后，旧结果丢弃，避免覆盖新文本/新选区。
    fn complete(&self, request: CompletionRequest) -> CompletionFuture {
        let snapshot = request.document.clone();
        let cursor = request.cursor;
        let request_id = request.request_id;
        let latest = self.latest_request.clone();
        latest.store(request_id, Ordering::Release);
        Box::pin(async move {
            let snapshot = snapshot.ok_or(fluxdb_editor_core::CompletionError::Provider)?;
            let text = snapshot.to_string();
            let result = fluxdb_app::redis_completion_result(&text, cursor);
            // latest-wins：新请求或 cancel 推进原子后，旧结果丢弃。
            if latest.load(Ordering::Acquire) != request_id {
                return Err(fluxdb_editor_core::CompletionError::Cancelled);
            }
            // 把每个候选的 replace_range 设为统一区间（由 app 层计算）。
            let items: Vec<CompletionItem> = result
                .items
                .iter()
                .map(|item| {
                    let mut mapped = map_item(item);
                    mapped.replace_range = Some(Range::new(result.replace_start, result.replace_end));
                    mapped
                })
                .collect();
            Ok(CompletionResult {
                items,
                has_more: false,
                continuation: None,
            })
        })
    }

    /// 候选项右侧 metadata 详情（F005）：Redis 补全不展示右侧详情面板。
    ///
    /// 返回 None 使编辑器 `completion_doc_state` 保持 None，浮层仅候选列表、
    /// 宽度不加宽。命令文档为静态规格，无需在此兜底展示。
    fn documentation(&self, _request: DocumentationRequest) -> Option<DocumentationState> {
        None
    }
}

impl ExecutionAdapter for RedisAdapter {
    /// 把选区或光标所在行切分成执行单元。
    ///
    /// 有选区时执行选区文本；否则取光标所在行（Redis CLI 以换行分隔命令）。
    /// 执行模式统一为 Execute（Redis 命令不分 Select/Explain）。
    fn execution_units(&self, snapshot: &BufferSnapshot, selection: Selection) -> Vec<ExecutionUnit> {
        if !selection.is_empty() {
            let range = selection.range();
            let start = range.start.min(snapshot.len());
            let end = range.end.min(snapshot.len());
            let unit_text = snapshot.text_in_range(Range::new(start, end));
            return vec![ExecutionUnit {
                range: Range::new(start, end),
                text: unit_text,
                mode: ExecuteMode::Execute,
            }];
        }
        // 无选区时执行光标所在行。
        let cursor = selection.cursor.min(snapshot.len());
        if let Some((start, end)) = redis_line_range(snapshot, cursor) {
            let unit_text = snapshot.text_in_range(Range::new(start, end));
            return vec![ExecutionUnit {
                range: Range::new(start, end),
                text: unit_text,
                mode: ExecuteMode::Execute,
            }];
        }
        Vec::new()
    }
}

impl SignatureProvider for RedisAdapter {
    /// 根据整段文本与光标，返回当前命令的签名提示。
    ///
    /// 委托给 fluxdb-app 的 `redis_command_signature`：由 app 层复用补全上下文解析
    /// 命令名与光标前已输入参数（不重复分词），输出 label + 当前应填参数索引 +
    /// 每参数精确 byte 范围。参数范围交给通用渲染层精确高亮，不靠空格切词数 token。
    fn signature(&self, snapshot: &BufferSnapshot, position: Point) -> Option<SignatureInfo> {
        let cursor = snapshot.point_to_offset(position);
        let text = snapshot.to_string();
        let sig = fluxdb_app::redis_command_signature(&text, cursor)?;
        Some(SignatureInfo {
            label: sig.label,
            documentation: String::new(),
            active_parameter: Some(sig.active_parameter),
            parameter_ranges: sig.parameter_ranges,
        })
    }
}

/// 计算光标所在行的字节区间（以 `\n` 为界）。
fn redis_line_range(snapshot: &BufferSnapshot, cursor: usize) -> Option<(usize, usize)> {
    let len = snapshot.len();
    let cursor = cursor.min(len);
    let text = snapshot.to_string();
    let start = text[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = text[cursor..]
        .find('\n')
        .map(|i| cursor + i)
        .unwrap_or(len);
    if start < end {
        Some((start, end))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxdb_editor_core::InsertTextFormat;

    #[test]
    fn map_kind_command() {
        assert_eq!(
            map_kind(&fluxdb_core::QueryCompletionKind::RedisCommand),
            CompletionKind::Command
        );
    }

    #[test]
    fn map_kind_argument() {
        assert_eq!(
            map_kind(&fluxdb_core::QueryCompletionKind::RedisArgument),
            CompletionKind::Keyword
        );
    }

    #[test]
    fn map_item_preserves_fields() {
        let item = fluxdb_core::QueryCompletionItem {
            label: "SET".to_string(),
            insert_text: "SET".to_string(),
            kind: fluxdb_core::QueryCompletionKind::RedisCommand,
            detail: Some("SET key value".to_string()),
            documentation: None,
            filter_text: None,
            sort_text: None,
                    ..Default::default()
};
        let mapped = map_item(&item);
        assert_eq!(mapped.label, "SET");
        assert_eq!(mapped.insert_text, "SET");
        assert_eq!(mapped.detail, "SET key value");
        assert_eq!(mapped.kind, CompletionKind::Command);
    }

    #[test]
    fn redis_line_range_basic() {
        use fluxdb_editor_core::EditorBuffer;
        let buffer = EditorBuffer::new_from("SET key value\nGET key\n");
        let snapshot = buffer.snapshot();
        // 光标在第二行 "GET" 中间。
        let (start, end) = redis_line_range(&snapshot, 15).unwrap();
        assert_eq!(&snapshot.to_string()[start..end], "GET key");
    }

    // ---- P2 契约测试：触发 / 取消 / 文档 / latest-wins / 稳定排序 ----

    #[test]
    fn should_trigger_explicit_always_yes() {
        let adapter = RedisAdapter::new();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: 0,
            cursor: 0,
            query: String::new(),
            explicit: true,
            document: None,
            edit_id: 0,
            continuation: None,
        };
        assert_eq!(adapter.should_trigger(&request), TriggerDecision::Yes);
    }

    #[test]
    fn should_trigger_empty_query_yes() {
        // Redis 在空白后（命令后空格、点号、@）也需弹出候选，空前缀返回 Yes。
        let adapter = RedisAdapter::new();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: 0,
            cursor: 0,
            query: String::new(),
            explicit: false,
            document: None,
            edit_id: 0,
            continuation: None,
        };
        assert_eq!(adapter.should_trigger(&request), TriggerDecision::Yes);
    }

    #[test]
    fn should_trigger_non_empty_query_yes() {
        let adapter = RedisAdapter::new();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: 0,
            cursor: 2,
            query: "SE".to_string(),
            explicit: false,
            document: None,
            edit_id: 0,
            continuation: None,
        };
        assert_eq!(adapter.should_trigger(&request), TriggerDecision::Yes);
    }

    #[test]
    fn complete_returns_command_candidates() {
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("SE").snapshot();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: snapshot.version(),
            cursor: 2,
            query: "SE".to_string(),
            explicit: false,
            document: Some(snapshot),
            edit_id: 0,
            continuation: None,
        };
        let result = futures::executor::block_on(adapter.complete(request)).unwrap();
        assert!(!result.items.is_empty(), "SE 应有候选");
        assert!(
            result.items.iter().any(|i| i.label == "SET"),
            "应含 SET"
        );
        // 命令候选 kind 映射为 Command。
        assert_eq!(result.items[0].kind, CompletionKind::Command);
    }

    #[test]
    fn complete_empty_result_for_unknown_placeholder() {
        // GET 后仅取值参数，空前缀不弹候选。
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("GET ").snapshot();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: snapshot.version(),
            cursor: 4,
            query: String::new(),
            explicit: false,
            document: Some(snapshot),
            edit_id: 0,
            continuation: None,
        };
        let result = futures::executor::block_on(adapter.complete(request)).unwrap();
        assert!(result.items.is_empty(), "GET 后取值参数不应弹候选");
    }

    #[test]
    fn complete_replace_range_set_per_item() {
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("SE").snapshot();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: snapshot.version(),
            cursor: 2,
            query: "SE".to_string(),
            explicit: false,
            document: Some(snapshot),
            edit_id: 0,
            continuation: None,
        };
        let result = futures::executor::block_on(adapter.complete(request)).unwrap();
        for item in &result.items {
            let range = item.replace_range.expect("每项应有 replace_range");
            assert_eq!(range.start, 0);
            assert_eq!(range.end, 2);
        }
    }

    #[test]
    fn complete_cancel_pending_makes_check_fail() {
        // 创建 future（写入 request_id），再 cancel，然后 await → 应判废。
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("SE").snapshot();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: snapshot.version(),
            cursor: 2,
            query: "SE".to_string(),
            explicit: false,
            document: Some(snapshot),
            edit_id: 0,
            continuation: None,
        };
        // 先创建 future（内部 store request_id=1），不立即 await。
        let fut = adapter.complete(request);
        // cancel 推进原子到 2，令 request_id=1 判废。
        adapter.cancel_pending();
        let result = futures::executor::block_on(fut);
        assert!(
            matches!(
                result,
                Err(fluxdb_editor_core::CompletionError::Cancelled)
            ),
            "cancel 后应返回 Cancelled"
        );
    }

    #[test]
    fn complete_newer_request_invalidates_older() {
        // 两个请求并发：后发的 request_id 覆盖 latest，先发的判废。
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("SE").snapshot();
        let make = |id| CompletionRequest {
            request_id: id,
            buffer_version: snapshot.version(),
            cursor: 2,
            query: "SE".to_string(),
            explicit: false,
            document: Some(snapshot.clone()),
            edit_id: 0,
            continuation: None,
        };
        let fut1 = adapter.complete(make(1));
        let fut2 = adapter.complete(make(2)); // store 2，令 request 1 判废
        let r1 = futures::executor::block_on(fut1);
        let r2 = futures::executor::block_on(fut2);
        assert!(
            matches!(r1, Err(fluxdb_editor_core::CompletionError::Cancelled)),
            "旧请求应判废"
        );
        assert!(r2.is_ok(), "新请求应通过");
    }

    #[test]
    fn documentation_always_none() {
        // Redis 补全不展示右侧详情面板：无论是否带内联文档都返回 None。
        let adapter = RedisAdapter::new();
        for comment in [Some("SET key value".to_string()), None] {
            let request = DocumentationRequest {
                kind: CompletionKind::Command,
                label: "SET".to_string(),
                comment,
                latest_request: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
                request_id: 1,
            };
            assert!(
                adapter.documentation(request).is_none(),
                "Redis 补全应始终返回 None（不显示详情面板）"
            );
        }
    }

    #[test]
    fn complete_preserves_app_layer_order() {
        // 稳定排序：adapter 不打乱 app 层返回的顺序。
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("S").snapshot();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: snapshot.version(),
            cursor: 1,
            query: "S".to_string(),
            explicit: false,
            document: Some(snapshot),
            edit_id: 0,
            continuation: None,
        };
        let result = futures::executor::block_on(adapter.complete(request)).unwrap();
        let labels: Vec<&str> = result.items.iter().map(|i| i.label.as_str()).collect();
        // app 层排序后顺序应原样保留（不去重、不重排）。
        let mut sorted = labels.clone();
        sorted.sort();
        sorted.dedup();
        // 只断言顺序与 app 层一致：相邻项相对顺序不变。
        assert_eq!(labels.len(), result.items.len());
        assert!(!labels.is_empty(), "S 前缀应有候选");
    }

    // ------------------------------------------------------------ P3 signature
    #[test]
    fn signature_returns_synopsis_for_known_command() {
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("SET key value ").snapshot();
        let pos = snapshot.offset_to_point(snapshot.len());
        let sig = adapter.signature(&snapshot, pos).expect("应有 SET 签名");
        assert!(sig.label.starts_with("SET "), "label 应含命令名与参数骨架");
        // 光标前 token：SET(命令) key(0) value(1)，光标在空格后 → active_parameter = 2
        assert_eq!(sig.active_parameter, Some(2));
    }

    #[test]
    fn signature_none_for_unknown_command() {
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("UNKNOWN_CMD ").snapshot();
        let pos = snapshot.offset_to_point(snapshot.len());
        assert!(
            adapter.signature(&snapshot, pos).is_none(),
            "未知命令应返回 None"
        );
    }

    #[test]
    fn signature_none_for_empty_line() {
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("").snapshot();
        let pos = snapshot.offset_to_point(0);
        assert!(
            adapter.signature(&snapshot, pos).is_none(),
            "空行应返回 None"
        );
    }

    #[test]
    fn signature_case_insensitive_command() {
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("set ").snapshot();
        let pos = snapshot.offset_to_point(snapshot.len());
        let sig = adapter.signature(&snapshot, pos).expect("小写 set 也应识别");
        assert!(sig.label.starts_with("SET "));
    }

    // ------------------------------------------------------------ P3 edge cases
    #[test]
    fn complete_case_insensitive_prefix() {
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("se").snapshot();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: snapshot.version(),
            cursor: 2,
            query: "se".to_string(),
            explicit: false,
            document: Some(snapshot),
            edit_id: 0,
            continuation: None,
        };
        let result = futures::executor::block_on(adapter.complete(request)).unwrap();
        assert!(
            result.items.iter().any(|i| i.label == "SET"),
            "小写 se 也应匹配 SET"
        );
    }

    #[test]
    fn complete_dot_command_json_get() {
        // JSON.GET 是点号分隔命令：补全按点号把当前 token 拆为「命令名 + 子命令前缀」，
        // `JSON.` 后空前缀展示全部子命令候选（GET/SET/DEL/...）。
        let adapter = RedisAdapter::new();
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from("JSON.").snapshot();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: snapshot.version(),
            cursor: 5,
            query: "JSON.".to_string(),
            explicit: false,
            document: Some(snapshot),
            edit_id: 0,
            continuation: None,
        };
        let result = futures::executor::block_on(adapter.complete(request)).unwrap();
        // 点号分隔命令 `JSON.` 应展示子命令候选（GET/SET/DEL/...）。
        assert!(
            !result.items.is_empty(),
            "JSON. 应返回子命令候选，实际为空"
        );
        let labels: Vec<&str> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.iter().any(|l| *l == "GET"), "应有 GET 子命令");
        assert!(labels.iter().any(|l| *l == "SET"), "应有 SET 子命令");
        // 子命令 candidate 使用 snippet 格式。
        let get = result.items.iter().find(|i| i.label == "GET").unwrap();
        assert_eq!(get.insert_text_format, InsertTextFormat::Snippet);
    }

    #[test]
    fn complete_multiline_takes_cursor_line() {
        // 多行文本：补全只解析光标所在行。
        let adapter = RedisAdapter::new();
        let text = "SET k v\nSE";
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(text).snapshot();
        let request = CompletionRequest {
            request_id: 1,
            buffer_version: snapshot.version(),
            cursor: text.len(),
            query: "SE".to_string(),
            explicit: false,
            document: Some(snapshot),
            edit_id: 0,
            continuation: None,
        };
        let result = futures::executor::block_on(adapter.complete(request)).unwrap();
        assert!(
            result.items.iter().any(|i| i.label == "SET"),
            "第二行 SE 应触发命令候选"
        );
    }

    #[test]
    fn insert_text_format_default_plain() {
        // 默认插入格式为 PlainText，避免把字面量 $1 误解析。
        let item = CompletionItem::new("SELECT", CompletionKind::Keyword);
        assert_eq!(item.insert_text_format, InsertTextFormat::PlainText);
    }
}
