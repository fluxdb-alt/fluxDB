// Included in crate-root scope by ../lib.rs; AppController behavior is split by responsibility.

include!("controller/api.rs");
include!("controller/ai_settings.rs");
include!("controller/ai_settings_dispatch.rs");
include!("controller/dispatch.rs");
include!("controller/dispatch_connection.rs");
include!("controller/dispatch_tabs.rs");
include!("controller/dispatch_user_admin.rs");
include!("controller/dispatch_server_panel.rs");
include!("controller/dispatch_redis_server_panel.rs");
include!("controller/dispatch_mongo_server_panel.rs");
include!("controller/dispatch_pg_user_admin.rs");
include!("controller/dispatch_sqlserver_user_admin.rs");
include!("controller/dispatch_oracle_user_admin.rs");
include!("controller/query_execution.rs");
include!("controller/dispatch_table.rs");
include!("controller/dispatch_mongo.rs");
include!("controller/dispatch_mongo_workbench.rs");
include!("controller/tabs.rs");
include!("controller/data_loading.rs");
include!("controller/cell_content.rs");
include!("controller/ai_table_page.rs");
include!("controller/query_completion.rs");
#[cfg(test)]
include!("controller/dispatch_tests.rs");
