#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserAdminDialect {
    MySql,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivilegeScope {
    MySql,
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
}

impl DatabaseUserAdminProvider {
    pub fn dialect(self) -> UserAdminDialect {
        self.dialect
    }

    pub fn default_scope(self) -> PrivilegeScope {
        self.default_scope
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
            UserAdminDialect::MySql => format!("SHOW GRANTS FOR {};", mysql_user_account(user)),
        }
    }

    pub fn create_user_sql(self, input: &CreatePrincipalInput) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                let auth_plugin = input
                    .auth_plugin
                    .as_deref()
                    .map(str::trim)
                    .filter(|plugin| !plugin.is_empty())
                    .map(|plugin| format!(" WITH {}", quote_mysql_identifier(plugin)))
                    .unwrap_or_default();
                format!(
                    "CREATE USER {} IDENTIFIED{} BY {};",
                    mysql_user_account(&DatabaseUserIdentity {
                        user: input.user.clone(),
                        host: input.host.clone(),
                        plugin: None,
                    }),
                    auth_plugin,
                    quote_mysql_string(&input.password)
                )
            }
        }
    }

    pub fn alter_password_sql(self, user: &DatabaseUserIdentity, password: &str) -> String {
        match self.dialect {
            UserAdminDialect::MySql => format!(
                "ALTER USER {} IDENTIFIED BY {};",
                mysql_user_account(user),
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
                let password_sql = password
                    .map(|password| format!(" BY {}", quote_mysql_string(password)))
                    .unwrap_or_default();
                format!(
                    "ALTER USER {} IDENTIFIED WITH {}{};",
                    mysql_user_account(user),
                    quote_mysql_identifier(plugin.trim()),
                    password_sql
                )
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
                format!("ALTER USER {} {};", mysql_user_account(user), suffix)
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
                format!("ALTER USER {} WITH {};", mysql_user_account(user), clauses.join(" "))
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
                format!("ALTER USER {} REQUIRE {};", mysql_user_account(user), requirement)
            }
        })
    }

    pub fn alter_login_sql(self, user: &DatabaseUserIdentity, enabled: bool) -> String {
        match self.dialect {
            UserAdminDialect::MySql => format!(
                "ALTER USER {} ACCOUNT {};",
                mysql_user_account(user),
                if enabled { "UNLOCK" } else { "LOCK" }
            ),
        }
    }

    pub fn drop_user_sql(self, user: &DatabaseUserIdentity) -> String {
        match self.dialect {
            UserAdminDialect::MySql => format!("DROP USER {};", mysql_user_account(user)),
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
                    mysql_user_account(&input.user),
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
                    mysql_user_account(&input.user)
                )
            }
        }
    }

    pub fn grant_role_sql(self, role: &DatabaseUserIdentity, user: &DatabaseUserIdentity) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                format!(
                    "GRANT {} TO {};",
                    mysql_user_account(role),
                    mysql_user_account(user)
                )
            }
        }
    }

    pub fn revoke_role_sql(self, role: &DatabaseUserIdentity, user: &DatabaseUserIdentity) -> String {
        match self.dialect {
            UserAdminDialect::MySql => {
                format!(
                    "REVOKE {} FROM {};",
                    mysql_user_account(role),
                    mysql_user_account(user)
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
                        .map(mysql_user_account)
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!("SET DEFAULT ROLE {} TO {};", roles_sql, mysql_user_account(user))
            }
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
        DatabaseKind::MySql | DatabaseKind::TiDb => Some(DatabaseUserAdminProvider {
            dialect: UserAdminDialect::MySql,
            default_scope: PrivilegeScope::MySql,
        }),
        DatabaseKind::Sqlite | DatabaseKind::MongoDb | DatabaseKind::Redis => None,
    }
}

pub fn supports_database_user_admin(kind: DatabaseKind) -> bool {
    database_user_admin_provider(kind).is_some()
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

pub fn quote_mysql_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
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
    let rest = rest.strip_prefix('@')?;
    let rest = rest.strip_prefix(quote)?;
    let (host, rest) = parse_mysql_quoted_part(rest, quote)?;
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
        let Some((roles_part, target_part)) = split_once_ascii(rest, " TO ") else {
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
