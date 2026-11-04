// AI 设置职责拆分；由 ai.rs include! 引入，保持原有作用域。

/// 工具与 MCP 页签。
#[derive(Clone, Copy, PartialEq, Eq)]
enum AiToolsTab {
    Tools,
    Mcp,
    Plugins,
    Audit,
}

use fluxdb_app::{AI_AGENT_TOOL_CATALOG, AI_MCP_ONLY_TOOL_CATALOG};

fn ai_tool_text(id: &str, group: &str, desc: &str) -> (SharedString, SharedString) {
    let group = match group {
        "common" => tr("app.ai.tools.group.common"),
        "sql" => "SQL".into(),
        "redis" => "Redis".into(),
        "mongo" => "MongoDB".into(),
        "relationship" => tr("app.ai.tools.group.relationship"),
        "artifact" => tr("app.ai.tools.group.artifact"),
        _ => group.into(),
    };
    let desc: SharedString = match id {
        "connections.list" => tr("app.ai.tools.desc.connections_list"),
        "editor.search" => tr("app.ai.tools.desc.editor_search"),
        "editor.apply_edit" => tr("app.ai.tools.desc.editor_apply_edit"),
        "editor.read_range" => tr("app.ai.tools.desc.editor_read_range"),
        "page.read_rows" => tr("app.ai.tools.desc.page_read_rows"),
        "page.find_table" => tr("app.ai.tools.desc.page_find_table"),
        "page.browse_objects" => tr("app.ai.tools.desc.page_browse_objects"),
        "page.open_table" => tr("app.ai.tools.desc.page_open_table"),
        "page.query_table" => tr("app.ai.tools.desc.page_query_table"),
        "file.search" => tr("app.ai.tools.desc.file_search"),
        "file.search_content" => tr("app.ai.tools.desc.file_search_content"),
        "file.read" => tr("app.ai.tools.desc.file_read"),
        "context.read" => tr("app.ai.tools.desc.context_read"),
        "docs.search" => tr("app.ai.tools.desc.docs_search"),
        "docs.read" => tr("app.ai.tools.desc.docs_read"),
        "plan.update" => tr("app.ai.tools.desc.plan_update"),

        "toolbox.inspect" => tr("app.ai.tools.desc.toolbox_inspect"),
        "toolbox.search" => tr("app.ai.tools.desc.toolbox_search"),
        "toolbox.load" => tr("app.ai.tools.desc.toolbox_load"),
        "config.propose" => tr("app.ai.tools.desc.config_propose"),
        "db.search_objects" => tr("app.ai.tools.desc.db_search_objects"),
        "db.describe_object" => tr("app.ai.tools.desc.db_describe_object"),
        "db.get_relationships" => tr("app.ai.tools.desc.db_get_relationships"),
        "db.find_join_paths" => tr("app.ai.tools.desc.db_find_join_paths"),
        "er.propose_relationships" => tr("app.ai.tools.desc.er_propose_relationships"),
        "db.run" => tr("app.ai.tools.desc.db_run"),
        "redis.scan" => tr("app.ai.tools.desc.redis_scan"),
        "redis.inspect" => tr("app.ai.tools.desc.redis_inspect"),
        "redis.execute" => tr("app.ai.tools.desc.redis_execute"),
        "redis.subscribe" => tr("app.ai.tools.desc.redis_subscribe"),
        "redis.server_info" => tr("app.ai.tools.desc.redis_server_info"),
        "mongo.read_documents" => tr("app.ai.tools.desc.mongo_read_documents"),
        "result.read_page" => tr("app.ai.tools.desc.result_read_page"),
        "diagnostics.read_turns" => tr("app.ai.tools.desc.diagnostics_read_turns"),
        "artifact.create" => tr("app.ai.tools.desc.artifact_create"),
        "artifact.add_block" => tr("app.ai.tools.desc.artifact_add_block"),
        "artifact.finalize" => tr("app.ai.tools.desc.artifact_finalize"),
        "operation.status" => tr("app.ai.tools.desc.operation_status"),
        _ => desc.into(),
    };
    (group, desc)
}

/// 页签按钮：选中态下边框高亮。
fn ai_tools_tab_button(
    label: impl Into<SharedString>,
    value: AiToolsTab,
    active: AiToolsTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let label = label.into();
    let debug_label = label.clone();
    let is_active = value == active;
    div()
        .id(SharedString::from(format!("ai-tools-tab:{label}")))
        .debug_selector(move || format!("ai-tools-tab:{debug_label}"))
        .px_3()
        .pt_1()
        .pb_2()
        .cursor_pointer()
        .border_b_2()
        .border_color(if is_active {
            colors.info
        } else {
            colors.info.alpha(0.0)
        })
        .text_size(px(12.))
        .text_color(if is_active { colors.text } else { colors.muted })
        .font_weight(if is_active {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .hover(move |style| style.text_color(colors.text))
        .child(label.clone())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.ai_tools_tab = value;
                if matches!(value, AiToolsTab::Mcp) {
                    this.refresh_mcp_install_status(cx);
                    // 默认 Profile 缺失时就地补建，页面无需额外的加载入口。
                    this.ensure_mcp_profile(cx);
                }
                cx.notify();
                cx.stop_propagation();
            }),
        )
}

/// 页签条。
fn ai_tools_tab_bar(active: AiToolsTab, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .border_b_1()
        .border_color(colors.border_soft)
        .flex()
        .items_center()
        .gap_1()
        .child(ai_tools_tab_button(
            tr("app.ai.tools.tab.tools"),
            AiToolsTab::Tools,
            active,
            colors,
            cx,
        ))
        .child(ai_tools_tab_button(
            tr("app.ai.tools.tab.mcp"),
            AiToolsTab::Mcp,
            active,
            colors,
            cx,
        ))
        .child(ai_tools_tab_button(
            tr("app.ai.tools.tab.plugins"),
            AiToolsTab::Plugins,
            active,
            colors,
            cx,
        ))
        .child(ai_tools_tab_button(
            tr("app.ai.tools.tab.audit"),
            AiToolsTab::Audit,
            active,
            colors,
            cx,
        ))
        .child(div().flex_1())
        .child(
            Button::new("ai-refresh-tools")
                .debug_selector(|| "ai-refresh-tools".into())
                .small()
                .ghost()
                .child(tr("app.common.refresh"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reload_ai_settings(cx);
                })),
        )
}

/// 表头单元（固定宽）。
fn ai_table_head(label: impl Into<SharedString>, w: f32, colors: UiColors) -> Div {
    let label = label.into();
    div()
        .w(px(w))
        .text_size(px(11.))
        .text_color(colors.muted)
        .child(label)
}

/// 「工具」页签：标准工具表。
fn ai_tools_catalog_panel(
    state: &AppState,
    panel: &AiSettingsPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let header = div()
        .border_t_1()
        .border_color(colors.border_soft)
        .px_4()
        .py_2()
        .flex()
        .items_center()
        .gap_3()
        .child(ai_table_head(tr("app.ai.tools.header.tool"), 210., colors))
        .child(ai_table_head(tr("app.ai.tools.header.purpose"), 320., colors))
        .child(ai_table_head(tr("app.ai.tools.header.risk"), 120., colors))
        .child(ai_table_head(tr("app.ai.tools.header.group"), 80., colors))
        .child(ai_table_head(tr("app.ai.tools.header.enabled"), 60., colors))
        .child(div().flex_1());
    let mut rows = div().flex().flex_col();
    for (id, group, desc, write) in AI_AGENT_TOOL_CATALOG.iter().chain(AI_MCP_ONLY_TOOL_CATALOG) {
        let (group_label, desc) = ai_tool_text(id, group, desc);
        // 没有行 = 从未被显式关闭，按默认启用显示。
        let setting = panel.tool_settings.iter().find(|t| t.tool_id == *id);
        let enabled = setting.map(|t| t.enabled).unwrap_or(true);
        let revision = setting.map(|t| t.revision).unwrap_or(0);
        rows = rows.child(ai_tool_row(
            id,
            group_label,
            desc,
            *group == "artifact",
            *write,
            enabled,
            revision,
            colors,
            cx,
        ));
    }
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            ai_execution_policy_panel(state,colors,cx),
        )
        .child(
            settings_panel_group(tr("app.ai.tools.catalog_title"), colors)
                .child(
                    div()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(
                                    tr("app.ai.tools.catalog_count")
                                        .replace(
                                            "%{count}",
                                            &(AI_AGENT_TOOL_CATALOG.len()
                                                + AI_MCP_ONLY_TOOL_CATALOG.len())
                                            .to_string(),
                                        ),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_4()
                        .pb_3()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child({
                            let status = if state.ai_plugin_instances.iter().any(|plugin| {
                                plugin.id == fluxdb_app::BI_INSTANCE_ID && plugin.enabled
                            }) {
                                tr("app.ai.status.enabled").to_string()
                            } else {
                                tr("app.ai.status.disabled").to_string()
                            };
                            tr("app.ai.tools.artifact_note").replace("%{status}", &status)
                        }),
                )
                // 窄窗口：工具表保持最小宽度并横向滚动，避免列被压扁/裁切。
                .child(
                    div()
                        .w_full()
                        .overflow_x_scrollbar()
                        .child(
                            div()
                                .min_w(px(760.))
                                .flex()
                                .flex_col()
                                .child(header)
                                .child(rows),
                        ),
                ),
        )
}

/// 单个工具行。
#[allow(clippy::too_many_arguments)]
fn ai_tool_row(
    id: &str,
    group: SharedString,
    desc: SharedString,
    artifact: bool,
    write: bool,
    enabled: bool,
    revision: i64,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let tool_id_for_switch = id.to_string();
    div()
        .border_t_1()
        .border_color(colors.border_soft)
        // 与表头同内边距，保证工具名列不贴面板左边缘。
        .px_4()
        .py_2()
        .min_h(px(46.))
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(210.))
                .text_size(px(12.))
                .text_color(colors.text)
                .child(id.to_string()),
        )
        .child(
            div()
                .w(px(320.))
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(desc),
        )
        .child(div().w(px(120.)).flex().items_start().child(if artifact {
            ai_pill(tr("app.ai.tools.risk.artifact"), colors.info, colors).into_any_element()
        } else if matches!(id, "page.find_table" | "page.browse_objects" | "page.open_table" | "page.query_table") {
            ai_pill(tr("app.ai.page_query.risk"), colors.info, colors).into_any_element()
        } else if id == "editor.apply_edit" {
            ai_pill(tr("app.ai.sql_edit.local_edit"), colors.info, colors).into_any_element()
        } else if id == "toolbox.load" || id == "config.propose" {
            ai_pill(tr("app.ai.tools.risk.proposal"), colors.info, colors).into_any_element()
        } else if write {
            ai_pill(tr("app.ai.tools.risk.write"), colors.warning, colors).into_any_element()
        } else {
            ai_pill(tr("app.ai.tools.risk.read"), colors.success, colors).into_any_element()
        }))
        .child(
            div()
                .w(px(80.))
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(group),
        )
        .child(
            div()
                .w(px(60.))
                .child(div().debug_selector(|| format!("ai-tool:{id}")).child(
                    Switch::new(SharedString::from(format!("ai-tool:{id}")))
                        .checked(enabled)
                        .on_click(cx.listener(move |this, checked, _, cx| {
                            // 全局启停经命令落库；授权时与 Agent 白名单取交集。
                            this.set_ai_tool_enabled(
                                tool_id_for_switch.clone(),
                                *checked,
                                revision,
                                cx,
                            );
                        })),
                )),
        )
        .child(div().flex_1())
}

/// 内置能力的图标：按 package_key 归类，未知实例用通用插件图标。
fn ai_builtin_icon(package_key: &str) -> AppIcon {
    match package_key {
        "fluxdb.context.ui" => AppIcon::Text,
        "fluxdb.context.files" => AppIcon::FileSearch,
        "fluxdb.model.router" => AppIcon::Workflow,
        "fluxdb.decision.jev" => AppIcon::Activity,
        "fluxdb.context.compaction" => AppIcon::Filter,
        "fluxdb.security.audit" => AppIcon::Check,
        "fluxdb.usage.budget" => AppIcon::CalendarClock,
        "fluxdb.output.bi" => AppIcon::Table,
        "fluxdb.progress.plan" => AppIcon::List,
        _ => AppIcon::Plug,
    }
}

/// 按内置目录顺序（设计 §23.2）展示能力；未知实例排末尾，不隐藏。
///
/// 数据库按名称排序，中文名会把"BI 与报表输出"排到最前，与目录语义不符，
/// 因此展示顺序由这里统一决定，不改存储层排序。
fn ai_plugins_in_catalog_order(
    items: &[fluxdb_app::AiPluginItem],
) -> Vec<&fluxdb_app::AiPluginItem> {
    let mut rows: Vec<&fluxdb_app::AiPluginItem> = items.iter().collect();
    rows.sort_by_key(|p| {
        (
            fluxdb_app::builtin_catalog_order(&p.package_key).unwrap_or(usize::MAX),
            p.id.clone(),
        )
    });
    rows
}

/// 「内置插件」页签：能力说明、启停状态，以及常驻能力/待实现标签。
fn ai_plugins_panel(state: &AppState, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    let body: Vec<gpui::AnyElement> = if state.ai_plugin_instances.is_empty() {
        vec![ai_empty_state(
            &tr("app.ai.tools.plugins_empty"),
            colors,
        )]
    } else {
        ai_plugins_in_catalog_order(&state.ai_plugin_instances)
            .into_iter()
            .map(|p| {
                let this = cx.entity().downgrade();
                let id = p.id.clone();
                let manage_id = p.id.clone();
                let enabled = p.enabled;
                let revision = p.revision;
                let mandatory = p.mandatory;
                let retired = p.retired;
                // 「管理」入口由插件自己的配置声明决定：没有可配置项就不显示，
                // 也不按插件 id 特判。
                let has_config = !p.config_fields.is_empty();
                let config_error = p.config_error.clone();
                // Jev 有独立的装配/熔断状态，不复用通用启停标签。
                let is_jev = p.package_key == "fluxdb.decision.jev";
                let jev_status = state.ai_jev_status.clone();
                // 管理入口与插件行共用同一个 owner，避免在 `when` 闭包里被行元素遮蔽。
                let manage_owner = this.clone();
                let manage_selector = format!("ai-plugin-manage:{manage_id}");
                let manage_click_id = manage_id.clone();
                let manage_button = Button::new(SharedString::from(manage_selector.clone()))
                    .debug_selector({
                        let selector = manage_selector.clone();
                        move || selector.clone()
                    })
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .child(tr("app.ai.tools.manage"))
                    .on_click(move |_, window, cx| {
                        let _ = manage_owner.update(cx, |this, cx| {
                            this.open_ai_plugin_config_dialog(&manage_click_id, window, cx);
                        });
                    });
                ai_row_container(&p.id, colors)
                    .min_h(px(52.))
                    .py_2()
                    // 无实现的占位行整行置灰：只占目录位，不给出可操作的启停状态。
                    .when(retired, |this| this.opacity(0.45))
                    .child(app_icon_box(
                        ai_builtin_icon(&p.package_key),
                        28.,
                        15.,
                        colors.muted,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if retired { colors.muted } else { colors.text })
                                    .child(localize_ai_plugin_text(&p.display_name)),
                            )
                            .when(!p.summary.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(colors.muted)
                                        .child(localize_ai_plugin_text(&p.summary)),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(colors.muted)
                                            .child(tr("app.ai.tools.builtin").replace(
                                                "%{package}",
                                                &p.package_key,
                                            )),
                                    )
                                    .when(retired, |this| {
                                        this.child(ai_pill(
                                            tr("app.ai.tools.unimplemented"),
                                            colors.muted,
                                            colors,
                                        ))
                                    })
                                    .when(mandatory, |this| {
                                        this.child(ai_pill(
                                            tr("app.ai.tools.mandatory"),
                                            colors.info,
                                            colors,
                                        ))
                                    })
                                    .when(!mandatory && !retired, |this| {
                                        this.child(ai_status_pill(enabled, colors))
                                    })
                                    .when(is_jev, |this| {
                                        this.child(ai_jev_status_pill(&jev_status, colors))
                                    })
                                    .when_some(config_error.clone(), |this, message| {
                                        this.child(ai_pill(
                                            tr("app.ai.tools.config_unreadable"),
                                            colors.warning,
                                            colors,
                                        ))
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(colors.warning)
                                                    .child(message),
                                            )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(has_config, |this| {
                                this.child(manage_button)
                            })
                            .child(div().debug_selector(|| format!("ai-plugin:{id}")).child(
                                Switch::new(SharedString::from(format!("ai-plugin:{id}")))
                                    .checked(enabled)
                                    .disabled(mandatory || retired)
                                    .on_click(move |checked, _, cx| {
                                        let _ = this.update(cx, |this, cx| {
                                            let _ = this.controller.dispatch(
                                                AppCommand::SetAiPluginInstanceEnabled {
                                                    id: id.clone(),
                                                    enabled: *checked,
                                                    expected_revision: revision,
                                                },
                                            );
                                            cx.notify();
                                        });
                                    }),
                            )),
                    )
                    .into_any_element()
            })
            .collect()
    };
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            settings_panel_group_el(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(tr("app.ai.tools.builtin_title")),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(
                                tr("app.ai.tools.item_count").replace(
                                    "%{count}",
                                    &state.ai_plugin_instances.len().to_string(),
                                ),
                            ),
                    ),
            )
            .child(div().children(body)),
        )
}

/// 启停状态标签：可选能力显示真实启停，不显示成"一排错误"。
fn ai_status_pill(enabled: bool, colors: UiColors) -> impl IntoElement {
    if enabled {
        ai_pill(tr("app.ai.status.enabled"), colors.success, colors)
    } else {
        ai_pill(tr("app.ai.status.disabled"), colors.muted, colors)
    }
}

/// Jev 状态标签（设计 §18 的状态表）：状态而不是错误条。
///
/// Jev 是内置可选能力，永远不是常驻能力，开关始终可用。
fn ai_jev_status_pill(status: &fluxdb_app::JevStatus, colors: UiColors) -> impl IntoElement {
    use fluxdb_app::JevStatusKind as K;
    let (label, color) = match status.kind {
        K::Available => (tr("app.ai.jev.available"), colors.success),
        K::NeedsChoice => (tr("app.ai.jev.needs_choice"), colors.warning),
        K::Unavailable => (tr("app.ai.jev.unavailable"), colors.warning),
        K::NotConfigured => (tr("app.ai.jev.not_configured"), colors.muted),
        K::Disabled => (tr("app.ai.jev.disabled"), colors.muted),
    };
    let text = match (status.detail.clone(), status.circuit_until_ms) {
        (Some(reason), _) => format!("{label}: {}", localize_ai_plugin_text(&reason)),
        (None, Some(until)) => format!(
            "{}: {until}",
            tr("app.ai.jev.circuit_until").replace("%{label}", &label)
        ),
        (None, None) => label.to_string(),
    };
    div().id("ai-jev-status").child(ai_pill(label.clone(), color, colors)).tooltip(move |window, cx| {
        Tooltip::new(text.clone()).build(window, cx)
    })
}

/// 「安全审计」页签：只读记录。
///
/// 结构与设置页其余页签一致：先给"审计始终开启"的不可变事实，再给统计筛选与列表。
/// 列表来自 `AppState.ai_audit_rows`（读库失败时为 `ai_audit_error`，与空态分开显示）。
/// 行可展开查看最终计划、命中规则、plan hash；本页不提供任何执行/修改操作。
fn ai_audit_panel(state: &AppState, panel: &AiSettingsPanelState, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    let rows = &state.ai_audit_rows;
    // 统计基于当前快照（不单独查库），与列表同源，避免数字与行数不一致。
    let counts = fluxdb_app::AiAuditCounts {
        all: rows.len(),
        allow: rows.iter().filter(|r| r.decision == fluxdb_app::AiAuditDecision::Allow).count(),
        approval: rows.iter().filter(|r| r.decision == fluxdb_app::AiAuditDecision::RequireApproval).count(),
        deny: rows.iter().filter(|r| r.decision == fluxdb_app::AiAuditDecision::Deny).count(),
    };
    let filter = panel.audit_filter;
    let query = panel.audit_filter_text.trim().to_ascii_lowercase();
    let visible: Vec<&fluxdb_app::AiAuditRow> = rows
        .iter()
        .filter(|row| filter.is_none() || filter == Some(row.decision))
        .filter(|row| {
            query.is_empty()
                || row.tool_id.to_ascii_lowercase().contains(&query)
                || row.target.to_ascii_lowercase().contains(&query)
                || row.rule_ids.iter().any(|id| id.to_ascii_lowercase().contains(&query))
        })
        .collect();

    let mut panel_body = div().flex().flex_col().gap_3().child(
        settings_panel_group(tr("app.ai.audit.title"), colors).child(
            div()
                .px_4()
                .py_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(app_icon(AppIcon::Check, 14., colors.success))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(tr("app.ai.audit.always_on")),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(tr("app.ai.audit.description")),
                ),
        ),
    );

    // 统计卡即筛选器：点一张筛一类，再点同一张回到全部。
    let mut stats = div().flex().items_center().gap_2();
    for (decision, label, count) in [
        (None, tr("app.ai.audit.filter.all"), counts.all),
        (
            Some(fluxdb_app::AiAuditDecision::Allow),
            tr("app.ai.audit.filter.allow"),
            counts.allow,
        ),
        (
            Some(fluxdb_app::AiAuditDecision::RequireApproval),
            tr("app.ai.audit.filter.approval"),
            counts.approval,
        ),
        (
            Some(fluxdb_app::AiAuditDecision::Deny),
            tr("app.ai.audit.filter.deny"),
            counts.deny,
        ),
    ] {
        stats = stats.child(ai_audit_stat_card(
            decision,
            label,
            count,
            filter == decision,
            colors,
            cx,
        ));
    }
    panel_body = panel_body.child(stats);

    // 搜索与筛选只作用于当前快照；清空筛选按钮在有筛选时可见。
    let mut toolbar = div().flex().items_center().gap_2().child(ai_session_search_field(
        &panel.audit_filter_input,
        colors,
        cx,
    ));
    if filter.is_some() || !query.is_empty() {
        toolbar = toolbar.child(
            Button::new("ai-audit-clear-filter")
                .debug_selector(|| "ai-audit-clear-filter".into())
                .small()
                .ghost()
                .cursor_pointer()
                .child(tr("app.ai.audit.clear_filter"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.clear_ai_audit_filter(window, cx)
                })),
        );
    }
    toolbar = toolbar.child(div().flex_1()).child(
        div()
            .text_size(px(11.))
            .text_color(colors.muted)
            .child(
                tr("app.ai.audit.retention")
                    .replace("%{count}", &fluxdb_app::AI_AUDIT_PAGE_SIZE.to_string()),
            ),
    );
    panel_body = panel_body.child(toolbar);

    // 错误态优先于空态：读库失败不能显示成"暂无记录"。
    let list_body = if let Some(error) = state.ai_audit_error.clone() {
        ai_audit_error_state(&error, colors)
    } else if visible.is_empty() {
        {
            let text = if rows.is_empty() {
                tr("app.ai.audit.empty").to_string()
            } else {
                tr("app.ai.audit.no_match").to_string()
            };
            ai_empty_state(&text, colors)
        }
    } else {
        let mut list = div().flex().flex_col();
        for row in &visible {
            list = list.child(ai_audit_row(row, panel, colors, cx));
        }
        list.into_any_element()
    };

    panel_body.child(
        settings_panel_group(tr("app.ai.audit.records"), colors)
            .child(
                div()
                    .border_t_1()
                    .border_color(colors.border_soft)
                    .px_4()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(ai_table_head(tr("app.ai.audit.header.source"), 240., colors))
                    .child(ai_table_head(tr("app.ai.audit.header.target"), 190., colors))
                    .child(ai_table_head(tr("app.ai.audit.header.decision"), 90., colors))
                    .child(ai_table_head(tr("app.ai.audit.header.rules"), 210., colors))
                    .child(div().flex_1()),
            )
            .child(list_body),
    )
}

/// 一张统计卡：既是数字也是筛选入口。
fn ai_audit_stat_card(
    decision: Option<fluxdb_app::AiAuditDecision>,
    label: impl Into<SharedString>,
    count: usize,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let label = label.into();
    let selector = format!(
        "ai-audit-stat:{}",
        decision.map(fluxdb_app::AiAuditDecision::key).unwrap_or("all")
    );
    let dot = decision.map(|d| match d {
        fluxdb_app::AiAuditDecision::Allow => colors.success,
        fluxdb_app::AiAuditDecision::RequireApproval => colors.warning,
        fluxdb_app::AiAuditDecision::Deny => colors.danger,
    });
    // 自绘卡片而非 Button：组件 Button 的行高被 small() 压扁，撑不开"标题+数字"两行；
    // 这里保留同样的点击/筛选语义（点击即筛选，再点取消）。
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector({
            let selector = selector.clone();
            move || selector.clone()
        })
        .cursor_pointer()
        .min_w(px(132.))
        .px_3()
        .py_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(if selected { colors.info } else { colors.border })
        .bg(colors.panel_bg)
        .hover(|style| style.bg(colors.hover))
        .flex()
        .flex_col()
        .items_start()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(if selected { colors.text } else { colors.muted })
                        .child(label),
                )
                .when_some(dot, |this, color| {
                    this.child(div().size(px(6.)).rounded_full().bg(color))
                }),
        )
        .child(
            div()
                .text_size(px(18.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(if selected { colors.text } else { colors.muted })
                .child(count.to_string()),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_ai_audit_filter(decision, cx);
        }))
        .into_any_element()
}

/// 审计行：摘要行 + 可展开详情。
fn ai_audit_row(
    row: &fluxdb_app::AiAuditRow,
    panel: &AiSettingsPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let expanded = panel.expanded_audit.as_deref() == Some(row.id.as_str());
    let id = row.id.clone();
    let click_id = row.id.clone();
    let (tag_label, tag_color) = ai_audit_decision_tag(row.decision, colors);
    let row_selector = format!("ai-audit-row:{}", row.id);
    let mut summary = div()
        .id(SharedString::from(row_selector.clone()))
        .debug_selector({
            let row_selector = row_selector.clone();
            move || row_selector.clone()
        })
        .border_t_1()
        .border_color(colors.border_soft)
        .px_4()
        .py_2()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .child(
            div()
                .w(px(240.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(row.tool_id.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("{} · {}", row.origin, row.id)),
                ),
        )
        .child(div().w(px(190.)).text_size(px(12.)).text_color(colors.text).child(row.target.clone()))
        .child(div().w(px(90.)).child(ai_pill(tag_label, tag_color, colors)))
        .child(
            div()
                .w(px(210.))
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(ai_audit_rule_label(row)),
        )
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(if expanded {
                    tr("app.ai.audit.collapse")
                } else {
                    tr("app.ai.audit.details")
                }),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_ai_audit_detail(&click_id, cx);
        }));
    let _ = id;
    if expanded {
        summary = summary.bg(colors.border_soft);
    }
    let mut container = div().flex().flex_col().child(summary);
    if expanded {
        container = container.child(ai_audit_detail(row, colors));
    }
    container.into_any_element()
}

/// 命中规则的展示：机器键直接显示，未命中显示占位符。
fn ai_audit_rule_label(row: &fluxdb_app::AiAuditRow) -> String {
    if row.rule_ids.is_empty() {
        "—".to_string()
    } else {
        row.rule_ids.join(" · ")
    }
}

/// 展开详情：最终计划、命中规则与原因码、plan hash 与规则集版本，以及不泄漏秘密的说明。
fn ai_audit_detail(row: &fluxdb_app::AiAuditRow, colors: UiColors) -> Div {
    let mut detail = div()
        .px_4()
        .pb_3()
        .flex()
        .flex_col()
        .gap_2()
        .bg(colors.border_soft);
    let line = |key: SharedString, value: gpui::AnyElement| {
        div()
            .flex()
            .items_start()
            .gap_3()
            .child(
                div()
                    .w(px(88.))
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(key),
            )
            .child(value)
    };
    if !row.statement.is_empty() {
        detail = detail.child(line(
            tr("app.ai.audit.detail.final_plan"),
            div()
                .flex_1()
                .min_w(px(0.))
                .px_3()
                .py_2()
                .rounded(colors.radius)
                .bg(colors.panel_bg)
                .border_1()
                .border_color(colors.border_soft)
                .text_size(px(11.))
                .text_color(colors.text)
                .child(row.statement.clone())
                .into_any_element(),
        ));
    }
    detail = detail.child(line(
        tr("app.ai.audit.detail.matched_rules"),
        div()
            .flex_1()
            .min_w(px(0.))
            .text_size(px(11.))
            .text_color(colors.text)
            .child(if row.reason_codes.is_empty() {
                ai_audit_rule_label(row)
            } else {
                format!("{} · {}", ai_audit_rule_label(row), row.reason_codes.join("、"))
            })
            .into_any_element(),
    ));
    detail = detail.child(line(
        tr("app.ai.audit.detail.plan_hash"),
        div()
            .flex_1()
            .min_w(px(0.))
            .text_size(px(11.))
            .text_color(colors.muted)
            .child(format!(
                "{} · {} · {} {}",
                row.plan_hash,
                row.stage,
                tr("app.ai.audit.ruleset"),
                row.ruleset_version
            ))
            .into_any_element(),
    ));
    detail.child(line(
        tr("app.ai.audit.detail.note"),
        div()
            .flex_1()
            .min_w(px(0.))
            .text_size(px(11.))
            .text_color(colors.muted)
            .child(tr("app.ai.audit.detail.note_text"))
            .into_any_element(),
    ))
}

/// 判定标签：颜色与工具卡的允许/需审批/拒绝一致（绿/黄/红）。
fn ai_audit_decision_tag(
    decision: fluxdb_app::AiAuditDecision,
    colors: UiColors,
) -> (SharedString, gpui::Rgba) {
    match decision {
        fluxdb_app::AiAuditDecision::Allow => {
            (tr("app.ai.audit.filter.allow"), colors.success)
        }
        fluxdb_app::AiAuditDecision::RequireApproval => {
            (tr("app.ai.audit.filter.approval"), colors.warning)
        }
        fluxdb_app::AiAuditDecision::Deny => (tr("app.ai.audit.filter.deny"), colors.danger),
    }
}

/// 审计列表读库失败：给出可见原因，不伪装成空态。
fn ai_audit_error_state(message: &str, colors: UiColors) -> gpui::AnyElement {
    div()
        .px_4()
        .py_6()
        .flex()
        .items_center()
        .gap_2()
        .child(app_icon(AppIcon::CircleSlash, 14., colors.danger))
        .child(
            div()
                .debug_selector(|| "ai-audit-error".into())
                .text_size(px(12.))
                .text_color(colors.danger)
                .child(format!(
                    "{}: {message}",
                    tr("app.ai.audit.load_failed")
                )),
        )
        .into_any_element()
}

/// 「工具与 MCP」分区：四页签。
fn settings_ai_tools_mcp_panel(
    state: &AppState,
    panel: &AiSettingsPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let body = match panel.tools_tab {
        AiToolsTab::Tools => ai_tools_catalog_panel(state,panel, colors, cx),
        AiToolsTab::Mcp => ai_mcp_panel(state, panel, colors, cx),
        AiToolsTab::Plugins => ai_plugins_panel(state, colors, cx),
        AiToolsTab::Audit => ai_audit_panel(state, panel, colors, cx),
    };
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(ai_tools_tab_bar(panel.tools_tab, colors, cx))
        .child(body)
}

// ---- 模型新建/编辑弹框 ----

