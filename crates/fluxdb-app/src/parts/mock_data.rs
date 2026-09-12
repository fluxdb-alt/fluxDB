fn mock_connections() -> Vec<ConnectionConfig> {
    vec![
        ConnectionConfig {
            id: ConnectionId(1),
            name: "SQLite Demo".to_string(),
            kind: DatabaseKind::Sqlite,
            endpoint: Endpoint::SqliteFile {
                path: "demo.db".into(),
                read_only: false,
            },
            credential_ref: None,
            options: demo_connection_options(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        },
        ConnectionConfig {
            id: ConnectionId(2),
            name: "MySQL Local".to_string(),
            kind: DatabaseKind::MySql,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 3306,
                database: None,
            },
            credential_ref: Some("gdb.connection.2".to_string()),
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
        },
    ]
}

fn demo_connection_options() -> BTreeMap<String, String> {
    let mut options = BTreeMap::new();
    options.insert("demo".to_string(), "true".to_string());
    options
}

fn mock_objects(connection_id: ConnectionId) -> Vec<ObjectSummary> {
    // demo 元数据集（T081）：4 张关联表，与 fluxdb-connectors/shared.rs 的 mock_objects 保持一致。
    vec![
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Product".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(504),
            modified_at: None,
            comment: Some("Products sold or used in manufacturing.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "ProductCategory".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(4),
            modified_at: None,
            comment: Some("High-level product categories.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Order".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(1200),
            modified_at: None,
            comment: Some("Sales orders referencing products and customers.".to_string()),
        },
        ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some("main".to_string()),
                schema: None,
                name: "Customer".to_string(),
                kind: ObjectKind::Table,
            },
            rows: Some(88),
            modified_at: None,
            comment: Some("Customers who placed orders.".to_string()),
        },
    ]
}

fn mock_child_objects(parent: Option<&ObjectPath>) -> Vec<ObjectSummary> {
    mock_objects(
        parent
            .map(|path| path.connection_id)
            .unwrap_or(ConnectionId(1)),
    )
}

fn test_connection(config: &ConnectionConfig) -> fluxdb_core::Result<()> {
    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => MySqlConnector::new().test_connection(config),
        DatabaseKind::Sqlite => SqliteConnector::new().test_connection(config),
        DatabaseKind::Postgres => PostgresConnector::new().test_connection(config),
        DatabaseKind::MongoDb => MockConnector::new(config.kind).test_connection(config),
        DatabaseKind::Redis => RedisConnector::new().test_connection(config),
    }
}

fn list_objects_for_connection(
    config: &ConnectionConfig,
    path: Option<&ObjectPath>,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).list_objects(path);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).list_objects(path)
        }
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone()).list_objects(path),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone()).list_objects(path),
        DatabaseKind::MongoDb => MockConnector::new(config.kind).list_objects(path),
        DatabaseKind::Redis => RedisConnector::with_config(config.clone()).list_objects(path),
    }
}

fn create_database_for_connection(
    config: &ConnectionConfig,
    request: &CreateDatabaseRequest,
) -> fluxdb_core::Result<()> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).create_database(request);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).create_database(request)
        }
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone()).create_database(request),
        DatabaseKind::Postgres => {
            PostgresConnector::with_config(config.clone()).create_database(request)
        }
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            MockConnector::new(config.kind).create_database(request)
        }
    }
}

fn create_schema_for_connection(
    config: &ConnectionConfig,
    connection_id: ConnectionId,
    schema: &str,
) -> fluxdb_core::Result<()> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).create_schema(connection_id, schema);
    }

    match config.kind {
        DatabaseKind::Postgres => {
            PostgresConnector::with_config(config.clone()).create_schema(connection_id, schema)
        }
        _ => MockConnector::new(config.kind).create_schema(connection_id, schema),
    }
}

/// 角色/ACL 真实路由：PG 走 PostgresConnector，其余走 MockConnector 默认错误。
/// 用泛型闭包转发 Connector 上的角色方法，避免为每个动作重复 match。
fn role_operation_for_connection<T, F>(
    config: &ConnectionConfig,
    operation: F,
) -> fluxdb_core::Result<T>
where
    F: FnOnce(&dyn fluxdb_core::Connector) -> fluxdb_core::Result<T>,
{
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return operation(&MockConnector::new(config.kind));
    }
    match config.kind {
        DatabaseKind::Postgres => {
            operation(&PostgresConnector::with_config(config.clone()))
        }
        _ => operation(&MockConnector::new(config.kind)),
    }
}

fn delete_database_for_connection(
    config: &ConnectionConfig,
    connection_id: ConnectionId,
    database: &str,
) -> fluxdb_core::Result<()> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).delete_database(connection_id, database);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).delete_database(connection_id, database)
        }
        DatabaseKind::Sqlite => {
            SqliteConnector::with_config(config.clone()).delete_database(connection_id, database)
        }
        DatabaseKind::Postgres => {
            PostgresConnector::with_config(config.clone()).delete_database(connection_id, database)
        }
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            MockConnector::new(config.kind).delete_database(connection_id, database)
        }
    }
}

fn execute_query_for_connection(
    config: &ConnectionConfig,
    request: &QueryRequest,
) -> fluxdb_core::Result<QueryExecutionResult> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).execute(request);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).execute(request)
        }
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone()).execute(request),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone()).execute(request),
        DatabaseKind::MongoDb | DatabaseKind::Redis => MockConnector::new(config.kind).execute(request),
    }
}

fn execute_query_for_connection_with_progress(
    config: &ConnectionConfig,
    request: &QueryRequest,
    on_summary: &mut dyn FnMut(QueryExecutionSummary),
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<QueryExecutionResult> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).execute_with_progress(
            request,
            on_summary,
            should_cancel,
        );
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).execute_with_progress(
                request,
                on_summary,
                should_cancel,
            )
        }
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone()).execute_with_progress(
            request,
            on_summary,
            should_cancel,
        ),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone())
            .execute_with_progress(request, on_summary, should_cancel),
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            MockConnector::new(config.kind).execute_with_progress(
                request,
                on_summary,
                should_cancel,
            )
        }
    }
}

/// 命令执行器（Workbench）批量分发：按「执行单元 = 单条命令」切分输入文本，
/// 在同一个连接上逐条执行并返回「每条命令一个 execution」的列表。
/// 目前只有 Redis 开放；其余后端不开放（返回空列表，不追加任何历史卡片）。
fn execute_command_workbench_commands_for_connection(
    config: &ConnectionConfig,
    request: &CommandWorkbenchRequest,
) -> fluxdb_core::Result<Vec<CommandWorkbenchExecution>> {
    match config.kind {
        DatabaseKind::Redis => RedisConnector::with_config(config.clone())
            .execute_command_workbench_commands(request),
        // SQL / 其他后端不开放 Workbench 批量执行入口。
        _ => Ok(Vec::new()),
    }
}

fn list_completion_tables_for_connection_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<CompletionTable>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind)
            .list_completion_tables_with_cancel(database, schema, filter, limit, should_cancel);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => MySqlConnector::with_config(config.clone())
            .list_completion_tables_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone())
            .list_completion_tables_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone())
            .list_completion_tables_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            MockConnector::new(config.kind)
                .list_completion_tables_with_cancel(database, schema, filter, limit, should_cancel)
        }
    }
}

fn loaded_completion_tables(
    state: &AppState,
    connection_id: ConnectionId,
    database: Option<&str>,
    schema: Option<&str>,
) -> Vec<CompletionTable> {
    state
        .connections
        .iter()
        .find(|connection| connection.config.id == connection_id)
        .into_iter()
        .flat_map(|connection| connection.objects.iter())
        .filter(|object| matches!(object.path.kind, ObjectKind::Table | ObjectKind::View))
        .filter(|object| {
            database.is_none_or(|database| {
                object
                    .path
                    .database
                    .as_deref()
                    .is_some_and(|object_database| object_database.eq_ignore_ascii_case(database))
            })
        })
        .filter(|object| {
            schema.is_none_or(|schema| {
                object
                    .path
                    .schema
                    .as_deref()
                    .is_some_and(|object_schema| object_schema.eq_ignore_ascii_case(schema))
            })
        })
        .map(|object| CompletionTable {
            database: object.path.database.clone(),
            schema: object.path.schema.clone(),
            name: object.path.name.clone(),
            kind: object.path.kind,
            comment: None,
        })
        .collect()
}

fn loaded_completion_columns(
    state: &AppState,
    connection_id: ConnectionId,
    database: Option<&str>,
    table: &str,
) -> Vec<CompletionColumn> {
    loaded_completion_columns_in_schema(state, connection_id, database, None, table)
}

/// 从已打开的数据编辑 tab 复用列元数据（避免重复拉 catalog）。
///
/// 匹配优先级：**名称精确**（PG 允许 `"Foo"` 与 `"foo"` 并存，先精确才能取到正确对象）
/// → 退回忽略大小写（MySQL/SQLite 常见的大小写差异输入）。给了 schema 时必须 schema 一致，
/// 否则跨 schema 同名表会互相取到对方的列元数据（§8.4）。
fn loaded_completion_columns_in_schema(
    state: &AppState,
    connection_id: ConnectionId,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
) -> Vec<CompletionColumn> {
    let matches = |editor: &DataEditorState, exact: bool| {
        editor.object.connection_id == connection_id
            && if exact {
                editor.object.name == table
            } else {
                editor.object.name.eq_ignore_ascii_case(table)
            }
            && database.is_none_or(|database| {
                editor
                    .object
                    .database
                    .as_deref()
                    .is_some_and(|object_database| object_database.eq_ignore_ascii_case(database))
            })
            && schema.is_none_or(|schema| {
                editor
                    .object
                    .schema
                    .as_deref()
                    .is_some_and(|object_schema| object_schema == schema)
            })
    };
    let collect = |state: &AppState, exact: bool| -> Vec<CompletionColumn> {
        state
            .tabs
            .iter()
            .filter_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if matches(editor, exact) => {
                    editor.page.as_ref().map(|page| (editor, page))
                }
                _ => None,
            })
            .flat_map(|(editor, page)| {
                page.columns.iter().map(|column| CompletionColumn {
                    database: editor.object.database.clone(),
                    schema: editor.object.schema.clone(),
                    table: editor.object.name.clone(),
                    name: column.name.clone(),
                    type_name: column.type_name.clone(),
                    nullable: column.nullable,
                    primary_key: column.primary_key,
                    comment: column.comment.clone(),
                })
            })
            .collect()
    };
    let exact = collect(state, true);
    if !exact.is_empty() {
        return exact;
    }
    collect(state, false)
}

fn list_completion_columns_for_connection(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
) -> fluxdb_core::Result<Vec<fluxdb_core::CompletionColumn>> {
    list_completion_columns_for_connection_with_cancel(config, database, schema, table, &|| false)
}

fn list_completion_columns_for_connection_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<fluxdb_core::CompletionColumn>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind)
            .list_completion_columns_with_cancel(database, schema, table, should_cancel);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => MySqlConnector::with_config(config.clone())
            .list_completion_columns_with_cancel(database, schema, table, should_cancel),
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone())
            .list_completion_columns_with_cancel(database, schema, table, should_cancel),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone())
            .list_completion_columns_with_cancel(database, schema, table, should_cancel),
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            MockConnector::new(config.kind)
                .list_completion_columns_with_cancel(database, schema, table, should_cancel)
        }
    }
}

fn list_completion_columns_for_tables_for_connection_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    tables: &[String],
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<fluxdb_core::CompletionColumn>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind)
            .list_completion_columns_for_tables_with_cancel(database, schema, tables, should_cancel);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => MySqlConnector::with_config(config.clone())
            .list_completion_columns_for_tables_with_cancel(database, schema, tables, should_cancel),
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone())
            .list_completion_columns_for_tables_with_cancel(database, schema, tables, should_cancel),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone())
            .list_completion_columns_for_tables_with_cancel(database, schema, tables, should_cancel),
        DatabaseKind::MongoDb | DatabaseKind::Redis => MockConnector::new(config.kind)
            .list_completion_columns_for_tables_with_cancel(database, schema, tables, should_cancel),
    }
}

fn list_completion_routines_for_connection_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<fluxdb_core::CompletionRoutine>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind)
            .list_completion_routines_with_cancel(database, schema, filter, limit, should_cancel);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => MySqlConnector::with_config(config.clone())
            .list_completion_routines_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone())
            .list_completion_routines_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone())
            .list_completion_routines_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::MongoDb | DatabaseKind::Redis => MockConnector::new(config.kind)
            .list_completion_routines_with_cancel(database, schema, filter, limit, should_cancel),
    }
}

fn fk_object_path(
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
) -> ObjectPath {
    ObjectPath {
        connection_id: ConnectionId(0),
        database: database.map(str::to_string),
        schema: schema.map(str::to_string),
        name: table.to_string(),
        kind: ObjectKind::Table,
    }
}

/// 拉取某张表的真实外键元数据（P2.13 FK JOIN），按连接类型分发到对应 connector。
fn list_foreign_keys_for_connection_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<fluxdb_core::ForeignKeyInfo>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind)
            .list_foreign_keys_with_cancel(&fk_object_path(database, schema, table), should_cancel);
    }

    let path = fk_object_path(database, schema, table);
    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).list_foreign_keys_with_cancel(&path, should_cancel)
        }
        DatabaseKind::Sqlite => {
            SqliteConnector::with_config(config.clone()).list_foreign_keys_with_cancel(&path, should_cancel)
        }
        DatabaseKind::Postgres => {
            PostgresConnector::with_config(config.clone()).list_foreign_keys_with_cancel(&path, should_cancel)
        }
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            MockConnector::new(config.kind).list_foreign_keys_with_cancel(&path, should_cancel)
        }
    }
}

fn list_completion_triggers_for_connection_with_cancel(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    filter: &str,
    limit: u64,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<fluxdb_core::CompletionTrigger>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind)
            .list_completion_triggers_with_cancel(database, schema, filter, limit, should_cancel);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => MySqlConnector::with_config(config.clone())
            .list_completion_triggers_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone())
            .list_completion_triggers_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone())
            .list_completion_triggers_with_cancel(database, schema, filter, limit, should_cancel),
        DatabaseKind::MongoDb | DatabaseKind::Redis => MockConnector::new(config.kind)
            .list_completion_triggers_with_cancel(database, schema, filter, limit, should_cancel),
    }
}

fn load_data_for_connection(
    config: &ConnectionConfig,
    object: &ObjectPath,
    pagination: Pagination,
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataPage> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).load_data(
            object,
            pagination.offset,
            pagination.limit,
            sort,
            filters,
        );
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => MySqlConnector::with_config(config.clone()).load_data(
            object,
            pagination.offset,
            pagination.limit,
            sort,
            filters,
        ),
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone()).load_data(
            object,
            pagination.offset,
            pagination.limit,
            sort,
            filters,
        ),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone()).load_data(
            object,
            pagination.offset,
            pagination.limit,
            sort,
            filters,
        ),
        DatabaseKind::MongoDb => MockConnector::new(config.kind).load_data(
            object,
            pagination.offset,
            pagination.limit,
            sort,
            filters,
        ),
        DatabaseKind::Redis => RedisConnector::with_config(config.clone()).load_data(
            object,
            pagination.offset,
            pagination.limit,
            sort,
            filters,
        ),
    }
}

/// 一致快照分页导出路由：各后端调用其 `export_pages`（PG 覆写为单 REPEATABLE READ 事务快照，
/// 其余用默认 load_data 逐页）。供桌面导出驱动接线。
fn export_pages_for_connection(
    config: &ConnectionConfig,
    object: &ObjectPath,
    sort: &[SortSpec],
    filters: &[FilterSpec],
    on_cancel: &dyn Fn() -> bool,
    on_page: &mut dyn FnMut(DataPage) -> bool,
) -> fluxdb_core::Result<()> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).export_pages(
            object, sort, filters, on_cancel, on_page,
        );
    }
    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).export_pages(object, sort, filters, on_cancel, on_page)
        }
        DatabaseKind::Sqlite => {
            SqliteConnector::with_config(config.clone()).export_pages(object, sort, filters, on_cancel, on_page)
        }
        DatabaseKind::Postgres => {
            PostgresConnector::with_config(config.clone()).export_pages(object, sort, filters, on_cancel, on_page)
        }
        DatabaseKind::MongoDb => MockConnector::new(config.kind).export_pages(object, sort, filters, on_cancel, on_page),
        DatabaseKind::Redis => RedisConnector::with_config(config.clone()).export_pages(object, sort, filters, on_cancel, on_page),
    }
}

/// Redis Key 列表元信息懒加载的调度入口：批量补齐给定键名的类型/值/大小/TTL。
/// demo 连接没有真实 Redis，直接返回空页（不会命中：demo 的 RedisDb 页本身也无真实键）。
fn load_redis_key_metadata_for_connection(
    config: &ConnectionConfig,
    object: &ObjectPath,
    keys: &[String],
) -> fluxdb_core::Result<DataPage> {
    if keys.is_empty() {
        return Ok(DataPage {
            columns: vec![],
            rows: vec![],
            offset: 0,
            limit: 0,
            has_more: false,
        });
    }
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return Ok(DataPage {
            columns: vec![],
            rows: vec![],
            offset: 0,
            limit: 0,
            has_more: false,
        });
    }
    RedisConnector::with_config(config.clone()).load_key_metadata(object, keys)
}

fn preview_data_export_for_connection(
    config: &ConnectionConfig,
    object: &ObjectPath,
    fields: &[String],
    sort: &[SortSpec],
    filters: &[FilterSpec],
) -> fluxdb_core::Result<DataExportPreview> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).preview_data_export(object, fields, sort, filters);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).preview_data_export(object, fields, sort, filters)
        }
        DatabaseKind::Sqlite => {
            SqliteConnector::with_config(config.clone()).preview_data_export(object, fields, sort, filters)
        }
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone())
            .preview_data_export(object, fields, sort, filters),
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            MockConnector::new(config.kind).preview_data_export(object, fields, sort, filters)
        }
    }
}

fn apply_data_changes_for_connection(
    config: &ConnectionConfig,
    changes: &DataChangeSet,
) -> fluxdb_core::Result<AppliedChangeOutcome> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).apply_changes(changes);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).apply_changes(changes)
        }
        DatabaseKind::Sqlite => SqliteConnector::with_config(config.clone()).apply_changes(changes),
        DatabaseKind::Postgres => PostgresConnector::with_config(config.clone()).apply_changes(changes),
        DatabaseKind::MongoDb => MockConnector::new(config.kind).apply_changes(changes),
        DatabaseKind::Redis => RedisConnector::with_config(config.clone()).apply_changes(changes),
    }
}

fn load_cell_binary_for_connection(
    config: &ConnectionConfig,
    object: &ObjectPath,
    identity: &RowIdentity,
    column: &str,
) -> fluxdb_core::Result<Vec<u8>> {
    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return MockConnector::new(config.kind).load_cell_binary(object, identity, column);
    }

    match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            MySqlConnector::with_config(config.clone()).load_cell_binary(object, identity, column)
        }
        DatabaseKind::Sqlite => {
            SqliteConnector::with_config(config.clone()).load_cell_binary(object, identity, column)
        }
        DatabaseKind::Postgres => {
            PostgresConnector::with_config(config.clone()).load_cell_binary(object, identity, column)
        }
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            MockConnector::new(config.kind).load_cell_binary(object, identity, column)
        }
    }
}

fn load_table_info_for_connection(
    config: &ConnectionConfig,
    object: &ObjectPath,
    tab: TableInfoTab,
) -> fluxdb_core::Result<TableInfoResult> {
    if tab == TableInfoTab::Columns {
        return Err(Error::new(ErrorKind::Internal, "字段元数据来自数据页"));
    }

    if config
        .options
        .get("demo")
        .is_some_and(|value| value == "true")
    {
        return load_table_info_from_connector(&MockConnector::new(config.kind), object, tab);
    }

    let result = match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => load_table_info_from_connector(
            &MySqlConnector::with_config(config.clone()),
            object,
            tab,
        ),
        DatabaseKind::Sqlite => load_table_info_from_connector(
            &SqliteConnector::with_config(config.clone()),
            object,
            tab,
        ),
        DatabaseKind::Postgres => load_table_info_from_connector(
            &PostgresConnector::with_config(config.clone()),
            object,
            tab,
        ),
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            load_table_info_from_connector(&MockConnector::new(config.kind), object, tab)
        }
    }?;

    Ok(match result {
        TableInfoResult::Ddl(ddl) => {
            TableInfoResult::Ddl(format_sql_text_for_dialect(&ddl, config.kind))
        }
        result => result,
    })
}

/// 取表的展示 DDL（PG 用；MySQL/SQLite 走各自方言）。设计器保存前的结构指纹校验用。
fn table_ddl_for_connection(
    config: &ConnectionConfig,
    object: &ObjectPath,
) -> fluxdb_core::Result<String> {
    let connector: Box<dyn Connector> = match config.kind {
        DatabaseKind::MySql | DatabaseKind::TiDb => {
            Box::new(MySqlConnector::with_config(config.clone()))
        }
        DatabaseKind::Sqlite => Box::new(SqliteConnector::with_config(config.clone())),
        DatabaseKind::Postgres => Box::new(PostgresConnector::with_config(config.clone())),
        DatabaseKind::MongoDb | DatabaseKind::Redis => {
            return Err(Error::new(ErrorKind::Unsupported, "该连接类型不支持表 DDL"))
        }
    };
    connector.table_ddl(object)
}

fn load_table_info_from_connector(
    connector: &dyn Connector,
    object: &ObjectPath,
    tab: TableInfoTab,
) -> fluxdb_core::Result<TableInfoResult> {
    match tab {
        TableInfoTab::Columns => Err(Error::new(ErrorKind::Internal, "字段元数据来自数据页")),
        TableInfoTab::Indexes => connector.list_indexes(object).map(TableInfoResult::Indexes),
        TableInfoTab::ForeignKeys => connector
            .list_foreign_keys(object)
            .map(TableInfoResult::ForeignKeys),
        TableInfoTab::Triggers => connector
            .list_triggers(object)
            .map(TableInfoResult::Triggers),
        TableInfoTab::Ddl => connector.table_ddl(object).map(TableInfoResult::Ddl),
    }
}
