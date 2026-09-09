use fluxdb_core::{
    BinaryCellSummary, CellValue, Column, CommandBulk, CommandExecutionItem,
    CommandExecutionStatus, CommandExecutionSummary, CommandExecutionTarget, CommandReply,
    CommandWorkbenchExecution, CommandWorkbenchRequest, CompletionColumn, CompletionRoutine,
    CompletionRoutineKind, CompletionTable, CompletionTrigger, ConnectionConfig, ConnectionId,
    ConnectionOverview, Connector, CreateDatabaseRequest, DataChangeSet, DataExportPreview,
    DataPage, DatabaseKind, Endpoint, Error, ErrorKind, FilterOp, FilterSpec, ForeignKeyInfo,
    IndexInfo, ObjectKind, ObjectPath, ObjectSummary, Pagination, QueryExecutionResult,
    QueryExecutionSummary, QueryRequest, QueryStatementKind, RedisHashFieldTtl, RedisServerVersion,
    Row, RowUpdate, SortDirection, SortSpec, TriggerInfo, is_binary_type_name,
    sqlite_attached_databases,
};
use sqlx::{
    Column as SqlxColumn, ColumnIndex, ConnectOptions as _, Connection as _, MySql, QueryBuilder,
    Row as SqlxRow, Sqlite, TypeInfo as _,
    mysql::{MySqlConnectOptions, MySqlRow, types::MySqlTime},
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
include!("parts/mysql.rs");
include!("parts/redis.rs");
include!("parts/sqlite.rs");
include!("parts/shared.rs");
include!("parts/tests.rs");
