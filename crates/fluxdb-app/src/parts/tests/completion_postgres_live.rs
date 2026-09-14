/// T14 验收：补全候选的对象身份（schema）必须随候选带出，详情才能按
/// `(库, schema, 表)` 取到真实列。
///
/// 用 `FLUXDB_PG_SMOKE=host:port:user:password:db` 开启真实 PG 冒烟；未配置时跳过。
/// 对应 `docs/design/2026-09-12-postgresql-manual-ops.md` E2/E7：
/// - `SELECT * FROM tenant_a.order` 只出 `tenant_a.orders`，不混入 public 的其它表；
/// - 候选携带作用域 schema，详情按该身份取列（`tenant_a` 不在 search_path，索引只有表名，
///   故这里走的是「按需拉一次并写回索引」的路径）；
/// - 身份丢失（`schema=None`）时不拿其它 schema 的列伪造清单；
/// - 真实 catalog 元数据产生的 64 位表指纹能落盘并读回（表名哈希溢出 i64 时曾整份写不出）。
#[test]
fn postgres_completion_carries_schema_identity_for_documentation() {
    let Some(params) = completion_smoke_params() else {
        tracing::warn!(target: "fluxdb_app", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG 补全冒烟");
        return;
    };
    let config = completion_smoke_config(&params);
    let connection_id = config.id;
    let database = params.4.clone();
    let storage_root = temp_sqlite_path("completion-live-storage").with_extension("cache");
    let storage = fluxdb_storage::FileStorage::new(storage_root);
    let mut controller = AppController::with_mock_data();
    controller.set_completion_index_storage(storage.clone());
    controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));

    let sql = "SELECT * FROM tenant_a.order";
    let result = controller
        .query_completions_for_text(
            connection_id,
            Some(database.clone()),
            None,
            sql.to_string(),
            sql.len(),
            false,
        )
        .expect("补全请求应成功");
    let orders = result
        .items
        .iter()
        .find(|item| item.kind == QueryCompletionKind::Table && item.label == "orders")
        .expect("应出 tenant_a.orders 表候选");
    assert_eq!(
        orders.schema.as_deref(),
        Some("tenant_a"),
        "候选必须携带作用域 schema（索引桶键身份）"
    );
    assert!(
        result.items.iter().all(|item| item.label != "OrderItems"),
        "不应混入 public 的表：{:?}",
        result.items.iter().map(|item| item.label.as_str()).collect::<Vec<_>>()
    );
    assert!(
        result.items.iter().all(|item| item.label != "order"),
        "表名位置不应出现小写关键字候选 `order`"
    );

    // 详情按候选身份取列：索引未覆盖 → 按 (库, schema, 表) 拉一次并写回索引。
    match controller.completion_item_documentation(
        connection_id,
        Some(database.as_str()),
        orders.schema.as_deref(),
        orders,
        &|| false,
    ) {
        CompletionDocumentationState::Ready(text) => {
            for expected in ["id", "amount", "note"] {
                assert!(text.contains(expected), "列清单应含 {expected}: {text}");
            }
        }
        other => panic!("按身份查询应解析出列清单, got {other:?}"),
    }

    // 反证：身份丢失（schema=None）时查不到 tenant_a 的列，也不会拿别的 schema 顶替。
    match controller.completion_item_documentation(
        connection_id,
        Some(database.as_str()),
        None,
        orders,
        &|| false,
    ) {
        CompletionDocumentationState::Ready(text) => assert!(
            !text.contains("amount"),
            "无身份不应命中 tenant_a 的列: {text}"
        ),
        CompletionDocumentationState::Error(_) => {}
        other => panic!("无身份不应产出 tenant_a 的列清单, got {other:?}"),
    }

    // 按需拉取会把真实 catalog 元数据写回索引并落盘：指纹是真实表名/列名算出的 64 位哈希，
    // 必须能写入快照（哈希溢出 i64 时曾整份写不出，这里即是那条回归的真库侧证据）。
    let snapshot = storage
        .load_completion_index(&config, Some(database.as_str()), Some("tenant_a"))
        .expect("读取落盘快照不应报错")
        .expect("按身份取列后应留下 tenant_a 作用域的快照");
    assert!(
        snapshot.columns.iter().any(|column| column.column == "amount"),
        "快照应包含真实列: {:?}",
        snapshot.columns.iter().map(|column| column.column.as_str()).collect::<Vec<_>>()
    );
}

/// 读 `FLUXDB_PG_SMOKE=host:port:user:password:db`（与其它真实 PG 冒烟同一门控）。
fn completion_smoke_params() -> Option<(String, u16, String, String, String)> {
    let value = std::env::var("FLUXDB_PG_SMOKE").ok()?;
    let mut parts = value.split(':');
    let host = parts.next()?.to_string();
    let port: u16 = parts.next()?.parse().ok()?;
    let user = parts.next()?.to_string();
    let password = parts.next()?.to_string();
    let db = parts.next()?.to_string();
    Some((host, port, user, password, db))
}

fn completion_smoke_config(
    (host, port, user, password, db): &(String, u16, String, String, String),
) -> ConnectionConfig {
    ConnectionConfig {
        // 与 `AppController::with_mock_data` 的连接 id 对齐，替换后即用真实 PG 连接补全。
        id: ConnectionId(1),
        name: "PG Completion Smoke".to_string(),
        kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp {
            host: host.clone(),
            port: *port,
            database: Some(db.clone()),
        },
        credential_ref: None,
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(fluxdb_core::PostgresConnectionProfile {
            basic: fluxdb_core::PostgresBasicOptions {
                host: host.clone(),
                port: *port,
                maintenance_database: db.clone(),
                username: user.clone(),
                password: fluxdb_core::SecretRef::inline(password),
            },
            ..Default::default()
        }),
    }
}
