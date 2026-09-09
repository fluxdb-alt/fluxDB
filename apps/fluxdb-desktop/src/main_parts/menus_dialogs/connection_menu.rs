fn connection_context_menu(
    menu: ConnectionContextMenu,
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let connection = state
        .connections
        .iter()
        .find(|connection| connection.config.id == menu.connection_id);
    let connected = connection.is_some_and(|connection| connection.connected);
    let grouped = state
        .sidebar_layout
        .is_connection_grouped(menu.connection_id);
    let user_admin_action = connection
        .filter(|connection| supports_database_user_admin(connection.config.kind))
        .map(|_| ConnectionMenuAction::UserAdmin);
    // Redis 连接仅保留数据浏览/键管理类入口；执行 SQL 文件、新建数据库、用户与权限、
    // 选择显示数据库四项仅在非 Redis 连接下显示。
    let is_redis = connection
        .is_some_and(|connection| connection.config.kind == DatabaseKind::Redis);

    let mut menu_view = div()
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(230.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .occlude()
        .shadow(vec![box_shadow(
            px(0.),
            px(14.),
            px(30.),
            px(0.),
            hsla(0., 0., 0., 0.16),
        )])
        .p_2()
        .text_size(px(14.))
        .text_color(colors.text)
        .key_context("ConnectionContextMenu")
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(if connected {
            connection_menu_item(
                "关闭连接",
                "disconnect",
                None,
                false,
                Some(ConnectionMenuAction::Disconnect),
                menu.connection_id,
                colors,
                cx,
            )
        } else {
            connection_menu_item(
                "打开连接",
                "connect",
                None,
                false,
                Some(ConnectionMenuAction::Open),
                menu.connection_id,
                colors,
                cx,
            )
        })
        .child(connection_menu_item(
            "新建查询",
            "query",
            None,
            false,
            Some(ConnectionMenuAction::NewQuery),
            menu.connection_id,
            colors,
            cx,
        ))
        .when(!is_redis, |this| {
            this.child(connection_menu_item(
                "用户与权限",
                "users",
                None,
                false,
                user_admin_action,
                menu.connection_id,
                colors,
                cx,
            ))
        })
        .when(!is_redis, |this| {
            this.child(connection_menu_item(
                "执行 SQL 文件",
                "file-sql",
                None,
                false,
                Some(ConnectionMenuAction::ExecuteSqlFile),
                menu.connection_id,
                colors,
                cx,
            ))
        })
        .when(!is_redis, |this| {
            this.child(connection_menu_item(
                "新建数据库",
                "plus",
                None,
                false,
                Some(ConnectionMenuAction::NewDatabase),
                menu.connection_id,
                colors,
                cx,
            ))
        })
        .child(connection_menu_separator(colors));

    menu_view = menu_view.child(move_to_group_menu_item(menu, colors, cx));
    if menu.show_group_submenu {
        menu_view = menu_view.child(connection_group_submenu(menu, state, colors, cx));
    }

    if grouped {
        menu_view = menu_view.child(connection_menu_item(
            "取消分组",
            "ungroup",
            None,
            false,
            Some(ConnectionMenuAction::Ungroup),
            menu.connection_id,
            colors,
            cx,
        ));
    }

    menu_view
        .child(connection_menu_item(
            "刷新",
            "refresh",
            Some("F5"),
            false,
            Some(ConnectionMenuAction::Refresh),
            menu.connection_id,
            colors,
            cx,
        ))
        .when(!is_redis, |this| {
            this.child(connection_menu_item(
                "选择显示数据库",
                "filter",
                None,
                false,
                Some(ConnectionMenuAction::SelectDatabases),
                menu.connection_id,
                colors,
                cx,
            ))
        })
        .child(connection_menu_item(
            "编辑连接",
            "edit",
            None,
            false,
            Some(ConnectionMenuAction::Edit),
            menu.connection_id,
            colors,
            cx,
        ))
        .child(connection_menu_item(
            "复制连接",
            "copy",
            None,
            false,
            Some(ConnectionMenuAction::Copy),
            menu.connection_id,
            colors,
            cx,
        ))
        .child(connection_menu_separator(colors))
        .child(connection_menu_item(
            "删除连接",
            "delete",
            Some("Del"),
            true,
            Some(ConnectionMenuAction::Delete),
            menu.connection_id,
            colors,
            cx,
        ))
}

fn database_context_menu(
    menu: DatabaseContextMenu,
    pinned_databases: &BTreeSet<String>,
    is_redis: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let database_key = database_tree_key(menu.connection_id, &menu.database);
    let pinned = pinned_databases.contains(&database_key);
    let pin_label = if pinned { "取消置顶" } else { "置顶" };
    let open_label = if menu.expanded {
        "关闭数据库"
    } else {
        "打开数据库"
    };
    // Redis 数据库使用 Workbench 命令执行器，SQL 连接使用查询编辑器。
    let query_label = if is_redis { "Redis Workbench" } else { "新建查询" };

    // 备份节点右键：只渲染「新建备份」一个菜单项，而非完整库菜单。
    if menu.backup_only {
        return div()
            .absolute()
            .top(menu.position.y)
            .left(menu.position.x)
            .rounded(colors.radius_lg)
            .border_1()
            .border_color(colors.border)
            .bg(menu_surface_bg(colors))
            .occlude()
            .shadow(vec![box_shadow(
                px(0.),
                px(14.),
                px(30.),
                px(0.),
                hsla(0., 0., 0., 0.16),
            )])
            .p_1()
            .text_size(px(13.))
            .text_color(colors.text)
            .key_context("DatabaseContextMenu")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(database_menu_item(
                "新建备份",
                "save",
                false,
                DatabaseMenuAction::Backup,
                menu.clone(),
                colors,
                cx,
            ))
            .child(database_menu_item(
                "刷新",
                "refresh",
                false,
                DatabaseMenuAction::Refresh,
                menu,
                colors,
                cx,
            ));
    }

    div()
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(238.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .occlude()
        .shadow(vec![box_shadow(
            px(0.),
            px(14.),
            px(30.),
            px(0.),
            hsla(0., 0., 0., 0.16),
        )])
        .p_2()
        .text_size(px(14.))
        .text_color(colors.text)
        .key_context("DatabaseContextMenu")
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(database_menu_item(
            open_label,
            "database",
            false,
            DatabaseMenuAction::ToggleOpen,
            menu.clone(),
            colors,
            cx,
        ))
        // 置顶/设置默认数据库仅对 SQL 型连接有意义（Redis 无默认库语义）。
        .when(!is_redis, |this| {
            this.child(database_menu_item(
                pin_label,
                "pin",
                false,
                DatabaseMenuAction::TogglePin,
                menu.clone(),
                colors,
                cx,
            ))
            .child(database_menu_item(
                "设置默认数据库",
                "default",
                false,
                DatabaseMenuAction::SetDefault,
                menu.clone(),
                colors,
                cx,
            ))
            .child(connection_menu_separator(colors))
            // 新建表/执行 SQL 文件仅对 SQL 型连接有意义。
            .child(database_menu_item(
                "新建表",
                "table",
                false,
                DatabaseMenuAction::NewTable,
                menu.clone(),
                colors,
                cx,
            ))
            .child(database_menu_item(
                "执行 SQL 文件",
                "file-sql",
                false,
                DatabaseMenuAction::RunSqlFile,
                menu.clone(),
                colors,
                cx,
            ))
            // 数据库备份仅对 SQL 型连接有意义（MySQL/TiDB/SQLite）。
            .child(database_menu_item(
                "备份",
                "save",
                false,
                DatabaseMenuAction::Backup,
                menu.clone(),
                colors,
                cx,
            ))
        })
        .child(database_menu_item(
            query_label,
            "query",
            false,
            DatabaseMenuAction::NewQuery,
            menu.clone(),
            colors,
            cx,
        ))
        // Redis CLI 终端：独立 terminal tab，仅在 Redis 连接下显示。
        .when(is_redis, |this| {
            this.child(database_menu_item(
                "Redis CLI",
                "terminal",
                false,
                DatabaseMenuAction::RedisCli,
                menu.clone(),
                colors,
                cx,
            ))
        })
        // Redis Pub/Sub：打开该数据库的订阅/发布实时会话页，仅在 Redis 连接下显示。
        .when(is_redis, |this| {
            this.child(database_menu_item(
                "打开 Pub/Sub",
                "broadcast",
                false,
                DatabaseMenuAction::PubSub,
                menu.clone(),
                colors,
                cx,
            ))
        })
        .child(connection_menu_separator(colors))
        // 在数据库中查找（表级搜索）仅对 SQL 型连接有意义。
        .when(!is_redis, |this| {
            this.child(database_menu_item(
                "在数据库中查找",
                "file-search",
                false,
                DatabaseMenuAction::FindInDatabase,
                menu.clone(),
                colors,
                cx,
            ))
        })
        .child(database_menu_item(
            "刷新",
            "refresh",
            false,
            DatabaseMenuAction::Refresh,
            menu.clone(),
            colors,
            cx,
        ))
        // 删除数据库仅对 SQL 型连接有意义（Redis 逻辑库为内建索引，不可删除）。
        .when(!is_redis, |this| {
            this.child(connection_menu_separator(colors))
                .child(database_menu_item(
                    "删除数据库",
                    "delete",
                    true,
                    DatabaseMenuAction::Delete,
                    menu,
                    colors,
                    cx,
                ))
        })
}

fn connection_group_submenu(
    menu: ConnectionContextMenu,
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut submenu = div()
        .absolute()
        .left(px(230.))
        .top(px(166.))
        .w(px(220.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(menu_surface_bg(colors))
        .occlude()
        .shadow(vec![box_shadow(
            px(0.),
            px(14.),
            px(30.),
            px(0.),
            hsla(0., 0., 0., 0.16),
        )])
        .p_2()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        });

    for group in &state.sidebar_layout.groups {
        submenu = submenu.child(connection_menu_item(
            group.name.clone(),
            "folder",
            None,
            false,
            Some(ConnectionMenuAction::MoveToGroup(group.id)),
            menu.connection_id,
            colors,
            cx,
        ));
    }

    if !state.sidebar_layout.groups.is_empty() {
        submenu = submenu.child(connection_menu_separator(colors));
    }

    submenu.child(connection_menu_item(
        "新建分组...",
        "folder-plus",
        None,
        false,
        Some(ConnectionMenuAction::MoveToNewGroup),
        menu.connection_id,
        colors,
        cx,
    ))
}

fn move_to_group_menu_item(
    menu: ConnectionContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    div()
        .id(("move-to-group-menu-item", menu.connection_id.0))
        .h(px(32.))
        .rounded(colors.radius_lg)
        .px_2()
        .flex()
        .items_center()
        .gap_3()
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .on_hover(cx.listener(move |this, hovered, _, cx| {
            if *hovered {
                if let Some(active_menu) = &mut this.connection_context_menu {
                    if active_menu.connection_id == menu.connection_id {
                        active_menu.show_group_submenu = true;
                    }
                }
                cx.notify();
            }
        }))
        .child(
            div()
                .w(px(22.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(menu_icon("move-group", false, colors)),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("移至分组"),
        )
        .child(div().text_color(colors.muted).child("›"))
}
