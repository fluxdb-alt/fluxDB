fn group_context_menu(
    menu: GroupContextMenu,
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let group_name = state
        .sidebar_layout
        .groups
        .iter()
        .find(|group| group.id == menu.group_id)
        .map(|group| group.name.clone())
        .unwrap_or_else(|| "分组".to_string());

    div()
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(210.))
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
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(group_name),
        )
        .child(connection_menu_separator(colors))
        .child(group_menu_item(
            "新建连接",
            "plus",
            false,
            GroupMenuAction::NewConnection,
            menu.group_id,
            colors,
            cx,
        ))
        .child(group_menu_item(
            "重命名分组",
            "edit",
            false,
            GroupMenuAction::Rename,
            menu.group_id,
            colors,
            cx,
        ))
        .child(connection_menu_separator(colors))
        .child(group_menu_item(
            "删除分组",
            "delete",
            true,
            GroupMenuAction::Delete,
            menu.group_id,
            colors,
            cx,
        ))
}

fn connection_menu_separator(colors: UiColors) -> Div {
    div().h(px(1.)).mx_1().my_1().bg(colors.border_soft)
}

fn menu_surface_bg(colors: UiColors) -> gpui::Rgba {
    if colors.is_dark {
        rgb(0x181b20)
    } else {
        rgb(0xffffff)
    }
}

fn menu_icon(icon: &'static str, destructive: bool, colors: UiColors) -> gpui::AnyElement {
    let color = if destructive {
        rgb(0xff4d57)
    } else {
        colors.text
    };
    let mapped_icon = match icon {
        "connect" | "disconnect" => Some(AppIcon::Plug),
        "broadcast" => Some(AppIcon::Broadcast),
        "database" => Some(AppIcon::Database),
        "pin" => Some(AppIcon::Pin),
        "query" | "terminal" => Some(AppIcon::Query),
        "users" => Some(AppIcon::Users),
        "file-sql" => Some(AppIcon::FileSql),
        "file-search" => Some(AppIcon::FileSearch),
        "table" => Some(AppIcon::Table),
        "default" => Some(AppIcon::Check),
        "plus" | "folder-plus" => Some(AppIcon::Plus),
        "move-group" => Some(AppIcon::FolderInput),
        "ungroup" => Some(AppIcon::FolderUp),
        "folder" => Some(AppIcon::Folder),
        "refresh" => Some(AppIcon::Refresh),
        "filter" => Some(AppIcon::Filter),
        "edit" => Some(AppIcon::Edit),
        "copy" => Some(AppIcon::Copy),
        "save" => Some(AppIcon::Save),
        "delete" => Some(AppIcon::Trash),
        _ => None,
    };

    if let Some(icon) = mapped_icon {
        app_icon(icon, 18., color)
    } else {
        div()
            .size(px(18.))
            .text_color(color)
            .child(icon)
            .into_any_element()
    }
}

fn tab_menu_item(
    label: impl Into<String>,
    icon: &'static str,
    destructive: bool,
    action: Option<TabMenuAction>,
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let label = label.into();
    let enabled = action.is_some();
    div()
        .id((
            "tab-menu-item",
            menu_item_hash(ConnectionId(tab_id.0), icon, &label),
        ))
        .h(px(36.))
        .rounded(colors.radius_lg)
        .px_1p5()
        .flex()
        .items_center()
        .gap_2()
        .text_color(if !enabled {
            colors.muted
        } else if destructive {
            rgb(0xe5484d)
        } else {
            colors.text
        })
        .opacity(if enabled { 1. } else { 0.55 })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
        })
        .when_some(action, |this, action| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.handle_tab_menu_action(action, tab_id, cx);
                    cx.stop_propagation();
                }),
            )
        })
        .child(
            div()
                .w(px(22.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(tab_menu_icon(icon, destructive, enabled, colors)),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .flex()
                .items_center()
                .line_height(px(20.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label),
        )
}

fn tab_menu_icon(
    icon: &'static str,
    destructive: bool,
    enabled: bool,
    colors: UiColors,
) -> gpui::AnyElement {
    let color = if !enabled {
        colors.muted
    } else if destructive {
        rgb(0xff4d57)
    } else {
        colors.text
    };

    let mapped_icon = match icon {
        "copy" => Some(AppIcon::Copy),
        "pin" => Some(AppIcon::Pin),
        "x" => Some(AppIcon::Close),
        _ => None,
    };

    match mapped_icon {
        Some(icon) => app_icon(icon, 18., color),
        _ => div()
            .size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(20.))
            .line_height(px(20.))
            .text_color(color)
            .child(icon)
            .into_any_element(),
    }
}

fn group_menu_item(
    label: &'static str,
    icon: &'static str,
    destructive: bool,
    action: GroupMenuAction,
    group_id: ConnectionGroupId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(32.))
        .rounded(colors.radius_lg)
        .px_2()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .text_color(if destructive {
            rgb(0xe5484d)
        } else {
            colors.text
        })
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.handle_group_menu_action(action, group_id, window, cx);
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
                .child(menu_icon(icon, destructive, colors)),
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

fn connection_menu_item(
    label: impl Into<String>,
    icon: &'static str,
    shortcut: Option<&'static str>,
    destructive: bool,
    action: Option<ConnectionMenuAction>,
    connection_id: ConnectionId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let label = label.into();
    let enabled = action.is_some();
    let hides_group_submenu = !matches!(
        action,
        Some(ConnectionMenuAction::MoveToGroup(_) | ConnectionMenuAction::MoveToNewGroup)
    );
    div()
        .id((
            "connection-menu-item",
            menu_item_hash(connection_id, icon, &label),
        ))
        .h(px(32.))
        .rounded(colors.radius_lg)
        .px_2()
        .flex()
        .items_center()
        .gap_3()
        .text_color(if !enabled {
            colors.muted
        } else if destructive {
            rgb(0xe5484d)
        } else {
            colors.text
        })
        .opacity(if enabled { 1. } else { 0.55 })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
        })
        .on_hover(cx.listener(move |this, hovered, _, cx| {
            if *hovered && hides_group_submenu {
                if let Some(active_menu) = &mut this.connection_context_menu {
                    if active_menu.connection_id == connection_id && active_menu.show_group_submenu
                    {
                        active_menu.show_group_submenu = false;
                        cx.notify();
                    }
                }
            }
        }))
        .when_some(action, |this, action| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.handle_connection_menu_action(action, connection_id, window, cx);
                    cx.stop_propagation();
                }),
            )
        })
        .child(
            div()
                .w(px(22.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(menu_icon(icon, destructive, colors)),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label),
        )
        .when_some(shortcut, |this, shortcut| {
            this.child(
                div()
                    .h(px(22.))
                    .px_2()
                    .rounded(colors.radius_lg)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.panel_alt)
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(colors.muted)
                    .child(shortcut),
            )
        })
}

fn database_menu_item(
    label: impl Into<String>,
    icon: &'static str,
    destructive: bool,
    action: DatabaseMenuAction,
    menu: DatabaseContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id((
            "database-menu-item",
            menu_item_hash(menu.connection_id, icon, &label),
        ))
        .h(px(32.))
        .rounded(colors.radius_lg)
        .px_2()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .text_color(if destructive {
            rgb(0xe5484d)
        } else {
            colors.text
        })
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.handle_database_menu_action(action, menu.clone(), window, cx);
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
                .child(menu_icon(icon, destructive, colors)),
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

fn menu_item_hash(connection_id: ConnectionId, icon: &str, label: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    connection_id.hash(&mut hasher);
    icon.hash(&mut hasher);
    label.hash(&mut hasher);
    hasher.finish()
}
