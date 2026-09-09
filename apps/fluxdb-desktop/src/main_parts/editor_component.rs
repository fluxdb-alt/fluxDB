// 独立编辑器内核（fluxdb-editor-core）的 GPUI 通用前端。
// 该模块只依赖 fluxdb-editor-core，不依赖 fluxdb-app / connector / 数据库驱动 / NavicatMain。
// 通过 include! 拆分到子文件，但包在 pub(crate) mod 内避免与旧的 sql_editor 动作/常量在
// crate 根命名空间冲突（Task 10 删除旧 SQL 编辑器后可再扁平化）。
pub(crate) mod editor_component {
    include!("editor_component/mod.rs");
    include!("editor_component/input.rs");
    include!("editor_component/completion_ui.rs");
    include!("editor_component/completion_popup.rs");
    include!("editor_component/layout.rs");
    include!("editor_component/render.rs");
}
