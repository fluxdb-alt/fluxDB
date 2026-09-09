// 「新增 Key」Stream 类型子表单：Entry ID（可选，留空用 `*`） + Field/Value 可增删行。
// 对齐 RedisInsight AddKey 的 Stream 条目 ID 与多字段编排形态。

fn redis_add_key_stream_form(
    stream_id_input: &Entity<InputState>,
    rows: &[RedisAddKeyNameValueRow],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        // Entry ID：可选，留空表示由服务端自动生成 `*`；格式为 `毫秒-序号`。
        .child(redis_add_key_form_label("Entry ID（可选，留空自动生成）", colors))
        .child(redis_stream_add_input_box(stream_id_input.clone(), colors))
        .child(redis_add_key_form_label("添加字段", colors))
        .child(redis_add_key_name_value_rows_panel(
            RedisAddKeyKind::Stream,
            rows,
            applying,
            colors,
            cx,
        ))
}
