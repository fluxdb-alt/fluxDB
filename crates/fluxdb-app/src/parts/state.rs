pub const HEX_EDIT_LIMIT: u64 = 64 * 1024;
pub const BINARY_FILE_UPLOAD_LIMIT: u64 = 64 * 1024 * 1024;
const COMPLETION_METADATA_LIMIT: u64 = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct AppState {
    pub connections: Vec<ConnectionState>,
    pub tabs: Vec<TabState>,
    pub active_tab: Option<TabId>,
    pub pending_dirty_tab_close: Option<TabId>,
    pub settings: Settings,
    pub sidebar_layout: SidebarLayout,
    pub tasks: Vec<TaskState>,
    pub query_history: Vec<QueryHistoryEntry>,
    /// Redis Workbench 命令历史（全量扁平，按连接 + 逻辑库过滤展示），持久化到
    /// `workbench-history.toml`。与 `query_history` 同层级：App 层写入、桌面层持久化。
    pub redis_workbench_history: Vec<RedisWorkbenchHistoryEntry>,
    /// 为 `redis_workbench_history` 分配历史记录 ID 的单调递增计数器。
    pub next_redis_workbench_history_id: u64,
    pub last_error: Option<UserFacingError>,
}

impl AppState {
    pub fn active_tab(&self) -> Option<&TabState> {
        self.active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            connections: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            pending_dirty_tab_close: None,
            settings: Settings::default(),
            sidebar_layout: SidebarLayout::default(),
            tasks: Vec::new(),
            query_history: Vec::new(),
            redis_workbench_history: Vec::new(),
            next_redis_workbench_history_id: 0,
            last_error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionState {
    pub config: ConnectionConfig,
    pub connected: bool,
    pub expanded: bool,
    pub objects: Vec<ObjectSummary>,
    /// Redis 连接级运行概览（版本 / 内存 / CPU），只在连接为 Redis 时有意义。
    pub redis_overview: RedisConnectionOverview,
}

/// Redis 连接级运行概览，供底栏连接状态摘要展示。
/// 参考 RedisInsight 数据库概览语义，与 key 详情无关。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisConnectionOverview {
    /// Redis 服务端版本，如 `7.2.4`。
    pub version: String,
    /// Redis 实例占用内存字节数。
    pub used_memory_bytes: u64,
    /// CPU 使用率百分比（两次采样增量计算）。尚无第二次采样（或服务端重启）时为 None。
    pub cpu_usage_percent: Option<f64>,
    /// 上一次采样的累计 CPU 秒数（sys+user），用于下一次增量推导缓存。
    prev_cpu_seconds: f64,
    /// 上一次采样时的 uptime 秒数。
    prev_uptime_seconds: f64,
}

impl Default for RedisConnectionOverview {
    fn default() -> Self {
        Self {
            version: String::new(),
            used_memory_bytes: 0,
            cpu_usage_percent: None,
            prev_cpu_seconds: 0.0,
            prev_uptime_seconds: 0.0,
        }
    }
}

impl RedisConnectionOverview {
    /// 合入一次新的原始快照，并用两次采样增量推导 CPU 使用率（对齐 RedisInsight）。
    /// 只有 uptime 严格递增时才能可靠换算出百分比，否则保留 None 等下一次采样。
    pub fn apply(&mut self, overview: ConnectionOverview) {
        self.version = overview.version;
        self.used_memory_bytes = overview.used_memory_bytes;
        if let Some(cpu) = overview.cpu {
            let current_cumulative = cpu.sys_seconds + cpu.user_seconds;
            if self.prev_uptime_seconds > 0.0 && cpu.uptime_seconds > self.prev_uptime_seconds {
                let time_delta = cpu.uptime_seconds - self.prev_uptime_seconds;
                let cpu_delta = current_cumulative - self.prev_cpu_seconds;
                self.cpu_usage_percent = Some(((cpu_delta / time_delta) * 100.0).max(0.0));
            } else {
                self.cpu_usage_percent = None;
            }
            self.prev_cpu_seconds = current_cumulative;
            self.prev_uptime_seconds = cpu.uptime_seconds;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TabId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct TabState {
    pub id: TabId,
    pub title: String,
    pub kind: TabKind,
    pub dirty: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TabKind {
    ObjectList(ObjectListState),
    DataEditor(DataEditorState),
    QueryEditor(QueryEditorState),
    RedisWorkbench(RedisWorkbenchState),
    RedisCli(RedisCliState),
    RedisPubSub(RedisPubSubState),
    CreateTable(CreateTableState),
    UserAdmin(UserAdminState),
    Settings,
    /// 数据库备份列表 tab（侧边栏「备份」节点单击打开，按库一个）。
    BackupList(BackupListState),
}

/// 备份列表 tab 的标识状态：仅记录连接 + 库；行数据由 UI 侧渲染时
/// 扫描备份目录（含 .meta.json 元数据）与内存运行任务合成，无需存入 controller。
#[derive(Clone, Debug, PartialEq)]
pub struct BackupListState {
    pub connection_id: ConnectionId,
    pub database: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserAdminState {
    pub connection_id: ConnectionId,
    pub active_detail_tab: UserAdminDetailTab,
    pub users: Vec<DatabaseUserIdentity>,
    pub selected_user: Option<DatabaseUserIdentity>,
    pub creating_user: bool,
    pub grants: Vec<String>,
    pub grants_loaded_user: Option<DatabaseUserIdentity>,
    pub member_grants: Vec<UserRoleMember>,
    pub member_grants_loaded_role: Option<DatabaseUserIdentity>,
    pub search: String,
    pub loading_users: bool,
    pub loading_grants: bool,
    pub loading_member_grants: bool,
    pub applying: bool,
    pub users_error: Option<UserFacingError>,
    pub grants_error: Option<UserFacingError>,
    pub member_grants_error: Option<UserFacingError>,
    pub apply_error: Option<UserFacingError>,
    pub privilege_scope: PrivilegeScope,
    pub privilege_database: String,
    pub privilege_table: String,
    pub privilege_role: String,
    pub grant_option: bool,
    pub selected_privileges: Vec<String>,
    pub privilege_rows: Vec<UserAdminPrivilegeRow>,
    pub base_privilege_rows: Vec<UserAdminPrivilegeRow>,
    pub next_privilege_row_id: u64,
    pub create_user: String,
    pub create_host: String,
    pub auth_plugin: String,
    pub password_expiry_policy: String,
    pub create_password: String,
    pub new_password: String,
    pub max_queries_per_hour: String,
    pub max_updates_per_hour: String,
    pub max_connections_per_hour: String,
    pub max_user_connections: String,
    pub ssl_type: String,
    pub ssl_cipher: String,
    pub ssl_issuer: String,
    pub ssl_subject: String,
    pub role_membership_edits: Vec<UserRoleMembership>,
    pub member_grant_edits: Vec<UserRoleMember>,
    pub pending_sql: Option<UserAdminPendingSql>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserAdminPrivilegeRow {
    pub id: u64,
    pub database: String,
    pub privileges: Vec<String>,
    pub grant_option: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UserAdminDetailTab {
    #[default]
    General,
    Advanced,
    MemberOf,
    Members,
    Privileges,
    SqlPreview,
}

impl UserAdminState {
    pub fn new(connection_id: ConnectionId, database: Option<String>, scope: PrivilegeScope) -> Self {
        Self {
            connection_id,
            active_detail_tab: UserAdminDetailTab::General,
            users: Vec::new(),
            selected_user: None,
            creating_user: false,
            grants: Vec::new(),
            grants_loaded_user: None,
            member_grants: Vec::new(),
            member_grants_loaded_role: None,
            search: String::new(),
            loading_users: false,
            loading_grants: false,
            loading_member_grants: false,
            applying: false,
            users_error: None,
            grants_error: None,
            member_grants_error: None,
            apply_error: None,
            privilege_scope: scope,
            privilege_database: database.unwrap_or_else(|| "*".to_string()),
            privilege_table: "*".to_string(),
            privilege_role: String::new(),
            grant_option: false,
            selected_privileges: vec!["SELECT".to_string()],
            privilege_rows: Vec::new(),
            base_privilege_rows: Vec::new(),
            next_privilege_row_id: 1,
            create_user: "app_user".to_string(),
            create_host: "%".to_string(),
            auth_plugin: "caching_sha2_password".to_string(),
            password_expiry_policy: "DEFAULT".to_string(),
            create_password: String::new(),
            new_password: String::new(),
            max_queries_per_hour: "0".to_string(),
            max_updates_per_hour: "0".to_string(),
            max_connections_per_hour: "0".to_string(),
            max_user_connections: "0".to_string(),
            ssl_type: "NONE".to_string(),
            ssl_cipher: String::new(),
            ssl_issuer: String::new(),
            ssl_subject: String::new(),
            role_membership_edits: Vec::new(),
            member_grant_edits: Vec::new(),
            pending_sql: None,
        }
    }

    pub fn draft_user_identity(&self) -> DatabaseUserIdentity {
        DatabaseUserIdentity {
            user: self.create_user.clone(),
            host: self.create_host.clone(),
            plugin: Some(self.auth_plugin.clone()),
        }
    }

    pub fn reset_advanced_defaults(&mut self) {
        self.max_queries_per_hour = "0".to_string();
        self.max_updates_per_hour = "0".to_string();
        self.max_connections_per_hour = "0".to_string();
        self.max_user_connections = "0".to_string();
        self.ssl_type = "NONE".to_string();
        self.ssl_cipher.clear();
        self.ssl_issuer.clear();
        self.ssl_subject.clear();
    }

    pub fn effective_role_memberships(&self) -> Vec<UserRoleMembership> {
        let Some(user) = self.selected_user.as_ref().filter(|_| !self.creating_user) else {
            return Vec::new();
        };
        let mut memberships = role_memberships_from_grants(&self.users, &self.grants, user);
        for edit in &self.role_membership_edits {
            if let Some(membership) = memberships
                .iter_mut()
                .find(|membership| membership.role == edit.role)
            {
                membership.granted = edit.granted;
                membership.default_role = edit.default_role;
            }
        }
        memberships
    }

    pub fn role_membership_dirty(&self) -> bool {
        let Some(user) = self.selected_user.as_ref().filter(|_| !self.creating_user) else {
            return false;
        };
        let base = role_memberships_from_grants(&self.users, &self.grants, user);
        let effective = self.effective_role_memberships();
        effective.iter().any(|membership| {
            base.iter()
                .find(|base| base.role == membership.role)
                .is_some_and(|base| {
                    base.granted != membership.granted
                        || base.default_role != membership.default_role
                })
        })
    }

    pub fn grants_loaded_for_selected_user(&self) -> bool {
        self.selected_user.as_ref().is_some_and(|selected| {
            self.grants_loaded_user
                .as_ref()
                .is_some_and(|loaded| loaded == selected)
        })
    }

    pub fn effective_role_members(&self) -> Vec<UserRoleMember> {
        let Some(role) = self.selected_user.as_ref().filter(|_| !self.creating_user) else {
            return Vec::new();
        };
        let mut members = self
            .member_grants
            .iter()
            .filter(|member| member.member != *role)
            .cloned()
            .collect::<Vec<_>>();
        for edit in &self.member_grant_edits {
            if let Some(member) = members
                .iter_mut()
                .find(|member| member.member == edit.member)
            {
                member.granted = edit.granted;
            }
        }
        members
    }

    pub fn role_members_dirty(&self) -> bool {
        if !self.member_grants_loaded_for_selected_role() {
            return false;
        }
        let base = &self.member_grants;
        self.effective_role_members().iter().any(|member| {
            base.iter()
                .find(|base| base.member == member.member)
                .is_some_and(|base| base.granted != member.granted)
        })
    }

    pub fn member_grants_loaded_for_selected_role(&self) -> bool {
        self.selected_user.as_ref().is_some_and(|selected| {
            self.member_grants_loaded_role
                .as_ref()
                .is_some_and(|loaded| loaded == selected)
        })
    }

    pub fn add_privilege_row(&mut self, database: String) {
        let id = self.next_privilege_row_id;
        self.next_privilege_row_id += 1;
        self.privilege_rows.push(UserAdminPrivilegeRow {
            id,
            database,
            privileges: Vec::new(),
            grant_option: false,
        });
    }

    pub fn set_privilege_row_database(&mut self, row_id: u64, database: String) {
        if let Some(row) = self
            .privilege_rows
            .iter_mut()
            .find(|row| row.id == row_id)
        {
            row.database = database;
        }
    }

    pub fn toggle_privilege_row_privilege(&mut self, row_id: u64, privilege: String) {
        let Some(row) = self
            .privilege_rows
            .iter_mut()
            .find(|row| row.id == row_id)
        else {
            return;
        };
        if let Some(index) = row.privileges.iter().position(|item| item == &privilege) {
            row.privileges.remove(index);
        } else {
            row.privileges.push(privilege);
        }
    }

    pub fn set_privilege_row_grant_option(&mut self, row_id: u64, enabled: bool) {
        if let Some(row) = self
            .privilege_rows
            .iter_mut()
            .find(|row| row.id == row_id)
        {
            row.grant_option = enabled;
        }
    }

    pub fn privileges_dirty(&self) -> bool {
        normalized_user_admin_privilege_rows(&self.privilege_rows)
            != normalized_user_admin_privilege_rows(&self.base_privilege_rows)
    }

    pub fn set_privilege_grants(&mut self, grants: Vec<DatabasePrivilegeGrant>) {
        self.next_privilege_row_id = 1;
        self.privilege_rows = grants
            .into_iter()
            .map(|grant| {
                let id = self.next_privilege_row_id;
                self.next_privilege_row_id += 1;
                UserAdminPrivilegeRow {
                    id,
                    database: grant.database,
                    privileges: grant.privileges,
                    grant_option: grant.grant_option,
                }
            })
            .collect();
        self.base_privilege_rows = self.privilege_rows.clone();
    }

    pub fn set_role_membership_granted(&mut self, role: DatabaseUserIdentity, granted: bool) {
        let current = self
            .effective_role_memberships()
            .into_iter()
            .find(|membership| membership.role == role);
        let default_role = current.as_ref().is_some_and(|membership| membership.default_role);
        self.set_role_membership_edit(role, granted, default_role);
    }

    pub fn set_role_membership_default(&mut self, role: DatabaseUserIdentity, default_role: bool) {
        let current = self
            .effective_role_memberships()
            .into_iter()
            .find(|membership| membership.role == role);
        let granted = current.as_ref().is_some_and(|membership| membership.granted);
        self.set_role_membership_edit(role, granted, default_role);
    }

    pub fn set_role_member_granted(&mut self, member: DatabaseUserIdentity, granted: bool) {
        if let Some(edit) = self
            .member_grant_edits
            .iter_mut()
            .find(|edit| edit.member == member)
        {
            edit.granted = granted;
        } else {
            self.member_grant_edits.push(UserRoleMember { member, granted });
        }
    }

    fn set_role_membership_edit(
        &mut self,
        role: DatabaseUserIdentity,
        granted: bool,
        default_role: bool,
    ) {
        if let Some(edit) = self
            .role_membership_edits
            .iter_mut()
            .find(|edit| edit.role == role)
        {
            edit.granted = granted;
            edit.default_role = default_role;
        } else {
            self.role_membership_edits.push(UserRoleMembership {
                role,
                granted,
                default_role,
            });
        }
    }
}

fn normalized_user_admin_privilege_rows(
    rows: &[UserAdminPrivilegeRow],
) -> Vec<(String, Vec<String>, bool)> {
    let mut rows = rows
        .iter()
        .filter(|row| !row.privileges.is_empty())
        .map(|row| {
            let mut privileges = row.privileges.clone();
            privileges.sort();
            privileges.dedup();
            (row.database.clone(), privileges, row.grant_option)
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserAdminPendingSql {
    pub sql: String,
    pub danger: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectListState {
    pub parent: Option<ObjectPath>,
    pub objects: Vec<ObjectSummary>,
    pub loading: bool,
    pub error: Option<UserFacingError>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DataEditorState {
    pub object: ObjectPath,
    pub page: Option<DataPage>,
    pub original_page: Option<DataPage>,
    pub pagination: Pagination,
    pub changes: Option<DataChangeSet>,
    pub editing_cell: Option<CellPosition>,
    pub cell_detail_panel: CellDetailPanelState,
    pub table_info: TableInfoState,
    pub loading: bool,
    pub error: Option<UserFacingError>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableInfoState {
    pub open: bool,
    pub active_tab: TableInfoTab,
    pub width: f32,
    pub search: String,
    pub highlighted_column: Option<String>,
    pub ddl_wrap: bool,
    pub indexes: LoadState<Vec<IndexInfo>>,
    pub foreign_keys: LoadState<Vec<ForeignKeyInfo>>,
    pub triggers: LoadState<Vec<TriggerInfo>>,
    pub ddl: LoadState<String>,
}

impl Default for TableInfoState {
    fn default() -> Self {
        Self {
            open: false,
            active_tab: TableInfoTab::Columns,
            // 默认给字段名、类型和注释留出稳定的三列空间，避免右侧预览一打开就换行。
            width: 460.,
            search: String::new(),
            highlighted_column: None,
            ddl_wrap: false,
            indexes: LoadState::NotLoaded,
            foreign_keys: LoadState::NotLoaded,
            triggers: LoadState::NotLoaded,
            ddl: LoadState::NotLoaded,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum TableInfoTab {
    Columns,
    Indexes,
    ForeignKeys,
    Triggers,
    Ddl,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState<T> {
    NotLoaded,
    Loading,
    Loaded(T),
    Failed(UserFacingError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TableInfoResult {
    Indexes(Vec<IndexInfo>),
    ForeignKeys(Vec<ForeignKeyInfo>),
    Triggers(Vec<TriggerInfo>),
    Ddl(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellPosition {
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellDetailPanelState {
    pub open: bool,
    pub active_cell: Option<CellPosition>,
    pub mode: CellDetailMode,
    pub edit_value: String,
}

impl Default for CellDetailPanelState {
    fn default() -> Self {
        Self {
            open: false,
            active_cell: None,
            mode: CellDetailMode::View,
            edit_value: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellDetailMode {
    View,
    Edit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryEditorState {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// 编辑器当前 schema 作用域（PG）；MySQL/TiDB/Redis 恒为 None。
    pub schema: Option<String>,
    pub text: String,
    pub origin: Option<QueryOrigin>,
    pub saved_fingerprint: Option<QueryFingerprint>,
    pub running: bool,
    pub results: Vec<DataPage>,
    pub result_editors: BTreeMap<usize, DataEditorState>,
    pub active_result_editor: Option<usize>,
    pub summaries: Vec<QueryExecutionSummary>,
    pub error: Option<UserFacingError>,
}

/// Redis 命令执行器（Workbench）面板状态。
///
/// 与 SQL 查询面板分离：结果不是表格数据（DataPage），而是通用的命令回复
/// （CommandWorkbenchExecution），由 RedisConnector::execute_command_workbench 产出。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisWorkbenchState {
    pub connection_id: ConnectionId,
    /// 目标逻辑数据库编号（0-15）。
    pub database: u32,
    /// 顶部输入框的草稿文本。点击 Run 后立刻清空，命令文本进入 `executions` 记录。
    pub text: String,
    pub running: bool,
    /// 当前会话内按执行顺序追加的命令执行记录（一执行一条记录）。
    /// 每条都能单独重跑（Run）或删除（Delete），不跨会话持久化。
    pub executions: Vec<CommandWorkbenchExecution>,
    pub error: Option<UserFacingError>,
    /// 最近一次已执行命令文本的指纹；用于让「未保存（未执行改动）」标记在标题上实时更新：
    /// 输入后标记脏，执行成功后清除脏标记，再次修改后又复现。
    /// 点击 Run 清空输入时置为空文本指纹，避免空输入仍被标脏。
    pub saved_fingerprint: Option<QueryFingerprint>,
    /// 用于为 `executions` 记录分配 `CommandWorkbenchExecution.id` 的单调递增计数器。
    pub next_execution_id: u64,
    /// 当前处于折叠态的执行记录 ID 集合（仅 UI 展示用，会话内有效，不持久化）。
    /// 折叠后只显示记录头部（命令文本 + 状态 + 耗时 + 操作），隐藏其下各子命令的 reply。
    pub collapsed: BTreeSet<u64>,
    /// 以 JSON 视图展示的结果项集合，键为 `(execution_id, command_index)`。
    ///
    /// 仅当某条命令属于 JSON 命令（如 `JSON.GET`）时才允许加入；非 JSON 命令不显示
    /// `Text / JSON` 切换，也就不会出现在此集合。按执行项维度记忆，与会话内有效、不持久化，
    /// 保证一次执行里的多条命令可以各自独立选择展示模式。
    pub json_views: BTreeSet<(u64, usize)>,
}

impl RedisWorkbenchState {
    /// 是否有未执行（未保存）的命令改动。已有执行指纹时与最新文本比对；
    /// 从未执行过（无指纹）则只要有非空文本即视为脏。
    pub fn has_unsaved_text(&self) -> bool {
        match self.saved_fingerprint {
            Some(saved) => saved != QueryFingerprint::for_text(&self.text),
            None => !self.text.trim().is_empty(),
        }
    }
}

/// Redis CLI 终端标签页：只定位到「某个连接的某个库」，具体交互（PTY 进程 / 网格 / 交互）
/// 由桌面端 `TerminalSessionModel` 承担，state 侧仅作为 tab 定位与复用 key。
/// 与 RedisWorkbench 并存：Workbench 走输入框 + 结果面板，Redis CLI 走真实终端 surface。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisCliState {
    pub connection_id: ConnectionId,
    /// 目标逻辑数据库编号（0-15）。
    pub database: u32,
}

impl RedisCliState {
    /// tab 复用 key：同一连接 + 同一库在右侧菜单 / 双击打开时复用已有标签页。
    pub fn session_key(&self) -> fluxdb_core::terminal::TerminalSessionKey {
        fluxdb_core::terminal::TerminalSessionKey::Redis {
            connection_id: self.connection_id.0,
            database: self.database,
        }
    }
}

/// Redis Pub/Sub 标签页：只定位到「某个连接的某个库」，具体的订阅连接、消息流与交互
/// 由桌面端 `PubSubSessionModel` 承担，state 侧仅作为 tab 定位与复用 key。
/// 与 RedisCli 并列：CLI 走真实终端 surface，Pub/Sub 走上「订阅 + 实时消息流」专用页面。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisPubSubState {
    pub connection_id: ConnectionId,
    /// 目标逻辑数据库编号（0-15）。
    pub database: u32,
}

impl QueryEditorState {
    pub fn has_unsaved_sql(&self) -> bool {
        match (&self.origin, self.saved_fingerprint) {
            (None, _) => !self.text.trim().is_empty(),
            (Some(_), Some(saved)) => saved != QueryFingerprint::for_text(&self.text),
            (Some(_), None) => !self.text.trim().is_empty(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QueryOrigin {
    Connection { query_id: u64 },
    File { path: PathBuf },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryFingerprint {
    pub len: usize,
    pub hash: u64,
}

impl QueryFingerprint {
    pub fn for_text(text: &str) -> Self {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in text.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        Self {
            len: text.len(),
            hash,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryHistoryEntry {
    pub connection_id: ConnectionId,
    pub database: Option<String>,
    /// 历史记录所属 schema 作用域（PG）；MySQL/TiDB/Redis 恒为 None。
    pub schema: Option<String>,
    pub text: String,
    pub tables: Vec<String>,
    pub kind: QueryHistoryKind,
    pub success: bool,
    pub summary: QueryExecutionSummary,
    pub executed_at_unix_secs: u64,
    pub object: Option<String>,
    pub rollback_snapshot: Option<QueryRollbackSnapshot>,
    /// 写入是否已提交（§8.4/R11）。显式事务里未 COMMIT 的写入不显示为已提交。
    pub transaction_state: QueryHistoryTransactionState,
}

/// 写入的事务状态。默认 `Committed`（无显式事务、旧记录缺省时按已提交展示）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QueryHistoryTransactionState {
    /// 已提交（自动提交或显式 COMMIT）。
    #[default]
    Committed,
    /// 事务仍未提交（显式事务进行中）。
    Uncommitted,
    /// 已回滚（显式 ROLLBACK，或连接释放时未提交被服务端回滚）。
    RolledBack,
}

impl QueryHistoryTransactionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Uncommitted => "uncommitted",
            Self::RolledBack => "rolled_back",
        }
    }

    /// 从持久化字符串还原；未知值按已提交（旧记录兼容）。
    pub fn from_storage(value: Option<&str>) -> Self {
        match value {
            Some("uncommitted") => Self::Uncommitted,
            Some("rolled_back") => Self::RolledBack,
            _ => Self::Committed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryHistoryKind {
    Query,
    DataChange,
    SchemaChange,
}

/// Redis Workbench 命令历史单条记录（App 层内存态，与 `QueryHistoryEntry` 同层级）。
///
/// 与 RedisInsight 对齐：命令文本 + 结果摘要 + 时间 + 来源，按连接 + 逻辑库隔离；
/// 成功 / 失败都会入历史，保留排障信息。底层持久化字段见 fluxdb-storage 的
/// `RedisWorkbenchHistoryRecord`。
#[derive(Clone, Debug, PartialEq)]
pub struct RedisWorkbenchHistoryEntry {
    /// scope 内去重的记录 ID（用于删除定位）。
    pub id: u64,
    pub connection_id: ConnectionId,
    /// Redis 逻辑数据库编号（0-15）。
    pub database: u32,
    /// 命令文本，可回填到 Workbench 输入框。
    pub text: String,
    /// 执行是否成功。
    pub success: bool,
    /// 执行 Unix 秒级时间戳。
    pub executed_at_unix_secs: u64,
    /// 人类可读的结果摘要。
    pub summary: String,
    /// 来源类型（Workbench / HistoryRerun / KeyShortcut）。
    pub source: CommandExecutionSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskState {
    pub label: String,
    pub running: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppCommand {
    LoadConnections,
    ReplaceConnections(Vec<ConnectionConfig>),
    ReplaceSidebarLayout(SidebarLayout),
    CreateConnection(ConnectionDraft),
    UpdateConnection(ConnectionConfig),
    CreateConnectionGroup(String),
    RenameConnectionGroup {
        group_id: ConnectionGroupId,
        name: String,
    },
    ToggleConnectionGroup(ConnectionGroupId),
    DeleteConnectionGroup(ConnectionGroupId),
    MoveConnectionToGroup {
        connection_id: ConnectionId,
        group_id: ConnectionGroupId,
    },
    MoveConnectionToTopLevel(ConnectionId),
    TestConnection(ConnectionConfig),
    /// 从「连接串 / 云资源标识」自动发现一条 Redis 连接（URI 导入 / 云自动发现统一入口）。
    /// `provider` 为云提供商标识（`azure` / `redis-cloud` / 空表示普通连接串导入）。
    DiscoverRedisConnection {
        provider: String,
        connection_string: String,
    },
    OpenConnection(ConnectionId),
    ToggleConnectionExpanded(ConnectionId),
    DisconnectConnection(ConnectionId),
    DisconnectDatabase {
        connection_id: ConnectionId,
        database: String,
    },
    DeleteConnection(ConnectionId),
    CreateDatabase(CreateDatabaseRequest),
    DeleteDatabase {
        connection_id: ConnectionId,
        database: String,
    },
    LoadObjectChildren(ObjectPath),
    RefreshObject(Option<ObjectPath>),
    OpenObjectList(Option<ObjectPath>),
    OpenDataEditor(ObjectPath),
    /// 打开某数据库的备份列表 tab（传入数据库 ObjectPath）。
    OpenBackupList(ObjectPath),
    CopyTable {
        object: ObjectPath,
        new_name: String,
        copy_data: bool,
    },
    DropTable {
        object: ObjectPath,
        foreign_key_check: ForeignKeyCheckMode,
    },
    TruncateTable {
        object: ObjectPath,
        foreign_key_check: ForeignKeyCheckMode,
    },
    LoadDataPage(TabId),
    SetDataPagePagination {
        tab_id: TabId,
        offset: u64,
        limit: u64,
    },
    LoadDataPageWithSort {
        tab_id: TabId,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
    },
    LoadRedisKey {
        tab_id: TabId,
        key: String,
    },
    /// 惰性补齐 Redis Key 列表元信息：给定当前可见的一批键名，批量取回类型/值/大小/TTL。
    /// 对应 [`AppEvent::RedisKeyMetadataLoaded`]；空数组时直接成功返回空结果（不再发请求）。
    LoadRedisKeyMetadata {
        tab_id: TabId,
        keys: Vec<String>,
    },
    /// 拉取 Redis 连接级运行概览（版本 / 内存 / CPU），刷新底栏连接状态摘要。
    LoadRedisOverview(ConnectionId),
    /// 拉取 Redis 服务端版本，作为字段级 TTL 编辑等能力开关。
    /// 结果经 [`AppEvent::RedisServerVersionLoaded`] 回传；解析/命令失败统一回 `None`。
    LoadRedisServerVersion(ConnectionId),
    FinishRedisKeyRefresh {
        tab_id: TabId,
        key: String,
        result: std::result::Result<Row, UserFacingError>,
    },
    ApplyRedisKeyValue {
        tab_id: TabId,
        key: String,
        new_key: String,
        ttl: Option<String>,
        value: Option<String>,
    },
    AddRedisStreamEntry {
        tab_id: TabId,
        key: String,
        id: String,
        fields: Vec<(String, String)>,
        /// 追加后按 `MAXLEN ~ n` 近似裁剪；None 表示不裁剪。
        maxlen: Option<u64>,
    },
    DeleteRedisStreamEntry {
        tab_id: TabId,
        key: String,
        entry_id: String,
    },
    DeleteRedisSetMember {
        tab_id: TabId,
        key: String,
        member: String,
    },
    AddRedisSetMember {
        tab_id: TabId,
        key: String,
        member: String,
    },
    LoadRedisSetMembers {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    },
    LoadRedisHashFields {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    },
    SetRedisHashField {
        tab_id: TabId,
        key: String,
        field: String,
        value: String,
        /// 该字段 TTL 的处置方式：保留原值 / 清除 / 设为指定秒数。
        ttl: RedisHashFieldTtl,
    },
    /// 完整值弹框保存：携带完整原始值（可能 >1MB 被截断标记），放行截断 guard。
    /// 行内编辑走 [`AppCommand::SetRedisHashField`]（default 拒截断）。
    SetRedisHashFieldRaw {
        tab_id: TabId,
        key: String,
        field: String,
        value: String,
        ttl: RedisHashFieldTtl,
    },
    /// 仅更新 Hash 字段 TTL，不触碰字段值（纯 TTL 命令，Redis 7.4+ 字段级 TTL）。
    /// 用于大字段（>1MB 被截断）的 TTL 编辑：行内 `SetRedisHashField` 会重写 value，
    /// 对截断片段回写是危险的，必须走纯 TTL 路径。
    SetRedisHashFieldTtl {
        tab_id: TabId,
        key: String,
        field: String,
        ttl: RedisHashFieldTtl,
    },
    /// 完整值弹框懒加载：读取 hash 字段完整原始值（不截断），
    /// 结果经 [`AppEvent::RedisHashFieldFullValueLoaded`] 回传。
    LoadRedisHashFieldFull {
        tab_id: TabId,
        key: String,
        field: String,
    },
    /// String 详情值加载：`full=false` 取前 4999 字节预览（STRLEN+GETRANGE），
    /// `full=true` 取完整值（GET）。结果经 [`AppEvent::RedisStringValueLoaded`] 回传。
    LoadRedisStringValue {
        tab_id: TabId,
        key: String,
        full: bool,
    },
    /// String / JSON 值下载：取原始字节供导出文件。结果经 [`AppEvent::RedisStringValueDownloaded`] 回传。
    DownloadRedisStringValue {
        tab_id: TabId,
        key: String,
    },
    RenameRedisHashField {
        tab_id: TabId,
        key: String,
        old_field: String,
        new_field: String,
        value: String,
    },
    DeleteRedisHashField {
        tab_id: TabId,
        key: String,
        field: String,
    },
    LoadRedisZSetMembers {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
    },
    AddRedisZSetMember {
        tab_id: TabId,
        key: String,
        member: String,
        score: String,
    },
    UpdateRedisZSetScore {
        tab_id: TabId,
        key: String,
        member: String,
        score: String,
    },
    DeleteRedisZSetMember {
        tab_id: TabId,
        key: String,
        member: String,
    },
    LoadRedisListItems {
        tab_id: TabId,
        key: String,
        /// 按「包含」过滤元素；空串表示不过滤。
        query: String,
        cursor: String,
    },
    /// 分页拉取 Stream 条目：cursor 为空取最新一页，非空表示「加载更多」续页。
    LoadRedisStreamEntries {
        tab_id: TabId,
        key: String,
        /// 可选的时间范围（毫秒时间戳），下推给 XREVRANGE 由服务端过滤。
        since_ms: Option<u64>,
        until_ms: Option<u64>,
        cursor: String,
    },
    /// 读取 Stream 的消费者组概览（只读）。
    LoadRedisStreamGroups {
        tab_id: TabId,
        key: String,
    },
    PushRedisListItems {
        tab_id: TabId,
        key: String,
        items: Vec<String>,
        head: bool,
    },
    /// 新建一个 Redis Key（对齐 RedisInsight AddKey）。`request` 携带类型化字段，
    /// 后端据类型执行 SET/HSET/SADD/ZADD/RPUSH/XADD/JSON.SET 并应用可选 TTL。
    /// 成功后回传 [`AppEvent::RedisKeyCreated`]。
    CreateRedisKey {
        tab_id: TabId,
        request: RedisAddKeyRequest,
    },
    SetRedisListItem {
        tab_id: TabId,
        key: String,
        index: usize,
        expected_old: Option<String>,
        value: String,
    },
    DeleteRedisListItem {
        tab_id: TabId,
        key: String,
        index: usize,
        expected_old: Option<String>,
    },
    /// 从 List 头部/尾部按数量弹出元素（LPOP/RPOP，对齐 RedisInsight Remove elements）。
    PopRedisListItems {
        tab_id: TabId,
        key: String,
        /// true 从头弹出（LPOP），false 从尾弹出（RPOP）。
        head: bool,
        /// 弹出数量；1 全版本可用，>1 需 Redis ≥ 6.2。
        count: usize,
    },
    FinishDataPageLoad {
        tab_id: TabId,
        result: std::result::Result<DataPage, UserFacingError>,
    },
    ToggleTableInfo {
        tab_id: TabId,
        tab: TableInfoTab,
    },
    CloseTableInfo(TabId),
    SelectTableInfoTab {
        tab_id: TabId,
        tab: TableInfoTab,
    },
    LoadTableInfo {
        tab_id: TabId,
        tab: TableInfoTab,
    },
    FinishTableInfoLoad {
        tab_id: TabId,
        tab: TableInfoTab,
        result: std::result::Result<TableInfoResult, UserFacingError>,
    },
    SetTableInfoSearch {
        tab_id: TabId,
        search: String,
    },
    SetTableInfoWidth {
        tab_id: TabId,
        width: f32,
    },
    ToggleDdlWrap(TabId),
    HighlightDataColumn {
        tab_id: TabId,
        column: String,
    },
    EditDataCell {
        tab_id: TabId,
        row: usize,
        column: usize,
        value: CellValue,
    },
    LoadBinaryPreview {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    DownloadBinaryCell {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    UpdateBinaryCell {
        tab_id: TabId,
        row: usize,
        column: usize,
        payload: BinaryUpdatePayload,
    },
    SetBinaryCellNull {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    ReplaceBinaryCellFromFile {
        tab_id: TabId,
        row: usize,
        column: usize,
        path: PathBuf,
    },
    OpenCellDetail {
        tab_id: TabId,
        row: usize,
        column: usize,
    },
    CloseCellDetail(TabId),
    StartCellDetailEdit(TabId),
    UpdateCellDetailEditValue {
        tab_id: TabId,
        value: String,
    },
    CancelCellDetailEdit(TabId),
    SaveCellDetailEdit(TabId),
    SetCellDetailNull(TabId),
    RestoreCellDetailOriginalValue(TabId),
    InsertDataRow {
        tab_id: TabId,
        result_index: Option<usize>,
        after_row: Option<usize>,
    },
    CloneDataRow {
        tab_id: TabId,
        result_index: Option<usize>,
        row: usize,
        after_row: Option<usize>,
    },
    DeleteDataRow {
        tab_id: TabId,
        result_index: Option<usize>,
        row: usize,
    },
    DiscardDataChanges(TabId),
    ApplyDataChanges(TabId),
    ApplyDataChangesWithView {
        tab_id: TabId,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
    },
    OpenQueryEditor(ConnectionId),
    OpenUserAdmin(ConnectionId),
    OpenSettings,
    OpenQueryEditorInDatabase {
        connection_id: ConnectionId,
        database: Option<String>,
        /// 编辑器初始 schema 作用域（PG）；MySQL/TiDB 恒为 None。
        schema: Option<String>,
    },
    OpenRedisWorkbench {
        connection_id: ConnectionId,
        database: u32,
    },
    OpenRedisCli {
        connection_id: ConnectionId,
        database: u32,
    },
    OpenRedisPubSub {
        connection_id: ConnectionId,
        database: u32,
    },
    OpenCreateTable {
        connection_id: ConnectionId,
        database: Option<String>,
        /// schema 作用域（PG）；None/空表示用连接默认 search_path。
        schema: Option<String>,
    },
    OpenDesignTable(ObjectPath),
    RenameTable {
        object: ObjectPath,
        new_name: String,
    },
    StartCreateTableApply(TabId),
    ApplyCreateTable(TabId),
    FinishCreateTableApply {
        tab_id: TabId,
        result: std::result::Result<(), UserFacingError>,
    },
    SetCreateTableField {
        tab_id: TabId,
        field: CreateTableField,
        value: String,
    },
    SetCreateTableOptionField {
        tab_id: TabId,
        field: CreateTableOptionField,
        value: String,
    },
    ToggleCreateTablePartitionEnabled(TabId),
    SetCreateTablePartitionField {
        tab_id: TabId,
        field: CreateTablePartitionField,
        value: String,
    },
    SelectCreateTableTab {
        tab_id: TabId,
        create_tab: CreateTableTab,
    },
    SelectCreateTableColumn {
        tab_id: TabId,
        column_id: u64,
    },
    AddCreateTableColumn(TabId),
    MoveCreateTableColumnUp {
        tab_id: TabId,
        column_id: u64,
    },
    MoveCreateTableColumnDown {
        tab_id: TabId,
        column_id: u64,
    },
    RemoveCreateTableColumn {
        tab_id: TabId,
        column_id: u64,
    },
    SetCreateTableColumnField {
        tab_id: TabId,
        column_id: u64,
        field: CreateTableColumnField,
        value: String,
    },
    ToggleCreateTableColumnFlag {
        tab_id: TabId,
        column_id: u64,
        flag: CreateTableColumnFlag,
    },
    SelectCreateTableIndex {
        tab_id: TabId,
        index_id: u64,
    },
    AddCreateTableIndex(TabId),
    MoveCreateTableIndexUp {
        tab_id: TabId,
        index_id: u64,
    },
    MoveCreateTableIndexDown {
        tab_id: TabId,
        index_id: u64,
    },
    RemoveCreateTableIndex {
        tab_id: TabId,
        index_id: u64,
    },
    SetCreateTableIndexField {
        tab_id: TabId,
        index_id: u64,
        field: CreateTableIndexField,
        value: String,
    },
    AddCreateTableIndexColumn {
        tab_id: TabId,
        index_id: u64,
    },
    MoveCreateTableIndexColumnUp {
        tab_id: TabId,
        index_id: u64,
        column_index: usize,
    },
    MoveCreateTableIndexColumnDown {
        tab_id: TabId,
        index_id: u64,
        column_index: usize,
    },
    RemoveCreateTableIndexColumn {
        tab_id: TabId,
        index_id: u64,
        column_index: usize,
    },
    SetCreateTableIndexColumnField {
        tab_id: TabId,
        index_id: u64,
        column_index: usize,
        field: CreateTableIndexColumnField,
        value: String,
    },
    SelectCreateTableCheck {
        tab_id: TabId,
        check_id: u64,
    },
    AddCreateTableCheck(TabId),
    MoveCreateTableCheckUp {
        tab_id: TabId,
        check_id: u64,
    },
    MoveCreateTableCheckDown {
        tab_id: TabId,
        check_id: u64,
    },
    RemoveCreateTableCheck {
        tab_id: TabId,
        check_id: u64,
    },
    SetCreateTableCheckField {
        tab_id: TabId,
        check_id: u64,
        field: CreateTableCheckField,
        value: String,
    },
    ToggleCreateTableCheckNotEnforced {
        tab_id: TabId,
        check_id: u64,
    },
    SelectCreateTableTrigger {
        tab_id: TabId,
        trigger_id: u64,
    },
    AddCreateTableTrigger(TabId),
    MoveCreateTableTriggerUp {
        tab_id: TabId,
        trigger_id: u64,
    },
    MoveCreateTableTriggerDown {
        tab_id: TabId,
        trigger_id: u64,
    },
    RemoveCreateTableTrigger {
        tab_id: TabId,
        trigger_id: u64,
    },
    SetCreateTableTriggerField {
        tab_id: TabId,
        trigger_id: u64,
        field: CreateTableTriggerField,
        value: String,
    },
    SetCreateTableTriggerEvent {
        tab_id: TabId,
        trigger_id: u64,
        event: CreateTableTriggerEvent,
    },
    SelectCreateTableForeignKey {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    AddCreateTableForeignKey(TabId),
    MoveCreateTableForeignKeyUp {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    MoveCreateTableForeignKeyDown {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    RemoveCreateTableForeignKey {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    SetCreateTableForeignKeyField {
        tab_id: TabId,
        foreign_key_id: u64,
        field: CreateTableForeignKeyField,
        value: String,
    },
    StartCreateTableReferenceColumnsLoad {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    LoadCreateTableReferenceColumns {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    FinishCreateTableReferenceColumnsLoad {
        tab_id: TabId,
        foreign_key_id: u64,
        result: std::result::Result<Vec<String>, UserFacingError>,
    },
    AddCreateTableForeignKeyColumn {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    MoveCreateTableForeignKeyColumnUp {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    MoveCreateTableForeignKeyColumnDown {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    RemoveCreateTableForeignKeyColumn {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    SetCreateTableForeignKeyColumn {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
        value: String,
    },
    AddCreateTableForeignKeyReferencedColumn {
        tab_id: TabId,
        foreign_key_id: u64,
    },
    MoveCreateTableForeignKeyReferencedColumnUp {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    MoveCreateTableForeignKeyReferencedColumnDown {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    RemoveCreateTableForeignKeyReferencedColumn {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
    },
    SetCreateTableForeignKeyReferencedColumn {
        tab_id: TabId,
        foreign_key_id: u64,
        column_index: usize,
        value: String,
    },

    UpdateQueryText {
        tab_id: TabId,
        text: String,
    },
    RequestQueryCompletions {
        tab_id: TabId,
        request_seq: u64,
        cursor: usize,
        explicit: bool,
    },
    WarmCompletionIndex {
        connection_id: ConnectionId,
        database: Option<String>,
    },
    FormatQuerySql(TabId),
    CompressQuerySql(TabId),
    MarkQuerySaved {
        tab_id: TabId,
        title: String,
        origin: QueryOrigin,
    },
    StartQueryExecution(TabId),
    ExecuteQuery(TabId),
    ExecuteQueryText {
        tab_id: TabId,
        text: String,
    },
    ExecuteQueryTextWithOptions {
        tab_id: TabId,
        text: String,
        options: QueryExecutionOptions,
    },
    FinishQueryExecution {
        tab_id: TabId,
        result: std::result::Result<QueryExecutionResult, UserFacingError>,
    },
    UpdateRedisWorkbenchText {
        tab_id: TabId,
        text: String,
    },
    ExecuteRedisWorkbench(TabId),
    FinishRedisWorkbenchExecution {
        tab_id: TabId,
        result: std::result::Result<CommandWorkbenchExecution, UserFacingError>,
    },
    /// 重跑结果区某条执行记录（按 `CommandWorkbenchExecution.id` 定位，复用其命令文本）。
    RerunRedisWorkbenchRecord {
        tab_id: TabId,
        execution_id: u64,
    },
    /// 删除结果区某条执行记录，只删当前一条。
    DeleteRedisWorkbenchRecord {
        tab_id: TabId,
        execution_id: u64,
    },
    ClearRedisWorkbenchResults(TabId),
    /// 切换结果区某条执行记录的折叠/展开（仅 UI 展示态，会话内有效）。
    ToggleRedisWorkbenchRecordCollapse { tab_id: TabId, execution_id: u64 },
    /// 切换结果区某条子命令的 `Text / JSON` 展示（仅 JSON 命令结果显示切换；按执行项维度记忆）。
    ToggleRedisWorkbenchJsonView {
        tab_id: TabId,
        execution_id: u64,
        command_index: usize,
    },
    /// 删除 Redis Workbench「历史」抽屉中指定 scope 的一条历史记录。
    DeleteRedisWorkbenchHistory {
        scope: WorkbenchHistoryScope,
        id: u64,
    },
    /// 清空 Redis Workbench「历史」抽屉中指定 scope 的全部历史。
    ClearRedisWorkbenchHistory { scope: WorkbenchHistoryScope },
    FinishQueryResultPageRefresh {
        tab_id: TabId,
        result_index: usize,
        page_index: usize,
        result: std::result::Result<QueryExecutionResult, UserFacingError>,
    },
    ActivateQueryResultEditor {
        tab_id: TabId,
        page_index: Option<usize>,
    },
    LoadUserAdminUsers(TabId),
    StartUserAdminUsersLoad(TabId),
    FinishUserAdminUsersLoad {
        tab_id: TabId,
        result: std::result::Result<Vec<DatabaseUserIdentity>, UserFacingError>,
    },
    LoadUserAdminGrants {
        tab_id: TabId,
        user: DatabaseUserIdentity,
    },
    StartUserAdminGrantsLoad {
        tab_id: TabId,
        user: DatabaseUserIdentity,
    },
    FinishUserAdminGrantsLoad {
        tab_id: TabId,
        user: DatabaseUserIdentity,
        result: std::result::Result<Vec<String>, UserFacingError>,
    },
    LoadUserAdminMemberGrants {
        tab_id: TabId,
        role: DatabaseUserIdentity,
    },
    StartUserAdminMemberGrantsLoad {
        tab_id: TabId,
        role: DatabaseUserIdentity,
    },
    FinishUserAdminMemberGrantsLoad {
        tab_id: TabId,
        role: DatabaseUserIdentity,
        result: std::result::Result<Vec<UserRoleMember>, UserFacingError>,
    },
    SelectUserAdminUser {
        tab_id: TabId,
        user: DatabaseUserIdentity,
    },
    BeginUserAdminCreateUser(TabId),
    SelectUserAdminDetailTab {
        tab_id: TabId,
        detail_tab: UserAdminDetailTab,
    },
    SetUserAdminSearch {
        tab_id: TabId,
        search: String,
    },
    SetUserAdminPrivilegeDatabase {
        tab_id: TabId,
        database: String,
    },
    SetUserAdminPrivilegeTable {
        tab_id: TabId,
        table: String,
    },
    ToggleUserAdminPrivilege {
        tab_id: TabId,
        privilege: String,
    },
    SetUserAdminGrantOption {
        tab_id: TabId,
        enabled: bool,
    },
    AddUserAdminPrivilegeRow {
        tab_id: TabId,
        database: String,
    },
    SetUserAdminPrivilegeRowDatabase {
        tab_id: TabId,
        row_id: u64,
        database: String,
    },
    ToggleUserAdminPrivilegeRowPrivilege {
        tab_id: TabId,
        row_id: u64,
        privilege: String,
    },
    SetUserAdminPrivilegeRowGrantOption {
        tab_id: TabId,
        row_id: u64,
        enabled: bool,
    },
    SetUserAdminCreateUser {
        tab_id: TabId,
        user: String,
    },
    SetUserAdminCreateHost {
        tab_id: TabId,
        host: String,
    },
    SetUserAdminAuthPlugin {
        tab_id: TabId,
        plugin: String,
    },
    SetUserAdminPasswordExpiryPolicy {
        tab_id: TabId,
        policy: String,
    },
    SetUserAdminCreatePassword {
        tab_id: TabId,
        password: String,
    },
    SetUserAdminNewPassword {
        tab_id: TabId,
        password: String,
    },
    SetUserAdminMaxQueriesPerHour {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminMaxUpdatesPerHour {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminMaxConnectionsPerHour {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminMaxUserConnections {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminSslType {
        tab_id: TabId,
        ssl_type: String,
    },
    SetUserAdminSslCipher {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminSslIssuer {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminSslSubject {
        tab_id: TabId,
        value: String,
    },
    SetUserAdminRoleMembershipGranted {
        tab_id: TabId,
        role: DatabaseUserIdentity,
        granted: bool,
    },
    SetUserAdminRoleMembershipDefault {
        tab_id: TabId,
        role: DatabaseUserIdentity,
        default_role: bool,
    },
    SetUserAdminRoleMemberGranted {
        tab_id: TabId,
        member: DatabaseUserIdentity,
        granted: bool,
    },
    PreviewUserAdminSql {
        tab_id: TabId,
        sql: String,
        danger: bool,
    },
    ClearUserAdminPendingSql(TabId),
    StartUserAdminSqlApply(TabId),
    ApplyUserAdminSql {
        tab_id: TabId,
        sql: String,
    },
    FinishUserAdminSqlApply {
        tab_id: TabId,
        result: std::result::Result<(), UserFacingError>,
    },
    CloseTab(TabId),
    CloseTabs(Vec<TabId>),
    ConfirmCloseDirtyTab(TabId),
    CancelCloseDirtyTab(TabId),
    ActivateTab(TabId),
    /// 取消激活当前标签（active_tab 置空 → 显示首页/启动页）。已打开的标签保留在标签栏，
    /// 仅取消聚焦，非破坏性；与「关闭全部标签」(CloseTabs) 语义不同。
    DeactivateTab,
    PinTab(TabId),
    ReplaceQueryHistory(Vec<QueryHistoryEntry>),
    SaveSettings(Settings),
    SetTabDirty { tab_id: TabId, dirty: bool },
    ClearCompletionIndexCache,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    ConnectionsLoaded(Vec<ConnectionConfig>),
    SidebarLayoutChanged(SidebarLayout),
    ConnectionCreated(ConnectionConfig),
    ConnectionUpdated(ConnectionConfig),
    ConnectionGroupCreated(ConnectionGroup),
    ConnectionGroupDeleted(ConnectionGroupId),
    ConnectionDeleted(ConnectionId),
    DatabaseDeleted {
        connection_id: ConnectionId,
        database: String,
    },
    ConnectionTested(ConnectionId, fluxdb_core::Result<()>),
    /// 自动发现 / 连接串导入成功，弹框可据此预填一条 Redis 连接草稿。
    RedisConnectionDiscovered(ConnectionDraft),
    ObjectsLoaded(Option<ObjectPath>, Vec<ObjectSummary>),
    DataLoaded(TabId, DataPage),
    /// 惰性补齐无用的事件：元信息已由控制器合并进 `editor.page`，只用于通知 UI 续补下一批可见行。
    RedisKeyMetadataLoaded {
        tab_id: TabId,
    },
    RedisSetMembersLoaded {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        members: Vec<String>,
        next_cursor: String,
        total: usize,
    },
    RedisHashFieldsLoaded {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        fields: Vec<(String, String, String)>,
        next_cursor: String,
        total: usize,
    },
    /// 完整值弹框懒加载结果：对应 [`AppCommand::LoadRedisHashFieldFull`]，返回非截断原始值。
    RedisHashFieldFullValueLoaded {
        tab_id: TabId,
        field: String,
        value: String,
    },
    /// String 详情值加载结果：对应 [`AppCommand::LoadRedisStringValue`]。
    RedisStringValueLoaded {
        tab_id: TabId,
        key: String,
        value: String,
        len: u64,
        loaded_all: bool,
    },
    /// String / JSON 值下载结果：对应 [`AppCommand::DownloadRedisStringValue`]，返回原始字节。
    RedisStringValueDownloaded {
        tab_id: TabId,
        key: String,
        bytes: Vec<u8>,
    },
    RedisZSetMembersLoaded {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        members: Vec<(String, String)>,
        next_cursor: String,
        total: usize,
    },
    RedisListItemsLoaded {
        tab_id: TabId,
        key: String,
        query: String,
        cursor: String,
        items: Vec<(usize, String)>,
        next_cursor: String,
        total: usize,
    },
    RedisStreamGroupsLoaded {
        tab_id: TabId,
        key: String,
        /// (组名, 消费者数, 未确认数, 最后投递 ID, [(消费者名, 未确认数, 空闲毫秒)])
        groups: Vec<(String, u64, u64, String, Vec<(String, u64, u64)>)>,
    },
    RedisStreamEntriesLoaded {
        tab_id: TabId,
        key: String,
        cursor: String,
        /// 一页条目：(流 ID, 由 ID 推导的本地时间, 字段列表)，顺序为从新到旧。
        entries: Vec<(String, String, Vec<(String, String)>)>,
        next_cursor: String,
        total: usize,
    },
    /// Redis 连接级运行概览拉取结果，供底栏连接状态摘要展示。
    RedisOverviewLoaded(ConnectionId, ConnectionOverview),
    /// Redis 服务端版本加载结果；`None` 表示非 Redis 连接 / 解析失败 / 命令失败（能力未知）。
    RedisServerVersionLoaded(ConnectionId, Option<RedisServerVersion>),
    /// 新建 Key 成功：`tab_id` 为目标库键列表页，`object` 为目标库（`RedisDb`）路径，
    /// `key` 为新键名，供 UI 关闭抽屉、刷新列表并打开新键详情。
    RedisKeyCreated {
        tab_id: TabId,
        object: ObjectPath,
        key: String,
    },
    BinaryPreviewLoaded {
        tab_id: TabId,
        row: usize,
        column: usize,
        preview: BinaryPreviewResponse,
    },
    BinaryCellDownloaded {
        tab_id: TabId,
        row: usize,
        column: usize,
        bytes: Vec<u8>,
    },
    QueryCompletionsLoaded(TabId, u64, QueryCompletionResult),
    CompletionIndexWarmed(ConnectionId, Option<String>),
    QueryFinished(TabId, QueryExecutionResult),
    RedisWorkbenchFinished(TabId, CommandWorkbenchExecution),
    CreateTableApplied(TabId),
    TableRenamed {
        object: ObjectPath,
        new_name: String,
    },
    TableCopied {
        object: ObjectPath,
        new_name: String,
    },
    TableDropped(ObjectPath),
    TableTruncated(ObjectPath),
    CreateTableReferenceColumnsLoaded {
        tab_id: TabId,
        foreign_key_id: u64,
        columns: Vec<String>,
    },
    QueryResultPageRefreshed {
        tab_id: TabId,
        result_index: usize,
        page_index: usize,
        page: DataPage,
    },
    TabOpened(TabId),
    TabClosed(TabId),
    TabCloseRequested(TabId),
    TabCloseCancelled(TabId),
    TabActivated(TabId),
    UserAdminUsersLoaded(TabId, Vec<DatabaseUserIdentity>),
    UserAdminGrantsLoaded(TabId, Vec<String>),
    UserAdminMemberGrantsLoaded(TabId, Vec<UserRoleMember>),
    UserAdminSqlApplied(TabId),
    TableInfoLoaded(TabId, TableInfoTab, TableInfoResult),
    TableInfoChanged(TabId, TableInfoTab),
    SettingsSaved,
    /// Redis Workbench 命令历史已变化（新增/删除/清空），供桌面层做持久化 diff 与刷新。
    RedisWorkbenchHistoryChanged,
    Failed(UserFacingError),
}

#[derive(Clone, Debug)]
struct CompletionCacheEntry<T> {
    value: T,
    fetched_at: u64,
}

impl<T> CompletionCacheEntry<T> {
    fn is_fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.fetched_at) <= COMPLETION_INDEX_TTL_SECONDS
    }
}

#[derive(Debug, Default)]
struct CompletionCache {
    tables: BTreeMap<CompletionTablesKey, CompletionCacheEntry<Vec<CompletionTable>>>,
    columns: BTreeMap<CompletionColumnsKey, CompletionCacheEntry<Vec<CompletionColumn>>>,
    routines: BTreeMap<CompletionRoutinesKey, CompletionCacheEntry<Vec<CompletionRoutine>>>,
    triggers: BTreeMap<CompletionTriggersKey, CompletionCacheEntry<Vec<CompletionTrigger>>>,
    /// 按表缓存的真实外键元数据（P2.13 FK JOIN）。
    foreign_keys:
        BTreeMap<CompletionColumnsKey, CompletionCacheEntry<Vec<fluxdb_core::ForeignKeyInfo>>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionTablesKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionColumnsKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
    table: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionRoutinesKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompletionTriggersKey {
    connection_id: ConnectionId,
    database: Option<String>,
    schema: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AppController {
    state: AppState,
    next_connection_id: u64,
    next_group_id: u64,
    next_tab_id: u64,
    completion_cache: Arc<Mutex<CompletionCache>>,
    completion_index: Arc<Mutex<CompletionIndex>>,
    completion_index_storage: Option<fluxdb_storage::FileStorage>,
    /// T071/F004：可关闭的轻量个性化（recency/frequency）。默认关闭，
    /// 关闭时排序与确定性基线一致。只记匿名 label，不记完整 SQL/敏感值。
    recency: Arc<Mutex<RecencyFrequency>>,
}
