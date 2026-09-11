fn tree(
    arrow: impl IntoElement,
    glyph: &'static str,
    text: String,
    indent: u8,
    selected: bool,
    star: bool,
    accent_color: Option<gpui::Rgba>,
    colors: UiColors,
) -> Div {
    tree_with_highlight(
        arrow,
        glyph,
        text,
        indent,
        selected,
        star,
        accent_color,
        None,
        None,
        colors,
    )
}

#[allow(clippy::too_many_arguments)]
fn tree_with_highlight(
    arrow: impl IntoElement,
    glyph: &'static str,
    text: String,
    indent: u8,
    selected: bool,
    star: bool,
    accent_color: Option<gpui::Rgba>,
    selected_bg: Option<gpui::Rgba>,
    highlight_query: Option<&str>,
    colors: UiColors,
) -> Div {
    let selected_bg = selected_bg.unwrap_or(colors.tree_selected);
    let row_bg = if selected {
        selected_bg
    } else {
        colors.tree_bg
    };
    let hover_bg = if selected { selected_bg } else { colors.hover };

    // 行背景撑满整行：虚拟化列表不提供外层全宽容器（旧整树由外层 w_full 约束），
    // 故行根必须自身 w_full，否则 selected/hover 背景只包内容宽度。
    div()
        .w_full()
        .h(px(26.))
        .mx_1()
        .rounded(colors.radius_lg)
        .pl(px(6. + indent as f32 * 16.))
        .pr_2()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .text_color(colors.text)
        .bg(row_bg)
        .hover(move |style| style.bg(hover_bg))
        .child(
            div()
                .w(px(18.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(arrow),
        )
        .child(
            div()
                .w(px(20.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(match glyph {
                    "database" => rgb(0xf0b400),
                    "table" | "tables" => rgb(0x10b85f),
                    "view" | "views" => rgb(0x9333ea),
                    "procedures" => rgb(0x2087ff),
                    "functions" => rgb(0xf59e0b),
                    "▦" => rgb(0x12b826),
                    "#" => rgb(0xf59e0b),
                    "⌁" => rgb(0x2588ff),
                    "⚡" => rgb(0xff8a00),
                    "┃" => rgb(0x12b826),
                    _ => rgb(0x3b4048),
                })
                .child(tree_icon(glyph)),
        )
        .when_some(accent_color, |this, color| {
            this.child(div().size(px(7.)).rounded_full().bg(color))
        })
        .child(
            div()
                .flex_1()
                .h_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .flex()
                .items_center()
                .child(highlighted_tree_label(&text, highlight_query, colors)),
        )
        .when(star, |this| {
            this.child(div().w(px(18.)).text_color(rgb(0xffc400)).child("★"))
        })
}

fn highlighted_tree_label(text: &str, query: Option<&str>, colors: UiColors) -> Div {
    let mut label = div()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(colors.text);
    let Some(query) = query.filter(|query| !query.is_empty()) else {
        return label.child(text.to_string());
    };
    let lower_text = text.to_ascii_lowercase();
    let Some(first_match) = lower_text.find(query) else {
        return label.child(text.to_string());
    };

    let highlight_bg = if colors.is_dark {
        rgb(0x5b4513)
    } else {
        rgb(0xffe7a3)
    };
    let highlight_text = if colors.is_dark {
        rgb(0xffd166)
    } else {
        rgb(0x8a4b00)
    };
    let mut cursor = 0;
    let mut search_from = first_match;
    while let Some(relative_start) = lower_text[search_from..].find(query) {
        let start = search_from + relative_start;
        let end = start + query.len();
        if start > cursor {
            label = label.child(text[cursor..start].to_string());
        }
        label = label.child(
            div()
                .rounded(colors.radius * 0.5)
                .px(px(1.5))
                .bg(highlight_bg)
                .text_color(highlight_text)
                .child(text[start..end].to_string()),
        );
        cursor = end;
        search_from = end;
    }
    if cursor < text.len() {
        label = label.child(text[cursor..].to_string());
    }
    label
}

fn tree_icon(glyph: &'static str) -> gpui::AnyElement {
    if matches!(glyph, "query" | "queries") {
        return app_icon(AppIcon::FileSql, 16., rgb(0x2588ff));
    }

    if matches!(glyph, "backup") {
        return app_icon(AppIcon::Save, 16., rgb(0x3b4048));
    }

    if let Some(path) = tree_icon_path(glyph) {
        img(path).size(px(16.)).into_any_element()
    } else {
        div()
            .text_size(px(15.))
            .text_color(match glyph {
                "▦" => rgb(0x12b826),
                "#" => rgb(0xf59e0b),
                "⌁" => rgb(0x2588ff),
                "⚡" => rgb(0xff8a00),
                "┃" => rgb(0x12b826),
                _ => rgb(0x3b4048),
            })
            .child(glyph)
            .into_any_element()
    }
}

fn tree_icon_path(glyph: &str) -> Option<&'static str> {
    match glyph {
        "database" => Some("tree/database.svg"),
        "table" | "tables" => Some("tree/table.svg"),
        "view" | "views" => Some("tree/view.svg"),
        "procedures" => Some("tree/procedure.svg"),
        "functions" => Some("tree/function.svg"),
        "folder" => Some("tree/folder.svg"),
        "folder-open" => Some("tree/folder-open.svg"),
        _ => None,
    }
}

fn arrow_element(expanded: bool, colors: UiColors) -> gpui::AnyElement {
    app_icon(
        if expanded {
            AppIcon::ChevronDown
        } else {
            AppIcon::ChevronRight
        },
        12.,
        colors.muted,
    )
}

fn empty_arrow_element() -> gpui::AnyElement {
    div().size(px(12.)).into_any_element()
}

fn loading_spinner(size: f32) -> impl IntoElement {
    loading_spinner_with_color(size, rgb(0x667085))
}

fn loading_spinner_with_color(size: f32, color: gpui::Rgba) -> impl IntoElement {
    svg()
        .size(px(size))
        .path("spinner.svg")
        .text_color(color)
        .with_animation(
            "tree_loading_spinner",
            Animation::new(Duration::from_secs_f64(0.75))
                .repeat()
                .with_easing(ease_in_out),
            |this, delta| this.with_transformation(Transformation::rotate(percentage(delta))),
        )
}

/// 数据库节点整行点击是否应触发展开/加载子节点。
/// Redis 数据库节点整行点击只负责打开数据库，key 列表不放进侧边栏。
fn database_row_click_expands(database_kind: ObjectKind) -> bool {
    database_kind != ObjectKind::RedisDb
}

fn database_tree(
    connection_id: ConnectionId,
    database_path: ObjectPath,
    indent: u8,
    expanded: bool,
    loading: bool,
    has_loaded_children: bool,
    selected: bool,
    pinned: bool,
    connection_color_hex: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let database = database_path
        .database
        .clone()
        .unwrap_or_else(|| database_path.name.clone());
    let database_for_click = database_path.clone();
    let database_for_open = database_path.clone();
    let database_for_menu = database_path.clone();
    let is_redis_db = database_path.kind == ObjectKind::RedisDb;
    // Redis 数据库节点不展开子树，整行点击直接打开 Redis 数据页。
    let arrow: gpui::AnyElement = if !database_row_click_expands(database_path.kind) {
        empty_arrow_element()
    } else if loading {
        loading_spinner(13.).into_any_element()
    } else {
        arrow_element(expanded, colors)
    };
    tree_with_highlight(
        arrow,
        "database",
        database,
        indent,
        selected,
        pinned,
        selected.then_some(connection_color_rgba(connection_color_hex)),
        Some(connection_color_row_bg(connection_color_hex, colors)),
        None,
        colors,
    )
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            this.close_context_menus(cx);
            if is_redis_db {
                // 单击打开 Redis 数据浏览（键浏览器）；双击打开 Redis Workbench 命令执行器。
                let db_number = database_for_open
                    .database
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(0);
                if event.click_count >= 2 {
                    this.dispatch(
                        AppCommand::OpenRedisWorkbench {
                            connection_id,
                            database: db_number,
                        },
                        cx,
                    );
                } else {
                    this.dispatch(AppCommand::OpenDataEditor(database_for_open.clone()), cx);
                }
            } else {
                // 其他数据库类型保持原行为：整行点击展开/加载子节点。
                this.toggle_database_tree(
                    connection_id,
                    database_for_click.clone(),
                    has_loaded_children,
                    cx,
                );
            }
            cx.stop_propagation();
        }),
    )
    .on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
            this.show_database_context_menu(
                DatabaseContextMenu {
                    connection_id,
                    database_path: database_for_menu.clone(),
                    database: database_for_menu
                        .database
                        .clone()
                        .unwrap_or_else(|| database_for_menu.name.clone()),
                    position: event.position,
                    expanded,
                    backup_only: false,
                },
                window,
                cx,
            );
            cx.stop_propagation();
        }),
    )
}

fn schema_tree(
    connection_id: ConnectionId,
    database_path: ObjectPath,
    schema: String,
    indent: u8,
    expanded: bool,
    loading: bool,
    has_loaded_children: bool,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let schema_for_click = database_path.clone();
    let database_for_menu = database_path.clone();
    let schema_name_for_menu = schema.clone();
    let arrow: gpui::AnyElement = if loading {
        loading_spinner(13.).into_any_element()
    } else {
        arrow_element(expanded, colors)
    };
    tree_with_highlight(
        arrow,
        "database",
        schema,
        indent,
        selected,
        false,
        None,
        None,
        None,
        colors,
    )
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            this.close_context_menus(cx);
            this.toggle_schema_tree(
                connection_id,
                schema_for_click.clone(),
                has_loaded_children,
                cx,
            );
            cx.stop_propagation();
        }),
    )
    .on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
            let database = database_for_menu
                .database
                .clone()
                .unwrap_or_else(|| database_for_menu.name.clone());
            this.show_schema_context_menu(
                SchemaContextMenu {
                    connection_id,
                    database,
                    schema: schema_name_for_menu.clone(),
                    position: event.position,
                },
                window,
                cx,
            );
            cx.stop_propagation();
        }),
    )
}

fn object_group_tree(
    connection_id: ConnectionId,
    database_path: ObjectPath,
    database: String,
    group: ObjectGroup,
    indent: u8,
    expanded: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 备份节点单击开 tab 需要 ObjectPath 的独立副本（右键闭包已消费 database_path）。
    let database_path_for_open = database_path.clone();
    tree(
        arrow_element(expanded, colors),
        group.icon_key(),
        group.label().to_string(),
        indent,
        false,
        false,
        None,
        colors,
    )
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            this.close_context_menus(cx);
            // 备份节点单击：打开该库的备份列表 tab（不再展开子行）。
            // 新建备份入口保留在右键菜单；运行中任务在备份 tab 内展示。
            if group == ObjectGroup::Backup {
                this.dispatch(AppCommand::OpenBackupList(database_path_for_open.clone()), cx);
            } else {
                this.toggle_object_group_tree(connection_id, database.clone(), group, cx);
            }
            cx.stop_propagation();
        }),
    )
    .when(
        group == ObjectGroup::Tables || group == ObjectGroup::Backup,
        |this| {
            this.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    if group == ObjectGroup::Backup {
                        // 备份节点右键弹出该库的菜单（内含「备份」→ 新建备份），
                        // 而非单击就打开弹框。
                        this.show_database_context_menu(
                            DatabaseContextMenu {
                                connection_id,
                                database_path: database_path.clone(),
                                database: database_path
                                    .database
                                    .clone()
                                    .unwrap_or_else(|| database_path.name.clone()),
                                position: event.position,
                                expanded,
                                backup_only: true,
                            },
                            window,
                            cx,
                        );
                    } else {
                        this.show_table_group_context_menu(
                            TableGroupContextMenu {
                                connection_id,
                                database_path: database_path.clone(),
                                database: database_path
                                    .database
                                    .clone()
                                    .unwrap_or_else(|| database_path.name.clone()),
                                position: event.position,
                            },
                            window,
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }),
            )
        },
    )
}

fn table_folder_tree(
    parent_key: String,
    name: &str,
    indent: u8,
    expanded: bool,
    selected_table_folder: Option<&(String, String)>,
    pending_rename_table_folder: Option<&PendingRenameTableFolder>,
    rename_input: Entity<InputState>,
    search_query: Option<&str>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let name = name.to_string();
    let selected = selected_table_folder.is_some_and(|(selected_parent, selected_name)| {
        selected_parent == &parent_key && selected_name == &name
    });
    let renaming = pending_rename_table_folder.is_some_and(|pending| {
        pending.parent_key == parent_key && pending.original_name == name
    });
    let row_bg = if selected || renaming {
        colors.tree_selected
    } else {
        colors.tree_bg
    };
    let hover_bg = if selected || renaming {
        colors.tree_selected
    } else {
        colors.hover
    };
    let click_parent_key = parent_key.clone();
    let click_name = name.clone();
    let menu_parent_key = parent_key.clone();
    let menu_name = name.clone();

    // 行背景撑满整行：虚拟化列表不提供外层全宽容器（旧整树由外层 w_full 约束），
    // 故行根必须自身 w_full，否则 selected/hover 背景只包内容宽度。
    div()
        .w_full()
        .h(px(26.))
        .mx_1()
        .rounded(colors.radius_lg)
        .pl(px(6. + indent as f32 * 16.))
        .pr_2()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .text_color(colors.text)
        .bg(row_bg)
        .hover(move |style| style.bg(hover_bg))
        .when(!renaming, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.close_context_menus(cx);
                    this.toggle_table_folder_tree(click_parent_key.clone(), click_name.clone(), cx);
                    cx.stop_propagation();
                }),
            )
        })
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.select_table_folder(menu_parent_key.clone(), menu_name.clone(), cx);
                this.show_table_folder_context_menu(
                    TableFolderContextMenu {
                        parent_key: menu_parent_key.clone(),
                        name: menu_name.clone(),
                        position: event.position,
                    },
                    window,
                    cx,
                );
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(18.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(arrow_element(expanded, colors)),
        )
        .child(
            div()
                .w(px(20.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(tree_icon(if expanded { "folder-open" } else { "folder" })),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .flex()
                .items_center()
                .when(renaming, |this| {
                    this.child(
                        div()
                            .w_full()
                            .h(px(22.))
                            .rounded(colors.radius)
                            .border_1()
                            .border_color(if colors.is_dark {
                                rgb(0x5e6673)
                            } else {
                                rgb(0x9ca3af)
                            })
                            .bg(colors.input_bg)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .on_mouse_down(MouseButton::Right, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                Input::new(&rename_input)
                                    .small()
                                    .appearance(false)
                                    .focus_bordered(false)
                                    .w_full()
                                    .h_full()
                                    .px_1()
                                    .text_size(px(13.)),
                            ),
                    )
                })
                .when(!renaming, |this| {
                    this.child(highlighted_tree_label(&name, search_query, colors))
                }),
        )
}

fn table_tree(
    object: &ObjectSummary,
    indent: u8,
    active_object_path: Option<&ObjectPath>,
    connection_color_hex: &str,
    pinned: bool,
    search_query: Option<&str>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let path = object.path.clone();
    let menu_path = object.path.clone();
    let is_table = object.path.kind == ObjectKind::Table;
    let selected = active_object_path == Some(&object.path);
    tree_with_highlight(
        empty_arrow_element(),
        object_glyph(object.path.kind),
        object.path.name.clone(),
        indent,
        selected,
        pinned,
        selected.then_some(connection_color_rgba(connection_color_hex)),
        Some(connection_color_row_bg(connection_color_hex, colors)),
        search_query,
        colors,
    )
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _event: &MouseDownEvent, _, cx| {
            this.close_context_menus(cx);
            this.dispatch(AppCommand::OpenDataEditor(path.clone()), cx);
            cx.stop_propagation();
        }),
    )
    .when(is_table, |this| {
        this.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.show_table_context_menu(
                    TableContextMenu {
                        object_path: menu_path.clone(),
                        position: event.position,
                        submenu: None,
                    },
                    window,
                    cx,
                );
                cx.stop_propagation();
            }),
        )
    })
}

fn saved_query_tree(
    query: &SavedQuery,
    indent: u8,
    search_query: Option<&str>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let query_id = query.id;
    tree_with_highlight(
        empty_arrow_element(),
        "query",
        query.name.clone(),
        indent,
        false,
        false,
        None,
        None,
        search_query,
        colors,
    )
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            this.close_context_menus(cx);
            this.open_saved_query(query_id, cx);
            cx.stop_propagation();
        }),
    )
}

fn group_tree(
    group_id: ConnectionGroupId,
    name: &str,
    collapsed: bool,
    renaming: bool,
    rename_input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let name = name.to_string();

    div()
        .w_full()
        .h(px(30.))
        .mx_1()
        .rounded(colors.radius_lg)
        .border_l_4()
        .border_color(hsla(0., 0., 0., 0.))
        .pl(px(10.))
        .pr_2()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .text_color(colors.text)
        .when(renaming, |this| {
            this.bg(if colors.is_dark {
                rgb(0x202326)
            } else {
                rgb(0xf0f2f5)
            })
        })
        .hover(move |style| style.bg(colors.hover))
        .drag_over::<DraggedConnection>(move |style, _, _, _| {
            style.bg(colors.tree_selected).border_color(rgb(0xf5a400))
        })
        .on_drop(cx.listener(move |this, drag: &DraggedConnection, _, cx| {
            this.move_connection_into_group(drag.connection_id, group_id, cx);
            cx.stop_propagation();
        }))
        .when(!renaming, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.close_context_menus(cx);
                    let _ = this
                        .controller
                        .dispatch(AppCommand::ToggleConnectionGroup(group_id));
                    this.persist_sidebar_layout();
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
        })
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.show_group_context_menu(group_id, event.position, window, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(TREE_ARROW_COL_WIDTH))
                .h(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .child(arrow_element(!collapsed, colors)),
        )
        .child(
            div()
                .size(px(TREE_ICON_COL_WIDTH))
                .flex()
                .items_center()
                .justify_center()
                .child(tree_icon(if collapsed { "folder" } else { "folder-open" })),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .flex()
                .items_center()
                .when(renaming, |this| {
                    this.child(
                        div()
                            .w_full()
                            .h(px(24.))
                            .rounded(colors.radius)
                            .border_1()
                            .border_color(if colors.is_dark {
                                rgb(0x5e6673)
                            } else {
                                rgb(0x9ca3af)
                            })
                            .bg(if colors.is_dark {
                                rgb(0x22262c)
                            } else {
                                rgb(0xffffff)
                            })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .on_mouse_down(MouseButton::Right, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                Input::new(&rename_input)
                                    .small()
                                    .appearance(false)
                                    .focus_bordered(false)
                                    .w_full()
                                    .h_full()
                                    .px_1()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD),
                            ),
                    )
                })
                .when(!renaming, |this| {
                    this.font_weight(gpui::FontWeight::SEMIBOLD).child(name)
                }),
        )
}

fn connection_tree(
    connection: &ConnectionState,
    connecting: bool,
    indent: u8,
    group_id: Option<ConnectionGroupId>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let color_hex = connection_config_color_hex(&connection.config.options);
    let color = connection_color_rgba(color_hex);
    let row_bg = connection_color_row_bg(color_hex, colors);
    let connection_id = connection.config.id;
    let connected = connection.connected;
    let dragged_connection = DraggedConnection {
        connection_id,
        name: connection.config.name.clone(),
    };

    div()
        .id(("connection-tree", connection_id.0))
        .w_full()
        .h(px(30.))
        .mx_1()
        .rounded(colors.radius_lg)
        .border_l_4()
        .border_color(color)
        .bg(row_bg)
        .pl(px(10. + indent as f32 * 16.))
        .pr_2()
        .flex()
        .items_center()
        .gap_2()
        .cursor_move()
        .hover(move |style| style.bg(colors.hover))
        .on_drag(dragged_connection, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .drag_over::<DraggedConnection>(move |style, _, _, _| {
            style.bg(colors.tree_selected).border_color(rgb(0xf5a400))
        })
        .on_drop(cx.listener(move |this, drag: &DraggedConnection, _, cx| {
            this.move_connection_after(drag.connection_id, connection_id, group_id, cx);
            cx.stop_propagation();
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.close_context_menus(cx);
                if this.connecting_connections.contains(&connection_id) {
                    cx.stop_propagation();
                    return;
                }

                if connected {
                    this.dispatch(AppCommand::ToggleConnectionExpanded(connection_id), cx);
                } else if event.click_count >= 2 {
                    this.open_connection_from_sidebar(connection_id, cx);
                }
                cx.stop_propagation();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.show_connection_context_menu(connection_id, event.position, window, cx);
                cx.stop_propagation();
            }),
        )
        .child(
            div()
                .w(px(TREE_ARROW_COL_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(if connecting {
                    loading_spinner(13.).into_any_element()
                } else if connection.expanded {
                    arrow_element(true, colors)
                } else {
                    arrow_element(false, colors)
                }),
        )
        .child(
            div()
                .size(px(TREE_ICON_COL_WIDTH))
                .rounded(colors.radius)
                .bg(rgb(0x24272d))
                .flex()
                .items_center()
                .justify_center()
                .child(img(database_kind_icon_path(connection.config.kind)).size(px(13.))),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(connection.config.name.clone()),
        )
        .when(connection.connected, |this| {
            this.child(div().size(px(7.)).rounded_full().bg(rgb(0x16a34a)))
        })
}

fn connection_config_color(options: &BTreeMap<String, String>) -> gpui::Rgba {
    connection_color_rgba(connection_config_color_hex(options))
}

fn connection_config_color_hex(options: &BTreeMap<String, String>) -> &str {
    options
        .get(CONNECTION_COLOR_OPTION)
        .map(String::as_str)
        .unwrap_or(DEFAULT_CONNECTION_COLOR)
}

fn connection_color_rgba(hex: &str) -> gpui::Rgba {
    CONNECTION_COLOR_PALETTE
        .iter()
        .find(|(candidate, _)| *candidate == hex)
        .map(|(_, value)| rgb(*value))
        .unwrap_or_else(|| rgb(0x202124))
}

fn connection_color_row_bg(hex: &str, colors: UiColors) -> gpui::Rgba {
    if colors.is_dark {
        match hex {
            "#12c95b" => rgb(0x173225),
            "#fbbc04" => rgb(0x302914),
            "#ff7a00" => rgb(0x362516),
            "#ff3b45" => rgb(0x371d23),
            "#3478f6" => rgb(0x172a42),
            "#a142f4" => rgb(0x2c1d3a),
            _ => colors.panel_alt,
        }
    } else {
        match hex {
            "#12c95b" => rgb(0xe8f8ee),
            "#fbbc04" => rgb(0xfff6d9),
            "#ff7a00" => rgb(0xffeddc),
            "#ff3b45" => rgb(0xffe6e8),
            "#3478f6" => rgb(0xe7f0ff),
            "#a142f4" => rgb(0xf1e7ff),
            _ => rgb(0xe9ecef),
        }
    }
}

fn object_glyph(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Table => "table",
        ObjectKind::View => "view",
        ObjectKind::Column => "┃",
        ObjectKind::Index => "#",
        ObjectKind::Collection => "◆",
        ObjectKind::RedisKey => "●",
        ObjectKind::Database | ObjectKind::Schema | ObjectKind::RedisDb => "database",
    }
}

fn group_objects<'a>(
    connection: &'a ConnectionState,
    database: &str,
    schema: Option<&str>,
    group: ObjectGroup,
) -> Vec<&'a ObjectSummary> {
    connection
        .objects
        .iter()
        .filter(|object| object.path.database.as_deref().unwrap_or("main") == database)
        // PG 按 schema 分桶；非 schema 数据库传 `None` 不额外过滤。
        .filter(|object| match schema {
            Some(schema) => object.path.schema.as_deref() == Some(schema),
            None => true,
        })
        .filter(|object| match group {
            ObjectGroup::Tables => object.path.kind == ObjectKind::Table,
            ObjectGroup::Views => object.path.kind == ObjectKind::View,
            ObjectGroup::Queries
            | ObjectGroup::Procedures
            | ObjectGroup::Functions
            | ObjectGroup::Backup => false,
        })
        .collect()
}

fn sorted_group_objects<'a>(
    connection: &'a ConnectionState,
    database: &str,
    schema: Option<&str>,
    group: ObjectGroup,
    pinned_tables: &BTreeSet<String>,
) -> Vec<&'a ObjectSummary> {
    let mut objects = group_objects(connection, database, schema, group);
    objects.sort_by_key(|object| (!pinned_tables.contains(&table_tree_key(&object.path)), &object.path.name));
    objects
}

fn sorted_unassigned_group_objects<'a>(
    connection: &'a ConnectionState,
    database: &str,
    schema: Option<&str>,
    group: ObjectGroup,
    pinned_tables: &BTreeSet<String>,
    table_folder_assignments: &BTreeMap<String, (String, String)>,
) -> Vec<&'a ObjectSummary> {
    sorted_group_objects(connection, database, schema, group, pinned_tables)
        .into_iter()
        .filter(|object| !table_folder_assignments.contains_key(&table_tree_key(&object.path)))
        .collect()
}

fn sorted_folder_table_objects<'a>(
    connection: &'a ConnectionState,
    database: &str,
    folder_parent_key: &str,
    folder: &str,
    pinned_tables: &BTreeSet<String>,
    table_folder_assignments: &BTreeMap<String, (String, String)>,
) -> Vec<&'a ObjectSummary> {
    sorted_group_objects(connection, database, None, ObjectGroup::Tables, pinned_tables)
        .into_iter()
        .filter(|object| {
            table_folder_assignments
                .get(&table_tree_key(&object.path))
                .is_some_and(|(parent_key, assigned_folder)| {
                    parent_key == folder_parent_key && assigned_folder == folder
                })
        })
        .collect()
}

fn table_folder_parent_key(connection_id: ConnectionId, database: &str) -> String {
    format!("{}:{database}:tables", connection_id.0)
}

fn table_folder_tree_key(parent_key: &str, folder: &str) -> String {
    format!("{parent_key}:folder:{folder}")
}

fn table_folder_parent_key_for_object(path: &ObjectPath) -> String {
    table_folder_parent_key(path.connection_id, object_path_database_name(path))
}

fn sorted_table_folders<'a>(
    table_folders: &'a BTreeMap<String, Vec<String>>,
    connection_id: ConnectionId,
    database: &str,
) -> Vec<&'a String> {
    sorted_table_folders_for_parent(
        table_folders,
        &table_folder_parent_key(connection_id, database),
    )
}

fn sorted_table_folders_for_parent<'a>(
    table_folders: &'a BTreeMap<String, Vec<String>>,
    parent_key: &str,
) -> Vec<&'a String> {
    table_folders
        .get(parent_key)
        .map(|folders| folders.iter().collect::<Vec<_>>())
        .unwrap_or_default()
}

fn move_table_folder_name(folders: &mut [String], name: &str, direction: isize) {
    let Some(index) = folders.iter().position(|folder| folder == name) else {
        return;
    };
    let next = index.saturating_add_signed(direction);
    if next < folders.len() {
        folders.swap(index, next);
    }
}

fn next_table_folder_name(existing: &[String]) -> String {
    let mut name = "新建组".to_string();
    let mut index = 1;
    while existing.iter().any(|existing| existing == &name) {
        name = format!("新建组 {index}");
        index += 1;
    }
    name
}

fn saved_queries_for_database<'a>(
    queries: &'a [SavedQuery],
    connection_id: ConnectionId,
    database: &str,
) -> Vec<&'a SavedQuery> {
    queries
        .iter()
        .filter(|query| query.connection_id == connection_id)
        .filter(|query| query.database.as_deref().unwrap_or("main") == database)
        .collect()
}

fn connection_databases(connection: &ConnectionState) -> Vec<ObjectSummary> {
    let visible_databases = configured_visible_databases(&connection.config.options);
    let mut databases: BTreeMap<String, ObjectSummary> = BTreeMap::new();
    for object in &connection.objects {
        match object.path.kind {
            ObjectKind::Database | ObjectKind::Schema | ObjectKind::RedisDb => {
                let database = object
                    .path
                    .database
                    .clone()
                    .unwrap_or_else(|| object.path.name.clone());
                databases.entry(database).or_insert_with(|| object.clone());
            }
            ObjectKind::Table
            | ObjectKind::View
            | ObjectKind::Collection
            | ObjectKind::RedisKey => {
                let database = object
                    .path
                    .database
                    .clone()
                    .unwrap_or_else(|| "main".to_string());
                databases
                    .entry(database.clone())
                    .or_insert_with(|| ObjectSummary {
                        path: ObjectPath {
                            connection_id: connection.config.id,
                            database: Some(database.clone()),
                            schema: None,
                            name: database,
                            kind: ObjectKind::Database,
                        },
                        rows: None,
                        modified_at: None,
                        comment: None,
                    });
            }
            ObjectKind::Column | ObjectKind::Index => {}
        }
    }

    databases
        .into_values()
        .filter(|database| {
            let name = database_display_name(database);
            if let Some(visible) = &visible_databases {
                visible.contains(&name)
            } else {
                !is_system_database(&name)
            }
        })
        .collect()
}

/// 该库已加载的表/视图按 schema 分桶的升序 schema 名；无 schema（MySQL/SQLite/Redis）返回空。
///
/// PG 对象 `path.schema` 为 `Some`；空 schema / "main" 等非 schema 数据库视为未分桶。
fn database_schemas(connection: &ConnectionState, database: &str) -> Vec<String> {
    let mut schemas: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for object in &connection.objects {
        let matches_db = object.path.database.as_deref().unwrap_or("main") == database;
        let is_relation = matches!(
            object.path.kind,
            ObjectKind::Table | ObjectKind::View
        );
        if matches_db && is_relation {
            if let Some(schema) = object.path.schema.as_deref() {
                if !schema.is_empty() {
                    schemas.insert(schema.to_string());
                }
            }
        }
    }
    schemas.into_iter().collect()
}

fn all_connection_database_names(connection: &ConnectionState) -> Vec<String> {
    let mut names = connection_databases_unfiltered(connection)
        .into_iter()
        .map(|database| database_display_name(&database))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn connection_databases_unfiltered(connection: &ConnectionState) -> Vec<ObjectSummary> {
    let mut connection = connection.clone();
    connection.config.options.remove(VISIBLE_DATABASES_OPTION);
    connection_databases(&connection)
}

fn database_display_name(database: &ObjectSummary) -> String {
    database
        .path
        .database
        .clone()
        .unwrap_or_else(|| database.path.name.clone())
}

fn configured_visible_databases(options: &BTreeMap<String, String>) -> Option<BTreeSet<String>> {
    options.get(VISIBLE_DATABASES_OPTION).and_then(|value| {
        let databases = value
            .lines()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        (!databases.is_empty()).then_some(databases)
    })
}

fn is_system_database(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "information_schema" | "mysql" | "performance_schema" | "sys" | "admin" | "local"
    )
}

fn connection_has_loaded_children(connection: &ConnectionState, database: &str) -> bool {
    connection.objects.iter().any(|object| {
        matches!(
            object.path.kind,
            ObjectKind::Table | ObjectKind::View | ObjectKind::RedisKey
        )
            && object.path.database.as_deref().unwrap_or("main") == database
    })
}

fn database_tree_key(connection_id: ConnectionId, database: &str) -> String {
    format!("{}:{database}", connection_id.0)
}

fn table_tree_key(path: &ObjectPath) -> String {
    format!(
        "{}:{}:{}:{}",
        path.connection_id.0,
        path.database.as_deref().unwrap_or("main"),
        path.schema.as_deref().unwrap_or(""),
        path.name.as_str()
    )
}

fn object_group_tree_key(
    connection_id: ConnectionId,
    database: &str,
    group: ObjectGroup,
) -> String {
    format!("{}:{database}:{}", connection_id.0, group.key())
}

/// 分组在 schema 作用域下的 key：PG 按 schema 分桶后，同一数据库不同 schema 的
/// “Tables/Views” 分组必须互不共享展开态，故 key 带 schema。
fn object_group_tree_key_scoped(
    connection_id: ConnectionId,
    database: &str,
    schema: Option<&str>,
    group: ObjectGroup,
) -> String {
    match schema {
        Some(schema) => object_group_tree_key(connection_id, &format!("{database}.{schema}"), group),
        None => object_group_tree_key(connection_id, database, group),
    }
}

/// PostgreSQL schema 行 key：连接 + 库 + schema，保证同名 schema 分属不同库时唯一。
fn schema_tree_key(connection_id: ConnectionId, database: &str, schema: &str) -> String {
    format!("{}:{database}:{schema}", connection_id.0)
}

fn table_header(colors: UiColors) -> impl IntoElement {
    div()
        .h(px(28.))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .flex()
        .items_center()
        .text_size(px(15.))
        .text_color(colors.muted)
        .child(cell("名称", 210., colors))
        .child(cell("行", 55., colors))
        .child(cell("修改日期", 150., colors))
        .child(cell_flex("注释", colors))
}

fn object_row(
    object: &ObjectSummary,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let path = object.path.clone();

    table_row(
        object.path.name.clone(),
        object.rows.map(|rows| rows.to_string()).unwrap_or_default(),
        object.modified_at.clone().unwrap_or_default(),
        object.comment.clone().unwrap_or_default(),
        object.path.name == "Product",
        colors,
    )
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            if event.click_count >= 2 {
                this.dispatch(AppCommand::OpenDataEditor(path.clone()), cx);
                cx.stop_propagation();
            }
        }),
    )
}

fn visible_objects(state: &AppState) -> Vec<&ObjectSummary> {
    state
        .connections
        .iter()
        .flat_map(|connection| connection.objects.iter())
        .collect()
}

fn database_kind_count(state: &AppState) -> usize {
    let mut kinds = Vec::new();
    for connection in &state.connections {
        if !kinds.contains(&connection.config.kind) {
            kinds.push(connection.config.kind);
        }
    }
    kinds.len()
}

fn connection_summary(config: &ConnectionConfig) -> String {
    match &config.endpoint {
        Endpoint::Tcp {
            host,
            port,
            database,
        } => format!(
            "{} · {}:{}{}",
            database_kind_name(config.kind),
            host,
            port,
            database
                .as_deref()
                .filter(|database| !database.is_empty())
                .map(|database| format!(" / {database}"))
                .unwrap_or_default()
        ),
        Endpoint::SqliteFile { path, .. } => {
            format!("{} · {}", database_kind_name(config.kind), path.display())
        }
        Endpoint::Uri { uri } => format!("{} · {}", database_kind_name(config.kind), uri),
    }
}

fn database_kind_name(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "MySQL",
        DatabaseKind::TiDb => "TiDB",
        DatabaseKind::Sqlite => "SQLite",
        DatabaseKind::MongoDb => "MongoDB",
        DatabaseKind::Redis => "Redis",
        DatabaseKind::Postgres => "PostgreSQL",
    }
}

fn table_row(
    name: String,
    rows: String,
    changed: String,
    note: String,
    selected: bool,
    colors: UiColors,
) -> Div {
    let bg = if selected {
        rgb(0x1f73db)
    } else {
        colors.panel_bg
    };
    let fg = if selected {
        rgb(0xffffff)
    } else {
        rgb(0x202124)
    };
    div()
        .h(px(24.))
        .bg(bg)
        .text_color(fg)
        .flex()
        .items_center()
        .text_size(px(16.))
        .child(cell_with_icon(name, 210., colors))
        .child(cell(rows, 55., colors))
        .child(cell(changed, 150., colors))
        .child(cell_flex(note, colors))
}

fn cell(text: impl Into<String>, width: f32, colors: UiColors) -> impl IntoElement {
    div()
        .w(px(width))
        .h_full()
        .px_4()
        .flex()
        .items_center()
        .overflow_hidden()
        .border_r_1()
        .border_color(colors.border_soft)
        .child(text.into())
}

fn cell_flex(text: impl Into<String>, colors: UiColors) -> impl IntoElement {
    div()
        .flex_1()
        .h_full()
        .px_4()
        .flex()
        .items_center()
        .overflow_hidden()
        .text_color(colors.text)
        .child(text.into())
}

fn cell_with_icon(text: impl Into<String>, width: f32, colors: UiColors) -> impl IntoElement {
    div()
        .w(px(width))
        .h_full()
        .px_4()
        .flex()
        .items_center()
        .gap_2()
        .overflow_hidden()
        .border_r_1()
        .border_color(colors.border_soft)
        .child(table_icon(18., colors))
        .child(text.into())
}

#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use std::ffi::c_void;

    use cocoa::{
        appkit::{NSApp, NSApplication, NSImage},
        base::{id, nil},
        foundation::{NSData, NSUInteger},
    };

    const APP_ICON: &[u8] = include_bytes!("../../assets/app-icon.icns");

    unsafe {
        let data = NSData::dataWithBytes_length_(
            nil,
            APP_ICON.as_ptr() as *const c_void,
            APP_ICON.len() as NSUInteger,
        );
        let image: id = NSImage::initWithData_(NSImage::alloc(nil), data);

        if image != nil {
            NSApp().setApplicationIconImage_(image);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon() {}

fn startup_window_bounds(cx: &App) -> Bounds<Pixels> {
    let desired_size = size(px(2048.), px(1024.));
    let Some(display) = cx.primary_display() else {
        return Bounds::centered(None, desired_size, cx);
    };

    let display_bounds = display.bounds();
    let margin = px(32.);
    let max_width = if display_bounds.size.width > margin * 2.0 {
        display_bounds.size.width - margin * 2.0
    } else {
        display_bounds.size.width
    };
    let max_height = if display_bounds.size.height > margin * 2.0 {
        display_bounds.size.height - margin * 2.0
    } else {
        display_bounds.size.height
    };
    let window_size: Size<Pixels> = size(
        desired_size.width.min(max_width),
        desired_size.height.min(max_height),
    );

    Bounds::centered_at(display_bounds.center(), window_size)
}

fn theme_mode_from_app_theme(theme: AppTheme) -> ThemeMode {
    match theme {
        AppTheme::Dark => ThemeMode::Dark,
        AppTheme::System | AppTheme::Light => ThemeMode::Light,
    }
}

fn app_theme_from_mode(mode: ThemeMode) -> AppTheme {
    match mode {
        ThemeMode::Dark => AppTheme::Dark,
        ThemeMode::Light => AppTheme::Light,
    }
}

/// sidebar 虚拟化拉平：连接浏览器树的行种类。
use fluxdb_core::SidebarLayout;

