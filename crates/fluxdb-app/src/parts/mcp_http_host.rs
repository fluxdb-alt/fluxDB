//! 应用内 MCP HTTP 端点（Streamable HTTP）。
//!
//! 设计要点：
//! - 端点跑在应用进程内，直接使用应用运行时的连接与凭据，不再有独立执行进程；
//! - 仅监听 `127.0.0.1`，每个请求强制校验 `Authorization: Bearer <token>`；
//! - token 是**稳定凭据**（首次生成后写入 `mcp-token`，0600），客户端配置一次即可；
//! - 端点生命周期跟随 MCP Profile 的启用开关，关闭时通过取消令牌停止全部会话；
//! - 工具执行走 [`McpExecutionService`]，与内部 AI 会话共用同一套授权、审计与写入策略。

use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fluxdb_storage::Storage as _;
use fluxdb_storage::ai_platform::mcp_server_runtime::{
    ServerRequestStatus, finish_server_request, load_server_request,
    recover_server_requests_on_start, start_server_request, start_server_session,
    touch_server_session, try_load_server_request,
};
use fluxdb_storage::{FileStorage, ai_platform};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{ErrorData, RoleServer};
use tokio_util::sync::CancellationToken;
use tower_service::Service as _;

use crate::{
    McpAuditFact, McpCallError, McpCallIdentity, McpCallOutcome, McpExecutionService,
    McpRequestHost,
};

/// 默认本机端点端口。端口被占用时启动会明确失败，由设置页提示用户修改。
pub const DEFAULT_MCP_HTTP_PORT: u16 = 47821;

/// 稳定的 MCP 访问令牌文件名（位于存储根目录，权限 0600）。
const MCP_TOKEN_FILE: &str = "mcp-token";

/// 端点对外信息：客户端配置只需要这两个值。
#[derive(Clone, PartialEq, Eq)]
pub struct McpHttpEndpoint {
    pub url: String,
    pub token: String,
}

impl std::fmt::Debug for McpHttpEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpHttpEndpoint")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// 运行中的端点句柄：`stop` 后不再接受新会话，已有会话被取消。
pub struct McpHttpHandle {
    endpoint: McpHttpEndpoint,
    shutdown: CancellationToken,
}

impl McpHttpHandle {
    pub fn endpoint(&self) -> &McpHttpEndpoint {
        &self.endpoint
    }

    /// 停止端点（幂等）。
    pub fn stop(&self) {
        self.shutdown.cancel();
    }
}

/// 读取或首次生成稳定令牌。
///
/// 令牌写在存储根目录的 `mcp-token`（0600）。用户可以在设置页重新生成：
/// 重新生成会让已配置的客户端立即失效，属于显式的破坏性操作。
pub fn load_or_create_token(root: &Path) -> Result<String, String> {
    let path = root.join(MCP_TOKEN_FILE);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let token = existing.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let token = random_hex(32)?;
    std::fs::write(&path, &token).map_err(|error| format!("写入 MCP 令牌失败: {error}"))?;
    fluxdb_storage::harden_local_file_perms(&path)
        .map_err(|error| format!("收紧 MCP 令牌权限失败: {}", error.message))?;
    Ok(token)
}

/// 生成新的令牌并覆盖 `mcp-token`，返回新值。
pub fn reset_token(root: &Path) -> Result<String, String> {
    let token = random_hex(32)?;
    let path = root.join(MCP_TOKEN_FILE);
    std::fs::write(&path, &token).map_err(|error| format!("写入 MCP 令牌失败: {error}"))?;
    fluxdb_storage::harden_local_file_perms(&path)
        .map_err(|error| format!("收紧 MCP 令牌权限失败: {}", error.message))?;
    Ok(token)
}

fn random_hex(bytes: usize) -> Result<String, String> {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut buffer = vec![0u8; bytes];
    SystemRandom::new()
        .fill(&mut buffer)
        .map_err(|_| "生成 MCP 令牌失败：随机源不可用".to_string())?;
    Ok(buffer.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// 启动端点：先同步绑定端口（失败立即可见），再在独立线程的运行时装服务。
///
/// 必须由应用主线程外的线程持有 tokio runtime：工具执行内部会再 `block_on`，
/// 与 GPUI 事件循环不能共用。
pub fn start(
    storage: FileStorage,
    profile_id: impl Into<String>,
    port: u16,
) -> Result<McpHttpHandle, String> {
    let profile_id = profile_id.into();
    let token = load_or_create_token(storage.root_dir())?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .map_err(|error| format!("MCP 端口 {port} 不可用: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("MCP 端口设置失败: {error}"))?;

    // 端口 0 由系统分配（测试/多实例场景），对外 URL 必须使用实际端口。
    let port = listener
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or(port);
    let shutdown = CancellationToken::new();
    let thread_shutdown = shutdown.clone();
    let thread_token = token.clone();
    std::thread::Builder::new()
        .name("fluxdb-mcp-http".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            else {
                tracing::error!("无法创建 MCP HTTP 运行时，端点未启动");
                return;
            };
            runtime.block_on(async move {
                if let Err(error) =
                    serve(listener, storage, profile_id, thread_token, thread_shutdown).await
                {
                    tracing::error!(%error, "MCP HTTP 端点已停止");
                }
            });
        })
        .map_err(|error| format!("启动 MCP HTTP 线程失败: {error}"))?;

    Ok(McpHttpHandle {
        endpoint: McpHttpEndpoint {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token,
        },
        shutdown,
    })
}

async fn serve(
    listener: std::net::TcpListener,
    storage: FileStorage,
    profile_id: String,
    token: String,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|error| format!("MCP 监听器初始化失败: {error}"))?;

    // 上次进程中断在 SQL 已提交、终态未落库的窗口：先收敛残留 request。
    storage
        .with_ai_connection(|conn| recover_server_requests_on_start(conn, now_ms()))
        .map_err(|error| error.message)?;

    let target = Arc::new(ServeTarget::load(storage, profile_id)?);
    let service = StreamableHttpService::new(
        {
            let target = target.clone();
            move || FluxDbMcpServer::new(target.clone()).map_err(std::io::Error::other)
        },
        Arc::new(LocalSessionManager::default()),
        {
            // 配置是 non_exhaustive：先取默认值，再覆盖本端点的策略。
            let mut config = StreamableHttpServerConfig::default();
            config.stateful_mode = true;
            config.sse_keep_alive = Some(Duration::from_secs(30));
            // 只接受回环 Host，避免 DNS rebinding 把本机端点暴露给网页。
            config.allowed_hosts = vec!["127.0.0.1".to_string(), "localhost".to_string()];
            config.cancellation_token = shutdown.clone();
            config
        },
    );

    loop {
        let accepted = tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let (stream, peer) = match accepted {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, "MCP 接受连接失败");
                continue;
            }
        };
        // 双保险：只服务回环地址（绑定已限制，这里防止未来改绑时漏掉）。
        if !peer.ip().is_loopback() {
            tracing::warn!(%peer, "拒绝非回环 MCP 连接");
            continue;
        }
        let service = service.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let handler = service_fn(move |request: Request<Incoming>| {
                let mut service = service.clone();
                let token = token.clone();
                async move {
                    if !authorized(request.headers(), &token) {
                        return Ok::<_, Infallible>(unauthorized());
                    }
                    service.call(request).await
                }
            });
            if let Err(error) = ConnBuilder::new(TokioExecutor::new())
                .serve_connection(io, handler)
                .await
            {
                tracing::debug!(%error, "MCP 连接结束");
            }
        });
    }
    Ok(())
}

/// 常量时间比较 Bearer 令牌，避免按字节计时泄漏。
fn authorized(headers: &hyper::HeaderMap, token: &str) -> bool {
    let Some(value) = headers.get(hyper::header::AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Some(presented) = value.strip_prefix("Bearer ") else {
        return false;
    };
    // 定长逐字节比较：长度差异直接返回（令牌长度本身不是秘密），内容差异不提前退出。
    let presented = presented.as_bytes();
    let expected = token.as_bytes();
    if presented.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in presented.iter().zip(expected) {
        diff |= left ^ right;
    }
    diff == 0
}

fn unauthorized() -> Response<BoxBody<Bytes, Infallible>> {
    let body = Full::new(Bytes::from_static(b"unauthorized"))
        .map_err(|never| match never {})
        .boxed();
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::UNAUTHORIZED;
    response
}

/// 加载好的运行目标：已装配的执行服务 + 工具视图 + 审计所需的存储句柄。
#[derive(Clone)]
pub struct ServeTarget {
    storage: FileStorage,
    service: Arc<McpExecutionService>,
    profile_id: String,
    session_id: String,
}

impl ServeTarget {
    /// 从存储加载 Profile、授权与连接并装配执行服务。
    ///
    /// Profile 不存在/禁用 → 明确错误；不默认开放所有已有连接。
    pub fn load(storage: FileStorage, profile_id: String) -> Result<Self, String> {
        let (profile, datasources, global_policy, disabled) = storage
            .with_ai_connection(|conn| {
                let profile = ai_platform::mcp_server::load_server_profile(conn, &profile_id)?
                    .ok_or_else(|| {
                        fluxdb_core::Error::new(
                            fluxdb_core::ErrorKind::Internal,
                            format!("Server Profile 不存在: {profile_id}"),
                        )
                    })?;
                let datasources = ai_platform::mcp_server::list_datasources(conn, &profile.id)?;
                let (settings, _) = ai_platform::execution_policy::load(conn)?;
                let disabled = ai_platform::tool_settings::disabled_tool_ids(conn)?;
                Ok((profile, datasources, settings.mcp_default, disabled))
            })
            .map_err(|error| error.message)?;
        if !profile.enabled {
            return Err(format!("Server Profile 已禁用: {}", profile.id));
        }
        let connections = storage.load_connections().map_err(|e| e.message)?;

        // 结果仓库与内部一致：句柄只在本次 session 内可读。
        let session_id = format!("mcp-{}", uuid::Uuid::new_v4());
        let results = Arc::new(Mutex::new(crate::ResultStore::for_turn(
            crate::RESULT_STORE_MEMORY_BYTES,
            &session_id,
            &session_id,
            None,
        )));
        let service = McpExecutionService::new(
            connections,
            profile.clone(),
            datasources,
            global_policy,
            disabled,
            results,
        )
        .map_err(|e| e.message)?;
        let host = Arc::new(StorageRequestHost {
            storage: storage.clone(),
            profile_id: profile.id.clone(),
            session_id: session_id.clone(),
        });
        let service = Arc::new(
            service
                .with_request_host(host)
                .with_relationship_source(Arc::new(crate::ErRelationshipAccess::standalone(
                    storage.clone(),
                )))
                .with_execution_security(storage.clone()),
        );
        Ok(Self {
            storage,
            service,
            profile_id: profile.id,
            session_id,
        })
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    fn tool(&self, name: &str) -> Option<Tool> {
        let views = self.current_service().ok()?.tool_views().ok()?;
        views.iter().find(|view| view.name == name).map(|view| {
            let schema: serde_json::Map<String, serde_json::Value> =
                view.input_schema.as_object().cloned().unwrap_or_else(|| {
                    serde_json::json!({"type":"object"})
                        .as_object()
                        .cloned()
                        .unwrap()
                });
            let mut tool = Tool::new(
                view.name.clone(),
                view.description.clone(),
                Arc::new(schema),
            );
            tool.annotations = Some(tool_annotations(view.risk));
            tool
        })
    }

    /// 每次调用重新读取 Profile 与授权：设置页的改动立即生效，不需要重启端点。
    fn current_service(&self) -> Result<McpExecutionService, String> {
        let (profile, datasources, global_policy, disabled) = self
            .storage
            .with_ai_connection(|conn| {
                let profile = ai_platform::mcp_server::load_server_profile(conn, &self.profile_id)?
                    .filter(|profile| profile.enabled)
                    .ok_or_else(|| {
                        fluxdb_core::Error::new(
                            fluxdb_core::ErrorKind::Permission,
                            "MCP 服务已关闭",
                        )
                    })?;
                let datasources = ai_platform::mcp_server::list_datasources(conn, &profile.id)?;
                let (settings, _) = ai_platform::execution_policy::load(conn)?;
                let disabled = ai_platform::tool_settings::disabled_tool_ids(conn)?;
                Ok((profile, datasources, settings.mcp_default, disabled))
            })
            .map_err(|e| e.message)?;
        McpExecutionService::new(
            self.storage.load_connections().map_err(|e| e.message)?,
            profile,
            datasources,
            global_policy,
            disabled,
            self.service.results.clone(),
        )
        .map(|service| {
            service
                .with_request_host(Arc::new(StorageRequestHost {
                    storage: self.storage.clone(),
                    profile_id: self.profile_id.clone(),
                    session_id: self.session_id.clone(),
                }))
                .with_execution_security(self.storage.clone())
        })
        .map_err(|e| e.message)
    }
}

/// rmcp ServerHandler 实现：把协议请求映射到共享执行服务。
pub struct FluxDbMcpServer {
    target: Arc<ServeTarget>,
    started: Mutex<bool>,
}

impl FluxDbMcpServer {
    pub fn new(target: Arc<ServeTarget>) -> Result<Self, String> {
        Ok(Self {
            target,
            started: Mutex::new(false),
        })
    }

    fn ensure_session(&self, protocol_version: &str, client_info_json: &str) {
        let mut started = self.started.lock().expect("session 状态锁");
        if *started {
            return;
        }
        let now = now_ms();
        if let Err(error) = self.target.storage.with_ai_connection(|conn| {
            start_server_session(
                conn,
                now,
                &self.target.session_id,
                &self.target.profile_id,
                protocol_version,
                client_info_json,
            )
            .map(|_| ())
        }) {
            tracing::warn!(%error, "MCP session 记录失败（不阻断服务）");
        }
        *started = true;
    }

    /// 回传结果，并附带本次调用在账本里的 request_id。
    fn tool_result(&self, request_id: &str, outcome: McpCallOutcome) -> CallToolResult {
        let mut structured = outcome.structured;
        if let Some(object) = structured.as_object_mut() {
            object.insert("request_id".into(), serde_json::json!(request_id));
        }
        let mut result = CallToolResult::success(vec![Content::text(outcome.text.clone())]);
        result.structured_content = Some(structured);
        result.is_error = Some(!outcome.ok);
        result
    }

    fn record_request(
        &self,
        identity: &McpCallIdentity,
        mcp_name: &str,
        now: i64,
    ) -> Result<(), String> {
        let _ = self
            .target
            .storage
            .with_ai_connection(|conn| touch_server_session(conn, now, &self.target.session_id));
        self.target
            .storage
            .with_ai_connection(|conn| {
                start_server_request(
                    conn,
                    now,
                    &identity.request_id,
                    &self.target.session_id,
                    &self.target.profile_id,
                    "\"\"",
                    mcp_name,
                    1,
                    None,
                    "{}",
                    "{}",
                    "pending",
                )
            })
            .map(|_| ())
            .map_err(|_| "MCP 请求审计登记失败，未执行".into())
    }

    fn finish_request(&self, identity: &McpCallIdentity, outcome: &McpCallOutcome, now: i64) {
        let status = McpExecutionService::terminal_status(outcome);
        let receipt = serde_json::json!({
            "ok": outcome.ok,
            "request_id": identity.request_id,
        })
        .to_string();
        let report = self.target.storage.with_ai_connection(|conn| {
            if let Some(fact) = outcome.audit.as_ref() {
                persist_audit_fact(conn, now, &identity.request_id, fact)?;
            }
            let current = load_server_request(conn, &identity.request_id)?;
            finish_server_request(
                conn,
                now,
                &identity.request_id,
                status,
                Some(&receipt),
                current.revision,
            )
            .map(|_| ())
        });
        if let Err(error) = report {
            tracing::warn!(%error, "MCP request 终态记录失败");
        }
    }
}

impl ServerHandler for FluxDbMcpServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("fluxdb-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "fluxDB 本地 MCP Server：仅暴露本 Server Profile 已授权的连接/库/Key 范围。\
                 每个 tools/call 的结果都带 request_id，可用 fluxdb_operation_status 查询该请求的最终状态；\
                 若状态为 unknown（应用曾中断在该请求执行期间），表示结果待核实，\
                 不要重放同一写入，改用只读工具核实。"
                    .to_string(),
            )
    }

    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ServerInfo, ErrorData> {
        let client_info =
            serde_json::to_string(&request.client_info).unwrap_or_else(|_| "{}".into());
        self.ensure_session(request.protocol_version.as_str(), &client_info);
        context.peer.set_peer_info(request);
        Ok(self.get_info())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let views = self
            .target
            .current_service()
            .and_then(|service| service.tool_views().map_err(|e| e.message))
            .unwrap_or_default();
        let tools = views
            .iter()
            .filter_map(|view| self.target.tool(&view.name))
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.target.tool(name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let mcp_name = request.name.to_string();
        let arguments_json = arguments_to_json(request.arguments.as_ref());
        let identity = McpCallIdentity {
            session_id: self.target.session_id.clone(),
            request_id: format!("mcp-req-{}", uuid::Uuid::new_v4()),
        };
        let now = now_ms();
        if let Err(message) = self.record_request(&identity, &mcp_name, now) {
            return Ok(self.tool_result(
                &identity.request_id,
                McpCallOutcome::failure_of(McpCallError::new("audit_unavailable", message, false)),
            ));
        }
        // 工具执行内部会自建 tokio runtime 并 block_on，必须放到阻塞线程池。
        let target = Arc::clone(&self.target);
        let name_for_task = mcp_name.clone();
        let args_for_task = arguments_json;
        let identity_for_task = identity.clone();
        let cancel = fluxdb_core::CancelFlag::new();
        struct CancelOnDrop(fluxdb_core::CancelFlag);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.0.request();
            }
        }
        let _guard = CancelOnDrop(cancel.clone());
        let worker_cancel = cancel.clone();
        let mut execution = tokio::task::spawn_blocking(move || match target.current_service() {
            Ok(service) => service.call(
                &identity_for_task,
                &name_for_task,
                &args_for_task,
                &worker_cancel,
            ),
            Err(message) => {
                McpCallOutcome::failure_of(McpCallError::new("service_unavailable", message, false))
            }
        });
        let result = tokio::select! {
            result = &mut execution => result,
            _ = async {
                loop {
                    if context.ct.is_cancelled() || context.peer.is_transport_closed() { break; }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } => { cancel.request(); execution.await },
        };
        let outcome = result.unwrap_or_else(|_| {
            McpCallOutcome::failure_of(McpCallError::new(
                "execution_panicked",
                "工具执行线程异常结束",
                false,
            ))
        });
        self.finish_request(&identity, &outcome, now_ms());
        Ok(self.tool_result(&identity.request_id, outcome))
    }
}

/// 参数对象转 JSON 字符串（仅用于审计登记，不保存原始敏感值）。
fn arguments_to_json(arguments: Option<&serde_json::Map<String, serde_json::Value>>) -> String {
    match arguments {
        Some(map) => serde_json::Value::Object(map.clone()).to_string(),
        None => "{}".to_string(),
    }
}

/// 把强制审计事实落到 `ai_tool_audits`（MCP owner 分支，append-only）。
fn persist_audit_fact(
    conn: &fluxdb_storage::AiConnection,
    now: i64,
    request_id: &str,
    fact: &McpAuditFact,
) -> fluxdb_core::Result<()> {
    use fluxdb_storage::ai_platform::audits::{
        AuditDecision, AuditStage, ToolAuditEdit, insert_tool_audit,
    };
    let decision = match fact.decision {
        "allow" => AuditDecision::Allow,
        "deny" => AuditDecision::Deny,
        _ => AuditDecision::RequireApproval,
    };
    let edit = ToolAuditEdit {
        id: format!("mcp-audit-{request_id}"),
        tool_call_id: None,
        mcp_request_id: Some(request_id.to_string()),
        stage: AuditStage::PreExecute,
        hook_id: "kernel.security-audit".into(),
        hook_version: "1".into(),
        ruleset_version: fact.ruleset_version.clone(),
        plan_hash: fact.plan_hash.clone(),
        decision,
        risk: fact.risk.clone(),
        rule_ids: fact.rule_ids.clone(),
        reason_codes: fact.reason_codes.clone(),
    };
    insert_tool_audit(conn, now, &edit)
}

/// 宿主 request 仓储：用存储读写 `ai_mcp_server_requests`，只允许本 session 的请求。
struct StorageRequestHost {
    storage: FileStorage,
    profile_id: String,
    session_id: String,
}

impl McpRequestHost for StorageRequestHost {
    fn request_status(
        &self,
        _identity: &McpCallIdentity,
        request_id: &str,
    ) -> Result<serde_json::Value, McpCallError> {
        let view = self
            .storage
            .with_ai_connection(|conn| try_load_server_request(conn, request_id))
            .map_err(|error| McpCallError::new("request_unavailable", error.message, false))?;
        let view = match view {
            Some(view)
                if view.profile_id == self.profile_id && view.session_id == self.session_id =>
            {
                view
            }
            _ => {
                return Err(McpCallError::new(
                    "request_not_found",
                    "请求不存在或不属于当前会话",
                    false,
                ));
            }
        };
        let mut payload = serde_json::json!({
            "ok": true,
            "request_id": view.id,
            "tool_id": view.tool_id,
            "status": view.status.as_str(),
            "outcome": if view.status.is_terminal() { "terminal" } else { "running" },
        });
        if view.status == ServerRequestStatus::Unknown {
            payload["next_step"] = serde_json::json!(
                "结果待核实：该操作可能已在数据库提交。不要重放同一写入；先用只读工具核实实际状态，必要时请用户确认。"
            );
        }
        Ok(payload)
    }
}

fn tool_annotations(risk: fluxdb_core::ToolRisk) -> rmcp::model::ToolAnnotations {
    rmcp::model::ToolAnnotations::new()
        .read_only(risk == fluxdb_core::ToolRisk::Read)
        .destructive(risk == fluxdb_core::ToolRisk::Write)
        .idempotent(false)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "fluxdb-mcp-http-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn seed_enabled_profile(storage: &FileStorage) {
        storage
            .with_ai_connection(|conn| {
                conn.execute(
                    "INSERT INTO ai_mcp_server_profiles
                        (id, display_name, enabled, transport, options_json, revision,
                         created_at_ms, updated_at_ms)
                     VALUES ('fluxdb.local','本地 MCP',1,'http','{}',1,1,1)",
                    [],
                )
                .map(|_| ())
                .map_err(|error| {
                    fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, error.to_string())
                })
            })
            .expect("seed profile");
    }

    fn raw_post(port: u16, token: Option<&str>) -> String {
        use std::io::{Read, Write};
        use std::net::TcpStream;

        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
        let mut request = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(token) = token {
            request.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);

        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect endpoint");
        stream.write_all(request.as_bytes()).expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        response.lines().next().unwrap_or_default().to_string()
    }

    #[test]
    fn endpoint_requires_bearer_token() {
        let root = temp_root("auth");
        let storage = FileStorage::new(&root);
        seed_enabled_profile(&storage);
        let handle = start(storage, "fluxdb.local", 0).expect("start endpoint");
        let port = handle
            .endpoint()
            .url
            .trim_start_matches("http://127.0.0.1:")
            .trim_end_matches("/mcp")
            .parse::<u16>()
            .expect("port");

        let unauthenticated = raw_post(port, None);
        assert!(unauthenticated.contains("401"), "{unauthenticated}");
        let wrong = raw_post(port, Some("deadbeef"));
        assert!(wrong.contains("401"), "{wrong}");
        let authenticated = raw_post(port, Some(&handle.endpoint().token));
        assert!(!authenticated.contains("401"), "{authenticated}");

        handle.stop();
    }

    #[test]
    fn token_is_stable_across_starts() {
        let root = temp_root("token");
        let first = load_or_create_token(&root).expect("create token");
        let second = load_or_create_token(&root).expect("reuse token");
        assert_eq!(first, second);
        let rotated = reset_token(&root).expect("rotate token");
        assert_ne!(rotated, first);
        assert_eq!(load_or_create_token(&root).expect("rotated"), rotated);
    }
}
