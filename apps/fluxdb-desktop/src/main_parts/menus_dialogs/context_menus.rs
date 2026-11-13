fn context_menu_backdrop(cx: &mut Context<NavicatMain>) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_context_menus(cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, _, _, cx| {
                this.close_context_menus(cx);
                cx.stop_propagation();
            }),
        )
}

fn tab_context_menu(
    menu: TabContextMenu,
    state: &AppState,
    pinned_tabs: &BTreeSet<TabId>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let scoped_tab_ids = tab_context_menu_close_targets(
        state
            .tabs
            .iter()
            .map(|tab| (tab.id, tab_first_level_scope(tab))),
        menu.tab_id,
        true,
    );
    let other_tab_ids = tab_context_menu_close_targets(
        state
            .tabs
            .iter()
            .map(|tab| (tab.id, tab_first_level_scope(tab))),
        menu.tab_id,
        false,
    );
    let close_others_action = (!other_tab_ids.is_empty()).then_some(TabMenuAction::CloseOthers);
    let pin_label = if pinned_tabs.contains(&menu.tab_id) {
        tr("app.context.unpin").to_string()
    } else {
        tr("app.context.pin").to_string()
    };

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
        .text_size(ui_text_size(14.))
        .text_color(colors.text)
        .key_context("TabContextMenu")
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(tab_menu_item(
            tr("app.context.copy_table_name").to_string(),
            "copy",
            false,
            Some(TabMenuAction::CopyTableName),
            menu.tab_id,
            colors,
            cx,
        ))
        .child(connection_menu_separator(colors))
        .child(tab_menu_item(
            pin_label,
            "pin",
            false,
            Some(TabMenuAction::Pin),
            menu.tab_id,
            colors,
            cx,
        ))
        .child(connection_menu_separator(colors))
        .child(tab_menu_item(
            tr("app.context.close_tab").to_string(),
            "x",
            false,
            Some(TabMenuAction::Close),
            menu.tab_id,
            colors,
            cx,
        ))
        .child(tab_menu_item(
            tr("app.context.close_other_tabs").to_string(),
            "x",
            false,
            close_others_action,
            menu.tab_id,
            colors,
            cx,
        ))
        .child(tab_menu_item(
            tr("app.context.close_all_tabs").to_string(),
            "x",
            true,
            (!scoped_tab_ids.is_empty()).then_some(TabMenuAction::CloseAll),
            menu.tab_id,
            colors,
            cx,
        ))
}

fn table_group_context_menu(
    menu: TableGroupContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(188.))
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
            hsla(0., 0., 0., 0.18),
        )])
        .p_1()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .key_context("TableGroupContextMenu")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(table_group_menu_item(
            tr("app.context.new_table").to_string(),
            AppIcon::Plus,
            TableGroupMenuAction::NewTable,
            menu.clone(),
            colors,
            cx,
        ))
        .child(table_group_menu_item(
            tr("app.context.new_group").to_string(),
            AppIcon::Folder,
            TableGroupMenuAction::NewGroup,
            menu.clone(),
            colors,
            cx,
        ))
        .child(table_group_menu_item(
            tr("app.context.refresh").to_string(),
            AppIcon::Refresh,
            TableGroupMenuAction::Refresh,
            menu,
            colors,
            cx,
        ))
}

fn table_group_menu_item(
    label: impl Into<String>,
    icon: AppIcon,
    action: TableGroupMenuAction,
    menu: TableGroupContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    table_inert_menu_item(label, icon, false, colors).on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, window, cx| {
            this.handle_table_group_menu_action(action, menu.clone(), window, cx);
            cx.stop_propagation();
        }),
    )
}

fn table_folder_context_menu(
    menu: TableFolderContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(188.))
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
            hsla(0., 0., 0., 0.18),
        )])
        .p_1()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .key_context("TableFolderContextMenu")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(table_folder_menu_item(
            tr("app.context.move_up").to_string(),
            AppIcon::ChevronUp,
            false,
            TableFolderMenuAction::MoveUp,
            menu.clone(),
            colors,
            cx,
        ))
        .child(table_folder_menu_item(
            tr("app.context.move_down").to_string(),
            AppIcon::ChevronDown,
            false,
            TableFolderMenuAction::MoveDown,
            menu.clone(),
            colors,
            cx,
        ))
        .child(table_folder_menu_item(
            tr("app.context.rename").to_string(),
            AppIcon::Edit,
            false,
            TableFolderMenuAction::Rename,
            menu.clone(),
            colors,
            cx,
        ))
        .child(table_folder_menu_item(
            tr("app.context.delete_group").to_string(),
            AppIcon::Trash,
            true,
            TableFolderMenuAction::Delete,
            menu,
            colors,
            cx,
        ))
}

fn table_folder_menu_item(
    label: impl Into<String>,
    icon: AppIcon,
    destructive: bool,
    action: TableFolderMenuAction,
    menu: TableFolderContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    table_inert_menu_item(label, icon, destructive, colors).on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, window, cx| {
            this.handle_table_folder_menu_action(action, menu.clone(), window, cx);
            cx.stop_propagation();
        }),
    )
}

fn table_context_menu(
    menu: TableContextMenu,
    pinned_tables: &BTreeSet<String>,
    table_folders: &BTreeMap<String, Vec<String>>,
    table_folder_assignments: &BTreeMap<String, (String, String)>,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let pin_label = if pinned_tables.contains(&table_tree_key(&menu.object_path)) {
        tr("app.context.unpin")
    } else {
        tr("app.context.pin")
    };

    div()
        .debug_selector(|| "table-context-menu".into())
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(212.))
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
            hsla(0., 0., 0., 0.18),
        )])
        .p_1()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .key_context("TableContextMenu")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(table_menu_item(
            pin_label.to_string(),
            AppIcon::Pin,
            false,
            Some(TableMenuAction::TogglePin),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.copy_name").to_string(),
            AppIcon::Copy,
            false,
            Some(TableMenuAction::CopyName),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.set_shortcut").to_string(),
            AppIcon::Key,
            false,
            Some(TableMenuAction::SetShortcut),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.view_data").to_string(),
            AppIcon::Eye,
            false,
            Some(TableMenuAction::ViewData),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.open_er").to_string(),
            AppIcon::Schema,
            false,
            Some(TableMenuAction::OpenErDiagram),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.design_table").to_string(),
            AppIcon::Settings,
            false,
            Some(TableMenuAction::Design),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.new_table").to_string(),
            AppIcon::Plus,
            false,
            Some(TableMenuAction::NewTable),
            &menu,
            colors,
            cx,
        ))
        .child(data_cell_menu_separator(colors))
        .child(table_menu_item(
            tr("app.context.refresh").to_string(),
            AppIcon::Refresh,
            false,
            Some(TableMenuAction::Refresh),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.rename").to_string(),
            AppIcon::Edit,
            false,
            Some(TableMenuAction::Rename),
            &menu,
            colors,
            cx,
        ))
        .child(data_cell_menu_separator(colors))
        .child(table_menu_item(
            tr("app.context.copy_table").to_string(),
            AppIcon::Copy,
            false,
            Some(TableMenuAction::CopyTable),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.copy_structure").to_string(),
            AppIcon::Table,
            false,
            Some(TableMenuAction::CopyStructure),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.backup").to_string(),
            AppIcon::Save,
            false,
            Some(TableMenuAction::Backup),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.import_data").to_string(),
            AppIcon::FolderInput,
            false,
            Some(TableMenuAction::ImportData),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_submenu_item(
            tr("app.context.manage_groups").to_string(),
            AppIcon::FolderInput,
            TableContextSubmenu::ManageGroup,
            &menu,
            colors,
            cx,
        ))
        .when(menu.submenu == Some(TableContextSubmenu::ManageGroup), |this| {
            this.child(table_manage_group_submenu(
                &menu,
                table_folders,
                table_folder_assignments,
                colors,
                window,
                cx,
            ))
        })
        .child(table_menu_submenu_item(
            tr("app.context.export").to_string(),
            AppIcon::Save,
            TableContextSubmenu::Export,
            &menu,
            colors,
            cx,
        ))
        .when(menu.submenu == Some(TableContextSubmenu::Export), |this| {
            this.child(table_export_submenu(&menu, colors, window, cx))
        })
        .child(data_cell_menu_separator(colors))
        .child(table_menu_item(
            tr("app.context.drop_table").to_string(),
            AppIcon::Trash,
            true,
            Some(TableMenuAction::Drop),
            &menu,
            colors,
            cx,
        ))
        .child(table_menu_item(
            tr("app.context.truncate_table").to_string(),
            AppIcon::Trash,
            true,
            Some(TableMenuAction::Truncate),
            &menu,
            colors,
            cx,
        ))
}

/// MongoDB 集合/视图右键菜单。视图只读：不渲染重命名/导入/删除。
fn collection_context_menu(
    menu: CollectionContextMenu,
    pinned_tables: &BTreeSet<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let pin_label = if pinned_tables.contains(&table_tree_key(&menu.object_path)) {
        tr("app.context.unpin")
    } else {
        tr("app.context.pin")
    };
    div()
        .debug_selector(|| "collection-context-menu".into())
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(208.))
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
            hsla(0., 0., 0., 0.18),
        )])
        .p_1()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .key_context("CollectionContextMenu")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(collection_menu_item(
            pin_label.to_string(),
            AppIcon::Pin,
            false,
            CollectionMenuAction::TogglePin,
            &menu,
            colors,
            cx,
        ))
        .child(collection_menu_item(
            tr("app.context.copy_name").to_string(),
            AppIcon::Copy,
            false,
            CollectionMenuAction::CopyName,
            &menu,
            colors,
            cx,
        ))
        .child(data_cell_menu_separator(colors))
        .child(collection_menu_item(
            tr("app.context.refresh").to_string(),
            AppIcon::Refresh,
            false,
            CollectionMenuAction::Refresh,
            &menu,
            colors,
            cx,
        ))
        .child(collection_menu_item(
            tr("app.context.export").to_string(),
            AppIcon::Save,
            false,
            CollectionMenuAction::Export,
            &menu,
            colors,
            cx,
        ))
        .when(!menu.read_only, |this| {
            this.child(collection_menu_item(
                tr("app.context.rename").to_string(),
                AppIcon::Edit,
                false,
                CollectionMenuAction::Rename,
                &menu,
                colors,
                cx,
            ))
            .child(collection_menu_item(
                tr("app.context.import_data").to_string(),
                AppIcon::FolderInput,
                false,
                CollectionMenuAction::Import,
                &menu,
                colors,
                cx,
            ))
            .child(data_cell_menu_separator(colors))
            .child(collection_menu_item(
                tr("app.mongo.drop_collection.action").to_string(),
                AppIcon::Trash,
                true,
                CollectionMenuAction::Drop,
                &menu,
                colors,
                cx,
            ))
        })
}

fn collection_menu_item(
    label: impl Into<String>,
    icon: AppIcon,
    destructive: bool,
    action: CollectionMenuAction,
    menu: &CollectionContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let label = label.into();
    let selector = format!("collection-menu-item:{label}");
    let menu = menu.clone();
    table_inert_menu_item(label, icon, destructive, colors)
        .debug_selector(move || selector)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.handle_collection_menu_action(action, menu.clone(), window, cx);
                cx.stop_propagation();
            }),
        )
}

fn table_export_submenu(
    menu: &TableContextMenu,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // "导出"是第 13 项（其前有 12 个菜单项 + 2 个分隔符）；子菜单 2 项，贴底时向上收回
    let desired_top = f32::from(context_submenu_top(12., 2.));
    let height = 2. * 26. + 8.;
    table_submenu_shell(
        context_submenu_clamped_top(menu.position.y, desired_top, height, window),
        colors,
    )
        .child(table_submenu_action_item(
            tr("app.context.export_database").to_string(),
            AppIcon::Database,
            TableMenuAction::ExportDatabase,
            menu,
            colors,
            cx,
        ))
        .child(table_submenu_action_item(
            tr("app.context.export_data").to_string(),
            AppIcon::Save,
            TableMenuAction::ExportData,
            menu,
            colors,
            cx,
        ))
}

fn table_manage_group_submenu(
    menu: &TableContextMenu,
    table_folders: &BTreeMap<String, Vec<String>>,
    table_folder_assignments: &BTreeMap<String, (String, String)>,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let parent_key = table_folder_parent_key_for_object(&menu.object_path);
    let folders = sorted_table_folders_for_parent(table_folders, &parent_key);
    let table_key = table_tree_key(&menu.object_path);
    let in_group = table_folder_assignments.contains_key(&table_key);
    // "管理组"是第 12 项（其前有 11 个菜单项 + 2 个分隔符）；
    // 分组数量不定，子菜单贴底时按实际高度向上收回，避免超出视口。
    let desired_top = f32::from(context_submenu_top(11., 2.));
    let submenu_items = if in_group { 1. } else { 0. }
        + if folders.is_empty() { 1. } else { folders.len() as f32 };
    let submenu_separators = if in_group { 1. } else { 0. };
    let submenu_height = 8. + submenu_items * 26. + submenu_separators * 9.;
    let mut submenu = table_submenu_shell(
        context_submenu_clamped_top(menu.position.y, desired_top, submenu_height, window),
        colors,
    );
    if in_group {
        submenu = submenu.child(table_submenu_action_item(
            tr("app.context.remove_from_group").to_string(),
            AppIcon::FolderUp,
            TableMenuAction::RemoveFromGroup,
            menu,
            colors,
            cx,
        ));
        submenu = submenu.child(data_cell_menu_separator(colors));
    }
    if folders.is_empty() {
        return submenu.child(table_inert_menu_item(
            tr("app.context.no_groups").to_string(),
            AppIcon::Folder,
            false,
            colors,
        ));
    }
    for folder in folders {
        submenu = submenu.child(table_folder_assignment_item(
            folder,
            &parent_key,
            menu,
            colors,
            cx,
        ));
    }
    submenu
}

fn table_folder_assignment_item(
    folder: &str,
    parent_key: &str,
    menu: &TableContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let folder = folder.to_string();
    let parent_key = parent_key.to_string();
    let object_path = menu.object_path.clone();
    table_inert_menu_item(&folder, AppIcon::Folder, false, colors)
        .id(SharedString::from(format!("table-folder-assignment:{folder}")))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.assign_table_to_folder(object_path.clone(), parent_key.clone(), folder.clone(), cx);
                cx.stop_propagation();
            }),
        )
}

fn table_submenu_shell(top: Pixels, colors: UiColors) -> Div {
    div()
        .absolute()
        .left(px(210.))
        .top(top)
        .w(px(160.))
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
            hsla(0., 0., 0., 0.18),
        )])
        .p_1()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

fn table_menu_item(
    label: impl Into<String>,
    icon: AppIcon,
    destructive: bool,
    action: Option<TableMenuAction>,
    menu: &TableContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let label = label.into();
    let selector = format!("table-menu-item:{label}");
    let object_path = menu.object_path.clone();
    let action_path = menu.object_path.clone();
    table_inert_menu_item(label, icon, destructive, colors)
        .debug_selector(move || selector)
        .on_mouse_move(cx.listener(move |this, _, _, cx| {
            this.set_table_context_submenu(object_path.clone(), None, cx);
            cx.stop_propagation();
        }))
        .when_some(action, |this, action| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.handle_table_menu_action(action, action_path.clone(), window, cx);
                    cx.stop_propagation();
                }),
            )
        })
}

fn table_menu_submenu_item(
    label: impl Into<String>,
    icon: AppIcon,
    submenu: TableContextSubmenu,
    menu: &TableContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let label = label.into();
    let selector = format!("table-menu-item:{label}");
    let object_path = menu.object_path.clone();
    table_inert_menu_item(label, icon, false, colors)
        .debug_selector(move || selector)
        .on_mouse_move(cx.listener(move |this, _, _, cx| {
            this.set_table_context_submenu(object_path.clone(), Some(submenu), cx);
            cx.stop_propagation();
        }))
        .child(app_icon(AppIcon::ChevronRight, 14., colors.muted))
}

fn table_submenu_action_item(
    label: impl Into<String>,
    icon: AppIcon,
    action: TableMenuAction,
    menu: &TableContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let label = label.into();
    let selector = format!("table-submenu-item:{label}");
    let action_path = menu.object_path.clone();
    table_inert_menu_item(label, icon, false, colors)
        .debug_selector(move || selector)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.handle_table_menu_action(action, action_path.clone(), window, cx);
                cx.stop_propagation();
            }),
        )
}

fn table_inert_menu_item(
    label: impl Into<String>,
    icon: AppIcon,
    destructive: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let label = label.into();
    // 现有菜单由主窗口管理焦点和子菜单；PopupMenu 迁移需整体切换该生命周期，
    // ListItem 又内置 hover 样式，无法直接保留 UiColors。此次仅修复已有元素身份。
    // 稳定 ID 使框架按 hover 进出更新，子菜单的 bubble 拦截不影响 Capture。
    div()
        .id(SharedString::from(format!("table-menu-item:{label}")))
        .h(px(26.))
        .rounded(colors.radius)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .text_color(if destructive {
            rgb(0xe5484d)
        } else {
            colors.text
        })
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon_box(icon, 18., 14., colors.muted).flex_none())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(label),
        )
}

fn data_cell_context_menu(
    menu: DataCellContextMenu,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_modify = !data_type_is_binary(menu.type_name.as_str());
    let can_empty = can_modify && data_type_supports_empty_string(menu.type_name.as_str());
    let can_null = can_modify && menu.nullable;
    let can_paste = can_modify;
    let menu_for_empty = menu.clone();
    let menu_for_null = menu.clone();
    let menu_for_delete = menu.clone();
    let menu_for_copy_selection = menu.clone();
    let menu_for_export_selection = menu.clone();
    let menu_for_copy = menu.clone();
    let menu_for_copy_field = menu.clone();
    let menu_for_paste = menu.clone();
    let menu_for_eq = menu.clone();
    let menu_for_ne = menu.clone();
    let menu_for_like = menu.clone();
    let menu_for_not_like = menu.clone();
    let menu_for_lt = menu.clone();
    let menu_for_gt = menu.clone();
    let menu_for_remove_filter = menu.clone();
    let menu_for_asc = menu.clone();
    let menu_for_desc = menu.clone();
    let menu_for_remove_sort = menu.clone();
    let menu_for_clear = menu.clone();
    let menu_for_refresh = menu.clone();

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
            hsla(0., 0., 0., 0.18),
        )])
        .p_1()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(
            data_cell_action_menu_item(
                tr("app.context.set_empty_string").to_string(),
                AppIcon::Edit,
                can_empty,
                &menu,
                colors,
                cx,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if can_empty {
                        this.set_data_cell_value(
                            &menu_for_empty,
                            CellValue::Text(String::new()),
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_cell_action_menu_item(
                tr("app.context.set_null").to_string(),
                AppIcon::Square,
                can_null,
                &menu,
                colors,
                cx,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if can_null {
                            this.set_data_cell_value(&menu_for_null, CellValue::Null, cx);
                        }
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(data_cell_menu_separator(colors))
        .child(
            data_cell_action_menu_item(
                row_delete_record_label(menu.selection_row_count),
                AppIcon::Trash,
                true,
                &menu,
                colors,
                cx,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.request_delete_data_cell_row(menu_for_delete.clone(), cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(data_cell_menu_separator(colors))
        .when_some(menu.selection_copy_label.clone(), |this, label| {
            this.child(
                data_cell_action_menu_item(label, AppIcon::Copy, true, &menu, colors, cx)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.copy_data_table_selection(menu_for_copy_selection.tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
            )
        })
        .when_some(menu.selection_export_label.clone(), |this, label| {
            this.child(
                data_cell_action_menu_item(label, AppIcon::Save, true, &menu, colors, cx)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.export_data_table_selection(menu_for_export_selection.tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
            )
        })
        .when(
            menu.selection_copy_label.is_some() || menu.selection_export_label.is_some(),
            |this| this.child(data_cell_menu_separator(colors)),
        )
        .child(
            // 预览不是完整值：复制会让人把片段当权威数据，直接禁用并说明原因。
            data_cell_action_menu_item(
                tr("app.context.copy").to_string(),
                AppIcon::Copy,
                !menu.value_truncated,
                &menu,
                colors,
                cx,
            )
                .when(menu.value_truncated, |this| {
                    this.tooltip(|window, cx| {
                        Tooltip::new(tr("app.cell_detail.large_copy_disabled"))
                            .build(window, cx)
                    })
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if menu_for_copy.value_truncated {
                            this.show_message(
                                tr("app.cell_detail.large_copy_disabled"),
                                AppMessageKind::Warning,
                                cx,
                            );
                            cx.stop_propagation();
                            return;
                        }
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            menu_for_copy.value.clone(),
                        ));
                        this.data_cell_context_menu = None;
                        this.show_message(
                            tr("app.context.copied_cell"),
                            AppMessageKind::Success,
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            data_cell_action_menu_item(
                tr("app.context.copy_field_name").to_string(),
                AppIcon::Copy,
                true,
                &menu,
                colors,
                cx,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            menu_for_copy_field.column_name.clone(),
                        ));
                        this.data_cell_context_menu = None;
                        this.show_message(
                            tr("app.context.copied_field_name"),
                            AppMessageKind::Success,
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            data_cell_action_menu_item(
                tr("app.context.paste").to_string(),
                AppIcon::Edit,
                can_paste,
                &menu,
                colors,
                cx,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if !can_paste {
                            cx.stop_propagation();
                            return;
                        }
                        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                            let meta = DataTableColumnMeta {
                                name: menu_for_paste.column_name.clone(),
                                type_name: menu_for_paste.type_name.clone(),
                                comment: None,
                                nullable: menu_for_paste.nullable,
                                primary_key: false,
                                choices: Vec::new(),
                            };
                            match data_cell_value_from_text(&meta, text.as_str()) {
                                Ok(value) => this.set_data_cell_value(&menu_for_paste, value, cx),
                                Err(message) => {
                                    this.show_message(message, AppMessageKind::Warning, cx);
                                    this.data_cell_context_menu = None;
                                }
                            }
                        } else {
                            this.show_message(
                                tr("app.context.clipboard_empty"),
                                AppMessageKind::Warning,
                                cx,
                            );
                        }
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(data_cell_menu_separator(colors))
        .child(data_cell_menu_submenu_item(
            tr("app.context.filter").to_string(),
            AppIcon::Filter,
            menu.tab_id,
            DataCellContextSubmenu::Filter,
            colors,
            cx,
        ))
        .when(
            menu.submenu == Some(DataCellContextSubmenu::Filter),
            |this| {
                this.child(data_cell_filter_submenu(
                    menu_for_eq,
                    menu_for_ne,
                    menu_for_like,
                    menu_for_not_like,
                    menu_for_lt,
                    menu_for_gt,
                    menu_for_remove_filter,
                    colors,
                    window,
                    cx,
                ))
            },
        )
        .child(data_cell_menu_submenu_item(
            tr("app.context.sort").to_string(),
            AppIcon::List,
            menu.tab_id,
            DataCellContextSubmenu::Sort,
            colors,
            cx,
        ))
        .when(menu.submenu == Some(DataCellContextSubmenu::Sort), |this| {
            this.child(data_cell_sort_submenu(
                menu_for_asc,
                menu_for_desc,
                menu_for_remove_sort,
                colors,
                window,
                cx,
            ))
        })
        .child(data_cell_menu_separator(colors))
        .child(
            data_cell_action_menu_item(
                tr("app.context.clear_filter_sort").to_string(),
                AppIcon::Close,
                true,
                &menu,
                colors,
                cx,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.data_cell_context_menu = None;
                    let action = PendingDirtyDataAction::ClearFilterSort(menu_for_clear.tab_id);
                    if this.request_dirty_data_action(action, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    this.clear_data_filter_and_sort_rules(menu_for_clear.tab_id, cx);
                    this.perform_data_filter_and_sort(menu_for_clear.tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_cell_action_menu_item(
                tr("app.context.refresh").to_string(),
                AppIcon::Refresh,
                true,
                &menu,
                colors,
                cx,
            )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.data_cell_context_menu = None;
                        this.request_data_editor_refresh(menu_for_refresh.tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn row_delete_label(selection_row_count: usize) -> String {
    if selection_row_count > 1 {
        tr("app.context.delete_selected_rows").to_string()
    } else {
        tr("app.context.delete_row").to_string()
    }
}

fn row_delete_record_label(selection_row_count: usize) -> String {
    if selection_row_count > 1 {
        tr("app.context.delete_selected_rows").to_string()
    } else {
        tr("app.context.delete_record").to_string()
    }
}

fn data_row_context_menu(
    menu: DataRowContextMenu,
    _state: &AppState,
    window: &Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let detail_menu = menu.clone();
    let insert_menu = menu.clone();
    let clone_menu = menu.clone();
    let delete_menu = menu.clone();
    let copy_menu = menu.clone();
    let export_menu = menu.clone();
    let rows_editable = menu.rows_editable;

    div()
        .absolute()
        .top(menu.position.y)
        .left(menu.position.x)
        .w(px(196.))
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
            hsla(0., 0., 0., 0.18),
        )])
        .p_1()
        .text_size(ui_text_size(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .child(
            data_row_menu_item(
                tr("app.context.row_details").to_string(),
                AppIcon::List,
                menu.tab_id,
                colors,
                cx,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.open_data_row_viewer(
                        detail_menu.tab_id,
                        detail_menu.query_result_page_index,
                        detail_menu.source_row,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        )
        .child(data_cell_menu_separator(colors))
        .child(data_row_menu_submenu_item(
            tr("app.context.copy").to_string(),
            AppIcon::Copy,
            menu.tab_id,
            DataRowContextSubmenu::Copy,
            colors,
            cx,
        ))
        .when(menu.submenu == Some(DataRowContextSubmenu::Copy), |this| {
            this.child(data_row_copy_submenu(copy_menu, window, colors, cx))
        })
        .when(rows_editable, |this| {
            this.child(data_cell_menu_separator(colors))
                .child(
                    data_row_menu_item(
                        tr("app.context.insert_row").to_string(),
                        AppIcon::Plus,
                        menu.tab_id,
                        colors,
                        cx,
                    )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.insert_data_row(
                                    insert_menu.tab_id,
                                    insert_menu.query_result_page_index,
                                    Some(insert_menu.source_row),
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ),
                )
                .child(
                    data_row_menu_item(
                        tr("app.context.clone_row").to_string(),
                        AppIcon::Plus,
                        menu.tab_id,
                        colors,
                        cx,
                    )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.clone_data_row(
                                    clone_menu.tab_id,
                                    clone_menu.query_result_page_index,
                                    clone_menu.source_row,
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ),
                )
                .child(
                    data_row_menu_item(
                        row_delete_label(menu.selection_row_count),
                        AppIcon::Trash,
                        menu.tab_id,
                        colors,
                        cx,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.delete_data_row(
                                delete_menu.tab_id,
                                delete_menu.query_result_page_index,
                                delete_menu.source_row,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
                )
        })
        .child(data_cell_menu_separator(colors))
        .child(data_row_menu_submenu_item(
            tr("app.context.export").to_string(),
            AppIcon::Save,
            menu.tab_id,
            DataRowContextSubmenu::Export,
            colors,
            cx,
        ))
        .when(
            menu.submenu == Some(DataRowContextSubmenu::Export),
            |this| this.child(data_row_export_submenu(export_menu, window, colors, cx)),
        )
}
