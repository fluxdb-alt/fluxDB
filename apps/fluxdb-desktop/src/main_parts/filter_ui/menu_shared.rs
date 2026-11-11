/// 弹层表面：宽高固定，位置由调用方用 `anchored` 贴住触发点，不在此写死坐标。
fn data_filter_menu_surface(width: f32, height: f32, colors: UiColors) -> Div {
    div()
        .debug_selector(|| "data-filter-popover-menu".into())
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
        .text_size(ui_text_size(12.))
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
                .text_size(ui_text_size(11.))
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

fn data_filter_menu_empty_item(label: impl Into<SharedString>, colors: UiColors) -> Div {
    let label = label.into();
    div()
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .text_size(ui_text_size(12.))
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
        .text_size(ui_text_size(12.))
        .text_color(colors.muted)
        .child(app_icon(AppIcon::Search, 14., colors.muted))
        .child(
            Input::new(&search_input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .text_size(ui_text_size(13.)),
        )
}

fn data_filter_value_item(label: String, checked: bool, colors: UiColors) -> Div {
    div()
        .h(px(22.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .text_size(ui_text_size(12.))
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
                .text_size(ui_text_size(9.))
                .text_color(rgb(0xffffff))
                .child(if checked { "✓" } else { "" }),
        )
        .child(label)
}

fn data_filter_dialog_button(
    label: impl Into<SharedString>,
    primary: bool,
    colors: UiColors,
) -> Div {
    let label = label.into();
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
        .text_size(ui_text_size(12.))
        .text_color(colors.text)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

/// 建议值：取当前页该列的去重值，按列的数值语义排序。
///
/// 排序不能只看显示文本：int 列的 "10" 排在 "2" 前面（文本序）会误导用户。
/// 数值列按大小排、其余按文本序、NULL 恒排最后。
fn data_filter_recommended_values(
    page: &DataPage,
    field_name: &str,
    value_search: &str,
) -> Vec<String> {
    let Some((column_index, column)) = page
        .columns
        .iter()
        .enumerate()
        .find(|(_, column)| column.name == field_name)
    else {
        return Vec::new();
    };
    let search = value_search.trim().to_ascii_lowercase();
    // MySQL / SQLite 的数据页把所有列都投影成文本（见 connectors 的 select 表达式），
    // 数值列拿到的也是 CellValue::Text，因此数值与否以列声明类型为准。
    let numeric_column = is_numeric_column_type(column.type_name.as_deref());
    let mut seen = BTreeSet::new();
    let mut values = page
        .rows
        .iter()
        .filter_map(|row| row.values.get(column_index))
        .filter(|value| {
            search.is_empty()
                || cell_value_label(value)
                    .to_ascii_lowercase()
                    .contains(search.as_str())
        })
        // 按显示值去重：同一列在页内可能以不同 CellValue 变体表示同一值。
        .filter(|value| seen.insert(cell_value_label(value)))
        .cloned()
        .collect::<Vec<_>>();
    values.sort_by(|left, right| recommended_cell_value_order(left, right, numeric_column));
    values
        .into_iter()
        .map(|value| cell_value_label(&value))
        .take(200)
        .collect()
}

/// 建议值的跨类型比较：NULL 最后，数值列按数值大小，其余按文本序。
fn recommended_cell_value_order(
    left: &CellValue,
    right: &CellValue,
    numeric_column: bool,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (CellValue::Null, CellValue::Null) => return Ordering::Equal,
        // NULL 是「没有值」而不是可比较的值，恒排最后，避免混在数值中间。
        (CellValue::Null, _) => return Ordering::Greater,
        (_, CellValue::Null) => return Ordering::Less,
        (CellValue::Bool(left), CellValue::Bool(right)) => return left.cmp(right),
        _ => {}
    }
    if let (Some(left), Some(right)) = (
        recommended_value_number(left, numeric_column),
        recommended_value_number(right, numeric_column),
    ) {
        // total_cmp：非有限值也有确定次序，不会让排序结果依赖输入顺序。
        return left.total_cmp(&right);
    }
    let left_label = cell_value_label(left).to_ascii_lowercase();
    let right_label = cell_value_label(right).to_ascii_lowercase();
    left_label
        .cmp(&right_label)
        .then_with(|| left_label.cmp(&right_label))
}

/// 取参与比较的数值：类型化数值直接取；文本投影的数值列按标签解析。
///
/// 文本列不解析——否则 varchar 代码列（"007"、"8"）会被错误地按数字重排。
fn recommended_value_number(value: &CellValue, numeric_column: bool) -> Option<f64> {
    match value {
        CellValue::I64(value) => Some(*value as f64),
        CellValue::F64(value) => Some(*value),
        _ if numeric_column => cell_value_label(value).trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// 列声明类型是否按数值比较（覆盖 MySQL / PostgreSQL / SQLite 的常用类型名）。
///
/// `money` 等按服务端 locale 格式化的文本类型不在此列：标签不是纯数字，解析会失败。
fn is_numeric_column_type(type_name: Option<&str>) -> bool {
    let Some(type_name) = type_name else {
        return false;
    };
    // 去掉 (12,2) 之类的精度后缀与可能的 schema 前缀。
    let base = type_name.trim().to_ascii_lowercase();
    let base = base.split('(').next().unwrap_or(base.as_str()).trim();
    matches!(
        base,
        "int"
            | "integer"
            | "tinyint"
            | "smallint"
            | "mediumint"
            | "bigint"
            | "int2"
            | "int4"
            | "int8"
            | "serial"
            | "smallserial"
            | "bigserial"
            | "year"
            | "numeric"
            | "decimal"
            | "dec"
            | "float"
            | "float4"
            | "float8"
            | "double"
            | "double precision"
            | "real"
    )
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
        .text_size(ui_text_size(10.))
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
        "+" => app_icon(AppIcon::Plus, 9., rgb(0x1677ff)),
        "()" => data_filter_group_icon(colors).into_any_element(),
        _ => div()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(ui_text_size(12.))
            .line_height(ui_text_size(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(label)
            .into_any_element(),
    };

    div()
        .w(px(18.))
        .h(px(16.))
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
        .w(px(12.))
        .h(px(11.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(ui_text_size(9.))
        .line_height(ui_text_size(9.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x1677ff))
        .child("()")
        .child(
            div()
                .absolute()
                .right(px(-2.))
                .top(px(-2.))
                .size(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .child(app_icon(AppIcon::Plus, 5., rgb(0x1677ff))),
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
        .text_size(ui_text_size(15.))
        .line_height(ui_text_size(15.))
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

