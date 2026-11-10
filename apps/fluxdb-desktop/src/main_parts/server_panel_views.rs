// 服务器面板的锁等待与实例指标视图。

fn server_locks_view(tab_id: TabId, panel: &ServerPanelState, colors: UiColors) -> Div {
    let Some(activity) = panel.activity.as_ref() else {
        return center_loading_message(tr("app.server_panel.loading"), colors);
    };
    // 与「只看我的会话」保持同一口径：面板自身连接与同一账号的会话都算我的。
    let my_sessions = panel
        .visible_sessions()
        .into_iter()
        .map(|session| session.id.as_str())
        .collect::<BTreeSet<_>>();
    let locks = activity
        .locks
        .iter()
        .filter(|lock| {
            !panel.only_mine
                || my_sessions.contains(lock.waiting_session.as_str())
                || my_sessions.contains(lock.blocking_session.as_str())
        })
        .collect::<Vec<_>>();
    let mut view = div()
        .size_full()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .p_4();
    if let Some(error) = activity.locks_error.as_ref() {
        view = view.child(server_inline_error(
            tr("app.server_panel.locks_error").replace("%{message}", error),
            colors,
        ));
    }
    if locks.is_empty() {
        return view.child(server_empty(
            tr("app.server_panel.empty_locks").to_string(),
            colors,
        ));
    }
    view.child(
        div()
            .id("server-panel-locks")
            .debug_selector(|| "server-panel-locks".into())
            .flex_1()
            .min_h(px(0.))
            .overflow_scrollbar()
            .child(
                div()
                    .min_w(px(1080.))
                    .child(server_table_header(
                        colors,
                        vec![
                            (tr("app.server_panel.col.waiting").to_string(), 100., false),
                            (tr("app.server_panel.col.blocking").to_string(), 100., false),
                            (tr("app.server_panel.col.object").to_string(), 220., false),
                            (tr("app.server_panel.col.lock_type").to_string(), 110., false),
                            (tr("app.server_panel.col.wait").to_string(), 80., false),
                            (tr("app.server_panel.col.state").to_string(), 120., false),
                            (tr("app.server_panel.col.sql").to_string(), 0., true),
                        ],
                    ))
                    .children(locks.into_iter().enumerate().map(|(index, lock)| {
                        server_lock_row(tab_id, lock, index, colors)
                    })),
            ),
    )
}

fn server_lock_row(
    tab_id: TabId,
    lock: &ServerLockWait,
    index: usize,
    colors: UiColors,
) -> Stateful<Div> {
    div()
        .id(gpui::ElementId::Name(
            format!("server-lock-row-{}-{}", tab_id.0, index).into(),
        ))
        .h(px(34.))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(colors.border_soft)
        .child(server_cell(lock.waiting_session.clone(), 100., false, colors.warning))
        .child(server_cell(lock.blocking_session.clone(), 100., false, colors.text))
        .child(server_cell(lock.object.clone(), 220., false, colors.muted))
        .child(server_cell(
            server_lock_type_label(&lock.lock_type),
            110.,
            false,
            colors.muted,
        ))
        .child(server_cell(
            server_format_secs(lock.wait_secs),
            80.,
            false,
            colors.muted,
        ))
        .child(server_cell(
            server_lock_state_label(&lock.state),
            120.,
            false,
            colors.muted,
        ))
        .child(server_cell(
            server_sql_preview(&lock.sql, 240),
            0.,
            true,
            colors.text,
        ))
}

fn server_status_view(
    panel: &ServerPanelState,
    kind: Option<DatabaseKind>,
    read_only: bool,
    action_busy: bool,
    tab_id: TabId,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> AnyElement {
    let Some(activity) = panel.activity.as_ref() else {
        return div()
            .size_full()
            .min_h(px(0.))
            .flex()
            .child(center_loading_message(tr("app.server_panel.loading"), colors))
            .into_any_element();
    };
    let maintenance_disabled = read_only || action_busy;
    let maintenance = match kind {
        Some(kind) if kind.is_postgres_family() => vec![
            (ServerMaintenanceAction::Vacuum, tr("app.server_panel.maintenance.vacuum").to_string()),
            (ServerMaintenanceAction::Analyze, tr("app.server_panel.maintenance.analyze").to_string()),
            (ServerMaintenanceAction::Reindex, tr("app.server_panel.maintenance.reindex").to_string()),
        ],
        Some(kind) if kind.is_mysql_family() => vec![
            (ServerMaintenanceAction::Analyze, tr("app.server_panel.maintenance.analyze").to_string()),
            (ServerMaintenanceAction::Optimize, tr("app.server_panel.maintenance.optimize").to_string()),
        ],
        Some(DatabaseKind::SqlServer) | Some(DatabaseKind::Oracle) => vec![
            (ServerMaintenanceAction::Analyze, tr("app.server_panel.maintenance.analyze").to_string()),
            (ServerMaintenanceAction::Reindex, tr("app.server_panel.maintenance.reindex").to_string()),
        ],
        // 达梦暂未验证统计信息/索引重建的等价命令，不展示维护入口。
        _ => Vec::new(),
    };

    let mut view = div()
        .id("server-panel-status")
        .debug_selector(|| "server-panel-status".into())
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
                .gap_3()
                .child(server_status_fact(
                    tr("app.server_panel.status.version").to_string(),
                    server_or_unknown(&activity.version),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.server_panel.status.current_user").to_string(),
                    server_or_unknown(&activity.current_user),
                    colors,
                ))
                .child(server_status_fact(
                    tr("app.server_panel.status.uptime").to_string(),
                    activity
                        .status
                        .uptime_secs
                        .map(server_format_secs)
                        .unwrap_or_else(|| "-".to_string()),
                    colors,
                ))
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .children(maintenance.into_iter().map(|(action, label)| {
                            server_small_button(
                                "server-panel-maintenance",
                                label,
                                AppIcon::Play,
                                !maintenance_disabled,
                                false,
                                colors,
                            )
                            .id(gpui::ElementId::Name(
                                format!("server-panel-maintenance-{}-{}", tab_id.0, action.as_str())
                                    .into(),
                            ))
                            .debug_selector({
                                let debug_action = format!(
                                    "server-panel-maintenance-{}-{}",
                                    tab_id.0,
                                    action.as_str()
                                );
                                move || debug_action.clone()
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.request_server_maintenance(tab_id, action, cx);
                                    cx.stop_propagation();
                                }),
                            )
                        })),
                ),
        );
    if read_only {
        view = view.child(
            div()
                .text_size(px(11.))
                .text_color(colors.muted)
                .child(tr("app.server_panel.maintenance.read_only_hint").to_string()),
        );
    }
    if let Some(error) = activity.status_error.as_ref() {
        view = view.child(server_inline_error(
            tr("app.server_panel.status_error").replace("%{message}", error),
            colors,
        ));
    }
    view.child(
        div()
            .rounded(colors.radius_lg)
            .border_1()
            .border_color(colors.border)
            .overflow_hidden()
            .child(server_table_header(
                colors,
                vec![
                    (tr("app.server_panel.col.metric").to_string(), 320., false),
                    (tr("app.server_panel.col.value").to_string(), 0., true),
                ],
            ))
            .children(
                activity
                    .status
                    .metrics
                    .iter()
                    .map(|metric| server_metric_row(metric, colors)),
            ),
    )
    .into_any_element()
}

fn server_metric_row(metric: &ServerMetric, colors: UiColors) -> Div {
    let value = if metric.key == "temp_bytes" {
        metric
            .value
            .parse::<u64>()
            .map(fluxdb_core::format_byte_length)
            .unwrap_or_else(|_| metric.value.clone())
    } else if metric.key == "physical_memory_kb" {
        metric
            .value
            .parse::<u64>()
            .map(|kb| fluxdb_core::format_byte_length(kb.saturating_mul(1024)))
            .unwrap_or_else(|_| metric.value.clone())
    } else {
        metric.value.clone()
    };
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
            server_metric_label(&metric.key),
            320.,
            false,
            if metric.warning { colors.warning } else { colors.text },
        ))
        .child(server_cell(value, 0., true, colors.text))
}
