// 「新增 Key」JSON 类型子表单：大号等宽代码编辑器 + 「格式化/校验」助手按钮。
// 助手按钮（AppIcon::Wand）调用控制器 `format_redis_add_key_json`：合法则美化回填，否则提示。
// 对齐 RedisInsight AddKey 的 JSON 粘贴编辑形态。

fn redis_add_key_json_form(
    json_editor: &JsonComponent,
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .child(redis_add_key_form_label(tr("app.redis.add_key.form.json_value"), colors))
                .child(redis_add_key_json_format_button(applying, colors, cx)),
        )
        .child(
            div()
                .w_full()
                .h(px(420.))
                .min_h(px(420.))
                // 组件自带边框/背景/多行编辑器（JSON 语法高亮），这里只给高度。
                .child(json_editor.render_edit_body(colors, applying)),
        )
}

/// 「格式化/校验」助手按钮：点击格式化并校验当前 JSON 输入。
fn redis_add_key_json_format_button(
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(26.))
        .px_2()
        .flex_none()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_1()
        .text_size(ui_text_size(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0x1677ff))
        .when(!applying, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                if !applying {
                    this.format_redis_add_key_json(window, cx);
                }
                cx.stop_propagation();
            }),
        )
        .child(app_icon(AppIcon::Wand, 12., rgb(0x1677ff)))
        .child(tr("app.redis.add_key.form.format_validate"))
}
