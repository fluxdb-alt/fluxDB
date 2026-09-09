fn new_query_scope_modal(
    selected_connection_id: ConnectionId,
    state: &AppState,
    connecting_connections: &BTreeSet<ConnectionId>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let selected_connection = state
        .connections
        .iter()
        .find(|connection| connection.config.id == selected_connection_id)
        .or_else(|| state.connections.first());

    let mut connection_list = div().flex().flex_col().gap_1();
    for connection in &state.connections {
        let connection_id = connection.config.id;
        let selected = Some(connection_id) == selected_connection.map(|connection| connection.config.id);
        connection_list = connection_list.child(
            div()
                .h(px(34.))
                .rounded(colors.radius_lg)
                .px_2()
                .cursor_pointer()
                .flex()
                .items_center()
                .gap_2()
                .bg(if selected { colors.hover } else { colors.panel_bg })
                .hover(move |style| style.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.pending_new_query_connection = Some(connection_id);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .size(px(22.))
                        .rounded(colors.radius)
                        .bg(rgb(0x24272d))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(img(database_kind_icon_path(connection.config.kind)).size(px(14.))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(connection.config.name.clone()),
                ),
        );
    }

    let mut database_list = div().flex().flex_col().gap_1();
    if let Some(connection) = selected_connection {
        let connection_id = connection.config.id;
        database_list = new_query_database_list(
            database_list,
            connection,
            connecting_connections.contains(&connection_id),
            state.last_error.as_ref(),
            colors,
            cx,
        );
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
                this.pending_new_query_connection = None;
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .child(
            div()
                .w(px(560.))
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
                                .child("新建查询"),
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
                                        this.pending_new_query_connection = None;
                                        cx.stop_propagation();
                                        cx.notify();
                                    }),
                                ),
                        ),
                )
                .child(div().h(px(1.)).bg(colors.border_soft))
                .child(
                    div()
                        .h(px(320.))
                        .flex()
                        .child(
                            div()
                                .w(px(230.))
                                .border_r_1()
                                .border_color(colors.border_soft)
                                .p_2()
                                .overflow_y_scrollbar()
                                .child(connection_list),
                        )
                        .child(
                            div()
                                .flex_1()
                                .p_2()
                                .overflow_y_scrollbar()
                                .child(database_list),
                        ),
                ),
        )
}

fn new_query_database_list(
    mut list: Div,
    connection: &ConnectionState,
    connecting: bool,
    error: Option<&fluxdb_core::UserFacingError>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let connection_id = connection.config.id;
    if let Some(database) = automatic_query_database(&connection.config) {
        return list.child(new_query_database_item(
            connection_id,
            Some(database.clone()),
            query_database_label(&connection.config, &database),
            colors,
            cx,
        ));
    }

    if !connection.connected {
        return list.child(new_query_connect_state(connection_id, connecting, error, colors, cx));
    }

    let mut has_database = false;
    for database in connection_databases(connection) {
        let name = database.path.database.unwrap_or(database.path.name);
        has_database = true;
        list = list.child(new_query_database_item(
            connection_id,
            Some(name.clone()),
            name,
            colors,
            cx,
        ));
    }

    if has_database {
        list
    } else {
        list.child(new_query_empty_state("没有可选库", colors))
    }
}

fn automatic_query_database(config: &ConnectionConfig) -> Option<String> {
    match config.kind {
        DatabaseKind::Sqlite => Some("main".to_string()),
        DatabaseKind::Redis => connection_default_database(config).or_else(|| Some("0".to_string())),
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MongoDb => None,
    }
}

fn query_database_label(config: &ConnectionConfig, database: &str) -> String {
    if config.kind == DatabaseKind::Redis && database == "0" {
        "db0".to_string()
    } else {
        database.to_string()
    }
}

fn new_query_connect_state(
    connection_id: ConnectionId,
    connecting: bool,
    error: Option<&fluxdb_core::UserFacingError>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let button_label = if connecting {
        "加载中..."
    } else if error.is_some() {
        "重试"
    } else {
        "连接并加载库"
    };
    div()
        .p_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child(if connecting {
                    "正在连接"
                } else {
                    "先连接后选择库"
                }),
        )
        .when_some(error, |this, error| {
            this.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0xff6b6b))
                    .child(error.message.clone()),
            )
        })
        .child(
            div()
                .h(px(32.))
                .rounded(colors.radius_lg)
                .px_3()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .bg(if connecting { colors.hover } else { colors.panel_alt })
                .when(!connecting, |this| {
                    this.cursor_pointer()
                        .hover(move |style| style.bg(colors.hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.open_connection_from_sidebar(connection_id, cx);
                                cx.stop_propagation();
                            }),
                        )
                })
                .child(button_label),
        )
}

fn new_query_empty_state(label: &'static str, colors: UiColors) -> impl IntoElement {
    div()
        .p_3()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child(label)
}

fn new_query_database_item(
    connection_id: ConnectionId,
    database: Option<String>,
    label: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .h(px(34.))
        .rounded(colors.radius_lg)
        .px_2()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.confirm_new_query_scope(connection_id, database.clone(), cx);
                cx.stop_propagation();
            }),
        )
        .child(app_icon(AppIcon::Database, 15., colors.muted))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(label),
        )
}
