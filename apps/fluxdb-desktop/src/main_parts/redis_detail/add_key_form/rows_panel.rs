// 「新增 Key」可增删行子表单面板：按集合类型渲染名称=值行或单成员行，并带「+ 添加」与行删除。
// Hash / ZSet / Stream 用名称=值行；Set / List 用单成员行。
// 行实体存于 NavicatMain 的对应集合，增删通过控制器 add/remove 委托并聚焦新行。

/// 名称=值 行面板：每行 = 名称输入 + 值输入 + 删除按钮，底部「+ 添加」。
/// `kind` 用于区分 ZSet（Member/Score）与其余（Field/Value）。
fn redis_add_key_name_value_rows_panel(
    kind: RedisAddKeyKind,
    rows: &[RedisAddKeyNameValueRow],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut body = div().w_full().flex().flex_col().gap_2();
    let add = move |this: &mut NavicatMain, window: &mut Window, cx: &mut Context<NavicatMain>| {
        // `kind` 为 Copy，直接按值传递，闭包自身可达 Copy 供循环中多次复用。
        this.add_redis_add_key_row(kind, window, cx);
    };
    let remove = move |this: &mut NavicatMain, index: usize, window: &mut Window, cx: &mut Context<NavicatMain>| {
        this.remove_redis_add_key_row(kind, index, window, cx);
    };

    for (row_index, row) in rows.iter().enumerate() {
        let can_delete = rows.len() > 1;
        body = body.child(
            div()
                .w_full()
                .h(px(34.))
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    redis_stream_add_input_box(row.name.clone(), colors)
                        .flex_1()
                        .min_w(px(0.)),
                )
                .child(
                    redis_stream_add_input_box(row.value.clone(), colors)
                        .flex_1()
                        .min_w(px(0.)),
                )
                .child(redis_add_key_remove_row_button(
                    row_index,
                    can_delete,
                    applying,
                    colors,
                    remove,
                    cx,
                )),
        );
    }
    body.child(redis_add_key_add_row_button(
        "+ 添加行",
        applying,
        colors,
        add,
        cx,
    ))
}

/// Hash 行面板：每行 = 字段名 + 值 + 可选字段级 TTL（秒）+ 删除按钮，底部「+ 添加行」。
/// 列宽对齐 Hash 详情页「新增字段」抽屉的三列布局（Field 固定宽 / Value flex_1 / TTL 固定宽）。
fn redis_add_key_hash_rows_panel(
    rows: &[RedisAddKeyHashRow],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut body = div().w_full().flex().flex_col().gap_2();
    let add = move |this: &mut NavicatMain, window: &mut Window, cx: &mut Context<NavicatMain>| {
        this.add_redis_add_key_row(RedisAddKeyKind::Hash, window, cx);
    };
    let remove = move |this: &mut NavicatMain, index: usize, window: &mut Window, cx: &mut Context<NavicatMain>| {
        this.remove_redis_add_key_row(RedisAddKeyKind::Hash, index, window, cx);
    };

    for (row_index, row) in rows.iter().enumerate() {
        let can_delete = rows.len() > 1;
        body = body.child(
            div()
                .w_full()
                .h(px(34.))
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    redis_stream_add_input_box(row.name.clone(), colors)
                        .w(px(220.))
                        .flex_none(),
                )
                .child(
                    redis_stream_add_input_box(row.value.clone(), colors)
                        .flex_1()
                        .min_w(px(0.)),
                )
                .child(
                    redis_stream_add_input_box(row.ttl.clone(), colors)
                        .w(px(96.))
                        .flex_none(),
                )
                .child(redis_add_key_remove_row_button(
                    row_index,
                    can_delete,
                    applying,
                    colors,
                    remove,
                    cx,
                )),
        );
    }
    body.child(redis_add_key_add_row_button(
        "+ 添加行",
        applying,
        colors,
        add,
        cx,
    ))
}

/// 单成员 行面板：每行 = 成员输入 + 删除按钮，底部「+ 添加」。
/// `placeholder` 区分行输入用途（Set=Member，List=Element），占位已写入对应 InputState。
fn redis_add_key_single_rows_panel(
    kind: RedisAddKeyKind,
    rows: &[RedisAddKeySingleRow],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut body = div().w_full().flex().flex_col().gap_2();
    let add = move |this: &mut NavicatMain, window: &mut Window, cx: &mut Context<NavicatMain>| {
        // `kind` 为 Copy，直接按值传递，闭包自身可达 Copy 供循环中多次复用。
        this.add_redis_add_key_row(kind, window, cx);
    };
    let remove = move |this: &mut NavicatMain, index: usize, window: &mut Window, cx: &mut Context<NavicatMain>| {
        this.remove_redis_add_key_row(kind, index, window, cx);
    };
    for (row_index, row) in rows.iter().enumerate() {
        let can_delete = rows.len() > 1;
        body = body.child(
            div()
                .w_full()
                .h(px(34.))
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    redis_stream_add_input_box(row.value.clone(), colors)
                        .flex_1()
                        .min_w(px(0.)),
                )
                .child(redis_add_key_remove_row_button(
                    row_index,
                    can_delete,
                    applying,
                    colors,
                    remove,
                    cx,
                )),
        );
    }
    body.child(redis_add_key_add_row_button(
        "+ 添加行",
        applying,
        colors,
        add,
        cx,
    ))
}
