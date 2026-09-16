// PostgreSQL 建库/删库（T07）。
//
// PostgreSQL 的 `CREATE DATABASE` / `DROP DATABASE` 不能运行于事务块内，且不能使用
// 扩展查询协议（parse/bind/execute 会隐式包裹事务）。因此这里用 `batch_execute`
// （simple query protocol，autocommit）执行。字符集/排序映射到 ENCODING / LC_COLLATE /
// LC_CTYPE；标识符按 PG 规则双引号引用（内部 `"` 转义为 `""`）。

/// 建库：ENCODING 取 charset，LC_COLLATE/LC_CTYPE 取 collation。
fn pg_create_database(
    config: &ConnectionConfig,
    request: &CreateDatabaseRequest,
) -> fluxdb_core::Result<()> {
    if config.kind != DatabaseKind::Postgres {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持新建数据库"));
    }
    if config.id != request.connection_id {
        return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
    }
    let sql = pg_create_database_sql(request)?;
    let database = pg_request_database(config, None);
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        // simple query protocol + autocommit：CREATE DATABASE 合法。
        session
            .client
            .batch_execute(&sql)
            .await
            .map_err(pg_error)?;
        Ok(())
    })
}

fn pg_create_database_sql(request: &CreateDatabaseRequest) -> fluxdb_core::Result<String> {
    let database = request.name.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "数据库名称不能为空"));
    }
    let mut statement = format!("CREATE DATABASE {}", pg_quote_identifier(database));

    let charset = request.charset.trim();
    if !charset.is_empty() {
        if !is_pg_encoding_name(charset) {
            return Err(Error::new(ErrorKind::Query, "编码(字符集)名称不合法"));
        }
        statement.push_str(&format!(" ENCODING '{charset}'"));
    }

    let collation = request.collation.trim();
    if !collation.is_empty() {
        if !is_pg_locale_name(collation) {
            return Err(Error::new(ErrorKind::Query, "排序规则名称不合法"));
        }
        // LC_COLLATE 与 LC_CTYPE 同为排序规则；locale 形如 `zh_CN.UTF-8`、`C`、`POSIX`。
        statement.push_str(&format!(" LC_COLLATE '{collation}' LC_CTYPE '{collation}'"));
    }

    // OWNER：角色名按标识符引用（非 schema 限定；空不指定）。
    let owner = request.owner.trim();
    if !owner.is_empty() {
        if !is_pg_identifier_name(owner) {
            return Err(Error::new(ErrorKind::Query, "数据库 owner 名称不合法"));
        }
        statement.push_str(&format!(" OWNER {}", pg_quote_identifier(owner)));
    }

    // TEMPLATE：模板库名按标识符引用（空不指定）。
    let template = request.template.trim();
    if !template.is_empty() {
        if !is_pg_identifier_name(template) {
            return Err(Error::new(ErrorKind::Query, "数据库模板名称不合法"));
        }
        statement.push_str(&format!(" TEMPLATE {}", pg_quote_identifier(template)));
    } else if !collation.is_empty() {
        // 显式指定 locale（LC_COLLATE/LC_CTYPE）时，若沿用默认模板 template1，
        // 其 locale 与目标不一致会报「new collation is incompatible with the
        // collation of the template database」。此时改用 template0（无 locale 依赖），
        // 与 PG 官方 HINT 一致，保证任意 locale 都能建库。
        statement.push_str(" TEMPLATE template0");
    }

    Ok(statement)
}

/// 角色/模板库名校验：纯标识符（字母/数字/下划线/`$`），阻止注入空格/引号/分号。
fn is_pg_identifier_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
}

/// 「可加引号的对象名」校验：用于会被 `pg_quote_identifier` 双引号引用后下发的名字
/// （schema 等）。引用后引号内不存在注入面，故允许中文/空格/大写等真实合法名字；
/// 只拒绝 PG 标识符本身不接受的内容：空、NUL/控制字符、超过 63 字节（PG 会静默截断）。
fn is_pg_quotable_object_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.chars().any(|ch| ch == '\0' || ch.is_control())
}

/// 编码名校验：字母/数字/下划线（UTF8、SQL_ASCII、LATIN1 等）。
fn is_pg_encoding_name(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// locale/排序规则名校验：允许字母/数字/下划线/点(TC)/连字符，阻止注入分号/引号。
fn is_pg_locale_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// 删库：连接维护库执行。PG 天然拒绝删“当前打开的库”；这里额外保护维护库，
/// 且不追加 `WITH (FORCE)` —— 有活动连接的删库按 PG 默认 RESTRICT 语义失败。
/// 建 schema：在**目标数据库**的独立 autocommit 连接执行 `CREATE SCHEMA "name"`。
///
/// PG 的 schema 隶属于某个数据库，不是集群级对象：必须连到用户选中的那个库执行，
/// 否则 schema 会落到维护库里（用户在目标库刷新永远看不到它）。`database` 为空时
/// 才回退维护库。schema 名按标识符引用后下发，允许中文/空格/大写等需要引号的名字。
fn pg_create_schema(
    config: &ConnectionConfig,
    _connection_id: ConnectionId,
    database: &str,
    schema: &str,
) -> fluxdb_core::Result<()> {
    let schema = schema.trim();
    if schema.is_empty() {
        return Err(Error::new(ErrorKind::Query, "schema 名称不能为空"));
    }
    if !is_pg_quotable_object_name(schema) {
        return Err(Error::new(ErrorKind::Query, "schema 名称不合法"));
    }
    let database = pg_request_database(config, Some(database));
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        let sql = format!("CREATE SCHEMA {}", pg_quote_identifier(schema));
        tracing::debug!(
            target: "fluxdb_connectors",
            database = %database,
            "CREATE SCHEMA 执行"
        );
        session
            .client
            .batch_execute(&sql)
            .await
            .map_err(pg_error)
    })
}

fn pg_delete_database(
    config: &ConnectionConfig,
    connection_id: ConnectionId,
    database: &str,
) -> fluxdb_core::Result<()> {
    if config.kind != DatabaseKind::Postgres {
        return Err(Error::new(ErrorKind::Connection, "连接类型不支持删除数据库"));
    }
    if config.id != connection_id {
        return Err(Error::new(ErrorKind::Connection, "连接不匹配"));
    }
    let database = database.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "数据库名称不能为空"));
    }
    let maintenance = pg_request_database(config, None);
    if database == maintenance {
        // 当前维护库保护：不通过“先断连再删”绕过，直接拒绝，避免误删正在使用的库。
        return Err(Error::new(
            ErrorKind::Query,
            format!("不能删除当前维护库 {database}"),
        ));
    }
    let sql = format!("DROP DATABASE {}", pg_quote_identifier(database));
    pg_runtime().block_on(async {
        let session = pg_connect(config, &maintenance).await?;
        // simple query protocol + autocommit：DROP DATABASE 合法。
        session
            .client
            .batch_execute(&sql)
            .await
            .map_err(pg_error)?;
        Ok(())
    })
}

/// PostgreSQL 标识符引用：双引号包裹，内部 `"` 转义为 `""`。
fn pg_quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod create_database_tests {
    use super::*;
    use fluxdb_core::ConnectionId;

    fn request(name: &str, collation: &str, template: &str) -> CreateDatabaseRequest {
        CreateDatabaseRequest {
            connection_id: ConnectionId(1),
            name: name.to_string(),
            charset: "UTF8".to_string(),
            collation: collation.to_string(),
            owner: String::new(),
            template: template.to_string(),
            path: None,
        }
    }

    /// 显式 locale 且未指定模板 → 追加 TEMPLATE template0，规避模板 locale 冲突。
    #[test]
    fn collation_infers_template0() {
        let sql = pg_create_database_sql(&request("db1", "C", "")).unwrap();
        assert!(sql.contains(" LC_COLLATE 'C' LC_CTYPE 'C'"));
        assert!(sql.contains(" TEMPLATE template0"));
    }

    /// 未指定 locale → 不加 TEMPLATE（沿用默认模板 template1）。
    #[test]
    fn no_collation_no_template_clause() {
        let sql = pg_create_database_sql(&request("db1", "", "")).unwrap();
        assert!(!sql.contains("TEMPLATE"));
    }

    /// 用户显式指定模板 → 尊重用户模板，不再自动改 template0。
    #[test]
    fn explicit_template_respected() {
        let sql = pg_create_database_sql(&request("db1", "C", "my_template")).unwrap();
        assert!(sql.contains("TEMPLATE \"my_template\""));
        assert!(!sql.contains("template0"));
    }
}
