use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

mod credential;
/// SQLite 结构版本迁移：旧库升级、迁移前备份与失败关闭，见 [`migrations`]。
mod migrations;
mod sqlite;

/// 共享数据密钥加载（AES-256-GCM）：连接凭据与模型 Key 共用一把密钥。
#[path = "parts/secret_crypto.rs"]
pub(crate) mod secret_crypto;

/// AI 平台持久化：管理实体与运行记录的建表与结构校验（设计文档 §42）。
#[path = "parts/ai_platform/mod.rs"]
pub mod ai_platform;

/// SQLite 连接类型再导出：供不直接依赖 rusqlite 的适配层
/// 在 `with_ai_connection` 闭包外显式标注连接参数。
pub use rusqlite::Connection as AiConnection;

#[path = "parts/connection_secrets.rs"]
mod connection_secrets;

/// 连接持久化：`connections` 表（一条连接一行）的建表与 row↔结构体适配。
#[path = "parts/connection_store.rs"]
mod connection_store;

/// 历史持久化：`history` 表（所有数据库类型共一张表）的建表、记录类型与读写。
#[path = "parts/history_store.rs"]
mod history_store;

/// 进程实例标识（`FLUXDB_PROFILE`）：并行多开时隔离数据目录与单实例锁。
#[path = "parts/instance_profile.rs"]
mod instance_profile;

#[path = "parts/mcp_clients.rs"]
mod mcp_clients;
pub use mcp_clients::McpClientLaunch;

pub use history_store::{
    QueryHistoryRecord, RedisKeySearchHistoryRecord, RedisWorkbenchHistoryRecord,
};
pub use instance_profile::{PROFILE_ENV, instance_profile};

use fluxdb_core::{
    ColumnRef, CompletionIndexMeta, CompletionIndexSnapshot, ConnectionConfig, ConnectionId,
    ErRebindEntity, ErRelationship, Error, ErrorKind, Result, RoutineRef, SavedQuery, SecretRef,
    Settings, SidebarLayout, TableRef, TriggerRef,
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

    /// 存储根目录（密钥加载需要；UI 不直接使用）。
    pub fn root_dir(&self) -> &std::path::Path {
        &self.root
    }

    /// 在 AI 平台连接上执行一次操作（连接在本方法内打开并关闭）。
    pub fn with_ai_connection<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<T>,
    ) -> Result<T> {
        let conn = self.open_ai_connection()?;
        f(&conn)
    }

    /// 打开 AI 平台使用的 SQLite 连接（已建好全部 AI 表）。
    ///
    /// 每次调用新开连接，与其他存储入口保持一致；UI 不直接持有该连接。
    pub fn open_ai_connection(&self) -> Result<rusqlite::Connection> {
        let conn = sqlite::open(&self.root)?;
        sqlite::create_schema(&conn)?;
        Ok(conn)
    }

    /// 解析默认持久化根目录（目录策略统一在此定义，见方案 §4.1）：
    /// - macOS：`~/Library/Application Support/fluxdb`（与历史版本一致，旧数据不受影响）
    /// - Windows：Known Folder LocalAppData 下的 `FluxDB`（即 `%LOCALAPPDATA%/FluxDB`）。
    ///   注意 dirs::data_dir() 在 Windows 是 Roaming，必须用 data_local_dir()。
    /// - Linux：`$XDG_DATA_HOME/fluxdb`，缺省 `~/.local/share/fluxdb`
    ///
    /// 设置了实例标识（`FLUXDB_PROFILE`）时目录追加 `-标识` 后缀，使同一台机器上的
    /// 多个实例各自持有独立配置、连接、历史与凭据，可并行运行；未设置时布局不变。
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
        let default_name = if cfg!(target_os = "windows") {
            "FluxDB"
        } else {
            "fluxdb"
        };
        Ok(base.join(instance_profile::app_dir_name(default_name)?))
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
    /// 与 `default_root` 同样受实例标识影响，保证并行实例的日志互不混淆。
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
            // 标识非法时不阻断启动：退回默认目录名，真正的报错由 `default_root`
            // （启动路径必经）给出，避免日志系统未初始化时无处输出诊断。
            let app_dir =
                instance_profile::app_dir_name("fluxdb").unwrap_or_else(|_| "fluxdb".to_string());
            return state
                .map(|s| s.join(app_dir).join("logs"))
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

    pub fn load_completion_index(
        &self,
        connection: &ConnectionConfig,
        database: Option<&str>,
        schema: Option<&str>,
    ) -> Result<Option<CompletionIndexSnapshot>> {
        let dir = self.completion_index_dir(connection, database, schema);
        self.load_completion_index_dir(&dir, connection.id)
    }

    /// 读取一个连接下所有已落盘的补全索引快照。
    ///
    /// AI 找表不知道用户会问哪个库，需要一次合并该连接的全部作用域；
    /// 单个快照损坏只跳过该作用域，不阻断其余索引。
    pub fn load_completion_indexes_for_connection(
        &self,
        connection: &ConnectionConfig,
    ) -> Result<Vec<CompletionIndexSnapshot>> {
        let dir = self
            .completion_index_root()
            .join("connections")
            .join(connection_fingerprint(connection))
            .join("databases");
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(storage_error(error)),
        };
        let mut snapshots = Vec::new();
        for entry in entries {
            let entry = entry.map_err(storage_error)?;
            if !entry.file_type().map_err(storage_error)?.is_dir() {
                continue;
            }
            match self.load_completion_index_dir(&entry.path(), connection.id) {
                Ok(Some(snapshot)) => snapshots.push(snapshot),
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    target: "fluxdb_storage",
                    %error,
                    connection_id = connection.id.0,
                    path = %entry.path().display(),
                    "补全索引快照损坏，跳过该作用域"
                ),
            }
        }
        Ok(snapshots)
    }

    fn load_completion_index_dir(
        &self,
        dir: &std::path::Path,
        connection_id: ConnectionId,
    ) -> Result<Option<CompletionIndexSnapshot>> {
        let meta_path = dir.join("meta.toml");
        if !meta_path.exists() {
            return Ok(None);
        }
        let header = read_toml_file::<CompletionIndexHeader>(&meta_path)?;
        if header.connection_id != connection_id {
            return Ok(None);
        }
        let tables = read_toml_file::<CompletionIndexTables>(&dir.join("tables.toml"))?.tables;
        let columns = read_toml_file::<CompletionIndexColumns>(&dir.join("columns.toml"))?.columns;
        let routines =
            read_toml_file::<CompletionIndexRoutines>(&dir.join("routines.toml"))?.routines;
        let triggers =
            read_toml_file::<CompletionIndexTriggers>(&dir.join("triggers.toml"))?.triggers;
        Ok(Some(CompletionIndexSnapshot {
            connection_id: header.connection_id,
            database: header.database,
            schema: header.schema,
            tables,
            columns,
            routines,
            triggers,
            meta: header.meta,
        }))
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

    /// 加载任务中心历史记录（已结束任务）。不存在时返回空列表。
    pub fn load_task_history(&self) -> Result<Vec<TaskHistoryRecord>> {
        let conn = self.open_sqlite()?;
        Ok(
            sqlite::get_json::<Vec<TaskHistoryRecord>>(&conn, sqlite::KEY_TASK_HISTORY)?
                .unwrap_or_default(),
        )
    }

    /// 全量保存任务中心历史记录。
    pub fn save_task_history(&self, records: &[TaskHistoryRecord]) -> Result<()> {
        let conn = self.open_sqlite()?;
        sqlite::put_json(&conn, sqlite::KEY_TASK_HISTORY, &records.to_vec())
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
        // 数据密钥只允许直接编辑 config.toml；设置 UI 的保存不得擦除密钥。
        match fs::read_to_string(self.config_path()) {
            Ok(current) => {
                let current: toml::Value = toml::from_str(&current).map_err(storage_error)?;
                if let Some(key) = current.get(secret_crypto::CONFIG_KEY) {
                    document
                        .as_table_mut()
                        .ok_or_else(|| storage_error("Settings TOML 不是表"))?
                        .insert(secret_crypto::CONFIG_KEY.into(), key.clone());
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
        // 各数据库的凭据槽位统一走 `DatabaseConnection`，不再按类型写平行分支。
        if let Some(profile) = connection.connection_profile_mut() {
            for (slot_kind, slot) in profile.secret_slots_mut() {
                if slot.inline.is_none() {
                    slot.inline = self.best_effort_secret_read(id, secret_slot_kind(slot_kind));
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
            DATABASE_PASSWORD, ENDPOINT_URI, ORACLE_WALLET_PASSWORD, PROXY_PASSWORD,
            SSH_PASSPHRASE, SSH_PASSWORD, URL_PARAMS,
        };
        for kind in [
            DATABASE_PASSWORD,
            SSH_PASSWORD,
            SSH_PASSPHRASE,
            PROXY_PASSWORD,
            ENDPOINT_URI,
            URL_PARAMS,
            ORACLE_WALLET_PASSWORD,
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
include!("parts/task_history.rs");

fn storage_error(error: impl ToString) -> Error {
    Error::new(ErrorKind::Internal, error.to_string())
}

/// 单实例锁文件路径（方案 §12.1，方案 A）：
/// - Linux：优先 `$XDG_RUNTIME_DIR/fluxdb.lock`（tmpfs 运行时目录）
/// - macOS/Windows：持久化根目录下 `fluxdb.lock`
///
/// 设置实例标识（`FLUXDB_PROFILE`）时锁文件带 `-标识` 后缀，与数据目录一致：
/// 不同标识的实例互不阻塞，同一标识仍保持单实例保护。
///
/// 锁本体由 flock（Unix）/ 命名互斥量（Windows）持有，文件只是锚点；
/// 进程退出（含强杀）时锁自动释放，不存在"崩溃后无法启动"的陈旧锁问题。
pub fn runtime_lock_file() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            // 标识非法时退回默认锁名：至少保住单实例保护，非法标识本身由
            // `default_root`（启动路径必经）在解析数据目录时报错退出。
            let name = match instance_profile() {
                Ok(Some(profile)) => format!("fluxdb-{profile}.lock"),
                Ok(None) | Err(_) => "fluxdb.lock".to_string(),
            };
            return runtime_dir.join(name);
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
/// 收紧本地文件权限为 0600（Windows 无操作）。
///
/// 供其他 crate 写入本机凭据文件（如 MCP 访问令牌）时复用同一套权限策略。
pub fn harden_local_file_perms(path: &std::path::Path) -> Result<()> {
    harden_file_perms(path)
}

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
        !key.contains("password") && !key.contains("secret") && !key.contains("passphrase")
    });
    // 结构化档案里的受控值靠 `SecretRef.inline` 的 `#[serde(skip)]` 保证不落盘；
    // 这里再显式清空一份供盘上转储副本，双保险。
    if let Some(profile) = connection.redis_profile.as_mut() {
        // 导入来源元数据可能带明文口令（历史版本/其它导入路径），一律不落盘。
        profile.cloud.imported_name.clear();
    }
    // 全部结构化档案共用一套受控槽位遍历：清空受控值，保证盘上副本零明文。
    // 证书/私钥路径是引用键而非密钥正文，由 trait 排除在槽位之外。
    if let Some(profile) = connection.connection_profile_mut() {
        for (_, slot) in profile.secret_slots_mut() {
            slot.inline = None;
        }
    }
    connection
}

/// 合并结构化档案的密码槽位，返回 `(secret_kind, SecretRef)`。
fn profile_secret_slots_combined(connection: &ConnectionConfig) -> Vec<(&'static str, &SecretRef)> {
    connection
        .connection_profile()
        .map(|profile| {
            profile
                .secret_slots()
                .into_iter()
                .map(|(slot_kind, secret)| (secret_slot_kind(slot_kind), secret))
                .collect()
        })
        .unwrap_or_default()
}

/// `DatabaseConnection` 的语义槽位到凭据表键名的唯一映射。
fn secret_slot_kind(slot: fluxdb_core::ConnectionSecretSlot) -> &'static str {
    use fluxdb_core::ConnectionSecretSlot as Slot;
    match slot {
        Slot::DatabasePassword => connection_secrets::DATABASE_PASSWORD,
        Slot::SshPassword => connection_secrets::SSH_PASSWORD,
        Slot::SshPassphrase => connection_secrets::SSH_PASSPHRASE,
        Slot::ProxyPassword => connection_secrets::PROXY_PASSWORD,
        Slot::TlsClientKeyPassphrase => connection_secrets::TLS_CLIENT_KEY_PASSPHRASE,
        Slot::OracleWalletPassword => connection_secrets::ORACLE_WALLET_PASSWORD,
    }
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
