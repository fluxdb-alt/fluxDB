// 「新增 Key」List 类型子表单：Element 可增删行 + 插入方向（头/尾）二选一控件。
// 方向对应连接器 RPUSH（尾，默认）/ LPUSH（头），保存在 NavicatMain 的方向字段。

fn redis_add_key_list_form(
    rows: &[RedisAddKeySingleRow],
    direction: RedisListDirection,
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(redis_add_key_form_label("插入方向", colors))
        .child(redis_add_key_direction_control(direction, applying, colors, cx))
        .child(redis_add_key_form_label("添加元素", colors))
        .child(redis_add_key_single_rows_panel(
            RedisAddKeyKind::List,
            rows,
            applying,
            colors,
            cx,
        ))
}

/// 插入方向二选一：Head（左端/LPUSH）与 Tail（右端/RPUSH，默认）。选中项高亮主色，禁用态置灰。
fn redis_add_key_direction_control(
    direction: RedisListDirection,
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut control = div().w_full().h(px(34.)).flex().flex_row().overflow_hidden().rounded(colors.radius).border_1().border_color(colors.border);
    for (option, label) in [
        (RedisListDirection::Head, "头（左端）"),
        (RedisListDirection::Tail, "尾（右端）"),
    ] {
        let selected = direction == option;
        control = control.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .bg(if selected {
                    if colors.is_dark {
                        rgb(0x173766)
                    } else {
                        rgb(0xe8f0ff)
                    }
                } else if colors.is_dark {
                    colors.panel_alt
                } else {
                    rgb(0xf5f5f5)
                })
                .text_color(if selected { rgb(0x1677ff) } else { colors.muted })
                .when(!applying, |this| {
                    this.cursor_pointer().hover(move |style| style.bg(colors.hover))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if !applying {
                            this.set_redis_add_key_list_direction(option, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(label),
        );
    }
    control
}
