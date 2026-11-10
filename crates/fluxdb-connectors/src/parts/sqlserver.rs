// SQL Server 连接器按职责拆分；文件仍经 `include!` 汇入 crate 根作用域。
//
// 边界：只使用 SQL 登录；命名实例首版要求显式端口。所有系统目录查询按
// database + schema 两层身份执行，避免跨库/跨 schema 同名对象串用。

include!("sqlserver/session.rs");
include!("sqlserver/sql.rs");
include!("sqlserver/value.rs");
include!("sqlserver/objects.rs");
include!("sqlserver/metadata.rs");
include!("sqlserver/data.rs");
include!("sqlserver/database.rs");
include!("sqlserver/user_admin.rs");
include!("sqlserver/apply_changes.rs");
include!("sqlserver/executor.rs");
include!("sqlserver/activity.rs");
include!("sqlserver/connector.rs");
