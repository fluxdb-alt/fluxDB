use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

mod credential;
mod sqlite;

#[path = "parts/connection_secrets.rs"]
mod connection_secrets;

/// 连接持久化：`connections` 表（一条连接一行）的建表与 row↔结构体适配。
#[path = "parts/connection_store.rs"]
mod connection_store;

/// 历史持久化：`history` 表（所有数据库类型共一张表）的建表、记录类型与读写。
#[path = "parts/history_store.rs"]
mod history_store;

pub use history_store::{
    QueryHistoryRecord, RedisKeySearchHistoryRecord, RedisWorkbenchHistoryRecord,
};

use fluxdb_core::{
    ColumnRef, CompletionIndexMeta, CompletionIndexSnapshot, ConnectionConfig, ConnectionId,
    ErRebindEntity, ErRelationship, Error, ErrorKind, MysqlConnectionProfile, MysqlTransportLayer,
    PostgresConnectionProfile, PostgresTransportLayer, RedisConnectionProfile, Result, RoutineRef,
    SavedQuery, SecretRef, Settings, SidebarLayout, TableRef, TriggerRef,
};

/// ER 逻辑关系目录的 kv key 前缀：`er_rel:{scope}` 存该作用域的 `Vec<ErRelationship>`。
pub const ER_REL_KEY_PREFIX: &str = "er_rel:";
/// ER 结构快照 kv key 前缀：`er_snapshot:{scope}` 存该作用域的 `Vec<ErRebindEntity>`。
/// 供结构刷新重绑比对（§5.2/§二.4）。
pub const ER_SNAPSHOT_KEY_PREFIX: &str = "er_snapshot:";
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const PLAINTEXT_PASSWORD_OPTION: &str = "password";

pub trait Storage {
    fn load_settings(&self) -> Result<Settings>;
    fn save_settings(&self, settings: &Settings) -> Result<()>;
    fn load_connections(&self) -> Result<Vec<ConnectionConfig>>;
    fn save_connections(&self, connections: &[ConnectionConfig]) -> Result<()>;
    /// 删除该连接在 SQLite 中拥有的加密凭据（不触及旧系统条目）。
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

    // ---- SQLite 加密凭据后端；以连接 ID + 凭据类别定位 ----
    fn secret_backend_read(&self, id: ConnectionId, kind: &str) -> Result<Option<String>> {
        use crate::credential::{backend, to_storage_error};
        backend(&self.root)
            .read(id.0, kind)
            .map_err(|e| to_storage_error(&e))
    }

    fn secret_backend_write(&self, id: ConnectionId, kind: &str, secret: &str) -> Result<()> {
        use crate::credential::{backend, to_storage_error};
        backend(&self.root)
            .write(id.0, kind, secret)
            .map_err(|e| to_storage_error(&e))
    }

    fn best_effort_secret_read(&self, id: ConnectionId, kind: &str) -> Option<String> {
        match self.secret_backend_read(id, kind) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(target: "fluxdb_storage", %error, connection_id = id.0, kind, "读取加密连接凭据失败，连接保留但不回填密码");
                None
            }
        }
    }

    fn secret_backend_delete(&self, id: ConnectionId, kind: &str) {
        if let Err(error) = self.secret_backend_delete_checked(id, kind) {
            tracing::warn!(target: "fluxdb_storage", %error, connection_id = id.0, kind, "删除加密连接凭据失败");
        }
    }

    fn secret_backend_delete_checked(&self, id: ConnectionId, kind: &str) -> Result<()> {
        use crate::credential::{backend, to_storage_error};
        backend(&self.root)
            .delete(id.0, kind)
            .map_err(|e| to_storage_error(&e))
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

    /// 读取所有 ER 作用域的视图状态（按 scope key → 状态），用于重开/重启恢复
    /// 分组、固定与坐标（§十/er-design §5.7 ErView 持久化）。缺失返回空。
    pub fn load_er_view_states(
        &self,
    ) -> Result<std::collections::BTreeMap<String, ErViewScopeState>> {
        let conn = self.open_sqlite()?;
        Ok(
            sqlite::get_json::<std::collections::BTreeMap<String, ErViewScopeState>>(
                &conn,
                sqlite::KEY_ER_VIEWS,
            )?
            .unwrap_or_default(),
        )
    }

    /// 全量写回 ER 作用域视图状态（幂等 upsert，JSON 化存 kv）。
    pub fn save_er_view_states(
        &self,
        states: &std::collections::BTreeMap<String, ErViewScopeState>,
    ) -> Result<()> {
        let conn = self.open_sqlite()?;
        sqlite::put_json(&conn, sqlite::KEY_ER_VIEWS, states)
    }

    /// 读取某 ER 作用域的本地逻辑关系目录（§5 D1-D9，§7）。缺失返回空。
    pub fn load_er_relationships(&self, scope_key: &str) -> Result<Vec<ErRelationship>> {
        let conn = self.open_sqlite()?;
        sqlite::get_json::<Vec<ErRelationship>>(&conn, &format!("{ER_REL_KEY_PREFIX}{scope_key}"))
            .map(|v| v.unwrap_or_default())
    }

    /// 全量写回某 ER 作用域的本地逻辑关系目录（幂等 upsert）。
    pub fn save_er_relationships(
        &self,
        scope_key: &str,
        relationships: &[ErRelationship],
    ) -> Result<()> {
        let conn = self.open_sqlite()?;
        sqlite::put_json(
            &conn,
            &format!("{ER_REL_KEY_PREFIX}{scope_key}"),
            &relationships.to_vec(),
        )
    }

    /// 读取某 ER 作用域的结构快照（§5.2 刷新重绑用）；缺失返回空。
    pub fn load_er_structure_snapshot(&self, scope_key: &str) -> Result<Vec<ErRebindEntity>> {
        let conn = self.open_sqlite()?;
        sqlite::get_json::<Vec<ErRebindEntity>>(
            &conn,
            &format!("{ER_SNAPSHOT_KEY_PREFIX}{scope_key}"),
        )
        .map(|v| v.unwrap_or_default())
    }

    /// 全量写回某 ER 作用域的结构快照（幂等 upsert；连接变更/换库按 scope key 分隔，不串）。
    pub fn save_er_structure_snapshot(
        &self,
        scope_key: &str,
        entities: &[ErRebindEntity],
    ) -> Result<()> {
        let conn = self.open_sqlite()?;
        sqlite::put_json(
            &conn,
            &format!("{ER_SNAPSHOT_KEY_PREFIX}{scope_key}"),
            &entities.to_vec(),
        )
    }

    /// 加载全部 SQL 查询历史（全量扁平，由上层按作用域过滤）。
    pub fn load_query_history(&self) -> Result<Vec<QueryHistoryRecord>> {
        let conn = self.open_sqlite()?;
        history_store::load_category(&conn, history_store::CATEGORY_QUERY)?
            .into_iter()
            .map(history_store::query_from_row)
            .collect()
    }

    /// 全量替换 SQL 查询历史（整表覆盖该类别的所有作用域，与旧的 blob 覆盖语义一致）。
    pub fn save_query_history(&self, entries: &[QueryHistoryRecord]) -> Result<()> {
        let rows = entries
            .iter()
            .map(history_store::query_to_row)
            .collect::<Result<Vec<_>>>()?;
        let conn = self.open_sqlite()?;
        let tx = conn.unchecked_transaction().map_err(storage_error)?;
        history_store::replace_category(&tx, history_store::CATEGORY_QUERY, &rows)?;
        tx.commit().map_err(storage_error)
    }

    /// 加载全部 Redis Key 搜索历史（全量扁平，由上层按作用域过滤）。
    pub fn load_redis_key_search_history(&self) -> Result<Vec<RedisKeySearchHistoryRecord>> {
        let conn = self.open_sqlite()?;
        Ok(
            history_store::load_category(&conn, history_store::CATEGORY_REDIS_KEY_SEARCH)?
                .into_iter()
                .map(history_store::redis_key_search_from_row)
                .collect(),
        )
    }

    /// 全量替换 Redis Key 搜索历史。
    pub fn save_redis_key_search_history(
        &self,
        entries: &[RedisKeySearchHistoryRecord],
    ) -> Result<()> {
        let rows = entries
            .iter()
            .map(history_store::redis_key_search_to_row)
            .collect::<Vec<_>>();
        let conn = self.open_sqlite()?;
        let tx = conn.unchecked_transaction().map_err(storage_error)?;
        history_store::replace_category(&tx, history_store::CATEGORY_REDIS_KEY_SEARCH, &rows)?;
        tx.commit().map_err(storage_error)
    }

    /// 加载所有连接 / 库的 Redis Workbench 命令历史（全量扁平，
    /// 由上层按 scope 过滤）。不存在时返回空列表。
    pub fn load_redis_workbench_history(&self) -> Result<Vec<RedisWorkbenchHistoryRecord>> {
        let conn = self.open_sqlite()?;
        history_store::load_category(&conn, history_store::CATEGORY_REDIS_COMMAND)?
            .into_iter()
            .map(history_store::redis_command_from_row)
            .collect()
    }

    /// 全量替换 Redis Workbench 命令历史（每个作用域保留最近
    /// [`history_store::PER_SCOPE_LIMIT`] 条，由存储层裁剪）。
    pub fn save_redis_workbench_history(
        &self,
        entries: &[RedisWorkbenchHistoryRecord],
    ) -> Result<()> {
        let rows = entries
            .iter()
            .map(history_store::redis_command_to_row)
            .collect::<Vec<_>>();
        let conn = self.open_sqlite()?;
        let tx = conn.unchecked_transaction().map_err(storage_error)?;
        history_store::replace_category(&tx, history_store::CATEGORY_REDIS_COMMAND, &rows)?;
        tx.commit().map_err(storage_error)
    }

    /// 加载全部备份记录（全量扁平，由上层按连接/库过滤）。不存在时返回空列表。
    pub fn load_backup_records(&self) -> Result<Vec<BackupRecord>> {
        let conn = self.open_sqlite()?;
        Ok(
            sqlite::get_json::<Vec<BackupRecord>>(&conn, sqlite::KEY_BACKUP_RECORDS)?
                .unwrap_or_default(),
        )
    }

    /// 全量保存备份记录（单条 key，整个数组一个 JSON blob，与其它 kv 数据一致）。
    pub fn save_backup_records(&self, records: &[BackupRecord]) -> Result<()> {
        let conn = self.open_sqlite()?;
        sqlite::put_json(&conn, sqlite::KEY_BACKUP_RECORDS, &records.to_vec())
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
        ensure_private_dir(&self.root)?;
        let mut document = toml::Value::try_from(settings).map_err(storage_error)?;
        // connection_secret_key 只允许直接编辑 config.toml；设置 UI 的保存不得擦除密钥。
        match fs::read_to_string(self.config_path()) {
            Ok(current) => {
                let current: toml::Value = toml::from_str(&current).map_err(storage_error)?;
                if let Some(key) = current.get("connection_secret_key") {
                    document
                        .as_table_mut()
                        .ok_or_else(|| storage_error("Settings TOML 不是表"))?
                        .insert("connection_secret_key".into(), key.clone());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(storage_error(e)),
        }
        let text = toml::to_string_pretty(&document).map_err(storage_error)?;
        fs::write(self.config_path(), text).map_err(storage_error)?;
        // 配置文件收敛为 0o600（方案 §12.2；含存量旧文件）。
        harden_file_perms(&self.config_path())
    }

    fn load_connections(&self) -> Result<Vec<ConnectionConfig>> {
        let conn = self.open_sqlite()?;
        connection_store::load_all(&conn)?
            .into_iter()
            .map(|connection| self.load_connection_secret(connection))
            .collect()
    }

    fn save_connections(&self, connections: &[ConnectionConfig]) -> Result<()> {
        // 方案 §4.2 / §10.4-1：凭据表与连接表使用不同连接，不能共享事务，采用"暂存-提交-切换"：
        // 1) 把整套新凭据写入临时 staging 键（验证可写，失败只删 staging，正式旧值不动）；
        // 2) 读取正式键旧值后切换正式凭据；任一写入失败则恢复全部旧值；
        // 3) 原子提交 SQLite 配置；提交失败同样恢复正式凭据，最后清理 staging。
        // 由此保证进程内任一阶段失败都不会留下“新配置 + 部分新密码”的状态。
        let mut staged: Vec<(ConnectionId, String)> = Vec::new(); // 已写入的 staging 键
        let mut pending: Vec<(ConnectionId, String, String)> = Vec::new(); // (正式键, 新值)
        for connection in connections {
            match self.stage_connection_secret(connection, &mut staged, &mut pending) {
                Ok(()) => {}
                Err(e) => {
                    self.cleanup_staged_credentials(&staged);
                    return Err(e);
                }
            }
        }

        let conn = match self.open_sqlite() {
            Ok(conn) => conn,
            Err(error) => {
                self.cleanup_staged_credentials(&staged);
                return Err(error);
            }
        };
        let mut layout = match sqlite::get_json::<SidebarLayout>(&conn, sqlite::KEY_SIDEBAR_LAYOUT)
        {
            Ok(layout) => layout.unwrap_or_else(|| SidebarLayout::for_connections(connections)),
            Err(error) => {
                self.cleanup_staged_credentials(&staged);
                return Err(error);
            }
        };
        layout.repair(connections);
        let mut old_credentials = Vec::with_capacity(pending.len());
        for (id, kind, _) in &pending {
            match self.secret_backend_read(*id, kind) {
                Ok(value) => old_credentials.push((*id, kind.clone(), value)),
                Err(error) => {
                    self.cleanup_staged_credentials(&staged);
                    return Err(error);
                }
            }
        }

        for (id, kind, value) in &pending {
            if let Err(write_error) = self.secret_backend_write(*id, kind, value) {
                let rollback_errors = self.restore_credentials(&old_credentials);
                self.cleanup_staged_credentials(&staged);
                return Err(credential_switch_error(write_error, rollback_errors));
            }
        }

        // connections 与 layout 在同一 SQLite 事务中提交；失败时恢复正式凭据。
        if let Err(e) = self.write_connections_and_layout(&conn, connections, &layout) {
            let rollback_errors = self.restore_credentials(&old_credentials);
            self.cleanup_staged_credentials(&staged);
            return Err(credential_switch_error(e, rollback_errors));
        }
        self.cleanup_staged_credentials(&staged);
        Ok(())
    }

    fn delete_connection_secrets(&self, connection: &ConnectionConfig) {
        self.delete_owned_secrets(connection);
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
    fn load_connection_secret(&self, mut connection: ConnectionConfig) -> Result<ConnectionConfig> {
        use connection_secrets::{DATABASE_PASSWORD, ENDPOINT_URI, URL_PARAMS};
        let id = connection.id;
        // 新格式只依赖连接 ID 和类别；旧格式的 SecretRef.key 仅作一次性迁移提示。
        if let fluxdb_core::Endpoint::Uri { uri } = &mut connection.endpoint {
            if uri.is_empty() {
                if let Some(secret) = self.best_effort_secret_read(id, ENDPOINT_URI) {
                    *uri = secret;
                }
            }
        }
        if !connection.options.contains_key("url_params") {
            if let Some(value) = self.best_effort_secret_read(id, URL_PARAMS) {
                connection.options.insert("url_params".into(), value);
            }
        }
        if !connection.options.contains_key(PLAINTEXT_PASSWORD_OPTION) {
            if let Some(secret) = self.best_effort_secret_read(id, DATABASE_PASSWORD) {
                connection
                    .options
                    .insert(PLAINTEXT_PASSWORD_OPTION.into(), secret);
            }
        }
        if let Some(profile) = connection.redis_profile.as_mut() {
            for (kind, slot) in profile_secret_slots_mut(profile) {
                if slot.inline.is_none() {
                    slot.inline = self.best_effort_secret_read(id, kind);
                }
                slot.key.clear();
            }
        }
        if let Some(profile) = connection.mysql_profile.as_mut() {
            for (kind, slot) in mysql_profile_secret_slots_mut(profile) {
                if slot.inline.is_none() {
                    slot.inline = self.best_effort_secret_read(id, kind);
                }
                slot.key.clear();
            }
        }
        if let Some(profile) = connection.postgres_profile.as_mut() {
            for (kind, slot) in postgres_profile_secret_slots_mut(profile) {
                if slot.inline.is_none() {
                    slot.inline = self.best_effort_secret_read(id, kind);
                }
                slot.key.clear();
            }
        }
        Ok(connection)
    }

    fn stage_connection_secret(
        &self,
        connection: &ConnectionConfig,
        staged: &mut Vec<(ConnectionId, String)>,
        pending: &mut Vec<(ConnectionId, String, String)>,
    ) -> Result<()> {
        use connection_secrets::{DATABASE_PASSWORD, ENDPOINT_URI, URL_PARAMS};
        let id = connection.id;
        let mut batch: Vec<(String, String)> = Vec::new();
        if let fluxdb_core::Endpoint::Uri { uri } = &connection.endpoint {
            batch.push((ENDPOINT_URI.into(), uri.clone()));
        }
        if let Some(params) = connection.options.get("url_params") {
            batch.push((URL_PARAMS.into(), params.clone()));
        }
        if let Some(password) = connection.options.get(PLAINTEXT_PASSWORD_OPTION) {
            batch.push((DATABASE_PASSWORD.into(), password.clone()));
        }
        for (kind, slot) in profile_secret_slots_combined(connection) {
            if let Some(value) = slot.inline.as_deref() {
                batch.push((kind.into(), value.into()));
            }
        }
        let mut written: Vec<(ConnectionId, String)> = Vec::new();
        for (kind, value) in &batch {
            let temp_kind = staging_kind(kind);
            if let Err(error) = self.secret_backend_write(id, &temp_kind, value) {
                for (staged_id, staged_kind) in &written {
                    self.secret_backend_delete(*staged_id, staged_kind);
                }
                return Err(error);
            }
            written.push((id, temp_kind));
        }
        staged.extend(written);
        pending.extend(batch.into_iter().map(|(kind, value)| (id, kind, value)));
        Ok(())
    }

    fn cleanup_staged_credentials(&self, staged: &[(ConnectionId, String)]) {
        for (id, kind) in staged {
            self.secret_backend_delete(*id, kind);
        }
    }

    fn restore_credentials(
        &self,
        old_credentials: &[(ConnectionId, String, Option<String>)],
    ) -> Vec<String> {
        let mut errors = Vec::new();
        for (id, kind, old_value) in old_credentials {
            let result = match old_value {
                Some(value) => self.secret_backend_write(*id, kind, value),
                None => self.secret_backend_delete_checked(*id, kind),
            };
            if let Err(error) = result {
                tracing::error!(target: "fluxdb_storage", %error, connection_id = id.0, kind, "恢复连接凭据失败");
                errors.push(format!("{}:{kind}: {error}", id.0));
            }
        }
        errors
    }

    fn delete_owned_secrets(&self, connection: &ConnectionConfig) {
        use connection_secrets::{
            DATABASE_PASSWORD, ENDPOINT_URI, PROXY_PASSWORD, SSH_PASSPHRASE, SSH_PASSWORD,
            URL_PARAMS,
        };
        for kind in [
            DATABASE_PASSWORD,
            SSH_PASSWORD,
            SSH_PASSPHRASE,
            PROXY_PASSWORD,
            ENDPOINT_URI,
            URL_PARAMS,
        ] {
            self.secret_backend_delete(connection.id, kind);
        }
    }

    /// 打开本 root 下的 sqlite 数据库并确保 kv 表存在。
    fn open_sqlite(&self) -> Result<rusqlite::Connection> {
        let conn = sqlite::open(&self.root)?;
        sqlite::create_schema(&conn).map(|_| ())?;
        Ok(conn)
    }

    /// 把连接列表（剥离明文）与 SidebarLayout 一并写入 sqlite。
    ///
    /// 连接进 `connections` 表（整表替换），布局仍是 kv JSON：布局是纯 UI 状态，
    /// 没有按字段查询需求，两者一起提交以保证「保存连接」与「保存布局」不互相覆盖。
    /// 同一事务内清理已删除连接的历史（级联）。
    fn write_connections_and_layout(
        &self,
        conn: &rusqlite::Connection,
        connections: &[ConnectionConfig],
        layout: &SidebarLayout,
    ) -> Result<()> {
        // 测试注入：模拟"凭据已写但配置提交失败"。仅测试/ test-util 下有效，生产恒 false。
        if crate::credential::consume_commit_failure() {
            return Err(Error::new(
                ErrorKind::Internal,
                "注入：配置提交失败（测试）",
            ));
        }
        let stripped: Vec<ConnectionConfig> =
            connections.iter().map(strip_plaintext_secrets).collect();
        let tx = conn.unchecked_transaction().map_err(storage_error)?;
        connection_store::replace_all(&tx, &stripped)?;
        history_store::delete_history_of_missing_connections(
            &tx,
            &stripped
                .iter()
                .map(|connection| connection.id)
                .collect::<Vec<_>>(),
        )?;
        sqlite::put_json(&tx, sqlite::KEY_SIDEBAR_LAYOUT, layout)?;
        tx.commit().map_err(storage_error)?;
        Ok(())
    }
}

include!("parts/backup_restore.rs");

fn storage_error(error: impl ToString) -> Error {
    Error::new(ErrorKind::Internal, error.to_string())
}

/// 单实例锁文件路径（方案 §12.1，方案 A）：
/// - Linux：优先 `$XDG_RUNTIME_DIR/fluxdb.lock`（tmpfs 运行时目录）
/// - macOS/Windows：持久化根目录下 `fluxdb.lock`
/// 锁本体由 flock（Unix）/ 命名互斥量（Windows）持有，文件只是锚点；
/// 进程退出（含强杀）时锁自动释放，不存在"崩溃后无法启动"的陈旧锁问题。
pub fn runtime_lock_file() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            return runtime_dir.join("fluxdb.lock");
        }
    }
    FileStorage::default_root()
        .unwrap_or_else(|_| std::env::temp_dir().join("fluxdb"))
        .join("fluxdb.lock")
}

/// 创建仅属主可访问的目录（Unix 0o700，创建后立即收紧、不依赖 umask；Windows 为 no-op，
/// ACL 收敛由方案 §12.2 后续 Windows 实测处理）。敏感目录（持久化根目录）统一走此函数。
pub(crate) fn ensure_private_dir(path: &std::path::Path) -> Result<()> {
    fs::create_dir_all(path).map_err(storage_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(storage_error)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// 将已存在的文件权限收紧为仅属主可读写（Unix 0o600；Windows no-op）。
/// 用于配置文件（历史上可能含明文 password option）与 SQLite 数据库。
pub(crate) fn harden_file_perms(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(storage_error)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn read_toml_file<T: DeserializeOwned>(path: &PathBuf) -> Result<T> {
    let text = fs::read_to_string(path).map_err(storage_error)?;
    toml::from_str::<T>(&text).map_err(storage_error)
}

fn write_toml_file_if_changed<T: Serialize>(path: &PathBuf, value: &T) -> Result<()> {
    let text = toml::to_string_pretty(value).map_err(storage_error)?;
    let existed = path.exists();
    if existed && fs::read_to_string(path).map_err(storage_error)? == text {
        // 内容未变也要收敛存量文件权限（升级场景：旧版本可能以宽松权限创建）。
        harden_file_perms(path)?;
        return Ok(());
    }
    fs::write(path, text).map_err(storage_error)?;
    // 配置/历史文件含业务数据，创建与覆写后均收紧为 0o600（方案 §12.2）。
    harden_file_perms(path)?;
    Ok(())
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

/// 单个 ER 作用域的视图状态（§十/§5.7）：分组、固定表、坐标、视口。与关系模型分离——
/// 只存视图层，不为每个局部图复制关系目录。坐标是逻辑像素（f32），finite，无 NaN。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ErViewScopeState {
    /// 当前分组（schema 过滤）；None=全部。
    pub group: Option<String>,
    /// 用户定义的业务分组：组名 -> 表展示名；旧视图文件缺失时兼容为空。
    #[serde(default)]
    pub custom_groups: std::collections::BTreeMap<String, Vec<String>>,
    /// 手动拖动过（固定）的表展示名。
    pub pinned: Vec<String>,
    /// 各表世界坐标（展示名, x, y）。仅存已定位表，缺失表重开按布局回退。
    pub positions: Vec<(String, f32, f32)>,
    /// 视口（pan_x, pan_y, scale）；None=未保存过，打开时走首次适配。
    /// 视图变换缩放下卡片固定屏幕尺寸，坐标乘 scale —— 保存原视口使重启后平移/缩放一致。
    pub view_port: Option<ErViewportState>,
}

/// 序列化的视口状态：平移 + 缩放。独立于 desktop 的 `ErViewport`（本 crate 不依赖 UI 类型）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErViewportState {
    pub pan_x: f32,
    pub pan_y: f32,
    pub scale: f32,
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

fn staging_kind(kind: &str) -> String {
    format!("__staging__:{kind}")
}

fn strip_plaintext_secrets(connection: &ConnectionConfig) -> ConnectionConfig {
    let mut connection = connection.clone();
    // 自由输入的 URL 参数也可能包含密码或令牌，整字段加密而非尝试猜测键名。
    connection.options.remove("url_params");
    if let fluxdb_core::Endpoint::Uri { uri } = &mut connection.endpoint {
        uri.clear(); // 连接 URI 整体存入 AES-GCM 密文表，避免 userinfo/query 泄露密码。
    }
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

/// 合并 Redis/MySQL/PostgreSQL 的密码槽位，返回 `(secret_kind, SecretRef)`。
fn profile_secret_slots_combined(connection: &ConnectionConfig) -> Vec<(&'static str, &SecretRef)> {
    let mut slots: Vec<(&'static str, &SecretRef)> = Vec::new();
    if let Some(profile) = connection.redis_profile.as_ref() {
        slots.extend(profile_secret_slots(profile));
    }
    if let Some(profile) = connection.mysql_profile.as_ref() {
        slots.extend(mysql_profile_secret_slots(profile));
    }
    if let Some(profile) = connection.postgres_profile.as_ref() {
        slots.extend(postgres_profile_secret_slots(profile));
    }
    slots
}

fn credential_switch_error(error: Error, rollback_errors: Vec<String>) -> Error {
    if rollback_errors.is_empty() {
        return error;
    }
    Error::new(
        ErrorKind::Internal,
        format!(
            "{error}；恢复旧凭据时仍有失败：{}",
            rollback_errors.join("；")
        ),
    )
}

fn profile_secret_slots(profile: &RedisConnectionProfile) -> Vec<(&'static str, &SecretRef)> {
    let mut slots: Vec<(&'static str, &SecretRef)> = vec![(
        connection_secrets::DATABASE_PASSWORD,
        &profile.basic.password,
    )];
    if profile.ssh.enabled {
        slots.push((connection_secrets::SSH_PASSWORD, &profile.ssh.password));
        slots.push((connection_secrets::SSH_PASSPHRASE, &profile.ssh.passphrase));
    }
    slots
}

/// 可变版，供回填 / 剥离时原地改写槽位。
fn profile_secret_slots_mut(
    profile: &mut RedisConnectionProfile,
) -> Vec<(&'static str, &mut SecretRef)> {
    let mut slots: Vec<(&'static str, &mut SecretRef)> = vec![(
        connection_secrets::DATABASE_PASSWORD,
        &mut profile.basic.password,
    )];
    if profile.ssh.enabled {
        slots.push((connection_secrets::SSH_PASSWORD, &mut profile.ssh.password));
        slots.push((
            connection_secrets::SSH_PASSPHRASE,
            &mut profile.ssh.passphrase,
        ));
    }
    slots
}

/// 遍历 MySQL 档案中需要走 SQLite 加密凭据表的「密码类」槽位。
/// 返回 `(secret_kind, 该槽的 SecretRef)`。
///
/// 证书/私钥文件一律以文件路径引用（`key` 存路径，非密码语义），不在此列；
/// 只有真正的密码（基础密码、SSH 密码、SSH 私钥口令、代理密码）才进 SQLite 加密凭据表。
fn mysql_profile_secret_slots(profile: &MysqlConnectionProfile) -> Vec<(&'static str, &SecretRef)> {
    let mut slots: Vec<(&'static str, &SecretRef)> = vec![(
        connection_secrets::DATABASE_PASSWORD,
        &profile.basic.password,
    )];
    for layer in &profile.transport {
        match layer {
            MysqlTransportLayer::Ssh(ssh) if ssh.enabled => {
                slots.push((connection_secrets::SSH_PASSWORD, &ssh.password));
                slots.push((connection_secrets::SSH_PASSPHRASE, &ssh.passphrase));
            }
            MysqlTransportLayer::Proxy(proxy) if proxy.enabled => {
                slots.push((connection_secrets::PROXY_PASSWORD, &proxy.password));
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
    let mut slots: Vec<(&'static str, &mut SecretRef)> =
        vec![(connection_secrets::DATABASE_PASSWORD, &mut basic.password)];
    for layer in transport.iter_mut() {
        match layer {
            MysqlTransportLayer::Ssh(ssh) if ssh.enabled => {
                slots.push((connection_secrets::SSH_PASSWORD, &mut ssh.password));
                slots.push((connection_secrets::SSH_PASSPHRASE, &mut ssh.passphrase));
            }
            MysqlTransportLayer::Proxy(proxy) if proxy.enabled => {
                slots.push((connection_secrets::PROXY_PASSWORD, &mut proxy.password));
            }
            _ => {}
        }
    }
    slots
}

/// 遍历 PostgreSQL 档案中需要走 SQLite 加密凭据表的「密码类」槽位。
/// 返回 `(secret_kind, 该槽的 SecretRef)`。
///
/// 证书/私钥文件一律以文件路径引用（`key` 存路径，非密码语义），不在此列；
/// 只有真正的密码（基础密码、SSH 密码、SSH 私钥口令、代理密码）才进 SQLite 加密凭据表。
fn postgres_profile_secret_slots(
    profile: &PostgresConnectionProfile,
) -> Vec<(&'static str, &SecretRef)> {
    let mut slots: Vec<(&'static str, &SecretRef)> = vec![(
        connection_secrets::DATABASE_PASSWORD,
        &profile.basic.password,
    )];
    for layer in &profile.transport {
        match layer {
            PostgresTransportLayer::Ssh(ssh) if ssh.enabled => {
                slots.push((connection_secrets::SSH_PASSWORD, &ssh.password));
                slots.push((connection_secrets::SSH_PASSPHRASE, &ssh.passphrase));
            }
            PostgresTransportLayer::Proxy(proxy) if proxy.enabled => {
                slots.push((connection_secrets::PROXY_PASSWORD, &proxy.password));
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
    let mut slots: Vec<(&'static str, &mut SecretRef)> =
        vec![(connection_secrets::DATABASE_PASSWORD, &mut basic.password)];
    for layer in transport.iter_mut() {
        match layer {
            PostgresTransportLayer::Ssh(ssh) if ssh.enabled => {
                slots.push((connection_secrets::SSH_PASSWORD, &mut ssh.password));
                slots.push((connection_secrets::SSH_PASSPHRASE, &mut ssh.passphrase));
            }
            PostgresTransportLayer::Proxy(proxy) if proxy.enabled => {
                slots.push((connection_secrets::PROXY_PASSWORD, &mut proxy.password));
            }
            _ => {}
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    use fluxdb_core::{
        COMPLETION_INDEX_VERSION, ColumnRef, CompletionIndexMeta, CompletionIndexSnapshot,
        ConnectionGroup, ConnectionGroupId, ConnectionId, DatabaseKind, Endpoint, LogLevel,
        ObjectKind, OperationKind, SavedQuery, SidebarOrderEntry, TableFingerprint, TableRef,
        Theme,
    };

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    // ---- 线程隔离的 InMemoryBackend 驱动异常分支，不访问真实用户数据 ----

    use crate::credential::{
        InMemoryBackend, UnavailableBackend, clear_test_backend, set_test_backend,
    };
    use std::sync::Arc;

    /// 构造带扁平密码的 MySQL 连接。
    fn conn_with_password(id: u64, password: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(id),
            name: format!("conn-{id}"),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 3306,
                database: Some("app".to_string()),
            },
            options: BTreeMap::from([(
                PLAINTEXT_PASSWORD_OPTION.to_string(),
                password.to_string(),
            )]),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        }
    }

    // ① 移除假成功：后端写失败时 save_connections 必须返回 Err（不得 Ok(())=假装保存成功）。
    #[test]
    fn save_connection_secret_propagates_write_failure() {
        let backend = Arc::new(InMemoryBackend::new());
        backend.fail_writes_with_prefix("1:__staging__:");
        set_test_backend(backend.clone());
        let storage = FileStorage::new(unique_temp_dir());

        let conn = conn_with_password(1, "s3cret");
        let err = storage.save_connections(&[conn]).unwrap_err();
        assert!(!err.to_string().is_empty(), "写失败应返回非空错误");
        // 正式键不应被写入（失败发生在暂存阶段）。
        assert!(
            backend
                .peek(1, connection_secrets::DATABASE_PASSWORD)
                .is_none()
        );
        clear_test_backend();
    }

    // ② 配置提交失败：凭据已暂存、但 SQLite 提交失败 → 返回 Err，且正式凭据与配置均保持旧值。
    #[test]
    fn save_connections_rolls_back_credentials_when_config_commit_fails() {
        let backend = Arc::new(InMemoryBackend::new());
        set_test_backend(backend.clone());
        let storage = FileStorage::new(unique_temp_dir());

        // 先保存成功（写入正式凭据 + 配置）。
        let v1 = conn_with_password(1, "old-pw");
        storage.save_connections(&[v1.clone()]).unwrap();

        // 注入：下一次配置提交失败；再保存新密码。
        crate::credential::fail_next_commit();
        let v2 = conn_with_password(1, "new-pw");
        assert!(storage.save_connections(&[v2.clone()]).is_err());

        // 正式凭据仍是旧值（新值未被写入）。
        assert_eq!(
            backend
                .peek(1, connection_secrets::DATABASE_PASSWORD)
                .as_deref(),
            Some("old-pw")
        );
        // 暂存键无残留。
        assert!(backend.peek(1, "__staging__:database_password").is_none());
        // 配置仍为旧连接（静默失效被阻止，连接资料保留）。
        let loaded = storage.load_connections().unwrap();
        assert_eq!(loaded.len(), 1);
        clear_test_backend();
    }

    // ③ 多个密码槽位跨连接部分失败：第二连接暂存失败时，第一连接已写的暂存被清理，正式键均未写。
    #[test]
    fn save_connections_cleans_staging_on_multi_slot_failure() {
        let backend = Arc::new(InMemoryBackend::new());
        // 第二个连接的暂存写失败。
        backend.fail_writes_with_prefix("2:__staging__:");
        set_test_backend(backend.clone());
        let storage = FileStorage::new(unique_temp_dir());

        let c1 = conn_with_password(1, "pw1");
        let c2 = conn_with_password(2, "pw2");
        assert!(storage.save_connections(&[c1, c2]).is_err());

        // 两连接的正式键都未写；第一连接的暂存被清理。
        assert!(
            backend
                .peek(1, connection_secrets::DATABASE_PASSWORD)
                .is_none()
        );
        assert!(
            backend
                .peek(2, connection_secrets::DATABASE_PASSWORD)
                .is_none()
        );
        assert!(backend.peek(1, "__staging__:database_password").is_none());
        clear_test_backend();
    }

    // ④ 正式键切换中途失败：已覆盖的键恢复旧值，配置保持旧版本，staging 无残留。
    #[test]
    fn save_connections_restores_formal_credentials_when_switch_fails() {
        let backend = Arc::new(InMemoryBackend::new());
        set_test_backend(backend.clone());
        let storage = FileStorage::new(unique_temp_dir());

        let old1 = conn_with_password(1, "old-1");
        let old2 = conn_with_password(2, "old-2");
        storage
            .save_connections(&[old1.clone(), old2.clone()])
            .unwrap();

        // 第一项先写成新值，第二项失败一次；恢复阶段不再失败。
        backend.fail_next_write("2:database_password");
        let new1 = conn_with_password(1, "new-1");
        let new2 = conn_with_password(2, "new-2");
        assert!(storage.save_connections(&[new1, new2]).is_err());

        assert_eq!(
            backend
                .peek(1, connection_secrets::DATABASE_PASSWORD)
                .as_deref(),
            Some("old-1")
        );
        assert_eq!(
            backend
                .peek(2, connection_secrets::DATABASE_PASSWORD)
                .as_deref(),
            Some("old-2")
        );
        assert!(backend.peek(1, "__staging__:database_password").is_none());
        assert!(backend.peek(2, "__staging__:database_password").is_none());
        let loaded = storage.load_connections().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded[0]
                .options
                .get(PLAINTEXT_PASSWORD_OPTION)
                .map(String::as_str),
            Some("old-1")
        );
        clear_test_backend();
    }

    // ⑤ 读失败降级：凭据服务不可用（Unavailable）时，连接资料保留、不等于"没有连接"，
    //    密码不回填（可重输）；失败不被吞成假成功，也不覆盖原配置。
    #[test]
    fn load_connections_preserves_connections_when_credential_unavailable() {
        // 先写一条连接（含凭据写入）。
        set_test_backend(Arc::new(InMemoryBackend::new()));
        let storage = FileStorage::new(unique_temp_dir());
        storage
            .save_connections(&[conn_with_password(1, "pw")])
            .unwrap();

        // 切换为不可用后端，验证读仍返回该连接、且密码未回填。
        set_test_backend(Arc::new(UnavailableBackend));
        let loaded = storage.load_connections().unwrap();
        assert_eq!(loaded.len(), 1, "凭据服务不可用不应导致连接列表为空");
        assert!(
            loaded[0].options.get(PLAINTEXT_PASSWORD_OPTION).is_none()
                || loaded[0]
                    .options
                    .get(PLAINTEXT_PASSWORD_OPTION)
                    .map(String::as_str)
                    == Some(""),
            "凭据不可用时密码不应被回填"
        );
        clear_test_backend();
    }

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

    // 权限收敛（方案 §12.2）：root 0o700、config/db 0o600，存量宽松权限文件也必须被收紧。
    #[cfg(unix)]
    #[test]
    fn perms_are_tightened_for_root_config_and_db() {
        use std::os::unix::fs::PermissionsExt;
        let storage = FileStorage::new(unique_temp_dir());
        storage.save_settings(&Settings::default()).unwrap();

        let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&storage.root), 0o700, "root 应为 0o700");
        assert_eq!(mode(&storage.config_path()), 0o600, "config 应为 0o600");

        // 预置一个宽松权限的存量 config，再次保存应被收紧。
        fs::set_permissions(storage.config_path(), fs::Permissions::from_mode(0o644)).unwrap();
        storage.save_settings(&Settings::default()).unwrap();
        assert_eq!(mode(&storage.config_path()), 0o600, "存量 config 应被收紧");

        let conn = storage.open_sqlite().unwrap();
        drop(conn);
        assert_eq!(
            mode(&sqlite::db_path(&storage.root)),
            0o600,
            "db 应为 0o600"
        );
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

    #[test]
    fn secret_kinds_are_stable_and_delete_is_idempotent() {
        assert_eq!(connection_secrets::DATABASE_PASSWORD, "database_password");
        assert_eq!(connection_secrets::SSH_PASSWORD, "ssh_password");
        let connection = conn_with_password(9, "secret");
        let storage = FileStorage::new(unique_temp_dir());
        storage.save_connections(&[connection.clone()]).unwrap();
        storage.delete_connection_secrets(&connection);
        storage.delete_connection_secrets(&connection);
        assert!(
            storage.load_connections().unwrap()[0]
                .options
                .get("password")
                .is_none()
        );
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
    fn er_view_states_roundtrip_across_instances() {
        let dir = unique_temp_dir();
        let storage = FileStorage::new(&dir);
        let scope = "1:demo:customers".to_string();
        let mut states = std::collections::BTreeMap::new();
        states.insert(
            scope.clone(),
            ErViewScopeState {
                group: Some("public".to_string()),
                custom_groups: std::collections::BTreeMap::from([(
                    "核心业务".to_string(),
                    vec!["orders".to_string()],
                )]),
                pinned: vec!["orders".to_string()],
                positions: vec![
                    ("customers".to_string(), 10.0, 20.0),
                    ("orders".to_string(), 300.0, 20.0),
                ],
                view_port: Some(ErViewportState {
                    pan_x: -123.5,
                    pan_y: 88.25,
                    scale: 0.65,
                }),
            },
        );
        storage.save_er_view_states(&states).unwrap();

        // 新实例（模拟重启）读回一致。
        let reloaded = FileStorage::new(&dir).load_er_view_states().unwrap();
        assert_eq!(reloaded.get(&scope), states.get(&scope));
    }

    #[test]
    fn er_view_state_legacy_without_custom_groups_still_loads() {
        let legacy = r#"{"group":"public","pinned":[],"positions":[],"view_port":null}"#;
        let state: ErViewScopeState = serde_json::from_str(legacy).unwrap();
        assert!(state.custom_groups.is_empty());
        assert_eq!(state.group.as_deref(), Some("public"));
    }

    #[test]
    fn er_relationships_roundtrip_per_scope() {
        use fluxdb_core::{
            ErCardinality, ErCardinalityBasis, ErCardinalityBound, ErColumnPair, ErEnforcementKind,
            ErFilterOp, ErLiteral, ErMatchCardinality, ErRelationSide, ErRelationship,
            ErRelationshipEnforcement, ErRelationshipOrigin, ErRelationshipReview,
            ErRequiredFilter, ErReviewState, ErValidity, ErValidityState,
        };
        let dir = unique_temp_dir();
        let storage = FileStorage::new(&dir);
        let rel = ErRelationship {
            id: "r1".into(),
            revision: 1,
            left_entity: "e-orders".into(),
            right_entity: "e-customers".into(),
            role: "order_customer".into(),
            column_pairs: vec![ErColumnPair {
                left_column: "orders-customer_id".into(),
                right_column: "customers-id".into(),
            }],
            required_filters: vec![ErRequiredFilter {
                side: ErRelationSide::Right,
                column_id: "customers-is_deleted".into(),
                op: ErFilterOp::Eq,
                literal: ErLiteral::Int(0),
            }],
            match_cardinality: ErMatchCardinality {
                left_to_right: ErCardinality {
                    min: ErCardinalityBound::Zero,
                    max: ErCardinalityBound::One,
                },
                right_to_left: ErCardinality {
                    min: ErCardinalityBound::Zero,
                    max: ErCardinalityBound::Many,
                },
                basis: ErCardinalityBasis::UserAssertion,
            },
            origin: ErRelationshipOrigin::User,
            review: ErRelationshipReview {
                state: ErReviewState::Confirmed,
                confirmed_revision: Some(1),
                confirmed_by: Some("alice".into()),
            },
            enforcement: ErRelationshipEnforcement {
                kind: ErEnforcementKind::None,
                constraint_ref: None,
                enforced: None,
            },
            validity: ErValidity {
                state: ErValidityState::Current,
                reason: None,
            },
            description: Some("订单归属于客户".into()),
            evidence_refs: vec!["ev1".into()],
        };
        storage
            .save_er_relationships("conn:db:", &[rel.clone()])
            .unwrap();
        // 同一作用域读回一致；跨作用域不串数据。
        let loaded = storage.load_er_relationships("conn:db:").unwrap();
        assert_eq!(loaded, vec![rel]);
        assert!(
            storage
                .load_er_relationships("conn:other:")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn structure_snapshot_roundtrip_per_scope() {
        use fluxdb_core::ErRebindColumn;
        let storage = FileStorage::new(unique_temp_dir());
        let entities = vec![
            fluxdb_core::ErRebindEntity {
                entity_id: "db:pub:orders".into(),
                qualified_name: "public.orders".into(),
                stable_id: None, // 无稳定对象标识，只按限定名/列名重绑（§5.2 第2/3/4步）。
                columns: vec![ErRebindColumn {
                    column_id: "db:pub:orders::id".into(),
                    name: "id".into(),
                    stable_id: None,
                }],
            },
            fluxdb_core::ErRebindEntity {
                entity_id: "db:pub:customers".into(),
                qualified_name: "public.customers".into(),
                stable_id: None,
                columns: vec![
                    ErRebindColumn {
                        column_id: "db:pub:customers::id".into(),
                        name: "id".into(),
                        stable_id: None,
                    },
                    ErRebindColumn {
                        column_id: "db:pub:customers::is_deleted".into(),
                        name: "is_deleted".into(),
                        stable_id: None,
                    },
                ],
            },
        ];
        // 同一作用域读回一致；跨作用域不串。
        storage
            .save_er_structure_snapshot("conn:db:", &entities)
            .unwrap();
        assert_eq!(
            storage.load_er_structure_snapshot("conn:db:").unwrap(),
            entities
        );
        assert!(
            storage
                .load_er_structure_snapshot("conn:other:")
                .unwrap()
                .is_empty()
        );
    }

    /// 构造一条 SQL 查询历史记录（默认只读查询）。
    fn query_record(connection_id: u64, database: Option<&str>, text: &str) -> QueryHistoryRecord {
        QueryHistoryRecord {
            connection_id: ConnectionId(connection_id),
            database: database.map(str::to_string),
            schema: None,
            text: text.to_string(),
            tables: vec!["orders".to_string()],
            operation_kind: OperationKind::Query,
            success: true,
            executed_at_unix_secs: 1_700_000_000,
            object: None,
            rollback_sql: None,
            rollback_snapshot: None,
            transaction_state: None,
            message: None,
            returned_rows: 0,
            affected_rows: 0,
            elapsed_ms: 0,
        }
    }

    /// 构造一条不含凭据的连接（不触发本机凭据库写入）。
    fn plain_connection(id: u64, kind: DatabaseKind, endpoint: Endpoint) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(id),
            name: format!("conn-{id}"),
            kind,
            endpoint,
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        }
    }

    #[test]
    fn saves_loads_and_limits_query_history_per_scope() {
        let storage = FileStorage::new(unique_temp_dir());
        // 连接 7 超限 2 条，连接 8 只有 3 条：裁剪按作用域独立，互不影响。
        let mut entries = (0..1002)
            .map(|index| query_record(7, Some("shop"), &format!("select {index}")))
            .collect::<Vec<_>>();
        entries
            .extend((0..3).map(|index| query_record(8, Some("shop"), &format!("select {index}"))));

        storage.save_query_history(&entries).unwrap();

        let loaded = storage.load_query_history().unwrap();
        assert_eq!(
            loaded.len(),
            1003,
            "连接 7 裁剪到 1000 条，连接 8 的 3 条不受影响"
        );
        assert_eq!(loaded[0].text, "select 2", "丢弃连接 7 最早的两条");
        assert_eq!(loaded[999].text, "select 1001");
        assert_eq!(loaded[1000].text, "select 0", "连接 8 的记录完整保留");
    }

    #[test]
    fn query_history_round_trips_extra_fields_and_operation_kind() {
        let storage = FileStorage::new(unique_temp_dir());
        let mut record = query_record(7, Some("shop"), "update t set a=1");
        record.operation_kind = OperationKind::Write;
        record.schema = Some("public".to_string());
        record.tables = vec!["orders".to_string(), "items".to_string()];
        record.object = Some("orders".to_string());
        record.transaction_state = Some("committed".to_string());
        record.message = Some("1 行受影响".to_string());
        record.affected_rows = 1;

        storage.save_query_history(&[record.clone()]).unwrap();

        assert_eq!(storage.load_query_history().unwrap(), vec![record]);
    }

    /// 新版本写入的未知操作类别取值，旧版本读回按 `unknown` 兜底（不报错）。
    #[test]
    fn unknown_operation_kind_value_falls_back_to_unknown() {
        let storage = FileStorage::new(unique_temp_dir());
        storage
            .save_query_history(&[query_record(7, Some("shop"), "select 1")])
            .unwrap();
        let conn = crate::sqlite::open(&storage.root).unwrap();
        conn.execute("UPDATE history SET operation_kind = 'mystery'", [])
            .unwrap();

        let loaded = storage.load_query_history().unwrap();
        assert_eq!(loaded[0].operation_kind, OperationKind::Unknown);
    }

    /// 历史行的 `kind` 按连接解析成数据库类型；连接删除时其历史级联清理。
    #[test]
    fn history_kind_follows_connection_and_cascades_on_delete() {
        let storage = FileStorage::new(unique_temp_dir());
        let mysql = plain_connection(
            1,
            DatabaseKind::MySql,
            Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 3306,
                database: Some("app".to_string()),
            },
        );
        let sqlite = plain_connection(
            2,
            DatabaseKind::Sqlite,
            Endpoint::SqliteFile {
                path: "demo.db".into(),
                read_only: false,
            },
        );
        storage.save_connections(&[mysql, sqlite.clone()]).unwrap();
        storage
            .save_query_history(&[
                query_record(1, Some("app"), "select mysql"),
                query_record(2, None, "select sqlite"),
            ])
            .unwrap();

        assert_eq!(stored_history_kinds(&storage), vec!["mysql", "sqlite"]);

        // 连接 1 被删除（保存剩下的连接列表）→ 它的历史同事务清理。
        storage.save_connections(&[sqlite]).unwrap();

        let loaded = storage.load_query_history().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].connection_id, ConnectionId(2));
    }

    /// 连接表的定位列/通用列是结构体的投影：tcp / file / uri 三种定位等值往返。
    #[test]
    fn connections_round_trip_endpoint_columns_and_extra() {
        let storage = FileStorage::new(unique_temp_dir());
        let mut mysql = plain_connection(
            1,
            DatabaseKind::MySql,
            Endpoint::Tcp {
                host: "db.internal".to_string(),
                port: 3307,
                database: Some("app".to_string()),
            },
        );
        mysql.options.insert("ssl".to_string(), "true".to_string());
        let sqlite = plain_connection(
            2,
            DatabaseKind::Sqlite,
            Endpoint::SqliteFile {
                path: "/tmp/demo.db".into(),
                read_only: true,
            },
        );
        let mongo = plain_connection(
            3,
            DatabaseKind::MongoDb,
            Endpoint::Uri {
                uri: "mongodb://localhost:27017".to_string(),
            },
        );

        storage
            .save_connections(&[mysql.clone(), sqlite.clone(), mongo.clone()])
            .unwrap();

        assert_eq!(
            storage.load_connections().unwrap(),
            vec![mysql, sqlite, mongo]
        );
    }

    /// 读取 history 表里按写入顺序排列的数据库类型列。
    fn stored_history_kinds(storage: &FileStorage) -> Vec<String> {
        let conn = crate::sqlite::open(&storage.root).unwrap();
        let mut stmt = conn
            .prepare("SELECT kind FROM history ORDER BY id")
            .unwrap();
        let kinds = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        kinds
    }

    #[test]
    fn saves_and_loads_backup_records() {
        let storage = FileStorage::new(unique_temp_dir());
        let records = vec![
            BackupRecord {
                manifest: None,
                id: "bk-1".to_string(),
                connection_id: ConnectionId(4),
                database: "shop".to_string(),
                output_path: "/tmp/fluxdb/shop/shop_20260917.sql".to_string(),
                created_unix: 1_700_000_000,
                size: 2048,
                tables: Some(vec!["orders".to_string()]),
                include_views: false,
                note: "周备份".to_string(),
            },
            BackupRecord {
                manifest: None,
                id: "bk-2".to_string(),
                connection_id: ConnectionId(4),
                database: "shop".to_string(),
                output_path: "/tmp/fluxdb/shop/shop_20260910.sql".to_string(),
                created_unix: 1_690_000_000,
                size: 1024,
                tables: None,
                include_views: true,
                note: String::new(),
            },
        ];

        storage.save_backup_records(&records).unwrap();

        let loaded = storage.load_backup_records().unwrap();
        assert_eq!(loaded, records);
    }

    /// Key 搜索历史不携带时间：整表替换时沿用同键旧行的时间，避免每次落盘刷新时间。
    #[test]
    fn redis_key_search_history_keeps_first_seen_time_across_replaces() {
        let storage = FileStorage::new(unique_temp_dir());
        let entry = |text: &str| RedisKeySearchHistoryRecord {
            connection_id: ConnectionId(4),
            database: Some("0".to_string()),
            text: text.to_string(),
        };
        storage
            .save_redis_key_search_history(&[entry("user:*")])
            .unwrap();
        let first_seen = stored_key_search_time(&storage, "user:*");

        storage
            .save_redis_key_search_history(&[entry("user:*"), entry("order:*")])
            .unwrap();

        assert_eq!(
            stored_key_search_time(&storage, "user:*"),
            first_seen,
            "已有词应沿用首次落盘时间"
        );
        assert!(
            stored_key_search_time(&storage, "order:*") >= first_seen,
            "新词应记当前时间"
        );
    }

    /// 读取 Key 搜索历史某条文本的落盘时间（该类别通过接口读不到时间，测试直接查表）。
    fn stored_key_search_time(storage: &FileStorage, text: &str) -> u64 {
        let conn = crate::sqlite::open(&storage.root).unwrap();
        conn.query_row(
            "SELECT executed_at_unix_secs FROM history
              WHERE category = ?1 AND text = ?2",
            rusqlite::params![crate::history_store::CATEGORY_REDIS_KEY_SEARCH, text],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as u64
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
    fn settings_save_preserves_config_only_secret_key() {
        let storage = FileStorage::new(unique_temp_dir());
        let key = "ab".repeat(32);
        fs::create_dir_all(&storage.root).unwrap();
        fs::write(
            storage.config_path(),
            format!("connection_secret_key = \"{key}\"\n"),
        )
        .unwrap();
        let settings = Settings::default();
        storage.save_settings(&settings).unwrap();
        assert_eq!(storage.load_settings().unwrap(), settings);
        let document: toml::Value = fs::read_to_string(storage.config_path())
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            document
                .get("connection_secret_key")
                .and_then(toml::Value::as_str),
            Some(key.as_str())
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
        // 临时目录也使用加密凭据表，读取后完整回填内存。
        assert_eq!(loaded[0], connections[0]);
        assert_eq!(loaded[1], connections[1]);
        assert_eq!(loaded[2], connections[2]);
        // Redis 档案的非密钥字段应保留。
        let redis = loaded[3].redis_profile.as_ref().expect("redis profile");
        assert_eq!(redis.basic.host, "127.0.0.1");
        assert_eq!(redis.ssh.host, "bastion");
        assert_eq!(redis.basic.password.inline.as_deref(), Some("topsecret"));

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
    fn uri_credentials_are_encrypted_and_recovered_by_connection_id() {
        let storage = FileStorage::new(unique_temp_dir());
        let uri = "mongodb://alice:uri-secret@host/db?password=query-secret";
        let connection = ConnectionConfig {
            id: ConnectionId(31),
            name: "uri".into(),
            kind: DatabaseKind::MongoDb,
            endpoint: Endpoint::Uri { uri: uri.into() },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        };
        storage.save_connections(&[connection.clone()]).unwrap();
        let conn = storage.open_sqlite().unwrap();
        let disk_uri: String = conn
            .query_row("SELECT uri FROM connections WHERE id = 31", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(disk_uri.is_empty());
        assert_eq!(
            storage.load_connections().unwrap(),
            vec![connection.clone()]
        );
        storage.delete_connection_secrets(&connection);
        assert_eq!(
            storage.load_connections().unwrap()[0].endpoint,
            Endpoint::Uri { uri: String::new() }
        );
    }

    #[test]
    fn url_params_are_encrypted_and_restored() {
        let storage = FileStorage::new(unique_temp_dir());
        let mut connection = sample_connections()[0].clone();
        connection
            .options
            .insert("url_params".into(), "?password=very-sensitive".into());
        storage.save_connections(&[connection.clone()]).unwrap();
        let db = storage.open_sqlite().unwrap();
        let extra: String = db
            .query_row(
                "SELECT extra_json FROM connections WHERE id = ?1",
                [connection.id.0],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!extra.contains("very-sensitive"));
        assert_eq!(storage.load_connections().unwrap(), vec![connection]);
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
        assert_eq!(
            storage.load_connections().unwrap()[0]
                .options
                .get("password")
                .map(String::as_str),
            Some("do-not-save-this")
        );
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
            vec![
                connection_secrets::DATABASE_PASSWORD,
                connection_secrets::SSH_PASSWORD,
                connection_secrets::SSH_PASSPHRASE,
                connection_secrets::PROXY_PASSWORD
            ]
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
            vec![
                connection_secrets::DATABASE_PASSWORD,
                connection_secrets::SSH_PASSWORD,
                connection_secrets::SSH_PASSPHRASE,
                connection_secrets::PROXY_PASSWORD
            ]
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
            redis_record(0, 1, 0, "GET a"),
            redis_record(0, 1, 1, "GET b"),
            redis_record(0, 2, 0, "SET k v"),
        ];

        storage.save_redis_workbench_history(&entries).unwrap();

        // 记录 ID 由数据库按 rowid 分配（写入时忽略传入 id），其余字段等值往返。
        let loaded = storage.load_redis_workbench_history().unwrap();
        assert_eq!(
            loaded.iter().map(|record| record.id).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "ID 按写入顺序由 rowid 分配"
        );
        let normalized = loaded
            .into_iter()
            .map(|mut record| {
                record.id = 0;
                record
            })
            .collect::<Vec<_>>();
        assert_eq!(normalized, entries);
    }

    #[test]
    fn missing_redis_workbench_history_returns_empty() {
        let storage = FileStorage::new(unique_temp_dir());
        assert!(storage.load_redis_workbench_history().unwrap().is_empty());
    }

    #[test]
    fn save_redis_workbench_history_truncates_to_per_scope_limit() {
        let storage = FileStorage::new(unique_temp_dir());
        // 同一作用域写入 1005 条，只保留最近 1000 条（丢弃最早 5 条）。
        let entries: Vec<_> = (0..1005)
            .map(|i| redis_record(0, 1, 0, &format!("CMD {i}")))
            .collect();

        storage.save_redis_workbench_history(&entries).unwrap();

        let loaded = storage.load_redis_workbench_history().unwrap();
        assert_eq!(loaded.len(), 1000, "应裁剪到最近 1000 条");
        assert_eq!(loaded[0].text, "CMD 5", "最早的 5 条应被丢弃");
        assert_eq!(loaded[999].text, "CMD 1004", "最新一条应保留");
    }
}
