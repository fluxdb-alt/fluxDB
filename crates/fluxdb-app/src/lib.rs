use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use fluxdb_connectors::{RedisConnector, RedisStreamRange, connector_for};
// 对外再导出建 Key 相关的连接器类型，供桌面端（fluxdb-desktop）匹配/构造 AppCommand 使用。
pub use fluxdb_connectors::{
    NativeTlsPaths, PgDumpInvocation, PgDumpScope, PgNativeSshTunnel, PgPsqlInvocation,
    PubSubMessage, PubSubPollOutcome, PubSubPublishResult, PubSubSubscriptionEvent,
    RedisAddKeyKind, RedisAddKeyRequest, RedisListDirection, RedisPubSubSession, SshTunnelAuth,
    SshTunnelInvocation, pg_dump_invocation, pg_dump_version_compatible, pg_hostaddr_env,
    pg_native_tls_paths, pg_open_native_ssh_tunnel, pg_psql_invocation,
    pg_script_needs_native_mode, pg_server_major_version, pg_ssh_tunnel_invocation,
    pg_sslmode_value, pg_tool_major_version,
};
use fluxdb_core::{
    AppliedChangeOutcome, BinaryCellSummary, COMPLETION_INDEX_VERSION, CellUpdate, CellUpdateValue,
    CellValue, Column, ColumnRef, CommandExecutionItem, CommandExecutionSource,
    CommandExecutionStatus, CommandExecutionSummary, CommandExecutionTarget, CommandReply,
    CommandResultsMode, CommandRunMode, CommandWorkbenchExecution, CommandWorkbenchRequest,
    CompletionColumn, CompletionIndexMeta, CompletionIndexSnapshot, CompletionRoutine,
    CompletionRoutineKind, CompletionTable, CompletionTrigger, ConnectionConfig, ConnectionDraft,
    ConnectionGroup, ConnectionGroupId, ConnectionId, ConnectionOverview, Connector,
    CreateDatabaseRequest, DataChangeSet, DataExportPreview, DataPage, DatabaseKind,
    DatabasePrivilegeGrant, DatabaseUserIdentity, Endpoint, ErColumn, ErForeignKeyEdge,
    ErGraphData, ErLoadStatus, ErRelationship, ErTableNode, ErTableRef, Error, ErrorKind,
    ExecutionPolicy, ExecutionPolicySettings, FilterExpression, FilterSpec, ForeignKeyInfo,
    IndexInfo, InsertTextFormat, ModelRisk, MongoCollectionInfo, MongoDocumentsPage,
    MongoImportOptions, MongoImportPreview, MongoImportProgress, MongoImportReport, MongoIndexInfo,
    MongoServerInfo, MongoShellResult, ObjectKind, ObjectPath, ObjectSummary, Pagination,
    PgEffectivePrivilege, PgGrantTargetLists, PgObjectGrantScope, PgObjectGrants, PgPasswordOp,
    PgRelationKind, PgRole, PgRoleChange, PgRoleDraft, PgRoleMembership, PgRoleSavePlan,
    PgValidUntilOp, PrivilegeScope, QueryCompletionItem, QueryCompletionKind,
    QueryCompletionResult, QueryDeleteRollbackSnapshot, QueryExecutionOptions,
    QueryExecutionResult, QueryExecutionSummary, QueryInsertRollbackSnapshot, QueryRequest,
    QueryRollbackRowSnapshot, QueryRollbackSnapshot, QueryStatementKind,
    QueryUpdateRollbackSnapshot, RedisConnectionProfile, RedisHashFieldTtl, RedisServerVersion,
    RoutineRef, Row, RowIdentity, RowUpdate, Settings, SidebarLayout, SortSpec, SqlDialect as _,
    TableFingerprint, TableRef, TriggerInfo, TriggerRef, UploadPayload, UserAdminSqlDialect,
    UserFacingError, UserRoleMember, UserRoleMembership, WorkbenchHistoryItem,
    WorkbenchHistoryScope, WorkbenchHistoryStore, WriteCommit, WriteReviewOutcome,
    WriteRiskDecision, WriteRiskFacts, database_user_admin_provider, grants_from_query_result,
    role_memberships_from_grants, set_sqlite_attached_database, sqlite_attached_database_path,
};
pub use fluxdb_core::{
    TableDataImport, TableImportFormat, TableImportProgress, TableImportReport, TableImportRequest,
};
use sqlformat::{Dialect, FormatOptions, Indent, QueryParams};
use sqlparser::ast::{Expr, ObjectName, ObjectNamePart, Statement};

// First-pass source split: included files remain in crate-root scope while module boundaries are refined.
include!("parts/ai_conversation_state.rs");
include!("parts/sqlserver_user_admin_state.rs");
include!("parts/mongo_collection_state.rs");
include!("parts/mongo_fields.rs");
include!("parts/oracle_user_admin_state.rs");
include!("parts/state.rs");
include!("parts/ai_model_catalog.rs");
include!("parts/create_table_model.rs");
include!("parts/create_table_state.rs");
include!("parts/create_table_metadata.rs");
include!("parts/create_table_sql.rs");
include!("parts/create_table_actions.rs");
include!("parts/table_actions_postgres.rs");
include!("parts/create_table_design_statements.rs");
include!("parts/create_table_foreign_keys.rs");
include!("parts/create_table_provider.rs");
include!("parts/create_table_postgres.rs");
include!("parts/create_table_postgres_design.rs");
include!("parts/create_table_dameng.rs");
include!("parts/create_table_oracle.rs");
include!("parts/controller.rs");
include!("parts/data_editor.rs");
include!("parts/cell_content_store.rs");
include!("parts/cell_content_view.rs");
include!("parts/read_only_guard.rs");
include!("parts/mock_data.rs");
include!("parts/structure_tools.rs");
include!("parts/sql_format.rs");
include!("parts/table_info.rs");
include!("parts/user_admin.rs");
include!("parts/sqlserver_user_admin.rs");
include!("parts/oracle_user_admin.rs");
include!("parts/completion_index.rs");
include!("parts/query_completion.rs");
include!("parts/completion_engine.rs");
include!("parts/redis_commands.rs");
include!("parts/redis_completion.rs");
include!("parts/terminal_redis.rs");
include!("parts/terminal_mongo.rs");
include!("parts/native_client_io.rs");
include!("parts/updater.rs");
include!("parts/pg_client_tools.rs");
#[path = "parts/mysql_client_tools.rs"]
mod mysql_client_tools;
pub use mysql_client_tools::*;
include!("parts/query_history.rs");
include!("parts/workbench_history.rs");
include!("parts/query_result_edit.rs");
include!("parts/er_service.rs");
include!("parts/er_catalog.rs");
include!("parts/er_layout.rs");
include!("parts/er_model_service.rs");
include!("parts/er_relationship_source.rs");

pub use fluxdb_connectors::{
    MySqlClientVersion, MySqlDumpInvocation, MySqlDumpOptions, mysql_client_version,
    mysql_dump_invocation,
};
// 编辑器语句切分与执行侧共用同一套方言规则：`;` / 独占一行的 `/` / `GO`、
// `q'…'` 与块深度都由 connectors 的词法层判定，UI 只消费字节区间。
// 名字带 `byte_ranges`：本 crate 内 `sql_statement_ranges` 已是另一个只看分号的旧实现。
pub use fluxdb_connectors::script_syntax::statement_ranges as sql_statement_byte_ranges;

include!("parts/ai_transport.rs");
include!("parts/ai_retry.rs");
include!("parts/ai_executor.rs");
include!("parts/ai_runtime.rs");
include!("parts/ai_provider.rs");
include!("parts/ai_loop.rs");
include!("parts/ai_compaction.rs");
include!("parts/ai_routing.rs");
include!("parts/ai_decision_provider.rs");
include!("parts/ai_jev.rs");
include!("parts/ai_usage.rs");
include!("parts/ai_tool_registry.rs");
include!("parts/ai_toolbox.rs");
include!("parts/ai_tool_management.rs");
include!("parts/ai_tool_output.rs");
include!("parts/ai_plugins.rs");
include!("parts/ai_plugin_config.rs");
include!("parts/ai_plan.rs");
include!("parts/ai_diagnostics.rs");
include!("parts/ai_artifact_renderer.rs");
include!("parts/ai_bi_artifacts.rs");
include!("parts/ai_artifact_snapshot.rs");
include!("parts/ai_artifact_tools.rs");
include!("parts/ai_artifact_report.rs");
include!("parts/ai_report_html.rs");
include!("parts/ai_results.rs");
include!("parts/ai_result_retention.rs");
include!("parts/ai_result_pages.rs");
include!("parts/ai_hooks.rs");
include!("parts/ai_audit_rules.rs");
include!("parts/ai_permission.rs");
include!("parts/ai_write_risk_sql.rs");
include!("parts/ai_write_risk_redis.rs");
include!("parts/ai_write_review.rs");
include!("parts/ai_execution_policy.rs");
include!("parts/ai_context.rs");
include!("parts/ai_page_context.rs");
include!("parts/settings_surface.rs");
include!("parts/ai_ui_actions.rs");
include!("parts/ai_config_tools.rs");
include!("parts/config_proposal_apply.rs");
include!("parts/ai_content_search.rs");
include!("parts/ai_editor_read.rs");
include!("parts/ai_sql_editor_edit.rs");
include!("parts/ai_table_page.rs");
include!("parts/sql_files.rs");
include!("parts/ai_references.rs");
include!("parts/ai_file_tools.rs");
include!("parts/manual_docs.rs");
include!("parts/ai_docs_tools.rs");
include!("parts/ai_file_content.rs");
include!("parts/ai_reference_details.rs");
include!("parts/ai_database_context.rs");
include!("parts/ai_connection_tools.rs");
#[path = "parts/ai_sql_policy/mod.rs"]
mod ai_sql_policy;
pub use ai_sql_policy::*;
include!("parts/ai_sql_exec.rs");
include!("parts/ai_sql_receipt.rs");
include!("parts/ai_sql_tools.rs");
include!("parts/ai_relationship_query.rs");
include!("parts/ai_relationship_tools.rs");
#[cfg(test)]
include!("parts/ai_relationship_test_support.rs");
include!("parts/ai_redis_policy.rs");
include!("parts/ai_redis_keys.rs");
include!("parts/ai_redis_read.rs");
include!("parts/ai_redis_exec.rs");
include!("parts/ai_mcp_server.rs");
include!("parts/ai_mcp_install.rs");
include!("parts/ai_mcp_clients.rs");
include!("parts/ai_scripted.rs");
include!("parts/backup_restore.rs");

include!("parts/ai_conversation.rs");
include!("parts/ai_conversation_provider.rs");
include!("parts/ai_conversation_records.rs");
include!("parts/ai_recovery.rs");
include!("parts/ai_session_title.rs");
