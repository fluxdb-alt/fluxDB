// Included in crate-root scope by ../main.rs; Redis 详情 UI 按数据面板与控制器拆分。
// 与 navicat_main/ 一致：多个 `impl NavicatMain {}` 块分布在各 controller_*.rs，靠 include! 汇入根作用域。

include!("redis_detail/types.rs");
include!("redis_detail/common.rs");
// Redis Key 前缀折叠树：纯函数「建树 + 拍平」能力，供 Folder 模式复用。
include!("redis_detail/key_tree.rs");
include!("redis_detail/key_list.rs");
include!("redis_detail/key_meta.rs");
include!("redis_detail/string_panel.rs");
include!("redis_detail/set_panel.rs");
include!("redis_detail/hash_table.rs");
include!("redis_detail/hash_panel.rs");
include!("redis_detail/zset_panel.rs");
include!("redis_detail/list_table.rs");
include!("redis_detail/list_panel.rs");
include!("redis_detail/stream_table.rs");
include!("redis_detail/stream_panel.rs");
include!("redis_detail/controller_common.rs");
include!("redis_detail/controller_string.rs");
include!("redis_detail/controller_set.rs");
include!("redis_detail/controller_hash.rs");
include!("redis_detail/controller_zset.rs");
include!("redis_detail/controller_list.rs");
include!("redis_detail/controller_stream.rs");
include!("redis_detail/controller_add_key.rs");
// 「新增 Key」子表单组件（对齐 RedisInsight AddKey）：公共字段 + 各类型独立子表单。
include!("redis_detail/add_key_form/shared.rs");
include!("redis_detail/add_key_form/rows_panel.rs");
include!("redis_detail/add_key_form/common_fields.rs");
include!("redis_detail/add_key_form/string_form.rs");
include!("redis_detail/add_key_form/json_form.rs");
include!("redis_detail/add_key_form/hash_form.rs");
include!("redis_detail/add_key_form/zset_form.rs");
include!("redis_detail/add_key_form/set_form.rs");
include!("redis_detail/add_key_form/list_form.rs");
include!("redis_detail/add_key_form/stream_form.rs");
// 「新增 Key」抽屉壳：三段式布局（标题 / 可滚动表单 / 固定 footer）。
include!("redis_detail/add_key_panel.rs");
