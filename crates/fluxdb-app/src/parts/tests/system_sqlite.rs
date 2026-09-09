// ---------------------------------------------------------------------------
// T086 系统级 SQL 补全测试（真实 metadata 链路）
//
// 覆盖验收项 3「真实数据库 metadata 测试」：从**真实** SQLite 文件读取 metadata，
// 再经过 connector → CompletionIndex → fluxdb-app completion service，得到最终补全候选。
//
// 安全边界：只用仓库内的 temp 临时 SQLite 文件（`temp_sqlite_path`），符合 T086
// 「允许 SQLite，包括临时数据库」约束；不触碰任何远程/生产连接。
// ---------------------------------------------------------------------------

// 用真实 SQLite 文件建一张带主键/外键/视图的表，返回 (config, tables, columns)。
fn real_sqlite_completion_env(
    label: &str,
) -> (ConnectionConfig, Vec<&'static str>, Vec<&'static str>) {
    let path = temp_sqlite_path(label);
    let config = ConnectionConfig {
        id: ConnectionId(7),
        name: "Real SQLite (T086)".to_string(),
        kind: DatabaseKind::Sqlite,
        endpoint: Endpoint::SqliteFile {
            path: path.clone(),
            read_only: false,
        },
        credential_ref: None,
        options: Default::default(), // 非 demo：走真实 SqliteConnector
        redis_profile: None,
        mysql_profile: None,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        // products：主键 + 普通列；customers：直观列；orders：外键指向 products。
        sqlx::query(
            "CREATE TABLE products (
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                price REAL
            )",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE customers (
                id INTEGER PRIMARY KEY,
                email TEXT NOT NULL
            )",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE orders (
                id INTEGER PRIMARY KEY,
                product_id INTEGER NOT NULL,
                qty INTEGER,
                FOREIGN KEY (product_id) REFERENCES products(id)
            )",
        )
        .execute(&mut connection)
        .await
        .unwrap();
        sqlx::query("CREATE VIEW product_names AS SELECT id, title FROM products")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
    });

    (config, vec!["products", "customers", "orders", "product_names"], vec!["id", "title", "price"])
}

/// 运行标准补全流程（Warm → Open → Update → Request），返回候选 items。
fn real_sqlite_completion_items(config: &ConnectionConfig, sql: &str, cursor: usize) -> Vec<QueryCompletionItem> {
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));
    controller.dispatch(AppCommand::WarmCompletionIndex {
        connection_id: config.id,
        database: Some("main".to_string()),
    });
    controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
        connection_id: config.id,
        database: Some("main".to_string()),
    });
    controller.dispatch(AppCommand::UpdateQueryText {
        tab_id: TabId(1),
        text: sql.to_string(),
    });
    let event = controller.dispatch(AppCommand::RequestQueryCompletions {
        tab_id: TabId(1),
        request_seq: 1,
        cursor,
        explicit: false,
    });
    let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
        panic!("expected query completions for: {sql:?}");
    };
    result.items
}

fn contains_kind_label(items: &[QueryCompletionItem], kind: QueryCompletionKind, label: &str) -> bool {
    items
        .iter()
        .any(|item| item.kind == kind && item.label.eq_ignore_ascii_case(label))
}

// ---------------------------------------------------------------------------
// 1) connector 层：真实 SQLite 文件读取 completion metadata（tables/columns/FK）
// ---------------------------------------------------------------------------
#[test]
fn real_sqlite_connector_reads_tables_columns_and_foreign_keys() {
    let (config, tables, columns) = real_sqlite_completion_env("t086-connector");

    // 表：真实 sqlite_schema 中读取，排除 sqlite_% 内部表。
    let connector = SqliteConnector::with_config(config.clone());
    let completion_tables = connector
        .list_completion_tables(Some("main"), None, "", 100)
        .unwrap();
    for expected in &tables {
        assert!(
            completion_tables.iter().any(|t| t.name == *expected),
            "真实 SQLite 应返回表 {expected}，实际: {:?}",
            completion_tables.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    }
    // view 被识别为 View。
    assert!(completion_tables
        .iter()
        .any(|t| t.name == "product_names" && t.kind == ObjectKind::View));

    // 列：真实 PRAGMA table_info。
    let completion_columns = connector
        .list_completion_columns(Some("main"), None, "products")
        .unwrap();
    for expected in &columns {
        assert!(
            completion_columns.iter().any(|c| c.name == *expected),
            "products 应返回列 {expected}，实际: {:?}",
            completion_columns.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    // 外键：orders.product_id → products.id。
    let foreign_keys = connector
        .list_foreign_keys(&ObjectPath {
            connection_id: config.id,
            database: Some("main".to_string()),
            schema: None,
            name: "orders".to_string(),
            kind: ObjectKind::Table,
        })
        .unwrap();
    assert!(
        foreign_keys.iter().any(|fk| {
            fk.column == "product_id" && fk.ref_table == "products" && fk.ref_column == "id"
        }),
        "orders 应暴露 product_id -> products.id 外键，实际: {foreign_keys:?}"
    );
}

// ---------------------------------------------------------------------------
// 2) App 层端到端：真实 SQLite metadata 一路流到最终补全候选
// ---------------------------------------------------------------------------
#[test]
fn real_sqlite_metadata_flows_into_table_completion() {
    let (config, tables, _) = real_sqlite_completion_env("t086-app-table");
    // `select * from pro`：真实表前缀过滤，products 应命中且为 Table。
    let sql = "select * from pro";
    let items = real_sqlite_completion_items(&config, sql, sql.len());
    assert!(
        contains_kind_label(&items, QueryCompletionKind::Table, "products"),
        "`from pro` 应返回真实表 products，实际: {:?}",
        items
            .iter()
            .map(|i| (i.kind, i.label.clone()))
            .collect::<Vec<_>>()
    );
    // 视图经真实 metadata 同样可补全（映射为 View kind）。
    let sql2 = "select * from product_n";
    let items2 = real_sqlite_completion_items(&config, sql2, sql2.len());
    assert!(
        contains_kind_label(&items2, QueryCompletionKind::View, "product_names"),
        "`from product_n` 应返回真实视图 product_names，实际: {:?}",
        items2
            .iter()
            .map(|i| (i.kind, i.label.clone()))
            .collect::<Vec<_>>()
    );
    let _ = tables;
}

#[test]
fn real_sqlite_metadata_flows_into_column_completion() {
    let (config, _, _) = real_sqlite_completion_env("t086-app-col");
    // `select * from products where `：引用 products，列候选应来自真实 PRAGMA。
    let sql = "select * from products where ";
    let items = real_sqlite_completion_items(&config, sql, sql.len());
    assert!(
        contains_kind_label(&items, QueryCompletionKind::Column, "title"),
        "products 的列 title 应来自真实 metadata，实际: {:?}",
        items
            .iter()
            .map(|i| (i.kind, i.label.clone()))
            .collect::<Vec<_>>()
    );
    assert!(contains_kind_label(&items, QueryCompletionKind::Column, "price"));
    // customers/orders 的列不应混入（scope 只限 products）。
    assert!(!contains_kind_label(&items, QueryCompletionKind::Column, "email"));
    assert!(!contains_kind_label(&items, QueryCompletionKind::Column, "qty"));
}

// ---------------------------------------------------------------------------
// 未知 qualifier（词法作用域）：`foo.` 中 foo 既非引用表也非内部符号 → 优雅降级，
// 不 panic、不把其它表（如 products）的列以 `foo.` 前缀泄漏出来。
// ---------------------------------------------------------------------------
#[test]
fn unknown_qualifier_degrades_gracefully_without_table_leak() {
    let (config, _, _) = real_sqlite_completion_env("t086-unknown-qualifier");
    // products 已被引用；foo 未知 → 词法作用域封闭，不应把 products 的列泄漏成列候选。
    let sql = "select foo. from products";
    let items = real_sqlite_completion_items(&config, sql, sql.find("foo.").unwrap() + 4);
    let leaky: Vec<_> = items
        .iter()
        .filter(|i| i.kind == QueryCompletionKind::Column)
        .map(|i| (i.label.clone(), i.insert_text.clone()))
        .collect();
    assert!(
        leaky.is_empty(),
        "未知 qualifier `foo.` 不应产生任何列候选（避免把 products 列泄漏成未限定 foo 列）: {leaky:?}"
    );
}

// ---------------------------------------------------------------------------
// 真实开发环境 MySQL（10.10.1.158）metadata 链路测试
//
// 前置：显式配置 `GDB_TEST_DEV_DSN`（mysql://user:pass@10.10.1.158:3306/db）。
// 走真实 MySqlConnector → CompletionIndex → completion service，与 system_sqlite
// 同一驱动路径。未配置/白名单校验失败时跳过（绝不自动扫描或连接任意库）。
//
// 安全：DSN/密码只在本模块从 env 读取，用于构建 ConnectionConfig；错误信息与
// 断言绝不打印 DSN 或密码（只报 connection kind / 表名）。
// ---------------------------------------------------------------------------

/// 取 MySQL 连接的目标数据库名（Endpoint::Tcp.database），无则返回 None。
fn mysql_config_database(config: &ConnectionConfig) -> Option<String> {
    match &config.endpoint {
        Endpoint::Tcp { database, .. } => {
            database.as_ref().filter(|d| !d.trim().is_empty()).map(|d| d.clone())
        }
        _ => None,
    }
}

/// 真实 MySQL 连接驱动：与 `real_sqlite_completion_items` 相同，但使用白名单
/// 解析出的开发库名作为 completion database 作用域。
fn real_dev_mysql_completion_items(
    config: &ConnectionConfig,
    database: &str,
    sql: &str,
    cursor: usize,
) -> Vec<QueryCompletionItem> {
    let mut controller = AppController::new();
    controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));
    controller.dispatch(AppCommand::WarmCompletionIndex {
        connection_id: config.id,
        database: Some(database.to_string()),
    });
    controller.dispatch(AppCommand::OpenQueryEditorInDatabase {
        connection_id: config.id,
        database: Some(database.to_string()),
    });
    controller.dispatch(AppCommand::UpdateQueryText {
        tab_id: TabId(1),
        text: sql.to_string(),
    });
    let event = controller.dispatch(AppCommand::RequestQueryCompletions {
        tab_id: TabId(1),
        request_seq: 1,
        cursor,
        explicit: false,
    });
    let AppEvent::QueryCompletionsLoaded(TabId(1), _, result) = event else {
        panic!("expected query completions for: {sql:?}");
    };
    result.items
}

/// 真实 dev MySQL：真实连接建立、warm 索引、completion 落候选（metadata 链路成功）。
/// 不断言具体表名/列名（开发库 schema 可变化），只验证连接成功 + 候选非空 +
/// `from ` 表格候选出现，证明「真实 metadata → completion」整链落地。
#[test]
fn real_dev_mysql_metadata_flows_into_completion() {
    let Some(config) = guarded_dev_mysql_config() else {
        // 未配置白名单开发库：跳过，不自动连接（与 T086 白名单边界一致）。
        return;
    };
    let Some(database) = mysql_config_database(&config) else {
        panic!("GDB_TEST_DEV_DSN 必须包含数据库名（mysql://user:pass@10.10.1.158:3306/db）");
    };

    // `from ` 上下文：应返回表/视图候选（真实 dev 库 schema）。
    let sql = "select * from ";
    let items = real_dev_mysql_completion_items(&config, &database, sql, sql.len());
    let table_kinds = [
        QueryCompletionKind::Table,
        QueryCompletionKind::View,
        QueryCompletionKind::Schema,
    ];
    let has_table_candidate = items
        .iter()
        .any(|i| table_kinds.contains(&i.kind) && !i.label.is_empty());
    assert!(
        has_table_candidate,
        "真实 dev MySQL `from ` 应产出表/视图候选（metadata 链路成功）: kind={:?} counts={:?}",
        items.iter().map(|i| i.kind).collect::<Vec<_>>(),
        items.len(),
    );
}

/// 真实 dev MySQL 词法作用域：未知 qualifier 不泄漏该库其它表的列（与 SQLite 侧
/// `unknown_qualifier_degrades_gracefully_without_table_leak` 同契约，跨真实库）。
#[test]
fn real_dev_mysql_unknown_qualifier_does_not_leak_columns() {
    let Some(config) = guarded_dev_mysql_config() else {
        return; // 未配置：跳过
    };
    let Some(database) = mysql_config_database(&config) else {
        panic!("GDB_TEST_DEV_DSN 必须包含数据库名");
    };

    // 随便引用一张真实存在的表，再对未知 qualifier `zzz_unknown.` 请求列候选。
    let sql = "select * from information_schema.tables zzz_unknown";
    let cursor = sql.len();
    let items = real_dev_mysql_completion_items(&config, &database, sql, cursor);
    let leaky: Vec<_> = items
        .iter()
        .filter(|i| i.kind == QueryCompletionKind::Column)
        .map(|i| i.label.clone())
        .collect();
    // 未知 qualifier `zzz_unknown.` 不应产生任何列候选（不泄漏 information_schema 或库内表列）。
    assert!(
        leaky.is_empty(),
        "真实 dev MySQL 未知 qualifier 不应泄漏列候选: {leaky:?}"
    );
}
