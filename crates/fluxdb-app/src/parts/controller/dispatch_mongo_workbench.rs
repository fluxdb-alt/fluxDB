// MongoDB 命令工作台（库级、跨集合）的 App 层实现。
//
// 形态对齐 Redis Workbench：输入一条语句 → 追加一条执行记录，结果区可重跑 / 删除。
// 与 CLI（`MongoCliState`）的分工：CLI 是真实终端 REPL，工作台是「可回看的执行台」。
// 语句语法与只读边界完全交给连接器的 `mongo_shell`，这里只做状态与记录管理。
impl AppController {
    /// 打开（或复用）命令工作台标签页：同一连接 + 同一库只保留一个。
    pub(crate) fn open_mongo_workbench(
        &mut self,
        connection_id: ConnectionId,
        database: String,
    ) -> AppEvent {
        if let Some(tab) = self.state.tabs.iter().find(|tab| {
            matches!(
                &tab.kind,
                TabKind::MongoWorkbench(workbench)
                    if workbench.connection_id == connection_id && workbench.database == database
            )
        }) {
            self.state.active_tab = Some(tab.id);
            return AppEvent::TabActivated(tab.id);
        }
        let tab_id = self.next_tab_id();
        // 标题由展示层按语言包覆盖；这里给 AI 上下文与拖拽预览用稳定中文。
        let title = format!("MongoDB 命令工作台 - {database}");
        self.push_tab(TabState {
            id: tab_id,
            title,
            kind: TabKind::MongoWorkbench(MongoWorkbenchState::new(connection_id, database)),
            dirty: false,
        });
        AppEvent::TabOpened(tab_id)
    }

    fn mongo_workbench_mut(&mut self, tab_id: TabId) -> Option<&mut MongoWorkbenchState> {
        self.find_tab_mut(tab_id)
            .and_then(|tab| match &mut tab.kind {
                TabKind::MongoWorkbench(workbench) => Some(workbench),
                _ => None,
            })
    }

    /// 解析本次执行要用的库与语句文本。
    ///
    /// 文本按 `execution_id` 从状态里取，不接 UI 传值：`RunMongoWorkbench` 跑在后台线程的
    /// 控制器副本上，而主线程在派发前已经清空了草稿。
    fn mongo_workbench_run_scope(
        &self,
        tab_id: TabId,
        execution_id: Option<u64>,
    ) -> Option<(ConnectionId, String, String)> {
        let tab = self.find_tab(tab_id)?;
        let TabKind::MongoWorkbench(workbench) = &tab.kind else {
            return None;
        };
        let text = match execution_id {
            Some(execution_id) => workbench
                .executions
                .iter()
                .find(|execution| execution.id == execution_id)
                .map(|execution| execution.text.clone())?,
            None => workbench.text.clone(),
        };
        Some((workbench.connection_id, workbench.database.clone(), text))
    }

    pub(crate) fn dispatch_mongo_workbench_command(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::OpenMongoWorkbench {
                connection_id,
                database,
            } => self.open_mongo_workbench(connection_id, database),
            AppCommand::UpdateMongoWorkbenchText { tab_id, text } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "MongoDB 命令工作台标签页不存在"));
                };
                let TabKind::MongoWorkbench(workbench) = &mut tab.kind else {
                    return self.fail(Error::new(ErrorKind::Internal, "MongoDB 命令工作台标签页不存在"));
                };
                workbench.text = text;
                workbench.error = None;
                tab.dirty = workbench.has_unsaved_text();
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::BeginMongoWorkbenchExecution {
                tab_id,
                execution_id,
            } => {
                // 重跑要按 id 定位记录；记录已被删除时明确失败，不进入运行态。
                if execution_id.is_some()
                    && self.mongo_workbench_run_scope(tab_id, execution_id).is_none()
                {
                    return self.fail(Error::new(ErrorKind::Internal, "MongoDB 工作台记录不存在"));
                }
                let Some(workbench) = self.mongo_workbench_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "MongoDB 命令工作台标签页不存在"));
                };
                workbench.running = true;
                workbench.error = None;
                // 执行顶部草稿才清空输入框；重跑结果区记录不打扰当前草稿。
                if execution_id.is_none() {
                    workbench.text.clear();
                    workbench.saved_fingerprint = Some(QueryFingerprint::for_text(""));
                }
                let dirty = workbench.has_unsaved_text();
                if let Some(tab) = self.find_tab_mut(tab_id) {
                    tab.dirty = dirty;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::RunMongoWorkbench {
                tab_id,
                execution_id,
            } => {
                // 只执行、不写标签页状态：本命令跑在后台线程的控制器副本上。
                let Some((connection_id, database, text)) =
                    self.mongo_workbench_run_scope(tab_id, execution_id)
                else {
                    return self.fail(Error::new(ErrorKind::Internal, "MongoDB 命令工作台标签页不存在"));
                };
                // 空语句不往返数据库：直接按成功回一个空结果，避免把空行记成一次失败执行。
                if text.trim().is_empty() {
                    return AppEvent::MongoWorkbenchRan {
                        tab_id,
                        text,
                        result: Ok(MongoShellResult::default()),
                    };
                }
                let result = self
                    .connection_config(connection_id)
                    .cloned()
                    .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))
                    .and_then(|config| {
                        connector_for(&config)
                            .and_then(|connector| connector.mongo_shell(connection_id, &database, &text))
                    })
                    .map_err(UserFacingError::from);
                AppEvent::MongoWorkbenchRan {
                    tab_id,
                    text,
                    result,
                }
            }
            AppCommand::FinishMongoWorkbenchExecution {
                tab_id,
                text,
                result,
            } => self.finish_mongo_workbench_execution(tab_id, text, result),
            AppCommand::DeleteMongoWorkbenchRecord {
                tab_id,
                execution_id,
            } => {
                let Some(workbench) = self.mongo_workbench_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "MongoDB 命令工作台标签页不存在"));
                };
                workbench
                    .executions
                    .retain(|execution| execution.id != execution_id);
                workbench.collapsed.remove(&execution_id);
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::ClearMongoWorkbenchResults(tab_id) => {
                let Some(workbench) = self.mongo_workbench_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "MongoDB 命令工作台标签页不存在"));
                };
                workbench.executions.clear();
                workbench.collapsed.clear();
                workbench.error = None;
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::ToggleMongoWorkbenchRecordCollapse {
                tab_id,
                execution_id,
            } => {
                let Some(workbench) = self.mongo_workbench_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "MongoDB 命令工作台标签页不存在"));
                };
                if !workbench.collapsed.remove(&execution_id) {
                    workbench.collapsed.insert(execution_id);
                }
                AppEvent::TabActivated(tab_id)
            }
            _ => unreachable!("dispatch_mongo_workbench_command 收到其它命令"),
        }
    }

    fn finish_mongo_workbench_execution(
        &mut self,
        tab_id: TabId,
        text: String,
        result: std::result::Result<MongoShellResult, UserFacingError>,
    ) -> AppEvent {
        let started_at_unix_secs = current_unix_secs();
        let Some(workbench) = self.mongo_workbench_mut(tab_id) else {
            // 标签页已关闭：结果直接丢弃，不写任何状态。
            return AppEvent::TabActivated(tab_id);
        };
        workbench.running = false;
        let (output, error) = match result {
            Ok(shell) => {
                // `use <db>` 切换只影响本工作台会话，不写回连接配置。
                if let Some(database) = shell.database {
                    workbench.database = database;
                }
                (Some(shell.output), None)
            }
            // 失败也入记录：用户要在结果区看到是哪条语句、错在哪。
            Err(error) => (None, Some(error)),
        };
        let id = workbench.next_execution_id;
        workbench.next_execution_id += 1;
        workbench.executions.push(MongoWorkbenchExecution {
            id,
            text: text.trim().to_string(),
            database: workbench.database.clone(),
            output,
            error,
            elapsed_ms: 0,
            started_at_unix_secs,
        });
        workbench.error = None;
        let dirty = workbench.has_unsaved_text();
        if let Some(tab) = self.find_tab_mut(tab_id) {
            tab.dirty = dirty;
        }
        AppEvent::TabActivated(tab_id)
    }
}

#[cfg(test)]
mod mongo_workbench_dispatch_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn mongo_controller() -> (AppController, ConnectionId) {
        let mut controller = AppController::new();
        let draft = ConnectionDraft {
            name: "mongo".into(),
            kind: DatabaseKind::MongoDb,
            endpoint: Endpoint::Uri {
                uri: "mongodb://127.0.0.1:1/demo".into(),
            },
            options: BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };
        let AppEvent::ConnectionCreated(connection) =
            controller.dispatch(AppCommand::CreateConnection(draft))
        else {
            panic!("连接必须创建成功");
        };
        (controller, connection.id)
    }

    fn open_workbench(controller: &mut AppController, connection_id: ConnectionId) -> TabId {
        let AppEvent::TabOpened(tab_id) = controller.dispatch(AppCommand::OpenMongoWorkbench {
            connection_id,
            database: "demo".into(),
        }) else {
            panic!("工作台必须打开");
        };
        tab_id
    }

    fn workbench(controller: &AppController, tab_id: TabId) -> &MongoWorkbenchState {
        controller
            .state()
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::MongoWorkbench(workbench) => Some(workbench),
                _ => None,
            })
            .expect("工作台标签页存在")
    }

    fn finish_ok(controller: &mut AppController, tab_id: TabId, text: &str, output: &str) {
        controller.dispatch(AppCommand::FinishMongoWorkbenchExecution {
            tab_id,
            text: text.into(),
            result: Ok(MongoShellResult {
                output: output.into(),
                database: None,
            }),
        });
    }

    #[test]
    fn reopening_the_same_database_reuses_the_tab() {
        let (mut controller, connection_id) = mongo_controller();
        let tab_id = open_workbench(&mut controller, connection_id);
        let AppEvent::TabActivated(reused) =
            controller.dispatch(AppCommand::OpenMongoWorkbench {
                connection_id,
                database: "demo".into(),
            })
        else {
            panic!("同一连接 + 同一库必须复用");
        };
        assert_eq!(reused, tab_id);
        assert_eq!(
            controller
                .state()
                .tabs
                .iter()
                .filter(|tab| matches!(tab.kind, TabKind::MongoWorkbench(_)))
                .count(),
            1
        );
    }

    #[test]
    fn beginning_a_draft_execution_clears_the_input_and_marks_clean() {
        let (mut controller, connection_id) = mongo_controller();
        let tab_id = open_workbench(&mut controller, connection_id);
        controller.dispatch(AppCommand::UpdateMongoWorkbenchText {
            tab_id,
            text: "show dbs".into(),
        });
        assert!(controller.state().tabs[0].dirty, "有草稿时应标记未执行改动");

        controller.dispatch(AppCommand::BeginMongoWorkbenchExecution {
            tab_id,
            execution_id: None,
        });
        let state = workbench(&controller, tab_id);
        assert!(state.running);
        assert!(state.text.is_empty());
        assert!(!controller.state().tabs[0].dirty);
    }

    #[test]
    fn rerun_of_a_deleted_record_fails_instead_of_entering_running_state() {
        let (mut controller, connection_id) = mongo_controller();
        let tab_id = open_workbench(&mut controller, connection_id);
        controller.dispatch(AppCommand::FinishMongoWorkbenchExecution {
            tab_id,
            text: "show dbs".into(),
            result: Ok(MongoShellResult::default()),
        });
        let record_id = workbench(&controller, tab_id).executions[0].id;
        controller.dispatch(AppCommand::DeleteMongoWorkbenchRecord {
            tab_id,
            execution_id: record_id,
        });

        let event = controller.dispatch(AppCommand::BeginMongoWorkbenchExecution {
            tab_id,
            execution_id: Some(record_id),
        });
        assert!(matches!(event, AppEvent::Failed(_)));
        assert!(!workbench(&controller, tab_id).running);
    }

    #[test]
    fn successful_execution_appends_a_record_and_switches_database() {
        let (mut controller, connection_id) = mongo_controller();
        let tab_id = open_workbench(&mut controller, connection_id);
        controller.dispatch(AppCommand::FinishMongoWorkbenchExecution {
            tab_id,
            text: "  use shop  ".into(),
            result: Ok(MongoShellResult {
                output: "switched to db shop".into(),
                database: Some("shop".into()),
            }),
        });
        let state = workbench(&controller, tab_id);
        assert_eq!(state.database, "shop", "use 只切本会话的库");
        assert_eq!(state.executions.len(), 1);
        let record = &state.executions[0];
        assert_eq!(record.text, "use shop");
        assert!(record.is_success());
        assert_eq!(record.database, "shop");
        assert!(!state.running);
    }

    #[test]
    fn failed_execution_still_lands_in_the_result_list() {
        let (mut controller, connection_id) = mongo_controller();
        let tab_id = open_workbench(&mut controller, connection_id);
        controller.dispatch(AppCommand::FinishMongoWorkbenchExecution {
            tab_id,
            text: "db.events.insertOne({})".into(),
            result: Err(UserFacingError {
                title: "执行失败".into(),
                message: "本终端不支持写操作".into(),
                detail: None,
                retryable: false,
            }),
        });
        let state = workbench(&controller, tab_id);
        let record = &state.executions[0];
        assert!(!record.is_success());
        assert!(record.output.is_none());
        assert_eq!(
            record.error.as_ref().map(|error| error.message.as_str()),
            Some("本终端不支持写操作")
        );
    }

    #[test]
    fn record_ids_stay_unique_across_executions_and_reruns() {
        let (mut controller, connection_id) = mongo_controller();
        let tab_id = open_workbench(&mut controller, connection_id);
        finish_ok(&mut controller, tab_id, "show dbs", "admin");
        finish_ok(&mut controller, tab_id, "show collections", "events");
        let ids = workbench(&controller, tab_id)
            .executions
            .iter()
            .map(|execution| execution.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(workbench(&controller, tab_id).next_execution_id, 3);
    }

    #[test]
    fn collapse_toggles_per_record_and_clear_drops_everything() {
        let (mut controller, connection_id) = mongo_controller();
        let tab_id = open_workbench(&mut controller, connection_id);
        finish_ok(&mut controller, tab_id, "show dbs", "admin");
        finish_ok(&mut controller, tab_id, "show collections", "events");

        controller.dispatch(AppCommand::ToggleMongoWorkbenchRecordCollapse {
            tab_id,
            execution_id: 1,
        });
        assert!(workbench(&controller, tab_id).collapsed.contains(&1));
        controller.dispatch(AppCommand::ToggleMongoWorkbenchRecordCollapse {
            tab_id,
            execution_id: 1,
        });
        assert!(!workbench(&controller, tab_id).collapsed.contains(&1));

        controller.dispatch(AppCommand::ClearMongoWorkbenchResults(tab_id));
        assert!(workbench(&controller, tab_id).executions.is_empty());
    }

    #[test]
    fn empty_statement_does_not_reach_the_connector() {
        // 空语句直接回一个空结果：不该为一行空白发起一次数据库往返，也不该记成失败。
        let (mut controller, connection_id) = mongo_controller();
        let tab_id = open_workbench(&mut controller, connection_id);
        controller.dispatch(AppCommand::UpdateMongoWorkbenchText {
            tab_id,
            text: "   ".into(),
        });
        controller.dispatch(AppCommand::BeginMongoWorkbenchExecution {
            tab_id,
            execution_id: None,
        });
        let AppEvent::MongoWorkbenchRan { result, .. } =
            controller.dispatch(AppCommand::RunMongoWorkbench {
                tab_id,
                execution_id: None,
            })
        else {
            panic!("必须回传执行结果事件");
        };
        assert!(result.is_ok());
    }
}
