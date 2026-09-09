fn field_filter_popover_layer(
    tab_id: TabId,
    all_fields: &[String],
    visible_fields: &BTreeSet<String>,
    search: &str,
    search_input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let normalized_search = search.trim().to_ascii_lowercase();
    let filtered_fields = all_fields
        .iter()
        .filter(|field| {
            normalized_search.is_empty()
                || field
                    .to_ascii_lowercase()
                    .contains(normalized_search.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    let visible_count = visible_fields.len();
    let total_count = all_fields.len();
    let all_fields_vec = all_fields.to_vec();

    let mut list = div()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .px_2()
        .py_1();

    for field in filtered_fields {
        let checked = visible_fields.contains(&field);
        let field_for_click = field.clone();
        let all_fields_for_click = all_fields_vec.clone();
        list = list.child(
            field_filter_field_row(field, checked, search, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_visible_table_field(
                        tab_id,
                        all_fields_for_click.clone(),
                        field_for_click.clone(),
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        );
    }

    div()
        .absolute()
        .top(px(42.))
        .right(px(12.))
        .w(px(360.))
        .h(px(468.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(if colors.is_dark {
            rgb(0x1f2024)
        } else {
            rgb(0xffffff)
        })
        .shadow(vec![box_shadow(
            0.,
            12.,
            28.,
            0.,
            hsla(0., 0., 0., if colors.is_dark { 0.52 } else { 0.18 }),
        )])
        .flex()
        .flex_col()
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .h(px(40.))
                .px_3()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("字段筛选"),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(format!("{visible_count}/{total_count}")),
                ),
        )
        .child(
            div()
                .h(px(50.))
                .px_3()
                .border_b_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(AppIcon::Search, 17., colors.muted))
                .child(
                    Input::new(&search_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .text_size(px(14.)),
                ),
        )
        .child(list)
        .child(
            div()
                .h(px(48.))
                .border_t_1()
                .border_color(colors.border)
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child("至少保留一列可见。"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(field_filter_footer_action("反选", colors).on_mouse_down(
                            MouseButton::Left,
                            cx.listener({
                                let all_fields = all_fields_vec.clone();
                                move |this, _, _, cx| {
                                    this.invert_visible_table_fields(
                                        tab_id,
                                        all_fields.clone(),
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }
                            }),
                        ))
                        .child(
                            field_filter_footer_action("显示全部", colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.show_all_table_fields(tab_id, all_fields_vec.clone(), cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}

fn field_filter_field_row(label: String, checked: bool, search: &str, colors: UiColors) -> Div {
    div()
        .h(px(32.))
        .rounded(colors.radius)
        .px_1()
        .flex()
        .items_center()
        .gap_3()
        .text_size(px(14.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .size(px(18.))
                .rounded(colors.radius)
                .border_1()
                .border_color(if checked {
                    if colors.is_dark {
                        rgb(0xd7dae2)
                    } else {
                        rgb(0x8892a0)
                    }
                } else {
                    colors.border
                })
                .bg(if checked {
                    if colors.is_dark {
                        rgb(0xd7dae2)
                    } else {
                        rgb(0xf1f3f6)
                    }
                } else {
                    colors.input_bg
                })
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if colors.is_dark {
                    rgb(0x14161a)
                } else {
                    rgb(0x20242a)
                })
                .child(if checked { "✓" } else { "" }),
        )
        .child(
            div()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(data_filter_menu_label(label.as_str(), Some(search), colors)),
        )
}

fn field_filter_footer_action(label: &'static str, colors: UiColors) -> Div {
    div()
        .h(px(26.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .cursor_pointer()
        .flex()
        .items_center()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

