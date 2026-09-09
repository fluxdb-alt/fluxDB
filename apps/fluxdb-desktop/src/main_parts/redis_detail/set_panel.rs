fn redis_key_detail_set_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    applying: bool,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 服务端分页搜索：以已提交查询为键取页（与 store 路径一致，不 trim）。
    // 有页时成员/总数/游标都以服务端 SSCAN 结果为准；无页（首开搜索在飞）走 preview 兜底。
    let search_query = this
        .redis_set_member_search_queries
        .get(&(tab_id, detail.key.clone()))
        .cloned()
        .unwrap_or_default();
    let page = this
        .redis_set_member_search_pages
        .get(&(tab_id, detail.key.clone(), search_query.clone()));
    let rows = this.redis_set_member_rows.clone();
    // 滚动句柄提前克隆：后续闭包会可变借用 this，句柄本身是 Rc 共享偏移
    let member_scroll = this.redis_set_member_panel_scroll.clone();
    let detail_key = detail.key.clone();
    let visible_count = page.map_or(rows.len(), |page| page.members.len());
    let search_loading = this
        .redis_set_member_search_loading
        .as_ref()
        .is_some_and(|loading| {
            loading == &(tab_id, detail.key.clone(), search_query.clone())
        });
    let more_loading = this
        .redis_set_member_search_more_loading
        .as_ref()
        .is_some_and(|loading| {
            loading == &(tab_id, detail.key.clone(), search_query.clone())
        });
    // 分页未遍历完显示「加载更多」，取服务端游标续页
    let has_more = page.is_some_and(|page| page.next_cursor != "0");
    let total = page.map_or(rows.len(), |page| page.total);
    // 有搜索词时 footer 显示命中数，否则显示分页进度
    let footer_left = if search_query.is_empty() {
        format!("显示 {visible_count} / 共 {total} 个成员")
    } else {
        format!("找到 {visible_count} 个")
    };
    redis_detail_panel(colors)
        .relative()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .child(redis_detail_panel_title("Set Data", colors))
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(redis_set_search_box(
                            this.redis_set_member_search_input.clone(),
                            colors,
                        ))
                        .child(redis_set_member_add_button(!applying, colors).on_mouse_down(
                            MouseButton::Left,
                            cx.listener({
                                let key = detail.key.clone();
                                move |this, _, window, cx| {
                                    if !applying {
                                        this.open_redis_set_member_add_drawer(
                                            tab_id,
                                            key.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                    cx.stop_propagation();
                                }
                            }),
                        )),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .flex()
                .flex_col()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border_soft)
                .bg(colors.input_bg)
                .child(
                    div()
                        .h(px(34.))
                        .flex_none()
                        .px_3()
                        .border_b_1()
                        .border_color(colors.border_soft)
                        .bg(colors.panel_alt)
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(div().w(px(42.)).flex_none().child("#"))
                        .child(div().flex_1().min_w(px(0.)).child("Member"))
                        .child(div().w(px(40.)).flex_none()),
                )
                .child(
                    // 成员列表滚动区：外层只占剩余高度（不能用 h_full，否则视口高度等于整块面板高度，
                    // 内容永远撑不出滚动条且底部行会被父级 overflow_hidden 裁掉）
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .w_full()
                        .relative()
                        .child(
                            div()
                                .id(("redis-set-member-scroll", tab_id.0))
                                .size_full()
                                .flex()
                                .flex_col()
                                .track_scroll(&member_scroll)
                                .overflow_y_scrollbar()
                                .child(if rows.is_empty() && search_loading {
                                    div()
                                        .h(px(120.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .flex_col()
                                        .gap_2()
                                        .text_size(px(12.))
                                        .text_color(colors.muted)
                                        .child(loading_spinner_with_color(22., colors.muted))
                                } else if rows.is_empty() {
                                    div()
                                        .h(px(120.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .flex_col()
                                        .gap_2()
                                        .text_size(px(12.))
                                        .text_color(colors.muted)
                                        .child(app_icon(
                                            if search_query.is_empty() {
                                                AppIcon::List
                                            } else {
                                                AppIcon::Search
                                            },
                                            22.,
                                            colors.muted,
                                        ))
                                        .child(if search_query.is_empty() {
                                            "暂无成员"
                                        } else {
                                            "没有匹配成员"
                                        })
                                } else {
                                    redis_set_member_rows_panel(
                                        tab_id,
                                        &detail_key,
                                        &rows,
                                        applying,
                                        this.pending_redis_set_member_delete,
                                        colors,
                                        cx,
                                    )
                                }),
                        )
                        // 常显纵向滚动条：内容未超出容器时组件自身不绘制
                        .child(
                            div().absolute().inset_0().child(
                                Scrollbar::vertical(&member_scroll)
                                    ,
                            ),
                        ),
                )
                .child(
                    div()
                        .h(px(32.))
                        .flex_none()
                        .px_3()
                        .border_t_1()
                        .border_color(colors.border_soft)
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .child(footer_left)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(!search_query.is_empty(), |this| {
                                    this.child(format!(
                                        "筛选: {}",
                                        redis_ellipsis_text(&search_query, 24)
                                    ))
                                })
                                .when(has_more, |this| {
                                    this.child(
                                        if more_loading {
                                            loading_spinner_with_color(13., colors.muted)
                                                .into_any_element()
                                        } else {
                                            redis_detail_toolbar_button(
                                                "加载更多",
                                                AppIcon::ChevronDown,
                                                true,
                                                colors,
                                            )
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener({
                                                    let key = detail.key.clone();
                                                    let search_query = search_query.clone();
                                                    move |this: &mut NavicatMain, _, window, cx| {
                                                        let next_cursor = this
                                                            .redis_set_member_search_pages
                                                            .get(&(
                                                                tab_id,
                                                                key.clone(),
                                                                search_query.clone(),
                                                            ))
                                                            .map(|page| page.next_cursor.clone())
                                                            .unwrap_or_default();
                                                        if !next_cursor.is_empty() {
                                                            this.request_redis_set_member_search(
                                                                tab_id,
                                                                key.clone(),
                                                                search_query.clone(),
                                                                next_cursor,
                                                                cx,
                                                            );
                                                        }
                                                        cx.stop_propagation();
                                                        let _ = window;
                                                    }
                                                }),
                                            )
                                            .into_any_element()
                                        },
                                    )
                                }),
                        ),
                ),
        )
        .when(
            this.pending_redis_set_member_drawer
                .as_ref()
                .is_some_and(|pending| pending.tab_id == tab_id && pending.key == detail.key),
            |panel| {
                panel.child(redis_set_member_add_drawer(
                    tab_id,
                    detail_key,
                    &this.redis_set_member_drawer_rows,
                    &this.redis_set_member_drawer_scroll,
                    applying,
                    colors,
                    window,
                    cx,
                ))
            },
        )
}

// —— Redis Hash 明细表（gpui-component Table）——
// 列宽为权重分配：序列 10 / Field 30 / Value 30 / TTL 20 / 删除 10（和 100，归一化到可用宽度），
// 具体像素宽度由覆盖层 canvas 测量容器宽度后计算得出。
// 表格基于 uniform_list（均匀高度列表），所有行必须是同一高度，无法单独撑高某一行。
// 因此值编辑采用「浮层」方案：编辑行保持普通行高，编辑框以绝对定位锚定在编辑行下方、
// 向上/向下覆盖若干行；其余行不受任何影响。相关常量与表格行高（Size::Large = 40px）保持一致。

fn redis_set_member_rows_panel(
    tab_id: TabId,
    key: &str,
    rows: &[Entity<InputState>],
    applying: bool,
    pending_delete: Option<RedisSetMemberDeleteTarget>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 服务端 SSCAN 已按 MATCH 过滤，这里不再做前端 contains 过滤。
    // flex_none：作为滚动区直接子节点时不允许被压缩，否则内容会被裁到视口高度而无法滚动
    let mut body = div()
        .w_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap_2();
    let view = cx.entity().downgrade();
    for (row_index, input) in rows.iter().cloned().enumerate() {
        body = body.child(redis_set_member_row(
            tab_id,
            key,
            row_index,
            rows.len(),
            input,
            applying,
            pending_delete,
            view.clone(),
            colors,
            cx,
        ));
    }
    body
}

fn redis_set_member_row(
    tab_id: TabId,
    key: &str,
    row_index: usize,
    rows_len: usize,
    input: Entity<InputState>,
    applying: bool,
    pending_delete: Option<RedisSetMemberDeleteTarget>,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_delete = rows_len > 1;
    let member = input.read(cx).value().to_string();
    let confirm_pending = matches!(
        pending_delete,
        Some(RedisSetMemberDeleteTarget::PanelRow(i)) if i == row_index
    );
    div()
        .h(px(40.))
        .flex_none()
        .w_full()
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .w(px(42.))
                .flex_none()
                .flex()
                .items_center()
                .font_family("Menlo")
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(format!("{:02}", row_index + 1)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .line_height(px(18.))
                .font_family("Menlo")
                .text_color(colors.text)
                .child(member.clone()),
        )
        .child(
            div()
                .size(px(26.))
                .flex_none()
                .rounded(colors.radius * 0.5)
                .flex()
                .items_center()
                .justify_center()
                .opacity(if can_delete { 0.72 } else { 0.35 })
                .when(can_delete && !applying, |this| {
                    this.cursor_pointer().hover(move |style| style.bg(colors.hover))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if !applying && can_delete {
                            // 先弹二次确认浮层，确认后再真正删除
                            this.pending_redis_set_member_delete =
                                Some(RedisSetMemberDeleteTarget::PanelRow(row_index));
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(
                    AppIcon::Trash,
                    14.,
                    if can_delete { rgb(0xe5484d) } else { colors.border },
                ))
                .when(confirm_pending, |this| {
                    this.child(redis_set_member_delete_confirm_popover(
                        tab_id,
                        key.to_owned(),
                        RedisSetMemberDeleteTarget::PanelRow(row_index),
                        member.clone(),
                        view,
                        colors,
                        cx,
                    ))
                }),
        )
}

// Set 成员删除二次确认浮层：锚定在行内删除图标旁，确认后执行对应删除

fn redis_set_member_delete_confirm_popover(
    tab_id: TabId,
    key: String,
    target: RedisSetMemberDeleteTarget,
    member: String,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 面板行删除会随保存落库，删除不可撤销，提示成员内容
    let title = member;
    let message = "将被删除，此操作不可撤销。";
    div()
        .absolute()
        .right(px(34.))
        .top(px(-74.))
        .size(px(1.))
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::TopRight)
                    .child(
                        div()
                            .occlude()
                            .w(px(238.))
                            .rounded(colors.radius)
                            .border_1()
                            .border_color(colors.border)
                            .shadow_lg()
                            .bg(colors.panel_bg)
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .on_mouse_down_out(move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.pending_redis_set_member_delete = None;
                                    cx.notify();
                                });
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_family("Menlo")
                                            .text_size(px(14.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(colors.text)
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(colors.muted)
                                            .child(message),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(redis_key_delete_confirm_button("取消", false, colors).on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.pending_redis_set_member_delete = None;
                                            cx.notify();
                                            cx.stop_propagation();
                                        }),
                                    ))
                                    .child(redis_key_delete_confirm_button("确认删除", true, colors).on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            let RedisSetMemberDeleteTarget::PanelRow(i) = target;
                                            let member = this
                                                .redis_set_member_rows
                                                .get(i)
                                                .map(|input| input.read(cx).value().to_string())
                                                .unwrap_or_default();
                                            // per-member 删除（SREM）：只删当前行，避免分页下整集合重写误删未加载成员
                                            this.request_redis_set_member_delete(
                                                tab_id,
                                                key.clone(),
                                                member,
                                                cx,
                                            );
                                            this.pending_redis_set_member_delete = None;
                                            cx.stop_propagation();
                                        }),
                                    )),
                            ),
                    ),
            )
            .with_priority(1),
        )
}

fn redis_set_preview_members(value: &str) -> (String, Vec<String>) {
    let mut lines = value.lines();
    let summary = lines.next().unwrap_or_default().to_string();
    (summary, lines.map(str::to_string).collect())
}

fn redis_set_search_box(input: Entity<InputState>, colors: UiColors) -> Div {
    div()
        .w(px(240.))
        .h(px(30.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .px_2()
        .gap_2()
        .child(app_icon(AppIcon::Search, 14., colors.muted))
        .child(
            Input::new(&input)
                .appearance(false)
                .focus_bordered(false)
                .w_full()
                .h_full()
                .text_size(px(12.)),
        )
}

// Set 类型面板头部「新增成员」按钮：仿 stream 详情中的 + 按钮

fn redis_set_member_add_button(enabled: bool, colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .px_3()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_1()
        .justify_center()
        .border_1()
        .border_color(if enabled { rgb(0x1687ff) } else { colors.border_soft })
        .bg(if enabled {
            if colors.is_dark {
                rgb(0x12334f)
            } else {
                rgb(0xeaf4ff)
            }
        } else {
            colors.panel_alt
        })
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if enabled { rgb(0x1687ff) } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .child(app_icon(
            AppIcon::Plus,
            13.,
            if enabled { rgb(0x1687ff) } else { colors.border },
        ))
        .child("新增")
}

fn redis_set_member_add_drawer(
    tab_id: TabId,
    key: String,
    rows: &[Entity<InputState>],
    scroll: &ScrollHandle,
    applying: bool,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 抽屉最大高度：视口高度减去底部留白，超出部分交给内容区滚动
    let max_height = (f32::from(window.viewport_size().height) - 120.).min(480.);
    // 首个输入框的焦点句柄，用于 Esc 关闭抽屉时定位焦点作用域
    let first_focus = rows.first().map(|input| input.read(cx).focus_handle(cx).clone());
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
        .items_end()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.cancel_redis_set_member_drawer(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w_full()
                .max_h(px(max_height))
                .overflow_hidden()
                .rounded_t(colors.radius_lg)
                .border_t_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(-18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., if colors.is_dark { 0.42 } else { 0.18 }),
                )])
                .text_color(colors.text)
                .flex()
                .flex_col()
                .when_some(first_focus, |this, handle| this.track_focus(&handle))
                .key_context("RedisSetMemberAddDrawer")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    // 删除确认浮层开着时，Esc 只关浮层，不关抽屉
                    if this.pending_redis_set_member_delete.take().is_some() {
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    this.cancel_redis_set_member_drawer(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    // 抽屉标题栏
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(17.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("新增 Member"),
                        ),
                )
                .child(
                    // 成员输入区：达到最大高度后滚动，新增行时自动滚动到底部
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .relative()
                        .child(
                            div()
                                .id("redis-set-member-drawer-scroll")
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
                                        .gap_2()
                                        .child(redis_set_member_drawer_rows_panel(
                                            rows,
                                            applying,
                                            colors,
                                            cx,
                                        ))
                                        .child(
                                            div()
                                                .flex()
                                                .justify_end()
                                                .pt_1()
                                                .child(
                                                    redis_set_member_drawer_add_button(
                                                        !applying,
                                                        colors,
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |this, _, window, cx| {
                                                            if !applying {
                                                                this.add_redis_set_member_drawer_row(
                                                                    window,
                                                                    cx,
                                                                );
                                                            }
                                                            cx.stop_propagation();
                                                        }),
                                                    ),
                                                ),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .child(
                                    Scrollbar::new(scroll)
                                        ,
                                ),
                        ),
                )
                .child(
                    // 分隔线
                    div()
                        .h(px(1.))
                        .flex_none()
                        .bg(colors.border),
                )
                .child(
                    // 底部按钮区：flex_none 保持固定，不随内容滚动
                    div()
                        .h(px(58.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .px_5()
                        .child(
                            redis_detail_action_button("取消", false, !applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.cancel_redis_set_member_drawer(cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            redis_detail_action_button("保存", true, !applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener({
                                    let key = key.clone();
                                    move |this, _, window, cx| {
                                        // 抽屉只做 per-member 新增（SADD），确认后由搜索重跑刷新面板；
                                        // 不复用整集合重写（分页下会误删未加载成员）
                                        this.confirm_redis_set_member_drawer(
                                            tab_id,
                                            key.clone(),
                                            window,
                                            cx,
                                        );
                                        cx.stop_propagation();
                                    }
                                }),
                            ),
                        ),
                )
                )
}

// 抽屉内成员输入行列表

fn redis_set_member_drawer_rows_panel(
    rows: &[Entity<InputState>],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut body = div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2();
    for (row_index, input) in rows.iter().cloned().enumerate() {
        body = body.child(redis_set_member_drawer_row(
            row_index,
            rows.len(),
            input,
            applying,
            colors,
            cx,
        ));
    }
    body
}

// 抽屉内单行：输入框 + 删除图标（至少保留一行）

fn redis_set_member_drawer_row(
    row_index: usize,
    rows_len: usize,
    input: Entity<InputState>,
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 多行时删除图标移除整行；单行时仅当前行有内容才可点击清除内容
    let can_delete = rows_len > 1;
    let has_value = !input.read(cx).value().is_empty();
    let button_enabled = can_delete || has_value;
    div()
        .h(px(34.))
        .flex_none()
        .w_full()
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .rounded(colors.radius)
                .border_1()
                .border_color(colors.border)
                .bg(colors.input_bg)
                .child(
                    Input::new(&input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .px_2()
                        .text_size(px(13.)),
                ),
        )
        .child(
            div()
                .size(px(28.))
                .rounded(colors.radius * 0.5)
                .flex()
                .items_center()
                .justify_center()
                .text_color(if button_enabled {
                    rgb(0xe5484d)
                } else {
                    colors.border
                })
                .when(button_enabled && !applying, |this| {
                    this.cursor_pointer().hover(move |style| style.bg(colors.hover))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        // 多行删除整行；单行仅清空当前输入内容
                        if !applying && button_enabled {
                            if can_delete {
                                this.remove_redis_set_member_drawer_row(row_index, window, cx);
                            } else {
                                input.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                });
                            }
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(
                    AppIcon::Trash,
                    14.,
                    if button_enabled { rgb(0xe5484d) } else { colors.border },
                ))
        )
}

// 抽屉内「新增一行」文字按钮：加号 + 中文文字，靠右显示（父级容器 justify_end）

fn redis_set_member_drawer_add_button(enabled: bool, colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .px_3()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(if enabled { rgb(0x1677ff) } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .child(app_icon(
            AppIcon::Plus,
            13.,
            if enabled { rgb(0x1677ff) } else { colors.muted },
        ))
        .child("新增一行")
}

fn redis_set_member_search_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 60)
}

// Set 成员 per-member 增删（SADD/SREM）的任务 id：与搜索任务区分，互不阻塞

fn redis_set_member_mutation_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 59)
}
