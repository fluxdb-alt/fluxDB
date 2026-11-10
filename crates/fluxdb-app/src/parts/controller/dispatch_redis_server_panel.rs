impl AppController {
    // Redis 服务器面板独立成帧：读取与终止/清空命令不占用公共分派器栈空间。
    fn dispatch_redis_server_panel_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::StartRedisServerPanelLoad(tab_id) => {
                match self.redis_server_panel_state_mut(tab_id) {
                    Some(panel) => {
                        panel.loading = true;
                        panel.error = None;
                        AppEvent::TabActivated(tab_id)
                    }
                    None => self.fail(Error::new(ErrorKind::Internal, "Redis 服务器标签页不存在")),
                }
            }
            // 只读连接器读取，不改 AppState；桌面端在后台线程执行本条命令。
            AppCommand::LoadRedisServerPanel(tab_id) => {
                let Some(panel) = self.redis_server_panel_state(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "Redis 服务器标签页不存在"));
                };
                let connection_id = panel.connection_id;
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let result = fluxdb_connectors::connector_for(&config)
                    .and_then(|connector| connector.redis_server_activity(connection_id))
                    .map_err(UserFacingError::from);
                AppEvent::RedisServerPanelLoaded { tab_id, result }
            }
            AppCommand::FinishRedisServerPanelLoad { tab_id, result } => {
                let Some(panel) = self.redis_server_panel_state_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "Redis 服务器标签页不存在"));
                };
                panel.loading = false;
                match result {
                    Ok(activity) => {
                        panel.last_loaded_unix_secs = unix_now_secs();
                        panel.error = None;
                        panel.activity = Some(activity);
                    }
                    Err(error) => {
                        // 保留上一次成功的数据，局部错误只落在面板内，不升级为全局失败。
                        panel.error = Some(error);
                    }
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetRedisServerPanelTab { tab_id, tab } => {
                if let Some(panel) = self.redis_server_panel_state_mut(tab_id) {
                    panel.active_tab = tab;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetRedisServerPanelAutoRefresh { tab_id, enabled } => {
                if let Some(panel) = self.redis_server_panel_state_mut(tab_id) {
                    panel.auto_refresh = enabled;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::RedisKillClient { tab_id, client_id } => self
                .run_redis_server_panel_action(tab_id, "kill_client", |connector, config| {
                    connector
                        .redis_kill_client(config.id, &client_id)
                        .map(|_| None)
                }),
            AppCommand::RedisClearSlowlog { tab_id } => self.run_redis_server_panel_action(
                tab_id,
                "clear_slowlog",
                |connector, config| connector.redis_clear_slowlog(config.id).map(|_| None),
            ),
            _ => unreachable!("非 Redis 服务器面板命令不应进入 dispatch_redis_server_panel_command"),
        }
    }

    /// 终止客户端 / 清空慢日志的共用入口：只读连接统一拦截，
    /// 结果回传专用事件而不污染全局错误。
    fn run_redis_server_panel_action<F>(&mut self, tab_id: TabId, action: &str, run: F) -> AppEvent
    where
        F: FnOnce(
            &dyn Connector,
            &ConnectionConfig,
        ) -> fluxdb_core::Result<Option<String>>,
    {
        let Some(panel) = self.redis_server_panel_state(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "Redis 服务器标签页不存在"));
        };
        let connection_id = panel.connection_id;
        let Some(config) = self.connection_config(connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        if let Err(error) = ensure_read_only_allows(&config, WriteIntent::Mutation) {
            return AppEvent::RedisServerPanelActionFinished {
                tab_id,
                action: action.to_string(),
                result: Err(UserFacingError::from(error)),
            };
        }
        let result = fluxdb_connectors::connector_for(&config)
            .and_then(|connector| run(connector.as_ref(), &config))
            .map(|message| message.unwrap_or_default())
            .map_err(UserFacingError::from);
        AppEvent::RedisServerPanelActionFinished {
            tab_id,
            action: action.to_string(),
            result,
        }
    }

    fn redis_server_panel_state(&self, tab_id: TabId) -> Option<&RedisServerPanelState> {
        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::RedisServerPanel(panel) => Some(panel),
            _ => None,
        })
    }

    fn redis_server_panel_state_mut(
        &mut self,
        tab_id: TabId,
    ) -> Option<&mut RedisServerPanelState> {
        self.find_tab_mut(tab_id).and_then(|tab| match &mut tab.kind {
            TabKind::RedisServerPanel(panel) => Some(panel),
            _ => None,
        })
    }
}
