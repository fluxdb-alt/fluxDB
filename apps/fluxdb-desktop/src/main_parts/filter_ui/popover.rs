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
    let fallback_field = tr("app.filter.field");
    let selected_field = rule
        .and_then(|rule| rule.field.as_deref())
        .or_else(|| field_names.first().map(String::as_str))
        .unwrap_or(fallback_field.as_str());
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
            colors,
            cx,
        ),
        DataFilterPopoverKind::Operator => data_filter_operator_menu(
            tab_id,
            rule_index,
            selected_operator,
            colors,
            cx,
        ),
        // 单值形态用 Navicat 式单值弹层（输入框 + 建议值 + 确定/取消）；
        // 区间/列表才用带「已选 N 个值 / 批量粘贴」的多值弹层。
        DataFilterPopoverKind::Value => {
            if selected_operator.value_arity() == DataFilterValueArity::Single {
                data_filter_single_value_menu(
                    tab_id,
                    rule_index,
                    recommended_values,
                    value_draft,
                    value_input,
                    search_input,
                    value_search,
                    value_search_loading,
                    colors,
                    cx,
                )
            } else {
                data_filter_value_menu(
                    tab_id,
                    rule_index,
                    recommended_values,
                    value_draft,
                    value_input,
                    search_input,
                    batch_input,
                    value_search,
                    value_search_loading,
                    colors,
                    cx,
                )
            }
        }
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
        // 弹层按触发点的窗口坐标显示（下移 4px 避免盖住被点的文字），
        // 贴近窗口边缘时由 anchored 自动避让，不再写死 top/left；
        // deferred 让它画在表格/筛选面板之上，不被裁切。
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::TopLeft)
                    .position(popover.anchor)
                    .offset(point(px(0.), px(4.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(menu),
            )
            .with_priority(2),
        )
}

fn data_filter_panel_height(
    rules: &[DataFilterRule],
    mode: DataFilterMode,
    available_data_height: f32,
) -> f32 {
    let max_height = (available_data_height * DATA_FILTER_PANEL_MAX_DATA_RATIO)
        .clamp(DATA_FILTER_PANEL_MIN_HEIGHT, DATA_FILTER_PANEL_MAX_HEIGHT)
        .min(available_data_height);
    let filter_height = if rules.is_empty() {
        28.
    } else {
        let mut height: f32 = 0.;
        let mut previous_depth = 0u8;
        for rule in rules {
            if rule.group_depth > previous_depth {
                height += f32::from(rule.group_depth - previous_depth) * 18.;
            } else if rule.group_depth < previous_depth {
                height += f32::from(previous_depth - rule.group_depth) * 18.;
            }
            previous_depth = rule.group_depth;
            if rule.advanced_expression.is_some() {
                height += 36.;
            } else if rule.group_depth > 0 {
                height += 24.;
            } else {
                height += 28.;
            }
        }
        height += f32::from(previous_depth) * 18.;
        height
    };
    let fixed_height = 38. + 38.;
    let content_height = if mode == DataFilterMode::Text {
        112.
    } else {
        filter_height
    };
    (content_height + fixed_height)
        .max(DATA_FILTER_PANEL_MIN_HEIGHT)
        .min(max_height)
}

fn clamp_data_filter_panel_height(height: f32, available_data_height: f32) -> f32 {
    let max_height = (available_data_height * DATA_FILTER_PANEL_MAX_DATA_RATIO)
        .clamp(DATA_FILTER_PANEL_MIN_HEIGHT, DATA_FILTER_PANEL_MAX_HEIGHT)
        .min(available_data_height);
    height.clamp(DATA_FILTER_PANEL_MIN_HEIGHT, max_height)
}

#[cfg(test)]
mod data_filter_panel_height_tests {
    use super::*;

    fn rule(index: usize) -> DataFilterRule {
        DataFilterRule {
            enabled: true,
            field: Some(format!("field_{index}")),
            operator: DataFilterOperator::Eq,
            values: vec![index.to_string()],
            advanced_expression: None,
            typed_values: None,
            connector: fluxdb_core::FilterCombinator::And,
            group_depth: 0,
            group_end: 0,
        }
    }

    #[test]
    fn many_rules_never_exceed_thirty_five_percent_of_data_area() {
        let rules = (0..40).map(rule).collect::<Vec<_>>();
        for available in [400., 700., 1000., 1600.] {
            let height = data_filter_panel_height(&rules, DataFilterMode::Builder, available);
            assert!(height <= available * DATA_FILTER_PANEL_MAX_DATA_RATIO);
            assert!(height >= DATA_FILTER_PANEL_MIN_HEIGHT);
        }
    }

    #[test]
    fn text_mode_and_user_height_keep_fixed_actions_within_the_cap() {
        let available = 700.;
        let max = available * DATA_FILTER_PANEL_MAX_DATA_RATIO;
        let default_height =
            data_filter_panel_height(&[], DataFilterMode::Text, available);
        assert!(default_height <= max);
        assert_eq!(
            clamp_data_filter_panel_height(DATA_FILTER_PANEL_MAX_HEIGHT, available),
            max
        );
    }
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

    let mut list = div().flex().flex_col();

    let mut index = 0;
    let mut current_depth = 0u8;
    while index < rules.len() {
        let rule = &rules[index];
        if rule.group_depth > current_depth {
            for depth in current_depth + 1..=rule.group_depth {
                list = list.child(data_filter_group_open_row(depth, colors));
            }
            current_depth = rule.group_depth;
        } else if rule.group_depth < current_depth {
            while current_depth > rule.group_depth {
                let closing_depth = current_depth;
                current_depth -= 1;
                list = list.child(data_filter_group_close_row(
                    tab_id,
                    index.saturating_sub(1),
                    closing_depth,
                    current_depth == rule.group_depth,
                    rules[index.saturating_sub(1)].connector,
                    false,
                    default_field.clone(),
                    colors,
                    cx,
                ));
            }
        }
        let show_connector = rules
            .get(index + 1)
            .is_some_and(|next| next.group_depth >= rule.group_depth);
        list = list.child(data_filter_rule_row(
            tab_id,
            index,
            rule,
            default_field.clone(),
            rule.group_depth,
            show_connector,
            colors,
            cx,
        ));
        index += 1;
    }
    while current_depth > 0 {
        let closing_depth = current_depth;
        current_depth -= 1;
        list = list.child(data_filter_group_close_row(
            tab_id,
            rules.len().saturating_sub(1),
            closing_depth,
            false,
            rules.last().map(|rule| rule.connector).unwrap_or_default(),
            current_depth == 0,
            default_field.clone(),
            colors,
            cx,
        ));
    }
    list
}

fn data_filter_group_open_row(depth: u8, colors: UiColors) -> Div {
    div()
        .h(px(18.))
        .pl(px(12. + f32::from(depth.saturating_sub(1)) * 24.))
        .pr_3()
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(colors.hover))
        .child(div().w(px(28.)))
        .child(
            div()
                .text_size(ui_text_size(14.))
                .line_height(ui_text_size(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(0x006bd6))
                .child("("),
        )
        .child(div().flex_1())
}

fn data_filter_group_close_row(
    tab_id: TabId,
    last_rule_index: usize,
    group_depth: u8,
    show_connector: bool,
    connector: fluxdb_core::FilterCombinator,
    show_actions: bool,
    default_field: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let group_default_field = default_field.clone();
    div()
        .h(px(18.))
        .pl(px(12. + f32::from(group_depth.saturating_sub(1)) * 24.))
        .pr_3()
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(colors.hover))
        .child(div().w(px(28.)))
        .child(
            div()
                .text_size(ui_text_size(14.))
                .line_height(ui_text_size(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(0x006bd6))
                .child(")"),
        )
        .when(show_connector, |this| {
            this.child(data_filter_connector_button(
                tab_id,
                last_rule_index,
                connector,
                colors,
                cx,
            ))
        })
        .when(show_actions, |this| {
            this.child(data_filter_small_button("+", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.add_data_filter_rule_after(
                        tab_id,
                        last_rule_index,
                        default_field.clone(),
                        group_depth.saturating_sub(1),
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
                        last_rule_index,
                        group_default_field.clone(),
                        group_depth,
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
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.filter.add_rule_hint")),
        )
}

fn data_filter_rule_row(
    tab_id: TabId,
    rule_index: usize,
    rule: &DataFilterRule,
    default_field: Option<String>,
    group_depth: u8,
    show_and: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if let Some(expression) = rule.advanced_expression.as_deref() {
        return data_filter_advanced_rule_row(
            tab_id,
            rule_index,
            expression,
            rule.enabled,
            show_and,
            rule.connector,
            group_depth,
            default_field.clone(),
            colors,
            cx,
        );
    }
    let fallback_field = tr("app.filter.field");
    let selected_field = rule
        .field
        .as_deref()
        .or(default_field.as_deref())
        .unwrap_or(fallback_field.as_str());
    let add_default_field = default_field.clone();
    let group_default_field = default_field.clone();
    let nested = group_depth > 0;
    let indent = 12. + f32::from(group_depth) * 24.;
    // 值形态决定值槽文案与弹层形态：单值只显示那一个值，区间/列表显示值个数。
    let arity = rule.operator.value_arity();
    let selected_values = rule
        .operator
        .requires_values()
        .then(|| rule.values.to_vec());
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
        .when(rule.enabled, |this| {
            this.bg(if colors.is_dark {
                rgb(0x18314d)
            } else {
                rgb(0xeaf4fe)
            })
        })
        .hover(move |style| style.bg(colors.hover))
        .child(data_filter_check(rule.enabled, colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_data_filter_rule_enabled(tab_id, rule_index, cx);
                cx.stop_propagation();
            }),
        ))
        .child(
            data_filter_clickable_text(selected_field.to_string(), colors)
                .debug_selector(move || format!("data-filter-field-{rule_index}"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.toggle_data_filter_popover(
                            tab_id,
                            Some(rule_index),
                            DataFilterPopoverKind::Field,
                            event.position,
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
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.toggle_data_filter_popover(
                        tab_id,
                        Some(rule_index),
                        DataFilterPopoverKind::Operator,
                        event.position,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        )
        .when(arity != DataFilterValueArity::None, |this| {
            this.child(if arity == DataFilterValueArity::Single {
                data_filter_single_value_slot(tab_id, rule_index, rule, colors, cx)
            } else {
                data_filter_clickable_text(
                    value_label.clone().unwrap_or_else(|| "<?>".to_string()),
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.toggle_data_filter_popover(
                            tab_id,
                            Some(rule_index),
                            DataFilterPopoverKind::Value,
                            event.position,
                            window,
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )
            })
        })
        .when(show_and, |this| {
            this.child(data_filter_connector_button(
                tab_id,
                rule_index,
                rule.connector,
                colors,
                cx,
            ))
        })
        .when(!show_and, |this| {
            this.child(
                data_filter_small_button("+", colors)
                    .debug_selector(move || format!("data-filter-add-rule-{rule_index}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.add_data_filter_rule_after(
                                tab_id,
                                rule_index,
                                add_default_field.clone(),
                                group_depth,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child(
                data_filter_small_button("()", colors)
                    .debug_selector(move || format!("data-filter-add-group-{rule_index}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.add_data_filter_rule_after(
                                tab_id,
                                rule_index,
                                group_default_field.clone(),
                                group_depth.saturating_add(1),
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
            )
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

/// 输入条件无法无损拆成字段/运算符/值时保留的高级条件行。
/// 它只读，但和普通规则一样可禁用/删除并显示全局 AND/OR。
fn data_filter_advanced_rule_row(
    tab_id: TabId,
    rule_index: usize,
    expression: &str,
    enabled: bool,
    show_and: bool,
    connector: fluxdb_core::FilterCombinator,
    group_depth: u8,
    default_field: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(36.))
        .pl(px(12. + f32::from(group_depth) * 24.))
        .pr_3()
        .flex()
        .items_center()
        .gap_2()
        .when(enabled, |this| {
            this.bg(if colors.is_dark {
                rgb(0x18314d)
            } else {
                rgb(0xeaf4fe)
            })
        })
        .hover(move |style| style.bg(colors.hover))
        .child(data_filter_check(enabled, colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_data_filter_rule_enabled(tab_id, rule_index, cx);
                cx.stop_propagation();
            }),
        ))
        .child(
            div()
                .debug_selector(|| format!("data-filter-advanced-rule-{rule_index}"))
                .flex()
                .flex_col()
                .min_w(px(0.))
                .flex_1()
                .overflow_hidden()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.filter.advanced_condition").to_string()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(expression.to_string()),
                ),
        )
        .when(show_and, |this| {
            this.child(data_filter_connector_button(
                tab_id,
                rule_index,
                connector,
                colors,
                cx,
            ))
        })
        .when(!show_and, |this| {
            let add_default_field = default_field.clone();
            let group_default_field = default_field;
            this.child(data_filter_small_button("+", colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.add_data_filter_rule_after(
                        tab_id,
                        rule_index,
                        add_default_field.clone(),
                        group_depth,
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
                        group_depth.saturating_add(1),
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ))
        })
        .child(data_filter_delete_button(colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.delete_data_filter_rule(tab_id, rule_index, cx);
                cx.stop_propagation();
            }),
        ))
}

fn data_filter_connector_button(
    tab_id: TabId,
    rule_index: usize,
    connector: fluxdb_core::FilterCombinator,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .debug_selector(move || format!("data-filter-connector-{rule_index}"))
        .h(px(18.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .text_size(ui_text_size(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x006bd6))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| {
            style.bg(if colors.is_dark {
                rgb(0x244565)
            } else {
                rgb(0xc8e5fb)
            })
        })
        .child(filter_combinator_label(connector))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.toggle_data_filter_combinator(tab_id, rule_index, cx);
                cx.stop_propagation();
            }),
        )
}

fn data_filter_value_label(selected_values: &[String]) -> String {
    if selected_values.is_empty() {
        "<?>".to_string()
    } else if selected_values.len() == 1 {
        selected_values[0].clone()
    } else {
        tr("app.filter.value_count").replace("%{count}", &selected_values.len().to_string())
    }
}

/// 单值运算符的值槽：点开 Navicat 式的单值弹层（输入框 + 建议值 + 确定/取消），
/// 与区间/列表的多选弹层区分开——单值没有「已选 N 个值」这回事。
fn data_filter_single_value_slot(
    tab_id: TabId,
    rule_index: usize,
    rule: &DataFilterRule,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let label = rule
        .values
        .first()
        .cloned()
        .unwrap_or_else(|| "<?>".to_string());
    data_filter_clickable_text(label, colors).on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
            this.toggle_data_filter_popover(
                tab_id,
                Some(rule_index),
                DataFilterPopoverKind::Value,
                event.position,
                window,
                cx,
            );
            cx.stop_propagation();
        }),
    )
}

fn data_sort_builder_section(
    tab_id: TabId,
    sort_rules: &[DataSortRule],
    can_undo_clear: bool,
    default_field: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut pills = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2();
    let add_field = sort_rules
        .last()
        .map(|rule| rule.field.clone())
        .or_else(|| default_field.clone());
    for (sort_index, rule) in sort_rules.iter().enumerate() {
        pills = pills.child(data_sort_rule_pill(
            tab_id,
            sort_index,
            rule,
            colors,
            cx,
        ));
    }
    pills = pills.child(
        data_filter_small_button("+", colors)
            .debug_selector(|| "data-sort-add".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.add_data_sort_rule(tab_id, add_field.clone(), cx);
                    cx.stop_propagation();
                }),
            ),
    );
    if !sort_rules.is_empty() {
        pills = pills
            .child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .bg(colors.border_soft)
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .debug_selector(|| "data-sort-clear".into())
                    .size(px(24.))
                    .rounded(colors.radius)
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(move |style| style.bg(colors.hover))
                    .child(app_icon(AppIcon::Trash, 13., colors.muted))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.clear_data_sort_rules(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
            );
    }
    if can_undo_clear {
        pills = pills.child(
            div()
                .debug_selector(|| "data-sort-undo-clear".into())
                .h(px(24.))
                .px_2()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .text_size(ui_text_size(12.))
                .text_color(colors.text)
                .cursor_pointer()
                .flex()
                .items_center()
                .hover(move |style| style.bg(colors.hover))
                .child(tr("app.common.undo"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.undo_clear_data_sort_rules(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
        );
    }
    div()
        .min_h(px(38.))
        .px_3()
        .py_1()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_3()
        .child(data_filter_section_label(tr("app.filter.sort_mode"), colors))
        .child(pills)
}

fn data_filter_section_label(label: impl Into<SharedString>, colors: UiColors) -> Div {
    let label = label.into();
    div()
        .text_size(ui_text_size(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(label)
}

fn data_sort_rule_pill(
    tab_id: TabId,
    sort_index: usize,
    rule: &DataSortRule,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let direction_tooltip: SharedString = if rule.ascending {
        tr("app.context.sort_desc")
    } else {
        tr("app.context.sort_asc")
    };
    div()
        .debug_selector(move || format!("data-sort-pill-{sort_index}"))
        .h(px(24.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(if colors.is_dark {
            rgb(0x13233a)
        } else {
            rgb(0xf7faff)
        })
        .flex()
        .items_center()
        .overflow_hidden()
        .text_color(colors.text)
        .on_drop(cx.listener(move |this, drag: &DataSortDrag, _, cx| {
            if drag.tab_id == tab_id {
                this.move_data_sort_rule(tab_id, drag.from, sort_index, cx);
            }
            cx.stop_propagation();
        }))
        .drag_over::<DataSortDrag>(move |style, _, _, _| {
            style.border_color(rgb(0x1677ff)).bg(if colors.is_dark {
                rgb(0x18314d)
            } else {
                rgb(0xeaf4ff)
            })
        })
        .child(
            div()
                .id(("data-sort-handle", sort_index))
                .debug_selector(move || format!("data-sort-handle-{sort_index}"))
                .h_full()
                .w(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_move()
                .child(app_icon(AppIcon::GripVertical, 12., colors.muted))
                .on_drag(
                    DataSortDrag {
                        tab_id,
                        from: sort_index,
                        field: rule.field.clone(),
                        colors,
                    },
                    |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    },
                ),
        )
        .child(
            div()
                .debug_selector(move || format!("data-sort-field-{sort_index}"))
                .h_full()
                .px_1()
                .flex()
                .items_center()
                .text_size(ui_text_size(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .child(rule.field.clone())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.data_filter_popover = Some(DataFilterPopover {
                            tab_id,
                            kind: DataFilterPopoverKind::SortField,
                            rule_index: None,
                            sort_index: Some(sort_index),
                            anchor: event.position,
                        });
                        this.sync_table_hover_overlay_block(cx);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            div()
                .id(("data-sort-direction", sort_index))
                .debug_selector(move || format!("data-sort-direction-{sort_index}"))
                .h_full()
                .w(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .tooltip(move |window, cx| {
                    Tooltip::new(direction_tooltip.clone()).build(window, cx)
                })
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
                .debug_selector(move || format!("data-sort-remove-{sort_index}"))
                .h_full()
                .w(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .child(app_icon(AppIcon::Close, 11., colors.muted))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.delete_data_sort_rule(tab_id, sort_index, cx);
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
        .text_size(ui_text_size(12.))
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
        list = list.child(data_filter_menu_empty_item(
            tr("app.filter.no_matching_fields"),
            colors,
        ));
    }

    data_filter_menu_surface(200., 260., colors)
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
        list = list.child(data_filter_menu_empty_item(
            tr("app.filter.no_matching_fields"),
            colors,
        ));
    }

    data_filter_menu_surface(200., 260., colors)
        .child(list)
        .child(data_filter_menu_search_footer(search_input, colors))
}

fn data_filter_operator_menu(
    tab_id: TabId,
    rule_index: usize,
    selected_operator: DataFilterOperator,
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

    data_filter_menu_surface(132., 300., colors).child(list)
}


/// 单值形态的值弹层：输入框 + 建议值 + 搜索 + 确定/取消（对齐 Navicat 的单值编辑）。
///
/// 与多值弹层的区别只在「只有一个值」：没有已选标签区、清空、批量粘贴，
/// 建议值勾选即把该值填进输入框，再点同一项取消勾选（清空输入）。
fn data_filter_single_value_menu(
    tab_id: TabId,
    rule_index: usize,
    recommended_values: Vec<String>,
    draft: Option<&DataFilterValueDraft>,
    value_input: Entity<InputState>,
    search_input: Entity<InputState>,
    search: &str,
    search_loading: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let draft = draft.filter(|d| d.tab_id == tab_id && d.rule_index == rule_index);
    let field_label = draft
        .map(|draft| draft.field.clone())
        .filter(|field| !field.is_empty())
        .unwrap_or_else(|| tr("app.filter.filter_value").to_string());
    let current_value = draft.map(|draft| draft.input.clone()).unwrap_or_default();

    div()
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
        .flex()
        .flex_col()
        .occlude()
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .key_context("DataFilterSingleValueMenu")
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
        // 输入值：单值就是输入框本身，回车或「确定」写回条件。
        .child(
            div()
                .px_2()
                .pt_2()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .px_1()
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.filter.input_value")),
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
                        .child(
                            Input::new(&value_input)
                                .appearance(false)
                                .focus_bordered(false)
                                .w_full()
                                .h_full()
                                .text_size(ui_text_size(13.)),
                        ),
                ),
        )
        // 建议值：勾选即设为该值，搜索只过滤建议值。
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
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.filter.suggested_values")),
                )
                .child(data_filter_single_suggested_values_list(
                    recommended_values,
                    &current_value,
                    search,
                    search_loading,
                    &colors,
                    cx,
                )),
        )
        .child(data_filter_menu_search_footer(search_input, colors))
        // 底部按钮
        .child(
            div()
                .h(px(44.))
                .border_t_1()
                .border_color(colors.border_soft)
                .px_2()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(data_filter_dialog_button(tr("app.common.cancel"), false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.cancel_data_filter_value(cx);
                        cx.stop_propagation();
                    }),
                ))
                .child(data_filter_dialog_button(tr("app.filter.ok"), true, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.apply_data_filter_value_draft(tab_id, cx);
                        cx.stop_propagation();
                    }),
                )),
        )
}

/// 单值建议值列表：勾选状态 = 输入框当前值；点击切换（选中→清空，空→填入）。
fn data_filter_single_suggested_values_list(
    values: Vec<String>,
    current_value: &str,
    search: &str,
    search_loading: bool,
    colors: &UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut list = div()
        .max_h(px(240.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col();

    if values.is_empty() {
        list = list.child(data_filter_menu_empty_item(
            if search.trim().is_empty() && !search_loading {
                tr("app.filter.no_suggestions")
            } else {
                tr("app.filter.no_matching_values")
            },
            *colors,
        ));
    } else {
        for value in values {
            let checked = value == current_value;
            let value_for_click = value.clone();
            let item_id = format!("single:{value}");
            list = list.child(
                data_filter_value_item(value, checked, *colors)
                    .id(item_id)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.pick_data_filter_single_value(
                                value_for_click.clone(),
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
    }
    div().child(list)
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
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let draft = draft.filter(|d| d.tab_id == tab_id && d.rule_index == rule_index);
    let selected = draft.map(|draft| draft.values.clone()).unwrap_or_default();
    let field_label = draft
        .map(|draft| draft.field.clone())
        .filter(|field| !field.is_empty())
        .unwrap_or_else(|| tr("app.filter.value").to_string());
    let selected_count = selected.len();

    div()
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
                                .text_size(ui_text_size(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(
                                    tr("app.filter.selected_count")
                                        .replace("%{count}", &selected_count.to_string()),
                                ),
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
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.placeholder.input_value")),
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
                                .text_size(ui_text_size(13.)),
                        )
                        .child(
                            data_filter_value_small_button(
                                tr("app.filter.add").to_string(),
                                colors,
                            )
                            .on_mouse_down(
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
                                .text_size(ui_text_size(11.))
                                .text_color(colors.muted)
                                .child(tr("app.filter.dedupe_hint")),
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
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.filter.suggested_values")),
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
                                .text_size(ui_text_size(13.)),
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
                        .text_size(ui_text_size(12.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.filter.selected_count")
                                .replace("%{count}", &selected_count.to_string()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            data_filter_dialog_button(tr("app.common.cancel"), false, colors)
                                .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.cancel_data_filter_value(cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            data_filter_dialog_button(tr("app.filter.ok"), true, colors)
                                .on_mouse_down(
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
                .text_size(ui_text_size(14.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(tr("app.filter.edit_values")),
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
                        .text_size(ui_text_size(12.))
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
        .text_size(ui_text_size(11.))
        .text_color(colors.muted)
        .cursor_pointer()
        .flex()
        .items_center()
        .hover(move |style| style.bg(colors.hover))
        .child(tr("app.filter.clear"))
}

fn data_filter_value_small_button(label: impl Into<String>, colors: UiColors) -> Div {
    let label = label.into();
    div()
        .h(px(20.))
        .px_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .text_size(ui_text_size(12.))
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
        .text_size(ui_text_size(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x006bd6))
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon(if open { AppIcon::ChevronUp } else { AppIcon::ChevronDown }, 12., rgb(0x006bd6)))
        .child(if open {
            tr("app.filter.hide_batch_paste")
        } else {
            tr("app.filter.batch_paste")
        })
}

/// 已选值标签区：短值横向排列自动换行，UUID 等长值截断并提示查看；数量多时限制高度滚动。
fn data_filter_selected_values_tags(
    tab_id: TabId,
    selected: &[String],
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
                .text_size(ui_text_size(12.))
                .text_color(colors.muted)
                .child(tr("app.filter.no_selected_values")),
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
            .text_size(ui_text_size(12.))
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
                .text_size(ui_text_size(11.))
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
                        .text_size(ui_text_size(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(tr("app.filter.batch_paste")),
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
                        .text_size(ui_text_size(12.)),
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
                        .text_size(ui_text_size(11.))
                        .text_color(colors.muted)
                        .child(
                            tr("app.filter.batch_summary")
                                .replace("%{new}", &new_count.to_string())
                                .replace("%{duplicate}", &dup_count.to_string()),
                        ),
                )
                .child(
                    data_filter_value_small_button(
                        tr("app.filter.add_to_selected").to_string(),
                        colors,
                    )
                    .on_mouse_down(
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
    selected: &[String],
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
                tr("app.filter.no_suggestions")
            } else {
                tr("app.filter.no_matching_values")
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
