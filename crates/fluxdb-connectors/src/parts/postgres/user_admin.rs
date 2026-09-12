// PostgreSQL 角色/用户管理（T26 增量一）。
//
// 角色是集群级主体（LOGIN=NOLOGIN），DDL 不依赖具体数据库，沿用建库的独立 autocommit
// 连接（`pg_connect` + `batch_execute`）。角色密码语句禁止进入 SQL 历史/日志（本文件只生成
// 语句不写日志，凭密码内容不加日志字段）。角色名/权限关键字一律 `pg_quote_identifier` 引用，
// 密码走专属转义（`quote_pg_string_literal`），不做字符串拼接进参数（PG role DDL 不支持
// 全参数绑定，需明确的字符串引用逻辑，设计 §12）。

use fluxdb_core::{
    PgEffectivePrivilege, PgGrantEntry, PgObjectGrantScope, PgObjectGrants, PgRelationKind,
    PgRole, PgRoleMembership,
};

/// 列出全部角色（pg_roles 自带有效连接数/SUPERUSER 等列，无需逐角色二次查询）。
async fn pg_list_roles_async(
    client: &tokio_postgres::Client,
) -> fluxdb_core::Result<Vec<PgRole>> {
    let rows = client
        .query(
            "SELECT rolname, rolcanlogin::text, rolsuper::text, rolcreatedb::text, \
                    rolcreaterole::text, rolinherit::text, rolreplication::text, \
                    rolbypassrls::text, rolconnlimit, \
                    COALESCE(rolvaliduntil::text, ''), \
                    COALESCE(obj_description(oid, 'pg_roles'), '') \
             FROM pg_roles ORDER BY rolname",
            &[],
        )
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let bool_of = |index: usize| row.get::<_, String>(index) == "true";
            PgRole {
                name: row.get(0),
                can_login: bool_of(1),
                is_superuser: bool_of(2),
                can_create_db: bool_of(3),
                can_create_role: bool_of(4),
                inherit: bool_of(5),
                is_replication: bool_of(6),
                bypass_rls: bool_of(7),
                connection_limit: row.get(8),
                valid_until: {
                    let value: String = row.get(9);
                    (!value.trim().is_empty()).then_some(value)
                },
                comment: {
                    let value: String = row.get(10);
                    (!value.trim().is_empty()).then_some(value)
                },
            }
        })
        .collect())
}

/// 执行一次性角色 DDL（autocommit 独立连接，复用建库口径）。
fn pg_exec_role_sql(config: &ConnectionConfig, sql: &str) -> fluxdb_core::Result<()> {
    let database = pg_request_database(config, None);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        session
            .client
            .batch_execute(sql)
            .await
            .map_err(pg_error)?;
        Ok(())
    })
}

/// PG 单引号字符串字面量转义（`'` → `''`）。仅用于密码/文本 literal，不用于标识符。
fn quote_pg_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 列出角色（同步入口）。
fn pg_list_roles(config: &ConnectionConfig) -> fluxdb_core::Result<Vec<PgRole>> {
    if config.kind != DatabaseKind::Postgres {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持角色管理"));
    }
    let database = pg_request_database(config, None);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        pg_list_roles_async(session.client.as_ref()).await
    })
}

/// 创建角色：LOGIN 用户或 NOLOGIN 组角色；可带密码（LOGIN）。密码字面量独立转义防注入。
fn pg_create_role(
    config: &ConnectionConfig,
    name: &str,
    can_login: bool,
    password: Option<&str>,
) -> fluxdb_core::Result<()> {
    if !is_pg_identifier_name(name) {
        return Err(Error::new(ErrorKind::Query, "角色名不合法"));
    }
    let mut sql = format!("CREATE ROLE {}", pg_quote_identifier(name));
    sql.push_str(if can_login { " LOGIN" } else { " NOLOGIN" });
    sql.push_str(" INHERIT");
    if let Some(password) = password.filter(|p| !p.is_empty()) {
        sql.push_str(&format!(" PASSWORD {}", quote_pg_string_literal(password)));
    }
    sql.push_str(";");
    pg_exec_role_sql(config, &sql)
}

/// 修改角色密码：ALTER ROLE name PASSWORD '...'；密码不落日志/历史。
fn pg_alter_role_password(
    config: &ConnectionConfig,
    name: &str,
    password: &str,
) -> fluxdb_core::Result<()> {
    if name.is_empty() {
        return Err(Error::new(ErrorKind::Query, "角色名不能为空"));
    }
    let sql = format!(
        "ALTER ROLE {} PASSWORD {};",
        pg_quote_identifier(name),
        quote_pg_string_literal(password)
    );
    pg_exec_role_sql(config, &sql)
}

/// 重命名角色：ALTER ROLE old RENAME TO new（RENAME 只接单段新名）。
fn pg_rename_role(
    config: &ConnectionConfig,
    old_name: &str,
    new_name: &str,
) -> fluxdb_core::Result<()> {
    if old_name.is_empty() || new_name.is_empty() {
        return Err(Error::new(ErrorKind::Query, "角色名不能为空"));
    }
    if !is_pg_identifier_name(new_name) {
        return Err(Error::new(ErrorKind::Query, "新角色名不合法"));
    }
    let sql = format!(
        "ALTER ROLE {} RENAME TO {};",
        pg_quote_identifier(old_name),
        pg_quote_identifier(new_name)
    );
    pg_exec_role_sql(config, &sql)
}

/// 删除角色：DROP ROLE（默认 RESTRICT；存在依赖时由服务端报告，不自动 DROP OWNED/CASCADE）。
fn pg_drop_role(config: &ConnectionConfig, name: &str) -> fluxdb_core::Result<()> {
    if name.is_empty() {
        return Err(Error::new(ErrorKind::Query, "角色名不能为空"));
    }
    let sql = format!("DROP ROLE {};", pg_quote_identifier(name));
    pg_exec_role_sql(config, &sql)
}

/// 更新角色属性：SUPERUSER/CREATEDB/CREATEROLE/LOGIN/INHERIT/REPLICATION/BYPASSRLS/CONNECTION LIMIT/VALID UNTIL。
/// 仅对非空/非默认字段生成对应子句；`None` 表示不改该属性。密码单独走 pg_alter_role_password。
fn pg_alter_role_options(
    config: &ConnectionConfig,
    name: &str,
    can_login: Option<bool>,
    is_superuser: Option<bool>,
    can_create_db: Option<bool>,
    can_create_role: Option<bool>,
    inherit: Option<bool>,
    is_replication: Option<bool>,
    bypass_rls: Option<bool>,
    connection_limit: Option<i32>,
    valid_until: Option<&str>,
) -> fluxdb_core::Result<()> {
    if name.is_empty() {
        return Err(Error::new(ErrorKind::Query, "角色名不能为空"));
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(value) = can_login {
        parts.push(if value { "LOGIN".to_string() } else { "NOLOGIN".to_string() });
    }
    if let Some(value) = is_superuser {
        parts.push(if value { "SUPERUSER".to_string() } else { "NOSUPERUSER".to_string() });
    }
    if let Some(value) = can_create_db {
        parts.push(if value { "CREATEDB".to_string() } else { "NOCREATEDB".to_string() });
    }
    if let Some(value) = can_create_role {
        parts.push(if value { "CREATEROLE".to_string() } else { "NOCREATEROLE".to_string() });
    }
    if let Some(value) = inherit {
        parts.push(if value { "INHERIT".to_string() } else { "NOINHERIT".to_string() });
    }
    if let Some(value) = is_replication {
        parts.push(if value { "REPLICATION".to_string() } else { "NOREPLICATION".to_string() });
    }
    if let Some(value) = bypass_rls {
        parts.push(if value { "BYPASSRLS".to_string() } else { "NOBYPASSRLS".to_string() });
    }
    if let Some(limit) = connection_limit {
        parts.push(format!("CONNECTION LIMIT {limit}"));
    }
    if let Some(until) = valid_until {
        parts.push(format!("VALID UNTIL {}", quote_pg_string_literal(until)));
    }
    if parts.is_empty() {
        return Ok(());
    }
    let sql = format!("ALTER ROLE {} {};", pg_quote_identifier(name), parts.join(" "));
    pg_exec_role_sql(config, &sql)
}

/// 是否 PG16+（支持成员级 INHERIT/SET 选项），同步入口（独立会话查 server_version_num）。
fn pg_supports_member_options(config: &ConnectionConfig) -> fluxdb_core::Result<bool> {
    pg_runtime().block_on(async {
        let session = pg_connect(config, &pg_request_database(config, None)).await?;
        pg_auth_members_has_member_options(session.client.as_ref()).await
    })
}

/// 成员关系：GRANT role TO member [+ 各成员级选项]。
///
/// PG 每次 GRANT 只能带**一个** `WITH {ADMIN|INHERIT|SET}` 子句（见 `\h GRANT`），因此组合
/// admin/inherit/set 选项需拆成多条语句经 batch_execute 一次执行。PG16+ 支持成员级 INHERIT/SET
/// 选项；PG≤14 无此语法，仅 ADMIN。只对**非默认**值补发语句（默认 admin=false、inherit=true、set=true），
/// 先 `GRANT role TO member` 确立成员关系（幂等）再按需补选项。
fn pg_grant_role_membership(
    config: &ConnectionConfig,
    role: &str,
    member: &str,
    admin_option: bool,
    inherit_option: bool,
    set_option: bool,
) -> fluxdb_core::Result<()> {
    if role.is_empty() || member.is_empty() {
        return Err(Error::new(ErrorKind::Query, "角色与成员名不能为空"));
    }
    let role_q = pg_quote_identifier(role);
    let member_q = pg_quote_identifier(member);
    // 每次 GRANT 只能带一个 WITH 子句；先确立成员关系（幂等），再按版本下发选项。
    let mut stmts = vec![format!("GRANT {role_q} TO {member_q};")];
    let supports_options = pg_supports_member_options(config)?;
    // ADMIN：true 用 `WITH ADMIN OPTION`（各版本可）；false 仅在 PG16+ 用 `WITH ADMIN FALSE`
    // 显式关闭（PG≤14 无 FALSE 语法，且旧版无法通过 GRANT 关闭既有 ADMIN——需先 REVOKE）。
    if admin_option {
        stmts.push(format!("GRANT {role_q} TO {member_q} WITH ADMIN OPTION;"));
    } else if supports_options {
        stmts.push(format!("GRANT {role_q} TO {member_q} WITH ADMIN FALSE;"));
    }
    if supports_options {
        let inherit_clause = if inherit_option { "TRUE" } else { "FALSE" };
        stmts.push(format!("GRANT {role_q} TO {member_q} WITH INHERIT {inherit_clause};"));
        let set_clause = if set_option { "TRUE" } else { "FALSE" };
        stmts.push(format!("GRANT {role_q} TO {member_q} WITH SET {set_clause};"));
    }
    pg_exec_role_sql(config, &stmts.join("\n"))
}

/// 撤销成员关系：REVOKE role FROM member。
fn pg_revoke_role_membership(
    config: &ConnectionConfig,
    role: &str,
    member: &str,
) -> fluxdb_core::Result<()> {
    if role.is_empty() || member.is_empty() {
        return Err(Error::new(ErrorKind::Query, "角色与成员名不能为空"));
    }
    let sql = format!(
        "REVOKE {} FROM {};",
        pg_quote_identifier(role),
        pg_quote_identifier(member)
    );
    pg_exec_role_sql(config, &sql)
}

/// 对象授权：GRANT priv ON object TO grantee [WITH GRANT OPTION]。priv 为白名单关键字
/// （SELECT/INSERT/…）；object 形如 `TABLE "s"."t"` / `"db"` / `SCHEMA "s"`（由 app 侧渲染）。
fn pg_grant_object_privilege(
    config: &ConnectionConfig,
    privilege: &str,
    object_sql: &str,
    grantee: &str,
    grant_option: bool,
) -> fluxdb_core::Result<()> {
    if !is_pg_privilege_name(privilege) || !is_pg_identifier_name(grantee) {
        return Err(Error::new(ErrorKind::Query, "权限/授权对象名不合法"));
    }
    let suffix = if grant_option { " WITH GRANT OPTION" } else { "" };
    let sql = format!(
        "GRANT {} ON {} TO {}{};",
        privilege, object_sql, grantee, suffix
    );
    pg_exec_role_sql(config, &sql)
}

/// 对象撤销：REVOKE priv ON object FROM grantee。
fn pg_revoke_object_privilege(
    config: &ConnectionConfig,
    privilege: &str,
    object_sql: &str,
    grantee: &str,
) -> fluxdb_core::Result<()> {
    if !is_pg_privilege_name(privilege) || !is_pg_identifier_name(grantee) {
        return Err(Error::new(ErrorKind::Query, "权限/授权对象名不合法"));
    }
    let sql = format!(
        "REVOKE {} ON {} FROM {};",
        privilege, object_sql, grantee
    );
    pg_exec_role_sql(config, &sql)
}

/// 权限关键字白名单（防注入：只允许已知 SQL 关键字，不做任意文本透传）。
fn is_pg_privilege_name(value: &str) -> bool {
    matches!(
        value,
        "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "TRUNCATE" | "REFERENCES"
            | "TRIGGER" | "USAGE" | "CREATE" | "CONNECT" | "TEMP" | "TEMPORARY"
            | "EXECUTE" | "ALL"
    )
}

/// 是否 PG16+（pg_auth_members 有 inherit_option/set_option 列）。PG ≤14 无此列，成员选项恒默认
/// （INHERIT=true 且可 SET ROLE）。按版本选择 SQL，避免在旧版本查询不存在的列报错。
async fn pg_auth_members_has_member_options(
    client: &tokio_postgres::Client,
) -> fluxdb_core::Result<bool> {
    let row = client
        .query_one(
            "SELECT current_setting('server_version_num')::int >= 160000",
            &[],
        )
        .await
        .map_err(pg_error)?;
    let has: bool = row.get(0);
    Ok(has)
}

/// 列成员关系（PG 全选项：admin/inherit/set，版本感知）—— 供 UI 读取，聚合查询一次返回。
/// PG16+ 读取 inherit_option/set_option 列；PG≤14 回填默认 true。
async fn pg_list_role_membership_async(
    client: &tokio_postgres::Client,
) -> fluxdb_core::Result<Vec<PgRoleMembership>> {
    let has_member_options = pg_auth_members_has_member_options(client).await?;
    // 选列字符串（PG16 有 inherit/set，否则回填常数 true）。
    let (inherit_expr, set_expr) = if has_member_options {
        ("gm.inherit_option", "gm.set_option")
    } else {
        ("true", "true")
    };
    let sql = format!(
        "SELECT gp.rolname AS grantee, r.rolname AS member, \
                gm.admin_option, {inherit_expr} AS inherit_option, {set_expr} AS set_option \
         FROM pg_auth_members gm \
         JOIN pg_roles r ON r.oid = gm.member \
         JOIN pg_roles gp ON gp.oid = gm.roleid \
         ORDER BY grantee, member"
    );
    let rows = client.query(&sql, &[]).await.map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let admin: bool = row.get(2);
            let inherit: bool = row.get(3);
            let set: bool = row.get(4);
            PgRoleMembership {
                grantee: row.get(0),
                member: row.get(1),
                admin_option: admin,
                inherit_option: inherit,
                set_option: set,
            }
        })
        .collect())
}

/// 列成员关系（同步入口，供 UI/命令用）。
fn pg_list_role_membership(config: &ConnectionConfig) -> fluxdb_core::Result<Vec<PgRoleMembership>> {
    pg_runtime().block_on(async {
        let session = pg_connect(config, &pg_request_database(config, None)).await?;
        pg_list_role_membership_async(session.client.as_ref()).await
    })
}

/// 列某表的对象权限（aclexplode 展开 ACL 到「授权对象 × 权限 × grant option」）。
///
/// 返回 (grantee, privilege, grant_option)；grantee 为空视为 PUBLIC。PG 的 relacl 为 NULL
/// 时表示默认权限（owner 全权、其余无），此处不展开默认，交由 UI 明示。
async fn pg_list_relation_grants_async(
    client: &tokio_postgres::Client,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<(String, String, bool)>> {
    let rows = client
        .query(
            "SELECT COALESCE(grantee.rolname, '') AS grantee, \
                    acl.privilege_type, acl.is_grantable \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             CROSS JOIN LATERAL aclexplode(c.relacl) AS acl \
             LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
             WHERE n.nspname = $1 AND c.relname = $2 \
             ORDER BY grantee, acl.privilege_type",
            &[&schema, &table],
        )
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let grant_option: bool = row.get(2);
            (row.get(0), row.get(1), grant_option)
        })
        .collect())
}

/// 列某表的对象权限（同步入口）。
fn pg_list_relation_grants(
    config: &ConnectionConfig,
    schema: &str,
    table: &str,
) -> fluxdb_core::Result<Vec<(String, String, bool)>> {
    pg_runtime().block_on(async {
        let session = pg_connect(config, &pg_request_database(config, None)).await?;
        pg_list_relation_grants_async(session.client.as_ref(), schema, table).await
    })
}

/// 定位对象并返回 (owner, acl 是否 NULL) 的元数据 SQL 段（由各 scope 提供 catalog 表别名与过滤）。
struct ObjectGrantSql {
    /// 返回 owner 与 acl_is_null 的查询（固定两列 AS owner, acl_is_null）；`{ACL}` 占位替换为 ACL 列。
    meta_sql: &'static str,
    /// 返回显式 ACL 条目的 aclexplode 查询（固定三列 grantee, privilege_type, is_grantable）。
    entries_sql: &'static str,
    args: Vec<String>,
}

/// 各对象类型对应的 owner/ACL 列与定位过滤。函数用 `pg_get_function_identity_arguments` 签名区分重载。
fn pg_object_grant_sql(scope: &PgObjectGrantScope) -> ObjectGrantSql {
    match scope {
        PgObjectGrantScope::Database { database } => ObjectGrantSql {
            meta_sql: "SELECT d.datdba::regrole::text AS owner, (d.datacl IS NULL) AS acl_is_null \
                       FROM pg_database d WHERE d.datname = $1",
            entries_sql: "SELECT COALESCE(grantee.rolname, '') AS grantee, acl.privilege_type, \
                          acl.is_grantable FROM pg_database d \
                          CROSS JOIN LATERAL aclexplode(d.datacl) AS acl \
                          LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                          WHERE d.datname = $1 ORDER BY grantee, acl.privilege_type",
            args: vec![database.clone()],
        },
        PgObjectGrantScope::Schema { schema } => ObjectGrantSql {
            meta_sql: "SELECT n.nspowner::regrole::text AS owner, (n.nspacl IS NULL) AS acl_is_null \
                       FROM pg_namespace n WHERE n.nspname = $1",
            entries_sql: "SELECT COALESCE(grantee.rolname, '') AS grantee, acl.privilege_type, \
                          acl.is_grantable FROM pg_namespace n \
                          CROSS JOIN LATERAL aclexplode(n.nspacl) AS acl \
                          LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                          WHERE n.nspname = $1 ORDER BY grantee, acl.privilege_type",
            args: vec![schema.clone()],
        },
        PgObjectGrantScope::Relation { schema, name, .. } => {
            // relkind 由调用方以内联常数替换（relkind_list），不作为参数绑定（避免 char[] 类型推断问题）。
            ObjectGrantSql {
                meta_sql: "SELECT c.relowner::regrole::text AS owner, (c.relacl IS NULL) AS acl_is_null \
                           FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                           WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN (REPLACE)",
                entries_sql: "SELECT COALESCE(grantee.rolname, '') AS grantee, acl.privilege_type, \
                              acl.is_grantable FROM pg_class c \
                              JOIN pg_namespace n ON n.oid = c.relnamespace \
                              CROSS JOIN LATERAL aclexplode(c.relacl) AS acl \
                              LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                              WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN (REPLACE) \
                              ORDER BY grantee, acl.privilege_type",
                args: vec![schema.clone(), name.clone()],
            }
        }
        PgObjectGrantScope::Routine { schema, name, signature } => ObjectGrantSql {
            meta_sql: "SELECT p.proowner::regrole::text AS owner, (p.proacl IS NULL) AS acl_is_null \
                       FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                       WHERE n.nspname = $1 AND p.proname = $2 \
                         AND pg_get_function_identity_arguments(p.oid) = $3",
            entries_sql: "SELECT COALESCE(grantee.rolname, '') AS grantee, acl.privilege_type, \
                          acl.is_grantable FROM pg_proc p \
                          JOIN pg_namespace n ON n.oid = p.pronamespace \
                          CROSS JOIN LATERAL aclexplode(p.proacl) AS acl \
                          LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                          WHERE n.nspname = $1 AND p.proname = $2 \
                            AND pg_get_function_identity_arguments(p.oid) = $3 \
                          ORDER BY grantee, acl.privilege_type",
            args: vec![schema.clone(), name.clone(), signature.clone()],
        },
    }
}

/// 把带 `{ACL}` 占位的 meta_sql 内联 relkind 常数并执行，取 owner 与 ACL 是否 NULL。
async fn pg_object_grants_meta(
    client: &tokio_postgres::Client,
    sqlgen: &ObjectGrantSql,
    relkind_inline: &str,
) -> fluxdb_core::Result<(String, bool)> {
    let sql = sqlgen.meta_sql.replace("REPLACE", relkind_inline);
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = sqlgen
        .args
        .iter()
        .map(|arg| arg as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    let rows = client.query(&sql, &params).await.map_err(pg_error)?;
    let row = rows.first().ok_or_else(|| {
        Error::new(
            ErrorKind::Query,
            "未找到该对象（可能已被删除或不在当前 schema）",
        )
    })?;
    Ok((row.get(0), row.get(1)))
}

/// 执行 `aclexplode` 条目查询，返回显式 ACL 条目（grantee 空=PUBLIC）。
async fn pg_object_grants_entries(
    client: &tokio_postgres::Client,
    sqlgen: &ObjectGrantSql,
    relkind_inline: &str,
    owner: &str,
) -> fluxdb_core::Result<Vec<PgGrantEntry>> {
    let sql = sqlgen.entries_sql.replace("REPLACE", relkind_inline);
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = sqlgen
        .args
        .iter()
        .map(|arg| arg as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    let rows = client.query(&sql, &params).await.map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| PgGrantEntry {
            grantee: row.get(0),
            privilege: row.get(1),
            grant_option: row.get(2),
            is_owner: {
                let grantee: String = row.get(0);
                !grantee.is_empty() && grantee == owner
            },
        })
        .collect())
}

/// 列 PG 对象权限（同步入口）。返回 owner / ACL 是否默认 / 显式条目（直接授权 + PUBLIC + owner 标记）。
pub fn pg_list_object_grants(
    config: &ConnectionConfig,
    scope: &PgObjectGrantScope,
) -> fluxdb_core::Result<PgObjectGrants> {
    if config.kind != DatabaseKind::Postgres {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持对象权限"));
    }
    let relkind_inline = match scope {
        PgObjectGrantScope::Relation { kind, .. } => kind.relkind_list(),
        _ => "",
    };
    pg_runtime().block_on(async {
        let session = pg_connect(config, &pg_request_database(config, None)).await?;
        let client = session.client.as_ref();
        let sqlgen = pg_object_grant_sql(scope);
        let (owner, acl_is_null) = pg_object_grants_meta(client, &sqlgen, relkind_inline).await?;
        let entries = pg_object_grants_entries(client, &sqlgen, relkind_inline, &owner).await?;
        Ok(PgObjectGrants {
            owner,
            acl_is_null,
            entries,
        })
    })
}

/// 各对象类型候选权限关键字与 PG `has_*_privilege` 检查函数。函数对象字符串取
/// `schema.name(类型列表)`（经 identity 签名剥去参数名），供 `has_function_privilege`。
struct EffectiveQuery {
    /// SQL 片段：`has_<obj>_privilege($1, <obj_expr>, p.priv)`。`$1` 为角色，`$2` 系列为定位参数。
    check_sql: String,
    /// 定位参数（角色之后）。obj_expr 用 $2/$3…
    args: Vec<String>,
}

/// 构建某 scope 的 `has_*_privilege` 检查（角色为 $1，后续为对象定位参数）。
fn pg_effective_query(sqlgen: &ObjectGrantSql, scope: &PgObjectGrantScope) -> fluxdb_core::Result<EffectiveQuery> {
    let params = &sqlgen.args;
    // 定位参数从 $2 开始依次对应 sqlgen.args 的顺序与对象过滤列一致。
    match scope {
        PgObjectGrantScope::Database { .. } => Ok(EffectiveQuery {
            check_sql: "SELECT p.priv, has_database_privilege($1, $2, p.priv) FROM \
                        (VALUES ('CONNECT'),('CREATE'),('TEMP')) AS p(priv)"
                .to_string(),
            args: params.clone(),
        }),
        PgObjectGrantScope::Schema { .. } => Ok(EffectiveQuery {
            check_sql: "SELECT p.priv, has_schema_privilege($1, $2, p.priv) FROM \
                        (VALUES ('USAGE'),('CREATE')) AS p(priv)"
                .to_string(),
            args: params.clone(),
        }),
        PgObjectGrantScope::Relation { kind, .. } => {
            let has_fn = match kind {
                PgRelationKind::Sequence => "has_sequence_privilege($1, $2 || '.' || $3, p.priv)",
                PgRelationKind::Table | PgRelationKind::View => {
                    "has_table_privilege($1, $2 || '.' || $3, p.priv)"
                }
            };
            let privs = kind
                .effective_privileges()
                .iter()
                .map(|p| format!("('{p}')"))
                .collect::<Vec<_>>()
                .join(",");
            Ok(EffectiveQuery {
                check_sql: format!("SELECT p.priv, {has_fn} FROM (VALUES {privs}) AS p(priv)"),
                args: params.clone(),
            })
        }
        PgObjectGrantScope::Routine { schema, name, signature } => Ok(EffectiveQuery {
            check_sql: "SELECT p.priv, has_function_privilege($1, $2 || '.' || $3 || '(' || $4 || ')', p.priv) FROM (VALUES ('EXECUTE')) AS p(priv)".to_string(),
            args: vec![
                schema.clone(),
                name.clone(),
                pg_routine_type_list(signature),
            ],
        }),
    }
}

/// 剥去 `pg_get_function_identity_arguments` 输出的参数名，得到类型列表串（用于 has_function_privilege）。
fn pg_routine_type_list(signature: &str) -> String {
    // 形如 `a integer, b text` → 每段取最后一个空白后的类型 → `integer, text`。
    signature
        .split(',')
        .map(|seg| {
            let seg = seg.trim();
            seg.rsplit_once(' ').map(|(_, ty)| ty).unwrap_or(seg)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// 某角色对某对象的有效权限（owner/直接/PUBLIC/继承统一经 PG 判定）——供 T27 区分直接与继承。
pub fn pg_role_effective_grants(
    config: &ConnectionConfig,
    scope: &PgObjectGrantScope,
    role: &str,
) -> fluxdb_core::Result<Vec<PgEffectivePrivilege>> {
    if config.kind != DatabaseKind::Postgres {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持角色有效权限"));
    }
    let relkind_inline = match scope {
        PgObjectGrantScope::Relation { kind, .. } => kind.relkind_list(),
        _ => "",
    };
    pg_runtime().block_on(async {
        let session = pg_connect(config, &pg_request_database(config, None)).await?;
        let client = session.client.as_ref();
        let sqlgen = pg_object_grant_sql(scope);
        let (owner, _) = pg_object_grants_meta(client, &sqlgen, relkind_inline).await?;
        // 直接授权（该角色的显式 ACL 条目）。
        let direct = pg_object_grants_entries(client, &sqlgen, relkind_inline, &owner)
            .await?
            .into_iter()
            .filter(|e| e.grantee == role)
            .collect::<Vec<_>>();
        let q = pg_effective_query(&sqlgen, scope).map_err(Error::from)?;
        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            vec![&role as &(dyn tokio_postgres::types::ToSql + Sync)];
        for arg in &q.args {
            params.push(arg as &(dyn tokio_postgres::types::ToSql + Sync));
        }
        let rows = client.query(q.check_sql.as_str(), &params).await.map_err(pg_error)?;
        let mut out = Vec::new();
        for row in rows {
            let privilege: String = row.get(0);
            let effective: bool = row.get(1);
            let direct_entry = direct.iter().find(|e| e.privilege == privilege);
            out.push(PgEffectivePrivilege {
                privilege,
                effective,
                direct: direct_entry.is_some(),
                grant_option: direct_entry.is_some_and(|e| e.grant_option),
            });
        }
        // 若 role 即 owner，owner 对其对象类型拥有全部权限（has_*_privilege 已含，无需叠加）。
        Ok(out)
    })
}
