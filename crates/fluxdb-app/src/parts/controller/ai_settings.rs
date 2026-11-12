// AI 设置页的读写编排：加载模型/Agent/插件/MCP 列表与启停切换（T013/T014）。
// UI 只发 AppCommand；本文件在控制器副本上访问存储，UI 不接触 SQLite 或凭据。
// 该文件以 include! 引入 crate 根作用域，Error/Result 等已在上方导入。

use fluxdb_storage::ai_platform::{
    agents, models, plugins, settings as ai_settings, tool_settings,
};
use fluxdb_storage::Storage as _;
use rusqlite::Connection as AiConn;

impl AppController {
    /// 克隆当前 AI 存储句柄；未注入时给出明确错误而不是静默空列表。
    ///
    /// 返回克隆值而不是借用，调用方可在持有它的同时修改 `AppState`。
    pub fn ai_storage(&self) -> fluxdb_core::Result<fluxdb_storage::FileStorage> {
        self.ai_storage.clone().ok_or_else(|| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Internal,
                "AI 存储尚未初始化，请重启应用",
            )
        })
    }

    /// 让应用内 MCP HTTP 端点与 Profile 的启用开关保持一致。
    ///
    /// 幂等：已启动且仍启用时不动；端口被占用等启动失败只记录日志并保持关闭，
    /// 用户下次切换开关会重试。关闭时立即取消全部会话，释放端口。
    pub(crate) fn sync_mcp_http_endpoint(&self) {
        let Ok(storage) = self.ai_storage() else {
            return;
        };
        let enabled = storage
            .with_ai_connection(|conn| {
                fluxdb_storage::ai_platform::mcp_server::load_server_profile(
                    conn,
                    crate::DEFAULT_MCP_PROFILE_ID,
                )
                .map(|profile| profile.is_some_and(|profile| profile.enabled))
            })
            .unwrap_or(false);
        let Ok(mut slot) = self.mcp_http.lock() else {
            return;
        };
        if !enabled {
            if let Some(handle) = slot.take() {
                handle.stop();
            }
            if let Ok(mut error) = self.mcp_http_error.lock() {
                *error = None;
            }
            return;
        }
        if slot.is_some() {
            return;
        }
        match crate::mcp_http_host::start(
            storage,
            crate::DEFAULT_MCP_PROFILE_ID,
            crate::mcp_http_host::DEFAULT_MCP_HTTP_PORT,
        ) {
            Ok(handle) => {
                *slot = Some(handle);
                if let Ok(mut error) = self.mcp_http_error.lock() {
                    *error = None;
                }
            }
            Err(message) => {
                tracing::warn!(%message, "MCP HTTP 端点未启动");
                if let Ok(mut error) = self.mcp_http_error.lock() {
                    *error = Some(message);
                }
            }
        }
    }

    /// 显式解密模型 API Key，供编辑弹框「眼睛」按钮按需调用。
    ///
    /// 明文只作为本函数返回值短暂存在：不进入 `AppEvent`/`AppState`、任何 DTO、
    /// 日志或错误文本；调用方用完应立即丢弃。
    pub fn reveal_ai_model_secret(&self, id: &str) -> fluxdb_core::Result<String> {
        use fluxdb_storage::ai_platform::model_secret::CredentialStatus;

        let no_key_error = || {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Internal,
                "该模型没有可显示的 API Key",
            )
        };

        let storage = self.ai_storage()?;
        let (view, sealed) = with_ai_storage(&storage, |conn| {
            Ok((
                models::load_model(conn, id)?,
                models::load_encrypted_secret(conn, id)?,
            ))
        })?;
        if view.credential_status != CredentialStatus::Configured {
            return Err(no_key_error());
        }
        let Some(sealed) = sealed else {
            return Err(no_key_error());
        };
        let (cipher, _source) = fluxdb_storage::ai_platform::load_ai_cipher(storage.root_dir())
            .map_err(|e| {
                fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, e.to_string())
            })?;
        fluxdb_storage::ai_platform::model_secret::open(&cipher, id, &sealed)
            .map_err(|e| fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, e.to_string()))
    }
}

/// 在 AI 存储连接上执行一次操作（存储层负责开关连接）。
fn with_ai_storage<T>(
    storage: &fluxdb_storage::FileStorage,
    f: impl FnOnce(&AiConn) -> fluxdb_core::Result<T>,
) -> fluxdb_core::Result<T> {
    storage.with_ai_connection(|conn| f(conn).map_err(Into::into))
}

/// 插件实例 → 设置页列表项：附带声明字段与宿主解析好的候选项。
///
/// 声明解析失败不隐藏实例，而是带上 `config_error`，让「插件在、但配置读不出来」可见。
pub fn ai_plugin_items(conn: &AiConn) -> fluxdb_core::Result<Vec<AiPluginItem>> {
    let registry = BuiltinRegistry::default();
    let mut items = Vec::new();
    for instance in plugins::list_plugin_instances(conn)? {
        let summary = registry
            .find_by_package(&instance.package_key)
            .map(|descriptor| descriptor.summary.to_string())
            .unwrap_or_default();
        let (config_fields, config_error) = ai_plugin_config_fields(conn, &instance.id);
        items.push(AiPluginItem {
            mandatory: plugins::is_mandatory(&instance.id),
            retired: registry.is_retired(&instance.id),
            id: instance.id,
            display_name: instance.display_name,
            package_key: instance.package_key,
            summary,
            enabled: instance.enabled,
            config_fields,
            config_error,
            revision: instance.revision,
        });
    }
    Ok(items)
}

/// 解析一个实例的声明并解析每个字段的候选；失败转成可见文案而不是整页 `Err`。
fn ai_plugin_config_fields(
    conn: &AiConn,
    instance_id: &str,
) -> (Vec<AiPluginConfigFieldItem>, Option<String>) {
    let declaration = match instance_declaration(conn, instance_id) {
        Ok(Some(declaration)) => declaration,
        Ok(None) => return (Vec::new(), None),
        Err(error) => return (Vec::new(), Some(error.message)),
    };
    // 标量字段的值存在实例自己的 `config_json`；读不出来按空配置处理（字段默认值兜底）。
    let instance_config = fluxdb_storage::ai_platform::plugins::load_plugin_instance_config(
        conn,
        instance_id,
    )
    .map(|config| config.values)
    .unwrap_or_default();
    let mut fields = Vec::new();
    for field in &declaration.fields {
        if !field.is_resource() {
            let resolved = resolve_scalar_field(&instance_config, field);
            fields.push(AiPluginConfigFieldItem {
                key: field.key.clone(),
                kind: scalar_kind(field.kind),
                group: field.group.clone(),
                label: field.label.clone(),
                help: field.help.clone(),
                hint: String::new(),
                placeholder: field.placeholder.clone(),
                required: field.required,
                selection: resolved.selection,
                selection_usable: resolved.selection_usable,
                choices: Vec::new(),
                options: field
                    .options
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|option| AiPluginConfigOptionItem {
                        value: option.value,
                        label: option.label,
                    })
                    .collect(),
                rejection: resolved.rejection,
            });
            continue;
        }
        match resolve_field(conn, field) {
            Ok(resolved) => {
                let selection_usable = resolved.selection_is_usable();
                let rejection = plugin_field_rejection(conn, field, &resolved);
                fields.push(AiPluginConfigFieldItem {
                    key: field.key.clone(),
                    kind: AiPluginConfigFieldKind::Resource,
                    group: field.group.clone(),
                    label: field.label.clone(),
                    help: field.help.clone(),
                    hint: field_hint(field),
                    placeholder: None,
                    required: field.required,
                    selection: resolved.selection,
                    selection_usable,
                    choices: resolved
                        .choices
                        .into_iter()
                        .map(|choice| AiPluginConfigChoiceItem {
                            id: choice.id,
                            label: choice.label,
                            detail: choice.detail,
                        })
                        .collect(),
                    options: Vec::new(),
                    rejection,
                });
            }
            Err(error) => return (fields, Some(error.message)),
        }
    }
    (fields, None)
}

/// 审计事实 → 审计列表行。
///
/// 目标与语句只做**有界投影**：目标从 `conn:<id>` 句柄还原成可读的连接名+库/schema，
/// 语句取入库前已脱敏的参数；解析失败时留可见占位而不是编造内容。
fn ai_audit_row(row: fluxdb_storage::ai_platform::audits::ToolAuditListRow) -> AiAuditRow {
    use fluxdb_storage::ai_platform::audits::{AuditDecision, AuditStage};
    let target = ai_audit_target_label(&row.target_json);
    let statement = ai_audit_statement(&row.arguments_redacted_json);
    AiAuditRow {
        id: row.id,
        tool_id: row.tool_id,
        origin: if row.origin == "mcp" {
            "对外 MCP".to_string()
        } else {
            "内部 Agent".to_string()
        },
        target,
        decision: match row.decision {
            AuditDecision::Allow => AiAuditDecision::Allow,
            AuditDecision::RequireApproval => AiAuditDecision::RequireApproval,
            AuditDecision::Deny => AiAuditDecision::Deny,
        },
        rule_ids: row.rule_ids,
        reason_codes: row.reason_codes,
        statement,
        ruleset_version: row.ruleset_version,
        plan_hash: row.plan_hash,
        created_at_ms: row.created_at_ms,
        stage: match row.stage {
            AuditStage::PreExecute => "pre_execute".to_string(),
            AuditStage::Revalidate => "revalidate".to_string(),
        },
    }
}

/// 审计目标的可读投影：从目标 JSON/句柄还原连接名与库/schema，不做网络查询。
fn ai_audit_target_label(target_json: &str) -> String {
    // 句柄形态：`conn:3[/db:demo][/schema:public]`。
    let handle = target_json.trim_matches('"');
    if handle.starts_with("conn:") {
        let mut parts = handle.split('/');
        let connection = parts.next().unwrap_or_default().replace("conn:", "");
        let mut label = format!("连接 #{connection}");
        for part in parts {
            if let Some(rest) = part.strip_prefix("db:") {
                label.push_str(&format!(" / {rest}"));
            } else if let Some(rest) = part.strip_prefix("schema:") {
                label.push_str(&format!(" · {rest}"));
            }
        }
        return label;
    }
    // JSON 形态：`{"connection_id":3,...}`。
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(target_json) {
        if let Some(id) = value.get("connection_id").and_then(|v| v.as_i64()) {
            let mut label = format!("连接 #{id}");
            if let Some(db) = value.get("database").and_then(|v| v.as_str()) {
                label.push_str(&format!(" / {db}"));
            }
            if let Some(schema) = value.get("schema").and_then(|v| v.as_str()) {
                label.push_str(&format!(" · {schema}"));
            }
            return label;
        }
    }
    "未指定目标".to_string()
}

/// 审计语句/命令的只读投影：取脱敏参数里的 `sql` 或 `argv`，都取不到时留空。
fn ai_audit_statement(arguments_redacted_json: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments_redacted_json) else {
        return String::new();
    };
    if let Some(sql) = value.get("sql").and_then(|v| v.as_str()) {
        return sql.to_string();
    }
    if let Some(argv) = value.get("argv").and_then(|v| v.as_array()) {
        return argv
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }
    String::new()
}

/// 声明字段种类 → 展示侧种类。
fn scalar_kind(kind: PluginFieldKind) -> AiPluginConfigFieldKind {
    match kind {
        PluginFieldKind::Switch => AiPluginConfigFieldKind::Switch,
        PluginFieldKind::Text => AiPluginConfigFieldKind::Text,
        PluginFieldKind::TextList => AiPluginConfigFieldKind::TextList,
        PluginFieldKind::Select => AiPluginConfigFieldKind::Select,
        PluginFieldKind::Resource => AiPluginConfigFieldKind::Resource,
    }
}

/// 字段当前不能直接使用时的可见原因：选择失效优先，其次是没有满足协议要求的候选。
fn plugin_field_rejection(
    conn: &AiConn,
    field: &PluginConfigField,
    resolved: &ResolvedPluginField,
) -> Option<String> {
    if let Some(id) = resolved.selection.as_deref() {
        return if resolved.selection_is_usable() {
            None
        } else {
            Some(selection_rejection(conn, field, id))
        };
    }
    if resolved.choices.is_empty() && resolved.protocol_rejected > 0 {
        return Some(format!(
            "已配置的 {} 个模型不满足该插件声明的协议要求",
            resolved.protocol_rejected
        ));
    }
    None
}

/// 加载设置页需要的全部 AI 列表（模型/Agent/插件/MCP）。
/// 会话历史每页条数；分页续读由 [`load_more_ai_sessions`] 负责。
pub const AI_SESSION_PAGE_SIZE: i64 = 100;

/// 安全审计列表一次加载的条数上界；审计是只读回溯，不分页续读。
pub const AI_AUDIT_PAGE_SIZE: i64 = 200;

pub fn refresh_ai_settings(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
) -> fluxdb_core::Result<()> {
    let (
        models,
        agents_list,
        plugin_items,
        sessions_page,
        sessions_total,
        runtime,
        tools_list,
    ) = with_ai_storage(&storage, |conn| {
        Ok((
            models::list_models_page(conn, 200, 0)?,
            agents::list_agents(conn)?,
            ai_plugin_items(conn)?,
            fluxdb_storage::ai_platform::sessions::list_session_overviews_page(
                conn,
                AI_SESSION_PAGE_SIZE,
                None,
                None,
            )?,
            fluxdb_storage::ai_platform::sessions::count_sessions(conn, None)?,
            ai_settings::load_runtime_settings(conn)?,
            tool_settings::list_tool_settings(conn)?,
        ))
    })?;
    state.ai_models = models
        .into_iter()
        .map(|m| {
            let capability_labels = crate::model_capability_labels(&m)
                .into_iter()
                .map(str::to_string)
                .collect();
            AiModelItem {
            id: m.id,
            display_name: m.display_name,
            provider: m.provider,
            model: m.model,
            endpoint: m.endpoint,
            endpoint_is_full_url: m.endpoint_is_full_url,
            kind: match m.kind {
                fluxdb_storage::ai_platform::models::ModelKind::Generation => {
                    crate::AiModelKind::Generation
                }
                fluxdb_storage::ai_platform::models::ModelKind::Decision => {
                    crate::AiModelKind::Decision
                }
            },
            enabled: m.enabled,
            egress: m.egress == fluxdb_storage::ai_platform::models::EgressPermission::Allow,
            capability_labels,
            context_window_tokens: m.context_window_tokens,
            max_output_tokens: m.max_output_tokens,
            input_price_micros: m.input_price_micros_per_million,
            output_price_micros: m.output_price_micros_per_million,
            cached_input_price_micros: m.cached_input_price_micros_per_million,
            credential_status: credential_status_value(m.credential_status),
            connection_status: m.connection_status,
            connection_checked_at_ms: m.connection_checked_at_ms,
            revision: m.revision,
            }
        })
        .collect();
    state.ai_tool_settings = tools_list
        .into_iter()
        .map(|t| AiToolSettingItem {
            tool_id: t.tool_id,
            enabled: t.enabled,
            revision: t.revision,
        })
        .collect();
    state.ai_agents = agents_list
        .into_iter()
        .map(|a| AiAgentItem {
            compression_trigger_percent: a.compression_trigger_percent,
            max_iterations: a.max_iterations,
            id: a.id,
            display_name: a.display_name,
            description: a.description,
            system_prompt: a.system_prompt,
            model_profile_id: a.default_model_profile_id,
            allowed_tools: a.allowed_tools,
            enabled: a.enabled,
            revision: a.revision,
        })
        .collect();
    state.ai_plugin_instances = plugin_items;
    // 对外 Server 的 Profile / 数据源加载。
    // 连接名称来自普通配置（config.toml），与 AI SQLite 无关，先读一次避免在闭包内再进存储。
    let connection_names = storage
        .load_connections()
        .unwrap_or_default()
        .iter()
        .map(|config| (config.id.0 as i64, config.name.clone(), config.kind))
        .collect::<Vec<_>>();
    let (server_profiles, datasources) = with_ai_storage(&storage, |conn| {
        use fluxdb_storage::ai_platform::mcp_server;

        let profiles = mcp_server::list_server_profiles(conn)?;
        let datasources = profiles
            .iter()
            .flat_map(|profile| {
                mcp_server::list_datasources(conn, &profile.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|datasource| (profile.id.clone(), datasource))
            })
            .collect::<Vec<_>>();
        Ok((profiles, datasources))
    })?;
    state.ai_mcp_server_profiles = server_profiles
        .into_iter()
        .map(|profile| AiMcpServerProfileItem {
            id: profile.id,
            display_name: profile.display_name,
            enabled: profile.enabled,
            revision: profile.revision,
        })
        .collect();
    state.ai_mcp_datasources = datasources
        .into_iter()
        .map(|(profile_id, datasource)| {
            let (connection_name, kind) = connection_names
                .iter()
                .find(|(id, _, _)| *id == datasource.connection_id)
                .map(|(_, name, kind)| (name.clone(), *kind))
                .unwrap_or_else(|| {
                    (
                        format!("连接 #{}", datasource.connection_id),
                        fluxdb_core::DatabaseKind::Sqlite,
                    )
                });
            let resource_kind = if kind == fluxdb_core::DatabaseKind::Redis {
                "redis"
            } else {
                "sql"
            }
            .to_string();
            AiMcpDatasourceItem {
                profile_id,
                connection_id: datasource.connection_id,
                connection_name,
                resource_kind,
                write_policy: datasource.write_policy.as_str().to_string(),
                revision: datasource.revision,
            }
        })
        .collect();
    // 审计列表：读库失败转成可见原因，不伪装成"暂无记录"（空态与错误态必须区分）。
    match with_ai_storage(storage, |conn| {
        fluxdb_storage::ai_platform::audits::list_recent_tool_audits(conn, None, AI_AUDIT_PAGE_SIZE)
    }) {
        Ok(rows) => {
            state.ai_audit_rows = rows.into_iter().map(ai_audit_row).collect();
            state.ai_audit_error = None;
        }
        Err(error) => {
            state.ai_audit_rows = Vec::new();
            state.ai_audit_error = Some(error.message);
        }
    }
    state.ai_sessions = sessions_page.sessions.into_iter().map(ai_session_item).collect();
    state.ai_sessions_total = sessions_total;
    state.ai_sessions_cursor = sessions_page.next_cursor;
    state.ai_sessions_has_more = state.ai_sessions_cursor.is_some();
    state.ai_sessions_loading_more = false;
    // 默认主模型与运行设置 revision 一起回填，供大模型列表的「默认」单选使用。
    state.ai_default_model_id = runtime.default_model_profile_id.clone();
    state.ai_runtime_revision = runtime.revision;
    state.ai_execution_policy=with_ai_storage(storage,fluxdb_storage::ai_platform::execution_policy::load)?.0;
    state.ai_jev_status = with_ai_storage(storage, |conn| {
        reconcile_jev_builtins(conn, ai_chat_now())
    })?;
    Ok(())
}

/// 把后台克隆控制器加载出的 AI 设置快照合并回 live 控制器的 `AppState`。
///
/// 后台刷新跑在自己的克隆控制器上（不阻塞 UI），克隆状态与页面持有的控制器无关，
/// 因此必须显式回搬，否则页面仍显示刷新前的旧记录。写入字段与
/// [`refresh_ai_settings`] 的写入集合严格一致；会话面板与其它 UI 私有状态
/// （如 `ai_conversation`）不在合并范围内，避免用点击瞬间的旧快照覆盖。
pub fn merge_ai_settings(state: &mut AppState, loaded: &AppState) {
    state.ai_models = loaded.ai_models.clone();
    state.ai_tool_settings = loaded.ai_tool_settings.clone();
    state.ai_agents = loaded.ai_agents.clone();
    state.ai_plugin_instances = loaded.ai_plugin_instances.clone();
    state.ai_mcp_server_profiles = loaded.ai_mcp_server_profiles.clone();
    state.ai_mcp_datasources = loaded.ai_mcp_datasources.clone();
    state.ai_audit_rows = loaded.ai_audit_rows.clone();
    state.ai_audit_error = loaded.ai_audit_error.clone();
    // 刷新回到首页：会话分页游标与已加载页一起被替换，"加载更多" 按新游标续读。
    state.ai_sessions = loaded.ai_sessions.clone();
    state.ai_sessions_total = loaded.ai_sessions_total;
    state.ai_sessions_cursor = loaded.ai_sessions_cursor.clone();
    state.ai_sessions_has_more = loaded.ai_sessions_has_more;
    state.ai_sessions_loading_more = loaded.ai_sessions_loading_more;
    state.ai_default_model_id = loaded.ai_default_model_id.clone();
    state.ai_runtime_revision = loaded.ai_runtime_revision;
    state.ai_execution_policy = loaded.ai_execution_policy.clone();
    state.ai_jev_status = loaded.ai_jev_status.clone();
}

/// 会话概览 → 设置页列表项（不含正文与凭据）。
fn ai_session_item(session: fluxdb_storage::ai_platform::sessions::SessionOverview) -> AiSessionItem {
    AiSessionItem {
        id: session.id,
        title: session.title,
        agent_id: session.agent_id,
        status: session.status.as_str().to_string(),
        last_turn_status: session
            .last_turn_status
            .map(|status| status.as_str().to_string()),
        last_target_json: session.last_target_json,
        message_count: session.message_count,
        turn_count: session.turn_count,
        updated_at_ms: session.updated_at_ms,
        revision: session.revision,
    }
}

/// 续读会话历史下一页：按当前游标追加，按 id 去重，不覆盖已有行。
///
/// 返回本页新增条数；调用后 `ai_sessions_has_more`/`ai_sessions_cursor` 反映下一页状态。
pub fn load_more_ai_sessions(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
) -> fluxdb_core::Result<usize> {
    if !state.ai_sessions_has_more || state.ai_sessions_loading_more {
        return Ok(0);
    }
    let cursor = state.ai_sessions_cursor.clone();
    state.ai_sessions_loading_more = true;
    let loaded = with_ai_storage(storage, |conn| {
        fluxdb_storage::ai_platform::sessions::list_session_overviews_page(
            conn,
            AI_SESSION_PAGE_SIZE,
            cursor.as_ref(),
            None,
        )
    });
    state.ai_sessions_loading_more = false;
    let page = loaded?;
    let existing: std::collections::HashSet<String> =
        state.ai_sessions.iter().map(|session| session.id.clone()).collect();
    let mut appended = 0usize;
    for session in page.sessions {
        if existing.contains(&session.id) {
            continue;
        }
        state.ai_sessions.push(ai_session_item(session));
        appended += 1;
    }
    state.ai_sessions_cursor = page.next_cursor;
    state.ai_sessions_has_more = state.ai_sessions_cursor.is_some();
    Ok(appended)
}

/// 切换模型启用状态（对象级 CAS）。
pub fn set_ai_model_enabled(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    id: &str,
    enabled: bool,
    expected_revision: i64,
    now_ms: i64,
) -> fluxdb_core::Result<()> {
    with_ai_storage(storage, |conn| {
        models::set_model_enabled(conn, now_ms, id, enabled, expected_revision)?;
        reconcile_jev_builtins(conn, now_ms)
    })?;
    refresh_jev_status(state, storage)?;
    // 就地更新列表，避免整页重载闪烁。
    if let Some(m) = state.ai_models.iter_mut().find(|m| m.id == id) {
        m.enabled = enabled;
        m.revision = expected_revision + 1;
    }
    Ok(())
}

/// 切换内置插件实例启停。
pub fn set_ai_plugin_enabled(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    id: &str,
    enabled: bool,
    expected_revision: i64,
    now_ms: i64,
) -> fluxdb_core::Result<()> {
    // 已下线的占位行没有实现，不能靠开关制造"已启用"的假象。
    if BuiltinRegistry::default().is_retired(id) {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Internal,
            "该内置能力已下线，暂不可启停",
        ));
    }
    with_ai_storage(storage, |conn| {
        plugins::set_plugin_instance_enabled(conn, now_ms, id, enabled, expected_revision)?;
        reconcile_jev_builtins(conn, now_ms)
    })?;
    refresh_jev_status(state, storage)?;
    if let Some(p) = state.ai_plugin_instances.iter_mut().find(|p| p.id == id) {
        p.enabled = enabled;
        p.revision = expected_revision + 1;
    }
    Ok(())
}

/// 保存插件配置：按插件声明校验后写回宿主槽位，再就地刷新可见状态。
///
/// 应用层在这里承担三件事：从声明定位需求（不认插件 id 特判）、按声明过滤候选
/// （不满足要求的模型即使存在也存不进去）、把结果落回宿主自己的角色槽位。
pub fn save_ai_plugin_config(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    instance_id: &str,
    selections: &[AiPluginFieldSelection],
    expected_revision: i64,
    now_ms: i64,
) -> fluxdb_core::Result<i64> {
    let revision = with_ai_storage(storage, |conn| {
        let declaration = instance_declaration(conn, instance_id)?.ok_or_else(|| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Internal,
                "该插件没有配置声明，无法保存配置",
            )
        })?;
        save_selection(
            conn,
            instance_id,
            &declaration,
            selections,
            expected_revision,
            now_ms,
        )
    })?;
    // 候选与状态都由槽位推导，保存后就地重算，避免整页刷新闪烁。
    let runtime = with_ai_storage(storage, ai_settings::load_runtime_settings)?;
    state.ai_default_model_id = runtime.default_model_profile_id.clone();
    state.ai_runtime_revision = revision;
    state.ai_plugin_instances = with_ai_storage(storage, ai_plugin_items)?;
    refresh_jev_status(state, storage)?;
    Ok(revision)
}

/// 切换标准工具的全局启停。
///
/// 全局开关与 Agent 白名单取交集。保留白名单原意，避免删除最后一项后
/// 变成空列表（空列表表示“不限制”），意外放开其它工具。
pub fn set_ai_tool_enabled(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    tool_id: &str,
    enabled: bool,
    expected_revision: i64,
    now_ms: i64,
) -> fluxdb_core::Result<Vec<String>> {
    let mut conn = storage.open_ai_connection()?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, e.to_string()))?;
    let saved = tool_settings::save_tool_setting(
        &tx,
        now_ms,
        tool_id,
        enabled,
        expected_revision,
    )?;
    // 返回有效能力受影响的 Agent id，但不改写其白名单或 revision。
    let mut affected = Vec::new();
    if !enabled {
        for agent in agents::list_agents(&tx)? {
            if !agent.allowed_tools.iter().any(|id| id == tool_id) {
                continue;
            }
            affected.push(agent.id);
        }
    }
    tx.commit()
        .map_err(|e| fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, e.to_string()))?;

    // 就地回填，避免整页刷新闪烁。
    match state
        .ai_tool_settings
        .iter_mut()
        .find(|t| t.tool_id == tool_id)
    {
        Some(item) => {
            item.enabled = enabled;
            item.revision = saved.revision;
        }
        None => state.ai_tool_settings.push(AiToolSettingItem {
            tool_id: tool_id.to_string(),
            enabled,
            revision: saved.revision,
        }),
    }
    Ok(affected)
}

/// 读取「询问 AI」面板偏好（打开面板时用）。
pub fn load_panel_preferences(
    storage: &fluxdb_storage::FileStorage,
) -> fluxdb_core::Result<ai_settings::PanelPreferences> {
    with_ai_storage(storage, |conn| ai_settings::load_panel_preferences(conn))
}

/// 可选模型 Provider。
///
/// Provider 是模型类型与协议的**唯一来源**：UI 不再让用户单独选「模型用途」，
/// 选了 `decisions` 即表示 Jev 决策模型（§18 内置可选）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProvider {
    OpenAiChat,
    OpenAiResponses,
    Anthropic,
    Decisions,
}

/// Provider 稳定标识（落库到 `ai_model_profiles.provider`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderId {
    OpenAiChat,
    OpenAiResponses,
    Anthropic,
    Decisions,
}

impl ModelProvider {
    pub const ALL: [ModelProvider; 4] = [
        ModelProvider::OpenAiChat,
        ModelProvider::OpenAiResponses,
        ModelProvider::Anthropic,
        ModelProvider::Decisions,
    ];

    /// UI 展示名。
    pub fn label(self) -> &'static str {
        match self {
            ModelProvider::OpenAiChat => "OpenAI Chat Completions",
            ModelProvider::OpenAiResponses => "OpenAI Responses",
            ModelProvider::Anthropic => "Anthropic Messages",
            ModelProvider::Decisions => "Decisions",
        }
    }

    pub fn id(self) -> ProviderId {
        match self {
            ModelProvider::OpenAiChat => ProviderId::OpenAiChat,
            ModelProvider::OpenAiResponses => ProviderId::OpenAiResponses,
            ModelProvider::Anthropic => ProviderId::Anthropic,
            ModelProvider::Decisions => ProviderId::Decisions,
        }
    }

    /// 解析 [`Self::stored_id`] 写下的稳定标识；未知值回退 OpenAI Chat。
    ///
    /// 只用于**读取**历史数据（行里存的就是稳定标识）。写入路径必须先校验标识已知，
    /// 否则同样的兜底会把用户选的类型静默换回 OpenAI Chat。
    pub fn from_stored(value: &str) -> Self {
        match value {
            "openai-responses" => ModelProvider::OpenAiResponses,
            "anthropic" => ModelProvider::Anthropic,
            "decisions" => ModelProvider::Decisions,
            _ => ModelProvider::OpenAiChat,
        }
    }

    /// 解析 UI 展示名（Provider 下拉的值就是它）；未知值返回 `None`。
    ///
    /// 展示名与稳定标识是两套取值空间，不能混在一个解析器里，否则下拉里改了
    /// Provider 会被按存储标识解析后静默回落成默认项。
    pub fn from_label(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.label() == value)
    }

    /// 数据库中的稳定 Provider 标识；UI 改名不能改变历史值语义。
    pub fn stored_id(self) -> &'static str {
        match self {
            ModelProvider::OpenAiChat => "openai",
            ModelProvider::OpenAiResponses => "openai-responses",
            ModelProvider::Anthropic => "anthropic",
            ModelProvider::Decisions => "decisions",
        }
    }

    fn api_protocol(self) -> &'static str {
        match self {
            ModelProvider::OpenAiChat => "openai-compatible",
            ModelProvider::OpenAiResponses => "openai-responses",
            ModelProvider::Anthropic => "anthropic-messages",
            ModelProvider::Decisions => "system-one",
        }
    }
}

/// 按 Provider 推导模型类型与协议。
pub fn resolve_provider(
    provider: &str,
) -> (fluxdb_storage::ai_platform::models::ModelKind, &'static str) {
    let p = ModelProvider::from_stored(provider);
    let kind = match p {
        // Jev 决策模型：只做 Choice/Score 分类，不做工具调用。
        ModelProvider::Decisions => fluxdb_storage::ai_platform::models::ModelKind::Decision,
        ModelProvider::OpenAiChat | ModelProvider::OpenAiResponses | ModelProvider::Anthropic => {
            fluxdb_storage::ai_platform::models::ModelKind::Generation
        }
    };
    (kind, p.api_protocol())
}

/// 新建 id：AI 实体使用 UUID 文本（连接仍是 INTEGER ID）。
fn new_entity_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

/// 保存模型档案：新建或按 revision 更新，Key 走 Keep/Set/Clear。
///
/// 明文 Key 只在本次调用内进入加解密路径，不写日志/事件/状态。
pub fn save_ai_model(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    input: &AiModelSaveInput,
) -> fluxdb_core::Result<(String, i64)> {
    let root = storage.root_dir();
    let (cipher, _source) = fluxdb_storage::ai_platform::load_ai_cipher(root)
        .map_err(|e| fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, e.to_string()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let auth_mode = if input.api_key.is_some() || input.clear_key {
        fluxdb_storage::ai_platform::models::AuthMode::ApiKey
    } else {
        // 无 Key 变更时沿用原认证方式；新建默认无需凭据。
        match input.expected_revision {
            None => fluxdb_storage::ai_platform::models::AuthMode::None,
            Some(rev) => state
                .ai_models
                .iter()
                .find(|m| m.id == input.id.clone().unwrap_or_default())
                .filter(|m| m.revision == rev)
                .map(|_| fluxdb_storage::ai_platform::models::AuthMode::ApiKey)
                .unwrap_or(fluxdb_storage::ai_platform::models::AuthMode::None),
        }
    };
    let change = if let Some(key) = &input.api_key {
        let secret = fluxdb_storage::ai_platform::model_secret::SecretInput::new(key.clone())
            .map_err(|e| {
                fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, e.to_string())
            })?;
        fluxdb_storage::ai_platform::models::ApiKeyChange::Set(secret)
    } else if input.clear_key {
        fluxdb_storage::ai_platform::models::ApiKeyChange::Clear
    } else {
        fluxdb_storage::ai_platform::models::ApiKeyChange::Keep
    };

    if let Some(endpoint) = &input.endpoint {
        ai_endpoint_url(endpoint)?;
    } else if input.endpoint_is_full_url {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Internal,
            "完整 URL 模式必须填写请求地址",
        ));
    }
    // 写入路径必须校验 Provider 已知：`from_stored` 的兜底分支会把未知值静默当成
    // OpenAI Chat，那会让用户在下拉里选的 Provider 存不进去（协议也随之被换掉）。
    if !ModelProvider::ALL
        .iter()
        .any(|provider| provider.stored_id() == input.provider)
    {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Internal,
            format!("Provider 取值无效：{}", input.provider),
        ));
    }
    let provider = ModelProvider::from_stored(&input.provider);
    let (kind, api_protocol) = resolve_provider(input.provider.as_str());
    let edit = fluxdb_storage::ai_platform::models::ModelEdit {
        id: input.id.clone().unwrap_or_else(|| new_entity_id("model")),
        display_name: input.display_name.clone(),
        provider: provider.stored_id().to_string(),
        api_protocol: api_protocol.to_string(),
        kind,
        model: input.model.clone(),
        endpoint: input.endpoint.clone(),
        endpoint_is_full_url: input.endpoint_is_full_url,
        enabled: input.enabled,
        auth_mode,
        egress: if input.egress {
            fluxdb_storage::ai_platform::models::EgressPermission::Allow
        } else {
            fluxdb_storage::ai_platform::models::EgressPermission::Deny
        },
        context_window_tokens: input.context_window_tokens,
        max_output_tokens: input.max_output_tokens,
        input_price_micros_per_million: input.input_price_micros,
        output_price_micros_per_million: input.output_price_micros,
        cached_input_price_micros_per_million: input.cached_input_price_micros,
    };

    let saved = with_ai_storage(storage, |conn| {
        let saved = fluxdb_storage::ai_platform::models::save_model(
            conn,
            &cipher,
            "k1",
            now,
            &edit,
            input.expected_revision,
            change,
        )?;
        // 模型保存会改用途/协议/启停/凭据（Keep/Set/Clear），任一变化都要重新装配。
        reconcile_jev_builtins(conn, now)?;
        Ok(saved)
    })?;
    refresh_jev_status(state, storage)?;
    let (id, revision) = (saved.id.clone(), saved.revision);
    // 就地更新列表，避免整页刷新闪烁。
    match state.ai_models.iter_mut().find(|m| m.id == id) {
        Some(existing) => {
            let capability_labels = crate::model_capability_labels(&saved)
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            existing.display_name = saved.display_name;
            existing.enabled = saved.enabled;
            existing.egress = saved.egress
                == fluxdb_storage::ai_platform::models::EgressPermission::Allow;
            existing.provider = saved.provider;
            existing.model = saved.model;
            existing.endpoint = saved.endpoint;
            existing.endpoint_is_full_url = saved.endpoint_is_full_url;
            // Provider 变化会同时改变模型类型（decisions=决策模型），就地列表必须同步。
            existing.kind =
                if saved.kind == fluxdb_storage::ai_platform::models::ModelKind::Decision {
                    crate::AiModelKind::Decision
                } else {
                    crate::AiModelKind::Generation
                };
            existing.context_window_tokens = saved.context_window_tokens;
            existing.max_output_tokens = saved.max_output_tokens;
            existing.input_price_micros = saved.input_price_micros_per_million;
            existing.output_price_micros = saved.output_price_micros_per_million;
            existing.cached_input_price_micros = saved.cached_input_price_micros_per_million;
            existing.revision = revision;
            existing.capability_labels = capability_labels;
            existing.credential_status = credential_status_value(saved.credential_status);
            existing.connection_status = saved.connection_status;
            existing.connection_checked_at_ms = saved.connection_checked_at_ms;
        }
        None => {
            let capability_labels = crate::model_capability_labels(&saved)
                .into_iter()
                .map(str::to_string)
                .collect();
            state.ai_models.push(AiModelItem {
            id: id.clone(),
            display_name: saved.display_name,
            provider: saved.provider,
            model: saved.model,
            endpoint: saved.endpoint,
            endpoint_is_full_url: saved.endpoint_is_full_url,
            kind: if saved.kind == fluxdb_storage::ai_platform::models::ModelKind::Decision {
                crate::AiModelKind::Decision
            } else {
                crate::AiModelKind::Generation
            },
            enabled: saved.enabled,
            egress: saved.egress
                == fluxdb_storage::ai_platform::models::EgressPermission::Allow,
            capability_labels,
            context_window_tokens: saved.context_window_tokens,
            max_output_tokens: saved.max_output_tokens,
            input_price_micros: saved.input_price_micros_per_million,
            output_price_micros: saved.output_price_micros_per_million,
            cached_input_price_micros: saved.cached_input_price_micros_per_million,
            credential_status: credential_status_value(saved.credential_status),
            connection_status: saved.connection_status,
            connection_checked_at_ms: saved.connection_checked_at_ms,
            revision,
            });
        }
    }
    Ok((id, revision))
}

fn credential_status_value(
    status: fluxdb_storage::ai_platform::model_secret::CredentialStatus,
) -> crate::AiCredentialStatus {
    use fluxdb_storage::ai_platform::model_secret::CredentialStatus as S;
    match status {
        S::NotRequired => crate::AiCredentialStatus::NotRequired,
        S::Missing => crate::AiCredentialStatus::Missing,
        S::Configured => crate::AiCredentialStatus::Configured,
        S::Unavailable => crate::AiCredentialStatus::Unavailable,
    }
}

/// 重新读取 Jev 状态（reconcile 后调用，保证 UI 与运行期看到同一判定）。
pub fn refresh_jev_status(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
) -> fluxdb_core::Result<()> {
    state.ai_jev_status = with_ai_storage(storage, |conn| jev_status(conn, ai_chat_now()))?;
    Ok(())
}

/// 新建或保存 Agent 配置（提示词直接随 Agent 行 CAS）。
///
/// `input.id` 为 `None` 走新建，`Some` 走带 `expected_revision` 的对象级 CAS。
/// 成功后返回 agent id，并就地更新 `state.ai_agents`（新增则 push），避免整页重载。
pub fn save_ai_agent(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    input: AiAgentSaveInput,
    now_ms: i64,
) -> fluxdb_core::Result<String> {
    use fluxdb_storage::ai_platform::agents;

    let display_name = input.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Internal,
            "名称不能为空",
        ));
    }

    let saved = with_ai_storage(storage, |conn| {
        // 编辑时先读当前行，保留本表单不编辑的预算策略与允许工具，避免误清空。
        let existing = match &input.id {
            Some(id) => Some(agents::load_agent(conn, id)?),
            None => None,
        };
        let edit = agents::AgentEdit {
            compression_trigger_percent: input.compression_trigger_percent,
            max_iterations: input.max_iterations,
            id: input.id.clone().unwrap_or_else(|| new_entity_id("agent")),
            display_name: display_name.clone(),
            description: input.description.clone(),
            enabled: input.enabled,
            default_model_profile_id: input.model_profile_id.clone(),
            system_prompt: input.system_prompt.clone(),
            budget_policy_id: existing.as_ref().and_then(|a| a.budget_policy_id.clone()),
            allowed_tools: input.allowed_tools.clone(),
        };
        let saved = match &input.id {
            None => agents::create_agent(conn, now_ms, &edit),
            Some(_) => agents::save_agent(conn, now_ms, &edit, input.expected_revision),
        };
        // Agent 新建/编辑是设计 §18.1 列出的 reconcile 触发点。
        saved.and_then(|agent| {
            reconcile_jev_builtins(conn, now_ms)?;
            Ok(agent)
        })
    })?;
    refresh_jev_status(state, storage)?;

    let id = saved.id.clone();
    let item = AiAgentItem {
        compression_trigger_percent: saved.compression_trigger_percent,
        max_iterations: saved.max_iterations,
        id: saved.id,
        display_name: saved.display_name,
        description: saved.description,
        system_prompt: saved.system_prompt,
        model_profile_id: saved.default_model_profile_id,
        allowed_tools: saved.allowed_tools,
        enabled: saved.enabled,
        revision: saved.revision,
    };
    match state.ai_agents.iter_mut().find(|a| a.id == id) {
        Some(existing) => *existing = item,
        None => state.ai_agents.push(item),
    }
    Ok(id)
}

/// 保存默认主模型选择（运行设置单例 CAS）。
///
/// 应用层强校验：主模型必须是存在的 `generation`；决策与压缩决策槽位由各自的插件
/// 管理弹框写入，这里保持原值不动（默认 Agent、最大活动任务数同理）。
pub fn set_ai_default_model(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    model_id: Option<String>,
    expected_revision: i64,
    now_ms: i64,
) -> fluxdb_core::Result<i64> {
    use fluxdb_storage::ai_platform::models::ModelKind;
    let saved = with_ai_storage(storage, |conn| {
        if let Some(id) = &model_id {
            let model = models::load_model(conn, id)?;
            if model.kind != ModelKind::Generation {
                return Err(fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Internal,
                    "默认主模型必须是生成模型",
                ));
            }
        }
        let current = ai_settings::load_runtime_settings(conn)?;
        let next = ai_settings::RuntimeSettings {
            default_agent_id: current.default_agent_id.clone(),
            default_model_profile_id: model_id.clone(),
            default_decision_model_profile_id: current
                .default_decision_model_profile_id
                .clone(),
            compaction_decision_model_profile_id: current
                .compaction_decision_model_profile_id
                .clone(),
            max_active_turns: current.max_active_turns,
            revision: current.revision,
        };
        ai_settings::save_runtime_settings(conn, now_ms, &next, expected_revision)
    })?;
    // 就地回填，避免整页刷新闪烁。
    state.ai_default_model_id = model_id;
    state.ai_runtime_revision = saved.revision;
    Ok(saved.revision)
}

/// 新建或保存对外 MCP Server Profile。
///
/// 新建时存储层强制 `enabled=false`。
pub fn save_ai_mcp_server_profile(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    input: AiMcpServerProfileSaveInput,
    now_ms: i64,
) -> fluxdb_core::Result<(String, i64)> {
    use fluxdb_storage::ai_platform::mcp_server::ServerProfileEdit;

    let display_name = input.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Internal,
            "Server Profile 名称不能为空",
        ));
    }
    let edit = ServerProfileEdit {
        id: input.id.clone().unwrap_or_else(|| new_entity_id("mcp-server")),
        display_name,
        enabled: input.enabled,
        options_json: "{}".to_string(),
    };
    let saved = with_ai_storage(&storage, |conn| {
        fluxdb_storage::ai_platform::mcp_server::save_server_profile(
            conn,
            now_ms,
            &edit,
            input.expected_revision,
        )
    })?;
    let item = AiMcpServerProfileItem {
        id: saved.id.clone(),
        display_name: saved.display_name,
        enabled: saved.enabled,
        revision: saved.revision,
    };
    // 新建 push、编辑就地替换 revision，避免整页重载闪烁。
    match state
        .ai_mcp_server_profiles
        .iter_mut()
        .find(|profile| profile.id == item.id)
    {
        Some(slot) => *slot = item,
        None => state.ai_mcp_server_profiles.push(item),
    }
    state
        .ai_mcp_server_profiles
        .sort_by(|left, right| left.display_name.cmp(&right.display_name));
    Ok((saved.id, saved.revision))
}

/// 新增或更新 MCP 数据源（写入策略：`default` / `read_only` / `read_write`）。
pub fn save_ai_mcp_datasource(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    input: AiMcpDatasourceSaveInput,
    now_ms: i64,
) -> fluxdb_core::Result<(i64, i64)> {
    use fluxdb_storage::ai_platform::mcp_server::{
        DatasourceWritePolicy, save_datasource,
    };

    if input.profile_id.trim().is_empty() {
        return Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Internal,
            "数据源必须归属一个 Server Profile",
        ));
    }
    let policy = DatasourceWritePolicy::parse(&input.write_policy)?;
    let connections = storage.load_connections().unwrap_or_default();
    let config = connections
        .iter()
        .find(|config| config.id.0 as i64 == input.connection_id)
        .ok_or_else(|| {
            fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, "数据源引用的连接不存在")
        })?;
    let saved = with_ai_storage(&storage, |conn| {
        save_datasource(
            conn,
            now_ms,
            &input.profile_id,
            input.connection_id,
            policy,
            input.expected_revision,
        )
    })?;
    let item = AiMcpDatasourceItem {
        profile_id: saved.profile_id.clone(),
        connection_id: saved.connection_id,
        connection_name: config.name.clone(),
        resource_kind: if config.kind == fluxdb_core::DatabaseKind::Redis {
            "redis"
        } else {
            "sql"
        }
        .to_string(),
        write_policy: saved.write_policy.as_str().to_string(),
        revision: saved.revision,
    };
    match state
        .ai_mcp_datasources
        .iter_mut()
        .find(|ds| ds.profile_id == item.profile_id && ds.connection_id == item.connection_id)
    {
        Some(slot) => *slot = item,
        None => state.ai_mcp_datasources.push(item),
    }
    Ok((saved.connection_id, saved.revision))
}

/// 删除 MCP 数据源（从对外服务移除该连接）。
pub fn delete_ai_mcp_datasource(
    state: &mut AppState,
    storage: &fluxdb_storage::FileStorage,
    profile_id: &str,
    connection_id: i64,
) -> fluxdb_core::Result<()> {
    with_ai_storage(&storage, |conn| {
        fluxdb_storage::ai_platform::mcp_server::delete_datasource(conn, profile_id, connection_id)
    })?;
    state
        .ai_mcp_datasources
        .retain(|ds| !(ds.profile_id == profile_id && ds.connection_id == connection_id));
    Ok(())
}

include!("ai_model_network.rs");

#[cfg(test)]
mod model_provider_tests {
    use super::*;

    #[test]
    fn every_provider_label_round_trips_to_its_stored_id() {
        for provider in ModelProvider::ALL {
            let label = provider.label();
            assert_eq!(
                ModelProvider::from_label(label),
                Some(provider),
                "下拉展示名必须能唯一解析回 Provider（{label}）"
            );
            assert_eq!(
                ModelProvider::from_stored(provider.stored_id()),
                provider,
                "存储标识必须能解析回 Provider（{label}）"
            );
        }
    }

    #[test]
    fn a_label_is_not_a_stored_id() {
        // 回归：UI 传的是展示名，若按存储标识解析会被兜底成 OpenAI Chat，
        // 用户在下拉里换 Provider 就存不进去。
        let label = ModelProvider::Anthropic.label();

        assert_eq!(ModelProvider::from_label(label), Some(ModelProvider::Anthropic));
        assert_eq!(
            ModelProvider::from_stored(label),
            ModelProvider::OpenAiChat,
            "读取路径的兜底行为保持不变（历史行兼容）"
        );
    }

    #[test]
    fn unknown_label_is_rejected_instead_of_defaulting() {
        assert_eq!(ModelProvider::from_label("不存在的 Provider"), None);
    }
}
