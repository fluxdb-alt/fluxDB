use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;

mod sqlite;

use fluxdb_core::{
    ColumnRef, CompletionIndexMeta, CompletionIndexSnapshot, ConnectionConfig, ConnectionId, Error,
    ErrorKind, MysqlConnectionProfile, MysqlTransportLayer, PostgresConnectionProfile,
    PostgresTransportLayer, QueryRollbackSnapshot, RedisConnectionProfile, Result, RoutineRef,
    SavedQuery, SecretRef, Settings, SidebarLayout, TableRef, TriggerRef,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const PLAINTEXT_PASSWORD_OPTION: &str = "password";
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "com.fluxdb.connection";

pub trait Storage {
    fn load_settings(&self) -> Result<Settings>;
    fn save_settings(&self, settings: &Settings) -> Result<()>;
    fn load_connections(&self) -> Result<Vec<ConnectionConfig>>;
    fn save_connections(&self, connections: &[ConnectionConfig]) -> Result<()>;
    /// 删除某连接所拥有的全部 Keychain 条目（按 credential_ref 拥有权，保护其他连接）。
    fn delete_connection_secrets(&self, connection: &ConnectionConfig);
    fn load_sidebar_layout(&self, connections: &[ConnectionConfig]) -> Result<SidebarLayout>;
    fn save_sidebar_layout(
        &self,
        connections: &[ConnectionConfig],
        layout: &SidebarLayout,
    ) -> Result<()>;

    fn import_connections_if_empty(
        &self,
        defaults: &[ConnectionConfig],
    ) -> Result<Vec<ConnectionConfig>> {
        let connections = self.load_connections()?;
        if connections.is_empty() {
            self.save_connections(defaults)?;
            Ok(defaults.to_vec())
        } else {
            Ok(connections)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStorage {
    root: PathBuf,
}

impl FileStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 解析默认持久化根目录（目录策略统一在此定义，见方案 §4.1）：
    /// - macOS：`~/Library/Application Support/fluxdb`（与历史版本一致，旧数据不受影响）
    /// - Windows：Known Folder LocalAppData 下的 `FluxDB`（即 `%LOCALAPPDATA%/FluxDB`）。
    ///   注意 dirs::data_dir() 在 Windows 是 Roaming，必须用 data_local_dir()。
    /// - Linux：`$XDG_DATA_HOME/fluxdb`，缺省 `~/.local/share/fluxdb`
    ///
    /// 解析失败（系统目录不存在等极端环境）返回 Err，由调用方给出可见错误；
    /// 不再静默回退到当前目录/安装目录。
    pub fn default_root() -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        let base = dirs::data_dir();
        #[cfg(target_os = "windows")]
        let base = dirs::data_local_dir();
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let base = dirs::data_dir();

        let base =
            base.ok_or_else(|| Error::new(ErrorKind::Internal, "无法解析系统应用数据目录"))?;
        // Windows 产品目录名用大写 FluxDB（计划 §4.1）；macOS/Linux 保持小写以兼容旧目录。
        let app_dir = if cfg!(target_os = "windows") {
            "FluxDB"
        } else {
            "fluxdb"
        };
        Ok(base.join(app_dir))
    }

    /// `default_root` 的显式失败版本，供启动路径使用：
    /// 目录解析失败时由调用方输出可见错误并退出，而不是静默用错误目录继续运行。
    pub fn try_default() -> Result<Self> {
        Ok(Self::new(Self::default_root()?))
    }

    /// 默认日志目录（方案 §4.1）：
    /// - Linux：`$XDG_STATE_HOME/fluxdb/logs`，缺省 `~/.local/state/fluxdb/logs`
    /// - macOS/Windows：持久化根目录下 `logs/`（与历史布局一致）
    ///
    /// 日志目录解析失败不阻断启动，回退系统临时目录（调用方应记录该降级）。
    pub fn default_log_dir() -> PathBuf {
        let fallback = || std::env::temp_dir().join("fluxdb").join("logs");
        #[cfg(target_os = "linux")]
        {
            let state = std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")));
            return state
                .map(|s| s.join("fluxdb").join("logs"))
                .unwrap_or_else(fallback);
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::default_root()
                .map(|root| root.join("logs"))
                .unwrap_or_else(|_| fallback())
        }
    }

    /// 默认导出下载目录（方案 §12.4）：优先平台下载目录（Known Folder / XDG），
    /// 退回用户主目录，最后退回临时目录；不回退到进程工作目录/安装目录。
    pub fn default_download_dir() -> PathBuf {
        dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir)
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    fn completion_index_root(&self) -> PathBuf {
        self.root.join("completion-index")
    }

    fn completion_index_dir(
        &self,
        connection: &ConnectionConfig,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> PathBuf {
        self.completion_index_root()
            .join("connections")
            .join(connection_fingerprint(connection))
            .join("databases")
            .join(database_hash(database, schema))
    }

    fn legacy_completion_index_path(
        &self,
        connection: &ConnectionConfig,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> PathBuf {
        self.completion_index_dir(connection, database, schema)
            .join("index.toml")
    }

    pub fn load_completion_index(
        &self,
        connection: &ConnectionConfig,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Result<Option<CompletionIndexSnapshot>> {
        let dir = self.completion_index_dir(connection, database, schema);
        let meta_path = dir.join("meta.toml");
        if meta_path.exists() {
            let header = read_toml_file::<CompletionIndexHeader>(&meta_path)?;
            let tables = read_toml_file::<CompletionIndexTables>(&dir.join("tables.toml"))?.tables;
            let columns =
                read_toml_file::<CompletionIndexColumns>(&dir.join("columns.toml"))?.columns;
            let routines =
                read_toml_file::<CompletionIndexRoutines>(&dir.join("routines.toml"))?.routines;
            let triggers =
                read_toml_file::<CompletionIndexTriggers>(&dir.join("triggers.toml"))?.triggers;
            return Ok(Some(CompletionIndexSnapshot {
                connection_id: header.connection_id,
                database: header.database,
                schema: header.schema,
                tables,
                columns,
                routines,
                triggers,
                meta: header.meta,
            }));
        }

        let legacy_path = self.legacy_completion_index_path(connection, database, schema);
        if !legacy_path.exists() {
            return Ok(None);
        }
        read_toml_file::<CompletionIndexSnapshot>(&legacy_path).map(Some)
    }

    pub fn save_completion_index(
        &self,
        connection: &ConnectionConfig,
        database: Option<&str>,
        schema: Option<&str>,
        snapshot: &CompletionIndexSnapshot,
    ) -> Result<()> {
        let dir = self.completion_index_dir(connection, database, schema);
        fs::create_dir_all(&dir).map_err(storage_error)?;
        write_toml_file_if_changed(
            &dir.join("meta.toml"),
            &CompletionIndexHeader {
                connection_id: snapshot.connection_id,
                database: snapshot.database.clone(),
                schema: snapshot.schema.clone(),
                meta: snapshot.meta.clone(),
            },
        )?;
        write_toml_file_if_changed(
            &dir.join("tables.toml"),
            &CompletionIndexTables {
                tables: snapshot.tables.clone(),
            },
        )?;
        write_toml_file_if_changed(
            &dir.join("columns.toml"),
            &CompletionIndexColumns {
                columns: snapshot.columns.clone(),
            },
        )?;
        write_toml_file_if_changed(
            &dir.join("routines.toml"),
            &CompletionIndexRoutines {
                routines: snapshot.routines.clone(),
            },
        )?;
        write_toml_file_if_changed(
            &dir.join("triggers.toml"),
            &CompletionIndexTriggers {
                triggers: snapshot.triggers.clone(),
            },
        )?;
        Ok(())
    }

    pub fn delete_completion_index_for_connection(
        &self,
        connection: &ConnectionConfig,
    ) -> Result<()> {
        let path = self
            .completion_index_root()
            .join("connections")
            .join(connection_fingerprint(connection));
        if path.exists() {
            fs::remove_dir_all(path).map_err(storage_error)?;
        }
        Ok(())
    }

    pub fn clear_completion_indexes(&self) -> Result<()> {
        let path = self.completion_index_root();
        if path.exists() {
            fs::remove_dir_all(path).map_err(storage_error)?;
        }
        Ok(())
    }

    pub fn load_saved_queries(&self) -> Result<Vec<SavedQuery>> {
        let conn = self.open_sqlite()?;
        Ok(
            sqlite::get_json::<Vec<SavedQuery>>(&conn, sqlite::KEY_SAVED_QUERIES)?
                .unwrap_or_default(),
        )
    }

    pub fn save_saved_queries(&self, queries: &[SavedQuery]) -> Result<()> {
        let conn = self.open_sqlite()?;
        sqlite::put_json(&conn, sqlite::KEY_SAVED_QUERIES, &queries.to_vec())
    }

    pub fn load_query_history(&self) -> Result<Vec<QueryHistoryRecord>> {
        let conn = self.open_sqlite()?;
        Ok(
            sqlite::get_json::<Vec<QueryHistoryRecord>>(&conn, sqlite::KEY_QUERY_HISTORY)?
                .unwrap_or_default(),
        )
    }

    pub fn save_query_history(&self, entries: &[QueryHistoryRecord]) -> Result<()> {
        let conn = self.open_sqlite()?;
        let start = entries.len().saturating_sub(1000);
        sqlite::put_json(&conn, sqlite::KEY_QUERY_HISTORY, &entries[start..].to_vec())
    }

    pub fn load_redis_key_search_history(&self) -> Result<Vec<RedisKeySearchHistoryRecord>> {
        let conn = self.open_sqlite()?;
        Ok(sqlite::get_json::<Vec<RedisKeySearchHistoryRecord>>(
            &conn,
            sqlite::KEY_REDIS_KEY_SEARCH_HISTORY,
        )?
        .unwrap_or_default())
    }

    pub fn save_redis_key_search_history(
        &self,
        entries: &[RedisKeySearchHistoryRecord],
    ) -> Result<()> {
        let conn = self.open_sqlite()?;
        let start = entries.len().saturating_sub(1000);
        sqlite::put_json(
            &conn,
            sqlite::KEY_REDIS_KEY_SEARCH_HISTORY,
            &entries[start..].to_vec(),
        )
    }

    /// 加载所有连接 / 库的 Redis Workbench 命令历史（全量扁平，
    /// 由上层按 scope 过滤）。不存在时返回空列表。
    pub fn load_redis_workbench_history(&self) -> Result<Vec<RedisWorkbenchHistoryRecord>> {
        let conn = self.open_sqlite()?;
        Ok(sqlite::get_json::<Vec<RedisWorkbenchHistoryRecord>>(
            &conn,
            sqlite::KEY_REDIS_WORKBENCH_HISTORY,
        )?
        .unwrap_or_default())
    }

    /// 全量保存 Redis Workbench 命令历史并截断（与 SQL 历史同样保留最近 1000 条）。
    pub fn save_redis_workbench_history(
        &self,
        entries: &[RedisWorkbenchHistoryRecord],
    ) -> Result<()> {
        let conn = self.open_sqlite()?;
        let start = entries.len().saturating_sub(1000);
        sqlite::put_json(
            &conn,
            sqlite::KEY_REDIS_WORKBENCH_HISTORY,
            &entries[start..].to_vec(),
        )
    }
}

impl Storage for FileStorage {
    fn load_settings(&self) -> Result<Settings> {
        let path = self.config_path();
        if !path.exists() {
            return Ok(Settings::default());
        }

        let text = fs::read_to_string(path).map_err(storage_error)?;
        toml::from_str(&text).map_err(storage_error)
    }

    fn save_settings(&self, settings: &Settings) -> Result<()> {
        fs::create_dir_all(&self.root).map_err(storage_error)?;
        let text = toml::to_string_pretty(settings).map_err(storage_error)?;
        fs::write(self.config_path(), text).map_err(storage_error)
    }

    fn load_connections(&self) -> Result<Vec<ConnectionConfig>> {
        let conn = self.open_sqlite()?;
        Ok(
            sqlite::get_json::<Vec<ConnectionConfig>>(&conn, sqlite::KEY_CONNECTIONS)?
                .unwrap_or_default()
                .into_iter()
                .map(|connection| self.load_connection_secret(connection))
                .collect(),
        )
    }

    fn save_connections(&self, connections: &[ConnectionConfig]) -> Result<()> {
        for connection in connections {
            self.save_connection_secret(connection)?;
        }
        // 与历史行为一致：保存连接时若已有 layout 则保留并修复，否则按连接重建。
        let conn = self.open_sqlite()?;
        let mut layout = sqlite::get_json::<SidebarLayout>(&conn, sqlite::KEY_SIDEBAR_LAYOUT)?
            .unwrap_or_else(|| SidebarLayout::for_connections(connections));
        layout.repair(connections);
        self.write_connections_and_layout(&conn, connections, &layout)
    }

    fn delete_connection_secrets(&self, connection: &ConnectionConfig) {
        self.delete_owned_keychain_secrets(connection);
    }

    fn load_sidebar_layout(&self, connections: &[ConnectionConfig]) -> Result<SidebarLayout> {
        let conn = self.open_sqlite()?;
        let mut layout = sqlite::get_json::<SidebarLayout>(&conn, sqlite::KEY_SIDEBAR_LAYOUT)?
            .unwrap_or_else(|| SidebarLayout::for_connections(connections));
        layout.repair(connections);
        Ok(layout)
    }

    fn save_sidebar_layout(
        &self,
        connections: &[ConnectionConfig],
        layout: &SidebarLayout,
    ) -> Result<()> {
        let mut layout = layout.clone();
        layout.repair(connections);
        let conn = self.open_sqlite()?;
        self.write_connections_and_layout(&conn, connections, &layout)
    }
}

impl FileStorage {
    fn load_connection_secret(&self, mut connection: ConnectionConfig) -> ConnectionConfig {
        let Some(credential_ref) = self.credential_ref_for_keychain(&connection) else {
            return connection;
        };

        // 扁平历史参数：`options["password"]` 从 Keychain 取回。
        if !connection.options.contains_key(PLAINTEXT_PASSWORD_OPTION) {
            if let Some(secret) = read_keychain_password(credential_ref.clone()) {
                connection
                    .options
                    .insert(PLAINTEXT_PASSWORD_OPTION.to_string(), secret);
            }
        }

        // 结构化档案：逐槽位把受控值从 Keychain 回填到内存。
        if let Some(profile) = connection.redis_profile.as_mut() {
            for (suffix, slot) in profile_secret_slots_mut(profile) {
                if slot.key.is_empty() {
                    slot.key = format!("{credential_ref}{suffix}");
                }
                if slot.inline.is_none() {
                    slot.inline = read_keychain_password(slot.key.clone());
                }
            }
        }

        // MySQL 结构化档案：同理回填（MySQL/Redis 不同栈，槽位后缀无冲突）。
        if let Some(profile) = connection.mysql_profile.as_mut() {
            for (suffix, slot) in mysql_profile_secret_slots_mut(profile) {
                if slot.key.is_empty() {
                    slot.key = format!("{credential_ref}{suffix}");
                }
                if slot.inline.is_none() {
                    slot.inline = read_keychain_password(slot.key.clone());
                }
            }
        }

        // PostgreSQL 结构化档案：同理回填（与 MySQL/Redis 槽位后缀无冲突）。
        if let Some(profile) = connection.postgres_profile.as_mut() {
            for (suffix, slot) in postgres_profile_secret_slots_mut(profile) {
                if slot.key.is_empty() {
                    slot.key = format!("{credential_ref}{suffix}");
                }
                if slot.inline.is_none() {
                    slot.inline = read_keychain_password(slot.key.clone());
                }
            }
        }

        connection
    }

    fn save_connection_secret(&self, connection: &ConnectionConfig) -> Result<()> {
        let Some(credential_ref) = self.credential_ref_for_keychain(connection) else {
            return Ok(());
        };

        // 扁平历史参数：`options["password"]` 写入 Keychain。
        if let Some(password) = connection.options.get(PLAINTEXT_PASSWORD_OPTION) {
            write_keychain_password(&credential_ref, password)?;
        }

        // 结构化档案：逐槽位写 Keychain 后清空内存值。
        if let Some(profile) = connection.redis_profile.as_ref() {
            for (suffix, slot) in profile_secret_slots(profile) {
                let account = if slot.key.is_empty() {
                    format!("{credential_ref}{suffix}")
                } else {
                    slot.key.clone()
                };
                if let Some(value) = slot.inline.as_deref() {
                    write_keychain_password(&account, value)?;
                }
            }
        }

        // MySQL 结构化档案：同理写 Keychain（MySQL/Redis 不同栈，槽位后缀无冲突）。
        if let Some(profile) = connection.mysql_profile.as_ref() {
            for (suffix, slot) in mysql_profile_secret_slots(profile) {
                let account = if slot.key.is_empty() {
                    format!("{credential_ref}{suffix}")
                } else {
                    slot.key.clone()
                };
                if let Some(value) = slot.inline.as_deref() {
                    write_keychain_password(&account, value)?;
                }
            }
        }

        // PostgreSQL 结构化档案：同理写 Keychain（与 MySQL/Redis 槽位后缀无冲突）。
        if let Some(profile) = connection.postgres_profile.as_ref() {
            for (suffix, slot) in postgres_profile_secret_slots(profile) {
                let account = if slot.key.is_empty() {
                    format!("{credential_ref}{suffix}")
                } else {
                    slot.key.clone()
                };
                if let Some(value) = slot.inline.as_deref() {
                    write_keychain_password(&account, value)?;
                }
            }
        }

        Ok(())
    }

    /// 删除该连接自身 credential_ref 所拥有的全部 Keychain 条目（扁平密码 + 各类档案槽位）。
    ///
    /// 按连接拥有权清理：每个连接经 CreateConnection 派生独立 ref，删除只清本连接 ref 下的条目，
    /// 不触碰其他连接的 ref（即保护共享引用——引用共享只在显式共享时才有，本路径不跨连接）。
    /// 删除幂等：条目不存在按成功处理。Keychain 不可用（非常规 root / 非 macOS）时无副作用。
    fn delete_owned_keychain_secrets(&self, connection: &ConnectionConfig) {
        let Some(credential_ref) = self.credential_ref_for_keychain(connection) else {
            return;
        };
        // 扁平历史密码与结构化档案槽位同属该 ref。
        delete_keychain_password(&credential_ref);
        let mut accounts: Vec<String> = Vec::new();
        if let Some(profile) = connection.redis_profile.as_ref() {
            for (suffix, slot) in profile_secret_slots(profile) {
                accounts.push(secret_slot_account(&credential_ref, suffix, slot));
            }
        }
        if let Some(profile) = connection.mysql_profile.as_ref() {
            for (suffix, slot) in mysql_profile_secret_slots(profile) {
                accounts.push(secret_slot_account(&credential_ref, suffix, slot));
            }
        }
        if let Some(profile) = connection.postgres_profile.as_ref() {
            for (suffix, slot) in postgres_profile_secret_slots(profile) {
                accounts.push(secret_slot_account(&credential_ref, suffix, slot));
            }
        }
        for account in accounts {
            delete_keychain_password(&account);
        }
    }

    fn credential_ref_for_keychain(&self, connection: &ConnectionConfig) -> Option<String> {
        if !self.uses_system_keychain() {
            return None;
        }

        connection.credential_ref.clone()
    }

    fn uses_system_keychain(&self) -> bool {
        Self::default_root().is_ok_and(|root| root == self.root)
    }

    /// 打开本 root 下的 sqlite 数据库并确保 kv 表存在。
    fn open_sqlite(&self) -> Result<rusqlite::Connection> {
        let conn = sqlite::open(&self.root)?;
        sqlite::create_schema(&conn).map(|_| ())?;
        Ok(conn)
    }

    /// 把连接列表（剥离明文）与 SidebarLayout 一并写入 sqlite。
    ///
    /// 与历史 toml 行为一致：connections.toml 同时持有两者，保存任一时另一份
    /// 也一并落盘，避免两个入口互相覆盖。
    fn write_connections_and_layout(
        &self,
        conn: &rusqlite::Connection,
        connections: &[ConnectionConfig],
        layout: &SidebarLayout,
    ) -> Result<()> {
        let stripped: Vec<ConnectionConfig> =
            connections.iter().map(strip_plaintext_secrets).collect();
        sqlite::put_json(conn, sqlite::KEY_CONNECTIONS, &stripped)?;
        sqlite::put_json(conn, sqlite::KEY_SIDEBAR_LAYOUT, layout)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryHistoryRecord {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// 历史记录所属 schema（PG）；旧记录缺省为 None，`#[serde(default)]` 兼容加载。
    #[serde(default)]
    pub schema: Option<String>,
    pub text: String,
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default = "default_query_history_kind")]
    pub kind: String,
    #[serde(default = "default_query_history_success")]
    pub success: bool,
    #[serde(default)]
    pub executed_at_unix_secs: u64,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub rollback_sql: Option<String>,
    #[serde(default)]
    pub rollback_snapshot: Option<QueryRollbackSnapshot>,
    /// 写入事务状态（committed/uncommitted/rolled_back，§8.4）；旧记录缺省按已提交。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_state: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub returned_rows: u64,
    #[serde(default)]
    pub affected_rows: u64,
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// Redis Key 搜索历史单条记录，按连接 + 数据库隔离。
///
/// - `connection_id`：所属连接 ID。
/// - `database`：Redis DB 索引字符串（如 `"0"`、`"1"`），`None` 视为 `"0"`。
/// - `text`：搜索词。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedisKeySearchHistoryRecord {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    pub text: String,
}

/// Redis Workbench 命令历史单条记录，按连接 + 逻辑数据库隔离（对齐 RedisInsight 的
/// databaseId 作用域）。
///
/// - `id`：scope 内去重的记录 ID，用于删除定位。
/// - `connection_id` / `database`：归属的连接与逻辑库。
/// - `text`：命令文本，可回填到 Workbench 输入框。
/// - `summary` / `source`：结果摘要与来源（Workbench / HistoryRerun / KeyShortcut）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedisWorkbenchHistoryRecord {
    pub id: u64,
    pub connection_id: ConnectionId,
    pub database: u32,
    pub text: String,
    #[serde(default = "default_redis_workbench_history_success")]
    pub success: bool,
    #[serde(default)]
    pub executed_at_unix_secs: u64,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub source: String,
}

fn default_redis_workbench_history_success() -> bool {
    true
}

fn default_query_history_kind() -> String {
    "query".to_string()
}

fn default_query_history_success() -> bool {
    true
}

fn storage_error(error: impl ToString) -> Error {
    Error::new(ErrorKind::Internal, error.to_string())
}

fn read_toml_file<T: DeserializeOwned>(path: &PathBuf) -> Result<T> {
    let text = fs::read_to_string(path).map_err(storage_error)?;
    toml::from_str::<T>(&text).map_err(storage_error)
}

fn write_toml_file_if_changed<T: Serialize>(path: &PathBuf, value: &T) -> Result<()> {
    let text = toml::to_string_pretty(value).map_err(storage_error)?;
    if path.exists() && fs::read_to_string(path).map_err(storage_error)? == text {
        return Ok(());
    }
    fs::write(path, text).map_err(storage_error)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompletionIndexHeader {
    connection_id: fluxdb_core::ConnectionId,
    database: Option<String>,
    schema: Option<String>,
    meta: CompletionIndexMeta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompletionIndexTables {
    tables: Vec<TableRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompletionIndexColumns {
    columns: Vec<ColumnRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompletionIndexRoutines {
    routines: Vec<RoutineRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompletionIndexTriggers {
    triggers: Vec<TriggerRef>,
}

fn connection_fingerprint(connection: &ConnectionConfig) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    connection.id.hash(&mut hasher);
    format!("{:?}", connection.kind).hash(&mut hasher);
    format!("{:?}", connection.endpoint).hash(&mut hasher);
    connection.name.hash(&mut hasher);
    format!("c{:016x}", hasher.finish())
}

fn database_hash(database: Option<&str>, schema: Option<&str>) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    database
        .unwrap_or("")
        .to_ascii_lowercase()
        .hash(&mut hasher);
    schema.unwrap_or("").to_ascii_lowercase().hash(&mut hasher);
    format!("d{:016x}", hasher.finish())
}

fn strip_plaintext_secrets(connection: &ConnectionConfig) -> ConnectionConfig {
    let mut connection = connection.clone();
    connection.options.retain(|key, _| {
        let key = key.to_ascii_lowercase();
        !key.contains("password") && !key.contains("secret")
    });
    // 结构化档案里的受控值靠 `SecretRef.inline` 的 `#[serde(skip)]` 保证不落盘；
    // 这里再显式清空一份供盘上转储副本，双保险。
    if let Some(profile) = connection.redis_profile.as_mut() {
        for (_, slot) in profile_secret_slots_mut(profile) {
            slot.inline = None;
        }
        // 导入来源元数据可能带明文口令（历史版本/其它导入路径），一律不落盘。
        profile.cloud.imported_name.clear();
    }
    // MySQL 档案同理：清空全部受控值，保证盘上副本零明文。
    if let Some(profile) = connection.mysql_profile.as_mut() {
        for (_, slot) in mysql_profile_secret_slots_mut(profile) {
            slot.inline = None;
        }
    }
    // PostgreSQL 档案同理：清空全部受控值，保证盘上副本零明文。
    if let Some(profile) = connection.postgres_profile.as_mut() {
        for (_, slot) in postgres_profile_secret_slots_mut(profile) {
            slot.inline = None;
        }
    }
    connection
}

/// 遍历 Redis 档案中需要走 Keychain 的「密码类」槽位。
/// 返回 `(Keychain 账号后缀, 该槽的 SecretRef)`。
///
/// 证书/私钥文件一律以文件路径引用（`key` 存路径，非密码语义），不在此列；
/// 只有真正的密码（基础密码、SSH 密码、SSH 私钥口令）才进 Keychain。
/// 槽位在 Keychain 中的 account：未显式设 key 时按 `{ref}{suffix}` 推导，否则用显式 key。
fn secret_slot_account(credential_ref: &str, suffix: &str, slot: &SecretRef) -> String {
    if slot.key.is_empty() {
        format!("{credential_ref}{suffix}")
    } else {
        slot.key.clone()
    }
}

fn profile_secret_slots(profile: &RedisConnectionProfile) -> Vec<(&'static str, &SecretRef)> {
    let mut slots: Vec<(&'static str, &SecretRef)> = vec![("", &profile.basic.password)];
    if profile.ssh.enabled {
        slots.push((".ssh_password", &profile.ssh.password));
        slots.push((".ssh_passphrase", &profile.ssh.passphrase));
    }
    slots
}

/// 可变版，供回填 / 剥离时原地改写槽位。
fn profile_secret_slots_mut(
    profile: &mut RedisConnectionProfile,
) -> Vec<(&'static str, &mut SecretRef)> {
    let mut slots: Vec<(&'static str, &mut SecretRef)> = vec![("", &mut profile.basic.password)];
    if profile.ssh.enabled {
        slots.push((".ssh_password", &mut profile.ssh.password));
        slots.push((".ssh_passphrase", &mut profile.ssh.passphrase));
    }
    slots
}

/// 遍历 MySQL 档案中需要走 Keychain 的「密码类」槽位。
/// 返回 `(Keychain 账号后缀, 该槽的 SecretRef)`。
///
/// 证书/私钥文件一律以文件路径引用（`key` 存路径，非密码语义），不在此列；
/// 只有真正的密码（基础密码、SSH 密码、SSH 私钥口令、代理密码）才进 Keychain。
fn mysql_profile_secret_slots(profile: &MysqlConnectionProfile) -> Vec<(&'static str, &SecretRef)> {
    let mut slots: Vec<(&'static str, &SecretRef)> = vec![("", &profile.basic.password)];
    for layer in &profile.transport {
        match layer {
            MysqlTransportLayer::Ssh(ssh) if ssh.enabled => {
                slots.push((".ssh_password", &ssh.password));
                slots.push((".ssh_passphrase", &ssh.passphrase));
            }
            MysqlTransportLayer::Proxy(proxy) if proxy.enabled => {
                slots.push((".proxy_password", &proxy.password));
            }
            _ => {}
        }
    }
    slots
}

/// MySQL 档案槽位的可变版，供回填 / 剥离时原地改写槽位。
fn mysql_profile_secret_slots_mut(
    profile: &mut MysqlConnectionProfile,
) -> Vec<(&'static str, &mut SecretRef)> {
    // 解构以取得不重叠的借用（basic 与 transport 互不借用）。
    let MysqlConnectionProfile {
        basic, transport, ..
    } = profile;
    let mut slots: Vec<(&'static str, &mut SecretRef)> = vec![("", &mut basic.password)];
    for layer in transport.iter_mut() {
        match layer {
            MysqlTransportLayer::Ssh(ssh) if ssh.enabled => {
                slots.push((".ssh_password", &mut ssh.password));
                slots.push((".ssh_passphrase", &mut ssh.passphrase));
            }
            MysqlTransportLayer::Proxy(proxy) if proxy.enabled => {
                slots.push((".proxy_password", &mut proxy.password));
            }
            _ => {}
        }
    }
    slots
}

/// 遍历 PostgreSQL 档案中需要走 Keychain 的「密码类」槽位。
/// 返回 `(Keychain 账号后缀, 该槽的 SecretRef)`。
///
/// 证书/私钥文件一律以文件路径引用（`key` 存路径，非密码语义），不在此列；
/// 只有真正的密码（基础密码、SSH 密码、SSH 私钥口令、代理密码）才进 Keychain。
fn postgres_profile_secret_slots(
    profile: &PostgresConnectionProfile,
) -> Vec<(&'static str, &SecretRef)> {
    let mut slots: Vec<(&'static str, &SecretRef)> = vec![("", &profile.basic.password)];
    for layer in &profile.transport {
        match layer {
            PostgresTransportLayer::Ssh(ssh) if ssh.enabled => {
                slots.push((".ssh_password", &ssh.password));
                slots.push((".ssh_passphrase", &ssh.passphrase));
            }
            PostgresTransportLayer::Proxy(proxy) if proxy.enabled => {
                slots.push((".proxy_password", &proxy.password));
            }
            _ => {}
        }
    }
    slots
}

/// PostgreSQL 档案槽位的可变版，供回填 / 剥离时原地改写槽位。
fn postgres_profile_secret_slots_mut(
    profile: &mut PostgresConnectionProfile,
) -> Vec<(&'static str, &mut SecretRef)> {
    // 解构以取得不重叠的借用（basic 与 transport 互不借用）。
    let PostgresConnectionProfile {
        basic, transport, ..
    } = profile;
    let mut slots: Vec<(&'static str, &mut SecretRef)> = vec![("", &mut basic.password)];
    for layer in transport.iter_mut() {
        match layer {
            PostgresTransportLayer::Ssh(ssh) if ssh.enabled => {
                slots.push((".ssh_password", &mut ssh.password));
                slots.push((".ssh_passphrase", &mut ssh.passphrase));
            }
            PostgresTransportLayer::Proxy(proxy) if proxy.enabled => {
                slots.push((".proxy_password", &mut proxy.password));
            }
            _ => {}
        }
    }
    slots
}

#[cfg(target_os = "macos")]
fn read_keychain_password(account: String) -> Option<String> {
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            &account,
            "-w",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let password = String::from_utf8(output.stdout).ok()?;
    Some(password.trim_end_matches(['\r', '\n']).to_string())
        .filter(|password| !password.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn read_keychain_password(_: String) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn write_keychain_password(account: &str, password: &str) -> Result<()> {
    let status = Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
            "-w",
            password,
            "-U",
        ])
        .status()
        .map_err(storage_error)?;

    if status.success() {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::Internal, "保存密码到 Keychain 失败"))
    }
}

#[cfg(target_os = "macos")]
fn delete_keychain_password(account: &str) {
    let _ = Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
        ])
        .status();
    // 条目不存在（含未保存过）按成功处理：删除幂等，不因缺条目报错。
}

#[cfg(not(target_os = "macos"))]
fn delete_keychain_password(_: &str) {}

#[cfg(not(target_os = "macos"))]
fn write_keychain_password(_: &str, _: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    use fluxdb_core::{
        COMPLETION_INDEX_VERSION, ColumnRef, CompletionIndexMeta, CompletionIndexSnapshot,
        ConnectionGroup, ConnectionGroupId, ConnectionId, DatabaseKind, Endpoint, LogLevel,
        ObjectKind, SavedQuery, SidebarOrderEntry, TableFingerprint, TableRef, Theme,
    };

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    // 平台目录解析：三平台 CI 各自原生运行，验证本平台分支（AI-01，方案 §4.1）。
    #[test]
    fn default_root_resolves_platform_data_dir() {
        let root = FileStorage::default_root().expect("default_root 应能解析");
        #[cfg(target_os = "macos")]
        assert!(
            root.ends_with("Library/Application Support/fluxdb"),
            "macOS 根目录不符: {root:?}"
        );
        #[cfg(target_os = "windows")]
        assert!(
            root.ends_with("FluxDB") && root.to_string_lossy().contains("Local"),
            "Windows 根目录应为 %LOCALAPPDATA%/FluxDB: {root:?}"
        );
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert!(
            root.ends_with("fluxdb") && root.to_string_lossy().contains(".local/share"),
            "Linux 根目录应为 XDG data/fluxdb: {root:?}"
        );
    }

    #[test]
    fn default_log_dir_is_absolute() {
        let dir = FileStorage::default_log_dir();
        assert!(dir.is_absolute(), "日志目录应为绝对路径: {dir:?}");
        assert!(dir.ends_with("logs"), "日志目录应以 logs 结尾: {dir:?}");
    }

    #[test]
    fn default_download_dir_is_absolute() {
        assert!(FileStorage::default_download_dir().is_absolute());
    }

    #[test]
    fn missing_settings_returns_default() {
        let storage = FileStorage::new(unique_temp_dir());

        assert_eq!(storage.load_settings().unwrap(), Settings::default());
    }

    /// 迁移守卫：缺少后加字段的旧配置文件必须仍能加载。
    ///
    /// 这条测的是**新增字段是否带了 `#[serde(default...)]`**。漏了的话
    /// `toml::from_str` 会整体失败，而 `app_boot` 处是
    /// `load_settings().unwrap_or_default()` —— 结果就是**用户全部设置被静默重置**。
    /// 这是唯一会破坏用户数据的失败模式，且平时跑不出来。
    ///
    /// 旧文件不是手写常量，而是从真实序列化产物里**删掉**这些字段得到：
    /// 这样它永远等于「上个版本写出的文件」，也不会因无关必填字段增减而失效。
    #[test]
    fn settings_toml_missing_newer_fields_still_loads() {
        const NEWER_FIELDS: [&str; 3] = [
            "data_table_page_size",
            "results_placement",
            "redis_workbench_editor_width",
        ];
        let full = toml::to_string_pretty(&Settings::default()).unwrap();
        let legacy: String = full
            .lines()
            .filter(|line| {
                !NEWER_FIELDS
                    .iter()
                    .any(|field| line.trim_start().starts_with(field))
            })
            .collect::<Vec<_>>()
            .join("\n");

        // 先确认真的删干净了，否则这条测试会退化成一个假绿。
        for field in NEWER_FIELDS {
            assert!(!legacy.contains(field), "{field} 应从旧配置文件里被删掉");
        }

        let parsed: Settings = toml::from_str(&legacy)
            .expect("缺少后加字段的旧配置必须仍能加载，否则用户设置会被重置");
        assert_eq!(parsed, Settings::default());
    }

    /// 凭据清理的槽位 account 推导必须与保存路径一致（save/delete 共用 secret_slot_account），
    /// 保证「按连接 ref 只删本连接」，且删除幂等不报错。
    #[test]
    fn secret_account_derivation_matches_save_and_delete_is_idempotent() {
        // 空 key 按 ref+suffix 推导；显式 key 用显式值。
        let slot_empty = SecretRef::inline("pw");
        assert_eq!(
            secret_slot_account("gdb.connection.7", ":pg:password", &slot_empty),
            "gdb.connection.7:pg:password"
        );
        let slot_named = SecretRef::ref_key("user-shared-key");
        assert_eq!(
            secret_slot_account("gdb.connection.7", ":pg:password", &slot_named),
            "user-shared-key"
        );

        // 携带 PG 档案的连接：delete_owned_keychain_secrets 遍历槽位，不 panic、幂等。
        // 测试目录非系统 Keychain（uses_system_keychain=false）→ credential_ref_for_keychain 返回 None，
        // 走无副作用路径；这里主要锁定枚举逻辑与「无 Keychain 时安全无操作」。
        let mut profile = fluxdb_core::PostgresConnectionProfile::default();
        profile.basic.password = SecretRef::inline("secret");
        let mut config = ConnectionConfig {
            id: ConnectionId(9),
            name: "pg".into(),
            kind: DatabaseKind::Postgres,
            endpoint: Endpoint::Tcp {
                host: "h".into(),
                port: 5432,
                database: None,
            },
            credential_ref: Some("gdb.connection.9".into()),
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: Some(profile),
        };
        let storage = FileStorage::new(unique_temp_dir());
        storage.delete_connection_secrets(&config);
        storage.delete_connection_secrets(&config); // 重复删除幂等
        config.credential_ref = None; // 无 ref 也安全无操作
        storage.delete_connection_secrets(&config);
    }

    #[test]
    fn saves_and_loads_settings() {
        let storage = FileStorage::new(unique_temp_dir());
        let settings = Settings {
            theme: Theme::Dark,
            log_level: LogLevel::Debug,
            log_path: "/tmp/gdb-test-logs".to_string(),
            button_radius: 8,
            large_radius: 12,
            show_shadows: false,
            focus_ring: false,
            scrollbar_mode: fluxdb_core::ScrollbarMode::Always,
            ui_density: fluxdb_core::UiDensity::Compact,
            show_status_bar: false,
            reduce_motion: true,
            performance_diagnostics: true,
            global_font_family: "Inter".to_string(),
            light_theme: "Default Light".to_string(),
            dark_theme: "Default Dark".to_string(),
            page_size: 250,
            data_table_page_size: 500,
            show_sidebar: false,
            show_inspector: true,
            editor_font_size: 13,
            editor_line_height: 18,
            editor_tab_width: 2,
            editor_word_wrap: true,
            confirm_dangerous_sql: false,
            confirm_dangerous_redis: false,
            dangerous_sql_actions: std::collections::BTreeSet::from(["truncate".to_string()]),
            enable_completion_index: true,
            redis_workbench_editor_ratio: 63,
            results_placement: fluxdb_core::ResultsPlacement::Right,
            redis_workbench_editor_width: 900,
            custom_keybindings: BTreeMap::from([(
                "app.refresh".to_string(),
                "ctrl-shift-r".to_string(),
            )]),
            backup_dir: String::new(),
            mysqldump_path: String::new(),
            mysql_client_dir: String::new(),
            mysql_client_download_source: String::new(),
            sqlite3_path: String::new(),
            pg_dump_path: String::new(),
            pg_client_dir: String::new(),
            pg_client_download_source: String::new(),
        };

        storage.save_settings(&settings).unwrap();

        assert_eq!(storage.load_settings().unwrap(), settings);
    }

    #[test]
    fn saves_and_loads_saved_queries() {
        let storage = FileStorage::new(unique_temp_dir());
        let queries = vec![SavedQuery {
            id: 1,
            connection_id: ConnectionId(7),
            database: Some("shop".to_string()),
            schema: None,
            name: "orders.sql".to_string(),
            text: "select * from orders".to_string(),
        }];

        storage.save_saved_queries(&queries).unwrap();

        assert_eq!(storage.load_saved_queries().unwrap(), queries);
    }

    #[test]
    fn saves_loads_and_limits_query_history() {
        let storage = FileStorage::new(unique_temp_dir());
        let entries = (0..1002)
            .map(|index| QueryHistoryRecord {
                connection_id: ConnectionId(7),
                database: Some("shop".to_string()),
                schema: None,
                text: format!("select {index}"),
                tables: vec!["orders".to_string()],
                kind: "query".to_string(),
                success: true,
                executed_at_unix_secs: 0,
                object: None,
                rollback_sql: None,
                rollback_snapshot: None,
                transaction_state: None,
                message: None,
                returned_rows: 0,
                affected_rows: 0,
                elapsed_ms: 0,
            })
            .collect::<Vec<_>>();

        storage.save_query_history(&entries).unwrap();

        let loaded = storage.load_query_history().unwrap();
        assert_eq!(loaded.len(), 1000);
        assert_eq!(loaded[0].text, "select 2");
        assert_eq!(loaded[999].text, "select 1001");
    }

    #[test]
    fn saves_and_loads_redis_key_search_history() {
        let storage = FileStorage::new(unique_temp_dir());
        let entries = vec![
            RedisKeySearchHistoryRecord {
                connection_id: ConnectionId(4),
                database: Some("0".to_string()),
                text: "user:*".to_string(),
            },
            RedisKeySearchHistoryRecord {
                connection_id: ConnectionId(4),
                database: Some("1".to_string()),
                text: "order:*".to_string(),
            },
        ];

        storage.save_redis_key_search_history(&entries).unwrap();

        assert_eq!(storage.load_redis_key_search_history().unwrap(), entries);
    }

    #[test]
    fn missing_redis_key_search_history_returns_empty() {
        let storage = FileStorage::new(unique_temp_dir());

        assert_eq!(storage.load_redis_key_search_history().unwrap(), Vec::new());
    }

    #[test]
    fn saves_loads_and_clears_completion_index_without_plaintext_path() {
        let storage = FileStorage::new(unique_temp_dir());
        let connection = sample_connections()[0].clone();
        let snapshot = sample_completion_snapshot(connection.id);

        storage
            .save_completion_index(&connection, Some("production"), None, &snapshot)
            .unwrap();

        assert_eq!(
            storage
                .load_completion_index(&connection, Some("production"), None)
                .unwrap(),
            Some(snapshot)
        );
        let index_root = storage.completion_index_root();
        let paths = fs::read_dir(index_root.join("connections"))
            .unwrap()
            .map(|entry| entry.unwrap().path().display().to_string())
            .collect::<Vec<_>>();
        assert!(paths.iter().all(|path| !path.contains("production")));
        let database_dir = storage.completion_index_dir(&connection, Some("production"), None);
        assert!(database_dir.join("meta.toml").exists());
        assert!(database_dir.join("tables.toml").exists());
        assert!(database_dir.join("columns.toml").exists());
        assert!(!database_dir.join("index.toml").exists());

        storage
            .delete_completion_index_for_connection(&connection)
            .unwrap();
        assert!(
            storage
                .load_completion_index(&connection, Some("production"), None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn completion_index_save_skips_unchanged_split_files() {
        let storage = FileStorage::new(unique_temp_dir());
        let connection = sample_connections()[0].clone();
        let snapshot = sample_completion_snapshot(connection.id);

        storage
            .save_completion_index(&connection, Some("production"), None, &snapshot)
            .unwrap();
        let columns_path = storage
            .completion_index_dir(&connection, Some("production"), None)
            .join("columns.toml");
        let first_modified = fs::metadata(&columns_path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        storage
            .save_completion_index(&connection, Some("production"), None, &snapshot)
            .unwrap();

        assert_eq!(
            fs::metadata(columns_path).unwrap().modified().unwrap(),
            first_modified
        );
    }

    #[test]
    fn completion_index_loads_legacy_single_file_snapshot() {
        let storage = FileStorage::new(unique_temp_dir());
        let connection = sample_connections()[0].clone();
        let snapshot = sample_completion_snapshot(connection.id);
        let legacy_path =
            storage.legacy_completion_index_path(&connection, Some("production"), None);
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(&legacy_path, toml::to_string_pretty(&snapshot).unwrap()).unwrap();

        assert_eq!(
            storage
                .load_completion_index(&connection, Some("production"), None)
                .unwrap(),
            Some(snapshot)
        );
    }

    /// 表指纹是 64 位哈希，取值范围可能超出 TOML 的 i64 整数上限：必须能落盘并原值读回。
    ///
    /// 回归：指纹曾按整数写入，哈希最高位为 1 时整份快照序列化即失败（错误一度被调用方
    /// 吞掉，表现为「补全索引从不落盘、每次冷启动重查目录」）。
    #[test]
    fn completion_index_round_trips_table_fingerprints_beyond_i64() {
        let storage = FileStorage::new(unique_temp_dir());
        let connection = sample_connections()[0].clone();
        let mut snapshot = sample_completion_snapshot(connection.id);
        snapshot.meta.table_fingerprints = vec![TableFingerprint {
            database: Some("production".to_string()),
            schema: None,
            table: "Product".to_string(),
            fingerprint: u64::MAX,
        }];

        storage
            .save_completion_index(&connection, Some("production"), None, &snapshot)
            .expect("溢出 i64 的指纹也必须能落盘");

        assert_eq!(
            storage
                .load_completion_index(&connection, Some("production"), None)
                .unwrap(),
            Some(snapshot)
        );
    }

    #[test]
    fn missing_connections_returns_empty_list() {
        let storage = FileStorage::new(unique_temp_dir());

        assert_eq!(storage.load_connections().unwrap(), Vec::new());
    }

    #[test]
    fn saves_and_loads_connections_without_plaintext_password() {
        let storage = FileStorage::new(unique_temp_dir());
        let connections = sample_connections();

        storage.save_connections(&connections).unwrap();

        let loaded = storage.load_connections().unwrap();
        // 临时目录不启用 Keychain：密钥不落盘（正确安全行为），非密钥字段应等值往返。
        assert_eq!(loaded[0], connections[0]);
        assert_eq!(loaded[1], connections[1]);
        assert_eq!(loaded[2], connections[2]);
        // Redis 档案的非密钥字段应保留。
        let redis = loaded[3].redis_profile.as_ref().expect("redis profile");
        assert_eq!(redis.basic.host, "127.0.0.1");
        assert_eq!(redis.ssh.host, "bastion");
        assert!(redis.basic.password.inline.is_none(), "密钥不应落盘");

        // sqlite 是二进制 WAL 文件，按字节读取断言明文密钥不入库。
        let bytes = fs::read(sqlite::db_path(&storage.root)).unwrap();
        for forbidden in ["topsecret", "sshpass", "do-not-save-this"] {
            assert!(
                !bytes
                    .windows(forbidden.len())
                    .any(|w| w == forbidden.as_bytes()),
                "明文密钥不应落入 sqlite: {forbidden}"
            );
        }
        assert!(
            bytes
                .windows("credential_ref".len())
                .any(|w| w == "credential_ref".as_bytes()),
            "credential_ref 应保留入库"
        );
    }

    #[test]
    fn import_connections_if_empty_keeps_existing_connections() {
        let storage = FileStorage::new(unique_temp_dir());
        let defaults = sample_connections();

        assert_eq!(
            storage.import_connections_if_empty(&defaults).unwrap(),
            defaults
        );

        let existing = vec![defaults[0].clone()];
        storage.save_connections(&existing).unwrap();

        assert_eq!(
            storage.import_connections_if_empty(&defaults).unwrap(),
            existing
        );
    }

    #[test]
    fn drops_plaintext_secret_options_when_saving_connections() {
        let storage = FileStorage::new(unique_temp_dir());
        let mut connections = sample_connections();
        connections[0]
            .options
            .insert("password".to_string(), "do-not-save-this".to_string());

        storage.save_connections(&connections).unwrap();

        // sqlite 是二进制文件，按字节读取断言明文密钥值不入库；
        // profile 里字段名含 password 属正常结构，不在校验范围。
        let bytes = fs::read(sqlite::db_path(&storage.root)).unwrap();
        assert!(
            !bytes
                .windows("do-not-save-this".len())
                .any(|w| w == "do-not-save-this".as_bytes()),
            "明文密钥值不应落入 sqlite"
        );
        assert!(storage.load_connections().unwrap()[0].options.is_empty());
    }

    #[test]
    fn mysql_profile_strips_secret_inlines_and_enumerates_slots() {
        use fluxdb_core::{
            MysqlBasicOptions, MysqlConnectionProfile, MysqlProxy, MysqlProxyType, MysqlSshOptions,
            MysqlTransportLayer,
        };
        let mut profile = MysqlConnectionProfile {
            basic: MysqlBasicOptions {
                host: "db".into(),
                port: 3306,
                password: SecretRef::inline("mysql-secret"),
                ..Default::default()
            },
            transport: vec![
                MysqlTransportLayer::Ssh(MysqlSshOptions {
                    enabled: true,
                    host: "jump".into(),
                    port: 22,
                    username: "bob".into(),
                    password: SecretRef::inline("ssh-secret"),
                    passphrase: SecretRef::inline("ssh-pass"),
                    ..Default::default()
                }),
                MysqlTransportLayer::Proxy(MysqlProxy {
                    enabled: true,
                    proxy_type: MysqlProxyType::Socks5,
                    host: "proxy".into(),
                    port: 1080,
                    password: SecretRef::inline("proxy-secret"),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        };

        // 槽位枚举：基础密码 + SSH 密码 + SSH 口令 + 代理密码。
        let suffixes: Vec<_> = mysql_profile_secret_slots(&profile)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(
            suffixes,
            vec!["", ".ssh_password", ".ssh_passphrase", ".proxy_password"]
        );

        // 剥离后所有内联密钥清空。
        for (_, slot) in mysql_profile_secret_slots_mut(&mut profile) {
            slot.inline = None;
        }
        assert!(profile.basic.password.inline.is_none());
        let ssh = &profile.transport[0];
        let MysqlTransportLayer::Ssh(ssh) = ssh else {
            panic!("expected ssh")
        };
        assert!(ssh.password.inline.is_none());
        assert!(ssh.passphrase.inline.is_none());
        let proxy = &profile.transport[1];
        let MysqlTransportLayer::Proxy(proxy) = proxy else {
            panic!("expected proxy")
        };
        assert!(proxy.password.inline.is_none());
    }

    #[test]
    fn postgres_profile_strips_secret_inlines_and_enumerates_slots() {
        use fluxdb_core::{
            PostgresBasicOptions, PostgresConnectionProfile, PostgresProxy, PostgresProxyType,
            PostgresSshOptions, PostgresTransportLayer,
        };
        let mut profile = PostgresConnectionProfile {
            basic: PostgresBasicOptions {
                host: "db".into(),
                port: 5432,
                username: "postgres".into(),
                password: SecretRef::inline("pg-secret"),
                ..Default::default()
            },
            transport: vec![
                PostgresTransportLayer::Ssh(PostgresSshOptions {
                    enabled: true,
                    host: "jump".into(),
                    port: 22,
                    username: "bob".into(),
                    password: SecretRef::inline("ssh-secret"),
                    passphrase: SecretRef::inline("ssh-pass"),
                    ..Default::default()
                }),
                PostgresTransportLayer::Proxy(PostgresProxy {
                    enabled: true,
                    proxy_type: PostgresProxyType::Socks5,
                    host: "proxy".into(),
                    port: 1080,
                    password: SecretRef::inline("proxy-secret"),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        };

        // 槽位枚举：基础密码 + SSH 密码 + SSH 口令 + 代理密码。
        let suffixes: Vec<_> = postgres_profile_secret_slots(&profile)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(
            suffixes,
            vec!["", ".ssh_password", ".ssh_passphrase", ".proxy_password"]
        );

        // 剥离后所有内联密钥清空。
        for (_, slot) in postgres_profile_secret_slots_mut(&mut profile) {
            slot.inline = None;
        }
        assert!(profile.basic.password.inline.is_none());
        let ssh = &profile.transport[0];
        let PostgresTransportLayer::Ssh(ssh) = ssh else {
            panic!("expected ssh")
        };
        assert!(ssh.password.inline.is_none());
        assert!(ssh.passphrase.inline.is_none());
        let proxy = &profile.transport[1];
        let PostgresTransportLayer::Proxy(proxy) = proxy else {
            panic!("expected proxy")
        };
        assert!(proxy.password.inline.is_none());
    }

    #[test]
    fn saves_and_loads_sidebar_layout() {
        let storage = FileStorage::new(unique_temp_dir());
        let connections = sample_connections();
        storage.save_connections(&connections).unwrap();

        let layout = SidebarLayout {
            groups: vec![ConnectionGroup {
                id: ConnectionGroupId(1),
                name: "开发".to_string(),
                collapsed: true,
            }],
            order: vec![
                SidebarOrderEntry::Group {
                    id: ConnectionGroupId(1),
                    connection_ids: vec![ConnectionId(1)],
                },
                SidebarOrderEntry::Connection {
                    id: ConnectionId(2),
                },
            ],
            table_folders: BTreeMap::from([(
                "1:main:tables".to_string(),
                vec!["业务".to_string(), "日志".to_string()],
            )]),
            table_folder_assignments: BTreeMap::from([(
                "1:main::orders".to_string(),
                ("1:main:tables".to_string(), "业务".to_string()),
            )]),
        };

        storage.save_sidebar_layout(&connections, &layout).unwrap();

        let mut expected = layout;
        expected.repair(&connections);
        assert_eq!(storage.load_sidebar_layout(&connections).unwrap(), expected);
    }

    fn unique_temp_dir() -> PathBuf {
        let counter = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "fluxdb-storage-test-{}-{}-{}",
            std::process::id(),
            counter,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn sample_completion_snapshot(connection_id: ConnectionId) -> CompletionIndexSnapshot {
        CompletionIndexSnapshot {
            connection_id,
            database: Some("production".to_string()),
            schema: None,
            tables: vec![TableRef {
                database: Some("production".to_string()),
                schema: None,
                name: "Product".to_string(),
                kind: ObjectKind::Table,
                rows: None,
                comment: None,
            }],
            columns: vec![ColumnRef {
                database: Some("production".to_string()),
                schema: None,
                table: "Product".to_string(),
                column: "name".to_string(),
                type_name: Some("varchar(255)".to_string()),
                nullable: false,
                primary_key: false,
                ordinal_position: Some(2),
                comment: None,
            }],
            routines: Vec::new(),
            triggers: Vec::new(),
            meta: CompletionIndexMeta {
                app_index_version: COMPLETION_INDEX_VERSION,
                db_kind: DatabaseKind::MySql,
                last_indexed_at: 1,
                last_verified_at: 1,
                ttl_seconds: 1800,
                dirty: false,
                table_count: 1,
                table_fingerprints: Vec::new(),
            },
        }
    }

    fn sample_connections() -> Vec<ConnectionConfig> {
        vec![
            ConnectionConfig {
                id: ConnectionId(1),
                name: "MySQL Local".to_string(),
                kind: DatabaseKind::MySql,
                endpoint: Endpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: 3306,
                    database: Some("app".to_string()),
                },
                credential_ref: Some("gdb.connection.1".to_string()),
                options: BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
            },
            ConnectionConfig {
                id: ConnectionId(2),
                name: "SQLite Demo".to_string(),
                kind: DatabaseKind::Sqlite,
                endpoint: Endpoint::SqliteFile {
                    path: "demo.db".into(),
                    read_only: false,
                },
                credential_ref: None,
                options: BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
            },
            ConnectionConfig {
                id: ConnectionId(3),
                name: "Mongo Dev".to_string(),
                kind: DatabaseKind::MongoDb,
                endpoint: Endpoint::Uri {
                    uri: "mongodb://localhost:27017".to_string(),
                },
                credential_ref: Some("gdb.connection.3".to_string()),
                options: BTreeMap::new(),
                redis_profile: None,
                mysql_profile: None,
                postgres_profile: None,
            },
            ConnectionConfig {
                id: ConnectionId(4),
                name: "Redis Cache".to_string(),
                kind: DatabaseKind::Redis,
                endpoint: Endpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port: 6379,
                    database: Some("0".to_string()),
                },
                credential_ref: Some("gdb.connection.4".to_string()),
                options: BTreeMap::new(),
                redis_profile: Some(RedisConnectionProfile {
                    basic: fluxdb_core::RedisBasicOptions {
                        host: "127.0.0.1".to_string(),
                        port: 6379,
                        database: Some("0".to_string()),
                        username: Some("default".to_string()),
                        password: SecretRef::inline("topsecret"),
                    },
                    ssh: fluxdb_core::RedisSshOptions {
                        enabled: true,
                        host: "bastion".to_string(),
                        port: 22,
                        username: "suv".to_string(),
                        password: SecretRef::inline("sshpass"),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                mysql_profile: None,
                postgres_profile: None,
            },
        ]
    }

    fn redis_record(
        id: u64,
        connection_id: u64,
        database: u32,
        text: &str,
    ) -> RedisWorkbenchHistoryRecord {
        RedisWorkbenchHistoryRecord {
            id,
            connection_id: ConnectionId(connection_id),
            database,
            text: text.to_string(),
            success: true,
            executed_at_unix_secs: 1000 + id,
            summary: "1 个命令 · 成功 1".to_string(),
            source: "workbench".to_string(),
        }
    }

    #[test]
    fn saves_and_loads_redis_workbench_history_round_trip() {
        let storage = FileStorage::new(unique_temp_dir());
        let entries = vec![
            redis_record(1, 1, 0, "GET a"),
            redis_record(2, 1, 1, "GET b"),
            redis_record(3, 2, 0, "SET k v"),
        ];

        storage.save_redis_workbench_history(&entries).unwrap();

        // 加载后应与写入一致（保持记录字段 + 跨连接 / 库的记录都保留）。
        let loaded = storage.load_redis_workbench_history().unwrap();
        assert_eq!(loaded, entries);
    }

    #[test]
    fn missing_redis_workbench_history_returns_empty() {
        let storage = FileStorage::new(unique_temp_dir());
        assert!(storage.load_redis_workbench_history().unwrap().is_empty());
    }

    #[test]
    fn save_redis_workbench_history_truncates_to_1000() {
        let storage = FileStorage::new(unique_temp_dir());
        // 写入 1005 条，应只保留最近 1000 条（丢弃最早 5 条）。
        let entries: Vec<_> = (0..1005).map(|i| redis_record(i, 1, 0, "CMD")).collect();

        storage.save_redis_workbench_history(&entries).unwrap();

        let loaded = storage.load_redis_workbench_history().unwrap();
        assert_eq!(loaded.len(), 1000, "应截断到最近 1000 条");
        assert_eq!(loaded[0].id, 5, "最早的 5 条应被丢弃，保留从 id=5 起");
        assert_eq!(loaded[999].id, 1004, "最新一条应保留");
    }
}
