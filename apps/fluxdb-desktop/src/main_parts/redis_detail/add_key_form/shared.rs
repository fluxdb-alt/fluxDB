// 「新增 Key」子表单公共构建块：区块标题、行标签、删除行图标按钮、「+ 添加」按钮。
// 这些辅助函数被各个类型子表单（string/hash/zset/set/list/stream/json）复用，
// 保持行内交互（删除/追加）与 RedisInsight AddKey 一致的形态。

/// 子表单区块标题（如 Key Name / TTL 的输入标签），小号普通字重。
fn redis_add_key_form_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .flex_none()
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.muted)
        .child(label)
}

/// 可删行的删除按钮：`AppIcon::Close` 图标，悬停高亮；仅当 `can_delete && !applying` 可点击，
/// 点击触发 `remove(row_index)`，并阻止冒泡避免误关抽屉。
fn redis_add_key_remove_row_button(
    row_index: usize,
    can_delete: bool,
    applying: bool,
    colors: UiColors,
    remove: impl Fn(&mut NavicatMain, usize, &mut Window, &mut Context<NavicatMain>) + 'static,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let color = if can_delete { rgb(0xe5484d) } else { colors.border };
    div()
        .size(px(28.))
        .flex_none()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .text_color(color)
        .when(can_delete && !applying, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                if !applying && can_delete {
                    remove(this, row_index, window, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon(AppIcon::Close, 14., color))
}

/// 「+ 添加」行按钮：追加一行新输入并聚焦新行，阻止冒泡避免误关抽屉。
fn redis_add_key_add_row_button(
    label: &'static str,
    applying: bool,
    colors: UiColors,
    add: impl Fn(&mut NavicatMain, &mut Window, &mut Context<NavicatMain>) + 'static,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(30.))
        .px_2()
        .flex_none()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x1677ff))
        .when(!applying, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                if !applying {
                    add(this, window, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon(AppIcon::Plus, 12., rgb(0x1677ff)))
        .child(label)
}
