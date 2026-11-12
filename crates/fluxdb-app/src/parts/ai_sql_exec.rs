// AI SQL 执行服务（设计 §20–21，T023/T024）。
//
// 目标：把「元数据探索 + 受控 SQL 执行」做成**独立服务**，供 Agent 工具（将来也给 MCP）
// 复用同一执行链；不依赖 GPUI，也不复用编辑器标签的会话——Agent 用隔离连接执行，
// 避免与用户编辑器的临时表 / 事务互相串（设计 §21.6）。
//
// 硬规则（每条都有对应测试）：
// 1. **单条语句**：`split_statements=false`、`continue_on_error=false`，多语句由策略层拒绝；
// 2. **只读入口只放只读**：内部查询执行器只执行分类为 `ReadOnly` 的语句，
//    其余一律拒绝并给出可见原因，不"降级执行"；
// 3. **结果有界**：`max_rows` 行 + 字节上限，超限只标记 `truncated`，不静默丢；
// 4. **可取消**：在连接器检查点轮询 `ToolExecutionContext.cancel`，取消返回真实状态；
// 5. **参数化**：当前 Connector 尚无 bind 能力，因此**拒绝**带参数的请求（返回受控错误），
//    而不是退化成字符串拼接（设计 §20.3）。

/// 连接解析：把 AI 的 target 句柄映射到连接配置。
///
/// 只暴露非敏感配置（kind/endpoint/options）；凭据解密发生在 connector 层，
/// 不经过这里，也不进任何事件/日志。
pub trait SqlExecutionResolver: Send + Sync {
    fn sql_object_allowed(&self, _target:&AiSqlTarget, _name:&str)->bool { true }
    fn connection_config(
        &self,
        id: fluxdb_core::ConnectionId,
    ) -> Option<fluxdb_core::ConnectionConfig>;

    /// 该连接下 AI 工具被授权的 Redis Key 范围。
    ///
    /// 默认按用户显式选中的连接放行该 DB 全部 Key——这是引入 prefix 概念之前的既有
    /// 行为，写命令仍逐次审批。Server Profile 的 `all_keys` / `key_prefixes` 授权资源
    /// （设计 §28.6.1，T057）落地后由实现方返回 `Prefixes` 收紧，不得由模型声明范围。
    fn redis_key_scope(
        &self,
        _id: fluxdb_core::ConnectionId,
        database: u32,
    ) -> RedisKeyScope {
        RedisKeyScope::AllKeys { database }
    }

    /// 返回当前 Agent 执行上下文专属的查询会话标识。
    ///
    /// 该标识落在宿主托管段（`QuerySessionId::host_managed`），与编辑器查询标签
    /// 从 1 起的 ID 段天然不重叠，因此后端不会把 AI 与编辑器当作同一条会话。
    /// 默认只读读取是「每次有界独立事务」：单次调用结束即释放后端会话，
    /// 不把临时表/未提交状态长期挂着（设计 §21）。
    fn query_session_id(&self) -> fluxdb_core::QuerySessionId;
}

/// 内存实现：给测试与未来运行时装"当前可见连接"用。
pub struct MapSqlResolver {
    sql_object_scopes: HashMap<(fluxdb_core::ConnectionId,Option<String>,Option<String>),BTreeSet<String>>,
    configs: HashMap<fluxdb_core::ConnectionId, fluxdb_core::ConnectionConfig>,
    query_session_id: fluxdb_core::QuerySessionId,
    /// 按连接给定的 Redis Key 授权范围；缺项表示未配置（按全 Key 处理）。
    redis_key_scopes: HashMap<fluxdb_core::ConnectionId, RedisKeyScope>,
}

impl MapSqlResolver {
    pub fn new(configs: impl IntoIterator<Item = fluxdb_core::ConnectionConfig>) -> Self {
        Self {
            sql_object_scopes:HashMap::new(),
            configs: configs
                .into_iter()
                .map(|config| (config.id, config))
                .collect(),
            query_session_id: fluxdb_core::QuerySessionId::host_managed(
                NEXT_AI_QUERY_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ),
            redis_key_scopes: HashMap::new(),
        }
    }

    pub fn with_sql_object_scope(mut self,target:&AiSqlTarget,names:BTreeSet<String>)->Self {
        self.sql_object_scopes.insert((target.connection_id,target.database.clone(),target.schema.clone()),names);
        self
    }

    /// 收紧指定连接的 Redis Key 授权范围（prefix 授权的接入点）。
    pub fn with_redis_key_scope(
        mut self,
        connection_id: fluxdb_core::ConnectionId,
        scope: RedisKeyScope,
    ) -> Self {
        self.redis_key_scopes.insert(connection_id, scope);
        self
    }
}

impl SqlExecutionResolver for MapSqlResolver {
    fn sql_object_allowed(&self,target:&AiSqlTarget,name:&str)->bool {
        self.sql_object_scopes.get(&(target.connection_id,target.database.clone(),target.schema.clone()))
            .is_none_or(|names|names.contains(name))
    }
    fn connection_config(
        &self,
        id: fluxdb_core::ConnectionId,
    ) -> Option<fluxdb_core::ConnectionConfig> {
        self.configs.get(&id).cloned()
    }

    fn redis_key_scope(&self, id: fluxdb_core::ConnectionId, database: u32) -> RedisKeyScope {
        // prompt / 工具参数不能声明范围；授权只来自连接配置与显式 API。
        self.redis_key_scopes
            .get(&id)
            .cloned()
            .or_else(|| {
                self.configs.get(&id).and_then(|config| {
                    let raw = config.options.get("ai_key_prefixes")?;
                    let prefixes = raw
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    // 配了空串 / 全逗号视同未配置：仍走既有默认行为而非「一个 key 都不许」。
                    if prefixes.is_empty() {
                        None
                    } else {
                        Some(RedisKeyScope::Prefixes { database, prefixes })
                    }
                })
            })
            .unwrap_or(RedisKeyScope::AllKeys { database })
    }

    fn query_session_id(&self) -> fluxdb_core::QuerySessionId {
        self.query_session_id
    }
}

/// AI 会话自增段：与编辑器查询标签 ID 段隔离，避免同一连接上命中同一条后端会话。
static NEXT_AI_QUERY_SESSION_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// 结果上限（工具层硬上限，模型不能放大）。
pub const SQL_MAX_ROWS_HARD: u64 = 1000;
pub const SQL_MAX_ROWS_DEFAULT: u64 = 200;
pub const SQL_RESULT_MAX_BYTES: usize = 128 * 1024;
/// 结果仓库的内存预算（与工具层投影上限无关）。
///
/// 必须小于 `SQL_RESULT_MAX_BYTES`：否则工具投影已被截断到 128 KiB，
/// 仓库永远装得下、溢写分支不可达，spill / retained 在生产里就成了死代码。
pub const RESULT_STORE_MEMORY_BYTES: usize = 32 * 1024;

/// 受控执行错误（作为 ToolReceipt 的 `error_code`，不是 panic/Err）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlExecError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl SqlExecError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
        }
    }
}

/// AI 执行目标（不含凭据）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiSqlTarget {
    pub connection_id: fluxdb_core::ConnectionId,
    pub database: Option<String>,
    pub schema: Option<String>,
}

impl AiSqlTarget {
    /// 解析内核提供的资源句柄：`conn:<id>[/db:<database>][/schema:<schema>]`。
    ///
    /// 句柄是**定位信息**，不是连接字符串：解析结果只用于选连接与默认作用域，
    /// 不允许模型借它注入任意 endpoint。
    pub fn parse(handle: &str) -> Result<Self, SqlExecError> {
        let mut parts = handle.split('/');
        let head = parts.next().unwrap_or_default();
        let id_text = head
            .strip_prefix("conn:")
            .or_else(|| head.strip_prefix("connection:"))
            .ok_or_else(|| {
                SqlExecError::new(
                    "invalid_target",
                    "target 必须是内核句柄（conn:<id>[/db:<database>][/schema:<schema>]）",
                )
            })?;
        let connection_id = id_text.parse::<u64>().map_err(|_| {
            SqlExecError::new(
                "invalid_target",
                format!(
                    "target 连接 ID 非法: {id_text}；合法格式：conn:<id>[/db:<database>][/schema:<schema>]（id 为数字）"
                ),
            )
        })?;
        let mut database = None;
        let mut schema = None;
        for part in parts {
            if let Some(value) = part.strip_prefix("db:") {
                database = Some(value.to_string());
            } else if let Some(value) = part.strip_prefix("schema:") {
                schema = Some(value.to_string());
            } else if !part.is_empty() {
                return Err(SqlExecError::new(
                    "invalid_target",
                    format!(
                        "target 片段无法识别: {part}；合法格式：conn:<id>[/db:<database>][/schema:<schema>]"
                    ),
                ));
            }
        }
        Ok(Self {
            connection_id: fluxdb_core::ConnectionId(connection_id),
            database,
            schema,
        })
    }
}

/// 连接在 target 未带库时可用的默认库（口径同连接器 `mysql_database_for_path`）。
pub(crate) fn default_database(config: &fluxdb_core::ConnectionConfig) -> Option<&str> {
    match &config.endpoint {
        fluxdb_core::Endpoint::Tcp {
            database: Some(database),
            ..
        } => Some(database.trim()).filter(|value| !value.is_empty()),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AiSqlObjectRef {
    database: Option<String>,
    schema: Option<String>,
    name: String,
    kind: fluxdb_core::ObjectKind,
}

impl AiSqlObjectRef {
    fn parse(
        value: &serde_json::Value,
        target: &AiSqlTarget,
    ) -> Result<Self, SqlExecError> {
        let object = value
            .as_object()
            .ok_or_else(|| SqlExecError::new("invalid_object_ref", "object_ref 必须是对象"))?;
        let name = object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| SqlExecError::new("invalid_object_ref", "object_ref 缺少 name"))?;
        let kind = match object
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| SqlExecError::new("invalid_object_ref", "object_ref 缺少 kind"))?
        {
            "table" => fluxdb_core::ObjectKind::Table,
            "view" => fluxdb_core::ObjectKind::View,
            other => {
                return Err(SqlExecError::new(
                    "invalid_object_ref",
                    format!("不支持的对象类型: {other}"),
                ));
            }
        };
        let database = object
            .get("database")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| target.database.clone());
        let schema = object
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| target.schema.clone());
        if target.database.is_some() && database != target.database {
            return Err(SqlExecError::new(
                "scope_mismatch",
                "object_ref 的 database 超出当前 target 作用域；删除 object_ref.database，或让 target 的 db 与之一致",
            ));
        }
        if target.schema.is_some() && schema != target.schema {
            return Err(SqlExecError::new(
                "scope_mismatch",
                "object_ref 的 schema 超出当前 target 作用域；删除 object_ref.schema，或让 target 的 schema 与之一致",
            ));
        }
        Ok(Self {
            database,
            schema,
            name: name.to_string(),
            kind,
        })
    }

    fn to_path(&self, connection_id: fluxdb_core::ConnectionId) -> fluxdb_core::ObjectPath {
        fluxdb_core::ObjectPath {
            connection_id,
            database: self.database.clone(),
            schema: self.schema.clone(),
            name: self.name.clone(),
            kind: self.kind,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "database": self.database,
            "schema": self.schema,
            "name": self.name,
            "kind": match self.kind {
                fluxdb_core::ObjectKind::Table => "table",
                fluxdb_core::ObjectKind::View => "view",
                _ => "unknown",
            },
        })
    }
}

/// 解析后的 SQL 工具入参。
#[derive(Clone, Debug)]
struct ParsedSqlArgs {
    target: AiSqlTarget,
    sql: String,
    max_rows: u64,
    /// EXPLAIN 的模式（`plan` 默认；`execute` 当前不开放）。
    mode: Option<String>,
    /// 精确 tagged 参数（驱动真实 bind；不支持绑定的连接器会明确拒绝）。
    parameters: Vec<fluxdb_core::SqlParameter>,
}

fn parse_sql_args(arguments_json: &str) -> Result<ParsedSqlArgs, SqlExecError> {
    let value: serde_json::Value = serde_json::from_str(arguments_json)
        .map_err(|_| SqlExecError::new("invalid_arguments", "工具入参不是合法 JSON"))?;
    let handle = value
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SqlExecError::new("invalid_arguments", "缺少 target"))?;
    let sql = value
        .get("sql")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SqlExecError::new("invalid_arguments", "缺少 sql"))?
        .to_string();
    if sql.trim().is_empty() {
        return Err(SqlExecError::new("invalid_arguments", "sql 不能为空"));
    }
    let parameters = parse_sql_parameters(value.get("parameters"))?;
    let max_rows = value
        .get("max_rows")
        .and_then(|v| v.as_u64())
        .unwrap_or(SQL_MAX_ROWS_DEFAULT)
        .clamp(1, SQL_MAX_ROWS_HARD);
    let mode = value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());
    Ok(ParsedSqlArgs {
        target: AiSqlTarget::parse(handle)?,
        sql,
        max_rows,
        mode,
        parameters,
    })
}

/// 参数上限：schema 允许 1024 项，但执行链给更小的硬上限（避免一次调用把驱动压垮）。
const SQL_PARAM_MAX_ITEMS: usize = 64;
const SQL_PARAM_MAX_TEXT_BYTES: usize = 16 * 1024;
const SQL_PARAM_MAX_BYTES: usize = 64 * 1024;
const SQL_PARAM_MAX_DECIMAL_CHARS: usize = 64;

/// 解析设计 §20.2 的 tagged wire 值：`{type:"text",value:"..."}` / `{type:"null"}` / …
///
/// 只做**类型与范围校验**，值原样交给驱动 bind；这里绝不把值格式化成 SQL 文本。
fn parse_sql_parameters(
    value: Option<&serde_json::Value>,
) -> Result<Vec<fluxdb_core::SqlParameter>, SqlExecError> {
    use fluxdb_core::SqlParameter;
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value.as_array().ok_or_else(|| {
        SqlExecError::new("invalid_parameters", "parameters 必须是 tagged 值数组")
    })?;
    if items.len() > SQL_PARAM_MAX_ITEMS {
        return Err(SqlExecError::new(
            "invalid_parameters",
            format!("参数个数超过上限 {SQL_PARAM_MAX_ITEMS}"),
        ));
    }
    let mut parameters = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let tag = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SqlExecError::new("invalid_parameters", format!("parameters[{index}] 缺少 type"))
            })?;
        let text_value = || -> Result<&str, SqlExecError> {
            item.get("value")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    SqlExecError::new(
                        "invalid_parameters",
                        format!("parameters[{index}] 缺少字符串 value"),
                    )
                })
        };
        let parameter = match tag {
            "null" => SqlParameter::Null,
            "bool" => SqlParameter::Bool(
                item.get("value")
                    .and_then(serde_json::Value::as_bool)
                    .ok_or_else(|| {
                        SqlExecError::new(
                            "invalid_parameters",
                            format!("parameters[{index}] 不是布尔值"),
                        )
                    })?,
            ),
            "text" => {
                let value = text_value()?;
                if value.len() > SQL_PARAM_MAX_TEXT_BYTES {
                    return Err(SqlExecError::new(
                        "invalid_parameters",
                        format!("parameters[{index}] 文本超过 {SQL_PARAM_MAX_TEXT_BYTES} 字节"),
                    ));
                }
                SqlParameter::Text(value.to_string())
            }
            "i64" => {
                let value = text_value()?;
                value.parse::<i64>().map_err(|_| {
                    SqlExecError::new(
                        "invalid_parameters",
                        format!("parameters[{index}] 不是合法 i64 文本"),
                    )
                })?;
                SqlParameter::I64(value.to_string())
            }
            "u64" => {
                let value = text_value()?;
                value.parse::<u64>().map_err(|_| {
                    SqlExecError::new(
                        "invalid_parameters",
                        format!("parameters[{index}] 不是合法 u64 文本"),
                    )
                })?;
                SqlParameter::U64(value.to_string())
            }
            "f64" => {
                let value = item
                    .get("value")
                    .and_then(serde_json::Value::as_f64)
                    .ok_or_else(|| {
                        SqlExecError::new(
                            "invalid_parameters",
                            format!("parameters[{index}] 不是数值"),
                        )
                    })?;
                if !value.is_finite() {
                    return Err(SqlExecError::new(
                        "invalid_parameters",
                        format!("parameters[{index}] 必须是有限数值"),
                    ));
                }
                SqlParameter::F64(value)
            }
            "decimal" => {
                let value = text_value()?;
                if value.len() > SQL_PARAM_MAX_DECIMAL_CHARS
                    || !looks_like_decimal(value)
                {
                    return Err(SqlExecError::new(
                        "invalid_parameters",
                        format!("parameters[{index}] 不是合法十进制文本"),
                    ));
                }
                SqlParameter::Decimal(value.to_string())
            }
            "bytes" => {
                let value = text_value()?;
                use base64::Engine as _;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .map_err(|_| {
                        SqlExecError::new(
                            "invalid_parameters",
                            format!("parameters[{index}] 不是合法 base64"),
                        )
                    })?;
                if decoded.len() > SQL_PARAM_MAX_BYTES {
                    return Err(SqlExecError::new(
                        "invalid_parameters",
                        format!("parameters[{index}] 字节数超过 {SQL_PARAM_MAX_BYTES}"),
                    ));
                }
                SqlParameter::Bytes(decoded)
            }
            other => {
                return Err(SqlExecError::new(
                    "invalid_parameters",
                    format!("parameters[{index}] 未知类型标记: {other}"),
                ))
            }
        };
        parameters.push(parameter);
    }
    Ok(parameters)
}

/// 只接受 `[+-]?digits[.digits]`（不做浮点转换，避免丢精度）。
fn looks_like_decimal(value: &str) -> bool {
    let body = value.strip_prefix(['+', '-']).unwrap_or(value);
    let mut parts = body.split('.');
    let (Some(int_part), frac, None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if int_part.is_empty() || !int_part.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    match frac {
        None => true,
        Some(frac) => !frac.is_empty() && frac.bytes().all(|byte| byte.is_ascii_digit()),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 一次受控 SQL 执行的结果（已按字节/行数有界）。
#[derive(Clone, Debug, PartialEq)]
pub struct SqlExecutionOutcome {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: u64,
    pub truncated: bool,
    pub elapsed_ms: u64,
    pub query_session_id: fluxdb_core::QuerySessionId,
    pub source: fluxdb_core::CommandExecutionSource,
    pub original_sql: String,
    pub executed_sql: String,
    /// Connector summary，供运行时/审计确认执行阶段和取消边界。
    pub progress: Vec<serde_json::Value>,
    /// 受影响行数（写语句）；只读为 None。
    pub affected_rows: Option<i64>,
    pub statement_kind: &'static str,
}


fn cell_to_json(cell: &fluxdb_core::CellValue) -> serde_json::Value {
    match cell {
        fluxdb_core::CellValue::Null => serde_json::Value::Null,
        fluxdb_core::CellValue::Bool(v) => serde_json::Value::Bool(*v),
        fluxdb_core::CellValue::I64(v) => serde_json::json!(*v),
        fluxdb_core::CellValue::F64(v) => serde_json::json!(*v),
        fluxdb_core::CellValue::Text(v) => serde_json::Value::String(v.clone()),
        fluxdb_core::CellValue::Json(v) => serde_json::Value::String(v.clone()),
        // 必须标明是预览：AI 若把片段当完整值引用或据此写回就会丢数据。
        fluxdb_core::CellValue::TruncatedText {
            preview,
            total_chars,
        } => serde_json::json!({
            "preview": preview,
            "total_chars": total_chars,
            "truncated": true,
        }),
        // 超大字段：只给预览、大小与不透明引用；**不含**任何文件路径，
        // AI 要用完整值必须走有界只读工具。
        fluxdb_core::CellValue::LargeValue(large) => serde_json::json!({
            "preview": large.preview,
            "byte_length": large.byte_length,
            "preview_truncated": large.preview_truncated,
            "content_ref": large.content_id,
            "truncated": true,
        }),
        fluxdb_core::CellValue::Bytes(bytes) => {
            serde_json::json!({ "bytes": bytes.len() })
        }
        fluxdb_core::CellValue::BinarySummary(summary) => serde_json::json!({
            "type": summary.type_name,
            "is_null": summary.is_null,
            "byte_length": summary.byte_length,
        }),
    }
}

/// 单次 AI 查询结束即释放后端会话。
///
/// AI 读取按设计 §21 是「每次有界独立事务」，不应把连接与未提交状态长期挂在注册表里：
/// 既避免 AI 的临时表/事务残留污染后续调用，也避免 PostgreSQL 显式查询会话撞上硬上限。
/// 用 RAII 保证提前返回（取消、策略拒绝、执行错误）也会关闭。
struct AiSqlSessionGuard {
    connection_id: fluxdb_core::ConnectionId,
    session_id: fluxdb_core::QuerySessionId,
}

impl Drop for AiSqlSessionGuard {
    fn drop(&mut self) {
        fluxdb_connectors::pg_close_query_session(self.connection_id, self.session_id);
    }
}

include!("ai_sql_query_execution.rs");
