/// 顶部栏图标按钮：统一尺寸/圆角/手形光标/hover/tooltip，直接以 AppIcon 渲染。
/// 「仅图标」按钮的通用封装，用于顶部栏收起侧边栏 / 首页等无标签按钮。
fn topbar_icon_button(
    icon: AppIcon,
    tooltip: &'static str,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .size(px(30.))
        .ml(px(2.))
        .rounded(colors.radius_lg)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .id(tooltip)
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(app_icon_box(icon, 30., 17., colors.muted))
}

/// 顶部栏连接信息文本：超长省略 + 悬停 tooltip 展示完整连接名。
fn connection_info_label(name: String, colors: UiColors) -> Stateful<Div> {
    let tooltip_name = name.clone();
    div()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(13.))
        .text_color(colors.text)
        .id("topbar-connection-name")
        .tooltip(move |window, cx| Tooltip::new(tooltip_name.clone()).build(window, cx))
        .child(name)
}

/// 顶部栏：只保留原生窗口控制（红绿灯预留区）、侧边栏收起、首页与当前连接信息。
/// 展开/收起两态内容不同：
/// - 展开（show_connection_browser）：红绿灯 + 收起侧边栏 + 首页 + 连接信息。
/// - 收起：红绿灯 + 连接信息（收起/首页按钮移入收起侧边栏条）。
fn topbar(
    state: &AppState,
    show_connection_browser: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    // 当前活动标签所属连接名；无活动标签或 Settings 标签（无连接）时不展示。
    let connection_info = state.active_tab().and_then(|tab| {
        tab_workspace_scope(tab).map(|scope| connection_name(state, scope.connection_id))
    });

    div()
        .h(px(36.))
        .bg(colors.app_bg)
        .border_b_1()
        .border_color(colors.border)
        .flex()
        .items_center()
        .gap_1()
        // 保留 macOS 原生红绿灯（关闭/最小化/全屏）预留区。
        .pl(px(84.))
        .pr_3()
        // 侧边栏展开/收起切换按钮（图标随状态变化）+ 首页按钮，两态都保留在顶部栏。
        .child(
            topbar_icon_button(
                if show_connection_browser {
                    AppIcon::PanelLeftClose
                } else {
                    AppIcon::PanelLeftOpen
                },
                if show_connection_browser {
                    "收起侧边栏"
                } else {
                    "展开侧边栏"
                },
                colors,
            )
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.show_connection_browser = !this.show_connection_browser;
                cx.stop_propagation();
                cx.notify();
            })),
        )
        .child(topbar_icon_button(AppIcon::Home, "首页", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.dispatch(AppCommand::DeactivateTab, cx);
                cx.stop_propagation();
            }),
        ))
        .when_some(connection_info, |this, name| {
            // 当前打开的数据库连接信息（文本），与活动表/库保持关联。
            this.child(
                div()
                    .flex_none()
                    .max_w(px(300.))
                    .h(px(28.))
                    .px_3()
                    .rounded(colors.radius_lg)
                    .bg(colors.panel_bg)
                    .border_1()
                    .border_color(colors.border)
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(app_icon_box(AppIcon::Database, 26., 15., colors.muted))
                    .child(connection_info_label(name, colors)),
            )
        })
        .child(
            div()
                .flex_1()
                .h_full()
                .on_mouse_down(MouseButton::Left, |event, window, cx| {
                    if event.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                    cx.stop_propagation();
                }),
        )
        // 顶部栏最右侧：历史 + 设置（业务按钮移除后收纳在此，保持功能可达）。
        .child(topbar_icon_button(AppIcon::CalendarClock, "历史", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.toggle_history(window, cx);
                cx.stop_propagation();
            }),
        ))
        .child(topbar_icon_button(AppIcon::Settings, "设置", colors).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.dispatch(AppCommand::OpenSettings, cx);
                cx.stop_propagation();
            }),
        ))
}

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

fn new_connection_modal(
    kind: DatabaseKind,
    active_tab: NewConnectionTab,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    editing: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let view = cx.entity();
    let title = if editing {
        "编辑连接"
    } else {
        "新建连接"
    };

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if colors.is_dark {
            opaque_grey(0.08, 0.62)
        } else {
            opaque_grey(0.6, 0.36)
        })
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            div()
                .relative()
                .w(px(820.))
                .h(px(560.))
                .rounded(colors.radius_lg)
                .bg(colors.panel_bg)
                .border_1()
                .border_color(colors.border)
                .flex()
                .flex_col()
                .overflow_hidden()
                .text_color(colors.text)
                .key_context("NewConnectionModal")
                .on_action(cx.listener(|this, _: &CancelDialog, _, cx| {
                    this.cancel_new_connection(cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div().absolute().top(px(12.)).right(px(14.)).child(
                        Button::new("new-connection-close")
                            .label("×")
                            .ghost()
                            .w(px(34.))
                            .h(px(34.))
                            .text_size(px(24.))
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        this.cancel_new_connection(cx);
                                    });
                                    cx.stop_propagation();
                                }
                            }),
                    ),
                )
                .child(
                    div()
                        .h(px(62.))
                        .px_5()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(div().w(px(34.))),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .gap_5()
                        .px_5()
                        .pb_2()
                        .overflow_hidden()
                        .child(
                            div()
                                .w(px(350.))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    div()
                                        .h(px(34.))
                                        .rounded(colors.radius_lg)
                                        .border_1()
                                        .border_color(colors.border)
                                        .bg(colors.input_bg)
                                        .px_3()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_size(px(14.))
                                        .text_color(colors.muted)
                                        .child(app_icon(AppIcon::Search, 14., colors.muted))
                                        .child("搜索数据库类型"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_3()
                                        .child(kind_tile(
                                            "MySQL",
                                            DatabaseKind::MySql,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "TiDB",
                                            DatabaseKind::TiDb,
                                            kind,
                                            true,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "SQLite",
                                            DatabaseKind::Sqlite,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_3()
                                        .child(kind_tile(
                                            "Redis",
                                            DatabaseKind::Redis,
                                            kind,
                                            editing,
                                            colors,
                                            cx,
                                        ))
                                        .child(kind_tile(
                                            "MongoDB",
                                            DatabaseKind::MongoDb,
                                            kind,
                                            true,
                                            colors,
                                            cx,
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .overflow_hidden()
                                .child(connection_tab_bar(active_tab, colors, cx))
                                .child(div().h(px(1.)).bg(colors.border))
                                .child(
                                    div()
                                        .id("new-connection-form-scroll")
                                        .flex_1()
                                        .min_h(px(0.))
                                        .overflow_scroll()
                                        .scrollbar_width(px(8.))
                                        .child(new_connection_tab_content(
                                            active_tab, kind, form, inputs, colors, window, cx,
                                        )),
                                ),
                        ),
                )
                .child(
                    div()
                        .h(px(58.))
                        .flex_none()
                        .flex()
                        .items_end()
                        .gap_3()
                        .px_5()
                        .pb_5()
                        .when_some(form.test_status.as_ref(), |this, status| {
                            this.child(
                                connection_test_status(status)
                                    .flex_1()
                                    .min_w(px(0.))
                                    .justify_start(),
                            )
                        })
                        .when_none(&form.test_status, |this| {
                            this.child(div().flex_1().min_w(px(0.)))
                        })
                        .child(Button::new("new-connection-test").label("测试").on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.test_new_connection(cx);
                                });
                                cx.stop_propagation();
                            }
                        }))
                        .child(
                            Button::new("new-connection-save")
                                .label("保存并连接")
                                .primary()
                                .w(px(116.))
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.create_connection_from_form(cx);
                                        });
                                        cx.stop_propagation();
                                    }
                                }),
                        ),
                ),
        )
}

fn kind_tile(
    label: &'static str,
    kind: DatabaseKind,
    selected: DatabaseKind,
    locked: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    let selected = kind == selected;
    let disabled = locked && !selected;

    div()
        .w(px(168.))
        .h(px(112.))
        .rounded(colors.radius_lg)
        .bg(if disabled {
            if colors.is_dark {
                rgb(0x1b1e23)
            } else {
                rgb(0xf1f2f4)
            }
        } else if selected {
            if colors.is_dark {
                rgb(0x1a3157)
            } else {
                rgb(0xe9f1ff)
            }
        } else {
            colors.panel_alt
        })
        .border_1()
        .border_color(if disabled {
            if colors.is_dark {
                rgb(0x2b3038)
            } else {
                rgb(0xd6dae0)
            }
        } else if selected {
            rgb(0x5b8def)
        } else {
            colors.border
        })
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .when(!locked, |this| {
            this.hover(move |style| {
                style
                    .bg(if selected {
                        if colors.is_dark {
                            rgb(0x213c66)
                        } else {
                            rgb(0xe3edff)
                        }
                    } else {
                        colors.hover
                    })
                    .border_color(if selected {
                        rgb(0x3478f6)
                    } else {
                        rgb(0xb9c0ca)
                    })
            })
        })
        .child(
            div()
                .size(px(48.))
                .rounded(colors.radius_lg)
                .bg(if disabled {
                    if colors.is_dark {
                        rgb(0x24272c)
                    } else {
                        rgb(0xe3e5e8)
                    }
                } else if selected {
                    rgb(0x24272d)
                } else {
                    rgb(0x2a2d33)
                })
                .flex()
                .items_center()
                .justify_center()
                .child(database_kind_icon(kind)),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if disabled {
                    if colors.is_dark {
                        rgb(0x6f7783)
                    } else {
                        rgb(0x9aa1ac)
                    }
                } else if selected {
                    if colors.is_dark {
                        rgb(0x9cc2ff)
                    } else {
                        rgb(0x1d4f91)
                    }
                } else {
                    colors.text
                })
                .child(label),
        )
        .when(!locked, |this| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.set_new_connection_kind(kind, window, cx);
                    cx.stop_propagation();
                }),
            )
        })
}

fn database_kind_icon(kind: DatabaseKind) -> impl IntoElement {
    img(database_kind_icon_path(kind)).size(px(32.))
}

fn database_kind_icon_path(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "db/mysql.svg",
        DatabaseKind::TiDb => "db/tidb.svg",
        DatabaseKind::Sqlite => "db/sqlite.svg",
        DatabaseKind::MongoDb => "db/mongodb.svg",
        DatabaseKind::Redis => "db/redis.svg",
    }
}

/// 新建连接表单顶部分页栏：用 gpui-component 分段 TabBar 渲染（与「连接信息」表单统一样式）。
fn connection_tab_bar(
    active_tab: NewConnectionTab,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    const TABS: [(NewConnectionTab, &str); 4] = [
        (NewConnectionTab::Connection, "连接信息"),
        (NewConnectionTab::Tls, "TLS/SSL"),
        (NewConnectionTab::Ssh, "SSH 隧道/代理"),
        (NewConnectionTab::Advanced, "高级"),
    ];
    let _ = colors;
    let selected_index = TABS
        .iter()
        .position(|(tab, _)| *tab == active_tab)
        .unwrap_or(0);
    let view = cx.entity();
    TABS.iter()
        .map(|(_, label)| Tab::from(*label))
        .fold(
            TabBar::new("new-connection-tabs")
                .segmented()
                .small()
                .selected_index(selected_index)
                .on_click(move |index, _, cx| {
                    if let Some((tab, _)) = TABS.get(*index) {
                        let _ = view.update(cx, |this, cx| {
                            this.set_new_connection_tab(*tab, cx);
                        });
                    }
                }),
            |bar, tab| bar.child(tab),
        )
        .into_element()
}

fn new_connection_tab_content(
    tab: NewConnectionTab,
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    match tab {
        NewConnectionTab::Connection => connection_form(kind, form, inputs, colors, window, cx),
        // TLS / SSH / Advanced：Redis 与 MySQL/TiDB 都有实际表单，其余类型显示提示占位。
        NewConnectionTab::Tls => match kind {
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::Redis => {
                tls_form(kind, form, inputs, colors, window, cx)
            }
            _ => redis_only_settings_hint(colors),
        },
        NewConnectionTab::Ssh => match kind {
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::Redis => {
                ssh_form(kind, form, inputs, colors, window, cx)
            }
            _ => redis_only_settings_hint(colors),
        },
        NewConnectionTab::Advanced => match kind {
            DatabaseKind::MySql | DatabaseKind::TiDb => mysql_advanced_form(form, inputs, colors, window, cx),
            DatabaseKind::Redis => advanced_form(form, inputs, colors, window, cx),
            _ => redis_only_settings_hint(colors),
        },
    }
}

/// 非 Redis 连接在 TLS/SSH/高级页签下的提示占位。
fn redis_only_settings_hint(colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .w_full()
                .h(px(120.))
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .bg(colors.panel_alt)
                .flex()
                .items_center()
                .justify_center()
                .px_4()
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("该设置仅对 Redis 连接可用"),
        )
}

/// TLS 页签：启用开关 + 证书/私钥文件路径 + SNI + 校验证书。
fn tls_form(
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let enabled = form.tls_enabled;
    let is_mysql = matches!(kind, DatabaseKind::MySql | DatabaseKind::TiDb);
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("传输层安全 (TLS)", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    "启用 TLS",
                    ConnectionToggleField::TlsEnabled,
                    form.tls_enabled,
                    colors,
                    cx,
                )),
        )
        // TLS 参数明细：启用时正常显示，未启用时整体置灰。
        .when(enabled, |this| {
            this.child(tls_parameters_block(
                is_mysql, form, inputs, colors, window, cx,
            ))
        })
        .when(!enabled, |this| {
            this.child(tls_parameters_block(
                is_mysql, form, inputs, colors, window, cx,
            )
            .opacity(0.45))
        })
}

/// TLS 参数明细块：CA 证书 / 客户端证书 / 客户端密钥 / SNI / 校验证书。
/// MySQL/TiDB 额外渲染 ssl_mode 与字符集。
fn tls_parameters_block(
    is_mysql: bool,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .when(is_mysql, |this| {
            this.child(mysql_tls_mode_block(form, inputs, colors, window, cx))
        })
        .child(
            h_form()
                .label_width(px(112.))
                .child(file_field_row_light(
                    "CA 证书",
                    ConnectionField::TlsCa,
                    "选择 TLS CA 证书文件",
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(file_field_row_light(
                    "客户端证书",
                    ConnectionField::TlsClientCert,
                    "选择 TLS 客户端证书文件",
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(file_field_row_light(
                    "客户端密钥",
                    ConnectionField::TlsClientKey,
                    "选择 TLS 客户端私钥文件",
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "SNI / 主机名",
                    ConnectionField::TlsSni,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(toggle_row_light(
                    "校验服务器证书",
                    ConnectionToggleField::TlsVerify,
                    form.tls_verify,
                    colors,
                    cx,
                )),
        )
}

/// MySQL/TiDB TLS 模式块：SSL 模式 + 连接字符集。
fn mysql_tls_mode_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "SSL 模式",
                    ConnectionField::MysqlTlsSslMode,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "连接字符集",
                    ConnectionField::MysqlCharset,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// SSH 页签：启用开关 + 跳板机参数 + 认证方式。
fn ssh_form(
    kind: DatabaseKind,
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 认证方式："password" 显示/可用密码，私钥方式显示/可用私钥与口令。
    let password_mode = form.ssh_auth != "private_key";
    let enabled = form.ssh_enabled;
    let is_mysql = matches!(kind, DatabaseKind::MySql | DatabaseKind::TiDb);
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("SSH 隧道", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    "启用 SSH 隧道",
                    ConnectionToggleField::SshEnabled,
                    form.ssh_enabled,
                    colors,
                    cx,
                )),
        )
        // 隧道参数：启用时正常显示，未启用时整体置灰。
        .when(enabled, |this| {
            this.child(ssh_tunnel_block(form, inputs, colors, window, cx))
        })
        .when(!enabled, |this| {
            this.child(ssh_tunnel_block(form, inputs, colors, window, cx).opacity(0.45))
        })
        // MySQL/TiDB 额外：SSH 连接超时 + 心跳间隔。
        .when(is_mysql, |this| {
            this.child(mysql_ssh_tuning_block(inputs, colors, window, cx))
        })
        // 密码认证区块：私钥模式时置灰。
        .when(password_mode, |this| {
            this.child(ssh_password_block(inputs, colors, window, cx))
        })
        .when(!password_mode, |this| {
            this.child(ssh_password_block(inputs, colors, window, cx).opacity(0.45))
        })
        // 私钥认证区块：密码模式时置灰。
        .when(password_mode, |this| {
            this.child(ssh_private_key_block(inputs, colors, window, cx).opacity(0.45))
        })
        .when(!password_mode, |this| {
            this.child(ssh_private_key_block(inputs, colors, window, cx))
        })
}

/// SSH 隧道参数块：主机 / 端口 / 用户名 / 认证方式。
fn ssh_tunnel_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "主机",
                    ConnectionField::SshHost,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "端口",
                    ConnectionField::SshPort,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "用户名",
                    ConnectionField::SshUsername,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(connection_select_field(
                    "认证方式",
                    &inputs.ssh_auth_select,
                    "",
                )),
        )
}

/// SSH 密码认证块。
fn ssh_password_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "密码",
                    ConnectionField::SshPassword,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// SSH 私钥认证块：私钥文件 + 口令。
fn ssh_private_key_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(file_field_row_light(
                    "私钥文件",
                    ConnectionField::SshPrivateKey,
                    "选择 SSH 私钥文件",
                    inputs,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "口令 (passphrase)",
                    ConnectionField::SshPassphrase,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// MySQL/TiDB SSH 调优块：连接超时 + 心跳间隔。
fn mysql_ssh_tuning_block(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("SSH 调优", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "连接超时 (秒, 0=继承)",
                    ConnectionField::MysqlSshConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "心跳间隔 (秒, 0=不发送)",
                    ConnectionField::MysqlSshKeepalive,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// MySQL/TiDB 高级页签：代理 + 连接/查询/空闲 TTL 超时 + TCP 保活。
fn mysql_advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let proxy_enabled = form.mysql_proxy_enabled;
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("代理", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    "启用代理",
                    ConnectionToggleField::MysqlProxyEnabled,
                    form.mysql_proxy_enabled,
                    colors,
                    cx,
                )),
        )
        // 代理参数：启用时正常显示，未启用时整体置灰。
        .when(proxy_enabled, |this| {
            this.child(mysql_proxy_block(form, inputs, colors, window, cx))
        })
        .when(!proxy_enabled, |this| {
            this.child(mysql_proxy_block(form, inputs, colors, window, cx).opacity(0.45))
        })
        .child(redis_section_label("高级连接选项", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "建连超时 (秒)",
                    ConnectionField::MysqlConnectTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "查询超时 (秒, 0=不设限)",
                    ConnectionField::MysqlQueryTimeout,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "空闲 TTL (秒, 0=不回收)",
                    ConnectionField::MysqlIdleTtl,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(
            div()
                .pl_1()
                .text_size(px(12.))
                .text_color(colors.muted)
                .child("待连接复用启用后生效"),
        )
        .child(
            h_form()
                .label_width(px(112.))
                .child(toggle_row_light(
                    "TCP 长连接保活",
                    ConnectionToggleField::MysqlTcpKeepalive,
                    form.mysql_tcp_keepalive,
                    colors,
                    cx,
                )),
        )
}

/// MySQL/TiDB 代理参数块：类型 / 主机 / 端口 / 用户名 / 密码。
fn mysql_proxy_block(
    _form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 与「连接信息」表单一致：label 在前、输入框在后（h_form 定宽对齐）。
    div()
        .w_full()
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "代理类型",
                    ConnectionField::MysqlProxyType,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "主机",
                    ConnectionField::MysqlProxyHost,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "端口",
                    ConnectionField::MysqlProxyPort,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "用户名 (可选)",
                    ConnectionField::MysqlProxyUsername,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "密码 (可选)",
                    ConnectionField::MysqlProxyPassword,
                    inputs,
                    true,
                    colors,
                    window,
                    cx,
                )),
        )
}

/// 高级页签：Sentinel / Cluster / 云自动发现 / 连接串导入。
fn advanced_form(
    form: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(redis_section_label("Sentinel 模式", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "主库名",
                    ConnectionField::SentinelMasterName,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "节点列表 (host:port, 逗号或换行分隔)",
                    ConnectionField::SentinelEndpoints,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(redis_section_label("Cluster 模式", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(field_row_light(
                    "起始节点 (host:port, 逗号或换行分隔)",
                    ConnectionField::ClusterStartNodes,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(toggle_row_light(
                    "允许重定向到从节点",
                    ConnectionToggleField::ClusterAllowReadonly,
                    form.cluster_allow_readonly,
                    colors,
                    cx,
                )),
        )
        .child(redis_section_label("云自动发现", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(connection_select_field(
                    "云提供方",
                    &inputs.cloud_provider_select,
                    "",
                ))
                .child(field_row_light(
                    "订阅 / 账号",
                    ConnectionField::CloudSubscription,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                ))
                .child(field_row_light(
                    "资源 / 数据库",
                    ConnectionField::CloudResource,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )),
        )
        .child(redis_section_label("连接串导入", colors))
        .child(
            h_form()
                .label_width(px(112.))
                .child(discovery_uri_row(inputs, colors, window, cx)),
        )
}

/// 连接串导入行：URI 输入框 + 「导入/发现」按钮。
fn discovery_uri_row(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().label("连接串 / URI").items_center().child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                input_box_light(ConnectionField::DiscoveryUri, inputs, false, colors, window, cx)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(
                div()
                    .h(px(34.))
                    .flex_none()
                    .px_3()
                    .rounded(colors.radius_lg)
                    .border_1()
                    .border_color(rgb(0x1687ff))
                    .bg(if colors.is_dark { rgb(0x16324f) } else { rgb(0xe6f2ff) })
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(0x1687ff))
                    .cursor_pointer()
                    .hover(move |style| style.bg(colors.hover))
                    .child("导入 / 发现")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.import_redis_connection_string(cx);
                            cx.stop_propagation();
                        }),
                    ),
            ),
    )
}

/// 小节标题。
fn redis_section_label(text: &str, colors: UiColors) -> Div {
    div()
        .pt_1()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(text.to_string())
}

/// 新建连接弹框里的下拉行：用 gpui-component Select 渲染，绑定根实体上的 SelectState 光标。
fn connection_select_field(
    label: &'static str,
    select: &Entity<SelectState<SearchableVec<String>>>,
    search_placeholder: &'static str,
) -> Field {
    field()
        .label(label)
        .items_center()
        .child(Select::new(select).small().search_placeholder(search_placeholder))
}

/// 布尔开关行：把 Checkbox 的点击写回表单对应开关字段。
fn toggle_row_light(
    label: &'static str,
    field_id: ConnectionToggleField,
    checked: bool,
    _colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Field {
    let view = cx.entity();
    let id = connection_toggle_id(field_id);
    field().label(label).items_center().child(
        Checkbox::new(id)
            .checked(checked)
            .on_click(move |new_checked, _, cx| {
                let field_id = field_id;
                let new_checked = *new_checked;
                let _ = view.update(cx, |this, cx| {
                    this.set_connection_toggle_field(field_id, new_checked, cx);
                });
            }),
    )
}

/// 文件选择行：文件路径输入 + 「选择文件」按钮。
fn file_field_row_light(
    label: &'static str,
    field_id: ConnectionField,
    prompt: &'static str,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().label(label).items_center().child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                input_box_light(field_id, inputs, false, colors, window, cx)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(file_picker_button_light(field_id, prompt, colors, cx).flex_none()),
    )
}

/// 「选择文件」按钮：调用对应字段的通用文件选择器。
fn file_picker_button_light(
    field_id: ConnectionField,
    prompt: &'static str,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .size(px(38.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover).border_color(rgb(0xaeb7c2)))
        .child(
            div()
                .relative()
                .w(px(18.))
                .h(px(14.))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(2.))
                        .w(px(8.))
                        .h(px(4.))
                        .rounded(colors.radius * 0.5)
                        .bg(colors.muted),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w(px(18.))
                        .h(px(12.))
                        .rounded(colors.radius * 0.5)
                        .border_2()
                        .border_color(colors.muted)
                        .bg(colors.input_bg),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                let prompt = prompt;
                this.choose_connection_file(field_id, prompt, window, cx);
                cx.stop_propagation();
            }),
        )
}

/// 布尔开关的稳定元素 ID。
fn connection_toggle_id(field: ConnectionToggleField) -> &'static str {
    match field {
        ConnectionToggleField::TlsEnabled => "new-connection-toggle-tls",
        ConnectionToggleField::TlsVerify => "new-connection-toggle-tls-verify",
        ConnectionToggleField::SshEnabled => "new-connection-toggle-ssh",
        ConnectionToggleField::ClusterAllowReadonly => "new-connection-toggle-cluster-readonly",
        ConnectionToggleField::MysqlProxyEnabled => "new-connection-toggle-mysql-proxy",
        ConnectionToggleField::MysqlTcpKeepalive => "new-connection-toggle-mysql-tcp-keepalive",
    }
}

fn connection_form(
    kind: DatabaseKind,
    form_state: &NewConnectionForm,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let base = h_form()
        .label_width(px(112.))
        .child(field_row_light(
            "名称",
            ConnectionField::Name,
            inputs,
            false,
            colors,
            window,
            cx,
        ))
        .child(color_row_light(&form_state.color, colors, cx));

    let form = match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => base
            .child(host_port_row_light("主机", inputs, colors, window, cx))
            .child(field_row_light(
                "用户名",
                ConnectionField::Username,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "密码",
                ConnectionField::Password,
                inputs,
                true,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "数据库",
                ConnectionField::Database,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "URL 参数",
                ConnectionField::UrlParams,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        DatabaseKind::MongoDb => base
            .child(connection_method_row_light(
                "连接方式",
                "表单",
                "URL",
                colors,
            ))
            .child(host_port_row_light("主机", inputs, colors, window, cx))
            .child(checkbox_row_light("SRV (MongoDB Atlas)", colors))
            .child(field_row_light(
                "用户名",
                ConnectionField::Username,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "密码",
                ConnectionField::Password,
                inputs,
                true,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "默认库",
                ConnectionField::MongoDefaultDb,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "认证库",
                ConnectionField::MongoAuthDb,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(dropdown_row_light("认证机制", "默认", colors)),
        DatabaseKind::Redis => base
            .child(host_port_row_light("主机", inputs, colors, window, cx))
            .child(field_row_light(
                "用户名",
                ConnectionField::Username,
                inputs,
                false,
                colors,
                window,
                cx,
            ))
            .child(field_row_light(
                "密码",
                ConnectionField::Password,
                inputs,
                true,
                colors,
                window,
                cx,
            ))
            // Redis 没有连接串，TLS / Sentinel 这些开关走同一个参数输入框
            .child(field_row_light(
                "参数",
                ConnectionField::UrlParams,
                inputs,
                false,
                colors,
                window,
                cx,
            )),
        DatabaseKind::Sqlite => base.child(sqlite_file_row_light(inputs, colors, window, cx)),
    };

    div().w_full().child(form)
}

fn field_row_light(
    label: &'static str,
    field_id: ConnectionField,
    inputs: &NewConnectionInputs,
    secure: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().label(label).items_center().child(input_box_light(
        field_id, inputs, secure, colors, window, cx,
    ))
}

fn host_port_row_light(
    label: &'static str,
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().label(label).items_center().child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                input_box_light(ConnectionField::Host, inputs, false, colors, window, cx)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(
                input_box_light(ConnectionField::Port, inputs, false, colors, window, cx)
                    .w(px(96.))
                    .flex_none(),
            ),
    )
}

fn sqlite_file_row_light(
    inputs: &NewConnectionInputs,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Field {
    field().label("文件路径").items_center().child(
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                input_box_light(
                    ConnectionField::SqlitePath,
                    inputs,
                    false,
                    colors,
                    window,
                    cx,
                )
                .flex_1()
                .min_w(px(0.)),
            )
            .child(folder_button_light(colors, cx).flex_none()),
    )
}

fn connection_method_row_light(
    label: &'static str,
    selected_label: &'static str,
    secondary_label: &'static str,
    colors: UiColors,
) -> Field {
    field().label(label).items_center().child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(segment_button_light(selected_label, true, colors))
            .child(segment_button_light(secondary_label, false, colors)),
    )
}

fn segment_button_light(label: &'static str, selected: bool, colors: UiColors) -> Div {
    div()
        .h(px(28.))
        .px_3()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(if selected {
            rgb(0x9aa3af)
        } else {
            colors.border
        })
        .bg(if selected {
            colors.tree_selected
        } else {
            colors.input_bg
        })
        .flex()
        .items_center()
        .justify_center()
        .text_color(if selected { colors.text } else { colors.muted })
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .hover(move |style| style.bg(colors.hover).border_color(rgb(0xaeb7c2)))
        .child(label)
}

fn checkbox_row_light(label: &'static str, _colors: UiColors) -> Field {
    field()
        .label("")
        .items_center()
        .child(Checkbox::new("new-connection-checkbox-srv").label(label))
}

fn dropdown_row_light(label: &'static str, value: &'static str, colors: UiColors) -> Field {
    field().label(label).items_center().child(
        div()
            .w(px(112.))
            .h(px(34.))
            .rounded(colors.radius_lg)
            .border_1()
            .border_color(colors.border)
            .bg(colors.input_bg)
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .text_color(colors.text)
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .hover(move |style| style.bg(colors.hover).border_color(rgb(0xaeb7c2)))
            .child(value)
            .child(app_icon(AppIcon::ChevronDown, 14., colors.muted)),
    )
}

fn color_row_light(selected_color: &str, colors: UiColors, cx: &mut Context<NavicatMain>) -> Field {
    let mut swatches = div().flex().items_center().gap_2();
    for &(hex, value) in CONNECTION_COLOR_PALETTE {
        swatches = swatches.child(color_swatch_light(
            hex,
            rgb(value),
            selected_color == hex,
            colors,
            cx,
        ));
    }

    field().label("颜色").items_center().child(swatches)
}

fn color_swatch_light(
    hex: &'static str,
    color: gpui::Rgba,
    selected: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .size(px(26.))
        .rounded_full()
        .border_2()
        .border_color(if selected {
            if colors.is_dark {
                rgb(0xd1d5db)
            } else {
                rgb(0x858b96)
            }
        } else {
            colors.border
        })
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover).border_color(color))
        .child(
            div()
                .size(px(18.))
                .rounded_full()
                .bg(color)
                .border_1()
                .border_color(colors.border),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.set_new_connection_color(hex, cx);
                cx.stop_propagation();
            }),
        )
}

fn connection_test_status(status: &ConnectionTestStatus) -> Div {
    let (text, fg) = match status {
        ConnectionTestStatus::Success(text) => (text.clone(), rgb(0x16a34a)),
        ConnectionTestStatus::Error(text) => (text.clone(), rgb(0xff0000)),
        ConnectionTestStatus::Pending(text) => (text.clone(), rgb(0x667085)),
    };

    div()
        .h(px(36.))
        .flex()
        .items_center()
        .overflow_hidden()
        .truncate()
        .text_size(px(14.))
        .text_color(fg)
        .child(text)
}

fn input_box_light(
    field: ConnectionField,
    inputs: &NewConnectionInputs,
    secure: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focused = inputs.for_field(field).focus_handle(cx).is_focused(window);
    let input = Input::new(inputs.for_field(field))
        .small()
        .appearance(false)
        .focus_bordered(false);

    div()
        .w_full()
        .h(px(34.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(if focused {
            if colors.is_dark {
                rgb(0x8ab4ff)
            } else {
                rgb(0x111111)
            }
        } else {
            colors.border
        })
        .bg(colors.input_bg)
        .shadow(vec![box_shadow(
            px(0.),
            px(1.),
            px(4.),
            px(0.),
            hsla(0., 0., 0., if focused { 0.08 } else { 0.11 }),
        )])
        .flex()
        .items_center()
        .overflow_hidden()
        .child(input.w_full().h_full().px_3().text_size(px(14.)))
        .when(secure, |this| this.child(password_eye_button(colors, cx)))
}

fn password_eye_button(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .size(px(30.))
        .mr_1()
        .rounded(colors.radius)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.muted)
        .hover(move |style| style.bg(colors.hover))
        .child(
            div()
                .relative()
                .w(px(16.))
                .h(px(10.))
                .rounded_full()
                .border_1()
                .border_color(colors.muted)
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(4.)).rounded_full().bg(colors.muted)),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.toggle_new_connection_password_visibility(window, cx);
                cx.stop_propagation();
            }),
        )
}

fn folder_button_light(colors: UiColors, cx: &mut Context<NavicatMain>) -> Div {
    div()
        .size(px(38.))
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.input_bg)
        .flex()
        .items_center()
        .justify_center()
        .hover(move |style| style.bg(colors.hover).border_color(rgb(0xaeb7c2)))
        .child(
            div()
                .relative()
                .w(px(18.))
                .h(px(14.))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(2.))
                        .w(px(8.))
                        .h(px(4.))
                        .rounded(colors.radius * 0.5)
                        .bg(colors.muted),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w(px(18.))
                        .h(px(12.))
                        .rounded(colors.radius * 0.5)
                        .border_2()
                        .border_color(colors.muted)
                        .bg(colors.input_bg),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.choose_sqlite_file(window, cx);
                cx.stop_propagation();
            }),
        )
}

fn database_default_port(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::MySql => "3306",
        DatabaseKind::TiDb => "4000",
        DatabaseKind::MongoDb => "27017",
        DatabaseKind::Redis => "6379",
        DatabaseKind::Sqlite => "",
    }
}

fn database_default_port_u16(kind: DatabaseKind) -> u16 {
    database_default_port(kind).parse().unwrap_or(0)
}

fn parse_port(port: &str) -> Result<u16, String> {
    let port = port.trim();
    if port.is_empty() {
        return Err("请填写端口".to_string());
    }
    port.parse::<u16>()
        .map_err(|_| "端口必须是 1-65535 的数字".to_string())
}

fn non_empty_option(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn first_connection_field(kind: DatabaseKind) -> ConnectionField {
    connection_fields(kind)[0]
}

fn connection_fields(kind: DatabaseKind) -> &'static [ConnectionField] {
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::Database,
            ConnectionField::UrlParams,
        ],
        DatabaseKind::Sqlite => &[ConnectionField::Name, ConnectionField::SqlitePath],
        DatabaseKind::MongoDb => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::MongoDefaultDb,
            ConnectionField::MongoAuthDb,
        ],
        DatabaseKind::Redis => &[
            ConnectionField::Name,
            ConnectionField::Host,
            ConnectionField::Port,
            ConnectionField::Username,
            ConnectionField::Password,
            ConnectionField::UrlParams,
        ],
    }
}

fn connection_field_placeholder(field: ConnectionField) -> &'static str {
    match field {
        ConnectionField::Name => "连接名称",
        ConnectionField::Host => "127.0.0.1",
        ConnectionField::Port => "端口",
        ConnectionField::Username => "可选",
        ConnectionField::Password => "可选",
        ConnectionField::Database | ConnectionField::MongoDefaultDb => "可选",
        ConnectionField::UrlParams => "key=value&key2=value2（Redis: tls=true&sentinel_master=mymaster）",
        ConnectionField::SqlitePath => "/path/to/database.db or :memory:",
        ConnectionField::MongoAuthDb => "可选，通常为 admin",
        ConnectionField::TlsCa => "CA 证书文件路径",
        ConnectionField::TlsClientCert => "客户端证书文件路径",
        ConnectionField::TlsClientKey => "客户端私钥文件路径",
        ConnectionField::TlsSni => "服务器名指示（SNI），可选",
        ConnectionField::SshHost => "跳板机主机",
        ConnectionField::SshPort => "22",
        ConnectionField::SshUsername => "SSH 用户名",
        ConnectionField::SshPassword => "SSH 密码",
        ConnectionField::SshPrivateKey => "私钥文件路径",
        ConnectionField::SshPassphrase => "私钥口令，可选",
        // —— MySQL / TiDB ——
        ConnectionField::MysqlTlsSslMode => "preferred (disabled/preferred/required)",
        ConnectionField::MysqlCharset => "utf8mb4",
        ConnectionField::MysqlProxyType => "socks5 (socks5/http_connect)",
        ConnectionField::MysqlSshConnectTimeout => "0=继承全局",
        ConnectionField::MysqlSshKeepalive => "0=不发送",
        ConnectionField::MysqlProxyHost => "代理主机",
        ConnectionField::MysqlProxyPort => "代理端口",
        ConnectionField::MysqlProxyUsername => "可选",
        ConnectionField::MysqlProxyPassword => "可选",
        ConnectionField::MysqlConnectTimeout => "5",
        ConnectionField::MysqlQueryTimeout => "0=不设限",
        ConnectionField::MysqlIdleTtl => "0=不回收",
        ConnectionField::SentinelMasterName => "Sentinel 主库名",
        ConnectionField::SentinelEndpoints => "host:port, host:port",
        ConnectionField::ClusterStartNodes => "host:port, host:port",
        ConnectionField::CloudSubscription => "订阅 / 账号 ID",
        ConnectionField::CloudResource => "资源 / 数据库名",
        ConnectionField::DiscoveryUri => "redis:// 或 rediss:// 连接串",
    }
}
