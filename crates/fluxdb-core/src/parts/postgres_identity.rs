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
