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

impl TabState {
    /// 返回标签页所属的「连接 + 库」工作区；首页打开的全局标签没有归属。
    pub fn workspace(&self) -> Option<TabWorkspace> {
        match &self.kind {
            TabKind::DataEditor(editor) => Some(TabWorkspace {
                connection_id: editor.object.connection_id,
                database: editor
                    .object
                    .database
                    .clone()
                    .unwrap_or_else(|| "main".to_string()),
            }),
            TabKind::QueryEditor(editor) => Some(TabWorkspace {
                connection_id: editor.connection_id,
                database: editor
                    .database
                    .clone()
                    .unwrap_or_else(|| "默认库".to_string()),
            }),
            TabKind::RedisWorkbench(workbench) => Some(TabWorkspace {
                connection_id: workbench.connection_id,
                database: workbench.database.to_string(),
            }),
            TabKind::RedisCli(cli) => Some(TabWorkspace {
                connection_id: cli.connection_id,
                database: cli.database.to_string(),
            }),
            TabKind::RedisPubSub(pubsub) => Some(TabWorkspace {
                connection_id: pubsub.connection_id,
                database: pubsub.database.to_string(),
            }),
            TabKind::CreateTable(create) => Some(TabWorkspace {
                connection_id: create.connection_id,
                database: create
                    .database
                    .clone()
                    .unwrap_or_else(|| "默认库".to_string()),
            }),
            TabKind::ObjectList(list) => list.parent.as_ref().map(|parent| TabWorkspace {
                connection_id: parent.connection_id,
                database: parent
                    .database
                    .clone()
                    .unwrap_or_else(|| parent.name.clone()),
            }),
            TabKind::UserAdmin(admin) => Some(TabWorkspace {
                connection_id: admin.connection_id,
                database: "默认库".to_string(),
            }),
            TabKind::BackupList(list) => Some(TabWorkspace {
                connection_id: list.connection_id,
                database: list.database.clone(),
            }),
            TabKind::Settings(settings) => settings.workspace.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TabWorkspace {
    pub connection_id: ConnectionId,
    pub database: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsTabState {
    pub workspace: Option<TabWorkspace>,
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
    Settings(SettingsTabState),
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
    /// 待确认删除的用户（MySQL 用户与权限删除确认弹框）。
    pub pending_delete_user: Option<DatabaseUserIdentity>,
    /// PostgreSQL 对象权限（T27）：授权目标种类 + schema/对象/函数签名。
    pub pg_grant_kind: PgGrantObjectKind,
    pub pg_grant_schema: String,
    pub pg_grant_object: String,
    pub pg_grant_signature: String,
    /// 选中对象权限的读取结果（owner/默认/显式条目）与角色生效权限（直接 vs 继承）。
    pub pg_object_grants: Option<PgObjectGrants>,
    pub pg_effective_grants: Vec<PgEffectivePrivilege>,
    pub loading_pg_grants: bool,
    pub pg_grants_error: Option<UserFacingError>,
    // ===== PG 用户与角色工作台（改版）：角色列表 / 草稿 / 成员 / 授权草稿 / 保存状态 =====
    /// 全量角色列表（权威数据，替代 DatabaseUserIdentity[host=""] 承载）。
    pub pg_roles: Vec<PgRole>,
    /// 角色列表加载错误（失败不显示为「无角色」）。
    pub pg_roles_error: Option<UserFacingError>,
    /// 当前选中角色名。
    pub pg_selected_role: Option<String>,
    /// 当前编辑/新建草稿；None 表示无未保存编辑会话。
    pub pg_draft: Option<PgRoleDraft>,
    /// 全量成员关系（pg_auth_members），供「所属角色 / 此角色的成员」双向展示。
    pub pg_memberships: Vec<PgRoleMembership>,
    /// 成员关系是否已加载完成（区分「无成员」与「未加载」）。
    pub pg_memberships_loaded: bool,
    /// 成员关系草稿变更（Grant/Revoke），保存时并入变更计划。
    pub pg_membership_edits: Vec<PgRoleChange>,
    /// 权限页目标数据库（一期同批只允许一个数据库的对象授权）。
    pub pg_grant_database: String,
    /// 权限页可选目标（数据库/schema/表·视图·序列/函数）。
    pub pg_grant_targets: Option<PgGrantTargetLists>,
    pub pg_loading_targets: bool,
    pub pg_targets_error: Option<UserFacingError>,
    /// 对象授权草稿变更（Grant/Revoke/RevokeGrantOption）。
    pub pg_grant_edits: Vec<PgRoleChange>,
    /// 保存状态：空闲 / 提交中 / 结果待核实（提交时断线等不确定结果）。
    pub pg_save_status: PgRoleSaveStatus,
    /// 变更计划构建/渲染错误。
    pub pg_plan_error: Option<UserFacingError>,
    /// SQL 预览页内容（脱敏渲染结果）；None 表示尚未生成。
    pub pg_plan_preview: Option<Vec<String>>,
    /// 预览生成中（后台任务未返回）。
    pub pg_preview_loading: bool,
    /// 预览中是否包含被脱敏的密码语句（UI 标注「不能直接执行」）。
    pub pg_plan_preview_masked: bool,
    /// 角色列表筛选：all / login / nologin / predefined。
    pub pg_role_filter: String,
    /// 待确认删除的角色（PG 删除确认弹框）。
    pub pg_pending_delete: Option<String>,
    /// 有草稿时切换角色：待确认的目标角色（确认后丢弃草稿并切换）。
    pub pg_pending_switch: Option<String>,
    /// 服务端是否支持成员级 INHERIT/SET 选项（PG16+；None = 未探测）。
    pub pg_member_options_supported: Option<bool>,
    /// 最近一次对象权限读取的目标指纹（kind|schema|object|signature），用于过期检测。
    pub pg_loaded_target: String,
}

/// PG 变更计划保存状态。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PgRoleSaveStatus {
    #[default]
    Idle,
    /// 提交中（禁用重复保存）。
    Saving,
    /// 提交结果不确定（如提交时断线）：先重新读取核实，不直接重试。
    NeedsVerify,
}

/// PG 草稿布尔属性字段（高级页开关行）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PgDraftAttrField {
    IsSuperuser,
    CanCreateDb,
    CanCreateRole,
    Inherit,
    IsReplication,
    BypassRls,
}

/// PG 对象授权草稿操作。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PgGrantEditOp {
    /// 授予权限（grant_option 决定是否带 GRANT OPTION）。
    Grant,
    /// 撤销基础权限。
    Revoke,
    /// 仅取消可再授权（REVOKE GRANT OPTION FOR），不动基础权限。
    RevokeGrantOption,
}

/// PgRoleDraft 的 UI 扩展（app 层专属展示辅助；外部类型不能写 inherent impl，用 trait 扩展）。
pub trait PgRoleDraftUiExt {
    /// 读取布尔属性字段（UI 开关行通用取值）。
    fn attr(&self, field: PgDraftAttrField) -> bool;
    /// 密码操作下拉的当前标签（与 UI 选项一致）。
    fn password_label(&self) -> String;
    /// 密码有效期模式下拉的当前标签。
    fn valid_until_mode_label(&self) -> String;
}

impl PgRoleDraftUiExt for fluxdb_core::PgRoleDraft {
    fn attr(&self, field: PgDraftAttrField) -> bool {
        match field {
            PgDraftAttrField::IsSuperuser => self.is_superuser,
            PgDraftAttrField::CanCreateDb => self.can_create_db,
            PgDraftAttrField::CanCreateRole => self.can_create_role,
            PgDraftAttrField::Inherit => self.inherit,
            PgDraftAttrField::IsReplication => self.is_replication,
            PgDraftAttrField::BypassRls => self.bypass_rls,
        }
    }

    fn password_label(&self) -> String {
        if self.create {
            match &self.password {
                PgPasswordOp::Set(_) => "设置新密码".to_string(),
                _ => "不设置密码".to_string(),
            }
        } else {
            match &self.password {
                PgPasswordOp::Set(_) => "设置新密码".to_string(),
                PgPasswordOp::Clear => "清除密码".to_string(),
                PgPasswordOp::Keep => "保持不变".to_string(),
            }
        }
    }

    fn valid_until_mode_label(&self) -> String {
        match &self.valid_until {
            PgValidUntilOp::Clear => "清除截止时间（永不过期）".to_string(),
            PgValidUntilOp::At(_) => "自定义截止时间…".to_string(),
            PgValidUntilOp::Keep => "保持不变".to_string(),
        }
    }
}

#[allow(dead_code)]
fn _assert_pg_role_draft_ui_ext_usable() {
    // 仅为保证 trait 扩展编译可用；不参与运行时逻辑。
}

/// PG 对象权限授权目标种类（UI 选择器用；映射到 `PgObjectGrantScope`）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PgGrantObjectKind {
    #[default]
    Table,
    View,
    Sequence,
    Schema,
    Database,
    Routine,
}

impl PgGrantObjectKind {
    pub fn label(self) -> &'static str {
        match self {
            PgGrantObjectKind::Table => "表",
            PgGrantObjectKind::View => "视图",
            PgGrantObjectKind::Sequence => "序列",
            PgGrantObjectKind::Schema => "schema",
            PgGrantObjectKind::Database => "数据库",
            PgGrantObjectKind::Routine => "函数",
        }
    }

    pub fn all() -> [PgGrantObjectKind; 6] {
        [
            PgGrantObjectKind::Table,
            PgGrantObjectKind::View,
            PgGrantObjectKind::Sequence,
            PgGrantObjectKind::Schema,
            PgGrantObjectKind::Database,
            PgGrantObjectKind::Routine,
        ]
    }
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
            pending_delete_user: None,
            pg_grant_kind: PgGrantObjectKind::default(),
            pg_grant_schema: String::new(),
            pg_grant_object: String::new(),
            pg_grant_signature: String::new(),
            pg_object_grants: None,
            pg_effective_grants: Vec::new(),
            loading_pg_grants: false,
            pg_grants_error: None,
            pg_roles: Vec::new(),
            pg_roles_error: None,
            pg_selected_role: None,
            pg_draft: None,
            pg_memberships: Vec::new(),
            pg_memberships_loaded: false,
            pg_membership_edits: Vec::new(),
            pg_grant_database: String::new(),
            pg_grant_targets: None,
            pg_loading_targets: false,
            pg_targets_error: None,
            pg_grant_edits: Vec::new(),
            pg_save_status: PgRoleSaveStatus::Idle,
            pg_plan_error: None,
            pg_plan_preview: None,
            pg_preview_loading: false,
            pg_plan_preview_masked: false,
            pg_role_filter: "all".to_string(),
            pg_pending_delete: None,
            pg_pending_switch: None,
            pg_member_options_supported: None,
            pg_loaded_target: String::new(),
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

    // ===== PG 用户与角色工作台辅助（改版）=====

    /// 选中角色的基线数据（来自 pg_roles）；新建草稿时为 None。
    pub fn pg_baseline_role(&self) -> Option<&PgRole> {
        let selected = self.pg_selected_role.as_deref()?;
        self.pg_roles.iter().find(|role| role.name == selected)
    }

    /// 从基线派生编辑草稿（选中角色后右侧面板以草稿渲染）。
    /// 干净草稿与基线 diff 为空，不计入未保存变更。
    pub fn pg_reset_draft_from_baseline(&mut self) {
        self.pg_draft = self.pg_baseline_role().map(PgRoleDraft::from_role);
    }

    /// 清空权限页目标选择及其读取结果；数据库/Schema/对象都必须由用户重新显式选择。
    pub fn pg_reset_grant_target_session(&mut self) {
        self.pg_grant_kind = PgGrantObjectKind::default();
        self.pg_grant_database.clear();
        self.pg_grant_schema.clear();
        self.pg_grant_object.clear();
        self.pg_grant_signature.clear();
        self.pg_grant_targets = None;
        self.pg_loading_targets = false;
        self.pg_targets_error = None;
        self.pg_object_grants = None;
        self.pg_effective_grants.clear();
        self.loading_pg_grants = false;
        self.pg_grants_error = None;
        self.pg_grant_edits.clear();
        self.pg_loaded_target.clear();
    }

    /// 右侧角色编辑会话回到初始态：选中角色/角色列表保留，目标、成员、草稿编辑和预览清空。
    pub fn pg_reset_role_editor_session(&mut self) {
        self.pg_membership_edits.clear();
        self.pg_memberships.clear();
        self.pg_memberships_loaded = false;
        self.pg_reset_grant_target_session();
        self.pg_pending_switch = None;
        self.pg_plan_preview = None;
        self.pg_preview_loading = false;
        self.pg_plan_preview_masked = false;
        self.pg_plan_error = None;
        self.active_detail_tab = UserAdminDetailTab::General;
    }

    /// 预定义角色（pg_ 前缀）：不提供属性编辑/删除，仅可查看权限与按授权能力管理成员。
    pub fn pg_is_predefined_role(name: &str) -> bool {
        name.starts_with("pg_")
    }

    /// 草稿角色名（新建待填时为空串）。
    pub fn pg_draft_name(&self) -> Option<&str> {
        self.pg_draft.as_ref().map(|draft| draft.name.trim())
    }

    /// 授权/成员变更的受影响角色名：新建草稿用草稿名（保存时先建角色再授权），
    /// 编辑既有角色用选中名。
    pub fn pg_effective_grantee_name(&self) -> String {
        self.pg_draft_name()
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .or_else(|| self.pg_selected_role.clone())
            .unwrap_or_default()
    }

    /// 是否存在未保存的草稿变更（常规/高级/成员/授权任一）。
    ///
    /// 纯函数判断：与基线 diff + 密码/有效期操作 + 成员与授权草稿列表非空。
    pub fn pg_has_draft_changes(&self) -> bool {
        let Some(draft) = &self.pg_draft else {
            return false;
        };
        if !self.pg_membership_edits.is_empty() || !self.pg_grant_edits.is_empty() {
            return true;
        }
        if draft.password != PgPasswordOp::Keep || draft.valid_until != PgValidUntilOp::Keep {
            return true;
        }
        if draft.create {
            return true;
        }
        let Some(baseline) = self.pg_baseline_role() else {
            return !draft.name.trim().is_empty();
        };
        draft.name.trim() != baseline.name
            || fluxdb_core::pg_role_attributes_diff(Some(baseline), draft).is_some()
    }

    /// 「所属角色」：当前角色直接所在的组角色（读取 pg_auth_members 过滤）。
    pub fn pg_member_of_roles(&self) -> Vec<&PgRoleMembership> {
        let selected = self.pg_selected_role.as_deref();
        self.pg_memberships
            .iter()
            .filter(|m| Some(m.member.as_str()) == selected)
            .collect()
    }

    /// 「此角色的成员」：把当前角色授予了哪些成员。
    pub fn pg_members_of_role(&self) -> Vec<&PgRoleMembership> {
        let selected = self.pg_selected_role.as_deref();
        self.pg_memberships
            .iter()
            .filter(|m| Some(m.grantee.as_str()) == selected)
            .collect()
    }

    /// 成员关系草稿是否已包含对该 (role, member) 的变更（避免重复条目）。
    pub fn pg_membership_edit_index(&self, role: &str, member: &str) -> Option<usize> {
        self.pg_membership_edits.iter().position(|edit| match edit {
            PgRoleChange::GrantMembership { role: r, member: m, .. }
            | PgRoleChange::RevokeMembership { role: r, member: m } => r == role && m == member,
            _ => false,
        })
    }

    /// 对象授权草稿定位：同 privilege + 同 scope 的既有变更条目。
    pub fn pg_grant_edit_index(&self, privilege: &str, scope: &PgObjectGrantScope) -> Option<usize> {
        self.pg_grant_edits.iter().position(|edit| match edit {
            PgRoleChange::GrantObject { privilege: p, scope: s, .. }
            | PgRoleChange::RevokeObject { privilege: p, scope: s, .. }
            | PgRoleChange::RevokeGrantOption { privilege: p, scope: s, .. } => {
                p == privilege && s == scope
            }
            _ => false,
        })
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

include!("query_history_state.rs");

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
    PrepareBackup { request: BackupRequest, method: BackupMethod },
    RunBackup(BackupRequest),
    PrepareRestore(RestoreRequest),
    ProbeRestore(RestoreRequest),
    RunRestore { request: RestoreRequest, plan: RestorePlan },
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
    CreateSchema {
        connection_id: ConnectionId,
        /// 目标数据库（PG 的 schema 属于库）；空串表示回退连接维护库。
        database: String,
        schema: String,
    },
    /// 角色管理（T26）：列表 + CRUD（PG 集群级角色）。
    LoadPgRoles(ConnectionId),
    CreatePgRole {
        connection_id: ConnectionId,
        name: String,
        can_login: bool,
        password: Option<String>,
    },
    AlterPgRolePassword {
        connection_id: ConnectionId,
        name: String,
        password: String,
    },
    RenamePgRole {
        connection_id: ConnectionId,
        old_name: String,
        new_name: String,
    },
    DropPgRole {
        connection_id: ConnectionId,
        name: String,
    },
    DeleteDatabase {
        connection_id: ConnectionId,
        database: String,
    },
    LoadObjectChildren(ObjectPath),
    RefreshObject(Option<ObjectPath>),
    /// 侧边栏「刷新连接树」：只重拉**已展开**连接的第一层对象（库 / Schema / RedisDb）。
    /// 不写 `connected` / `expanded`，也不触碰已加载的表 / 视图行（详见 `replace_connection_level0`）。
    RefreshConnectionTree,
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
        /// PG：是否 RESTART IDENTITY（默认 false = CONTINUE IDENTITY）。
        restart_identity: bool,
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
    /// 请求停止当前标签正在执行的查询（工具栏「停止」）。置位该标签的取消标志，
    /// 执行线程在下一次检查点向服务端发送取消（PG CancelToken），不会强杀执行线程。
    CancelQueryExecution(TabId),
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
    /// Redis Workbench 执行的准备步：只置运行态并按需清空顶部草稿，不做网络 I/O。
    ///
    /// `execution_id` 给定时重跑结果区该条记录（保留顶部草稿），为 None 时执行顶部草稿。
    /// UI 紧接着在后台线程派发 [`AppCommand::RunRedisWorkbench`]，再用
    /// [`AppCommand::FinishRedisWorkbenchExecution`] 把结果落回主线程。
    BeginRedisWorkbenchExecution {
        tab_id: TabId,
        execution_id: Option<u64>,
    },
    /// 执行 Redis 命令：在后台线程的控制器副本上跑，只读标签页作用域，不改动任何标签页状态。
    RunRedisWorkbench {
        tab_id: TabId,
        execution_id: Option<u64>,
    },
    FinishRedisWorkbenchExecution {
        tab_id: TabId,
        /// 本次实际执行的命令文本：失败入历史时要用，不能取执行期间的草稿。
        text: String,
        /// 一次运行按「一条命令一条记录」产出多条执行记录。
        result: std::result::Result<Vec<CommandWorkbenchExecution>, UserFacingError>,
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
    /// 结束「新建」态（取消新建或提交完成），回到选中既有对象。
    EndUserAdminCreateUser(TabId),
    // ===== PG 用户与角色工作台（改版）=====
    /// 切换选中 PG 角色（UI 在有草稿时先弹确认，确认后走 DiscardPgDraftAndSelect）。
    SelectPgRole {
        tab_id: TabId,
        name: String,
    },
    /// 挂起「切换角色」等待用户确认丢弃草稿。
    SetPgRoleSwitchPending {
        tab_id: TabId,
        target: String,
    },
    /// 取消切换（留在当前角色，草稿保留）。
    PgCancelSwitchRole(TabId),
    /// 丢弃当前草稿并切换到目标角色（切换确认框确认后）。
    DiscardPgDraftAndSelect {
        tab_id: TabId,
        name: String,
    },
    /// 开始「新建角色」草稿（角色名留空待填，不自动误建）。
    PgBeginCreateRole(TabId),
    /// 取消/关闭当前草稿（仅清理本地草稿，不写库）。
    PgCancelDraft(TabId),
    SetPgDraftName {
        tab_id: TabId,
        name: String,
    },
    SetPgDraftCanLogin {
        tab_id: TabId,
        can_login: bool,
    },
    /// 布尔属性草稿切换（SUPERUSER/CREATEDB/CREATEROLE/INHERIT/REPLICATION/BYPASSRLS）。
    SetPgDraftAttr {
        tab_id: TabId,
        field: PgDraftAttrField,
        value: bool,
    },
    SetPgDraftConnectionLimit {
        tab_id: TabId,
        value: String,
    },
    /// 密码有效期操作：Keep/Clear(infinity)/At(绝对时间)。
    SetPgDraftValidUntil {
        tab_id: TabId,
        op: PgValidUntilOp,
    },
    /// 密码操作：Keep/Set/Clear。
    SetPgDraftPasswordOp {
        tab_id: TabId,
        op: PgPasswordOp,
    },
    /// 录入新密码明文（仅内存；写入 Set 操作）。
    SetPgDraftPassword {
        tab_id: TabId,
        password: String,
    },
    /// 开始加载 PG 角色列表（权威 pg_roles 数据）。
    StartUserAdminPgRolesLoad(TabId),
    LoadUserAdminPgRoles(TabId),
    FinishUserAdminPgRolesLoad {
        tab_id: TabId,
        result: Result<Vec<PgRole>, UserFacingError>,
    },
    /// 加载全量成员关系（双向展示用；同时探测服务端成员选项版本支持）。
    StartPgMembershipsLoad(TabId),
    LoadPgMemberships(TabId),
    FinishPgMembershipsLoad {
        tab_id: TabId,
        result: Result<Vec<PgRoleMembership>, UserFacingError>,
        member_options_supported: bool,
    },
    /// 成员关系草稿：授予 role → member（含 ADMIN/INHERIT/SET 选项）。
    PgMembershipGrant {
        tab_id: TabId,
        role: String,
        member: String,
        admin: bool,
        inherit: bool,
        set: bool,
    },
    /// 成员关系草稿：撤销 role ← member。
    PgMembershipRevoke {
        tab_id: TabId,
        role: String,
        member: String,
    },
    /// 移除一条成员关系草稿变更。
    PgMembershipRemoveEdit {
        tab_id: TabId,
        index: usize,
    },
    /// 权限页：切换目标数据库（一期同批只允许一个数据库）。
    SetPgGrantDatabase {
        tab_id: TabId,
        database: String,
    },
    /// 开始加载权限页目标列表（数据库/schema/对象/函数）。
    StartPgGrantTargetsLoad(TabId),
    LoadPgGrantTargets {
        tab_id: TabId,
        database: String,
    },
    FinishPgGrantTargetsLoad {
        tab_id: TabId,
        result: Result<PgGrantTargetLists, UserFacingError>,
    },
    /// 对象授权草稿：授予/撤销/仅取消可再授权。
    PgToggleGrant {
        tab_id: TabId,
        privilege: String,
        scope: PgObjectGrantScope,
        /// Grant（可带 grant_option）/ Revoke / RevokeGrantOption。
        op: PgGrantEditOp,
        grant_option: bool,
    },
    /// 移除一条对象授权草稿变更。
    PgRemoveGrantEdit {
        tab_id: TabId,
        index: usize,
    },
    /// 生成脱敏 SQL 预览（渲染与执行共用 connector 规则）。
    StartPgPlanPreview(TabId),
    LoadPgPlanPreview(TabId),
    FinishPgPlanPreview {
        tab_id: TabId,
        result: Result<Vec<String>, UserFacingError>,
    },
    /// 保存：以单事务应用全部草稿变更（App 内部构建计划并执行）。
    StartPgPlanApply(TabId),
    ApplyPgRolePlan(TabId),
    FinishPgRolePlanApply {
        tab_id: TabId,
        plan: PgRoleSavePlan,
        result: Result<Vec<String>, UserFacingError>,
    },
    /// 删除角色确认框。
    PgBeginDeleteRole(TabId),
    PgCancelDeleteRole(TabId),
    /// 角色列表筛选：all / login / nologin / predefined。
    SetPgRoleFilter {
        tab_id: TabId,
        filter: String,
    },
    /// PostgreSQL 对象权限：设置授权目标（种类/schema/对象/签名）。
    SetUserAdminPgGrantTarget {
        tab_id: TabId,
        kind: PgGrantObjectKind,
        schema: String,
        object: String,
        signature: String,
    },
    /// 读取当前授权目标的对象权限（owner/默认/显式条目 + 选中角色生效权限）。
    LoadUserAdminPgObjectGrants(TabId),
    /// 置对象权限面板为「加载中」。
    StartUserAdminPgObjectGrantsLoad(TabId),
    /// 对象权限读取完成回填（Ok 为对象读模型+生效权限；Err 为失败）。
    FinishUserAdminPgObjectGrantsLoad {
        tab_id: TabId,
        target_fingerprint: String,
        result: Result<(PgObjectGrants, Vec<PgEffectivePrivilege>), UserFacingError>,
    },
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
    /// MySQL：打开「删除用户」确认弹框（针对当前选中的已有用户）。
    BeginUserAdminDeleteUser(TabId),
    /// MySQL：关闭「删除用户」确认弹框（取消）。
    CancelUserAdminDeleteUser(TabId),
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
    BackupPrepared(BackupRequest),
    BackupCompleted(BackupManifest),
    RestorePrepared { request: RestoreRequest, plan: RestorePlan },
    RestoreObjectsProbed(Vec<RestoreObjectProbe>),
    RestoreCompleted(RestoreOutcome),
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
    /// schema 新建成功；UI 据此失效该库 schema 缓存并重取，使新 schema 出现在树。
    SchemaCreated {
        connection_id: ConnectionId,
        schema: String,
    },
    /// PG 角色列表加载完成（T26）。
    PgRolesLoaded(ConnectionId, Vec<fluxdb_core::PgRole>),
    /// PG 角色变更成功（CRUD 后 UI 重取列表）。
    PgRoleChanged(ConnectionId),
    /// PG 对象权限读取完成（对象读模型 + 选中角色生效权限）；Err 为读取失败。
    UserAdminPgObjectGrantsLoaded(
        TabId,
        Result<(PgObjectGrants, Vec<PgEffectivePrivilege>), UserFacingError>,
    ),
    /// PG 对象权限授予/撤销成功（UI 重新读取该对象权限）。
    UserAdminPgGrantsChanged(TabId),
    /// PG 角色列表加载完成回填（改版工作台）。
    UserAdminPgRolesLoaded(TabId, Vec<PgRole>),
    /// PG 成员关系加载完成回填（含服务端是否支持成员级 INHERIT/SET 选项）。
    UserAdminPgMembershipsLoaded(TabId, Vec<PgRoleMembership>, bool),
    /// PG 权限目标列表加载完成回填。
    UserAdminPgGrantTargetsLoaded(TabId, PgGrantTargetLists),
    /// PG 变更计划脱敏预览生成完成。
    UserAdminPgPlanPreview(TabId, Vec<String>),
    /// PG 变更计划应用完成（含计划本体，供 Finish 回填；Err 为失败/结果不确定）。
    UserAdminPgRolePlanFinished(
        TabId,
        PgRoleSavePlan,
        Result<Vec<String>, UserFacingError>,
    ),
    /// PG 变更计划应用成功（UI 刷新角色列表并清空草稿）。
    UserAdminPgRolePlanApplied(TabId),
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
    /// Redis Workbench 后台执行完成：一次运行按「一条命令一条记录」产出多条结果。
    /// 失败也用本事件回传，让实际执行的命令文本随结果一起回到主线程写历史。
    RedisWorkbenchCommandsRan {
        tab_id: TabId,
        text: String,
        result: std::result::Result<Vec<CommandWorkbenchExecution>, UserFacingError>,
    },
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
    /// 已置位某标签的查询取消标志（可能仍在等语句收尾）；UI 据此提示「已请求停止」。
    QueryCancelRequested(TabId),
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
    /// 查询取消标志（按标签）。`StartQueryExecution` 登记新标志，后台执行线程克隆 `Arc`
    /// 后轮询，「停止」按钮经 `CancelQueryExecution` 置位。必须放在共享 `Arc` 里而不是
    /// `AppState`：UI 线程与后台执行线程各持一份 `AppController` 副本，只有共享 `Arc`
    /// 才能让两边看到同一个标志。
    query_cancel_flags: Arc<Mutex<BTreeMap<TabId, Arc<std::sync::atomic::AtomicBool>>>>,
}
