fn data_filter_menu_surface(width: f32, height: f32, colors: UiColors) -> Div {
    div()
        .absolute()
        .w(px(width))
        .h(px(height))
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
        .on_mouse_move(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
}

fn data_filter_menu_item(
    label: String,
    selected: bool,
    highlight_query: Option<&str>,
    colors: UiColors,
) -> Div {
    div()
        .h(px(21.))
        .px_2()
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(colors.text)
        .bg(if selected {
            if colors.is_dark {
                rgb(0x17385f)
            } else {
                rgb(0xd7ebff)
            }
        } else {
            colors.panel_bg
        })
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .w(px(11.))
                .text_size(px(11.))
                .text_color(colors.text)
                .child(if selected { "✓" } else { "" }),
        )
        .child(data_filter_menu_label(
            label.as_str(),
            highlight_query,
            colors,
        ))
}

fn data_filter_menu_label(label: &str, highlight_query: Option<&str>, colors: UiColors) -> Div {
    let mut content = div()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap();
    let Some(query) = highlight_query
        .map(str::trim)
        .filter(|query| !query.is_empty())
    else {
        return content.child(label.to_string());
    };
    let lower_label = label.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    let Some(first_match) = lower_label.find(query.as_str()) else {
        return content.child(label.to_string());
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
    while let Some(relative_start) = lower_label[search_from..].find(query.as_str()) {
        let start = search_from + relative_start;
        let end = start + query.len();
        if start > cursor {
            content = content.child(label[cursor..start].to_string());
        }
        content = content.child(
            div()
                .rounded(colors.radius * 0.5)
                .px(px(1.5))
                .bg(highlight_bg)
                .text_color(highlight_text)
                .child(label[start..end].to_string()),
        );
        cursor = end;
        search_from = end;
    }
    if cursor < label.len() {
        content = content.child(label[cursor..].to_string());
    }
    content
}

fn data_filter_menu_empty_item(label: &'static str, colors: UiColors) -> Div {
    div()
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(label)
}

fn data_filter_menu_search_footer(search_input: Entity<InputState>, colors: UiColors) -> Div {
    div()
        .h(px(34.))
        .border_t_1()
        .border_color(colors.border_soft)
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(app_icon(AppIcon::Search, 14., colors.muted))
        .child(
            Input::new(&search_input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .text_size(px(13.)),
        )
}

fn data_filter_value_item(label: String, checked: bool, colors: UiColors) -> Div {
    div()
        .h(px(22.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .text_color(colors.text)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .size(px(12.))
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
        .child(label)
}

fn data_filter_dialog_button(label: &'static str, primary: bool, colors: UiColors) -> Div {
    div()
        .h(px(24.))
        .w(px(72.))
        .rounded(colors.radius)
        .border_1()
        .border_color(if primary {
            rgb(0x9db7e8)
        } else {
            colors.border
        })
        .bg(if primary {
            if colors.is_dark {
                rgb(0x1a3157)
            } else {
                rgb(0xffffff)
            }
        } else {
            colors.panel_alt
        })
        .text_size(px(12.))
        .text_color(colors.text)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn data_filter_recommended_values(
    page: &DataPage,
    field_name: &str,
    value_search: &str,
) -> Vec<String> {
    let Some(column_index) = page
        .columns
        .iter()
        .position(|column| column.name == field_name)
    else {
        return Vec::new();
    };
    let search = value_search.trim().to_ascii_lowercase();
    let mut values = page
        .rows
        .iter()
        .filter_map(|row| row.values.get(column_index))
        .map(cell_value_label)
        .filter(|value| search.is_empty() || value.to_ascii_lowercase().contains(search.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    values.truncate(200);
    values
}

fn data_filter_check(checked: bool, colors: UiColors) -> Div {
    div()
        .size(px(14.))
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
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.))
        .text_color(rgb(0xffffff))
        .hover(move |style| {
            style.bg(if checked {
                rgb(0x0f75e8)
            } else if colors.is_dark {
                rgb(0x20242a)
            } else {
                rgb(0xf4f7fb)
            })
        })
        .child(if checked { "✓" } else { "" })
}

fn data_filter_small_button(label: &'static str, colors: UiColors) -> Div {
    let content = match label {
        "+" => app_icon(AppIcon::Plus, 13., rgb(0x1677ff)),
        "()" => data_filter_group_icon(colors).into_any_element(),
        _ => div()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .line_height(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(label)
            .into_any_element(),
    };

    div()
        .w(px(28.))
        .h(px(20.))
        .rounded(colors.radius)
        .border_1()
        .border_color(rgb(0x1677ff))
        .text_color(rgb(0x1677ff))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| {
            style.bg(if colors.is_dark {
                rgb(0x18314d)
            } else {
                rgb(0xe8f2ff)
            })
        })
        .child(content)
}

fn data_filter_group_icon(_colors: UiColors) -> Div {
    div()
        .relative()
        .w(px(18.))
        .h(px(15.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .line_height(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x1677ff))
        .child("()")
        .child(
            div()
                .absolute()
                .right(px(-2.))
                .top(px(-1.))
                .size(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .child(app_icon(AppIcon::Plus, 7., rgb(0x1677ff))),
        )
}

fn data_filter_delete_button(colors: UiColors) -> Div {
    div()
        .size(px(20.))
        .rounded(colors.radius)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(15.))
        .line_height(px(15.))
        .text_color(colors.muted)
        .hover(move |style| {
            style
                .bg(if colors.is_dark {
                    rgb(0x3a2024)
                } else {
                    rgb(0xffeeee)
                })
                .text_color(if colors.is_dark {
                    rgb(0xff9a9a)
                } else {
                    rgb(0xd93025)
                })
        })
        .child(app_icon(AppIcon::Close, 14., colors.muted))
}

