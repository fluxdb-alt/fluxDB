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
    // MongoDB 没有 SQL 概念：不显示用户与权限 / 执行 SQL 文件 / 新建查询 / 选择显示数据库，
    // 改为「显示连接信息」与「查看性能指标」（后者首版置灰）。
    let is_mongo = connection
        .is_some_and(|connection| connection.config.kind == DatabaseKind::MongoDb);

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
                tr("app.connection_menu.close_connection").to_string(),
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
                tr("app.connection_menu.open_connection").to_string(),
                "connect",
                None,
                false,
                Some(ConnectionMenuAction::Open),
                menu.connection_id,
                colors,
                cx,
            )
        })
        .when(!is_mongo, |this| {
            this.child(connection_menu_item(
                tr("app.connection_menu.new_query").to_string(),
                "query",
                None,
                false,
                Some(ConnectionMenuAction::NewQuery),
                menu.connection_id,
                colors,
                cx,
            ))
        })
        .when(!is_redis && !is_mongo, |this| {
            this.child(connection_menu_item(
                tr("app.connection_menu.user_admin").to_string(),
                "users",
                None,
                false,
                user_admin_action,
                menu.connection_id,
                colors,
                cx,
            ))
        })
        .when(!is_redis && !is_mongo, |this| {
            this.child(connection_menu_item(
                tr("app.connection_menu.run_sql_file").to_string(),
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
                tr("app.connection_menu.new_database").to_string(),
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
            tr("app.connection_menu.ungroup").to_string(),
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
            tr("app.connection_menu.refresh").to_string(),
            "refresh",
            Some("F5"),
            false,
            Some(ConnectionMenuAction::Refresh),
            menu.connection_id,
            colors,
            cx,
        ))
        .when(is_mongo, |this| {
            this.child(connection_menu_item(
                tr("app.connection_menu.mongo_info").to_string(),
                "info",
                None,
                false,
                Some(ConnectionMenuAction::MongoInfo),
                menu.connection_id,
                colors,
                cx,
            ))
            // 首版未采集指标：置灰占位，避免点了没有反馈。
            .child(connection_menu_item(
                tr("app.connection_menu.mongo_performance").to_string(),
                "activity",
                None,
                false,
                None,
                menu.connection_id,
                colors,
                cx,
            ))
        })
        .when(!is_redis && !is_mongo, |this| {
            this.child(connection_menu_item(
                tr("app.connection_menu.select_databases").to_string(),
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
            tr("app.connection_menu.edit_connection").to_string(),
            "edit",
            None,
            false,
            Some(ConnectionMenuAction::Edit),
            menu.connection_id,
            colors,
            cx,
        ))
        .child(connection_menu_item(
            tr("app.connection_menu.copy_connection").to_string(),
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
            tr("app.connection_menu.delete_connection").to_string(),
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
    is_postgres: bool,
    is_sqlserver: bool,
    is_dameng: bool,
    is_mongo: bool,
    is_oracle: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let database_key = database_tree_key(menu.connection_id, &menu.database);
    let pinned = pinned_databases.contains(&database_key);
    let pin_label = if pinned {
        tr("app.context.unpin").to_string()
    } else {
        tr("app.context.pin").to_string()
    };
    let open_label = if menu.expanded {
        tr("app.connection_menu.close_database").to_string()
    } else {
        tr("app.connection_menu.open_database").to_string()
    };
    // Redis 数据库使用 Workbench 命令执行器，SQL 连接使用查询编辑器。
    let query_label = if is_redis {
        tr("app.connection_menu.redis_workbench").to_string()
    } else {
        tr("app.connection_menu.new_query").to_string()
    };

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
                tr("app.connection_menu.new_backup").to_string(),
                "save",
                false,
                DatabaseMenuAction::Backup,
                menu.clone(),
                colors,
                cx,
            ))
            .child(database_menu_item(
                tr("app.connection_menu.refresh").to_string(),
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
        // MongoDB：创建集合 / 命令行终端 / 刷新 / 删除数据库（无表、无默认库语义）。
        .when(is_mongo, |this| {
            let menu = menu.clone();
            this.child(connection_menu_separator(colors))
                .child(database_menu_item(
                    tr("app.connection_menu.new_collection").to_string(),
                    "collection",
                    false,
                    DatabaseMenuAction::NewCollection,
                    menu.clone(),
                    colors,
                    cx,
                ))
                .child(database_menu_item(
                    tr("app.connection_menu.mongo_cli").to_string(),
                    "terminal",
                    false,
                    DatabaseMenuAction::MongoCli,
                    menu.clone(),
                    colors,
                    cx,
                ))
                .child(database_menu_item(
                    tr("app.connection_menu.mongo_workbench").to_string(),
                    "query",
                    false,
                    DatabaseMenuAction::MongoWorkbench,
                    menu.clone(),
                    colors,
                    cx,
                ))
                .child(connection_menu_separator(colors))
                .child(database_menu_item(
                    tr("app.connection_menu.refresh").to_string(),
                    "refresh",
                    false,
                    DatabaseMenuAction::Refresh,
                    menu.clone(),
                    colors,
                    cx,
                ))
                .child(database_menu_item(
                    tr("app.connection_menu.delete_database").to_string(),
                    "delete",
                    true,
                    DatabaseMenuAction::Delete,
                    menu,
                    colors,
                    cx,
                ))
        })
        // 置顶/设置默认数据库仅对 SQL 型连接有意义（Redis 无默认库语义）。
        .when(!is_redis && !is_mongo, |this| {
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
                tr("app.connection_menu.set_default_database").to_string(),
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
                tr("app.context.new_table").to_string(),
                "table",
                false,
                DatabaseMenuAction::NewTable,
                menu.clone(),
                colors,
                cx,
            ))
            // Oracle 的 schema 即账号，`create_schema` 会建账号（需 CREATE USER 权限）。
            .when(
                is_postgres || is_sqlserver || is_dameng || is_oracle,
                |this| {
                this.child(database_menu_item(
                    tr("app.connection_menu.new_schema").to_string(),
                    "folder-open",
                    false,
                    DatabaseMenuAction::NewSchema,
                    menu.clone(),
                    colors,
                    cx,
                ))
                },
            )
            .child(database_menu_item(
                tr("app.connection_menu.run_sql_file").to_string(),
                "file-sql",
                false,
                DatabaseMenuAction::RunSqlFile,
                menu.clone(),
                colors,
                cx,
            ))
            // 数据库备份仅对 SQL 型连接有意义（MySQL/TiDB/SQLite）。
            .child(database_menu_item(
                tr("app.context.backup").to_string(),
                "save",
                false,
                DatabaseMenuAction::Backup,
                menu.clone(),
                colors,
                cx,
            ))
        })
        .when(!is_mongo, |this| {
            let menu = menu.clone();
            this.child(database_menu_item(
                query_label,
                "query",
                false,
                DatabaseMenuAction::NewQuery,
                menu,
                colors,
                cx,
            ))
        })
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
                tr("app.connection_menu.open_pubsub").to_string(),
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
        .when(!is_redis && !is_mongo, |this| {
            this.child(database_menu_item(
                tr("app.connection_menu.find_in_database").to_string(),
                "file-search",
                false,
                DatabaseMenuAction::FindInDatabase,
                menu.clone(),
                colors,
                cx,
            ))
        })
        // Mongo 的刷新/删除数据库已在上方分支渲染，这里仅处理其余连接类型。
        .when(!is_mongo, |this| {
            let menu = menu.clone();
            this.child(database_menu_item(
                tr("app.connection_menu.refresh").to_string(),
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
                        tr("app.connection_menu.delete_database").to_string(),
                        "delete",
                        true,
                        DatabaseMenuAction::Delete,
                        menu,
                        colors,
                        cx,
                    ))
            })
        })
}

/// PostgreSQL schema 节点右键菜单：与数据库菜单隔离，动作均为 schema 作用域。
fn schema_context_menu(
    menu: SchemaContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(204.))
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
        .key_context("SchemaContextMenu")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(schema_menu_item(
            &tr("app.connection_menu.new_query"),
            "query",
            SchemaMenuAction::NewQuery,
            &menu,
            colors,
            cx,
        ))
        .child(schema_menu_item(
            &tr("app.context.new_table"),
            "table",
            SchemaMenuAction::NewTable,
            &menu,
            colors,
            cx,
        ))
        .child(connection_menu_separator(colors))
        .child(schema_menu_item(
            &tr("app.connection_menu.set_default_schema"),
            "default",
            SchemaMenuAction::SetDefault,
            &menu,
            colors,
            cx,
        ))
        .child(schema_menu_item(
            &tr("app.connection_menu.refresh"),
            "refresh",
            SchemaMenuAction::Refresh,
            &menu,
            colors,
            cx,
        ))
}

fn schema_menu_item(
    label: &str,
    icon: &'static str,
    action: SchemaMenuAction,
    menu: &SchemaContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let label = label.to_string();
    let menu = menu.clone();
    div()
        .id(("schema-menu-item", schema_menu_item_hash(menu.connection_id, &label)))
        .h(px(32.))
        .rounded(colors.radius_lg)
        .px_2()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.handle_schema_menu_action(action, menu.clone(), window, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(22.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(menu_icon(icon, false, colors)),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label),
        )
}

fn schema_menu_item_hash(connection_id: ConnectionId, label: &str) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    connection_id.hash(&mut hasher);
    label.hash(&mut hasher);
    hasher.finish()
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
        tr("app.connection_menu.new_group").to_string(),
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
                .child(tr("app.connection_menu.move_to_group")),
        )
        .child(div().text_color(colors.muted).child("›"))
}
