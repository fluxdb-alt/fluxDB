// json_editor/controller.rs —— 与 `NavicatMain` / `InputState` 协作的 JSON 编辑器控制逻辑。
//
// 职责：维护 `redis_json_editor` 状态（权威全文 `source`、诊断、折叠），并把 `source` 与
// `redis_json_key_value_input`（InputState::code_editor）双向同步：
// - 查看态：结构化行渲染（折叠/层级线/错误高亮），不展示 InputState。
// - 编辑态：InputState 展示完整 pretty JSON，用户直接编辑全文。
// - 保存仍走现有 `request_redis_key_value_apply`（connector 侧 `JSON.SET key . value`）。

impl NavicatMain {
    /// 该 key 是否为当前 JSON 编辑器归属的活动 key。
    fn redis_json_editor_is_active(&self, tab_id: TabId, key: &str) -> bool {
        self.redis_json_editor_active
            .as_ref()
            .is_some_and(|(t, k)| *t == tab_id && k == key)
    }

    /// 每帧同步 JSON 编辑器状态：
    /// 1. 切换 key 时，用已加载值（或 preview）作为权威 `source`，重置折叠/诊断/编辑态；
    /// 2. 进入编辑态前确保 `source` 反映最新完整值。
    fn sync_redis_json_editor(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = detail.key.clone();
        let active = (tab_id, key.clone());

        if self.redis_json_editor_active.as_ref() != Some(&active) {
            // 首次进入 / 切换 key：以「已加载值」为基线重建编辑器状态。
            self.redis_json_editor_active = Some(active);
            self.reset_redis_json_editor(tab_id, detail, window, cx);
            return;
        }

        // 同一 key：完整值刚加载（例如点击「加载全部」或打开完整详情）且与当前基线不同 → 刷新基线。
        // 编辑态下不覆盖用户输入；仅查看态更新展示基线。
        if !self.redis_json_editor.editing {
            if let Some(st) = self.redis_string_values.get(&(tab_id, key.clone())) {
                if st.loaded_all && !st.value.is_empty() && st.value != self.redis_json_editor.source {
                    self.set_redis_json_source(st.value.clone(), window, cx);
                    return;
                }
            }
        }

        // 同一 key：若未加载值（首次 preview 或已清缓存）则回退到列表 preview 作为展示基线。
        if self.redis_json_editor.source.is_empty() {
            self.set_redis_json_source(detail.value.clone(), window, cx);
        }
    }

    /// 切换 key 时的完整重置：编辑态强制退出、草稿清除、诊断清空，并以已加载值重建状态。
    fn reset_redis_json_editor(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.redis_json_editor.editing = false;
        self.redis_json_editor.dirty = false;
        self.redis_json_editor.diagnostic = None;
        self.redis_key_value_drafts.remove(&(tab_id, detail.key.clone()));
        let loaded = self
            .redis_string_values
            .get(&(tab_id, detail.key.clone()))
            .map(|state| state.value.clone())
            .unwrap_or_else(|| detail.value.clone());
        self.set_redis_json_source(loaded, window, cx);
    }

    /// 用指定文本重置编辑器：更新 `source`、重建 pretty/折叠节点、清空诊断。
    ///
    /// 查看态下不向 InputState 写入任何内容（InputState 仅在编辑态承载全文编辑，进入编辑态时
    /// 会由 `begin_redis_json_edit` 重新设置），从而避免非编辑态 set_value 触发 Change 回写污染 `source`。
    fn set_redis_json_source(&mut self, source: String, _window: &mut Window, cx: &mut Context<Self>) {
        let indent = self.redis_json_editor.config.indent_size;
        // `format_on_load`：加载即按标准 pretty 规整，作为查看/编辑的一致基线。
        let source = if self.redis_json_editor.config.format_on_load {
            format_pretty(&source, indent).unwrap_or(source)
        } else {
            source
        };
        // `preserve_fold_state`：为 true 时保留仍存在路径的折叠，否则彻底展开。
        let old_folds = if self.redis_json_editor.config.preserve_fold_state {
            self.redis_json_editor.folded_set()
        } else {
            BTreeSet::new()
        };
        match build_pretty(&source, indent, &BTreeSet::new()) {
            Ok(pretty) => {
                self.redis_json_editor.source = source;
                let kept = preserve_fold_paths(&old_folds, &pretty.nodes);
                // 用保留后的折叠路径重新生成展示文本与折叠态。
                let rebuilt = build_pretty(&self.redis_json_editor.source, indent, &kept)
                    .unwrap_or(pretty);
                self.redis_json_editor.nodes = rebuilt.nodes;
                self.redis_json_editor.display = rebuilt.text;
                self.redis_json_editor.diagnostic = None;
            }
            Err(dia) => {
                self.redis_json_editor.source = source;
                self.redis_json_editor.diagnostic = Some(dia);
                self.redis_json_editor.nodes = Vec::new();
            }
        }
        cx.notify();
    }

    /// 进入编辑态：以「完整 pretty JSON」为输入框基线，聚焦编辑器，允许直接改全文。
    fn begin_redis_json_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.redis_json_editor.source.clone();
        let pretty = match format_pretty(&source, self.redis_json_editor.config.indent_size) {
            Ok(text) => text,
            Err(_) => source,
        };
        self.redis_json_editor.editing = true;
        self.redis_json_editor.dirty = false;
        self.redis_key_value_syncing = true;
        self.redis_json_key_value_input.update(cx, |input, cx| {
            input.set_value(pretty, window, cx);
        });
        self.redis_key_value_syncing = false;
        self.redis_json_key_value_input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    /// 取消编辑：恢复最近一次加载的服务端完整值，清空诊断与草稿，回到结构化查看态。
    fn cancel_redis_json_edit(
        &mut self,
        tab_id: TabId,
        detail: &RedisKeyDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = detail.key.clone();
        self.redis_json_editor.editing = false;
        self.redis_json_editor.dirty = false;
        self.redis_json_editor.diagnostic = None;
        let loaded = self
            .redis_string_values
            .get(&(tab_id, key.clone()))
            .map(|state| state.value.clone())
            .unwrap_or_else(|| detail.value.clone());
        self.redis_key_value_drafts.remove(&(tab_id, key));
        self.set_redis_json_source(loaded, window, cx);
    }

    /// 编辑态下输入变化：把 InputState 全文写回 `source`，重新校验并重建折叠节点（保留仍存在折叠）。
    fn on_redis_json_input_change(
        &mut self,
        tab_id: TabId,
        key: &str,
        cx: &mut Context<Self>,
    ) {
        // 仅处理编辑态下的真实用户输入。查看态的 InputState 仅承载展示（且折叠不再回写它），
        // 任何非编辑态 Change（例如程序化 set_value 触发的延迟事件）都必须忽略，严禁把折叠
        // display 当草稿回写 `source`，避免原始完整 JSON 被 `{ … }` / `[ … ]` 污染。
        if !self.redis_json_editor.editing {
            return;
        }
        if self.redis_key_value_syncing {
            return;
        }
        if !self.redis_json_editor_is_active(tab_id, key) {
            return;
        }
        let text = self.redis_json_key_value_input.read(cx).value().to_string();
        let loaded = self
            .redis_string_values
            .get(&(tab_id, key.to_string()))
            .map(|state| state.value.clone())
            .unwrap_or_default();
        self.redis_json_editor.dirty = text != loaded;
        self.redis_json_editor.source = text;
        let indent = self.redis_json_editor.config.indent_size;
        let old_folds = self.redis_json_editor.folded_set();
        match build_pretty(&self.redis_json_editor.source, indent, &BTreeSet::new()) {
            Ok(pretty) => {
                self.redis_json_editor.diagnostic = None;
                let kept = preserve_fold_paths(&old_folds, &pretty.nodes);
                let rebuilt = build_pretty(&self.redis_json_editor.source, indent, &kept)
                    .unwrap_or(pretty);
                self.redis_json_editor.nodes = rebuilt.nodes;
                self.redis_json_editor.display = rebuilt.text;
            }
            Err(dia) => {
                self.redis_json_editor.diagnostic = Some(dia);
            }
        }
        self.sync_redis_json_input_diagnostics(cx);
        cx.notify();
    }

    /// 把当前 JSON 诊断同步到 InputState 的内建 DiagnosticSet（CodeEditor 内建能力），
    /// 使编辑态在错误 token 位置显示红色波浪下划线，并在悬停时展示错误气泡；
    /// JSON 合法时清空标记与气泡。
    ///
    /// `config.diagnostics = false` 时，即使内部检测到非法 JSON，也一律清空
    /// InputState 的既有诊断，不写入新诊断——UI 不显示错误高亮/气泡；
    /// 保存前的 JSON 合法性校验仍由 `redis_json_save_prepare` / 保存分支独立承担。
    fn sync_redis_json_input_diagnostics(&mut self, cx: &mut Context<Self>) {
        use gpui_component::highlighter::{Diagnostic, DiagnosticSeverity};
        // 纯函数决定「写入 InputState 的诊断」：`config.diagnostics=false` 时返回 None（不写入），
        // 但下方 update 仍会 `diagnostics.clear()`，从而清空既有红波浪/气泡。
        let diagnostic = redis_json_input_diagnostic(
            self.redis_json_editor.config.diagnostics,
            &self.redis_json_editor.diagnostic,
        );
        self.redis_json_key_value_input.update(cx, |input, _| {
            let Some(diagnostics) = input.diagnostics_mut() else {
                return;
            };
            diagnostics.clear();
            if let Some(dia) = diagnostic {
                let start = Position::new(dia.line as u32, dia.column as u32);
                let end = Position::new(dia.line as u32, dia.column as u32 + 1);
                diagnostics.push(
                    Diagnostic::new(start..end, dia.bubble_text())
                        .with_severity(DiagnosticSeverity::Error),
                );
            }
        });
    }

    /// 校验当前 JSON 源文本；返回 `Some(诊断)` 表示非法。
    fn redis_json_validate(&self) -> Option<JsonEditorDiagnostic> {
        self.redis_json_editor.diagnostic.clone()
    }

    /// 格式化：仅编辑态生效。合法 JSON 则 pretty 回填并清空错误；非法则不改写文本、更新诊断并提示。
    fn redis_json_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.redis_json_key_value_input.read(cx).value().to_string();
        match format_pretty(&text, self.redis_json_editor.config.indent_size) {
            Ok(pretty) => {
                self.redis_json_editor.source = pretty.clone();
                self.redis_json_editor.diagnostic = None;
                self.redis_json_editor.dirty = true;
                self.redis_key_value_syncing = true;
                self.redis_json_key_value_input.update(cx, |input, cx| {
                    input.set_value(pretty, window, cx);
                });
                self.redis_key_value_syncing = false;
                self.show_message("JSON 已格式化", AppMessageKind::Success, cx);
            }
            Err(dia) => {
                self.redis_json_editor.diagnostic = Some(dia.clone());
                self.sync_redis_json_input_diagnostics(cx);
                self.show_message(dia.summary(), AppMessageKind::Error, cx);
            }
        }
        cx.notify();
    }

    /// 切换某个 object/array 节点的折叠（仅查看态生效；编辑态不展示折叠按钮）。
    ///
    /// 折叠只改动 `display` / `nodes`，绝不回写 `source`，也绝不向 InputState 写入折叠后的
    /// 展示文本（查看态不需要同步 input，避免其 Change 事件把 `{ … }` 占位当成草稿污染 source）。
    fn redis_json_toggle_fold(&mut self, path: Vec<JsonKey>, _window: &mut Window, cx: &mut Context<Self>) {
        toggle_fold_state(&mut self.redis_json_editor, &path);
        cx.notify();
    }
}

/// 在 `JsonEditorState` 上切换某路径的折叠并重建展示文本（折叠的核心状态迁移）。
///
/// `source` 始终是权威完整 JSON，折叠只重建 `display` / `nodes`，因而反复折叠/展开不会破坏
/// 原始数据，也不会把 `{ … }` / `[ … ]` 占位混进 `source`。
fn toggle_fold_state(state: &mut JsonEditorState, path: &[JsonKey]) {
    let indent = state.config.indent_size;
    let old_folds = state.folded_set();
    let new_folds = toggle_fold(&old_folds, path);
    if let Ok(pretty) = build_pretty(&state.source, indent, &new_folds) {
        state.nodes = pretty.nodes;
        state.display = pretty.text;
    }
}

/// 决定「应写入 InputState 的诊断」，纯函数便于单测：
///
/// - `diagnostics` 开启时，返回内部检测到的诊断（含 `None` —— 无诊断即 JSON 合法）。
/// - `diagnostics` 关闭时，一律返回 `None`，即不向 InputState 写入任何新诊断，
///   但调用方仍会 `diagnostics.clear()` 清空既有标记，保证 `diagnostics=false` 下
///   编辑态绝不展示错误高亮/悬停气泡（保存前合法性校验仍由保存分支独立承担）。
fn redis_json_input_diagnostic(
    diagnostics: bool,
    detected: &Option<JsonEditorDiagnostic>,
) -> Option<JsonEditorDiagnostic> {
    if !diagnostics {
        return None;
    }
    detected.clone()
}

#[cfg(test)]
mod redis_fold_state_tests {
    use super::*;

    /// 由完整 JSON 文本构造一个「查看态基线」的编辑器状态（未折叠）。
    fn state_from(src: &str) -> JsonEditorState {
        let mut st = JsonEditorState::new(JsonEditorConfig::default());
        let pretty = build_pretty(src, 2, &BTreeSet::new()).unwrap();
        st.source = src.to_string();
        st.display = pretty.text;
        st.nodes = pretty.nodes;
        st
    }

    fn is_folded(st: &JsonEditorState, path: &[JsonKey]) -> bool {
        st.nodes
            .iter()
            .any(|n| n.path == path && n.folded)
    }

    #[test]
    fn folding_preserves_source_without_placeholder() {
        // 折叠后 `source` 仍是完整 JSON，不含 `{ … }` / `[ … ]` 占位；占位只出现在查看态 display。
        let src = r#"{"user":{"name":"n"},"tags":["a","b"]}"#;
        let mut st = state_from(src);
        toggle_fold_state(&mut st, &[JsonKey::Object("user".into())]);
        assert_eq!(st.source, src);
        assert!(!st.source.contains('\u{2026}'));
        assert!(st.display.contains('\u{2026}'));
        // 折叠后的 source 依旧能重新解析出完整可折叠节点树。
        assert!(build_pretty(&st.source, 2, &BTreeSet::new()).is_ok());
    }

    #[test]
    fn toggling_same_path_folds_then_expands() {
        let mut st = state_from(r#"{"user":{"name":"n"},"age":1}"#);
        let path: Vec<JsonKey> = vec![JsonKey::Object("user".into())];
        toggle_fold_state(&mut st, &path);
        assert!(is_folded(&st, &path), "第一次点击应折叠");
        toggle_fold_state(&mut st, &path);
        assert!(!is_folded(&st, &path), "第二次点击应展开");
        // 来回折叠/展开后，source 始终不变。
        assert_eq!(st.source, r#"{"user":{"name":"n"},"age":1}"#);
    }

    #[test]
    fn folding_one_node_then_folding_sibling_still_works() {
        // 折叠 a 之后，兄弟节点 b 仍可继续折叠（行号重建不该影响后续折叠）。
        let src = r#"{"a":{"x":1},"b":{"y":2}}"#;
        let mut st = state_from(src);
        toggle_fold_state(&mut st, &[JsonKey::Object("a".into())]);
        toggle_fold_state(&mut st, &[JsonKey::Object("b".into())]);
        assert!(is_folded(&st, &[JsonKey::Object("a".into())]));
        assert!(is_folded(&st, &[JsonKey::Object("b".into())]));
        assert_eq!(st.source, src);
    }

    #[test]
    fn root_node_can_fold_via_empty_path() {
        // 根对象用空路径表示，同样可以折叠/展开。
        let src = r#"{"a":1,"b":2}"#;
        let mut st = state_from(src);
        let root: Vec<JsonKey> = Vec::new();
        toggle_fold_state(&mut st, &root);
        assert!(is_folded(&st, &root));
        toggle_fold_state(&mut st, &root);
        assert!(!is_folded(&st, &root));
        assert_eq!(st.source, src);
    }
}

#[cfg(test)]
mod redis_json_input_diagnostic_tests {
    use super::*;

    /// 构造一个「非法 JSON」诊断样例，仅用于断言诊断是否会被写入 InputState。
    fn sample_diagnostic() -> JsonEditorDiagnostic {
        JsonEditorDiagnostic {
            message: "非法字符".to_string(),
            offset: 0,
            line: 0,
            column: 0,
            span: (0, 1),
        }
    }

    #[test]
    fn diagnostics_enabled_returns_detected() {
        // `config.diagnostics=true`：返回内部检测到的非法诊断（并保留其定位信息）。
        let detected = Some(sample_diagnostic());
        let out = redis_json_input_diagnostic(true, &detected);
        assert!(out.is_some());
        assert_eq!(out.unwrap().message, "非法字符");
    }

    #[test]
    fn diagnostics_enabled_no_error_returns_none() {
        // 开启诊断但 JSON 合法（无内部诊断）时，同样不写入任何诊断。
        assert!(redis_json_input_diagnostic(true, &None).is_none());
    }

    #[test]
    fn diagnostics_disabled_never_writes_even_when_detected() {
        // `config.diagnostics=false`：即使检测到非法 JSON，也一律不向 InputState 写入诊断，
        // 使编辑态不显示红波浪/悬停气泡；保存前的合法性校验仍由保存分支独立承担。
        let detected = Some(sample_diagnostic());
        assert!(redis_json_input_diagnostic(false, &detected).is_none());
        assert!(redis_json_input_diagnostic(false, &None).is_none());
    }

    #[test]
    fn config_defaults_keep_diagnostics_enabled() {
        // 默认配置下 diagnostics 开启，确保现有编辑态错误高亮行为不被本次改动破坏。
        let cfg = JsonEditorConfig::default();
        assert!(cfg.diagnostics);
        assert!(cfg.editable);
        assert!(cfg.folding);
        assert!(cfg.line_numbers);
        assert!(cfg.show_gutter);
        assert!(cfg.syntax_highlight);
    }
}
