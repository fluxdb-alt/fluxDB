use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use fluxdb_connectors::{PostgresConnector, SqliteConnector};
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
    AppliedChangeOutcome, BinaryCellSummary, BinaryPreviewResponse, BinaryUpdatePayload,
    COMPLETION_INDEX_VERSION, CellUpdate, CellValue, Column, ColumnRef, CommandExecutionSource,
    CommandExecutionSummary, CommandExecutionTarget, CommandResultsMode, CommandRunMode,
    CommandWorkbenchExecution, CommandWorkbenchRequest, CompletionColumn, CompletionIndexMeta,
    CompletionIndexSnapshot, CompletionRoutine, CompletionRoutineKind, CompletionTable,
    CompletionTrigger, ConnectionConfig, ConnectionDraft, ConnectionGroup, ConnectionGroupId,
    ConnectionId, ConnectionOverview, Connector, CreateDatabaseRequest, DataChangeSet,
    DataExportPreview, DataPage, DatabaseKind, DatabasePrivilegeGrant, DatabaseUserIdentity,
    Endpoint, ErColumn, ErForeignKeyEdge, ErGraphData, ErLoadStatus, ErRelationship, ErTableNode,
    ErTableRef, Error, ErrorKind, FilterSpec, ForeignKeyInfo, IndexInfo, InsertTextFormat,
    ObjectKind, ObjectPath, ObjectSummary, Pagination, PgEffectivePrivilege, PgGrantTargetLists,
    PgObjectGrantScope, PgObjectGrants, PgPasswordOp, PgRelationKind, PgRole, PgRoleChange,
    PgRoleDraft, PgRoleMembership, PgRoleSavePlan, PgValidUntilOp, PrivilegeScope,
    QueryCompletionItem, QueryCompletionKind, QueryCompletionResult, QueryDeleteRollbackSnapshot,
    QueryExecutionOptions, QueryExecutionResult, QueryExecutionSummary,
    QueryInsertRollbackSnapshot, QueryRequest, QueryRollbackRowSnapshot, QueryRollbackSnapshot,
    QueryStatementKind, QueryUpdateRollbackSnapshot, RedisConnectionProfile, RedisHashFieldTtl,
    RedisServerVersion, RoutineRef, Row, RowIdentity, RowUpdate, Settings, SidebarLayout, SortSpec,
    TableFingerprint, TableRef, TriggerInfo, TriggerRef, UserFacingError, UserRoleMember,
    UserRoleMembership, WorkbenchHistoryItem, WorkbenchHistoryScope, WorkbenchHistoryStore,
    database_user_admin_provider, grants_from_query_result, infer_cloud_from_host,
    redact_uri_password, role_memberships_from_grants, set_sqlite_attached_database,
    sqlite_attached_database_path,
};
use sqlformat::{Dialect, FormatOptions, Indent, QueryParams};
use sqlparser::{
    ast::{
        Expr, FromTable, ObjectName, ObjectNamePart, Query as SqlAstQuery, SetExpr, Statement,
        TableAlias, TableFactor, TableObject, TableWithJoins, UpdateTableFromKind,
        visit_expressions, visit_relations,
    },
    dialect::MySqlDialect,
    parser::Parser,
};

// First-pass source split: included files remain in crate-root scope while module boundaries are refined.
include!("parts/state.rs");
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
include!("parts/controller.rs");
include!("parts/data_editor.rs");
include!("parts/mock_data.rs");
include!("parts/sql_format.rs");
include!("parts/table_info.rs");
include!("parts/user_admin.rs");
include!("parts/completion_index.rs");
include!("parts/query_completion.rs");
include!("parts/redis_commands.rs");
include!("parts/redis_completion.rs");
include!("parts/terminal_redis.rs");
include!("parts/native_client_io.rs");
include!("parts/pg_client_tools.rs");
#[path = "parts/mysql_client_tools.rs"]
mod mysql_client_tools;
pub use mysql_client_tools::*;
include!("parts/query_history.rs");
include!("parts/workbench_history.rs");
include!("parts/query_result_edit.rs");
include!("parts/tests.rs");
include!("parts/er_service.rs");
include!("parts/er_catalog.rs");
include!("parts/er_layout.rs");
include!("parts/er_model_service.rs");

pub use fluxdb_connectors::{
    MySqlClientVersion, MySqlDumpInvocation, MySqlDumpOptions, mysql_client_version,
    mysql_dump_invocation,
};

include!("parts/backup_restore.rs");
