// Oracle 连接器按职责拆分；文件仍经 `include!` 汇入 crate 根作用域。
//
// 边界：schema 即对象 Owner，对象引用只生成两段名 `"OWNER"."TABLE"`；元数据优先走
// `ALL_*` 视图，不依赖 `DBA_*` / `V$*`；所有值走 `:N` 绑定，不做字符串插值。

include!("oracle/session.rs");
include!("oracle/value.rs");
include!("oracle/objects.rs");
include!("oracle/metadata.rs");
include!("oracle/data.rs");
include!("oracle/apply_changes.rs");
include!("oracle/executor.rs");
include!("oracle/user_admin.rs");
include!("oracle/activity.rs");
include!("oracle/connector.rs");
