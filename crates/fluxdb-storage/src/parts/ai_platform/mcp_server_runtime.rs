//! 对外 MCP Server 的 session 与 request 持久化（设计 §28.4、§28.8、§42.11、T057）。
//!
//! 三条规则：
//! 1. request 记录身份是 `(session_id, seq)`，不是协议 request ID：协议 ID 允许客户端复用，
//!    只能作为关联字段保存，不能当永远唯一的幂等键。
//! 2. 归属靠复合外键：request 的 `(profile_id, session_id)` 必须落在同一 Profile 内，
//!    跨 Profile 的引用会被数据库拒绝。
//! 3. 结果分内部 Turn / 外部 MCP 两支，不建假 Turn（见 [`insert_mcp_result`] 与 audits）。

use rusqlite::{Connection, OptionalExtension, params};

use crate::sqlite::sqlite_error;
use fluxdb_core::{Error, ErrorKind, Result};

/// Server session 状态（与 DDL CHECK 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerSessionStatus {
    Connected,
    Disconnected,
    Expired,
    Failed,
}

impl ServerSessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ServerSessionStatus::Connected => "connected",
            ServerSessionStatus::Disconnected => "disconnected",
            ServerSessionStatus::Expired => "expired",
            ServerSessionStatus::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "connected" => Ok(ServerSessionStatus::Connected),
            "disconnected" => Ok(ServerSessionStatus::Disconnected),
            "expired" => Ok(ServerSessionStatus::Expired),
            "failed" => Ok(ServerSessionStatus::Failed),
            other => Err(Error::new(
                ErrorKind::Internal,
                format!("未知 MCP session 状态: {other}"),
            )),
        }
    }
}

/// Server session 视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSessionView {
    pub id: String,
    pub profile_id: String,
    pub client_info_json: String,
    pub protocol_version: String,
    pub status: ServerSessionStatus,
    pub next_request_seq: i64,
    pub started_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub closed_at_ms: Option<i64>,
}

/// 建立一个 MCP Client 连接记录（应用内 HTTP 端点上的会话）。`client_info_json` 仅用于审计，不是认证/授权凭据。
pub fn start_server_session(
    conn: &Connection,
    now_ms: i64,
    id: &str,
    profile_id: &str,
    protocol_version: &str,
    client_info_json: &str,
) -> Result<ServerSessionView> {
    if id.trim().is_empty() || profile_id.trim().is_empty() {
        return Err(Error::new(
            ErrorKind::Internal,
            "session/profile ID 不能为空",
        ));
    }
    let info: serde_json::Value = serde_json::from_str(client_info_json)
        .map_err(|_| Error::new(ErrorKind::Internal, "clientInfo 不是合法 JSON"))?;
    if !info.is_object() {
        return Err(Error::new(ErrorKind::Internal, "clientInfo 必须是对象"));
    }
    conn.execute(
        "INSERT INTO ai_mcp_server_sessions (id, profile_id, client_info_json, protocol_version,
            status, next_request_seq, started_at_ms, last_seen_at_ms)
         VALUES (?1,?2,?3,?4,'connected',1,?5,?5)",
        params![id, profile_id, client_info_json, protocol_version, now_ms],
    )
    .map_err(sqlite_error)?;
    load_server_session(conn, id)
}

/// 读取 session。
pub fn load_server_session(conn: &Connection, id: &str) -> Result<ServerSessionView> {
    conn.query_row(
        "SELECT id, profile_id, client_info_json, protocol_version, status, next_request_seq,
                started_at_ms, last_seen_at_ms, closed_at_ms
           FROM ai_mcp_server_sessions WHERE id=?1",
        [id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<i64>>(8)?,
            ))
        },
    )
    .optional()
    .map_err(sqlite_error)?
    .ok_or_else(|| Error::new(ErrorKind::Internal, "MCP session 不存在"))
    .and_then(|t| {
        Ok(ServerSessionView {
            id: t.0,
            profile_id: t.1,
            client_info_json: t.2,
            protocol_version: t.3,
            status: ServerSessionStatus::parse(&t.4)?,
            next_request_seq: t.5,
            started_at_ms: t.6,
            last_seen_at_ms: t.7,
            closed_at_ms: t.8,
        })
    })
}

/// 更新 session 的 last_seen（保活）。
pub fn touch_server_session(conn: &Connection, now_ms: i64, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE ai_mcp_server_sessions SET last_seen_at_ms=?2 WHERE id=?1",
        params![id, now_ms],
    )
    .map_err(sqlite_error)?;
    Ok(())
}

/// 关闭 session（断开/过期/失败时）。
pub fn close_server_session(
    conn: &Connection,
    now_ms: i64,
    id: &str,
    status: ServerSessionStatus,
) -> Result<ServerSessionView> {
    if status == ServerSessionStatus::Connected {
        return Err(Error::new(
            ErrorKind::Internal,
            "关闭 session 时状态不能是 connected",
        ));
    }
    conn.execute(
        "UPDATE ai_mcp_server_sessions SET status=?2, closed_at_ms=?3, last_seen_at_ms=?3
          WHERE id=?1",
        params![id, status.as_str(), now_ms],
    )
    .map_err(sqlite_error)?;
    load_server_session(conn, id)
}

/// request 状态（与 DDL CHECK 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerRequestStatus {
    Prepared,
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
    Denied,
    Cancelled,
    Unknown,
}

impl ServerRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ServerRequestStatus::Prepared => "prepared",
            ServerRequestStatus::Running => "running",
            ServerRequestStatus::WaitingApproval => "waiting_approval",
            ServerRequestStatus::Succeeded => "succeeded",
            ServerRequestStatus::Failed => "failed",
            ServerRequestStatus::Denied => "denied",
            ServerRequestStatus::Cancelled => "cancelled",
            ServerRequestStatus::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "prepared" => Ok(ServerRequestStatus::Prepared),
            "running" => Ok(ServerRequestStatus::Running),
            "waiting_approval" => Ok(ServerRequestStatus::WaitingApproval),
            "succeeded" => Ok(ServerRequestStatus::Succeeded),
            "failed" => Ok(ServerRequestStatus::Failed),
            "denied" => Ok(ServerRequestStatus::Denied),
            "cancelled" => Ok(ServerRequestStatus::Cancelled),
            "unknown" => Ok(ServerRequestStatus::Unknown),
            other => Err(Error::new(
                ErrorKind::Internal,
                format!("未知 MCP request 状态: {other}"),
            )),
        }
    }

    /// 终态：不再改变，也不再算作进行中。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ServerRequestStatus::Succeeded
                | ServerRequestStatus::Failed
                | ServerRequestStatus::Denied
                | ServerRequestStatus::Cancelled
                | ServerRequestStatus::Unknown
        )
    }
}

/// request 视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRequestView {
    pub id: String,
    pub profile_id: String,
    pub session_id: String,
    pub seq: i64,
    pub protocol_request_id_json: String,
    pub tool_id: String,
    pub tool_version: i64,
    /// 本次调用绑定的连接（数据源）；服务内工具（如 connections.list）为 `None`。
    pub connection_id: Option<i64>,
    pub target_json: String,
    pub arguments_redacted_json: String,
    pub plan_hash: String,
    pub status: ServerRequestStatus,
    pub receipt_json: Option<String>,
    pub revision: i64,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}

/// 开始一次调用：分配 `seq` 与记录身份（同一事务原子完成）。
///
/// `protocol_request_id_json` 是协议层 ID 的 JSON 形式（可为字符串或数值），只作关联。
pub fn start_server_request(
    conn: &Connection,
    now_ms: i64,
    id: &str,
    session_id: &str,
    profile_id: &str,
    protocol_request_id_json: &str,
    tool_id: &str,
    tool_version: i64,
    connection_id: Option<i64>,
    target_json: &str,
    arguments_redacted_json: &str,
    plan_hash: &str,
) -> Result<ServerRequestView> {
    for (label, value) in [("tool_id", tool_id), ("plan_hash", plan_hash)] {
        if value.trim().is_empty() {
            return Err(Error::new(
                ErrorKind::Internal,
                format!("MCP request {label} 不能为空"),
            ));
        }
    }
    if tool_version < 1 {
        return Err(Error::new(ErrorKind::Internal, "tool_version 必须 >= 1"));
    }
    for (label, raw) in [
        ("protocol_request_id", protocol_request_id_json),
        ("target", target_json),
        ("arguments", arguments_redacted_json),
    ] {
        let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| {
            Error::new(
                ErrorKind::Internal,
                format!("MCP request {label} 不是合法 JSON"),
            )
        })?;
        let _ = value;
    }
    let tx = conn.unchecked_transaction().map_err(sqlite_error)?;
    let seq: i64 = tx
        .query_row(
            "UPDATE ai_mcp_server_sessions
                SET next_request_seq = next_request_seq + 1, last_seen_at_ms=?2
              WHERE id=?1 AND profile_id=?3
             RETURNING next_request_seq - 1",
            params![session_id, now_ms, profile_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_error)?
        .ok_or_else(|| Error::new(ErrorKind::Internal, "MCP session 不存在或不属于该 Profile"))?;
    tx.execute(
        "INSERT INTO ai_mcp_server_requests (id, profile_id, session_id, seq,
            protocol_request_id_json, tool_id, tool_version, connection_id, target_json,
            arguments_redacted_json, plan_hash, status, revision, started_at_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'prepared',1,?12)",
        params![
            id,
            profile_id,
            session_id,
            seq,
            protocol_request_id_json,
            tool_id,
            tool_version,
            connection_id,
            target_json,
            arguments_redacted_json,
            plan_hash,
            now_ms,
        ],
    )
    .map_err(sqlite_error)?;
    tx.commit().map_err(sqlite_error)?;
    load_server_request(conn, id)
}

/// 读取 request。
pub fn load_server_request(conn: &Connection, id: &str) -> Result<ServerRequestView> {
    try_load_server_request(conn, id)?
        .ok_or_else(|| Error::new(ErrorKind::Internal, "MCP request 不存在"))
}

/// 读取 request，缺失返回 `None`。
///
/// 与 [`load_server_request`] 的区别只在于把"不存在"和"读库失败"分开：
/// 状态查询要靠它回答"这条不属于你"，不能把缺失当成存储故障上报。
pub fn try_load_server_request(conn: &Connection, id: &str) -> Result<Option<ServerRequestView>> {
    conn.query_row(
        "SELECT id, profile_id, session_id, seq, protocol_request_id_json, tool_id, tool_version,
                connection_id, target_json, arguments_redacted_json, plan_hash, status,
                receipt_json, revision, started_at_ms, finished_at_ms
           FROM ai_mcp_server_requests WHERE id=?1",
        [id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, i64>(14)?,
                row.get::<_, Option<i64>>(15)?,
            ))
        },
    )
    .optional()
    .map_err(sqlite_error)?
    .map(|t| {
        Ok(ServerRequestView {
            id: t.0,
            profile_id: t.1,
            session_id: t.2,
            seq: t.3,
            protocol_request_id_json: t.4,
            tool_id: t.5,
            tool_version: t.6,
            connection_id: t.7,
            target_json: t.8,
            arguments_redacted_json: t.9,
            plan_hash: t.10,
            status: ServerRequestStatus::parse(&t.11)?,
            receipt_json: t.12,
            revision: t.13,
            started_at_ms: t.14,
            finished_at_ms: t.15,
        })
    })
    .transpose()
}

/// 标记 request 进入运行中（行级 CAS）。
pub fn mark_server_request_running(
    conn: &Connection,
    now_ms: i64,
    id: &str,
    expected_revision: i64,
) -> Result<ServerRequestView> {
    let _ = now_ms;
    let changed = conn
        .execute(
            "UPDATE ai_mcp_server_requests SET status='running', revision=revision+1
              WHERE id=?1 AND status='prepared' AND revision=?2",
            params![id, expected_revision],
        )
        .map_err(sqlite_error)?;
    if changed == 0 {
        return Err(Error::new(
            ErrorKind::Internal,
            "MCP request 状态已变化，无法进入运行中",
        ));
    }
    load_server_request(conn, id)
}

/// 结束 request（终态 + 回执 + 行级 CAS）。
pub fn finish_server_request(
    conn: &Connection,
    now_ms: i64,
    id: &str,
    status: ServerRequestStatus,
    receipt_json: Option<&str>,
    expected_revision: i64,
) -> Result<ServerRequestView> {
    if !status.is_terminal() {
        return Err(Error::new(ErrorKind::Internal, "结束 request 必须使用终态"));
    }
    if let Some(receipt) = receipt_json {
        serde_json::from_str::<serde_json::Value>(receipt)
            .map_err(|_| Error::new(ErrorKind::Internal, "MCP 回执不是合法 JSON"))?;
    }
    let changed = conn
        .execute(
            "UPDATE ai_mcp_server_requests
                SET status=?2, receipt_json=?3, finished_at_ms=?4, revision=revision+1
              WHERE id=?1 AND revision=?5 AND status NOT IN
                    ('succeeded','failed','denied','cancelled','unknown')",
            params![id, status.as_str(), receipt_json, now_ms, expected_revision],
        )
        .map_err(sqlite_error)?;
    if changed == 0 {
        return Err(Error::new(
            ErrorKind::Internal,
            "MCP request 已结束或状态已变化",
        ));
    }
    load_server_request(conn, id)
}

/// 启动恢复：把上次进程退出时残留的非终态 request 收敛为终态。
///
/// 进程可能在 SQL 已提交、但终态还没落库时被杀，账本就永远停在 `running`：
/// 数据库已改、fluxDB 不知道结果。把它标成 `unknown` 而不是 `failed`，
/// 是为了让执行策略层（`ai_execution_policy` 的未知写冲突检查）拦住后续写入，
/// 直到人工核实——否则调用方重试会造成重复写。
///
/// 判定与内部 Turn 的启动恢复（`recovery::recover_on_startup`）一致：
/// - `prepared`：还没占用执行权，SQL 未发出，确定未执行 → `cancelled`
/// - `running` + 已实现的只读工具：读操作不改数据 → `cancelled`
/// - `running` + 其他（含 `db.run`/`redis.execute`）：结果待核实 → `unknown`
///
/// 等待审批的 request 不在这里处理：没有用户在场时不能把历史批准变成执行许可，
/// 也不该由本进程替用户决定，应保持 `waiting_approval` 等待重新审批。
pub fn recover_server_requests_on_start(conn: &Connection, now_ms: i64) -> Result<u64> {
    let raw: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id,tool_id,status FROM ai_mcp_server_requests WHERE status IN ('prepared','running')",
            )
            .map_err(sqlite_error)?;
        stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sqlite_error)?
    };
    let mut changed = 0u64;
    for (id, tool_id, raw_status) in raw {
        // status 受 DDL CHECK 约束；解析失败说明数据已损坏，必须报错而不是猜一个终态。
        let status = ServerRequestStatus::parse(&raw_status)?;
        let next = match status {
            // 尚未占用执行权：SQL 未发出，无论什么工具都确定未执行。
            ServerRequestStatus::Prepared => ServerRequestStatus::Cancelled,
            ServerRequestStatus::Running if is_interrupted_read_only_tool(&tool_id) => {
                ServerRequestStatus::Cancelled
            }
            _ => ServerRequestStatus::Unknown,
        };
        let updated = conn
            .execute(
                "UPDATE ai_mcp_server_requests
                    SET status=?2, finished_at_ms=?3, revision=revision+1
                  WHERE id=?1 AND status IN ('prepared','running')",
                params![id, next.as_str(), now_ms],
            )
            .map_err(sqlite_error)?;
        changed += updated as u64;
    }
    Ok(changed)
}

/// 启动恢复时的只读判定：与内部 Turn 恢复用同一份工具清单。
///
/// 只列已实现的专用只读工具；`redis.*` 不列入——它们的中断后果由各自的
/// 有界游标/停止句柄处理，不按"只读"断言已无副作用。
fn is_interrupted_read_only_tool(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "db.search_objects"
            | "db.describe_object"
            | "fluxdb_search_objects"
            | "fluxdb_describe_object"
    )
}

/// 列出某 session 的请求（活动页/状态查询用）。
pub fn list_requests_for_session(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<ServerRequestView>> {
    let ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT id FROM ai_mcp_server_requests WHERE session_id=?1 ORDER BY seq")
            .map_err(sqlite_error)?;
        stmt.query_map([session_id], |row| row.get::<_, String>(0))
            .map_err(sqlite_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sqlite_error)?
    };
    ids.into_iter()
        .map(|id| load_server_request(conn, &id))
        .collect()
}

/// 列出某 Profile 最近的请求（协议/审批审计用）。
pub fn list_requests_for_profile(
    conn: &Connection,
    profile_id: &str,
    limit: i64,
) -> Result<Vec<ServerRequestView>> {
    let ids: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM ai_mcp_server_requests WHERE profile_id=?1
                  ORDER BY started_at_ms DESC, seq DESC LIMIT ?2",
            )
            .map_err(sqlite_error)?;
        stmt.query_map(params![profile_id, limit], |row| row.get::<_, String>(0))
            .map_err(sqlite_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sqlite_error)?
    };
    ids.into_iter()
        .map(|id| load_server_request(conn, &id))
        .collect()
}

/// 为 MCP request 登记结果（外部 owner 分支，不建假 Turn）。
///
/// 与内部 Turn 结果分属两支；DDL 的 CHECK 保证 `mcp_session_id`/`mcp_request_id` 与
/// `session_id`/`turn_id` 二选一，跨 Client 取数据会被复合外键拒绝。
pub fn insert_mcp_result(
    conn: &Connection,
    now_ms: i64,
    id: &str,
    session_id: &str,
    request_id: &str,
    metadata_json: &str,
    retention_kind: &str,
    status: &str,
    relative_path: Option<&str>,
    byte_count: i64,
    content_hash: Option<&str>,
) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(metadata_json)
        .map_err(|_| Error::new(ErrorKind::Internal, "结果元数据不是合法 JSON"))?;
    if !["available", "expired", "deleted", "failed"].contains(&status) {
        return Err(Error::new(ErrorKind::Internal, "非法结果状态"));
    }
    if !["memory", "retained"].contains(&retention_kind) {
        return Err(Error::new(ErrorKind::Internal, "非法结果保留方式"));
    }
    if retention_kind == "retained" {
        let path = relative_path
            .ok_or_else(|| Error::new(ErrorKind::Internal, "retained 结果必须有相对路径"))?;
        let path = std::path::Path::new(path);
        if path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(Error::new(
                ErrorKind::Internal,
                "保留结果的路径必须是受控根内的相对路径",
            ));
        }
    }
    conn.execute(
        "INSERT INTO ai_results (id, mcp_session_id, mcp_request_id, metadata_json,
            retention_kind, status, relative_path, byte_count, content_hash, created_at_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id,
            session_id,
            request_id,
            metadata_json,
            retention_kind,
            status,
            relative_path,
            byte_count,
            content_hash,
            now_ms,
        ],
    )
    .map_err(|e| {
        let message = e.to_string();
        if message.contains("FOREIGN KEY") {
            Error::new(
                ErrorKind::Internal,
                "MCP result 的 session/request 不属于同一 Profile 会话",
            )
        } else {
            sqlite_error(e)
        }
    })?;
    Ok(())
}
