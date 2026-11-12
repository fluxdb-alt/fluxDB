//! 对外本地 MCP Server 的 Profile 与数据源持久化（设计 §28.6、§42.10–42.11、T060）。
//!
//! 与 `mcp.rs`（本应用作为 **Client** 接入外部工具）严格区分：这些表描述的是
//! 本应用 **对外** 暴露的 MCP Server（应用内 Streamable HTTP 端点）。规则：
//! 1. 对象级 CAS：Profile 与数据源的保存都带 `expected_revision`，命中 0 行即冲突。
//! 2. 默认最小权限：新建 Profile 强制 `enabled=0`；新建数据源策略为 `default`
//!    （跟随全局，全局默认是只读）。
//! 3. 数据源只描述“哪个连接对外、写入策略如何”；表/Key 范围不在此配置，数据范围
//!    等于该连接自身的授权范围。

use rusqlite::{Connection, params};

use crate::sqlite::sqlite_error;
use fluxdb_core::{Error, ErrorKind, Result};

/// 数据源的写入策略：`default` 跟随全局，`read_only` / `read_write` 显式覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasourceWritePolicy {
    Default,
    ReadOnly,
    ReadWrite,
}

impl DatasourceWritePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            DatasourceWritePolicy::Default => "default",
            DatasourceWritePolicy::ReadOnly => "read_only",
            DatasourceWritePolicy::ReadWrite => "read_write",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "default" => Ok(DatasourceWritePolicy::Default),
            "read_only" => Ok(DatasourceWritePolicy::ReadOnly),
            "read_write" => Ok(DatasourceWritePolicy::ReadWrite),
            other => Err(Error::new(
                ErrorKind::Internal,
                format!("未知 MCP 数据源写策略: {other}"),
            )),
        }
    }

    /// 解析为“是否允许写入”的确定结论：`default` 用全局策略回填。
    pub fn allows_write(self, global: fluxdb_core::McpWritePolicy) -> bool {
        match self {
            DatasourceWritePolicy::Default => global.allows_write(),
            DatasourceWritePolicy::ReadOnly => false,
            DatasourceWritePolicy::ReadWrite => true,
        }
    }
}

/// Server Profile 视图（不含任何秘密）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProfileView {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub options_json: String,
    pub revision: i64,
}

/// Profile 保存输入。
#[derive(Debug, Clone)]
pub struct ServerProfileEdit {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub options_json: String,
}

fn validate_options_json(raw: &str, label: &str) -> Result<()> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|_| Error::new(ErrorKind::Internal, format!("{label}不是合法 JSON")))?;
    if !value.is_object() {
        return Err(Error::new(
            ErrorKind::Internal,
            format!("{label}必须是对象"),
        ));
    }
    // 秘密不得落到 Profile 普通配置：凭据只走已有 AES 连接凭据。
    for forbidden in [
        "ap[REDACTED]ey",
        "token",
        "pas[REDACTED]rd",
        "aut[REDACTED]on",
        "sec[REDACTED]et",
    ] {
        if value.get(forbidden).is_some() {
            return Err(Error::new(
                ErrorKind::Internal,
                "MCP Server 配置不得包含凭据字段",
            ));
        }
    }
    Ok(())
}

/// 列出全部 Server Profile（管理页用）。
pub fn list_server_profiles(conn: &Connection) -> Result<Vec<ServerProfileView>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, display_name, enabled, options_json, revision
               FROM ai_mcp_server_profiles ORDER BY display_name, id",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ServerProfileView {
                id: row.get(0)?,
                display_name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                options_json: row.get(3)?,
                revision: row.get(4)?,
            })
        })
        .map_err(sqlite_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sqlite_error)
}

/// 读取单个 Server Profile；不存在返回 `None`。
pub fn load_server_profile(conn: &Connection, id: &str) -> Result<Option<ServerProfileView>> {
    Ok(list_server_profiles(conn)?.into_iter().find(|p| p.id == id))
}

/// 保存 Server Profile（对象级 CAS；`expected_revision=None` 表示首次创建）。
///
/// 新建强制 `enabled=0`：保存即生效会让"看一眼配置"意外放开对外服务。
pub fn save_server_profile(
    conn: &Connection,
    now_ms: i64,
    edit: &ServerProfileEdit,
    expected_revision: Option<i64>,
) -> Result<ServerProfileView> {
    if edit.id.trim().is_empty() {
        return Err(Error::new(
            ErrorKind::Internal,
            "Server Profile ID 不能为空",
        ));
    }
    validate_options_json(&edit.options_json, "Server Profile 配置")?;
    let first_create = expected_revision.is_none();
    let enabled = if first_create {
        0
    } else {
        i64::from(edit.enabled)
    };
    let sql = "INSERT INTO ai_mcp_server_profiles (id, display_name, enabled, transport,
            options_json, revision, created_at_ms, updated_at_ms)
        VALUES (?1,?2,?3,'http',?4,1,?5,?5)
        ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name, enabled=excluded.enabled,
            options_json=excluded.options_json,
            revision=ai_mcp_server_profiles.revision+1, updated_at_ms=excluded.updated_at_ms
        WHERE ai_mcp_server_profiles.revision=?6";
    let changed = conn
        .execute(
            sql,
            params![
                edit.id,
                edit.display_name,
                enabled,
                edit.options_json,
                now_ms,
                expected_revision.unwrap_or(0),
            ],
        )
        .map_err(sqlite_error)?;
    if changed == 0 {
        return Err(Error::new(
            ErrorKind::Internal,
            "MCP Server Profile 已被其他窗口修改，请重新加载",
        ));
    }
    load_server_profile(conn, &edit.id)?
        .ok_or_else(|| Error::new(ErrorKind::Internal, "MCP Server Profile 保存后读取失败"))
}

/// 数据源视图（对外可见的连接 + 写入策略）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasourceView {
    pub profile_id: String,
    pub connection_id: i64,
    pub write_policy: DatasourceWritePolicy,
    pub revision: i64,
}

/// 列出某 Profile 的数据源（按连接 id 排序）。
pub fn list_datasources(conn: &Connection, profile_id: &str) -> Result<Vec<DatasourceView>> {
    let mut stmt = conn
        .prepare(
            "SELECT profile_id, connection_id, write_policy, revision
               FROM ai_mcp_server_datasources WHERE profile_id=?1 ORDER BY connection_id",
        )
        .map_err(sqlite_error)?;
    let rows = stmt
        .query_map([profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(sqlite_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (profile_id, connection_id, policy, revision) = row.map_err(sqlite_error)?;
        out.push(DatasourceView {
            profile_id,
            connection_id,
            write_policy: DatasourceWritePolicy::parse(&policy)?,
            revision,
        });
    }
    Ok(out)
}

/// 读取单个数据源；不存在返回 `None`。
pub fn load_datasource(
    conn: &Connection,
    profile_id: &str,
    connection_id: i64,
) -> Result<Option<DatasourceView>> {
    Ok(list_datasources(conn, profile_id)?
        .into_iter()
        .find(|d| d.connection_id == connection_id))
}

/// 保存（新增或更新）一条数据源；新建时策略为 `default` 时不写覆盖行仍会登记数据源。
///
/// `expected_revision=None` 表示新增：同连接重复新增会被主键冲突拒绝。
pub fn save_datasource(
    conn: &Connection,
    now_ms: i64,
    profile_id: &str,
    connection_id: i64,
    policy: DatasourceWritePolicy,
    expected_revision: Option<i64>,
) -> Result<DatasourceView> {
    if profile_id.trim().is_empty() {
        return Err(Error::new(
            ErrorKind::Internal,
            "数据源必须归属一个 Server Profile",
        ));
    }
    let sql = "INSERT INTO ai_mcp_server_datasources (profile_id, connection_id, write_policy,
            revision, created_at_ms, updated_at_ms)
        VALUES (?1,?2,?3,1,?4,?4)
        ON CONFLICT(profile_id, connection_id) DO UPDATE SET write_policy=excluded.write_policy,
            revision=ai_mcp_server_datasources.revision+1, updated_at_ms=excluded.updated_at_ms
        WHERE ai_mcp_server_datasources.revision=?5";
    let changed = conn
        .execute(
            sql,
            params![
                profile_id,
                connection_id,
                policy.as_str(),
                now_ms,
                expected_revision.unwrap_or(0),
            ],
        )
        .map_err(sqlite_error)?;
    if changed == 0 {
        return Err(Error::new(
            ErrorKind::Internal,
            "MCP 数据源已被其他窗口修改或已存在，请重新加载",
        ));
    }
    load_datasource(conn, profile_id, connection_id)?
        .ok_or_else(|| Error::new(ErrorKind::Internal, "MCP 数据源保存后读取失败"))
}

/// 删除数据源（用户从对外服务移除该连接）。
pub fn delete_datasource(conn: &Connection, profile_id: &str, connection_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM ai_mcp_server_datasources WHERE profile_id=?1 AND connection_id=?2",
        params![profile_id, connection_id],
    )
    .map_err(sqlite_error)?;
    Ok(())
}
