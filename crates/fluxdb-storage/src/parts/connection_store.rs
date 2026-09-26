//! `connections` 表：一条数据库连接一行，替代原来的 kv JSON blob。
//!
//! 建表口径（对数据库类型中立，新增类型不改表结构）：
//! - 只有**所有类型都成立**的字段才建列：身份（id / name / kind）、定位
//!   （endpoint_kind + host/port/file_path/uri）、默认库、只读、凭据引用；
//! - **类型专属**字段（扁平 `options` 与 redis/mysql/postgres 结构化档案）统一进
//!   `extra_json`，按 `kind` 解释；
//! - 列是 [`ConnectionConfig`] 的**投影**：落盘时由结构体派生，加载时以列为准回填，
//!   避免同一份数据出现两份真相。

use std::collections::BTreeMap;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use fluxdb_core::{
    ConnectionConfig, DatabaseKind, Endpoint, Error, ErrorKind, MysqlConnectionProfile,
    PostgresConnectionProfile, RedisConnectionProfile, Result,
};

use crate::sqlite::sqlite_error;

/// 建表语句（幂等；由 [`crate::sqlite::create_schema`] 执行）。
///
/// `kind` / `endpoint_kind` 不加 CHECK：枚举取值会随新数据库类型扩展，CHECK 会让
/// 「老库（`IF NOT EXISTS` 不重建）放行、新库拒绝」产生不一致，取值校验放到加载时的
/// 枚举还原里做。
pub const CREATE_SQL: &str = "
CREATE TABLE IF NOT EXISTS connections (
    id               INTEGER PRIMARY KEY,           -- ConnectionId(u64)，由应用分配
    name             TEXT    NOT NULL,              -- 连接名（展示用，允许重名）
    kind             TEXT    NOT NULL,              -- mysql|tidb|postgres|sqlite|redis|mongo…
    endpoint_kind    TEXT    NOT NULL,              -- tcp|file|uri
    host             TEXT,                          -- endpoint_kind=tcp
    port             INTEGER,                       -- endpoint_kind=tcp
    file_path        TEXT,                          -- endpoint_kind=file
    uri              TEXT,                          -- endpoint_kind=uri
    default_database TEXT,                          -- 默认库/逻辑库序号（各类型通用）
    read_only        INTEGER NOT NULL DEFAULT 0,    -- 只读连接
    credential_ref   TEXT,                          -- 凭据引用（账号名）；NULL=无凭据
    extra_json       TEXT    NOT NULL DEFAULT '{}'  -- 类型专属：options + 结构化档案
);
";

/// 定位枚举 `Endpoint` 的三条取值，落 `endpoint_kind` 列。
const ENDPOINT_TCP: &str = "tcp";
const ENDPOINT_FILE: &str = "file";
const ENDPOINT_URI: &str = "uri";

/// 连接类型无法识别（例如新版本写入的新类型被旧版本读到）时落盘用的兜底类型名。
pub(crate) const UNKNOWN_KIND: &str = "unknown";

/// `extra_json` 的内容：类型专属参数。
///
/// 全部字段可缺省，保证档案结构演进（增删字段）不破坏已落盘数据。
#[derive(Debug, Default, Serialize, Deserialize)]
struct ConnectionExtra {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    options: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redis_profile: Option<RedisConnectionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mysql_profile: Option<MysqlConnectionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    postgres_profile: Option<PostgresConnectionProfile>,
}

/// 数据库类型的落盘取值。
pub(crate) fn kind_to_str(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "mysql",
        DatabaseKind::TiDb => "tidb",
        DatabaseKind::Sqlite => "sqlite",
        DatabaseKind::MongoDb => "mongo",
        DatabaseKind::Redis => "redis",
        DatabaseKind::Postgres => "postgres",
    }
}

/// 从落盘取值还原数据库类型；认不出返回 `None`。
pub(crate) fn kind_from_str(value: &str) -> Option<DatabaseKind> {
    match value {
        "mysql" => Some(DatabaseKind::MySql),
        "tidb" => Some(DatabaseKind::TiDb),
        "sqlite" => Some(DatabaseKind::Sqlite),
        "mongo" => Some(DatabaseKind::MongoDb),
        "redis" => Some(DatabaseKind::Redis),
        "postgres" => Some(DatabaseKind::Postgres),
        _ => None,
    }
}

/// 全量读取连接（按 id 升序，与历史写入顺序一致）。
pub(crate) fn load_all(conn: &Connection) -> Result<Vec<ConnectionConfig>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, kind, endpoint_kind, host, port, file_path, uri,
                    default_database, read_only, credential_ref, extra_json
               FROM connections
              ORDER BY id",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(StoredRow {
                id: row.get::<_, i64>(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                endpoint_kind: row.get(3)?,
                host: row.get(4)?,
                port: row.get::<_, Option<i64>>(5)?,
                file_path: row.get(6)?,
                uri: row.get(7)?,
                default_database: row.get(8)?,
                read_only: row.get::<_, i64>(9)? != 0,
                credential_ref: row.get(10)?,
                extra_json: row.get(11)?,
            })
        })
        .map_err(sqlite_error)?;
    let mut connections = Vec::new();
    for row in rows {
        connections.push(to_config(row.map_err(sqlite_error)?)?);
    }
    Ok(connections)
}

/// 全量替换连接表（调用方负责事务；与旧的 blob 覆盖语义一致：整表替换）。
pub(crate) fn replace_all(conn: &Connection, connections: &[ConnectionConfig]) -> Result<()> {
    conn.execute("DELETE FROM connections", [])
        .map_err(sqlite_error)?;
    let mut stmt = conn
        .prepare(
            "INSERT INTO connections
                 (id, name, kind, endpoint_kind, host, port, file_path, uri,
                  default_database, read_only, credential_ref, extra_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .map_err(sqlite_error)?;
    for connection in connections {
        let store = from_config(connection)?;
        let extra_json = serde_json::to_string(&store.extra)
            .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;
        stmt.execute(params![
            store.id,
            store.name,
            store.kind,
            store.endpoint_kind,
            store.host,
            store.port,
            store.file_path,
            store.uri,
            store.default_database,
            store.read_only as i64,
            store.credential_ref,
            extra_json,
        ])
        .map_err(sqlite_error)?;
    }
    Ok(())
}

/// 从连接列表解析出「连接 id → 数据库类型」映射，供历史行标注类型。
///
/// 缺失的连接（例如同一批次里刚被删除）不在映射中，调用方按 [`KIND_UNKNOWN`] 兜底。
pub(crate) fn kind_map(conn: &Connection) -> Result<BTreeMap<u64, String>> {
    let mut stmt = conn
        .prepare("SELECT id, kind FROM connections")
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?))
        })
        .map_err(sqlite_error)?;
    let mut map = BTreeMap::new();
    for row in rows {
        let (id, kind) = row.map_err(sqlite_error)?;
        map.insert(id, kind);
    }
    Ok(map)
}

/// 落库形态：列值已按 `ConnectionConfig` 投影好。
struct StoredRow {
    id: i64,
    name: String,
    kind: String,
    endpoint_kind: String,
    host: Option<String>,
    port: Option<i64>,
    file_path: Option<String>,
    uri: Option<String>,
    default_database: Option<String>,
    read_only: bool,
    credential_ref: Option<String>,
    extra_json: String,
}

/// 建 INSERT 参数用的投影结果。
struct StoredInsert {
    id: i64,
    name: String,
    kind: &'static str,
    endpoint_kind: &'static str,
    host: Option<String>,
    port: Option<i64>,
    file_path: Option<String>,
    uri: Option<String>,
    default_database: Option<String>,
    read_only: bool,
    credential_ref: Option<String>,
    extra: ConnectionExtra,
}

/// `ConnectionConfig` → 列投影。
fn from_config(connection: &ConnectionConfig) -> Result<StoredInsert> {
    let (endpoint_kind, host, port, file_path, uri, default_database) = match &connection.endpoint {
        Endpoint::Tcp {
            host,
            port,
            database,
        } => (
            ENDPOINT_TCP,
            Some(host.clone()),
            Some(i64::from(*port)),
            None,
            None,
            database.clone(),
        ),
        Endpoint::SqliteFile { path, .. } => {
            // serde 对 PathBuf 同样要求合法 UTF-8（非 UTF-8 直接报错），此处保持一致：
            // 宁可明确失败，也不做 lossy 转换把路径悄悄改掉。
            let path = path.to_str().ok_or_else(|| {
                Error::new(
                    ErrorKind::Internal,
                    "连接文件路径包含非 UTF-8 字符，无法落盘",
                )
            })?;
            (
                ENDPOINT_FILE,
                None,
                None,
                Some(path.to_string()),
                None,
                None,
            )
        }
        Endpoint::Uri { uri } => (ENDPOINT_URI, None, None, None, Some(uri.clone()), None),
    };
    let read_only = matches!(
        connection.endpoint,
        Endpoint::SqliteFile {
            read_only: true,
            ..
        }
    );
    Ok(StoredInsert {
        id: connection.id.0 as i64,
        name: connection.name.clone(),
        kind: kind_to_str(connection.kind),
        endpoint_kind,
        host,
        port,
        file_path,
        uri,
        default_database,
        read_only,
        credential_ref: connection.credential_ref.clone(),
        extra: ConnectionExtra {
            options: connection.options.clone(),
            redis_profile: connection.redis_profile.clone(),
            mysql_profile: connection.mysql_profile.clone(),
            postgres_profile: connection.postgres_profile.clone(),
        },
    })
}

/// 列 → `ConnectionConfig`。
fn to_config(row: StoredRow) -> Result<ConnectionConfig> {
    let kind = kind_from_str(&row.kind).ok_or_else(|| {
        Error::new(
            ErrorKind::Internal,
            format!("连接 {} 的数据库类型无法识别: {}", row.id, row.kind),
        )
    })?;
    // port 列可能与 host 不匹配（仅可能来自人工改库），按 TCP 定位的最低要求校验。
    let endpoint = match row.endpoint_kind.as_str() {
        ENDPOINT_TCP => {
            let host = row.host.ok_or_else(|| {
                Error::new(ErrorKind::Internal, format!("连接 {} 缺少主机地址", row.id))
            })?;
            let port = row.port.ok_or_else(|| {
                Error::new(ErrorKind::Internal, format!("连接 {} 缺少端口", row.id))
            })?;
            let port = u16::try_from(port).map_err(|_| {
                Error::new(
                    ErrorKind::Internal,
                    format!("连接 {} 的端口超出范围: {port}", row.id),
                )
            })?;
            Endpoint::Tcp {
                host,
                port,
                database: row.default_database,
            }
        }
        ENDPOINT_FILE => {
            let path = row.file_path.ok_or_else(|| {
                Error::new(ErrorKind::Internal, format!("连接 {} 缺少文件路径", row.id))
            })?;
            Endpoint::SqliteFile {
                path: path.into(),
                read_only: row.read_only,
            }
        }
        ENDPOINT_URI => {
            let uri = row.uri.ok_or_else(|| {
                Error::new(ErrorKind::Internal, format!("连接 {} 缺少连接串", row.id))
            })?;
            Endpoint::Uri { uri }
        }
        other => {
            return Err(Error::new(
                ErrorKind::Internal,
                format!("连接 {} 的定位类型无法识别: {other}", row.id),
            ));
        }
    };
    let extra: ConnectionExtra = serde_json::from_str(&row.extra_json).map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("连接 {} 的类型专属参数无法解析: {error}", row.id),
        )
    })?;
    Ok(ConnectionConfig {
        id: fluxdb_core::ConnectionId(row.id as u64),
        name: row.name,
        kind,
        endpoint,
        credential_ref: row.credential_ref,
        options: extra.options,
        redis_profile: extra.redis_profile,
        mysql_profile: extra.mysql_profile,
        postgres_profile: extra.postgres_profile,
    })
}
