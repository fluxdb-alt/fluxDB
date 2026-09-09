fn redis_key_detail_list_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    applying: bool,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    this.sync_redis_list_item_inputs(tab_id, detail, window, cx);
    let page = this.active_redis_list_page(tab_id, &detail.key);
    // 当前页实际条数（用于底部"显示 x / 共 y"统计），由分页数据直接派生，不再维护独立行列表。
    let shown = page.as_ref().map(|p| p.items.len()).unwrap_or(0);
    let loading = this
        .redis_list_item_search_loading
        .as_ref()
        .is_some_and(|loading| loading.0 == tab_id && loading.1 == detail.key);
    let more_loading = this
        .redis_list_item_search_more_loading
        .as_ref()
        .is_some_and(|loading| loading.0 == tab_id && loading.1 == detail.key);
    let has_more = page.as_ref().is_some_and(|page| page.next_cursor != "0");
    let total = page.as_ref().map_or(shown, |page| page.total);
    let panel_applying = applying
        || this
            ._data_load_tasks
            .contains_key(&redis_list_item_mutation_task_id(tab_id));
    // 把渲染期派生出的数据（当前页的 List 元素：下标 + 值）同步进表格 delegate；
    // defer 延迟到本帧渲染结束后执行，避免在 NavicatMain 渲染过程中直接 update 表格实体造成 double-lease。
    let table_state = this.redis_list_table_state.clone();
    let sync_items = page
        .as_ref()
        .map(|page| page.items.clone())
        .unwrap_or_default();
    let sync_key = detail.key.clone();
    // 编辑状态（编辑目标行、hover 行、编辑输入框）随当前渲染快照同步进 delegate。
    let sync_editing = this.redis_list_item_editing.clone();
    let sync_hovered = this.redis_list_item_hovered.clone();
    let sync_edit_input = this.redis_list_value_edit_input.clone();
    let sync_panel_applying = panel_applying;
    cx.defer_in(window, move |this, _, cx| {
        this.redis_list_table_state.update(cx, |table, cx| {
            if table.delegate_mut().set_data(
                tab_id,
                sync_key.clone(),
                sync_items.clone(),
                sync_editing.clone(),
                sync_hovered.clone(),
                sync_edit_input.clone(),
                sync_panel_applying,
                loading,
                colors,
            ) {
                table.refresh(cx);
            }
        });
    });
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
                .child(redis_detail_panel_title("List Data", colors))
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        // 搜索框放在「新增」按钮前面，布局对齐 Hash 面板：按索引跳转（LINDEX）
                        .child(redis_set_search_box(
                            this.redis_list_item_search_input.clone(),
                            colors,
                        ))
                        .child(
                            redis_set_member_add_button(!panel_applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener({
                                    let key = detail.key.clone();
                                    move |this, _, window, cx| {
                                        if !panel_applying {
                                            this.open_redis_list_item_add_drawer(
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
                        )
                        .child(
                            redis_list_remove_button(!panel_applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener({
                                    let key = detail.key.clone();
                                    move |this, _, window, cx| {
                                        if !panel_applying {
                                            this.open_redis_list_item_remove_drawer(
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
                .child(redis_list_item_table(&table_state))
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
                        .child(format!("显示 {} / 共 {} 个条目", shown, total))
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
                                            move |this: &mut NavicatMain, _, _, cx| {
                                                let next_cursor = this
                                                    .active_redis_list_page(tab_id, &key)
                                                    .map(|page| page.next_cursor)
                                                    .unwrap_or_default();
                                                let query = this
                                                    .redis_list_item_search_queries
                                                    .get(&(tab_id, key.clone()))
                                                    .cloned()
                                                    .unwrap_or_default();
                                                // "0" 表示已到最后一个元素，不再续页。
                                                if !next_cursor.is_empty() && next_cursor != "0" {
                                                    this.request_redis_list_item_search(
                                                        tab_id,
                                                        key.clone(),
                                                        query,
                                                        next_cursor,
                                                        cx,
                                                    );
                                                }
                                            }
                                        }))
                                        .into_any_element()
                                    })
                                }),
                        ),
                ),
        )
        .when(
            this.pending_redis_list_item_drawer
                .as_ref()
                .is_some_and(|pending| pending.tab_id == tab_id && pending.key == detail.key),
            |panel| {
                panel.child(redis_list_item_add_drawer(
                    tab_id,
                    detail.key.clone(),
                    &this.redis_list_item_drawer_rows,
                    &this.redis_list_item_drawer_scroll,
                    panel_applying,
                    colors,
                    window,
                    cx,
                ))
            },
        )
        .when(
            this.redis_list_item_remove_drawer
                .as_ref()
                .is_some_and(|remove| remove.tab_id == tab_id && remove.key == detail.key),
            |panel| {
                // 二次确认目标仅在同属本抽屉时透传，由确认浮层渲染。
                let confirm = this
                    .redis_list_item_remove_confirm
                    .as_ref()
                    .filter(|t| t.tab_id == tab_id && t.key == detail.key)
                    .cloned();
                panel.child(redis_list_item_remove_drawer(
                    tab_id,
                    detail.key.clone(),
                    &this.redis_list_item_remove_select,
                    this.redis_list_item_remove_count_input.clone(),
                    panel_applying,
                    total,
                    confirm,
                    colors,
                    window,
                    cx,
                ))
            },
        )
}

/// List 明细数据表：gpui-component 的 `Table`，序号/Value 两列（只读展示），比例宽度由 canvas 测量后换算。
fn redis_list_item_table(
    table_state: &Entity<TableState<RedisListTableDelegate>>,
) -> Div {
    let measured_table = table_state.clone();
    div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .overflow_hidden()
        .child(
            DataTable::new(&table_state)
                .with_size(gpui_component::Size::Large)
                .stripe(false)
                .bordered(false)
                .scrollbar_visible(true, true),
        )
        .child(
            // 覆盖层 canvas 负责测量容器宽度，把比例列宽换算成像素
            canvas(
                move |bounds, _, cx| {
                    measured_table.update(cx, |table, cx| {
                        if table
                            .delegate_mut()
                            .sync_redis_list_table_width(redis_table_fit_width(bounds.size.width))
                        {
                            table.refresh(cx);
                        }
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
}

fn redis_list_item_add_drawer(
    tab_id: TabId,
    key: String,
    rows: &[RedisListItemInputs],
    scroll: &ScrollHandle,
    applying: bool,
    colors: UiColors,
    _window: &Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let first_focus = rows.first().map(|row| row.value_input.read(cx).focus_handle(cx).clone());
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
            this.cancel_redis_list_item_drawer(cx);
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
                .key_context("RedisListItemAddDrawer")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_redis_list_item_drawer(cx);
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
                        .child(div().text_size(px(17.)).font_weight(gpui::FontWeight::SEMIBOLD).child("新增条目")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .relative()
                        .child(
                            div()
                                .id("redis-list-item-drawer-scroll")
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
                                        .child(redis_list_item_drawer_rows_panel(
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
                                                                this.add_redis_list_item_drawer_row(
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
                                    this.cancel_redis_list_item_drawer(cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            redis_detail_action_button("尾部添加", true, !applying, colors)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener({
                                        let key = key.clone();
                                        move |this, _, window, cx| {
                                            if !applying {
                                                this.confirm_redis_list_item_drawer(
                                                    tab_id,
                                                    key.clone(),
                                                    false,
                                                    window,
                                                    cx,
                                                );
                                            }
                                            cx.stop_propagation();
                                        }
                                    }),
                                ),
                        )
                        .child(
                            redis_detail_action_button("头部添加", true, !applying, colors)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener({
                                        let key = key.clone();
                                        move |this, _, window, cx| {
                                            if !applying {
                                                this.confirm_redis_list_item_drawer(
                                                    tab_id,
                                                    key.clone(),
                                                    true,
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

/// List「删除元素」抽屉（对齐 RedisInsight Remove elements）：
/// 上方为表单区（位置下拉 + 数量输入），底部独立 footer 放「取消 / 删除」按钮；
/// 点击「删除」先弹 RedisInsight 风格的二次确认浮层（红色破坏性确认按钮），确认后才真正 LPOP/RPOP。
/// `total` 为当前列表长度，用于在「删除数量 ≥ 长度」时提示将清空整列表。
#[allow(clippy::too_many_arguments)]
fn redis_list_item_remove_drawer(
    tab_id: TabId,
    key: String,
    select: &Entity<SelectState<SearchableVec<String>>>,
    count_input: Entity<InputState>,
    applying: bool,
    total: usize,
    pending_confirm: Option<RedisListItemRemoveConfirmTarget>,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let max_height = (f32::from(window.viewport_size().height) - 140.).min(320.);
    let count_focus = count_input.read(cx).focus_handle(cx).clone();
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
            this.cancel_redis_list_item_remove_drawer(cx);
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
                .track_focus(&count_focus)
                .key_context("RedisListItemRemoveDrawer")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_redis_list_item_remove_drawer(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // 标题行
                .child(
                    div()
                        .px_5()
                        .pt_4()
                        .pb_3()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_size(px(17.)).font_weight(gpui::FontWeight::SEMIBOLD).child("删除元素")),
                )
                // 表单区（RedisInsight 风格）：位置下拉 + 数量输入 靠左一行，占满剩余高度
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .px_5()
                        .pb_4()
                        .pt_1()
                        .relative()
                        // 点击表单区（浮层之外的空白/输入）只撤销二次确认浮层，抽屉保持打开
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                            if this.redis_list_item_remove_confirm.is_some() {
                                this.redis_list_item_remove_confirm = None;
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }))
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap(px(16.))
                                // 位置下拉：可见圆角边框 + 浅灰底（与白色面板区分），内容留白不贴边
                                .child(
                                    h_flex()
                                        .h(px(34.))
                                        .w(px(220.))
                                        .flex_none()
                                        .rounded(colors.radius)
                                        .border_1()
                                        .border_color(colors.border)
                                        .bg(if colors.is_dark {
                                            colors.panel_alt
                                        } else {
                                            rgb(0xf5f5f5)
                                        })
                                        .px_3()
                                        .items_center()
                                        .child(
                                            Select::new(select)
                                                .appearance(false)
                                                .small()
                                                .w_full()
                                                .h_full()
                                                .placeholder("选择删除位置")
                                                .menu_width(px(240.)),
                                        ),
                                )
                                // 数量输入：普通 Input（无 +/ - 加减步进按钮），
                                // 与位置下拉同高（34px）的圆角边框外框，样式对齐下拉框。
                                .child(
                                    div()
                                        .h(px(34.))
                                        .w(px(180.))
                                        .flex_none()
                                        .rounded(colors.radius)
                                        .border_1()
                                        .border_color(colors.border)
                                        .bg(colors.input_bg)
                                        .px_3()
                                        .items_center()
                                        .child(
                                            // placeholder 在 InputState 创建时（app_boot）已设置为 "请输入数量"
                                            Input::new(&count_input)
                                                .appearance(false)
                                                .focus_bordered(false)
                                                .w_full()
                                                .h_full(),
                                        ),
                                ),
                        )
                )
                // 底部 footer（对齐 RedisInsight）：分隔线 + 取消 / 删除按钮
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
                        // 取消：描边样式（白底、浅灰边框、深色文字）
                        .child(
                            redis_detail_action_button("取消", false, !applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.cancel_redis_list_item_remove_drawer(cx);
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        // 删除：实心主色，点击先弹二次确认浮层
                        .child(
                            redis_detail_action_button("删除", true, !applying, colors)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener({
                                        let key = key.clone();
                                        move |this, _, window, cx| {
                                            if !applying {
                                                this.open_redis_list_item_remove_confirm(
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
                )
        )
        // 二次确认浮层：相对全屏遮罩层定位，锚定在 footer 删除按钮上方（对齐 RedisInsight）。
        // 作为抽屉面板的兄弟节点放在遮罩层下，向上增长不会被面板的 overflow_hidden 裁剪。
        .when_some(pending_confirm, |this, target| {
            this.child(redis_list_item_remove_confirm_popover(
                target,
                total,
                colors,
                cx,
            ))
        })
}

/// List 元素删除二次确认浮层（对齐 RedisInsight ConfirmationPopover）：
/// 绝对定位在整个面板右下方、紧贴 footer 删除按钮之上（对齐 RedisInsight ConfirmationPopover），
/// 用简洁文案展示删除方向/数量与不可撤销提示，底部红色「确认删除」按钮真正执行。
/// `total` 为当前列表长度，删除数量覆盖整列表时额外给出警示。
fn redis_list_item_remove_confirm_popover(
    target: RedisListItemRemoveConfirmTarget,
    total: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 删除数量 ≥ 当前长度时，本次删除将清空整个列表，给出与 RedisInsight 一致的警示。
    let will_delete_all = total > 0 && target.count >= total;
    let direction = if target.head { "头部" } else { "尾部" };
    let cancel_view = cx.entity().downgrade();
    let confirm_view = cx.entity().downgrade();
    let confirm_target = target.clone();
    div()
        .absolute()
        .right(px(6.))
        // 58px footer + 分隔线，向上留出确认浮层的立足空间，避免被内容区高度裁剪
        .bottom(px(68.))
        .occlude()
        .w(px(272.))
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .shadow_lg()
        .bg(colors.panel_bg)
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .gap_2()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // 标题
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(format!("删除 {} 个元素", target.count)),
        )
        // 简洁描述：从哪个方向移除 + 不可撤销，合并为一行，减少浮层高度
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(format!("从「{}」移除 {} 个，不可撤销。", direction, target.count)),
        )
        .when(will_delete_all, |this| {
            this.child(
                div()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(if colors.is_dark { rgb(0xffb4a1) } else { rgb(0xb3402a) })
                    .child("删除后清空整个列表。"),
            )
        })
        .child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    redis_key_delete_confirm_button("取消", false, colors).on_mouse_down(
                        MouseButton::Left,
                        move |_, _, cx| {
                            let _ = cancel_view.clone().update(cx, |this, cx| {
                                this.redis_list_item_remove_confirm = None;
                                cx.notify();
                            });
                            cx.stop_propagation();
                        },
                    ),
                )
                .child(
                    redis_key_delete_confirm_button("确认删除", true, colors).on_mouse_down(
                        MouseButton::Left,
                        move |_, _, cx| {
                            let _ = confirm_view.clone().update(cx, |this, cx| {
                                this.apply_redis_list_item_remove(
                                    confirm_target.tab_id,
                                    confirm_target.key.clone(),
                                    confirm_target.head,
                                    confirm_target.count,
                                    cx,
                                );
                                this.redis_list_item_remove_confirm = None;
                                cx.notify();
                            });
                            cx.stop_propagation();
                        },
                    ),
                ),
        )
}

fn redis_list_item_drawer_rows_panel(
    rows: &[RedisListItemInputs],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut body = div().w_full().flex().flex_col().gap_2();
    for (row_index, row) in rows.iter().enumerate() {
        body = body.child(redis_list_item_drawer_row(
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

fn redis_list_item_drawer_row(
    row_index: usize,
    rows_len: usize,
    row: &RedisListItemInputs,
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
            redis_stream_add_input_box(row.value_input.clone(), colors)
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
                            this.remove_redis_list_item_drawer_row(row_index, window, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(AppIcon::Trash, 14., if can_delete { rgb(0xe5484d) } else { colors.border })),
        )
}

/// List「删除元素」按钮：复用「新增」同款样式，但用垃圾桶图标与红/警示色，点击弹出首/尾+数量删除抽屉。
fn redis_list_remove_button(enabled: bool, colors: UiColors) -> Div {
    div()
        .h(px(30.))
        .px_3()
        .rounded(colors.radius)
        .flex()
        .items_center()
        .gap_1()
        .justify_center()
        .border_1()
        .border_color(if enabled { rgb(0xe5484d) } else { colors.border_soft })
        .bg(if enabled {
            if colors.is_dark {
                rgb(0x3b1a1e)
            } else {
                rgb(0xffefef)
            }
        } else {
            colors.panel_alt
        })
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if enabled { rgb(0xe5484d) } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(colors.hover))
        })
        .child(app_icon(
            AppIcon::Trash,
            13.,
            if enabled { rgb(0xe5484d) } else { colors.border },
        ))
        .child("删除")
}

// Set 类型「新增成员」底部抽屉：达到最大高度后内容区滚动，底部按钮固定不动

fn redis_list_item_search_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 54)
}

fn redis_list_item_mutation_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 53)
}
