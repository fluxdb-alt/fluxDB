// json_editor/types.rs —— 可复用 JSON 编辑组件的配置项、诊断、折叠路径与运行态类型。
//
// 这些类型不依赖任何数据库连接，供 Redis JSON 值详情及其它调用点复用。

/// 折叠路径：根节点用空路径表示；object 用 key，array 用下标，逐层定位到某个可折叠节点。
/// 例如 `["user", "address"]` 表示根对象下 `user` 对象的 `address` 字段值。
/// 数组折叠用 `JsonKey::Index`，对象字段折叠用 `JsonKey::Object`。
pub(crate) type JsonFoldPath = Vec<JsonKey>;

/// 折叠路径中的一层 key。object 字段用 key 名，array 元素用下标。
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum JsonKey {
    Object(String),
    Index(usize),
}

impl JsonKey {
    fn key_id(&self) -> String {
        match self {
            JsonKey::Object(k) => format!("o:{k}"),
            JsonKey::Index(i) => format!("i:{i}"),
        }
    }
}

/// JSON 诊断（校验失败信息）。行列均为 0-based，可映射到编辑器内 byte offset 用于高亮。
#[derive(Clone, Debug)]
pub(crate) struct JsonEditorDiagnostic {
    /// 面向用户的中文错误描述，例如「第 2 行第 5 列，非法字符」。
    pub message: String,
    /// 错误所在字节位置（0-based），对应 `serde_json::Error` 的行列。
    #[allow(dead_code)] // 可复用组件的定位元数据，供后续精确字节级高亮/跳转使用
    pub offset: usize,
    /// 错误所在行（0-based，来自 serde_json）。
    pub line: usize,
    /// 错误所在列（0-based，来自 serde_json）。
    pub column: usize,
    /// 可高亮的字节范围 `[start, end)`（闭区间近似）；若无法确定范围则与 offset 重合。
    #[allow(dead_code)] // 可复用组件的定位元数据，供后续精确字节级高亮/跳转使用
    pub span: (usize, usize),
}

impl JsonEditorDiagnostic {
    /// 组装成适合 `show_message` 的轻量提示文案。
    pub fn summary(&self) -> String {
        format!(
            "JSON 格式错误：第 {} 行第 {} 列，{}",
            self.line + 1,
            self.column + 1,
            self.message
        )
    }

    /// 悬浮气泡文案：单行 `第 N 行第 M 列：错误原因`，锚定在错误 token 附近。
    pub fn bubble_text(&self) -> String {
        format!("第 {} 行第 {} 列：{}", self.line + 1, self.column + 1, self.message)
    }
}

/// 编辑器配置项：全部取自组件调用点，组件内部不写死主题或行为。
#[derive(Clone, Debug)]
pub(crate) struct JsonEditorConfig {
    /// 是否可编辑（只读时仍可选择 / 复制 / 折叠）。
    pub editable: bool,
    /// 是否对 JSON 做语法高亮。
    pub syntax_highlight: bool,
    /// 是否显示错误定位高亮（非法 JSON 时在编辑器内标红）。
    pub diagnostics: bool,
    /// 是否支持 object / array 折叠。
    pub folding: bool,
    /// 是否显示行列号（gutter 中的行号列）。
    pub line_numbers: bool,
    /// 是否显示左侧 gutter（折叠按钮 + 层级线 + 行号）。
    pub show_gutter: bool,
    /// 赋值时是否自动 pretty 格式化。
    pub format_on_load: bool,
    /// 保存前是否强制 pretty（配合保存链路使用；本组件保存前校验始终以 serde_json 为准）。
    pub format_on_save: bool,
    /// 编辑后重新解析时是否保留仍然存在的折叠路径。
    pub preserve_fold_state: bool,
    /// 编辑区最小可见行数（px 兜底，防止面板过矮时无法阅读）。
    pub min_rows: usize,
    /// 缩进空格数（pretty 采用 2 空格，与新 Navicat 规格一致）。
    pub indent_size: usize,
    /// 空值占位文案。
    pub placeholder: String,
}

impl Default for JsonEditorConfig {
    fn default() -> Self {
        JsonEditorConfig {
            editable: true,
            syntax_highlight: true,
            diagnostics: true,
            folding: true,
            line_numbers: true,
            show_gutter: true,
            format_on_load: true,
            format_on_save: false,
            preserve_fold_state: true,
            min_rows: 8,
            indent_size: 2,
            placeholder: "(空 JSON)".to_string(),
        }
    }
}

/// 可折叠节点：为 object / array。记录其在 pretty 文本中的起始/结束行和折叠状态。
#[derive(Clone, Debug)]
pub(crate) struct JsonFoldNode {
    pub path: JsonFoldPath,
    /// 节点起始行（0-based，pretty 文本内）。
    pub start_line: usize,
    /// 节点结束行（0-based，右侧括号所在行）。
    #[allow(dead_code)] // 折叠在查看态为单行占位，end_line 保留作折叠范围元数据
    pub end_line: usize,
    /// 当前是否折叠。
    pub folded: bool,
}

/// 运行态：绑定一个 `InputState`（编辑 + 高亮），并持有权威全文与折叠/诊断状态。
pub(crate) struct JsonEditorState {
    pub config: JsonEditorConfig,
    /// 权威全文（未折叠的完整 JSON；保存以它或 InputState 当前文本为准）。
    pub source: String,
    /// 当前展示文本（pretty + 折叠占位；未折叠时与 source 的 pretty 形式一致）。
    pub display: String,
    /// 当前诊断；`None` 表示 JSON 合法。
    pub diagnostic: Option<JsonEditorDiagnostic>,
    /// 可折叠节点（覆盖整棵树中的 object/array）。
    pub nodes: Vec<JsonFoldNode>,
    /// 是否脏（相对最后一次加载的服务端值）。
    pub dirty: bool,
    /// 是否处于编辑态。
    pub editing: bool,
}

impl JsonEditorState {
    pub fn new(config: JsonEditorConfig) -> Self {
        JsonEditorState {
            config,
            source: String::new(),
            display: String::new(),
            diagnostic: None,
            nodes: Vec::new(),
            dirty: false,
            editing: false,
        }
    }

    /// 由当前节点折叠态推导出折叠路径集合，供 `build_pretty` 生成折叠展示文本。
    pub fn folded_set(&self) -> std::collections::BTreeSet<String> {
        self.nodes
            .iter()
            .filter(|n| n.folded)
            .map(|n| json_fold_path_key(&n.path))
            .collect()
    }
}

#[cfg(test)]
mod json_editor_types_tests {
    use super::*;

    #[test]
    fn config_defaults_satisfy_navicat_spec() {
        // 默认配置应满足设计文档（新 Navicat）规格：
        // 可编辑、语法高亮、错误诊断、折叠、行列号、gutter、载入时格式化、保留折叠态；
        // 保存前不自动格式化（由保存链路决定，避免改动用户已有格式）。
        let cfg = JsonEditorConfig::default();
        assert!(cfg.editable);
        assert!(cfg.syntax_highlight);
        assert!(cfg.diagnostics);
        assert!(cfg.folding);
        assert!(cfg.line_numbers);
        assert!(cfg.show_gutter);
        assert!(cfg.format_on_load);
        assert!(!cfg.format_on_save);
        assert!(cfg.preserve_fold_state);
        assert_eq!(cfg.min_rows, 8);
        assert_eq!(cfg.indent_size, 2);
        assert!(!cfg.placeholder.is_empty());
    }

    #[test]
    fn state_initializes_clean() {
        let st = JsonEditorState::new(JsonEditorConfig::default());
        assert!(st.source.is_empty());
        assert!(st.display.is_empty());
        assert!(st.diagnostic.is_none());
        assert!(st.nodes.is_empty());
        assert!(!st.dirty);
        assert!(!st.editing);
        assert!(st.folded_set().is_empty());
    }
}
