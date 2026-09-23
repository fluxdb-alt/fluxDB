/// 真实 PostgreSQL 元数据回归：设置 FLUXDB_PG_SMOKE 后，在本进程独立 schema
/// 验证表改名、同名重建、删列的结构身份变化；未配置则跳过，不访问用户现有表。
#[test]
fn postgres_er_rebind_rename_rebuild_and_drop_column() {
    let Some(params) = postgres_create_table_smoke_params() else {
        tracing::warn!(target: "fluxdb_app", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG ER A/B/C 冒烟");
        return;
    };
    let config = postgres_create_table_smoke_config(&params);
    let connector = PostgresConnector::with_config(config.clone());
    let database = params.4;
    let schema = format!("er_rebind_{}", std::process::id());
    let execute = |sql: String| {
        let result = connector.execute(&QueryRequest {
            connection_id: config.id,
            database: Some(database.clone()),
            session_id: None,
            schema: None,
            text: sql,
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions::default(),
        }).expect("隔离 schema 的 DDL 应成功");
        assert!(result.summaries.iter().all(|summary| summary.success),
            "隔离 schema DDL 存在语句级失败：{:?}", result.summaries);
    };
    execute(format!("CREATE SCHEMA {schema}; CREATE TABLE {schema}.customers(id bigint PRIMARY KEY); CREATE TABLE {schema}.orders(id bigint PRIMARY KEY, customer_id bigint);"));
    let snapshot = |table: &str| {
        let objects = connector.list_objects(Some(&ObjectPath {
            connection_id: config.id, database: Some(database.clone()),
            schema: Some(schema.clone()), name: schema.clone(), kind: ObjectKind::Schema,
        })).expect("读取隔离 schema 表目录");
        let object = objects.into_iter().find(|o| o.path.name == table).expect("表应存在");
        let columns = connector.list_completion_columns(Some(&database), Some(&schema), table)
            .expect("读取隔离 schema 列目录");
        let id = format!("{database}:{schema}:{table}");
        fluxdb_core::ErRebindEntity {
            entity_id: id.clone(), qualified_name: format!("{schema}.{table}"),
            stable_id: object.stable,
            columns: columns.into_iter().map(|column| fluxdb_core::ErRebindColumn {
                column_id: format!("{id}::{}", column.name),
                name: column.name, stable_id: column.stable,
            }).collect(),
        }
    };
    let old_orders = snapshot("orders");
    assert!(old_orders.stable_id.is_some(), "PG 表应提供稳定 oid");
    execute(format!("ALTER TABLE {schema}.orders RENAME TO sales_orders"));
    let sales_orders = snapshot("sales_orders");
    let id = format!("{}::customer_id", old_orders.entity_id);
    let renamed = fluxdb_core::rebind_entity(&old_orders, &[id.clone()], &[sales_orders.clone()]);
    assert_eq!(renamed.matched_entity, Some(sales_orders.entity_id.clone()));
    assert!(!renamed.entity_needs_review && !renamed.columns[0].unresolved);
    assert!(old_orders.columns.iter().all(|column| column.stable_id.is_some()),
        "PG attnum 应贯通列结构快照");
    execute(format!("ALTER TABLE {schema}.sales_orders RENAME COLUMN customer_id TO client_id"));
    let renamed_column = fluxdb_core::rebind_entity(
        &old_orders, &[id.clone()], &[snapshot("sales_orders")],
    );
    assert!(!renamed_column.columns[0].unresolved);
    assert_eq!(renamed_column.columns[0].new_column_id,
        Some(format!("{database}:{schema}:sales_orders::client_id")));

    let old_customers = snapshot("customers");
    execute(format!("DROP TABLE {schema}.customers; CREATE TABLE {schema}.customers(id bigint PRIMARY KEY, tag text);"));
    let rebuilt_customers = snapshot("customers");
    assert_ne!(old_customers.stable_id, rebuilt_customers.stable_id);
    let rebuilt = fluxdb_core::rebind_entity(
        &old_customers, &[format!("{}::id", old_customers.entity_id)], &[rebuilt_customers],
    );
    assert!(rebuilt.entity_needs_review, "同名新对象不能自动继承旧确认");

    execute(format!("ALTER TABLE {schema}.sales_orders DROP COLUMN client_id"));
    let dropped = fluxdb_core::rebind_entity(&old_orders, &[id], &[snapshot("sales_orders")]);
    assert!(dropped.columns[0].unresolved, "删列不能猜接到其它列");
    execute(format!("DROP SCHEMA {schema} CASCADE"));
}

/// 真实 PostgreSQL 可见性回归：受限角色只能看到被授权的关系，无权访问的表
/// 不出现在表目录；外键指向无权访问的表时该边必须被剔除（不泄露隐藏表名）。
/// 设置 FLUXDB_PG_SMOKE 后运行；未配置则跳过，不访问用户现有表。
#[test]
fn postgres_er_visibility_hides_unauthorized_tables() {
    let Some(params) = postgres_create_table_smoke_params() else {
        tracing::warn!(target: "fluxdb_app", "未设置 FLUXDB_PG_SMOKE，跳过真实 PG ER 可见性冒烟");
        return;
    };
    let admin_config = postgres_create_table_smoke_config(&params);
    let (host, port, _user, _password, database) = params.clone();
    let admin = PostgresConnector::with_config(admin_config.clone());
    let pid = std::process::id();
    let schema = format!("er_vis_{pid}");
    let role = format!("er_viewer_{pid}");
    let viewer_password = "er_viewer_pw_only";
    let execute = |connector: &PostgresConnector, sql: String| {
        let result = connector.execute(&QueryRequest {
            connection_id: admin_config.id,
            database: Some(database.clone()),
            session_id: None,
            schema: None,
            text: sql,
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions::default(),
        }).expect("隔离 schema 的 DDL 应成功");
        assert!(result.summaries.iter().all(|summary| summary.success),
            "隔离 schema DDL 存在语句级失败：{:?}", result.summaries);
    };
    // 幂等清理，避免上次异常残留影响本次。
    execute(&admin, format!("DROP SCHEMA IF EXISTS {schema} CASCADE; DROP ROLE IF EXISTS {role};"));
    // orders 同时引用可见表 customers 与不可见表 secret，用于验证两种边。
    execute(&admin, format!(
        "CREATE SCHEMA {schema}; \
         CREATE TABLE {schema}.customers(id bigint PRIMARY KEY); \
         CREATE TABLE {schema}.secret(id bigint PRIMARY KEY); \
         CREATE TABLE {schema}.orders(id bigint PRIMARY KEY, \
             customer_id bigint REFERENCES {schema}.customers(id), \
             secret_id bigint REFERENCES {schema}.secret(id)); \
         CREATE ROLE {role} LOGIN PASSWORD '{viewer_password}'; \
         GRANT USAGE ON SCHEMA {schema} TO {role}; \
         GRANT SELECT ON {schema}.customers, {schema}.orders TO {role};"
    ));

    // 受限角色连接：只有 customers/orders 被授权，secret 未授权。
    let viewer_config = ConnectionConfig {
        id: ConnectionId(11),
        name: "PG ER Visibility Smoke".to_string(),
        kind: DatabaseKind::Postgres,
        endpoint: Endpoint::Tcp { host: host.clone(), port, database: Some(database.clone()) },
        credential_ref: None,
        options: Default::default(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: Some(fluxdb_core::PostgresConnectionProfile {
            basic: fluxdb_core::PostgresBasicOptions {
                host: host.clone(),
                port,
                maintenance_database: database.clone(),
                username: role.clone(),
                password: fluxdb_core::SecretRef::inline(viewer_password),
            },
            ..Default::default()
        }),
    };
    let viewer = PostgresConnector::with_config(viewer_config.clone());

    // 目录：包含被授权的两表，隐藏未授权的 secret。
    let objects = viewer
        .list_objects(Some(&ObjectPath {
            connection_id: viewer_config.id,
            database: Some(database.clone()),
            schema: Some(schema.clone()),
            name: schema.clone(),
            kind: ObjectKind::Schema,
        }))
        .expect("以受限角色读取隔离 schema 表目录");
    let names: Vec<String> = objects.iter().map(|o| o.path.name.clone()).collect();
    assert!(names.iter().any(|n| n == "customers"), "被授权的表应可见：{names:?}");
    assert!(names.iter().any(|n| n == "orders"), "被授权的表应可见：{names:?}");
    assert!(!names.iter().any(|n| n == "secret"), "无权访问的表不得出现在目录：{names:?}");

    // 关系：orders→customers 保留；orders→secret 因对端不可见被剔除。
    let edges = load_er_relations_from_db_with(
        &viewer, &viewer_config, Some(&database), Some(&schema),
    )
    .expect("以受限角色读取隔离 schema 关系");
    assert!(
        edges.iter().any(|e| e.from_reference.name == "orders"
            && e.to_reference.name == "customers"),
        "两端都可见的外键应保留：{edges:?}"
    );
    assert!(
        edges.iter().all(|e| e.to_reference.name != "secret"
            && e.from_reference.name != "secret"),
        "指向无权访问表的边必须剔除，不泄露隐藏表名：{edges:?}"
    );

    execute(&admin, format!("DROP SCHEMA IF EXISTS {schema} CASCADE; DROP ROLE IF EXISTS {role};"));
}
