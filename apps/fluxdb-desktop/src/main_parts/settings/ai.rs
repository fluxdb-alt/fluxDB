// 设置页「AI」分区：大模型与工具/MCP（T013/T014）。
//
// 交互以 docs/prototype/ai-agent-ui.html 为基准，控件全部复用 gpui-component；
// UI 只读 AppState 中的公开列表并派发 AppCommand，不接触 SQLite 或任何凭据。

/// 行容器：既有 `settings_row_container` 需要 `&'static str`，AI 行 ID 来自数据库，
/// 因此这里按同样的视觉规范用运行时 ID 复刻一份，不改动既有 helper 签名。
fn ai_row_container(id: &str, colors: UiColors) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("settings-ai-row:{id}")))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_4()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
}

/// AI 设置页的运行时 UI 状态：渲染期间不能自读 `NavicatMain`，由调用方注入。
#[derive(Clone)]
struct AiSettingsPanelState {
    filter: String,
    filter_input: Entity<InputState>,
    testing_id: Option<String>,
    deleting_id: Option<String>,
    settings_loading: bool,
    settings_error: Option<String>,
    /// 「会话历史」页搜索框实体与过滤文本。
    session_filter_input: Entity<InputState>,
    session_filter: String,
    /// 正在归档的会话 id（行内 loading/disabled）。
    session_archiving_id: Option<String>,
    tools_tab: AiToolsTab,
    tool_settings: Vec<fluxdb_app::AiToolSettingItem>,
    /// 本地 MCP 客户端配置状态；不代表进程或会话已连接。
    mcp_install_status: Vec<fluxdb_app::McpInstallClientStatus>,
    mcp_clients_busy: Option<String>,
    mcp_clients_error: Option<String>,
    mcp_connection_details: Option<fluxdb_app::McpConnectionDetails>,
    mcp_connection_error: Option<String>,
    mcp_details_open: bool,
    /// 「安全审计」页签：判定筛选（`None` = 全部）、搜索文本、展开的审计行 id。
    audit_filter: Option<fluxdb_app::AiAuditDecision>,
    audit_filter_input: Entity<InputState>,
    audit_filter_text: String,
    expanded_audit: Option<String>,
    agent_editor: Option<AiAgentEditorState>,
}

/// 通用小胶囊：浅色底 + 彩色文字。
fn ai_pill(
    label: impl Into<SharedString>,
    color: gpui::Rgba,
    colors: UiColors,
) -> impl IntoElement {
    let label = label.into();
    let _ = colors;
    div()
        .px_2()
        .py(px(1.))
        .rounded(px(4.))
        .bg(color.alpha(0.14))
        .text_size(ui_text_size(11.))
        .text_color(color)
        .child(label)
}

/// 凭据状态标签：四态用不同颜色区分，避免「4 态里 3 态同色」。
fn ai_credential_tag(
    status: &fluxdb_app::AiCredentialStatus,
    colors: UiColors,
) -> impl IntoElement {
    use fluxdb_app::AiCredentialStatus as S;
    let color = match status {
        S::Configured => colors.success,
        S::Missing => colors.warning,
        S::NotRequired => colors.muted,
        S::Unavailable => colors.danger,
    };
    let label = match status {
        S::Configured => tr("app.ai.credential.configured"),
        S::Missing => tr("app.ai.credential.missing"),
        S::NotRequired => tr("app.ai.credential.not_required"),
        S::Unavailable => tr("app.ai.credential.unavailable"),
    };
    ai_pill(label, color, colors)
}

/// 生成 / 决策 能力标签。
fn ai_capability_tag(kind: &fluxdb_app::AiModelKind, colors: UiColors) -> impl IntoElement {
    let (label, color) = match kind {
        fluxdb_app::AiModelKind::Decision => (tr("app.ai.capability.decision"), colors.info),
        fluxdb_app::AiModelKind::Generation => (tr("app.ai.capability.generation"), colors.muted),
    };
    ai_pill(label, color, colors)
}

/// 连接状态标签：连接成功(绿) / 连接失败(红) / 未连接(灰)。
fn ai_connection_tag(status: &str, colors: UiColors) -> impl IntoElement {
    let (label, icon, color) = match status {
        "ok" => (tr("app.ai.connection.ok"), AppIcon::Check, colors.success),
        "failed" => (
            tr("app.ai.connection.failed"),
            AppIcon::CircleSlash,
            colors.danger,
        ),
        _ => (tr("app.ai.connection.idle"), AppIcon::Plug, colors.muted),
    };
    div()
        .id("ai-connection-status")
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .child(app_icon(icon, 16., color))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
}

/// AI 设置加载失败块（红色标记 + 重试按钮）。
fn ai_settings_error_block(
    message: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    div()
        .px_4()
        .py_4()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::CircleSlash, 15., colors.danger))
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(colors.danger)
                        .child(format!("{}: {message}", tr("app.ai.error.load_failed"))),
                ),
        )
        .child(
            Button::new("ai-settings-retry")
                .small()
                .ghost()
                .child(tr("app.ai.retry"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reload_ai_settings(cx);
                })),
        )
        .into_any_element()
}

/// 空态行。
fn ai_empty_state(text: &str, colors: UiColors) -> gpui::AnyElement {
    div()
        .px_4()
        .py_6()
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(text.to_string())
        .into_any_element()
}

include!("ai_models.rs");
include!("ai_plugin_text.rs");
include!("ai_tools.rs");
include!("ai_mcp_server.rs");
include!("ai_execution_policy.rs");
include!("ai_mcp_server_actions.rs");
include!("ai_model_dialog.rs");
include!("ai_plugin_config_dialog.rs");
include!("ai_actions.rs");
include!("ai_mcp_clients.rs");
include!("ai_agents.rs");
include!("ai_session_rename.rs");

include!("ai_capacity.rs");
include!("ai_agent_delete.rs");
include!("ai_model_fetch.rs");
