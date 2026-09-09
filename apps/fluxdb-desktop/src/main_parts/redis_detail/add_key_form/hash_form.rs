// 「新增 Key」Hash 类型子表单：Field/Value/TTL 可增删行，每个字段支持独立 TTL（Redis 7.4+ 字段级 TTL）。
// 对齐 RedisInsight AddKey 的 Hash 多字段编辑形态，并复用 Hash 详情页「新增字段」的三列行布局。

fn redis_add_key_hash_form(
    rows: &[RedisAddKeyHashRow],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(redis_add_key_form_label("添加字段", colors))
        .child(redis_add_key_hash_rows_panel(
            rows,
            applying,
            colors,
            cx,
        ))
}
