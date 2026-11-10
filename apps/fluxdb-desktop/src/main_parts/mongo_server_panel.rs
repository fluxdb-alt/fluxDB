// 连接级 MongoDB「服务器」面板：概览与当前操作。
//
// 数据来自连接器的只读读取（serverStatus / hello / $currentOp）；终止操作经
// `AppCommand`，只读连接在 UI、控制器和连接器三层都拒绝执行。
// 慢操作不单独成页，只是「当前操作」的一个过滤条件。

const MONGO_SERVER_PANEL_REFRESH_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(5);

impl NavicatMain {
    fn open_mongo_server_panel_for_connection(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        match self.dispatch(AppCommand::OpenMongoServerPanel(connection_id), cx) {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => {
                self.start_mongo_server_panel_load(tab_id, cx);
                self.start_mongo_server_panel_refresh_loop(tab_id, cx);
            }
            _ => {}
        }
    }

    fn mongo_server_panel_state_for(&self, tab_id: TabId) -> Option<&MongoServerPanelState> {
        self.controller.state().tabs.iter().find_map(|tab| {
            if tab.id != tab_id {
                return None;
            }
            match &tab.kind {
                TabKind::MongoServerPanel(panel) => Some(panel),
                _ => None,
            }
        })
    }

    fn mongo_server_panel_action_busy(&self, tab_id: TabId) -> bool {
        self._server_panel_action_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
    }

    fn mongo_server_panel_is_read_only(&self, tab_id: TabId) -> bool {
        self.mongo_server_panel_state_for(tab_id).is_some_and(|panel| {
            self.controller
                .state()
                .connections
                .iter()
                .find(|connection| connection.config.id == panel.connection_id)
                .is_some_and(|connection| connection.config.is_read_only())
        })
    }

    fn start_mongo_server_panel_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self
            ._server_panel_load_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
        {
            return;
        }
        self.dispatch(AppCommand::StartMongoServerPanelLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadMongoServerPanel(tab_id)) {
                        AppEvent::MongoServerPanelLoaded { result, .. } => result,
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: tr("app.mongo_server_panel.error_title").to_string(),
                            message: tr("app.mongo_server_panel.error_unknown").to_string(),
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
                        AppCommand::FinishMongoServerPanelLoad { tab_id, result },
                        cx,
                    );
                    cx.notify();
                });
            });
        });
        self._server_panel_load_tasks.insert(tab_id, task);
    }

    fn start_mongo_server_panel_refresh_loop(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self
            ._server_panel_refresh_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
        {
            return;
        }
        let task = cx.spawn(async move |view, cx| loop {
            cx.background_executor()
                .timer(MONGO_SERVER_PANEL_REFRESH_INTERVAL)
                .await;
            let keep = view
                .update(cx, |this, cx| {
                    let Some(panel) = this.mongo_server_panel_state_for(tab_id) else {
                        return false;
                    };
                    if !panel.auto_refresh {
                        return false;
                    }
                    this.start_mongo_server_panel_load(tab_id, cx);
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        });
        self._server_panel_refresh_tasks.insert(tab_id, task);
    }

    fn set_mongo_server_panel_auto_refresh(
        &mut self,
        tab_id: TabId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            AppCommand::SetMongoServerPanelAutoRefresh { tab_id, enabled },
            cx,
        );
        if enabled {
            self.start_mongo_server_panel_refresh_loop(tab_id, cx);
            self.start_mongo_server_panel_load(tab_id, cx);
        }
    }

    fn start_mongo_server_panel_action(
        &mut self,
        tab_id: TabId,
        action: &'static str,
        command: AppCommand,
        cx: &mut Context<Self>,
    ) {
        if self.mongo_server_panel_action_busy(tab_id) {
            return;
        }
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move { controller.dispatch(command) })
                .await;
            let (action, result) = match event {
                AppEvent::MongoServerPanelActionFinished { action, result, .. } => (action, result),
                AppEvent::Failed(error) => (action.to_string(), Err(error)),
                _ => (
                    action.to_string(),
                    Err(fluxdb_core::UserFacingError {
                        title: tr("app.mongo_server_panel.action_failed").to_string(),
                        message: tr("app.mongo_server_panel.error_unknown").to_string(),
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
                            } else if action == "kill_operation" {
                                tr("app.mongo_server_panel.action.kill_operation_done").to_string()
                            } else {
                                message.clone()
                            };
                            this.show_message(text, AppMessageKind::Success, cx);
                        }
                        Err(error) => {
                            this.show_message(
                                format!(
                                    "{}: {}",
                                    tr("app.mongo_server_panel.action_failed"),
                                    error.message
                                ),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                    this.start_mongo_server_panel_load(tab_id, cx);
                    cx.notify();
                });
            });
        });
        self._server_panel_action_tasks.insert(tab_id, task);
    }

    fn request_mongo_server_kill_operation(
        &mut self,
        tab_id: TabId,
        opid: String,
        cx: &mut Context<Self>,
    ) {
        if self.mongo_server_panel_action_busy(tab_id)
            || self.mongo_server_panel_is_read_only(tab_id)
        {
            return;
        }
        self.mongo_server_kill_confirm = Some(MongoServerKillConfirm { tab_id, opid });
        cx.notify();
    }

    fn confirm_mongo_server_kill_operation(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.mongo_server_kill_confirm.take() else {
            return;
        };
        self.start_mongo_server_panel_action(
            confirm.tab_id,
            "kill_operation",
            AppCommand::MongoKillOperation {
                tab_id: confirm.tab_id,
                opid: confirm.opid,
            },
            cx,
        );
        cx.notify();
    }
}

fn mongo_server_panel_content(
    state: &AppState,
    tab_id: TabId,
    panel: &MongoServerPanelState,
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
    let operation_count = activity.map(|activity| activity.operations.len()).unwrap_or(0);
    let slow_count = activity
        .map(|activity| {
            activity
                .operations
                .iter()
                .filter(|operation| operation.is_slow())
                .count()
        })
        .unwrap_or(0);
    let action_busy = this.mongo_server_panel_action_busy(tab_id);

    div()
        .id("mongo-server-panel")
        .debug_selector(|| "mongo-server-panel".into())
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
                                .child(mongo_server_panel_identity(activity, panel)),
                        ),
                )
                .when(read_only, |this| {
                    this.child(
                        server_badge(
                            tr("app.mongo_server_panel.read_only").to_string(),
                            colors.warning,
                            colors,
                        )
                        .debug_selector(|| "mongo-server-panel-read-only".into()),
                    )
                })
                .child(div().flex_1())
                .child(server_count_badge(
                    tr("app.mongo_server_panel.count.operations")
                        .replace("%{count}", &operation_count.to_string()),
                    colors,
                ))
                .child(server_count_badge(
                    tr("app.mongo_server_panel.count.slow")
                        .replace("%{count}", &slow_count.to_string()),
                    colors,
                ))
                .child(
                    server_toolbar_button(
                        "mongo-server-panel-refresh",
                        AppIcon::Refresh,
                        tr("app.mongo_server_panel.refresh").to_string(),
                        false,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.start_mongo_server_panel_load(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    ),
                )
                .child(
                    server_toggle(
                        ("mongo-server-panel-auto", tab_id.0),
                        tr("app.mongo_server_panel.auto_refresh").to_string(),
                        panel.auto_refresh,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            let enabled = this
                                .mongo_server_panel_state_for(tab_id)
                                .is_some_and(|panel| !panel.auto_refresh);
                            this.set_mongo_server_panel_auto_refresh(tab_id, enabled, cx);
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
        .child(mongo_server_panel_subtabs(tab_id, panel, colors, cx))
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
                                tr("app.mongo_server_panel.error_title"),
                                error.message
                            )),
                    )
                    .child(
                        server_toolbar_button(
                            "mongo-server-panel-retry",
                            AppIcon::Refresh,
                            tr("app.mongo_server_panel.retry").to_string(),
                            false,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.start_mongo_server_panel_load(tab_id, cx);
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
                    MongoServerTab::Overview => {
                        mongo_server_overview_view(panel, colors).into_any_element()
                    }
                    MongoServerTab::Operations => mongo_server_operations_view(
                        tab_id,
                        panel,
                        read_only,
                        action_busy,
                        colors,
                        cx,
                    ),
                    MongoServerTab::Performance => mongo_server_performance_view(panel, colors),
                }),
        )
        .when_some(this.mongo_server_kill_confirm.clone(), |this, confirm| {
            this.child(mongo_server_kill_modal(confirm, colors, cx))
        })
}

fn mongo_server_panel_identity(
    activity: Option<&MongoServerActivity>,
    panel: &MongoServerPanelState,
) -> String {
    match activity {
        Some(activity) if !activity.version.trim().is_empty() => {
            let topology = mongo_server_topology_label(activity.topology, &activity.member_state);
            format!("{} · {} · {}", activity.version, topology, panel.database)
        }
        _ => panel.database.clone(),
    }
}

fn mongo_server_topology_label(topology: MongoTopology, member_state: &str) -> String {
    let base = match topology {
        MongoTopology::Standalone => tr("app.mongo_server_panel.topology.standalone").to_string(),
        MongoTopology::ReplicaSet => tr("app.mongo_server_panel.topology.replica_set").to_string(),
        MongoTopology::Sharded => tr("app.mongo_server_panel.topology.sharded").to_string(),
        MongoTopology::Unknown => tr("app.mongo_server_panel.topology.unknown").to_string(),
    };
    if member_state.trim().is_empty() {
        base
    } else {
        format!("{base} · {member_state}")
    }
}

fn mongo_server_panel_subtabs(
    tab_id: TabId,
    panel: &MongoServerPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let item = |tab: MongoServerTab, label: String| {
        let active = panel.active_tab == tab;
        let debug_subtab =
            format!("mongo-server-panel-subtab-{}-{}", tab_id.0, mongo_server_tab_index(tab));
        div()
            .id(gpui::ElementId::Name(
                format!("mongo-server-panel-subtab-{}-{}", tab_id.0, mongo_server_tab_index(tab))
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
                        AppCommand::SetMongoServerPanelTab { tab_id, tab },
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
            MongoServerTab::Overview,
            tr("app.mongo_server_panel.tab.overview").to_string(),
        ))
        .child(item(
            MongoServerTab::Operations,
            tr("app.mongo_server_panel.tab.operations").to_string(),
        ))
        .child(item(
            MongoServerTab::Performance,
            tr("app.mongo_server_panel.tab.performance").to_string(),
        ))
}

fn mongo_server_tab_index(tab: MongoServerTab) -> u8 {
    match tab {
        MongoServerTab::Overview => 0,
        MongoServerTab::Operations => 1,
        MongoServerTab::Performance => 2,
    }
}

fn mongo_server_overview_view(
    panel: &MongoServerPanelState,
    colors: UiColors,
) -> AnyElement {
    let Some(activity) = panel.activity.as_ref() else {
        return div()
            .size_full()
            .min_h(px(0.))
            .flex()
            .child(center_loading_message(
                tr("app.mongo_server_panel.loading"),
                colors,
            ))
            .into_any_element();
    };
    div()
        .id("mongo-server-panel-overview")
        .debug_selector(|| "mongo-server-panel-overview".into())
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
                    tr("app.mongo_server_panel.status.version").to_string(),
                    server_or_unknown(&activity.version),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.mongo_server_panel.status.edition").to_string(),
                    server_or_unknown(&activity.edition),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.mongo_server_panel.status.topology").to_string(),
                    mongo_server_topology_label(activity.topology, &activity.member_state),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.mongo_server_panel.status.replica_set").to_string(),
                    if activity.replica_set_name.trim().is_empty() {
                        "-".to_string()
                    } else {
                        activity.replica_set_name.clone()
                    },
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.mongo_server_panel.status.uptime").to_string(),
                    activity
                        .uptime_secs
                        .map(server_format_secs)
                        .unwrap_or_else(|| "-".to_string()),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.mongo_server_panel.status.connections").to_string(),
                    match activity.connections_available {
                        Some(available) => {
                            format!("{} / +{}", activity.connections_current, available)
                        }
                        None => activity.connections_current.to_string(),
                    },
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
                        (tr("app.mongo_server_panel.col.metric").to_string(), 320., false),
                        (tr("app.mongo_server_panel.col.value").to_string(), 0., true),
                    ],
                ))
                .children(
                    activity
                        .metrics
                        .iter()
                        .map(|metric| mongo_server_metric_row(metric, colors)),
                ),
        )
        .into_any_element()
}

fn mongo_server_metric_row(metric: &ServerMetric, colors: UiColors) -> Div {
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
            mongo_server_metric_label(&metric.key),
            320.,
            false,
            if metric.warning { colors.warning } else { colors.text },
        ))
        .child(server_cell(metric.value.clone(), 0., true, colors.text))
}

/// 指标 key 是 `serverStatus` 的层级路径；界面按 key 本地化，未知 key 原样展示。
fn mongo_server_metric_label(key: &str) -> String {
    let label = match key {
        "opcounters.insert" => "app.mongo_server_panel.metric.opcounters_insert",
        "opcounters.query" => "app.mongo_server_panel.metric.opcounters_query",
        "opcounters.update" => "app.mongo_server_panel.metric.opcounters_update",
        "opcounters.delete" => "app.mongo_server_panel.metric.opcounters_delete",
        "opcounters.getmore" => "app.mongo_server_panel.metric.opcounters_getmore",
        "opcounters.command" => "app.mongo_server_panel.metric.opcounters_command",
        "connections.current" => "app.mongo_server_panel.metric.connections_current",
        "connections.available" => "app.mongo_server_panel.metric.connections_available",
        "connections.totalCreated" => "app.mongo_server_panel.metric.connections_total_created",
        "connections.active" => "app.mongo_server_panel.metric.connections_active",
        "mem.resident" => "app.mongo_server_panel.metric.mem_resident",
        "mem.virtual" => "app.mongo_server_panel.metric.mem_virtual",
        "network.bytesIn" => "app.mongo_server_panel.metric.network_bytes_in",
        "network.bytesOut" => "app.mongo_server_panel.metric.network_bytes_out",
        "globalLock.activeClients.total" => {
            "app.mongo_server_panel.metric.global_lock_active_clients"
        }
        "globalLock.currentQueue.total" => {
            "app.mongo_server_panel.metric.global_lock_current_queue"
        }
        "asserts.warning" => "app.mongo_server_panel.metric.asserts_warning",
        "asserts.msg" => "app.mongo_server_panel.metric.asserts_msg",
        "asserts.user" => "app.mongo_server_panel.metric.asserts_user",
        _ => return key.to_string(),
    };
    tr(label).to_string()
}

fn mongo_server_operations_view(
    tab_id: TabId,
    panel: &MongoServerPanelState,
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
            .child(center_loading_message(
                tr("app.mongo_server_panel.loading"),
                colors,
            ))
            .into_any_element();
    };
    let mut view = div()
        .size_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .p_4()
        .gap_3();
    if let Some(error) = activity.operations_error.as_ref() {
        view = view.child(server_inline_error(
            tr("app.mongo_server_panel.operations_error").replace("%{message}", error),
            colors,
        ));
    }
    view = view.child(
        div()
            .flex()
            .items_center()
            .gap_4()
            .child(server_status_fact(
                tr("app.mongo_server_panel.status.operations").to_string(),
                activity.operations.len().to_string(),
                colors,
            ))
            .child(server_status_fact(
                tr("app.mongo_server_panel.status.slow_operations").to_string(),
                activity
                    .operations
                    .iter()
                    .filter(|operation| operation.is_slow())
                    .count()
                    .to_string(),
                colors,
            ))
            .child(div().flex_1())
            .child(
                server_toggle(
                    ("mongo-server-panel-only-slow", tab_id.0),
                    tr("app.mongo_server_panel.only_slow").to_string(),
                    panel.only_slow,
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        let only_slow = this
                            .mongo_server_panel_state_for(tab_id)
                            .is_some_and(|panel| !panel.only_slow);
                        this.dispatch(
                            AppCommand::SetMongoServerPanelOnlySlow { tab_id, only_slow },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                ),
            ),
    );
    let operations = panel.visible_operations();
    if operations.is_empty() {
        return view
            .child(server_empty(
                tr("app.mongo_server_panel.empty_operations").to_string(),
                colors,
            ))
            .into_any_element();
    }
    let can_kill = !read_only && !action_busy;
    view.child(
        div()
            .id("mongo-server-panel-operations")
            .debug_selector(|| "mongo-server-panel-operations".into())
            .flex_1()
            .min_h(px(0.))
            .overflow_scrollbar()
            .child(
                div()
                    .min_w(px(1120.))
                    .child(server_table_header(
                        colors,
                        vec![
                            (tr("app.mongo_server_panel.col.opid").to_string(), 90., false),
                            (tr("app.mongo_server_panel.col.client").to_string(), 180., false),
                            (tr("app.mongo_server_panel.col.op").to_string(), 90., false),
                            (
                                tr("app.mongo_server_panel.col.namespace").to_string(),
                                200.,
                                false,
                            ),
                            (tr("app.mongo_server_panel.col.secs").to_string(), 80., false),
                            (tr("app.mongo_server_panel.col.description").to_string(), 160., false),
                            (tr("app.mongo_server_panel.col.plan").to_string(), 0., true),
                            (String::new(), 90., false),
                        ],
                    ))
                    .children(
                        operations
                            .iter()
                            .map(|operation| mongo_server_operation_row(
                                tab_id, operation, can_kill, colors, cx,
                            )),
                    ),
            ),
    )
    .into_any_element()
}


/// 「性能」子页：把面板自身的采样画成趋势条，不做历史持久化。
///
/// 至少两个采样点才能画出趋势；不足时整页显示加载态，而不是三张空图卡。
fn mongo_server_performance_view(
    panel: &MongoServerPanelState,
    colors: UiColors,
) -> AnyElement {
    let samples = &panel.samples;
    if samples.len() < 2 {
        return div()
            .id("mongo-server-panel-performance-loading")
            .debug_selector(|| "mongo-server-panel-performance-loading".into())
            .size_full()
            .min_h(px(0.))
            .flex()
            .child(center_loading_message(
                tr("app.mongo_server_panel.performance.empty"),
                colors,
            ))
            .into_any_element();
    }
    // 操作数存的是累计值，相邻采样求差再除以真实间隔，得到每秒操作数。
    let ops_per_sec: Vec<f64> = samples
        .windows(2)
        .map(|pair| {
            let elapsed = pair[1]
                .at_unix_secs
                .saturating_sub(pair[0].at_unix_secs)
                .max(1);
            let delta = pair[1].ops_total.saturating_sub(pair[0].ops_total);
            delta as f64 / elapsed as f64
        })
        .collect();
    let connections: Vec<f64> = samples.iter().map(|s| s.connections as f64).collect();
    let slow_operations: Vec<f64> = samples.iter().map(|s| s.slow_operations as f64).collect();
    let current = |series: &[f64]| {
        series
            .last()
            .map(|value| format!("{value:.0}"))
            .unwrap_or_else(|| "-".to_string())
    };

    div()
        .id("mongo-server-panel-performance")
        .debug_selector(|| "mongo-server-panel-performance".into())
        .size_full()
        .min_h(px(0.))
        .overflow_y_scrollbar()
        .p_4()
        .flex()
        .flex_col()
        .gap_4()
        .child(mongo_server_performance_chart(
            tr("app.mongo_server_panel.performance.ops_per_sec").to_string(),
            current(&ops_per_sec),
            &ops_per_sec,
            AppIcon::Activity,
            colors.info,
            colors,
        ))
        .child(mongo_server_performance_chart(
            tr("app.mongo_server_panel.performance.connections").to_string(),
            current(&connections),
            &connections,
            AppIcon::Plug,
            colors.success,
            colors,
        ))
        .child(mongo_server_performance_chart(
            tr("app.mongo_server_panel.performance.slow_operations").to_string(),
            current(&slow_operations),
            &slow_operations,
            AppIcon::TriangleAlert,
            colors.warning,
            colors,
        ))
        .child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(
                    tr("app.mongo_server_panel.performance.note")
                        .replace("%{count}", &fluxdb_app::MONGO_SERVER_SAMPLE_LIMIT.to_string()),
                ),
        )
        .into_any_element()
}

fn mongo_server_performance_chart(
    label: String,
    current: String,
    series: &[f64],
    icon: AppIcon,
    color: gpui::Rgba,
    colors: UiColors,
) -> Div {
    let max = series.iter().copied().fold(0.0_f64, f64::max).max(1.0);
    div()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(app_icon(icon, 14., color))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(current),
                ),
        )
        .child(
            div()
                .h(px(64.))
                .flex()
                .items_end()
                .gap_1()
                .when(series.is_empty(), |this| {
                    this.child(div().w_full().h(px(1.)).bg(colors.border_soft))
                })
                .children(series.iter().map(|value| {
                    let height = 4. + (value / max) * 56.;
                    div()
                        .flex_1()
                        .h(px(height as f32))
                        .rounded_t(px(2.))
                        .bg(color)
                })),
        )
}

fn mongo_server_operation_row(
    tab_id: TabId,
    operation: &MongoOperation,
    can_kill: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let opid = operation.opid.clone();
    // 分片/副本集内部的 opid 可能不是数字，killOp 只接受数字，提前禁用而不是点了才报错。
    let kill_enabled = can_kill && operation.opid.parse::<i64>().is_ok();
    let description = if operation.waiting_for_lock {
        format!(
            "{} ({})",
            operation.description,
            tr("app.mongo_server_panel.waiting_for_lock")
        )
    } else {
        operation.description.clone()
    };
    div()
        .id(gpui::ElementId::Name(
            format!("mongo-server-operation-row-{}-{}", tab_id.0, operation.opid).into(),
        ))
        .debug_selector({
            let debug_row = format!("mongo-server-operation-row-{}-{}", tab_id.0, operation.opid);
            move || debug_row.clone()
        })
        .h(px(34.))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .when(operation.is_slow(), |this| this.bg(colors.panel_alt))
        .child(server_cell(operation.opid.clone(), 90., false, colors.muted))
        .child(server_cell(operation.client.clone(), 180., false, colors.muted))
        .child(server_cell(operation.op.clone(), 90., false, colors.text))
        .child(server_cell(operation.namespace.clone(), 200., false, colors.muted))
        .child(server_cell(
            server_format_secs(operation.secs_running),
            80.,
            false,
            if operation.is_slow() {
                colors.warning
            } else {
                colors.muted
            },
        ))
        .child(server_cell(description, 160., false, colors.muted))
        .child(server_cell(
            server_sql_preview(&operation.plan_summary, 240),
            0.,
            true,
            colors.text,
        ))
        .child(
            div()
                .w(px(90.))
                .flex_none()
                .child(
                    server_small_button(
                        "mongo-server-panel-kill-operation",
                        tr("app.mongo_server_panel.action.kill_operation").to_string(),
                        AppIcon::Close,
                        kill_enabled,
                        true,
                        colors,
                    )
                    .id(gpui::ElementId::Name(
                        format!("mongo-server-panel-kill-operation-{}-{}", tab_id.0, operation.opid)
                            .into(),
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.request_mongo_server_kill_operation(
                                tab_id,
                                opid.clone(),
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
                ),
        )
}

fn mongo_server_kill_modal(
    confirm: MongoServerKillConfirm,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let opid = confirm.opid.clone();
    server_modal_shell(
        "mongo-server-kill-modal",
        colors,
        div()
            .w(px(440.))
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
                            .child(tr("app.mongo_server_panel.kill.title")),
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
                                    this.mongo_server_kill_confirm = None;
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .px_5()
                    .pb_4()
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child(
                        tr("app.mongo_server_panel.kill.message")
                            .replace("%{id}", &opid),
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
                            tr("app.mongo_server_panel.cancel").to_string(),
                            false,
                            true,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.mongo_server_kill_confirm = None;
                                cx.notify();
                                cx.stop_propagation();
                            }),
                        ),
                    )
                    .child(
                        server_dialog_button(
                            tr("app.mongo_server_panel.kill.confirm").to_string(),
                            true,
                            true,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.confirm_mongo_server_kill_operation(cx);
                                cx.stop_propagation();
                            }),
                        ),
                    ),
            ),
    )
}
