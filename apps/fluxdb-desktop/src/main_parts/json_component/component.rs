// json_component/component.rs —— 可复用的 JSON 组件核心。
//
// 与 `json_editor`（作为 `NavicatMain` 的一部分工作）不同，`JsonComponent` 是一个
// 自包含单元：它只依赖 `json_editor` 中已抽出的纯逻辑（校验 / pretty / 折叠 / 诊断），
// 自己持有值、展示状态，并在 `Edit` 模式内部创建和管理 `InputState`。
//
// 调用方只需传入 `value`、模式与展示配置，不直接持有输入框状态：
// - `Preview`：只读展示，内部不创建 `InputState`。
// - `Edit`：内部创建并维护 `InputState`，调用方通过组件方法读写。

/// JSON 组件：一个可复用的 JSON 展示 / 编辑单元。
///
/// 通过 `JsonMode` 参数选择行为；`InputState` 仅存在于 `Edit` 模式且始终由组件内部持有。
pub(crate) struct JsonComponent {
    mode: JsonMode,
    config: JsonEditorConfig,
    /// 纯展示 / 折叠 / 诊断状态（复用 `json_editor::types` 的 `JsonEditorState`）。
    state: JsonEditorState,
    /// `Edit` 模式内部持有的编辑器实体；`Preview` 模式恒为 `None`。调用方不直接接触该字段。
    input: Option<Entity<InputState>>,
    /// 折叠路径集合，随 `load` / `toggle_fold` 维护，用于重建折叠展示文本。
    folds: BTreeSet<String>,
}

impl JsonComponent {
    /// 以给定配置创建一个组件；默认处于 `Preview` 模式。
    pub(crate) fn new(config: JsonEditorConfig) -> Self {
        JsonComponent {
            mode: JsonMode::Preview,
            state: JsonEditorState::new(config.clone()),
            config,
            input: None,
            folds: BTreeSet::new(),
        }
    }

    /// 当前展示模式。
    #[allow(dead_code)] // 可复用组件的公开 API：编辑态能力供未来消费方接入，当前仅 Preview 被 Workbench 使用
    pub(crate) fn mode(&self) -> JsonMode {
        self.mode
    }

    /// 切换展示模式。进入 / 退出 `Edit` 会同步编辑态标记；`InputState` 按需在
    /// `begin_edit` 时加载（见 `is_editing` / `cancel_edit`）。
    #[allow(dead_code)] // 可复用组件的公开 API：编辑态能力供未来消费方接入，当前仅 Preview 被 Workbench 使用
    pub(crate) fn set_mode(&mut self, mode: JsonMode) {
        self.mode = mode;
        self.state.editing = mode.is_editable();
        if mode.is_editable() && self.input.is_none() {
            // 首次进入编辑态时才由组件创建 InputState（延迟到 `begin_edit` 真正需要时）。
            // 这里只标记模式；实体由 `begin_edit(window, cx)` 创建。
        } else if !mode.is_editable() {
            self.input = None;
        }
    }

    /// 载入源文本（纯逻辑，不依赖窗口）：pretty 规整并重建折叠节点，保留仍然存在的折叠路径。
    pub(crate) fn load(&mut self, source: &str) {
        let indent = self.config.indent_size;
        // `format_on_load`：加载即按标准 pretty 规整，作为查看 / 编辑的一致基线。
        let source = if self.config.format_on_load {
            format_pretty(source, indent).unwrap_or_else(|_| source.to_string())
        } else {
            source.to_string()
        };
        // `preserve_fold_state`：为 true 时保留仍存在路径的折叠，否则彻底展开。
        let old_folds = if self.config.preserve_fold_state {
            self.folds.clone()
        } else {
            BTreeSet::new()
        };
        match build_pretty(&source, indent, &BTreeSet::new()) {
            Ok(pretty) => {
                let kept = preserve_fold_paths(&old_folds, &pretty.nodes);
                let rebuilt = build_pretty(&source, indent, &kept).unwrap_or(pretty);
                self.folds = kept;
                self.state = JsonEditorState {
                    config: self.config.clone(),
                    source,
                    display: rebuilt.text,
                    diagnostic: None,
                    nodes: rebuilt.nodes,
                    dirty: false,
                    editing: self.mode.is_editable(),
                };
            }
            Err(dia) => {
                self.folds.clear();
                self.state = JsonEditorState {
                    config: self.config.clone(),
                    source,
                    display: String::new(),
                    diagnostic: Some(dia),
                    nodes: Vec::new(),
                    dirty: false,
                    editing: self.mode.is_editable(),
                };
            }
        }
    }

    /// 权威全文（未折叠 JSON）。
    #[allow(dead_code)] // 可复用组件的公开 API：编辑态能力供未来消费方接入，当前仅 Preview 被 Workbench 使用
    pub(crate) fn source(&self) -> String {
        self.state.source.clone()
    }

    /// 当前诊断；`None` 表示 JSON 合法。
    #[allow(dead_code)] // 可复用组件的公开 API：编辑态能力供未来消费方接入，当前仅 Preview 被 Workbench 使用
    pub(crate) fn diagnostic(&self) -> Option<JsonEditorDiagnostic> {
        self.state.diagnostic.clone()
    }

    /// 只读视图所需的状态（供渲染复用结构化行逻辑）。
    pub(crate) fn state(&self) -> &JsonEditorState {
        &self.state
    }

    /// 是否处于编辑态（组件内部已持有 `InputState`）。
    #[allow(dead_code)] // 可复用组件的公开 API：编辑态能力供未来消费方接入，当前仅 Preview 被 Workbench 使用
    pub(crate) fn is_editing(&self) -> bool {
        self.input.is_some()
    }

    /// 切换某条折叠路径（仅查看态意义；编辑态不展示折叠）。
    #[allow(dead_code)] // 可复用组件的公开 API：编辑态能力供未来消费方接入，当前仅 Preview 被 Workbench 使用
    pub(crate) fn toggle_fold(&mut self, path: &[JsonKey]) {
        toggle_fold_state(&mut self.state, path);
        self.folds = self.state.folded_set();
    }

    /// 进入编辑态：内部创建 `InputState`（若尚未创建）并填入 pretty 全文。
    /// `Preview` 模式通常不需要调用；`InputState` 只在 `Edit` 模式下由组件持有。
    #[allow(dead_code)] // 供未来消费方（如 Redis 值详情、Add Key JSON 表单）复用；当前仅 Preview 被 Workbench 使用
    pub(crate) fn begin_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) {
        self.mode = JsonMode::Edit;
        self.state.editing = true;
        if self.input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor(JSON_HIGHLIGHT_LANGUAGE)
                    .line_number(true)
                    .placeholder("值")
                    .rows(12)
            });
            self.input = Some(input);
        }
    }

    /// 退出编辑态：丢弃内部 `InputState` 与编辑草稿，回到 `Preview`。
    #[allow(dead_code)] // 供未来消费方复用；见 `begin_edit`
    pub(crate) fn cancel_edit(&mut self) {
        self.input = None;
        self.state.editing = false;
        self.state.dirty = false;
        self.state.diagnostic = None;
        self.mode = JsonMode::Preview;
    }

    /// 渲染主体：按当前模式派发到只读预览或编辑态界面（见 `render.rs` 的 `json_component_view`）。
    #[allow(dead_code)] // 可复用组件的完整渲染入口（含编辑态）；Workbench 轻量接入走 `render_preview`
    pub(crate) fn render(
        &self,
        colors: UiColors,
        _window: &mut Window,
        cx: &mut Context<NavicatMain>,
    ) -> Div {
        json_component_view(self, colors, _window, cx)
    }

    /// 只读预览渲染（`Preview` 模式）：不依赖窗口 / 交互，直接产出结构化高亮预览。
    ///
    /// 这是供轻量调用点（如 Workbench 结果区 `Text / JSON` 切换）复用的入口：
    /// 组件内部按 `Preview` 模态渲染，非法 JSON 自动回退为原文，保证结果始终可读。
    pub(crate) fn render_preview(&self, colors: UiColors) -> Div {
        let rows = json_editor_rows(self.state());
        json_component_rows_block(&rows, JsonEditorTheme::from_colors(colors), self.config.indent_size, colors)
    }
}

#[cfg(test)]
mod json_component_tests {
    use super::*;

    fn cfg() -> JsonEditorConfig {
        JsonEditorConfig::default()
    }

    fn key(path: &[JsonKey]) -> String {
        json_fold_path_key(path)
    }

    #[test]
    fn new_defaults_to_preview_and_clean() {
        let comp = JsonComponent::new(cfg());
        assert_eq!(comp.mode(), JsonMode::Preview);
        assert!(comp.source().is_empty());
        assert!(comp.diagnostic().is_none());
        assert!(!comp.is_editing());
    }

    #[test]
    fn load_pretty_formats_valid_json() {
        let mut comp = JsonComponent::new(cfg());
        comp.load(r#"{"a":1,"b":[1,2]}"#);
        assert_eq!(comp.source(), "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ]\n}");
        assert!(comp.diagnostic().is_none());
        // 可折叠节点覆盖根对象与 b 数组。
        assert!(comp.state().nodes.iter().any(|n| n.path.is_empty()));
    }

    #[test]
    fn load_invalid_json_preserves_raw_and_diagnostic() {
        let mut comp = JsonComponent::new(cfg());
        comp.load(r#"{"a": }"#);
        // 非法 JSON：source 保持原文（加载异步、不改写），给出诊断。
        assert_eq!(comp.source(), r#"{"a": }"#);
        assert!(comp.diagnostic().is_some());
    }

    #[test]
    fn set_mode_edit_marks_editable_without_entity() {
        // 切换到 Edit 只标记编辑态；`InputState` 实体延迟到 `begin_edit(window, cx)` 才创建，
        // 纯逻辑层（无窗口）不创建实体，符合「InputState 仅属于编辑模式且内部持有」。
        let mut comp = JsonComponent::new(cfg());
        comp.set_mode(JsonMode::Edit);
        assert_eq!(comp.mode(), JsonMode::Edit);
        // 无窗口上下文时不会误创建 InputState（纯逻辑可测）。
        assert!(!comp.is_editing());
    }

    #[test]
    fn toggle_fold_preserves_source() {
        let mut comp = JsonComponent::new(cfg());
        comp.load(r#"{"user":{"name":"n"},"age":1}"#);
        let path: Vec<JsonKey> = vec![JsonKey::Object("user".into())];
        comp.toggle_fold(&path);
        assert!(comp.folds.contains(&key(&path)));
        comp.toggle_fold(&path);
        assert!(!comp.folds.contains(&key(&path)));
        // 折叠 / 展开来回后，source 始终是完整 JSON。
        assert_eq!(comp.source(), "{\n  \"user\": {\n    \"name\": \"n\"\n  },\n  \"age\": 1\n}");
    }

    #[test]
    fn mode_edit_flag_reflected_in_state() {
        let mut comp = JsonComponent::new(cfg());
        comp.load(r#"{"a":1}"#);
        comp.set_mode(JsonMode::Edit);
        assert!(comp.state().editing);
    }
}
