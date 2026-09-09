// 「新增 Key」抽屉壳（对齐 RedisInsight AddKey）：从右侧滑出的全高三段式抽屉。
// 顶部：标题区（标题 + 关闭按钮）；中部：可滚动表单区（公共字段 + 当前类型子表单）；
// 底部：固定 footer（取消 / 新建）。点击遮罩、关闭按钮或 Escape 关闭。
//
// 三个部分各自独立渲染 —— 壳只负责布局与三段分发，
// 公共字段与各类型子表单由 add_key_form/ 下的独立组件组成。

#[allow(clippy::too_many_arguments)]
fn redis_add_key_drawer(
    tab_id: TabId,
    type_select: &Entity<SelectState<SearchableVec<String>>>,
    name_input: &Entity<InputState>,
    ttl_input: &Entity<InputState>,
    active_form: Div,
    scroll: ScrollHandle,
    applying: bool,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 抽屉宽度：占窗口宽 85%、上限 560px，窄窗口时自适应不溢出。
    // 放宽上限便于 Hash/ZSet/Stream 及 JSON 多行排版，对齐 RedisInsight 的布局体量。
    let drawer_width = (f32::from(window.viewport_size().width) * 0.85).min(560.);
    let name_focus = name_input.read(cx).focus_handle(cx).clone();
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            hsla(220. / 360., 0.12, 0.08, 0.54)
        } else {
            hsla(210. / 360., 0.20, 0.20, 0.16)
        })
        .flex()
        .justify_end()
        // 点击遮罩空白区域关闭抽屉
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
            if this
                .redis_add_key_drawer
                .as_ref()
                .is_some_and(|drawer| drawer.tab_id == tab_id)
            {
                this.cancel_redis_add_key_drawer(cx);
            }
            cx.stop_propagation();
        }))
        .child(
            div()
                .h_full()
                .w(px(drawer_width))
                .flex_none()
                .overflow_hidden()
                .rounded_l(colors.radius_lg)
                .border_l_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(18.),
                    px(0.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .text_color(colors.text)
                .flex()
                .flex_col()
                .track_focus(&name_focus)
                .key_context("RedisAddKeyDrawer")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_redis_add_key_drawer(cx);
                    cx.stop_propagation();
                }))
                // 点击抽屉内部不冒泡关闭
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // 顶部标题区（三段式第一节）
                .child(
                    div()
                        .px_5()
                        .py_4()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("新增 Key"),
                        )
                        .child(redis_add_key_close_button(colors, cx)),
                )
                // 中部可滚动表单区（三段式第二节）
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .relative()
                        .child(
                            div()
                                .id("redis-add-key-scroll")
                                .h_full()
                                .track_scroll(&scroll)
                                .overflow_y_scrollbar()
                                .px_5()
                                .pb_4()
                                .child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .flex_col()
                                        .gap_4()
                                        // 公共字段组件 + 当前类型子表单组件
                                        .child(redis_add_key_common_fields(
                                            type_select,
                                            name_input,
                                            ttl_input,
                                            applying,
                                            colors,
                                        ))
                                        .child(active_form),
                                ),
                        )
                        .child(div().absolute().inset_0().child(
                            Scrollbar::vertical(&scroll),
                        )),
                )
                // 底部固定 footer（三段式第三节）
                .child(div().h(px(1.)).flex_none().bg(colors.border))
                .child(
                    div()
                        .h(px(58.))
                        .flex_none()
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            redis_detail_action_button("取消", false, !applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.cancel_redis_add_key_drawer(cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            redis_detail_action_button("新建", true, !applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    if !applying {
                                        this.apply_redis_add_key(tab_id, window, cx);
                                    }
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}

/// 顶部标题区的关闭按钮（AppIcon::Close），点击关闭建 Key 抽屉。
fn redis_add_key_close_button(colors: UiColors, cx: &mut Context<NavicatMain>) -> Stateful<Div> {
    div()
        .id("redis-add-key-close")
        .size(px(26.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .text_color(colors.muted)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.cancel_redis_add_key_drawer(cx);
                cx.stop_propagation();
            }),
        )
        .child(app_icon(AppIcon::Close, 14., colors.muted))
}
