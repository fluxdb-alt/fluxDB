// 「新增 Key」ZSet 类型子表单：Member/Score 可增删行，复用名称=值行面板（score 需为数字）。
// 对齐 RedisInsight AddKey 的 ZSet 多成员与分值编排形态。

fn redis_add_key_zset_form(
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
        .child(redis_add_key_form_label("添加成员与分值", colors))
        .child(redis_add_key_name_value_rows_panel(
            RedisAddKeyKind::ZSet,
            rows,
            applying,
            colors,
            cx,
        ))
}
