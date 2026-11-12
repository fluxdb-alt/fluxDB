// 对外 MCP Server 的共享执行服务（设计 §20.4–20.6、§28.4–28.8、§29.1，T054/T059/T065）。
//
// 内部 Agent 与外部 MCP Client 走**同一条执行服务**——相同的 schema 校验、
// 风险分类、强制审计、授权、连接器与回执；只换 Caller/Owner，不复制第二套实现，
// 也不创建假 Turn。协议层（Streamable HTTP/JSON-RPC）在应用内端点，UI 在桌面端，
// 两者都经由本模块对外部请求执行。
//
// 数据源模型：用户显式把某个连接加入对外服务，并为其选择「只读 / 允许读写 / 默认」。
// 没有表/Key 范围与写规则配置：数据范围等于该连接自身的授权范围。目标句柄为
// `conn:<id>[/db:<database>][/schema:<schema>]`，只有已加入数据源的连接可用。

// 本文件以 include! 引入 crate 根作用域：BTreeSet/Arc/Mutex 及内核类型（AuditOutcome 等）
// 已在 lib.rs 顶部导入，这里不重复 use（重复导入会触发 E0252）。

/// 对外 MCP 工具的稳定映射：MCP 名称 ↔ 内部 stable ToolId。
///
/// 外部用普通下划线名称方便 CLI 精确授权；内部不重复建第二实现。
/// `fluxdb_connections_list`/`fluxdb_operation_status` 由服务内实现（内部 id 为空），
/// 其余映射到既有工具 ID。
///
/// 不提供「取消进行中请求」的工具：客户端断开/协议取消已经真正中断执行，
/// 再暴露一个只能改账面状态、动不了正在跑的操作的入口只会误导调用方。
pub const MCP_TOOL_MAPPINGS: &[(&str, &str)] = &[
    ("fluxdb_connections_list", ""),
    ("fluxdb_search_objects", "db.search_objects"),
    ("fluxdb_describe_object", "db.describe_object"),
    ("fluxdb_run", "db.run"),
    ("fluxdb_get_relationships", "db.get_relationships"),
    ("fluxdb_find_join_paths", "db.find_join_paths"),
    ("fluxdb_redis_scan", "redis.scan"),
    ("fluxdb_redis_inspect", "redis.inspect"),
    ("fluxdb_redis_execute", "redis.execute"),
    ("fluxdb_redis_subscribe", "redis.subscribe"),
    ("fluxdb_redis_server_info", "redis.server_info"),
    ("fluxdb_result_page", "result.read_page"),
    ("fluxdb_operation_status", ""),
];

/// MCP 工具名 → 内部 ToolId；服务内实现的工具返回 `None`。
pub fn mcp_tool_internal_id(mcp_name: &str) -> Option<&'static str> {
    MCP_TOOL_MAPPINGS
        .iter()
        .find(|(name, _)| *name == mcp_name)
        .map(|(_, internal)| *internal)
        .filter(|internal| !internal.is_empty())
}

/// 内部 ToolId → MCP 工具名。
pub fn mcp_tool_name(internal_id: &str) -> Option<&'static str> {
    MCP_TOOL_MAPPINGS
        .iter()
        .find(|(_, internal)| *internal == internal_id)
        .map(|(name, _)| *name)
}

/// MCP 工具名 → 用于全局工具开关比对的内部 id；服务内实现的工具用点号别名。
pub fn mcp_permission_tool_id(name: &str) -> Option<&'static str> {
    mcp_tool_internal_id(name).or(match name {
        "fluxdb_connections_list" => Some("connections.list"),
        "fluxdb_operation_status" => Some("operation.status"),
        _ => None,
    })
}

/// 资源类型（决定暴露哪些工具，不做跨类型放行）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpResourceKind {
    Relational,
    Redis,
}

impl McpResourceKind {
    fn from_db_kind(kind: fluxdb_core::DatabaseKind) -> Self {
        match kind {
            fluxdb_core::DatabaseKind::Redis => McpResourceKind::Redis,
            _ => McpResourceKind::Relational,
        }
    }
}

/// 由连接配置推导资源类型（无配置时按关系型处理）。
fn resource_kind_for_config(config: Option<&fluxdb_core::ConnectionConfig>) -> McpResourceKind {
    config
        .map(|c| McpResourceKind::from_db_kind(c.kind))
        .unwrap_or(McpResourceKind::Relational)
}

/// 一个已加入对外服务的数据源：连接 + 有效写入策略。
///
/// 句柄不携带 endpoint/凭据；`read_only` 是「全局策略 + 数据源覆盖 + 连接只读标记」
/// 的最终结论，调用时据此拒绝写工具。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpDatasourceHandle {
    pub connection_id: fluxdb_core::ConnectionId,
    /// 对外发布的句柄 `conn:<id>`（不含库/schema；由调用参数补充且仍受连接约束）。
    pub target: String,
    pub label: String,
    pub kind: McpResourceKind,
    /// 有效只读：为 true 时写工具不列出也拒绝。
    pub read_only: bool,
    /// 库名提示：来自用户「选择展示数据库」设置，回落连接自带默认库；空表示无从得知。
    ///
    /// 只是静态提示，不探活、不代表授权新增范围；模型仍须在 target 里显式写 `/db:<database>`。
    pub databases: Vec<String>,
}

/// 桌面端「显示数据库」保存的 `options` 键（换行分隔的库名）。
const VISIBLE_DATABASES_OPTION: &str = "visible_databases";

/// 数据源的静态库清单：用户配置的可见库优先，其次连接默认库；按出现顺序去重。
fn configured_databases(config: &fluxdb_core::ConnectionConfig) -> Vec<String> {
    let visible = config
        .options
        .get(VISIBLE_DATABASES_OPTION)
        .map(|value| value.lines().map(str::trim))
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty());
    let mut databases: Vec<String> = Vec::new();
    for name in visible.chain(default_database(config)) {
        if !databases.iter().any(|existing| existing == name) {
            databases.push(name.to_string());
        }
    }
    databases
}

/// 由连接配置与数据源写入策略装配句柄。
pub fn datasource_handle(
    connection_id: u64,
    label: String,
    config: Option<&fluxdb_core::ConnectionConfig>,
    allows_write: bool,
) -> McpDatasourceHandle {
    let kind = resource_kind_for_config(config);
    let connection_read_only = config.is_some_and(connection_is_read_only);
    McpDatasourceHandle {
        connection_id: fluxdb_core::ConnectionId(connection_id),
        target: format!("conn:{connection_id}"),
        label,
        kind,
        read_only: !allows_write || connection_read_only,
        databases: config.map(configured_databases).unwrap_or_default(),
    }
}

/// 已授权数据源集合：按连接 id 精确解析模型提交的 target。
///
/// 模型提交的 target 必须指向某个已加入数据源的连接；未命中返回 `STALE_RESOURCE`，
/// 绝不静默换绑。库/schema 后缀由调用参数补充，仍受该连接权限约束。
pub struct McpDatasourceResolver {
    handles: Vec<McpDatasourceHandle>,
}

impl McpDatasourceResolver {
    pub fn new(handles: Vec<McpDatasourceHandle>) -> Self {
        Self { handles }
    }

    pub fn handles(&self) -> &[McpDatasourceHandle] {
        &self.handles
    }

    /// 解析模型提交的 target：连接必须是已加入的数据源。
    pub fn resolve(&self, target: &str) -> std::result::Result<&McpDatasourceHandle, McpCallError> {
        let parsed = AiSqlTarget::parse(target).map_err(|error| {
            McpCallError::new("invalid_target", error.message, false)
        })?;
        self.handles
            .iter()
            .find(|handle| handle.connection_id == parsed.connection_id)
            .ok_or_else(|| {
                McpCallError::new(
                    "STALE_RESOURCE",
                    format!(
                        "未授权或已失效的数据源句柄: {target}；请先调用 fluxdb_connections_list 获取有效 target"
                    ),
                    false,
                )
            })
    }
}

/// 受控调用错误（映射到 MCP `isError=true` 的 tool result）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpCallError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl McpCallError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
}

/// 一次调用产生的强制审计事实（供宿主落 `ai_tool_audits`，绑定 mcp_request_id）。
///
/// 服务本身不持有存储连接，因此把审计事实作为结果的一部分返回；宿主据此持久化。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpAuditFact {
    pub tool_id: String,
    pub tool_version: u32,
    pub plan_hash: String,
    /// `allow` / `deny` / `require_approval`。
    pub decision: &'static str,
    pub risk: String,
    /// 命中的规则 ID（机器键）；内核规则集没命中时为空。
    pub rule_ids: Vec<String>,
    /// 命中的原因码；只放规则标签与对象名，不含参数值。
    pub reason_codes: Vec<String>,
    /// 判定时使用的规则集版本。
    pub ruleset_version: String,
}

/// 一次外部调用的结果投影。
#[derive(Clone, Debug)]
pub struct McpCallOutcome {
    pub ok: bool,
    /// 结构化结果（object 顶层）；失败时为错误结构。
    pub structured: serde_json::Value,
    /// 可读 text JSON 投影（客户端不支持结构字段时的等价投影）。
    pub text: String,
    pub error: Option<McpCallError>,
    /// 本次调用的强制审计事实；服务内工具（无内核审计）为 `None`。
    pub audit: Option<McpAuditFact>,
}

impl McpCallOutcome {
    fn success(structured: serde_json::Value) -> Self {
        let text = serde_json::to_string(&structured).unwrap_or_else(|_| "{}".into());
        Self {
            ok: true,
            structured,
            text,
            error: None,
            audit: None,
        }
    }

    /// 公开的失败构造：供协议层在工具线程异常时给出受控错误。
    pub fn failure_of(error: McpCallError) -> Self {
        Self::failure(error)
    }

    /// 附加强制审计事实（宿主持久化用）。
    fn with_audit(mut self, audit: McpAuditFact) -> Self {
        self.audit = Some(audit);
        self
    }

    fn failure(error: McpCallError) -> Self {
        let structured = serde_json::json!({
            "ok": false,
            "code": error.code,
            "message": error.message,
            "retryable": error.retryable,
        });
        let text = serde_json::to_string(&structured).unwrap_or_else(|_| "{}".into());
        Self {
            ok: false,
            structured,
            text,
            error: Some(error),
            audit: None,
        }
    }
}

/// 宿主提供的 request 仓储：实现 `fluxdb_operation_status`。
///
/// 执行服务本身不持有存储连接（保持可测试、无 IO），由宿主（应用内 HTTP 端点）注入。
/// 未注入时该工具返回 `operation_unsupported`，不静默假装成功。
pub trait McpRequestHost: Send + Sync {
    fn request_status(
        &self,
        identity: &McpCallIdentity,
        request_id: &str,
    ) -> std::result::Result<serde_json::Value, McpCallError>;
}

/// 对外可见的一个工具条目（`tools/list` 投影）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpToolView {
    /// 对外 MCP 名称。
    pub name: String,
    /// 内部工具版本（供审计与状态查询）。
    pub internal_id: Option<String>,
    pub version: u32,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub risk: fluxdb_core::ToolRisk,
}

/// 由数据源集合装配对外工具集。
///
/// 没有对应的数据源类型就不出现该类工具；写工具只在至少一个数据源允许写时列出
/// （可见性不是授权：调用时仍按目标连接的有效策略再判定一次）。全局停用的工具不列出。
pub fn visible_tools(
    handles: &[McpDatasourceHandle],
    globally_disabled: &BTreeSet<String>,
    relationships_available: bool,
) -> fluxdb_core::Result<Vec<McpToolView>> {
    let has_relational = handles
        .iter()
        .any(|h| h.kind == McpResourceKind::Relational);
    let has_redis = handles.iter().any(|h| h.kind == McpResourceKind::Redis);
    let relational_writable = handles
        .iter()
        .any(|h| h.kind == McpResourceKind::Relational && !h.read_only);
    let redis_writable = handles
        .iter()
        .any(|h| h.kind == McpResourceKind::Redis && !h.read_only);

    let mut views = Vec::new();
    if has_relational {
        for spec in standard_sql_tool_specs()? {
            // `db.run` 在目录里按最坏情况标 Write，但真实风险由 SQL 分类；
            // 只读数据源也要看到它，实际写语句在调用边界被拒绝。
            if spec.id != "db.run"
                && spec.risk == fluxdb_core::ToolRisk::Write
                && !relational_writable
            {
                continue;
            }
            if let Some(view) = tool_view_from_spec(&spec)? {
                views.push(view);
            }
        }
        // 关系/路径只读工具：与内部会话共用实现与 schema。`er.propose_relationships`
        // 不在此列——本地 ER 审阅流程不属于对外数据源的授权范围，MCP 目录里也没有它的实现。
        // 未接入关系目录时不列出：列出却调不动，会被当成「没有关系」而不是「工具不可用」。
        if relationships_available {
            for spec in crate::er_relationship_tool_specs()? {
                if let Some(view) = tool_view_from_spec(&spec)? {
                    views.push(view);
                }
            }
        }
    }
    if has_redis {
        for spec in standard_redis_tool_specs()? {
            if spec.risk == fluxdb_core::ToolRisk::Write && !redis_writable {
                continue;
            }
            if let Some(view) = tool_view_from_spec(&spec)? {
                views.push(view);
            }
        }
    }
    // 公共工具：connections.list / result.read_page（仅关系型）/ operation.*。
    views.push(connections_list_view()?);
    if has_relational {
        if let Some(view) =
            tool_view_from_internal("result.read_page", &crate::result_read_page_spec()?)?
        {
            views.push(view);
        }
    }
    views.push(operation_status_view()?);

    // 全局工具开关收窄（与内部工具一致，MCP 不能重新打开被全局关掉的工具）。
    views.retain(|view| {
        mcp_permission_tool_id(&view.name).is_none_or(|id| !globally_disabled.contains(id))
    });
    Ok(views)
}

fn tool_view_from_spec(spec: &crate::ToolSpec) -> fluxdb_core::Result<Option<McpToolView>> {
    let Some(name) = mcp_tool_name(&spec.id) else {
        // 内部工具（artifact.* 等）不对外暴露。
        return Ok(None);
    };
    Ok(Some(McpToolView {
        name: name.to_string(),
        internal_id: Some(spec.id.clone()),
        version: spec.version,
        description: spec.description.clone(),
        input_schema: spec.input_schema.clone(),
        risk: spec.risk,
    }))
}

fn tool_view_from_internal(
    internal_id: &str,
    spec: &crate::ToolSpec,
) -> fluxdb_core::Result<Option<McpToolView>> {
    let Some(name) = mcp_tool_name(internal_id) else {
        return Ok(None);
    };
    Ok(Some(McpToolView {
        name: name.to_string(),
        internal_id: Some(internal_id.to_string()),
        version: spec.version,
        description: spec.description.clone(),
        input_schema: spec.input_schema.clone(),
        risk: spec.risk,
    }))
}

/// `fluxdb_connections_list` 的输入 schema：无参数。
pub fn connections_list_view() -> fluxdb_core::Result<McpToolView> {
    Ok(McpToolView {
        name: "fluxdb_connections_list".into(),
        internal_id: None,
        version: 1,
        description: "列出已加入对外服务的数据源、读写策略、库名提示（databases）与 target 前缀；\
后续检索/执行请把 target 写成 conn:<id>[/db:<database>][/schema:<schema>]（不返回 endpoint 或凭据）"
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        }),
        risk: fluxdb_core::ToolRisk::Read,
    })
}

/// `fluxdb_operation_status` 的输入 schema。
pub fn operation_status_view() -> fluxdb_core::Result<McpToolView> {
    Ok(McpToolView {
        name: "fluxdb_operation_status".into(),
        internal_id: None,
        version: 1,
        description: "查询本 caller 有权查看的 request 状态与已知结果引用；进程重启后被中断的写操作会标记为 unknown（结果待核实），此时不要重放同一写入".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["request_id"],
            "properties": {
                "request_id": { "type": "string", "minLength": 1, "maxLength": 128 }
            }
        }),
        risk: fluxdb_core::ToolRisk::Read,
    })
}

/// 共享执行服务的输入：数据源句柄、Profile 运行视图与结果仓库。
pub struct McpExecutionService {
    security: Option<ExecutionSecurity>,
    resolver: Arc<MapSqlResolver>,
    /// 已加入对外服务的数据源（按连接精确解析）。
    pub datasources: McpDatasourceResolver,
    pub profile: fluxdb_storage::ai_platform::mcp_server::ServerProfileView,
    /// 本 session 的结果仓库：ResultRef 只在本 session 内可读（跨 session 取不到）。
    pub results: Arc<Mutex<ResultStore>>,
    audit: Arc<dyn crate::SecurityAuditHook>,
    host: Option<Arc<dyn McpRequestHost>>,
    /// 全局停用的工具 id（调用时与可见性一致地再判定一次）。
    globally_disabled: BTreeSet<String>,
    /// ER 关系能力来源；未注入时关系/路径工具不装配，调用会落到「不可用」而不是空结果。
    relationships: Option<Arc<dyn ErRelationshipSource>>,
}

impl McpExecutionService {
    /// 装配服务：只需非敏感连接配置与数据源；凭据解密发生在连接器层。
    ///
    /// `datasources` 的 `write_policy` 已结合全局默认解析为「是否允许写」，
    /// 句柄的 `read_only` 再叠加连接自身的只读标记。
    pub fn new(
        connections: Vec<fluxdb_core::ConnectionConfig>,
        profile: fluxdb_storage::ai_platform::mcp_server::ServerProfileView,
        datasources: Vec<fluxdb_storage::ai_platform::mcp_server::DatasourceView>,
        global: fluxdb_core::McpWritePolicy,
        globally_disabled: BTreeSet<String>,
        results: Arc<Mutex<ResultStore>>,
    ) -> fluxdb_core::Result<Self> {
        let resolver = Arc::new(MapSqlResolver::new(connections.iter().cloned()));
        let handles = datasources
            .iter()
            .map(|datasource| {
                let config = connections
                    .iter()
                    .find(|c| c.id.0 == datasource.connection_id as u64);
                let label = config
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| format!("连接 #{}", datasource.connection_id));
                datasource_handle(
                    datasource.connection_id as u64,
                    label,
                    config,
                    datasource.write_policy.allows_write(global),
                )
            })
            .collect();
        Ok(Self {
            security: None,
            resolver,
            datasources: McpDatasourceResolver::new(handles),
            profile,
            results,
            audit: Arc::new(KernelAuditHook),
            host: None,
            globally_disabled,
            relationships: None,
        })
    }

    /// 注入 ER 关系能力来源（独立 MCP 进程自建缓存目录）。
    pub fn with_relationship_source(
        mut self,
        relationships: Arc<dyn ErRelationshipSource>,
    ) -> Self {
        self.relationships = Some(relationships);
        self
    }

    /// 注入宿主 request 仓储（`operation.status` 用）。
    pub fn with_request_host(mut self, host: Arc<dyn McpRequestHost>) -> Self {
        self.host = Some(host);
        self
    }

    /// 注入执行安全上下文（连接变更复核与强制审计超时/取消复核）。
    ///
    /// 同时把审计 Hook 换成带规则集的实现：MCP 与内部会话共用同一份安全审计插件配置。
    /// 规则读不出来时换成「审计不可用」Hook，让调用按 fail closed 被拒绝，
    /// 而不是退回恒放行的 [`KernelAuditHook`]。
    pub fn with_execution_security(mut self, storage: fluxdb_storage::FileStorage) -> Self {
        self.audit = match crate::load_audit_rules(&storage) {
            Ok(rules) => Arc::new(crate::ConfiguredAuditHook::new(rules)),
            Err(error) => Arc::new(crate::UnavailableAuditHook::new(error.message)),
        };
        self.security = Some(ExecutionSecurity::new(storage));
        self
    }

    /// 列出已加入的数据源（`fluxdb_connections_list` 的服务实现）。
    pub fn list_resources(&self) -> serde_json::Value {
        let resources = self
            .datasources
            .handles()
            .iter()
            .map(|handle| {
                serde_json::json!({
                    "target": handle.target,
                    "label": handle.label,
                    "kind": match handle.kind {
                        McpResourceKind::Relational => "relational",
                        McpResourceKind::Redis => "redis",
                    },
                    "write_policy": if handle.read_only { "read_only" } else { "read_write" },
                    // 静态库提示；真正可检索的库仍以连接器为准。
                    "databases": &handle.databases,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({ "ok": true, "resources": resources })
    }
}

/// 一次调用的身份：session/request（用于审计归属与结果绑定）。
#[derive(Clone, Debug)]
pub struct McpCallIdentity {
    pub session_id: String,
    pub request_id: String,
}

impl McpExecutionService {
    /// 该调用所属 request 的终态（供宿主写入 `ai_mcp_server_requests`）。
    pub fn terminal_status(
        outcome: &McpCallOutcome,
    ) -> fluxdb_storage::ai_platform::mcp_server_runtime::ServerRequestStatus {
        if outcome.ok {
            fluxdb_storage::ai_platform::mcp_server_runtime::ServerRequestStatus::Succeeded
        } else {
            match outcome.error.as_ref().map(|error| error.code.as_str()) {
                Some("write_disabled") | Some("tool_not_allowed") | Some("unknown_tool") => {
                    fluxdb_storage::ai_platform::mcp_server_runtime::ServerRequestStatus::Denied
                }
                Some("cancelled") => {
                    fluxdb_storage::ai_platform::mcp_server_runtime::ServerRequestStatus::Cancelled
                }
                Some("write_outcome_unknown") | Some("cancel_race_unknown") => {
                    fluxdb_storage::ai_platform::mcp_server_runtime::ServerRequestStatus::Unknown
                }
                _ => fluxdb_storage::ai_platform::mcp_server_runtime::ServerRequestStatus::Failed,
            }
        }
    }

    fn prepare_no_args(&self, arguments_json: &str) -> std::result::Result<(), McpCallError> {
        let value: serde_json::Value = serde_json::from_str(arguments_json).map_err(|_| {
            McpCallError::new("invalid_arguments", "工具入参不是合法 JSON", true)
        })?;
        if !value.is_object() {
            return Err(McpCallError::new(
                "invalid_arguments",
                "工具入参必须是对象",
                true,
            ));
        }
        Ok(())
    }

    /// 目标连接的内核权限：读恒可（真正范围由 resolver/连接层校验），写取决于有效策略与连接只读。
    fn kernel_policy(&self, target_json: &str) -> KernelPolicy {
        let handle = self
            .datasources
            .resolve(target_json)
            .ok();
        let writable = handle.is_some_and(|handle| {
            !handle.read_only
                && self
                    .resolver
                    .connection_config(handle.connection_id)
                    .is_some_and(|config| !connection_is_read_only(&config))
        });
        KernelPolicy {
            read_allowed: true,
            write_allowed: writable,
            admin_allowed: false,
        }
    }

    /// `operation.status`：需要宿主 request 仓储。
    fn handle_operation(
        &self,
        identity: &McpCallIdentity,
        arguments_json: &str,
    ) -> McpCallOutcome {
        let value: serde_json::Value = match serde_json::from_str(arguments_json) {
            Ok(value) => value,
            Err(_) => {
                return McpCallOutcome::failure(McpCallError::new(
                    "invalid_arguments",
                    "工具入参不是合法 JSON",
                    true,
                ));
            }
        };
        let Some(request_id) = value.get("request_id").and_then(|v| v.as_str()) else {
            return McpCallOutcome::failure(McpCallError::new(
                "invalid_arguments",
                "缺少 request_id",
                true,
            ));
        };
        let Some(host) = self.host.as_ref() else {
            return McpCallOutcome::failure(McpCallError::new(
                "operation_unsupported",
                "宿主未提供 request 仓储，操作状态不可用",
                false,
            ));
        };
        let result = host.request_status(identity, request_id);
        match result {
            Ok(structured) => McpCallOutcome::success(structured),
            Err(error) => McpCallOutcome::failure(error),
        }
    }

    fn build_catalog(
        &self,
        internal_id: &str,
    ) -> std::result::Result<ToolCatalog, McpCallError> {
        // 复用标准目录装配（读写工具都注册，授权由下方 kernel policy 逐次判定），
        // 再收窄到本次调用的单个工具，避免把未授权工具的实现带进来。
        let mut full = if internal_id.starts_with("redis.") {
            crate::standard_redis_catalog(Arc::clone(&self.resolver), true)
                .map_err(|error| McpCallError::new("catalog_error", error.message, false))?
        } else {
            crate::standard_sql_catalog_with_results(
                Arc::clone(&self.resolver),
                Arc::clone(&self.results),
                true,
            )
            .map_err(|error| McpCallError::new("catalog_error", error.message, false))?
        };
        // 关系/路径只读工具与内部会话共用实现；未注入来源时不装配，调用会明确落到
        // 「工具不可用」，不会返回一份空关系列表冒充「没有关系」。
        if matches!(internal_id, "db.get_relationships" | "db.find_join_paths") {
            let relationships = self.relationships.as_ref().cloned().ok_or_else(|| {
                McpCallError::new(
                    "tool_unavailable",
                    "ER 关系目录未接入，关系工具不可用",
                    false,
                )
            })?;
            crate::add_er_relationship_read_tools(
                &mut full,
                Arc::clone(&self.resolver),
                relationships,
            )
            .map_err(|error| McpCallError::new("catalog_error", error.message, false))?;
        }
        let disabled: BTreeSet<String> = full
            .specs()
            .iter()
            .map(|spec| spec.id.clone())
            .filter(|id| id != internal_id)
            .collect();
        let mut catalog = full;
        catalog.without_tools(&disabled);
        Ok(catalog)
    }

    /// 当前可见工具（`tools/list` 与 `tools/call` 前置检查共用）。
    pub fn tool_views(&self) -> fluxdb_core::Result<Vec<McpToolView>> {
        visible_tools(
            self.datasources.handles(),
            &self.globally_disabled,
            self.relationships.is_some(),
        )
    }
}

/// 从已校验参数取 `target` 字符串。
fn target_of(prepared: &crate::ValidatedArguments) -> Option<&str> {
    prepared.as_value().get("target").and_then(|v| v.as_str())
}

fn tool_error_to_call(error: ToolError) -> McpCallError {
    McpCallError::new(error.code, error.message, error.retryable)
}

#[cfg(test)]
mod configured_databases_tests {
    use super::*;

    fn connection(options: &[(&str, &str)], database: Option<&str>) -> fluxdb_core::ConnectionConfig {
        serde_json::from_value(serde_json::json!({
            "id": 1,
            "name": "c",
            "kind": "MySql",
            "endpoint": { "Tcp": { "host": "h", "port": 3306, "database": database } },
            "options": options.iter().copied().collect::<BTreeMap<_, _>>(),
        }))
        .expect("测试连接配置")
    }

    #[test]
    fn visible_databases_then_default_without_duplicates() {
        let config = connection(
            &[("visible_databases", "shop\n\n analytics \nshop")],
            Some("analytics"),
        );
        assert_eq!(
            configured_databases(&config),
            vec!["shop".to_string(), "analytics".to_string()]
        );
        assert_eq!(
            configured_databases(&connection(&[], Some("main"))),
            vec!["main".to_string()]
        );
        assert!(configured_databases(&connection(&[], None)).is_empty());
    }
}

include!("ai_mcp_call.rs");
