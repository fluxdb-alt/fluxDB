#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserAdminDialect {
    MySql,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivilegeScope {
    MySql,
    Postgres,
    SqlServer,
    /// Oracle：以账号为主体，权限分角色 / 系统权限 / 对象权限三类。
    Oracle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseUserIdentity {
    pub user: String,
    pub host: String,
    pub plugin: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatePrincipalInput {
    pub user: String,
    pub host: String,
    pub password: String,
    pub auth_plugin: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivilegeChangeInput {
    pub user: DatabaseUserIdentity,
    pub privileges: Vec<String>,
    pub database: String,
    pub table: String,
    pub grant_option: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabasePrivilegeGrant {
    pub database: String,
    pub privileges: Vec<String>,
    pub grant_option: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UserResourceLimits {
    pub max_queries_per_hour: Option<u64>,
    pub max_updates_per_hour: Option<u64>,
    pub max_connections_per_hour: Option<u64>,
    pub max_user_connections: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserRoleMembership {
    pub role: DatabaseUserIdentity,
    pub granted: bool,
    pub default_role: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserRoleMember {
    pub member: DatabaseUserIdentity,
    pub granted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabaseUserAdminProvider {
    dialect: UserAdminDialect,
    default_scope: PrivilegeScope,
    /// MariaDB 复用 MySQL 方言，但账号 DDL 与服务端插件集不同：
    /// `IDENTIFIED WITH ... BY` / `SET DEFAULT ROLE ... TO` 在 MariaDB 上会报语法错误。
    mariadb: bool,
}

impl DatabaseUserAdminProvider {
    pub fn dialect(self) -> UserAdminDialect {
        self.dialect
    }

    pub fn default_scope(self) -> PrivilegeScope {
        self.default_scope
    }

    /// 新建账号时预填的认证插件。MySQL 8 默认 `caching_sha2_password`；
    /// MariaDB 没有该插件，默认用兼容性最好的 `mysql_native_password`。
    pub fn default_auth_plugin(self) -> &'static str {
        if self.mariadb {
            "mysql_native_password"
        } else {
            "caching_sha2_password"
        }
    }

    /// 认证插件候选项。MariaDB 不支持 MySQL 8 的
    /// `caching_sha2_password` / `sha256_password` / `auth_socket`。
    pub fn auth_plugins(self) -> &'static [&'static str] {
        if self.mariadb {
            &[
                "mysql_native_password",
                "ed25519",
                "unix_socket",
                "pam",
            ]
        } else {
            &[
                "caching_sha2_password",
                "mysql_native_password",
                "sha256_password",
                "auth_socket",
                "mysql_clear_password",
            ]
        }
    }

    pub fn list_users_sql(self) -> &'static str {
        match self.dialect {
            UserAdminDialect::MySql => {
                "SELECT User AS user, Host AS host, plugin AS plugin FROM mysql.user ORDER BY User, Host;"
            }
        }
    }

    pub fn fallback_list_users_sql(self) -> Option<&'static str> {
        match self.dialect {
            UserAdminDialect::MySql => Some(
                "SELECT DISTINCT GRANTEE AS grantee FROM information_schema.USER_PRIVILEGES ORDER BY GRANTEE;",
            ),
        }
    }

    pub fn current_user_sql(self) -> &'static str {
        match self.dialect {
            UserAdminDialect::MySql => {
                "SELECT SUBSTRING_INDEX(CURRENT_USER(), '@', 1) AS user, SUBSTRING_INDEX(CURRENT_USER(), '@', -1) AS host, 'CURRENT_USER()' AS plugin;"
            }
        }
    }

    pub fn parse_users(self, page: &DataPage) -> Vec<DatabaseUserIdentity> {
        match self.dialect {
            UserAdminDialect::MySql => users_from_mysql_user_result(page),
        }
    }

    pub fn parse_fallback_users(self, page: &DataPage) -> Vec<DatabaseUserIdentity> {
        match self.dialect {
            UserAdminDialect::MySql => users_from_mysql_grantee_result(page),
        }
    }

    pub fn show_grants_sql(self, user: &DatabaseUserIdentity) -> String {
        match self.dialect {
            UserAdminDialect::MySql => format!("SHOW GRANTS FOR {};", self.mysql_account(user)),
        }
    }

    pub fn create_user_sql(self, input: &CreatePrincipalInput) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                let plugin = input
                    .auth_plugin
                    .as_deref()
                    .map(str::trim)
                    .filter(|plugin| !plugin.is_empty());
                let account = self.mysql_account(&DatabaseUserIdentity {
                    user: input.user.clone(),
                    host: input.host.clone(),
                    plugin: None,
                });
                let password = quote_mysql_string(&input.password);
                // MariaDB 用 `IDENTIFIED VIA <plugin> USING PASSWORD(...)`；
                // MySQL 的 `IDENTIFIED WITH <plugin> BY ...` 在 MariaDB 上是语法错误。
                let identified = if self.mariadb {
                    match plugin {
                        Some(plugin) => format!(
                            "VIA {} USING PASSWORD({})",
                            quote_mysql_identifier(plugin),
                            password
                        ),
                        None => format!("BY {password}"),
                    }
                } else {
                    match plugin {
                        Some(plugin) => format!(
                            "WITH {} BY {}",
                            quote_mysql_identifier(plugin),
                            password
                        ),
                        None => format!("BY {password}"),
                    }
                };
                format!("CREATE USER {account} IDENTIFIED {identified};")
            }
        }
    }

    pub fn alter_password_sql(self, user: &DatabaseUserIdentity, password: &str) -> String {
        match self.dialect {
            UserAdminDialect::MySql => format!(
                "ALTER USER {} IDENTIFIED BY {};",
                self.mysql_account(user),
                quote_mysql_string(password)
            ),
        }
    }

    pub fn alter_auth_plugin_sql(
        self,
        user: &DatabaseUserIdentity,
        plugin: &str,
        password: Option<&str>,
    ) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                let account = self.mysql_account(user);
                let plugin = quote_mysql_identifier(plugin.trim());
                if self.mariadb {
                    // MariaDB：`IDENTIFIED VIA plugin [USING PASSWORD('...')]`。
                    return match password {
                        Some(password) => format!(
                            "ALTER USER {account} IDENTIFIED VIA {plugin} USING PASSWORD({});",
                            quote_mysql_string(password)
                        ),
                        None => format!("ALTER USER {account} IDENTIFIED VIA {plugin};"),
                    };
                }
                let password_sql = password
                    .map(|password| format!(" BY {}", quote_mysql_string(password)))
                    .unwrap_or_default();
                format!("ALTER USER {account} IDENTIFIED WITH {plugin}{password_sql};")
            }
        }
    }

    pub fn alter_password_expiry_sql(
        self,
        user: &DatabaseUserIdentity,
        policy: &str,
    ) -> Option<String> {
        let expiry = match policy.trim() {
            "DEFAULT" => "DEFAULT",
            "NEVER" => "NEVER",
            "INTERVAL 90 DAY" => "INTERVAL 90 DAY",
            "EXPIRE NOW" => "",
            _ => return None,
        };
        let suffix = if expiry.is_empty() {
            "PASSWORD EXPIRE".to_string()
        } else {
            format!("PASSWORD EXPIRE {expiry}")
        };
        Some(match self.dialect {
            UserAdminDialect::MySql => {
                format!("ALTER USER {} {};", self.mysql_account(user), suffix)
            }
        })
    }

    pub fn alter_resource_limits_sql(
        self,
        user: &DatabaseUserIdentity,
        limits: &UserResourceLimits,
    ) -> Option<String> {
        let mut clauses = Vec::new();
        if let Some(value) = limits.max_queries_per_hour {
            clauses.push(format!("MAX_QUERIES_PER_HOUR {value}"));
        }
        if let Some(value) = limits.max_updates_per_hour {
            clauses.push(format!("MAX_UPDATES_PER_HOUR {value}"));
        }
        if let Some(value) = limits.max_connections_per_hour {
            clauses.push(format!("MAX_CONNECTIONS_PER_HOUR {value}"));
        }
        if let Some(value) = limits.max_user_connections {
            clauses.push(format!("MAX_USER_CONNECTIONS {value}"));
        }
        (!clauses.is_empty()).then(|| match self.dialect {
            UserAdminDialect::MySql => {
                format!("ALTER USER {} WITH {};", self.mysql_account(user), clauses.join(" "))
            }
        })
    }

    pub fn alter_ssl_requirement_sql(
        self,
        user: &DatabaseUserIdentity,
        ssl_type: &str,
        cipher: &str,
        issuer: &str,
        subject: &str,
    ) -> Option<String> {
        let requirement = match ssl_type.trim() {
            "ANY" => "SSL".to_string(),
            "X509" => "X509".to_string(),
            "SPECIFIED" => {
                let mut parts = Vec::new();
                if !cipher.trim().is_empty() {
                    parts.push(format!("CIPHER {}", quote_mysql_string(cipher.trim())));
                }
                if !issuer.trim().is_empty() {
                    parts.push(format!("ISSUER {}", quote_mysql_string(issuer.trim())));
                }
                if !subject.trim().is_empty() {
                    parts.push(format!("SUBJECT {}", quote_mysql_string(subject.trim())));
                }
                if parts.is_empty() {
                    return None;
                }
                parts.join(" AND ")
            }
            _ => return None,
        };
        Some(match self.dialect {
            UserAdminDialect::MySql => {
                format!("ALTER USER {} REQUIRE {};", self.mysql_account(user), requirement)
            }
        })
    }

    pub fn alter_login_sql(self, user: &DatabaseUserIdentity, enabled: bool) -> String {
        match self.dialect {
            UserAdminDialect::MySql => format!(
                "ALTER USER {} ACCOUNT {};",
                self.mysql_account(user),
                if enabled { "UNLOCK" } else { "LOCK" }
            ),
        }
    }

    pub fn drop_user_sql(self, user: &DatabaseUserIdentity) -> String {
        match self.dialect {
            UserAdminDialect::MySql => format!("DROP USER {};", self.mysql_account(user)),
        }
    }

    pub fn grant_privileges_sql(self, input: &PrivilegeChangeInput) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                let privileges = normalize_privileges(&input.privileges, "SELECT").join(", ");
                let grant_option = if input.grant_option {
                    " WITH GRANT OPTION"
                } else {
                    ""
                };
                format!(
                    "GRANT {} ON {} TO {}{};",
                    privileges,
                    mysql_privilege_target_sql(&input.database, &input.table),
                    self.mysql_account(&input.user),
                    grant_option
                )
            }
        }
    }

    pub fn revoke_privileges_sql(self, input: &PrivilegeChangeInput) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                let privileges = normalize_privileges(&input.privileges, "SELECT").join(", ");
                format!(
                    "REVOKE {} ON {} FROM {};",
                    privileges,
                    mysql_privilege_target_sql(&input.database, &input.table),
                    self.mysql_account(&input.user)
                )
            }
        }
    }

    pub fn grant_role_sql(self, role: &DatabaseUserIdentity, user: &DatabaseUserIdentity) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                format!(
                    "GRANT {} TO {};",
                    self.mysql_account(role),
                    self.mysql_account(user)
                )
            }
        }
    }

    pub fn revoke_role_sql(self, role: &DatabaseUserIdentity, user: &DatabaseUserIdentity) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                format!(
                    "REVOKE {} FROM {};",
                    self.mysql_account(role),
                    self.mysql_account(user)
                )
            }
        }
    }

    pub fn set_default_roles_sql(
        self,
        user: &DatabaseUserIdentity,
        roles: &[DatabaseUserIdentity],
    ) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                let roles_sql = if roles.is_empty() {
                    "NONE".to_string()
                } else {
                    roles
                        .iter()
                        .map(|role| self.mysql_account(role))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                // MariaDB 用 `FOR`，MySQL 用 `TO`；写错关键字服务端直接报语法错误。
                let keyword = if self.mariadb { "FOR" } else { "TO" };
                format!(
                    "SET DEFAULT ROLE {} {keyword} {};",
                    roles_sql,
                    self.mysql_account(user)
                )
            }
        }
    }

    /// MySQL 家族账号写法。
    ///
    /// MariaDB 的角色是 host 为空的账号：`SHOW GRANTS` / `GRANT` / `REVOKE` /
    /// `SET DEFAULT ROLE` 里只能写角色名，补上 `@''` 或 `@'%'` 会报 1141/1064。
    fn mysql_account(self, user: &DatabaseUserIdentity) -> String {
        if self.mariadb && user.host.is_empty() {
            quote_mysql_string(&user.user)
        } else {
            mysql_user_account(user)
        }
    }

    pub fn privileges_for_scope(self, _: PrivilegeScope) -> &'static [&'static str] {
        match self.dialect {
            UserAdminDialect::MySql => &MYSQL_COMMON_PRIVILEGES,
        }
    }

    pub fn default_privileges_for_scope(self, _: PrivilegeScope) -> Vec<String> {
        vec!["SELECT".to_string()]
    }

    pub fn privilege_grants_from_grants(self, grants: &[String]) -> Vec<DatabasePrivilegeGrant> {
        match self.dialect {
            UserAdminDialect::MySql => mysql_privilege_grants_from_grants(
                grants,
                self.privileges_for_scope(self.default_scope),
            ),
        }
    }

    pub fn label(self, user: &DatabaseUserIdentity) -> String {
        match self.dialect {
            UserAdminDialect::MySql => format!("{}@{}", user.user, user.host),
        }
    }

    pub fn detail(self, user: &DatabaseUserIdentity) -> Option<String> {
        match self.dialect {
            UserAdminDialect::MySql => user.plugin.clone(),
        }
    }
}

pub const MYSQL_COMMON_PRIVILEGES: [&str; 17] = [
    "ALTER",
    "ALTER ROUTINE",
    "CREATE",
    "CREATE ROUTINE",
    "CREATE TEMPORARY TABLES",
    "CREATE VIEW",
    "DELETE",
    "DROP",
    "EVENT",
    "EXECUTE",
    "INDEX",
    "INSERT",
    "REFERENCES",
    "SELECT",
    "SHOW VIEW",
    "TRIGGER",
    "UPDATE",
];

pub fn database_user_admin_provider(kind: DatabaseKind) -> Option<DatabaseUserAdminProvider> {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            Some(DatabaseUserAdminProvider {
                dialect: UserAdminDialect::MySql,
                default_scope: PrivilegeScope::MySql,
                mariadb: kind == DatabaseKind::MariaDb,
            })
        }
        DatabaseKind::Sqlite
        | DatabaseKind::MongoDb
        | DatabaseKind::Redis
        | DatabaseKind::Postgres
        | DatabaseKind::Kingbase
        | DatabaseKind::Dameng
        | DatabaseKind::SqlServer
        | DatabaseKind::Oracle => None,
    }
}

pub fn supports_database_user_admin(kind: DatabaseKind) -> bool {
    // PostgreSQL 与 SQL Server 分别走独立的角色/权限工作台，无需 MySQL 的
    // user@host SQL 生成 provider；达梦复用 Oracle 的账号面板。
    matches!(
        kind,
        DatabaseKind::Postgres
            | DatabaseKind::Kingbase
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle
            | DatabaseKind::Dameng
    ) || database_user_admin_provider(kind).is_some()
}

pub fn grants_from_query_result(result: &QueryExecutionResult) -> Vec<String> {
    result
        .results
        .first()
        .map(|page| {
            page.rows
                .iter()
                .filter_map(|row| row.values.first())
                .map(CellValue::display_label)
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn role_memberships_from_grants(
    candidates: &[DatabaseUserIdentity],
    grants: &[String],
    user: &DatabaseUserIdentity,
) -> Vec<UserRoleMembership> {
    let granted_roles = roles_granted_to_user_from_grants(grants, user);
    let default_roles = default_roles_for_user_from_grants(grants, user);
    candidates
        .iter()
        .filter(|candidate| *candidate != user)
        .map(|candidate| UserRoleMembership {
            role: candidate.clone(),
            granted: granted_roles
                .iter()
                .any(|role| same_database_account(role, candidate)),
            default_role: default_roles
                .iter()
                .any(|role| same_database_account(role, candidate)),
        })
        .collect()
}

fn mysql_privilege_grants_from_grants(
    grants: &[String],
    known_privileges: &[&str],
) -> Vec<DatabasePrivilegeGrant> {
    let mut rows: Vec<DatabasePrivilegeGrant> = Vec::new();
    for grant in grants {
        let Some(row) = mysql_privilege_grant_from_grant(grant, known_privileges) else {
            continue;
        };
        if let Some(existing) = rows
            .iter_mut()
            .find(|existing| existing.database == row.database)
        {
            existing.grant_option |= row.grant_option;
            existing.privileges.extend(row.privileges);
            existing.privileges = normalize_privileges(&existing.privileges, "");
        } else {
            rows.push(row);
        }
    }
    rows.sort_by(|left, right| left.database.cmp(&right.database));
    rows
}

fn mysql_privilege_grant_from_grant(
    grant: &str,
    known_privileges: &[&str],
) -> Option<DatabasePrivilegeGrant> {
    let grant = grant.trim().trim_end_matches(';').trim();
    let grant = strip_prefix_ascii_ci(grant, "GRANT ")?;
    let (privileges, rest) = split_once_ascii_ci(grant, " ON ")?;
    let (target, tail) = split_once_ascii_ci(rest, " TO ")?;
    let (database, table) = split_mysql_privilege_target(target.trim())?;
    if table != "*" {
        return None;
    }
    let grant_option = contains_ascii_ci(tail, " WITH GRANT OPTION");
    let privileges = mysql_grant_privileges(privileges, known_privileges);
    if privileges.is_empty() {
        return None;
    }
    Some(DatabasePrivilegeGrant {
        database,
        privileges,
        grant_option,
    })
}

fn mysql_grant_privileges(privileges: &str, known_privileges: &[&str]) -> Vec<String> {
    let privileges = privileges.trim();
    if privileges.eq_ignore_ascii_case("ALL")
        || privileges.eq_ignore_ascii_case("ALL PRIVILEGES")
    {
        return known_privileges.iter().map(|privilege| privilege.to_string()).collect();
    }

    let known = known_privileges
        .iter()
        .map(|privilege| privilege.to_ascii_uppercase())
        .collect::<Vec<_>>();
    let mut parsed = privileges
        .split(',')
        .map(|privilege| privilege.trim().to_ascii_uppercase())
        .filter(|privilege| known.iter().any(|known| known == privilege))
        .collect::<Vec<_>>();
    parsed.sort();
    parsed.dedup();
    parsed
}

fn split_mysql_privilege_target(target: &str) -> Option<(String, String)> {
    let split = mysql_target_dot_index(target)?;
    let database = unquote_mysql_identifier(target[..split].trim());
    let table = unquote_mysql_identifier(target[split + 1..].trim());
    Some((database, table))
}

fn mysql_target_dot_index(target: &str) -> Option<usize> {
    let mut in_backtick = false;
    let bytes = target.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'`' => {
                if in_backtick && bytes.get(index + 1) == Some(&b'`') {
                    index += 1;
                } else {
                    in_backtick = !in_backtick;
                }
            }
            b'.' if !in_backtick => return Some(index),
            _ => {}
        }
        index += 1;
    }
    None
}

fn unquote_mysql_identifier(value: &str) -> String {
    let value = value.trim();
    if value == "*" {
        return "*".to_string();
    }
    value
        .strip_prefix('`')
        .and_then(|value| value.strip_suffix('`'))
        .map(|value| value.replace("``", "`"))
        .unwrap_or_else(|| value.to_string())
}

fn strip_prefix_ascii_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        .then(|| &value[prefix.len()..])
}

fn split_once_ascii_ci<'a>(value: &'a str, needle: &str) -> Option<(&'a str, &'a str)> {
    let index = value
        .to_ascii_uppercase()
        .find(&needle.to_ascii_uppercase())?;
    Some((&value[..index], &value[index + needle.len()..]))
}

fn contains_ascii_ci(value: &str, needle: &str) -> bool {
    value
        .to_ascii_uppercase()
        .contains(&needle.to_ascii_uppercase())
}

pub fn quote_mysql_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

pub fn mysql_user_account(user: &DatabaseUserIdentity) -> String {
    format!(
        "{}@{}",
        quote_mysql_string(&user.user),
        quote_mysql_string(&user.host)
    )
}

pub fn mysql_privilege_target_sql(database: &str, table: &str) -> String {
    let db = if database.trim().is_empty() {
        "*"
    } else {
        database.trim()
    };
    let tbl = if table.trim().is_empty() {
        "*"
    } else {
        table.trim()
    };
    let db_sql = if db == "*" {
        "*".to_string()
    } else {
        quote_mysql_identifier(db)
    };
    let table_sql = if tbl == "*" {
        "*".to_string()
    } else {
        quote_mysql_identifier(tbl)
    };
    format!("{db_sql}.{table_sql}")
}

pub fn normalize_privileges(privileges: &[String], fallback: &str) -> Vec<String> {
    let mut normalized = privileges
        .iter()
        .map(|privilege| privilege.trim().to_ascii_uppercase())
        .filter(|privilege| !privilege.is_empty())
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        normalized.push(fallback.to_string());
    }
    normalized.sort();
    normalized.dedup();
    normalized
}

pub fn users_from_mysql_user_result(page: &DataPage) -> Vec<DatabaseUserIdentity> {
    let user_index = column_index(page, &["user", "User"]);
    let host_index = column_index(page, &["host", "Host"]);
    let plugin_index = column_index(page, &["plugin", "Plugin"]);
    let (Some(user_index), Some(host_index)) = (user_index, host_index) else {
        return Vec::new();
    };

    page.rows
        .iter()
        .filter_map(|row| {
            let user = row.values.get(user_index)?.display_label();
            let host = row.values.get(host_index)?.display_label();
            if user.is_empty() && host.is_empty() {
                return None;
            }
            let plugin = plugin_index
                .and_then(|index| row.values.get(index))
                .map(CellValue::display_label)
                .filter(|value| !value.is_empty() && value != "NULL");
            Some(DatabaseUserIdentity { user, host, plugin })
        })
        .collect()
}

pub fn users_from_mysql_grantee_result(page: &DataPage) -> Vec<DatabaseUserIdentity> {
    let Some(grantee_index) = column_index(page, &["grantee", "GRANTEE"]) else {
        return Vec::new();
    };
    page.rows
        .iter()
        .filter_map(|row| row.values.get(grantee_index))
        .filter_map(|value| parse_mysql_grantee(&value.display_label()))
        .collect()
}

fn column_index(page: &DataPage, names: &[&str]) -> Option<usize> {
    page.columns
        .iter()
        .position(|column| names.iter().any(|name| column.name.eq_ignore_ascii_case(name)))
}

fn parse_mysql_grantee(value: &str) -> Option<DatabaseUserIdentity> {
    parse_mysql_account(value)
}

pub fn parse_mysql_account(value: &str) -> Option<DatabaseUserIdentity> {
    let value = value.trim();
    let quote = value.chars().next()?;
    if quote != '\'' && quote != '`' {
        return None;
    }
    let rest = &value[quote.len_utf8()..];
    let (user, rest) = parse_mysql_quoted_part(rest, quote)?;
    // MariaDB 的角色账号没有 host，`SHOW GRANTS` 里只打印 `` `role` ``；
    // 这里把缺失的 `@host` 视为空 host，与 `mysql.user` 里的角色行保持一致。
    let (host, rest) = match rest.strip_prefix('@') {
        Some(rest) => {
            let rest = rest.strip_prefix(quote)?;
            parse_mysql_quoted_part(rest, quote)?
        }
        None => (String::new(), rest),
    };
    if !rest.trim().is_empty() {
        return None;
    }
    Some(DatabaseUserIdentity {
        user,
        host,
        plugin: None,
    })
}

fn parse_mysql_quoted_part(mut input: &str, quote: char) -> Option<(String, &str)> {
    let mut value = String::new();
    loop {
        let next_quote = input.find(quote)?;
        value.push_str(&input[..next_quote]);
        input = &input[next_quote + 1..];
        if let Some(rest) = input.strip_prefix(quote) {
            value.push(quote);
            input = rest;
        } else {
            return Some((value, input));
        }
    }
}

fn roles_granted_to_user_from_grants(
    grants: &[String],
    user: &DatabaseUserIdentity,
) -> Vec<DatabaseUserIdentity> {
    let mut roles = Vec::new();
    for grant in grants {
        let trimmed = grant.trim().trim_end_matches(';');
        let Some(rest) = strip_ascii_prefix(trimmed, "GRANT ") else {
            continue;
        };
        let Some((roles_part, target_part)) = split_once_ascii(rest, " TO ") else {
            continue;
        };
        let target_part = trim_mysql_grant_suffix(target_part);
        if !parse_mysql_account(target_part)
            .as_ref()
            .is_some_and(|target| same_database_account(target, user))
        {
            continue;
        }
        for account in split_mysql_account_list(roles_part) {
            if let Some(role) = parse_mysql_account(account) {
                push_unique_role(&mut roles, role);
            }
        }
    }
    roles
}

fn default_roles_for_user_from_grants(
    grants: &[String],
    user: &DatabaseUserIdentity,
) -> Vec<DatabaseUserIdentity> {
    let mut roles = Vec::new();
    for grant in grants {
        let trimmed = grant.trim().trim_end_matches(';');
        let Some(rest) = strip_ascii_prefix(trimmed, "SET DEFAULT ROLE ") else {
            continue;
        };
        // MySQL 输出 `SET DEFAULT ROLE ... TO ...`，MariaDB 输出 `... FOR ...`。
        let Some((roles_part, target_part)) =
            split_once_ascii(rest, " TO ").or_else(|| split_once_ascii(rest, " FOR "))
        else {
            continue;
        };
        if !parse_mysql_account(target_part)
            .as_ref()
            .is_some_and(|target| same_database_account(target, user))
        {
            continue;
        }
        for account in split_mysql_account_list(roles_part) {
            if let Some(role) = parse_mysql_account(account) {
                push_unique_role(&mut roles, role);
            }
        }
    }
    roles
}

fn trim_mysql_grant_suffix(value: &str) -> &str {
    split_once_ascii(value, " WITH ADMIN OPTION")
        .map(|(account, _)| account)
        .unwrap_or(value)
        .trim()
}

fn strip_ascii_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .then_some(value[prefix.len()..].trim())
}

fn split_once_ascii<'a>(value: &'a str, needle: &str) -> Option<(&'a str, &'a str)> {
    value
        .to_ascii_uppercase()
        .find(&needle.to_ascii_uppercase())
        .map(|index| (&value[..index], &value[index + needle.len()..]))
}

fn split_mysql_account_list(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let chars = value.char_indices().peekable();
    for (index, ch) in chars {
        if matches!(ch, '\'' | '`') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
        } else if ch == ',' && quote.is_none() {
            parts.push(value[start..index].trim());
            start = index + ch.len_utf8();
        }
    }
    parts.push(value[start..].trim());
    parts
}

fn push_unique_role(roles: &mut Vec<DatabaseUserIdentity>, role: DatabaseUserIdentity) {
    if !roles
        .iter()
        .any(|existing| same_database_account(existing, &role))
    {
        roles.push(role);
    }
}

fn same_database_account(left: &DatabaseUserIdentity, right: &DatabaseUserIdentity) -> bool {
    left.user == right.user && left.host == right.host
}

#[cfg(test)]
mod mariadb_user_admin_tests {
    use super::{
        CreatePrincipalInput, DatabaseKind, DatabaseUserAdminProvider, DatabaseUserIdentity,
        UserAdminDialect, database_user_admin_provider,
    };

    fn provider_for(kind: DatabaseKind) -> DatabaseUserAdminProvider {
        database_user_admin_provider(kind).expect("provider")
    }

    fn account(user: &str) -> DatabaseUserIdentity {
        DatabaseUserIdentity {
            user: user.to_string(),
            host: "%".to_string(),
            plugin: None,
        }
    }

    #[test]
    fn mariadb_reuses_mysql_user_admin_sql() {
        let provider: DatabaseUserAdminProvider = provider_for(DatabaseKind::MariaDb);
        assert_eq!(provider.dialect(), UserAdminDialect::MySql);
        assert!(provider.list_users_sql().contains("mysql.user"));
        assert_eq!(provider.default_auth_plugin(), "mysql_native_password");
        assert!(!provider.auth_plugins().contains(&"caching_sha2_password"));
    }

    #[test]
    fn mariadb_create_user_uses_via_syntax() {
        let sql = provider_for(DatabaseKind::MariaDb).create_user_sql(&CreatePrincipalInput {
            user: "app".into(),
            host: "%".into(),
            password: "p@ss".into(),
            auth_plugin: Some("mysql_native_password".into()),
        });
        assert_eq!(
            sql,
            "CREATE USER 'app'@'%' IDENTIFIED VIA `mysql_native_password` USING PASSWORD('p@ss');"
        );
        let sql = provider_for(DatabaseKind::MySql).create_user_sql(&CreatePrincipalInput {
            user: "app".into(),
            host: "%".into(),
            password: "p@ss".into(),
            auth_plugin: Some("caching_sha2_password".into()),
        });
        assert!(sql.contains("IDENTIFIED WITH `caching_sha2_password` BY 'p@ss'"));
    }

    #[test]
    fn mariadb_alter_auth_plugin_and_default_roles_use_mariadb_keywords() {
        let provider = provider_for(DatabaseKind::MariaDb);
        assert_eq!(
            provider.alter_auth_plugin_sql(
                &account("app"),
                "mysql_native_password",
                Some("p@ss")
            ),
            "ALTER USER 'app'@'%' IDENTIFIED VIA `mysql_native_password` USING PASSWORD('p@ss');"
        );
        assert_eq!(
            provider.alter_auth_plugin_sql(&account("app"), "unix_socket", None),
            "ALTER USER 'app'@'%' IDENTIFIED VIA `unix_socket`;"
        );
        // MariaDB 的角色没有 host：角色一侧只能写角色名。
        let role = DatabaseUserIdentity {
            user: "role_a".into(),
            host: String::new(),
            plugin: None,
        };
        assert_eq!(
            provider.set_default_roles_sql(&account("app"), std::slice::from_ref(&role)),
            "SET DEFAULT ROLE 'role_a' FOR 'app'@'%';"
        );
        assert_eq!(
            provider.grant_role_sql(&role, &account("app")),
            "GRANT 'role_a' TO 'app'@'%';"
        );
        assert_eq!(
            provider.revoke_role_sql(&role, &account("app")),
            "REVOKE 'role_a' FROM 'app'@'%';"
        );
        assert_eq!(
            provider.show_grants_sql(&role),
            "SHOW GRANTS FOR 'role_a';"
        );
        assert_eq!(
            provider_for(DatabaseKind::MySql).grant_role_sql(&account("role_a"), &account("app")),
            "GRANT 'role_a'@'%' TO 'app'@'%';"
        );
        assert_eq!(
            provider_for(DatabaseKind::MySql).set_default_roles_sql(&account("app"), &[]),
            "SET DEFAULT ROLE NONE TO 'app'@'%';"
        );
    }

    #[test]
    fn mariadb_show_grants_default_role_line_is_parsed() {
        let grants = vec![
            "GRANT `role_a` TO `app`@`%`".to_string(),
            "SET DEFAULT ROLE `role_a` FOR `app`@`%`".to_string(),
        ];
        // MariaDB 的 `SHOW GRANTS` 对无显式 host 的角色只打印角色名，候选也要用空 host。
        let role = DatabaseUserIdentity {
            user: "role_a".into(),
            host: String::new(),
            plugin: None,
        };
        let memberships =
            super::role_memberships_from_grants(&[role], &grants, &account("app"));
        assert_eq!(memberships.len(), 1);
        assert!(memberships[0].granted);
        assert!(memberships[0].default_role);
    }
}
