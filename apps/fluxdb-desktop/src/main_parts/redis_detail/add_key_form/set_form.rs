// 「新增 Key」Set 类型子表单：Member 可增删行，复用单成员行面板。
// 对齐 RedisInsight AddKey 的 Set 多成员编排形态。

fn redis_add_key_set_form(
    rows: &[RedisAddKeySingleRow],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(redis_add_key_form_label("添加成员", colors))
        .child(redis_add_key_single_rows_panel(
            RedisAddKeyKind::Set,
            rows,
            applying,
            colors,
            cx,
        ))
}
