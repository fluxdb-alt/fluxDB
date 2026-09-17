use fluxdb_core::{
    AppliedChangeOutcome, BinaryCellSummary, CellValue, CheckMeta, Column, ColumnMeta, CommandBulk,
    CommandExecutionItem, CommandExecutionStatus, CommandExecutionSummary, CommandExecutionTarget,
    CommandReply, CommandWorkbenchExecution, CommandWorkbenchRequest, CompletionColumn,
    CompletionRoutine, CompletionRoutineKind, CompletionTable, CompletionTrigger, ConnectionConfig,
    ConnectionId, ConnectionOverview, Connector, CreateDatabaseRequest, DataChangeSet,
    DataExportPreview, DataPage, DatabaseKind, Endpoint, Error, ErrorKind, FilterOp, FilterSpec,
    ForeignKeyInfo, ForeignKeyMeta, IndexColumnItem, IndexInfo, IndexMeta, ObjectKind, ObjectPath,
    ObjectSummary, Pagination, QueryExecutionResult, QueryExecutionSummary, QueryRequest,
    QuerySessionId, QueryStatementKind, RedisHashFieldTtl, RedisServerVersion, Row, RowIdentity,
    RowUpdate, SortDirection, SortSpec, TableStructure, TriggerInfo, TriggerMeta, UniqueKeyMeta,
    WriteValue, is_binary_type_name, sqlite_attached_databases,
};
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
include!("parts/mock.rs");
include!("parts/transport.rs");
include!("parts/mysql.rs");
include!("parts/postgres.rs");
include!("parts/redis.rs");
include!("parts/sqlite.rs");
include!("parts/factory.rs");
include!("parts/shared_cells.rs");
include!("parts/shared_write.rs");
include!("parts/shared_read_exec.rs");
include!("parts/shared_read_sql.rs");
include!("parts/shared_demo.rs");
include!("parts/tests.rs");

#[path = "parts/mysql/native_tools.rs"]
mod mysql_native_tools;
pub use mysql_native_tools::*;

include!("parts/backup_restore.rs");
