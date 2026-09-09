use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use fluxdb_connectors::{
    MockConnector, MySqlConnector, RedisConnector, RedisStreamRange, SqliteConnector,
};
// 对外再导出建 Key 相关的连接器类型，供桌面端（fluxdb-desktop）匹配/构造 AppCommand 使用。
pub use fluxdb_connectors::{
    PubSubMessage, PubSubPollOutcome, PubSubPublishResult, PubSubSubscriptionEvent,
    RedisAddKeyKind, RedisAddKeyRequest, RedisListDirection, RedisPubSubSession,
};
use fluxdb_core::{
    BinaryCellSummary, BinaryPreviewResponse, BinaryUpdatePayload, COMPLETION_INDEX_VERSION,
    CellUpdate, CellValue, Column, ColumnRef, CommandExecutionSource, CommandExecutionSummary,
    CommandExecutionTarget, CommandResultsMode, CommandRunMode, CommandWorkbenchExecution,
    CommandWorkbenchRequest, CompletionColumn, CompletionIndexMeta, CompletionIndexSnapshot,
    CompletionRoutine, CompletionRoutineKind, CompletionTable, CompletionTrigger, ConnectionConfig,
    ConnectionDraft, ConnectionGroup, ConnectionGroupId, ConnectionId, ConnectionOverview,
    Connector, CreateDatabaseRequest, DataChangeSet, DataExportPreview, DataPage, DatabaseKind,
    DatabasePrivilegeGrant, DatabaseUserIdentity, Endpoint, Error, ErrorKind, FilterSpec,
    ForeignKeyInfo, IndexInfo, InsertTextFormat, ObjectKind, ObjectPath, ObjectSummary, Pagination,
    PrivilegeScope, QueryCompletionItem, QueryCompletionKind, QueryCompletionResult,
    QueryDeleteRollbackSnapshot, QueryExecutionOptions, QueryExecutionResult,
    QueryExecutionSummary, QueryInsertRollbackSnapshot, QueryRequest, QueryRollbackRowSnapshot,
    QueryRollbackSnapshot, QueryStatementKind, QueryUpdateRollbackSnapshot, RedisConnectionProfile,
    RedisHashFieldTtl, RedisServerVersion, Row, RowIdentity, RowUpdate, Settings, SidebarLayout,
    SortSpec, TableFingerprint, TableRef, TriggerInfo, UserFacingError, UserRoleMember,
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
include!("parts/create_table_foreign_keys.rs");
include!("parts/create_table_provider.rs");
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
include!("parts/query_history.rs");
include!("parts/workbench_history.rs");
include!("parts/query_result_edit.rs");
include!("parts/tests.rs");
