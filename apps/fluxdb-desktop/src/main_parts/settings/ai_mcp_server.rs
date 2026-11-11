// 本地 MCP 管理：单实例控制与对外数据源读写策略。
//
// 边界：UI 只读 AppState 中的公开列表并派发 AppCommand，不接触 SQLite 或凭据；
// 按用户要求不做客户端配置导出。

/// 数据源写入策略的人读标签。
fn ai_mcp_write_policy_label(policy: &str) -> SharedString {
    match policy {
        "read_only" => tr("app.ai.mcp.policy.read_only"),
        "read_write" => tr("app.ai.mcp.policy.read_write"),
        _ => tr("app.ai.mcp.policy.default"),
    }
}

/// 一行 MCP 数据源：连接名 + 写入策略下拉（只读 / 允许读写 / 默认）。
///
/// 选择「默认」表示跟随全局策略；保存就地派发并做对象级 CAS。
fn ai_mcp_datasource_row(
    datasource: &fluxdb_app::AiMcpDatasourceItem,
    global: fluxdb_core::McpWritePolicy,
    colors: UiColors,
    this: WeakEntity<NavicatMain>,
) -> gpui::AnyElement {
    let selected = datasource.write_policy.clone();
    let profile_id = datasource.profile_id.clone();
    let connection_id = datasource.connection_id;
    let revision = datasource.revision;
    let effective = if selected == "read_only" {
        tr("app.ai.mcp.policy.read_only")
    } else if selected == "read_write" {
        tr("app.ai.mcp.policy.read_write")
    } else if global.allows_write() {
        tr("app.ai.mcp.policy.default_read_write")
    } else {
        tr("app.ai.mcp.policy.default_read_only")
    };
    let resource = if datasource.resource_kind == "redis" {
        "Redis".into()
    } else {
        tr("app.ai.mcp.resource.relational")
    };
    div()
        .border_t_1()
        .border_color(colors.border_soft)
        .px_4()
        .py_3()
        .min_h(px(56.))
        .flex()
        .items_center()
        .gap_3()
        .child(app_icon_box(
            if datasource.resource_kind == "redis" {
                AppIcon::List
            } else {
                AppIcon::Table
            },
            26.,
            15.,
            colors.muted,
        ))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(datasource.connection_name.clone()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(tr("app.ai.mcp.current_policy").replace(
                            "%{resource}",
                            &resource,
                        ).replace("%{policy}", &effective)),
                ),
        )
        .child(
            div()
                .w(px(150.))
                .flex()
                .items_center()
                .justify_end()
                .child(
                    Button::new(SharedString::from(format!(
                        "ai-mcp-datasource-policy:{connection_id}"
                    )))
                    .debug_selector({
                        let id = connection_id;
                        move || format!("ai-mcp-datasource-policy:{id}")
                    })
                    .small()
                    .cursor_pointer()
                    .min_w(px(130.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(ai_mcp_write_policy_label(&selected))
                            .child(app_icon(AppIcon::ChevronDown, 12., colors.muted)),
                    )
                    .dropdown_menu_with_anchor(gpui::Anchor::TopRight, {
                        let this = this.clone();
                        let profile_id = profile_id.clone();
                        move |menu, _, _| {
                            let mut menu = menu.min_w(px(180.));
                            for (value, label) in [
                                ("default", tr("app.ai.mcp.policy.default_follow")),
                                ("read_only", tr("app.ai.mcp.policy.read_only")),
                                ("read_write", tr("app.ai.mcp.policy.read_write")),
                            ] {
                                let this = this.clone();
                                let profile_id = profile_id.clone();
                                let option_id =
                                    format!("ai-mcp-datasource-policy-option:{connection_id}:{value}");
                                let selected = selected == value;
                                menu = menu.item(
                                    PopupMenuItem::element(move |_, _| {
                                        div()
                                            .id(SharedString::from(option_id.clone()))
                                            .debug_selector({
                                                let id = option_id.clone();
                                                move || id.clone()
                                            })
                                            .child(label.clone())
                                    })
                                    .checked(selected)
                                    .on_click(move |_, _, cx| {
                                        let profile_id = profile_id.clone();
                                        let value = value.to_string();
                                        let _ = this.clone().update(cx, |view, cx| {
                                            view.save_mcp_datasource_policy(
                                                &profile_id,
                                                connection_id,
                                                value,
                                                revision,
                                                cx,
                                            );
                                        });
                                    }),
                                );
                            }
                            menu
                        }
                    }),
                ),
        )
        .child(
            div()
                .w(px(56.))
                .flex()
                .items_center()
                .justify_end()
                .child(
                    Button::new(SharedString::from(format!(
                        "ai-mcp-datasource-remove:{connection_id}"
                    )))
                    .debug_selector({
                        let id = connection_id;
                        move || format!("ai-mcp-datasource-remove:{id}")
                    })
                    .small()
                    .ghost()
                    .cursor_pointer()
                    .child(tr("app.ai.mcp.remove"))
                    .on_click({
                        let this = this.clone();
                        move |_, _, cx| {
                            let profile_id = profile_id.clone();
                            let _ = this.clone().update(cx, |view, cx| {
                                view.remove_mcp_datasource(&profile_id, connection_id, cx);
                            });
                        }
                    }),
                ),
        )
        .into_any_element()
}

include!("ai_mcp_control.rs");
