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
        .unwrap_or_else(|| "该连接".to_string());

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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("删除连接"),
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
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(format!(
                            "确定要删除「{}」吗？此操作不可撤销。",
                            connection_name
                        )),
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
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_delete_connection(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("delete-connection-confirm")
                                .label("删除连接")
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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("删除数据库"),
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
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(format!(
                            "确定要删除数据库「{}」吗？此操作不可撤销。",
                            pending.database
                        )),
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
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_delete_database(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("delete-database-confirm")
                                .label("删除数据库")
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
        .unwrap_or_else(|| "该连接".to_string());
    let mut details = Vec::new();
    if pending.unsaved_queries > 0 {
        details.push(format!(
            "{} 个查询有未保存或已修改的 SQL",
            pending.unsaved_queries
        ));
    }
    if pending.running_queries > 0 {
        details.push(format!("{} 个查询正在执行", pending.running_queries));
    }
    let message = if details.is_empty() {
        format!("确定要关闭「{}」吗？", connection_name)
    } else {
        format!(
            "关闭「{}」会关闭相关标签页，{}。",
            connection_name,
            details.join("，")
        )
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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("关闭连接"),
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
                        .text_size(px(13.))
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
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_disconnect_connection(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("disconnect-connection-confirm")
                                .label("关闭连接")
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
        details.push(format!(
            "{} 个查询有未保存或已修改的 SQL",
            pending.unsaved_queries
        ));
    }
    if pending.running_queries > 0 {
        details.push(format!("{} 个查询正在执行", pending.running_queries));
    }
    let message = format!(
        "关闭「{}」会关闭下面的表和查询标签页，{}。",
        pending.scope.database,
        details.join("，")
    );

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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("关闭标签页"),
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
                        .text_size(px(13.))
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
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_close_workspace_scope(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("close-workspace-confirm")
                                .label("关闭")
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
        format!(
            "确定要删除选中的 {} 行记录吗？删除会先进入本地修改，提交后才写入数据库。",
            menu.selection_row_count
        )
    } else {
        format!(
            "确定要删除第 {} 行记录吗？删除会先进入本地修改，提交后才写入数据库。",
            menu.source_row + 1
        )
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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("删除记录"),
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
                        .text_size(px(13.))
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
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_delete_data_cell_row(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("delete-data-row-confirm")
                                .label("删除")
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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("未提交修改"),
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
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child("当前数据有未提交修改。继续操作会丢弃这些本地修改并重新加载数据，是否继续？"),
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
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_dirty_data_action(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("dirty-data-action-confirm")
                                .label("继续")
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
        .map(|tab| tab.title.clone())
        .unwrap_or_else(|| "当前查询".to_string());

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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(if settings_tab {
                                "关闭未保存设置"
                            } else {
                                "关闭未保存查询"
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
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(if settings_tab {
                            "当前设置有未保存修改。".to_string()
                        } else {
                            format!("“{tab_title}” 有未保存修改。")
                        })
                        .child(if settings_tab {
                            "关闭会恢复到上次保存的设置，是否继续？"
                        } else {
                            "关闭会丢弃这些修改，是否继续？"
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
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.dispatch(AppCommand::CancelCloseDirtyTab(tab_id), cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("dirty-tab-close-confirm")
                                .label("关闭")
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
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let (tab_id, change_count, statement_count, sql) = preview;
    let sql_for_copy = sql.clone();
    let mut sql_lines = div()
        .flex()
        .flex_col()
        .gap_1()
        .font_family("Menlo")
        .text_size(px(12.))
        .line_height(px(18.))
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
                                        .text_size(px(17.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child("提交更改"),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(colors.muted)
                                        .child(format!(
                                            "即将提交 {change_count} 处修改，执行 {statement_count} 条 SQL。"
                                        )),
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
                                .label("复制 SQL")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        sql_for_copy.clone(),
                                    ));
                                    this.show_message("已复制待提交 SQL", AppMessageKind::Success, cx);
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
                                        .label("取消")
                                        .w(px(78.))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_apply_data_changes(cx);
                                            cx.stop_propagation();
                                        })),
                                )
                                .child(
                                    Button::new("apply-data-changes-confirm")
                                        .label("提交")
                                        .primary()
                                        .w(px(78.))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if this.pending_apply_data_changes == Some(tab_id) {
                                                this.confirm_apply_data_changes(cx);
                                            }
                                            cx.stop_propagation();
                                        })),
                                ),
                        ),
                ),
        )
}

fn dangerous_query_modal(
    pending: PendingDangerousQuery,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let preview = single_line_summary_text(pending.text);
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
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.cancel_dangerous_query(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(460.))
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
                .key_context("DangerousQueryModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_dangerous_query(cx);
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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("确认执行危险 SQL"),
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
                                        this.cancel_dangerous_query(cx);
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
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child("该语句可能修改或删除大量数据。")
                        .child(preview),
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
                            Button::new("dangerous-query-cancel")
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_dangerous_query(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("dangerous-query-confirm")
                                .label("执行")
                                .danger()
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_dangerous_query(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}

/// Redis Workbench 危险命令二次确认弹框。
///
/// 交互语义对齐沿用的 modal 规范：遮罩点击或 Esc 取消（关闭），
/// 弹框内部点击用 stop_propagation 阻止穿透到遮罩。
fn dangerous_redis_command_modal(
    pending: PendingDangerousRedisCommand,
    focus_handle: FocusHandle,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let preview = single_line_summary_text(pending.text);
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
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.cancel_dangerous_redis_command(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(460.))
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
                .key_context("DangerousRedisCommandModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_dangerous_redis_command(cx);
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
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("确认执行危险 Redis 命令"),
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
                                        this.cancel_dangerous_redis_command(cx);
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
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child("该命令可能清空或破坏数据，请确认是否继续执行。")
                        .child(preview),
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
                            Button::new("dangerous-redis-command-cancel")
                                .label("取消")
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_dangerous_redis_command(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("dangerous-redis-command-confirm")
                                .label("执行")
                                .danger()
                                .w(px(78.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_dangerous_redis_command(cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
}
