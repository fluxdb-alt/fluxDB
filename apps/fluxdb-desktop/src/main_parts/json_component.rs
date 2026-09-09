// json_component —— 独立、可复用的 JSON 组件（支持 Preview / Edit 两种模式）。
//
// 与 `json_editor`（作为 `NavicatMain` 一部分工作）不同，本组件是一个自包含单元：
// - 只依赖 `json_editor` 中已抽出的纯逻辑（validate / pretty / folding / 诊断）。
// - `Preview`：只读展示（pretty / 高亮 / 折叠 / 行号），不创建 `InputState`。
// - `Edit`：可编辑全文，`InputState` 由组件内部创建并管理，不暴露给调用方。
// - 调用方只传入 `value`、模式、回调和展示参数。
//
// 同样按仓库 `include!` 风格拆分为职责文件（真实模块迁移待后续稳定后再做）。
include!("json_component/mode.rs");
include!("json_component/component.rs");
include!("json_component/render.rs");
