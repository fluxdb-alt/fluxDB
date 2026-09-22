fn data_filter_popover_layer(
    tab_id: TabId,
    page: &DataPage,
    rules: &[DataFilterRule],
    sort_rules: &[DataSortRule],
    popover: DataFilterPopover,
    value_input: Entity<InputState>,
    search_input: Entity<InputState>,
    batch_input: Entity<InputState>,
    value_search: &str,
    value_search_loading: bool,
    value_draft: Option<&DataFilterValueDraft>,
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
            value_draft,
            value_input,
            search_input,
            batch_input,
            value_search,
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
            cx.listener(move |this, _, _, cx| {
                // 外部点击关闭弹层：值弹层同时丢弃未确认草稿，避免下次误用。
                if matches!(popover.kind, DataFilterPopoverKind::Value) {
                    this.data_filter_value_draft = None;
                }
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
    recommended_values: Vec<String>,
    draft: Option<&DataFilterValueDraft>,
    value_input: Entity<InputState>,
    search_input: Entity<InputState>,
    batch_input: Entity<InputState>,
    search: &str,
    search_loading: bool,
    top: f32,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let draft = draft.filter(|d| d.tab_id == tab_id && d.rule_index == rule_index);
    let selected = draft.map(|draft| draft.values.clone()).unwrap_or_default();
    let field_label = draft
        .map(|draft| draft.field.clone())
        .filter(|field| !field.is_empty())
        .unwrap_or_else(|| "筛选值".to_string());
    let selected_count = selected.len();

    div()
        .absolute()
        .w(px(520.))
        .max_h(px(560.))
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
        .top(px(top))
        .left(px(150.))
        .flex()
        .flex_col()
        .occlude()
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .key_context("DataFilterValueMenu")
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            this.cancel_data_filter_value(cx);
            cx.stop_propagation();
        }))
        .child(data_filter_value_header(field_label, colors, cx)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.cancel_data_filter_value(cx);
                    cx.stop_propagation();
                }),
            ))
        // 已选区
        .child(
            div()
                .px_2()
                .pt_2()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_1()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(format!("已选 {selected_count} 个值")),
                        )
                        .child(
                            data_filter_value_clear_button(colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.clear_data_filter_values(tab_id, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                )
                .child(data_filter_selected_values_tags(tab_id, &selected, colors, cx)),
        )
        // 手动输入区
        .child(
            div()
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
                        .child("输入值"),
                )
                .child(
                    div()
                        .h(px(28.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Input::new(&value_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .text_size(px(13.)),
                        )
                        .child(
                            data_filter_value_small_button("添加", colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.add_data_filter_manual_value(tab_id, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                )
                .child(
                    div()
                        .px_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child("重复值自动去重"),
                        )
                        .child(
                            data_filter_batch_toggle(&draft
                                .map(|draft| draft.batch_open)
                                .unwrap_or(false), colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.toggle_data_filter_batch(tab_id, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
        .when(
            draft.is_some_and(|draft| draft.batch_open),
            |this| this.child(
                data_filter_batch_paste_section(
                    tab_id,
                    draft.unwrap(),
                    batch_input.clone(),
                    colors,
                    cx,
                ),
            ),
        )
        // 建议值区
        .child(
            div()
                .px_2()
                .pt_2()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_1()
                        .pb_1()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("建议值"),
                )
                .child(
                    div()
                        .h(px(28.))
                        .rounded(colors.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.input_bg)
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().opacity(if search_loading { 0.35 } else { 1. }).child(
                            app_icon(AppIcon::Search, 14., colors.muted),
                        ))
                        .child(
                            Input::new(&search_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .text_size(px(13.)),
                        ),
                ),
        )
        .child(
            data_filter_suggested_values_list(
                tab_id,
                rule_index,
                &recommended_values,
                &selected,
                search,
                &colors,
                cx,
            ),
        )
        // 底部按钮
        .child(
            div()
                .h(px(44.))
                .border_t_1()
                .border_color(colors.border_soft)
                .px_2()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(format!("已选 {selected_count} 个值")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            data_filter_dialog_button("取消", false, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.cancel_data_filter_value(cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            data_filter_dialog_button("确定", true, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.apply_data_filter_value_draft(tab_id, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}

fn data_filter_value_header(label: String, colors: UiColors, _cx: &mut Context<NavicatMain>) -> Div {
    div()
        .h(px(40.))
        .border_b_1()
        .border_color(colors.border_soft)
        .px_3()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child("编辑筛选值"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .max_w(px(200.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(0x006bd6))
                        .child(label),
                )
                .child(
                    div()
                        .size(px(20.))
                        .rounded(colors.radius)
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |style| style.bg(colors.hover))
                        .child(app_icon(AppIcon::Close, 14., colors.muted)),
                ),
        )
}

fn data_filter_value_clear_button(colors: UiColors) -> Div {
    div()
        .h(px(20.))
        .px_2()
        .rounded(colors.radius)
        .text_size(px(11.))
        .text_color(colors.muted)
        .cursor_pointer()
        .flex()
        .items_center()
        .hover(move |style| style.bg(colors.hover))
        .child("清空")
}

fn data_filter_value_small_button(label: &'static str, colors: UiColors) -> Div {
    div()
        .h(px(20.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(px(12.))
        .text_color(colors.text)
        .cursor_pointer()
        .flex()
        .items_center()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn data_filter_batch_toggle(open: &bool, colors: UiColors) -> Div {
    let open = *open;
    div()
        .h(px(20.))
        .px_2()
        .rounded(colors.radius)
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x006bd6))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon(if open { AppIcon::ChevronUp } else { AppIcon::ChevronDown }, 12., rgb(0x006bd6)))
        .child(if open { "收起批量粘贴" } else { "批量粘贴" })
}

/// 已选值标签区：短值横向排列自动换行，UUID 等长值截断并提示查看；数量多时限制高度滚动。
fn data_filter_selected_values_tags(
    tab_id: TabId,
    selected: &BTreeSet<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut wrap = div()
        .max_h(px(96.))
        .overflow_y_scrollbar()
        .px_1()
        .py_1()
        .flex()
        .flex_wrap()
        .gap_1();

    if selected.is_empty() {
        wrap = wrap.child(
            div()
                .h(px(24.))
                .px_1()
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("暂无已选值"),
        );
    }

    for (index, value) in selected.iter().enumerate() {
        let value_for_remove = value.clone();
        let shown = if value.chars().count() > 24 {
            let mut shortened: String = value.chars().take(24).collect();
            shortened.push('…');
            shortened
        } else {
            value.clone()
        };
        let title = value.clone();
        // 长值（如 UUID）截断后显示省略号，悬停可查看完整值；id 需同列表内唯一，
        // 用序号保证唯一（截断后不同的长值可能得到相同前缀）。
        let label = div()
            .id(("data-filter-value-tag", index))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(px(12.))
            .text_color(colors.text)
            .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
            .child(shown);
        wrap = wrap.child(
            div()
                .h(px(22.))
                .max_w(px(220.))
                .rounded_full()
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .px_2()
                .flex()
                .items_center()
                .gap_1()
                .child(label)
                .child(
                    div()
                        .size(px(16.))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |style| style.bg(colors.hover))
                        .child(app_icon(AppIcon::Close, 11., colors.muted))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.remove_data_filter_value(tab_id, value_for_remove.clone(), cx);
                                cx.stop_propagation();
                            }),
                        ),
                ),
        );
    }
    // overflow_y_scrollbar 返回 Scrollable<Div>，包一层普通 div 以返回 Div。
    div().child(wrap)
}

/// 批量粘贴区：分隔方式选择 + 多行文本框 + 待添加统计 + 添加到已选。
fn data_filter_batch_paste_section(
    tab_id: TabId,
    draft: &DataFilterValueDraft,
    batch_input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let current_separator = draft.batch_separator;
    let (added_preview, dup_count, _ignored_empty) =
        filter_batch_values_to_add(&draft.batch_text, draft.batch_separator, &draft.values);
    let new_count = added_preview.len();
    let mut separators = div().flex().items_center().gap_1();
    for separator in [BatchSeparator::Newline, BatchSeparator::Comma, BatchSeparator::Tab] {
        let selected = separator == current_separator;
        separators = separators.child(
            div()
                .h(px(20.))
                .px_2()
                .rounded(colors.radius)
                .bg(if selected {
                    rgb(0x1677ff)
                } else {
                    colors.panel_alt
                })
                .text_size(px(11.))
                .text_color(if selected { rgb(0xffffff) } else { colors.text })
                .cursor_pointer()
                .flex()
                .items_center()
                .hover(move |style| style.bg(if selected { rgb(0x1677ff) } else { colors.hover }))
                .child(separator.label())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.set_data_filter_batch_separator(tab_id, separator, cx);
                        cx.stop_propagation();
                    }),
                ),
        );
    }

    div()
        .px_3()
        .pt_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("批量粘贴"),
                )
                .child(separators),
        )
        .child(
            div()
                .h(px(72.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .p_2()
                .child(
                    Input::new(&batch_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(12.)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(format!("忽略空行，自动去重。待添加 {new_count} 个新值，重复 {dup_count} 个")),
                )
                .child(
                    data_filter_value_small_button("添加到已选", colors).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.add_data_filter_batch_values(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
        .pb_2()
}

/// 建议值复选框列表：勾选状态与草稿已选值实时同步；搜索只过滤建议值，不改已选。
///
/// 每条用稳定的 ElementId（以值本身为键），和表格行一样让 item 状态化：
/// hover 变色只在当前行局部重绘，不重建整棵值弹层。不使用虚拟列表——它对
/// 几十行的小列表会因滚动回收行导致 hover 命中/状态错乱（悬停不显示、滚动才出现）。
fn data_filter_suggested_values_list(
    tab_id: TabId,
    rule_index: usize,
    values: &[String],
    selected: &BTreeSet<String>,
    search: &str,
    colors: &UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div()
        .max_h(px(150.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    if values.is_empty() {
        list = list.child(data_filter_menu_empty_item(
            if search.trim().is_empty() {
                "没有建议值"
            } else {
                "没有匹配值"
            },
            *colors,
        ));
    } else {
        for value in values {
            let checked = selected.contains(value);
            let value_for_click = value.clone();
            let item_id = value.clone();
            list = list.child(
                data_filter_value_item(value.clone(), checked, *colors)
                    .id(item_id)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.toggle_data_filter_value(
                                tab_id,
                                rule_index,
                                value_for_click.clone(),
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
    }
    // overflow_y_scrollbar 返回 Scrollable<Div>，包一层普通 div 以返回 Div。
    div().child(list)
}
