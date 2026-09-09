// Redis Workbench 历史体系的 App 层单测。
//
// 覆盖 `WorkbenchHistoryStore` trait 的按数据库类型路由与 scope 隔离：
// - Redis 历史写入（成功 / 失败都要进历史）
// - load_history 按 scope 过滤、最新在前
// - delete_history / clear_history 只影响匹配 scope，不污染其他连接 / 数据库 / SQL
// - SQL scope 的 load / delete / clear 复用现有 query_history，且与 Redis scope 互不串台

/// 构造一个 Redis 目标的执行记录（成功汇总：0 失败）。
fn redis_execution(connection_id: u64, database: u32, text: &str, failed: usize) -> CommandWorkbenchExecution {
    CommandWorkbenchExecution {
        id: 0,
        target: CommandExecutionTarget::Redis {
            connection_id: ConnectionId(connection_id),
            database,
        },
        text: text.to_string(),
        commands: Vec::new(),
        summary: CommandExecutionSummary {
            total: 1,
            success: if failed == 0 { 1 } else { 0 },
            failed,
            skipped: 0,
        },
        run_mode: CommandRunMode::Text,
        results_mode: CommandResultsMode::Default,
        started_at_unix_secs: 1000 + database as u64,
        elapsed_ms: 5,
    }
}

/// 造数：在 controller 里写入若干 Redis 历史（跨两个连接、两个库），并写入一条 SQL 历史。
fn seed_history(controller: &mut AppController) -> Vec<WorkbenchHistoryScope> {
    // Redis scope A：连接 1、库 0。
    controller.record_redis_workbench_history_execution(&redis_execution(1, 0, "GET a", 0));
    controller.record_redis_workbench_history_execution(&redis_execution(1, 0, "SET k v", 0));
    controller.record_redis_workbench_history_failure(ConnectionId(1), 0, "GET missing", "ERR no such key");
    // Redis scope B：连接 1、库 1（不同库，应隔离）。
    controller.record_redis_workbench_history_execution(&redis_execution(1, 1, "GET b", 0));
    // Redis scope C：连接 2、库 0（不同连接，应隔离）。
    controller.record_redis_workbench_history_execution(&redis_execution(2, 0, "GET c", 0));
    // SQL 历史：应完全独立于 Redis scope（直接构造条目标注为 SQL 查询）。
    controller.state.query_history.push(QueryHistoryEntry {
        connection_id: ConnectionId(1),
        database: Some("main".to_string()),
        text: "SELECT 1".to_string(),
        tables: Vec::new(),
        kind: QueryHistoryKind::Query,
        success: true,
        summary: QueryExecutionSummary {
            sql: "SELECT 1".to_string(),
            kind: QueryStatementKind::ResultSet,
            success: true,
            message: "1 row".to_string(),
            returned_rows: 1,
            affected_rows: 0,
            elapsed_ms: 1,
        },
        executed_at_unix_secs: 900,
        object: None,
        rollback_snapshot: None,
    });

    vec![
        WorkbenchHistoryScope::Redis { connection_id: ConnectionId(1), database: 0 },
        WorkbenchHistoryScope::Redis { connection_id: ConnectionId(1), database: 1 },
        WorkbenchHistoryScope::Redis { connection_id: ConnectionId(2), database: 0 },
    ]
}

/// 验证 Redis 历史按 scope 隔离、最新在前，且 SQL 历史完全独立。
#[test]
fn redis_history_load_routes_and_scopes_by_database_type() {
    let mut controller = AppController::new();
    let scopes = seed_history(&mut controller);

    // scope A：连接 1 / 库 0 -> 3 条，最新在前（后写入的在前）。
    let a = controller.load_history(&scopes[0], 100);
    assert_eq!(a.len(), 3, "连接1/库0 应有 3 条 Redis 历史");
    // 最新在前：SET k v 后于 GET a，失败记录最后写入应排最前。
    assert_eq!(a[0].text, "GET missing", "失败记录最后写入应在最前");
    assert!(!a[0].success, "失败记录 success 应为 false");
    assert_eq!(a[2].text, "GET a");
    assert_eq!(a[1].summary, "1 个命令 · 成功 1 · 失败 0 · 跳过 0");

    // scope B：连接 1 / 库 1 -> 仅 1 条，不混入库 0 的记录。
    let b = controller.load_history(&scopes[1], 100);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].text, "GET b");

    // scope C：连接 2 / 库 0 -> 仅 1 条，与连接 1 隔离。
    let c = controller.load_history(&scopes[2], 100);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].text, "GET c");

    // SQL scope load：走 query_history，且与 Redis 数据互不串台。
    let sql_scope = WorkbenchHistoryScope::Sql {
        connection_id: ConnectionId(1),
        database: Some("main".to_string()),
    };
    let sql = controller.load_history(&sql_scope, 100);
    assert_eq!(sql.len(), 1, "SQL scope 只应命中 SQL 查询历史");
    assert_eq!(sql[0].text, "SELECT 1");
}

/// 验证清空只影响匹配 scope，不污染其他连接 / 库 / SQL。
#[test]
fn redis_history_clear_is_scope_scoped() {
    let mut controller = AppController::new();
    let scopes = seed_history(&mut controller);

    // 清空 scope A（连接1/库0）。
    controller.clear_history(&scopes[0]);

    assert!(controller.load_history(&scopes[0], 100).is_empty(), "scope A 应被清空");
    assert_eq!(controller.load_history(&scopes[1], 100).len(), 1, "scope B 不受影响");
    assert_eq!(controller.load_history(&scopes[2], 100).len(), 1, "scope C 不受影响");

    // SQL scope 不受影响。
    let sql_scope = WorkbenchHistoryScope::Sql {
        connection_id: ConnectionId(1),
        database: Some("main".to_string()),
    };
    assert_eq!(controller.load_history(&sql_scope, 100).len(), 1, "清空 Redis 不应影响 SQL 历史");
}

/// 验证删除单条只删匹配 scope + id，且不影响同 scope 其余记录。
#[test]
fn redis_history_delete_removes_only_matching_scope_and_id() {
    let mut controller = AppController::new();
    let scopes = seed_history(&mut controller);

    let before = controller.load_history(&scopes[0], 100);
    let target_id = before[0].id; // 最新一条（GET missing）。
    controller.delete_history(&scopes[0], target_id);

    let after = controller.load_history(&scopes[0], 100);
    assert_eq!(after.len(), 2, "scope A 应删掉 1 条剩 2 条");
    assert!(after.iter().all(|item| item.id != target_id), "被删的 id 不应残留");

    // 用错误的 scope 删同一个 id：不应误删。
    controller.delete_history(&scopes[1], target_id);
    assert_eq!(controller.load_history(&scopes[0], 100).len(), 2, "跨 scope 删除应为空操作");
    assert_eq!(controller.load_history(&scopes[1], 100).len(), 1, "scope B 仍只有自己的 1 条");
}

/// 验证 limit 截断（对标 RedisInsight 每 scope 上限）。
#[test]
fn redis_history_limit_truncates_newest_first() {
    let mut controller = AppController::new();
    let mut scope = None;
    for i in 0..10u64 {
        let mut ex = redis_execution(1, 0, &format!("CMD {i}"), 0);
        // 让不同记录时间递增，便于断言保留的是最新几条。
        ex.started_at_unix_secs = 1000 + i;
        controller.record_redis_workbench_history_execution(&ex);
        scope = Some(WorkbenchHistoryScope::Redis {
            connection_id: ConnectionId(1),
            database: 0,
        });
    }
    let scope = scope.unwrap();
    let limited = controller.load_history(&scope, 30);
    // 即便不足 30，也应全部返回、最新在前。
    assert_eq!(limited.len(), 10);
    assert_eq!(limited[0].text, "CMD 9", "最新在前");

    let truncated = controller.load_history(&scope, 3);
    assert_eq!(truncated.len(), 3);
    assert_eq!(truncated[0].text, "CMD 9");
    assert_eq!(truncated[2].text, "CMD 7", "只保留最新 3 条");
}
