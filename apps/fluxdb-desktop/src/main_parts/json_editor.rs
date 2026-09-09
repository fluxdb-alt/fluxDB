// Redis JSON 值编辑器：可复用 JSON 编辑组件（格式化 / 语法高亮 / 错误定位 / 折叠）。
// 按仓库 `include!` 风格拆分为职责文件，全部汇入 crate-root 作用域，便于后续稳定后再迁移为真实 `mod` 边界。
//
// 组件定位：不依赖任何数据库连接，纯 UI / 文本逻辑，可被 Redis JSON 值详情、Hash 大字段、
// 查询参数 JSON 编辑等位置复用。
include!("json_editor/types.rs");
include!("json_editor/parser.rs");
include!("json_editor/folding.rs");
include!("json_editor/theme.rs");
include!("json_editor/controller.rs");
include!("json_editor/render.rs");
