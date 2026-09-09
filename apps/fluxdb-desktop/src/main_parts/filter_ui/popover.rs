fn data_filter_popover_layer(
    tab_id: TabId,
    page: &DataPage,
    rules: &[DataFilterRule],
    sort_rules: &[DataSortRule],
    popover: DataFilterPopover,
    value_input: Entity<InputState>,
    search_input: Entity<InputState>,
    value_search: &str,
    value_search_loading: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let field_names = page
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let rule_index = popover.rule_index.unwrap_or(0);
    let rule = rules.get(rule_index);
    let selected_field = rule
        .and_then(|rule| rule.field.as_deref())
        .or_else(|| field_names.first().map(String::as_str))
        .unwrap_or("字段");
    let selected_operator = rule
        .map(|rule| rule.operator)
        .unwrap_or(DataFilterOperator::Contains);
    let selected_values = rule
        .map(|rule| rule.values.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let recommended_values = data_filter_recommended_values(page, selected_field, value_search);

    let menu = match popover.kind {
        DataFilterPopoverKind::Field => data_filter_field_menu(
            tab_id,
            rule_index,
            &field_names,
            selected_field,
            search_input.clone(),
            value_search,
            DATA_FILTER_POPOVER_TOP,
            colors,
            cx,
        ),
        DataFilterPopoverKind::Operator => data_filter_operator_menu(
            tab_id,
            rule_index,
            selected_operator,
            DATA_FILTER_POPOVER_TOP,
            colors,
            cx,
        ),
        DataFilterPopoverKind::Value => data_filter_value_menu(
            tab_id,
            rule_index,
            recommended_values,
            selected_values.as_slice(),
            value_input,
            search_input,
            value_search_loading,
            DATA_FILTER_POPOVER_TOP,
            colors,
            cx,
        ),
        DataFilterPopoverKind::SortField => data_sort_field_menu(
            tab_id,
            popover.sort_index.unwrap_or(0),
            &field_names,
            sort_rules
                .get(popover.sort_index.unwrap_or(0))
                .map(|rule| rule.field.as_str())
                .unwrap_or(selected_field),
            search_input.clone(),
            value_search,
            DATA_FILTER_POPOVER_TOP + 66.,
            colors,
            cx,
        ),
        DataFilterPopoverKind::SortMenu => data_sort_action_menu(
            tab_id,
            popover.sort_index.unwrap_or(0),
            sort_rules.get(popover.sort_index.unwrap_or(0)),
            DATA_FILTER_POPOVER_TOP + 66.,
            colors,
            cx,
        ),
    };

    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .occlude()
        .on_mouse_move(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.data_filter_popover = None;
                this.sync_table_hover_overlay_block(cx);
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(menu)
}

const DATA_FILTER_POPOVER_TOP: f32 = 94.;

fn data_filter_panel_height(rules: &[DataFilterRule], mode: DataFilterMode) -> f32 {
    if mode == DataFilterMode::Text {
        return 166.;
    }
    let filter_height = if rules.is_empty() {
        28.
    } else {
        let mut height: f32 = 0.;
        let mut index = 0;
        while index < rules.len() {
            if rules[index].grouped {
                let mut group_count = 0.;
                while index < rules.len() && rules[index].grouped {
                    group_count += 1.;
                    index += 1;
                }
                height += 36. + group_count * 24.;
            } else {
                height += 28.;
                index += 1;
            }
        }
        height
    };
    (28. + filter_height + 12. + 38. + 38.).max(166.)
}

fn clamp_data_filter_panel_height(height: f32) -> f32 {
    height.clamp(DATA_FILTER_PANEL_MIN_HEIGHT, DATA_FILTER_PANEL_MAX_HEIGHT)
}

fn data_filter_builder_rows(
    tab_id: TabId,
    rules: &[DataFilterRule],
    default_field: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if rules.is_empty() {
        return data_filter_empty_rule_row(tab_id, default_field, colors, cx);
    }

    let mut list = div()
        .bg(if colors.is_dark {
            rgb(0x18314d)
        } else {
            rgb(0xd8eefc)
        })
        .flex()
        .flex_col();

    let mut index = 0;
    while index < rules.len() {
        let rule = &rules[index];
        let show_and = index + 1 < rules.len();
        if rule.grouped {
            let group_start = index;
            let mut group_end = index;
            while group_end + 1 < rules.len() && rules[group_end + 1].grouped {
                group_end += 1;
            }
            list = list.child(data_filter_group_open_row(colors));
            for group_index in group_start..=group_end {
                list = list.child(data_filter_rule_row(
                    tab_id,
                    group_index,
                    &rules[group_index],
                    default_field.clone(),
                    true,
                    group_index < group_end,
                    colors,
                    cx,
                ));
            }
            list = list.child(data_filter_group_close_row(
                tab_id,
                group_end,
                default_field.clone(),
                group_end + 1 < rules.len(),
                colors,
                cx,
            ));
            index = group_end + 1;
        } else {
            list = list.child(data_filter_rule_row(
                tab_id,
                index,
                rule,
                default_field.clone(),
                false,
                show_and,
                colors,
                cx,
            ));
            index += 1;
        }
    }
    list
}

fn data_filter_group_open_row(_colors: UiColors) -> Div {
    div()
        .h(px(18.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(_colors.hover))
        .child(div().w(px(28.)))
        .child(
            div()
                .text_size(px(14.))
                .line_height(px(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(0x006bd6))
                .child("("),
        )
        .child(div().flex_1())
}

fn data_filter_group_close_row(
    tab_id: TabId,
    group_end_index: usize,
    default_field: Option<String>,
    show_and: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let group_default_field = default_field.clone();
    div()
        .h(px(18.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(colors.hover))
        .child(div().w(px(28.)))
        .child(
            div()
                .text_size(px(14.))
                .line_height(px(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(0x006bd6))
                .child(")"),
        )
        .when(show_and, |this| {
            this.child(
                div()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(0x006bd6))
                    .child("and"),
            )
        })
        .when(!show_and, |this| {
            this.child(data_filter_small_button("+", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.add_data_filter_rule_after(
                        tab_id,
                        group_end_index,
                        default_field.clone(),
                        false,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ))
            .child(data_filter_small_button("()", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.add_data_filter_rule_after(
                        tab_id,
                        group_end_index,
                        group_default_field.clone(),
                        true,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ))
        })
}

fn data_filter_empty_rule_row(
    tab_id: TabId,
    default_field: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let group_default_field = default_field.clone();
    div()
        .h(px(28.))
        .px_3()
        .bg(if colors.is_dark {
            rgb(0x18314d)
        } else {
            rgb(0xd8eefc)
        })
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(colors.hover))
        .child(data_filter_small_button("+", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.add_data_filter_rule(tab_id, default_field.clone(), cx);
                cx.stop_propagation();
            }),
        ))
        .child(data_filter_small_button("()", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.add_data_filter_group(tab_id, group_default_field.clone(), cx);
                cx.stop_propagation();
            }),
        ))
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("点击“+”以添加筛选准则"),
        )
}

fn data_filter_rule_row(
    tab_id: TabId,
    rule_index: usize,
    rule: &DataFilterRule,
    default_field: Option<String>,
    nested: bool,
    show_and: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected_field = rule
        .field
        .as_deref()
        .or(default_field.as_deref())
        .unwrap_or("字段");
    let add_default_field = default_field.clone();
    let group_default_field = default_field.clone();
    let indent = if nested { 40. } else { 12. };
    let add_grouped = nested;
    let group_button_grouped = true;
    let show_value = rule.operator.requires_values();
    let selected_values = show_value.then(|| rule.values.iter().cloned().collect::<Vec<_>>());
    let value_label = selected_values
        .as_deref()
        .map(data_filter_value_label);

    div()
        .h(px(if nested { 24. } else { 28. }))
        .pl(px(indent))
        .pr_3()
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(colors.hover))
        .child(data_filter_check(rule.enabled, colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_data_filter_rule_enabled(tab_id, rule_index, cx);
                cx.stop_propagation();
            }),
        ))
        .child(
            data_filter_clickable_text(selected_field.to_string(), colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.toggle_data_filter_popover(
                        tab_id,
                        Some(rule_index),
                        DataFilterPopoverKind::Field,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_filter_clickable_text(rule.operator.label().to_string(), colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.toggle_data_filter_popover(
                        tab_id,
                        Some(rule_index),
                        DataFilterPopoverKind::Operator,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        )
        .when(show_value, |this| {
            this.child(
                data_filter_clickable_text(
                    value_label.clone().unwrap_or_else(|| "<?>".to_string()),
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.toggle_data_filter_popover(
                            tab_id,
                            Some(rule_index),
                            DataFilterPopoverKind::Value,
                            window,
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                ),
            )
        })
        .when(show_and, |this| {
            this.child(
                div()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(0x006bd6))
                    .child("and"),
            )
        })
        .when(!show_and, |this| {
            this.child(data_filter_small_button("+", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.add_data_filter_rule_after(
                        tab_id,
                        rule_index,
                        add_default_field.clone(),
                        add_grouped,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ))
            .child(data_filter_small_button("()", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.add_data_filter_rule_after(
                        tab_id,
                        rule_index,
                        group_default_field.clone(),
                        group_button_grouped,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ))
        })
        .child(div().flex_1())
        .child(data_filter_delete_button(colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.delete_data_filter_rule(tab_id, rule_index, cx);
                cx.stop_propagation();
            }),
        ))
}

fn data_filter_value_label(selected_values: &[String]) -> String {
    if selected_values.is_empty() {
        "<?>".to_string()
    } else if selected_values.len() == 1 {
        selected_values[0].clone()
    } else {
        format!("{} 个值", selected_values.len())
    }
}

fn data_sort_empty_rule(
    tab_id: TabId,
    default_field: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(data_filter_small_button("+", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.add_data_sort_rule(tab_id, default_field.clone(), cx);
                cx.stop_propagation();
            }),
        ))
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("点击“+”以添加排序准则"),
        )
}

fn data_sort_builder_section(
    tab_id: TabId,
    sort_rules: &[DataSortRule],
    default_field: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut content = div()
        .h(px(38.))
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .child(data_filter_section_label("排序方式", colors));

    if sort_rules.is_empty() {
        content = content.child(data_sort_empty_rule(tab_id, default_field, colors, cx));
    } else {
        let add_field = sort_rules
            .last()
            .map(|rule| rule.field.clone())
            .or(default_field);
        for (sort_index, rule) in sort_rules.iter().enumerate() {
            content = content.child(data_sort_rule_row(tab_id, sort_index, rule, colors, cx));
        }
        content = content.child(data_filter_small_button("+", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.add_data_sort_rule(tab_id, add_field.clone(), cx);
                cx.stop_propagation();
            }),
        ));
    }

    content
}

fn data_filter_section_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(label)
}

fn data_sort_rule_row(
    tab_id: TabId,
    sort_index: usize,
    rule: &DataSortRule,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(data_sort_rule_pill(tab_id, sort_index, rule, colors, cx))
}

fn data_sort_rule_pill(
    tab_id: TabId,
    sort_index: usize,
    rule: &DataSortRule,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(22.))
        .rounded(colors.radius)
        .border_1()
        .border_color(rgb(0x1677ff))
        .bg(if colors.is_dark {
            rgb(0x13233a)
        } else {
            rgb(0xeaf4ff)
        })
        .flex()
        .items_center()
        .overflow_hidden()
        .text_color(rgb(0x1677ff))
        .child(
            div()
                .h_full()
                .px_2()
                .flex()
                .items_center()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .child(rule.field.clone())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.data_filter_popover = Some(DataFilterPopover {
                            tab_id,
                            kind: DataFilterPopoverKind::SortField,
                            rule_index: None,
                            sort_index: Some(sort_index),
                        });
                        this.sync_table_hover_overlay_block(cx);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            div()
                .h_full()
                .w(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .child(data_sort_direction_icon(rule.ascending, colors))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.toggle_data_sort_direction(tab_id, sort_index, cx);
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            div()
                .h_full()
                .w(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::ChevronDown, 13., rgb(0x1677ff)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.data_filter_popover = Some(DataFilterPopover {
                            tab_id,
                            kind: DataFilterPopoverKind::SortMenu,
                            rule_index: None,
                            sort_index: Some(sort_index),
                        });
                        this.sync_table_hover_overlay_block(cx);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                ),
        )
}

fn data_sort_direction_icon(ascending: bool, colors: UiColors) -> Div {
    let widths = if ascending {
        [6., 10., 14.]
    } else {
        [14., 10., 6.]
    };
    div()
        .w(px(15.))
        .h(px(14.))
        .flex()
        .flex_col()
        .justify_center()
        .items_end()
        .gap(px(2.))
        .child(data_sort_direction_bar(widths[0], colors))
        .child(data_sort_direction_bar(widths[1], colors))
        .child(data_sort_direction_bar(widths[2], colors))
}

fn data_sort_direction_bar(width: f32, _colors: UiColors) -> Div {
    div()
        .w(px(width))
        .h(px(1.5))
        .rounded_full()
        .bg(rgb(0x5f7f9f))
}

fn data_filter_clickable_text(text: String, colors: UiColors) -> Div {
    div()
        .h(px(20.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x006bd6))
        .cursor_pointer()
        .flex()
        .items_center()
        .hover(move |style| {
            style.bg(if colors.is_dark {
                rgb(0x244565)
            } else {
                rgb(0xc8e5fb)
            })
        })
        .child(text)
}

fn data_filter_field_menu(
    tab_id: TabId,
    rule_index: usize,
    field_names: &[String],
    selected_field: &str,
    search_input: Entity<InputState>,
    search_query: &str,
    top: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    let normalized_query = search_query.trim().to_ascii_lowercase();
    let mut matched_count = 0usize;
    for field in field_names.iter().filter(|field| {
        normalized_query.is_empty()
            || field
                .to_ascii_lowercase()
                .contains(normalized_query.as_str())
    }) {
        matched_count += 1;
        let selected = field == selected_field;
        let field_name = field.clone();
        list = list.child(
            data_filter_menu_item(field.clone(), selected, Some(search_query), colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.select_data_filter_field(tab_id, rule_index, field_name.clone(), cx);
                        cx.stop_propagation();
                    }),
                ),
        );
    }
    if matched_count == 0 {
        list = list.child(data_filter_menu_empty_item("没有匹配字段", colors));
    }

    data_filter_menu_surface(200., 260., colors)
        .top(px(top))
        .left(px(58.))
        .child(list)
        .child(data_filter_menu_search_footer(search_input, colors))
}

fn data_sort_field_menu(
    tab_id: TabId,
    sort_index: usize,
    field_names: &[String],
    selected_field: &str,
    search_input: Entity<InputState>,
    search_query: &str,
    top: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    let normalized_query = search_query.trim().to_ascii_lowercase();
    let mut matched_count = 0usize;
    for field in field_names.iter().filter(|field| {
        normalized_query.is_empty()
            || field
                .to_ascii_lowercase()
                .contains(normalized_query.as_str())
    }) {
        matched_count += 1;
        let selected = field == selected_field;
        let field_name = field.clone();
        list = list.child(
            data_filter_menu_item(field.clone(), selected, Some(search_query), colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.select_data_sort_field(tab_id, sort_index, field_name.clone(), cx);
                        cx.stop_propagation();
                    }),
                ),
        );
    }
    if matched_count == 0 {
        list = list.child(data_filter_menu_empty_item("没有匹配字段", colors));
    }

    data_filter_menu_surface(200., 260., colors)
        .top(px(top))
        .left(px(112.))
        .child(list)
        .child(data_filter_menu_search_footer(search_input, colors))
}

fn data_filter_operator_menu(
    tab_id: TabId,
    rule_index: usize,
    selected_operator: DataFilterOperator,
    top: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    for operator in DataFilterOperator::all() {
        let selected = *operator == selected_operator;
        let operator_value = *operator;
        list = list.child(
            data_filter_menu_item(operator.label().to_string(), selected, None, colors)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.select_data_filter_operator(tab_id, rule_index, operator_value, cx);
                        cx.stop_propagation();
                    }),
                ),
        );
    }

    data_filter_menu_surface(132., 300., colors)
        .top(px(top))
        .left(px(90.))
        .child(list)
}

fn data_sort_action_menu(
    tab_id: TabId,
    sort_index: usize,
    sort_rule: Option<&DataSortRule>,
    top: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let ascending = sort_rule.map(|rule| rule.ascending).unwrap_or(true);

    data_filter_menu_surface(270., 216., colors)
        .top(px(top))
        .left(px(116.))
        .child(
            data_sort_menu_item("更改字段", None, false, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.open_data_sort_field_menu(tab_id, sort_index, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(data_sort_menu_separator(colors))
        .child(
            data_sort_menu_item("升序排序", None, ascending, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_data_sort_direction(tab_id, sort_index, true, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_sort_menu_item("降序排序", None, !ascending, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_data_sort_direction(tab_id, sort_index, false, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(data_sort_menu_separator(colors))
        .child(data_sort_menu_item(
            "左移",
            Some("Ctrl+Left"),
            false,
            colors,
        ))
        .child(data_sort_menu_item(
            "右移",
            Some("Ctrl+Right"),
            false,
            colors,
        ))
        .child(data_sort_menu_separator(colors))
        .child(
            data_sort_menu_item("删除", None, false, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.delete_data_sort_rule(tab_id, sort_index, cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_sort_menu_item("清除所有排序", None, false, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.data_sort_draft_rules.remove(&tab_id);
                    this.data_filter_popover = None;
                    this.sync_table_hover_overlay_block(cx);
                    cx.notify();
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            data_sort_menu_item("清除所有筛选 & 排序", None, false, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.clear_data_filter_and_sort_rules(tab_id, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn data_sort_menu_item(
    label: &'static str,
    shortcut: Option<&'static str>,
    checked: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(24.))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .text_color(colors.text)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .w(px(14.))
                .text_size(px(12.))
                .text_color(colors.text)
                .child(if checked { "✓" } else { "" }),
        )
        .child(div().flex_1().child(label))
        .when_some(shortcut, |this, shortcut| {
            this.child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.muted)
                    .child(shortcut),
            )
        })
}

fn data_sort_menu_separator(colors: UiColors) -> Div {
    div()
        .h(px(8.))
        .flex()
        .items_center()
        .child(div().h(px(1.)).w_full().bg(colors.border_soft))
}

fn data_filter_value_menu(
    tab_id: TabId,
    rule_index: usize,
    values: Vec<String>,
    selected_values: &[String],
    value_input: Entity<InputState>,
    search_input: Entity<InputState>,
    search_loading: bool,
    top: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let selected = selected_values.iter().cloned().collect::<BTreeSet<_>>();
    let mut list = div().h(px(156.)).overflow_y_scrollbar().flex().flex_col();

    for value in values {
        let checked = selected.contains(&value);
        let value_for_click = value.clone();
        list = list.child(
            data_filter_value_item(value, checked, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_data_filter_value(tab_id, rule_index, value_for_click.clone(), cx);
                    cx.stop_propagation();
                }),
            ),
        );
    }

    data_filter_menu_surface(360., 320., colors)
        .top(px(top))
        .left(px(168.))
        .child(
            div()
                .h(px(50.))
                .px_2()
                .pt_2()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("值:"),
                )
                .child(
                    div()
                        .h(px(26.))
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
                .h(px(36.))
                .px_2()
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(colors.text)
                .child("建议值:"),
        )
        .child(list)
        .child(
            div()
                .h(px(34.))
                .border_t_1()
                .border_color(colors.border_soft)
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(if search_loading {
                    loading_spinner(14.).into_any_element()
                } else {
                    app_icon(AppIcon::Search, 14., colors.muted)
                })
                .child(
                    Input::new(&search_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(13.)),
                ),
        )
        .child(
            div()
                .h(px(42.))
                .border_t_1()
                .border_color(colors.border_soft)
                .px_2()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                    data_filter_dialog_button("确定", true, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.data_filter_popover = None;
                            this.sync_table_hover_overlay_block(cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    data_filter_dialog_button("取消", false, colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.data_filter_popover = None;
                            this.sync_table_hover_overlay_block(cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
                ),
        )
}
