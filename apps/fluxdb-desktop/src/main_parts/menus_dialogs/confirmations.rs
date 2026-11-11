fn delete_connection_modal(
    connection_id: ConnectionId,
    state: &AppState,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let connection_name = state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
        .map(|connection| connection.config.name.clone())
        .unwrap_or_else(|| tr("app.confirm.unknown_connection").to_string());

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .w(px(420.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("DeleteConnectionModal")
                .on_action(cx.listener(|this, _: &DeleteConnectionShortcut, _, cx| {
                    this.confirm_delete_connection(cx);
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(|this, _: &CancelDeleteConnection, _, cx| {
                    this.cancel_delete_connection(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.confirm.delete_connection_title")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_delete_connection(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.confirm.delete_connection_message")
                                .replace("%{name}", &connection_name),
                        ),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("delete-connection-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_delete_connection(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("delete-connection-confirm")
                                .label(tr("app.confirm.delete_connection_title"))
                                .danger()
                                .w(px(96.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_delete_connection(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn delete_database_modal(
    pending: PendingDeleteDatabase,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_delete_database(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(420.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("DeleteDatabaseModal")
                .on_action(cx.listener(|this, _: &DeleteConnectionShortcut, _, cx| {
                    this.confirm_delete_database(cx);
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    // Esc 关闭（全局 escape→CancelDialog 绑定）
                    this.cancel_delete_database(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.confirm.delete_database_title")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_delete_database(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.confirm.delete_database_message")
                                .replace("%{name}", &pending.database),
                        ),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("delete-database-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_delete_database(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("delete-database-confirm")
                                .label(tr("app.confirm.delete_database_title"))
                                .danger()
                                .w(px(104.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_delete_database(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn disconnect_connection_modal(
    pending: PendingDisconnectConnection,
    state: &AppState,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let connection_name = state
        .connections
        .iter()
        .find(|connection| connection.config.id == pending.connection_id)
        .map(|connection| connection.config.name.clone())
        .unwrap_or_else(|| tr("app.confirm.unknown_connection").to_string());
    let mut details = Vec::new();
    if pending.unsaved_queries > 0 {
        details.push(
            tr("app.confirm.unsaved_queries")
                .replace("%{count}", &pending.unsaved_queries.to_string()),
        );
    }
    if pending.running_queries > 0 {
        details.push(
            tr("app.confirm.running_queries")
                .replace("%{count}", &pending.running_queries.to_string()),
        );
    }
    let message = if details.is_empty() {
        tr("app.confirm.disconnect_message").replace("%{name}", &connection_name)
    } else {
        tr("app.confirm.disconnect_with_tabs")
            .replace("%{name}", &connection_name)
            .replace("%{details}", &details.join(", "))
    };

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w(px(440.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .track_focus(&focus_handle)
                .key_context("DisconnectConnectionModal")
                .on_action(cx.listener(|this, _: &DeleteConnectionShortcut, _, cx| {
                    this.confirm_disconnect_connection(cx);
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(|this, _: &CancelDeleteConnection, _, cx| {
                    this.cancel_disconnect_connection(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.confirm.close_connection_title")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_disconnect_connection(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(message),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("disconnect-connection-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_disconnect_connection(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("disconnect-connection-confirm")
                                .label(tr("app.confirm.close_connection_title"))
                                .danger()
                                .w(px(96.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_disconnect_connection(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn close_workspace_modal(
    pending: PendingCloseWorkspace,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let mut details = Vec::new();
    if pending.unsaved_queries > 0 {
        details.push(
            tr("app.confirm.unsaved_queries")
                .replace("%{count}", &pending.unsaved_queries.to_string()),
        );
    }
    if pending.running_queries > 0 {
        details.push(
            tr("app.confirm.running_queries")
                .replace("%{count}", &pending.running_queries.to_string()),
        );
    }
    let message = match &pending.scope {
        FirstLevelScope::Database(workspace) => tr("app.confirm.close_workspace_message")
            .replace("%{database}", &workspace.database)
            .replace("%{details}", &details.join(", ")),
        FirstLevelScope::RedisConnection(_) => tr("app.confirm.close_redis_connection_message")
            .replace(
                "%{connection}",
                pending.display_name.as_deref().unwrap_or_default(),
            )
            .replace("%{details}", &details.join(", ")),
        FirstLevelScope::Settings => tr("app.confirm.close_settings_message")
            .replace("%{details}", &details.join(", ")),
        FirstLevelScope::AiChat => tr("app.confirm.close_ai_chat_message")
            .replace("%{details}", &details.join(", ")),
    };

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w(px(440.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.context.close_tab")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_close_workspace_scope(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(message),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("close-workspace-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_close_workspace_scope(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("close-workspace-confirm")
                                .label(tr("app.common.close"))
                                .danger()
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_close_workspace_scope(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn delete_data_row_modal(
    menu: DataCellContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let message = if menu.selection_row_count > 1 {
        tr("app.confirm.delete_rows_message")
            .replace("%{count}", &menu.selection_row_count.to_string())
    } else {
        tr("app.confirm.delete_row_message")
            .replace("%{row}", &(menu.source_row + 1).to_string())
    };

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w(px(420.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.context.delete_record")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_delete_data_cell_row(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(message),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("delete-data-row-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_delete_data_cell_row(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("delete-data-row-confirm")
                                .label(tr("app.common.delete"))
                                .danger()
                                .w(px(78.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.pending_delete_data_row.is_some() {
                                        this.confirm_delete_data_cell_row(cx);
                                    }
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn dirty_data_action_modal(
    action: PendingDirtyDataAction,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w(px(430.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(tr("app.confirm.uncommitted_title")),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_dirty_data_action(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(tr("app.confirm.uncommitted_message")),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("dirty-refresh-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_dirty_data_action(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("dirty-data-action-confirm")
                                .label(tr("app.confirm.continue"))
                                .danger()
                                .w(px(78.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this
                                        .pending_dirty_data_action
                                        .as_ref()
                                        .is_some_and(|pending| pending.tab_id() == action.tab_id())
                                    {
                                        this.confirm_dirty_data_action(cx);
                                    }
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn dirty_tab_close_modal(
    tab_id: TabId,
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let settings_tab = state
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .is_some_and(|tab| matches!(&tab.kind, TabKind::Settings(_)));
    let tab_title = state
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .map(localized_tab_title)
        .unwrap_or_else(|| tr("app.confirm.current_query").to_string());

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.dispatch(AppCommand::CancelCloseDirtyTab(tab_id), cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(430.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(if settings_tab {
                                tr("app.confirm.close_unsaved_settings")
                            } else {
                                tr("app.confirm.close_unsaved_query")
                            }),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.dispatch(AppCommand::CancelCloseDirtyTab(tab_id), cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .pb_4()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_size(ui_text_size(13.))
                        .text_color(colors.muted)
                        .child(if settings_tab {
                            tr("app.confirm.settings_unsaved_message").to_string()
                        } else {
                            tr("app.confirm.query_unsaved_message")
                                .replace("%{title}", &tab_title)
                        })
                        .child(if settings_tab {
                            tr("app.confirm.settings_discard_message")
                        } else {
                            tr("app.confirm.query_discard_message")
                        }),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("dirty-tab-close-cancel")
                                .label(tr("app.common.cancel"))
                                .w(px(78.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::CancelCloseDirtyTab(tab_id), cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("dirty-tab-close-confirm")
                                .label(tr("app.common.close"))
                                .danger()
                                .w(px(78.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::ConfirmCloseDirtyTab(tab_id), cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

fn apply_data_changes_modal(
    preview: (TabId, usize, usize, String),
    submitting: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let (tab_id, change_count, statement_count, sql) = preview;
    let sql_for_copy = sql.clone();
    let mut sql_lines = div()
        .flex()
        .flex_col()
        .gap_1()
        .font_family(crate::PLATFORM_MONOSPACE_FONT)
        .text_size(ui_text_size(12.))
        .line_height(ui_text_size(18.))
        .text_color(colors.text);
    for line in sql.lines() {
        sql_lines = sql_lines.child(div().child(line.to_string()));
    }

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_apply_data_changes(cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(720.))
                .max_w(px(920.))
                .h(px(520.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .overflow_hidden()
                .text_color(colors.text)
                .flex()
                .flex_col()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(ui_text_size(17.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(tr("app.confirm.apply_changes_title")),
                                )
                                .child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .text_color(colors.muted)
                                        .child(
                                            tr("app.confirm.apply_changes_summary")
                                                .replace(
                                                    "%{changes}",
                                                    &change_count.to_string(),
                                                )
                                                .replace(
                                                    "%{statements}",
                                                    &statement_count.to_string(),
                                                ),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Close, 15., colors.muted))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_apply_data_changes(cx);
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .mx_5()
                        .mt_4()
                        .mb_4()
                        .flex_1()
                        .min_h(px(0.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border_soft)
                        .bg(if colors.is_dark {
                            rgb(0x101418)
                        } else {
                            rgb(0xf8fafc)
                        })
                        .overflow_y_scrollbar()
                        .p_3()
                        .child(sql_lines),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(58.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            Button::new("apply-data-changes-copy-sql")
                                .label(tr("app.confirm.copy_sql"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        sql_for_copy.clone(),
                                    ));
                                    this.show_message(
                                        tr("app.confirm.copied_pending_sql"),
                                        AppMessageKind::Success,
                                        cx,
                                    );
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Button::new("apply-data-changes-cancel")
                                        .debug_selector(|| "apply-data-changes-cancel".into())
                                        .label(tr("app.common.cancel"))
                                        .w(px(78.))
                                        .disabled(submitting)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_apply_data_changes(cx);
                                            cx.stop_propagation();
                                        })),
                                )
                                .child(
                                    Button::new("apply-data-changes-confirm")
                                        .debug_selector(|| "apply-data-changes-confirm".into())
                                        .label(if submitting {
                                            tr("app.confirm.submitting")
                                        } else {
                                            tr("app.confirm.submit")
                                        })
                                        .loading(submitting)
                                        .primary()
                                        .w(px(78.))
                                        .disabled(submitting)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if this.pending_apply_data_changes == Some(tab_id)
                                                && !this.data_change_apply_in_flight(tab_id)
                                            {
                                                this.confirm_apply_data_changes(window, cx);
                                            }
                                            cx.stop_propagation();
                                        })),
                                ),
                        ),
                ),
        )
}

include!("dangerous_confirmation.rs");
