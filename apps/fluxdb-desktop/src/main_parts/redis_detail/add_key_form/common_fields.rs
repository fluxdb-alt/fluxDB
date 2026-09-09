// 「新增 Key」公共字段子表单（独立组件，置于三类表之上，对齐 RedisInsight AddKey 首栏）：
// Key Type 类型下拉 + Key Name 名称 + 可选 TTL 过期时间。
// 仅做展示与本地编辑，状态保存在 NavicatMain 的字段实体中，类型切换的重置由控制器订阅处理。

fn redis_add_key_common_fields(
    type_select: &Entity<SelectState<SearchableVec<String>>>,
    name_input: &Entity<InputState>,
    ttl_input: &Entity<InputState>,
    applying: bool,
    colors: UiColors,
) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        // 第一行：Key Type 类型下拉 + TTL 过期时间并排（各占一半宽）
        .child(
            div()
                .w_full()
                .flex()
                .gap_2()
                // Key Type：类型下拉
                // 用 gpui-component 的原生外观（appearance 默认开），让 Select 自己画唯一的一层边框：
                // 常态边框取 `theme.input`（其上被设为 colors.border，明暗随主题一致），
                // 焦点时取 `theme.ring` 作为焦点边框 —— 全程只有 Select 这一层边框，避免外来外层
                // 边框与 Select 内部焦点 ring 叠加成双框（参考 create_database.rs 的 Select 用法）。
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(redis_add_key_form_label("Key Type", colors))
                        .child(
                            div()
                                .h(px(34.))
                                .w_full()
                                .when(applying, |this| this.opacity(0.5))
                                .child(
                                    Select::new(type_select)
                                        .placeholder("选择类型")
                                        .small()
                                        .w_full()
                                        .h_full()
                                        .menu_width(px(220.)),
                                ),
                        ),
                )
                // TTL：过期时间（可选），空串表示永不超时
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(redis_add_key_form_label("TTL（可选，留空为永不超时）", colors))
                        .child(redis_stream_add_input_box(ttl_input.clone(), colors)),
                ),
        )
        // Key Name：键名输入
        .child(redis_add_key_form_label("Key Name", colors))
        .child(redis_stream_add_input_box(name_input.clone(), colors))
}
