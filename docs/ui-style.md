# UI 样式规范

适用于新增和修改桌面 UI；组件复用、主题和交互要求见仓库根目录的 [AGENTS.md](../AGENTS.md)。

## 按钮
- `gpui-component::Button` 默认使用 `.rounded_md()`；明确的图标按钮或组件语义要求可例外。

## 表单输入框
- 使用 `gpui-component::Input`，放在自定义外框内；外框高度约 34px，宽度按表单语义稳定约束。
- 外框使用 `rounded_md`、`border_1`、`colors.input_bg`，默认边框为 `colors.border`，hover 使用主题适配的增强边框。
- focus 边框：明亮主题 `rgb(0x111111)`，暗黑主题 `rgb(0x8ab4ff)`；hover 不得覆盖 focus。
- 内部 Input 使用 `appearance(false)`、`focus_bordered(false)`、`w_full()`、`h_full()`，字号约 13px，保证光标垂直居中。

## 右键菜单
- 保持紧凑宽度，文字使用略重字重。
- 二级浮层贴齐主菜单右侧边缘，避免明显水平缝隙或垂直错位。
