// PostgreSQL 角色/主体身份（T26 增量一）。
//
// PG 主体是集群级 role（LOGIN 与否），不是 user@host；不复用 MySQL 的 DatabaseUserIdentity
// 把 role 塞进 host 字段（设计 §12）。这里用独立 PgRole 承载 role 的属性与连接数/有效期。

/// PostgreSQL 角色（集群级主体，LOGIN 即用户、NOLOGIN 即组角色）。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PgRole {
    /// 角色名（catalog 原样，不折叠大小写）。
    pub name: String,
    /// 是否可登录（rolcanlogin）。
    pub can_login: bool,
    /// 超级用户（rolsuper）。
    pub is_superuser: bool,
    /// 可建库（rolcreatedb）。
    pub can_create_db: bool,
    /// 可建角色（rolcreaterole）。
    pub can_create_role: bool,
    /// 继承父角色权限（rolinherit）。
    pub inherit: bool,
    /// 流复制（rolreplication）。
    pub is_replication: bool,
    /// 绕过行级安全（rolbypassrls）。
    pub bypass_rls: bool,
    /// 连接数限制（-1 表示不限制）。
    pub connection_limit: i32,
    /// 有效截止（UTC，空表示永不过期）。
    pub valid_until: Option<String>,
    /// pg_roles 描述（可选）。
    pub comment: Option<String>,
}

impl PgRole {
    /// 供 UI/历史展示的稳定标识：role 名即其唯一身份。
    pub fn key(&self) -> String {
        self.name.clone()
    }
}
