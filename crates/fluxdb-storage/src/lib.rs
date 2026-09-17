use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

mod credential;
mod sqlite;

use fluxdb_core::{
    ColumnRef, CompletionIndexMeta, CompletionIndexSnapshot, ConnectionConfig, ConnectionId, Error,
    ErrorKind, MysqlConnectionProfile, MysqlTransportLayer, PostgresConnectionProfile,
    PostgresTransportLayer, QueryRollbackSnapshot, RedisConnectionProfile, Result, RoutineRef,
    SavedQuery, SecretRef, Settings, SidebarLayout, TableRef, TriggerRef,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const PLAINTEXT_PASSWORD_OPTION: &str = "password";

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

    // ---- 系统凭据后端（方案 §4.2）----
    // 通过 credential::backend() 单例访问，保留非系统 root（测试）跳过系统凭据的语义。
    // 读/写失败传播错误（写不再假成功），删除对 NotFound 幂等容忍。

    fn secret_backend_read(&self, account: &str) -> Result<Option<String>> {
        if !self.should_use_credential_backend() {
            return Ok(None);
        }
        use crate::credential::{backend, to_storage_error};
        backend().read(account).map_err(|e| to_storage_error(&e))
    }

    fn secret_backend_write(&self, account: &str, secret: &str) -> Result<()> {
        if !self.should_use_credential_backend() {
            return Ok(());
        }
        use crate::credential::{backend, to_storage_error};
        backend()
            .write(account, secret)
            .map_err(|e| to_storage_error(&e))
    }

    fn best_effort_secret_read(&self, account: &str) -> Option<String> {
        match self.secret_backend_read(account) {
            Ok(v) => v,
            Err(e) => {
                // 凭据服务不可用/锁定：保留连接资料，不回填密码并记日志，避免把"无凭据服务"
                // 误处理成"没有连接"或覆盖原配置。UI 级可理解提示属界面层（AI-03）。
                tracing::warn!(target: "fluxdb_storage", error = %e, account, "读取系统凭据失败，连接保留但不回填密码");
                None
            }
        }
    }

    fn secret_backend_delete(&self, account: &str) {
        if !self.should_use_credential_backend() {
            return;
        }
        use crate::credential::{backend, to_storage_error};
        if let Err(e) = backend().delete(account) {
            // 删除幂等：NotFound 仍视为已删；其他错误记录日志（删除失败不阻断主流程）。
            let storage_err = to_storage_error(&e);
            if !matches!(e, crate::credential::CredentialError::NotFound) {
                tracing::warn!(target: "fluxdb_storage", error = %storage_err, account, "删除系统凭据失败");
            }
        }
    }

    /// 是否应访问系统凭据后端。测试注入 override 时绕过"非系统 root 短路"，
    /// 以便用隔离内存后端驱动凭据路径；否则按系统 root 判断（保持原有语义）。
    fn should_use_credential_backend(&self) -> bool {
        #[cfg(any(test, feature = "test-util"))]
        {
            if crate::credential::test_override_active() {
                return true;
            }
        }
        self.uses_system_keychain()
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
        ensure_private_dir(&self.root)?;
        let text = toml::to_string_pretty(settings).map_err(storage_error)?;
        fs::write(self.config_path(), text).map_err(storage_error)?;
        // 配置文件收敛为 0o600（方案 §12.2；含存量旧文件）。
        harden_file_perms(&self.config_path())
    }

    fn load_connections(&self) -> Result<Vec<ConnectionConfig>> {
        let conn = self.open_sqlite()?;
        sqlite::get_json::<Vec<ConnectionConfig>>(&conn, sqlite::KEY_CONNECTIONS)?
            .unwrap_or_default()
            .into_iter()
            .map(|connection| self.load_connection_secret(connection))
            .collect()
    }

    fn save_connections(&self, connections: &[ConnectionConfig]) -> Result<()> {
        // 方案 §4.2 / §10.4-1：系统凭据与 SQLite 配置不是同一事务，采用"暂存-提交-切换"：
        // 1) 把整套新凭据写入临时 staging 键（验证可写，失败只删 staging，正式旧值不动）；
        // 2) 提交 SQLite 配置（失败→删 staging 并报错，正式旧值+旧配置均完好）；
        // 3) 配置提交成功后，才把新值写入正式键（覆盖旧值），最后删 staging。
        // 由此保证：任意阶段失败都不误删其它连接引用的正式凭据；补偿只作用于 staging 前缀。
        let mut staged: Vec<String> = Vec::new(); // 已写入的 staging 键
        let mut pending: Vec<(String, String)> = Vec::new(); // (正式键, 新值)
        for connection in connections {
            match self.stage_connection_secret(connection, &mut staged, &mut pending) {
                Ok(()) => {}
                Err(e) => {
                    self.cleanup_staged_credentials(&staged);
                    return Err(e);
                }
            }
        }

        // 提交配置前先检查配置写入路径是否被注入失败（测试用）。配置提交失败不动正式凭据。
        let conn = self.open_sqlite()?;
        let mut layout = sqlite::get_json::<SidebarLayout>(&conn, sqlite::KEY_SIDEBAR_LAYOUT)?
            .unwrap_or_else(|| SidebarLayout::for_connections(connections));
        layout.repair(connections);
        if let Err(e) = self.write_connections_and_layout(&conn, connections, &layout) {
            self.cleanup_staged_credentials(&staged);
            return Err(e);
        }

        // 配置提交成功：把新值写入正式键。
        let mut commit_err: Option<String> = None;
        for (real, value) in &pending {
            if let Err(e) = self.secret_backend_write(real, value) {
                commit_err = Some(e.to_string());
            }
        }
        self.cleanup_staged_credentials(&staged);
        if let Some(msg) = commit_err {
            return Err(Error::new(ErrorKind::Internal, msg));
        }
        Ok(())
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
    fn load_connection_secret(&self, mut connection: ConnectionConfig) -> Result<ConnectionConfig> {
        let Some(credential_ref) = self.credential_ref_for_keychain(&connection) else {
            return Ok(connection);
        };

        // 扁平历史参数：`options["password"]` 从 Keychain 取回。
        if !connection.options.contains_key(PLAINTEXT_PASSWORD_OPTION) {
            if let Some(secret) = self.best_effort_secret_read(&credential_ref) {
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
                    slot.inline = self.best_effort_secret_read(&slot.key);
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
                    slot.inline = self.best_effort_secret_read(&slot.key);
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
                    slot.inline = self.best_effort_secret_read(&slot.key);
                }
            }
        }

        Ok(connection)
    }

    // 方案 §4.2 / §10.4-1 的"暂存-提交-切换"辅助：见 save_connections 编排注释。

    fn stage_connection_secret(
        &self,
        connection: &ConnectionConfig,
        staged: &mut Vec<String>,
        pending: &mut Vec<(String, String)>,
    ) -> Result<()> {
        let Some(credential_ref) = self.credential_ref_for_keychain(connection) else {
            return Ok(());
        };
        let mut batch: Vec<(String, String)> = Vec::new();
        if let Some(password) = connection.options.get(PLAINTEXT_PASSWORD_OPTION) {
            batch.push((credential_ref.clone(), password.clone()));
        }
        for (suffix, slot) in profile_secret_slots_combined(connection) {
            if let Some(value) = slot.inline.as_deref() {
                batch.push((
                    secret_slot_account(&credential_ref, suffix, slot),
                    value.to_string(),
                ));
            }
        }

        // 写 staging；任一失败，删本次已写 staging（正式旧值不动），报错。
        let mut written_staging: Vec<String> = Vec::new();
        for (real, value) in &batch {
            let key = staging_key(real);
            match self.secret_backend_write(&key, value) {
                Ok(()) => written_staging.push(key),
                Err(e) => {
                    for k in &written_staging {
                        self.secret_backend_delete(k);
                    }
                    return Err(e);
                }
            }
        }
        staged.extend(written_staging);
        pending.extend(batch);
        Ok(())
    }

    fn cleanup_staged_credentials(&self, staged: &[String]) {
        for key in staged {
            self.secret_backend_delete(key);
        }
    }

    fn delete_owned_keychain_secrets(&self, connection: &ConnectionConfig) {
        let Some(credential_ref) = self.credential_ref_for_keychain(connection) else {
            return;
        };
        // 扁平历史密码与结构化档案槽位同属该 ref。
        self.secret_backend_delete(&credential_ref);
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
            self.secret_backend_delete(&account);
        }
    }

    fn credential_ref_for_keychain(&self, connection: &ConnectionConfig) -> Option<String> {
        if !self.should_use_credential_backend() {
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
        // 测试注入：模拟"凭据已写但配置提交失败"。仅测试/ test-util 下有效，生产恒 false。
        if crate::credential::consume_commit_failure() {
            return Err(Error::new(
                ErrorKind::Internal,
                "注入：配置提交失败（测试）",
            ));
        }
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

/// 合并 Redis/MySQL/PostgreSQL 三栈的密码槽位，返回 `(后缀, SecretRef)` 列表。
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

/// 暂存（staging）凭据键名：与正式键一一对应，前缀唯一，绝不与正式/其它连接键冲突。
/// 提交成功后删除 staging；任何失败只清理 staging，正式旧值不受影响。
fn staging_key(real_account: &str) -> String {
    format!("__fluxdb_staging__/{real_account}")
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

    // ---- AI-02 系统凭据：用线程隔离的 InMemoryBackend 驱动，不访问真实密码库 ----

    use crate::credential::{
        InMemoryBackend, UnavailableBackend, clear_test_backend, set_test_backend,
    };
    use std::sync::Arc;

    /// 构造带扁平密码的 MySQL 连接。
    fn conn_with_password(id: u64, credential_ref: &str, password: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(id),
            name: format!("conn-{id}"),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 3306,
                database: Some("app".to_string()),
            },
            credential_ref: Some(credential_ref.to_string()),
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
        backend.fail_writes_with_prefix("__fluxdb_staging__/gdb.connection.1");
        set_test_backend(backend.clone());
        let storage = FileStorage::new(unique_temp_dir());

        let conn = conn_with_password(1, "gdb.connection.1", "s3cret");
        let err = storage.save_connections(&[conn]).unwrap_err();
        assert!(!err.to_string().is_empty(), "写失败应返回非空错误");
        // 正式键不应被写入（失败发生在暂存阶段）。
        assert!(backend.peek("gdb.connection.1").is_none());
        // 正式键不应被写入（失败发生在暂存阶段）。
        assert!(backend.peek("gdb.connection.1").is_none());
        clear_test_backend();
    }

    // ② 配置提交失败：凭据已暂存、但 SQLite 提交失败 → 返回 Err，且正式凭据与配置均保持旧值。
    #[test]
    fn save_connections_rolls_back_credentials_when_config_commit_fails() {
        let backend = Arc::new(InMemoryBackend::new());
        set_test_backend(backend.clone());
        let storage = FileStorage::new(unique_temp_dir());

        // 先保存成功（写入正式凭据 + 配置）。
        let v1 = conn_with_password(1, "gdb.connection.1", "old-pw");
        storage.save_connections(&[v1.clone()]).unwrap();

        // 注入：下一次配置提交失败；再保存新密码。
        crate::credential::fail_next_commit();
        let v2 = conn_with_password(1, "gdb.connection.1", "new-pw");
        assert!(storage.save_connections(&[v2.clone()]).is_err());

        // 正式凭据仍是旧值（新值未被写入）。
        assert_eq!(backend.peek("gdb.connection.1").as_deref(), Some("old-pw"));
        // 暂存键无残留。
        assert!(
            backend
                .peek("__fluxdb_staging__/gdb.connection.1")
                .is_none()
        );
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
        backend.fail_writes_with_prefix("__fluxdb_staging__/gdb.connection.2");
        set_test_backend(backend.clone());
        let storage = FileStorage::new(unique_temp_dir());

        let c1 = conn_with_password(1, "gdb.connection.1", "pw1");
        let c2 = conn_with_password(2, "gdb.connection.2", "pw2");
        assert!(storage.save_connections(&[c1, c2]).is_err());

        // 两连接的正式键都未写；第一连接的暂存被清理。
        assert!(backend.peek("gdb.connection.1").is_none());
        assert!(backend.peek("gdb.connection.2").is_none());
        assert!(
            backend
                .peek("__fluxdb_staging__/gdb.connection.1")
                .is_none()
        );
        clear_test_backend();
    }

    // ④ 读失败降级：凭据服务不可用（Unavailable）时，连接资料保留、不等于"没有连接"，
    //    密码不回填（可重输）；失败不被吞成假成功，也不覆盖原配置。
    #[test]
    fn load_connections_preserves_connections_when_credential_unavailable() {
        // 先写一条连接（含凭据写入）。
        set_test_backend(Arc::new(InMemoryBackend::new()));
        let storage = FileStorage::new(unique_temp_dir());
        storage
            .save_connections(&[conn_with_password(1, "gdb.connection.1", "pw")])
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
