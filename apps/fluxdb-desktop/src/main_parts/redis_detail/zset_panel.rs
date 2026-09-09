fn redis_key_detail_zset_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    applying: bool,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    this.sync_redis_zset_member_inputs(tab_id, detail, window, cx);
    let search_query = this
        .redis_zset_member_search_queries
        .get(&(tab_id, detail.key.clone()))
        .cloned()
        .unwrap_or_default();
    let page = this
        .redis_zset_member_search_pages
        .get(&(tab_id, detail.key.clone(), search_query.clone()));
    let rows = this.redis_zset_member_rows.clone();
    // score 行内编辑态（编辑行下标 / hover 行下标 / 编辑输入框），传给行渲染用；
    // 保持与 controller 读写同一份状态，避免渲染期再 read 视图。
    let score_editing = this.redis_zset_member_score_editing;
    let score_hovered = this.redis_zset_member_score_hover;
    let score_edit_input = this.redis_zset_member_score_edit_input.clone();
    let search_loading = this
        .redis_zset_member_search_loading
        .as_ref()
        .is_some_and(|loading| loading == &(tab_id, detail.key.clone(), search_query.clone()));
    let more_loading = this
        .redis_zset_member_search_more_loading
        .as_ref()
        .is_some_and(|loading| loading == &(tab_id, detail.key.clone(), search_query.clone()));
    let has_more = page.is_some_and(|page| page.next_cursor != "0");
    let total = page.map_or(rows.len(), |page| page.total);
    let panel_applying = applying
        || this
            ._data_load_tasks
            .contains_key(&redis_zset_member_mutation_task_id(tab_id));
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
                .child(redis_detail_panel_title("ZSet Data", colors))
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(redis_set_search_box(
                            this.redis_zset_member_search_input.clone(),
                            colors,
                        ))
                        .child(
                            redis_set_member_add_button(!panel_applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener({
                                    let key = detail.key.clone();
                                    move |this, _, window, cx| {
                                        if !panel_applying {
                                            this.open_redis_zset_member_add_drawer(
                                                tab_id,
                                                key.clone(),
                                                window,
                                                cx,
                                            );
                                        }
                                        cx.stop_propagation();
                                    }
                                }),
                            ),
                        ),
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
                        .child(div().w(px(140.)).flex_none().child("Score"))
                        .child(div().w(px(40.)).flex_none()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .w_full()
                        .relative()
                        .child(
                            div()
                                .id(("redis-zset-member-scroll", tab_id.0))
                                .size_full()
                                .flex()
                                .flex_col()
                                .track_scroll(&this.redis_zset_member_panel_scroll)
                                .overflow_y_scrollbar()
                                .child(if rows.is_empty() && search_loading {
                                    div()
                                        .h(px(120.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(12.))
                                        .text_color(colors.muted)
                                        .child(loading_spinner_with_color(22., colors.muted))
                                } else if rows.is_empty() {
                                    div()
                                        .h(px(120.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(12.))
                                        .text_color(colors.muted)
                                        .child("暂无成员")
                                } else {
                                    redis_zset_member_rows_panel(
                                        tab_id,
                                        &detail.key,
                                        &rows,
                                        panel_applying,
                                        score_editing,
                                        score_hovered,
                                        score_edit_input,
                                        this.pending_redis_zset_member_delete,
                                        colors,
                                        window,
                                        cx,
                                    )
                                }),
                        )
                        .child(
                            div().absolute().inset_0().child(
                                Scrollbar::vertical(&this.redis_zset_member_panel_scroll)
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
                        .child(format!("显示 {} / 共 {} 个成员", rows.len(), total))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(has_more, |this| {
                                    this.child(if more_loading {
                                        loading_spinner_with_color(13., colors.muted)
                                            .into_any_element()
                                    } else {
                                        redis_detail_toolbar_button(
                                            "加载更多",
                                            AppIcon::ChevronDown,
                                            true,
                                            colors,
                                        )
                                        .on_mouse_down(MouseButton::Left, cx.listener({
                                            let key = detail.key.clone();
                                            let search_query = search_query.clone();
                                            move |this: &mut NavicatMain, _, _, cx| {
                                                let next_cursor = this
                                                    .redis_zset_member_search_pages
                                                    .get(&(
                                                        tab_id,
                                                        key.clone(),
                                                        search_query.clone(),
                                                    ))
                                                    .map(|page| page.next_cursor.clone())
                                                    .unwrap_or_default();
                                                if !next_cursor.is_empty() {
                                                    this.request_redis_zset_member_search(
                                                        tab_id,
                                                        key.clone(),
                                                        search_query.clone(),
                                                        next_cursor,
                                                        cx,
                                                    );
                                                }
                                            }
                                        }))
                                        .into_any_element()
                                    })
                                })
                        ),
                ),
        )
        .when(
            this.pending_redis_zset_member_drawer
                .as_ref()
                .is_some_and(|pending| pending.tab_id == tab_id && pending.key == detail.key),
            |panel| {
                panel.child(redis_zset_member_add_drawer(
                    tab_id,
                    detail.key.clone(),
                    &this.redis_zset_member_drawer_rows,
                    &this.redis_zset_member_drawer_scroll,
                    panel_applying,
                    colors,
                    window,
                    cx,
                ))
            },
        )
}

fn redis_zset_member_rows_panel(
    tab_id: TabId,
    key: &str,
    rows: &[RedisZSetMemberInputs],
    applying: bool,
    score_editing: Option<usize>,
    score_hovered: Option<usize>,
    score_edit_input: Option<Entity<InputState>>,
    pending_delete: Option<usize>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let view = cx.entity().downgrade();
    // 数据行之间不加 gap：行本身固定高度 40px 且带分隔线，若加 gap_2() 会从第二行起
    // 在行上方多出 8px 间隙，导致视觉上"行高变高"，与列头错位。改为紧致等间距排列。
    let mut body = div().w_full().flex_none().flex().flex_col();
    for (row_index, row) in rows.iter().enumerate() {
        body = body.child(redis_zset_member_row(
            tab_id,
            key,
            row_index,
            rows.len(),
            row,
            applying,
            score_editing,
            score_hovered,
            score_edit_input.clone(),
            pending_delete,
            view.clone(),
            colors,
            window,
            cx,
        ));
    }
    body
}

/// ZSet 详情表的成员行：member 为只读纯文本；score 默认只读，hover 显示编辑图标，
/// 点击后进入行内编辑（输入框 + x 取消 / 勾确认）。
fn redis_zset_member_row(
    tab_id: TabId,
    key: &str,
    row_index: usize,
    rows_len: usize,
    row: &RedisZSetMemberInputs,
    applying: bool,
    score_editing: Option<usize>,
    score_hovered: Option<usize>,
    score_edit_input: Option<Entity<InputState>>,
    pending_delete: Option<usize>,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let member = row.member_input.read(cx).value().to_string();
    let score = row.score_input.read(cx).value().to_string();
    let can_delete = rows_len > 0;
    // 当前行是否处于 score 行内编辑态。
    let editing = score_editing == Some(row_index);
    let hovered = score_hovered == Some(row_index);
    // 当前行是否为待确认删除的行（点击垃圾桶后弹出二次确认浮层）。
    let confirm_pending = pending_delete == Some(row_index);
    let key = key.to_string();
    div()
        .id(("redis-zset-member-score-row", row_index))
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
        .on_hover(cx.listener(move |this, is_hovered: &bool, _, cx| {
            if *is_hovered {
                this.redis_zset_member_score_hover = Some(row_index);
            } else if this.redis_zset_member_score_hover == Some(row_index) {
                this.redis_zset_member_score_hover = None;
            }
            cx.notify();
        }))
        // 点击「其它」行（非当前编辑行）时取消编辑：对齐 RedisInsight 的点击外部取消。
        // 子元素（如删除按钮 / 编辑图标）已 stop_propagation，不受影响。
        .when(editing == false && score_editing.is_some(), |this| {
            this.on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                this.cancel_redis_zset_member_score_edit(cx);
                cx.stop_propagation();
            }))
        })
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
        // member 列：只读纯文本（member 不可修改）。
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .flex()
                .items_center()
                .px_2()
                .font_family("Menlo")
                .text_size(px(13.))
                .text_color(colors.text)
                .child(member.clone()),
        )
        // score 列：默认只读；hover 显示编辑图标，点击进入行内编辑。
        .child(if editing {
            redis_zset_member_score_editor(
                tab_id,
                key.clone(),
                row_index,
                score_edit_input,
                colors,
                window,
                cx,
            )
        } else {
            redis_zset_member_score_display(
                row_index,
                score,
                hovered,
                applying,
                colors,
                window,
                cx,
            )
        })
        .child(
            div()
                .size(px(26.))
                .rounded(colors.radius * 0.5)
                .flex()
                .items_center()
                .justify_center()
                .text_color(if can_delete { rgb(0xe5484d) } else { colors.border })
                .when(can_delete && !applying, |this| {
                    this.cursor_pointer().hover(move |style| style.bg(colors.hover))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if !applying && can_delete {
                            // 点击垃圾桶先弹二次确认浮层（对齐 RedisInsight PopoverDelete），
                            // 确认后再真正执行 ZREM 删除。
                            this.pending_redis_zset_member_delete = Some(row_index);
                            // 删除浮层悬挂期间不支持再挂场景无关的编辑取消。
                            this.redis_zset_member_score_hover = None;
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(AppIcon::Trash, 14., if can_delete { rgb(0xe5484d) } else { colors.border }))
                // 二次确认浮层：锚定在行内删除图标旁，确认后执行对应删除。
                .when(confirm_pending, |this| {
                    this.child(redis_zset_member_delete_confirm_popover(
                        tab_id,
                        key.clone(),
                        row_index,
                        member.clone(),
                        view,
                        colors,
                        cx,
                    ))
                }),
        )
}

/// ZSet 成员删除二次确认浮层（对齐 RedisInsight PopoverDelete / 项目内 Set 删除确认）：
/// 锚定在行内删除图标旁，标题为成员名，文案提示不可撤销，确认后才执行 ZREM。
fn redis_zset_member_delete_confirm_popover(
    tab_id: TabId,
    key: String,
    row_index: usize,
    member: String,
    view: WeakEntity<NavicatMain>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 面板行删除会随「保存」落库（ZREM），删除不可撤销，展示成员名以明确目标。
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
                            // 点击浮层外部（其它任意位置）关闭确认，不执行删除。
                            .on_mouse_down_out(move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.pending_redis_zset_member_delete = None;
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
                                            this.pending_redis_zset_member_delete = None;
                                            cx.notify();
                                            cx.stop_propagation();
                                        }),
                                    ))
                                    .child(redis_key_delete_confirm_button("确认删除", true, colors).on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            // 确认：真正执行 ZREM 删除；从面板行集合取最新 member。
                                            let member = this
                                                .redis_zset_member_rows
                                                .get(row_index)
                                                .map(|row| row.member_input.read(cx).value().to_string())
                                                .unwrap_or_default();
                                            if !member.is_empty() {
                                                this.request_redis_zset_member_delete(
                                                    tab_id,
                                                    key.clone(),
                                                    member,
                                                    cx,
                                                );
                                            }
                                            this.pending_redis_zset_member_delete = None;
                                            cx.stop_propagation();
                                        }),
                                    )),
                            ),
                    ),
            )
            .with_priority(1),
        )
}

/// score 只读展示：hover 时显示编辑图标，点击图标进入行内编辑。
fn redis_zset_member_score_display(
    row_index: usize,
    score: String,
    hovered: bool,
    applying: bool,
    colors: UiColors,
    _window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .relative()
        .w(px(140.))
        .flex_none()
        .h_full()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .cursor_text()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .font_family("Menlo")
                .text_size(px(13.))
                .text_color(colors.text)
                .child(score),
        )
        .when(hovered && !applying, |this| {
            // hover 显示编辑图标，点击进入行内编辑。
            this.child(
                div()
                    .id(("redis-zset-score-edit-icon", row_index))
                    .size(px(20.))
                    .rounded(colors.radius * 0.5)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .tooltip(move |window, cx| Tooltip::new("编辑").build(window, cx))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.begin_redis_zset_member_score_edit(row_index, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(app_icon(AppIcon::Edit, 12., colors.muted)),
            )
        })
}

/// score 行内编辑态：输入框 + 右侧 x（取消）/ 勾（确认）按钮。
/// 勾后会立即提交该 member 的 score 修改（ZADD），无需再走底部「保存」。
fn redis_zset_member_score_editor(
    tab_id: TabId,
    key: String,
    row_index: usize,
    edit_input: Option<Entity<InputState>>,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = edit_input
        .as_ref()
        .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
    let focus_border = cx.theme().primary;
    let Some(edit_input) = edit_input else {
        // 编辑态与输入框应成对出现；异常缺失时回退为只读展示，避免 unwrap 崩溃。
        return redis_zset_member_score_display(
            row_index,
            String::new(),
            false,
            false,
            colors,
            window,
            cx,
        );
    };
    div()
        .relative()
        .w(px(140.))
        .flex_none()
        .h_full()
        .flex()
        .items_center()
        .gap_1()
        .cursor_text()
        .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
            this.cancel_redis_zset_member_score_edit(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(30.))
                .rounded(colors.radius)
                .border_1()
                .border_color(if focused { focus_border } else { colors.border.into() })
                .bg(colors.input_bg)
                .child(
                    Input::new(&edit_input)
                        .appearance(false)
                        .focus_bordered(false)
                        .w_full()
                        .h_full()
                        .px_2()
                        .font_family("Menlo")
                        .text_size(px(13.)),
                ),
        )
        .child(
            redis_zset_score_edit_icon_button(AppIcon::Close, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.cancel_redis_zset_member_score_edit(cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            redis_zset_score_edit_icon_button(AppIcon::Check, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.apply_redis_zset_member_score_edit(
                        tab_id,
                        key.clone(),
                        row_index,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            ),
        )
}

/// score 行内编辑的图标按钮（x / 勾）。
fn redis_zset_score_edit_icon_button(icon: AppIcon, colors: UiColors) -> Div {
    div()
        .size(px(24.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover).border_color(colors.border))
        .child(app_icon(icon, 12., colors.text))
}

fn redis_zset_member_add_drawer(
    tab_id: TabId,
    key: String,
    rows: &[RedisZSetMemberInputs],
    scroll: &ScrollHandle,
    applying: bool,
    colors: UiColors,
    _window: &Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let first_focus = rows.first().map(|row| row.member_input.read(cx).focus_handle(cx).clone());
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
            this.cancel_redis_zset_member_drawer(cx);
            cx.stop_propagation();
        }))
        .child(
            div()
                .w_full()
                .max_h(px(480.))
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
                .key_context("RedisZSetMemberAddDrawer")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_redis_zset_member_drawer(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_size(px(17.)).font_weight(gpui::FontWeight::SEMIBOLD).child("新增成员")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .relative()
                        .child(
                            div()
                                .id("redis-zset-member-drawer-scroll")
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
                                        .child(redis_zset_member_drawer_rows_panel(
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
                                                                this.add_redis_zset_member_drawer_row(
                                                                    window, cx,
                                                                );
                                                            }
                                                            cx.stop_propagation();
                                                        }),
                                                    ),
                                                ),
                                        ),
                                ),
                        )
                        .child(div().absolute().inset_0().child(
                            Scrollbar::vertical(scroll),
                        )),
                )
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
                                    this.cancel_redis_zset_member_drawer(cx);
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
                                        if !applying {
                                            this.confirm_redis_zset_member_drawer(
                                                tab_id,
                                                key.clone(),
                                                window,
                                                cx,
                                            );
                                        }
                                        cx.stop_propagation();
                                    }
                                }),
                            ),
                        ),
                ),
        )
}

fn redis_zset_member_drawer_rows_panel(
    rows: &[RedisZSetMemberInputs],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut body = div().w_full().flex().flex_col().gap_2();
    for (row_index, row) in rows.iter().enumerate() {
        body = body.child(redis_zset_member_drawer_row(
            row_index,
            rows.len(),
            row,
            applying,
            colors,
            cx,
        ));
    }
    body
}

fn redis_zset_member_drawer_row(
    row_index: usize,
    rows_len: usize,
    row: &RedisZSetMemberInputs,
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let can_delete = rows_len > 1;
    div()
        .h(px(34.))
        .flex_none()
        .w_full()
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .child(
            redis_stream_add_input_box(row.member_input.clone(), colors)
                .w(px(220.))
                .flex_none(),
        )
        .child(
            redis_stream_add_input_box(row.score_input.clone(), colors)
                .flex_1()
                .min_w(px(0.)),
        )
        .child(
            div()
                .size(px(28.))
                .rounded(colors.radius * 0.5)
                .flex()
                .items_center()
                .justify_center()
                .text_color(if can_delete { rgb(0xe5484d) } else { colors.border })
                .when(can_delete && !applying, |this| {
                    this.cursor_pointer().hover(move |style| style.bg(colors.hover))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if !applying && can_delete {
                            this.remove_redis_zset_member_drawer_row(row_index, window, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(AppIcon::Trash, 14., if can_delete { rgb(0xe5484d) } else { colors.border })),
        )
}

/// Redis List 明细数据表的 delegate，把当前页的 List 元素（下标 + 值）只读渲染成
/// gpui-component 的 `Table`。两列：序号 / Value（值以纯文本展示，不做行内编辑）。
// —— Redis List 明细表（gpui-component Table）——
// 只读两列（序号 # / Value），Value 单元格支持行内编辑（LSET），交互与浮层方案对齐 Hash 值编辑：
// 编辑行保持普通行高，编辑框以绝对定位浮层锚定在编辑行下方、向下覆盖 2 行，其余行不受影响。
// 行高与 Size::Large（40px）保持一致。

fn redis_zset_member_search_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 56)
}

fn redis_zset_member_mutation_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 55)
}
