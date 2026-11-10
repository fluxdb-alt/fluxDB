// 连接级 Redis「服务器」面板：概览、客户端与慢日志。
//
// 数据来自连接器的只读读取；终止客户端与清空慢日志经 `AppCommand`，
// 只读连接在 UI、控制器和连接器三层都拒绝执行。

const REDIS_SERVER_PANEL_REFRESH_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(5);

impl NavicatMain {
    fn open_redis_server_panel_for_connection(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        match self.dispatch(AppCommand::OpenRedisServerPanel(connection_id), cx) {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => {
                self.start_redis_server_panel_load(tab_id, cx);
                self.start_redis_server_panel_refresh_loop(tab_id, cx);
            }
            _ => {}
        }
    }

    fn redis_server_panel_state_for(&self, tab_id: TabId) -> Option<&RedisServerPanelState> {
        self.controller.state().tabs.iter().find_map(|tab| {
            if tab.id != tab_id {
                return None;
            }
            match &tab.kind {
                TabKind::RedisServerPanel(panel) => Some(panel),
                _ => None,
            }
        })
    }

    fn redis_server_panel_action_busy(&self, tab_id: TabId) -> bool {
        self._server_panel_action_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
    }

    fn redis_server_panel_is_read_only(&self, tab_id: TabId) -> bool {
        self.redis_server_panel_state_for(tab_id).is_some_and(|panel| {
            self.controller
                .state()
                .connections
                .iter()
                .find(|connection| connection.config.id == panel.connection_id)
                .is_some_and(|connection| connection.config.is_read_only())
        })
    }

    fn start_redis_server_panel_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self
            ._server_panel_load_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
        {
            return;
        }
        self.dispatch(AppCommand::StartRedisServerPanelLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadRedisServerPanel(tab_id)) {
                        AppEvent::RedisServerPanelLoaded { result, .. } => result,
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: tr("app.redis_server_panel.error_title").to_string(),
                            message: tr("app.redis_server_panel.error_unknown").to_string(),
                            detail: None,
                            retryable: true,
                        }),
                    }
                })
                .await;
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._server_panel_load_tasks.remove(&tab_id);
                    this.dispatch(
                        AppCommand::FinishRedisServerPanelLoad { tab_id, result },
                        cx,
                    );
                    cx.notify();
                });
            });
        });
        self._server_panel_load_tasks.insert(tab_id, task);
    }

    fn start_redis_server_panel_refresh_loop(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self
            ._server_panel_refresh_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
        {
            return;
        }
        let task = cx.spawn(async move |view, cx| loop {
            cx.background_executor()
                .timer(REDIS_SERVER_PANEL_REFRESH_INTERVAL)
                .await;
            let keep = view
                .update(cx, |this, cx| {
                    let Some(panel) = this.redis_server_panel_state_for(tab_id) else {
                        return false;
                    };
                    if !panel.auto_refresh {
                        return false;
                    }
                    this.start_redis_server_panel_load(tab_id, cx);
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        });
        self._server_panel_refresh_tasks.insert(tab_id, task);
    }

    fn set_redis_server_panel_auto_refresh(
        &mut self,
        tab_id: TabId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            AppCommand::SetRedisServerPanelAutoRefresh { tab_id, enabled },
            cx,
        );
        if enabled {
            self.start_redis_server_panel_refresh_loop(tab_id, cx);
            self.start_redis_server_panel_load(tab_id, cx);
        }
    }

    fn start_redis_server_panel_action(
        &mut self,
        tab_id: TabId,
        action: &'static str,
        command: AppCommand,
        cx: &mut Context<Self>,
    ) {
        if self.redis_server_panel_action_busy(tab_id) {
            return;
        }
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move { controller.dispatch(command) })
                .await;
            let (action, result) = match event {
                AppEvent::RedisServerPanelActionFinished { action, result, .. } => (action, result),
                AppEvent::Failed(error) => (action.to_string(), Err(error)),
                _ => (
                    action.to_string(),
                    Err(fluxdb_core::UserFacingError {
                        title: tr("app.redis_server_panel.action_failed").to_string(),
                        message: tr("app.redis_server_panel.error_unknown").to_string(),
                        detail: None,
                        retryable: true,
                    }),
                ),
            };
            cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._server_panel_action_tasks.remove(&tab_id);
                    match &result {
                        Ok(message) => {
                            let text = if !message.trim().is_empty() {
                                message.clone()
                            } else if action == "kill_client" {
                                tr("app.redis_server_panel.action.kill_client_done").to_string()
                            } else {
                                tr("app.redis_server_panel.action.clear_slowlog_done").to_string()
                            };
                            this.show_message(text, AppMessageKind::Success, cx);
                        }
                        Err(error) => {
                            this.show_message(
                                format!(
                                    "{}: {}",
                                    tr("app.redis_server_panel.action_failed"),
                                    error.message
                                ),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                    this.start_redis_server_panel_load(tab_id, cx);
                    cx.notify();
                });
            });
        });
        self._server_panel_action_tasks.insert(tab_id, task);
    }

    fn request_redis_server_kill_client(
        &mut self,
        tab_id: TabId,
        client_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.redis_server_panel_action_busy(tab_id)
            || self.redis_server_panel_is_read_only(tab_id)
        {
            return;
        }
        let is_self = self
            .redis_server_panel_state_for(tab_id)
            .and_then(|panel| panel.activity.as_ref())
            .is_some_and(|activity| {
                activity
                    .clients
                    .iter()
                    .any(|client| client.id == client_id && client.is_self)
            });
        if is_self {
            return;
        }
        self.redis_server_kill_confirm = Some(RedisServerKillConfirm {
            tab_id,
            client_id,
            checked: false,
        });
        cx.notify();
    }

    fn confirm_redis_server_kill_client(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.redis_server_kill_confirm.clone() else {
            return;
        };
        if !confirm.checked {
            return;
        }
        self.redis_server_kill_confirm = None;
        let tab_id = confirm.tab_id;
        self.start_redis_server_panel_action(
            tab_id,
            "kill_client",
            AppCommand::RedisKillClient {
                tab_id,
                client_id: confirm.client_id,
            },
            cx,
        );
        cx.notify();
    }

    fn request_redis_server_clear_slowlog(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self.redis_server_panel_action_busy(tab_id)
            || self.redis_server_panel_is_read_only(tab_id)
        {
            return;
        }
        self.redis_server_clear_confirm = Some(RedisServerClearConfirm { tab_id });
        cx.notify();
    }

    fn confirm_redis_server_clear_slowlog(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.redis_server_clear_confirm.take() else {
            return;
        };
        self.start_redis_server_panel_action(
            confirm.tab_id,
            "clear_slowlog",
            AppCommand::RedisClearSlowlog {
                tab_id: confirm.tab_id,
            },
            cx,
        );
        cx.notify();
    }
}

fn redis_server_panel_content(
    state: &AppState,
    tab_id: TabId,
    panel: &RedisServerPanelState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let read_only = state
        .connections
        .iter()
        .find(|connection| connection.config.id == panel.connection_id)
        .is_some_and(|connection| connection.config.is_read_only());
    let activity = panel.activity.as_ref();
    let client_count = activity.map(|activity| activity.clients.len()).unwrap_or(0);
    let slowlog_count = activity.map(|activity| activity.slowlog.len()).unwrap_or(0);
    let action_busy = this.redis_server_panel_action_busy(tab_id);

    div()
        .id("redis-server-panel")
        .debug_selector(|| "redis-server-panel".into())
        .size_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(colors.panel_bg)
        .child(
            div()
                .h(px(52.))
                .flex_none()
                .px_4()
                .flex()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(colors.border)
                .child(
                    div()
                        .size(px(32.))
                        .rounded(colors.radius)
                        .bg(colors.panel_alt)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(app_icon(AppIcon::Activity, 18., colors.info)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(panel.connection_name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.muted)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(redis_server_panel_identity(activity, panel)),
                        ),
                )
                .when(read_only, |this| {
                    this.child(
                        server_badge(
                            tr("app.redis_server_panel.read_only").to_string(),
                            colors.warning,
                            colors,
                        )
                        .debug_selector(|| "redis-server-panel-read-only".into()),
                    )
                })
                .child(div().flex_1())
                .child(server_count_badge(
                    tr("app.redis_server_panel.count.clients")
                        .replace("%{count}", &client_count.to_string()),
                    colors,
                ))
                .child(server_count_badge(
                    tr("app.redis_server_panel.count.slowlog")
                        .replace("%{count}", &slowlog_count.to_string()),
                    colors,
                ))
                .child(
                    server_toolbar_button(
                        "redis-server-panel-refresh",
                        AppIcon::Refresh,
                        tr("app.redis_server_panel.refresh").to_string(),
                        false,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.start_redis_server_panel_load(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(
                    server_toggle(
                        ("redis-server-panel-auto", tab_id.0),
                        tr("app.redis_server_panel.auto_refresh").to_string(),
                        panel.auto_refresh,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            let enabled = this
                                .redis_server_panel_state_for(tab_id)
                                .is_some_and(|panel| !panel.auto_refresh);
                            this.set_redis_server_panel_auto_refresh(tab_id, enabled, cx);
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
        .child(redis_server_panel_subtabs(tab_id, panel, colors, cx))
        .when_some(panel.error.clone(), |this, error| {
            this.child(
                div()
                    .flex_none()
                    .mx_4()
                    .mt_3()
                    .px_3()
                    .py_2()
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(colors.danger)
                    .bg(colors.panel_alt)
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(colors.danger)
                    .child(app_icon(AppIcon::TriangleAlert, 14., colors.danger))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(format!(
                                "{}: {}",
                                tr("app.redis_server_panel.error_title"),
                                error.message
                            )),
                    )
                    .child(
                        server_toolbar_button(
                            "redis-server-panel-retry",
                            AppIcon::Refresh,
                            tr("app.redis_server_panel.retry").to_string(),
                            false,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.start_redis_server_panel_load(tab_id, cx);
                                cx.stop_propagation();
                            }),
                        ),
                    ),
            )
        })
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(match panel.active_tab {
                    RedisServerTab::Overview => {
                        redis_server_overview_view(panel, colors).into_any_element()
                    }
                    RedisServerTab::Clients => redis_server_clients_view(
                        tab_id,
                        panel,
                        read_only,
                        action_busy,
                        colors,
                        cx,
                    ),
                    RedisServerTab::SlowLog => redis_server_slowlog_view(
                        tab_id,
                        panel,
                        read_only,
                        action_busy,
                        colors,
                        cx,
                    ),
                }),
        )
        .when_some(this.redis_server_kill_confirm.clone(), |this, confirm| {
            this.child(redis_server_kill_modal(confirm, colors, cx))
        })
        .when_some(this.redis_server_clear_confirm, |this, confirm| {
            this.child(redis_server_clear_modal(confirm, colors, cx))
        })
}

fn redis_server_panel_identity(
    activity: Option<&RedisServerActivity>,
    panel: &RedisServerPanelState,
) -> String {
    let database = format!("DB {}", panel.database);
    match activity {
        Some(activity) if !activity.version.trim().is_empty() => format!(
            "{} · {} · {}",
            activity.version,
            activity.mode,
            database
        ),
        _ => database,
    }
}

fn redis_server_panel_subtabs(
    tab_id: TabId,
    panel: &RedisServerPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let item = |tab: RedisServerTab, label: String| {
        let active = panel.active_tab == tab;
        let debug_subtab =
            format!("redis-server-panel-subtab-{}-{}", tab_id.0, redis_server_tab_index(tab));
        div()
            .id(gpui::ElementId::Name(
                format!("redis-server-panel-subtab-{}-{}", tab_id.0, redis_server_tab_index(tab))
                    .into(),
            ))
            .debug_selector(move || debug_subtab.clone())
            .h_full()
            .px_3()
            .flex()
            .items_center()
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(if active {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .text_color(if active { colors.text } else { colors.muted })
            .border_b_2()
            .border_color(if active { colors.info } else { colors.border_soft })
            .hover(move |style| style.bg(colors.hover))
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.dispatch(
                        AppCommand::SetRedisServerPanelTab { tab_id, tab },
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
    };
    div()
        .h(px(36.))
        .flex_none()
        .px_4()
        .flex()
        .items_center()
        .gap_1()
        .border_b_1()
        .border_color(colors.border)
        .child(item(
            RedisServerTab::Overview,
            tr("app.redis_server_panel.tab.overview").to_string(),
        ))
        .child(item(
            RedisServerTab::Clients,
            tr("app.redis_server_panel.tab.clients").to_string(),
        ))
        .child(item(
            RedisServerTab::SlowLog,
            tr("app.redis_server_panel.tab.slowlog").to_string(),
        ))
}

fn redis_server_tab_index(tab: RedisServerTab) -> u8 {
    match tab {
        RedisServerTab::Overview => 0,
        RedisServerTab::Clients => 1,
        RedisServerTab::SlowLog => 2,
    }
}

fn redis_server_overview_view(
    panel: &RedisServerPanelState,
    colors: UiColors,
) -> AnyElement {
    let Some(activity) = panel.activity.as_ref() else {
        return div()
            .size_full()
            .min_h(px(0.))
            .flex()
            .child(center_loading_message(tr("app.redis_server_panel.loading"), colors))
            .into_any_element();
    };
    div()
        .id("redis-server-panel-overview")
        .debug_selector(|| "redis-server-panel-overview".into())
        .size_full()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(server_status_fact(
                    tr("app.redis_server_panel.status.version").to_string(),
                    server_or_unknown(&activity.version),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.redis_server_panel.status.mode").to_string(),
                    server_or_unknown(&activity.mode),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.redis_server_panel.status.role").to_string(),
                    server_or_unknown(&activity.role),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.redis_server_panel.status.uptime").to_string(),
                    activity
                        .uptime_secs
                        .map(server_format_secs)
                        .unwrap_or_else(|| "-".to_string()),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.redis_server_panel.status.clients").to_string(),
                    activity.connected_clients.to_string(),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.redis_server_panel.status.memory").to_string(),
                    fluxdb_core::format_byte_length(activity.used_memory_bytes),
                    colors,
                )),
        )
        .child(
            div()
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .overflow_hidden()
                .child(server_table_header(
                    colors,
                    vec![
                        (tr("app.redis_server_panel.col.metric").to_string(), 320., false),
                        (tr("app.redis_server_panel.col.value").to_string(), 0., true),
                    ],
                ))
                .children(
                    activity
                        .metrics
                        .iter()
                        .map(|metric| redis_server_metric_row(metric, colors)),
                ),
        )
        .into_any_element()
}

fn redis_server_metric_row(metric: &ServerMetric, colors: UiColors) -> Div {
    div()
        .h(px(34.))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .when(metric.warning, |this| this.bg(colors.panel_alt))
        .child(server_cell(
            redis_server_metric_label(&metric.key),
            320.,
            false,
            if metric.warning { colors.warning } else { colors.text },
        ))
        .child(server_cell(metric.value.clone(), 0., true, colors.text))
}

fn redis_server_metric_label(key: &str) -> String {
    let label = match key {
        "redis_version" => "app.redis_server_panel.metric.redis_version",
        "redis_mode" => "app.redis_server_panel.metric.redis_mode",
        "role" => "app.redis_server_panel.metric.role",
        "uptime_in_seconds" => "app.redis_server_panel.metric.uptime_in_seconds",
        "connected_clients" => "app.redis_server_panel.metric.connected_clients",
        "blocked_clients" => "app.redis_server_panel.metric.blocked_clients",
        "used_memory" => "app.redis_server_panel.metric.used_memory",
        "used_memory_human" => "app.redis_server_panel.metric.used_memory_human",
        "used_memory_peak_human" => "app.redis_server_panel.metric.used_memory_peak_human",
        "maxmemory_human" => "app.redis_server_panel.metric.maxmemory_human",
        "maxmemory_policy" => "app.redis_server_panel.metric.maxmemory_policy",
        "mem_fragmentation_ratio" => "app.redis_server_panel.metric.mem_fragmentation_ratio",
        "instantaneous_ops_per_sec" => "app.redis_server_panel.metric.instantaneous_ops_per_sec",
        "total_connections_received" => {
            "app.redis_server_panel.metric.total_connections_received"
        }
        "total_commands_processed" => "app.redis_server_panel.metric.total_commands_processed",
        "keyspace_hits" => "app.redis_server_panel.metric.keyspace_hits",
        "keyspace_misses" => "app.redis_server_panel.metric.keyspace_misses",
        "expired_keys" => "app.redis_server_panel.metric.expired_keys",
        "evicted_keys" => "app.redis_server_panel.metric.evicted_keys",
        "rejected_connections" => "app.redis_server_panel.metric.rejected_connections",
        "rdb_last_bgsave_status" => "app.redis_server_panel.metric.rdb_last_bgsave_status",
        "aof_last_write_status" => "app.redis_server_panel.metric.aof_last_write_status",
        _ => return key.to_string(),
    };
    tr(label).to_string()
}

fn redis_server_clients_view(
    tab_id: TabId,
    panel: &RedisServerPanelState,
    read_only: bool,
    action_busy: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let Some(activity) = panel.activity.as_ref() else {
        return div()
            .size_full()
            .min_h(px(0.))
            .flex()
            .child(center_loading_message(tr("app.redis_server_panel.loading"), colors))
            .into_any_element();
    };
    let mut view = div()
        .size_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .p_4();
    if let Some(error) = activity.clients_error.as_ref() {
        view = view.child(server_inline_error(
            tr("app.redis_server_panel.clients_error").replace("%{message}", error),
            colors,
        ));
    }
    if activity.clients.is_empty() {
        return view
            .child(server_empty(
                tr("app.redis_server_panel.empty_clients").to_string(),
                colors,
            ))
            .into_any_element();
    }
    let can_kill = !read_only && !action_busy;
    view.child(
        div()
            .id("redis-server-panel-clients")
            .debug_selector(|| "redis-server-panel-clients".into())
            .flex_1()
            .min_h(px(0.))
            .overflow_scrollbar()
            .child(
                div()
                    .min_w(px(1060.))
                    .child(server_table_header(
                        colors,
                        vec![
                            (tr("app.redis_server_panel.col.id").to_string(), 70., false),
                            (tr("app.redis_server_panel.col.addr").to_string(), 190., false),
                            (tr("app.redis_server_panel.col.name").to_string(), 130., false),
                            (tr("app.redis_server_panel.col.user").to_string(), 100., false),
                            (tr("app.redis_server_panel.col.db").to_string(), 60., false),
                            (tr("app.redis_server_panel.col.command").to_string(), 150., false),
                            (tr("app.redis_server_panel.col.age").to_string(), 80., false),
                            (tr("app.redis_server_panel.col.idle").to_string(), 80., false),
                            (tr("app.redis_server_panel.col.flags").to_string(), 80., false),
                            (String::new(), 90., false),
                        ],
                    ))
                    .children(activity.clients.iter().map(|client| {
                        redis_server_client_row(
                            tab_id, client, can_kill, colors, cx,
                        )
                    })),
            ),
    )
    .into_any_element()
}

fn redis_server_client_row(
    tab_id: TabId,
    client: &RedisClient,
    can_kill: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let client_id = client.id.clone();
    let kill_enabled = can_kill && !client.is_self;
    div()
        .id(gpui::ElementId::Name(
            format!("redis-server-client-row-{}-{}", tab_id.0, client.id).into(),
        ))
        .debug_selector({
            let debug_row = format!("redis-server-client-row-{}-{}", tab_id.0, client.id);
            move || debug_row.clone()
        })
        .h(px(34.))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .when(client.is_self, |this| this.bg(colors.tree_selected))
        .child(server_cell(
            if client.is_self {
                format!("{} (me)", client.id)
            } else {
                client.id.clone()
            },
            70.,
            false,
            colors.text,
        ))
        .child(server_cell(client.addr.clone(), 190., false, colors.muted))
        .child(server_cell(client.name.clone(), 130., false, colors.muted))
        .child(server_cell(client.user.clone(), 100., false, colors.text))
        .child(server_cell(client.database.clone(), 60., false, colors.muted))
        .child(server_cell(client.command.clone(), 150., false, colors.muted))
        .child(server_cell(
            server_format_secs(client.age_secs),
            80.,
            false,
            colors.muted,
        ))
        .child(server_cell(
            server_format_secs(client.idle_secs),
            80.,
            false,
            colors.muted,
        ))
        .child(server_cell(client.flags.clone(), 80., false, colors.muted))
        .child(
            div()
                .w(px(90.))
                .flex_none()
                .child(
                    server_small_button(
                        "redis-server-panel-kill-client",
                        tr("app.redis_server_panel.action.kill_client").to_string(),
                        AppIcon::Close,
                        kill_enabled,
                        true,
                        colors,
                    )
                    .id(gpui::ElementId::Name(
                        format!("redis-server-panel-kill-client-{}-{}", tab_id.0, client.id).into(),
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.request_redis_server_kill_client(
                                tab_id,
                                client_id.clone(),
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
}

fn redis_server_slowlog_view(
    tab_id: TabId,
    panel: &RedisServerPanelState,
    read_only: bool,
    action_busy: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let Some(activity) = panel.activity.as_ref() else {
        return div()
            .size_full()
            .min_h(px(0.))
            .flex()
            .child(center_loading_message(tr("app.redis_server_panel.loading"), colors))
            .into_any_element();
    };
    let mut view = div()
        .size_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .p_4()
        .gap_3();
    if let Some(error) = activity.slowlog_error.as_ref() {
        view = view.child(server_inline_error(
            tr("app.redis_server_panel.slowlog_error").replace("%{message}", error),
            colors,
        ));
    }
    view = view.child(
        div()
            .flex()
            .items_center()
            .gap_4()
            .child(server_status_fact(
                tr("app.redis_server_panel.slowlog.len").to_string(),
                activity.slowlog_len.to_string(),
                colors,
            ))
            .child(server_status_fact(
                tr("app.redis_server_panel.slowlog.threshold").to_string(),
                activity
                    .slowlog_log_slower_than_micros
                    .map(|micros| format!("{micros} μs"))
                    .unwrap_or_else(|| "-".to_string()),
                colors,
            ))
            .child(server_status_fact(
                tr("app.redis_server_panel.slowlog.max_len").to_string(),
                activity
                    .slowlog_max_len
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                colors,
            ))
            .child(div().flex_1())
            .child(
                server_small_button(
                    "redis-server-panel-clear-slowlog",
                    tr("app.redis_server_panel.action.clear_slowlog").to_string(),
                    AppIcon::Trash,
                    !read_only && !action_busy,
                    true,
                    colors,
                )
                .id(gpui::ElementId::Name(
                    format!("redis-server-panel-clear-slowlog-{}", tab_id.0).into(),
                ))
                .debug_selector({
                    let debug_action =
                        format!("redis-server-panel-clear-slowlog-{}", tab_id.0);
                    move || debug_action.clone()
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.request_redis_server_clear_slowlog(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ),
            ),
    );
    if activity.slowlog.is_empty() {
        return view
            .child(server_empty(
                tr("app.redis_server_panel.empty_slowlog").to_string(),
                colors,
            ))
            .into_any_element();
    }
    view.child(
        div()
            .id("redis-server-panel-slowlog")
            .debug_selector(|| "redis-server-panel-slowlog".into())
            .flex_1()
            .min_h(px(0.))
            .overflow_scrollbar()
            .child(
                div()
                    .min_w(px(1040.))
                    .child(server_table_header(
                        colors,
                        vec![
                            (tr("app.redis_server_panel.col.id").to_string(), 70., false),
                            (tr("app.redis_server_panel.col.time").to_string(), 160., false),
                            (
                                tr("app.redis_server_panel.col.duration").to_string(),
                                100.,
                                false,
                            ),
                            (tr("app.redis_server_panel.col.client").to_string(), 190., false),
                            (tr("app.redis_server_panel.col.command").to_string(), 0., true),
                        ],
                    ))
                    .children(
                        activity
                            .slowlog
                            .iter()
                            .map(|entry| redis_server_slowlog_row(entry, colors)),
                    ),
            ),
    )
    .into_any_element()
}

fn redis_server_slowlog_row(entry: &RedisSlowLogEntry, colors: UiColors) -> Div {
    let time = chrono::DateTime::from_timestamp(entry.timestamp_unix_secs, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| entry.timestamp_unix_secs.to_string());
    let client = if entry.client_name.trim().is_empty() {
        entry.client_addr.clone()
    } else {
        format!("{} ({})", entry.client_addr, entry.client_name)
    };
    div()
        .h(px(34.))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(server_cell(entry.id.to_string(), 70., false, colors.muted))
        .child(server_cell(time, 160., false, colors.muted))
        .child(server_cell(
            format!("{} μs", entry.duration_micros),
            100.,
            false,
            colors.text,
        ))
        .child(server_cell(client, 190., false, colors.muted))
        .child(server_cell(
            server_sql_preview(&entry.command, 240),
            0.,
            true,
            colors.text,
        ))
}

fn redis_server_kill_modal(
    confirm: RedisServerKillConfirm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let checked = confirm.checked;
    let client_id = confirm.client_id.clone();
    server_modal_shell(
        "redis-server-kill-modal",
        colors,
        div()
            .w(px(460.))
            .child(
                div()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(tr("app.redis_server_panel.kill.title")),
                    )
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(colors.radius)
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(move |style| style.bg(colors.hover))
                            .child(app_icon(AppIcon::Close, 15., colors.muted))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.redis_server_kill_confirm = None;
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .px_5()
                    .pb_3()
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child(
                        tr("app.redis_server_panel.kill.message")
                            .replace("%{id}", &client_id),
                    ),
            )
            .child(
                div()
                    .mx_5()
                    .mb_3()
                    .child(
                        server_check_row(
                            tr("app.redis_server_panel.kill.ack").to_string(),
                            checked,
                            colors,
                        )
                        .id("redis-server-panel-kill-ack")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                if let Some(confirm) = this.redis_server_kill_confirm.as_mut() {
                                    confirm.checked = !confirm.checked;
                                }
                                cx.notify();
                                cx.stop_propagation();
                            }),
                        ),
                    ),
            )
            .child(div().h(px(1.)).bg(colors.border_soft))
            .child(
                div()
                    .h(px(58.))
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(
                        server_dialog_button(
                            tr("app.redis_server_panel.cancel").to_string(),
                            false,
                            true,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.redis_server_kill_confirm = None;
                                cx.notify();
                                cx.stop_propagation();
                            }),
                        ),
                    )
                    .child(
                        server_dialog_button(
                            tr("app.redis_server_panel.kill.confirm").to_string(),
                            true,
                            checked,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.confirm_redis_server_kill_client(cx);
                                cx.stop_propagation();
                            }),
                        ),
                    ),
            ),
    )
}

fn redis_server_clear_modal(
    _confirm: RedisServerClearConfirm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    server_modal_shell(
        "redis-server-clear-modal",
        colors,
        div()
            .w(px(430.))
            .child(
                div()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .text_size(px(17.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(tr("app.redis_server_panel.slowlog.clear_title")),
            )
            .child(
                div()
                    .px_5()
                    .pb_4()
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child(tr("app.redis_server_panel.slowlog.clear_message").to_string()),
            )
            .child(div().h(px(1.)).bg(colors.border_soft))
            .child(
                div()
                    .h(px(58.))
                    .px_5()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(
                        server_dialog_button(
                            tr("app.redis_server_panel.cancel").to_string(),
                            false,
                            true,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.redis_server_clear_confirm = None;
                                cx.notify();
                                cx.stop_propagation();
                            }),
                        ),
                    )
                    .child(
                        server_dialog_button(
                            tr("app.redis_server_panel.slowlog.clear_confirm").to_string(),
                            true,
                            true,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.confirm_redis_server_clear_slowlog(cx);
                                cx.stop_propagation();
                            }),
                        ),
                    ),
            ),
    )
}
