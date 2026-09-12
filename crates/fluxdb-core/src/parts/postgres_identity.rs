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

/// PG 关系种类：决定读取 relacl 时对 relkind 的匹配范围。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PgRelationKind {
    /// 表（普通/分区/外部表；relkind r/p/f）。
    Table,
    /// 视图/物化视图（relkind v/m）。
    View,
    /// 序列（relkind S）。
    Sequence,
}

impl PgRelationKind {
    /// 匹配 pg_class.relkind 的常数 IN 列表。
    pub fn relkind_list(self) -> &'static str {
        match self {
            PgRelationKind::Table => "'r','p','f'",
            PgRelationKind::View => "'v','m'",
            PgRelationKind::Sequence => "'S'",
        }
    }

    /// 该关系种类的候选权限关键字（与 PG 各对象的 effective-acl 权限一致）。
    pub fn effective_privileges(self) -> &'static [&'static str] {
        match self {
            PgRelationKind::Table | PgRelationKind::View => &[
                "SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER",
            ],
            PgRelationKind::Sequence => &["USAGE", "SELECT", "UPDATE"],
        }
    }
}

/// PG 对象授权目标（设计 §12）：数据库 / schema / 表·视图·序列 / 函数（含签名区分重载）。
///
/// 由连接器据此分别查 `pg_database.datacl` / `pg_namespace.nspacl` / `pg_class.relacl` /
/// `pg_proc.proacl`（aclexplode 展开）。Routine 的 `signature` 为 `pg_get_function_identity_arguments`
/// 得到的参数类型串，用于区分同名重载。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PgObjectGrantScope {
    Database {
        database: String,
    },
    Schema {
        schema: String,
    },
    Relation {
        schema: String,
        name: String,
        kind: PgRelationKind,
    },
    Routine {
        schema: String,
        name: String,
        signature: String,
    },
}

/// 一条显式 ACL 展开条目。`grantee` 为空表示 PUBLIC。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PgGrantEntry {
    /// 授权对象（角色名）；空串即 PUBLIC（非普通角色）。
    pub grantee: String,
    /// 权限关键字（SELECT/USAGE/EXECUTE…）。
    pub privilege: String,
    /// 是否带 GRANT OPTION（可再授权）。
    pub grant_option: bool,
    /// 该授权对象是否就是对象 owner（owner 的权限来自属主身份而非 ACL）。
    pub is_owner: bool,
}

/// PG 对象权限的完整读模型（区分默认权限/owner/直接授权/PUBLIC/继承）。
///
/// `acl_is_null=true` 表示对象 ACL 为 NULL，即「默认权限」：owner 全权、其余无显式授权——
/// 这**不等于**「没有任何有效权限」，UI 须明示为默认权限而非空表。`owner` 为对象属主角色。
/// `entries` 只含显式 ACL 条目（直接授权 + PUBLIC），不含纯继承（未显式记录）；继承关系的
/// 推导由更高层基于角色成员关系叠加，避免把继承误当可直接撤销的直接授权。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PgObjectGrants {
    /// 对象属主角色名。
    pub owner: String,
    /// ACL 是否为 NULL（默认权限语义：owner 全权、其余无显式授权）。
    pub acl_is_null: bool,
    /// 显式 ACL 条目（直接授权 + PUBLIC）。
    pub entries: Vec<PgGrantEntry>,
}

/// PG 角色成员关系（`pg_auth_members` 一行）：成员是某个组角色的成员。
///
/// `inherit_option`/`set_option` 为 PG16+ 引入的成员级选项（PG ≤14 无这两列，语义为成员默认
/// INHERIT=true 且可 SET ROLE，读取时按版本回填默认值）。`admin_option` 表示成员可否再授权。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PgRoleMembership {
    /// 组角色（授权方，`roleid`）。
    pub grantee: String,
    /// 成员角色（`member`）。
    pub member: String,
    /// 是否有 ADMIN OPTION（可再授权）。
    pub admin_option: bool,
    /// 成员是否继承组角色权限（PG16+ 列，PG≤14 恒 true）。
    pub inherit_option: bool,
    /// 成员是否可 SET ROLE 到组角色（PG16+ 列，PG≤14 恒 true）。
    pub set_option: bool,
}

/// 某角色对某对象的一种权限的**有效**状态（经 PG 原生 has_*_privilege 判定，天然含 owner/继承/PUBLIC）。
///
/// `effective` 表示该角色实际拥有该权限；`direct` 表示该角色在显式 ACL 中有本条直接授权。
/// 语义组合：
/// - `effective && direct`：直接授权（可直接撤销）；
/// - `effective && !direct`：来自 owner / 继承(PUBLIC 或成员角色) —— 只读展示，**不可直接撤销**，
///   撤销应到来源处（owner 不可撤销；PUBLIC 单独 revoke；继承来自成员角色）；
/// - `!effective`：无权限。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PgEffectivePrivilege {
    /// 权限关键字（SELECT/USAGE/EXECUTE…）。
    pub privilege: String,
    /// 该角色实际是否拥有该权限（含 owner/继承/PUBLIC）。
    pub effective: bool,
    /// 该角色是否在显式 ACL 中直接持有此权限（可直接撤销）。
    pub direct: bool,
    /// 显式 ACL 中该条是否带 GRANT OPTION。
    pub grant_option: bool,
}
