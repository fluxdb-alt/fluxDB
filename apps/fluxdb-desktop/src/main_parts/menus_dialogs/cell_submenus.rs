fn data_cell_filter_submenu(
    menu_for_eq: DataCellContextMenu,
    menu_for_ne: DataCellContextMenu,
    menu_for_like: DataCellContextMenu,
    menu_for_not_like: DataCellContextMenu,
    menu_for_lt: DataCellContextMenu,
    menu_for_gt: DataCellContextMenu,
    menu_for_remove_filter: DataCellContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    data_cell_submenu_shell(px(154.), colors)
        .child(
            data_cell_menu_item("字段 = 值", AppIcon::Filter, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.apply_context_filter(&menu_for_eq, DataFilterOperator::Eq, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_cell_menu_item("字段 != 值", AppIcon::Filter, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.apply_context_filter(&menu_for_ne, DataFilterOperator::Ne, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_cell_menu_item("字段类似值", AppIcon::Filter, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.apply_context_filter(&menu_for_like, DataFilterOperator::Contains, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_cell_menu_item("字段不类似值", AppIcon::Filter, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.apply_context_filter(
                        &menu_for_not_like,
                        DataFilterOperator::NotContains,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_cell_menu_item("字段 < 值", AppIcon::Filter, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.apply_context_filter(&menu_for_lt, DataFilterOperator::Lt, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_cell_menu_item("字段 > 值", AppIcon::Filter, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.apply_context_filter(&menu_for_gt, DataFilterOperator::Gt, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(data_cell_menu_separator(colors))
        .child(
            data_cell_menu_item("移除当前字段筛选", AppIcon::Filter, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.remove_context_filter(&menu_for_remove_filter, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn data_cell_sort_submenu(
    menu_for_asc: DataCellContextMenu,
    menu_for_desc: DataCellContextMenu,
    menu_for_remove_sort: DataCellContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    data_cell_submenu_shell(px(180.), colors)
        .child(
            data_cell_menu_item("升序排序", AppIcon::List, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.apply_context_sort(&menu_for_asc, true, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_cell_menu_item("降序排序", AppIcon::List, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.apply_context_sort(&menu_for_desc, false, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(data_cell_menu_separator(colors))
        .child(
            data_cell_menu_item("移除当前字段排序", AppIcon::List, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.remove_context_sort(&menu_for_remove_sort, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn data_cell_submenu_shell(top: Pixels, colors: UiColors) -> Div {
    div()
        .absolute()
        .left(px(236.))
        .top(top)
        .w(px(206.))
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
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

fn data_cell_menu_submenu_item(
    label: &'static str,
    icon: AppIcon,
    tab_id: TabId,
    submenu: DataCellContextSubmenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let submenu_id = match submenu {
        DataCellContextSubmenu::Filter => 1,
        DataCellContextSubmenu::Sort => 2,
    };
    div()
        .id(("data-cell-submenu-item", tab_id.0 as usize + submenu_id))
        .h(px(26.))
        .rounded(colors.radius)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .on_hover(cx.listener(move |this, hovered, _, cx| {
            if *hovered {
                this.set_data_cell_context_submenu(tab_id, Some(submenu), cx);
            }
        }))
        .child(app_icon(icon, 14., colors.muted))
        .child(div().flex_1().child(label))
        .child(app_icon(AppIcon::ChevronRight, 14., colors.muted))
}

fn data_cell_action_menu_item(
    label: impl Into<String>,
    icon: AppIcon,
    enabled: bool,
    menu: &DataCellContextMenu,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let tab_id = menu.tab_id;
    data_cell_menu_item_text(label, icon, enabled, colors)
        .font_weight(gpui::FontWeight::BOLD)
        .on_mouse_move(cx.listener(move |this, _, _, cx| {
            this.set_data_cell_context_submenu(tab_id, None, cx);
            cx.stop_propagation();
        }))
}

fn data_cell_menu_item(
    label: &'static str,
    icon: AppIcon,
    enabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    data_cell_menu_item_text(label, icon, enabled, colors)
}

fn data_cell_menu_item_text(
    label: impl Into<String>,
    icon: AppIcon,
    enabled: bool,
    colors: UiColors,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(SharedString::from(format!("data-cell-menu-item:{label}")))
        .h(px(26.))
        .rounded(colors.radius)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .opacity(if enabled { 1.0 } else { 0.42 })
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |this| this.cursor_pointer())
        .hover(move |style| {
            if enabled {
                style.bg(colors.hover)
            } else {
                style
            }
        })
        .child(
            app_icon_box(icon, 18., 14., colors.muted)
                .flex_none()
                .opacity(if enabled { 1.0 } else { 0.72 }),
        )
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

fn data_cell_menu_separator(colors: UiColors) -> Div {
    div().h(px(1.)).mx_1().my_1().bg(colors.border_soft)
}
