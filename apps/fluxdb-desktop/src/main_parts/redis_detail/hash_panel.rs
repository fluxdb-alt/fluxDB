fn redis_key_detail_hash_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    applying: bool,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    this.sync_redis_hash_field_inputs(tab_id, detail, window, cx);
    // 惰性探测服务端版本（字段级 TTL 能力开关）：缓存命中或探测中则跳过
    this.ensure_redis_server_version_cache(tab_id, cx);
    // 切换 key/标签后清掉遗留的完整值查看状态（内嵌面板只在 tab+key 匹配时渲染）
    if this
        .redis_hash_full_value_viewer
        .borrow()
        .as_ref()
        .is_some_and(|viewer| viewer.tab_id != tab_id || viewer.key != detail.key)
    {
        // 清状态前取消该 tab 在飞的完整值加载任务（drop Task 即取消），
        // 避免旧任务完成时误写新 key 的 viewer 或遗留去重占位阻塞下次打开
        let stale_tab = this
            .redis_hash_full_value_viewer
            .borrow()
            .as_ref()
            .map(|viewer| viewer.tab_id)
            .unwrap_or(tab_id);
        this._data_load_tasks
            .remove(&redis_hash_field_mutation_task_id(stale_tab));
        *this.redis_hash_full_value_viewer.borrow_mut() = None;
    }
    // 当前 tab+key 是否打开完整值内嵌面板
    let viewer = this.redis_hash_full_value_viewer.borrow().clone();
    let viewer_open = viewer
        .as_ref()
        .is_some_and(|viewer| viewer.tab_id == tab_id && viewer.key == detail.key);
    let search_query = this
        .redis_hash_field_search_queries
        .get(&(tab_id, detail.key.clone()))
        .cloned()
        .unwrap_or_default();
    let page = this
        .redis_hash_field_search_pages
        .get(&(tab_id, detail.key.clone(), search_query.clone()));
    let rows = this.redis_hash_field_rows.clone();
    let editing = this.redis_hash_field_editing.clone();
    let hovered = this.redis_hash_field_hovered.clone();
    let search_loading = this
        .redis_hash_field_search_loading
        .as_ref()
        .is_some_and(|loading| loading == &(tab_id, detail.key.clone(), search_query.clone()));
    let more_loading = this
        .redis_hash_field_search_more_loading
        .as_ref()
        .is_some_and(|loading| loading == &(tab_id, detail.key.clone(), search_query.clone()));
    let has_more = page.is_some_and(|page| page.next_cursor != "0");
    let total = page.map_or(rows.len(), |page| page.total);
    let panel_applying = applying
        || this
            ._data_load_tasks
            .contains_key(&redis_hash_field_mutation_task_id(tab_id));
    // 把渲染期派生出的数据同步进表格 delegate；defer 延迟到本帧渲染结束后执行，
    // 避免在 NavicatMain 渲染过程中直接 update 表格实体造成 double-lease。
    let table_state = this.redis_hash_table_state.clone();
    let sync_rows = rows.clone();
    let sync_editing = editing.clone();
    let sync_hovered = hovered.clone();
    let sync_key = detail.key.clone();
    let sync_pending_delete = this.pending_redis_hash_field_delete.clone();
    let sync_value_input = this.redis_hash_value_edit_input.clone();
    let sync_ttl_input = this.redis_hash_ttl_edit_input.clone();
    cx.defer_in(window, move |this, _, cx| {
        let table_state = this.redis_hash_table_state.clone();
        // 从版本缓存取当前连接的版本；取不到（探测中/失败）为 None，维持保守禁用
        let sync_version = this
            .redis_tab_connection_id(tab_id)
            .and_then(|connection_id| this.redis_server_versions.get(&connection_id).copied())
            .flatten();
        table_state.update(cx, |table, cx| {
            if table.delegate_mut().set_data(
                tab_id,
                sync_key.clone(),
                sync_rows.clone(),
                sync_editing.clone(),
                sync_hovered.clone(),
                sync_pending_delete.clone(),
                panel_applying,
                search_loading,
                sync_value_input.clone(),
                sync_ttl_input.clone(),
                colors,
                sync_version,
            ) {
                table.refresh(cx);
            }
        });
    });
    // 点击面板内任意非编辑区域：确认当前编辑（Redis Insight 失焦提交语义）
    // 编辑单元格与 Value/TTL/删除单元格都 stop_propagation，只有点击空白/序列/Field/表头等才冒泡到这里
    let commit_tab_id = tab_id;
    let commit_key = detail.key.clone();
    redis_detail_panel(colors)
        .relative()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _window, cx| {
            let is_editing_this_key = this.redis_hash_field_editing.as_ref().is_some_and(
                |editing| editing.tab_id == commit_tab_id && editing.key == commit_key,
            );
            if is_editing_this_key {
                let _ = this.confirm_redis_hash_field_edit(cx);
            }
        }))
        .child(
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .child(redis_detail_panel_title("Hash Data", colors))
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(redis_set_search_box(
                            this.redis_hash_field_search_input.clone(),
                            colors,
                        ))
                        .child(
                            redis_set_member_add_button(!panel_applying, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener({
                                    let key = detail.key.clone();
                                    move |this, _, window, cx| {
                                        if !panel_applying {
                                            this.open_redis_hash_field_add_drawer(
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
                // 打开完整值内嵌面板时替换表格区域；否则渲染字段表格 + 底部计数/加载更多
                .when(viewer_open, |panel| {
                    panel.child(redis_hash_full_value_inline_panel(
                        tab_id,
                        viewer.clone().unwrap(),
                        this.redis_hash_value_edit_input.clone(),
                        colors,
                        cx,
                    ))
                })
                .when(!viewer_open, |panel| {
                    panel
                        .child(redis_hash_table(&table_state))
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
                                .child(format!("显示 {} / 共 {} 个字段", rows.len(), total))
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
                                                            .redis_hash_field_search_pages
                                                            .get(&(
                                                                tab_id,
                                                                key.clone(),
                                                                search_query.clone(),
                                                            ))
                                                            .map(|page| page.next_cursor.clone())
                                                            .unwrap_or_default();
                                                        if !next_cursor.is_empty() {
                                                            this.request_redis_hash_field_search(
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
                                        }),
                                ),
                        )
                }),
        )
        .when(
            this.pending_redis_hash_field_drawer
                .as_ref()
                .is_some_and(|pending| pending.tab_id == tab_id && pending.key == detail.key),
            |panel| {
                panel.child(redis_hash_field_add_drawer(
                    tab_id,
                    detail.key.clone(),
                    &this.redis_hash_field_drawer_rows,
                    &this.redis_hash_field_drawer_scroll,
                    panel_applying,
                    colors,
                    window,
                    cx,
                ))
            },
        )
}

fn redis_hash_field_edit_icon(
    icon: AppIcon,
    tooltip: &'static str,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(tooltip)
        .size(px(22.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .hover(move |style| style.bg(colors.hover))
        .child(app_icon(icon, 14., colors.muted))
}

/// 截断行的「查看完整值」入口图标：点击打开完整值内嵌面板（懒加载非截断原始值）。
fn redis_hash_field_view_icon(
    icon: AppIcon,
    tooltip: &'static str,
    colors: UiColors,
    view: WeakEntity<NavicatMain>,
    tab_id: TabId,
    key: String,
    field: String,
    cx: &mut Context<TableState<RedisHashTableDelegate>>,
) -> Stateful<Div> {
    div()
        .id(tooltip)
        .size(px(22.))
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, _, window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                let _ = view.update(cx, |this, cx| {
                    this.open_redis_hash_full_value_viewer(tab_id, key.clone(), field.clone(), window, cx);
                });
                cx.stop_propagation();
            }),
        )
        .child(app_icon(icon, 14., colors.muted))
}

/// 完整值内嵌面板（查看/编辑 >1MB 被截断 hash 字段）：内嵌渲染在 Hash Data 表格区域内，
/// 不遮挡左侧 key 列表与面板标题/搜索/新增按钮。未编辑态只读自动换行文本（纵向滚动，
/// 无断词超长串横向滚动兜底）；编辑态复用 `redis_hash_value_edit_input` 多行输入。
fn redis_hash_full_value_inline_panel(
    tab_id: TabId,
    viewer: RedisHashFullValueViewer,
    input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let saving = viewer.saving;
    let key = viewer.key.clone();
    let field = viewer.field.clone();
    let bytes = viewer
        .full_value
        .as_ref()
        .map(|value| value.len())
        .unwrap_or(0);
    // 头部副标题：加载中 / 加载失败 / 字节数
    let subtitle = if viewer.loading {
        "加载中...".to_string()
    } else if viewer.error {
        "加载失败".to_string()
    } else {
        format!("{} 字节", bytes)
    };
    let body = div().flex_1().min_h(px(0.)).overflow_hidden();
    let body = if viewer.loading {
        // 加载中：表格区域内居中 loading
        body.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(loading_spinner_with_color(22., colors.muted)),
        )
    } else if viewer.error {
        // 加载失败：错误提示 + 重试按钮
        body.child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(0xe5484d))
                        .child("加载完整值失败，请重试"),
                )
                .child({
                    let retry_key = key.clone();
                    let retry_field = field.clone();
                    Button::new("redis-hash-full-value-retry")
                        .label("重试")
                        .small()
                        .outline()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let window_handle = window.window_handle();
                            this.load_redis_hash_full_value(
                                tab_id,
                                retry_key.clone(),
                                retry_field.clone(),
                                window_handle,
                                cx,
                            );
                            cx.stop_propagation();
                        }))
                }),
        )
    } else if let Some(full_value) = viewer.full_value.as_deref() {
        if viewer.editing {
            // 编辑态：用共享多行输入展示完整值，底部提供 取消/保存。
            // 必须用 Input::new 包装（裸 child(Entity) 不会接线焦点/键盘事件）。
            body.child(
                div()
                    .h_full()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .w_full()
                            .child(
                                div()
                                    .w_full()
                                    .h_full()
                                    .min_h(px(0.))
                                    .rounded(colors.radius)
                                    .border_1()
                                    .border_color(colors.border)
                                    .bg(colors.input_bg)
                                    .overflow_hidden()
                                    .child(
                                        Input::new(&input)
                                            .appearance(false)
                                            .focus_bordered(false)
                                            .disabled(saving)
                                            .w_full()
                                            .h_full()
                                            .px_2()
                                            .py_2()
                                            .font_family("Menlo")
                                            .text_size(px(13.))
                                            .line_height(px(19.)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("redis-hash-full-value-cancel-edit")
                                    .label("取消编辑")
                                    .small()
                                    .outline()
                                    .w(px(82.))
                                    .disabled(saving)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !saving {
                                            this.cancel_redis_hash_full_value_edit(cx);
                                        }
                                        cx.stop_propagation();
                                    })),
                            )
                            .child(
                                Button::new("redis-hash-full-value-save")
                                    .label("保存")
                                    .small()
                                    .primary()
                                    .w(px(72.))
                                    .disabled(saving)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !saving {
                                            this.save_redis_hash_full_value(cx);
                                        }
                                        cx.stop_propagation();
                                    })),
                            ),
                    ),
            )
        } else {
            // 查看态：只读自动换行文本（长文本按面板宽度折行，纵向滚动阅读；
            // 极端无断词超长串如 base64/hex 仍保留横向滚动兜底，避免撑破布局），空值显示空态
            body.child(
                div()
                    .id("redis-hash-full-value-scroll")
                    .size_full()
                    .overflow_x_scroll()
                    .overflow_y_scrollbar()
                    .font_family("Menlo")
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .text_color(colors.text)
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .child(if full_value.is_empty() {
                                div()
                                    .text_color(colors.muted)
                                    .child("(空值)")
                                    .into_any_element()
                            } else {
                                full_value.to_string().into_any_element()
                            }),
                    ),
            )
        }
    } else {
        // full_value 为 None 且非 loading/error（理论不可达）：空态兜底
        body.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(div().text_color(colors.muted).child("(空值)")),
        )
    };
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
                .flex_none()
                .h(px(36.))
                .px_3()
                .border_b_1()
                .border_color(colors.border_soft)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(format!("完整值 · {}", field)),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .child(subtitle),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new("redis-hash-full-value-close")
                                .label("关闭")
                                .small()
                                .outline()
                                .w(px(72.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.close_redis_hash_full_value_viewer(cx);
                                    cx.stop_propagation();
                                })),
                        )
                        .child(
                            Button::new("redis-hash-full-value-edit")
                                .label("编辑")
                                .small()
                                .outline()
                                .w(px(72.))
                                .disabled(
                                    viewer.loading
                                        || viewer.error
                                        || viewer.editing
                                        || viewer.saving
                                        || viewer.full_value.is_none(),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.begin_redis_hash_full_value_edit(window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
        )
        .child(body)
}

/// Hash 添加字段抽屉（对齐 `redis_hash_field_add_drawer` 原结构）。
fn redis_hash_field_add_drawer(
    tab_id: TabId,
    key: String,
    rows: &[RedisHashFieldDrawerInputs],
    scroll: &ScrollHandle,
    applying: bool,
    colors: UiColors,
    _window: &Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let first_focus = rows.first().map(|row| row.field_input.read(cx).focus_handle(cx).clone());
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
            this.cancel_redis_hash_field_drawer(cx);
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
                .key_context("RedisHashFieldAddDrawer")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_redis_hash_field_drawer(cx);
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
                        .child(div().text_size(px(17.)).font_weight(gpui::FontWeight::SEMIBOLD).child("新增字段")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .relative()
                        .child(
                            div()
                                .id("redis-hash-field-drawer-scroll")
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
                                        .child(redis_hash_field_drawer_rows_panel(
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
                                                                this.add_redis_hash_field_drawer_row(
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
                                    this.cancel_redis_hash_field_drawer(cx);
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
                                            this.confirm_redis_hash_field_drawer(
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

fn redis_hash_field_drawer_rows_panel(
    rows: &[RedisHashFieldDrawerInputs],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut body = div().w_full().flex().flex_col().gap_2();
    for (row_index, row) in rows.iter().enumerate() {
        body = body.child(redis_hash_field_drawer_row(
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

fn redis_hash_field_drawer_row(
    row_index: usize,
    rows_len: usize,
    row: &RedisHashFieldDrawerInputs,
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
            redis_stream_add_input_box(row.field_input.clone(), colors)
                .w(px(220.))
                .flex_none(),
        )
        .child(
            redis_stream_add_input_box(row.value_input.clone(), colors)
                .flex_1()
                .min_w(px(0.)),
        )
        .child(
            redis_stream_add_input_box(row.ttl_input.clone(), colors)
                .w(px(96.))
                .flex_none(),
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
                            this.remove_redis_hash_field_drawer_row(row_index, window, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(AppIcon::Trash, 14., if can_delete { rgb(0xe5484d) } else { colors.border })),
        )
}

fn redis_hash_field_search_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 58)
}

fn redis_hash_field_mutation_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 57)
}

/// 连接器对大 Hash 字段值（>1MB）返回的截断标记前缀。
/// 见 `crates/fluxdb-connectors/src/parts/redis.rs` 的 REDIS_HASH_TRUNCATED_MARKER。
const REDIS_HASH_TRUNCATED_MARKER: &str = "[Truncated due to length]";

/// 值被截断时不允许编辑：截断串只是原值前 30 字符的片段，回写等于用片段覆盖完整数据。
fn redis_hash_value_is_truncated(value: &str) -> bool {
    value.starts_with(REDIS_HASH_TRUNCATED_MARKER)
}

/// 大字段（值被截断）的 TTL 单元格是否可编辑的 gating：
/// - 版本 ≥7.4 → 可编辑（走纯 TTL 命令 `HPEXPIRE`/`HPERSIST`，只改 TTL 不重写 value）。
/// - 版本未知（`None`）→ 截断行维持禁用（保守兜底），非截断行可编辑。
/// - 版本 <7.4 → 禁用并提示需 7.4+（字段级 TTL 不可用）。
fn redis_hash_field_ttl_editable(version: Option<&RedisServerVersion>, truncated: bool) -> bool {
    match version {
        Some(version) => version.at_least(7, 4),
        None => !truncated,
    }
}

/// 截断行 TTL 禁用时的 tooltip 文案：
/// 版本 <7.4 → 明确提示字段级 TTL 需版本；版本未知 → 保留原来的「避免误写原始数据」提示。
fn redis_hash_field_ttl_disabled_tooltip(
    version: Option<&RedisServerVersion>,
) -> String {
    match version {
        Some(version) => format!(
            "当前 Redis 版本 {}.{}.{} 不支持字段级 TTL，需 7.4+",
            version.major, version.minor, version.patch
        ),
        None => "值已被截断，为避免误写原始数据，TTL 编辑已禁用".to_string(),
    }
}

fn redis_hash_field_ttl_command(
    input: &str,
    changed: bool,
) -> std::result::Result<RedisHashFieldTtl, &'static str> {
    if !changed {
        return Ok(RedisHashFieldTtl::Keep);
    }
    let input = input.trim();
    if input.is_empty() {
        return Ok(RedisHashFieldTtl::Persist);
    }
    match input.parse::<u64>() {
        Ok(seconds) if seconds > 0 => Ok(RedisHashFieldTtl::Seconds(seconds)),
        _ => Err("Redis TTL 必须是大于 0 的秒数，留空表示永不过期"),
    }
}

fn redis_hash_field_ttl_display_value(ttl: &str) -> String {
    let trimmed = ttl.trim();
    if let Some(seconds) = trimmed.strip_suffix('s')
        && !seconds.is_empty()
        && seconds.chars().all(|ch| ch.is_ascii_digit())
    {
        return seconds.to_string();
    }
    trimmed.to_string()
}
