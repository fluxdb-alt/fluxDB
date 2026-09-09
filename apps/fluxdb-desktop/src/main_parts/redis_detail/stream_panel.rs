fn redis_key_detail_stream_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    applying: bool,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    this.sync_redis_stream_entry_page(tab_id, detail, window, cx);
    let page = this.active_redis_stream_page(tab_id, &detail.key);
    let entries = page
        .as_ref()
        .map(|page| page.entries.clone())
        .unwrap_or_default();
    let columns = redis_stream_entry_columns(&entries);
    let total = page.as_ref().map_or(entries.len(), |page| page.total);
    let has_more = page.as_ref().is_some_and(|page| page.next_cursor != "0");
    let more_loading = this
        .redis_stream_entry_more_loading
        .as_ref()
        .is_some_and(|loading| loading.0 == tab_id && loading.1 == detail.key);
    let stream_applying = applying
        || this
            ._redis_key_value_apply_tasks
            .contains_key(&redis_stream_entry_apply_task_id(tab_id));
    let pending_entry = this
        .pending_redis_stream_entry_delete
        .as_ref()
        .filter(|pending| pending.tab_id == tab_id && pending.key == detail.key)
        .map(|pending| pending.entry_id.clone());
    let pending_entry_for_overlay = pending_entry.clone();
    let stream_table = this.redis_stream_table_state.clone();
    let sync_entries = entries.clone();
    let sync_columns = columns.clone();
    let sync_key = detail.key.clone();
    cx.defer_in(window, move |this, _, cx| {
        let stream_table = this.redis_stream_table_state.clone();
        stream_table.update(cx, |table, cx| {
            if table.delegate_mut().set_data(
                tab_id,
                sync_key.clone(),
                sync_entries.clone(),
                sync_columns.clone(),
                stream_applying,
                pending_entry.clone(),
                colors,
            ) {
                table.refresh(cx);
            }
        });
    });
    redis_detail_panel(colors)
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
                .child(redis_detail_panel_title("Stream Data", colors))
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(redis_stream_range_controls(
                            tab_id,
                            detail.key.clone(),
                            this.redis_stream_since_input.clone(),
                            this.redis_stream_until_input.clone(),
                            colors,
                            cx,
                        ))
                        .child(redis_stream_add_button(!stream_applying, colors).on_mouse_down(
                            MouseButton::Left,
                            cx.listener({
                                let key = detail.key.clone();
                                move |this, _, window, cx| {
                                    if !stream_applying {
                                        this.open_redis_stream_entry_add(
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
        .child(redis_stream_table(&stream_table, colors))
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
                .child(format!("显示 {} / 共 {} 个条目", entries.len(), total))
                .when(has_more, |footer| {
                    footer.child(if more_loading {
                        loading_spinner_with_color(13., colors.muted).into_any_element()
                    } else {
                        redis_detail_toolbar_button("加载更多", AppIcon::ChevronDown, true, colors)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener({
                                    let key = detail.key.clone();
                                    move |this: &mut NavicatMain, _, _, cx| {
                                        let next_cursor = this
                                            .active_redis_stream_page(tab_id, &key)
                                            .map(|page| page.next_cursor)
                                            .unwrap_or_default();
                                        // "0" 表示已翻到最旧一条，不再续页。
                                        if !next_cursor.is_empty() && next_cursor != "0" {
                                            this.request_redis_stream_entry_search(
                                                tab_id,
                                                key.clone(),
                                                next_cursor,
                                                cx,
                                            );
                                        }
                                        cx.stop_propagation();
                                    }
                                }),
                            )
                            .into_any_element()
                    })
                }),
        )
        .child(redis_stream_groups_section(
            tab_id,
            detail.key.clone(),
            this,
            colors,
            cx,
        ))
        .when_some(pending_entry_for_overlay, |panel, entry_id| {
            panel.child(redis_stream_table_delete_confirm_overlay(
                tab_id,
                detail.key.clone(),
                entry_id,
                stream_applying,
                colors,
                cx,
            ))
        })
        .when(
            this.pending_redis_stream_entry_add
                .as_ref()
                .is_some_and(|pending| pending.tab_id == tab_id && pending.key == detail.key),
            |panel| {
                panel.child(redis_stream_entry_add_drawer(
                    tab_id,
                    detail.key.clone(),
                    this.redis_stream_entry_id_input.clone(),
                    this.redis_stream_maxlen_input.clone(),
                    &this.redis_stream_entry_field_rows,
                    &this.redis_stream_entry_drawer_scroll,
                    stream_applying,
                    colors,
                    window,
                    cx,
                ))
            },
        )
}

fn redis_stream_add_button(enabled: bool, colors: UiColors) -> Div {
    redis_set_member_add_button(enabled, colors)
}

// Stream「新增 Entry」底部抽屉：与 Set 新增成员抽屉同形，字段行超出后内部滚动

/// Stream 时间范围过滤栏：两个时间输入 + 应用/清除。
/// 解析失败时给出提示，不改动生效中的范围。
fn redis_stream_range_controls(
    tab_id: TabId,
    key: String,
    since_input: Entity<InputState>,
    until_input: Entity<InputState>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let apply_key = key.clone();
    let clear_key = key;
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(redis_stream_add_input_box(since_input, colors).w(px(150.)))
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("→"),
        )
        .child(redis_stream_add_input_box(until_input, colors).w(px(150.)))
        .child(
            redis_detail_toolbar_button("应用", AppIcon::Search, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this: &mut NavicatMain, _, _, cx| {
                    this.apply_redis_stream_range(tab_id, apply_key.clone(), cx);
                    cx.stop_propagation();
                }),
            ),
        )
        .child(
            redis_detail_toolbar_button("清除", AppIcon::Close, true, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this: &mut NavicatMain, _, window, cx| {
                    this.clear_redis_stream_range(tab_id, clear_key.clone(), window, cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

/// 消费者组只读概览：折叠标题 + 展开后的组/消费者列表。
fn redis_stream_groups_section(
    tab_id: TabId,
    key: String,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let expanded = this.redis_stream_groups_expanded;
    let loading = this
        .redis_stream_groups_loading
        .as_ref()
        .is_some_and(|loading| loading.0 == tab_id && loading.1 == key);
    let groups = this
        .redis_stream_groups
        .get(&(tab_id, key.clone()))
        .cloned()
        .unwrap_or_default();
    let toggle_key = key;
    let mut section = div()
        .flex_none()
        .w_full()
        .border_t_1()
        .border_color(colors.border_soft)
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(30.))
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(if expanded {
                    format!("▾ 消费者组（{}）", groups.len())
                } else {
                    "▸ 消费者组".to_string()
                })
                .when(loading, |this| {
                    this.child(loading_spinner_with_color(13., colors.muted))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this: &mut NavicatMain, _, _, cx| {
                        this.toggle_redis_stream_groups(tab_id, toggle_key.clone(), cx);
                        cx.stop_propagation();
                    }),
                ),
        );
    if !expanded {
        return section;
    }
    if groups.is_empty() && !loading {
        return section.child(
            div()
                .px_3()
                .pb_2()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child("该 Stream 没有消费者组"),
        );
    }
    for group in groups {
        let mut rows = div()
            .px_3()
            .pb_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.text)
                    .child(format!(
                        "{}　消费者 {}　未确认 {}　最后投递 {}",
                        group.name, group.consumers, group.pending, group.last_delivered_id
                    )),
            );
        for (name, pending, idle_ms) in group.consumer_detail {
            rows = rows.child(
                div()
                    .pl_4()
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .font_family("Menlo")
                    .child(format!(
                        "{name}　未确认 {pending}　空闲 {}s",
                        idle_ms / 1000
                    )),
            );
        }
        section = section.child(rows);
    }
    section
}

fn redis_stream_entry_add_drawer(
    tab_id: TabId,
    key: String,
    id_input: Entity<InputState>,
    maxlen_input: Entity<InputState>,
    field_rows: &[RedisStreamEntryFieldInputs],
    scroll: &ScrollHandle,
    applying: bool,
    colors: UiColors,
    window: &Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let id_value = id_input.read(cx).value().to_string();
    let row_values = field_rows
        .iter()
        .map(|row| {
            (
                row.field_input.read(cx).value().to_string(),
                row.value_input.read(cx).value().to_string(),
            )
        })
        .collect::<Vec<_>>();
    let id_error = redis_stream_entry_id_validation_error(&id_value);
    let fields_error = redis_stream_entry_fields_validation_error(&row_values);
    let validation_error = id_error.clone().or(fields_error.clone());
    let save_enabled = !applying && validation_error.is_none();
    // 抽屉最大高度：视口高度减去顶部留白，超出部分交给字段区滚动
    let max_height = (f32::from(window.viewport_size().height) - 120.).min(480.);

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
            this.pending_redis_stream_entry_add = None;
            cx.notify();
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
                .track_focus(&id_input.read(cx).focus_handle(cx))
                .key_context("RedisStreamEntryAddDrawer")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.pending_redis_stream_entry_add = None;
                    cx.notify();
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
                                .child("新增 Entry"),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .px_5()
                        .pb_4()
                        .child(
                            div()
                                .w_full()
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            redis_stream_add_input(
                                                "Entry ID",
                                                id_input.clone(),
                                                colors,
                                            )
                                            .w(px(320.)),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(if id_error.is_some() {
                                                    rgb(0xe5484d)
                                                } else {
                                                    colors.muted
                                                })
                                                .child(match id_error {
                                                    Some(error) => error,
                                                    None => "时间戳-序列号 或 *".to_string(),
                                                }),
                                        ),
                                )
                                .child(
                                    // MAXLEN 近似裁剪：留空表示不裁剪
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            redis_stream_add_input(
                                                "MAXLEN",
                                                maxlen_input.clone(),
                                                colors,
                                            )
                                            .w(px(320.)),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(colors.muted)
                                                .child(
                                                    "留空不裁剪；填数字则追加后按 MAXLEN ~ n 近似裁剪",
                                                ),
                                        ),
                                ),
                        )
                        .child(
                            // 字段行区：超出抽屉高度后滚动，右侧常显滚动条
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .w_full()
                                .pt_3()
                                .relative()
                                .child(
                                    div()
                                        .id("redis-stream-entry-drawer-scroll")
                                        .size_full()
                                        .flex()
                                        .flex_col()
                                        .track_scroll(&scroll)
                                        .overflow_y_scrollbar()
                                        .child(
                                            redis_stream_entry_fields_panel(
                                                field_rows, applying, colors, cx,
                                            )
                                            .flex_none()
                                            .when_some(fields_error, |this, error| {
                                                this.child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(rgb(0xe5484d))
                                                        .child(error),
                                                )
                                            })
                                            // 与 Set 抽屉一致：新增行按钮跟在行列表末尾
                                            .child(
                                                div().flex().justify_end().pt_1().child(
                                                    redis_set_member_drawer_add_button(
                                                        !applying, colors,
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |this, _, window, cx| {
                                                            if !applying {
                                                                this.add_redis_stream_entry_field_row(
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
                                .child(
                                    div().absolute().inset_0().child(
                                        Scrollbar::vertical(scroll)
                                            ,
                                    ),
                                ),
                        ),
                )
                .child(div().h(px(1.)).flex_none().bg(colors.border))
                .child(
                    // 底部按钮区：flex_none 固定，不随字段区滚动
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
                                    this.pending_redis_stream_entry_add = None;
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                        )
                        .child(
                            redis_detail_action_button("保存", true, save_enabled, colors).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    if save_enabled {
                                        this.request_redis_stream_entry_add(tab_id, key.clone(), cx);
                                    }
                                    cx.stop_propagation();
                                }),
                            ),
                        ),
                ),
        )
}

fn redis_stream_entry_field_row(
    row_index: usize,
    row: &RedisStreamEntryFieldInputs,
    rows_len: usize,
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let can_delete = rows_len > 1;
    let field_input = row.field_input.clone();
    let value_input = row.value_input.clone();
    div()
        .w_full()
        .h(px(34.))
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        // 标签与输入框同行：Field [输入框] Value [输入框] 🗑
        .child(redis_stream_add_input_label("Field", colors))
        .child(redis_stream_add_input_box(field_input, colors).w(px(200.)).flex_none())
        .child(redis_stream_add_input_label("Value", colors))
        .child(
            redis_stream_add_input_box(value_input, colors)
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
                .text_color(if can_delete {
                    rgb(0xe5484d)
                } else {
                    colors.border
                })
                .when(can_delete && !applying, |this| {
                    this.cursor_pointer().hover(move |style| style.bg(colors.hover))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if !applying && can_delete {
                            this.remove_redis_stream_entry_field_row(row_index, window, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(AppIcon::Trash, 14., if can_delete { rgb(0xe5484d) } else { colors.border })),
        )
}

fn redis_stream_entry_fields_panel(
    field_rows: &[RedisStreamEntryFieldInputs],
    applying: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut rows = div().w_full().flex().flex_col().gap_2();
    for (row_index, row) in field_rows.iter().enumerate() {
        rows = rows.child(redis_stream_entry_field_row(
            row_index,
            row,
            field_rows.len(),
            applying,
            colors,
            cx,
        ));
    }
    rows
}

// 带标签的输入项（Entry ID 用），标签在输入框上方

fn redis_stream_add_input(label: &'static str, input: Entity<InputState>, colors: UiColors) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(label),
        )
        .child(redis_stream_add_input_box(input, colors))
}

// 行内标签：与输入框同一行，宽度固定保证多行左对齐

fn redis_stream_add_input_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .flex_none()
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(label)
}

// 无标签输入框：Field / Value 行复用，标签由 redis_stream_add_input_label 内联提供

fn redis_stream_add_input_box(input: Entity<InputState>, colors: UiColors) -> Div {
    div()
        .w_full()
        .h_full()
        .min_h(px(34.))
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
                .text_size(px(12.)),
        )
}

fn redis_stream_entry_columns(entries: &[RedisStreamEntryRow]) -> Vec<String> {
    entries
        .iter()
        .flat_map(|entry| entry.fields.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn redis_stream_entry_search_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 52)
}

fn redis_stream_groups_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 51)
}

fn redis_stream_entry_apply_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 61)
}

/// 把 Hash 字段 TTL 输入框的内容翻译成写入语义。
/// `changed` 为 false 表示用户没动这一格，必须保留原 TTL，否则 HSET 会把它清掉。
/// 解析 MAXLEN 输入：留空 → None（不裁剪），否则必须是大于 0 的整数。
fn redis_stream_maxlen_from_input(
    input: &str,
) -> std::result::Result<Option<u64>, &'static str> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    match input.parse::<u64>() {
        Ok(maxlen) if maxlen > 0 => Ok(Some(maxlen)),
        _ => Err("Stream MAXLEN 必须是大于 0 的整数，留空表示不裁剪"),
    }
}

/// 毫秒时间戳 → 时间范围输入框文本（本地时间），与 `redis_stream_time_from_input` 互逆。
fn redis_stream_time_to_input(millis: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(millis as i64).single() {
        Some(time) => time.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => String::new(),
    }
}

/// 解析时间范围输入框：留空 → None；否则按本地时间 `YYYY-MM-DD HH:MM:SS` 解析成毫秒时间戳。
/// 也接受只写日期（按当天 00:00:00 处理）。
fn redis_stream_time_from_input(input: &str) -> std::result::Result<Option<u64>, &'static str> {
    use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let naive = NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M"))
        .or_else(|_| {
            NaiveDate::parse_from_str(input, "%Y-%m-%d")
                .map(|date| date.and_hms_opt(0, 0, 0).unwrap_or_default())
        })
        .map_err(|_| "时间格式应为 YYYY-MM-DD HH:MM:SS")?;
    let millis = Local
        .from_local_datetime(&naive)
        .single()
        .ok_or("该时间在本地时区不存在或有歧义")?
        .timestamp_millis();
    u64::try_from(millis)
        .map(Some)
        .map_err(|_| "时间超出可表示范围")
}

fn redis_stream_entry_field_pair(field: String, value: String) -> Result<(String, String), String> {
    let field = field.trim().to_string();
    if field.is_empty() {
        return Err("字段名不能为空".to_string());
    }
    Ok((field, value))
}

fn redis_stream_entry_id_validation_error(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() {
        return Some("Entry ID 不能为空".to_string());
    }
    if id == "*" {
        return None;
    }
    let Some((timestamp, sequence)) = id.split_once('-') else {
        return Some("Entry ID 必须是 timestamp-sequence 或 *".to_string());
    };
    if timestamp.is_empty()
        || sequence.is_empty()
        || timestamp.parse::<u64>().is_err()
        || sequence.parse::<u64>().is_err()
    {
        return Some("Entry ID 必须是 timestamp-sequence 或 *".to_string());
    }
    None
}

fn redis_stream_entry_fields_validation_error(fields: &[(String, String)]) -> Option<String> {
    if fields.is_empty() {
        return Some("至少添加一组 Field/Value".to_string());
    }
    for (field, value) in fields {
        if redis_stream_entry_field_pair(field.clone(), value.clone()).is_err() {
            return Some("字段名不能为空".to_string());
        }
    }
    None
}

fn redis_stream_entry_field_pairs_from_snapshot(
    fields: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    if fields.is_empty() {
        return Err("至少添加一组 Field/Value".to_string());
    }
    fields
        .iter()
        .cloned()
        .map(|(field, value)| redis_stream_entry_field_pair(field, value))
        .collect()
}
