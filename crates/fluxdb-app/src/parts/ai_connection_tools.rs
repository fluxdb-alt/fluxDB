/// 连接发现只保存公开属性；返回列表不会扩大数据库 resolver 的执行范围。
struct AiConnectionsListTool {
    descriptor: fluxdb_core::ToolDescriptor,
    snapshot: serde_json::Value,
}

impl AiConnectionsListTool {
    fn new(
        connections: &[fluxdb_core::ConnectionConfig],
        current_connection: Option<fluxdb_core::ConnectionId>,
    ) -> Self {
        let connections = connections
            .iter()
            .map(|config| {
                let can_run_sql = Some(config.id) == current_connection;
                let can_browse_page = matches!(
                    config.kind,
                    fluxdb_core::DatabaseKind::Sqlite
                        | fluxdb_core::DatabaseKind::Postgres
                        | fluxdb_core::DatabaseKind::Kingbase
                        | fluxdb_core::DatabaseKind::MySql
                        | fluxdb_core::DatabaseKind::TiDb
                        | fluxdb_core::DatabaseKind::MariaDb
                        | fluxdb_core::DatabaseKind::Dameng
                        | fluxdb_core::DatabaseKind::SqlServer
                );
                serde_json::json!({
                    "connection_id": config.id.0,
                    "connection_name": config.name,
                    "database_kind": config.kind,
                    "connection_read_only": connection_is_read_only(config),
                    "is_current": Some(config.id) == current_connection,
                    // 目录能力必须与 resolver 的真实范围一致：列出连接不等于可执行 SQL。
                    "can_run_sql": can_run_sql,
                    "can_browse_page": can_browse_page,
                })
            })
            .collect::<Vec<_>>();
        Self {
            descriptor: fluxdb_core::ToolDescriptor {
                id: "connections.list".into(),
                version: 1,
                description: "列出已保存的 SQL 连接 ID、名称、数据库类型、只读标记，并明确 can_run_sql（只有当前绑定连接为 true）与 can_browse_page。不返回地址、路径或凭据。can_run_sql=false 时不要用 db.* 猜连接 ID；按表名或业务描述查表改用 page.find_table，需要逐级浏览时用 page.browse_objects。".into(),
                risk: fluxdb_core::ToolRisk::Read,
                input_schema_json: r#"{"type":"object","properties":{},"additionalProperties":false}"#.into(),
            },
            snapshot: serde_json::json!({
                "connections": connections,
                "snapshot_scope": "turn",
                "execution_scope": if current_connection.is_some() {
                    "current_connection_only"
                } else {
                    "no_bound_target"
                },
                "bound_connection_id": current_connection.map(|id| id.0),
                "guidance": if current_connection.is_some() {
                    "db.* only accepts the bound connection. Use page.find_table/page.browse_objects for any other saved connection."
                } else {
                    "No SQL execution target is bound. Do not try conn:<id>; use page.find_table or page.browse_objects to discover and open a table."
                },
            }),
        }
    }
}

impl fluxdb_core::AiTool for AiConnectionsListTool {
    fn descriptor(&self) -> &fluxdb_core::ToolDescriptor {
        &self.descriptor
    }

    fn execute(
        &self,
        ctx: fluxdb_core::ToolExecutionContext,
        _arguments_json: String,
    ) -> fluxdb_core::AiFuture<'_, fluxdb_core::ToolReceipt> {
        Box::pin(async move {
            let started_at_ms = now_ms();
            let cancelled = ctx.cancel.is_cancelled();
            Ok(fluxdb_core::ToolReceipt {
                call_id: ctx.call_id,
                tool_id: self.descriptor.id.clone(),
                status: if cancelled {
                    fluxdb_core::ToolReceiptStatus::Cancelled
                } else {
                    fluxdb_core::ToolReceiptStatus::Succeeded
                },
                result_json: if cancelled {
                    serde_json::json!({"error": "连接发现已取消", "retryable": false}).to_string()
                } else {
                    self.snapshot.to_string()
                },
                error_code: cancelled.then(|| "cancelled".into()),
                affected_rows: None,
                started_at_ms,
                finished_at_ms: now_ms(),
            })
        })
    }
}
