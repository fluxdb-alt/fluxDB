// 连接级「服务器」面板：会话、锁等待、实例指标与维护。
//
// 数据只来自连接器的只读读取；终止查询/会话与维护动作都经 `AppCommand`，
// 只读连接在 UI、控制器和连接器三层都拒绝执行。

const SERVER_PANEL_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

impl NavicatMain {
    fn open_server_panel_for_connection(
        &mut self,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        match self.dispatch(AppCommand::OpenServerPanel(connection_id), cx) {
            AppEvent::TabOpened(tab_id) | AppEvent::TabActivated(tab_id) => {
                self.start_server_panel_load(tab_id, cx);
                self.start_server_panel_refresh_loop(tab_id, cx);
            }
            _ => {}
        }
    }

    fn server_panel_state_for(&self, tab_id: TabId) -> Option<&ServerPanelState> {
        self.controller.state().tabs.iter().find_map(|tab| {
            if tab.id != tab_id {
                return None;
            }
            match &tab.kind {
                TabKind::ServerPanel(panel) => Some(panel),
                _ => None,
            }
        })
    }

    fn server_panel_action_busy(&self, tab_id: TabId) -> bool {
        self._server_panel_action_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
    }

    fn server_panel_is_read_only(&self, tab_id: TabId) -> bool {
        self.server_panel_state_for(tab_id).is_some_and(|panel| {
            self.controller
                .state()
                .connections
                .iter()
                .find(|connection| connection.config.id == panel.connection_id)
                .is_some_and(|connection| connection.config.is_read_only())
        })
    }

    /// UI 已禁用按钮，这里再兜底一次：只读、忙或目标为面板自身会话时不打开确认框。
    fn can_request_server_kill(&self, tab_id: TabId, session_id: &str) -> bool {
        if self.server_panel_action_busy(tab_id) || self.server_panel_is_read_only(tab_id) {
            return false;
        }
        self.server_panel_state_for(tab_id).is_some_and(|panel| {
            panel
                .activity
                .as_ref()
                .and_then(|activity| {
                    activity
                        .sessions
                        .iter()
                        .find(|session| session.id == session_id)
                })
                .is_some_and(|session| !session.is_self)
        })
    }

    /// 后台读取会话 / 锁 / 指标；同一页签不并发读取。
    fn start_server_panel_load(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self
            ._server_panel_load_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
        {
            return;
        }
        self.dispatch(AppCommand::StartServerPanelLoad(tab_id), cx);
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    match controller.dispatch(AppCommand::LoadServerPanel(tab_id)) {
                        AppEvent::ServerPanelLoaded { result, .. } => result,
                        AppEvent::Failed(error) => Err(error),
                        _ => Err(fluxdb_core::UserFacingError {
                            title: tr("app.server_panel.error_title").to_string(),
                            message: tr("app.server_panel.error_unknown").to_string(),
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
                    this.dispatch(AppCommand::FinishServerPanelLoad { tab_id, result }, cx);
                    cx.notify();
                });
            });
        });
        self._server_panel_load_tasks.insert(tab_id, task);
    }

    /// 5 秒自动刷新；暂停或页签关闭后下一拍自然结束。
    fn start_server_panel_refresh_loop(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if self
            ._server_panel_refresh_tasks
            .get(&tab_id)
            .is_some_and(|task| !task.is_ready())
        {
            return;
        }
        let task = cx.spawn(async move |view, cx| loop {
            cx.background_executor()
                .timer(SERVER_PANEL_REFRESH_INTERVAL)
                .await;
            let keep = view
                .update(cx, |this, cx| {
                    let Some(panel) = this.server_panel_state_for(tab_id) else {
                        return false;
                    };
                    if !panel.auto_refresh {
                        return false;
                    }
                    this.start_server_panel_load(tab_id, cx);
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        });
        self._server_panel_refresh_tasks.insert(tab_id, task);
    }

    fn set_server_panel_auto_refresh(&mut self, tab_id: TabId, enabled: bool, cx: &mut Context<Self>) {
        self.dispatch(
            AppCommand::SetServerPanelAutoRefresh { tab_id, enabled },
            cx,
        );
        if enabled {
            self.start_server_panel_refresh_loop(tab_id, cx);
            self.start_server_panel_load(tab_id, cx);
        }
    }

    /// 终止查询/会话与维护共用后台任务：先确认只读，完成后刷新面板数据。
    fn start_server_panel_action(
        &mut self,
        tab_id: TabId,
        action: &'static str,
        command: AppCommand,
        cx: &mut Context<Self>,
    ) {
        if self.server_panel_action_busy(tab_id) {
            return;
        }
        let mut controller = self.controller.clone();
        let task = cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move { controller.dispatch(command) })
                .await;
            let (action, result) = match event {
                AppEvent::ServerPanelActionFinished { action, result, .. } => (action, result),
                AppEvent::Failed(error) => (action.to_string(), Err(error)),
                _ => (
                    action.to_string(),
                    Err(fluxdb_core::UserFacingError {
                        title: tr("app.server_panel.action_failed").to_string(),
                        message: tr("app.server_panel.error_unknown").to_string(),
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
                            } else if action == "cancel_query" {
                                tr("app.server_panel.action.cancel_query_done").to_string()
                            } else if action == "kill_session" {
                                tr("app.server_panel.action.kill_session_done").to_string()
                            } else {
                                tr("app.server_panel.action.maintenance_done").to_string()
                            };
                            this.show_message(text, AppMessageKind::Success, cx);
                        }
                        Err(error) => {
                            this.show_message(
                                format!("{}: {}", tr("app.server_panel.action_failed"), error.message),
                                AppMessageKind::Error,
                                cx,
                            );
                        }
                    }
                    this.start_server_panel_load(tab_id, cx);
                    cx.notify();
                });
            });
        });
        self._server_panel_action_tasks.insert(tab_id, task);
    }

    fn request_server_kill(
        &mut self,
        tab_id: TabId,
        session_id: String,
        kind: ServerKillKind,
        cx: &mut Context<Self>,
    ) {
        if !self.can_request_server_kill(tab_id, &session_id) {
            return;
        }
        self.server_kill_confirm = Some(ServerKillConfirm {
            tab_id,
            session_id,
            kind,
            checked: false,
        });
        cx.notify();
    }

    fn confirm_server_kill(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.server_kill_confirm.clone() else {
            return;
        };
        if !confirm.checked {
            return;
        }
        self.server_kill_confirm = None;
        let command = match confirm.kind {
            ServerKillKind::Query => AppCommand::CancelServerQuery {
                tab_id: confirm.tab_id,
                session_id: confirm.session_id,
            },
            ServerKillKind::Session => AppCommand::KillServerSession {
                tab_id: confirm.tab_id,
                session_id: confirm.session_id,
            },
        };
        let action = match confirm.kind {
            ServerKillKind::Query => "cancel_query",
            ServerKillKind::Session => "kill_session",
        };
        self.start_server_panel_action(confirm.tab_id, action, command, cx);
        cx.notify();
    }

    fn request_server_maintenance(
        &mut self,
        tab_id: TabId,
        action: ServerMaintenanceAction,
        cx: &mut Context<Self>,
    ) {
        if self.server_panel_action_busy(tab_id) || self.server_panel_is_read_only(tab_id) {
            return;
        }
        self.server_maintenance_confirm = Some(ServerMaintenanceConfirm { tab_id, action });
        cx.notify();
    }

    fn confirm_server_maintenance(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.server_maintenance_confirm.take() else {
            return;
        };
        self.start_server_panel_action(
            confirm.tab_id,
            confirm.action.as_str(),
            AppCommand::RunServerMaintenance {
                tab_id: confirm.tab_id,
                action: confirm.action,
            },
            cx,
        );
        cx.notify();
    }

    fn server_panel_open_session_editor(&mut self, tab_id: TabId, sql: &str, cx: &mut Context<Self>) {
        let Some(panel) = self.server_panel_state_for(tab_id) else {
            return;
        };
        let database = panel.database.clone();
        let connection_id = panel.connection_id;
        let event = self.dispatch(
            AppCommand::OpenQueryEditorInDatabase {
                connection_id,
                database: (!database.trim().is_empty()).then_some(database),
                schema: None,
            },
            cx,
        );
        if let AppEvent::TabOpened(editor_id) | AppEvent::TabActivated(editor_id) = event {
            self.dispatch(
                AppCommand::UpdateQueryText {
                    tab_id: editor_id,
                    text: sql.to_string(),
                },
                cx,
            );
        }
    }
}

fn server_panel_content(
    state: &AppState,
    tab_id: TabId,
    panel: &ServerPanelState,
    this: &mut NavicatMain,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let read_only = state
        .connections
        .iter()
        .find(|connection| connection.config.id == panel.connection_id)
        .is_some_and(|connection| connection.config.is_read_only());
    let kind = state
        .connections
        .iter()
        .find(|connection| connection.config.id == panel.connection_id)
        .map(|connection| connection.config.kind);
    let activity = panel.activity.as_ref();
    let session_count = activity.map(|activity| activity.sessions.len()).unwrap_or(0);
    let lock_count = activity.map(|activity| activity.locks.len()).unwrap_or(0);
    let action_busy = this.server_panel_action_busy(tab_id);

    div()
        .id("server-panel")
        .debug_selector(|| "server-panel".into())
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
                                .child(server_panel_identity(activity, panel)),
                        ),
                )
                .when(read_only, |this| {
                    this.child(
                        server_badge(
                            tr("app.server_panel.read_only").to_string(),
                            colors.warning,
                            colors,
                        )
                        .debug_selector(|| "server-panel-read-only".into()),
                    )
                })
                .child(div().flex_1())
                .child(server_count_badge(
                    tr("app.server_panel.count.sessions").replace("%{count}", &session_count.to_string()),
                    colors,
                ))
                .child(server_count_badge(
                    tr("app.server_panel.count.locks").replace("%{count}", &lock_count.to_string()),
                    colors,
                ))
                .child(server_toolbar_button(
                    "server-panel-refresh",
                    AppIcon::Refresh,
                    tr("app.server_panel.refresh").to_string(),
                    false,
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.start_server_panel_load(tab_id, cx);
                        cx.stop_propagation();
                    }),
                ))
                .child(server_toggle(
                    ("server-panel-auto", tab_id.0),
                    tr("app.server_panel.auto_refresh").to_string(),
                    panel.auto_refresh,
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        let enabled = this
                            .server_panel_state_for(tab_id)
                            .is_some_and(|panel| !panel.auto_refresh);
                        this.set_server_panel_auto_refresh(tab_id, enabled, cx);
                        cx.stop_propagation();
                    }),
                ))
                .child(server_toggle(
                    ("server-panel-only-mine", tab_id.0),
                    tr("app.server_panel.only_mine").to_string(),
                    panel.only_mine,
                    colors,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        let only_mine = this
                            .server_panel_state_for(tab_id)
                            .is_some_and(|panel| !panel.only_mine);
                        this.dispatch(
                            AppCommand::SetServerPanelOnlyMine { tab_id, only_mine },
                            cx,
                        );
                        cx.stop_propagation();
                    }),
                )),
        )
        .child(server_panel_subtabs(tab_id, panel, colors, cx))
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
                            .child(format!("{}: {}", tr("app.server_panel.error_title"), error.message)),
                    )
                    .child(server_toolbar_button(
                        "server-panel-retry",
                        AppIcon::Refresh,
                        tr("app.server_panel.retry").to_string(),
                        false,
                        colors,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.start_server_panel_load(tab_id, cx);
                            cx.stop_propagation();
                        }),
                    )),
            )
        })
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(match panel.active_tab {
                    ServerPanelTab::Sessions => {
                        server_sessions_view(tab_id, panel, read_only, kind, colors, cx)
                            .into_any_element()
                    }
                    ServerPanelTab::Locks => {
                        server_locks_view(tab_id, panel, colors).into_any_element()
                    }
                    ServerPanelTab::Status => {
                        server_status_view(panel, kind, read_only, action_busy, tab_id, colors, cx)
                    }
                }),
        )
        .when_some(this.server_kill_confirm.clone(), |this, confirm| {
            this.child(server_kill_modal(confirm, panel, colors, cx))
        })
        .when_some(
            this.server_maintenance_confirm,
            |this, confirm| this.child(server_maintenance_modal(confirm, panel, colors, cx)),
        )
}

fn server_panel_identity(activity: Option<&ServerActivity>, panel: &ServerPanelState) -> String {
    let database = if panel.database.trim().is_empty() {
        tr("app.server_panel.no_database").to_string()
    } else {
        panel.database.clone()
    };
    match activity {
        Some(activity) if !activity.version.trim().is_empty() => format!(
            "{} · {} · {}",
            activity.version,
            database,
            if activity.current_user.trim().is_empty() {
                tr("app.server_panel.unknown").to_string()
            } else {
                activity.current_user.clone()
            }
        ),
        _ => database,
    }
}

fn server_panel_subtabs(
    tab_id: TabId,
    panel: &ServerPanelState,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let item = |tab: ServerPanelTab, label: String| {
        let active = panel.active_tab == tab;
        let debug_subtab =
            format!("server-panel-subtab-{}-{}", tab_id.0, server_panel_tab_index(tab));
        div()
            .id(gpui::ElementId::Name(
                format!("server-panel-subtab-{}-{}", tab_id.0, server_panel_tab_index(tab)).into(),
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
                    this.dispatch(AppCommand::SetServerPanelTab { tab_id, tab }, cx);
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
            ServerPanelTab::Sessions,
            tr("app.server_panel.tab.sessions").to_string(),
        ))
        .child(item(
            ServerPanelTab::Locks,
            tr("app.server_panel.tab.locks").to_string(),
        ))
        .child(item(
            ServerPanelTab::Status,
            tr("app.server_panel.tab.status").to_string(),
        ))
}

fn server_panel_tab_index(tab: ServerPanelTab) -> u8 {
    match tab {
        ServerPanelTab::Sessions => 0,
        ServerPanelTab::Locks => 1,
        ServerPanelTab::Status => 2,
    }
}

fn server_sessions_view(
    tab_id: TabId,
    panel: &ServerPanelState,
    read_only: bool,
    kind: Option<DatabaseKind>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let sessions = panel.visible_sessions();
    let selected = panel.selected_session_detail().cloned();
    let table: AnyElement = if let Some(_activity) = panel.activity.as_ref() {
        if sessions.is_empty() {
            server_empty(tr("app.server_panel.empty_sessions").to_string(), colors)
                .into_any_element()
        } else {
            div()
                .id("server-panel-sessions")
                .debug_selector(|| "server-panel-sessions".into())
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .overflow_scrollbar()
                .child(
                    div()
                        .min_w(px(1120.))
                        .child(server_sessions_header(colors))
                        .children(sessions.into_iter().map(|session| {
                            let is_selected = panel.selected_session.as_deref() == Some(session.id.as_str());
                            let session_id = session.id.clone();
                            server_session_row(
                                tab_id,
                                session,
                                is_selected,
                                colors,
                                cx,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.dispatch(
                                        AppCommand::SelectServerPanelSession {
                                            tab_id,
                                            session_id: Some(session_id.clone()),
                                        },
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            )
                        })),
                )
                .into_any_element()
        }
    } else {
        div()
            .size_full()
            .min_h(px(0.))
            .flex()
            .child(center_loading_message(tr("app.server_panel.loading"), colors))
            .into_any_element()
    };

    let mut body = div()
        .size_full()
        .min_h(px(0.))
        .flex()
        .flex_row()
        .gap_0()
        .px_4()
        .py_3()
        .child(table);
    if let (Some(activity), Some(detail)) = (panel.activity.as_ref(), selected) {
        body = body.child(server_session_inspector(
            tab_id,
            activity,
            &detail,
            read_only,
            kind,
            colors,
            cx,
        ));
    }
    body
}

fn server_sessions_header(colors: UiColors) -> Div {
    server_table_header(
        colors,
        vec![
            (tr("app.server_panel.col.status").to_string(), 86., false),
            (tr("app.server_panel.col.session").to_string(), 66., false),
            (tr("app.server_panel.col.user").to_string(), 110., false),
            (tr("app.server_panel.col.host").to_string(), 150., false),
            (tr("app.server_panel.col.database").to_string(), 110., false),
            (tr("app.server_panel.col.command").to_string(), 90., false),
            (tr("app.server_panel.col.elapsed").to_string(), 72., false),
            (tr("app.server_panel.col.wait_event").to_string(), 150., false),
            (tr("app.server_panel.col.sql").to_string(), 0., true),
        ],
    )
}

fn server_session_row(
    tab_id: TabId,
    session: &ServerSession,
    selected: bool,
    colors: UiColors,
    _cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let status = server_session_status(session);
    let status_color = if status == tr("app.server_panel.status.waiting") {
        colors.warning
    } else if status == tr("app.server_panel.status.running") {
        colors.info
    } else {
        colors.muted
    };
    let debug_row = format!("server-session-row-{}-{}", tab_id.0, session.id);
    div()
        .id(gpui::ElementId::Name(
            format!("server-session-row-{}-{}", tab_id.0, session.id).into(),
        ))
        .debug_selector(move || debug_row.clone())
        .h(px(34.))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .cursor_pointer()
        .border_b_1()
        .border_color(colors.border_soft)
        .when(selected, |this| this.bg(colors.tree_selected))
        .hover(move |style| style.bg(colors.hover))
        .child(server_cell(status, 86., false, status_color))
        .child(server_cell(
            if session.is_self {
                format!("{} (me)", session.id)
            } else {
                session.id.clone()
            },
            66.,
            false,
            colors.text,
        ))
        .child(server_cell(session.user.clone(), 110., false, colors.text))
        .child(server_cell(session.host.clone(), 150., false, colors.muted))
        .child(server_cell(session.database.clone(), 110., false, colors.muted))
        .child(server_cell(session.command.clone(), 90., false, colors.muted))
        .child(server_cell(
            server_format_secs(session.elapsed_secs),
            72.,
            false,
            colors.muted,
        ))
        .child(server_cell(
            session.wait_event.clone(),
            150.,
            false,
            colors.muted,
        ))
        .child(server_cell(
            server_sql_preview(&session.sql, 240),
            0.,
            true,
            colors.text,
        ))
}

fn server_session_inspector(
    tab_id: TabId,
    activity: &ServerActivity,
    session: &ServerSession,
    read_only: bool,
    kind: Option<DatabaseKind>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    let can_kill = !read_only && !session.is_self;
    // SQL Server 没有「只终止查询」命令；Oracle 的取消查询会被驱动卡住，
    // 两者都只提供终止会话，避免按钮产生错误语义或挂住查询线程。
    let can_cancel_query = can_kill
        && kind != Some(DatabaseKind::SqlServer)
        && kind != Some(DatabaseKind::Oracle);
    let sql = session.sql.clone();
    let copy_sql = sql.clone();
    let editor_sql = sql.clone();
    let session_id = session.id.clone();
    let session_id_query = session.id.clone();
    let blocker = activity.locks.iter().find(|lock| {
        lock.waiting_session == session.id || lock.blocking_session == session.id
    });
    div()
        .id("server-panel-detail")
        .debug_selector(|| "server-panel-detail".into())
        .w(px(360.))
        .flex_none()
        .min_h(px(0.))
        .ml_3()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .bg(colors.panel_alt)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(38.))
                .flex_none()
                .px_3()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(colors.border)
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr("app.server_panel.detail.title")),
                )
                .child(server_badge(
                    format!("#{}", session.id),
                    colors.info,
                    colors,
                )),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scrollbar()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(server_detail_row(
                    tr("app.server_panel.col.user").to_string(),
                    session.user.clone(),
                    colors,
                ))
                .child(server_detail_row(
                    tr("app.server_panel.col.host").to_string(),
                    session.host.clone(),
                    colors,
                ))
                .child(server_detail_row(
                    tr("app.server_panel.col.database").to_string(),
                    session.database.clone(),
                    colors,
                ))
                .child(server_detail_row(
                    tr("app.server_panel.col.command").to_string(),
                    session.command.clone(),
                    colors,
                ))
                .child(server_detail_row(
                    tr("app.server_panel.detail.elapsed").to_string(),
                    server_format_secs(session.elapsed_secs),
                    colors,
                ))
                .child(server_detail_row(
                    tr("app.server_panel.col.wait_event").to_string(),
                    session.wait_event.clone(),
                    colors,
                ))
                .when_some(blocker, |this, lock| {
                    this.child(server_detail_row(
                        tr("app.server_panel.detail.blocker").to_string(),
                        format!(
                            "{} → {} · {}",
                            lock.waiting_session,
                            lock.blocking_session,
                            server_lock_type_label(&lock.lock_type)
                        ),
                        colors,
                    ))
                })
                .child(server_sql_block(
                    tr("app.server_panel.detail.sql").to_string(),
                    &sql,
                    colors,
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(server_small_button(
                            "server-panel-copy-sql",
                            tr("app.server_panel.action.copy_sql").to_string(),
                            AppIcon::Copy,
                            !sql.trim().is_empty(),
                            false,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_sql.clone()));
                                this.show_message(
                                    tr("app.server_panel.action.copy_sql_done").to_string(),
                                    AppMessageKind::Success,
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        ))
                        .child(server_small_button(
                            "server-panel-open-editor",
                            tr("app.server_panel.action.open_editor").to_string(),
                            AppIcon::Query,
                            !editor_sql.trim().is_empty(),
                            false,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.server_panel_open_session_editor(tab_id, &editor_sql, cx);
                                cx.stop_propagation();
                            }),
                        ))
                        .when(can_cancel_query, |this| {
                            this.child(server_small_button(
                                "server-panel-cancel-query",
                                tr("app.server_panel.action.cancel_query").to_string(),
                                AppIcon::CircleSlash,
                                true,
                                false,
                                colors,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.request_server_kill(
                                        tab_id,
                                        session_id_query.clone(),
                                        ServerKillKind::Query,
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            ))
                        })
                        .child(server_small_button(
                            "server-panel-kill-session",
                            tr("app.server_panel.action.kill_session").to_string(),
                            AppIcon::Close,
                            can_kill,
                            true,
                            colors,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.request_server_kill(
                                    tab_id,
                                    session_id.clone(),
                                    ServerKillKind::Session,
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        )),
                )
                .when(!can_kill, |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(if read_only {
                                tr("app.server_panel.read_only_hint").to_string()
                            } else {
                                tr("app.server_panel.self_hint").to_string()
                            }),
                    )
                }),
        )
}
