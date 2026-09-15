// PostgreSQL 用户与角色工作台的结构化变更计划（T27 改版）。
//
// UI 的所有编辑先进入 `PgRoleDraft`，保存时由 App 层 diff 出 `PgRoleSavePlan`；
// SQL 渲染与执行在 connector（`pg_render_role_plan` / `pg_apply_role_plan`），
// UI 不拼 SQL。密码只以明文存在于内存中的 draft/change，`Debug` 手工实现做脱敏，
// 保证日志/错误路径不会带出密码（设计 §12）。

/// 角色密码操作：保持不变 / 设置新密码 / 清除（PASSWORD NULL）。
/// 空白输入不等于清除——清除必须是显式操作。
#[derive(Clone, Default, PartialEq, Eq)]
pub enum PgPasswordOp {
    #[default]
    Keep,
    /// 设置为新密码（明文，仅内存态；渲染时按需脱敏）。
    Set(String),
    /// 清除密码：`ALTER ROLE ... PASSWORD NULL`。
    Clear,
}

impl std::fmt::Debug for PgPasswordOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PgPasswordOp::Keep => write!(f, "Keep"),
            // 脱敏：任何 Debug 输出（日志/错误回显）都不得带出明文密码。
            PgPasswordOp::Set(_) => write!(f, "Set(********)"),
            PgPasswordOp::Clear => write!(f, "Clear"),
        }
    }
}

/// 密码有效期（rolvaliduntil）操作：保持 / 清除（显式 infinity）/ 设置绝对时间。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PgValidUntilOp {
    #[default]
    Keep,
    /// 清除已有截止时间：生成显式 `VALID UNTIL 'infinity'`，不能用「不修改」代替。
    Clear,
    /// 设置绝对时间（服务端可解析的时间字面量，含时区）。
    At(String),
}

/// 角色属性集合（变更计划中仅携带被修改的项；`None` = 不修改）。
///
/// `connection_limit`：`None` 不修改；`Some(-1)` 不限制；`Some(n)` 限制 n。
/// `valid_until`：`None` 不修改；`Some(PgValidUntilOp)` 为本次变更。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PgRoleAttributes {
    pub can_login: Option<bool>,
    pub is_superuser: Option<bool>,
    pub can_create_db: Option<bool>,
    pub can_create_role: Option<bool>,
    pub inherit: Option<bool>,
    pub is_replication: Option<bool>,
    pub bypass_rls: Option<bool>,
    pub connection_limit: Option<i32>,
    pub valid_until: Option<PgValidUntilOp>,
}

impl PgRoleAttributes {
    /// 是否没有任何属性变更。
    pub fn is_empty(&self) -> bool {
        *self == PgRoleAttributes::default()
    }
}

/// UI 侧草稿：角色基础字段 + 属性 + 密码操作。新建与编辑共用；
/// 新建时 `create` 为 true（name 为空即待填，避免自动误建）。
#[derive(Clone, Debug, PartialEq)]
pub struct PgRoleDraft {
    /// 是否为「新建角色」草稿（否则为编辑既有角色）。
    pub create: bool,
    /// 角色名（新建必填；编辑时改动即重命名）。
    pub name: String,
    pub can_login: bool,
    pub is_superuser: bool,
    pub can_create_db: bool,
    pub can_create_role: bool,
    pub inherit: bool,
    pub is_replication: bool,
    pub bypass_rls: bool,
    /// 连接数限制文本（输入框直接编辑；"-1" 表示不限，提交时解析校验）。
    pub connection_limit_text: String,
    pub valid_until: PgValidUntilOp,
    pub password: PgPasswordOp,
}

impl PgRoleDraft {
    /// 由既有角色基线构造编辑草稿（密码未知，恒为 Keep）。
    pub fn from_role(role: &PgRole) -> Self {
        PgRoleDraft {
            create: false,
            name: role.name.clone(),
            can_login: role.can_login,
            is_superuser: role.is_superuser,
            can_create_db: role.can_create_db,
            can_create_role: role.can_create_role,
            inherit: role.inherit,
            is_replication: role.is_replication,
            bypass_rls: role.bypass_rls,
            connection_limit_text: role.connection_limit.to_string(),
            valid_until: PgValidUntilOp::Keep,
            password: PgPasswordOp::Keep,
        }
    }

    /// 新建草稿：角色名留空待填，默认 LOGIN + INHERIT、不限连接、不设密码。
    pub fn new_create() -> Self {
        PgRoleDraft {
            create: true,
            name: String::new(),
            can_login: true,
            is_superuser: false,
            can_create_db: false,
            can_create_role: false,
            inherit: true,
            is_replication: false,
            bypass_rls: false,
            connection_limit_text: "-1".to_string(),
            valid_until: PgValidUntilOp::Keep,
            password: PgPasswordOp::Keep,
        }
    }

    /// 解析连接数限制文本；空或非法返回 None（由保存入口报错）。
    pub fn parsed_connection_limit(&self) -> Option<i32> {
        self.connection_limit_text.trim().parse::<i32>().ok()
    }
}

/// 单条变更：创建 / 重命名 / 属性 / 密码 / 成员 / 对象授权。
///
/// 执行顺序由 connector 保证：Create → Rename → AlterAttributes/SetPassword →
/// GrantMembership/RevokeMembership → GrantObject/RevokeObject；改名后的操作引用新名。
#[derive(Clone, PartialEq, Eq)]
pub enum PgRoleChange {
    /// 创建角色（可带初始属性与密码）。
    Create {
        name: String,
        can_login: bool,
        password: PgPasswordOp,
        attributes: PgRoleAttributes,
    },
    /// 重命名（RENAME TO 只接单段新名）。
    Rename { from: String, to: String },
    /// 修改角色属性（仅携带变更项）。
    AlterAttributes { name: String, attributes: PgRoleAttributes },
    /// 设置/清除密码：`Some(pw)` 设置；`None` 生成 `PASSWORD NULL`。
    SetPassword { name: String, password: Option<String> },
    /// 授予成员关系（GRANT role TO member [+ ADMIN/INHERIT/SET 选项]）。
    GrantMembership {
        role: String,
        member: String,
        admin: bool,
        inherit: bool,
        set: bool,
    },
    /// 撤销成员关系（REVOKE role FROM member）。
    RevokeMembership { role: String, member: String },
    /// 对象授权 GRANT privilege ON <scope> TO grantee [WITH GRANT OPTION]。
    GrantObject {
        privilege: String,
        scope: PgObjectGrantScope,
        grantee: String,
        grant_option: bool,
    },
    /// 撤销基础权限 REVOKE privilege ON <scope> FROM grantee。
    RevokeObject {
        privilege: String,
        scope: PgObjectGrantScope,
        grantee: String,
    },
    /// 仅取消可再授权（REVOKE GRANT OPTION FOR ...），不撤销基础权限。
    RevokeGrantOption {
        privilege: String,
        scope: PgObjectGrantScope,
        grantee: String,
    },
}

impl std::fmt::Debug for PgRoleChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 手工 Debug：SetPassword/Create 的密码字段一律脱敏。
        match self {
            PgRoleChange::Create { name, can_login, password, attributes } => f
                .debug_struct("Create")
                .field("name", name)
                .field("can_login", can_login)
                .field("password", password)
                .field("attributes", attributes)
                .finish(),
            PgRoleChange::SetPassword { name, password } => f
                .debug_struct("SetPassword")
                .field("name", name)
                .field(
                    "password",
                    &password.as_ref().map(|_| "********".to_string()),
                )
                .finish(),
            other => write!(f, "{other:?}"),
        }
    }
}

/// 本次保存的完整计划：目标数据库（对象 ACL 读写库；角色 DDL/成员关系为集群级，
/// 在同一连接执行）+ 有序变更列表。一期同批只涉及一个数据库的对象授权。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PgRoleSavePlan {
    /// 对象授权所在数据库；None 表示本批无对象授权（或仅集群级变更），用维护库连接。
    pub database: Option<String>,
    /// 受影响角色（改名后为最终名），供预览摘要与确认提示。
    pub role_name: String,
    pub changes: Vec<PgRoleChange>,
}

impl PgRoleSavePlan {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// 权限页目标列表（`schema.name` 形式字符串；函数带 `(参数类型列表)` 签名区分重载）。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PgGrantTargetLists {
    pub databases: Vec<String>,
    pub schemas: Vec<String>,
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub sequences: Vec<String>,
    /// `schema.name(signature)`。
    pub routines: Vec<String>,
}

/// 由基线与草稿 diff 出属性变更（纯函数，便于测试）。
///
/// `create=true` 时所有属性都随 CREATE 下发；`connection_limit_text` 解析失败返回 None。
pub fn pg_role_attributes_diff(baseline: Option<&PgRole>, draft: &PgRoleDraft) -> Option<PgRoleAttributes> {
    let limit = match draft.parsed_connection_limit() {
        Some(limit) => limit,
        None => return None,
    };
    let mut attrs = PgRoleAttributes::default();
    let mut changed = false;
    let mut push = |field: &mut Option<bool>, base: bool, next: bool, create: bool| {
        if create || base != next {
            *field = Some(next);
            changed = true;
        }
    };
    match baseline {
        None => {
            // 新建：全部属性随 CREATE ROLE 下发（未显式给出的用服务端默认）。
            changed = true;
            attrs.can_login = Some(draft.can_login);
            attrs.is_superuser = Some(draft.is_superuser);
            attrs.can_create_db = Some(draft.can_create_db);
            attrs.can_create_role = Some(draft.can_create_role);
            attrs.inherit = Some(draft.inherit);
            attrs.is_replication = Some(draft.is_replication);
            attrs.bypass_rls = Some(draft.bypass_rls);
            if limit != -1 {
                attrs.connection_limit = Some(limit);
            }
        }
        Some(base) => {
            push(&mut attrs.can_login, base.can_login, draft.can_login, false);
            push(&mut attrs.is_superuser, base.is_superuser, draft.is_superuser, false);
            push(&mut attrs.can_create_db, base.can_create_db, draft.can_create_db, false);
            push(&mut attrs.can_create_role, base.can_create_role, draft.can_create_role, false);
            push(&mut attrs.inherit, base.inherit, draft.inherit, false);
            push(&mut attrs.is_replication, base.is_replication, draft.is_replication, false);
            push(&mut attrs.bypass_rls, base.bypass_rls, draft.bypass_rls, false);
            if base.connection_limit != limit {
                attrs.connection_limit = Some(limit);
                changed = true;
            }
        }
    }
    // valid_until 的基线是字符串（rolvaliduntil::text），草稿只有 Keep/Clear/At；
    // Clear/At 均视为变更（Keep 不动）。
    match &draft.valid_until {
        PgValidUntilOp::Keep => {}
        PgValidUntilOp::Clear => {
            attrs.valid_until = Some(PgValidUntilOp::Clear);
            changed = true;
        }
        PgValidUntilOp::At(value) => {
            // 与基线相同文本则视为未变，避免无意义重写。
            let same = baseline
                .and_then(|b| b.valid_until.as_deref())
                .is_some_and(|base| base == value.as_str());
            if !same {
                attrs.valid_until = Some(PgValidUntilOp::At(value.clone()));
                changed = true;
            }
        }
    }
    changed.then_some(attrs)
}
