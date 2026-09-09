// json_editor/theme.rs —— 明亮/暗黑主题的 JSON token 颜色与错误色。
//
// 所有颜色都从 `UiColors` + 明暗标记派生，不硬编码只适用于浅色主题的值，保证两套主题可读。
//
// 高亮风格（见设计文档「Navicat 截图文字化规格」）：
// - key        ：深色 / 偏紫灰，与 string value 区分。
// - string     ：亮红 / 玫红。
// - number     ：亮绿。
// - boolean/null：亮蓝。
// - punctuation：中性弱化色。
// - error      ：明亮主题浅红底 + 红色边框；暗黑主题深红底 + 亮红文字/边框。

/// JSON 编辑器主题色板。
#[derive(Clone, Copy)]
pub(crate) struct JsonEditorTheme {
    /// key 颜色（偏紫灰，深色主题下偏亮）。
    pub key: gpui::Rgba,
    /// string value 颜色（亮红 / 玫红）。
    pub string: gpui::Rgba,
    /// number 颜色（亮绿）。
    pub number: gpui::Rgba,
    /// boolean / null 颜色（亮蓝）。
    pub boolean: gpui::Rgba,
    /// 标点（`{` `}` `[` `]` `:` `,`）中性色。
    pub punctuation: gpui::Rgba,
    /// 错误背景（亮色主题浅红、暗色主题深红）。
    pub error_bg: gpui::Rgba,
    /// 错误边框。
    pub error_border: gpui::Rgba,
    /// gutter 层级线颜色。
    pub guide: gpui::Rgba,
    /// 折叠按钮 hover 背景。
    pub fold_hover: gpui::Rgba,
}

impl JsonEditorTheme {
    /// 由 `UiColors` 派生主题。`colors.is_dark` 决定采用暗色还是亮色 token 色。
    pub(crate) fn from_colors(colors: UiColors) -> Self {
        if colors.is_dark {
            JsonEditorTheme {
                // 暗色：key 用偏亮的紫灰，string 用玫红，number/bool 用高对比亮色。
                key: rgb(0xbb9aff),
                string: rgb(0xff7b9c),
                number: rgb(0x7ee787),
                boolean: rgb(0x79c0ff),
                punctuation: rgb(0x8b949e),
                error_bg: rgb(0x3d1f1f),
                error_border: rgb(0xf85149),
                guide: rgb(0x30363d),
                fold_hover: rgb(0x2d333b),
            }
        } else {
            JsonEditorTheme {
                // 亮色：key 用深紫灰，string 用玫红，number 用深绿，bool 用深蓝。
                key: rgb(0x7d5bb0),
                string: rgb(0xc7377e),
                number: rgb(0x1a7f37),
                boolean: rgb(0x0969da),
                punctuation: rgb(0x57606a),
                error_bg: rgb(0xffebe9),
                error_border: rgb(0xff8181),
                guide: rgb(0xd0d7de),
                fold_hover: rgb(0xeaeef2),
            }
        }
    }
}
