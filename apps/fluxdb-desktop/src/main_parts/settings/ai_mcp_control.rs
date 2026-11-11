fn ai_mcp_control_panel(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> GroupBox {
    // 只有默认 Profile 一个对象，页面没有可编辑草稿：开关直接落到已存配置上。
    let enabled = default_mcp_profile(state).is_some_and(|profile| profile.enabled);
    let enabled_control = Switch::new("ai-mcp-profile-enabled")
        .checked(enabled)
        .on_click(cx.listener(|this, checked, _, cx| this.set_mcp_profile_enabled(*checked, cx)))
        .into_any_element();
    settings_panel_group(tr("app.ai.mcp.control_title"), colors)
        .child(ai_policy_row(
            &tr("app.ai.mcp.enable_server"),
            &tr("app.ai.mcp.enable_server_detail"),
            enabled_control,
            colors,
        ))
        .child(ai_policy_row(
            &tr("app.ai.mcp.default_policy"),
            &tr("app.ai.mcp.default_policy_detail"),
            ai_mcp_global_policy_button(state, colors, cx),
            colors,
        ))
}

/// 当前默认 Profile；缺失表示还没初始化（进入页签时 `EnsureMcpControl` 会补建）。
fn default_mcp_profile(state: &AppState) -> Option<&fluxdb_app::AiMcpServerProfileItem> {
    state
        .ai_mcp_server_profiles
        .iter()
        .find(|profile| profile.id == fluxdb_app::DEFAULT_MCP_PROFILE_ID)
}

/// 全局默认写入策略下拉：只读 / 允许读写。
fn ai_mcp_global_policy_button(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let selected = state.ai_execution_policy.mcp_default;
    let owner = cx.entity().downgrade();
    Button::new("ai-mcp-global-policy")
        .debug_selector(|| "ai-mcp-global-policy".into())
        .small()
        .cursor_pointer()
        .min_w(px(150.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(mcp_write_policy_label(selected))
                .child(app_icon(AppIcon::ChevronDown, 12., colors.muted)),
        )
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            let mut menu = menu.min_w(px(180.));
            for mode in fluxdb_core::McpWritePolicy::ALL {
                let owner = owner.clone();
                let option_id = format!("ai-mcp-global-policy-option:{mode:?}");
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id(SharedString::from(option_id.clone()))
                            .debug_selector({
                                let id = option_id.clone();
                                move || id.clone()
                            })
                            .child(mcp_write_policy_label(mode))
                    })
                    .checked(selected == mode)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.save_mcp_global_policy(mode, cx)
                        });
                    }),
                );
            }
            menu
        })
        .into_any_element()
}

/// 数据源面板：列出已加入对外服务的连接，每行一个写入策略选择器 + 新增入口。
fn ai_mcp_datasources_panel(
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> GroupBox {
    let global = state.ai_execution_policy.mcp_default;
    let datasources = state
        .ai_mcp_datasources
        .iter()
        .filter(|d| d.profile_id == fluxdb_app::DEFAULT_MCP_PROFILE_ID)
        .collect::<Vec<_>>();
    let mut panel = settings_panel_group(tr("app.ai.mcp.datasources"), colors).child(
        div()
            .px_4()
            .py_2()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(colors.muted)
                    .child(tr("app.ai.mcp.datasource_help")),
            )
            .child(ai_mcp_datasource_add_button(
                state,
                fluxdb_app::DEFAULT_MCP_PROFILE_ID,
                colors,
                cx,
            )),
    );
    if datasources.is_empty() {
        panel = panel.child(
            div()
                .px_4()
                .py_3()
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.ai.mcp.no_datasource")),
        );
    }
    for datasource in datasources {
        panel = panel.child(ai_mcp_datasource_row(
            datasource,
            global,
            colors,
            cx.entity().downgrade(),
        ));
    }
    panel
}

/// 「新增数据源」下拉：列出尚未加入的连接，点选即以「默认」策略加入。
///
/// 用下拉而非弹框，和页面其它策略选择器一致；没有候选时按钮禁用并给出提示。
fn ai_mcp_datasource_add_button(
    state: &AppState,
    profile_id: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> gpui::AnyElement {
    let existing: std::collections::HashSet<i64> = state
        .ai_mcp_datasources
        .iter()
        .filter(|d| d.profile_id == profile_id)
        .map(|d| d.connection_id)
        .collect();
    let candidates: Vec<(i64, String)> = state
        .connections
        .iter()
        .filter(|c| !existing.contains(&(c.config.id.0 as i64)))
        .map(|c| (c.config.id.0 as i64, c.config.name.clone()))
        .collect();
    let has_candidates = !candidates.is_empty();
    let owner = cx.entity().downgrade();
    let profile_id = profile_id.to_string();
    Button::new("ai-mcp-datasource-new")
        .debug_selector(|| "ai-mcp-datasource-new".into())
        .small()
        .ghost()
        .cursor_pointer()
        .disabled(!has_candidates)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Plus, 12., colors.muted))
                .child(tr("app.ai.mcp.add_datasource")),
        )
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| {
            if candidates.is_empty() {
                return menu.item(
                    PopupMenuItem::element(|_, _| {
                        div()
                            .px_3()
                            .py_2()
                            .text_size(ui_text_size(11.))
                            .child(tr("app.ai.mcp.no_candidate"))
                    })
                    .disabled(true),
                );
            }
            let mut menu = menu.min_w(px(240.));
            for (id, name) in &candidates {
                let id = *id;
                let owner = owner.clone();
                let profile_id = profile_id.clone();
                let label = name.clone();
                let option_id = format!("ai-mcp-datasource-add:{id}");
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .id(SharedString::from(option_id.clone()))
                            .debug_selector({
                                let id = option_id.clone();
                                move || id.clone()
                            })
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(label.clone())
                    })
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.save_mcp_datasource_policy(
                                &profile_id,
                                id,
                                "default".to_string(),
                                0,
                                cx,
                            );
                        });
                    }),
                );
            }
            menu
        })
        .into_any_element()
}

fn mcp_write_policy_label(policy: fluxdb_core::McpWritePolicy) -> SharedString {
    match policy {
        fluxdb_core::McpWritePolicy::ReadOnly => tr("app.ai.mcp.policy.read_only"),
        fluxdb_core::McpWritePolicy::ReadWrite => tr("app.ai.mcp.policy.read_write"),
    }
}

fn ai_mcp_panel(
    state: &AppState,
    panel: &AiSettingsPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(ai_mcp_control_panel(state, colors, cx))
        .child(ai_mcp_datasources_panel(state, colors, cx))
        .child(ai_mcp_clients_panel(panel, colors, cx))
}
