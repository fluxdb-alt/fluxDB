use fluxdb_core::{
    AppliedChangeOutcome, BinaryCellSummary, CellValue, CheckMeta, Column, ColumnMeta, CommandBulk,
    CommandExecutionItem, CommandExecutionStatus, CommandExecutionSummary, CommandExecutionTarget,
    CommandReply, CommandWorkbenchExecution, CommandWorkbenchRequest, CompletionColumn,
    CompletionRoutine, CompletionRoutineKind, CompletionTable, CompletionTrigger, ConnectionConfig,
    ConnectionId, ConnectionOverview, Connector, CreateDatabaseRequest, DataChangeSet,
    DataExportPreview, DataPage, DatabaseKind, Endpoint, Error, ErrorKind, FilterOp, FilterSpec,
    ForeignKeyInfo, ForeignKeyMeta, IndexColumnItem, IndexInfo, IndexMeta, MongoCollectionInfo,
    MongoCollectionKind, MongoDatabaseStat, MongoDocument, MongoDocumentsPage, MongoFindRequest,
    MongoImportField, MongoImportFormat, MongoImportOptions, MongoImportPreview,
    MongoImportPreviewRequest, MongoImportProgress, MongoImportReport, MongoImportRequest,
    MongoIndexField, MongoIndexInfo, MongoOperation, MongoServerActivity, MongoServerInfo,
    MongoTopology, ObjectKind, ObjectPath, ObjectSummary, Pagination, QueryExecutionResult,
    QueryExecutionSummary, QueryRequest, QuerySessionId, QueryStatementKind, RedisClient,
    RedisHashFieldTtl, RedisServerActivity, RedisServerVersion, RedisSlowLogEntry, Row,
    RowIdentity, RowUpdate, ServerActivity, ServerLockWait, ServerMaintenanceAction,
    ServerMaintenanceRequest, ServerMetric, ServerSession, ServerStatus, SortDirection, SortSpec,
    SqlDialect as _, TableStructure, TriggerInfo, TriggerMeta, UniqueKeyMeta, WriteValue,
    completion_fuzzy_like_pattern, is_binary_type_name, matches_completion_fuzzy,
    sqlite_attached_databases,
};
// MongoDB 驱动类型（crate-root include 作用域共用，避免每文件重复 use）。
use mongodb::Client as MongoClient;
use mongodb::bson::{Bson, Document, doc};
use mongodb::options::{Collation, Hint, IndexOptions, ReadPreference, SelectionCriteria};
use mongodb::results::CollectionType;
use sqlx::{
    Column as SqlxColumn, ColumnIndex, ConnectOptions as _, Connection as _, MySql, QueryBuilder,
    Row as SqlxRow, Sqlite, TypeInfo as _,
    mysql::{MySqlConnectOptions, MySqlConnection, MySqlRow, types::MySqlTime},
    sqlite::{SqliteConnectOptions, SqliteRow},
    types::{
        BigDecimal,
        chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc},
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

// First-pass source split: included files remain in crate-root scope while connector modules are refined.
include!("parts/common.rs");
pub mod filter_dialects;
pub mod script_syntax;
include!("parts/mock.rs");
include!("parts/transport.rs");
include!("parts/mysql.rs");
include!("parts/postgres.rs");
include!("parts/dameng.rs");
include!("parts/sqlserver.rs");
include!("parts/oracle.rs");
include!("parts/redis.rs");
include!("parts/mongo.rs");
include!("parts/sqlite.rs");
include!("parts/sqlite_bounded_update.rs");
include!("parts/factory.rs");
include!("parts/shared_cells.rs");
include!("parts/shared_write.rs");
include!("parts/shared_read_exec.rs");
include!("parts/shared_read_sql.rs");
include!("parts/shared_demo.rs");
include!("parts/table_import.rs");
include!("parts/table_transfer.rs");

#[path = "parts/mysql/native_tools.rs"]
mod mysql_native_tools;
pub use mysql_native_tools::*;

include!("parts/backup_restore.rs");

/// 注册进程级筛选方言实现。由 app 初始化调用；重复调用安全。
pub fn register_filter_dialects() {
    fluxdb_core::register_filter_dialect_resolver(filter_dialects::filter_dialect);
}
