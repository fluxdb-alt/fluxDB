fn local_filter_popover_layer(
    page: &DataPage,
    popover: LocalFilterPopover,
    selected_values: BTreeSet<String>,
    value_input: Entity<InputState>,
    search_input: Entity<InputState>,
    search: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    const POPOVER_WIDTH: f32 = 380.;
    const POPOVER_HEIGHT: f32 = 430.;
    let values = data_filter_recommended_values(page, popover.field_name.as_str(), search);
    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .px_1();

    if values.is_empty() {
        list = list.child(data_filter_menu_empty_item("没有匹配值", colors));
    } else {
        for value in values {
            let checked = selected_values.contains(&value);
            let value_for_click = value.clone();
            list = list.child(
                local_filter_value_item(value, checked, search, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.toggle_local_filter_value(value_for_click.clone(), cx);
                        cx.stop_propagation();
                    }),
                ),
            );
        }
    }

    div()
        .absolute()
        .left(px(304.))
        .top(px(92.))
        .w(px(POPOVER_WIDTH))
        .h(px(POPOVER_HEIGHT))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            0.,
            10.,
            24.,
            0.,
            hsla(0., 0., 0., if colors.is_dark { 0.48 } else { 0.18 }),
        )])
        .flex()
        .flex_col()
        .occlude()
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .px_3()
                .pt_3()
                .pb_2()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("值:"),
                )
                .child(
                    div()
                        .h(px(32.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .px_2()
                        .flex()
                        .items_center()
                        .child(
                            Input::new(&value_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .text_size(px(13.)),
                        ),
                ),
        )
        .child(
            div()
                .px_3()
                .pb_2()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("建议值:"),
        )
        .child(list)
        .child(data_filter_menu_search_footer(search_input, colors))
        .child(
            div()
                .h(px(48.))
                .border_t_1()
                .border_color(colors.border_soft)
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    data_filter_dialog_button("清除筛选", false, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.clear_local_filter(cx);
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            data_filter_dialog_button("确定", true, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.apply_local_filter(cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            data_filter_dialog_button("取消", false, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.cancel_local_filter(cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}

fn local_filter_manager_layer(
    tab_id: TabId,
    page: &DataPage,
    draft_filters: BTreeMap<String, BTreeSet<String>>,
    field_open: bool,
    values_open: bool,
    selected_field: Option<String>,
    search_input: Entity<InputState>,
    value_search: &str,
    value_text: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    const MANAGER_POPOVER_WIDTH: f32 = 376.;

    let field_names = page
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let selected_field = selected_field
        .or_else(|| field_names.first().cloned())
        .unwrap_or_else(|| "字段".to_string());
    let selected_values = draft_filters
        .get(&selected_field)
        .cloned()
        .unwrap_or_default();
    let is_editing = field_open || values_open;
    let editing_field = is_editing.then_some(selected_field.as_str());
    let conditions = local_filter_manager_condition_entries(&draft_filters, editing_field);
    let dropdown_top = 52. + 44. + 12. + (conditions.len() as f32 * 35.);
    let dropdown_left = 18.;
    let new_condition_fields = field_names.clone();

    let mut list = div().flex().flex_col().gap_1();
    if !conditions.is_empty() {
        for (field, values) in conditions.iter().cloned() {
            list = list.child(local_filter_manager_condition_row(
                field, values, colors, cx,
            ));
        }
    }

    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.local_filter_manager_popover = None;
                this.sync_table_hover_overlay_block(cx);
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .child(
            div()
                .absolute()
                .left(px(18.))
                .top(px(52.))
                .w(px(MANAGER_POPOVER_WIDTH))
                .max_h(px(408.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    0.,
                    10.,
                    24.,
                    0.,
                    hsla(0., 0., 0., if colors.is_dark { 0.48 } else { 0.18 }),
                )])
                .flex()
                .flex_col()
                .occlude()
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.close_local_filter_manager_dropdowns(cx);
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .h(px(44.))
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("筛选"),
                        )
                        .child(
                            div()
                                .h(px(26.))
                                .px_2()
                                .rounded(colors.radius)
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .hover(move |style| style.bg(colors.hover))
                                .child(app_icon(AppIcon::Plus, 13., colors.text))
                                .child("新增条件")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.start_local_filter_manager_condition(
                                            new_condition_fields.clone(),
                                            cx,
                                        );
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .px_3()
                        .pb_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(list)
                        .when(is_editing, |this| {
                            this.child(local_filter_manager_new_row(
                                &draft_filters,
                                selected_field.clone(),
                                value_text,
                                colors,
                                cx,
                            ))
                        }),
                )
                .child(
                    div()
                        .h(px(46.))
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            data_filter_dialog_button("清除筛选", false, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.clear_local_filter_manager(tab_id, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    data_filter_dialog_button("重置条件", false, colors)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, _, cx| {
                                                this.reset_local_filter_manager(tab_id, cx);
                                                cx.stop_propagation();
                                            }),
                                        ),
                                )
                                .child(
                                    data_filter_dialog_button("应用筛选", true, colors)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, _, cx| {
                                                this.apply_local_filter_manager(tab_id, cx);
                                                cx.stop_propagation();
                                            }),
                                        ),
                                ),
                        ),
                ),
        )
        .when(field_open, |this| {
            this.child(local_filter_manager_fields_panel(
                field_names,
                dropdown_top,
                dropdown_left + 12.,
                colors,
                cx,
            ))
        })
        .when(values_open, |this| {
            this.child(local_filter_manager_values_panel(
                page,
                selected_field,
                selected_values,
                dropdown_top,
                dropdown_left + 144.,
                search_input,
                value_search,
                colors,
                cx,
            ))
        })
}

fn local_filter_manager_new_row(
    draft_filters: &BTreeMap<String, BTreeSet<String>>,
    selected_field: String,
    value_text: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected_values = draft_filters
        .get(&selected_field)
        .cloned()
        .unwrap_or_default();

    div().relative().h(px(32.)).flex().flex_col().child(
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(local_filter_manager_field_select(
                selected_field.clone(),
                colors,
                cx,
            ))
            .child(local_filter_manager_value_button(
                selected_values.len(),
                value_text,
                colors,
                cx,
            )),
    )
}

fn local_filter_manager_condition_row(
    field: String,
    values: BTreeSet<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let field_for_delete = field.clone();
    let field_for_edit = field.clone();
    let value_label = local_filter_value_summary(&values);
    div()
        .h(px(31.))
        .flex()
        .items_center()
        .gap_1()
        .hover(move |style| style.bg(colors.hover))
        .child(local_filter_manager_static_select(field, colors))
        .child(
            div()
                .h(px(28.))
                .w(px(176.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .px_2()
                .cursor_pointer()
                .flex()
                .items_center()
                .overflow_hidden()
                .text_size(px(13.))
                .text_color(colors.text)
                .hover(move |style| style.bg(colors.hover))
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(value_label),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.open_local_filter_manager_values_for_field(field_for_edit.clone(), cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            div()
                .size(px(24.))
                .rounded(colors.radius)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Trash, 15., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.remove_local_filter_manager_field(field_for_delete.clone(), cx);
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn local_filter_value_summary(values: &BTreeSet<String>) -> String {
    if values.is_empty() {
        return "值".to_string();
    }
    if values.len() == 1 {
        return values.iter().next().cloned().unwrap_or_default();
    }
    format!("{} 个值", values.len())
}

fn local_filter_manager_static_select(label: impl Into<String>, colors: UiColors) -> Div {
    div()
        .h(px(28.))
        .w(px(120.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .px_2()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(label.into())
}

fn local_filter_manager_field_select(
    selected_field: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(32.))
        .w(px(128.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(selected_field),
        )
        .child(app_icon(AppIcon::ChevronDown, 14., colors.muted))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_local_filter_manager_fields(cx);
                cx.stop_propagation();
            }),
        )
}

fn local_filter_manager_value_button(
    selected_count: usize,
    value_text: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = if value_text.trim().is_empty() {
        if selected_count == 0 {
            "值".to_string()
        } else {
            format!("{selected_count} 个值")
        }
    } else {
        value_text.to_string()
    };

    div()
        .h(px(32.))
        .w(px(178.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .px_2()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if value_text.trim().is_empty() && selected_count == 0 {
                    colors.muted
                } else {
                    colors.text
                })
                .child(label),
        )
        .child(app_icon(AppIcon::ChevronDown, 14., colors.muted))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_local_filter_manager_values(cx);
                cx.stop_propagation();
            }),
        )
}

fn local_filter_manager_fields_panel(
    field_names: Vec<String>,
    top: f32,
    left: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div().flex().flex_col();
    for field in field_names {
        let field_for_click = field.clone();
        list = list.child(
            data_filter_menu_item(field, false, None, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.select_local_filter_manager_field(field_for_click.clone(), window, cx);
                    cx.stop_propagation();
                }),
            ),
        );
    }

    div()
        .absolute()
        .top(px(top))
        .left(px(left))
        .w(px(218.))
        .h(px(210.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            0.,
            8.,
            20.,
            0.,
            hsla(0., 0., 0., if colors.is_dark { 0.45 } else { 0.18 }),
        )])
        .flex()
        .flex_col()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .child(list),
        )
}

fn local_filter_manager_values_panel(
    page: &DataPage,
    selected_field: String,
    selected_values: BTreeSet<String>,
    top: f32,
    left: f32,
    search_input: Entity<InputState>,
    value_search: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let values = data_filter_recommended_values(page, selected_field.as_str(), value_search);
    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    if values.is_empty() {
        list = list.child(data_filter_menu_empty_item("没有匹配值", colors));
    } else {
        for value in values {
            let checked = selected_values.contains(&value);
            let field_for_click = selected_field.clone();
            let value_for_click = value.clone();
            list = list.child(
                local_filter_manager_value_item(value, checked, value_search, colors)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.toggle_local_filter_manager_value(
                                field_for_click.clone(),
                                value_for_click.clone(),
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
    }

    div()
        .absolute()
        .top(px(top))
        .left(px(left))
        .w(px(218.))
        .h(px(132.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_bg)
        .shadow(vec![box_shadow(
            0.,
            8.,
            20.,
            0.,
            hsla(0., 0., 0., if colors.is_dark { 0.45 } else { 0.18 }),
        )])
        .flex()
        .flex_col()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(list)
        .child(data_filter_menu_search_footer(search_input, colors))
}

fn local_filter_manager_value_item(
    label: String,
    checked: bool,
    search: &str,
    colors: UiColors,
) -> Div {
    local_filter_value_item(label, checked, search, colors)
}

fn local_filter_value_item(label: String, checked: bool, search: &str, colors: UiColors) -> Div {
    div()
        .h(px(24.))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(13.))
        .text_color(colors.text)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .size(px(13.))
                .flex_none()
                .rounded(colors.radius * 0.5)
                .border_1()
                .border_color(if checked {
                    rgb(0x1677ff)
                } else {
                    colors.border
                })
                .bg(if checked {
                    rgb(0x1677ff)
                } else {
                    colors.input_bg
                })
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(9.))
                .text_color(rgb(0xffffff))
                .child(if checked { "✓" } else { "" }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(data_filter_menu_label(label.as_str(), Some(search), colors)),
        )
}

