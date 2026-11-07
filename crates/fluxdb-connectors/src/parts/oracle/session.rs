// Oracle 连接建立与错误归一化。
//
// 驱动 `oracledb` 是同步阻塞实现（TTC 直连），与达梦一致：调用方在短生命周期
// 线程里直接执行，不需要 runtime 包装。密码只从 `SecretRef` 取值，不写日志。

use oracledb::{
    Config as OracleConfig, Connection as OracleConnection, DbType as OracleDbType,
    Error as OracleError, ErrorKind as OracleErrorKind, JsonValue as OracleJsonValue,
    Lob as OracleLob, Metadata as OracleMetadata, OracleNumber, OracleTimestamp,
    Row as OracleRow, ToDbValue as OracleToDbValue,
};
use fluxdb_core::{OracleConnectionProfile, OraclePrivilege};

/// 绑定值：统一持有所有权，供 `&[&dyn ToDbValue]` 借用。
///
/// 不使用字符串插值；数值保持整数/浮点真实类型，避免把 NUMBER 拼成文本后
/// 由会话 NLS 设置决定解析结果。
#[derive(Clone)]
enum OracleBindValue {
    Text(String),
    Integer(i64),
    Float(f64),
    /// 超出 i64 范围或带小数的精确十进制。
    Number(OracleNumber),
    Bytes(Vec<u8>),
    Null,
}

/// 无类型 NULL 绑定。Oracle 需要显式声明绑定类型，这里用 VARCHAR2 空值。
static ORACLE_NULL_BIND: &OracleDbType = oracledb::DB_TYPE_VARCHAR;

impl OracleBindValue {
    fn as_db_value(&self) -> &dyn OracleToDbValue {
        match self {
            Self::Text(value) => value,
            Self::Integer(value) => value,
            Self::Float(value) => value,
            Self::Number(value) => value,
            Self::Bytes(value) => value,
            Self::Null => &ORACLE_NULL_BIND,
        }
    }
}

fn oracle_bind_refs(values: &[OracleBindValue]) -> Vec<&dyn OracleToDbValue> {
    values.iter().map(OracleBindValue::as_db_value).collect()
}

fn oracle_profile(config: &ConnectionConfig) -> fluxdb_core::Result<OracleConnectionProfile> {
    if config.kind != DatabaseKind::Oracle {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }
    let profile = config
        .oracle_profile
        .clone()
        .unwrap_or_else(|| OracleConnectionProfile::from_options(&config.options));
    if let Some(message) = profile.validate() {
        return Err(Error::new(ErrorKind::Connection, message));
    }
    Ok(profile)
}

/// 建立一条 Oracle 连接。每次调用独立登录，调用方负责在结束时关闭。
///
/// 会话 schema 取连接档案里的默认值；SQL 执行入口改用
/// [`oracle_connect_with_schema`] 让请求级 schema 生效。
fn oracle_connect(config: &ConnectionConfig) -> fluxdb_core::Result<OracleSession> {
    oracle_connect_with_schema(config, None)
}

/// Oracle 连接句柄：驱动连接 + 可能存在的 SSH 隧道。
///
/// 隧道必须与连接同生命周期存活；Deref 让既有 helper（接收 `&OracleConnection`）
/// 无需改动即可继续使用。
struct OracleSession {
    connection: OracleConnection,
    _tunnel: Option<SshTunnel>,
}

impl std::ops::Deref for OracleSession {
    type Target = OracleConnection;
    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl std::ops::DerefMut for OracleSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}

/// 建立连接并按需切换会话 schema，`request_schema` 优先于连接档案的默认值。
///
/// 只影响未限定名的解析目标，**不授予任何权限**：即使切到别的 owner，对象访问仍按
/// 用户实际拥有的授权判定。会话随连接结束而销毁，不存在跨请求残留。
fn oracle_connect_with_schema(
    config: &ConnectionConfig,
    request_schema: Option<&str>,
) -> fluxdb_core::Result<OracleSession> {
    let mut profile = oracle_profile(config)?;
    // SSH 隧道：连接串改写为本地转发端口；隧道随返回句柄存活。
    let tunnel = if profile.ssh.enabled {
        let target = (profile.basic.host.clone(), profile.basic.port);
        let tunnel = open_tunnel_for_profile(
            &profile.ssh,
            (&target.0, target.1),
            profile.advanced.connect_timeout_secs,
        )?;
        profile.basic.host = "127.0.0.1".into();
        profile.basic.port = tunnel.local_port;
        Some(tunnel)
    } else {
        None
    };
    let mut options = OracleConfig::default()
        .set_connect_string(&profile.connect_string())
        .map_err(|error| oracle_error("Oracle 连接串无效", error))?
        .set_credentials(
            profile.basic.username.trim(),
            profile.password().unwrap_or(""),
        );
    let auth_mode = match profile.basic.privilege {
        OraclePrivilege::Normal => None,
        OraclePrivilege::SysDba => Some(oracledb::AUTH_MODE_SYSDBA),
        OraclePrivilege::SysOper => Some(oracledb::AUTH_MODE_SYSOPER),
    };
    if let Some(mode) = auth_mode {
        options = options.set_auth_mode(mode);
    }
    let wallet_location = profile.advanced.wallet_location.trim();
    if !wallet_location.is_empty() {
        options = options.set_wallet_location(wallet_location);
        if let Some(password) = profile.wallet_password() {
            options = options.set_wallet_password(password);
        }
    }
    let application_name = profile.advanced.application_name.trim();
    if !application_name.is_empty() {
        options = options
            .set_program(application_name)
            .map_err(|error| oracle_error("Oracle 客户端程序名无效", error))?;
    }
    // 建连超时由驱动的 transport 层执行，覆盖 DNS/TCP/TLS 握手；0 表示不设限。
    if profile.advanced.connect_timeout_secs > 0 {
        options = options.set_transport_connect_timeout(Some(Duration::from_secs(u64::from(
            profile.advanced.connect_timeout_secs,
        ))));
    }
    let connection =
        oracledb::connect(options).map_err(|error| oracle_error("建立 Oracle 连接失败", error))?;
    if profile.advanced.query_timeout_secs > 0 {
        connection
            .set_call_timeout(Some(Duration::from_secs(u64::from(
                profile.advanced.query_timeout_secs,
            ))))
            .map_err(|error| oracle_error("设置 Oracle 语句超时失败", error))?;
    }
    oracle_set_iso_datetime_formats(&connection)?;
    let schema = oracle_session_schema(request_schema, profile.basic.schema.trim());
    if !schema.is_empty() {
        // 只改变未限定名的解析目标，不增加任何权限；对象引用仍带 Owner。
        let sql = format!(
            "ALTER SESSION SET CURRENT_SCHEMA = {}",
            fluxdb_core::OracleDialect.quote_identifier(schema)
        );
        oracle_execute(&connection, &sql, &[]).map_err(|error| {
            Error::new(
                error.kind,
                format!("切换 Oracle 会话 Schema「{schema}」失败：{}", error.message),
            )
        })?;
    }
    Ok(OracleSession {
        connection,
        _tunnel: tunnel,
    })
}

/// 把会话日期时间格式钉成 fluxDB 自己读写用的 ISO 形态。
///
/// 展示与导出都用 `YYYY-MM-DD HH24:MI:SS[.fffffffff]`，但 Oracle 把 VARCHAR 绑定写入
/// `DATE`/`TIMESTAMP` 列时按会话 `NLS_DATE_FORMAT`（默认 `DD-MON-RR`）解析，导入、
/// 行编辑和筛选都会撞 `ORA-01861`。显式设置后两端格式一致，不依赖服务端默认值。
///
/// 只能影响本会话：每次 `oracle_connect*` 都独立登录，不会残留到别的连接。
fn oracle_set_iso_datetime_formats(connection: &OracleConnection) -> fluxdb_core::Result<()> {
    oracle_execute(
        connection,
        "ALTER SESSION SET \
         NLS_DATE_FORMAT = 'YYYY-MM-DD HH24:MI:SS' \
         NLS_TIMESTAMP_FORMAT = 'YYYY-MM-DD HH24:MI:SS.FF9' \
         NLS_TIMESTAMP_TZ_FORMAT = 'YYYY-MM-DD HH24:MI:SS.FF9 TZH:TZM'",
        &[],
    )
    .map_err(|error| {
        Error::new(
            error.kind,
            format!("设置 Oracle 会话日期格式失败：{}", error.message),
        )
    })?;
    Ok(())
}

/// 在连接上开启只读事务：服务端拒绝 DML 与 `SELECT ... FOR UPDATE`（`ORA-01456`）。
///
/// 必须在本会话第一次数据库调用之前发送，且必须排在 `ALTER SESSION` 之后：
/// `ALTER SESSION` 会隐式提交，先开只读事务会被它立刻结束。
///
/// 它挡不住 DDL（隐式提交会结束只读事务并另开读写事务），也挡不住 `NEXTVAL`、
/// 自治事务与 dblink 写入，因此调用方仍要先在执行前拒绝非查询语句，这一层只负责
/// 兜住分类器漏掉的 DML。
fn oracle_begin_read_only(connection: &OracleConnection) -> fluxdb_core::Result<()> {
    oracle_execute(connection, "SET TRANSACTION READ ONLY", &[]).map_err(|error| {
        Error::new(
            error.kind,
            format!("开启 Oracle 只读事务失败：{}", error.message),
        )
    })?;
    Ok(())
}

/// 决定会话要切换到的 schema：请求级优先，回退连接档案默认值。
///
/// 返回空串表示两者都没有，此时不发 `ALTER SESSION`，未限定名落到登录用户自己的 schema。
fn oracle_session_schema<'a>(request_schema: Option<&'a str>, profile_schema: &'a str) -> &'a str {
    request_schema
        .map(str::trim)
        .filter(|schema| !schema.is_empty())
        .unwrap_or(profile_schema)
}

/// Oracle 错误归一化。保留 ORA 错误码便于排障，但绝不把连接串或密码带进消息。
fn oracle_error(prefix: &str, error: OracleError) -> Error {
    let kind = match error.kind() {
        OracleErrorKind::DbError(db) => match db.code() {
            // 账号/口令/锁定/过期：属于身份验证失败，不是网络问题。
            1017 | 28000 | 28001 | 28002 => ErrorKind::Authentication,
            1031 => ErrorKind::Permission,
            // 监听器拒绝、服务/SID 未注册、实例不可用。
            1034 | 12154 | 12505 | 12514 | 12541 | 12545 | 12560 | 12570 | 12571 => {
                ErrorKind::Connection
            }
            _ => ErrorKind::Query,
        },
        OracleErrorKind::CallTimeoutExceeded => ErrorKind::Timeout,
        OracleErrorKind::DeadConnection
        | OracleErrorKind::NotConnected
        | OracleErrorKind::ListenerRefusedConnection(..)
        | OracleErrorKind::InvalidServiceName(..)
        | OracleErrorKind::InvalidSid(..)
        | OracleErrorKind::InvalidConnectString(..)
        | OracleErrorKind::InvalidRedirect(..)
        | OracleErrorKind::NoConnectString
        | OracleErrorKind::NoCredentials
        | OracleErrorKind::TnsAliasNotFound(..)
        | OracleErrorKind::NoConfigDir
        | OracleErrorKind::StreamOperation
        | OracleErrorKind::TlsOperation
        | OracleErrorKind::WalletUnreadable(..) => ErrorKind::Connection,
        OracleErrorKind::ServerVersionNotSupported => ErrorKind::Unsupported,
        _ => ErrorKind::Query,
    };
    let message = match error.kind() {
        OracleErrorKind::DbError(db) => format!("ORA-{:05}: {}", db.code(), db.message()),
        _ => error.to_string(),
    };
    Error::new(kind, format!("{prefix}：{message}"))
}

/// 执行一条不需要结果集的语句（DDL、SET 等）。
fn oracle_execute(
    connection: &OracleConnection,
    sql: &str,
    values: &[OracleBindValue],
) -> fluxdb_core::Result<u64> {
    let refs = oracle_bind_refs(values);
    let mut statement = connection
        .statement(sql)
        .map_err(|error| oracle_error("解析 Oracle 语句失败", error))?
        .exclude_from_cache()
        .build()
        .map_err(|error| oracle_error("解析 Oracle 语句失败", error))?;
    statement
        .execute(&refs)
        .map(|result| result.rows_affected())
        .map_err(|error| oracle_error("Oracle 语句执行失败", error))
}

fn oracle_qualified_table(schema: &str, table: &str) -> String {
    format!(
        "{}.{}",
        fluxdb_core::OracleDialect.quote_identifier(schema),
        fluxdb_core::OracleDialect.quote_identifier(table)
    )
}

fn oracle_request_schema(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<String> {
    path.schema
        .clone()
        .filter(|schema| !schema.trim().is_empty())
        .or_else(|| {
            config
                .oracle_profile
                .as_ref()
                .map(|profile| profile.basic.schema.clone())
                .filter(|schema| !schema.trim().is_empty())
        })
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少 Oracle schema 上下文"))
}

/// Oracle 未加引号的标识符折叠为大写，目录视图里存的是准确名字。
/// 这里按目录结果反查一次，避免把用户输入的小写名字直接加引号后找不到对象。
fn oracle_resolve_owner(
    connection: &OracleConnection,
    requested: &str,
) -> fluxdb_core::Result<String> {
    let rows = oracle_query(
        connection,
        // 驱动按占位符出现次数校验绑定，所以大小写两种写法各绑一次，不能复用 `:1`。
        "SELECT USERNAME FROM ALL_USERS WHERE USERNAME = :1 OR USERNAME = :2",
        &[
            OracleBindValue::Text(requested.trim().to_string()),
            OracleBindValue::Text(requested.trim().to_uppercase()),
        ],
    )?;
    let mut rows = rows;
    if let Some(owner) = rows
        .first_mut()
        .and_then(|row| oracle_meta_text(row, 0))
    {
        return Ok(owner);
    }
    // 目录里看不到（无权限）时保留用户输入，让后续查询给出真实的 ORA 错误，
    // 而不是伪造一个“对象不存在”。
    Ok(requested.trim().to_string())
}

/// 执行查询并收集结果行。
///
/// 一律 `exclude_from_cache`：同一个 SQL 在数据页/筛选里会用不同类型重绑（文本、
/// 整数、浮点），语句缓存遇到类型变化会直接报 `DifferentTypes`。这里宁可每次
/// 重新解析，也不让用户看到莫名其妙的绑定错误。
///
/// 一律 `fetch_lobs`：驱动默认把 CLOB/NCLOB/BLOB 改写成 `LONG`/`LONG RAW`，
/// 于是 `CellValue` 层看到的不再是 LOB，只能把整块内容物化进内存（`has_more`
/// 之外没有任何上界），空 CLOB 还会被压成 NULL。保留定位符才能按预览上限有界读取，
/// 也才能把「空 CLOB」和 NULL 区分开。代价是每个 LOB 列多一次往返，只发生在真正
/// 选了 LOB 列的查询上。
fn oracle_query(
    connection: &OracleConnection,
    sql: &str,
    values: &[OracleBindValue],
) -> fluxdb_core::Result<Vec<OracleRow>> {
    let refs = oracle_bind_refs(values);
    let statement = connection
        .statement(sql)
        .map_err(|error| oracle_error("Oracle 查询解析失败", error))?
        .exclude_from_cache()
        .fetch_lobs()
        .build()
        .map_err(|error| oracle_error("Oracle 查询解析失败", error))?;
    let cursor = statement
        .query(&refs)
        .map_err(|error| oracle_error("Oracle 查询失败", error))?;
    collect_oracle_cursor(cursor)
}

fn collect_oracle_cursor(
    cursor: oracledb::Cursor,
) -> fluxdb_core::Result<Vec<OracleRow>> {
    cursor
        .map(|row| row.map_err(|error| oracle_error("Oracle 读取结果失败", error)))
        .collect()
}

#[cfg(test)]
mod oracle_session_schema_tests {
    use super::oracle_session_schema;

    #[test]
    fn request_schema_wins_over_profile() {
        assert_eq!(oracle_session_schema(Some("HR"), "SALES"), "HR");
    }

    #[test]
    fn blank_or_missing_request_schema_falls_back_to_profile() {
        assert_eq!(oracle_session_schema(Some("   "), "SALES"), "SALES");
        assert_eq!(oracle_session_schema(Some(""), "SALES"), "SALES");
        assert_eq!(oracle_session_schema(None, "SALES"), "SALES");
    }

    #[test]
    fn both_empty_means_no_alter_session() {
        assert_eq!(oracle_session_schema(None, ""), "");
        assert_eq!(oracle_session_schema(Some("  "), ""), "");
    }
}
