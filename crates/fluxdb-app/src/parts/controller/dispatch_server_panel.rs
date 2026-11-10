impl AppController {
    // 服务器面板独立成帧：后台读取与终止/维护命令不占用公共分派器栈空间。
    fn dispatch_server_panel_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::StartServerPanelLoad(tab_id) => match self.server_panel_state_mut(tab_id) {
                Some(panel) => {
                    panel.loading = true;
                    panel.error = None;
                    AppEvent::TabActivated(tab_id)
                }
                None => self.fail(Error::new(ErrorKind::Internal, "服务器标签页不存在")),
            },
            // 只读连接器读取，不改 AppState；桌面端在后台线程执行本条命令。
            AppCommand::LoadServerPanel(tab_id) => {
                let Some(panel) = self.server_panel_state(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "服务器标签页不存在"));
                };
                let connection_id = panel.connection_id;
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let result = fluxdb_connectors::connector_for(&config)
                    .and_then(|connector| connector.server_activity(connection_id))
                    .map_err(UserFacingError::from);
                AppEvent::ServerPanelLoaded { tab_id, result }
            }
            AppCommand::FinishServerPanelLoad { tab_id, result } => {
                let Some(panel) = self.server_panel_state_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "服务器标签页不存在"));
                };
                panel.loading = false;
                match result {
                    Ok(activity) => {
                        panel.last_loaded_unix_secs = unix_now_secs();
                        panel.error = None;
                        panel.activity = Some(activity);
                        let still_visible = panel.selected_session.as_deref().is_some_and(|id| {
                            panel
                                .visible_sessions()
                                .iter()
                                .any(|session| session.id == id)
                        });
                        if !still_visible {
                            // 选中会话已消失或被过滤时，退回到当前可见的第一条。
                            panel.selected_session = panel
                                .visible_sessions()
                                .first()
                                .map(|session| session.id.clone());
                        }
                    }
                    Err(error) => {
                        // 保留上一次成功的数据，局部错误只落在面板内，不升级为全局失败。
                        panel.error = Some(error);
                    }
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetServerPanelTab { tab_id, tab } => {
                if let Some(panel) = self.server_panel_state_mut(tab_id) {
                    panel.active_tab = tab;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SelectServerPanelSession { tab_id, session_id } => {
                if let Some(panel) = self.server_panel_state_mut(tab_id) {
                    panel.selected_session = session_id;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetServerPanelOnlyMine { tab_id, only_mine } => {
                if let Some(panel) = self.server_panel_state_mut(tab_id) {
                    panel.only_mine = only_mine;
                    // 过滤后选中项可能已不可见，避免详情区展示一条列表外的会话。
                    let selected_visible = panel.selected_session.as_deref().is_some_and(|id| {
                        panel
                            .visible_sessions()
                            .iter()
                            .any(|session| session.id == id)
                    });
                    if !selected_visible {
                        panel.selected_session = panel
                            .visible_sessions()
                            .first()
                            .map(|session| session.id.clone());
                    }
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetServerPanelAutoRefresh { tab_id, enabled } => {
                if let Some(panel) = self.server_panel_state_mut(tab_id) {
                    panel.auto_refresh = enabled;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::CancelServerQuery { tab_id, session_id } => self
                .run_server_panel_action(tab_id, "cancel_query", |connector, config| {
                    connector
                        .cancel_server_query(config.id, &session_id)
                        .map(|_| None)
                }),
            AppCommand::KillServerSession { tab_id, session_id } => self
                .run_server_panel_action(tab_id, "kill_session", |connector, config| {
                    connector
                        .kill_server_session(config.id, &session_id)
                        .map(|_| None)
                }),
            AppCommand::RunServerMaintenance { tab_id, action } => {
                let database = match self.server_panel_state(tab_id) {
                    Some(panel) => panel.database.clone(),
                    None => {
                        return self.fail(Error::new(ErrorKind::Internal, "服务器标签页不存在"));
                    }
                };
                self.run_server_panel_action(tab_id, action.as_str(), |connector, config| {
                    connector
                        .run_server_maintenance(&ServerMaintenanceRequest {
                            connection_id: config.id,
                            database: database.clone(),
                            action,
                        })
                        .map(Some)
                })
            }
            _ => unreachable!("非服务器面板命令不应进入 dispatch_server_panel_command"),
        }
    }

    /// 终止/维护的共用入口：只读连接统一拦截，结果回传专用事件而不污染全局错误。
    fn run_server_panel_action<F>(&mut self, tab_id: TabId, action: &str, run: F) -> AppEvent
    where
        F: FnOnce(
            &dyn Connector,
            &ConnectionConfig,
        ) -> fluxdb_core::Result<Option<String>>,
    {
        let Some(panel) = self.server_panel_state(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "服务器标签页不存在"));
        };
        let connection_id = panel.connection_id;
        let Some(config) = self.connection_config(connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        // 只读连接不得终止他人会话，也不得执行维护 DDL；连接器侧还有一道同类保护。
        if let Err(error) = ensure_read_only_allows(&config, WriteIntent::Mutation) {
            return AppEvent::ServerPanelActionFinished {
                tab_id,
                action: action.to_string(),
                result: Err(UserFacingError::from(error)),
            };
        }
        let result = fluxdb_connectors::connector_for(&config)
            .and_then(|connector| run(connector.as_ref(), &config))
            .map(|message| message.unwrap_or_default())
            .map_err(UserFacingError::from);
        AppEvent::ServerPanelActionFinished {
            tab_id,
            action: action.to_string(),
            result,
        }
    }

    fn server_panel_state(&self, tab_id: TabId) -> Option<&ServerPanelState> {
        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::ServerPanel(panel) => Some(panel),
            _ => None,
        })
    }

    fn server_panel_state_mut(&mut self, tab_id: TabId) -> Option<&mut ServerPanelState> {
        self.find_tab_mut(tab_id).and_then(|tab| match &mut tab.kind {
            TabKind::ServerPanel(panel) => Some(panel),
            _ => None,
        })
    }
}

fn unix_now_secs() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}
