// Redis Workbench 分发层单测：验证 `execute_command_workbench_commands_for_connection`
// 按数据库类型路由。Redis 走真实 `RedisConnector`，其余后端（SQL / 其他）返回空列表（不追加卡片）。

#[test]
fn workbench_dispatch_rejects_sql_and_routes_redis() {
    // SQL（SQLite）后端不开放 Workbench 入口：应返回空列表，证明没有误路由到 Redis。
    let sqlite = ConnectionConfig {
        id: ConnectionId(91),
        name: "SQLite".to_string(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: "demo.db".into(),
            read_only: false,
        },
        credential_ref: None,
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
    };
    let request = CommandWorkbenchRequest {
        target: CommandExecutionTarget::Redis {
            connection_id: ConnectionId(91),
            database: 0,
        },
        text: "GET a".to_string(),
        run_mode: CommandRunMode::Text,
        results_mode: CommandResultsMode::Default,
        batch_size: 0,
        continue_on_error: true,
        source: CommandExecutionSource::Workbench,
    };
    assert!(
        execute_command_workbench_commands_for_connection(&sqlite, &request)
            .unwrap()
            .is_empty(),
        "SQL 后端不应产出任何命令执行记录"
    );

    // Redis 后端：必须路由到 RedisConnector。本机是否有 Redis 服务不影响断言：
    // 成功则通过，失败也绝不可是 Unsupported。
    let redis = ConnectionConfig {
        id: ConnectionId(92),
        name: "Redis".to_string(),
        kind: DatabaseKind::Redis,
        endpoint: Endpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 6379,
            database: None,
        },
        credential_ref: None,
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
    };
    match execute_command_workbench_commands_for_connection(&redis, &request) {
        Ok(_) => {}
        Err(error) => assert_ne!(
            error.kind,
            ErrorKind::Unsupported,
            "Redis 后端不应返回 Unsupported"
        ),
    }
}

/// 构造一个 Redis Workbench 状态，便于测 `has_unsaved_text` 的脏标记推导。
fn redis_workbench_state(text: &str, saved_fingerprint: Option<QueryFingerprint>) -> RedisWorkbenchState {
    RedisWorkbenchState {
        connection_id: ConnectionId(92),
        database: 0,
        text: text.to_string(),
        running: false,
        executions: Vec::new(),
        error: None,
        saved_fingerprint,
        next_execution_id: 1,
        collapsed: std::collections::BTreeSet::new(),
        json_views: std::collections::BTreeSet::new(),
    }
}

#[test]
fn redis_workbench_dirty_tracks_executed_text_fingerprint() {
    // 从未执行：有非空文本即视为脏。
    assert!(!redis_workbench_state("", None).has_unsaved_text());
    assert!(redis_workbench_state("SET a 1", None).has_unsaved_text());
    // 执行成功后记录指纹：与当前文本一致 → 清除脏标记。
    let executed = redis_workbench_state(
        "SET a 1",
        Some(QueryFingerprint::for_text("SET a 1")),
    );
    assert!(!executed.has_unsaved_text());
    // 执行后再修改文本：指纹不同 → 再次变脏（标题实时复现 * 标记）。
    let edited = redis_workbench_state(
        "SET a 2",
        Some(QueryFingerprint::for_text("SET a 1")),
    );
    assert!(edited.has_unsaved_text());
}

/// 打开一个 Redis Workbench 标签页并返回其 tab_id（无需真实连接）。
fn open_workbench(controller: &mut AppController) -> TabId {
    controller.dispatch(AppCommand::OpenRedisWorkbench {
        connection_id: ConnectionId(92),
        database: 0,
    });
    controller
        .state()
        .active_tab()
        .expect("应打开 Redis Workbench 标签页")
        .id
}

/// 取当前活动的 Redis Workbench 状态引用。
fn active_workbench(controller: &AppController) -> &RedisWorkbenchState {
    let Some(tab) = controller.state().active_tab() else {
        panic!("缺少活动标签页");
    };
    match &tab.kind {
        TabKind::RedisWorkbench(workbench) => workbench,
        _ => panic!("活动标签页不是 Redis Workbench"),
    }
}

/// 构造一条合成执行记录（connector 产出的 id 恒为 0，App 层会重新分配）。
fn synthetic_execution(text: &str, failed: usize) -> CommandWorkbenchExecution {
    CommandWorkbenchExecution {
        id: 0,
        target: CommandExecutionTarget::Redis {
            connection_id: ConnectionId(92),
            database: 0,
        },
        text: text.to_string(),
        commands: Vec::new(),
        summary: fluxdb_core::CommandExecutionSummary {
            total: 1,
            success: usize::from(failed == 0),
            failed,
            skipped: 0,
        },
        run_mode: CommandRunMode::Text,
        results_mode: CommandResultsMode::Default,
        started_at_unix_secs: 0,
        elapsed_ms: 5,
    }
}

#[test]
fn redis_workbench_execute_clears_input_immediately() {
    // 点 Run 应立即清空顶部输入框（清空发生在发起底层执行之前，与执行成败无关）。
    let mut controller = AppController::with_mock_data();
    let tab_id = open_workbench(&mut controller);
    controller.dispatch(AppCommand::UpdateRedisWorkbenchText {
        tab_id,
        text: "SET a 1".to_string(),
    });
    assert_eq!(active_workbench(&controller).text, "SET a 1");

    controller.dispatch(AppCommand::ExecuteRedisWorkbench(tab_id));

    // 无论底层 Redis 是否可达，输入框都已被同步清空。
    assert!(
        active_workbench(&controller).text.is_empty(),
        "Run 后顶部输入框应被立即清空"
    );
}

#[test]
fn redis_workbench_finish_appends_record_and_assigns_id() {
    // 一次执行在结果区追加一条记录，并为 connector 产出的 execution 分配自增 id。
    let mut controller = AppController::with_mock_data();
    let tab_id = open_workbench(&mut controller);

    let event = controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("GET a", 0)),
    });
    assert!(matches!(event, AppEvent::RedisWorkbenchFinished(..)));

    let workbench = active_workbench(&controller);
    assert_eq!(workbench.executions.len(), 1);
    assert_eq!(workbench.executions[0].id, 1);
    assert_eq!(workbench.executions[0].text, "GET a");
    assert_eq!(workbench.next_execution_id, 2);

    // 再次执行：追加第二条记录，id 单调递增，互不影响。
    controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("SET a 2", 0)),
    });
    let workbench = active_workbench(&controller);
    assert_eq!(workbench.executions.len(), 2);
    assert_eq!(workbench.executions[0].id, 1);
    assert_eq!(workbench.executions[1].id, 2);
}

#[test]
fn redis_workbench_delete_removes_only_target_record() {
    // Delete 只删除指定 id 的一条记录，其它记录与草稿不受影响。
    let mut controller = AppController::with_mock_data();
    let tab_id = open_workbench(&mut controller);
    controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("GET a", 0)),
    });
    controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("GET b", 0)),
    });
    assert_eq!(active_workbench(&controller).executions.len(), 2);

    controller.dispatch(AppCommand::DeleteRedisWorkbenchRecord {
        tab_id,
        execution_id: 1,
    });

    let workbench = active_workbench(&controller);
    assert_eq!(workbench.executions.len(), 1, "只应删除 1 条记录");
    assert_eq!(workbench.executions[0].id, 2, "应保留未被删除的记录");
    assert!(workbench.executions.iter().all(|e| e.id != 1));
}

#[test]
fn redis_workbench_rerun_selects_record_text_and_keeps_draft() {
    // 记录重跑：从记录自身取命令文本执行，且不打扰（不清理）顶部草稿。
    let mut controller = AppController::with_mock_data();
    let tab_id = open_workbench(&mut controller);
    // 顶部保留一段草稿，用于验证重跑不清空它。
    controller.dispatch(AppCommand::UpdateRedisWorkbenchText {
        tab_id,
        text: "草稿内容".to_string(),
    });
    controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("GET a", 0)),
    });

    // 重跑 id=1 的记录：无论底层 Redis 是否可达，重跑都不应触碰草稿。
    controller.dispatch(AppCommand::RerunRedisWorkbenchRecord {
        tab_id,
        execution_id: 1,
    });
    assert_eq!(
        active_workbench(&controller).text,
        "草稿内容",
        "记录重跑不应清空顶部草稿"
    );

    // 不存在的记录 id：应返回失败事件，而不是静默吞掉。
    let event = controller.dispatch(AppCommand::RerunRedisWorkbenchRecord {
        tab_id,
        execution_id: 999,
    });
    assert!(matches!(event, AppEvent::Failed(_)));
}

#[test]
fn redis_workbench_toggle_collapse_toggles_record() {
    // 折叠/展开切换（需求 5）：第一次切换进入折叠态，第二次切回复原，且默认未折叠。
    let mut controller = AppController::with_mock_data();
    let tab_id = open_workbench(&mut controller);
    controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("GET a", 0)),
    });
    let record_id = active_workbench(&controller).executions[0].id;

    // 默认全部展开（collapsed 为空）。
    assert!(active_workbench(&controller).collapsed.is_empty());

    // 第一次切换：折叠该记录。
    controller.dispatch(AppCommand::ToggleRedisWorkbenchRecordCollapse {
        tab_id,
        execution_id: record_id,
    });
    assert!(active_workbench(&controller).collapsed.contains(&record_id));

    // 第二次切换：展开恢复。
    controller.dispatch(AppCommand::ToggleRedisWorkbenchRecordCollapse {
        tab_id,
        execution_id: record_id,
    });
    assert!(active_workbench(&controller).collapsed.is_empty());
}

#[test]
fn redis_workbench_toggle_json_view_membership() {
    // JSON 视图开启状态按 (execution_id, command_index) 维度记忆：首次切换加入集合，
    // 再次切换移除；不同 command_index 互不影响。
    let mut controller = AppController::with_mock_data();
    let tab_id = open_workbench(&mut controller);
    controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("JSON.GET a", 0)),
    });
    let record_id = active_workbench(&controller).executions[0].id;

    // 默认不开启任何 JSON 视图。
    assert!(active_workbench(&controller).json_views.is_empty());

    // 开启第一条子命令的 JSON 视图。
    controller.dispatch(AppCommand::ToggleRedisWorkbenchJsonView {
        tab_id,
        execution_id: record_id,
        command_index: 0,
    });
    assert!(active_workbench(&controller).json_views.contains(&(record_id, 0)));

    // 开启同执行记录的另一条子命令，互不影响。
    controller.dispatch(AppCommand::ToggleRedisWorkbenchJsonView {
        tab_id,
        execution_id: record_id,
        command_index: 1,
    });
    let workbench = active_workbench(&controller);
    assert!(workbench.json_views.contains(&(record_id, 0)));
    assert!(workbench.json_views.contains(&(record_id, 1)));

    // 再次切换同一条：关闭恢复。
    controller.dispatch(AppCommand::ToggleRedisWorkbenchJsonView {
        tab_id,
        execution_id: record_id,
        command_index: 1,
    });
    let workbench = active_workbench(&controller);
    assert!(workbench.json_views.contains(&(record_id, 0)));
    assert!(!workbench.json_views.contains(&(record_id, 1)));
}

#[test]
fn redis_workbench_delete_and_clear_prune_collapsed() {
    // Delete 只清理被删记录的折叠态；Clear 清空全部折叠态，避免残留过期 id。
    let mut controller = AppController::with_mock_data();
    let tab_id = open_workbench(&mut controller);
    controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("GET a", 0)),
    });
    controller.dispatch(AppCommand::FinishRedisWorkbenchExecution {
        tab_id,
        result: Ok(synthetic_execution("GET b", 0)),
    });
    let first_id = active_workbench(&controller).executions[0].id;
    let second_id = active_workbench(&controller).executions[1].id;
    for id in [first_id, second_id] {
        controller.dispatch(AppCommand::ToggleRedisWorkbenchRecordCollapse {
            tab_id,
            execution_id: id,
        });
    }
    assert_eq!(active_workbench(&controller).collapsed.len(), 2);
    // 同时开启两条记录的 JSON 视图（command_index 0），验证删除 / 清空也会清理该状态。
    for id in [first_id, second_id] {
        controller.dispatch(AppCommand::ToggleRedisWorkbenchJsonView {
            tab_id,
            execution_id: id,
            command_index: 0,
        });
    }
    assert_eq!(active_workbench(&controller).json_views.len(), 2);

    // 删除第一条记录：其折叠态与 JSON 视图均被清理，第二条仍保留。
    controller.dispatch(AppCommand::DeleteRedisWorkbenchRecord {
        tab_id,
        execution_id: first_id,
    });
    let workbench = active_workbench(&controller);
    assert!(!workbench.collapsed.contains(&first_id));
    assert!(workbench.collapsed.contains(&second_id));
    assert!(!workbench.json_views.contains(&(first_id, 0)));
    assert!(workbench.json_views.contains(&(second_id, 0)));

    // 清空全部结果：折叠态与 JSON 视图一并清空。
    controller.dispatch(AppCommand::ClearRedisWorkbenchResults(tab_id));
    let workbench = active_workbench(&controller);
    assert!(workbench.executions.is_empty());
    assert!(workbench.collapsed.is_empty());
    assert!(workbench.json_views.is_empty());
}
