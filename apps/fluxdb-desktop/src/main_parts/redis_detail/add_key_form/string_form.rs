// 「新增 Key」String 类型子表单：单个值输入，使用大号多行文本输入（便于输入长文本，对齐 RedisInsight Value 编辑形态）。
// 状态保存在 NavicatMain 的 string 输入实体，提交时由控制器读取。

fn redis_add_key_string_form(
    string_input: &Entity<InputState>,
    colors: UiColors,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(redis_add_key_form_label("Value", colors))
        .child(
            div()
                .w_full()
                .min_h(px(140.))
                .h(px(140.))
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .overflow_hidden()
                .child(
                    Input::new(string_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .px_2()
                        .py_2()
                        .font_family("Menlo")
                        .text_size(px(12.))
                        .line_height(px(18.)),
                ),
        )
}
