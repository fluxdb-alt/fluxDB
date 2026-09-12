// PostgreSQL 角色/用户管理（T26 增量一）。
//
// 角色是集群级主体（LOGIN=NOLOGIN），DDL 不依赖具体数据库，沿用建库的独立 autocommit
// 连接（`pg_connect` + `batch_execute`）。角色密码语句禁止进入 SQL 历史/日志（本文件只生成
// 语句不写日志，凭密码内容不加日志字段）。角色名/权限关键字一律 `pg_quote_identifier` 引用，
// 密码走专属转义（`quote_pg_string_literal`），不做字符串拼接进参数（PG role DDL 不支持
// 全参数绑定，需明确的字符串引用逻辑，设计 §12）。

use fluxdb_core::{PgObjectGrantScope, PgRole};

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

/// 成员关系：GRANT member TO role [WITH ADMIN OPTION]（成员可再授权的 ADMIN OPTION 分开）。
fn pg_grant_role_membership(
    config: &ConnectionConfig,
    role: &str,
    member: &str,
    admin_option: bool,
) -> fluxdb_core::Result<()> {
    if role.is_empty() || member.is_empty() {
        return Err(Error::new(ErrorKind::Query, "角色与成员名不能为空"));
    }
    let sql = format!(
        "GRANT {} TO {}{};",
        pg_quote_identifier(role),
        pg_quote_identifier(member),
        if admin_option { " WITH ADMIN OPTION" } else { "" }
    );
    pg_exec_role_sql(config, &sql)
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

/// 对象授权：GRANT priv TO grantee ON object。priv 为已知白名单关键字（SELECT/INSERT/…）。
/// object 形如 `"schema"."table"` 或 `"database"`/`"schema"`；用闭包渲染以保持参数安全。
fn pg_grant_object_privilege(
    config: &ConnectionConfig,
    privilege: &str,
    object_sql: &str,
    grantee: &str,
) -> fluxdb_core::Result<()> {
    if !is_pg_privilege_name(privilege) || !is_pg_identifier_name(grantee) {
        return Err(Error::new(ErrorKind::Query, "权限/授权对象名不合法"));
    }
    let sql = format!(
        "GRANT {} ON {} TO {};",
        privilege, object_sql, grantee
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

/// 列成员关系（grantees with admin option）—— 供 UI 读取，聚合查询一次返回。
async fn pg_list_role_membership_async(
    client: &tokio_postgres::Client,
) -> fluxdb_core::Result<Vec<(String, String, bool)>> {
    let rows = client
        .query(
            "SELECT gp.rolname AS grantee, r.rolname AS member, gm.admin_option \
             FROM pg_auth_members gm \
             JOIN pg_roles r ON r.oid = gm.member \
             JOIN pg_roles gp ON gp.oid = gm.roleid \
             ORDER BY grantee, member",
            &[],
        )
        .await
        .map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            // admin_option 是原生 bool（pg_auth_members），非 text 投影。
            let admin: bool = row.get(2);
            (row.get(0), row.get(1), admin)
        })
        .collect())
}

/// 列成员关系（同步入口，供 UI/命令用）。
fn pg_list_role_membership(config: &ConnectionConfig) -> fluxdb_core::Result<Vec<(String, String, bool)>> {
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

/// 按 scope 定位对象并 `aclexplode(ACL列)` 列出 (grantee, privilege, grant_option)。
///
/// 覆盖数据库(datacl)/schema(nspacl)/表·视图·序列(relacl)/函数(proacl)；grantee 空视为 PUBLIC。
/// 函数用 `pg_get_function_identity_arguments` 的签名区分重载。无显式 ACL（NULL）时表示
/// owner 全权、其余无默认权限，此处返回空交由 UI 明示「默认权限」。
async fn pg_list_object_grants_async(
    client: &tokio_postgres::Client,
    scope: &PgObjectGrantScope,
) -> fluxdb_core::Result<Vec<(String, String, bool)>> {
    // 各对象类型返回 (acl_column_ref, 标识行的 WHERE)，统一由 aclexplode 展开。
    struct Target {
        sql: &'static str,
        args: Vec<String>,
    }
    let target: Target = match scope {
        PgObjectGrantScope::Database { database } => Target {
            sql: "SELECT COALESCE(grantee.rolname, '') AS grantee, acl.privilege_type, \
                  acl.is_grantable FROM pg_database d \
                  CROSS JOIN LATERAL aclexplode(d.datacl) AS acl \
                  LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                  WHERE d.datname = $1 ORDER BY grantee, acl.privilege_type",
            args: vec![database.clone()],
        },
        PgObjectGrantScope::Schema { schema } => Target {
            sql: "SELECT COALESCE(grantee.rolname, '') AS grantee, acl.privilege_type, \
                  acl.is_grantable FROM pg_namespace n \
                  CROSS JOIN LATERAL aclexplode(n.nspacl) AS acl \
                  LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                  WHERE n.nspname = $1 ORDER BY grantee, acl.privilege_type",
            args: vec![schema.clone()],
        },
        PgObjectGrantScope::Relation { schema, name, .. } => Target {
            // relkind 由调用方以常数内联（relkind_list），不作为参数绑定（避免 int2/char 类型推断问题）。
            sql: "SELECT COALESCE(grantee.rolname, '') AS grantee, acl.privilege_type, \
                  acl.is_grantable FROM pg_class c \
                  JOIN pg_namespace n ON n.oid = c.relnamespace \
                  CROSS JOIN LATERAL aclexplode(c.relacl) AS acl \
                  LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                  WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN (REPLACE) \
                  ORDER BY grantee, acl.privilege_type",
            args: vec![schema.clone(), name.clone()],
        },
        PgObjectGrantScope::Routine { schema, name, signature } => Target {
            sql: "SELECT COALESCE(grantee.rolname, '') AS grantee, acl.privilege_type, \
                  acl.is_grantable FROM pg_proc p \
                  JOIN pg_namespace n ON n.oid = p.pronamespace \
                  CROSS JOIN LATERAL aclexplode(p.proacl) AS acl \
                  LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                  WHERE n.nspname = $1 AND p.proname = $2 \
                    AND pg_get_function_identity_arguments(p.oid) = $3 \
                  ORDER BY grantee, acl.privilege_type",
            args: vec![schema.clone(), name.clone(), signature.clone()],
        },
    };
    let sql = target.sql.replace("REPLACE", match scope {
        PgObjectGrantScope::Relation { kind, .. } => kind.relkind_list(),
        _ => "",
    });
    // &String 实现 ToSql；显式 cast 为 trait 对象数组供 tokio-postgres query 参数绑定。
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = target
        .args
        .iter()
        .map(|arg| arg as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    let rows = client.query(&sql, &params).await.map_err(pg_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let grant_option: bool = row.get(2);
            (row.get(0), row.get(1), grant_option)
        })
        .collect())
}

/// 列 PG 对象权限（同步入口）。
pub fn pg_list_object_grants(
    config: &ConnectionConfig,
    scope: &PgObjectGrantScope,
) -> fluxdb_core::Result<Vec<(String, String, bool)>> {
    if config.kind != DatabaseKind::Postgres {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持对象权限"));
    }
    pg_runtime().block_on(async {
        let session = pg_connect(config, &pg_request_database(config, None)).await?;
        pg_list_object_grants_async(session.client.as_ref(), scope).await
    })
}
