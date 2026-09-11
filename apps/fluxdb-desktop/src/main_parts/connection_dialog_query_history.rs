fn query_history_drawer(
    state: &AppState,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let entries = query_history_entries(state, this);
    let search_input = this.query_history_search_input.clone();
    let detail = this.query_history_detail.clone();
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.58)
        } else {
            opaque_grey(0.75, 0.28)
        })
        .flex()
        .justify_end()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.query_history_open = false;
            cx.notify();
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(420.))
                .h_full()
                .border_l_1()
                .border_color(colors.border)
                .bg(if colors.is_dark {
                    rgb(0x181b20)
                } else {
                    rgb(0xffffff)
                })
                .shadow(vec![box_shadow(
                    px(-12.),
                    px(0.),
                    px(28.),
                    px(0.),
                    hsla(0., 0., 0., 0.18),
                )])
                .flex()
                .flex_col()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(query_history_header(entries.len(), colors, cx))
                .child(
                    div()
                        .relative()
                        .px_3()
                        .pt_3()
                        .pb_2()
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(Input::new(&search_input).small())
                        .child(query_history_kind_filters(this.query_history_kind_filter, colors, cx))
                        .child(query_history_scope_filters(state, this, colors, cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .p_2()
                        .when(entries.is_empty(), |this| {
                            this.child(
                                div()
                                    .h(px(120.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(13.))
                                    .text_color(colors.muted)
                                    .child("没有匹配的 SQL 历史"),
                            )
                        })
                        .children(entries.into_iter().map(|entry| query_history_row(entry, state, this.query_history_search.as_str(), colors, cx))),
                ),
        )
        .when_some(detail, |this, entry| {
            this.child(query_history_detail_modal(entry, state, colors, cx))
        })
}

/// Redis Workbench 历史抽屉：按 `redis_history_scope`（连接 + 逻辑库）拉取并展示命令历史，
/// 支持重跑 / 删除单条与清空；视觉样式与 SQL 历史抽屉保持一致（明暗双主题）。
fn redis_workbench_history_drawer(
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let Some(scope) = this.redis_history_scope.clone() else {
        return div();
    };
    // 仅加载当前 scope 的最近 30 条历史，再在内存里按搜索词进一步过滤；
    // 不改变 scope（连接 + 逻辑库）的过滤逻辑。
    let loaded = this.controller.load_history(&scope, 30);
    let loaded_total = loaded.len();
    let connection_name = this
        .controller
        .state()
        .connections
        .iter()
        .find(|connection| connection.config.id == scope.connection_id())
        .map(|connection| connection.config.name.clone());
    let db = redis_scope_database(&scope);
    let search_text = this.redis_history_search.trim().to_lowercase();
    let has_search = !search_text.is_empty();
    let items = if has_search {
        loaded
            .into_iter()
            .filter(|item| {
                redis_history_matches(item, connection_name.as_deref(), db, &search_text)
            })
            .collect::<Vec<_>>()
    } else {
        loaded
    };
    let count = items.len();
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.58)
        } else {
            opaque_grey(0.75, 0.28)
        })
        .flex()
        .justify_end()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
            this.redis_history_open = false;
            this.redis_history_scope = None;
            this.close_redis_history_search(window, cx);
            cx.notify();
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(420.))
                .h_full()
                .border_l_1()
                .border_color(colors.border)
                .bg(if colors.is_dark {
                    rgb(0x181b20)
                } else {
                    rgb(0xffffff)
                })
                .shadow(vec![box_shadow(
                    px(-12.),
                    px(0.),
                    px(28.),
                    px(0.),
                    hsla(0., 0., 0., 0.18),
                )])
                .flex()
                .flex_col()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(redis_workbench_history_header(scope.clone(), count, colors, cx))
                .child(
                    div()
                        .relative()
                        .px_3()
                        .pt_3()
                        .pb_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .child(Input::new(&this.redis_history_search_input).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .p_2()
                        .when(items.is_empty(), |this| {
                            // 无历史与「搜索无结果」两种空态文案区分。
                            let message = if loaded_total == 0 {
                                "暂无命令历史"
                            } else {
                                "没有匹配的命令历史"
                            };
                            this.child(
                                div()
                                    .h(px(120.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(13.))
                                    .text_color(colors.muted)
                                    .child(message),
                            )
                        })
                        .children(
                            items
                                .into_iter()
                                .map(|item| {
                                    redis_workbench_history_row(
                                        item,
                                        connection_name.as_deref(),
                                        scope.clone(),
                                        colors,
                                        cx,
                                    )
                                }),
                        ),
                ),
        )
}

/// Redis 历史抽屉头部：标题 + 记录数 + 「清空」/ 关闭。
fn redis_workbench_history_header(
    scope: WorkbenchHistoryScope,
    count: usize,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(54.))
        .px_4()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(colors.text)
                        .child("历史 — 命令历史"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(format!("共 {} 条", count)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(redis_history_action_chip("清空".to_string(), false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(
                            AppCommand::ClearRedisWorkbenchHistory {
                                scope: scope.clone(),
                            },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                ))
                .child(
                    div()
                        .size(px(28.))
                        .rounded(colors.radius_lg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(colors.hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.redis_history_open = false;
                                this.redis_history_scope = None;
                                this.close_redis_history_search(window, cx);
                                cx.notify();
                                cx.stop_propagation();
                            }),
                        )
                        .child(app_icon(AppIcon::Close, 16., colors.muted)),
                ),
        )
}

/// 单条 Redis 历史记录行：命令文本（等宽、截断）+ 时间 + 摘要 + 成功/失败态 + 重跑/删除。
fn redis_workbench_history_row(
    item: WorkbenchHistoryItem,
    connection_name: Option<&str>,
    scope: WorkbenchHistoryScope,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let text = item.text.clone();
    let time = query_history_time_label(item.executed_at_unix_secs);
    let summary = item.summary.clone();
    let status = if item.success { "成功" } else { "失败" };
    let status_color = if item.success {
        rgb(0x20c76a)
    } else {
        rgb(0xd64545)
    };
    let scope_for_rerun = scope.clone();
    let scope_for_delete = scope.clone();
    let db = redis_scope_database(&scope);
    let id = item.id;
    div()
        .min_h(px(64.))
        .rounded(colors.radius_lg)
        .px_2()
        .py_2()
        .flex()
        .items_center()
        .gap_2()
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_family("Menlo")
                        .text_color(colors.text)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(query_history_line(&text)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(if let Some(name) = connection_name {
                            format!("{name} · DB{db} · {time}")
                        } else {
                            format!("DB{db} · {time}")
                        }),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(status_color)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(format!("{status} · {summary}")),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(redis_history_action_chip("重跑".to_string(), false, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        let scope_ok = matches!(
                            scope_for_rerun,
                            WorkbenchHistoryScope::Redis { .. }
                        );
                        if scope_ok
                            && let Some(tab_id) = this.controller.state().active_tab().map(|tab| tab.id)
                        {
                            this.dispatch(
                                AppCommand::UpdateRedisWorkbenchText {
                                    tab_id,
                                    text: text.clone(),
                                },
                                cx,
                            );
                            // 含破坏性命令先弹确认框；确认后才派发执行。
                            if this.request_redis_dangerous_confirmation(tab_id, &text, cx) {
                                // 已弹确认框，交由确认流程执行。
                            } else {
                                this.dispatch(AppCommand::ExecuteRedisWorkbench(tab_id), cx);
                            }
                        }
                        cx.stop_propagation();
                    }),
                ))
                .child(redis_history_action_chip("删除".to_string(), true, colors).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.dispatch(
                            AppCommand::DeleteRedisWorkbenchHistory {
                                scope: scope_for_delete.clone(),
                                id,
                            },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )),
        )
}

/// Redis 历史条目的本地文本匹配：以命令文本为主，顺带匹配摘要、连接名与 DB 编号，大小写不敏感。
/// `query` 需已 trim + 转小写。
fn redis_history_matches(
    item: &WorkbenchHistoryItem,
    connection_name: Option<&str>,
    db: u32,
    query: &str,
) -> bool {
    let connection_lower = connection_name.map(str::to_lowercase);
    let haystacks = [
        Some(item.text.to_lowercase()),
        Some(item.summary.to_lowercase()),
        connection_lower,
        Some(format!("db{db}")),
    ];
    haystacks.into_iter().flatten().any(|haystack| haystack.contains(query))
}

/// 从 scope 提取 Redis 逻辑库编号（仅 Redis 分支有效，SQL 分支回退 0）。
fn redis_scope_database(scope: &WorkbenchHistoryScope) -> u32 {
    match scope {
        WorkbenchHistoryScope::Redis { database, .. } => *database,
        WorkbenchHistoryScope::Sql { .. } => 0,
    }
}

/// 历史抽屉专用的小型操作 chip（与 `query_history_chip` 同视觉，额外支持危险色）。
fn redis_history_action_chip(label: String, danger: bool, colors: UiColors) -> Div {
    div()
        .h(px(24.))
        .flex_none()
        .px_2()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(if colors.is_dark {
            rgb(0x20232a)
        } else {
            rgb(0xffffff)
        })
        .text_size(px(12.))
        .text_color(if danger { rgb(0xd64545) } else { colors.text })
        .flex()
        .items_center()
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn query_history_quick_search_modal(
    state: &AppState,
    this: &NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let entries = query_history_quick_entries(state, this);
    let selected = if entries.is_empty() {
        0
    } else {
        this.query_history_quick_selected.min(entries.len() - 1)
    };
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.48)
        } else {
            opaque_grey(0.75, 0.20)
        })
        .flex()
        .items_start()
        .justify_center()
        .pt(px(72.))
        .px_4()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.query_history_quick_open = false;
            cx.notify();
            cx.stop_propagation();
        }))
        .child(
            div()
                .w_full()
                .max_w(px(680.))
                .max_h(px(520.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(18.),
                    px(42.),
                    px(0.),
                    hsla(0., 0., 0., 0.26),
                )])
                .flex()
                .flex_col()
                .overflow_hidden()
                .key_context("QueryHistoryQuickSearch")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(54.))
                        .px_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(app_icon(AppIcon::Search, 16., colors.muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(Input::new(&this.query_history_quick_search_input).small()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .whitespace_nowrap()
                                .child("Esc"),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius_lg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |style| style.bg(colors.hover))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.query_history_quick_open = false;
                                        cx.notify();
                                        cx.stop_propagation();
                                    }),
                                )
                                .child(app_icon(AppIcon::Close, 16., colors.muted)),
                        ),
                )
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(colors.border)
                        .child(query_history_quick_kind_filters(
                            this.query_history_quick_kind_filter,
                            colors,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .p_2()
                        .when(entries.is_empty(), |this| {
                            this.child(
                                div()
                                    .h(px(118.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(13.))
                                    .text_color(colors.muted)
                                    .child("没有匹配的 SQL"),
                            )
                        })
                        .children(entries.into_iter().enumerate().map(|(index, entry)| {
                            query_history_quick_row(
                                entry,
                                state,
                                this.query_history_quick_search.as_str(),
                                selected == index,
                                colors,
                                cx,
                            )
                        })),
                ),
        )
}

fn query_history_header(count: usize, colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .h(px(54.))
        .px_4()
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(colors.text)
                        .child("SQL 历史"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(format!("匹配 {}", count)),
                ),
        )
        .child(
            div()
                .size(px(28.))
                .rounded(colors.radius_lg)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.query_history_open = false;
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(app_icon(AppIcon::Close, 16., colors.muted)),
        )
}

#[derive(Clone)]
struct QueryHistoryListEntry {
    entry: QueryHistoryEntry,
}

fn query_history_entries(state: &AppState, this: &NavicatMain) -> Vec<QueryHistoryListEntry> {
    let mut entries = state
        .query_history
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            if !query_history_filter_matches(entry, state, this) {
                return None;
            }
            let connection = query_history_connection_name(state, entry.connection_id);
            let score = query_history_search_score(entry, connection.as_deref(), &this.query_history_search)?;
            Some((score, index, QueryHistoryListEntry { entry: entry.clone() }))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    entries
        .into_iter()
        .take(50)
        .map(|(_, _, entry)| entry)
        .collect()
}

fn query_history_quick_entries(state: &AppState, this: &NavicatMain) -> Vec<QueryHistoryListEntry> {
    let mut entries = state
        .query_history
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            if !query_history_kind_filter_matches(entry, this.query_history_quick_kind_filter) {
                return None;
            }
            let connection = query_history_connection_name(state, entry.connection_id);
            let score =
                query_history_search_score(entry, connection.as_deref(), &this.query_history_quick_search)?;
            Some((score, index, QueryHistoryListEntry { entry: entry.clone() }))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    entries
        .into_iter()
        .take(20)
        .map(|(_, _, entry)| entry)
        .collect()
}

fn query_history_filter_matches(
    entry: &QueryHistoryEntry,
    _state: &AppState,
    this: &NavicatMain,
) -> bool {
    if this
        .query_history_connection_filter
        .is_some_and(|connection| entry.connection_id != connection)
    {
        return false;
    }
    if let Some(database) = &this.query_history_database_filter
        && entry.database.as_deref() != Some(database.as_str())
    {
        return false;
    }
    if let Some(table) = &this.query_history_table_filter
        && !entry.tables.iter().any(|value| value.eq_ignore_ascii_case(table))
    {
        return false;
    }
    query_history_kind_filter_matches(entry, this.query_history_kind_filter)
}

fn query_history_kind_filter_matches(
    entry: &QueryHistoryEntry,
    filter: QueryHistoryKindFilter,
) -> bool {
    match filter {
        QueryHistoryKindFilter::All => {}
        QueryHistoryKindFilter::Failed if entry.success => return false,
        QueryHistoryKindFilter::Failed => {}
        QueryHistoryKindFilter::Query if entry.kind != QueryHistoryKind::Query => return false,
        QueryHistoryKindFilter::DataChange if entry.kind != QueryHistoryKind::DataChange => {
            return false;
        }
        QueryHistoryKindFilter::SchemaChange if entry.kind != QueryHistoryKind::SchemaChange => {
            return false;
        }
        _ => {}
    }
    true
}

fn query_history_search_score(
    entry: &QueryHistoryEntry,
    connection: Option<&str>,
    query: &str,
) -> Option<i32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    let mut score = query_history_field_score(&entry.text, query);
    score = score.max(query_history_field_score(
        entry.database.as_deref().unwrap_or_default(),
        query,
    ));
    score = score.max(query_history_field_score(connection.unwrap_or_default(), query));
    for table in &entry.tables {
        score = score.max(query_history_field_score(table, query) + 8);
    }
    (score > 0).then_some(score)
}

fn query_history_field_score(value: &str, query: &str) -> i32 {
    let value = value.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    if value == query {
        140
    } else if value.starts_with(&query) {
        110
    } else if value.contains(&query) {
        80
    } else if query_history_fuzzy_match(&value, &query) {
        40
    } else {
        0
    }
}

fn query_history_fuzzy_match(value: &str, query: &str) -> bool {
    let mut value_chars = value.chars();
    query
        .chars()
        .all(|query_char| value_chars.any(|value_char| value_char == query_char))
}

fn query_history_kind_filters(
    active: QueryHistoryKindFilter,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let filters = [
        (QueryHistoryKindFilter::All, "全部"),
        (QueryHistoryKindFilter::Query, "查询"),
        (QueryHistoryKindFilter::DataChange, "数据变更"),
        (QueryHistoryKindFilter::SchemaChange, "结构变更"),
        (QueryHistoryKindFilter::Failed, "失败"),
    ];
    div()
        .flex()
        .items_center()
        .gap_1()
        .flex_wrap()
        .children(filters.into_iter().map(|(filter, label)| {
            query_history_chip(label.to_string(), active == filter, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_query_history_kind_filter(filter, cx);
                    cx.stop_propagation();
                }),
            )
        }))
}

fn query_history_quick_kind_filters(
    active: QueryHistoryKindFilter,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let filters = [
        (QueryHistoryKindFilter::All, "全部"),
        (QueryHistoryKindFilter::Query, "查询"),
        (QueryHistoryKindFilter::DataChange, "数据变更"),
        (QueryHistoryKindFilter::SchemaChange, "结构变更"),
        (QueryHistoryKindFilter::Failed, "失败"),
    ];
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(filters.into_iter().map(|(filter, label)| {
            query_history_chip(label.to_string(), active == filter, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_query_history_quick_kind_filter(filter, cx);
                    cx.stop_propagation();
                }),
            )
        }))
}

fn query_history_scope_filters(
    _state: &AppState,
    this: &NavicatMain,
    _colors: UiColors,
    _cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(query_history_select_box(
            "连接 ",
            "全部连接",
            "搜索连接",
            &this.query_history_connection_select,
        ))
        .child(query_history_select_box(
            "库 ",
            "全部库",
            "搜索库",
            &this.query_history_database_select,
        ))
        .child(query_history_select_box(
            "表 ",
            "全部表",
            "搜索表",
            &this.query_history_table_select,
        ))
}

fn query_history_select_box<D>(
    prefix: &'static str,
    placeholder: &'static str,
    search_placeholder: &'static str,
    select: &Entity<SelectState<SearchableVec<D>>>,
) -> Div
where
    D: SelectItem + 'static,
{
    div()
        .h(px(30.))
        .w(px(124.))
        .child(
            Select::new(select)
                .small()
                .title_prefix(prefix)
                .placeholder(placeholder)
                .search_placeholder(search_placeholder)
                .menu_width(px(180.)),
        )
}

fn query_history_chip(label: String, active: bool, colors: UiColors) -> Div {
    div()
        .h(px(24.))
        .flex_none()
        .px_2()
        .rounded_full()
        .border_1()
        .border_color(if active { rgb(0x1687ff) } else { colors.border })
        .bg(if active {
            if colors.is_dark { rgb(0x16324f) } else { rgb(0xe6f2ff) }
        } else if colors.is_dark {
            rgb(0x20232a)
        } else {
            rgb(0xffffff)
        })
        .text_size(px(12.))
        .text_color(if active { rgb(0x1687ff) } else { colors.text })
        .flex()
        .items_center()
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .child(label)
}

fn query_history_row(
    item: QueryHistoryListEntry,
    state: &AppState,
    search: &str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let entry = item.entry;
    let title = query_history_line(&entry.text);
    let subtitle = query_history_scope(&entry, state);
    let status = if entry.success {
        query_history_kind_label(entry.kind)
    } else {
        "失败"
    };
    let status_color = if entry.success { colors.muted } else { rgb(0xd64545) };
    div()
        .min_h(px(70.))
        .rounded(colors.radius_lg)
        .px_2()
        .py_2()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener({
                let entry = entry.clone();
                move |this, _, _, cx| {
                    this.open_query_history_entry(entry.clone(), cx);
                    cx.stop_propagation();
                }
            }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.text)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(query_history_highlighted_text(&title, search, colors)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(subtitle),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(status_color)
                        .child(status),
                ),
        )
        .child(
            query_history_chip("查看".to_string(), false, colors).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.show_query_history_detail(entry.clone(), cx);
                    cx.stop_propagation();
                }),
            ),
        )
}

fn query_history_quick_row(
    item: QueryHistoryListEntry,
    state: &AppState,
    search: &str,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let entry = item.entry;
    let title = query_history_line(&entry.text);
    let mut subtitle = query_history_scope(&entry, state);
    if !entry.tables.is_empty() {
        subtitle.push_str(" / ");
        subtitle.push_str(&entry.tables.join(", "));
    }
    let status = if entry.success {
        query_history_kind_label(entry.kind)
    } else {
        "失败"
    };
    subtitle.push_str(" · ");
    subtitle.push_str(status);
    let selected_bg = if colors.is_dark {
        rgb(0x263244)
    } else {
        rgb(0xe8f2ff)
    };
    let status_color = if entry.success {
        colors.muted
    } else {
        rgb(0xd64545)
    };
    div()
        .h(px(58.))
        .rounded(colors.radius_lg)
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .bg(if selected { selected_bg } else { colors.panel_bg })
        .hover(move |style| style.bg(if selected { selected_bg } else { colors.hover }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener({
                let entry = entry.clone();
                move |this, _, _, cx| {
                    this.query_history_quick_open = false;
                    this.open_query_history_entry(entry.clone(), cx);
                    cx.stop_propagation();
                }
            }),
        )
        .child(
            div()
                .size(px(28.))
                .flex_none()
                .rounded(colors.radius_lg)
                .bg(if colors.is_dark {
                    rgb(0x202a36)
                } else {
                    rgb(0xf0f4f8)
                })
                .flex()
                .items_center()
                .justify_center()
                .child(app_icon(AppIcon::FileSql, 15., colors.muted)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(colors.text)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(query_history_highlighted_text(&title, search, colors)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .line_height(px(15.))
                        .text_color(status_color)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(subtitle),
                ),
        )
}

fn query_history_line(sql: &str) -> String {
    let line = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 110 {
        format!("{}...", line.chars().take(110).collect::<String>())
    } else {
        line
    }
}

fn query_history_scope(entry: &fluxdb_app::QueryHistoryEntry, state: &AppState) -> String {
    let connection = query_history_connection_name(state, entry.connection_id)
        .unwrap_or_else(|| format!("连接 #{}", entry.connection_id.0));
    match &entry.database {
        Some(database) if !database.is_empty() => format!("{connection} / {database}"),
        _ => connection,
    }
}

fn query_history_connection_name(state: &AppState, connection_id: ConnectionId) -> Option<String> {
    state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
        .map(|connection| connection.config.name.clone())
}

fn query_history_connection_filter_items(
    state: &AppState,
) -> Vec<QueryHistoryConnectionFilterItem> {
    let mut items = vec![QueryHistoryConnectionFilterItem {
        label: "全部连接".to_string(),
        value: None,
    }];
    items.extend(state.connections.iter().map(|connection| {
        QueryHistoryConnectionFilterItem {
            label: connection.config.name.clone(),
            value: Some(connection.config.id),
        }
    }));
    items
}

fn query_history_database_filter_items(
    state: &AppState,
    connection_filter: Option<ConnectionId>,
) -> Vec<QueryHistoryTextFilterItem> {
    let mut items = vec![QueryHistoryTextFilterItem {
        label: "全部库".to_string(),
        value: None,
    }];
    items.extend(
        query_history_databases(state, connection_filter)
            .into_iter()
            .map(|database| QueryHistoryTextFilterItem {
                label: database.clone(),
                value: Some(database),
            }),
    );
    items
}

fn query_history_table_filter_items(
    state: &AppState,
    connection_filter: Option<ConnectionId>,
    database_filter: Option<&str>,
) -> Vec<QueryHistoryTextFilterItem> {
    let mut items = vec![QueryHistoryTextFilterItem {
        label: "全部表".to_string(),
        value: None,
    }];
    items.extend(
        query_history_tables_for_scope(state, connection_filter, database_filter)
            .into_iter()
            .take(200)
            .map(|table| QueryHistoryTextFilterItem {
                label: table.clone(),
                value: Some(table),
            }),
    );
    items
}

fn query_history_connection_filter_index(
    items: &[QueryHistoryConnectionFilterItem],
    value: Option<ConnectionId>,
) -> Option<IndexPath> {
    Some(IndexPath::new(
        items
            .iter()
            .position(|item| item.value == value)
            .unwrap_or(0),
    ))
}

fn query_history_text_filter_index(
    items: &[QueryHistoryTextFilterItem],
    value: Option<&str>,
) -> Option<IndexPath> {
    Some(IndexPath::new(
        items
            .iter()
            .position(|item| item.value.as_deref() == value)
            .unwrap_or(0),
    ))
}

fn query_history_databases(
    state: &AppState,
    connection_filter: Option<ConnectionId>,
) -> Vec<String> {
    let mut databases = state
        .query_history
        .iter()
        .filter(|entry| {
            connection_filter.is_none_or(|connection_id| entry.connection_id == connection_id)
        })
        .filter_map(|entry| entry.database.clone())
        .filter(|database| !database.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    databases.sort_by_key(|database| database.to_ascii_lowercase());
    databases
}

fn query_history_tables_for_scope(
    state: &AppState,
    connection_filter: Option<ConnectionId>,
    database_filter: Option<&str>,
) -> Vec<String> {
    let mut tables = state
        .query_history
        .iter()
        .filter(|entry| {
            connection_filter
                .is_none_or(|connection_id| entry.connection_id == connection_id)
        })
        .filter(|entry| {
            database_filter
                .is_none_or(|database| entry.database.as_deref() == Some(database))
        })
        .flat_map(|entry| entry.tables.iter().cloned())
        .filter(|table| !table.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    tables.sort_by_key(|table| table.to_ascii_lowercase());
    tables
}

fn query_history_kind_label(kind: QueryHistoryKind) -> &'static str {
    match kind {
        QueryHistoryKind::Query => "查询",
        QueryHistoryKind::DataChange => "数据变更",
        QueryHistoryKind::SchemaChange => "结构变更",
    }
}

fn query_history_highlighted_text(text: &str, query: &str, colors: UiColors) -> Div {
    let mut label = div()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(colors.text);
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return label.child(text.to_string());
    }

    let lower_text = text.to_ascii_lowercase();
    let Some(first_match) = lower_text.find(&query) else {
        return label.child(text.to_string());
    };
    let highlight_bg = if colors.is_dark {
        rgb(0x5b4513)
    } else {
        rgb(0xffe7a3)
    };
    let highlight_text = if colors.is_dark {
        rgb(0xffd166)
    } else {
        rgb(0x8a4b00)
    };
    let mut cursor = 0;
    let mut search_from = first_match;
    while let Some(relative_start) = lower_text[search_from..].find(&query) {
        let start = search_from + relative_start;
        let end = start + query.len();
        if start > cursor {
            label = label.child(text[cursor..start].to_string());
        }
        label = label.child(
            div()
                .rounded(colors.radius * 0.5)
                .px(px(1.5))
                .bg(highlight_bg)
                .text_color(highlight_text)
                .child(text[start..end].to_string()),
        );
        cursor = end;
        search_from = end;
    }
    if cursor < text.len() {
        label = label.child(text[cursor..].to_string());
    }
    label
}

fn query_history_detail_modal(
    entry: QueryHistoryEntry,
    state: &AppState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let scope = query_history_scope(&entry, state);
    let kind = query_history_kind_label(entry.kind);
    let tables = if entry.tables.is_empty() {
        "无".to_string()
    } else {
        entry.tables.join(", ")
    };
    let status = if entry.success { "成功" } else { "失败" };
    let status_color = if entry.success {
        rgb(0x22a06b)
    } else {
        rgb(0xd64545)
    };
    let title = entry
        .object
        .clone()
        .filter(|object| !object.trim().is_empty())
        .unwrap_or_else(|| query_history_line(&entry.text));
    let operation = query_history_operation_label(&entry.text);
    let executed_at = query_history_time_label(entry.executed_at_unix_secs);
    let elapsed = format!("{} ms", entry.summary.elapsed_ms);
    let affected = entry.summary.affected_rows.to_string();
    let returned = entry.summary.returned_rows.to_string();
    let rollback_status = if entry.rollback_sql().is_some() {
        "可回滚"
    } else if entry.rollback_snapshot.is_some() {
        "有快照，无法生成 SQL"
    } else {
        "无原始快照"
    };
    let sql_for_copy = entry.text.clone();
    let rollback_for_copy = entry.rollback_sql();
    let rollback_entry = entry.rollback_sql().map(|rollback_sql| {
        let mut entry = entry.clone();
        entry.text = rollback_sql.clone();
        entry.summary.sql = rollback_sql;
        entry.rollback_snapshot = None;
        entry
    });
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.02, 0.48)
        } else {
            opaque_grey(0.75, 0.24)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.query_history_detail = None;
            cx.notify();
            cx.stop_propagation();
        }))
        .child(
            div()
                .w(px(680.))
                .max_w(px(680.))
                .max_h(px(560.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_bg)
                .shadow(vec![box_shadow(
                    px(0.),
                    px(12.),
                    px(32.),
                    px(0.),
                    hsla(0., 0., 0., 0.22),
                )])
                .flex()
                .flex_col()
                .overflow_hidden()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .min_h(px(58.))
                        .px_4()
                        .border_b_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(colors.text)
                                .child(title),
                        )
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(colors.radius_lg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |style| style.bg(colors.hover))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.query_history_detail = None;
                                        cx.notify();
                                        cx.stop_propagation();
                                    }),
                                )
                                .child(app_icon(AppIcon::Close, 16., colors.muted)),
                        ),
                )
                .child(
                    div()
                        .px_4()
                        .py_3()
                        .border_b_1()
                        .border_color(colors.border)
                        .grid()
                        .grid_cols(3)
                        .gap_2()
                        .text_size(px(12.))
                        .child(query_history_detail_meta("类型", kind.to_string(), colors))
                        .child(query_history_detail_meta("操作", operation, colors))
                        .child(
                            query_history_detail_meta("状态", status.to_string(), colors)
                                .text_color(status_color),
                        )
                        .child(query_history_detail_meta("连接", scope, colors))
                        .child(query_history_detail_meta(
                            "数据库",
                            entry.database.clone().unwrap_or_else(|| "默认库".to_string()),
                            colors,
                        ))
                        .child(query_history_detail_meta(
                            "对象",
                            entry.object.clone().unwrap_or_else(|| tables.clone()),
                            colors,
                        ))
                        .child(query_history_detail_meta("时间", executed_at, colors))
                        .child(query_history_detail_meta("耗时", elapsed, colors))
                        .child(query_history_detail_meta(
                            "影响/返回",
                            format!("{affected} / {returned}"),
                            colors,
                        ))
                        .child(query_history_detail_meta(
                            "回滚",
                            rollback_status.to_string(),
                            colors,
                        ))
                        .child(query_history_detail_meta(
                            "事务",
                            entry.transaction_state.label().to_string(),
                            colors,
                        ))
                        .when(!entry.success, |this| {
                            this.child(query_history_detail_meta(
                                "错误",
                                entry.summary.message.clone(),
                                colors,
                            ))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_y_scrollbar()
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(query_history_sql_block(
                            "执行 SQL",
                            entry.text.clone(),
                            "复制 SQL",
                            colors,
                            {
                                let sql_for_copy = sql_for_copy.clone();
                                move |this: &mut NavicatMain, cx: &mut Context<NavicatMain>| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        sql_for_copy.clone(),
                                    ));
                                    this.show_message("已复制 SQL", AppMessageKind::Success, cx);
                                }
                            },
                            cx,
                        ))
                        .when_some(entry.rollback_snapshot_summary(), |this, snapshot| {
                            this.child(query_history_sql_block(
                                "原始行快照",
                                snapshot,
                                "复制快照",
                                colors,
                                move |this: &mut NavicatMain, cx: &mut Context<NavicatMain>| {
                                    if let Some(sql) = rollback_for_copy.clone() {
                                        cx.write_to_clipboard(ClipboardItem::new_string(sql));
                                        this.show_message(
                                            "已复制动态回滚 SQL",
                                            AppMessageKind::Success,
                                            cx,
                                        );
                                    } else {
                                        this.show_message(
                                            "当前快照无法生成回滚 SQL",
                                            AppMessageKind::Warning,
                                            cx,
                                        );
                                    }
                                },
                                cx,
                            ))
                        }),
                )
                .child(
                    div()
                        .h(px(56.))
                        .px_4()
                        .border_t_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(query_history_detail_button(
                            "AI 分析",
                            AppIcon::Wand,
                            false,
                            colors,
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    query_history_detail_button(
                                        "恢复到编辑器",
                                        AppIcon::FileSql,
                                        true,
                                        colors,
                                    )
                                    .on_mouse_down(MouseButton::Left, {
                                        let entry = entry.clone();
                                        cx.listener(move |this, _, _, cx| {
                                            this.query_history_detail = None;
                                            this.open_query_history_entry(entry.clone(), cx);
                                            this.show_message(
                                                "已恢复到编辑器",
                                                AppMessageKind::Success,
                                                cx,
                                            );
                                            cx.stop_propagation();
                                        })
                                    }),
                                )
                                .when_some(rollback_entry, |this, rollback_entry| {
                                    this.child(
                                        query_history_detail_button(
                                            "回滚",
                                            AppIcon::Undo,
                                            true,
                                            colors,
                                        )
                                        .on_mouse_down(MouseButton::Left, cx.listener(
                                            move |this, _, _, cx| {
                                                this.query_history_detail = None;
                                                this.open_query_history_entry(
                                                    rollback_entry.clone(),
                                                    cx,
                                                );
                                                this.show_message(
                                                    "已打开回滚 SQL",
                                                    AppMessageKind::Success,
                                                    cx,
                                                );
                                                cx.stop_propagation();
                                            },
                                        )),
                                    )
                                }),
                        ),
                ),
        )
}

fn query_history_detail_meta(label: &str, value: String, colors: UiColors) -> Div {
    div()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.text)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(value),
        )
}

fn query_history_sql_block<F>(
    title: &str,
    sql: String,
    copy_label: &'static str,
    colors: UiColors,
    on_copy: F,
    cx: &mut Context<NavicatMain>,
) -> Div
where
    F: Fn(&mut NavicatMain, &mut Context<NavicatMain>) + 'static,
{
    div()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .overflow_hidden()
        .child(
            div()
                .h(px(36.))
                .px_3()
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(colors.text)
                        .child(title.to_string()),
                )
                .child(
                    query_history_detail_button(copy_label, AppIcon::Copy, true, colors)
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            on_copy(this, cx);
                            cx.stop_propagation();
                        })),
                ),
        )
        .child(
            div()
                .p_3()
                .font_family(EDITOR_FONT)
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(colors.text)
                .bg(if colors.is_dark {
                    rgb(0x121417)
                } else {
                    rgb(0xf7f8fa)
                })
                .child(sql),
        )
}

fn query_history_detail_button(
    label: &'static str,
    icon: AppIcon,
    enabled: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(30.))
        .px_3()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(if enabled { colors.panel_bg } else { colors.panel_alt })
        .text_size(px(12.))
        .text_color(if enabled { colors.text } else { colors.muted })
        .flex()
        .items_center()
        .gap_1()
        .when(enabled, |this| this.cursor_pointer())
        .when(enabled, |this| this.hover(move |style| style.bg(colors.hover)))
        .child(app_icon(icon, 14., if enabled { colors.text } else { colors.muted }))
        .child(label)
}

fn query_history_operation_label(sql: &str) -> String {
    sql.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .find(|token| !token.is_empty())
        .map(|keyword| keyword.to_ascii_uppercase())
        .unwrap_or_else(|| "SQL".to_string())
}

fn query_history_time_label(secs: u64) -> String {
    if secs == 0 {
        return "未知".to_string();
    }
    Local
        .timestamp_opt(secs as i64, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "未知".to_string())
}

