// ---------------------------------------------------------------------------
// 表/视图悬停预览字段加载（`AppController::load_hover_columns`）最小测试。
//
// 覆盖：节点类型门控（仅 Table/View 返回字段）、真实 SQLite 表字段加载、缓存命中
// 二次调用一致性。复用 `real_sqlite_completion_env`（T086 真实 metadata 链路）。
// ---------------------------------------------------------------------------

#[test]
fn hover_columns_gated_to_table_and_view_only() {
    // 非表/视图节点：数据库节点直接返回空列表，不触发任何连接/查询。
    let controller = AppController::new();
    let db_path = ObjectPath {
        connection_id: ConnectionId(99),
        database: None,
        schema: None,
        name: "some_database".to_string(),
        kind: ObjectKind::Database,
    };
    let columns = controller.load_hover_columns(&db_path, &|| false).unwrap();
    assert!(columns.is_empty(), "非表/视图节点应返回空字段列表");
}

#[test]
fn hover_columns_loads_columns_for_sqlite_table() {
    let (config, tables, expected) = real_sqlite_completion_env("hover-load");
    assert!(tables.contains(&"products"));

    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));

    // 表节点：真实 SQLite 读取字段清单。
    let table_path = ObjectPath {
        connection_id: config.id,
        database: Some("main".to_string()),
        schema: None,
        name: "products".to_string(),
        kind: ObjectKind::Table,
    };
    let columns = controller.load_hover_columns(&table_path, &|| false).unwrap();
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert!(!columns.is_empty(), "产品表应读取到字段");
    for expect in &expected {
        assert!(names.contains(expect), "应包含字段 {expect}: {names:?}");
    }
    // 主键标记正确（id 为主键）。
    let id_col = columns.iter().find(|c| c.name == "id").expect("id 列存在");
    assert!(id_col.primary_key, "id 应为 primary_key");
}

#[test]
fn hover_columns_cache_hit_returns_identical_result() {
    let (config, _, _) = real_sqlite_completion_env("hover-cache");
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));

    let table_path = ObjectPath {
        connection_id: config.id,
        database: Some("main".to_string()),
        schema: None,
        name: "products".to_string(),
        kind: ObjectKind::Table,
    };

    // 首次：走 connector 拉取（写缓存）。
    let first = controller.load_hover_columns(&table_path, &|| false).unwrap();
    assert!(!first.is_empty());
    // 二次：命中 CompletionCache，结果一致（确定性），无需再次连库。
    let second = controller.load_hover_columns(&table_path, &|| false).unwrap();
    assert_eq!(
        first.len(),
        second.len(),
        "缓存命中应返回相同字段数"
    );
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.type_name, b.type_name);
        assert_eq!(a.primary_key, b.primary_key);
        assert_eq!(a.nullable, b.nullable);
    }
}
