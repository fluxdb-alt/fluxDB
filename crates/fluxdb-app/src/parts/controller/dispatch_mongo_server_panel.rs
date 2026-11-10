// MongoDB 连接级服务器面板的 App 层实现。
//
// 形态对齐 Redis 服务器面板：后台只读拉取 `serverStatus` / `$currentOp`，
// 终止操作走 `killOp`，只读连接在控制器与连接器两层都拦截。

impl AppController {
    /// 打开（或复用）连接级 MongoDB 服务器面板：同一连接只保留一个。
    pub(crate) fn open_mongo_server_panel(&mut self, connection_id: ConnectionId) -> AppEvent {
        if let Some(tab) = self.state.tabs.iter().find(|tab| {
            matches!(
                &tab.kind,
                TabKind::MongoServerPanel(panel) if panel.connection_id == connection_id
            )
        }) {
            self.state.active_tab = Some(tab.id);
            return AppEvent::TabActivated(tab.id);
        }
        let Some(connection) = self
            .state
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
        else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        if connection.config.kind != DatabaseKind::MongoDb {
            return self.fail(Error::new(
                ErrorKind::Unsupported,
                "当前数据库暂不支持 MongoDB 服务器面板",
            ));
        }
        // 观测命令都跑在 admin；库名只用于页签上下文。
        let database = connection
            .config
            .mongo_profile
            .as_ref()
            .map(|profile| profile.basic.auth_database.trim().to_string())
            .filter(|database| !database.is_empty())
            .unwrap_or_else(|| "admin".to_string());
        let name = connection.config.name.clone();
        let tab_id = self.next_tab_id();
        self.push_tab(TabState {
            id: tab_id,
            title: format!("服务器 · {name}"),
            kind: TabKind::MongoServerPanel(MongoServerPanelState::new(
                connection_id,
                name,
                database,
            )),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    fn dispatch_mongo_server_panel_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::StartMongoServerPanelLoad(tab_id) => {
                match self.mongo_server_panel_state_mut(tab_id) {
                    Some(panel) => {
                        panel.loading = true;
                        panel.error = None;
                        AppEvent::TabActivated(tab_id)
                    }
                    None => self.fail(Error::new(ErrorKind::Internal, "MongoDB 服务器标签页不存在")),
                }
            }
            // 只读连接器读取，不改 AppState；桌面端在后台线程执行本条命令。
            AppCommand::LoadMongoServerPanel(tab_id) => {
                let Some(panel) = self.mongo_server_panel_state(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "MongoDB 服务器标签页不存在",
                    ));
                };
                let connection_id = panel.connection_id;
                let Some(config) = self.connection_config(connection_id).cloned() else {
                    return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
                };
                let result = fluxdb_connectors::connector_for(&config)
                    .and_then(|connector| connector.mongo_server_activity(connection_id))
                    .map_err(UserFacingError::from);
                AppEvent::MongoServerPanelLoaded { tab_id, result }
            }
            AppCommand::FinishMongoServerPanelLoad { tab_id, result } => {
                let Some(panel) = self.mongo_server_panel_state_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "MongoDB 服务器标签页不存在",
                    ));
                };
                panel.loading = false;
                match result {
                    Ok(activity) => {
                        if let Some(now) = unix_now_secs() {
                            panel.last_loaded_unix_secs = Some(now);
                            // 每次成功读取都落一个采样点，供「性能」子页画趋势。
                            panel.record_sample(now, &activity);
                        }
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
            AppCommand::SetMongoServerPanelTab { tab_id, tab } => {
                if let Some(panel) = self.mongo_server_panel_state_mut(tab_id) {
                    panel.active_tab = tab;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetMongoServerPanelOnlySlow { tab_id, only_slow } => {
                if let Some(panel) = self.mongo_server_panel_state_mut(tab_id) {
                    panel.only_slow = only_slow;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::SetMongoServerPanelAutoRefresh { tab_id, enabled } => {
                if let Some(panel) = self.mongo_server_panel_state_mut(tab_id) {
                    panel.auto_refresh = enabled;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::MongoKillOperation { tab_id, opid } => self.run_mongo_server_panel_action(
                tab_id,
                "kill_operation",
                |connector, config| {
                    connector
                        .mongo_kill_operation(config.id, &opid)
                        .map(|_| None)
                },
            ),
            _ => {
                unreachable!("非 MongoDB 服务器面板命令不应进入 dispatch_mongo_server_panel_command")
            }
        }
    }

    /// 终止操作的统一入口：只读连接统一拦截，结果回传专用事件而不污染全局错误。
    fn run_mongo_server_panel_action<F>(&mut self, tab_id: TabId, action: &str, run: F) -> AppEvent
    where
        F: FnOnce(&dyn Connector, &ConnectionConfig) -> fluxdb_core::Result<Option<String>>,
    {
        let Some(panel) = self.mongo_server_panel_state(tab_id) else {
            return self.fail(Error::new(ErrorKind::Internal, "MongoDB 服务器标签页不存在"));
        };
        let connection_id = panel.connection_id;
        let Some(config) = self.connection_config(connection_id).cloned() else {
            return self.fail(Error::new(ErrorKind::Connection, "连接不存在"));
        };
        if let Err(error) = ensure_read_only_allows(&config, WriteIntent::Mutation) {
            return AppEvent::MongoServerPanelActionFinished {
                tab_id,
                action: action.to_string(),
                result: Err(UserFacingError::from(error)),
            };
        }
        let result = fluxdb_connectors::connector_for(&config)
            .and_then(|connector| run(connector.as_ref(), &config))
            .map(|message| message.unwrap_or_default())
            .map_err(UserFacingError::from);
        AppEvent::MongoServerPanelActionFinished {
            tab_id,
            action: action.to_string(),
            result,
        }
    }

    fn mongo_server_panel_state(&self, tab_id: TabId) -> Option<&MongoServerPanelState> {
        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
            TabKind::MongoServerPanel(panel) => Some(panel),
            _ => None,
        })
    }

    fn mongo_server_panel_state_mut(
        &mut self,
        tab_id: TabId,
    ) -> Option<&mut MongoServerPanelState> {
        self.find_tab_mut(tab_id).and_then(|tab| match &mut tab.kind {
            TabKind::MongoServerPanel(panel) => Some(panel),
            _ => None,
        })
    }
}

#[cfg(test)]
mod mongo_server_sample_tests {
    use super::*;
    use fluxdb_core::ServerMetric;

    fn activity(ops: &[&str], connections: u64, slow: usize) -> MongoServerActivity {
        MongoServerActivity {
            connections_current: connections,
            metrics: ops
                .iter()
                .map(|value| ServerMetric {
                    key: "opcounters.query".to_string(),
                    value: (*value).to_string(),
                    warning: false,
                })
                .collect(),
            operations: (0..slow)
                .map(|index| MongoOperation {
                    opid: index.to_string(),
                    secs_running: 2,
                    ..MongoOperation::default()
                })
                .collect(),
            ..MongoServerActivity::default()
        }
    }

    #[test]
    fn sample_uses_cumulative_opcounters_and_caps_window() {
        let mut panel = MongoServerPanelState::new(ConnectionId(1), "m".into(), "admin".into());
        panel.record_sample(1_000, &activity(&["3", "4"], 5, 1));
        let sample = panel.samples.last().expect("sample");
        assert_eq!(sample.ops_total, 7, "opcounters 求和");
        assert_eq!(sample.connections, 5);
        assert_eq!(sample.slow_operations, 1);

        // 非 opcounters 指标不参与累计；
        panel.record_sample(1_005, &activity(&["not-a-number"], 6, 0));
        assert_eq!(panel.samples.last().expect("sample").ops_total, 0);

        for index in 0..MONGO_SERVER_SAMPLE_LIMIT {
            panel.record_sample(2_000 + index as u64, &activity(&["1"], 1, 0));
        }
        assert_eq!(
            panel.samples.len(),
            MONGO_SERVER_SAMPLE_LIMIT,
            "超出窗口丢最旧的"
        );
        assert_eq!(panel.samples.last().expect("sample").at_unix_secs, 2_000 + MONGO_SERVER_SAMPLE_LIMIT as u64 - 1);
    }
}
